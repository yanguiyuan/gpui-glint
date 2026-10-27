//! Shared workbench state, construction, and workspace initialization.

use super::*;

pub(super) enum GroupEdit {
    CreateCollection,
    RenameCollection(String),
    CreateEnvironment,
    Create(String),
    Rename(String),
}
/// 标签页右键菜单的批量关闭方式。
pub(super) enum TabCloseMode {
    /// 只保留被点击的标签
    Others,
    /// 保留被点击标签及其右侧标签
    Left,
    /// 保留被点击标签及其左侧标签
    Right,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct TraceEntry {
    pub(super) elapsed_ms: u128,
    pub(super) message: String,
}
pub(super) struct PairInput {
    pub(super) enabled: bool,
    pub(super) key: Entity<TextEntry>,
    pub(super) value: Entity<TextEntry>,
    pub(super) note: Entity<TextEntry>,
}
pub(super) struct Workbench {
    pub(super) request_editors: HashMap<String, request_window::SharedRequest>,
    pub(super) request_binding: Option<request_window::Binding>,
    pub(super) directory_settings: Option<Entity<directory_ui::DirectorySettings>>,
    pub(super) directory_subscription: Option<Subscription>,
    pub(super) version_tag_edit: Option<(usize, String)>,
    pub(super) version_tag_input: Entity<TextEntry>,
    pub(super) toasts: Entity<ToastHost>,
    pub(super) update_state: update_ui::UpdateState,
    pub(super) update_task: Option<Task<()>>,
    pub(super) show_update_details: bool,
    pub(super) update_error: Option<String>,
    pub(super) update_source: Option<updater::UpdateSource>,
    pub(super) update_source_url: Entity<TextEntry>,
    pub(super) update_source_key: Entity<TextEntry>,
    pub(super) show_update_source_settings: bool,
    pub(super) storage: storage::Store,
    pub(super) workspace_list: Vec<storage::WorkspaceInfo>,
    pub(super) show_workspaces: bool,
    pub(super) workspace_name_input: Entity<TextEntry>,
    pub(super) workspace_edit: Option<String>,
    pub(super) workspace_delete: Option<String>,
    pub(super) workspace_error: Option<String>,
    pub(super) workspace_transfer: Option<workspace_transfer_ui::TransferDialog>,
    pub(super) workspace_transfer_task: Option<Task<()>>,
    pub(super) workspace_transfer_name: Entity<TextEntry>,
    pub(super) environment_names: Vec<String>,
    pub(super) responses: Vec<Option<HttpResponse>>,
    pub(super) storage_error: Option<String>,
    pub(super) storage_dirty: bool,
    pub(super) focus: FocusHandle,
    pub(super) theme: ThemePreset,
    pub(super) requests: Vec<Request>,
    pub(super) collections: Vec<RequestGroups>,
    pub(super) current_collection: usize,
    pub(super) show_collection_menu: bool,
    pub(super) current_directory: String,
    pub(super) sidebar_width: f32,
    pub(super) sidebar_resize_start: Option<(gpui::Pixels, f32)>,
    pub(super) request_config_height: Option<f32>,
    pub(super) request_resize_start: Option<(gpui::Pixels, f32)>,
    pub(super) collapsed_directories: HashSet<(usize, String)>,
    pub(super) hovered_tree_row: Option<String>,
    pub(super) tree_drop: Option<(model::TreeNode, model::Placement)>,
    pub(super) tree_menu: Option<(usize, String)>,
    pub(super) request_menu: Option<usize>,
    pub(super) tree_delete: Option<model::TreeNode>,
    pub(super) group_edit: Option<GroupEdit>,
    pub(super) group_dialog_open: bool,
    pub(super) group_name: Entity<TextEntry>,
    pub(super) group_error: Option<String>,
    pub(super) request_name: Entity<TextEntry>,
    pub(super) editing_request_name: bool,
    pub(super) request_name_error: Option<String>,
    pub(super) active: usize,
    pub(super) tabs: Vec<usize>,
    /// 标签页右键菜单：(请求下标, 鼠标位置)
    pub(super) tab_menu: Option<(usize, gpui::Point<gpui::Pixels>)>,
    pub(super) config: usize,
    pub(super) plugin_catalog: Vec<gpui_glint::plugins::Plugin>,
    pub(super) disabled_plugins: HashSet<String>,
    pub(super) plugin_errors: Vec<String>,
    pub(super) registry_name: Entity<TextEntry>,
    pub(super) registry_url: Entity<TextEntry>,
    pub(super) registry_key: Entity<TextEntry>,
    pub(super) registry_filter: Entity<TextEntry>,
    pub(super) registry_sources: Vec<glint_plugin_registry::Source>,
    pub(super) registry_releases: Vec<(
        glint_plugin_registry::Source,
        glint_plugin_registry::Release,
    )>,
    pub(super) registry_task: Option<Task<()>>,
    pub(super) registry_message: Option<String>,
    pub(super) show_registry_dialog: bool,
    pub(super) registry_adding_source: bool,
    pub(super) response_tab: usize,
    pub(super) traces: Vec<Vec<TraceEntry>>,
    pub(super) sent_requests: Vec<Option<String>>,
    pub(super) sent_curls: Vec<Option<String>>,
    pub(super) curl_input: Entity<CodeCard>,
    pub(super) show_curl_import: bool,
    pub(super) curl_import_needs_focus: bool,
    pub(super) curl_error: Option<String>,
    pub(super) request_preview: Entity<CodeCard>,
    pub(super) trace_started: Option<Instant>,
    pub(super) activity: usize,
    pub(super) show_activity: bool,
    pub(super) show_environments: bool,
    pub(super) show_plugins: bool,
    pub(super) plugin_manager_tab: usize,
    pub(super) editing_environment: usize,
    pub(super) editing_variable_collection: Option<String>,
    pub(super) revealed_variables: HashSet<(usize, u64)>,
    pub(super) variable_delete: Option<(usize, u64)>,
    pub(super) variable_search: Entity<TextEntry>,
    pub(super) variable_error: Option<String>,
    pub(super) variable_rows: Vec<Vec<VariableInput>>,
    pub(super) proxy_inputs: Vec<ProxyBindingInput>,
    pub(super) proxies: Vec<environment::ProxyProfile>,
    pub(super) profile_inputs: Vec<ProxyInput>,
    pub(super) profile_names: Vec<Entity<TextEntry>>,
    pub(super) editing_proxy: usize,
    pub(super) show_proxies: bool,
    pub(super) proxy_tab: usize,
    pub(super) proxy_settings_scroll: gpui::ScrollHandle,
    pub(super) proxy_services: std::collections::HashMap<String, proxy_service::RunningProxy>,
    pub(super) proxy_captures:
        std::collections::HashMap<String, std::sync::Arc<proxy_service::CaptureStore>>,
    pub(super) proxy_capture_panel: Entity<proxy_capture_ui::CapturePanel>,
    pub(super) proxy_app_menu: bool,
    pub(super) proxy_browsers: Vec<proxy_service::browser::Browser>,
    pub(super) proxy_certificate_status: String,
    pub(super) proxy_certificate_task: Option<Task<()>>,
    pub(super) variable_table: Entity<DataTable>,
    pub(super) next_variable_row: u64,
    pub(super) history_filter: usize,
    pub(super) selected_history: Option<(usize, usize)>,
    pub(super) history_delete: Option<(usize, usize)>,
    pub(super) current_versions: Vec<usize>,
    pub(super) duplicate_version: Option<version_history::DuplicateVersion>,
    pub(super) filter: Entity<TextEntry>,
    pub(super) history_search: Entity<TextEntry>,
    pub(super) authentication: Entity<SelectField>,
    pub(super) oauth: oauth_ui::Inputs,
    pub(super) oauth_namespace: String,
    pub(super) skip_tls_verification: bool,
    pub(super) endpoint: Entity<SelectField>,
    pub(super) environment: Entity<Select>,
    pub(super) saved_environment: usize,
    pub(super) environment_inputs: Vec<EnvironmentVariables>,
    pub(super) pair_delete: Option<(usize, gpui::EntityId)>,
    pub(super) params: Vec<PairInput>,
    pub(super) headers: Vec<PairInput>,
    pub(super) cookies: Vec<PairInput>,
    pub(super) extractors: Vec<PairInput>,
    pub(super) extractors_table: Entity<DataTable>,
    pub(super) params_table: Entity<DataTable>,
    pub(super) headers_table: Entity<DataTable>,
    pub(super) response_headers_table: Entity<DataTable>,
    pub(super) response_headers_scroll: gpui::ScrollHandle,
    pub(super) response_header_entries: Vec<[Entity<TextEntry>; 2]>,
    pub(super) cookies_table: Entity<DataTable>,
    pub(super) body: Entity<CodeCard>,
    pub(super) body_format: BodyFormat,
    pub(super) form: Vec<PairInput>,
    pub(super) form_table: Entity<DataTable>,
    pub(super) response: Entity<CodeCard>,
    pub(super) response_search: Entity<TextEntry>,
    pub(super) show_response_search: bool,
    pub(super) response_search_index: Option<usize>,
    pub(super) response_search_count: usize,
    pub(super) commands: Entity<CommandPalette>,
    pub(super) response_json: Option<serde_json::Value>,
    pub(super) json_nodes: Vec<json_picker::JsonNode>,
    pub(super) json_pick: Option<usize>,
    pub(super) json_hover: Option<usize>,
    pub(super) json_menu: Option<(json_picker::JsonNode, gpui::Point<gpui::Pixels>)>,
    pub(super) response_data: Option<HttpResponse>,
    pub(super) status: Option<u16>,
    pub(super) pretty: bool,
    pub(super) html_preview: bool,
    #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
    pub(super) html_webview: Option<Entity<gpui_glint::components::WebView>>,
    #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
    pub(super) html_webview_source: Option<String>,
    #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
    pub(super) html_window: Option<gpui::WindowId>,
    #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
    pub(super) html_windows: HashMap<
        gpui::WindowId,
        (
            Option<Entity<gpui_glint::components::WebView>>,
            Option<String>,
        ),
    >,
    pub(super) show_mcp: bool,
    pub(super) mcp_history_search: Entity<TextEntry>,
    pub(super) mcp_history_filter: usize,
    pub(super) mcp_history_rows: Vec<mcp_history::Summary>,
    pub(super) mcp_history_total: usize,
    pub(super) mcp_history_has_more: bool,
    pub(super) mcp_history_before: Option<i64>,
    pub(super) mcp_history_pages: Vec<Option<i64>>,
    pub(super) mcp_history_selected: Option<mcp_history::Record>,
    pub(super) mcp_history_detail: Entity<CodeCard>,
    pub(super) mcp_history_tab: usize,
    pub(super) mcp_history_error: Option<String>,
    pub(super) mcp_server: Option<mcp::Server>,
    pub(super) mcp_task: Option<Task<()>>,
    pub(super) mcp_error: Option<String>,
    pub(super) ws_background:
        std::collections::HashMap<String, background_tasks_ui::WsSessionState>,
    pub(super) ws_background_update: bool,
    pub(super) show_background_tasks: bool,
    pub(super) ws_environment: Option<environment::EnvironmentValues>,
    pub(super) ws_sequence: u64,
    pub(super) ws_history: Option<(String, String, Instant)>,
    pub(super) ws_commands: Option<async_channel::Sender<websocket::Command>>,
    pub(super) ws_connected: bool,
    pub(super) ws_active: bool,
    pub(super) ws_log: std::collections::VecDeque<websocket_ui::WsMessage>,
    pub(super) ws_scroll: gpui::ScrollHandle,
    pub(super) ws_detail_scroll: gpui::ScrollHandle,
    pub(super) ws_selected: Option<u64>,
    pub(super) ws_follow: bool,
    pub(super) ws_follow_latest: bool,
    pub(super) ws_raw: bool,
    pub(super) ws_detail_tab: usize,
    pub(super) ws_menu: Option<websocket_ui::WsMenu>,
    pub(super) ws_decoder_select: Entity<Select>,
    pub(super) ws_format_select: Entity<Select>,
    pub(super) ws_decrypted: Option<Result<String, String>>,
    pub(super) ws_decode_task: Option<Task<()>>,
    pub(super) ws_decode_generation: u64,
    pub(super) ws_output: Entity<CodeCard>,
    pub(super) sending: Option<Task<()>>,
    pub(super) notice: String,
    pub(super) error: Option<String>,
    pub(super) subscriptions: Vec<Subscription>,
    pub(super) row_subscriptions: Vec<Subscription>,
}
impl Workbench {
    pub(super) fn entry(
        value: &str,
        placeholder: &str,
        cx: &mut Context<Self>,
    ) -> Entity<TextEntry> {
        cx.new(|cx| TextEntry::new(cx, placeholder.to_owned(), value.to_owned()).font_size(12.))
    }

    pub(super) fn new(cx: &mut Context<Self>) -> Self {
        let path = storage::Store::default_path().expect("无法确定 SQLite 数据目录");
        let storage = storage::Store::open(&path).expect("无法打开 SQLite 数据库");
        let mut this = Self::with_store(storage, cx);
        this.start_mcp(cx);
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

    pub(super) fn with_store(mut storage: storage::Store, cx: &mut Context<Self>) -> Self {
        let data = storage
            .load()
            .expect("无法加载 SQLite 工作区，已保留原文件");
        Self::with_workspace(storage, data, cx)
    }

    pub(super) fn with_workspace(
        storage: storage::Store,
        mut data: storage::Workspace,
        cx: &mut Context<Self>,
    ) -> Self {
        let migrated_proxies = data.migrate_proxies();
        let workspace_list = storage.workspaces().expect("无法读取工作区列表");
        let update_source = data.update_source;
        let update_source_url = Self::entry(
            update_source.as_ref().map(|s| s.url.as_str()).unwrap_or(""),
            "留空使用默认 GitHub 源",
            cx,
        );
        let update_source_key = Self::entry(
            update_source
                .as_ref()
                .map(|s| s.public_key.as_str())
                .unwrap_or(""),
            "分发服务的可信公钥",
            cx,
        );
        let selected_environment = data.selected_environment;
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
        let oauth_namespace = storage
            .connection
            .path()
            .and_then(|path| std::fs::canonicalize(path).ok())
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let oauth_namespace = if storage.workspace_id == "default" {
            oauth_namespace
        } else {
            format!("{oauth_namespace}:{}", storage.workspace_id)
        };
        let oauth = oauth_ui::Inputs::new(cx);
        let authentication = cx.new(|cx| {
            SelectField::new(
                [
                    SelectOption::new("bearer", "Bearer"),
                    SelectOption::new("none", "No Auth"),
                    SelectOption::new("oauth2", "OAuth 2.0"),
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
                [
                    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "WS",
                ]
                .map(|s| SelectOption::new(s, s).color(method_color(s))),
                "https://",
                cx,
            )
        });
        let environment = cx.new(|cx| {
            Select::new(
                environment_names
                    .iter()
                    .enumerate()
                    .map(|(i, label)| SelectOption::new(i.to_string(), label.clone())),
                cx,
            )
            .selected(selected_environment.to_string())
            .width(150.)
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
                .viewport()
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
        for field in &oauth.fields {
            subscriptions.push(cx.observe(field, |this: &mut Self, _, cx| {
                this.oauth.cancel();
                this.oauth.ready = None;
                this.oauth.error = None;
                cx.notify();
            }));
        }
        subscriptions.push(cx.observe(&authentication, |_, _, cx| cx.notify()));
        subscriptions.push(cx.subscribe(
            &authentication,
            |this: &mut Self, field, event: &SelectFieldEvent, cx| {
                this.oauth.cancel();
                this.oauth.ready = None;
                let SelectFieldEvent::Changed(value) = event;
                let disabled = value.selected.as_deref() != Some("bearer");
                field.update(cx, |field, cx| field.set_input_disabled(disabled, cx));
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe(
            &environment,
            |this: &mut Self, _, event: &SelectEvent, cx| {
                let SelectEvent::Changed(value) = event;
                this.oauth.cancel();
                this.oauth.ready = None;
                this.oauth.message = "环境已切换，发送时检查该环境的授权".into();
                this.environment.update(cx, |select, cx| {
                    select.set_selected(Some(value.clone()), cx)
                });
                this.notice = format!(
                    "已切换到{}",
                    this.environment_name(value.parse::<usize>().unwrap_or(0))
                );
                this.sync_reference_inputs(cx);
                this.ws_invalidate_titles();
                this.ws_select_sync(cx);
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe(
            &commands,
            |this: &mut Self, _, event: &CommandPaletteEvent, cx| {
                if let CommandPaletteEvent::Executed(id) = event {
                    match id.as_ref() {
                        "import-curl" => this.open_curl_import(cx),
                        "import-workspace" => {
                            this.prepare_workspace_transfer(workspace_transfer_ui::Mode::Import, cx)
                        }
                        "export-workspace" => {
                            this.prepare_workspace_transfer(workspace_transfer_ui::Mode::Export, cx)
                        }
                        "mcp-config" => match mcp::configuration() {
                            Ok(config) => {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    serde_json::to_string_pretty(&config).unwrap(),
                                ));
                                this.notice = "已复制 MCP 配置，可粘贴到 agent 客户端".into();
                                cx.notify();
                            }
                            Err(error) => {
                                this.notice = error;
                                cx.notify();
                            }
                        },
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
        let form_table = cx.new(|cx| {
            make_table(cx)
                .empty_row("暂无表单字段。")
                .add_row("添加表单字段")
        });
        let extractors_table = cx.new(|cx| {
            DataTable::new(
                cx,
                [
                    TableColumn::new("", 48.),
                    TableColumn::new("环境变量名", 160.).flexible(),
                    TableColumn::new("JSONPath", 240.).flexible(),
                    TableColumn::new("说明", 140.),
                    TableColumn::new("", 36.),
                ],
                [],
            )
            .document()
            .compact()
            .empty_row("暂无提取规则。")
            .add_row("添加提取规则")
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
        for (kind, table) in [
            (0, &params_table),
            (1, &headers_table),
            (2, &cookies_table),
            (3, &extractors_table),
            (4, &form_table),
        ] {
            subscriptions.push(cx.subscribe(
                table,
                move |this: &mut Self, _, event: &TableEvent, cx| {
                    if let TableEvent::AddRow = event {
                        let row = if kind == 3 {
                            this.extractor_input(&Pair::new("", "", ""), cx)
                        } else {
                            this.pair(&Pair::new("", "", ""), cx)
                        };
                        match kind {
                            1 => this.headers.push(row),
                            2 => this.cookies.push(row),
                            3 => this.extractors.push(row),
                            4 => this.form.push(row),
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
        let current_versions = requests
            .iter()
            .map(|request| {
                let fingerprint = request_fingerprint::Fingerprint::new(&request.draft);
                request
                    .versions
                    .iter()
                    .position(|r| request_fingerprint::Fingerprint::new(&r.draft) == fingerprint)
                    .unwrap_or(0)
            })
            .collect();
        let sent_requests = data.previews;
        let (plugin_catalog, plugin_errors) = plugins::catalog();
        let disabled_plugins = storage.disabled_plugins().expect("无法读取插件启用状态");
        let available_plugins = plugin_catalog
            .iter()
            .filter(|p| !disabled_plugins.contains(&p.family_identity()))
            .cloned()
            .collect::<Vec<_>>();
        let ws_format_select = cx.new(|cx| {
            Select::new(
                [
                    SelectOption::new("pretty", "格式化"),
                    SelectOption::new("raw", "原文"),
                ],
                cx,
            )
            .selected("pretty")
            .width(90.)
        });
        subscriptions.push(cx.subscribe(
            &ws_format_select,
            |this: &mut Self, select, event: &SelectEvent, cx| {
                let SelectEvent::Changed(value) = event;
                this.ws_raw = value.as_ref() == "raw";
                this.capture_draft(cx);
                select.update(cx, |select, cx| {
                    select.set_selected(Some(value.clone()), cx)
                });
                this.ws_sync_detail(cx);
                cx.notify();
            },
        ));
        let ws_decoder_select = cx.new(|cx| {
            Select::new(websocket_ui::decoder_options(&available_plugins), cx)
                .selected("")
                .width(210.)
        });
        subscriptions.push(cx.subscribe(
            &ws_decoder_select,
            |this: &mut Self, select, event: &SelectEvent, cx| {
                let SelectEvent::Changed(value) = event;
                let binding = if value.is_empty() {
                    None
                } else {
                    let Some((_, binding, _)) =
                        websocket_ui::decoder_bindings(&this.enabled_plugin_catalog())
                            .into_iter()
                            .find(|(v, _, _)| v == value.as_ref())
                    else {
                        return;
                    };
                    Some(binding)
                };
                if let Some(request) = this.requests.get_mut(this.active) {
                    request.draft.event_decoder = binding;
                }
                select.update(cx, |select, cx| {
                    select.set_selected(Some(value.clone()), cx)
                });
                this.storage_dirty = true;
                this.ws_invalidate_titles();
                this.ws_select_sync(cx);
                cx.notify();
            },
        ));
        let registry_name = Self::entry("", "源名称，如 company", cx);
        let registry_url = Self::entry("", "https://plugins.example.com", cx);
        let registry_key = Self::entry("", "服务公钥（Base64，需从可信渠道确认）", cx);
        let registry_filter = Self::entry("", "筛选远程插件：名称、ID、源或版本", cx);
        subscriptions.push(cx.observe(&registry_filter, |_, _, cx| cx.notify()));
        let registry_sources = plugins::registry_store()
            .and_then(|store| store.sources())
            .unwrap_or_default();
        let proxy_certificate_status = proxy_listener_ui::certificate_status(&storage);
        let mut this = Self {
            request_editors: Default::default(),
            request_binding: None,
            toasts: cx.new(|_| ToastHost::new()),
            update_state: update_ui::UpdateState::Idle,
            update_task: None,
            show_update_details: false,
            update_error: None,
            update_source,
            update_source_url,
            update_source_key,
            show_update_source_settings: false,
            workspace_list,
            show_workspaces: false,
            workspace_name_input: Self::entry("", "工作区名称", cx),
            workspace_edit: None,
            workspace_delete: None,
            workspace_error: None,
            workspace_transfer: None,
            workspace_transfer_task: None,
            workspace_transfer_name: Self::entry("", "导入后的工作区名称", cx),
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
            directory_settings: None,
            directory_subscription: None,
            version_tag_edit: None,
            version_tag_input: Self::entry("", "例如：联调通过、上线基线", cx),
            current_directory: String::new(),
            sidebar_width: 220.,
            sidebar_resize_start: None,
            request_config_height: None,
            request_resize_start: None,
            collapsed_directories: HashSet::new(),
            hovered_tree_row: None,
            tree_drop: None,
            tree_menu: None,
            request_menu: None,
            tree_delete: None,
            group_name,
            group_edit: None,
            group_dialog_open: false,
            group_error: None,
            request_name,
            editing_request_name: false,
            request_name_error: None,
            active: 0,
            tabs,
            tab_menu: None,
            config: 0,
            plugin_catalog,
            disabled_plugins,
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
            sent_curls: data.curls,
            curl_input: cx.new(|cx| CodeCard::new(cx, "bash", "").show_header(false)),
            show_curl_import: false,
            curl_import_needs_focus: false,
            curl_error: None,
            request_preview,
            trace_started: None,
            activity: 0,
            show_activity: false,
            show_environments: false,
            show_plugins: false,
            plugin_manager_tab: 0,
            editing_variable_collection: None,
            editing_environment: 0,
            revealed_variables: HashSet::new(),
            variable_delete: None,
            variable_search,
            variable_error: None,
            variable_rows: vec![vec![]; environment_count],
            proxy_inputs: vec![],
            proxies: data.proxies,
            profile_inputs: vec![],
            profile_names: vec![],
            editing_proxy: 0,
            show_proxies: false,
            proxy_tab: 0,
            proxy_settings_scroll: gpui::ScrollHandle::new(),
            proxy_services: Default::default(),
            proxy_captures: Default::default(),
            proxy_capture_panel: cx.new(proxy_capture_ui::CapturePanel::new),
            proxy_app_menu: false,
            proxy_browsers: proxy_service::browser::discover(),
            proxy_certificate_status,
            proxy_certificate_task: None,
            variable_table,
            next_variable_row: 0,
            history_filter: 0,
            selected_history: None,
            history_delete: None,
            current_versions,
            duplicate_version: None,
            filter,
            history_search,
            authentication,
            oauth,
            oauth_namespace,
            skip_tls_verification: false,
            endpoint,
            environment,
            saved_environment: selected_environment,
            environment_inputs,
            pair_delete: None,
            params: vec![],
            headers: vec![],
            cookies: vec![],
            extractors: vec![],
            extractors_table,
            params_table,
            headers_table,
            response_headers_table,
            response_headers_scroll: gpui::ScrollHandle::new(),
            response_header_entries: vec![],
            cookies_table,
            body,
            body_format: BodyFormat::Raw,
            form: vec![],
            form_table,
            response,
            response_search,
            show_response_search: false,
            response_search_index: None,
            response_search_count: 0,
            commands,
            response_json: None,
            json_nodes: vec![],
            json_pick: None,
            json_hover: None,
            json_menu: None,
            response_data: None,
            status: None,
            pretty: true,
            html_preview: false,
            #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
            html_webview: None,
            #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
            html_webview_source: None,
            #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
            html_window: None,
            #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
            html_windows: HashMap::new(),
            show_mcp: false,
            mcp_history_search: Self::entry("", "搜索工具、客户端或错误…", cx),
            mcp_history_filter: 0,
            mcp_history_rows: vec![],
            mcp_history_total: 0,
            mcp_history_has_more: false,
            mcp_history_before: None,
            mcp_history_pages: vec![],
            mcp_history_selected: None,
            mcp_history_detail: cx.new(|cx| {
                CodeCard::new(cx, "json", "")
                    .read_only(true)
                    .show_header(false)
                    .viewport()
            }),
            mcp_history_tab: 0,
            mcp_history_error: None,
            mcp_server: None,
            mcp_task: None,
            mcp_error: None,
            ws_background: Default::default(),
            ws_background_update: false,
            show_background_tasks: false,
            ws_environment: None,
            ws_sequence: 0,
            ws_history: None,
            ws_commands: None,
            ws_connected: false,
            ws_active: false,
            ws_log: Default::default(),
            ws_scroll: gpui::ScrollHandle::new(),
            ws_detail_scroll: gpui::ScrollHandle::new(),
            ws_selected: None,
            ws_follow: true,
            ws_follow_latest: true,
            ws_raw: false,
            ws_detail_tab: 0,
            ws_menu: None,
            ws_decoder_select,
            ws_format_select,
            ws_decrypted: None,
            ws_decode_task: None,
            ws_decode_generation: 0,
            ws_output: cx.new(|cx| {
                CodeCard::new(cx, "text", "")
                    .read_only(true)
                    .show_header(false)
                    .prose(true)
                    .viewport()
            }),
            sending: None,
            notice: "HTTP / WebSocket · 请求与版本自动保存到 SQLite".into(),
            error: None,
            subscriptions,
            row_subscriptions: vec![],
        };
        let mut history_query = String::new();
        this.subscriptions.push(
            cx.observe(&this.mcp_history_search, move |this, entry, cx| {
                let query = entry.read(cx).value().to_owned();
                if query != history_query {
                    history_query = query;
                    this.mcp_history_before = None;
                    this.mcp_history_pages.clear();
                    this.reload_mcp_history(cx);
                }
            }),
        );
        this.reset_proxy_profiles(cx);
        this.storage_dirty |= migrated_proxies;
        this.reset_variable_rows(cx);
        this.load(&initial, cx);
        this.restore_response(cx);
        let subscription = cx.observe_self(|this, cx| this.persist(cx));
        this.subscriptions.push(subscription);
        this.set_theme(ThemePreset::Moonstone, cx);
        this
    }
}
