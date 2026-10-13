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
        for environment in &self.variables {
            environment.proxy.validate()?;
        }
        if self.variables[0].proxy.enabled {
            return Err("不使用环境时不能启用环境代理".into());
        }
        let mut collection_names = std::collections::HashSet::new();
        for collection in &self.collections {
            let mut paths = std::collections::HashSet::new();
            if collection.collection.trim().is_empty()
                || !collection_names.insert(&collection.collection)
            {
                return Err("集合名称为空或重复".into());
            }
            for path in &collection.names {
                if path.is_empty()
                    || path.split('/').any(|name| name.trim().is_empty())
                    || !paths.insert(path)
                    || !collection.contains(RequestGroups::parent(path))
                {
                    return Err("目录树数据不完整".into());
                }
            }
        }
        for request in &self.requests {
            if !self
                .collections
                .iter()
                .any(|c| c.collection == request.collection && c.contains(&request.group))
                || std::iter::once(&request.draft)
                    .chain(request.versions.iter().map(|r| &r.draft))
                    .chain(request.history.iter().map(|r| &r.draft))
                    .any(|d| d.environment >= self.environments.len())
            {
                return Err("请求引用了不存在的集合、目录或环境".into());
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

#[cfg(any(target_os = "windows", test))]
fn windows_database_path(
    local_app_data: Option<std::ffi::OsString>,
    user_profile: Option<std::ffi::OsString>,
) -> Result<PathBuf, String> {
    let base = local_app_data
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            user_profile
                .filter(|path| !path.is_empty())
                .map(|path| PathBuf::from(path).join("AppData").join("Local"))
        })
        .ok_or("无法确定 Windows 用户数据目录：LOCALAPPDATA 和 USERPROFILE 均未设置")?;
    Ok(base.join("Glint API").join("workspace.sqlite3"))
}

impl Store {
    pub fn default_path() -> Result<PathBuf, String> {
        if let Some(path) = std::env::var_os("GLINT_API_DB_PATH") {
            return Ok(path.into());
        }
        #[cfg(not(target_os = "windows"))]
        let home = std::env::var_os("HOME").ok_or("无法确定用户数据目录")?;
        #[cfg(target_os = "windows")]
        let path = windows_database_path(
            std::env::var_os("LOCALAPPDATA"),
            std::env::var_os("USERPROFILE"),
        )?;
        #[cfg(target_os = "macos")]
        let path =
            PathBuf::from(home).join("Library/Application Support/Glint API/workspace.sqlite3");
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
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
        if version > 2 {
            return Err("数据库版本较新，请使用更新版本的 Glint API".into());
        }
        connection.execute_batch("BEGIN; CREATE TABLE IF NOT EXISTS workspace (id INTEGER PRIMARY KEY CHECK(id = 1), data TEXT NOT NULL); COMMIT;").map_err(|e| e.to_string())?;
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
        let mut workspace: Workspace = match &data {
            Some(data) => serde_json::from_str(data).map_err(|e| format!("无法读取工作区：{e}"))?,
            None => Workspace::default(),
        };
        if let Some(data) = &data {
            let legacy: serde_json::Value =
                serde_json::from_str(data).map_err(|e| e.to_string())?;
            if let Some(collections) = legacy.get("collections").and_then(|value| value.as_array())
            {
                for (index, value) in collections.iter().enumerate() {
                    if value.get("names").is_none() {
                        continue;
                    }
                    let collection = &mut workspace.collections[index];
                    let defaults: Vec<_> = collection
                        .names
                        .iter()
                        .filter(|name| matches!(name.as_str(), "未分组" | "自定义"))
                        .cloned()
                        .collect();
                    for request in workspace
                        .requests
                        .iter_mut()
                        .filter(|r| r.collection == collection.collection)
                    {
                        if defaults.contains(&request.group) {
                            request.group.clear();
                        }
                    }
                    collection.names.retain(|name| !defaults.contains(name));
                    // Old group names containing slashes become nested paths; retain their requests.
                    for folder in collection.names.clone() {
                        let mut parent = RequestGroups::parent(&folder);
                        while !parent.is_empty() {
                            if !collection.names.iter().any(|folder| folder == parent) {
                                collection.names.push(parent.to_owned());
                            }
                            parent = RequestGroups::parent(parent);
                        }
                    }
                }
            }
        }
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
        transaction
            .execute_batch("PRAGMA user_version = 2")
            .map_err(|e| e.to_string())?;
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
    fn legacy_groups_migrate_to_directories_and_root_requests() {
        let mut requests = crate::model::test_requests();
        requests[0].group = "自定义".into();
        requests[1].group = "未分组".into();
        requests[2].group = "用户/详情".into();
        requests[3].group = "普通目录".into();
        let data = Workspace {
            collections: vec![RequestGroups {
                folder_order: Default::default(),
                collection: "Test API".into(),
                names: vec![
                    "自定义".into(),
                    "未分组".into(),
                    "用户/详情".into(),
                    "普通目录".into(),
                ],
            }],
            traces: vec![vec![]; requests.len()],
            responses: vec![None; requests.len()],
            previews: vec![None; requests.len()],
            requests,
            ..Default::default()
        };
        let mut legacy = serde_json::to_value(&data).unwrap();
        for request in legacy["requests"].as_array_mut().unwrap() {
            let folder = request.as_object_mut().unwrap().remove("folder").unwrap();
            request["group"] = folder;
        }
        let folders = legacy["collections"][0]
            .as_object_mut()
            .unwrap()
            .remove("folders")
            .unwrap();
        legacy["collections"][0]["names"] = folders;
        let mut store = Store {
            connection: Connection::open_in_memory().unwrap(),
            last_saved: None,
            serializations: 0,
        };
        store.connection.execute_batch("CREATE TABLE workspace (id INTEGER PRIMARY KEY, data TEXT); PRAGMA user_version = 1;").unwrap();
        store
            .connection
            .execute("INSERT INTO workspace VALUES (1, ?1)", [legacy.to_string()])
            .unwrap();
        let migrated = store.load().unwrap();
        assert!(migrated.requests[0].group.is_empty() && migrated.requests[1].group.is_empty());
        assert_eq!(migrated.requests[2].group, "用户/详情");
        assert!(migrated.collections[0].contains("用户"));
        assert!(!migrated.collections[0].contains("自定义"));
        assert_eq!(migrated.requests[2].draft, data.requests[2].draft);
        store.save(&migrated).unwrap();
        assert_eq!(
            store
                .connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            2
        );
        let saved: String = store
            .connection
            .query_row("SELECT data FROM workspace", [], |row| row.get(0))
            .unwrap();
        let saved: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert!(saved["collections"][0].get("folders").is_some());
        assert!(saved["requests"][0].get("group").is_none());
        assert_eq!(store.load().unwrap().requests[2].group, "用户/详情");
    }

    #[test]
    fn invalid_folder_references_and_missing_parents_are_rejected() {
        let mut data = Workspace {
            collections: vec![RequestGroups {
                folder_order: Default::default(),
                collection: "Test API".into(),
                names: vec![],
            }],
            requests: crate::model::test_requests()[..1].to_vec(),
            traces: vec![vec![]],
            responses: vec![None],
            previews: vec![None],
            ..Default::default()
        };
        assert!(data.validate().is_err());
        data.requests[0].group.clear();
        assert!(data.validate().is_ok());
        data.collections[0].names.push("a/b".into());
        assert!(data.validate().is_err());
        data.collections[0].names.push("a".into());
        assert!(data.validate().is_ok());
    }

    #[test]
    fn windows_path_uses_local_app_data_without_home() {
        let base = PathBuf::from("C:\\Users\\测试 用户\\AppData\\Local");
        let expected = base.join("Glint API").join("workspace.sqlite3");
        assert_eq!(
            windows_database_path(Some(base.clone().into_os_string()), None).unwrap(),
            expected
        );
        assert_eq!(
            windows_database_path(Some(base.into_os_string()), Some("D:\\Other".into())).unwrap(),
            expected
        );
    }
    #[test]
    fn windows_path_falls_back_to_profile_or_reports_missing_directory() {
        let profile = PathBuf::from("C:\\Users\\测试 用户");
        let expected = profile
            .join("AppData")
            .join("Local")
            .join("Glint API")
            .join("workspace.sqlite3");
        for local_app_data in [None, Some("".into())] {
            assert_eq!(
                windows_database_path(local_app_data, Some(profile.clone().into_os_string()))
                    .unwrap(),
                expected
            );
        }
        assert!(windows_database_path(None, None).is_err());
        assert!(windows_database_path(Some("".into()), Some("".into())).is_err());
    }
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
                folder_order: Default::default(),
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
                exchange: None,
            };
            data.requests.push(Request {
                tree_order: 0,
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
