//! Write-ahead snapshots survive a failed write, interrupted request or lost SSH reply.
use crate::{database, edits::Change, tools::ToolContext};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    pub change: Change,
    pub tool: String,
    pub subagent: bool,
    pub status: String,
    pub error: Option<String>,
    pub before_chars: Option<usize>,
    pub after_chars: usize,
}
fn open(anchor: &Path) -> Result<Connection, String> {
    let db = database::open(anchor)?;
    db.pragma_update(None, "synchronous", "FULL")
        .map_err(|e| e.to_string())?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS edit_journal(id TEXT PRIMARY KEY,session_id TEXT,project TEXT,partner TEXT,payload TEXT NOT NULL,created TEXT NOT NULL);CREATE INDEX IF NOT EXISTS edit_journal_session ON edit_journal(session_id,created);").map_err(|e|e.to_string())?;
    Ok(db)
}
pub fn prepare(context: &ToolContext, change: &Change, tool: &str) -> Result<(), String> {
    let entry = Entry {
        change: change.clone(),
        tool: tool.into(),
        subagent: matches!(context.delegation, crate::subagent::Delegation::Child),
        status: "prepared".into(),
        error: None,
        before_chars: change.before.as_deref().map(|s| s.chars().count()),
        after_chars: change.after.chars().count(),
    };
    open(&context.memory.anchor)?
        .execute(
            "INSERT INTO edit_journal VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                change.id.to_string(),
                context.session_id.map(|id| id.to_string()),
                context
                    .root
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned()),
                context.memory.partner.map(|id| id.to_string()),
                serde_json::to_string(&entry).map_err(|e| e.to_string())?,
                chrono::Local::now().to_rfc3339()
            ],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}
pub fn finish(
    anchor: &Path,
    id: uuid::Uuid,
    status: &str,
    error: Option<&str>,
) -> Result<(), String> {
    open(anchor)?.execute("UPDATE edit_journal SET payload=json_set(payload,'$.status',?2,'$.error',?3) WHERE id=?1",params![id.to_string(),status,error]).map_err(|e|e.to_string())?;
    Ok(())
}
pub fn entries(
    anchor: &Path,
    project: &Path,
    partner: Option<uuid::Uuid>,
    session: uuid::Uuid,
    change: Option<uuid::Uuid>,
) -> Result<Vec<Entry>, String> {
    let db = open(anchor)?;
    let mut query=db.prepare("SELECT CASE WHEN ?4 IS NULL THEN json_set(payload,'$.change.before',NULL,'$.change.after','') ELSE payload END FROM edit_journal WHERE session_id=?1 AND project=?2 AND (?3 IS NULL OR partner IS NULL OR partner=?3) AND (?4 IS NULL OR id=?4) ORDER BY created,id").map_err(|e|e.to_string())?;
    query
        .query_map(
            params![
                session.to_string(),
                project.to_string_lossy(),
                partner.map(|id| id.to_string()),
                change.map(|id| id.to_string())
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?
        .map(|row| {
            serde_json::from_str(&row.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
        })
        .collect()
}
