use super::*;
use serde_json::{Value, json};

fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args[key]
        .as_str()
        .ok_or_else(|| format!("缺少字符串参数 {key}"))
}
fn optional_text<'a>(args: &'a Value, key: &str) -> &'a str {
    args[key].as_str().unwrap_or("")
}
fn flag(args: &Value, key: &str) -> bool {
    args[key].as_bool().unwrap_or(false)
}
fn encode(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).expect("serializable workbench data")
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
pub(super) fn redact(value: &mut Value) {
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
fn patch_draft(base: &Draft, patch: &Value) -> Result<Draft, String> {
    mcp_tools::validate(&mcp_tools::draft_schema(), patch)?;
    let mut value = encode(base);
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
fn name(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        Err("名称不能为空".into())
    } else {
        Ok(value.into())
    }
}
fn record_summary(record: &Record) -> Value {
    json!({"id":record.id,"tag":record.tag,"label":record.label,"time":record.time,"detail":record.detail,"environment_name":record.environment_name,"elapsed_ms":record.elapsed_ms})
}
impl Workbench {
    pub(super) fn start_mcp(&mut self, cx: &mut Context<Self>) {
        let result = mcp::discovery_path().and_then(mcp::Server::bind);
        match result {
            Ok((server, receiver)) => {
                self.mcp_server = Some(server);
                self.mcp_task = Some(cx.spawn(async move |this, cx| {
                    while let Ok(call) = receiver.recv().await {
                        let _ = this.update(cx, |this, cx| {
                            this.execute_mcp_call(call, cx);
                        });
                    }
                }));
            }
            Err(error) => {
                self.mcp_error = Some(error);
            }
        }
    }
    fn mcp_workspace(&mut self, cx: &App) -> storage::Workspace {
        self.capture_draft(cx);
        storage::Workspace {
            update_source: self.update_source.clone(),
            selected_environment: self.selected_environment(cx),
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
    fn mcp_request_index(&self, args: &Value) -> Result<usize, String> {
        let id = text(args, "request_id")?;
        self.requests
            .iter()
            .position(|r| r.id == id)
            .ok_or_else(|| "请求 ID 不存在，请刷新 workspace_get".into())
    }
    fn mcp_collection(&self, args: &Value) -> Result<usize, String> {
        let collection = text(args, "collection")?;
        self.collections
            .iter()
            .position(|c| c.collection == collection)
            .ok_or_else(|| "集合不存在".into())
    }
    fn mcp_environment(&self, args: &Value) -> Result<usize, String> {
        let environment = text(args, "environment")?;
        self.environment_names
            .iter()
            .position(|e| e == environment)
            .ok_or_else(|| "环境不存在".into())
    }
    fn mcp_idle(&self) -> Result<(), String> {
        if self.sending.is_some() {
            Err("请求正在执行，请先 request_cancel；不会自动中断当前请求".into())
        } else {
            Ok(())
        }
    }
    fn mcp_select(&mut self, index: usize, cx: &mut Context<Self>) -> Result<(), String> {
        if index != self.active {
            if !self.ws_active {
                self.mcp_idle()?;
            }
            self.choose(index, cx);
        }
        Ok(())
    }
    fn mcp_environment_options(&mut self, selected: usize, cx: &mut Context<Self>) {
        self.oauth.cancel();
        self.oauth.ready = None;
        self.environment.update(cx, |select, cx| {
            select.set_options(
                self.environment_names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| SelectOption::new(i.to_string(), n.clone())),
                cx,
            );
            select.set_selected(Some(selected.to_string().into()), cx);
        });
        self.sync_reference_inputs(cx);
    }
    fn mcp_node(&self, value: &Value) -> Result<model::TreeNode, String> {
        if value.get("request_id").is_some() {
            if value.get("collection").is_some() || value.get("path").is_some() {
                return Err("节点不能同时指定 request_id 和 collection/path".into());
            }
            return Ok(model::TreeNode::Request(self.mcp_request_index(value)?));
        }
        let collection = self.mcp_collection(value)?;
        let path = optional_text(value, "path");
        if !self.collections[collection].contains(path) {
            return Err("目录不存在".into());
        }
        Ok(model::TreeNode::Directory {
            collection,
            path: path.into(),
        })
    }
    fn mcp_status(&self, index: usize) -> Value {
        let active = index == self.active;
        let response = self.responses[index].as_ref();
        let background = self.ws_background.get(&self.requests[index].id);
        json!({"request_id":self.requests[index].id,"active":active,"running":if active { self.sending.is_some() || self.ws_active } else { background.is_some_and(|state| state.active) },
            "websocket_connected":if active { self.ws_connected } else { background.is_some_and(|state| state.connected) },"sse":response.is_some_and(HttpResponse::is_event_stream),
            "status":response.map(|r| r.status),"bytes":response.map(|r| r.bytes),
            "notice":if active {Some(&self.notice)} else {None},"error":if active {self.error.as_ref()} else {None}})
    }
    pub(super) fn mcp_call(
        &mut self,
        tool: &str,
        args: Value,
        cx: &mut Context<Self>,
    ) -> Result<Value, String> {
        let catalog = mcp_tools::catalog();
        let spec = catalog
            .iter()
            .find(|item| item["name"] == tool)
            .ok_or("未知 MCP 工具")?;
        mcp_tools::validate(&spec["inputSchema"], &args)?;
        let read_only = spec["annotations"]["readOnlyHint"] == true;
        let operation = tool.strip_prefix("glint_").ok_or("未知工具前缀")?;
        self.capture_draft(cx);
        let result = match operation {
            "mcp_history_list" => {
                let success = match args["outcome"].as_str() {
                    Some("success") => Some(true),
                    Some("error") => Some(false),
                    _ => None,
                };
                let page = self.storage.mcp_call_page(
                    optional_text(&args, "query"),
                    success,
                    args["before_id"].as_i64(),
                    args["limit"].as_u64().unwrap_or(100) as usize,
                )?;
                json!({"total":page.total,"has_more":page.has_more,"next_before_id":page.rows.last().map(|r|r.id),"calls":page.rows.iter().map(|r|json!({"id":r.id,"date":r.date(),"time":r.time(),"started_at_ms":r.started_at_ms,"client":r.client,"rpc_id":r.rpc_id,"tool":r.tool,"success":r.success,"duration_ms":r.duration_ms})).collect::<Vec<_>>()})
            }
            "mcp_history_get" => serde_json::to_value(
                self.storage
                    .mcp_call_record(args["id"].as_i64().ok_or("调用 ID 无效")?)?,
            )
            .map_err(|e| e.to_string())?,
            "workspace_get" => {
                json!({"version":updater::VERSION,"workspace_id":self.storage.workspace_id,"workspace_name":self.workspace_label(),"active_request_id":self.requests.get(self.active).map(|r| &r.id),
                "selected_environment":self.environment_name(self.selected_environment(cx)),"environments":self.environment_names,
                "collections":self.collections.iter().map(|c| json!({"name":c.collection,"directories":c.names})).collect::<Vec<_>>(),
                "requests":self.requests.iter().map(|r| json!({"id":r.id,"name":r.name,"collection":r.collection,"path":r.group,"method":r.draft.method,"url":r.draft.url})).collect::<Vec<_>>(),
                "running":!self.running_request_tasks().is_empty(),"background_tasks":self.running_request_tasks().into_iter().map(|(request_id,label)|json!({"request_id":request_id,"label":label})).collect::<Vec<_>>(),"notice":self.notice,"error":self.error,"storage_error":self.storage_error,"mcp_error":self.mcp_error,
                "registry_running":self.registry_task.is_some()})
            }
            "workspace_export" => {
                json!({"workspace":self.mcp_workspace(cx),"redacted":!flag(&args,"include_secrets")})
            }
            "workspace_import" => {
                if !flag(&args, "replace") {
                    return Err("替换工作区必须显式设置 replace=true".into());
                }
                let mut workspace: storage::Workspace =
                    serde_json::from_value(args["workspace"].clone()).map_err(|e| e.to_string())?;
                workspace.validate()?;
                workspace.migrate_proxies();
                self.sending = None;
                self.ws_background.clear();
                self.ws_commands = None;
                self.ws_active = false;
                self.ws_connected = false;
                self.requests = workspace.requests;
                self.collections = workspace.collections;
                self.environment_names = workspace.environments;
                self.editing_variable_collection = None;
                self.environment_inputs = workspace.variables;
                self.proxies = workspace.proxies;
                self.reset_proxy_profiles(cx);
                self.traces = workspace.traces;
                self.responses = workspace.responses;
                self.sent_requests = workspace.previews;
                self.sent_curls = workspace.curls;
                self.sent_curls.resize(self.requests.len(), None);
                self.active = 0;
                self.current_collection = 0;
                self.current_directory.clear();
                self.tabs = if self.requests.is_empty() {
                    vec![]
                } else {
                    vec![0]
                };
                self.current_versions = vec![0; self.requests.len()];
                self.selected_history = None;
                self.directory_settings = None;
                self.group_edit = None;
                self.tree_menu = None;
                self.tree_delete = None;
                self.version_tag_edit = None;
                self.history_delete = None;
                self.collapsed_directories.clear();
                self.variable_rows = (0..self.environment_inputs.len()).map(|_| vec![]).collect();
                self.proxy_inputs.clear();
                self.editing_environment = 0;
                self.reset_variable_rows(cx);
                self.mcp_environment_options(workspace.selected_environment, cx);
                let draft = self
                    .requests
                    .first()
                    .map(|r| r.draft.clone())
                    .unwrap_or_default();
                self.load(&draft, cx);
                self.restore_response(cx);
                self.sync_request_preview(cx);
                self.error = None;
                self.trace_started = None;
                self.cancel_request_name(cx);
                self.sync_commands(cx);
                self.sync_groups(cx);
                json!({"imported_requests":self.requests.len(),"imported_collections":self.collections.len()})
            }
            "collection_create" => {
                let collection =
                    RequestGroups::create_collection(text(&args, "name")?, &self.collections)?;
                self.collections.push(collection);
                self.current_collection = self.collections.len() - 1;
                self.current_directory.clear();
                json!({"collection":self.collections[self.current_collection].collection})
            }
            "collection_rename" => {
                let index = self.mcp_collection(&args)?;
                let old = self.collections[index].collection.clone();
                let next = text(&args, "name")?.trim().to_owned();
                RequestGroups::rename_collection(
                    index,
                    &next,
                    &mut self.collections,
                    &mut self.requests,
                )?;
                if self.editing_variable_collection.as_deref() == Some(&old) {
                    self.editing_variable_collection = Some(next.clone());
                }
                self.directory_settings = None;
                self.sync_commands(cx);
                json!({"collection":next})
            }
            "collection_delete" => {
                self.mcp_idle()?;
                let index = self.mcp_collection(&args)?;
                let old = self.collections[index].collection.clone();
                let indices = self
                    .requests
                    .iter()
                    .enumerate()
                    .filter_map(|(i, r)| (r.collection == old).then_some(i))
                    .collect::<Vec<_>>();
                if !indices.is_empty() && !flag(&args, "delete_requests") {
                    return Err("集合非空，请显式设置 delete_requests=true".into());
                }
                for i in indices.into_iter().rev() {
                    self.delete_request(i, cx);
                }
                self.collections.remove(index);
                if self.editing_variable_collection.as_deref() == Some(&old) {
                    self.editing_variable_collection = None;
                    self.reset_variable_rows(cx);
                }
                self.directory_settings = None;
                self.collapsed_directories.clear();
                self.current_collection = 0;
                self.current_directory.clear();
                json!({"deleted":old})
            }
            "directory_create" | "directory_rename" | "directory_delete" => {
                let c = self.mcp_collection(&args)?;
                self.directory_settings = None;
                match operation {
                    "directory_create" => {
                        let path = self.collections[c]
                            .add_child(optional_text(&args, "parent"), text(&args, "name")?)?;
                        json!({"path":path})
                    }
                    "directory_rename" => {
                        let old = text(&args, "path")?;
                        self.collections[c].rename(
                            old,
                            text(&args, "name")?,
                            &mut self.requests,
                        )?;
                        self.current_directory.clear();
                        self.collapsed_directories.clear();
                        json!({"renamed":true})
                    }
                    _ => {
                        let path = text(&args, "path")?;
                        if path.is_empty() || !self.collections[c].contains(path) {
                            return Err("目录不存在；删除集合请使用 collection_delete".into());
                        }
                        self.collections[c].remove(path, &mut self.requests);
                        self.current_directory.clear();
                        self.collapsed_directories.clear();
                        json!({"deleted":path,"requests_preserved":true})
                    }
                }
            }
            "directory_defaults_get" | "directory_defaults_set" => {
                let c = self.mcp_collection(&args)?;
                let path = optional_text(&args, "path");
                if !self.collections[c].contains(path) {
                    return Err("目录不存在".into());
                }
                if operation.ends_with("set") {
                    let sections = serde_json::from_value(args["sections"].clone())
                        .map_err(|e| e.to_string())?;
                    let mut draft = patch_draft(&Draft::default(), &args["draft"])?;
                    if let Some(bindings) = args["plugin_bindings"].as_array() {
                        draft.plugins = bindings
                            .iter()
                            .map(|binding| {
                                let identity = text(binding, "identity")?;
                                let plugin = self
                                    .plugin_catalog
                                    .iter()
                                    .find(|p| p.identity() == identity)
                                    .ok_or("已安装插件不存在")?;
                                let action = optional_text(binding, "action_id");
                                if !plugin.manifest.contributions.is_empty()
                                    && !plugin
                                        .manifest
                                        .contributions
                                        .iter()
                                        .any(|c| c.id == action && c.kind == plugins::HOOK)
                                {
                                    return Err("插件动作不存在".into());
                                }
                                Ok(model::BeforeAction {
                                    approval: gpui_glint::plugins::Approval::for_plugin(plugin),
                                    action_id: action.into(),
                                    enabled: binding["enabled"] != false,
                                })
                            })
                            .collect::<Result<Vec<_>, String>>()?;
                    }
                    self.collections[c]
                        .defaults
                        .insert(path.into(), model::DirectoryDefaults { sections, draft });
                    self.directory_settings = None;
                }
                json!({"explicit":self.collections[c].defaults.get(path),"resolved":self.collections[c].request_defaults(path)})
            }
            "tree_move" => {
                let source = self.mcp_node(&args["source"])?;
                let target = self.mcp_node(&args["target"])?;
                let placement = match text(&args, "placement")? {
                    "before" => model::Placement::Before,
                    "after" => model::Placement::After,
                    _ => model::Placement::Inside,
                };
                if !matches!((&source,&target), (model::TreeNode::Directory{path:a,..},model::TreeNode::Directory{path:b,..}) if a.is_empty()&&b.is_empty()&&placement!=model::Placement::Inside)
                {
                    model::move_node(
                        &mut self.collections.clone(),
                        &mut self.requests.clone(),
                        &source,
                        &target,
                        placement,
                    )?;
                }
                self.drop_tree_node(&source, &target, placement, cx);
                json!({"moved":true})
            }
            "request_create" | "curl_import" => {
                self.mcp_idle()?;
                let c = self.mcp_collection(&args)?;
                let path = optional_text(&args, "path");
                let title = name(text(&args, "name")?)?;
                if !self.collections[c].contains(path) {
                    return Err("目录不存在".into());
                }
                let draft = if operation == "curl_import" {
                    curl::parse(text(&args, "curl")?)?
                } else {
                    patch_draft(
                        &self.collections[c].request_defaults(path),
                        args.get("draft").unwrap_or(&json!({})),
                    )?
                };
                self.current_collection = c;
                self.add_request_to(path.into(), cx);
                self.requests[self.active].name = title;
                self.load(&draft, cx);
                self.capture_draft(cx);
                self.requests[self.active].versions[0].draft = draft;
                json!({"request_id":self.requests[self.active].id})
            }
            "request_get" => {
                let i = self.mcp_request_index(&args)?;
                let r = &self.requests[i];
                json!({"id":r.id,"name":r.name,"collection":r.collection,"path":r.group,"draft":r.draft,
                    "versions":r.versions.iter().map(record_summary).collect::<Vec<_>>(),"history":r.history.iter().map(record_summary).collect::<Vec<_>>()})
            }
            "request_update" => {
                let i = self.mcp_request_index(&args)?;
                if i == self.active {
                    self.mcp_idle()?;
                }
                let title = args
                    .get("name")
                    .map(|v| name(v.as_str().unwrap()))
                    .transpose()?;
                let draft = args
                    .get("draft")
                    .map(|patch| patch_draft(&self.requests[i].draft, patch))
                    .transpose()?;
                if let Some(title) = title {
                    self.requests[i].name = title;
                }
                if let Some(draft) = draft {
                    self.requests[i].draft = draft.clone();
                    if i == self.active {
                        self.load(&draft, cx);
                    }
                }
                json!({"request_id":self.requests[i].id})
            }
            "request_select" => {
                let i = self.mcp_request_index(&args)?;
                self.mcp_select(i, cx)?;
                json!({"request_id":self.requests[i].id})
            }
            "request_delete" => {
                self.mcp_idle()?;
                let i = self.mcp_request_index(&args)?;
                self.delete_request(i, cx);
                json!({"deleted":true})
            }
            "request_send" => {
                let i = self.mcp_request_index(&args)?;
                if i == self.active || !self.ws_active {
                    self.mcp_idle()?;
                }
                if self
                    .ws_background
                    .get(&self.requests[i].id)
                    .is_some_and(|state| state.active)
                {
                    return Err("该请求的 WebSocket 已在后台运行".into());
                }
                let selected = args
                    .get("environment")
                    .map(|_| self.mcp_environment(&args))
                    .transpose()?;
                self.mcp_select(i, cx)?;
                if let Some(index) = selected {
                    self.mcp_environment_options(index, cx);
                }
                self.directory_settings = None;
                self.show_curl_import = false;
                self.send(cx);
                if self.sending.is_none() {
                    return Err(self.error.clone().unwrap_or_else(|| self.notice.clone()));
                }
                self.mcp_status(i)
            }
            "request_cancel" => {
                let i = self.mcp_request_index(&args)?;
                if i != self.active {
                    let id = self.requests[i].id.clone();
                    self.with_ws_request(&id, cx, |this, cx| this.disconnect_websocket(cx))
                        .ok_or("该请求没有正在执行的操作")?;
                    return Ok(self.mcp_status(i));
                }
                if self.sending.is_some() {
                    if self.ws_active {
                        self.disconnect_websocket(cx);
                    } else {
                        self.sending = None;
                        self.trace_started = None;
                        self.push_trace("Agent 已取消请求", cx);
                        self.notice = "已取消请求".into();
                    }
                }
                self.mcp_status(i)
            }
            "request_status" => {
                let i = self.mcp_request_index(&args)?;
                self.mcp_status(i)
            }
            "request_preview" => {
                let i = self.mcp_request_index(&args)?;
                let proxy = self.effective_proxy(self.selected_environment(cx));
                let mut request = PreparedRequest::new(
                    &self.requests[i].draft,
                    &self.environment_values(
                        self.selected_environment(cx),
                        Some(&self.requests[i].collection),
                    ),
                )?
                .with_proxy(&proxy)?;
                if self.requests[i].draft.oauth.is_some() {
                    request = request.with_oauth_token("[已隐藏]")?;
                }
                json!({"preview":request.preview,"curl":if self.requests[i].draft.method!="WS" {request.to_curl().ok()} else {None},"plugins_applied":false})
            }
            "response_read" => {
                let i = self.mcp_request_index(&args)?;
                let response = if let Some(id) = args["record_id"].as_str() {
                    self.requests[i]
                        .history
                        .iter()
                        .chain(&self.requests[i].versions)
                        .find(|r| r.id == id)
                        .ok_or("记录不存在")?
                        .exchange
                        .as_ref()
                        .and_then(|e| e.response.as_ref())
                } else {
                    self.responses[i].as_ref()
                }
                .ok_or("尚无响应")?;
                let mut offset = args["offset"].as_u64().unwrap_or(0) as usize;
                offset = offset.min(response.text.len());
                while !response.text.is_char_boundary(offset) {
                    offset += 1;
                }
                let mut end = (offset + args["limit"].as_u64().unwrap_or(65536) as usize)
                    .min(response.text.len());
                while !response.text.is_char_boundary(end) {
                    end -= 1;
                }
                if end == offset && end < response.text.len() {
                    end += 1;
                    while !response.text.is_char_boundary(end) {
                        end += 1;
                    }
                }
                json!({"status":response.status,"headers":response.headers,"bytes":response.bytes,"sse":response.is_event_stream(),
                    "text":&response.text[offset..end],"offset":offset,"next_offset":end,"snapshot_bytes":response.text.len(),"has_more":end<response.text.len()})
            }
            "curl_export" => {
                let i = self.mcp_request_index(&args)?;
                json!({"curl":self.sent_curls[i].as_ref().ok_or("尚无已发送的 HTTP 请求快照")?})
            }
            "traces_read" => {
                let i = self.mcp_request_index(&args)?;
                json!({"traces":self.traces[i],"preview":self.sent_requests[i]})
            }
            "response_clear" => {
                let i = self.mcp_request_index(&args)?;
                if i == self.active {
                    self.mcp_idle()?;
                }
                self.responses[i] = None;
                self.traces[i].clear();
                self.sent_requests[i] = None;
                self.sent_curls[i] = None;
                if i == self.active {
                    self.restore_response(cx);
                    self.sync_request_preview(cx);
                    self.error = None;
                    self.status = None;
                }
                json!({"cleared":true})
            }
            "version_save" => {
                self.mcp_idle()?;
                let i = self.mcp_request_index(&args)?;
                self.mcp_select(i, cx)?;
                self.directory_settings = None;
                self.save(cx);
                if let Some(tag) = args["tag"].as_str() {
                    self.requests[i].versions[0].tag = tag.trim().into();
                }
                record_summary(&self.requests[i].versions[0])
            }
            "record_get" | "record_restore" | "record_tag" | "record_delete" => {
                let i = self.mcp_request_index(&args)?;
                let id = text(&args, "record_id")?;
                let version = self.requests[i].versions.iter().position(|r| r.id == id);
                let history = self.requests[i].history.iter().position(|r| r.id == id);
                let record = version
                    .map(|v| self.requests[i].versions[v].clone())
                    .or_else(|| history.map(|h| self.requests[i].history[h].clone()))
                    .ok_or("版本或历史 ID 不存在")?;
                match operation {
                    "record_get" => {
                        let mut value = encode(&record);
                        if let Some(exchange) = value["exchange"].as_object_mut() {
                            exchange.remove("response");
                        }
                        value
                    }
                    "record_restore" => {
                        self.mcp_idle()?;
                        self.mcp_select(i, cx)?;
                        self.load(&record.draft, cx);
                        self.capture_draft(cx);
                        if let Some(v) = version {
                            self.current_versions[i] = v;
                            self.selected_history = None;
                            self.responses[i] = None;
                            self.error = None;
                            self.restore_response(cx);
                        }
                        if let Some(h) = history {
                            self.selected_history = Some((i, h));
                            self.responses[i] =
                                record.exchange.as_ref().and_then(|e| e.response.clone());
                            self.sent_requests[i] =
                                record.exchange.as_ref().and_then(|e| e.preview.clone());
                            self.sent_curls[i] =
                                record.exchange.as_ref().and_then(|e| e.curl.clone());
                            self.traces[i] = record
                                .exchange
                                .as_ref()
                                .map(|e| e.traces.clone())
                                .unwrap_or_default();
                            self.restore_response(cx);
                            self.sync_request_preview(cx);
                            self.error = record.exchange.as_ref().and_then(|e| e.error.clone());
                        }
                        json!({"restored":record.id})
                    }
                    "record_tag" => {
                        let v = version.ok_or("仅版本支持标签")?;
                        self.requests[i].versions[v].tag =
                            optional_text(&args, "tag").trim().into();
                        record_summary(&self.requests[i].versions[v])
                    }
                    _ => {
                        if let Some(v) = version {
                            self.requests[i].versions.remove(v);
                            self.current_versions[i] = 0;
                        } else {
                            self.delete_history_record(i, history.unwrap(), cx);
                        }
                        json!({"deleted":record.id})
                    }
                }
            }
            "environment_create" => {
                let title = name(text(&args, "name")?)?;
                if self.environment_names.contains(&title) {
                    return Err("环境名称已存在".into());
                }
                let selected = self.selected_environment(cx);
                self.environment_names.push(title.clone());
                self.environment_inputs
                    .push(EnvironmentVariables::default());
                self.variable_rows.push(vec![]);
                self.add_proxy_input(cx);
                self.mcp_environment_options(selected, cx);
                json!({"environment":title})
            }
            "environment_get" => {
                let i = self.mcp_environment(&args)?;
                let scope = args["collection"].as_str();
                let variables = if let Some(name) = scope {
                    let collection = self
                        .collections
                        .iter()
                        .find(|c| c.collection == name)
                        .ok_or("集合不存在")?;
                    collection
                        .environments
                        .get(self.environment_name(i))
                        .cloned()
                        .unwrap_or_default()
                } else {
                    self.environment_inputs[i].clone()
                };
                json!({"name":self.environment_names[i],"collection":scope,"variables":variables.variables,"proxy_id":self.environment_inputs[i].proxy_id,"proxy":self.effective_proxy(i),"available_proxies":self.proxies,"editor":if self.editing_variable_collection.as_deref()==scope { self.mcp_environment_editor(i,cx) } else { Value::Null }})
            }
            "environment_select" => {
                let i = self.mcp_environment(&args)?;
                self.mcp_environment_options(i, cx);
                json!({"environment":self.environment_names[i]})
            }
            "environment_rename" | "environment_delete" => {
                let i = self.mcp_environment(&args)?;
                if i == 0 {
                    return Err("不能修改内置的不使用环境项".into());
                }
                if self.has_unsaved_environment(cx) {
                    return Err("环境编辑器有未保存修改，请先 environment_draft 保存或放弃".into());
                }
                let selected = self.selected_environment(cx);
                if operation.ends_with("rename") {
                    let title = name(text(&args, "name")?)?;
                    if self
                        .environment_names
                        .iter()
                        .enumerate()
                        .any(|(j, n)| j != i && *n == title)
                    {
                        return Err("环境名称已存在".into());
                    }
                    let old = self.environment_names[i].clone();
                    for collection in &mut self.collections {
                        if let Some(variables) = collection.environments.remove(&old) {
                            collection.environments.insert(title.clone(), variables);
                        }
                    }
                    self.environment_names[i] = title;
                    self.mcp_environment_options(selected, cx);
                } else {
                    // Running HTTP post-actions capture environment indices.
                    self.mcp_idle()?;
                    let removed = self.environment_names.remove(i);
                    for collection in &mut self.collections {
                        collection.environments.remove(&removed);
                    }
                    self.environment_inputs.remove(i);
                    self.variable_rows.remove(i);
                    self.proxy_inputs.remove(i);
                    self.editing_environment = 0;
                    self.mcp_environment_options(
                        if selected == i {
                            0
                        } else {
                            selected - usize::from(selected > i)
                        },
                        cx,
                    );
                    self.sync_variable_table(cx);
                }
                json!({"updated":true})
            }
            "environment_variable_set"
            | "environment_variable_delete"
            | "environment_variables_replace"
            | "environment_proxy_set"
            | "environment_draft" => {
                let i = self.mcp_environment(&args)?;
                let scope = args["collection"].as_str().map(str::to_owned);
                if scope
                    .as_ref()
                    .is_some_and(|name| !self.collections.iter().any(|c| &c.collection == name))
                {
                    return Err("集合不存在".into());
                }
                if i == 0 && scope.is_none() {
                    return Err("不使用环境时不能设置变量或代理".into());
                }
                if operation == "environment_draft" {
                    self.set_variable_scope(scope.clone(), cx)?;
                    if text(&args, "action")? == "save" {
                        self.editing_environment = i;
                        self.save_environment_variables(cx);
                        if let Some(error) = &self.variable_error {
                            return Err(error.clone());
                        }
                    } else {
                        self.reset_environment_rows(i, cx);
                    }
                } else {
                    if self.has_unsaved_environment(cx) {
                        return Err(
                            "环境编辑器有未保存修改，请先 environment_draft 保存或放弃".into()
                        );
                    }
                    let saved = self.scoped_environment_mut(i, scope.as_deref())?.clone();
                    match operation {
                        "environment_variable_set" => {
                            let variable_name = text(&args, "name")?;
                            let id = args["variable_id"].as_u64().or_else(|| {
                                saved
                                    .variables
                                    .iter()
                                    .find(|v| v.name == variable_name)
                                    .map(|v| v.id)
                            });
                            let secret = args["secret"].as_bool().unwrap_or_else(|| {
                                saved
                                    .variables
                                    .iter()
                                    .find(|v| Some(v.id) == id)
                                    .is_some_and(|v| v.secret)
                            });
                            self.scoped_environment_mut(i, scope.as_deref())?.save(
                                id,
                                variable_name,
                                text(&args, "value")?,
                                secret,
                            )?;
                        }
                        "environment_variable_delete" => {
                            let variable_name = text(&args, "name")?;
                            let position = saved
                                .variables
                                .iter()
                                .position(|v| v.name == variable_name)
                                .ok_or("变量不存在")?;
                            self.scoped_environment_mut(i, scope.as_deref())?
                                .variables
                                .remove(position);
                        }
                        "environment_variables_replace" => {
                            let rows = args["variables"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|v| {
                                    Ok((
                                        text(v, "name")?.into(),
                                        text(v, "value")?.into(),
                                        flag(v, "secret"),
                                    ))
                                })
                                .collect::<Result<Vec<_>, String>>()?;
                            self.scoped_environment_mut(i, scope.as_deref())?
                                .replace(&rows)?;
                        }
                        _ => {
                            if let Some(binding) = args.get("proxy_id") {
                                let id = if binding.is_null() {
                                    None
                                } else {
                                    Some(
                                        binding
                                            .as_str()
                                            .ok_or("proxy_id 必须为字符串或 null")?
                                            .to_owned(),
                                    )
                                };
                                if id
                                    .as_ref()
                                    .is_some_and(|id| !self.proxies.iter().any(|p| &p.id == id))
                                {
                                    return Err("代理不存在".into());
                                }
                                self.environment_inputs[i].proxy_id = id;
                                self.environment_inputs[i].proxy = Default::default();
                            } else {
                                let proxy = environment::ProxySettings {
                                    enabled: flag(&args, "enabled"),
                                    built_in: flag(&args, "built_in"),
                                    mappings: serde_json::from_value(
                                        args.get("mappings").cloned().unwrap_or(json!([])),
                                    )
                                    .map_err(|e| e.to_string())?,
                                    url: args
                                        .get("url")
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                        .into(),
                                    skip_tls_verification: flag(&args, "skip_tls_verification"),
                                };
                                proxy.validate()?;
                                if self.has_unsaved_proxies(cx) {
                                    return Err("代理管理中有未保存修改，请先保存或丢弃".into());
                                }
                                let id = self.environment_inputs[i]
                                    .proxy_id
                                    .clone()
                                    .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                                if let Some(profile) = self.proxies.iter_mut().find(|p| p.id == id)
                                {
                                    profile.settings = proxy;
                                } else {
                                    self.proxies.push(environment::ProxyProfile {
                                        id: id.clone(),
                                        name: format!(
                                            "{} 代理 {}",
                                            self.environment_names[i],
                                            &id[..8]
                                        ),
                                        settings: proxy,
                                    });
                                }
                                self.environment_inputs[i].proxy_id = Some(id);
                                self.environment_inputs[i].proxy = Default::default();
                                self.reset_proxy_profiles(cx);
                            }
                        }
                    }
                    if self.editing_variable_collection == scope {
                        self.reset_environment_rows(i, cx);
                    }
                }
                self.sync_reference_inputs(cx);
                json!({"saved":true})
            }
            "websocket_send" | "websocket_read" | "websocket_clear" => {
                let i = self.mcp_request_index(&args)?;
                let request_id = self.requests[i].id.clone();
                self.with_ws_request(&request_id, cx, |this, cx| -> Result<Value, String> {
                Ok(match operation {
                    "websocket_send" => {
                        if !this.ws_connected {
                            return Err("WebSocket 尚未连接".into());
                        }
                        let message = text(&args, "text")?;
                        let message = if args["resolve_templates"] != false {
                            template::TemplateResolver::for_request(
                                &this.websocket_variables(cx),
                                &if this.ws_background_update { this.requests[this.active].draft.clone() } else { this.snapshot(cx) },
                            )?
                            .resolve(message)?
                        } else {
                            message.into()
                        };
                        if message.len() > 1024 * 1024 {
                            return Err("消息超过 1 MiB".into());
                        }
                        this.ws_commands
                            .as_ref()
                            .ok_or("WebSocket 未连接")?
                            .try_send(websocket::Command::Text(message))
                            .map_err(|_| "发送队列已满或连接已关闭")?;
                        json!({"queued":true})
                    }
                    "websocket_read" => {
                        let after = args["after"].as_u64().unwrap_or(0);
                        json!({"connected":this.ws_connected,"next_after":this.ws_sequence,"messages":this.ws_log.iter().filter(|event| event.id > after).map(|event| event.mcp_value()).collect::<Vec<_>>()})
                    }
                    _ => {
                        this.clear_websocket_messages(cx);
                        json!({"cleared":true})
                    }
                })
                }).ok_or("该请求没有 WebSocket 会话")??
            }
            "plugins_list" => {
                json!({"plugins":self.plugin_catalog.iter().map(|p|json!({"identity":p.identity(),"manifest":p.manifest,"origin":p.origin,"enabled":self.plugin_enabled(p)})).collect::<Vec<_>>(),"errors":self.plugin_errors,"sources":self.registry_sources,"releases":self.registry_releases,"running":self.registry_task.is_some(),"message":self.registry_message})
            }
            "plugin_bind" | "plugin_bindings_reorder" => {
                let i = self.mcp_request_index(&args)?;
                if i == self.active {
                    self.mcp_idle()?;
                }
                if operation == "plugin_bind" {
                    let identity = text(&args, "identity")?;
                    let action = optional_text(&args, "action_id");
                    let bindings = &mut self.requests[i].draft.plugins;
                    let existing = bindings
                        .iter()
                        .position(|b| b.id == identity && b.action_id == action);
                    if flag(&args, "remove") {
                        let position = existing.ok_or("绑定不存在")?;
                        bindings.remove(position);
                    } else {
                        let plugin = self
                            .plugin_catalog
                            .iter()
                            .find(|p| p.identity() == identity)
                            .ok_or("已安装插件不存在")?;
                        if !plugin.manifest.contributions.is_empty()
                            && !plugin
                                .manifest
                                .contributions
                                .iter()
                                .any(|c| c.id == action && c.kind == plugins::HOOK)
                        {
                            return Err("插件动作不存在".into());
                        }
                        let binding = model::BeforeAction {
                            approval: gpui_glint::plugins::Approval::for_plugin(plugin),
                            action_id: action.into(),
                            enabled: args["enabled"] != false,
                        };
                        if let Some(position) = existing {
                            bindings[position] = binding;
                        } else {
                            bindings.push(binding);
                        }
                    }
                } else {
                    let order = args["order"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_u64().unwrap() as usize)
                        .collect::<Vec<_>>();
                    let mut sorted = order.clone();
                    sorted.sort_unstable();
                    if sorted != (0..self.requests[i].draft.plugins.len()).collect::<Vec<_>>() {
                        return Err("order 必须包含所有现有索引且不重复".into());
                    }
                    self.requests[i].draft.plugins = order
                        .iter()
                        .map(|&j| self.requests[i].draft.plugins[j].clone())
                        .collect();
                }
                json!({"bindings":self.requests[i].draft.plugins})
            }
            "registry_source_add" | "registry_source_remove" => {
                if self.registry_task.is_some() {
                    return Err("插件源操作正在执行".into());
                }
                let store = plugins::registry_store()?;
                if operation.ends_with("add") {
                    store.add_source(glint_plugin_registry::Source {
                        name: text(&args, "name")?.into(),
                        url: text(&args, "url")?.into(),
                        public_key: text(&args, "public_key")?.into(),
                        allow_http: flag(&args, "allow_http"),
                    })?;
                } else {
                    store.remove_source(text(&args, "name")?)?;
                    self.registry_releases
                        .retain(|(source, _)| source.name != optional_text(&args, "name"));
                }
                self.registry_sources = store.sources()?;
                json!({"sources":self.registry_sources})
            }
            "registry_refresh" => {
                if self.registry_task.is_some() {
                    return Err("插件源操作正在执行".into());
                }
                self.refresh_registry(cx);
                json!({"started":true})
            }
            "plugin_install" | "plugin_uninstall" => {
                if self.registry_task.is_some() {
                    return Err("插件源操作正在执行".into());
                }
                let source = text(&args, "source")?.to_owned();
                let id = text(&args, "plugin_id")?.to_owned();
                let version = text(&args, "version")?.to_owned();
                let install = operation == "plugin_install";
                let store = plugins::registry_store()?;
                let configured = if install {
                    Some(store.source(&source)?)
                } else {
                    None
                };
                self.registry_operation(
                    move || {
                        if install {
                            store.install(
                                &glint_plugin_registry::Client::new()?,
                                configured.as_ref().unwrap(),
                                &id,
                                &version,
                                "glint-api",
                                updater::VERSION,
                            )?;
                        } else {
                            store.uninstall(&source, &id, &version)?;
                        }
                        Ok(vec![])
                    },
                    true,
                    cx,
                );
                json!({"started":true})
            }
            "ui_set" => {
                if let Some(theme) = args["theme"].as_str() {
                    self.set_theme(
                        if theme == "dark" {
                            ThemePreset::DarkObsidian
                        } else {
                            ThemePreset::Moonstone
                        },
                        cx,
                    );
                }
                if let Some(section) = args["section"].as_str() {
                    self.show_proxies = section == "proxies";
                    self.show_mcp = section == "mcp";
                    self.show_plugins = section == "plugins";
                    self.show_environments = section == "environments";
                    self.directory_settings = None;
                    if self.show_mcp {
                        self.reload_mcp_history(cx);
                    }
                }
                if let Some(tab) = args["config_tab"].as_u64() {
                    self.config = tab as usize;
                }
                if let Some(tab) = args["response_tab"].as_u64() {
                    self.response_tab = tab as usize;
                }
                if let Some(show) = args["show_activity"].as_bool() {
                    self.show_activity = show;
                }
                if let Some(activity) = args["activity"].as_u64() {
                    self.activity = activity as usize;
                }
                if let Some(filter) = args["filter"].as_str() {
                    self.filter
                        .update(cx, |input, cx| input.set_value(filter, cx));
                }
                json!({"updated":true})
            }
            "update_check" => {
                self.check_for_updates(false, cx);
                json!({"started":true})
            }
            "update_status" => self.mcp_update_status(),
            "update_download" => {
                let update = match &self.update_state {
                    update_ui::UpdateState::Available(update) => update.clone(),
                    _ => return Err("没有可下载的更新".into()),
                };
                if !updater::can_install() {
                    return Err("此构建不支持应用内安装更新".into());
                }
                self.download_update(update, cx);
                self.mcp_update_status()
            }
            "update_cancel" => {
                self.update_task = None;
                self.update_state = update_ui::UpdateState::Idle;
                json!({"cancelled":true})
            }
            "update_install" => {
                if !matches!(self.update_state, update_ui::UpdateState::Ready { .. }) {
                    return Err("更新尚未下载完成".into());
                }
                self.install_update(cx);
                if let Some(error) = &self.update_error {
                    return Err(error.clone());
                }
                json!({"restarting":true})
            }
            _ => return Err("工具未实现".into()),
        };
        if !read_only {
            if !matches!(
                operation,
                "ui_set"
                    | "request_cancel"
                    | "websocket_read"
                    | "websocket_clear"
                    | "websocket_send"
                    | "request_status"
                    | "update_status"
            ) {
                self.storage_dirty = true;
            }
            self.sync_commands(cx);
            self.sync_groups(cx);
            self.persist(cx);
            cx.notify();
            if let Some(error) = &self.storage_error {
                return Err(format!("操作已应用但持久化失败，请重试保存：{error}"));
            }
        }
        let mut result = result;
        if !flag(&args, "include_secrets") {
            redact(&mut result);
        }
        Ok(result)
    }
    fn mcp_update_status(&self) -> Value {
        use update_ui::UpdateState;
        let state = match &self.update_state {
            UpdateState::Idle => json!({"state":"idle"}),
            UpdateState::Checking => json!({"state":"checking"}),
            UpdateState::Current => json!({"state":"current"}),
            UpdateState::Available(update) => {
                json!({"state":"available","version":update.version.to_string(),"url":update.release_url()})
            }
            UpdateState::Downloading { received, total } => {
                json!({"state":"downloading","received":received,"total":total})
            }
            UpdateState::Ready { update, .. } => {
                json!({"state":"ready","version":update.version.to_string()})
            }
            UpdateState::Failed(error) => json!({"state":"failed","error":error}),
        };
        json!({"current_version":updater::VERSION,"update":state,"error":self.update_error})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn call(
        host: &mut Workbench,
        operation: &str,
        args: Value,
        cx: &mut Context<Workbench>,
    ) -> Value {
        host.mcp_call(&format!("glint_{operation}"), args, cx)
            .unwrap_or_else(|e| panic!("{operation}: {e}"))
    }
    #[gpui::test]
    fn mcp_collection_environment_scopes_preview_and_lifecycle(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let host = cx.new(|cx| {
            Workbench::with_store(
                storage::Store::open(&dir.path().join("scopes.sqlite3")).unwrap(),
                cx,
            )
        });
        host.update(cx, |host,cx| {
            for name in ["A","B"] { call(host,"collection_create",json!({"name":name}),cx); }
            call(host,"environment_create",json!({"name":"Dev"}),cx);
            call(host,"environment_select",json!({"environment":"Dev"}),cx);
            call(host,"environment_variable_set",json!({"environment":"Dev","name":"host","value":"https://global.test"}),cx);
            call(host,"environment_variable_set",json!({"environment":"Dev","collection":"A","name":"host","value":"https://a.test"}),cx);
            call(host,"environment_variable_set",json!({"environment":"Dev","collection":"A","name":"opaque","value":"private-value","secret":true}),cx);
            let hidden = call(host,"environment_get",json!({"environment":"Dev","collection":"A"}),cx);
            assert_eq!(hidden["variables"][1]["value"],"[REDACTED]");
            assert!(call(host,"environment_get",json!({"environment":"Dev","collection":"B"}),cx)["variables"].as_array().unwrap().is_empty());
            for (collection,url) in [("A","https://a.test"),("B","https://global.test")] {
                let request = call(host,"request_create",json!({"collection":collection,"name":"GET","draft":{"url":"{{host}}/api"}}),cx);
                let preview = call(host,"request_preview",json!({"request_id":request["request_id"],"include_secrets":true}),cx);
                assert!(preview["preview"].as_str().unwrap().contains(url));
            }
            call(host,"environment_rename",json!({"environment":"Dev","name":"Development"}),cx);
            assert!(host.collections[0].environments.contains_key("Development"));
            assert!(!host.collections[0].environments.contains_key("Dev"));
            call(host,"collection_rename",json!({"collection":"A","name":"Renamed"}),cx);
            assert_eq!(host.environment_values(1,Some("Renamed")).0["host"],"https://a.test");
            call(host,"environment_variable_delete",json!({"environment":"Development","collection":"Renamed","name":"host"}),cx);
            assert_eq!(host.environment_values(1,Some("Renamed")).0["host"],"https://global.test");
            call(host,"environment_delete",json!({"environment":"Development"}),cx);
            assert!(host.collections.iter().all(|c| c.environments.is_empty()));
        });
    }
    #[gpui::test]
    fn mcp_crud_uses_live_gui_and_stable_ids_across_save_and_reopen(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let host = cx.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        let (request_id,version_id)=host.update(cx,|host,cx|{
            call(host,"collection_create",json!({"name":"API"}),cx);
            call(host,"directory_create",json!({"collection":"API","name":"A"}),cx);
            call(host,"directory_create",json!({"collection":"API","parent":"A","name":"B"}),cx);
            call(host,"directory_defaults_set",json!({"collection":"API","sections":["endpoint"],"draft":{"url":"http://localhost/echo","method":"POST"}}),cx);
            let created=call(host,"request_create",json!({"collection":"API","path":"A","name":"Echo","draft":{"headers":[{"key":"X-Test","value":"hello"}]}}),cx);
            let id=created["request_id"].as_str().unwrap().to_owned();
            assert_eq!(host.snapshot(cx).url,"http://localhost/echo");
            assert_eq!(host.snapshot(cx).headers[0].note,"");
            call(host,"environment_create",json!({"name":"Dev"}),cx);
            call(host,"environment_variable_set",json!({"environment":"Dev","name":"api_token","value":"secret-value","secret":true}),cx);
            call(host,"environment_variable_set",json!({"environment":"Dev","name":"api_token","value":"new-secret"}),cx);
            let hidden=call(host,"environment_get",json!({"environment":"Dev"}),cx);
            assert_eq!(hidden["variables"][0]["value"],"[REDACTED]");
            assert_eq!(hidden["editor"]["variables"][0]["value"],"[REDACTED]");
            let revealed=call(host,"environment_get",json!({"environment":"Dev","include_secrets":true}),cx);
            assert_eq!(revealed["variables"][0]["value"],"new-secret");
            assert_eq!(revealed["variables"][0]["secret"],true);
            call(host,"environment_select",json!({"environment":"Dev"}),cx);
            assert_eq!(host.selected_environment(cx),1);
            call(host,"request_update",json!({"request_id":id,"draft":{"body":"{\"hello\":true}"}}),cx);
            let version=call(host,"version_save",json!({"request_id":id,"tag":"baseline"}),cx);
            let version_id=version["id"].as_str().unwrap().to_owned();
            call(host,"request_update",json!({"request_id":id,"draft":{"body":"changed"}}),cx);
            call(host,"record_restore",json!({"request_id":id,"record_id":version_id}),cx);
            assert_eq!(host.body.read(cx).content(),"{\"hello\":true}");
            let other=call(host,"request_create",json!({"collection":"API","path":"A/B","name":"Other"}),cx)["request_id"].as_str().unwrap().to_owned();
            call(host,"tree_move",json!({"source":{"request_id":id},"target":{"collection":"API","path":"A/B"},"placement":"inside"}),cx);
            call(host,"directory_rename",json!({"collection":"API","path":"A","name":"Renamed"}),cx);
            call(host,"request_delete",json!({"request_id":other}),cx);
            assert_eq!(host.requests.len(),1);
            let read=call(host,"request_get",json!({"request_id":id}),cx);
            assert_eq!(read["path"],"Renamed/B");
            assert_eq!(read["id"],id);
            let export=call(host,"workspace_export",json!({"include_secrets":true}),cx);
            call(host,"workspace_import",json!({"workspace":export["workspace"],"replace":true}),cx);
            assert_eq!(host.snapshot(cx).body,"{\"hello\":true}");
            (id,version_id)
        });
        let reopened = cx.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(cx, |host, cx| {
            assert_eq!(host.requests[0].id, request_id);
            let record = call(
                host,
                "record_get",
                json!({"request_id":request_id,"record_id":version_id}),
                cx,
            );
            assert_eq!(record["tag"], "baseline");
            assert_eq!(host.environment_inputs[1].variables[0].value, "new-secret");
        });
    }
    #[gpui::test]
    fn mcp_rejects_invalid_mutations_and_does_not_interrupt_running_work(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let host = cx.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        host.update(cx, |host, cx| {
            call(host, "collection_create", json!({"name":"API"}), cx);
            let id = call(
                host,
                "request_create",
                json!({"collection":"API","name":"One"}),
                cx,
            )["request_id"]
                .as_str()
                .unwrap()
                .to_owned();
            let second = call(
                host,
                "request_create",
                json!({"collection":"API","name":"Two"}),
                cx,
            )["request_id"]
                .as_str()
                .unwrap()
                .to_owned();
            let before = encode(host.mcp_workspace(cx));
            for (tool, args) in [
                (
                    "request_update",
                    json!({"request_id":id,"draft":{"method":"INVALID"}}),
                ),
                ("request_create", json!({"collection":"API","name":""})),
                ("collection_delete", json!({"collection":"API"})),
                ("request_delete", json!({"request_id":"missing"})),
                ("workspace_import", json!({"workspace":{},"replace":true})),
            ] {
                assert!(host.mcp_call(&format!("glint_{tool}"), args, cx).is_err());
                assert_eq!(encode(host.mcp_workspace(cx)), before);
            }
            host.sending = Some(cx.spawn(async |_, _| std::future::pending::<()>().await));
            assert!(
                host.mcp_call("glint_request_send", json!({"request_id":id}), cx)
                    .is_err()
            );
            assert!(
                host.mcp_call("glint_request_select", json!({"request_id":id}), cx)
                    .is_err()
            );
            assert!(host.sending.is_some());
            call(host, "request_get", json!({"request_id":id}), cx);
            assert_eq!(
                host.requests[host.active].id, second,
                "reading another request must not change the active execution"
            );
            call(host, "request_cancel", json!({"request_id":second}), cx);
            assert!(host.sending.is_none());
        });
    }
    #[gpui::test]
    fn mcp_ws_commands_cursors_plugin_grants_and_response_slices(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let host = cx.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        host.update(cx,|host,cx|{
            call(host,"collection_create",json!({"name":"API"}),cx);
            let id=call(host,"request_create",json!({"collection":"API","name":"WS","draft":{"method":"WS","url":"ws://localhost/"}}),cx)["request_id"].as_str().unwrap().to_owned();
            let plugin=host.plugin_catalog.iter().find(|p|p.manifest.id=="com.wps.kso1").unwrap();
            let identity=plugin.identity();let action=plugin.manifest.contributions.first().map(|c|c.id.clone()).unwrap_or_default();
            let result=call(host,"plugin_bind",json!({"request_id":id,"identity":identity,"action_id":action}),cx);
            assert_eq!(result["bindings"][0]["enabled"],true);
            assert!(host.requests[0].draft.plugins[0].approval.matches(host.plugin_catalog.iter().find(|p|p.identity()==identity).unwrap()));
            call(host,"plugin_bind",json!({"request_id":id,"identity":identity,"action_id":action,"remove":true}),cx);
            assert!(host.requests[0].draft.plugins.is_empty());
            let (sender,receiver)=async_channel::bounded(16);host.ws_commands=Some(sender);host.ws_connected=true;host.ws_active=true;
            call(host,"websocket_send",json!({"request_id":id,"text":"hello"}),cx);
            assert!(matches!(receiver.try_recv().unwrap(),websocket::Command::Text(text) if text=="hello"));
            host.ws_sequence=7;host.ws_append("received".into(),cx);
            let log=call(host,"websocket_read",json!({"request_id":id,"after":7}),cx);
            assert_eq!(log["messages"][0]["sequence"],8);
            assert!(call(host,"websocket_read",json!({"request_id":id,"after":8}),cx)["messages"].as_array().unwrap().is_empty());
            call(host,"websocket_clear",json!({"request_id":id}),cx);
            assert_eq!(host.ws_sequence,8);
            host.ws_active=false;host.ws_connected=false;host.ws_commands=None;
            host.responses[0]=Some(HttpResponse{status:200,headers:vec![("Set-Cookie".into(),"secret-session".into())],text:"你好world".into(),bytes:11,json:None});
            let response=call(host,"response_read",json!({"request_id":id,"offset":1,"limit":4}),cx);
            assert_eq!(response["text"],"好w");assert_eq!(response["offset"],3);
            assert_eq!(response["headers"][0][1],"[REDACTED]");
        });
    }
}
