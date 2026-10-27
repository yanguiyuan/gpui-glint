//! Local SQLite state. Legacy JSON is imported by each domain only after it
//! has been parsed and validated; original files remain untouched.
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn path(anchor: &Path) -> PathBuf {
    anchor
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("glint-agent.sqlite3")
}
pub fn open(anchor: &Path) -> Result<Connection, String> {
    let path = path(anchor);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let connection = Connection::open(path).map_err(|e| e.to_string())?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    connection
        .execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| e.to_string())?;
    let journal: String = connection
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if !journal.eq_ignore_ascii_case("wal") {
        // Simultaneous first opens may race while switching journal modes.
        // SQLite can report BUSY here without invoking the busy handler.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match connection.execute_batch("PRAGMA journal_mode=WAL;") {
                Ok(()) => break,
                Err(error)
                    if matches!(
                        error.sqlite_error_code(),
                        Some(
                            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                        )
                    ) && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => return Err(format!("无法启用 SQLite WAL：{error}")),
            }
        }
    }
    let version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if version > 2 {
        return Err("数据库版本高于当前应用，请使用较新版本打开".into());
    }
    if version < 2 {
        connection
            .execute_batch("BEGIN IMMEDIATE;")
            .map_err(|error| error.to_string())?;
        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if version == 0 {
            connection.execute_batch(r#"CREATE TABLE app_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE projects(path TEXT PRIMARY KEY, ordinal INTEGER NOT NULL);
CREATE TABLE sessions(id TEXT PRIMARY KEY, ordinal INTEGER NOT NULL, project TEXT, title TEXT NOT NULL, updated TEXT NOT NULL, draft TEXT NOT NULL, payload TEXT NOT NULL);
CREATE INDEX sessions_project ON sessions(project, updated DESC);
CREATE TABLE messages(session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE, ordinal INTEGER NOT NULL, role TEXT NOT NULL, text TEXT NOT NULL, time TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(session_id, ordinal));
CREATE TABLE providers(id TEXT PRIMARY KEY, ordinal INTEGER NOT NULL, payload TEXT NOT NULL);
CREATE TABLE schedules(id TEXT PRIMARY KEY, run_at INTEGER NOT NULL, enabled INTEGER NOT NULL, running INTEGER NOT NULL, payload TEXT NOT NULL);
CREATE INDEX schedules_due ON schedules(enabled, running, run_at);
CREATE TABLE memories(id TEXT PRIMARY KEY, project TEXT, title TEXT NOT NULL, summary TEXT NOT NULL, content TEXT NOT NULL, keywords TEXT NOT NULL, terms TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, source TEXT NOT NULL, fingerprint TEXT NOT NULL, UNIQUE(project, fingerprint));
CREATE VIRTUAL TABLE memory_fts USING fts5(id UNINDEXED, terms, tokenize='unicode61');
CREATE VIRTUAL TABLE memory_grams USING fts5(id UNINDEXED, terms, tokenize='unicode61');
CREATE UNIQUE INDEX memory_unique_scope ON memories(ifnull(project,''),fingerprint);
CREATE TABLE memory_settings(id INTEGER PRIMARY KEY CHECK(id=1), payload TEXT NOT NULL);
CREATE TABLE memory_cursors(session_id TEXT PRIMARY KEY, message_count INTEGER NOT NULL, byte_offset INTEGER NOT NULL DEFAULT 0);
CREATE TABLE memory_archive_jobs(id TEXT PRIMARY KEY, scheduled_day TEXT NOT NULL, status TEXT NOT NULL, started_at TEXT NOT NULL, finished_at TEXT, error TEXT, lease_until TEXT);
CREATE INDEX memory_archive_day ON memory_archive_jobs(scheduled_day, status);
PRAGMA user_version=2;"#).map_err(|e| e.to_string())?;
        }
        if version == 1 {
            connection.execute_batch("CREATE VIRTUAL TABLE memory_grams USING fts5(id UNINDEXED,terms,tokenize='unicode61'); CREATE UNIQUE INDEX memory_unique_scope ON memories(ifnull(project,''),fingerprint); ALTER TABLE memory_cursors ADD COLUMN byte_offset INTEGER NOT NULL DEFAULT 0; ALTER TABLE memory_archive_jobs ADD COLUMN lease_until TEXT; PRAGMA user_version=2;").map_err(|e|e.to_string())?;
        }
        connection
            .execute_batch("COMMIT;")
            .map_err(|error| error.to_string())?;
    }
    Ok(connection)
}
fn meta(connection: &Connection, key: &str) -> Result<Option<Value>, String> {
    let value: Option<String> = connection
        .query_row("SELECT value FROM app_meta WHERE key=?1", [key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|e| e.to_string())?;
    value
        .map(|value| serde_json::from_str(&value).map_err(|e| e.to_string()))
        .transpose()
}
fn set_meta(connection: &Connection, key: &str, value: &Value) -> Result<(), String> {
    connection.execute("INSERT INTO app_meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value WHERE value!=excluded.value", params![key,value.to_string()]).map_err(|e| e.to_string())?;
    Ok(())
}
fn strings(connection: &Connection, sql: &str) -> Result<Vec<String>, String> {
    let mut stmt = connection.prepare(sql).map_err(|e| e.to_string())?;
    stmt.query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())
}
pub fn load_history(anchor: &Path) -> Result<Option<Value>, String> {
    let mut database = open(anchor)?;
    let connection = database.transaction().map_err(|e| e.to_string())?;
    let Some(mut history) = meta(&connection, "history")? else {
        return Ok(None);
    };
    history["projects"] = json!(strings(
        &connection,
        "SELECT path FROM projects ORDER BY ordinal"
    )?);
    let mut sessions = Vec::new();
    let mut stmt = connection
        .prepare("SELECT id,project,title,updated,draft,payload FROM sessions ORDER BY ordinal")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (id, project, title, updated, draft, payload) = row.map_err(|e| e.to_string())?;
        let mut session: Value = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
        session["id"] = json!(id);
        session["project"] = json!(project);
        session["title"] = json!(title);
        session["updated"] = json!(updated);
        session["draft"] = json!(draft);
        let mut message_stmt = connection
            .prepare(
                "SELECT role,text,time,payload FROM messages WHERE session_id=?1 ORDER BY ordinal",
            )
            .map_err(|e| e.to_string())?;
        let messages = message_stmt
            .query_map([&id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .map(|row| {
                let (role, text, time, payload) = row.map_err(|e| e.to_string())?;
                let mut message: Value =
                    serde_json::from_str(&payload).map_err(|e| e.to_string())?;
                message["role"] = json!(role);
                message["text"] = json!(text);
                message["time"] = json!(time);
                Ok(message)
            })
            .collect::<Result<Vec<Value>, String>>()?;
        session["messages"] = json!(messages);
        sessions.push(session);
    }
    history["sessions"] = json!(sessions);
    Ok(Some(history))
}
pub fn save_history(anchor: &Path, history: &crate::model::History) -> Result<(), String> {
    let mut connection = open(anchor)?;
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    transaction.execute_batch("CREATE TEMP TABLE keep_sessions(id TEXT PRIMARY KEY); CREATE TEMP TABLE keep_projects(path TEXT PRIMARY KEY);").map_err(|e|e.to_string())?;
    for (ordinal, project) in history.projects.iter().enumerate() {
        let path = project.to_string_lossy();
        transaction
            .execute("INSERT INTO keep_projects VALUES(?1)", [path.as_ref()])
            .map_err(|e| e.to_string())?;
        transaction.execute("INSERT INTO projects(path,ordinal) VALUES(?1,?2) ON CONFLICT(path) DO UPDATE SET ordinal=excluded.ordinal WHERE ordinal!=excluded.ordinal",params![path.as_ref(),ordinal as i64]).map_err(|e|e.to_string())?;
    }
    transaction
        .execute(
            "DELETE FROM projects WHERE path NOT IN(SELECT path FROM keep_projects)",
            [],
        )
        .map_err(|e| e.to_string())?;
    for (ordinal, session) in history.sessions.iter().enumerate() {
        let id = session.id.to_string();
        transaction
            .execute("INSERT INTO keep_sessions VALUES(?1)", [&id])
            .map_err(|e| e.to_string())?;
        let mut payload = serde_json::to_value(session).map_err(|e| e.to_string())?;
        let map = payload.as_object_mut().unwrap();
        for key in ["id", "project", "title", "updated", "draft", "messages"] {
            map.remove(key);
        }
        transaction.execute("INSERT INTO sessions VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET ordinal=excluded.ordinal,project=excluded.project,title=excluded.title,updated=excluded.updated,draft=excluded.draft,payload=excluded.payload WHERE ordinal!=excluded.ordinal OR project IS NOT excluded.project OR title!=excluded.title OR updated!=excluded.updated OR draft!=excluded.draft OR payload!=excluded.payload",params![id,ordinal as i64,session.project.as_ref().map(|path|path.to_string_lossy().into_owned()),session.title,session.updated.to_rfc3339(),session.draft,payload.to_string()]).map_err(|e|e.to_string())?;
        for (index, message) in session.messages.iter().enumerate() {
            let mut payload = serde_json::to_value(message).map_err(|e| e.to_string())?;
            let role = payload["role"].as_str().unwrap_or("Notice").to_owned();
            let map = payload.as_object_mut().unwrap();
            for key in ["role", "text", "time"] {
                map.remove(key);
            }
            transaction.execute("INSERT INTO messages VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(session_id,ordinal) DO UPDATE SET role=excluded.role,text=excluded.text,time=excluded.time,payload=excluded.payload WHERE role!=excluded.role OR text!=excluded.text OR time!=excluded.time OR payload!=excluded.payload",params![id,index as i64,role,message.text,message.time.to_rfc3339(),payload.to_string()]).map_err(|e|e.to_string())?;
        }
        transaction
            .execute(
                "DELETE FROM messages WHERE session_id=?1 AND ordinal>=?2",
                params![id, session.messages.len() as i64],
            )
            .map_err(|e| e.to_string())?;
    }
    transaction
        .execute(
            "DELETE FROM sessions WHERE id NOT IN(SELECT id FROM keep_sessions)",
            [],
        )
        .map_err(|e| e.to_string())?;
    let mut state = serde_json::to_value(history).map_err(|e| e.to_string())?;
    state.as_object_mut().unwrap().remove("sessions");
    state.as_object_mut().unwrap().remove("projects");
    set_meta(&transaction, "history", &state)?;
    transaction.commit().map_err(|e| e.to_string())
}
pub fn load_catalog(anchor: &Path) -> Result<Option<Value>, String> {
    let mut database = open(anchor)?;
    let connection = database.transaction().map_err(|e| e.to_string())?;
    let Some(mut catalog) = meta(&connection, "catalog")? else {
        return Ok(None);
    };
    catalog["providers"] = json!(
        strings(
            &connection,
            "SELECT payload FROM providers ORDER BY ordinal"
        )?
        .into_iter()
        .map(|value| serde_json::from_str::<Value>(&value).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?
    );
    Ok(Some(catalog))
}
pub fn save_catalog(
    anchor: &Path,
    catalog: &crate::provider_catalog::ProviderCatalog,
) -> Result<(), String> {
    let mut connection = open(anchor)?;
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    transaction
        .execute("DELETE FROM providers", [])
        .map_err(|e| e.to_string())?;
    for (ordinal, provider) in catalog.providers.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO providers VALUES(?1,?2,?3)",
                params![
                    provider.id.to_string(),
                    ordinal as i64,
                    serde_json::to_string(provider).map_err(|e| e.to_string())?
                ],
            )
            .map_err(|e| e.to_string())?;
    }
    set_meta(
        &transaction,
        "catalog",
        &json!({"active":catalog.active,"active_model":catalog.active_model}),
    )?;
    transaction.commit().map_err(|e| e.to_string())
}
pub fn load_schedules(anchor: &Path) -> Result<Option<Vec<crate::schedules::Schedule>>, String> {
    let connection = open(anchor)?;
    if meta(&connection, "schedules")?.is_none() {
        return Ok(None);
    }
    strings(
        &connection,
        "SELECT payload FROM schedules ORDER BY run_at,id",
    )?
    .iter()
    .map(|value| serde_json::from_str(value).map_err(|e| e.to_string()))
    .collect::<Result<Vec<_>, _>>()
    .map(Some)
}
pub fn save_schedules(anchor: &Path, tasks: &[crate::schedules::Schedule]) -> Result<(), String> {
    let mut connection = open(anchor)?;
    let transaction = connection.transaction().map_err(|e| e.to_string())?;
    transaction
        .execute("DELETE FROM schedules", [])
        .map_err(|e| e.to_string())?;
    for task in tasks {
        put_schedule(&transaction, task)?;
    }
    set_meta(&transaction, "schedules", &json!(true))?;
    transaction.commit().map_err(|e| e.to_string())
}
pub fn put_schedule(
    connection: &Connection,
    task: &crate::schedules::Schedule,
) -> Result<(), String> {
    connection.execute("INSERT INTO schedules VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET run_at=excluded.run_at,enabled=excluded.enabled,running=excluded.running,payload=excluded.payload",params![task.id.to_string(),task.run_at.timestamp(),task.enabled,task.running,serde_json::to_string(task).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::History, provider::Config, provider_catalog::ProviderCatalog,
        schedules::ScheduleStore,
    };

    #[test]
    fn legacy_domains_import_once_into_one_database_and_preserve_originals() {
        let dir = tempfile::tempdir().unwrap();
        let history_path = dir.path().join("sessions.json");
        let catalog_path = dir.path().join("providers.json");
        let schedule_path = dir.path().join("schedules.json");
        let mut history = History::default();
        history.projects.push("/empty-project".into());
        history.active_mut().draft = "草稿".into();
        let catalog = ProviderCatalog {
            providers: vec![Config::default()],
            active: None,
            active_model: String::new(),
        };
        let old = serde_json::to_vec(&history).unwrap();
        std::fs::write(&history_path, &old).unwrap();
        std::fs::write(&catalog_path, serde_json::to_vec(&catalog).unwrap()).unwrap();
        std::fs::write(&schedule_path, b"[]").unwrap();
        History::load(&history_path).unwrap();
        ProviderCatalog::load(&catalog_path, &[]).unwrap();
        ScheduleStore::open(schedule_path.clone()).unwrap();
        assert_eq!(path(&history_path), path(&catalog_path));
        let conn = open(&history_path).unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM providers", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(std::fs::read(&history_path).unwrap(), old);
        history.active_mut().draft = "数据库中的新草稿".into();
        history.save(&history_path).unwrap();
        assert_eq!(
            History::load(&history_path).unwrap().active().draft,
            "数据库中的新草稿"
        );
        assert_eq!(std::fs::read(&history_path).unwrap(), old);
        assert_eq!(
            History::load(&history_path).unwrap().projects,
            history.projects
        );
    }
    #[test]
    fn concurrent_first_open_and_independent_domain_writes_preserve_each_other() {
        for _ in 0..16 {
            let dir = tempfile::tempdir().unwrap();
            let anchor = dir.path().join("sessions.json");
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
            let workers: Vec<_> = (0..3)
                .map(|i| {
                    let anchor = anchor.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        match i {
                            0 => save_history(&anchor, &History::default()),
                            1 => save_catalog(
                                &anchor,
                                &ProviderCatalog {
                                    providers: vec![Config::default()],
                                    active: None,
                                    active_model: String::new(),
                                },
                            ),
                            _ => save_schedules(&anchor, &[]),
                        }
                        .unwrap();
                    })
                })
                .collect();
            for worker in workers {
                worker.join().unwrap();
            }
            assert!(load_history(&anchor).unwrap().is_some());
            assert_eq!(
                load_catalog(&anchor).unwrap().unwrap()["providers"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
            assert!(load_schedules(&anchor).unwrap().is_some());
        }
    }
    #[test]
    fn newer_database_is_rejected_without_rewriting_version() {
        let dir = tempfile::tempdir().unwrap();
        let anchor = dir.path().join("sessions.json");
        let conn = open(&anchor).unwrap();
        conn.execute_batch("PRAGMA user_version=999").unwrap();
        assert!(open(&anchor).is_err());
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
                .unwrap(),
            999
        );
    }
}
