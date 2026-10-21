//! Portable, versioned workspace archives. Selection is applied on BOTH export
//! and import so omitted data never slips into an archive or destination.
use super::{mcp_history, storage};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::Path,
};

const FORMAT: &str = "glint-api.workspace";
const VERSION: u32 = 1;
const MAX_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Requests,
    CollectionSettings,
    Environments,
    Proxies,
    Versions,
    History,
    McpHistory,
    Preferences,
}
impl Section {
    pub const ALL: [Self; 8] = [
        Self::Requests,
        Self::CollectionSettings,
        Self::Environments,
        Self::Proxies,
        Self::Versions,
        Self::History,
        Self::McpHistory,
        Self::Preferences,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Requests => "全部集合与请求",
            Self::CollectionSettings => "集合与目录设置",
            Self::Environments => "环境与变量",
            Self::Proxies => "代理配置",
            Self::Versions => "请求版本",
            Self::History => "请求历史与响应",
            Self::McpHistory => "MCP 操作记录",
            Self::Preferences => "工作区设置",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::Requests => "集合、目录结构与顺序，以及所有请求的当前草稿",
            Self::CollectionSettings => "集合和目录的默认请求参数、鉴权、前后置动作等",
            Self::Environments => "全局与集合环境变量，保留变量原值及敏感标记",
            Self::Proxies => "代理、Hosts 映射、监听设置与环境代理绑定",
            Self::Versions => "版本快照、标签与 MCP 来源；需同时选择集合与请求",
            Self::History => "发送记录、响应、请求预览、cURL、日志及已保存的 WS 消息",
            Self::McpHistory => "当前工作区的 MCP 调用参数、执行结果与时间",
            Self::Preferences => "当前环境选择与更新源配置",
        }
    }
    pub fn requires_requests(self) -> bool {
        matches!(self, Self::Versions | Self::History)
    }
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Contents(pub BTreeSet<Section>);
impl Contents {
    pub fn all() -> Self {
        Self(Section::ALL.into_iter().collect())
    }
    pub fn has(&self, section: Section) -> bool {
        self.0.contains(&section)
    }
    pub fn set(&mut self, section: Section, checked: bool) {
        if checked {
            self.0.insert(section);
        } else {
            self.0.remove(&section);
        }
    }
    pub fn effective(&self) -> Self {
        let mut result = self.clone();
        if !self.has(Section::Requests) {
            result.set(Section::Versions, false);
            result.set(Section::History, false);
        }
        result
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Archive {
    format: String,
    version: u32,
    pub name: String,
    pub exported_at: String,
    pub contents: Contents,
    pub data: storage::Workspace,
    #[serde(default)]
    pub mcp_calls: Vec<mcp_history::Record>,
}
impl Archive {
    pub fn new(
        name: String,
        data: &storage::Workspace,
        contents: Contents,
        mcp_calls: Vec<mcp_history::Record>,
    ) -> Result<Self, String> {
        data.validate()?;
        let contents = contents.effective();
        if contents.0.is_empty() {
            return Err("请至少选择一项迁移内容".into());
        }
        let data = project(data, &contents);
        data.validate()?;
        if let Some(source) = &data.update_source {
            source.validate()?;
        }
        let mcp_calls = if contents.has(Section::McpHistory) {
            mcp_calls
        } else {
            vec![]
        };
        Ok(Self {
            format: FORMAT.into(),
            version: VERSION,
            name,
            exported_at: chrono::Utc::now().to_rfc3339(),
            contents,
            data,
            mcp_calls,
        })
    }
    pub fn selected(&self, selected: &Contents) -> Result<Self, String> {
        if !selected.0.is_subset(&self.contents.0) {
            return Err("选择的内容不在此迁移文件中".into());
        }
        let mut archive = Self::new(
            self.name.clone(),
            &self.data,
            selected.clone(),
            self.mcp_calls.clone(),
        )?;
        archive.exported_at = self.exported_at.clone();
        Ok(archive)
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("无法打开迁移文件：{e}"))?;
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("无法读取迁移文件：{e}"))?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("迁移文件超过 256 MiB，请减少历史或响应内容后重新导出".into());
        }
        Self::decode(&bytes)
    }
    fn decode(bytes: &[u8]) -> Result<Self, String> {
        // Check the envelope before interpreting data from a future schema.
        #[derive(Deserialize)]
        struct Header {
            format: String,
            version: u32,
        }
        let header: Header =
            serde_json::from_slice(bytes).map_err(|e| format!("迁移文件格式无效：{e}"))?;
        if header.format != FORMAT {
            return Err("请选择 Glint API 工作区迁移 JSON 文件".into());
        }
        if header.version != VERSION {
            return Err(format!(
                "不支持迁移格式版本 {}，请使用兼容版本的 Glint API",
                header.version
            ));
        }
        let archive: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("迁移文件内容无效：{e}"))?;
        archive.data.validate()?;
        // Discard even well-formed data hidden in sections marked as absent.
        archive.selected(&archive.contents)
    }
    pub fn write(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("导出内容超过 256 MiB，请取消部分历史或响应内容".into());
        }
        let parent = path.parent().ok_or("导出路径无效")?;
        let temporary = parent.join(format!(".glint-export-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&temporary)
                .map_err(|e| format!("无法创建迁移文件：{e}"))?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| format!("无法写入迁移文件：{e}"))?;
            drop(file);
            std::fs::rename(&temporary, path).map_err(|e| format!("无法保存迁移文件：{e}"))
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
    pub fn summary(&self) -> String {
        format!(
            "{} 个集合 · {} 个请求 · {} 个环境 · {} 个版本 · {} 条请求历史 · {} 条 MCP 记录",
            self.data.collections.len(),
            self.data.requests.len(),
            self.data.environments.len().saturating_sub(1),
            self.data
                .requests
                .iter()
                .map(|r| r.versions.len())
                .sum::<usize>(),
            self.data
                .requests
                .iter()
                .map(|r| r.history.len())
                .sum::<usize>(),
            self.mcp_calls.len()
        )
    }
}

fn project(source: &storage::Workspace, contents: &Contents) -> storage::Workspace {
    let mut data = source.clone();
    if !contents.has(Section::Preferences) {
        data.update_source = None;
        data.selected_environment = 0;
    }
    if !contents.has(Section::Requests) {
        data.requests.clear();
        data.traces.clear();
        data.responses.clear();
        data.previews.clear();
        data.curls.clear();
        if !contents.has(Section::CollectionSettings) {
            data.collections
                .retain(|c| contents.has(Section::Environments) && !c.environments.is_empty());
            for collection in &mut data.collections {
                collection.names.clear();
                collection.folder_order.clear();
            }
        }
    }
    for collection in &mut data.collections {
        if !contents.has(Section::CollectionSettings) {
            collection.defaults.clear();
        }
        if !contents.has(Section::Environments) {
            collection.environments.clear();
        }
    }
    if !contents.has(Section::Environments) {
        // Keep environment names for optional proxy bindings and saved selection.
        for variables in &mut data.variables {
            variables.variables.clear();
        }
    }
    if !contents.has(Section::Proxies) {
        data.proxies.clear();
        for variables in &mut data.variables {
            variables.proxy_id = None;
            variables.proxy = Default::default();
        }
    }
    if !contents.has(Section::Environments)
        && !contents.has(Section::Proxies)
        && !contents.has(Section::Preferences)
    {
        data.environments = storage::Workspace::default().environments;
        data.variables = storage::Workspace::default().variables;
        data.selected_environment = 0;
    }
    for request in &mut data.requests {
        if !contents.has(Section::Versions) {
            request.versions.clear();
        }
        if !contents.has(Section::History) {
            request.history.clear();
            for record in &mut request.versions {
                record.exchange = None;
            }
        }
    }
    if !contents.has(Section::History) {
        let n = data.requests.len();
        data.traces = vec![vec![]; n];
        data.responses = vec![None; n];
        data.previews = vec![None; n];
        data.curls = vec![None; n];
    }
    data
}

impl storage::Store {
    pub fn export_mcp_calls(&self) -> Result<Vec<mcp_history::Record>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM mcp_calls WHERE workspace_id=?1 ORDER BY id")
            .map_err(|e| e.to_string())?;
        let ids = statement
            .query_map([&self.workspace_id], |row| row.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        ids.map(|id| self.mcp_call_record(id.map_err(|e| e.to_string())?))
            .collect()
    }
    pub fn import_archive(
        &mut self,
        name: &str,
        archive: &Archive,
        selected: &Contents,
    ) -> Result<String, String> {
        let name = self.workspace_name(name, "")?;
        let archive = archive.selected(selected)?;
        let id = uuid::Uuid::new_v4().to_string();
        let json = serde_json::to_string(&archive.data).map_err(|e| e.to_string())?;
        let tx = self.connection.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO workspaces(id,name,data) VALUES(?1,?2,?3)",
            rusqlite::params![id, name, json],
        )
        .map_err(|e| e.to_string())?;
        for record in archive.mcp_calls {
            let s = record.summary;
            tx.execute("INSERT INTO mcp_calls(started_at_ms,client,rpc_id,tool,duration_ms,success,arguments,result,workspace_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)", rusqlite::params![s.started_at_ms,s.client,s.rpc_id,s.tool,s.duration_ms.min(i64::MAX as u64) as i64,s.success,record.arguments.to_string(),record.result.to_string(),id]).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        environment::{EnvironmentVariables, ProxyProfile, ProxySettings},
        model::{DirectoryDefaults, Draft, Record, RequestGroups, VersionSource},
    };
    use serde_json::json;

    fn fixture() -> storage::Workspace {
        let mut requests = crate::model::test_requests()[..1].to_vec();
        let mut collection = RequestGroups::create_collection("Test API", &[]).unwrap();
        collection.names = vec!["资源".into()];
        let mut variables = EnvironmentVariables::default();
        variables
            .replace(&[("key".into(), "secret-env-fixture".into(), true)])
            .unwrap();
        collection
            .environments
            .insert("beta".into(), variables.clone());
        collection.defaults.insert(
            "资源".into(),
            DirectoryDefaults {
                sections: [crate::model::DefaultSection::Auth].into_iter().collect(),
                draft: Draft {
                    token: "collection-token-fixture".into(),
                    auth: true,
                    ..Default::default()
                },
            },
        );
        let proxy = ProxyProfile {
            id: crate::model::new_id(),
            name: "测试代理".into(),
            settings: ProxySettings::default(),
        };
        variables.proxy_id = Some(proxy.id.clone());
        let exchange = crate::model::Exchange {
            websocket_connected: false,
            websocket_events: vec![],
            curl: Some("curl-fixture".into()),
            response: None,
            error: None,
            preview: Some("preview-fixture".into()),
            traces: vec![],
        };
        let version = Record {
            id: crate::model::new_id(),
            tag: "baseline".into(),
            source: VersionSource::Mcp,
            environment_name: Some("beta".into()),
            draft: requests[0].draft.clone(),
            label: "v1".into(),
            time: "2026-10-21 10:00:00".into(),
            detail: "fixture".into(),
            elapsed_ms: None,
            exchange: Some(exchange),
        };
        requests[0].versions.push(version.clone());
        let mut history = version;
        history.id = crate::model::new_id();
        requests[0].history.push(history);
        storage::Workspace {
            requests,
            collections: vec![collection],
            environments: vec!["不使用环境".into(), "beta".into()],
            variables: vec![EnvironmentVariables::default(), variables],
            selected_environment: 1,
            proxies: vec![proxy],
            traces: vec![vec![]],
            responses: vec![Some(crate::http::HttpResponse {
                status: 200,
                text: "response-fixture".into(),
                headers: vec![],
                bytes: 16,
                json: None,
            })],
            previews: vec![Some("preview-fixture".into())],
            curls: vec![Some("curl-fixture".into())],
            ..Default::default()
        }
    }
    #[test]
    fn all_sections_round_trip_through_a_portable_file_and_database() {
        let directory = tempfile::tempdir().unwrap();
        let data = fixture();
        data.validate().unwrap();
        let calls = vec![crate::mcp_history::tests::record(
            1234,
            "glint_request_update",
            true,
        )];
        let archive = Archive::new("项目 A".into(), &data, Contents::all(), calls).unwrap();
        let path = directory.path().join("迁移.glint-workspace.json");
        archive.write(&path).unwrap();
        archive.write(&path).unwrap(); // Replace an existing export atomically.
        let decoded = Archive::read(&path).unwrap();
        assert_eq!(
            serde_json::to_value(&decoded.data).unwrap(),
            serde_json::to_value(&data).unwrap()
        );
        let database = directory.path().join("destination.sqlite3");
        let mut store = storage::Store::open(&database).unwrap();
        let imported = store
            .import_archive("新项目", &decoded, &Contents::all())
            .unwrap();
        assert_eq!(store.workspace_id, "default");
        assert!(store.load().unwrap().requests.is_empty());
        let (imported_store, loaded) = store.prepare_workspace(&imported).unwrap();
        assert_eq!(
            serde_json::to_value(&loaded).unwrap(),
            serde_json::to_value(&data).unwrap()
        );
        assert_eq!(
            imported_store.export_mcp_calls().unwrap()[0].summary.tool,
            "glint_request_update"
        );
        assert_eq!(loaded.requests[0].versions[0].source, VersionSource::Mcp);
    }
    #[test]
    fn every_selection_is_valid_and_omitted_data_does_not_leak() {
        let source = fixture();
        let full = Archive::new(
            "项目".into(),
            &source,
            Contents::all(),
            vec![crate::mcp_history::tests::record(1, "test", true)],
        )
        .unwrap();
        for mask in 1..256 {
            let contents = Contents(
                Section::ALL
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| mask & (1 << i) != 0)
                    .map(|(_, s)| s)
                    .collect(),
            );
            if contents.effective().0.is_empty() {
                continue;
            }
            let archive = full.selected(&contents).unwrap();
            archive.data.validate().unwrap();
            let json = serde_json::to_string(&archive).unwrap();
            assert_eq!(
                archive.data.requests.is_empty(),
                !contents.has(Section::Requests)
            );
            if !contents.has(Section::Environments) {
                assert!(!json.contains("secret-env-fixture"), "{mask}");
            }
            if !contents.has(Section::CollectionSettings) {
                assert!(!json.contains("collection-token-fixture"), "{mask}");
            }
            if !contents.has(Section::Proxies) {
                assert!(archive.data.proxies.is_empty());
                assert!(archive.data.variables.iter().all(|v| v.proxy_id.is_none()));
            }
            if !contents.has(Section::History) || !contents.has(Section::Requests) {
                assert!(
                    !json.contains("response-fixture")
                        && !json.contains("preview-fixture")
                        && !json.contains("curl-fixture"),
                    "{mask}"
                );
            }
            if !contents.has(Section::McpHistory) {
                assert!(archive.mcp_calls.is_empty());
            }
            for request in &archive.data.requests {
                if !contents.has(Section::Versions) {
                    assert!(request.versions.is_empty());
                }
            }
            let bytes = serde_json::to_vec(&archive).unwrap();
            Archive::decode(&bytes).unwrap();
        }
        assert!(full.selected(&Contents::default()).is_err());
    }
    #[test]
    fn invalid_archives_and_failed_imports_leave_existing_workspaces_untouched() {
        let archive = Archive::new(
            "项目".into(),
            &fixture(),
            Contents::all(),
            vec![crate::mcp_history::tests::record(1, "test", true)],
        )
        .unwrap();
        let mut value = serde_json::to_value(&archive).unwrap();
        value["version"] = json!(99);
        assert!(
            Archive::decode(&serde_json::to_vec(&value).unwrap())
                .err()
                .unwrap()
                .contains("版本")
        );
        value["version"] = json!(1);
        value["data"]["requests"][0]["collection"] = json!("missing");
        assert!(Archive::decode(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(Archive::decode(b"bad json").is_err());
        let mut store = storage::Store::memory().unwrap();
        store.connection.execute_batch("CREATE TRIGGER fail_mcp_import BEFORE INSERT ON mcp_calls BEGIN SELECT RAISE(ABORT, 'simulated failure'); END;").unwrap();
        assert!(
            store
                .import_archive("回滚测试", &archive, &Contents::all())
                .is_err()
        );
        assert_eq!(store.workspaces().unwrap().len(), 1);
        assert!(store.export_mcp_calls().unwrap().is_empty());
        store
            .connection
            .execute_batch("DROP TRIGGER fail_mcp_import")
            .unwrap();
        store
            .import_archive("项目", &archive, &Contents::all())
            .unwrap();
        assert!(
            store
                .import_archive("项目", &archive, &Contents::all())
                .is_err()
        );
        assert_eq!(store.workspaces().unwrap().len(), 2);
    }
}
