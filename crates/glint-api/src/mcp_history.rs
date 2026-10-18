//! The audit log lives outside the workspace blob, so reading it never copies all payloads.
use chrono::{FixedOffset, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub id: i64,
    pub started_at_ms: i64,
    pub client: String,
    pub rpc_id: String,
    pub tool: String,
    pub duration_ms: u64,
    pub success: bool,
}
impl Summary {
    pub fn date(&self) -> String {
        self.formatted("%Y-%m-%d")
    }
    pub fn time(&self) -> String {
        self.formatted("%H:%M:%S")
    }
    pub fn timestamp(&self) -> String {
        self.formatted("%Y-%m-%d %H:%M:%S")
    }
    fn formatted(&self, format: &str) -> String {
        FixedOffset::east_opt(8 * 3600)
            .unwrap()
            .timestamp_millis_opt(self.started_at_ms)
            .single()
            .map(|time| time.format(format).to_string())
            .unwrap_or_else(|| "未知时间".into())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub summary: Summary,
    pub arguments: Value,
    pub result: Value,
}
pub struct Page {
    pub rows: Vec<Summary>,
    pub total: usize,
    pub has_more: bool,
}
pub fn clip(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.into();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}
pub fn bounded(value: Value) -> Value {
    let text = value.to_string();
    if text.len() <= 65536 {
        value
    } else {
        serde_json::json!({"truncated":true,"original_bytes":text.len(),"preview":clip(&text,65536)})
    }
}
impl super::storage::Store {
    pub fn append_mcp_call(&mut self, record: &Record) -> Result<i64, String> {
        let summary = &record.summary;
        self.connection.execute("INSERT INTO mcp_calls(started_at_ms,client,rpc_id,tool,duration_ms,success,arguments,result,workspace_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![summary.started_at_ms,summary.client,summary.rpc_id,summary.tool,summary.duration_ms.min(i64::MAX as u64) as i64,summary.success,record.arguments.to_string(),record.result.to_string(),self.workspace_id])
            .map_err(|e|e.to_string())?;
        Ok(self.connection.last_insert_rowid())
    }
    pub fn mcp_call_page(
        &self,
        query: &str,
        success: Option<bool>,
        before: Option<i64>,
        limit: usize,
    ) -> Result<Page, String> {
        let query = format!(
            "%{}%",
            query
                .trim()
                .replace('!', "!!")
                .replace('%', "!%")
                .replace('_', "!_")
        );
        let filter = "workspace_id=?3 AND (?1 IS NULL OR success=?1) AND (tool LIKE ?2 ESCAPE '!' OR client LIKE ?2 ESCAPE '!' OR (success=0 AND result LIKE ?2 ESCAPE '!'))";
        let total = self
            .connection
            .query_row(
                &format!("SELECT COUNT(*) FROM mcp_calls WHERE {filter}"),
                rusqlite::params![success, query, self.workspace_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())? as usize;
        let mut statement=self.connection.prepare(&format!("SELECT id,started_at_ms,client,rpc_id,tool,duration_ms,success FROM mcp_calls WHERE {filter} AND (?4 IS NULL OR started_at_ms<(SELECT started_at_ms FROM mcp_calls WHERE id=?4 AND workspace_id=?3) OR (started_at_ms=(SELECT started_at_ms FROM mcp_calls WHERE id=?4 AND workspace_id=?3) AND id<?4)) ORDER BY started_at_ms DESC,id DESC LIMIT ?5")).map_err(|e|e.to_string())?;
        let mut rows = statement
            .query_map(
                rusqlite::params![
                    success,
                    query,
                    self.workspace_id,
                    before,
                    limit.saturating_add(1).min(i64::MAX as usize) as i64
                ],
                summary_row,
            )
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        Ok(Page {
            rows,
            total,
            has_more,
        })
    }
    pub fn mcp_call_record(&self, id: i64) -> Result<Record, String> {
        self.connection.query_row("SELECT id,started_at_ms,client,rpc_id,tool,duration_ms,success,arguments,result FROM mcp_calls WHERE id=?1 AND workspace_id=?2",rusqlite::params![id,self.workspace_id],|row|{
            let parse=|index|->rusqlite::Result<Value>{
                let text:String=row.get(index)?;
                serde_json::from_str(&text).map_err(|error|rusqlite::Error::FromSqlConversionFailure(index,rusqlite::types::Type::Text,Box::new(error)))
            };
            Ok(Record{summary:summary_row(row)?,arguments:parse(7)?,result:parse(8)?})
        }).map_err(|e|e.to_string())
    }
}
fn summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Summary> {
    Ok(Summary {
        id: row.get(0)?,
        started_at_ms: row.get(1)?,
        client: row.get(2)?,
        rpc_id: row.get(3)?,
        tool: row.get(4)?,
        duration_ms: row.get::<_, i64>(5)?.max(0) as u64,
        success: row.get(6)?,
    })
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use serde_json::json;
    pub fn record(time: i64, tool: &str, success: bool) -> Record {
        Record {
            summary: Summary {
                id: 0,
                started_at_ms: time,
                client: "Agent test".into(),
                rpc_id: "1".into(),
                tool: tool.into(),
                duration_ms: 7,
                success,
            },
            arguments: json!({"foo":"bar"}),
            result: json!({"ok":success}),
        }
    }
    #[test]
    fn history_is_persistent_groupable_and_paginated_by_time() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let mut store = super::super::storage::Store::open(&path).unwrap();
        let start = chrono::DateTime::parse_from_rfc3339("2026-10-16T15:59:59Z")
            .unwrap()
            .timestamp_millis();
        let one = store
            .append_mcp_call(&record(start, "glint_request_get", true))
            .unwrap();
        let two = store
            .append_mcp_call(&record(start + 1000, "glint_request_send", false))
            .unwrap();
        let three = store
            .append_mcp_call(&record(start + 2000, "glint_workspace_get", true))
            .unwrap();
        // Insert an older event late: grouping must use timestamps, not insertion order.
        let four = store
            .append_mcp_call(&record(start - 1000, "glint_environment_get", false))
            .unwrap();
        let page = store.mcp_call_page("", None, None, 2).unwrap();
        assert_eq!(page.total, 4);
        assert!(page.has_more);
        assert_eq!(
            page.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            [three, two]
        );
        assert_eq!(page.rows[0].date(), "2026-10-17");
        store
            .append_mcp_call(&record(start + 3000, "glint_workspace_get", true))
            .unwrap();
        let second = store.mcp_call_page("", None, Some(two), 2).unwrap();
        assert_eq!(
            second.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            [one, four]
        );
        assert_eq!(second.rows[0].date(), "2026-10-16");
        assert!(!second.has_more);
        assert_eq!(
            store
                .mcp_call_page("request_", Some(false), None, 100)
                .unwrap()
                .rows[0]
                .id,
            two
        );
        assert!(
            store
                .mcp_call_page("%", None, None, 100)
                .unwrap()
                .rows
                .is_empty()
        );
        let reopened = super::super::storage::Store::open(&path).unwrap();
        assert_eq!(
            reopened.mcp_call_page("", None, None, 100).unwrap().total,
            5
        );
        assert_eq!(
            reopened.mcp_call_record(two).unwrap().result,
            json!({"ok":false})
        );
    }
    #[test]
    fn history_payload_bounds_preserve_json_and_utf8() {
        let bounded = bounded(json!({"text":"你好🌏".repeat(40000)}));
        assert_eq!(bounded["truncated"], true);
        assert!(bounded["preview"].as_str().unwrap().len() <= 65536);
        assert!(bounded["original_bytes"].as_u64().unwrap() > 65536);
        assert!(serde_json::from_str::<Value>(&bounded.to_string()).is_ok());
    }
}
