use crate::driver;
use crate::model::ConnectionConfig;
use serde_json::{Value, json};
use uuid::Uuid;

const MAX_AGENT_QUERY_ROWS: usize = 200;

pub type PreparedDbTool = Box<dyn FnOnce() -> Result<Value, String> + Send>;

pub fn tool_catalog() -> Vec<Value> {
    vec![
        json!({
            "name": "glint_db_list_connections",
            "description": "List saved database connections in Glint DB (SQLite, MySQL, PostgreSQL). Passwords are never returned.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            },
            "annotations": { "readOnlyHint": true }
        }),
        json!({
            "name": "glint_db_list_databases",
            "description": "List databases on a saved database connection (defaults to the active connection).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "connection_id": { "type": "string", "description": "Optional UUID of the saved connection." }
                }
            },
            "annotations": { "readOnlyHint": true }
        }),
        json!({
            "name": "glint_db_list_tables",
            "description": "List tables and views in a database connection.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "connection_id": { "type": "string", "description": "Optional UUID of the saved connection." },
                    "database": { "type": "string", "description": "Optional database/schema name." }
                }
            },
            "annotations": { "readOnlyHint": true }
        }),
        json!({
            "name": "glint_db_describe_table",
            "description": "Inspect column names, types, nullability, primary keys, and defaults for a table.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "connection_id": { "type": "string", "description": "Optional UUID of the saved connection." },
                    "database": { "type": "string", "description": "Optional database/schema name." },
                    "table": { "type": "string", "description": "Table name to inspect." }
                },
                "required": ["table"]
            },
            "annotations": { "readOnlyHint": true }
        }),
        json!({
            "name": "glint_db_query",
            "description": "Execute a read-only SQL query (SELECT, WITH, SHOW, PRAGMA, DESCRIBE, EXPLAIN) against a saved database connection. Returns up to 200 rows as JSON.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "connection_id": { "type": "string", "description": "Optional UUID of the saved connection." },
                    "database": { "type": "string", "description": "Optional database/schema name." },
                    "sql": { "type": "string", "description": "Read-only SQL query to execute." }
                },
                "required": ["sql"]
            },
            "annotations": { "readOnlyHint": true }
        }),
        json!({
            "name": "glint_db_execute",
            "description": "Execute a mutating or DDL SQL statement (INSERT, UPDATE, DELETE, CREATE, ALTER, DROP) against a saved database connection.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "connection_id": { "type": "string", "description": "Optional UUID of the saved connection." },
                    "database": { "type": "string", "description": "Optional database/schema name." },
                    "sql": { "type": "string", "description": "SQL statement to execute." }
                },
                "required": ["sql"]
            },
            "annotations": { "readOnlyHint": false }
        }),
    ]
}

fn resolve_connection(
    connections: &[ConnectionConfig],
    active: Option<Uuid>,
    default_db: Option<&str>,
    args: &Value,
) -> Result<(ConnectionConfig, Option<String>), String> {
    let conn = if let Some(id_str) = args["connection_id"].as_str().filter(|s| !s.trim().is_empty()) {
        let id = Uuid::parse_str(id_str.trim())
            .map_err(|_| format!("无效的 connection_id：{id_str}"))?;
        connections
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| format!("未找到 ID 为 {id} 的数据库连接"))?
    } else if let Some(active_id) = active {
        connections
            .iter()
            .find(|c| c.id == active_id)
            .cloned()
            .ok_or("当前没有可用的激活数据库连接")?
    } else {
        connections
            .first()
            .cloned()
            .ok_or("暂无已保存的数据库连接，请先在数据库管理页面新建连接")?
    };

    let db = args["database"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            if Some(conn.id) == active {
                default_db.map(str::to_owned)
            } else {
                None
            }
        });
    Ok((conn, db))
}

fn is_read_only_sql(sql: &str) -> bool {
    let trimmed = sql.trim_start();
    let first = trimmed
        .split(|c: char| c.is_whitespace() || c == '(' || c == ';')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    matches!(
        first.as_str(),
        "SELECT" | "WITH" | "SHOW" | "PRAGMA" | "DESCRIBE" | "DESC" | "EXPLAIN" | "VALUES" | "TABLE"
    )
}

pub fn prepare(
    connections: &[ConnectionConfig],
    active: Option<Uuid>,
    default_db: Option<&str>,
    name: &str,
    args: Value,
) -> Result<PreparedDbTool, String> {
    match name {
        "glint_db_list_connections" => {
            let list: Vec<Value> = connections
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id.to_string(),
                        "name": c.name,
                        "kind": c.kind.label(),
                        "summary": c.summary(),
                        "database": c.database,
                        "active": Some(c.id) == active,
                    })
                })
                .collect();
            Ok(Box::new(move || {
                Ok(json!({ "connections": list, "count": list.len() }))
            }))
        }
        "glint_db_list_databases" => {
            let (conn, _) = resolve_connection(connections, active, default_db, &args)?;
            Ok(Box::new(move || {
                let databases = driver::list_databases(&conn)?;
                Ok(json!({
                    "connection": conn.name,
                    "kind": conn.kind.label(),
                    "databases": databases,
                }))
            }))
        }
        "glint_db_list_tables" => {
            let (conn, db) = resolve_connection(connections, active, default_db, &args)?;
            Ok(Box::new(move || {
                let tables = driver::list_tables(&conn, db.as_deref())?;
                let items: Vec<Value> = tables
                    .into_iter()
                    .map(|t| json!({ "name": t.name, "kind": if t.is_view { "view" } else { "table" } }))
                    .collect();
                Ok(json!({
                    "connection": conn.name,
                    "database": db,
                    "tables": items,
                    "count": items.len(),
                }))
            }))
        }
        "glint_db_describe_table" => {
            let table = args["table"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or("缺少必需参数 table")?
                .to_owned();
            let (conn, db) = resolve_connection(connections, active, default_db, &args)?;
            Ok(Box::new(move || {
                let cols = driver::describe_table(&conn, db.as_deref(), &table)?;
                let columns: Vec<Value> = cols
                    .into_iter()
                    .map(|c| {
                        json!({
                            "name": c.name,
                            "data_type": c.data_type,
                            "nullable": c.nullable,
                            "primary_key": c.primary_key,
                            "default_value": c.default_value,
                        })
                    })
                    .collect();
                Ok(json!({
                    "connection": conn.name,
                    "database": db,
                    "table": table,
                    "columns": columns,
                }))
            }))
        }
        "glint_db_query" => {
            let sql = args["sql"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or("缺少必需参数 sql")?
                .to_owned();
            if !is_read_only_sql(&sql) {
                return Err("glint_db_query 仅支持只读查询（SELECT/WITH/SHOW/PRAGMA/DESCRIBE/EXPLAIN）；写操作请使用 glint_db_execute".into());
            }
            let (conn, db) = resolve_connection(connections, active, default_db, &args)?;
            Ok(Box::new(move || {
                let res = driver::execute_query(&conn, db.as_deref(), &sql)?;
                let total_returned = res.rows.len();
                let truncated = total_returned > MAX_AGENT_QUERY_ROWS;
                let rows: Vec<Value> = res
                    .rows
                    .into_iter()
                    .take(MAX_AGENT_QUERY_ROWS)
                    .map(|row| {
                        let mut obj = serde_json::Map::new();
                        for (col, val) in res.columns.iter().zip(row) {
                            obj.insert(col.clone(), Value::String(val));
                        }
                        Value::Object(obj)
                    })
                    .collect();
                Ok(json!({
                    "connection": conn.name,
                    "database": db,
                    "columns": res.columns,
                    "rows": rows,
                    "row_count": total_returned,
                    "truncated": truncated,
                    "elapsed_ms": res.elapsed_ms,
                }))
            }))
        }
        "glint_db_execute" => {
            let sql = args["sql"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or("缺少必需参数 sql")?
                .to_owned();
            let (conn, db) = resolve_connection(connections, active, default_db, &args)?;
            Ok(Box::new(move || {
                let res = driver::execute_query(&conn, db.as_deref(), &sql)?;
                Ok(json!({
                    "connection": conn.name,
                    "database": db,
                    "affected_rows": res.affected_rows,
                    "message": res.message,
                    "elapsed_ms": res.elapsed_ms,
                }))
            }))
        }
        _ => Err(format!("未知的数据库工具：{name}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn agent_db_tools_inspect_schema_query_and_reject_writes_on_readonly_query() {
        let file = NamedTempFile::new().unwrap();
        let mut cfg = ConnectionConfig::sqlite("local", file.path().to_string_lossy().as_ref());
        cfg.password = "secret-should-not-leak".into();

        driver::execute_query(
            &cfg,
            None,
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)",
        )
        .unwrap();
        driver::execute_query(&cfg, None, "INSERT INTO users (id, name) VALUES (1, 'alice')")
            .unwrap();

        let conns = vec![cfg.clone()];
        let listed = prepare(&conns, Some(cfg.id), None, "glint_db_list_connections", json!({}))
            .unwrap()()
            .unwrap();
        assert_eq!(listed["count"], 1);
        assert!(!listed.to_string().contains("secret-should-not-leak"));

        let tables = prepare(&conns, Some(cfg.id), None, "glint_db_list_tables", json!({}))
            .unwrap()()
            .unwrap();
        assert_eq!(tables["tables"][0]["name"], "users");

        let desc = prepare(
            &conns,
            Some(cfg.id),
            None,
            "glint_db_describe_table",
            json!({ "table": "users" }),
        )
        .unwrap()()
        .unwrap();
        assert_eq!(desc["columns"][1]["name"], "name");

        let err = prepare(
            &conns,
            Some(cfg.id),
            None,
            "glint_db_query",
            json!({ "sql": "DELETE FROM users" }),
        )
        .err()
        .unwrap();
        assert!(err.contains("只读查询"));

        let query = prepare(
            &conns,
            Some(cfg.id),
            None,
            "glint_db_query",
            json!({ "sql": "SELECT id, name FROM users" }),
        )
        .unwrap()()
        .unwrap();
        assert_eq!(query["row_count"], 1);
        assert_eq!(query["rows"][0]["name"], "alice");
    }
}
