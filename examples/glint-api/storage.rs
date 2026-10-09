use super::{EnvironmentVariables, HttpResponse, Request, RequestGroups, TraceEntry};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
pub struct Workspace {
    pub requests: Vec<Request>,
    pub collections: Vec<RequestGroups>,
    pub environments: Vec<String>,
    pub variables: Vec<EnvironmentVariables>,
    pub traces: Vec<Vec<TraceEntry>>,
    pub responses: Vec<Option<HttpResponse>>,
    pub previews: Vec<Option<String>>,
}
impl Default for Workspace {
    fn default() -> Self {
        Self {
            requests: vec![],
            collections: vec![],
            environments: vec!["不使用环境".into()],
            variables: vec![EnvironmentVariables::default()],
            traces: vec![],
            responses: vec![],
            previews: vec![],
        }
    }
}
impl Workspace {
    fn validate(&self) -> Result<(), String> {
        if self.environments.first().map(String::as_str) != Some("不使用环境")
            || self.environments.len() != self.variables.len()
            || self.requests.len() != self.traces.len()
            || self.requests.len() != self.responses.len()
            || self.requests.len() != self.previews.len()
        {
            return Err("工作区数据结构不完整".into());
        }
        for request in &self.requests {
            if !self
                .collections
                .iter()
                .any(|c| c.collection == request.collection && c.names.contains(&request.group))
                || std::iter::once(&request.draft)
                    .chain(request.versions.iter().map(|r| &r.draft))
                    .chain(request.history.iter().map(|r| &r.draft))
                    .any(|d| d.environment >= self.environments.len())
            {
                return Err("请求引用了不存在的集合、分组或环境".into());
            }
        }
        Ok(())
    }
}

pub struct Store {
    connection: Connection,
    last_saved: Option<String>,
    #[cfg(test)]
    pub(super) serializations: usize,
}
impl Store {
    pub fn default_path() -> Result<PathBuf, String> {
        if let Some(path) = std::env::var_os("GLINT_API_DB_PATH") {
            return Ok(path.into());
        }
        let home = std::env::var_os("HOME").ok_or("无法确定用户数据目录")?;
        #[cfg(target_os = "macos")]
        let path =
            PathBuf::from(home).join("Library/Application Support/Glint API/workspace.sqlite3");
        #[cfg(not(target_os = "macos"))]
        let path = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(home).join(".local/share"))
            .join("glint-api/workspace.sqlite3");
        Ok(path)
    }
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let connection = Connection::open(path).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        connection
            .busy_timeout(std::time::Duration::from_secs(2))
            .map_err(|e| e.to_string())?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if version > 1 {
            return Err("数据库版本较新，请使用更新版本的 Glint API".into());
        }
        connection.execute_batch("BEGIN; CREATE TABLE IF NOT EXISTS workspace (id INTEGER PRIMARY KEY CHECK(id = 1), data TEXT NOT NULL); PRAGMA user_version = 1; COMMIT;").map_err(|e| e.to_string())?;
        Ok(Self {
            connection,
            last_saved: None,
            #[cfg(test)]
            serializations: 0,
        })
    }
    pub fn load(&mut self) -> Result<Workspace, String> {
        let data: Option<String> = self
            .connection
            .query_row("SELECT data FROM workspace WHERE id = 1", [], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())?;
        let workspace = match &data {
            Some(data) => serde_json::from_str(data).map_err(|e| format!("无法读取工作区：{e}"))?,
            None => Workspace::default(),
        };
        workspace.validate()?;
        self.last_saved = data;
        Ok(workspace)
    }
    pub fn save(&mut self, workspace: &Workspace) -> Result<(), String> {
        workspace.validate()?;
        #[cfg(test)]
        {
            self.serializations += 1;
        }
        let data = serde_json::to_string(workspace).map_err(|e| e.to_string())?;
        if self.last_saved.as_ref() == Some(&data) {
            return Ok(());
        }
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        transaction.execute("INSERT INTO workspace(id, data) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET data = excluded.data", [&data]).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        self.last_saved = Some(data);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Draft, Record};
    #[test]
    fn sqlite_reopen_preserves_workspace_and_deletion_without_seeding() {
        let path = std::env::temp_dir().join(format!(
            "glint-api-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        {
            let mut store = Store::open(&path).unwrap();
            let mut data = store.load().unwrap();
            assert!(data.requests.is_empty() && data.collections.is_empty());
            assert_eq!(data.environments.len(), 1);
            data.collections.push(RequestGroups {
                collection: "My API".into(),
                names: vec!["未分组".into()],
            });
            data.environments.push("Test".into());
            let mut variables = EnvironmentVariables::default();
            variables.save(None, "token", "test-secret", true).unwrap();
            data.variables.push(variables);
            let draft = Draft {
                environment: 1,
                url: "https://example.test/".into(),
                ..Draft::default()
            };
            let record = Record {
                draft: draft.clone(),
                label: "v1".into(),
                time: "now".into(),
                detail: String::new(),
                elapsed_ms: Some(42),
            };
            data.requests.push(Request {
                name: "Request".into(),
                collection: "My API".into(),
                group: "未分组".into(),
                draft,
                versions: vec![record.clone()],
                history: vec![record],
            });
            data.traces.push(vec![]);
            data.responses.push(Some(HttpResponse {
                status: 201,
                headers: vec![("X-Test".into(), "yes".into())],
                text: "response".into(),
                bytes: 8,
                json: None,
            }));
            data.previews.push(Some("GET https://example.test/".into()));
            store.save(&data).unwrap();
        }
        {
            let mut store = Store::open(&path).unwrap();
            let mut data = store.load().unwrap();
            assert_eq!(data.variables[1].variables[0].value, "test-secret");
            assert!(data.variables[1].variables[0].secret);
            assert_eq!(data.requests[0].history[0].elapsed_ms, Some(42));
            assert_eq!(data.responses[0].as_ref().unwrap().status, 201);
            data.requests.clear();
            data.traces.clear();
            data.responses.clear();
            data.previews.clear();
            data.collections.clear();
            data.variables[1].replace(&[]).unwrap();
            store.save(&data).unwrap();
        }
        let data = Store::open(&path).unwrap().load().unwrap();
        assert!(
            data.requests.is_empty()
                && data.collections.is_empty()
                && data.variables[1].variables.is_empty()
        );
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn failed_transaction_preserves_saved_data_and_can_retry() {
        let mut store = Store {
            connection: Connection::open_in_memory().unwrap(),
            last_saved: None,
            #[cfg(test)]
            serializations: 0,
        };
        store
            .connection
            .execute_batch("CREATE TABLE workspace(id INTEGER PRIMARY KEY, data TEXT NOT NULL);")
            .unwrap();
        let mut data = Workspace::default();
        store.save(&data).unwrap();
        store.connection.execute_batch("CREATE TRIGGER reject_update BEFORE UPDATE ON workspace BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
        data.environments.push("New environment".into());
        data.variables.push(EnvironmentVariables::default());
        assert!(store.save(&data).is_err());
        assert_eq!(store.load().unwrap().environments.len(), 1);
        store
            .connection
            .execute_batch("DROP TRIGGER reject_update;")
            .unwrap();
        store.save(&data).unwrap();
        assert_eq!(store.load().unwrap().environments.len(), 2);
    }
    #[test]
    fn malformed_storage_is_not_overwritten() {
        let mut store = Store {
            connection: Connection::open_in_memory().unwrap(),
            last_saved: None,
            #[cfg(test)]
            serializations: 0,
        };
        store.connection.execute_batch("CREATE TABLE workspace(id INTEGER PRIMARY KEY, data TEXT); INSERT INTO workspace VALUES(1, 'bad json');").unwrap();
        assert!(store.load().is_err());
        let data: String = store
            .connection
            .query_row("SELECT data FROM workspace", [], |r| r.get(0))
            .unwrap();
        assert_eq!(data, "bad json");
    }
}
