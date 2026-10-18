use super::{EnvironmentVariables, HttpResponse, Request, RequestGroups, TraceEntry};
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
pub struct Workspace {
    #[serde(default)]
    pub update_source: Option<super::updater::UpdateSource>,
    #[serde(default)]
    pub selected_environment: usize,
    pub requests: Vec<Request>,
    pub collections: Vec<RequestGroups>,
    pub environments: Vec<String>,
    pub variables: Vec<EnvironmentVariables>,
    #[serde(default)]
    pub proxies: Vec<super::environment::ProxyProfile>,
    pub traces: Vec<Vec<TraceEntry>>,
    pub responses: Vec<Option<HttpResponse>>,
    pub previews: Vec<Option<String>>,
    #[serde(default)]
    pub curls: Vec<Option<String>>,
}
impl Default for Workspace {
    fn default() -> Self {
        Self {
            update_source: None,
            requests: vec![],
            collections: vec![],
            selected_environment: 0,
            environments: vec!["不使用环境".into()],
            variables: vec![EnvironmentVariables::default()],
            proxies: vec![],
            traces: vec![],
            responses: vec![],
            previews: vec![],
            curls: vec![],
        }
    }
}
impl Workspace {
    pub(super) fn migrate_proxies(&mut self) -> bool {
        let mut migrated_proxies = false;
        for (index, variables) in self.variables.iter_mut().enumerate().skip(1) {
            if variables.proxy != super::environment::ProxySettings::default() {
                let id = if let Some(profile) =
                    self.proxies.iter().find(|p| p.settings == variables.proxy)
                {
                    profile.id.clone()
                } else {
                    let id = uuid::Uuid::new_v4().to_string();
                    self.proxies.push(super::environment::ProxyProfile {
                        id: id.clone(),
                        name: {
                            let base = format!("{} 代理", self.environments[index]);
                            if self.proxies.iter().any(|p| p.name == base) {
                                format!("{base} {}", &id[..8])
                            } else {
                                base
                            }
                        },
                        settings: variables.proxy.clone(),
                    });
                    id
                };
                variables.proxy_id = Some(id);
                variables.proxy = Default::default();
                migrated_proxies = true;
            }
        }
        migrated_proxies
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.environments.first().map(String::as_str) != Some("不使用环境")
            || self.environments.len() != self.variables.len()
            || self.selected_environment >= self.environments.len()
            || self.requests.len() != self.traces.len()
            || self.requests.len() != self.responses.len()
            || self.requests.len() != self.previews.len()
        {
            return Err("工作区数据结构不完整".into());
        }
        let mut request_ids = std::collections::HashSet::new();
        for request in &self.requests {
            if uuid::Uuid::parse_str(&request.id).is_err() || !request_ids.insert(&request.id) {
                return Err("请求 ID 无效或重复".into());
            }
            let mut ids = std::collections::HashSet::new();
            if request
                .versions
                .iter()
                .chain(&request.history)
                .any(|r| uuid::Uuid::parse_str(&r.id).is_err() || !ids.insert(&r.id))
            {
                return Err("版本或历史 ID 无效或重复".into());
            }
        }
        let mut names = std::collections::HashSet::new();
        if self
            .environments
            .iter()
            .any(|name| name.trim().is_empty() || !names.insert(name))
        {
            return Err("环境名称为空或重复".into());
        }
        let mut proxy_ids = std::collections::HashSet::new();
        let mut proxy_names = std::collections::HashSet::new();
        for proxy in &self.proxies {
            if uuid::Uuid::parse_str(&proxy.id).is_err()
                || !proxy_ids.insert(&proxy.id)
                || proxy.name.trim().is_empty()
                || !proxy_names.insert(proxy.name.trim())
            {
                return Err("代理 ID 或名称无效、重复".into());
            }
            proxy.settings.validate()?;
        }
        for environment in &self.variables {
            if environment
                .proxy_id
                .as_ref()
                .is_some_and(|id| !proxy_ids.contains(id))
            {
                return Err("环境绑定了不存在的代理".into());
            }
            environment.validate()?;
        }
        if self.variables[0].proxy.enabled || self.variables[0].proxy_id.is_some() {
            return Err("不使用环境时不能启用环境代理".into());
        }
        let mut collection_names = std::collections::HashSet::new();
        let mut collection_ids = std::collections::HashSet::new();
        for collection in &self.collections {
            if uuid::Uuid::parse_str(&collection.id).is_err()
                || !collection_ids.insert(&collection.id)
            {
                return Err("集合 ID 无效或重复".into());
            }
            for (name, variables) in &collection.environments {
                if !self.environments.contains(name)
                    || (variables.proxy != Default::default() || variables.proxy_id.is_some())
                {
                    return Err("集合环境引用了不存在的环境或包含代理设置".into());
                }
                variables.validate()?;
            }
            let mut paths = std::collections::HashSet::new();
            if collection.collection.trim().is_empty()
                || !collection_names.insert(&collection.collection)
            {
                return Err("集合名称为空或重复".into());
            }
            if collection
                .defaults
                .keys()
                .any(|path| !collection.contains(path))
            {
                return Err("目录默认配置引用了不存在的目录".into());
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
            {
                return Err("请求引用了不存在的集合或目录".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceInfo {
    pub id: String,
    pub name: String,
}

pub struct Store {
    pub(super) workspace_id: String,
    pub(super) connection: Connection,
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
    pub fn disabled_plugins(&self) -> Result<std::collections::HashSet<String>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT family FROM disabled_plugins")
            .map_err(|e| e.to_string())?;
        statement
            .query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())
    }
    pub fn set_plugin_enabled(&self, family: &str, enabled: bool) -> Result<(), String> {
        if enabled {
            self.connection
                .execute("DELETE FROM disabled_plugins WHERE family=?1", [family])
        } else {
            self.connection.execute(
                "INSERT OR IGNORE INTO disabled_plugins(family) VALUES(?1)",
                [family],
            )
        }
        .map(|_| ())
        .map_err(|e| format!("无法保存插件启用状态：{e}"))
    }
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
        if version > 3 {
            return Err("数据库版本较新，请使用更新版本的 Glint API".into());
        }
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS disabled_plugins (family TEXT PRIMARY KEY NOT NULL);",
            )
            .map_err(|e| e.to_string())?;
        connection.execute_batch("BEGIN; CREATE TABLE IF NOT EXISTS workspace (id INTEGER PRIMARY KEY CHECK(id = 1), data TEXT NOT NULL); COMMIT;").map_err(|e| e.to_string())?;
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS mcp_calls (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            started_at_ms INTEGER NOT NULL,
            client TEXT NOT NULL,
            rpc_id TEXT NOT NULL,
            tool TEXT NOT NULL,
            duration_ms INTEGER NOT NULL,
            success INTEGER NOT NULL,
            arguments TEXT NOT NULL,
            result TEXT NOT NULL
        ); CREATE INDEX IF NOT EXISTS mcp_calls_chronological ON mcp_calls(started_at_ms DESC,id DESC);",
            )
            .map_err(|e| e.to_string())?;
        connection.execute_batch("BEGIN; CREATE TABLE IF NOT EXISTS workspaces (
            id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, data TEXT);
            INSERT OR IGNORE INTO workspaces(id,name) VALUES ('default','个人工作区');
            CREATE TABLE IF NOT EXISTS workspace_selection (id INTEGER PRIMARY KEY CHECK(id=1), workspace_id TEXT NOT NULL);
            INSERT OR IGNORE INTO workspace_selection VALUES(1,'default');").map_err(|e| e.to_string())?;
        let has_workspace_column = connection
            .prepare("PRAGMA table_info(mcp_calls)")
            .map_err(|e| e.to_string())?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|name| name == "workspace_id");
        if !has_workspace_column {
            connection.execute_batch("ALTER TABLE mcp_calls ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'default';").map_err(|e|e.to_string())?;
        }
        connection
            .execute_batch("CREATE INDEX IF NOT EXISTS mcp_calls_workspace ON mcp_calls(workspace_id,started_at_ms DESC,id DESC); PRAGMA user_version=3; COMMIT;")
            .map_err(|e| e.to_string())?;
        let workspace_id = connection
            .query_row(
                "SELECT workspace_id FROM workspace_selection WHERE id=1",
                [],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(Self {
            workspace_id,
            connection,
            last_saved: None,
            #[cfg(test)]
            serializations: 0,
        })
    }
    pub fn workspaces(&self) -> Result<Vec<WorkspaceInfo>, String> {
        self.connection
            .prepare("SELECT id,name FROM workspaces ORDER BY rowid")
            .map_err(|e| e.to_string())?
            .query_map([], |row| {
                Ok(WorkspaceInfo {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }
    fn workspace_name(&self, name: &str, except: &str) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err("名称须为 1–64 个字符".into());
        }
        if self
            .workspaces()?
            .iter()
            .any(|w| w.name == name && w.id != except)
        {
            return Err("工作区名称已存在".into());
        }
        Ok(name.into())
    }
    pub fn create_workspace(&mut self, name: &str) -> Result<String, String> {
        let name = self.workspace_name(name, "")?;
        let id = uuid::Uuid::new_v4().to_string();
        let data = serde_json::to_string(&Workspace::default()).map_err(|e| e.to_string())?;
        self.connection
            .execute(
                "INSERT INTO workspaces(id,name,data) VALUES(?1,?2,?3)",
                rusqlite::params![id, name, data],
            )
            .map_err(|e| e.to_string())?;
        Ok(id)
    }
    pub fn rename_workspace(&mut self, id: &str, name: &str) -> Result<(), String> {
        let name = self.workspace_name(name, id)?;
        if self
            .connection
            .execute(
                "UPDATE workspaces SET name=?1 WHERE id=?2",
                rusqlite::params![name, id],
            )
            .map_err(|e| e.to_string())?
            != 1
        {
            return Err("工作区不存在".into());
        }
        Ok(())
    }
    pub fn delete_workspace(&mut self, id: &str) -> Result<(), String> {
        if id == "default" || id == self.workspace_id {
            return Err("请保留默认工作区，并先切换再删除当前工作区".into());
        }
        let tx = self.connection.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM mcp_calls WHERE workspace_id=?1", [id])
            .map_err(|e| e.to_string())?;
        if tx
            .execute("DELETE FROM workspaces WHERE id=?1", [id])
            .map_err(|e| e.to_string())?
            != 1
        {
            return Err("工作区不存在".into());
        }
        tx.commit().map_err(|e| e.to_string())
    }
    /// Prepare and validate the destination before committing the active selection.
    pub fn prepare_workspace(&self, id: &str) -> Result<(Self, Workspace), String> {
        if !self.workspaces()?.iter().any(|w| w.id == id) {
            return Err("工作区不存在".into());
        }
        let path = self.connection.path().ok_or("工作区数据库路径不可用")?;
        let mut store = Self::open(Path::new(path))?;
        store.workspace_id = id.into();
        let data = store.load()?;
        Ok((store, data))
    }
    pub fn select_workspace(&self) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE workspace_selection SET workspace_id=?1 WHERE id=1",
                [&self.workspace_id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn load(&mut self) -> Result<Workspace, String> {
        let data: Option<String> = if self.workspace_id == "default" {
            self.connection
                .query_row("SELECT data FROM workspace WHERE id=1", [], |r| r.get(0))
                .optional()
                .map_err(|e| e.to_string())?
        } else {
            self.connection
                .query_row(
                    "SELECT data FROM workspaces WHERE id=?1",
                    [&self.workspace_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?
        };
        let mut workspace: Workspace = match &data {
            Some(data) => serde_json::from_str(data).map_err(|e| format!("无法读取工作区：{e}"))?,
            None => Workspace::default(),
        };
        if let Some(data) = &data {
            let legacy: serde_json::Value =
                serde_json::from_str(data).map_err(|e| e.to_string())?;
            if let Some(requests) = legacy.get("requests").and_then(|value| value.as_array()) {
                // Carry the initial selection forward once; future request restores never select an environment.
                if legacy.get("selected_environment").is_none() {
                    workspace.selected_environment = requests
                        .first()
                        .and_then(|r| r["draft"]["environment"].as_u64())
                        .map(|index| index as usize)
                        .filter(|index| *index < workspace.environments.len())
                        .unwrap_or(0);
                }
                for (request, saved) in workspace.requests.iter_mut().zip(requests) {
                    if let Some(history) = saved.get("history").and_then(|value| value.as_array()) {
                        for (record, saved) in request.history.iter_mut().zip(history) {
                            if record.environment_name.is_none() {
                                record.environment_name = saved["draft"]["environment"]
                                    .as_u64()
                                    .and_then(|index| workspace.environments.get(index as usize))
                                    .cloned();
                            }
                        }
                    }
                }
            }
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
        workspace.curls.resize(workspace.requests.len(), None);
        workspace.validate()?;
        let needs_ids = data.as_ref().is_some_and(|data| {
            serde_json::from_str::<serde_json::Value>(data)
                .ok()
                .and_then(|v| v["requests"].as_array().cloned())
                .is_some_and(|requests| {
                    requests.iter().any(|r| {
                        r.get("id").is_none()
                            || ["versions", "history"].iter().any(|key| {
                                r[key].as_array().is_some_and(|records| {
                                    records.iter().any(|record| record.get("id").is_none())
                                })
                            })
                    })
                })
        });
        self.last_saved = data;
        if needs_ids {
            self.save(&workspace)?;
        }
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
        if self.workspace_id == "default" {
            transaction.execute("INSERT INTO workspace(id, data) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET data = excluded.data", [&data]).map_err(|e| e.to_string())?;
        } else if transaction
            .execute(
                "UPDATE workspaces SET data=?1 WHERE id=?2",
                rusqlite::params![data, self.workspace_id],
            )
            .map_err(|e| e.to_string())?
            != 1
        {
            return Err("工作区不存在".into());
        }
        transaction
            .execute_batch("PRAGMA user_version = 3")
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
                id: crate::model::new_id(),
                environments: Default::default(),
                defaults: Default::default(),
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
            workspace_id: "default".into(),
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
            3
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
                id: crate::model::new_id(),
                environments: Default::default(),
                defaults: Default::default(),
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
                id: crate::model::new_id(),
                environments: Default::default(),
                defaults: Default::default(),
                folder_order: Default::default(),
                collection: "My API".into(),
                names: vec!["未分组".into()],
            });
            data.environments.push("Test".into());
            let mut variables = EnvironmentVariables::default();
            variables.save(None, "token", "test-secret", true).unwrap();
            data.variables.push(variables);
            let draft = Draft {
                url: "https://example.test/".into(),
                ..Draft::default()
            };
            let record = Record {
                id: crate::model::new_id(),
                tag: String::new(),
                environment_name: None,
                draft: draft.clone(),
                label: "v1".into(),
                time: "now".into(),
                detail: String::new(),
                elapsed_ms: Some(42),
                exchange: None,
            };
            data.requests.push(Request {
                id: crate::model::new_id(),
                tree_order: 0,
                name: "Request".into(),
                collection: "My API".into(),
                group: "未分组".into(),
                draft,
                versions: vec![record.clone()],
                history: vec![Record {
                    id: crate::model::new_id(),
                    ..record
                }],
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
            let mut legacy = serde_json::to_value(&data).unwrap();
            legacy
                .as_object_mut()
                .unwrap()
                .remove("selected_environment");
            legacy["requests"][0]["draft"]["environment"] = 1.into();
            legacy["requests"][0]["history"][0]["draft"]["environment"] = 1.into();
            store
                .connection
                .execute(
                    "UPDATE workspace SET data = ?1 WHERE id = 1",
                    [legacy.to_string()],
                )
                .unwrap();
        }
        {
            let mut store = Store::open(&path).unwrap();
            let mut data = store.load().unwrap();
            assert_eq!(data.selected_environment, 1);
            assert_eq!(
                data.requests[0].history[0].environment_name.as_deref(),
                Some("Test")
            );
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
            workspace_id: "default".into(),
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
            workspace_id: "default".into(),
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
    #[test]
    fn mcp_legacy_ids_migrate_once_and_reject_ambiguous_records() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let mut request = crate::model::test_requests().remove(0);
        let version = Record {
            id: crate::model::new_id(),
            tag: String::new(),
            environment_name: None,
            draft: request.draft.clone(),
            label: "v1".into(),
            time: "now".into(),
            detail: String::new(),
            elapsed_ms: None,
            exchange: None,
        };
        request.versions.push(version.clone());
        request.history.push(Record {
            id: crate::model::new_id(),
            ..version
        });
        let workspace = Workspace {
            collections: vec![RequestGroups::new(std::slice::from_ref(&request))],
            requests: vec![request],
            traces: vec![vec![]],
            responses: vec![None],
            previews: vec![None],
            curls: vec![None],
            ..Workspace::default()
        };
        let mut legacy = serde_json::to_value(workspace).unwrap();
        let request = &mut legacy["requests"][0];
        request.as_object_mut().unwrap().remove("id");
        for key in ["versions", "history"] {
            for record in request[key].as_array_mut().unwrap() {
                record.as_object_mut().unwrap().remove("id");
            }
        }
        let mut store = Store::open(&path).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO workspace(id,data) VALUES(1,?1)",
                [legacy.to_string()],
            )
            .unwrap();
        let migrated = store.load().unwrap();
        let ids = (
            &migrated.requests[0].id,
            &migrated.requests[0].versions[0].id,
            &migrated.requests[0].history[0].id,
        );
        let reloaded = Store::open(&path).unwrap().load().unwrap();
        assert_eq!(
            ids,
            (
                &reloaded.requests[0].id,
                &reloaded.requests[0].versions[0].id,
                &reloaded.requests[0].history[0].id
            )
        );
        let mut invalid = reloaded;
        invalid.requests[0].history[0].id = invalid.requests[0].versions[0].id.clone();
        assert!(store.save(&invalid).is_err());
        assert_ne!(
            store.load().unwrap().requests[0].history[0].id,
            invalid.requests[0].history[0].id
        );
    }
}

#[cfg(test)]
mod workspace_tests {
    use super::*;
    #[test]
    fn workspaces_isolate_data_and_audit_and_restore_selection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspaces.sqlite3");
        let mut first = Store::open(&path).unwrap();
        let mut data = first.load().unwrap();
        data.environments.push("Default private".into());
        data.variables.push(EnvironmentVariables::default());
        first.save(&data).unwrap();
        let record = super::super::mcp_history::tests::record(100, "glint_workspace_get", true);
        let default_call = first.append_mcp_call(&record).unwrap();
        let second_id = first.create_workspace("  项目 B  ").unwrap();
        assert!(first.create_workspace("项目 B").is_err());
        assert!(first.create_workspace("  ").is_err());
        assert!(first.create_workspace(&"长".repeat(65)).is_err());
        let (mut second, mut second_data) = first.prepare_workspace(&second_id).unwrap();
        assert_eq!(second_data.environments, ["不使用环境"]);
        assert_eq!(second.mcp_call_page("", None, None, 10).unwrap().total, 0);
        assert!(second.mcp_call_record(default_call).is_err());
        second_data.environments.push("B private".into());
        second_data.variables.push(EnvironmentVariables::default());
        second.save(&second_data).unwrap();
        second.select_workspace().unwrap();
        second.append_mcp_call(&record).unwrap();
        second.rename_workspace(&second_id, "项目 B 改名").unwrap();
        assert!(second.rename_workspace(&second_id, "个人工作区").is_err());
        assert!(second.delete_workspace(&second_id).is_err());
        assert!(second.delete_workspace("default").is_err());
        let mut reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.workspace_id, second_id);
        assert_eq!(
            reopened.load().unwrap().environments,
            ["不使用环境", "B private"]
        );
        assert_eq!(reopened.workspaces().unwrap()[1].name, "项目 B 改名");
        assert_eq!(reopened.mcp_call_page("", None, None, 10).unwrap().total, 1);
        let (mut original, original_data) = reopened.prepare_workspace("default").unwrap();
        original.select_workspace().unwrap();
        assert_eq!(
            original_data.environments,
            ["不使用环境", "Default private"]
        );
        original.delete_workspace(&second_id).unwrap();
        assert!(original.prepare_workspace(&second_id).is_err());
        assert_eq!(original.workspaces().unwrap().len(), 1);
        assert_eq!(original.mcp_call_page("", None, None, 10).unwrap().total, 1);
        assert_eq!(
            original
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM mcp_calls WHERE workspace_id=?1",
                    [second_id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
    #[test]
    fn legacy_database_and_audit_migrate_without_copying_or_overwriting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy.sqlite3");
        let connection = Connection::open(&path).unwrap();
        let mut data = Workspace::default();
        data.environments.push("已有环境".into());
        data.variables.push(EnvironmentVariables::default());
        let json = serde_json::to_string(&data).unwrap();
        connection.execute_batch("CREATE TABLE workspace(id INTEGER PRIMARY KEY CHECK(id=1),data TEXT NOT NULL); PRAGMA user_version=2;
            CREATE TABLE mcp_calls(id INTEGER PRIMARY KEY AUTOINCREMENT,started_at_ms INTEGER NOT NULL,client TEXT NOT NULL,rpc_id TEXT NOT NULL,tool TEXT NOT NULL,duration_ms INTEGER NOT NULL,success INTEGER NOT NULL,arguments TEXT NOT NULL,result TEXT NOT NULL);
            INSERT INTO mcp_calls VALUES(1,100,'test','1','glint_workspace_get',0,1,'{}','{}');").unwrap();
        connection
            .execute("INSERT INTO workspace VALUES(1,?1)", [&json])
            .unwrap();
        drop(connection);
        let mut store = Store::open(&path).unwrap();
        assert_eq!(
            store.load().unwrap().environments,
            ["不使用环境", "已有环境"]
        );
        assert_eq!(store.workspaces().unwrap()[0].name, "个人工作区");
        assert_eq!(store.mcp_call_page("", None, None, 10).unwrap().total, 1);
        assert_eq!(
            store
                .connection
                .query_row("SELECT data FROM workspace", [], |r| r.get::<_, String>(0))
                .unwrap(),
            json
        );
        let id = store.create_workspace("新项目").unwrap();
        store
            .connection
            .execute("UPDATE workspaces SET data='broken' WHERE id=?1", [&id])
            .unwrap();
        assert!(store.prepare_workspace(&id).is_err());
        assert_eq!(Store::open(&path).unwrap().workspace_id, "default");
    }
}

#[cfg(test)]
mod proxy_tests {
    use super::*;
    #[test]
    fn legacy_proxy_migration_deduplicates_and_rejects_dangling_bindings() {
        let mut data = Workspace::default();
        for name in ["Dev", "Test"] {
            data.environments.push(name.into());
            let mut variables = EnvironmentVariables::default();
            variables.proxy = super::super::environment::ProxySettings {
                enabled: true,
                url: "http://127.0.0.1:7890".into(),
                ..Default::default()
            };
            data.variables.push(variables);
        }
        assert!(data.migrate_proxies());
        assert!(!data.migrate_proxies());
        assert_eq!(data.proxies.len(), 1);
        assert_eq!(data.variables[1].proxy_id, data.variables[2].proxy_id);
        data.validate().unwrap();
        let encoded = serde_json::to_string(&data).unwrap();
        let mut decoded: Workspace = serde_json::from_str(&encoded).unwrap();
        decoded.validate().unwrap();
        decoded.proxies.clear();
        assert!(decoded.validate().is_err());
    }
}
