mod environment;
mod environment_ui;
mod http;
mod model;
mod storage;

use environment::EnvironmentVariables;
use environment_ui::VariableInput;
use gpui::{
    App, AppContext, Bounds, ClipboardItem, Context, Entity, FocusHandle, Focusable, FontWeight,
    KeyBinding, Subscription, Task, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    point, prelude::*, px, rgb, size,
};
use gpui_glint::{
    components::{
        AppLayout, Button, ButtonKind, ButtonSize, Checkbox, ClosableTab, ClosableTabs, CodeCard,
        CommandItem, CommandPalette, CommandPaletteEvent, DataTable, GlintAssets, GroupList,
        GroupListEvent, GroupListGroup, GroupListHeader, GroupListItem, Icon, IconButton, Popover,
        SearchInput, Select, SelectEvent, SelectField, SelectFieldEvent, SelectOption, TableCell,
        TableColumn, TableEvent, TableRow, TextEntry, TextReferenceResolver, ThemeSelector,
        ThemeSelectorEvent, bind_code_card_keys, bind_text_entry_keys,
    },
    theme::ThemePreset,
};
use http::{HttpResponse, PreparedRequest};
use model::{Draft, Pair, Record, Request, RequestGroups};
use std::collections::HashSet;
use std::time::Instant;

gpui::actions!(api_workbench, [SendRequest, SaveRequest, SearchCommands]);

enum GroupEdit {
    CreateCollection,
    CreateEnvironment,
    Create,
    Rename(String),
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct TraceEntry {
    elapsed_ms: u128,
    message: String,
}
struct PairInput {
    enabled: bool,
    key: Entity<TextEntry>,
    value: Entity<TextEntry>,
    note: Entity<TextEntry>,
}
struct Workbench {
    storage: storage::Store,
    environment_names: Vec<String>,
    responses: Vec<Option<HttpResponse>>,
    storage_error: Option<String>,
    storage_dirty: bool,
    focus: FocusHandle,
    theme: ThemePreset,
    requests: Vec<Request>,
    collections: Vec<RequestGroups>,
    current_collection: usize,
    show_collection_menu: bool,
    request_list: Entity<GroupList>,
    group_edit: Option<GroupEdit>,
    group_name: Entity<TextEntry>,
    group_error: Option<String>,
    active: usize,
    tabs: Vec<usize>,
    config: usize,
    response_tab: usize,
    traces: Vec<Vec<TraceEntry>>,
    sent_requests: Vec<Option<String>>,
    request_preview: Entity<CodeCard>,
    trace_started: Option<Instant>,
    activity: usize,
    show_activity: bool,
    show_environments: bool,
    editing_environment: usize,
    revealed_variables: HashSet<(usize, u64)>,
    variable_delete: Option<(usize, u64)>,
    variable_search: Entity<TextEntry>,
    variable_error: Option<String>,
    variable_rows: Vec<Vec<VariableInput>>,
    variable_table: Entity<DataTable>,
    next_variable_row: u64,
    requests_expanded: bool,
    history_filter: usize,
    selected_history: Option<(usize, usize)>,
    current_versions: Vec<usize>,
    filter: Entity<TextEntry>,
    history_search: Entity<TextEntry>,
    authentication: Entity<SelectField>,
    endpoint: Entity<SelectField>,
    theme_selector: Entity<ThemeSelector>,
    environment: Entity<Select>,
    environment_inputs: Vec<EnvironmentVariables>,
    params: Vec<PairInput>,
    headers: Vec<PairInput>,
    cookies: Vec<PairInput>,
    params_table: Entity<DataTable>,
    headers_table: Entity<DataTable>,
    response_headers_table: Entity<DataTable>,
    response_headers_scroll: gpui::ScrollHandle,
    cookies_table: Entity<DataTable>,
    body: Entity<CodeCard>,
    response: Entity<CodeCard>,
    commands: Entity<CommandPalette>,
    response_json: Option<serde_json::Value>,
    response_data: Option<HttpResponse>,
    status: Option<u16>,
    pretty: bool,
    html_preview: bool,
    #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
    html_webview: Option<Entity<gpui_glint::components::WebView>>,
    #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
    html_webview_source: Option<String>,
    sending: Option<Task<()>>,
    notice: String,
    error: Option<String>,
    subscriptions: Vec<Subscription>,
    row_subscriptions: Vec<Subscription>,
}
impl Workbench {
    fn entry(value: &str, placeholder: &str, cx: &mut Context<Self>) -> Entity<TextEntry> {
        cx.new(|cx| TextEntry::new(cx, placeholder.to_owned(), value.to_owned()).font_size(12.))
    }
    fn new(cx: &mut Context<Self>) -> Self {
        let path = storage::Store::default_path().expect("无法确定 SQLite 数据目录");
        let storage = storage::Store::open(&path).expect("无法打开 SQLite 数据库");
        Self::with_store(storage, cx)
    }
    fn with_store(mut storage: storage::Store, cx: &mut Context<Self>) -> Self {
        let data = storage
            .load()
            .expect("无法加载 SQLite 工作区，已保留原文件");
        let requests = data.requests;
        let collections = data.collections;
        let environment_names = data.environments;
        let environment_inputs = data.variables;
        let environment_count = environment_names.len();
        let request_list = cx.new(|_| GroupList::new([]));
        let group_name = Self::entry("", "名称", cx);
        let initial = requests
            .first()
            .map(|r| r.draft.clone())
            .unwrap_or_default();
        let authentication = cx.new(|cx| {
            SelectField::new(
                [
                    SelectOption::new("bearer", "Bearer"),
                    SelectOption::new("none", "No Auth"),
                ],
                "Token 或 {{api_token}}",
                cx,
            )
        });
        let filter = Self::entry("", "筛选集合、分组或请求…", cx);
        let history_search = Self::entry("", "搜索历史路径…", cx);
        let endpoint = cx.new(|cx| {
            SelectField::new(
                ["GET", "POST", "PUT", "PATCH", "DELETE"]
                    .map(|s| SelectOption::new(s, s).color(method_color(s))),
                "https://",
                cx,
            )
        });
        let theme_selector = cx.new(|cx| {
            ThemeSelector::new(cx)
                .themes([ThemePreset::Moonstone, ThemePreset::DarkObsidian])
                .selected(ThemePreset::Moonstone)
                .theme(ThemePreset::Moonstone)
                .width(220.)
        });
        let environment = cx.new(|cx| {
            Select::new(
                environment_names
                    .iter()
                    .enumerate()
                    .map(|(i, label)| SelectOption::new(i.to_string(), label.clone())),
                cx,
            )
            .selected("0")
            .width(112.)
        });
        let body = cx.new(|cx| CodeCard::new(cx, "json", initial.body.clone()));
        let response = cx.new(|cx| {
            CodeCard::new(cx, "json", "")
                .read_only(true)
                .show_header(false)
                .viewport()
        });
        let request_preview = cx.new(|cx| {
            CodeCard::new(cx, "http", "")
                .read_only(true)
                .show_header(false)
        });
        let commands = cx.new(|cx| {
            CommandPalette::new(
                requests
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        CommandItem::new(i.to_string(), r.name.clone())
                            .group("请求")
                            .keywords(format!("{} {}", r.draft.method, r.draft.url))
                            .icon(Icon::ApiDoc)
                    })
                    .chain([
                        CommandItem::new("send", "发送 HTTP 请求")
                            .group("操作")
                            .icon(Icon::Play)
                            .shortcut("⌘ Enter"),
                        CommandItem::new("save", "保存请求版本")
                            .group("操作")
                            .icon(Icon::Save)
                            .shortcut("⌘ S"),
                        CommandItem::new("theme", "切换明暗主题")
                            .group("操作")
                            .icon(Icon::HalfCircle),
                    ]),
                cx,
            )
        });
        let variable_search = Self::entry("", "搜索变量名称…", cx);
        let mut subscriptions = vec![];
        subscriptions.push(cx.observe(&variable_search, |this: &mut Self, _, cx| {
            this.sync_variable_table(cx);
            cx.notify();
        }));
        subscriptions.push(cx.observe(&filter, |this, _, cx| {
            this.sync_groups(cx);
            cx.notify();
        }));
        subscriptions.push(cx.subscribe(
            &request_list,
            |this: &mut Self, _, event: &GroupListEvent, cx| match event {
                GroupListEvent::ItemSelected { item, .. } => {
                    if let Ok(index) = item.parse::<usize>() {
                        this.choose(index, cx);
                    }
                }
                GroupListEvent::ItemAction { item, .. } => {
                    if let Ok(index) = item.parse::<usize>() {
                        this.delete_request(index, cx);
                    }
                }
                GroupListEvent::HeaderAction { group, index } => {
                    let Some((collection, group)) = group.split_once(':') else {
                        return;
                    };
                    let Ok(collection) = collection.parse::<usize>() else {
                        return;
                    };
                    if collection >= this.collections.len() {
                        return;
                    }
                    this.current_collection = collection;
                    match index {
                        0 => {
                            this.add_request_to(group.to_string(), cx);
                        }
                        1 => this.edit_group(Some(group.to_string()), cx),
                        2 => {
                            this.collections[this.current_collection]
                                .remove(group, &mut this.requests);
                            this.storage_dirty = true;
                            this.group_edit = None;
                            this.sync_groups(cx);
                            this.notice = format!("已删除分组 {group}，请求保留在未分组中");
                            cx.notify();
                        }
                        _ => {}
                    }
                }
                _ => {}
            },
        ));
        for entry in [&history_search, &group_name] {
            subscriptions.push(cx.observe(entry, |_, _, cx| cx.notify()));
        }
        subscriptions.push(cx.observe(&body, |_, _, cx| cx.notify()));
        subscriptions.push(cx.observe(&commands, |_, _, cx| cx.notify()));
        subscriptions.push(cx.observe(&endpoint, |_, _, cx| cx.notify()));
        subscriptions.push(cx.observe(&authentication, |_, _, cx| cx.notify()));
        subscriptions.push(cx.subscribe(
            &authentication,
            |_: &mut Self, field, event: &SelectFieldEvent, cx| {
                let SelectFieldEvent::Changed(value) = event;
                let disabled = value.selected.as_deref() == Some("none");
                field.update(cx, |field, cx| field.set_input_disabled(disabled, cx));
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe(
            &theme_selector,
            |this: &mut Self, _, event: &ThemeSelectorEvent, cx| {
                let ThemeSelectorEvent::Changed(theme) = event;
                this.set_theme(*theme, cx);
            },
        ));
        subscriptions.push(cx.subscribe(
            &environment,
            |this: &mut Self, _, event: &SelectEvent, cx| {
                let SelectEvent::Changed(value) = event;
                this.environment.update(cx, |select, cx| {
                    select.set_selected(Some(value.clone()), cx)
                });
                this.notice = format!(
                    "已切换到{}",
                    this.environment_name(value.parse::<usize>().unwrap_or(0))
                );
                this.sync_reference_inputs(cx);
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe(
            &commands,
            |this: &mut Self, _, event: &CommandPaletteEvent, cx| {
                if let CommandPaletteEvent::Executed(id) = event {
                    match id.as_ref() {
                        "send" => this.send(cx),
                        "save" => this.save(cx),
                        "theme" => this.toggle_theme(cx),
                        id => {
                            if let Ok(index) = id.parse::<usize>() {
                                this.choose(index, cx);
                            }
                        }
                    }
                }
            },
        ));
        let make_table = |cx: &mut Context<DataTable>| {
            DataTable::new(
                cx,
                [
                    TableColumn::new("", 48.),
                    TableColumn::new("键", 120.).flexible(),
                    TableColumn::new("值", 120.).flexible(),
                    TableColumn::new("说明", 140.),
                    TableColumn::new("", 36.),
                ],
                [],
            )
            .document()
            .compact()
            .add_row("添加空行")
        };
        let params_table = cx.new(make_table);
        let headers_table = cx.new(make_table);
        let cookies_table = cx.new(make_table);
        let response_headers_table = cx.new(|cx| {
            DataTable::new(
                cx,
                [
                    TableColumn::new("名称", 180.),
                    TableColumn::new("值", 240.).flexible(),
                ],
                [],
            )
            .document()
            .compact()
        });
        for (kind, table) in [(0, &params_table), (1, &headers_table), (2, &cookies_table)] {
            subscriptions.push(cx.subscribe(
                table,
                move |this: &mut Self, _, event: &TableEvent, cx| {
                    if let TableEvent::AddRow = event {
                        let row = this.pair(&Pair::new("", "", ""), cx);
                        match kind {
                            1 => this.headers.push(row),
                            2 => this.cookies.push(row),
                            _ => this.params.push(row),
                        }
                        this.sync_pair_tables(cx);
                        cx.notify();
                    }
                },
            ));
        }
        let variable_table = cx.new(|cx| {
            DataTable::new(
                cx,
                [
                    TableColumn::new("变量名称", 180.).flexible(),
                    TableColumn::new("变量值", 240.).flexible(),
                    // Compact cells reserve 8px on each side, plus a gap to row actions.
                    TableColumn::new("类型", 106.),
                    TableColumn::new("", 76.),
                ],
                [],
            )
            .document()
            .compact()
            .add_row("添加一行")
        });
        subscriptions.push(cx.subscribe(
            &variable_table,
            |this: &mut Self, _, event: &TableEvent, cx| {
                if let TableEvent::AddRow = event {
                    this.add_variable(cx);
                }
            },
        ));
        let tabs = if requests.is_empty() { vec![] } else { vec![0] };
        let traces = data.traces;
        let current_versions = vec![0; requests.len()];
        let sent_requests = data.previews;
        let mut this = Self {
            storage,
            storage_error: None,
            storage_dirty: true,
            environment_names,
            responses: data.responses,
            focus: cx.focus_handle(),
            theme: ThemePreset::Moonstone,
            requests,
            collections,
            current_collection: 0,
            show_collection_menu: false,
            request_list,
            group_name,
            group_edit: None,
            group_error: None,
            active: 0,
            tabs,
            config: 0,
            response_tab: 0,
            traces,
            sent_requests,
            request_preview,
            trace_started: None,
            activity: 0,
            show_activity: true,
            show_environments: false,
            editing_environment: 0,
            revealed_variables: HashSet::new(),
            variable_delete: None,
            variable_search,
            variable_error: None,
            variable_rows: vec![vec![]; environment_count],
            variable_table,
            next_variable_row: 0,
            requests_expanded: true,
            history_filter: 0,
            selected_history: None,
            current_versions,
            filter,
            history_search,
            authentication,
            endpoint,
            theme_selector,
            environment,
            environment_inputs,
            params: vec![],
            headers: vec![],
            cookies: vec![],
            params_table,
            headers_table,
            response_headers_table,
            response_headers_scroll: gpui::ScrollHandle::new(),
            cookies_table,
            body,
            response,
            commands,
            response_json: None,
            response_data: None,
            status: None,
            pretty: true,
            html_preview: false,
            #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
            html_webview: None,
            #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
            html_webview_source: None,
            sending: None,
            notice: "HTTP 请求 · 请求与版本自动保存到 SQLite".into(),
            error: None,
            subscriptions,
            row_subscriptions: vec![],
        };
        this.reset_variable_rows(cx);
        this.load(&initial, cx);
        this.restore_response(cx);
        let subscription = cx.observe_self(|this, cx| this.persist(cx));
        this.subscriptions.push(subscription);
        this.set_theme(ThemePreset::Moonstone, cx);
        this
    }
    fn environment_name(&self, index: usize) -> &str {
        self.environment_names
            .get(index)
            .map(String::as_str)
            .unwrap_or("不使用环境")
    }
    fn capture_draft(&mut self, cx: &App) {
        if !self.requests.is_empty() {
            let draft = self.snapshot(cx);
            if self.requests[self.active].draft != draft {
                self.requests[self.active].draft = draft;
                self.storage_dirty = true;
            }
        }
    }
    fn persist(&mut self, cx: &mut Context<Self>) {
        self.capture_draft(cx);
        if !self.storage_dirty {
            return;
        }
        let workspace = storage::Workspace {
            requests: self.requests.clone(),
            collections: self.collections.clone(),
            environments: self.environment_names.clone(),
            variables: self.environment_inputs.clone(),
            traces: self.traces.clone(),
            responses: self.responses.clone(),
            previews: self.sent_requests.clone(),
        };
        let error = self.storage.save(&workspace).err();
        if error.is_none() {
            self.storage_dirty = false;
        }
        if error != self.storage_error {
            self.storage_error = error;
            cx.notify();
        }
    }
    fn restore_response(&mut self, cx: &mut Context<Self>) {
        self.response_data = self.responses.get(self.active).cloned().flatten();
        self.status = self.response_data.as_ref().map(|r| r.status);
        self.response_json = self.response_data.as_ref().and_then(|r| r.json.clone());
        self.sync_request_preview(cx);
        self.update_response(cx);
    }
    fn snapshot(&self, cx: &App) -> Draft {
        let pairs = |rows: &[PairInput]| {
            rows.iter()
                .map(|r| Pair {
                    enabled: r.enabled,
                    key: r.key.read(cx).value().into(),
                    value: r.value.read(cx).value().into(),
                    note: r.note.read(cx).value().into(),
                })
                .collect()
        };
        Draft {
            method: self
                .endpoint
                .read(cx)
                .value()
                .selected
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or("GET".into()),
            url: self.endpoint.read(cx).value().text.to_string(),
            body: self.body.read(cx).content().into(),
            params: pairs(&self.params),
            headers: pairs(&self.headers),
            cookies: pairs(&self.cookies),
            token: self.authentication.read(cx).value().text.to_string(),
            auth: self.authentication.read(cx).value().selected.as_deref() == Some("bearer"),
            environment: self
                .environment
                .read(cx)
                .value()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        }
    }
    fn reference_resolver(&self, cx: &App) -> TextReferenceResolver {
        let values = self.environment_inputs[self.selected_environment(cx)].values();
        std::rc::Rc::new(move |expression| values.resolve(expression))
    }
    fn sync_reference_inputs(&mut self, cx: &mut Context<Self>) {
        let resolver = self.reference_resolver(cx);
        for row in self
            .params
            .iter()
            .chain(self.headers.iter())
            .chain(self.cookies.iter())
        {
            for entry in [&row.key, &row.value] {
                entry.update(cx, |entry, cx| {
                    entry.set_reference_resolver(Some(resolver.clone()), cx)
                });
            }
        }
        for field in [&self.endpoint, &self.authentication] {
            field.update(cx, |field, cx| {
                field.set_reference_resolver(Some(resolver.clone()), cx)
            });
        }
    }
    fn pair(&mut self, pair: &Pair, cx: &mut Context<Self>) -> PairInput {
        let key = Self::entry(&pair.key, "键", cx);
        let value = Self::entry(&pair.value, "值", cx);
        let note = Self::entry(&pair.note, "说明", cx);
        for entry in [&key, &value, &note] {
            entry.update(cx, |entry, cx| entry.set_theme(self.theme, cx));
            self.row_subscriptions
                .push(cx.observe(entry, |_, _, cx| cx.notify()));
        }
        let resolver = self.reference_resolver(cx);
        for entry in [&key, &value] {
            entry.update(cx, |entry, cx| {
                entry.set_reference_resolver(Some(resolver.clone()), cx)
            });
        }
        PairInput {
            enabled: pair.enabled,
            key,
            value,
            note,
        }
    }
    fn load(&mut self, draft: &Draft, cx: &mut Context<Self>) {
        self.authentication.update(cx, |field, cx| {
            field.set_selected(Some(if draft.auth { "bearer" } else { "none" }.into()), cx);
            field.set_text(draft.token.clone(), cx);
            field.set_input_disabled(!draft.auth, cx);
        });
        self.endpoint.update(cx, |field, cx| {
            field.set_selected(Some(draft.method.clone().into()), cx);
            field.set_text(draft.url.clone(), cx);
        });
        self.environment.update(cx, |select, cx| {
            select.set_selected(Some(draft.environment.to_string().into()), cx)
        });
        self.body
            .update(cx, |editor, cx| editor.set_content(draft.body.clone(), cx));
        self.row_subscriptions.clear();
        self.params = draft.params.iter().map(|p| self.pair(p, cx)).collect();
        self.headers = draft.headers.iter().map(|p| self.pair(p, cx)).collect();
        self.cookies = draft.cookies.iter().map(|p| self.pair(p, cx)).collect();
        self.sync_pair_tables(cx);
        self.sync_reference_inputs(cx);
        cx.notify();
    }
    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.requests.len() {
            return;
        }
        if !self.requests.is_empty() {
            self.capture_draft(cx);
        }
        if self.sending.take().is_some() {
            self.push_trace("切换请求，已取消等待中的请求", cx);
        }
        self.trace_started = None;
        self.active = index;
        if let Some(collection) = self
            .collections
            .iter()
            .position(|collection| collection.collection == self.requests[index].collection)
        {
            if self.current_collection != collection {
                self.group_edit = None;
                self.group_error = None;
            }
            self.current_collection = collection;
            self.requests_expanded = true;
        }
        self.sync_request_preview(cx);
        self.sync_groups(cx);
        if !self.tabs.contains(&index) {
            self.tabs.push(index);
        }
        let draft = self.requests[index].draft.clone();
        self.config = if matches!(draft.method.as_str(), "POST" | "PUT" | "PATCH") {
            3
        } else {
            0
        };
        self.load(&draft, cx);
        self.restore_response(cx);
        self.error = None;
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        if self.requests.is_empty() {
            return;
        }
        let draft = self.snapshot(cx);
        let request = &mut self.requests[self.active];
        request.draft = draft.clone();
        self.storage_dirty = true;
        request.versions.insert(
            0,
            Record {
                draft,
                label: format!("v{}", request.versions.len() + 1),
                time: record_time(),
                detail: "保存请求时生成的快照".into(),
                elapsed_ms: None,
            },
        );
        self.current_versions[self.active] = 0;
        self.notice = "已保存新版本".into();
        self.activity = 0;
        self.show_activity = true;
        cx.notify();
    }
    fn send(&mut self, cx: &mut Context<Self>) {
        if self.requests.is_empty() {
            return;
        }
        if self.sending.take().is_some() {
            self.push_trace("已取消请求", cx);
            self.trace_started = None;
            self.notice = "已取消请求".into();
            cx.notify();
            return;
        }
        let draft = self.snapshot(cx);
        self.trace_started = Some(Instant::now());
        self.traces[self.active].clear();
        self.push_trace(
            format!(
                "准备 {} 请求 · {}",
                draft.method,
                self.environment_name(draft.environment)
            ),
            cx,
        );
        self.response_json = None;
        self.response_data = None;
        self.status = None;
        self.error = None;
        let inputs = &self.environment_inputs[draft.environment];
        let variables = inputs.values();
        let request = match PreparedRequest::new(&draft, &variables) {
            Ok(request) => request,
            Err(error) => {
                self.push_trace(format!("校验失败：{error}"), cx);
                self.trace_started = None;
                self.error = Some(error);
                self.notice = "请求配置有误".into();
                cx.notify();
                return;
            }
        };
        self.sent_requests[self.active] = Some(request.preview.clone());
        self.sync_request_preview(cx);
        self.push_trace("URL、环境变量与请求头校验通过", cx);
        if draft
            .cookies
            .iter()
            .any(|p| p.enabled && !p.key.trim().is_empty())
        {
            self.push_trace("已合并 Cookie 覆盖表格到请求头", cx);
        }
        self.push_trace("发送 HTTP 请求，等待服务器响应", cx);
        self.requests[self.active].draft = draft.clone();
        self.notice = "正在发送 HTTP 请求".into();
        let index = self.active;
        let started = Instant::now();
        self.sending = Some(cx.spawn(async move |this, cx| {
            let result = http::execute(request).await;
            let elapsed_ms = started.elapsed().as_millis();
            let _ = this.update(cx, |this, cx| {
                let (label, detail) = match &result {
                    Ok(response) => (
                        status_label(response.status),
                        format!(
                            "{elapsed_ms} ms · {} · {} B",
                            this.environment_name(draft.environment),
                            response.bytes
                        ),
                    ),
                    Err(error) => ("请求失败".into(), format!("{elapsed_ms} ms · {error}")),
                };
                this.storage_dirty = true;
                this.requests[index].history.insert(
                    0,
                    Record {
                        draft,
                        label,
                        detail,
                        time: record_time(),
                        elapsed_ms: Some(elapsed_ms),
                    },
                );
                this.selected_history = Some((index, 0));
                match result {
                    Ok(response) => {
                        this.push_trace(
                            format!(
                                "收到响应 · {} · {} B",
                                status_label(response.status),
                                response.bytes
                            ),
                            cx,
                        );
                        this.push_trace(
                            if response.status < 400 {
                                "请求完成"
                            } else {
                                "请求完成 · HTTP 错误状态"
                            },
                            cx,
                        );
                        this.responses[index] = Some(response.clone());
                        this.status = Some(response.status);
                        this.response_json = response.json.clone();
                        this.response_data = Some(response);
                        this.update_response(cx);
                        this.notice = "请求完成".into();
                    }
                    Err(error) => {
                        this.push_trace(format!("请求失败：{error}"), cx);
                        this.error = Some(error);
                        this.notice = "请求失败".into();
                    }
                }
                this.trace_started = None;
                this.sending = None;
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn push_trace(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.storage_dirty = true;
        self.traces[self.active].push(TraceEntry {
            elapsed_ms: self
                .trace_started
                .map(|start| start.elapsed().as_millis())
                .unwrap_or(0),
            message: message.into(),
        });
        cx.notify();
    }
    fn trace_view(&self) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let entries = &self.traces[self.active];
        let mut body = div()
            .id("execution-trace-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .text_size(px(11.));
        if entries.is_empty() {
            return body
                .text_color(rgb(p.text_muted))
                .child("发送请求后显示执行轨迹。");
        }
        body = body.child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(p.text))
                .child(format!(
                    "{} 条记录 · {}",
                    entries.len(),
                    entries.last().unwrap().message
                )),
        );
        for entry in entries {
            body = body.child(
                div()
                    .flex()
                    .gap_3()
                    .items_start()
                    .flex_shrink_0()
                    .child(
                        div()
                            .w(px(64.))
                            .flex_shrink_0()
                            .font_family("Menlo")
                            .text_color(rgb(p.text_muted))
                            .child(format!("+{} ms", entry.elapsed_ms)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(rgb(p.text))
                            .child(entry.message.clone()),
                    ),
            );
        }
        body
    }
    fn response_content(&self) -> String {
        let Some(response) = &self.response_data else {
            return String::new();
        };
        if self.pretty {
            if let Some(value) = &response.json {
                return serde_json::to_string_pretty(value).unwrap();
            }
        }
        response.text.clone()
    }
    fn update_response(&mut self, cx: &mut Context<Self>) {
        let rows = self
            .response_data
            .iter()
            .flat_map(|response| {
                response
                    .headers
                    .iter()
                    .enumerate()
                    .map(|(index, (key, value))| {
                        let value = value.clone();
                        TableRow::new(
                            format!("response-header-{index}"),
                            [
                                TableCell::mono(key.clone()),
                                TableCell::custom(move || {
                                    div()
                                        .id(("response-header-value", index))
                                        .w_full()
                                        .min_w_0()
                                        .overflow_x_scroll()
                                        .font_family("Menlo")
                                        .text_size(px(11.))
                                        .whitespace_nowrap()
                                        .child(value.clone())
                                }),
                            ],
                        )
                    })
            })
            .collect::<Vec<_>>();
        self.response_headers_table
            .update(cx, |table, cx| table.set_rows(rows, cx));
        self.response_headers_scroll
            .set_offset(point(px(0.), px(0.)));
        let content = self.response_content();
        self.html_preview = false;
        let language = self
            .response_data
            .as_ref()
            .map(HttpResponse::language)
            .unwrap_or("text");
        self.response.update(cx, |editor, cx| {
            editor.set_language(language, cx);
            editor.set_content(content, cx);
        });
    }
    fn preview_html(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
        {
            let Some(response) = self
                .response_data
                .as_ref()
                .filter(|r| r.language() == "html")
            else {
                return;
            };
            if self.html_webview_source.as_deref() == Some(response.text.as_str())
                && self.html_webview.is_some()
            {
                self.html_preview = true;
                cx.notify();
                return;
            }
            let text = response.text.clone();
            let builder = gpui_glint::components::wry::WebViewBuilder::new()
                .with_html(&text)
                .with_incognito(true)
                .with_javascript_disabled()
                .with_navigation_handler(|url| url == "about:blank")
                .with_new_window_req_handler(|_, _| {
                    gpui_glint::components::wry::NewWindowResponse::Deny
                })
                .with_download_started_handler(|_, _| false);
            use gpui_glint::components::wry::raw_window_handle::HasWindowHandle;
            let result = window
                .window_handle()
                .map_err(|e| e.to_string())
                .and_then(|handle| {
                    builder
                        .with_visible(false)
                        .build_as_child(&handle)
                        .map_err(|e| e.to_string())
                });
            match result {
                Ok(native) => {
                    self.html_webview_source = Some(text);
                    self.html_webview =
                        Some(cx.new(|cx| gpui_glint::components::WebView::from_native(native, cx)));
                    self.html_preview = true;
                }
                Err(error) => self.notice = format!("无法打开 HTML 预览：{error}"),
            }
        }
        #[cfg(not(all(feature = "webview", any(target_os = "macos", target_os = "windows"))))]
        {
            let _ = window;
            self.notice = "HTML 预览需运行 cargo run --example glint-api --features webview（macOS / Windows）".into();
        }
        cx.notify();
    }
    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.set_theme(
            if self.theme.is_dark() {
                ThemePreset::Moonstone
            } else {
                ThemePreset::DarkObsidian
            },
            cx,
        );
    }
    fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        self.sync_groups(cx);
        self.sync_pair_tables(cx);
        self.response_headers_table
            .update(cx, |table, cx| table.set_theme(theme, cx));
        for entry in [&self.filter, &self.history_search, &self.group_name] {
            entry.update(cx, |e, cx| e.set_theme(theme, cx));
        }
        self.variable_search
            .update(cx, |entry, cx| entry.set_theme(theme, cx));
        for row in self.variable_rows.iter().flatten() {
            row.name.update(cx, |entry, cx| entry.set_theme(theme, cx));
            row.value.update(cx, |entry, cx| entry.set_theme(theme, cx));
            row.kind
                .update(cx, |select, cx| select.set_theme(theme, cx));
        }
        self.sync_variable_table(cx);
        for row in self
            .params
            .iter()
            .chain(self.headers.iter())
            .chain(self.cookies.iter())
        {
            for entry in [&row.key, &row.value, &row.note] {
                entry.update(cx, |e, cx| e.set_theme(theme, cx));
            }
        }
        for editor in [&self.body, &self.response, &self.request_preview] {
            editor.update(cx, |e, cx| e.set_theme(theme, cx));
        }
        self.endpoint.update(cx, |field, cx| {
            field.set_theme(theme, cx);
            field.set_options(
                ["GET", "POST", "PUT", "PATCH", "DELETE"]
                    .map(|s| SelectOption::new(s, s).color(theme.color(method_color(s)))),
                cx,
            );
        });
        self.authentication
            .update(cx, |field, cx| field.set_theme(theme, cx));
        self.environment.update(cx, |e, cx| e.set_theme(theme, cx));
        self.theme_selector.update(cx, |e, cx| {
            e.set_selected(theme, cx);
            e.set_theme(theme, cx);
        });
        self.commands.update(cx, |e, cx| e.set_theme(theme, cx));
        cx.notify();
    }
    fn open_commands(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commands
            .update(cx, |commands, cx| commands.open(window, cx));
    }
    fn topbar(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .pl(px(80.))
            .pr_3()
            .text_size(px(12.))
            .child(Icon::ApiDoc.render(18.).text_color(rgb(0x007aff)))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Glint API"))
            .child(
                div()
                    .text_color(rgb(self.theme.palette().text_muted))
                    .child(if self.show_environments {
                        format!(
                            "个人工作区  ›  环境管理  ›  {}",
                            self.environment_name(self.editing_environment)
                        )
                    } else {
                        self.requests
                            .get(self.active)
                            .map(|request| {
                                format!(
                                    "个人工作区  ›  {}  ›  {}",
                                    request.collection, request.group
                                )
                            })
                            .unwrap_or_else(|| "个人工作区  ›  暂无请求".into())
                    }),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("open-command-search")
                    .w(px(240.))
                    .h(px(30.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(self.theme.palette().border))
                    .bg(rgb(self.theme.palette().panel))
                    .text_color(rgb(self.theme.palette().text_muted))
                    .text_size(px(11.))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(self.theme.palette().surface)))
                    .child(Icon::Search.render(15.).flex_shrink_0())
                    .child(div().flex_1().child("搜索命令与请求"))
                    .child(div().flex_shrink_0().text_size(px(10.)).child(
                        if cfg!(target_os = "macos") {
                            "⌘ K"
                        } else {
                            "Ctrl K"
                        },
                    ))
                    .on_click(cx.listener(|this, _, window, cx| this.open_commands(window, cx))),
            )
            .child(self.theme_selector.clone())
            .when(!self.show_environments, |bar| {
                bar.child(
                    IconButton::new(Icon::Sidebar, "展开或收起记录")
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_activity = !this.show_activity;
                            cx.notify();
                        })),
                )
            })
            .child(
                div()
                    .rounded_full()
                    .bg(rgb(self.theme.palette().accent_tint))
                    .text_color(rgb(0x007aff))
                    .px_2()
                    .py_1()
                    .text_size(px(10.))
                    .child("YG"),
            )
    }
    fn rail(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .py_3()
            .child(
                IconButton::new(Icon::Folders, "请求集合")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_environments = false;
                        this.show_collection_menu = false;
                        cx.notify();
                    })),
            )
            .child(
                IconButton::new(Icon::Globe, "环境变量")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_environments = true;
                        this.editing_environment = this.selected_environment(cx);
                        this.revealed_variables.clear();
                        this.variable_delete = None;
                        this.variable_error = None;
                        this.variable_search
                            .update(cx, |entry, cx| entry.set_value("", cx));
                        this.show_collection_menu = false;
                        cx.notify();
                    })),
            )
            .child(
                IconButton::new(Icon::Clock, "请求历史")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.activity = 1;
                        this.show_activity = true;
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                IconButton::new(Icon::Settings, "切换明暗主题")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_theme(cx))),
            )
    }
    fn selected_environment(&self, cx: &App) -> usize {
        self.environment
            .read(cx)
            .value()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0)
    }
    fn environment_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let applied = self.selected_environment(cx);
        let mut list = div().flex().flex_col().gap_2().px_2().py_3();
        for (index, name) in self.environment_names.iter().enumerate().skip(1) {
            let selected = self.editing_environment == index;
            let base = self.environment_inputs[index]
                .variables
                .iter()
                .find(|variable| variable.name == "base_url")
                .map(|variable| variable.value.as_str())
                .unwrap_or("");
            let host = reqwest::Url::parse(base)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_else(|| "未配置服务地址".into());
            let caption = format!("{} 个变量", self.environment_inputs[index].variables.len());
            list = list.child(
                div()
                    .id(("environment-profile", index))
                    .px_3()
                    .py_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(if selected { p.accent } else { p.panel }))
                    .bg(rgb(if selected { p.accent_tint } else { p.panel }))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(p.accent_tint)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(if index == 2 {
                                Icon::Code.render(15.)
                            } else {
                                Icon::Globe.render(15.)
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(name.clone()),
                            )
                            .when(index == applied, |row| {
                                row.child(
                                    div()
                                        .text_size(px(9.))
                                        .text_color(rgb(p.accent))
                                        .child("使用中"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .mt_2()
                            .font_family("Menlo")
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .truncate()
                            .child(host),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(caption),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.editing_environment = index;
                        this.revealed_variables.clear();
                        this.variable_delete = None;
                        this.variable_error = None;
                        this.variable_search
                            .update(cx, |entry, cx| entry.set_value("", cx));
                        cx.notify();
                    })),
            );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(p.panel))
            .child(
                div()
                    .h(px(44.))
                    .flex_shrink_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("请求环境"))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(format!("{} 个环境", self.environment_names.len() - 1)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .id("environment-list-scroll")
                    .overflow_y_scroll()
                    .child(list)
                    .child(
                        Button::new("创建环境")
                            .icon(Icon::Plus)
                            .kind(ButtonKind::Gray)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .mx_2()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.group_name.update(cx, |e, cx| e.set_value("", cx));
                                this.group_edit = Some(GroupEdit::CreateEnvironment);
                                this.group_error = None;
                                cx.notify();
                            })),
                    )
                    .when(
                        matches!(self.group_edit, Some(GroupEdit::CreateEnvironment)),
                        |el| el.child(self.group_editor(cx)),
                    ),
            )
            .child(
                div()
                    .p_3()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("当前请求使用")
                    .child(
                        div()
                            .mt_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_color(rgb(p.text))
                            .child(div().size(px(6.)).rounded_full().bg(rgb(p.accent)))
                            .child(self.environment_name(applied).to_owned()),
                    ),
            )
    }
    fn sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let mut tree = div()
            .id("request-tree")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .pb_3();
        if self.show_environments {
            return self.environment_sidebar(cx);
        }
        {
            let query = self.filter.read(cx).value().to_lowercase();
            let mut visible_collections = 0;
            for (index, collection) in self.collections.iter().enumerate() {
                let matches_collection = collection.collection.to_lowercase().contains(&query);
                let has_matches = collection
                    .names
                    .iter()
                    .any(|group| group.to_lowercase().contains(&query))
                    || self.requests.iter().any(|request| {
                        request.collection == collection.collection
                            && format!(
                                "{} {} {}",
                                request.name, request.draft.method, request.draft.url
                            )
                            .to_lowercase()
                            .contains(&query)
                    });
                if !query.is_empty() && !matches_collection && !has_matches {
                    continue;
                }
                visible_collections += 1;
                let expanded = self.current_collection == index && self.requests_expanded;
                let count = self
                    .requests
                    .iter()
                    .filter(|request| request.collection == collection.collection)
                    .count();
                tree = tree.child(
                    div()
                        .id(("request-collection-toggle", index))
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .py_3()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(p.accent_tint)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.current_collection == index {
                                this.requests_expanded = !this.requests_expanded;
                            } else {
                                this.current_collection = index;
                                this.requests_expanded = true;
                            }
                            this.group_edit = None;
                            this.sync_groups(cx);
                            cx.notify();
                        }))
                        .font_weight(FontWeight::MEDIUM)
                        .child(if expanded {
                            Icon::ChevronDown.render(12.)
                        } else {
                            Icon::ChevronRight.render(12.)
                        })
                        .child(Icon::Folders.render(14.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .child(collection.collection.clone()),
                        )
                        .child(
                            IconButton::new(Icon::Plus, "新建分组")
                                .theme(self.theme)
                                .render()
                                .size(px(20.))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.current_collection = index;
                                    this.edit_group(None, cx);
                                    this.sync_groups(cx);
                                    window.focus(&this.group_name.read(cx).focus_handle(cx), cx);
                                })),
                        )
                        .child(div().text_color(rgb(p.text_muted)).child(count.to_string())),
                );
                if expanded {
                    tree = tree.child(self.request_list.clone());
                    if collection.names.is_empty() {
                        tree = tree.child(
                            div()
                                .px_3()
                                .pb_3()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child("暂无请求，点击集合旁的 + 创建分组"),
                        );
                    }
                }
            }
            if visible_collections == 0 {
                tree = tree.child(
                    div()
                        .p_3()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child(if self.collections.is_empty() {
                            "暂无集合，点击更多菜单创建集合"
                        } else {
                            "没有匹配的集合、分组或请求"
                        }),
                );
            }
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(p.panel))
            .child(
                div()
                    .flex()
                    .items_center()
                    .px_3()
                    .h(px(44.))
                    .gap_1()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(
                        if self.show_environments {
                            "环境变量"
                        } else {
                            "请求"
                        },
                    ))
                    .child(div().flex_1())
                    .child(
                        IconButton::new(Icon::Plus, "新建请求")
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.add_request(cx))),
                    )
                    .when(!self.show_environments, |header| {
                        header.child(self.collection_menu(cx))
                    }),
            )
            .child(
                div().px_3().pb_2().child(
                    SearchInput::new("筛选集合、分组或请求…")
                        .control(self.filter.clone())
                        .theme(self.theme)
                        .width(196.)
                        .render(),
                ),
            )
            .when(
                self.group_edit.is_some() && !self.show_environments,
                |sidebar| sidebar.child(self.group_editor(cx)),
            )
            .child(tree)
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .p_3()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("SQLite 存储 · 保存时生成版本快照"),
            )
    }
    fn collection_menu(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let mut trigger = div().relative().child(
            IconButton::new(Icon::More, "更多集合操作")
                .theme(self.theme)
                .render()
                .on_click(cx.listener(|this, _, window, cx| {
                    this.show_collection_menu = !this.show_collection_menu;
                    window.focus(&this.focus, cx);
                    cx.notify();
                })),
        );
        if self.show_collection_menu {
            let popup = div()
                .absolute()
                .top_0()
                .right_0()
                .w(px(160.))
                .pt(px(34.))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.show_collection_menu = false;
                    cx.notify();
                }))
                .child(
                    div()
                        .p_1()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.panel))
                        .shadow_md()
                        .child(
                            div()
                                .id("create-request-collection")
                                .h(px(32.))
                                .px_2()
                                .flex()
                                .items_center()
                                .gap_2()
                                .rounded_sm()
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(p.accent_tint)))
                                .child(Icon::Folders.render(14.))
                                .child("创建集合")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.edit_collection(window, cx);
                                })),
                        ),
                );
            trigger = trigger.child(gpui::deferred(popup).with_priority(20));
        }
        trigger
    }
    fn edit_collection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_collection_menu = false;
        self.show_environments = false;
        self.group_name
            .update(cx, |entry, cx| entry.set_value("", cx));
        self.group_edit = Some(GroupEdit::CreateCollection);
        self.group_error = None;
        window.focus(&self.group_name.read(cx).focus_handle(cx), cx);
        cx.notify();
    }
    fn sync_groups(&mut self, cx: &mut Context<Self>) {
        let mut query = self.filter.read(cx).value().to_lowercase();
        let Some(collection) = self.collections.get(self.current_collection) else {
            self.request_list
                .update(cx, |list, cx| list.set_groups(vec![], cx));
            return;
        };
        if collection.collection.to_lowercase().contains(&query) {
            query.clear();
        }
        let groups: Vec<_> = collection
            .names
            .iter()
            .filter_map(|name| {
                let items: Vec<_> = self
                    .requests
                    .iter()
                    .enumerate()
                    .filter(|(_, request)| {
                        &request.group == name
                            && request.collection == collection.collection
                            && (name.to_lowercase().contains(&query)
                                || format!(
                                    "{} {} {}",
                                    request.name, request.draft.method, request.draft.url
                                )
                                .to_lowercase()
                                .contains(&query))
                    })
                    .map(|(i, request)| {
                        GroupListItem::new(i.to_string(), request.name.clone())
                            .action(Icon::Trash)
                            .prefix(request.draft.method.clone())
                            .prefix_color(self.theme.color(method_color(&request.draft.method)))
                    })
                    .collect();
                if !query.is_empty() && items.is_empty() && !name.to_lowercase().contains(&query) {
                    return None;
                }
                let header = GroupListHeader::new(
                    format!("{}:{name}", self.current_collection),
                    name.clone(),
                )
                .actions(if name == "未分组" {
                    vec![Icon::Plus]
                } else {
                    vec![Icon::Plus, Icon::Pencil, Icon::Trash]
                });
                Some(GroupListGroup { header, items })
            })
            .collect();
        self.request_list.update(cx, |list, cx| {
            list.set_theme(self.theme, cx);
            list.set_groups(groups, cx);
            list.set_selected(
                (!self.requests.is_empty()).then(|| self.active.to_string().into()),
                cx,
            );
        });
    }
    fn edit_group(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        self.group_name.update(cx, |entry, cx| {
            entry.set_value(name.clone().unwrap_or_default(), cx)
        });
        self.group_edit = Some(match name {
            Some(name) => GroupEdit::Rename(name),
            None => GroupEdit::Create,
        });
        self.group_error = None;
        self.requests_expanded = true;
        cx.notify();
    }
    fn save_group(&mut self, cx: &mut Context<Self>) {
        let name = self.group_name.read(cx).value().to_owned();
        let result = match &self.group_edit {
            Some(GroupEdit::CreateEnvironment) => {
                let name = name.trim();
                if name.is_empty() {
                    Err("请输入环境名称".into())
                } else if self.environment_names.iter().any(|n| n == name) {
                    Err("环境名称已存在".into())
                } else {
                    self.environment_names.push(name.to_owned());
                    self.environment_inputs
                        .push(EnvironmentVariables::default());
                    self.variable_rows.push(vec![]);
                    self.editing_environment = self.environment_names.len() - 1;
                    self.environment.update(cx, |select, cx| {
                        select.set_options(
                            self.environment_names
                                .iter()
                                .enumerate()
                                .map(|(i, n)| SelectOption::new(i.to_string(), n.clone())),
                            cx,
                        )
                    });
                    self.sync_variable_table(cx);
                    Ok(())
                }
            }
            Some(GroupEdit::CreateCollection) => {
                RequestGroups::create_collection(&name, &self.collections).map(|collection| {
                    self.collections.push(collection);
                    self.current_collection = self.collections.len() - 1;
                    self.requests_expanded = true;
                    self.notice = format!("已创建集合 {}", name.trim());
                })
            }
            Some(GroupEdit::Create) => self.collections[self.current_collection]
                .add(&name)
                .map(|_| ()),
            Some(GroupEdit::Rename(old)) => {
                self.collections[self.current_collection].rename(old, &name, &mut self.requests)
            }
            None => return,
        };
        match result {
            Ok(()) => {
                self.storage_dirty = true;
                self.sync_groups(cx);
                self.group_edit = None;
                self.group_error = None;
                self.filter.update(cx, |entry, cx| entry.set_value("", cx));
            }
            Err(error) => self.group_error = Some(error),
        }
        cx.notify();
    }
    fn group_editor(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        div()
            .mx_3()
            .mb_2()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .flex()
            .flex_col()
            .gap_2()
            .text_size(px(11.))
            .child(match self.group_edit {
                Some(GroupEdit::CreateCollection) => "创建集合",
                Some(GroupEdit::CreateEnvironment) => "创建环境",
                Some(GroupEdit::Create) => "新建分组",
                _ => "重命名分组",
            })
            .child(
                div()
                    .h(px(30.))
                    .px_2()
                    .border_1()
                    .border_color(rgb(p.border))
                    .rounded_md()
                    .child(self.group_name.clone()),
            )
            .when_some(self.group_error.clone(), |editor, error| {
                editor.child(
                    div()
                        .text_color(rgb(self.theme.color(0xd9342b)))
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("取消")
                            .kind(ButtonKind::Gray)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.group_edit = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("确定")
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.save_group(cx))),
                    ),
            )
    }
    fn add_request(&mut self, cx: &mut Context<Self>) {
        self.add_request_to("自定义".into(), cx);
    }
    fn add_request_to(&mut self, group: String, cx: &mut Context<Self>) {
        if self.collections.is_empty() {
            self.group_edit = Some(GroupEdit::CreateCollection);
            self.group_error = None;
            cx.notify();
            return;
        }
        let draft = Draft::default();
        if !self.collections[self.current_collection]
            .names
            .contains(&group)
        {
            let _ = self.collections[self.current_collection].add(&group);
        }
        let i = self.requests.len();
        self.storage_dirty = true;
        self.requests.push(Request {
            name: format!("新建请求 {}", i + 1),
            collection: self.collections[self.current_collection].collection.clone(),
            group,
            draft: draft.clone(),
            versions: vec![Record {
                draft,
                label: "v1".into(),
                time: record_time(),
                detail: "初始版本".into(),
                elapsed_ms: None,
            }],
            history: vec![],
        });
        self.responses.push(None);
        self.traces.push(vec![]);
        self.sent_requests.push(None);
        self.current_versions.push(0);
        if i == 0 {
            self.load(&self.requests[0].draft.clone(), cx);
        }
        self.choose(i, cx);
    }
    fn delete_request(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.requests.len() {
            return;
        }
        self.capture_draft(cx);
        // Any removal changes captured indices in a pending response task.
        if self.sending.take().is_some() {
            self.push_trace("删除请求，已取消等待中的请求", cx);
        }
        self.trace_started = None;
        let removed_active = index == self.active;
        self.storage_dirty = true;
        let name = self.requests.remove(index).name;
        self.responses.remove(index);
        self.traces.remove(index);
        self.sent_requests.remove(index);
        self.current_versions.remove(index);
        self.tabs = self
            .tabs
            .iter()
            .filter_map(|&i| {
                if i == index {
                    None
                } else {
                    Some(i - usize::from(i > index))
                }
            })
            .collect();
        self.selected_history = self
            .selected_history
            .and_then(|(i, record)| (i != index).then(|| (i - usize::from(i > index), record)));
        if self.requests.is_empty() {
            self.active = 0;
        } else if removed_active {
            self.active = self
                .tabs
                .first()
                .copied()
                .unwrap_or(index.min(self.requests.len() - 1));
            if !self.tabs.contains(&self.active) {
                self.tabs.push(self.active);
            }
            self.load(&self.requests[self.active].draft.clone(), cx);
            self.config = 0;
        } else if self.active > index {
            self.active -= 1;
        }
        if removed_active {
            self.restore_response(cx);
            self.error = None;
        }
        self.sync_request_preview(cx);
        self.sync_groups(cx);
        self.notice = format!("已删除请求 {name}");
        cx.notify();
    }
    fn sync_request_preview(&mut self, cx: &mut Context<Self>) {
        let content = self
            .sent_requests
            .get(self.active)
            .and_then(|s| s.as_ref())
            .cloned()
            .unwrap_or_default();
        self.request_preview
            .update(cx, |editor, cx| editor.set_content(content, cx));
    }
    fn request_view(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let content = self.sent_requests.get(self.active).and_then(|s| s.clone());
        let mut view = div().flex_1().min_h_0().flex().flex_col();
        if let Some(content) = content {
            view = view
                .child(
                    div()
                        .h(px(36.))
                        .flex_shrink_0()
                        .px_5()
                        .flex()
                        .items_center()
                        .text_size(px(10.))
                        .text_color(rgb(p.text_muted))
                        .child("最近一次发送的 HTTP 请求 · 后续编辑不会改变此快照")
                        .child(div().flex_1())
                        .child(
                            Button::new("复制请求")
                                .kind(ButtonKind::Plain)
                                .size(ButtonSize::Mini)
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        content.clone(),
                                    ))
                                })),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .px_5()
                        .child(self.request_preview.clone()),
                );
        } else {
            view = view
                .items_center()
                .justify_center()
                .text_color(rgb(p.text_muted))
                .child("发送请求后，在这里查看最终 URL、请求头和请求体");
        }
        view
    }
    fn sync_pair_tables(&mut self, cx: &mut Context<Self>) {
        for kind in [0, 1, 2] {
            let rows = match kind {
                1 => &self.headers,
                2 => &self.cookies,
                _ => &self.params,
            };
            let theme = self.theme;
            let rows: Vec<_> = rows
                .iter()
                .enumerate()
                .map(|(i, row)| {
                    let owner = cx.entity().downgrade();
                    let enabled = row.enabled;
                    let check = TableCell::custom(move || {
                        let owner = owner.clone();
                        Checkbox::new("")
                            .theme(theme)
                            .checked(enabled)
                            .on_change(move |checked, _, cx| {
                                cx.stop_propagation();
                                let _ = owner.update(cx, |this, cx| {
                                    let rows = match kind {
                                        1 => &mut this.headers,
                                        2 => &mut this.cookies,
                                        _ => &mut this.params,
                                    };
                                    rows[i].enabled = checked;
                                    this.sync_pair_tables(cx);
                                    cx.notify();
                                });
                            })
                            .render()
                    });
                    let key = row.key.clone();
                    let value = row.value.clone();
                    let note = row.note.clone();
                    let owner = cx.entity().downgrade();
                    let delete = TableCell::custom(move || {
                        let owner = owner.clone();
                        div()
                            .id(("pair-delete", i))
                            .cursor_pointer()
                            .child(Icon::Close.render(12.))
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                let _ = owner.update(cx, |this, cx| {
                                    let rows = match kind {
                                        1 => &mut this.headers,
                                        2 => &mut this.cookies,
                                        _ => &mut this.params,
                                    };
                                    rows.remove(i);
                                    this.sync_pair_tables(cx);
                                    cx.notify();
                                });
                            })
                    });
                    TableRow::new(
                        i.to_string(),
                        [
                            check,
                            TableCell::custom(move || {
                                div().w_full().font_family("Menlo").child(key.clone())
                            }),
                            TableCell::custom(move || {
                                div().w_full().font_family("Menlo").child(value.clone())
                            }),
                            TableCell::custom(move || div().w_full().child(note.clone())),
                            delete,
                        ],
                    )
                })
                .collect();
            let table = match kind {
                1 => &self.headers_table,
                2 => &self.cookies_table,
                _ => &self.params_table,
            };
            table.update(cx, |table, cx| {
                table.set_theme(theme, cx);
                table.set_rows(rows, cx);
            });
        }
    }
    fn pair_table(&self, headers: bool) -> gpui::Div {
        let p = self.theme.palette();
        let table = div().child(if headers {
            self.headers_table.clone()
        } else {
            self.params_table.clone()
        });
        table.child(
            div()
                .flex()
                .items_center()
                .pt_3()
                .text_size(px(10.))
                .text_color(rgb(p.text_muted))
                .child(if headers {
                    "Authorization 由身份验证配置生成。"
                } else {
                    "发送时仅包含已勾选的参数。"
                }),
        )
    }
    fn configuration(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let view=match self.config {
            0=>self.pair_table(false),
            2=>div().flex().flex_col().gap_3()
                .child(self.pair_table(true))
                .child(div().text_size(px(11.)).font_weight(FontWeight::MEDIUM).child("Cookie 覆盖"))
                .child(self.cookies_table.clone())
                .child(div().text_size(px(10.)).text_color(rgb(p.text_muted)).child("将启用的 Cookie 键值合并到请求头 Cookie；同名键以此表为准，其余原始 Cookie 保留。禁用条目不参与覆盖。")),
            1=>div().flex().flex_col().gap_3()
                .child(div().text_size(px(11.)).child("验证方式 / Token"))
                .child(self.authentication.clone())
                .child(div().text_size(px(10.)).text_color(rgb(p.text_muted)).child("Bearer 使用 Token 或环境变量引用；请填写实际 Token，或在环境变量中设置 api_token。No Auth 不生成 Authorization。")),
            3=>div().h_full().flex().flex_col().gap_2().child(div().flex().items_center().text_size(px(11.)).child("JSON 请求体").child(div().flex_1()).child(Button::new("格式化 JSON").kind(ButtonKind::Plain).size(ButtonSize::Mini).theme(self.theme).render().on_click(cx.listener(|this,_,_,cx| {let valid=this.body.update(cx,|e,cx|e.format_json(cx));this.notice=if valid {"已格式化 JSON"} else {"JSON 格式有误，请检查后重试"}.into();cx.notify();}))))
                .child(div().flex_1().min_h_0().child(self.body.clone())),
            _=>div().p_3().text_size(px(12.)).text_color(rgb(p.text_muted)).child("真实 HTTP / HTTPS 请求。\n连接超时 10 秒，总超时 30 秒；响应内容最多 16 MiB。\n保留 3xx 响应，不自动跟随重定向；TLS 证书正常校验。\n支持 GET / POST / PUT / PATCH / DELETE，参数、请求头与身份验证随版本快照保存。"),
        };
        div()
            .id("configuration")
            .h(px(218.))
            .flex_shrink_0()
            .overflow_y_scroll()
            .px_5()
            .py_3()
            .child(view)
    }
    fn response_panel(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let mut head = div()
            .h(px(42.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_4()
            .px_5()
            .border_b_1()
            .border_color(rgb(p.border))
            .child(div().font_weight(FontWeight::MEDIUM).child("响应"));
        for (i, label) in ["响应体", "响应头", "测试", "轨迹", "请求"]
            .iter()
            .enumerate()
        {
            head = head.child(
                tab_button(label, i == self.response_tab, self.theme).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.response_tab = i;
                        cx.notify();
                    },
                )),
            );
        }
        head = head.child(div().flex_1());
        if let Some(status) = self.status {
            let bytes = self
                .response_data
                .as_ref()
                .map_or(0, |response| response.bytes);
            head = head.child(
                div()
                    .font_family("Menlo")
                    .text_size(px(10.))
                    .text_color(rgb(self.theme.color(if status < 400 {
                        0x24864c
                    } else {
                        0xd9342b
                    })))
                    .child(format!(
                        "● {}   {} ms   {} B",
                        status_label(status),
                        self.requests[self.active]
                            .history
                            .first()
                            .and_then(|r| r.elapsed_ms)
                            .unwrap_or(0),
                        bytes
                    )),
            );
        }
        let content = if self.response_tab == 4 {
            self.request_view(cx)
        } else if self.response_tab == 3 {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(self.trace_view())
        } else if self.sending.is_some() || self.error.is_some() || self.response_data.is_none() {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .text_color(rgb(p.text_muted))
                .child(Icon::Play.render(28.))
                .child(div().text_size(px(13.)).text_color(rgb(p.text)).child(
                    if self.sending.is_some() {
                        "正在等待服务器响应"
                    } else if self.error.is_some() {
                        "无法发送请求"
                    } else {
                        "发送请求，查看响应"
                    },
                ))
                .child(
                    div()
                        .text_size(px(11.))
                        .child(self.error.clone().unwrap_or_else(|| {
                            if self.sending.is_some() {
                                "点击取消可终止本次请求".into()
                            } else {
                                "配置参数后点击发送，或按 ⌘ / Ctrl + Enter".into()
                            }
                        })),
                )
        } else if self.response_tab == 1 {
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(
                    div()
                        .id("response-headers-scroll")
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .overflow_y_scroll()
                        .track_scroll(&self.response_headers_scroll)
                        .px_5()
                        .py_3()
                        .child(self.response_headers_table.clone()),
                )
        } else if self.response_tab == 2 {
            let success = self.status.is_some_and(|s| s < 400);
            div()
                .p_5()
                .text_size(px(12.))
                .text_color(rgb(self.theme.color(if success {
                    0x24864c
                } else {
                    0xd9342b
                })))
                .child(format!(
                    "{} 状态码小于 400\n{} 响应为有效 JSON",
                    if success { "✓" } else { "✕" },
                    if self.response_json.is_some() {
                        "✓"
                    } else {
                        "✕"
                    }
                ))
        } else {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .h(px(36.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_5()
                        .child(
                            tab_button(
                                if self.response_json.is_some() {
                                    "格式化"
                                } else {
                                    "源码"
                                },
                                self.pretty && !self.html_preview,
                                self.theme,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pretty = true;
                                this.update_response(cx);
                                cx.notify();
                            })),
                        )
                        .child(
                            tab_button("原始", !self.pretty && !self.html_preview, self.theme)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pretty = false;
                                    this.update_response(cx);
                                    cx.notify();
                                })),
                        )
                        .when(
                            self.response_data
                                .as_ref()
                                .is_some_and(|r| r.language() == "html"),
                            |el| {
                                el.child(
                                    tab_button("预览", self.html_preview, self.theme).on_click(
                                        cx.listener(|this, _, window, cx| {
                                            this.preview_html(window, cx)
                                        }),
                                    ),
                                )
                            },
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.text_muted))
                                .child(
                                    if self
                                        .response_data
                                        .as_ref()
                                        .is_some_and(|r| r.language() == "html")
                                    {
                                        "HTML 响应"
                                    } else if self.response_json.is_some() {
                                        "JSON 响应"
                                    } else {
                                        "文本响应"
                                    },
                                ),
                        )
                        .child(div().flex_1())
                        .child(
                            IconButton::new(Icon::Copy, "复制响应")
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if this.response_data.is_some() {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            this.response_content(),
                                        ));
                                        this.notice = "已复制响应".into();
                                        cx.notify();
                                    }
                                })),
                        ),
                )
                .child(
                    div()
                        .id("response-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px_3()
                        .pb_3()
                        .child(self.response_view()),
                )
        };
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(head)
            .child(content)
    }
    fn response_view(&self) -> gpui::AnyElement {
        #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
        if self.html_preview {
            if let Some(view) = &self.html_webview {
                return div().size_full().child(view.clone()).into_any_element();
            }
        }
        self.response.clone().into_any_element()
    }
    fn workspace(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        if self.requests.is_empty() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .bg(rgb(p.surface))
                .text_color(rgb(p.text_muted))
                .child("暂无请求")
                .child(
                    Button::new(if self.collections.is_empty() {
                        "创建集合"
                    } else {
                        "新建请求"
                    })
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| this.add_request(cx))),
                );
        }
        let draft = self.snapshot(cx);
        let dirty = self.requests[self.active]
            .versions
            .first()
            .is_none_or(|r| r.draft != draft);
        let select_owner = cx.entity();
        let close_owner = select_owner.clone();
        let add_owner = select_owner.clone();
        let tabs = ClosableTabs::new(self.tabs.iter().map(|&i| {
            let r = &self.requests[i];
            let method = if i == self.active {
                &draft.method
            } else {
                &r.draft.method
            };
            ClosableTab::new(i as u64, format!("{method}  {}", r.name))
                .modified(i == self.active && dirty)
        }))
        .selected(Some(self.active as u64))
        .theme(self.theme)
        .fill_height()
        .content_padding(0.)
        .on_select(move |id, _, app| {
            select_owner.update(app, |this, cx| this.choose(id as usize, cx));
        })
        .on_close(move |id, _, app| {
            close_owner.update(app, |this, cx| {
                let id = id as usize;
                if this.tabs.len() > 1 {
                    this.tabs.retain(|&i| i != id);
                    if this.active == id {
                        this.choose(this.tabs[0], cx);
                    }
                    cx.notify();
                }
            });
        })
        .on_add(move |window, app| {
            add_owner.update(app, |this, cx| this.open_commands(window, cx));
        });
        let top = div()
            .px_5()
            .py_4()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(17.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.requests[self.active].name.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .flex_shrink_0()
                            .gap(px(8.))
                            .child(
                                Button::new("保存")
                                    .icon(Icon::Save)
                                    .kind(ButtonKind::Plain)
                                    .size(ButtonSize::Regular)
                                    .theme(self.theme)
                                    .render()
                                    .text_size(px(12.))
                                    .px(px(10.))
                                    .bg(rgb(p.panel))
                                    .border_color(rgb(p.panel))
                                    .text_color(rgb(p.text_muted))
                                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                            )
                            .child(self.environment.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().flex_1().min_w_0().child(self.endpoint.clone()))
                    .child(
                        Button::new(if self.sending.is_some() {
                            "取消  ×"
                        } else {
                            "发送  ↗"
                        })
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| this.send(cx))),
                    ),
            );
        let mut config_tabs = div()
            .h(px(37.))
            .flex_shrink_0()
            .px_5()
            .flex()
            .items_center()
            .gap_5()
            .border_b_1()
            .border_color(rgb(p.border));
        for (i, label) in ["参数", "身份验证", "请求头", "请求体", "请求设置"]
            .iter()
            .enumerate()
        {
            config_tabs =
                config_tabs.child(tab_button(label, self.config == i, self.theme).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.config = i;
                        cx.notify();
                    }),
                ));
        }
        let content = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(p.surface))
            .child(top)
            .child(config_tabs)
            .child(self.configuration(cx))
            .child(
                div()
                    .h(px(6.))
                    .flex_shrink_0()
                    .border_y_1()
                    .border_color(rgb(p.border))
                    .bg(rgb(p.panel))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().w(px(28.)).h(px(2.)).bg(rgb(p.border)).rounded_full()),
            )
            .child(self.response_panel(cx))
            .child(
                div()
                    .h(px(25.))
                    .flex_shrink_0()
                    .px_4()
                    .flex()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .overflow_hidden()
                    .child(self.notice.clone()),
            );
        tabs.content(self.active as u64, content).render()
    }
    fn activity_panel(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let Some(request) = self.requests.get(self.active) else {
            return div();
        };
        let records = if self.activity == 0 {
            &request.versions
        } else {
            &request.history
        };
        let query = self.history_search.read(cx).value().to_lowercase();
        let mut list = div()
            .id("activity-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_3();
        let mut count = 0;
        let mut previous_date = "";
        let visible_history: Vec<_> = records
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r.draft.url.to_lowercase().contains(&query)
                    && (self.history_filter != 1 || r.label.starts_with('2'))
                    && (self.history_filter != 2 || !r.label.starts_with('2'))
            })
            .collect();
        if self.activity == 1 {
            list = list.child(
                div()
                    .pb_3()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("按日期分组 · 新到旧"),
            );
        }
        for (i, r) in records.iter().enumerate() {
            if self.activity == 1
                && (!r.draft.url.to_lowercase().contains(&query)
                    || (self.history_filter == 1 && !r.label.starts_with('2'))
                    || (self.history_filter == 2 && r.label.starts_with('2')))
            {
                continue;
            }
            count += 1;
            let record = r.clone();
            if self.activity == 1 {
                let date = r.time.split(' ').next().unwrap_or(&r.time);
                if previous_date != date {
                    previous_date = date;
                    let day_count = visible_history
                        .iter()
                        .filter(|(_, r)| r.time.starts_with(date))
                        .count();
                    list = list.child(
                        div()
                            .flex()
                            .items_center()
                            .pt_3()
                            .pb_2()
                            .border_t_1()
                            .border_color(rgb(p.border))
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(date.to_owned())
                            .child(div().flex_1())
                            .child(day_count.to_string()),
                    );
                }
                let status = r
                    .label
                    .split(' ')
                    .next()
                    .and_then(|s| s.parse::<u16>().ok())
                    .unwrap_or(0);
                let color = self.theme.color(if (200..400).contains(&status) {
                    0x24864c
                } else if (400..500).contains(&status) {
                    0xb87508
                } else {
                    0xd9342b
                });
                let selected = self.selected_history == Some((self.active, i));
                list = list.child(
                    div()
                        .id(("history-record", i))
                        .rounded_md()
                        .p_3()
                        .mb_2()
                        .cursor_pointer()
                        .when(selected, |s| s.bg(rgb(p.accent_tint)))
                        .hover(|s| s.bg(rgb(p.accent_tint)))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .px_2()
                                        .py_1()
                                        .rounded_md()
                                        .bg(gpui::rgba((color << 8) | 0x18))
                                        .text_color(rgb(color))
                                        .text_size(px(10.))
                                        .font_family("Menlo")
                                        .child(div().size(px(5.)).rounded_full().bg(rgb(color)))
                                        .child(r.label.clone()),
                                )
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .text_size(px(9.))
                                        .text_color(rgb(p.text_muted))
                                        .child(r.time.split(' ').nth(1).unwrap_or("").to_owned()),
                                ),
                        )
                        .child(
                            div()
                                .mt_2()
                                .flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .font_family("Menlo")
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_size(px(10.))
                                        .text_color(rgb(self
                                            .theme
                                            .color(method_color(&r.draft.method))))
                                        .child(r.draft.method.clone()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .font_family("Menlo")
                                        .text_size(px(10.))
                                        .child(r.draft.url.clone()),
                                ),
                        )
                        .child(
                            div()
                                .mt_2()
                                .text_size(px(9.))
                                .text_color(rgb(p.text_muted))
                                .child(format!(
                                    "{} ms · {}",
                                    r.elapsed_ms.unwrap_or(0),
                                    self.environment_name(r.draft.environment)
                                )),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.sending.take().is_some() {
                                this.push_trace("恢复历史配置，已取消请求", cx);
                            }
                            this.trace_started = None;
                            this.selected_history = Some((this.active, i));
                            this.load(&record.draft, cx);
                            this.notice = format!("已恢复 {} 的请求配置", record.label);
                            this.response_json = None;
                            this.response_data = None;
                            this.status = None;
                            this.error = None;
                            cx.notify();
                        })),
                );
                continue;
            }
            let current = self.current_versions[self.active] == i;
            let params = r
                .draft
                .params
                .iter()
                .filter(|pair| pair.enabled && !pair.key.trim().is_empty())
                .map(|pair| format!("{}={}", pair.key, pair.value))
                .collect::<Vec<_>>()
                .join(" · ");
            let time = if r.time.len() >= 16 && r.time.starts_with("20") {
                r.time[5..16].to_owned()
            } else {
                r.time.clone()
            };
            list = list.child(
                div()
                    .id(("version-record", i))
                    .h(px(92.))
                    .flex_shrink_0()
                    .flex()
                    .gap_2()
                    .px_2()
                    .rounded_lg()
                    .cursor_pointer()
                    .when(current, |s| s.bg(rgb(p.accent_tint)))
                    .hover(|s| s.bg(rgb(p.accent_tint)))
                    .child(
                        div()
                            .relative()
                            .w(px(16.))
                            .h_full()
                            .flex_shrink_0()
                            .child(
                                div()
                                    .absolute()
                                    .left(px(7.))
                                    .top(px(if i == 0 { 26. } else { 0. }))
                                    .bottom(px(if i + 1 == records.len() { 66. } else { 0. }))
                                    .w(px(1.))
                                    .bg(rgb(p.border)),
                            )
                            .when(current, |rail| {
                                rail.child(
                                    div()
                                        .absolute()
                                        .top(px(16.))
                                        .left(px(-2.))
                                        .size(px(20.))
                                        .rounded_full()
                                        .bg(gpui::rgba(0x007aff20)),
                                )
                            })
                            .child(
                                div()
                                    .absolute()
                                    .top(px(20.))
                                    .left(px(2.))
                                    .size(px(12.))
                                    .rounded_full()
                                    .border_2()
                                    .border_color(rgb(if current {
                                        p.accent
                                    } else {
                                        p.text_muted
                                    }))
                                    .bg(rgb(if current { p.accent } else { p.panel })),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .py_3()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_size(px(12.))
                                            .child(r.label.clone()),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py(px(2.))
                                            .rounded_full()
                                            .text_size(px(9.))
                                            .text_color(rgb(self.theme.color(if current {
                                                0x24864c
                                            } else {
                                                0x007aff
                                            })))
                                            .bg(gpui::rgba(if current {
                                                0x24864c18
                                            } else {
                                                0x007aff14
                                            }))
                                            .child(if current { "当前" } else { "基线" }),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        div()
                                            .font_family("Menlo")
                                            .text_size(px(9.))
                                            .text_color(rgb(p.text_muted))
                                            .child(time),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .font_family("Menlo")
                                    .text_size(px(10.))
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(self
                                                .theme
                                                .color(method_color(&r.draft.method))))
                                            .child(r.draft.method.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .child(r.draft.url.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .font_family("Menlo")
                                    .text_size(px(9.))
                                    .text_color(rgb(p.text_muted))
                                    .child(if params.is_empty() {
                                        format!("无查询参数 · {}", r.detail)
                                    } else {
                                        params
                                    }),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.sending.take().is_some() {
                            this.push_trace("恢复版本配置，已取消请求", cx);
                        }
                        this.trace_started = None;
                        this.current_versions[this.active] = i;
                        this.load(&record.draft, cx);
                        this.notice = format!("已恢复 {} 的请求配置", record.label);
                        this.response_json = None;
                        this.response_data = None;
                        this.status = None;
                        this.error = None;
                        cx.notify();
                    })),
            );
        }
        if count == 0 {
            list = list.child(
                div()
                    .p_4()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(if self.activity == 0 {
                        "保存请求以创建版本"
                    } else {
                        "暂无匹配记录，发送请求后在此查看"
                    }),
            );
        }
        let mut panel = div()
            .w(px(270.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(rgb(p.panel))
            .border_l_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .h(px(44.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .child(
                        tab_button(
                            &format!("版本历史 {}", request.versions.len()),
                            self.activity == 0,
                            self.theme,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.activity = 0;
                            cx.notify();
                        })),
                    )
                    .child(
                        tab_button(
                            &format!("请求历史 {}", request.history.len()),
                            self.activity == 1,
                            self.theme,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.activity = 1;
                            cx.notify();
                        })),
                    )
                    .child(div().flex_1())
                    .child(
                        IconButton::new(Icon::Close, "收起记录")
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_activity = false;
                                cx.notify();
                            })),
                    ),
            );
        if self.activity == 1 {
            let mut filters = div().flex().gap_3().pt_2();
            for (i, label) in ["全部", "成功", "失败"].iter().enumerate() {
                filters = filters.child(
                    tab_button(label, self.history_filter == i, self.theme).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.history_filter = i;
                            cx.notify();
                        },
                    )),
                );
            }
            panel = panel.child(
                div()
                    .p_3()
                    .child(
                        SearchInput::new("搜索历史路径…")
                            .width(240.)
                            .control(self.history_search.clone())
                            .theme(self.theme)
                            .render(),
                    )
                    .child(filters),
            );
        } else {
            panel = panel.child(
                div()
                    .p_3()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("提交记录 · 新到旧"),
            );
        }
        panel.child(list).child(
            div()
                .p_3()
                .border_t_1()
                .border_color(rgb(p.border))
                .text_size(px(10.))
                .text_color(rgb(p.text_muted))
                .child(format!("共 {count} 条记录")),
        )
    }
}
impl Render for Workbench {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
        if let Some(view) = &self.html_webview {
            let visible = self.html_preview
                && self.sending.is_none()
                && self.error.is_none()
                && self.response_data.is_some()
                && !self.show_environments
                && self.response_tab == 0
                && !self.commands.read(cx).is_open();
            if let Err(error) = view.update(cx, |view, cx| view.set_visible(visible, cx)) {
                self.notice = format!("WebView 显隐失败：{error}");
            }
        }
        let _ = &self.subscriptions;
        let content = div()
            .size_full()
            .flex()
            .child(if self.show_environments {
                self.environment_workspace(cx)
            } else {
                self.workspace(cx)
            })
            .when(self.show_activity && !self.show_environments, |s| {
                s.child(self.activity_panel(cx))
            });
        let content = div()
            .size_full()
            .flex()
            .flex_col()
            .when_some(self.storage_error.clone(), |el, error| {
                el.child(
                    div()
                        .p_2()
                        .text_color(rgb(0xd9342b))
                        .child(format!("SQLite 保存失败：{error}。修改尚未写入磁盘。")),
                )
            })
            .child(content);
        let shell = AppLayout::new(self.topbar(cx), self.sidebar(cx), content)
            .navigation_rail(self.rail(cx))
            .navigation_rail_width(46.)
            .sidebar_width(220.)
            .theme(self.theme);
        let overlay = self
            .commands
            .update(cx, |commands, cx| commands.overlay(cx));
        div()
            .key_context("ApiWorkbench")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .text_size(px(12.))
            .on_action(cx.listener(|this, _: &SendRequest, _, cx| this.send(cx)))
            .on_action(cx.listener(|this, _: &SaveRequest, _, cx| {
                if this.show_environments {
                    this.save_environment_variables(cx);
                } else {
                    this.save(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SearchCommands, w, cx| this.open_commands(w, cx)))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if this.show_collection_menu {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            this.show_collection_menu = false;
                            cx.stop_propagation();
                            cx.notify();
                        }
                        "enter" => {
                            this.edit_collection(window, cx);
                            cx.stop_propagation();
                        }
                        _ => {}
                    }
                }
            }))
            .child(shell)
            .children(overlay)
    }
}
fn tab_button(label: &str, selected: bool, theme: ThemePreset) -> gpui::Stateful<gpui::Div> {
    div()
        .id(label.to_owned())
        .h_full()
        .flex()
        .items_center()
        .text_size(px(11.))
        .cursor_pointer()
        .text_color(rgb(if selected {
            theme.palette().text
        } else {
            theme.palette().text_muted
        }))
        .when(selected, |s| s.border_b_2().border_color(rgb(0x007aff)))
        .hover(|s| s.text_color(rgb(0x007aff)))
        .child(label.to_owned())
}
fn method_color(method: &str) -> u32 {
    match method {
        "POST" => 0xb37716,
        "DELETE" => 0xd9342b,
        "PATCH" | "PUT" => 0x876ab4,
        _ => 0x24864c,
    }
}
fn status_label(status: u16) -> String {
    let reason = reqwest::StatusCode::from_u16(status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or("");
    format!("{status} {reason}").trim_end().to_string()
}
fn record_time() -> String {
    chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}
fn main() {
    gpui_platform::application()
        .with_assets(GlintAssets)
        .run(|cx: &mut App| {
            bind_text_entry_keys(cx);
            bind_code_card_keys(cx);
            cx.bind_keys([
                KeyBinding::new(
                    "cmd-enter",
                    SendRequest,
                    Some("ApiWorkbench > GlassCodeCard"),
                ),
                KeyBinding::new(
                    "ctrl-enter",
                    SendRequest,
                    Some("ApiWorkbench > GlassCodeCard"),
                ),
                KeyBinding::new("cmd-enter", SendRequest, Some("ApiWorkbench")),
                KeyBinding::new("ctrl-enter", SendRequest, Some("ApiWorkbench")),
                KeyBinding::new("cmd-s", SaveRequest, Some("ApiWorkbench")),
                KeyBinding::new("ctrl-s", SaveRequest, Some("ApiWorkbench")),
                KeyBinding::new("cmd-k", SearchCommands, Some("ApiWorkbench")),
                KeyBinding::new("ctrl-k", SearchCommands, Some("ApiWorkbench")),
            ]);
            let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(1050.), px(700.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Glint API".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(16.), px(14.))),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let app = cx.new(Workbench::new);
                    let focus = app.read(cx).focus.clone();
                    window.focus(&focus, cx);
                    app
                },
            )
            .expect("无法打开 API 工作台窗口");
            cx.activate(true);
        });
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[gpui::test]
    fn many_response_headers_scroll_within_the_panel(cx: &mut gpui::TestAppContext) {
        struct ResponsePanel(Entity<Workbench>);
        impl Render for ResponsePanel {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .w(px(480.))
                    .h(px(240.))
                    .flex()
                    .flex_col()
                    .child(self.0.update(cx, |host, cx| host.response_panel(cx)))
                    .child(div().h(px(25.)).flex_shrink_0().child("状态栏"))
            }
        }
        let path = std::env::temp_dir().join(format!(
            "glint-api-headers-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let host = cx.new(|cx| {
            let store = storage::Store::open(&path).unwrap();
            let mut host = Workbench::with_store(store, cx);
            host.add_request(cx);
            host.response_tab = 1;
            host.response_data = Some(HttpResponse {
                status: 200,
                headers: (0..60)
                    .map(|index| {
                        (
                            "set-cookie".into(),
                            format!("cookie-{index}={}", "x".repeat(600)),
                        )
                    })
                    .collect(),
                text: String::new(),
                bytes: 0,
                json: None,
            });
            host.update_response(cx);
            host
        });
        let (_, window) = cx.add_window_view(|_, _| ResponsePanel(host.clone()));
        let scroll = host.update(window, |host, _| host.response_headers_scroll.clone());
        assert!(scroll.bounds().bottom() <= px(215.));
        assert!(scroll.bounds().size.width <= px(480.));
        // Every repeated header remains a row, well beyond the viewport.
        assert!(scroll.max_offset().y > px(1800.));
        window.simulate_event(gpui::ScrollWheelEvent {
            position: scroll.bounds().center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-200.))),
            ..Default::default()
        });
        assert!(scroll.offset().y < px(0.));
        std::fs::remove_file(path).unwrap();
    }

    #[gpui::test]
    fn edits_and_environment_save_survive_workbench_restart(cx: &mut gpui::TestAppContext) {
        let path = std::env::temp_dir().join(format!(
            "glint-api-ui-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let store = storage::Store::open(&path).unwrap();
        let host = cx.new(|cx| Workbench::with_store(store, cx));
        host.update(cx, |host, cx| {
            assert!(host.requests.is_empty() && host.collections.is_empty());
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |e, cx| e.set_value("My API", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.endpoint
                .update(cx, |e, cx| e.set_text("https://example.test/edited", cx));
            host.group_edit = Some(GroupEdit::CreateEnvironment);
            host.group_name
                .update(cx, |e, cx| e.set_value("My environment", cx));
            host.save_group(cx);
            host.add_variable(cx);
            host.variable_rows[1][0]
                .name
                .update(cx, |e, cx| e.set_value("base_url", cx));
            host.variable_rows[1][0]
                .value
                .update(cx, |e, cx| e.set_value("https://example.test", cx));
            host.persist(cx);
            assert!(
                storage::Store::open(&path)
                    .unwrap()
                    .load()
                    .unwrap()
                    .variables[1]
                    .variables
                    .is_empty()
            );
            host.save_environment_variables(cx);
            cx.notify();
        });
        cx.run_until_parked();
        let serializations = host.update(cx, |host, _| host.storage.serializations);
        host.update(cx, |host, cx| {
            host.response_tab = 1;
            host.show_activity = false;
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(
            host.update(cx, |host, _| host.storage.serializations),
            serializations,
            "UI tab changes must not serialize the workspace"
        );
        let saved = storage::Store::open(&path).unwrap().load().unwrap();
        assert_eq!(saved.requests[0].draft.url, "https://example.test/edited");
        assert_eq!(saved.environments[1], "My environment");
        assert_eq!(
            saved.variables[1].variables[0].value,
            "https://example.test"
        );
        let store = storage::Store::open(&path).unwrap();
        let reopened = cx.new(|cx| Workbench::with_store(store, cx));
        reopened.update(cx, |host, cx| {
            assert_eq!(
                host.endpoint.read(cx).value().text.as_ref(),
                "https://example.test/edited"
            );
            host.delete_request(0, cx);
        });
        cx.run_until_parked();
        assert!(
            storage::Store::open(&path)
                .unwrap()
                .load()
                .unwrap()
                .requests
                .is_empty()
        );
        drop(host);
        drop(reopened);
        std::fs::remove_file(path).unwrap();
    }
}
