#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app_icon;
mod environment;
mod environment_ui;
mod http;
mod model;
mod plugin_ui;
mod plugins;
mod registry_ui;
mod storage;
mod template;
mod update_ui;
mod updater;

use environment::EnvironmentVariables;
use environment_ui::{ProxyInput, VariableInput};
use gpui::{
    App, AppContext, Bounds, ClipboardItem, Context, Entity, FocusHandle, Focusable, FontWeight,
    KeyBinding, Subscription, Task, TitlebarOptions, Window, WindowBounds, WindowOptions, div,
    point, prelude::*, px, rgb, size,
};
use gpui_glint::{
    components::{
        AppLayout, Button, ButtonKind, ButtonSize, Checkbox, ClosableTab, ClosableTabs, CodeCard,
        CommandItem, CommandPalette, CommandPaletteEvent, DataTable, GlintAssets, Icon, IconButton,
        Popover, SearchInput, Select, SelectEvent, SelectField, SelectFieldEvent, SelectOption,
        Separator, Switch, TableCell, TableColumn, TableEvent, TableRow, TextEntry,
        TextReferenceResolver, ThemeSelector, ThemeSelectorEvent, TreeDrop, TreeListItem,
        bind_code_card_keys, bind_text_entry_keys,
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
    Create(String),
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
    update_state: update_ui::UpdateState,
    update_task: Option<Task<()>>,
    show_update_details: bool,
    update_error: Option<String>,
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
    current_directory: String,
    collapsed_directories: HashSet<(usize, String)>,
    hovered_tree_row: Option<String>,
    tree_drop: Option<(model::TreeNode, model::Placement)>,
    tree_menu: Option<(usize, String)>,
    group_edit: Option<GroupEdit>,
    group_name: Entity<TextEntry>,
    group_error: Option<String>,
    request_name: Entity<TextEntry>,
    editing_request_name: bool,
    request_name_error: Option<String>,
    active: usize,
    tabs: Vec<usize>,
    config: usize,
    plugin_catalog: Vec<gpui_glint::plugins::Plugin>,
    plugin_errors: Vec<String>,
    registry_name: Entity<TextEntry>,
    registry_url: Entity<TextEntry>,
    registry_key: Entity<TextEntry>,
    registry_filter: Entity<TextEntry>,
    registry_sources: Vec<glint_plugin_registry::Source>,
    registry_releases: Vec<(
        glint_plugin_registry::Source,
        glint_plugin_registry::Release,
    )>,
    registry_task: Option<Task<()>>,
    registry_message: Option<String>,
    show_registry_dialog: bool,
    registry_adding_source: bool,
    response_tab: usize,
    traces: Vec<Vec<TraceEntry>>,
    sent_requests: Vec<Option<String>>,
    request_preview: Entity<CodeCard>,
    trace_started: Option<Instant>,
    activity: usize,
    show_activity: bool,
    show_environments: bool,
    show_plugins: bool,
    plugin_manager_tab: usize,
    editing_environment: usize,
    revealed_variables: HashSet<(usize, u64)>,
    variable_delete: Option<(usize, u64)>,
    variable_search: Entity<TextEntry>,
    variable_error: Option<String>,
    variable_rows: Vec<Vec<VariableInput>>,
    proxy_inputs: Vec<ProxyInput>,
    variable_table: Entity<DataTable>,
    next_variable_row: u64,
    history_filter: usize,
    selected_history: Option<(usize, usize)>,
    current_versions: Vec<usize>,
    filter: Entity<TextEntry>,
    history_search: Entity<TextEntry>,
    authentication: Entity<SelectField>,
    skip_tls_verification: bool,
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
    response_header_entries: Vec<[Entity<TextEntry>; 2]>,
    cookies_table: Entity<DataTable>,
    body: Entity<CodeCard>,
    response: Entity<CodeCard>,
    response_search: Entity<TextEntry>,
    show_response_search: bool,
    response_search_index: Option<usize>,
    response_search_count: usize,
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
        let mut this = Self::with_store(storage, cx);
        let report = updater::take_report();
        if let Some(report) = report {
            this.show_update_details = true;
            this.update_state = if report.success && report.version == updater::VERSION {
                update_ui::UpdateState::Current
            } else {
                update_ui::UpdateState::Failed(if report.success {
                    "安装后版本与目标版本不符".into()
                } else {
                    report.message
                })
            };
        } else if cfg!(target_os = "windows") && !cfg!(debug_assertions) {
            this.check_for_updates(false, cx);
        }
        this
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
        let group_name = Self::entry("", "名称", cx);
        let request_name = Self::entry("", "请求名称", cx);
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
            .select_width(112., cx)
        });
        let filter = Self::entry("", "筛选集合、目录或请求…", cx);
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
        let commands = cx.new(|cx| CommandPalette::new(Self::command_items(&requests), cx));
        let variable_search = Self::entry("", "搜索变量名称…", cx);
        let response_search = Self::entry("", "搜索响应体…", cx);
        let mut subscriptions = vec![];
        subscriptions.push(cx.observe(&response_search, |this: &mut Self, _, cx| {
            this.sync_response_search(false, cx);
        }));
        subscriptions.push(cx.observe(&variable_search, |this: &mut Self, _, cx| {
            this.sync_variable_table(cx);
            cx.notify();
        }));
        subscriptions.push(cx.observe(&filter, |this, _, cx| {
            this.sync_groups(cx);
            cx.notify();
        }));
        for entry in [&history_search, &group_name, &request_name] {
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
        let params_table = cx.new(|cx| make_table(cx).empty_row("暂无请求参数。"));
        let headers_table = cx.new(|cx| make_table(cx).empty_row("暂无请求头。"));
        let cookies_table = cx.new(|cx| {
            make_table(cx)
                .empty_row("暂无 Cookie 字段覆盖。")
                .add_row("添加 Cookie 字段覆盖")
        });
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
            .empty_row("暂无响应头。")
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
            .empty_row("暂无环境变量。")
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
        let (plugin_catalog, plugin_errors) = plugins::catalog();
        let registry_name = Self::entry("", "源名称，如 company", cx);
        let registry_url = Self::entry("", "https://plugins.example.com", cx);
        let registry_key = Self::entry("", "服务公钥（Base64，需从可信渠道确认）", cx);
        let registry_filter = Self::entry("", "筛选远程插件：名称、ID、源或版本", cx);
        subscriptions.push(cx.observe(&registry_filter, |_, _, cx| cx.notify()));
        let registry_sources = plugins::registry_store()
            .and_then(|store| store.sources())
            .unwrap_or_default();
        let mut this = Self {
            update_state: update_ui::UpdateState::Idle,
            update_task: None,
            show_update_details: false,
            update_error: None,
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
            current_directory: String::new(),
            collapsed_directories: HashSet::new(),
            hovered_tree_row: None,
            tree_drop: None,
            tree_menu: None,
            group_name,
            group_edit: None,
            group_error: None,
            request_name,
            editing_request_name: false,
            request_name_error: None,
            active: 0,
            tabs,
            config: 0,
            plugin_catalog,
            plugin_errors,
            registry_name,
            registry_url,
            registry_key,
            registry_filter,
            registry_sources,
            registry_releases: vec![],
            registry_task: None,
            registry_message: None,
            show_registry_dialog: false,
            registry_adding_source: false,
            response_tab: 0,
            traces,
            sent_requests,
            request_preview,
            trace_started: None,
            activity: 0,
            show_activity: true,
            show_environments: false,
            show_plugins: false,
            plugin_manager_tab: 0,
            editing_environment: 0,
            revealed_variables: HashSet::new(),
            variable_delete: None,
            variable_search,
            variable_error: None,
            variable_rows: vec![vec![]; environment_count],
            proxy_inputs: vec![],
            variable_table,
            next_variable_row: 0,
            history_filter: 0,
            selected_history: None,
            current_versions,
            filter,
            history_search,
            authentication,
            skip_tls_verification: false,
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
            response_header_entries: vec![],
            cookies_table,
            body,
            response,
            response_search,
            show_response_search: false,
            response_search_index: None,
            response_search_count: 0,
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
    fn command_items(requests: &[Request]) -> Vec<CommandItem> {
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
            ])
            .collect()
    }
    fn sync_commands(&mut self, cx: &mut Context<Self>) {
        let items = Self::command_items(&self.requests);
        self.commands
            .update(cx, |commands, cx| commands.set_items(items, cx));
    }
    fn edit_request_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.requests.get(self.active) else {
            return;
        };
        let name = request.name.clone();
        self.request_name
            .update(cx, |entry, cx| entry.set_value(name, cx));
        self.editing_request_name = true;
        self.request_name_error = None;
        window.focus(&self.request_name.read(cx).focus_handle(cx), cx);
        cx.notify();
    }
    fn cancel_request_name(&mut self, cx: &mut Context<Self>) {
        self.editing_request_name = false;
        self.request_name_error = None;
        cx.notify();
    }
    fn save_request_name(&mut self, cx: &mut Context<Self>) {
        if !self.editing_request_name {
            return;
        }
        let name = self.request_name.read(cx).value().trim().to_owned();
        if name.is_empty() {
            self.request_name_error = Some("请输入请求名称".into());
            cx.notify();
            return;
        }
        if let Some(request) = self.requests.get_mut(self.active) {
            if request.name != name {
                request.name = name;
                self.storage_dirty = true;
            }
        }
        self.cancel_request_name(cx);
        self.sync_groups(cx);
        self.sync_commands(cx);
    }
    fn request_name_view(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let p = self.theme.palette();
        if self.editing_request_name {
            return div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "enter" => {
                            this.save_request_name(cx);
                            cx.stop_propagation();
                        }
                        "escape" => {
                            this.cancel_request_name(cx);
                            cx.stop_propagation();
                        }
                        _ => return,
                    }
                    if !this.editing_request_name {
                        window.focus(&this.focus, cx);
                    }
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h(px(30.))
                                .px_2()
                                .rounded_md()
                                .border_1()
                                .border_color(rgb(p.accent))
                                .child(self.request_name.clone()),
                        )
                        .child(
                            IconButton::new(Icon::CheckCircle, "保存请求名称")
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_request_name(cx);
                                    if !this.editing_request_name {
                                        window.focus(&this.focus, cx);
                                    }
                                })),
                        )
                        .child(
                            IconButton::new(Icon::Close, "取消重命名")
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.cancel_request_name(cx);
                                    window.focus(&this.focus, cx);
                                })),
                        ),
                )
                .when_some(self.request_name_error.clone(), |view, error| {
                    view.child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(0xd9342b))
                            .child(error),
                    )
                })
                .into_any_element();
        }
        div()
            .id("edit-request-name")
            .group("request-name")
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .hover(|style| style.text_color(rgb(p.accent)))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(17.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.requests[self.active].name.clone()),
            )
            .child(
                Icon::Pencil
                    .render(14.)
                    .flex_shrink_0()
                    .opacity(0.)
                    .group_hover("request-name", |style| style.opacity(1.)),
            )
            .on_click(cx.listener(|this, _, window, cx| this.edit_request_name(window, cx)))
            .into_any_element()
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
            plugins: self
                .requests
                .get(self.active)
                .map(|r| r.draft.plugins.clone())
                .unwrap_or_default(),
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
            skip_tls_verification: self.skip_tls_verification,
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
        let resolver = template::TemplateResolver::for_request(&values, &self.snapshot(cx));
        std::rc::Rc::new(move |expression| match &resolver {
            Ok(resolver) => resolver.resolve(expression),
            Err(error) => Err(error.clone()),
        })
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
            let mut previous = entry.read(cx).value().to_string();
            self.row_subscriptions
                .push(cx.observe(entry, move |this, entry, cx| {
                    let current = entry.read(cx).value().to_string();
                    if current != previous {
                        previous = current;
                        this.sync_reference_inputs(cx);
                    }
                    cx.notify();
                }));
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
        if let Some(request) = self.requests.get_mut(self.active) {
            request.draft.plugins = draft.plugins.clone();
        }
        self.skip_tls_verification = draft.skip_tls_verification;
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
        self.show_plugins = false;
        self.show_environments = false;
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
        self.tree_menu = None;
        self.cancel_request_name(cx);
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
            self.current_directory = self.requests[index].group.clone();
            let mut path = self.current_directory.as_str();
            loop {
                self.collapsed_directories
                    .remove(&(collection, path.to_owned()));
                if path.is_empty() {
                    break;
                }
                path = RequestGroups::parent(path);
            }
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
        if request.draft != draft {
            request.draft = draft.clone();
            self.storage_dirty = true;
        }
        if request
            .versions
            .first()
            .is_some_and(|record| record.draft == draft)
        {
            self.current_versions[self.active] = 0;
            self.notice = "已保存，内容未修改".into();
            cx.notify();
            return;
        }
        self.storage_dirty = true;
        request.versions.insert(
            0,
            Record {
                draft,
                label: format!("v{}", request.versions.len() + 1),
                time: record_time(),
                detail: "保存请求时生成的快照".into(),
                elapsed_ms: None,
                exchange: None,
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
        let request = match PreparedRequest::new(&draft, &variables)
            .and_then(|request| request.with_proxy(&inputs.proxy))
        {
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
        let plugin_ids = draft.plugins.clone();
        let plugin_catalog = self.plugin_catalog.clone();
        let plugin_variables = variables.clone();
        self.sending = Some(cx.spawn(async move |this, cx| {
            let prepared = cx
                .background_executor()
                .spawn(async move {
                    plugins::apply(request, &plugin_ids, &plugin_catalog, &plugin_variables)
                })
                .await;
            let result = match prepared {
                Ok(request) => {
                    let _ = this.update(cx, |this, cx| {
                        this.sent_requests[index] = Some(request.preview.clone());
                        if this.active == index {
                            this.sync_request_preview(cx);
                        }
                        cx.notify();
                    });
                    http::execute(request).await
                }
                Err(error) => Err(error),
            };
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
                        exchange: Some(model::Exchange {
                            response: result.as_ref().ok().cloned(),
                            error: result.as_ref().err().cloned(),
                            preview: this.sent_requests[index].clone(),
                            traces: vec![],
                        }),
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
                        this.responses[index] = None;
                        this.response_data = None;
                        this.response_json = None;
                        this.status = None;
                        this.update_response(cx);
                        this.notice = "请求失败".into();
                    }
                }
                this.requests[index].history[0]
                    .exchange
                    .as_mut()
                    .unwrap()
                    .traces = this.traces[index].clone();
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
    fn sync_response_search(&mut self, force: bool, cx: &mut Context<Self>) {
        let query = if self.show_response_search {
            self.response_search.read(cx).value().to_owned()
        } else {
            String::new()
        };
        if !force && self.response.read(cx).search_query() == query {
            return;
        }
        self.response_search_count = self.response.update(cx, |editor, cx| {
            editor.set_search_query(query, cx);
            let count = editor.search_match_count();
            if count > 0 {
                editor.reveal_search_match(0, cx);
            }
            count
        });
        self.response_search_index = (self.response_search_count > 0).then_some(0);
        cx.notify();
    }
    fn step_response_search(&mut self, previous: bool, cx: &mut Context<Self>) {
        let count = self.response_search_count;
        if count == 0 {
            return;
        }
        let current = self.response_search_index.unwrap_or(0);
        let index = if previous {
            (current + count - 1) % count
        } else {
            (current + 1) % count
        };
        self.response_search_index = Some(index);
        self.response.update(cx, |editor, cx| {
            editor.reveal_search_match(index, cx);
        });
        cx.notify();
    }
    fn toggle_response_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_response_search = !self.show_response_search;
        if self.show_response_search {
            self.html_preview = false;
            window.focus(&self.response_search.focus_handle(cx), cx);
        } else {
            window.focus(&self.focus, cx);
        }
        self.sync_response_search(true, cx);
    }
    fn update_response(&mut self, cx: &mut Context<Self>) {
        let headers = self
            .response_data
            .as_ref()
            .map(|response| response.headers.clone())
            .unwrap_or_default();
        self.response_header_entries = headers
            .iter()
            .map(|(key, value)| {
                [key, value].map(|text| {
                    cx.new(|cx| {
                        TextEntry::new(cx, "", text.clone())
                            .read_only(true)
                            .fit_content()
                            .font_size(11.)
                            .theme(self.theme)
                    })
                })
            })
            .collect();
        let rows = self
            .response_header_entries
            .iter()
            .enumerate()
            .map(|(index, entries)| {
                TableRow::new(
                    format!("response-header-{index}"),
                    entries.iter().enumerate().map(|(column, entry)| {
                        let entry = entry.clone();
                        TableCell::custom(move || {
                            div()
                                .id(("response-header-cell", index * 2 + column))
                                .debug_selector(move || {
                                    format!("response-header-{index}-{column}").into()
                                })
                                .w_full()
                                .min_w_0()
                                .h(px(26.))
                                .overflow_x_scroll()
                                .font_family("Menlo")
                                .text_size(px(11.))
                                .child(entry.clone())
                        })
                    }),
                )
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
        self.sync_response_search(true, cx);
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
                self.show_response_search = false;
                self.sync_response_search(true, cx);
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
            self.notice =
                "HTML 预览需运行 cargo run -p glint-api --features webview（macOS / Windows）"
                    .into();
        }
        if self.html_preview {
            self.show_response_search = false;
            self.sync_response_search(true, cx);
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
        for entry in self.response_header_entries.iter().flatten() {
            entry.update(cx, |entry, cx| entry.set_theme(theme, cx));
        }
        for entry in [
            &self.filter,
            &self.history_search,
            &self.response_search,
            &self.group_name,
            &self.request_name,
        ] {
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
        for proxy in &self.proxy_inputs {
            proxy.url.update(cx, |entry, cx| entry.set_theme(theme, cx));
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
        self.sync_commands(cx);
        self.commands
            .update(cx, |commands, cx| commands.open(window, cx));
    }
    fn topbar(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .size_full()
            .flex()
            .items_center()
            .gap_3()
            .pl(px(if cfg!(target_os = "macos") { 80. } else { 16. }))
            .pr_3()
            .text_size(px(12.))
            .child(Icon::ApiDoc.render(18.).text_color(rgb(0x007aff)))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Glint API"))
            .child(
                div()
                    .text_color(rgb(self.theme.palette().text_muted))
                    .child(if self.show_plugins {
                        "个人工作区  ›  插件管理".into()
                    } else if self.show_environments {
                        format!(
                            "个人工作区  ›  环境管理  ›  {}",
                            self.environment_name(self.editing_environment)
                        )
                    } else {
                        self.requests
                            .get(self.active)
                            .map(|request| {
                                if request.group.is_empty() {
                                    format!("个人工作区  ›  {}", request.collection)
                                } else {
                                    format!(
                                        "个人工作区  ›  {}  ›  {}",
                                        request.collection,
                                        request.group.replace('/', "  ›  ")
                                    )
                                }
                            })
                            .unwrap_or_else(|| "个人工作区  ›  暂无请求".into())
                    }),
            )
            .child(gpui_glint::components::window_drag_region())
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
            .child(self.update_button(cx))
            .when(!self.show_environments && !self.show_plugins, |bar| {
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
                        this.show_plugins = false;
                        this.show_collection_menu = false;
                        cx.notify();
                    })),
            )
            .child(
                IconButton::new(Icon::Globe, "环境变量")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_plugins = false;
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
                IconButton::new(Icon::Script, "插件管理")
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "open-plugin-manager".into())
                    .on_click(cx.listener(|this, _, _, cx| this.open_plugin_manager(cx))),
            )
            .child(
                IconButton::new(Icon::Clock, "请求历史")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_plugins = false;
                        this.show_environments = false;
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
        if self.show_plugins {
            return self.plugin_sidebar(cx);
        }
        let p = self.theme.palette();
        let mut tree = div()
            .id("request-tree")
            .flex_1()
            .min_h_0()
            .overflow_scroll()
            .px_2()
            .pb_3();
        if self.show_environments {
            return self.environment_sidebar(cx);
        }
        let list = model::tree_list(
            &self.collections,
            &self.requests,
            &self.collapsed_directories,
            self.filter.read(cx).value(),
        );
        if list.visible_rows().is_empty() {
            tree = tree.child(
                div()
                    .p_3()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(if self.collections.is_empty() {
                        "先创建一个集合，再添加请求或目录"
                    } else {
                        "没有匹配的集合、目录或请求"
                    }),
            );
        }
        tree = tree.child(list.render_with(|row| self.tree_row(model::tree_entry(row), cx)));
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
                    SearchInput::new("筛选集合、目录或请求…")
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
        self.tree_menu = None;
        self.show_environments = false;
        self.show_plugins = false;
        self.group_name
            .update(cx, |entry, cx| entry.set_value("", cx));
        self.group_edit = Some(GroupEdit::CreateCollection);
        self.group_error = None;
        window.focus(&self.group_name.read(cx).focus_handle(cx), cx);
        cx.notify();
    }
    fn sync_groups(&mut self, cx: &mut Context<Self>) {
        // Tree rows are derived directly from collections and requests at render time.
        if !self
            .collections
            .get(self.current_collection)
            .is_some_and(|c| c.contains(&self.current_directory))
        {
            self.current_directory.clear();
        }
        cx.notify();
    }
    fn edit_directory(
        &mut self,
        collection: usize,
        parent: String,
        name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.current_collection = collection;
        self.current_directory = parent.clone();
        self.group_name.update(cx, |entry, cx| {
            entry.set_value(
                name.as_deref()
                    .map(|path| path.rsplit('/').next().unwrap_or(path))
                    .unwrap_or(""),
                cx,
            )
        });
        self.group_edit = Some(match name {
            Some(path) => GroupEdit::Rename(path),
            None => GroupEdit::Create(parent),
        });
        self.group_error = None;
        window.focus(&self.group_name.read(cx).focus_handle(cx), cx);
        cx.notify();
    }
    fn drop_tree_node(
        &mut self,
        source: &model::TreeNode,
        target: &model::TreeNode,
        placement: model::Placement,
        cx: &mut Context<Self>,
    ) {
        use model::{Placement, TreeNode};
        self.capture_draft(cx);
        let old_names = self
            .collections
            .iter()
            .map(|c| c.collection.clone())
            .collect::<Vec<_>>();
        let selected_name = old_names.get(self.current_collection).cloned();
        let moved = if let (
            TreeNode::Directory {
                collection: from,
                path,
            },
            TreeNode::Directory {
                collection: to,
                path: target_path,
            },
        ) = (source, target)
        {
            if path.is_empty() && target_path.is_empty() && placement != Placement::Inside {
                let name = old_names[*from].clone();
                let destination = old_names[*to].clone();
                if from != to {
                    let collection = self.collections.remove(*from);
                    let position = self
                        .collections
                        .iter()
                        .position(|c| c.collection == destination)
                        .unwrap()
                        + usize::from(placement == Placement::After);
                    self.collections.insert(position, collection);
                }
                Ok(TreeNode::Directory {
                    collection: self
                        .collections
                        .iter()
                        .position(|c| c.collection == name)
                        .unwrap(),
                    path: String::new(),
                })
            } else {
                model::move_node(
                    &mut self.collections,
                    &mut self.requests,
                    source,
                    target,
                    placement,
                )
            }
        } else {
            model::move_node(
                &mut self.collections,
                &mut self.requests,
                source,
                target,
                placement,
            )
        };
        match moved {
            Ok(moved) => {
                let remap = |collection: usize, path: String| {
                    if let (
                        TreeNode::Directory {
                            collection: from,
                            path: old,
                        },
                        TreeNode::Directory {
                            collection: to,
                            path: next,
                        },
                    ) = (source, &moved)
                    {
                        if !old.is_empty()
                            && collection == *from
                            && (path == *old || path.starts_with(&format!("{old}/")))
                        {
                            return (*to, format!("{next}{}", &path[old.len()..]));
                        }
                    }
                    (
                        self.collections
                            .iter()
                            .position(|c| Some(&c.collection) == old_names.get(collection))
                            .unwrap_or(0),
                        path,
                    )
                };
                self.collapsed_directories = std::mem::take(&mut self.collapsed_directories)
                    .into_iter()
                    .map(|(c, p)| remap(c, p))
                    .collect();
                let (c, path) = remap(self.current_collection, self.current_directory.clone());
                self.current_collection = c;
                self.current_directory = path;
                if let Some(name) = selected_name {
                    if matches!(source, TreeNode::Directory { path, .. } if path.is_empty()) {
                        self.current_collection = self
                            .collections
                            .iter()
                            .position(|c| c.collection == name)
                            .unwrap_or(0);
                    }
                }
                let (collection, mut parent) = match moved {
                    TreeNode::Directory { collection, path } => {
                        (collection, RequestGroups::parent(&path).to_owned())
                    }
                    TreeNode::Request(index) => (
                        self.collections
                            .iter()
                            .position(|c| c.collection == self.requests[index].collection)
                            .unwrap(),
                        self.requests[index].group.clone(),
                    ),
                };
                loop {
                    self.collapsed_directories
                        .remove(&(collection, parent.clone()));
                    if parent.is_empty() {
                        break;
                    }
                    parent = RequestGroups::parent(&parent).to_owned();
                }
                self.storage_dirty = true;
                self.tree_menu = None;
                self.group_edit = None;
                self.hovered_tree_row = None;
                self.notice = "已移动节点".into();
                self.sync_commands(cx);
                self.persist(cx);
            }
            Err(error) => self.notice = error,
        }
        self.tree_drop = None;
        cx.notify();
    }
    fn node_key(node: &model::TreeNode) -> String {
        match node {
            model::TreeNode::Request(index) => format!("request-leaf:{index}"),
            model::TreeNode::Directory { collection, path } => {
                format!("request-directory:{collection}:{path}")
            }
        }
    }
    fn node_from_key(&self, key: &str) -> Option<model::TreeNode> {
        if let Some(index) = key
            .strip_prefix("request-leaf:")
            .and_then(|s| s.parse::<usize>().ok())
        {
            return (index < self.requests.len()).then_some(model::TreeNode::Request(index));
        }
        let (collection, path) = key.strip_prefix("request-directory:")?.split_once(':')?;
        let collection = collection.parse::<usize>().ok()?;
        self.collections
            .get(collection)
            .filter(|c| c.contains(path))?;
        Some(model::TreeNode::Directory {
            collection,
            path: path.into(),
        })
    }
    fn tree_item(
        &self,
        node: model::TreeNode,
        label: String,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> TreeListItem {
        let owner = cx.entity().downgrade();
        let drop_owner = owner.clone();
        let indicator = self
            .tree_drop
            .as_ref()
            .filter(|(target, _)| *target == node)
            .map(|(_, p)| *p)
            .filter(|_| cx.has_active_drag());
        TreeListItem::new(Self::node_key(&node), label)
            .depth(depth)
            .theme(self.theme)
            .draggable(cx.entity_id())
            .drop_indicator(indicator)
            .on_drag_over(move |event: &TreeDrop, _, cx| {
                let _ = owner.update(cx, |this, cx| {
                    if let Some(target) = this.node_from_key(&event.target) {
                        let next = Some((target, event.placement));
                        if next != this.tree_drop {
                            this.tree_drop = next;
                            cx.notify();
                        }
                    }
                });
            })
            .on_drop(move |event: &TreeDrop, _, cx| {
                let _ = drop_owner.update(cx, |this, cx| {
                    if let (Some(source), Some(target)) = (
                        this.node_from_key(&event.source),
                        this.node_from_key(&event.target),
                    ) {
                        this.drop_tree_node(&source, &target, event.placement, cx);
                    }
                });
            })
    }
    fn tree_row(&self, entry: model::TreeEntry, cx: &mut Context<Self>) -> gpui::AnyElement {
        let (node, depth, label, expanded) = match entry {
            model::TreeEntry::Directory {
                collection,
                path,
                depth,
                expanded,
            } => {
                let label = if path.is_empty() {
                    self.collections[collection].collection.clone()
                } else {
                    path.rsplit('/').next().unwrap_or(&path).into()
                };
                (
                    model::TreeNode::Directory { collection, path },
                    depth,
                    label,
                    Some(expanded),
                )
            }
            model::TreeEntry::Request { index, depth } => (
                model::TreeNode::Request(index),
                depth,
                self.requests[index].name.clone(),
                None,
            ),
        };
        let key = Self::node_key(&node);
        let debug_key = key.clone();
        let hovered = self.hovered_tree_row.as_deref() == Some(key.as_str());
        let mut item = self.tree_item(node.clone(), label.clone(), depth, cx);
        match node {
            model::TreeNode::Directory { collection, path } => {
                item = item.branch(expanded.unwrap()).selected(
                    self.current_collection == collection && self.current_directory == path,
                );
                if path.is_empty() {
                    item = item.content(
                        div()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label),
                    );
                }
                if hovered || self.tree_menu.as_ref() == Some(&(collection, path.clone())) {
                    item = item.trailing(self.directory_menu(collection, path.clone(), cx));
                }
                item.render()
                    .debug_selector(move || debug_key.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tree_menu = None;
                        this.current_collection = collection;
                        this.current_directory = path.clone();
                        let key = (collection, path.clone());
                        if !this.collapsed_directories.remove(&key) {
                            this.collapsed_directories.insert(key);
                        }
                        this.group_edit = None;
                        this.group_error = None;
                        cx.notify();
                    }))
            }
            model::TreeNode::Request(index) => {
                let request = &self.requests[index];
                item = item.selected(index == self.active).content(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .w(px(34.))
                                .flex_shrink_0()
                                .text_size(px(9.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(self
                                    .theme
                                    .color(method_color(&request.draft.method))))
                                .child(request.draft.method.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(request.name.clone()),
                        ),
                );
                if hovered {
                    item = item.trailing(
                        IconButton::new(Icon::Trash, "删除请求")
                            .theme(self.theme)
                            .render()
                            .size(px(20.))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.delete_request(index, cx);
                            })),
                    );
                }
                item.render()
                    .debug_selector(move || debug_key.clone())
                    .on_click(cx.listener(move |this, _, _, cx| this.choose(index, cx)))
            }
        }
        .on_hover(cx.listener(move |this, hover, _, cx| {
            if *hover {
                this.hovered_tree_row = Some(key.clone());
            } else if this.hovered_tree_row.as_deref() == Some(key.as_str()) {
                this.hovered_tree_row = None;
            }
            cx.notify();
        }))
        .into_any_element()
    }
    fn directory_menu(&self, collection: usize, path: String, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let key = (collection, path.clone());
        let open = self.tree_menu.as_ref() == Some(&key);
        let toggle_key = key.clone();
        let debug_key = format!("directory-menu:{collection}:{path}");
        let mut trigger = div().relative().flex_shrink_0().child(
            IconButton::new(Icon::More, "目录操作")
                .theme(self.theme)
                .render()
                .id(format!("directory-menu:{collection}:{path}"))
                .debug_selector(move || debug_key.clone())
                .size(px(22.))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.tree_menu = if this.tree_menu.as_ref() == Some(&toggle_key) {
                        None
                    } else {
                        Some(toggle_key.clone())
                    };
                    this.show_collection_menu = false;
                    window.focus(&this.focus, cx);
                    cx.notify();
                })),
        );
        if open {
            let mut menu = div()
                .p_1()
                .rounded(px(8.))
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.panel))
                .shadow_md()
                .text_size(px(12.));
            for (action, icon, label) in [
                (0, Icon::Plus, "新建请求"),
                (1, Icon::Folder, "新建目录"),
                (2, Icon::Pencil, "重命名目录"),
                (3, Icon::Trash, "删除目录"),
            ] {
                if path.is_empty() && action >= 2 {
                    continue;
                }
                let path = path.clone();
                let debug_key = format!("directory-menu-action:{collection}:{path}:{action}");
                menu = menu.child(
                    div()
                        .id(format!(
                            "directory-menu-action:{collection}:{path}:{action}"
                        ))
                        .debug_selector(move || debug_key.clone())
                        .h(px(32.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded(px(5.))
                        .cursor_pointer()
                        .when(action == 3, |item| {
                            item.text_color(rgb(self.theme.color(0xd9342b)))
                        })
                        .hover(|style| style.bg(rgb(p.surface_subtle)))
                        .child(icon.render(14.).flex_shrink_0())
                        .child(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.tree_menu = None;
                            this.current_collection = collection;
                            this.current_directory = path.clone();
                            match action {
                                0 => this.add_request_to(path.clone(), cx),
                                1 => {
                                    this.edit_directory(collection, path.clone(), None, window, cx)
                                }
                                2 => this.edit_directory(
                                    collection,
                                    path.clone(),
                                    Some(path.clone()),
                                    window,
                                    cx,
                                ),
                                _ => {
                                    this.collections[collection].remove(&path, &mut this.requests);
                                    this.collapsed_directories.retain(|(c, folder)| {
                                        *c != collection
                                            || (folder != &path
                                                && !folder.starts_with(&format!("{path}/")))
                                    });
                                    this.current_directory =
                                        RequestGroups::parent(&path).to_owned();
                                    this.group_edit = None;
                                    this.storage_dirty = true;
                                    this.notice = "已删除目录，请求保留在上级目录中".into();
                                    this.sync_groups(cx);
                                }
                            }
                        })),
                );
            }
            let popup = div()
                .absolute()
                .top_0()
                .right_0()
                .w(px(168.))
                .pt(px(26.))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.tree_menu = None;
                    cx.notify();
                }))
                .child(menu);
            trigger = trigger.child(gpui::deferred(popup).with_priority(20));
        }
        trigger
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
                    self.add_proxy_input(cx);
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
                    self.current_directory.clear();
                    self.notice = format!("已创建集合 {}", name.trim());
                })
            }
            Some(GroupEdit::Create(parent)) => self.collections[self.current_collection]
                .add_child(parent, &name)
                .map(|path| {
                    let node = model::TreeNode::Directory {
                        collection: self.current_collection,
                        path: path.clone(),
                    };
                    let target = model::TreeNode::Directory {
                        collection: self.current_collection,
                        path: parent.clone(),
                    };
                    let _ = model::move_node(
                        &mut self.collections,
                        &mut self.requests,
                        &node,
                        &target,
                        model::Placement::Inside,
                    );
                    self.current_directory = path.clone();
                    self.collapsed_directories
                        .remove(&(self.current_collection, path));
                    let mut path = parent.as_str();
                    loop {
                        self.collapsed_directories
                            .remove(&(self.current_collection, path.to_owned()));
                        if path.is_empty() {
                            break;
                        }
                        path = RequestGroups::parent(path);
                    }
                }),
            Some(GroupEdit::Rename(old)) => {
                let parent = RequestGroups::parent(old);
                let new = if parent.is_empty() {
                    name.trim().to_owned()
                } else {
                    format!("{parent}/{}", name.trim())
                };
                self.collections[self.current_collection]
                    .rename(old, &name, &mut self.requests)
                    .map(|_| {
                        let prefix = format!("{old}/");
                        self.collapsed_directories = self
                            .collapsed_directories
                            .iter()
                            .map(|(collection, path)| {
                                if *collection == self.current_collection
                                    && (path == old || path.starts_with(&prefix))
                                {
                                    (*collection, format!("{new}{}", &path[old.len()..]))
                                } else {
                                    (*collection, path.clone())
                                }
                            })
                            .collect();
                        self.current_directory = new;
                    })
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
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "enter" => this.save_group(cx),
                    "escape" => {
                        this.group_edit = None;
                        this.group_error = None;
                        cx.notify();
                    }
                    _ => return,
                }
                cx.stop_propagation();
                if this.group_edit.is_none() {
                    window.focus(&this.focus, cx);
                }
            }))
            .child(match self.group_edit {
                Some(GroupEdit::CreateCollection) => "创建集合",
                Some(GroupEdit::CreateEnvironment) => "创建环境",
                Some(GroupEdit::Create(_)) => "新建目录",
                _ => "重命名目录",
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
        self.add_request_to(self.current_directory.clone(), cx);
    }
    fn add_request_to(&mut self, group: String, cx: &mut Context<Self>) {
        if self.collections.is_empty() {
            self.group_edit = Some(GroupEdit::CreateCollection);
            self.group_error = None;
            cx.notify();
            return;
        }
        let draft = Draft::default();
        if !self.collections[self.current_collection].contains(&group) {
            return;
        }
        let i = self.requests.len();
        self.storage_dirty = true;
        self.requests.push(Request {
            tree_order: 0,
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
                exchange: None,
            }],
            history: vec![],
        });
        let target = model::TreeNode::Directory {
            collection: self.current_collection,
            path: self.requests[i].group.clone(),
        };
        let _ = model::move_node(
            &mut self.collections,
            &mut self.requests,
            &model::TreeNode::Request(i),
            &target,
            model::Placement::Inside,
        );
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
        self.cancel_request_name(cx);
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
        self.sync_reference_inputs(cx);
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
    fn request_setting_row(
        &self,
        title: &'static str,
        description: &'static str,
        control: impl IntoElement,
    ) -> gpui::Div {
        let p = self.theme.palette();
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_5()
            .py_3()
            .border_b_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(description),
                    ),
            )
            .child(
                div()
                    .w(px(180.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_end()
                    .child(control),
            )
    }
    fn configuration(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let owner = cx.entity().downgrade();
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
            5=>self.plugin_configuration(cx),
            3=>div().h_full().flex().flex_col().gap_2().child(div().flex().items_center().text_size(px(11.)).child("JSON 请求体").child(div().flex_1()).child(Button::new("格式化 JSON").kind(ButtonKind::Plain).size(ButtonSize::Mini).theme(self.theme).render().on_click(cx.listener(|this,_,_,cx| {let valid=this.body.update(cx,|e,cx|e.format_json(cx));this.notice=if valid {"已格式化 JSON"} else {"JSON 格式有误，请检查后重试"}.into();cx.notify();}))))
                .child(div().flex_1().min_h_0().child(self.body.clone())),
            _=>div().w_full().flex().flex_col()
                .child(self.request_setting_row("跳过 TLS 证书校验",
                    "允许自签名、过期或域名不匹配的证书，仅作用于当前请求。",
                    Switch::new("")
                    .theme(self.theme)
                    .checked(self.skip_tls_verification)
                    .on_change(move |checked, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            this.skip_tls_verification = checked;
                            cx.notify();
                        });
                    })
                    .render().debug_selector(|| "skip-tls-verification".into())))
                .child(self.request_setting_row("连接超时", "建立连接时的最长等待时间。", div().text_size(px(12.)).child("10 秒")))
                .child(self.request_setting_row("请求超时", "包含连接和接收响应的总等待时间。", div().text_size(px(12.)).child("30 秒")))
                .child(self.request_setting_row("重定向", "保留服务器返回的原始 3xx 响应。", div().text_size(px(12.)).child("不自动跟随")))
                .child(self.request_setting_row("响应大小上限", "接收响应内容的最大大小。", div().text_size(px(12.)).child("16 MiB"))),
        };
        div()
            .id("configuration")
            .h(px(if self.config == 5 { 360. } else { 218. }))
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
                            .get(
                                self.selected_history
                                    .filter(|(request, _)| *request == self.active)
                                    .map_or(0, |(_, index)| index)
                            )
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
                            IconButton::new(Icon::Search, "搜索响应体")
                                .theme(self.theme)
                                .render()
                                .id("response-search-toggle")
                                .debug_selector(|| "response-search-toggle".into())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_response_search(window, cx);
                                })),
                        )
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
                .when(self.show_response_search, |el| {
                    el.child(
                        div()
                            .id("response-search-bar")
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_5()
                            .py_1()
                            .child(
                                SearchInput::new("搜索响应体…")
                                    .theme(self.theme)
                                    .width(220.)
                                    .control(self.response_search.clone())
                                    .render()
                                    .on_key_down(cx.listener(
                                        |this, event: &gpui::KeyDownEvent, window, cx| {
                                            if this.response_search.read(cx).is_composing() {
                                                return;
                                            }
                                            match event.keystroke.key.as_str() {
                                                "enter" => {
                                                    this.step_response_search(
                                                        event.keystroke.modifiers.shift,
                                                        cx,
                                                    );
                                                    cx.stop_propagation();
                                                }
                                                "escape" => {
                                                    this.toggle_response_search(window, cx);
                                                    cx.stop_propagation();
                                                }
                                                _ => {}
                                            }
                                        },
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.text_muted))
                                    .child(format!(
                                        "{}/{}",
                                        self.response_search_index.map_or(0, |i| i + 1),
                                        self.response_search_count
                                    )),
                            )
                            .child(
                                IconButton::new(Icon::ArrowUp, "上一处（Shift+Enter）")
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.step_response_search(true, cx)
                                    })),
                            )
                            .child(
                                IconButton::new(Icon::ChevronDown, "下一处（Enter）")
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.step_response_search(false, cx)
                                    })),
                            )
                            .child(div().flex_1())
                            .child(
                                IconButton::new(Icon::Close, "关闭搜索（Esc）")
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_response_search(window, cx)
                                    })),
                            ),
                    )
                })
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
                    .child(self.request_name_view(cx))
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
        for (i, label) in [
            "参数",
            "身份验证",
            "请求头",
            "请求体",
            "请求设置",
            "前置动作",
        ]
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
            .child(Separator::horizontal().theme(self.theme).render())
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
                        .debug_selector(move || format!("history-record-{i}").into())
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
                            let exchange = record.exchange.as_ref();
                            this.response_data = exchange.and_then(|e| e.response.clone());
                            this.response_json =
                                this.response_data.as_ref().and_then(|r| r.json.clone());
                            this.status = this.response_data.as_ref().map(|r| r.status);
                            this.error = exchange.and_then(|e| e.error.clone());
                            this.sent_requests[this.active] =
                                exchange.and_then(|e| e.preview.clone());
                            this.traces[this.active] =
                                exchange.map(|e| e.traces.clone()).unwrap_or_default();
                            this.responses[this.active] = this.response_data.clone();
                            this.sync_request_preview(cx);
                            this.update_response(cx);
                            this.notice = if exchange.is_some() {
                                format!("已恢复 {} 的请求与响应", record.label)
                            } else {
                                "已恢复历史请求配置，此旧记录未保存响应内容".into()
                            };
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
                        this.update_response(cx);
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.show_registry_dialog
            && [&self.registry_name, &self.registry_url, &self.registry_key]
                .iter()
                .any(|e| e.read(cx).focus_handle(cx).is_focused(window))
        {
            window.focus(&self.focus, cx);
        }
        #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
        if let Some(view) = &self.html_webview {
            let visible = self.html_preview
                && self.sending.is_none()
                && self.error.is_none()
                && self.response_data.is_some()
                && !self.show_environments
                && !self.show_plugins
                && self.response_tab == 0
                && !self.commands.read(cx).is_open();
            if let Err(error) = view.update(cx, |view, cx| view.set_visible(visible, cx)) {
                self.notice = format!("WebView 显隐失败：{error}");
            }
        }
        let _ = &self.subscriptions;
        let content = div()
            .w_full()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .child(if self.show_plugins {
                self.plugin_workspace(cx)
            } else if self.show_environments {
                self.environment_workspace(cx)
            } else {
                self.workspace(cx)
            })
            .when(
                self.show_activity && !self.show_environments && !self.show_plugins,
                |s| s.child(self.activity_panel(cx)),
            );
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
            .when(self.show_update_details, |el| {
                el.child(self.update_banner(cx))
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
            .on_action(cx.listener(|this, _: &SendRequest, _, cx| {
                if !this.show_plugins && !this.show_environments {
                    this.send(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SaveRequest, _, cx| {
                if this.show_plugins {
                    return;
                } else if this.show_environments {
                    this.save_environment_variables(cx);
                } else {
                    this.save(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SearchCommands, w, cx| {
                if !this.show_registry_dialog {
                    this.open_commands(w, cx);
                }
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if this.show_registry_dialog {
                    if event.keystroke.key == "tab" {
                        let fields = [&this.registry_name, &this.registry_url, &this.registry_key];
                        let current = fields
                            .iter()
                            .position(|e| e.read(cx).focus_handle(cx).is_focused(window))
                            .unwrap_or(0);
                        let next = if event.keystroke.modifiers.shift {
                            (current + 2) % 3
                        } else {
                            (current + 1) % 3
                        };
                        window.focus(&fields[next].read(cx).focus_handle(cx), cx);
                        cx.stop_propagation();
                    }
                    if event.keystroke.key == "escape" {
                        if !this.registry_adding_source {
                            this.show_registry_dialog = false;
                            window.focus(&this.focus, cx);
                            cx.notify();
                        }
                        cx.stop_propagation();
                    }
                    return;
                }
                if event.keystroke.key == "escape" && this.tree_menu.is_some() {
                    this.tree_menu = None;
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
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
            .children(self.registry_dialog(cx))
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
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--plugin-worker")) {
        if !plugins::worker() {
            std::process::exit(1);
        }
        return;
    }
    let mut args = std::env::args_os().skip(1);
    let action = args.next();
    if action.as_deref() == Some(std::ffi::OsStr::new("--version")) {
        println!("Glint API {}", updater::VERSION);
        return;
    }
    if action.as_deref() == Some(std::ffi::OsStr::new("--check-update")) {
        match updater::check_blocking() {
            Ok(update) => println!(
                "{}",
                serde_json::json!({
                    "currentVersion": updater::VERSION,
                    "availableVersion": update.as_ref().map(|update| update.version.to_string()),
                    "releaseUrl": update.as_ref().map(updater::Update::release_url),
                })
            ),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if action.as_deref() == Some(std::ffi::OsStr::new("--download-release")) {
        let result = args
            .next()
            .and_then(|value| value.into_string().ok())
            .ok_or("缺少发布版本".to_owned())
            .and_then(|version| {
                args.next()
                    .ok_or("缺少下载目录".to_owned())
                    .and_then(|directory| {
                        updater::download_release(&version, std::path::Path::new(&directory))
                    })
            });
        match result {
            Ok(path) => println!("{}", path.display()),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if action.as_deref() == Some(std::ffi::OsStr::new("--apply-update")) {
        let result = args
            .next()
            .ok_or("缺少更新任务路径".to_owned())
            .and_then(|path| updater::run_helper(std::path::Path::new(&path)));
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return;
    }
    gpui_platform::application()
        .with_assets(GlintAssets)
        .run(|cx: &mut App| {
            app_icon::install();
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
    fn native_tree_drag_reorders_moves_and_persists(cx: &mut gpui::TestAppContext) {
        let path = std::env::temp_dir().join(format!(
            "glint-api-drag-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.add_request(cx);
            host.group_edit = Some(GroupEdit::Create("".into()));
            host.group_name
                .update(cx, |entry, cx| entry.set_value("Folder", cx));
            host.save_group(cx);
            host
        });
        let drag = |window: &mut gpui::VisualTestContext,
                    source: gpui::Point<gpui::Pixels>,
                    target: gpui::Point<gpui::Pixels>| {
            window.simulate_event(gpui::MouseDownEvent {
                position: source,
                button: gpui::MouseButton::Left,
                click_count: 1,
                ..Default::default()
            });
            for _ in 0..2 {
                window.simulate_event(gpui::MouseMoveEvent {
                    position: target,
                    pressed_button: Some(gpui::MouseButton::Left),
                    ..Default::default()
                });
            }
            window.simulate_event(gpui::MouseUpEvent {
                position: target,
                button: gpui::MouseButton::Left,
                click_count: 1,
                ..Default::default()
            });
        };
        let source = window.debug_bounds("request-leaf:0").unwrap();
        let target = window.debug_bounds("request-leaf:1").unwrap();
        drag(
            window,
            source.center(),
            point(target.center().x, target.bottom() - px(2.)),
        );
        host.update(window, |host, _| {
            let order = model::siblings(&host.collections, &host.requests, 0, "");
            assert!(
                order
                    .iter()
                    .position(|node| *node == model::TreeNode::Request(1))
                    .unwrap()
                    < order
                        .iter()
                        .position(|node| *node == model::TreeNode::Request(0))
                        .unwrap()
            );
            assert_eq!(
                host.active, 1,
                "dragging must not switch the active request"
            );
            assert_eq!(host.requests[0].versions.len(), 1);
        });
        let source = window.debug_bounds("request-leaf:0").unwrap();
        let folder = window.debug_bounds("request-directory:0:Folder").unwrap();
        drag(window, source.center(), folder.center());
        host.update(window, |host, _| {
            assert_eq!(host.requests[0].group, "Folder")
        });
        host.update(window, |host, cx| {
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("Other", cx));
            host.save_group(cx);
        });
        let source = window.debug_bounds("request-directory:0:Folder").unwrap();
        let target = window.debug_bounds("request-directory:1:").unwrap();
        drag(window, source.center(), target.center());
        host.update(window, |host, _| {
            assert_eq!(host.requests[0].collection, "Other");
            assert_eq!(host.requests[0].group, "Folder");
            assert!(host.collections[1].contains("Folder"));
        });
        host.update(window, |host, cx| {
            host.collapsed_directories.insert((1, "Folder".into()));
            cx.notify();
        });
        let source = window.debug_bounds("request-directory:1:").unwrap();
        let target = window.debug_bounds("request-directory:0:").unwrap();
        drag(
            window,
            source.center(),
            point(target.center().x, target.top() + px(2.)),
        );
        host.update(window, |host, _| {
            assert_eq!(host.collections[0].collection, "Other");
            assert!(host.collapsed_directories.contains(&(0, "Folder".into())));
        });
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(window, |host, _| {
            assert_eq!(host.requests[0].collection, "Other");
            assert_eq!(host.requests[0].group, "Folder");
            assert!(host.requests[0].tree_order > 0);
            assert_eq!(host.requests[0].versions.len(), 1);
            assert_eq!(host.collections[0].collection, "Other");
        });
        std::fs::remove_file(path).unwrap();
    }
    #[gpui::test]
    fn history_click_restores_matching_response_headers_and_preview(cx: &mut gpui::TestAppContext) {
        let path = std::env::temp_dir().join(format!(
            "glint-api-history-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            for (status, text) in [(200, "first"), (404, "second")] {
                let mut record = host.requests[0].versions[0].clone();
                record.label = status_label(status);
                record.draft.url = format!("https://example.test/{text}");
                record.exchange = Some(model::Exchange {
                    response: Some(HttpResponse {
                        status,
                        headers: vec![("x-record".into(), text.into())],
                        text: text.into(),
                        bytes: text.len(),
                        json: None,
                    }),
                    error: None,
                    preview: Some(format!("GET /{text}")),
                    traces: vec![TraceEntry {
                        elapsed_ms: 10,
                        message: text.into(),
                    }],
                });
                host.requests[0].history.push(record);
            }
            let legacy = host.requests[0].versions[0].clone();
            host.requests[0].history.push(legacy);
            host.show_activity = true;
            host.activity = 1;
            host.storage_dirty = true;
            host.persist(cx);
            host
        });
        for (index, status, text) in [(0, 200, "first"), (1, 404, "second"), (0, 200, "first")] {
            let bounds = window
                .debug_bounds(if index == 0 {
                    "history-record-0"
                } else {
                    "history-record-1"
                })
                .unwrap();
            window.simulate_click(bounds.center(), gpui::Modifiers::default());
            host.update(window, |host, cx| {
                assert_eq!(host.status, Some(status));
                assert_eq!(host.response_data.as_ref().unwrap().text, text);
                assert_eq!(host.response.read(cx).content(), text);
                assert_eq!(host.response_header_entries[0][1].read(cx).value(), text);
                assert_eq!(
                    host.request_preview.read(cx).content(),
                    format!("GET /{text}")
                );
                assert_eq!(host.traces[0][0].message, text);
            });
        }
        let bounds = window.debug_bounds("history-record-2").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert!(
                host.response_data.is_none()
                    && host.status.is_none()
                    && host.response_header_entries.is_empty()
            );
            assert!(host.notice.contains("未保存响应"));
        });
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(window, |host, _| {
            assert_eq!(
                host.requests[0].history[1]
                    .exchange
                    .as_ref()
                    .unwrap()
                    .response
                    .as_ref()
                    .unwrap()
                    .text,
                "second"
            )
        });
        std::fs::remove_file(path).unwrap();
    }
    #[gpui::test]
    fn tls_setting_click_persists_and_restores_per_request(cx: &mut gpui::TestAppContext) {
        let path = std::env::temp_dir().join(format!(
            "glint-api-tls-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.config = 4;
            host
        });
        let bounds = window.debug_bounds("skip-tls-verification").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert!(host.snapshot(cx).skip_tls_verification);
            host.save(cx);
            assert!(host.requests[0].versions[0].draft.skip_tls_verification);
            host.add_request(cx);
            assert!(!host.snapshot(cx).skip_tls_verification);
            host.choose(0, cx);
            assert!(host.snapshot(cx).skip_tls_verification);
            let baseline = host.requests[0].versions[1].draft.clone();
            host.load(&baseline, cx);
            assert!(!host.snapshot(cx).skip_tls_verification);
            let saved = host.requests[0].versions[0].draft.clone();
            host.load(&saved, cx);
            host.persist(cx);
        });
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(window, |host, cx| {
            assert!(host.snapshot(cx).skip_tls_verification)
        });
        let mut legacy = serde_json::to_value(Draft::default()).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("skip_tls_verification");
        assert!(
            !serde_json::from_value::<Draft>(legacy)
                .unwrap()
                .skip_tls_verification
        );
        std::fs::remove_file(path).unwrap();
    }
    #[gpui::test]
    fn request_tree_menus_create_root_and_nested_requests_without_toggling_folders(
        cx: &mut gpui::TestAppContext,
    ) {
        let path = std::env::temp_dir().join(format!(
            "glint-api-tree-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.add_request(cx);
            assert!(
                host.requests.is_empty(),
                "a collection is required before creating a request"
            );
            host.group_name
                .update(cx, |entry, cx| entry.set_value("My API", cx));
            host.save_group(cx);
            host.add_request(cx);
            assert!(host.collections[0].names.is_empty());
            assert!(host.requests[0].group.is_empty());
            host.collections[0].add_child("", "目录").unwrap();
            host.collections[0].add_child("目录", "子目录").unwrap();
            host.add_request_to("目录/子目录".into(), cx);
            host
        });
        let root = window.debug_bounds("request-directory:0:").unwrap();
        window.simulate_event(gpui::MouseMoveEvent {
            position: root.center(),
            ..Default::default()
        });
        let more = window
            .debug_bounds("directory-menu:0:")
            .expect("hover reveals a single menu trigger");
        window.simulate_click(more.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert_eq!(host.tree_menu, Some((0, String::new())));
            assert!(!host.collapsed_directories.contains(&(0, String::new())));
        });
        assert!(window.debug_bounds("directory-menu-action:0::2").is_none());
        let new_request = window.debug_bounds("directory-menu-action:0::0").unwrap();
        window.simulate_click(new_request.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert_eq!(host.requests.len(), 3);
            assert!(host.requests[2].group.is_empty());
            assert!(host.tree_menu.is_none());
        });
        let folder = window
            .debug_bounds("request-directory:0:目录/子目录")
            .unwrap();
        window.simulate_click(folder.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert!(
                host.collapsed_directories
                    .contains(&(0, "目录/子目录".into()))
            )
        });
        window.simulate_event(gpui::MouseMoveEvent {
            position: folder.center(),
            ..Default::default()
        });
        let more = window.debug_bounds("directory-menu:0:目录/子目录").unwrap();
        window.simulate_click(more.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert!(
                host.collapsed_directories
                    .contains(&(0, "目录/子目录".into()))
            )
        });
        window.simulate_keystrokes("escape");
        host.update(window, |host, _| assert!(host.tree_menu.is_none()));
        window.simulate_click(more.center(), gpui::Modifiers::default());
        let create_folder = window
            .debug_bounds("directory-menu-action:0:目录/子目录:1")
            .unwrap();
        window.simulate_click(create_folder.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert!(matches!(&host.group_edit, Some(GroupEdit::Create(parent)) if parent == "目录/子目录"));
            host.group_name.update(cx, |entry, cx| entry.set_value("第三层", cx));
        });
        window.simulate_keystrokes("enter");
        host.update(window, |host, cx| {
            assert!(host.collections[0].contains("目录/子目录/第三层"));
            host.add_request(cx);
            assert_eq!(host.requests.last().unwrap().group, "目录/子目录/第三层");
            host.persist(cx);
        });
        let saved = storage::Store::open(&path).unwrap().load().unwrap();
        assert!(saved.requests[0].group.is_empty());
        assert_eq!(saved.requests.last().unwrap().group, "目录/子目录/第三层");
        std::fs::remove_file(path).unwrap();
    }

    #[gpui::test]
    fn response_search_opens_cycles_refreshes_and_closes(cx: &mut gpui::TestAppContext) {
        cx.update(bind_text_entry_keys);
        struct ResponsePanel(Entity<Workbench>);
        impl Render for ResponsePanel {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .w(px(850.))
                    .h(px(300.))
                    .flex()
                    .flex_col()
                    .child(self.0.update(cx, |host, cx| host.response_panel(cx)))
            }
        }
        let path =
            std::env::temp_dir().join(format!("glint-api-search-{}.sqlite3", uuid::Uuid::new_v4()));
        let host = cx.new(|cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.response_data = Some(HttpResponse {
                status: 200,
                headers: vec![],
                text: "Rust 中文\nRUST 中文".into(),
                bytes: 25,
                json: None,
            });
            host.update_response(cx);
            host
        });
        let (_, window) = cx.add_window_view(|_, _| ResponsePanel(host.clone()));
        let toggle = window.debug_bounds("response-search-toggle").unwrap();
        window.simulate_click(toggle.center(), gpui::Modifiers::default());
        let focus = host.update(window, |host, cx| {
            assert!(host.show_response_search);
            host.response_search.focus_handle(cx)
        });
        window.update(|window, _| assert!(focus.is_focused(window)));
        window.simulate_input("rust");
        host.update(window, |host, _| {
            assert_eq!(host.response_search_count, 2);
            assert_eq!(host.response_search_index, Some(0));
        });
        window.simulate_keystrokes("enter");
        host.update(window, |host, _| {
            assert_eq!(host.response_search_index, Some(1))
        });
        window.simulate_keystrokes("enter");
        host.update(window, |host, _| {
            assert_eq!(host.response_search_index, Some(0))
        });
        window.simulate_keystrokes("shift-enter");
        host.update(window, |host, cx| {
            assert_eq!(host.response_search_index, Some(1));
            host.response_data.as_mut().unwrap().text = "RUST 新响应".into();
            host.update_response(cx);
            assert_eq!(host.response_search_count, 1);
            assert_eq!(host.response_search_index, Some(0));
            host.response_search
                .update(cx, |entry, cx| entry.set_value("不存在", cx));
        });
        window.run_until_parked();
        host.update(window, |host, _| {
            assert_eq!(host.response_search_count, 0);
            assert_eq!(host.response_search_index, None);
        });
        window.simulate_keystrokes("escape");
        host.update(window, |host, cx| {
            assert!(!host.show_response_search);
            assert_eq!(host.response.read(cx).search_query(), "");
            assert_eq!(host.response_content(), "RUST 新响应");
        });
        drop(host);
        std::fs::remove_file(path).unwrap();
    }

    #[gpui::test]
    fn many_response_headers_scroll_within_the_panel(cx: &mut gpui::TestAppContext) {
        cx.update(bind_text_entry_keys);
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
        let value = window.debug_bounds("response-header-0-1").unwrap();
        window.simulate_click(value.center(), gpui::Modifiers::default());
        window.simulate_keystrokes("cmd-a cmd-c");
        assert_eq!(
            window.read_from_clipboard().unwrap().text().unwrap(),
            format!("cookie-0={}", "x".repeat(600))
        );
        window.simulate_keystrokes("backspace");
        host.update(window, |host, cx| {
            assert_eq!(
                host.response_header_entries[0][1].read(cx).value(),
                format!("cookie-0={}", "x".repeat(600))
            )
        });
        let name = window.debug_bounds("response-header-0-0").unwrap();
        window.simulate_click(name.center(), gpui::Modifiers::default());
        window.simulate_keystrokes("cmd-a cmd-c");
        assert_eq!(
            window.read_from_clipboard().unwrap().text().unwrap(),
            "set-cookie"
        );
        window.simulate_event(gpui::ScrollWheelEvent {
            position: scroll.bounds().center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-200.))),
            ..Default::default()
        });
        assert!(scroll.offset().y < px(0.));
        std::fs::remove_file(path).unwrap();
    }

    #[gpui::test]
    fn request_title_rename_saves_cancels_and_survives_restart(cx: &mut gpui::TestAppContext) {
        struct NamePanel {
            host: Entity<Workbench>,
            bounds: std::rc::Rc<std::cell::Cell<Bounds<gpui::Pixels>>>,
        }
        impl Render for NamePanel {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let bounds = self.bounds.clone();
                div()
                    .w(px(500.))
                    .h(px(100.))
                    .child(self.host.update(cx, |host, cx| host.request_name_view(cx)))
                    .on_children_prepainted(move |children, _, _| bounds.set(children[0]))
            }
        }
        let path = std::env::temp_dir().join(format!(
            "glint-api-rename-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let host = cx.new(|cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("My API", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.add_request(cx);
            host.choose(0, cx);
            host
        });
        let bounds = std::rc::Rc::new(std::cell::Cell::new(Bounds::default()));
        let (_, window) = cx.add_window_view(|_, _| NamePanel {
            host: host.clone(),
            bounds: bounds.clone(),
        });
        let click_title = |window: &mut gpui::VisualTestContext| {
            window.simulate_click(bounds.get().center(), gpui::Modifiers::default());
        };
        click_title(window);
        host.update(window, |host, cx| {
            assert!(
                host.editing_request_name,
                "clicking the title must enter editing"
            );
            host.request_name
                .update(cx, |entry, cx| entry.set_value("  ", cx));
        });
        window.simulate_keystrokes("enter");
        host.update(window, |host, cx| {
            assert!(host.editing_request_name && host.request_name_error.is_some());
            assert_eq!(host.requests[0].name, "新建请求 1");
            host.request_name
                .update(cx, |entry, cx| entry.set_value("  获取用户 🦀  ", cx));
        });
        window.simulate_keystrokes("enter");
        host.update(window, |host, cx| {
            assert!(!host.editing_request_name);
            assert_eq!(host.requests[0].name, "获取用户 🦀");
            assert_eq!(host.requests[0].versions.len(), 1);
            host.persist(cx);
        });
        click_title(window);
        host.update(window, |host, cx| {
            host.request_name
                .update(cx, |entry, cx| entry.set_value("取消的名字", cx));
        });
        window.simulate_keystrokes("escape");
        host.update(window, |host, _| {
            assert!(!host.editing_request_name);
            assert_eq!(host.requests[0].name, "获取用户 🦀");
        });
        click_title(window);
        host.update(window, |host, cx| {
            host.request_name
                .update(cx, |entry, cx| entry.set_value("切换时未提交", cx));
            host.choose(1, cx);
            host.save_request_name(cx);
            assert_eq!(host.requests[1].name, "新建请求 2");
            host.persist(cx);
        });
        let saved = storage::Store::open(&path).unwrap().load().unwrap();
        assert_eq!(saved.requests[0].name, "获取用户 🦀");
        assert_eq!(saved.requests[1].name, "新建请求 2");
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(window, |host, _| {
            assert_eq!(host.requests[0].name, "获取用户 🦀")
        });
        click_title(window);
        host.update(window, |host, cx| {
            host.request_name
                .update(cx, |entry, cx| entry.set_value("删除时未提交", cx));
            host.delete_request(0, cx);
            host.save_request_name(cx);
            assert!(!host.editing_request_name);
            assert_eq!(host.requests[0].name, "新建请求 2");
        });
        std::fs::remove_file(path).unwrap();
    }

    #[gpui::test]
    fn saving_only_versions_changes_since_latest_snapshot(cx: &mut gpui::TestAppContext) {
        let path = std::env::temp_dir().join(format!(
            "glint-api-save-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let host = cx.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        host.update(cx, |host, cx| {
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            let original = host.requests[0].versions[0].draft.clone();
            host.save(cx);
            host.save(cx);
            assert_eq!(host.requests[0].versions.len(), 1);
            host.endpoint.update(cx, |entry, cx| {
                entry.set_text("https://example.test/changed", cx)
            });
            // Drafts are persisted automatically; the version baseline must stay separate.
            host.persist(cx);
            host.save(cx);
            assert_eq!(host.requests[0].versions.len(), 2);
            assert_eq!(host.requests[0].versions[0].label, "v2");
            host.save(cx);
            assert_eq!(host.requests[0].versions.len(), 2);
            host.load(&original, cx);
            host.save(cx);
            assert_eq!(host.requests[0].versions.len(), 3);
            assert_eq!(host.requests[0].versions[0].draft, original);
            // Editing and undoing back to the latest saved content is also unchanged.
            host.endpoint.update(cx, |entry, cx| {
                entry.set_text("https://example.test/temporary", cx)
            });
            host.load(&original, cx);
            host.save(cx);
            assert_eq!(host.requests[0].versions.len(), 3);
            host.persist(cx);
        });
        let reopened = cx.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(cx, |host, cx| {
            host.save(cx);
            assert_eq!(host.requests[0].versions.len(), 3);
            assert_eq!(host.notice, "已保存，内容未修改");
        });
        drop(host);
        drop(reopened);
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
