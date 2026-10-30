//! API workspace data and domain operations. This layer has no GPUI entities or window context.
use crate::{
    environment,
    environment::EnvironmentVariables,
    http::HttpResponse,
    model::{Draft, Request, RequestGroups},
    storage, updater,
};
use serde_json::{Value, json};

pub(crate) struct Invocation {
    pub metadata: CallMetadata,
    pub name: String,
    pub arguments: Value,
    pub deadline: std::time::Instant,
    pub rejection: Option<String>,
}

pub(crate) struct ApiWorkspace {
    pub storage: storage::Store,
    pub storage_error: Option<String>,
    pub storage_dirty: bool,
    pub requests: Vec<Request>,
    pub collections: Vec<RequestGroups>,
    pub environment_names: Vec<String>,
    pub environment_inputs: Vec<EnvironmentVariables>,
    pub proxies: Vec<environment::ProxyProfile>,
    pub responses: Vec<Option<HttpResponse>>,
    pub traces: Vec<Vec<TraceEntry>>,
    pub sent_requests: Vec<Option<String>>,
    pub sent_curls: Vec<Option<String>>,
    pub update_source: Option<updater::UpdateSource>,
}

pub(crate) struct ToolCommand {
    pub operation: String,
    pub arguments: Value,
    pub read_only: bool,
}

impl ToolCommand {
    pub fn parse(name: &str, arguments: Value) -> Result<Self, String> {
        let spec = crate::mcp_tools::catalog()
            .into_iter()
            .find(|spec| spec["name"] == name)
            .ok_or("未知 MCP 工具")?;
        crate::mcp_tools::validate(&spec["inputSchema"], &arguments)?;
        Ok(Self {
            operation: name.strip_prefix("glint_").ok_or("未知工具前缀")?.into(),
            arguments,
            read_only: spec["annotations"]["readOnlyHint"] == true,
        })
    }
}

impl ApiWorkspace {
    pub fn delete_request(&mut self, index: usize) -> Result<Request, String> {
        if index >= self.requests.len() {
            return Err("请求不存在".into());
        }
        self.responses.remove(index);
        self.traces.remove(index);
        self.sent_requests.remove(index);
        self.sent_curls.remove(index);
        self.storage_dirty = true;
        Ok(self.requests.remove(index))
    }

    pub fn append_version(
        &mut self,
        index: usize,
        draft: Draft,
        source: crate::model::VersionSource,
    ) {
        let request = &mut self.requests[index];
        request.draft = draft.clone();
        request.versions.insert(
            0,
            crate::model::Record {
                id: crate::model::new_id(),
                tag: String::new(),
                source,
                environment_name: None,
                draft,
                label: format!("v{}", request.versions.len() + 1),
                time: record_time(),
                detail: if source == crate::model::VersionSource::Mcp {
                    "MCP 保存请求时生成的快照"
                } else {
                    "保存请求时生成的快照"
                }
                .into(),
                elapsed_ms: None,
                exchange: None,
            },
        );
        self.storage_dirty = true;
    }

    pub fn create_environment(&mut self, name: &str) -> Result<usize, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("名称不能为空".into());
        }
        if self.environment_names.iter().any(|n| n == name) {
            return Err("环境名称已存在".into());
        }
        self.environment_names.push(name.into());
        self.environment_inputs
            .push(EnvironmentVariables::default());
        self.storage_dirty = true;
        Ok(self.environment_names.len() - 1)
    }

    pub fn rename_environment(&mut self, index: usize, name: &str) -> Result<(), String> {
        if index == 0 {
            return Err("不能修改内置的不使用环境项".into());
        }
        let name = name.trim();
        if name.is_empty() {
            return Err("名称不能为空".into());
        }
        if self
            .environment_names
            .iter()
            .enumerate()
            .any(|(i, n)| i != index && n == name)
        {
            return Err("环境名称已存在".into());
        }
        let old = self
            .environment_names
            .get(index)
            .ok_or("环境不存在")?
            .clone();
        for collection in &mut self.collections {
            if let Some(variables) = collection.environments.remove(&old) {
                collection.environments.insert(name.into(), variables);
            }
        }
        self.environment_names[index] = name.into();
        self.storage_dirty = true;
        Ok(())
    }

    pub fn delete_environment(&mut self, index: usize) -> Result<(), String> {
        if index == 0 {
            return Err("不能修改内置的不使用环境项".into());
        }
        if index >= self.environment_names.len() {
            return Err("环境不存在".into());
        }
        let name = self.environment_names.remove(index);
        self.environment_inputs.remove(index);
        for collection in &mut self.collections {
            collection.environments.remove(&name);
        }
        self.storage_dirty = true;
        Ok(())
    }
    pub fn record_call(
        &mut self,
        call: &Invocation,
        result: &Result<Value, String>,
        secrets: &[String],
    ) -> Result<(), String> {
        let mut arguments = call.arguments.clone();
        if call.name == "glint_environment_variable_set" {
            if let Some(fields) = arguments.as_object_mut() {
                fields.insert("value".into(), json!("[REDACTED]"));
            }
        }
        let mut output = match result {
            Ok(value) => json!({"result":value}),
            Err(error) => json!({"error":error}),
        };
        redact(&mut arguments);
        redact(&mut output);
        scrub_values(&mut arguments, secrets);
        scrub_values(&mut output, secrets);
        let record = crate::mcp_history::Record {
            summary: crate::mcp_history::Summary {
                id: 0,
                started_at_ms: call.metadata.started_at_ms,
                client: call.metadata.client.clone(),
                rpc_id: call.metadata.rpc_id.clone(),
                tool: crate::mcp_history::clip(&call.name, 256),
                duration_ms: call
                    .metadata
                    .started
                    .elapsed()
                    .as_millis()
                    .min(u64::MAX as u128) as u64,
                success: result.is_ok(),
            },
            arguments: crate::mcp_history::bounded(arguments),
            result: crate::mcp_history::bounded(output),
        };
        self.storage.append_mcp_call(&record).map(|_| ())
    }
    pub fn create_request(
        &mut self,
        collection: usize,
        group: &str,
        title: &str,
        draft: Draft,
        source: crate::model::VersionSource,
    ) -> Result<usize, String> {
        let title = title.trim();
        if title.is_empty() {
            return Err("名称不能为空".into());
        }
        let groups = self.collections.get(collection).ok_or("集合不存在")?;
        if !groups.contains(group) {
            return Err("目录不存在".into());
        }
        let index = self.requests.len();
        self.requests.push(Request {
            id: crate::model::new_id(),
            tree_order: 0,
            name: title.into(),
            collection: groups.collection.clone(),
            group: group.into(),
            draft: draft.clone(),
            versions: vec![crate::model::Record {
                id: crate::model::new_id(),
                tag: String::new(),
                source,
                environment_name: None,
                draft,
                label: "v1".into(),
                time: record_time(),
                detail: "初始版本".into(),
                elapsed_ms: None,
                exchange: None,
            }],
            history: vec![],
        });
        crate::model::move_node(
            &mut self.collections,
            &mut self.requests,
            &crate::model::TreeNode::Request(index),
            &crate::model::TreeNode::Directory {
                collection,
                path: group.into(),
            },
            crate::model::Placement::Inside,
        )?;
        self.responses.push(None);
        self.traces.push(vec![]);
        self.sent_requests.push(None);
        self.sent_curls.push(None);
        self.storage_dirty = true;
        Ok(index)
    }
    pub fn snapshot(&self, selected_environment: usize) -> storage::Workspace {
        storage::Workspace {
            update_source: self.update_source.clone(),
            selected_environment,
            requests: self.requests.clone(),
            collections: self.collections.clone(),
            environments: self.environment_names.clone(),
            variables: self.environment_inputs.clone(),
            proxies: self.proxies.clone(),
            traces: self.traces.clone(),
            responses: self.responses.clone(),
            previews: self.sent_requests.clone(),
            curls: self.sent_curls.clone(),
        }
    }

    pub fn save(&mut self, selected_environment: usize) {
        let result = self.storage.save(&self.snapshot(selected_environment));
        self.storage_error = result.err();
        if self.storage_error.is_none() {
            self.storage_dirty = false;
        }
    }

    pub fn create_collection(&mut self, name: &str) -> Result<usize, String> {
        let collection = RequestGroups::create_collection(name, &self.collections)?;
        self.collections.push(collection);
        self.storage_dirty = true;
        Ok(self.collections.len() - 1)
    }

    pub fn rename_collection(&mut self, index: usize, name: &str) -> Result<(), String> {
        RequestGroups::rename_collection(index, name, &mut self.collections, &mut self.requests)?;
        self.storage_dirty = true;
        Ok(())
    }

    pub fn create_directory(
        &mut self,
        collection: usize,
        parent: &str,
        name: &str,
    ) -> Result<String, String> {
        let path = self
            .collections
            .get_mut(collection)
            .ok_or("集合不存在")?
            .add_child(parent, name)?;
        self.storage_dirty = true;
        Ok(path)
    }

    pub fn rename_directory(
        &mut self,
        collection: usize,
        path: &str,
        name: &str,
    ) -> Result<(), String> {
        self.collections
            .get_mut(collection)
            .ok_or("集合不存在")?
            .rename(path, name, &mut self.requests)?;
        self.storage_dirty = true;
        Ok(())
    }

    pub fn delete_directory(&mut self, collection: usize, path: &str) -> Result<(), String> {
        let groups = self.collections.get_mut(collection).ok_or("集合不存在")?;
        if path.is_empty() || !groups.contains(path) {
            return Err("目录不存在；删除集合请使用 collection_delete".into());
        }
        groups.remove(path, &mut self.requests);
        self.storage_dirty = true;
        Ok(())
    }

    pub fn patch_request(
        &mut self,
        index: usize,
        title: Option<String>,
        draft: Option<Draft>,
    ) -> Result<(), String> {
        let title = title
            .map(|title| {
                let title = title.trim();
                if title.is_empty() {
                    Err("名称不能为空".to_owned())
                } else {
                    Ok(title.to_owned())
                }
            })
            .transpose()?;
        let request = self.requests.get_mut(index).ok_or("请求不存在")?;
        if let Some(title) = title {
            request.name = title;
        }
        if let Some(draft) = draft {
            request.draft = draft;
        }
        self.storage_dirty = true;
        Ok(())
    }

    pub fn request_summary(&self, index: usize) -> Result<Value, String> {
        let request = self.requests.get(index).ok_or("请求不存在")?;
        let record = |record: &crate::model::Record| json!({"id":record.id,"tag":record.tag,"source":record.source,"label":record.label,"time":record.time,"detail":record.detail,"environment_name":record.environment_name,"elapsed_ms":record.elapsed_ms,"hash":crate::request_fingerprint::Fingerprint::new(&record.draft).hash});
        Ok(
            json!({"id":request.id,"name":request.name,"collection":request.collection,"path":request.group,"draft":request.draft,
            "versions":request.versions.iter().map(record).collect::<Vec<_>>(),"history":request.history.iter().map(record).collect::<Vec<_>>()}),
        )
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct TraceEntry {
    pub elapsed_ms: u128,
    pub message: String,
}

pub(crate) fn record_time() -> String {
    chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn sensitive(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "token"
            | "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "x-kso-authorization"
    )
}
pub(crate) fn redact(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            let secret = fields.get("secret").and_then(Value::as_bool) == Some(true)
                || ["key", "name"].iter().any(|key| {
                    fields
                        .get(*key)
                        .and_then(Value::as_str)
                        .is_some_and(sensitive)
                });
            for (key, value) in fields {
                if sensitive(key)
                    || secret && key == "value"
                    || matches!(key.as_str(), "curl" | "preview") && value.is_string()
                {
                    if !value.is_null() {
                        *value = json!("[REDACTED]");
                    }
                } else {
                    redact(value);
                }
            }
        }
        Value::Array(items) => {
            if items.len() == 2 && items[0].as_str().is_some_and(sensitive) {
                items[1] = json!("[REDACTED]");
            } else {
                for item in items {
                    redact(item);
                }
            }
        }
        _ => {}
    }
}
pub(crate) fn patch_draft(base: &Draft, patch: &Value) -> Result<Draft, String> {
    crate::mcp_tools::validate(&crate::mcp_tools::draft_schema(), patch)?;
    let mut value = serde_json::to_value(base).expect("serializable draft");
    for (key, item) in patch.as_object().ok_or("draft 必须是对象")? {
        value[key] = item.clone();
    }
    let mut draft: Draft =
        serde_json::from_value(value).map_err(|e| format!("请求配置无效：{e}"))?;
    if patch.get("oauth").is_none() && patch.get("auth").is_some() {
        draft.oauth = None;
    }
    if draft.oauth.is_some() {
        draft.auth = false;
        draft.token.clear();
    }
    Ok(draft)
}

impl ApiWorkspace {
    pub fn log_secrets(&self) -> Vec<String> {
        let mut values = self
            .environment_inputs
            .iter()
            .chain(
                self.collections
                    .iter()
                    .flat_map(|c| c.environments.values()),
            )
            .flat_map(|env| {
                env.variables
                    .iter()
                    .filter(|v| v.secret)
                    .map(|v| v.value.clone())
            })
            .collect::<Vec<_>>();
        for request in &self.requests {
            values.push(request.draft.token.clone());
            values.extend(
                request
                    .draft
                    .cookies
                    .iter()
                    .map(|cookie| cookie.value.clone()),
            );
            values.extend(
                request
                    .draft
                    .headers
                    .iter()
                    .filter(|h| {
                        matches!(
                            h.key.to_ascii_lowercase().as_str(),
                            "authorization"
                                | "cookie"
                                | "x-kso-authorization"
                                | "proxy-authorization"
                        )
                    })
                    .map(|h| h.value.clone()),
            );
        }
        values.retain(|value| value.len() >= 4);
        values.sort_by_key(|value| std::cmp::Reverse(value.len()));
        values.dedup();
        values
    }
}
fn scrub_values(value: &mut serde_json::Value, secrets: &[String]) {
    match value {
        serde_json::Value::String(text) => {
            for secret in secrets {
                if text.contains(secret) {
                    *text = text.replace(secret, "[REDACTED]");
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                scrub_values(item, secrets);
            }
        }
        serde_json::Value::Object(fields) => {
            for item in fields.values_mut() {
                scrub_values(item, secrets);
            }
        }
        _ => {}
    }
}

#[derive(Clone)]
pub struct CallMetadata {
    pub client: String,
    pub rpc_id: String,
    pub started_at_ms: i64,
    pub started: std::time::Instant,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(path: &std::path::Path) -> ApiWorkspace {
        let mut storage = storage::Store::open(path).unwrap();
        let data = storage.load().unwrap();
        ApiWorkspace {
            storage,
            storage_error: None,
            storage_dirty: false,
            requests: data.requests,
            collections: data.collections,
            environment_names: data.environments,
            environment_inputs: data.variables,
            proxies: data.proxies,
            responses: data.responses,
            traces: data.traces,
            sent_requests: data.previews,
            sent_curls: data.curls,
            update_source: data.update_source,
        }
    }

    #[test]
    fn workspace_domain_runs_without_gpui_and_persists_stable_ids() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let mut api = workspace(&path);
        let collection = api.create_collection("Service").unwrap();
        let environment = api.create_environment("Development").unwrap();
        api.collections[collection]
            .environments
            .insert("Development".into(), EnvironmentVariables::default());
        api.rename_environment(environment, "Production").unwrap();
        assert!(
            api.collections[collection]
                .environments
                .contains_key("Production")
        );
        assert!(api.delete_environment(0).is_err());
        api.delete_environment(environment).unwrap();
        assert!(
            !api.collections[collection]
                .environments
                .contains_key("Production")
        );
        assert_eq!(api.environment_names.len(), api.environment_inputs.len());
        api.create_directory(collection, "", "v1").unwrap();
        let draft = patch_draft(
            &Draft::default(),
            &json!({"method":"POST","url":"https://example.test/users"}),
        )
        .unwrap();
        let request = api
            .create_request(
                collection,
                "v1",
                "Create user",
                draft,
                crate::model::VersionSource::Mcp,
            )
            .unwrap();
        let id = api.requests[request].id.clone();
        api.rename_collection(collection, "Renamed").unwrap();
        api.rename_directory(collection, "v1", "v2").unwrap();
        assert_eq!(api.requests[request].collection, "Renamed");
        assert_eq!(api.requests[request].group, "v2");
        assert!(api.patch_request(request, Some("  ".into()), None).is_err());
        assert_eq!(api.requests[request].name, "Create user");
        api.patch_request(request, Some("Updated".into()), None)
            .unwrap();
        api.delete_directory(collection, "v2").unwrap();
        assert_eq!(api.requests[request].group, "");
        assert_eq!(api.request_summary(request).unwrap()["id"], id);
        api.save(0);
        assert!(api.storage_error.is_none());
        assert!(!api.storage_dirty);
        drop(api);
        let api = workspace(&path);
        assert_eq!(api.requests[0].id, id);
        assert_eq!(api.requests[0].name, "Updated");
        assert_eq!(
            api.requests[0].versions[0].source,
            crate::model::VersionSource::Mcp
        );
    }

    #[test]
    fn service_validates_commands_and_redacts_audit_without_a_view() {
        let directory = tempfile::tempdir().unwrap();
        let mut api = workspace(&directory.path().join("workspace.sqlite3"));
        assert!(ToolCommand::parse("glint_missing", json!({})).is_err());
        assert!(ToolCommand::parse("glint_collection_create", json!({"name":42})).is_err());
        let command =
            ToolCommand::parse("glint_request_get", json!({"request_id":"test"})).unwrap();
        assert!(command.read_only);
        let call = Invocation {
            metadata: CallMetadata {
                client: "headless-test".into(),
                rpc_id: "1".into(),
                started_at_ms: 1,
                started: std::time::Instant::now(),
            },
            name: "glint_environment_variable_set".into(),
            arguments: json!({"value":"private-value"}),
            deadline: std::time::Instant::now(),
            rejection: None,
        };
        api.record_call(
            &call,
            &Err("failed: private-value".into()),
            &["private-value".into()],
        )
        .unwrap();
        let page = api
            .storage
            .mcp_call_page("headless-test", None, None, 10)
            .unwrap();
        let record = api.storage.mcp_call_record(page.rows[0].id).unwrap();
        assert!(!record.summary.success);
        assert!(!record.arguments.to_string().contains("private-value"));
        assert!(!record.result.to_string().contains("private-value"));
    }
}
