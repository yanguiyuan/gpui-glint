use super::*;
use gpui_glint::components::{CollapsePanel, InputField, MenuItem, PromptEditor, Skeleton, Switch};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq)]
enum ChatGptAuth {
    Login,
    Check,
    SignOut,
}

fn response_timeout_content(
    entry: Entity<TextEntry>,
    theme: ThemePreset,
) -> impl Fn(&mut Window, &mut App) -> gpui::AnyElement {
    move |_, _| {
        let muted = theme.palette().text_muted;
        div().flex().flex_col().gap_2()
            .debug_selector(|| "provider-response-timeout".to_owned())
            .child(
                div().flex().items_center().gap_4()
                    .child(
                        div().w(px(128.)).flex_shrink_0().text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM).text_color(rgb(muted))
                            .child("响应空闲超时（秒）"),
                    )
                    .child(
                        div().flex_1().min_w_0()
                            .child(InputField::new("").control(entry.clone()).theme(theme).render().w_full()),
                    ),
            )
            .child(div().pl(px(144.)).text_size(px(11.)).text_color(rgb(muted))
                .child("连续未收到数据时的等待上限，默认 180 秒，可设 1–86400 秒。持续输出时不限制总时长。"))
            .into_any_element()
    }
}

fn parse_response_timeout(value: &str) -> Result<u64, String> {
    value
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|seconds| (1..=86400).contains(seconds))
        .ok_or_else(|| "响应空闲超时必须为 1–86400 秒的整数".into())
}

fn parse_context_window(value: &str) -> Result<Option<u64>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let cleaned = value.replace([',', '_'], "");
    let (number, multiplier) = match cleaned.chars().last() {
        Some('k' | 'K') => (&cleaned[..cleaned.len() - 1], 1_000_u64),
        Some('m' | 'M') => (&cleaned[..cleaned.len() - 1], 1_000_000_u64),
        _ => (cleaned.as_str(), 1_u64),
    };
    let parse = || -> Option<u64> {
        let (whole, fraction) = number.trim().split_once('.').unwrap_or((number.trim(), ""));
        if whole.is_empty()
            || !whole.bytes().all(|ch| ch.is_ascii_digit())
            || !fraction.bytes().all(|ch| ch.is_ascii_digit())
        {
            return None;
        }
        let base = whole.parse::<u64>().ok()?.checked_mul(multiplier)?;
        let extra = if fraction.is_empty() {
            0
        } else {
            let divisor = 10_u64.checked_pow(u32::try_from(fraction.len()).ok()?)?;
            let scaled = fraction.parse::<u64>().ok()?.checked_mul(multiplier)?;
            if scaled % divisor != 0 {
                return None;
            }
            scaled / divisor
        };
        base.checked_add(extra).filter(|value| *value > 0)
    };
    parse()
        .map(Some)
        .ok_or_else(|| "请输入正数 token 数，可用 256K、1M，或留空自动获取".into())
}

/// Index of the \"窗口外观\" segment that matches the current window theme. Every light
/// preset selects 浅色 and every dark preset selects 深色; only the two anchors are offered.
fn appearance_segment(theme: ThemePreset) -> usize {
    usize::from(theme.is_dark())
}

fn available_model_row(
    model: &str,
    added: bool,
    theme: ThemePreset,
    on_add: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let selector: SharedString = format!("add-model:{model}").into();
    div()
        .id(SharedString::from(format!("available-model:{model}")))
        .px_4()
        .py_2()
        .flex()
        .items_center()
        .gap_3()
        .child(div().flex_1().child(model.to_owned()))
        .child(
            Button::new(if added { "已添加" } else { "添加" })
                .icon(if added { Icon::CheckCircle } else { Icon::Plus })
                .kind(ButtonKind::Plain)
                .size(ButtonSize::Mini)
                .theme(theme)
                .when(added, |button| button.disabled())
                .render()
                .debug_selector(move || selector.to_string())
                .when(!added, |button| button.on_click(on_add)),
        )
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    General,
    Appearance,
    Layout,
    Storage,
    Shortcuts,
    Memory,
    Skills,
    Mcp,
    Tools,
    Provider,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Self::General => "常规",
            Self::Appearance => "外观",
            Self::Layout => "布局",
            Self::Storage => "存储",
            Self::Shortcuts => "键盘快捷键",
            Self::Memory => "记忆",
            Self::Skills => "技能",
            Self::Mcp => "MCP",
            Self::Tools => "工具",
            Self::Provider => "供应商",
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
pub enum ProviderTab {
    #[default]
    Connection,
    Models,
}

pub struct Settings {
    pub shortcut_recording: Option<crate::shortcuts::ProjectAction>,
    pub shortcut_message: Option<String>,
    pub compaction_threshold: Entity<TextEntry>,
    compaction_message: Option<String>,
    /// Optional override for the commit-message system prompt.
    pub commit_prompt: Entity<PromptEditor>,
    pub discovered: Vec<discovery::DiscoveredProvider>,
    pub selected: Option<Uuid>,
    original: provider::Config,
    drafts: HashMap<Uuid, (provider::Config, String, bool)>,
    pub models: Vec<String>,
    default_model: String,
    available_models: Vec<String>,
    context_windows: HashMap<String, u64>,
    context_origin: String,
    window_inputs: HashMap<String, Entity<TextEntry>>,
    new_model_window: Entity<TextEntry>,
    pub discovering: Option<Task<()>>,
    pub section: Section,
    /// Whether the "+" dropdown in the provider list header is open.
    add_menu_open: bool,
    /// Provider awaiting confirmation in the remove dialog.
    pub(crate) confirming_remove: Option<Uuid>,
    tab: ProviderTab,
    pub search: Entity<TextEntry>,
    pub kind: provider::Kind,
    protocol: provider::Protocol,
    pub name: Entity<TextEntry>,
    pub url: Entity<TextEntry>,
    response_timeout: Entity<TextEntry>,
    more_settings: Entity<CollapsePanel>,
    pub model: Entity<TextEntry>,
    pub key: Entity<TextEntry>,
    pub key_env: Entity<TextEntry>,
    use_env: bool,
    pub message: Option<String>,
    pub testing: Option<Task<()>>,
    pub auth_task: Option<Task<()>>,
    chatgpt_account: Option<String>,
}

impl Settings {
    pub fn new(
        config: Option<&provider::Config>,
        discovered: Vec<discovery::DiscoveredProvider>,
        cx: &mut Context<AgentWorkspace>,
    ) -> Self {
        let selected = config.map(|config| config.id);
        let config = config.cloned().unwrap_or_default();
        let response_timeout = cx.new(|cx| {
            TextEntry::new(
                cx,
                "默认 180 秒",
                config.response_timeout_seconds.to_string(),
            )
            .theme(ThemePreset::Moonstone)
        });
        let more_settings = cx.new(|_| {
            CollapsePanel::new("更多设置")
                .compact()
                .guard_line(true)
                .theme(ThemePreset::Moonstone)
                .content(response_timeout_content(
                    response_timeout.clone(),
                    ThemePreset::Moonstone,
                ))
        });
        let commit_prompt = cx.new(|cx| {
            PromptEditor::new(cx, commit_message::DEFAULT_PROMPT)
                .theme(ThemePreset::Moonstone)
                .min_rows(3)
        });
        let mut entry = |hint: &str, value: &str| {
            cx.new(|cx| {
                TextEntry::new(cx, hint.to_owned(), value.to_owned()).theme(ThemePreset::Moonstone)
            })
        };
        Self {
            shortcut_recording: None,
            shortcut_message: None,
            compaction_threshold: entry("1–99，例如 75", "75"),
            compaction_message: None,
            commit_prompt,
            selected,
            discovered,
            models: config.models.clone(),
            default_model: config.model.clone(),
            drafts: HashMap::new(),
            available_models: vec![],
            context_windows: config.model_context_windows.clone(),
            context_origin: config.base_url.clone(),
            window_inputs: config
                .models
                .iter()
                .map(|model| {
                    let value = config
                        .context_window_overrides
                        .get(model)
                        .map(u64::to_string)
                        .unwrap_or_default();
                    (model.clone(), entry("自动获取", &value))
                })
                .collect(),
            new_model_window: entry("例如 1M，留空自动", ""),
            discovering: None,
            section: Section::General,
            add_menu_open: false,
            confirming_remove: None,
            tab: ProviderTab::Connection,
            kind: config.kind,
            protocol: config.protocol,
            search: entry("搜索设置或供应商…", ""),
            name: entry("供应商名称", &config.name),
            url: entry("https://your-provider.example/v1", &config.base_url),
            response_timeout,
            more_settings,
            model: entry("输入模型 ID", ""),
            key: entry("输入或替换 API Key", ""),
            key_env: entry(
                "例如 DEEPSEEK_API_KEY",
                config.api_key_env.as_deref().unwrap_or(""),
            ),
            use_env: config.api_key_env.is_some(),
            chatgpt_account: chatgpt::saved_account_label(&config),
            original: config,
            message: None,
            testing: None,
            auth_task: None,
        }
    }

    pub(crate) fn load(
        &mut self,
        config: provider::Config,
        key: String,
        use_env: bool,
        cx: &mut Context<AgentWorkspace>,
    ) {
        self.more_settings
            .update(cx, |panel, cx| panel.set_expanded(false, cx));
        self.discovering = None;
        self.testing = None;
        self.auth_task = None;
        self.selected = Some(config.id);
        self.chatgpt_account = chatgpt::saved_account_label(&config);
        self.kind = config.kind;
        self.protocol = config.protocol;
        self.models = config.models.clone();
        self.context_windows = config.model_context_windows.clone();
        self.context_origin = config.base_url.clone();
        self.window_inputs = config
            .models
            .iter()
            .map(|model| {
                let value = config
                    .context_window_overrides
                    .get(model)
                    .map(u64::to_string)
                    .unwrap_or_default();
                let entry = cx
                    .new(|cx| TextEntry::new(cx, "自动获取", value).theme(ThemePreset::Moonstone));
                (model.clone(), entry)
            })
            .collect();
        self.new_model_window
            .update(cx, |entry, cx| entry.set_value("", cx));
        self.default_model = config.model.clone();
        self.use_env = use_env;
        self.available_models.clear();
        self.message = None;
        self.name
            .update(cx, |entry, cx| entry.set_value(config.name.clone(), cx));
        self.url
            .update(cx, |entry, cx| entry.set_value(config.base_url.clone(), cx));
        self.response_timeout.update(cx, |entry, cx| {
            entry.set_value(config.response_timeout_seconds.to_string(), cx)
        });
        self.key_env.update(cx, |entry, cx| {
            entry.set_value(config.api_key_env.clone().unwrap_or_default(), cx)
        });
        self.key.update(cx, |entry, cx| entry.set_value(key, cx));
        self.model.update(cx, |entry, cx| entry.set_value("", cx));
        self.original = config;
    }

    pub fn set_theme(&self, theme: ThemePreset, cx: &mut Context<AgentWorkspace>) {
        self.more_settings.update(cx, |panel, cx| {
            panel.set_theme(theme, cx);
            panel.set_content(
                response_timeout_content(self.response_timeout.clone(), theme),
                cx,
            );
        });
        for entry in self.window_inputs.values() {
            entry.update(cx, |entry, cx| entry.set_theme(theme, cx));
        }
        self.commit_prompt.update(cx, |editor, cx| {
            editor.set_theme(theme, cx);
        });
        for entry in [
            &self.search,
            &self.compaction_threshold,
            &self.name,
            &self.url,
            &self.response_timeout,
            &self.model,
            &self.new_model_window,
            &self.key,
            &self.key_env,
        ] {
            entry.update(cx, |entry, cx| entry.set_theme(theme, cx));
        }
    }
}

impl AgentWorkspace {
    /// The "+" menu above the provider list: an API provider or a ChatGPT subscription.
    fn add_provider_menu(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let content = div()
            .flex()
            .flex_col()
            .child(
                MenuItem::new("add-api-provider", "添加 API 供应商")
                    .icon(Icon::Globe)
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "add-api-provider".into())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.provider_settings.add_menu_open = false;
                        this.add_provider(provider::Kind::Compatible, window, cx);
                    })),
            )
            .child(
                MenuItem::new("add-chatgpt-provider", "连接 ChatGPT 订阅")
                    .icon(Icon::User)
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "add-chatgpt-provider".into())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.provider_settings.add_menu_open = false;
                        this.add_provider(provider::Kind::ChatGpt, window, cx);
                    })),
            )
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("订阅授权保存在本机，无需 API Key"),
            );
        let trigger = IconButton::new(Icon::Plus, "添加供应商")
            .plain()
            .theme(self.theme)
            .render()
            .debug_selector(|| "add-provider-trigger".into());
        let weak = cx.entity().downgrade();
        div().flex_shrink_0().child(
            Popover::new("provider-add-menu", trigger, content)
                .open(self.provider_settings.add_menu_open)
                .width(196.)
                .padding(4.)
                .theme(self.theme)
                .on_open_change(move |open, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.provider_settings.add_menu_open = open;
                        cx.notify();
                    });
                }),
        )
    }

    fn catalog_path(&self) -> PathBuf {
        self.path.with_file_name("providers.json")
    }

    pub(super) fn sync_provider(&mut self, cx: &mut Context<Self>) {
        self.provider = self.catalog.current();
        let chip = model_chip(&self.catalog);
        self.composer.update(cx, |composer, cx| {
            composer.set_menu_chip(chip, cx);
            composer.set_disabled(
                !self.active_model_enabled() || self.active_run().preparing.is_some(),
                cx,
            );
        });
        self.sync_context_usage(cx);
        self.context_models = None;
        self.refresh_context_models(cx);
        cx.notify();
    }

    pub(super) fn cache_context_windows(
        &mut self,
        id: Uuid,
        origin: &str,
        models: Vec<String>,
        windows: HashMap<String, u64>,
        cx: &mut Context<Self>,
    ) {
        let Some(profile) = self
            .catalog
            .providers
            .iter_mut()
            .find(|profile| profile.id == id && profile.base_url == origin)
        else {
            return;
        };
        let changed = profile.model_context_windows != windows;
        profile.model_context_windows = windows.clone();
        if changed {
            let _ = self.catalog.save(&self.catalog_path());
        }
        if self.provider_settings.selected == Some(id)
            && self
                .provider_settings
                .url
                .read(cx)
                .value()
                .trim()
                .trim_end_matches('/')
                == origin
        {
            self.provider_settings.available_models = models;
            self.provider_settings.context_windows = windows;
            self.provider_settings.context_origin = origin.to_owned();
        }
        if self.provider.id == id {
            self.provider = self.catalog.current();
        }
        self.sync_context_usage(cx);
        cx.notify();
    }

    pub(super) fn choose_model(&mut self, value: &str, cx: &mut Context<Self>) {
        let result = (|| {
            if self.session_busy(self.history.active) {
                return Err("请先停止当前回复，再切换模型".to_owned());
            }
            let (id, model) = value.split_once(':').ok_or("模型选择无效")?;
            let id = Uuid::parse_str(id).map_err(|_| "供应商 ID 无效")?;
            let mut next = self.catalog.clone();
            next.activate(id, model.to_owned())?;
            next.save(&self.catalog_path())?;
            self.catalog = next;
            Ok::<_, String>(())
        })();
        self.provider_error = result.err();
        self.sync_provider(cx);
    }

    fn edited_provider(&self, cx: &App) -> provider::Config {
        let settings = &self.provider_settings;
        let url = settings
            .url
            .read(cx)
            .value()
            .trim()
            .trim_end_matches('/')
            .to_owned();
        let mut config = settings.original.clone();
        config.credential_store = Some(self.path.clone());
        config.kind = settings.kind;
        config.protocol = settings.protocol;
        config.enabled = true;
        config.response_timeout_seconds =
            parse_response_timeout(settings.response_timeout.read(cx).value())
                .unwrap_or(settings.original.response_timeout_seconds);
        config.name = settings.name.read(cx).value().trim().to_owned();
        config.model = settings.default_model.clone();
        config.models = settings.models.clone();
        config.model_context_windows = if settings.context_origin == url {
            settings.context_windows.clone()
        } else {
            HashMap::new()
        };
        config.context_window_overrides = settings
            .models
            .iter()
            .filter_map(|model| {
                let input = settings.window_inputs.get(model)?;
                parse_context_window(input.read(cx).value())
                    .ok()
                    .flatten()
                    .map(|value| (model.clone(), value))
            })
            .collect();
        config.api_key_env = if settings.use_env {
            Some(settings.key_env.read(cx).value().trim().to_owned())
        } else {
            None
        };
        if url != config.base_url {
            config.credential_id = Uuid::new_v4();
            let old = reqwest::Url::parse(&config.base_url).ok();
            let new = reqwest::Url::parse(&url).ok();
            if old.as_ref().map(|url| url.origin()) != new.as_ref().map(|url| url.origin()) {
                config.models_url = None;
            }
        }
        config.base_url = url;
        config
    }

    fn stash_provider(&mut self, cx: &App) {
        if let Some(id) = self.provider_settings.selected {
            let config = self.edited_provider(cx);
            let key = self.provider_settings.key.read(cx).value().to_owned();
            self.provider_settings
                .drafts
                .insert(id, (config, key, self.provider_settings.use_env));
        }
    }

    fn select_provider(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.stash_provider(cx);
        let draft = self.provider_settings.drafts.get(&id).cloned().or_else(|| {
            self.catalog
                .providers
                .iter()
                .find(|provider| provider.id == id)
                .cloned()
                .map(|config| {
                    let use_env = config.api_key_env.is_some();
                    (config, String::new(), use_env)
                })
        });
        if let Some((config, key, use_env)) = draft {
            self.provider_settings.load(config, key, use_env, cx);
            self.provider_settings.section = Section::Provider;
            window.focus(&self.provider_settings.name.focus_handle(cx), cx);
            cx.notify();
        }
    }

    fn add_provider(&mut self, kind: provider::Kind, window: &mut Window, cx: &mut Context<Self>) {
        self.stash_provider(cx);
        let config = provider::Config {
            kind,
            credential_store: Some(self.path.clone()),
            name: if kind == provider::Kind::ChatGpt {
                "ChatGPT"
            } else {
                "新供应商"
            }
            .into(),
            ..provider::Config::default()
        };
        let id = config.id;
        self.catalog.providers.push(config.clone());
        self.provider_settings
            .load(config, String::new(), false, cx);
        self.provider_settings.section = Section::Provider;
        self.provider_settings.tab = ProviderTab::Connection;
        window.focus(&self.provider_settings.name.focus_handle(cx), cx);
        debug_assert_eq!(self.provider_settings.selected, Some(id));
        cx.notify();
    }

    /// 侧栏的「移除」先弹确认框，避免误删。
    fn request_remove_provider(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.provider_settings.confirming_remove = Some(id);
        cx.notify();
    }

    fn dismiss_remove_provider(&mut self, cx: &mut Context<Self>) {
        self.provider_settings.confirming_remove = None;
        cx.notify();
    }

    /// 删除供应商：清掉本地密钥和 ChatGPT 授权，正在使用的会自动换到别的供应商。
    fn remove_provider(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.provider_settings.confirming_remove != Some(id) {
            return;
        }
        self.provider_settings.confirming_remove = None;
        let mut warning = None;
        let next = (|| {
            if self.provider_file_state.is_injected(id) {
                return Err("这个供应商来自供应商配置文件，请在 Magpie 里移除".to_owned());
            }
            if self.catalog.active == Some(id) && self.any_run_busy() {
                return Err("请先停止当前回复，再移除正在使用的供应商".to_owned());
            }
            let mut catalog = self.catalog.clone();
            let removed = catalog.remove(id).ok_or("供应商不存在")?;
            if removed.kind == provider::Kind::ChatGpt {
                warning = chatgpt::forget(&removed).err();
            }
            if let Err(error) = removed.remove_key() {
                warning.get_or_insert(error);
            }
            catalog.save(&self.catalog_path())?;
            Ok::<_, String>(catalog)
        })();
        match next {
            Ok(catalog) => {
                self.catalog = catalog;
                self.provider_settings.drafts.remove(&id);
                self.sync_provider(cx);
                if self.provider_settings.selected == Some(id) {
                    match self.catalog.providers.first().cloned() {
                        Some(config) => {
                            self.provider_settings
                                .load(config, String::new(), false, cx)
                        }
                        None => {
                            self.provider_settings.selected = None;
                            self.provider_settings.kind = provider::Kind::Compatible;
                        }
                    }
                }
                self.provider_settings.message =
                    Some(warning.unwrap_or_else(|| "已移除供应商。".into()));
            }
            Err(error) => self.provider_settings.message = Some(error),
        }
        cx.notify();
    }

    pub(super) fn provider_remove_dialog(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(id) = self.provider_settings.confirming_remove else {
            return gpui::div().into_any_element();
        };
        let p = self.theme.palette();
        let name = self
            .catalog
            .providers
            .iter()
            .find(|provider| provider.id == id)
            .map(|provider| provider.name.clone())
            .unwrap_or_else(|| "这个供应商".into());
        let active = self.catalog.active == Some(id);
        let body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("移除“{name}”？")),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .line_height(px(24.))
                    .text_color(rgb(p.text_muted))
                    .child(if active {
                        "将删除这个供应商的配置和已保存的密钥，当前对话会切换到其他可用的供应商。聊天记录会保留。"
                    } else {
                        "将删除这个供应商的配置和已保存的密钥。聊天记录会保留。"
                    }),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_3()
                    .child(
                        Button::new("取消")
                            .kind(ButtonKind::Plain)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "cancel-remove-provider".into())
                            .on_click(cx.listener(|this, _, _, cx| this.dismiss_remove_provider(cx))),
                    )
                    .child(
                        Button::new("移除供应商")
                            .kind(ButtonKind::Danger)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "confirm-remove-provider".into())
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.remove_provider(id, cx)),
                            ),
                    ),
            );
        let weak = cx.entity().downgrade();
        gpui_glint::components::Dialog::new("provider-remove-dialog", body)
            .width(460.)
            .theme(self.theme)
            .on_close(move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| this.dismiss_remove_provider(cx));
            })
            .render()
    }

    fn save_provider(&mut self, activate: bool, cx: &mut Context<Self>) {
        let config = self.edited_provider(cx);
        let result = (|| {
            parse_response_timeout(self.provider_settings.response_timeout.read(cx).value())?;
            for model in &self.provider_settings.models {
                if let Some(input) = self.provider_settings.window_inputs.get(model) {
                    parse_context_window(input.read(cx).value())
                        .map_err(|error| format!("{model}：{error}"))?;
                }
            }
            if (self.session_busy(self.history.active))
                && (activate || self.catalog.active == Some(config.id))
            {
                return Err("请先停止当前回复，再修改正在使用的供应商".into());
            }
            config.validate()?;
            if config.kind == provider::Kind::Compatible
                && config.api_key_env.as_deref() == Some("")
            {
                return Err("请输入 API Key 环境变量名，或切换到 API Key 认证".into());
            }
            if config.kind == provider::Kind::Compatible && !self.provider_settings.use_env {
                let key = self.provider_settings.key.read(cx).value().trim();
                if !key.is_empty() {
                    config.save_key(key)?;
                }
            }
            let mut catalog = self.catalog.clone();
            catalog.upsert(config.clone());
            if activate || catalog.active.is_none() {
                catalog.activate(config.id, config.model.clone())?;
            } else if catalog.active == Some(config.id)
                && !config.models.contains(&catalog.active_model)
            {
                catalog.active_model = config.model.clone();
            }
            catalog.save(&self.catalog_path())?;
            self.catalog = catalog;
            Ok::<_, String>(())
        })();
        match result {
            Ok(()) => {
                self.provider_settings.original = config.clone();
                self.provider_settings.drafts.remove(&config.id);
                self.provider_settings
                    .key
                    .update(cx, |entry, cx| entry.set_value("", cx));
                self.provider_settings.message = Some(
                    if activate {
                        "已保存，当前对话将使用此供应商。"
                    } else {
                        "已保存供应商与模型列表。"
                    }
                    .into(),
                );
                self.provider_error = None;
                self.sync_provider(cx);
            }
            Err(error) => self.provider_settings.message = Some(error),
        }
        cx.notify();
    }

    fn add_model(&mut self, model: String, cx: &mut Context<Self>) {
        let model = model.trim().to_owned();
        if model.is_empty() {
            return;
        }
        if !self.provider_settings.models.contains(&model) {
            self.provider_settings.models.push(model.clone());
            self.provider_settings.window_inputs.insert(
                model.clone(),
                cx.new(|cx| TextEntry::new(cx, "自动获取", "").theme(self.theme)),
            );
        }
        if self.provider_settings.default_model.is_empty() {
            self.provider_settings.default_model = model.clone();
        }
        self.provider_settings.message = Some(format!("已添加 {model}，点击保存后即可用于对话。"));
        self.provider_settings
            .model
            .update(cx, |entry, cx| entry.set_value("", cx));
        cx.notify();
    }

    pub(super) fn refresh_models(&mut self, cx: &mut Context<Self>) {
        if self.provider_settings.discovering.is_some() || self.provider_settings.selected.is_none()
        {
            return;
        }
        let config = self.edited_provider(cx);
        let id = config.id;
        let provided = self
            .provider_settings
            .key
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let result = if config.kind == provider::Kind::ChatGpt {
            let auth_config = config.clone();
            provider::Job::start(async move { chatgpt::models(&auth_config).await })
        } else {
            let key = if !self.provider_settings.use_env && !provided.is_empty() {
                Ok(provided)
            } else {
                config.key()
            };
            key.and_then(|key| {
                provider::Job::start(provider::list_model_catalog(config.clone(), key))
            })
        };
        match result {
            Err(error) => self.provider_settings.message = Some(error),
            Ok(job) => {
                self.provider_settings.message = Some("正在获取模型列表…".into());
                self.provider_settings.discovering = Some(cx.spawn(async move |weak, cx| {
                    let result = job.result().await;
                    let _ = weak.update(cx, |this, cx| {
                        this.provider_settings.discovering = None;
                        if this.provider_settings.selected != Some(id) {
                            return;
                        }
                        if this
                            .provider_settings
                            .url
                            .read(cx)
                            .value()
                            .trim()
                            .trim_end_matches('/')
                            != config.base_url
                        {
                            return;
                        }
                        match result {
                            Ok(models) => {
                                this.provider_settings.available_models = models.models;
                                this.provider_settings.context_windows = models.context_windows;
                                this.provider_settings.context_origin = config.base_url.clone();
                                this.provider_settings.message = None;
                            }
                            Err(error) => this.provider_settings.message = Some(error),
                        }
                        cx.notify();
                    });
                }));
            }
        }
        cx.notify();
    }

    fn test_provider(&mut self, cx: &mut Context<Self>) {
        if self.provider_settings.testing.is_some() {
            self.provider_settings.testing = None;
            self.provider_settings.message = Some("已取消测试".into());
            cx.notify();
            return;
        }
        let config = self.edited_provider(cx);
        let id = config.id;
        let provided = self
            .provider_settings
            .key
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let result = (|| {
            let key = if config.kind == provider::Kind::ChatGpt {
                String::new()
            } else if !self.provider_settings.use_env && !provided.is_empty() {
                provided
            } else {
                config.key()?
            };
            provider::Request::start(
                config,
                key,
                vec![serde_json::json!({"role":"user","content":"Reply with OK."})],
                provider::Thinking::Default,
            )
        })();
        match result {
            Err(error) => self.provider_settings.message = Some(error),
            Ok(request) => {
                self.provider_settings.message = Some("正在测试默认模型…".into());
                self.provider_settings.testing = Some(cx.spawn(async move |weak, cx| {
                    let result = request.response().await;
                    let _ = weak.update(cx, |this, cx| {
                        this.provider_settings.testing = None;
                        if this.provider_settings.selected == Some(id) {
                            this.provider_settings.message = Some(match result {
                                Ok(_) => "连接成功，模型已返回回复。".into(),
                                Err(error) => error,
                            });
                        }
                        cx.notify();
                    });
                }));
            }
        }
        cx.notify();
    }

    fn chatgpt_auth(&mut self, action: ChatGptAuth, cx: &mut Context<Self>) {
        if self.provider_settings.auth_task.is_some() {
            self.provider_settings.auth_task = None;
            cx.notify();
            return;
        }
        let id = self.provider_settings.selected;
        let config = self.edited_provider(cx);
        self.provider_settings.message = Some("正在连接 ChatGPT…".into());
        self.provider_settings.auth_task = Some(cx.spawn(async move |weak, cx| {
            let result = async {
                if action == ChatGptAuth::Login {
                    let (connection, url) =
                        provider::Job::start(async move { chatgpt::start_login(&config).await })?
                            .result()
                            .await?;
                    weak.update(cx, |this, cx| {
                        this.provider_settings.message =
                            Some("请在浏览器中完成 ChatGPT 登录。".into());
                        cx.open_url(&url);
                        cx.notify();
                    })
                    .map_err(|_| "窗口已关闭".to_owned())?;
                    provider::Job::start(chatgpt::finish_login(connection))?
                        .result()
                        .await
                } else if action == ChatGptAuth::SignOut {
                    provider::Job::start(async move { chatgpt::sign_out(&config).await })?
                        .result()
                        .await
                } else {
                    provider::Job::start(async move { chatgpt::account(&config).await })?
                        .result()
                        .await
                }
            }
            .await;
            let _ = weak.update(cx, |this, cx| {
                this.provider_settings.auth_task = None;
                if this.provider_settings.selected == id {
                    this.provider_settings.chatgpt_account =
                        chatgpt::saved_account_label(&this.edited_provider(cx));
                    this.provider_settings.message = Some(match result {
                        Ok(message) | Err(message) => message,
                    });
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn collect_toggleable_tools(&self) -> Vec<(&'static str, Vec<(String, String)>)> {
        let builtin: Vec<(String, String)> = crate::tools::definitions(true)
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| {
                let name = t["name"].as_str()?.to_owned();
                let desc = t["description"].as_str().unwrap_or("").to_owned();
                Some((name, desc))
            })
            .collect();

        let mut db_tools = Vec::new();
        let mut api_tools = Vec::new();
        if let Some(host) = &self.host_tools {
            for tool in host.catalog() {
                let Some(name) = tool["name"].as_str() else {
                    continue;
                };
                let desc = tool["description"].as_str().unwrap_or("").to_owned();
                if name.starts_with("glint_db_") {
                    db_tools.push((name.to_owned(), desc));
                } else {
                    api_tools.push((name.to_owned(), desc));
                }
            }
        }

        let mut groups = vec![("内置工具", builtin)];
        if !db_tools.is_empty() {
            groups.push(("数据库工具", db_tools));
        }
        if !api_tools.is_empty() {
            groups.push(("API 与插件工具", api_tools));
        }
        groups
    }

    fn tools_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let query = self.provider_settings.search.read(cx).value().trim().to_lowercase();
        let groups = self.collect_toggleable_tools();
        let all_names: Vec<String> = groups
            .iter()
            .flat_map(|(_, items)| items.iter().map(|(n, _)| n.clone()))
            .collect();
        let total = all_names.len();
        let enabled_count = all_names
            .iter()
            .filter(|n| !self.history.disabled_tools.contains(*n))
            .count();

        let mut content = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("工具启用"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(p.text_muted))
                                    .child(format!(
                                        "默认全部启用，已启用 {enabled_count} / {total} 个工具（关闭后模型将不可见且无法调用）"
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("全部启用")
                                    .kind(ButtonKind::Glass)
                                    .size(ButtonSize::Small)
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(|| "settings-tools-enable-all".into())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.history.disabled_tools.clear();
                                        this.save(cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("全部停用")
                                    .kind(ButtonKind::Gray)
                                    .size(ButtonSize::Small)
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(|| "settings-tools-disable-all".into())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        for name in &all_names {
                                            this.history.disabled_tools.insert(name.clone());
                                        }
                                        this.save(cx);
                                        cx.notify();
                                    })),
                            ),
                    ),
            );

        for (group_title, items) in groups {
            let filtered: Vec<(String, String)> = items
                .into_iter()
                .filter(|(name, desc)| {
                    query.is_empty()
                        || name.to_lowercase().contains(&query)
                        || desc.to_lowercase().contains(&query)
                })
                .collect();
            if filtered.is_empty() {
                continue;
            }
            let mut card = div()
                .w_full()
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.surface))
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.panel))
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("{group_title} ({})", filtered.len())),
                );

            for (idx, (name, desc)) in filtered.into_iter().enumerate() {
                let enabled = !self.history.disabled_tools.contains(&name);
                let toggle_name = name.clone();
                let weak = cx.entity().downgrade();
                let mut row = div()
                    .id(SharedString::from(format!("settings-tool:{name}")))
                    .px_3()
                    .py_2p5()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3();
                if idx > 0 {
                    row = row.border_t_1().border_color(rgb(p.border));
                }
                row = row
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .font_family("Menlo")
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(p.text))
                                    .child(name),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.text_muted))
                                    .truncate()
                                    .child(desc),
                            ),
                    )
                    .child(
                        Switch::new("")
                            .checked(enabled)
                            .theme(self.theme)
                            .on_change(move |next, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    if next {
                                        this.history.disabled_tools.remove(&toggle_name);
                                    } else {
                                        this.history.disabled_tools.insert(toggle_name.clone());
                                    }
                                    this.save(cx);
                                    cx.notify();
                                });
                            })
                            .render(),
                    );
                card = card.child(row);
            }
            content = content.child(card);
        }

        self.settings_container(content)
    }

    pub fn settings_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        if self.provider_settings.section == Section::Mcp {
            return self.mcp_page(cx);
        }
        if self.provider_settings.section == Section::Tools {
            return self.tools_page(cx);
        }
        if self.provider_settings.section == Section::Skills {
            return self.skills_page(cx);
        }
        if self.provider_settings.section == Section::Memory {
            return self.memory_page(cx);
        }
        if self.provider_settings.section != Section::Provider {
            return self.settings_overview(cx);
        }
        let p = self.theme.palette();
        let settings = &self.provider_settings;
        if settings.selected.is_none() {
            return self.settings_container(
                div()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("供应商"),
                    )
                    .child(
                        div()
                            .text_color(rgb(p.text_muted))
                            .child("从左侧选择供应商，或添加一个新的连接。"),
                    ),
            );
        }
        let chatgpt = settings.kind == provider::Kind::ChatGpt;
        let title = settings.name.read(cx).value().to_owned();
        let mut page = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(28.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        Button::new("保存")
                            .kind(ButtonKind::Glass)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.save_provider(false, cx))),
                    )
                    .child(
                        Button::new(if self.catalog.active == settings.selected {
                            "当前使用"
                        } else {
                            "用于对话"
                        })
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| this.save_provider(true, cx))),
                    )
                    .when_some(
                        settings
                            .selected
                            .filter(|id| !self.provider_file_state.is_injected(*id)),
                        |row, id| {
                            row.child(
                                IconButton::new(Icon::Trash, "移除供应商")
                                    .plain()
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(|| "remove-selected-provider".into())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.request_remove_provider(id, cx)
                                    })),
                            )
                        },
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(p.text_muted))
                    .child(if chatgpt {
                        "ChatGPT 订阅 · 浏览器授权"
                    } else {
                        if settings.protocol == provider::Protocol::Anthropic {
                            "Anthropic Messages 协议"
                        } else {
                            "OpenAI Chat Completions 协议"
                        }
                    }),
            )
            .child(
                SegmentedControl::new([
                    SegmentOption::new("连接").debug_selector("provider-tab-connection"),
                    SegmentOption::new(format!("模型 · {}", settings.models.len()))
                        .debug_selector("provider-tab-models"),
                ])
                .id("provider-tabs")
                .selected(usize::from(settings.tab == ProviderTab::Models))
                .theme(self.theme)
                .on_change({
                    let weak = cx.entity().downgrade();
                    move |index, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.provider_settings.tab = if index == 1 {
                                ProviderTab::Models
                            } else {
                                ProviderTab::Connection
                            };
                            this.provider_settings.message = None;
                            cx.notify();
                        });
                    }
                })
                .render()
                .self_start(),
            );
        let field = |label: &str, entry: &Entity<TextEntry>| {
            InputField::new("")
                .label(label.to_owned())
                .control(entry.clone())
                .theme(self.theme)
                .render()
                .w_full()
        };
        // Connection form rows: label on the left, value/control right-aligned.
        let label_col = |label: &str| {
            div()
                .w(px(128.))
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(p.text_muted))
                .child(label.to_owned())
        };
        let hint_col = |hint: &str| {
            div()
                .pl(px(144.))
                .text_size(px(11.))
                .text_color(rgb(p.text_muted))
                .child(hint.to_owned())
        };
        let setting_row = |label: &str, control: gpui::Div| {
            div()
                .flex()
                .items_center()
                .gap_4()
                .child(label_col(label))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .justify_end()
                        .child(control),
                )
        };
        let field_row = |label: &str, entry: &Entity<TextEntry>| {
            let input = InputField::new("")
                .control(entry.clone())
                .theme(self.theme)
                .render()
                .w_full();
            setting_row(label, div().w_full().child(input))
        };
        if settings.tab == ProviderTab::Connection {
            let mut card = div()
                .p_5()
                .flex()
                .flex_col()
                .gap_5()
                .rounded(px(12.))
                .border_1()
                .border_color(rgb(p.border))
                .child(field_row("名称", &settings.name));
            if chatgpt {
                card = card
                    .child(setting_row(
                        "订阅授权",
                        div()
                            .flex()
                            .flex_wrap()
                            .justify_end()
                            .gap_3()
                            .child(
                                Button::new(if settings.auth_task.is_some() {
                                    "取消登录 / 检查"
                                } else {
                                    "Continue with ChatGPT"
                                })
                                .kind(ButtonKind::Glass)
                                .theme(self.theme)
                                .render()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.chatgpt_auth(ChatGptAuth::Login, cx)),
                                ),
                            )
                            .child(
                                Button::new("检查登录状态")
                                    .kind(ButtonKind::Plain)
                                    .theme(self.theme)
                                    .render()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.chatgpt_auth(ChatGptAuth::Check, cx)),
                                    ),
                            )
                            .child(
                                Button::new("退出 ChatGPT")
                                    .kind(ButtonKind::Plain)
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(|this, _, _, cx| this.chatgpt_auth(ChatGptAuth::SignOut, cx))),
                            ),
                    ))
                    .child(hint_col(
                        "通过官方 ChatGPT 页面授权订阅；凭据保存在本机，无需 API Key。每个供应商对应一个账号授权。",
                    ));
                if let Some(account) = &settings.chatgpt_account {
                    card = card.child(setting_row(
                        "当前账号",
                        div().text_size(px(12.)).child(account.clone()),
                    ));
                }
            } else {
                card = card
                    .child(setting_row(
                        "协议",
                        div().flex().items_center().gap_2().child(
                            SegmentedControl::new([
                                SegmentOption::new("Anthropic")
                                    .debug_selector("provider-protocol-anthropic"),
                                SegmentOption::new("OpenAI 兼容")
                                    .debug_selector("provider-protocol-openai"),
                            ])
                            .id("provider-protocol")
                            .selected(usize::from(settings.protocol == provider::Protocol::OpenAi))
                            .theme(self.theme)
                            .on_change({
                                let weak = cx.entity().downgrade();
                                move |index, _, cx| {
                                    let _ = weak.update(cx, |this, cx| {
                                        let protocol = if index == 1 {
                                            provider::Protocol::OpenAi
                                        } else {
                                            provider::Protocol::Anthropic
                                        };
                                        this.provider_settings.protocol = protocol;
                                        let address = this.provider_settings.url.read(cx).value();
                                        if matches!(
                                            address.trim_end_matches('/'),
                                            "https://api.deepseek.com"
                                                | "https://api.deepseek.com/v1"
                                                | "https://api.deepseek.com/anthropic"
                                        ) {
                                            let url = if protocol == provider::Protocol::Anthropic {
                                                "https://api.deepseek.com/anthropic"
                                            } else {
                                                "https://api.deepseek.com"
                                            };
                                            this.provider_settings
                                                .url
                                                .update(cx, |entry, cx| entry.set_value(url, cx));
                                            this.provider_settings.original.models_url =
                                                Some("https://api.deepseek.com/models".into());
                                        }
                                        cx.notify();
                                    });
                                }
                            })
                            .render(),
                        ),
                    ))
                    .child(field_row("Base URL", &settings.url))
                    .child(setting_row(
                        "认证方式",
                        div().flex().items_center().gap_2().child(
                            SegmentedControl::new([
                                SegmentOption::new("环境变量").debug_selector("provider-auth-env"),
                                SegmentOption::new("API Key").debug_selector("provider-auth-key"),
                            ])
                            .id("provider-auth")
                            .selected(usize::from(!settings.use_env))
                            .theme(self.theme)
                            .on_change({
                                let weak = cx.entity().downgrade();
                                move |index, _, cx| {
                                    let _ = weak.update(cx, |this, cx| {
                                        this.provider_settings.use_env = index == 0;
                                        cx.notify();
                                    });
                                }
                            })
                            .render(),
                        ),
                    ))
                    .child(if settings.use_env {
                        field_row("环境变量名", &settings.key_env)
                    } else {
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(setting_row(
                                "API Key",
                                div()
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div().flex_1().min_w_0().child(
                                            InputField::new("")
                                                .control(settings.key.clone())
                                                .theme(self.theme)
                                                .render()
                                                .w_full(),
                                        ),
                                    )
                                    .child(
                                        IconButton::new(Icon::Trash, "清除已保存的密钥")
                                            .plain()
                                            .theme(self.theme)
                                            .render()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.provider_settings.message = Some(
                                                    match this
                                                        .provider_settings
                                                        .original
                                                        .remove_key()
                                                    {
                                                        Ok(()) => {
                                                            "已清除 SQLite 中保存的密钥。".into()
                                                        }
                                                        Err(error) => error,
                                                    },
                                                );
                                                cx.notify();
                                            })),
                                    ),
                            ))
                            .child(hint_col("留空保留已保存的密钥；密钥只写在本机 SQLite。"))
                    });
            }
            card = card.child(settings.more_settings.clone());
            card = card.child(
                div()
                    .pt_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(setting_row(
                        "连接测试",
                        div().flex().items_center().gap_3().child(
                            Button::new(if settings.testing.is_some() {
                                "取消测试"
                            } else {
                                "测试连接"
                            })
                            .kind(ButtonKind::Glass)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.test_provider(cx))),
                        ),
                    ))
                    .child(hint_col("使用默认模型发送一条简短请求。")),
            );
            page = page.child(card);
        } else {
            let mut models = div()
                .rounded(px(12.))
                .border_1()
                .border_color(rgb(p.border))
                .flex()
                .flex_col();
            if settings.models.is_empty() {
                models =
                    models.child(div().p_5().text_color(rgb(p.text_muted)).child(if chatgpt {
                        "请先获取账号可用模型并选择默认模型。"
                    } else {
                        "添加模型后即可用于对话。"
                    }));
            }
            for (index, model) in settings.models.iter().enumerate() {
                let default_model = model == &settings.default_model;
                let make_default = model.clone();
                let remove = model.clone();
                models = models.child(
                    div()
                        .id(SharedString::from(format!("configured-model:{model}")))
                        .px_4()
                        .py_3()
                        .flex()
                        .items_center()
                        .gap_3()
                        .when(index > 0, |row| {
                            row.border_t_1().border_color(rgb(p.border))
                        })
                        .child(Icon::Workflow.render(15.).text_color(rgb(p.text_muted)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(model.clone()),
                                )
                                .when_some(
                                    settings.context_windows.get(model),
                                    |label, capacity| {
                                        label.child(
                                            div()
                                                .text_size(px(10.))
                                                .text_color(rgb(p.text_muted))
                                                .child(format!("接口窗口：{capacity} tokens")),
                                        )
                                    },
                                ),
                        )
                        .when_some(settings.window_inputs.get(model), |row, input| {
                            row.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .flex_shrink_0()
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(rgb(p.text_muted))
                                            .child("上下文窗口（tokens）"),
                                    )
                                    .child(div().w(px(180.)).child(input.clone())),
                            )
                        })
                        .child(
                            Button::new(if default_model {
                                "默认模型"
                            } else {
                                "设为默认"
                            })
                            .kind(if default_model {
                                ButtonKind::Tinted
                            } else {
                                ButtonKind::Plain
                            })
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.provider_settings.default_model = make_default.clone();
                                    cx.notify();
                                },
                            )),
                        )
                        .child(
                            IconButton::new(Icon::Close, format!("移除模型 {model}"))
                                .plain()
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.provider_settings
                                        .models
                                        .retain(|model| model != &remove);
                                    this.provider_settings.window_inputs.remove(&remove);
                                    if this.provider_settings.default_model == remove {
                                        this.provider_settings.default_model = this
                                            .provider_settings
                                            .models
                                            .first()
                                            .cloned()
                                            .unwrap_or_default();
                                    }
                                    cx.notify();
                                })),
                        ),
                );
            }
            page = page
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child("上下文窗口支持 1M、256K（十进制）；留空使用接口值，手动填写优先。"),
                )
                .child(models)
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap_3()
                        .child(div().flex_1().child(field("手动添加模型", &settings.model)))
                        .child(
                            div()
                                .w(px(180.))
                                .child(field("上下文窗口（tokens）", &settings.new_model_window)),
                        )
                        .child(
                            Button::new("添加")
                                .kind(ButtonKind::Glass)
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let model =
                                        this.provider_settings.model.read(cx).value().to_owned();
                                    if model.trim().is_empty() {
                                        this.provider_settings.message =
                                            Some("请输入模型 ID".into());
                                        cx.notify();
                                        return;
                                    }
                                    let window = this
                                        .provider_settings
                                        .new_model_window
                                        .read(cx)
                                        .value()
                                        .to_owned();
                                    match parse_context_window(&window) {
                                        Ok(_) => {
                                            let model = model.trim().to_owned();
                                            this.add_model(model.clone(), cx);
                                            if let Some(input) =
                                                this.provider_settings.window_inputs.get(&model)
                                            {
                                                input.update(cx, |input, cx| {
                                                    input.set_value(window, cx)
                                                });
                                            }
                                            this.provider_settings
                                                .new_model_window
                                                .update(cx, |input, cx| input.set_value("", cx));
                                        }
                                        Err(error) => this.provider_settings.message = Some(error),
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .font_weight(FontWeight::MEDIUM)
                                .child("服务中的可用模型"),
                        )
                        .child(
                            Button::new(if settings.discovering.is_some() {
                                "正在获取…"
                            } else {
                                "获取模型列表"
                            })
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Small)
                            .theme(self.theme)
                            .when(settings.discovering.is_some(), |button| button.disabled())
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.refresh_models(cx))),
                        ),
                );
            let available: Vec<_> = settings.available_models.iter().collect();
            if available.is_empty() && settings.discovering.is_some() {
                page = page.child(
                    div()
                        .p_3()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(rgb(p.border))
                        .debug_selector(|| "models-loading-skeleton".into())
                        .child(
                            Skeleton::new("available-models-loading")
                                .lines(5)
                                .height(12.)
                                .gap(20.)
                                .theme(self.theme),
                        ),
                );
            }
            if !available.is_empty() {
                page = page.child(
                    div()
                        .id("available-models")
                        .max_h(px(240.))
                        .overflow_y_scroll()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(rgb(p.border))
                        .children(available.into_iter().map(|model| {
                            let add = model.clone();
                            available_model_row(
                                model,
                                settings.models.contains(model),
                                self.theme,
                                cx.listener(move |this, _, _, cx| this.add_model(add.clone(), cx)),
                            )
                        })),
                );
            }
        }
        page = page.when_some(settings.message.clone(), |page, message| {
            page.child(
                div()
                    .p_3()
                    .rounded(px(8.))
                    .bg(rgb(p.surface_subtle))
                    .text_size(px(12.))
                    .child(message),
            )
        });
        self.settings_container(page)
    }

    pub fn settings_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let query = self
            .provider_settings
            .search
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let mut navigation = div()
            .id("settings-navigation")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_3()
            .pb_4();
        for (section, icon) in [
            (Section::General, Icon::Settings),
            (Section::Appearance, Icon::HalfCircle),
            (Section::Layout, Icon::Sidebar),
            (Section::Storage, Icon::Folder),
            (Section::Memory, Icon::Database),
            (Section::Skills, Icon::Workflow),
            (Section::Mcp, Icon::Workflow),
            (Section::Tools, Icon::Puzzle),
            (Section::Shortcuts, Icon::Code),
        ] {
            if !query.is_empty() && !section.title().to_lowercase().contains(&query) {
                continue;
            }
            navigation = navigation.child(
                div()
                    .id(("settings-section", section as usize))
                    .h(px(38.))
                    .px_3()
                    .mb_1()
                    .rounded(px(9.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .cursor_pointer()
                    .when(self.provider_settings.section == section, |row| {
                        row.bg(rgb(p.surface_subtle))
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .hover(|s| s.bg(rgb(p.surface_subtle)))
                    .child(icon.render(17.))
                    .child(section.title())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.provider_settings.section = section;
                        if section == Section::Mcp {
                            this.reload_mcp(cx);
                        }
                        if section == Section::Skills {
                            this.reload_skills(cx);
                        }
                        if section == Section::Memory {
                            this.reload_memories(cx);
                        }
                        window.focus(&this.provider_settings.search.focus_handle(cx), cx);
                        cx.notify();
                    })),
            );
        }
        navigation = navigation.child(
            div()
                .px_3()
                .pt_5()
                .pb_2()
                .flex()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child("供应商"),
                )
                .child(self.add_provider_menu(cx)),
        );
        for profile in &self.catalog.providers {
            if !query.is_empty()
                && !profile.name.to_lowercase().contains(&query)
                && !profile.base_url.to_lowercase().contains(&query)
            {
                continue;
            }
            let id = profile.id;
            let detected = self.provider_settings.discovered.iter().any(|found| {
                found.config.kind == profile.kind && found.config.base_url == profile.base_url
            });
            navigation = navigation.child(
                div()
                    .id(gpui::SharedString::from(format!("provider-{id}")))
                    .group(gpui::SharedString::from(format!("provider-row-{id}")))
                    .px_3()
                    .py_3()
                    .mb_1()
                    .rounded(px(9.))
                    .cursor_pointer()
                    .when(
                        self.provider_settings.section == Section::Provider
                            && self.provider_settings.selected == Some(id),
                        |row| row.bg(rgb(p.surface_subtle)),
                    )
                    .hover(|s| s.bg(rgb(p.surface_subtle)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                if profile.kind == provider::Kind::ChatGpt {
                                    Icon::User
                                } else {
                                    Icon::Globe
                                }
                                .render(17.),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .child(profile.name.clone()),
                            )
                            .when(self.catalog.active == Some(id), |row| {
                                row.child(Icon::CheckCircle.render(13.).text_color(rgb(p.accent)))
                            })
                            // 配置文件注入的供应商归文件管，不给移除按钮。
                            .when(!self.provider_file_state.is_injected(id), |row| {
                                let group = gpui::SharedString::from(format!("provider-row-{id}"));
                                row.child(
                                    div()
                                        .id(gpui::SharedString::from(format!(
                                            "remove-provider-{id}"
                                        )))
                                        .debug_selector(move || format!("remove-provider-{id}"))
                                        .size(px(22.))
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(6.))
                                        .opacity(0.)
                                        .group_hover(group, |style| style.opacity(1.))
                                        .hover(|style| style.bg(rgb(p.border)))
                                        .cursor_pointer()
                                        .child(
                                            Icon::Trash.render(13.).text_color(rgb(p.text_muted)),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.request_remove_provider(id, cx);
                                        })),
                                )
                            }),
                    )
                    .child(
                        div()
                            .pl(px(29.))
                            .pt_1()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(format!(
                                "{} 个模型{}",
                                profile.models.len(),
                                if self.provider_file_state.is_injected(profile.id) {
                                    " · 配置文件"
                                } else if detected {
                                    " · 自动发现"
                                } else {
                                    ""
                                }
                            )),
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.select_provider(id, window, cx)
                        }),
                    ),
            );
        }
        navigation = navigation.child(
            div()
                .px_5()
                .pt_3()
                .text_size(px(10.))
                .text_color(rgb(p.text_muted))
                .child(if self.provider_file_state.connected() {
                    "已合并供应商配置文件里的供应商，改动 2 秒内自动同步"
                } else {
                    "在 Magpie 中选择 Glint Workbench 即可注入供应商"
                }),
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(p.panel))
            .child(
                div()
                    .px_5()
                    .pt_5()
                    .pb_4()
                    .text_size(px(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("设置"),
            )
            .child(
                div().px_3().pb_4().child(
                    SearchInput::new("搜索设置或供应商…")
                        .control(self.provider_settings.search.clone())
                        .theme(self.theme)
                        .render()
                        .w_full(),
                ),
            )
            .child(navigation)
    }
    pub(crate) fn settings_container(&self, page: gpui::Div) -> gpui::Div {
        div().size_full().child(
            div()
                .id("settings-content")
                .size_full()
                .overflow_y_scroll()
                .px(px(56.))
                .pt(px(72.))
                .pb(px(64.))
                .child(div().w_full().max_w(px(820.)).mx_auto().child(page)),
        )
    }

    fn settings_overview(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let section = self.provider_settings.section;
        let row = |title: &str, description: String, value: String| {
            div()
                .py_4()
                .flex()
                .items_center()
                .gap_5()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(title.to_owned()),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(p.text_muted))
                                .child(description),
                        ),
                )
                .child(
                    div()
                        .max_w(px(280.))
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child(value),
                )
        };
        let mut card = div()
            .w_full()
            .px_5()
            .rounded(px(14.))
            .border_1()
            .border_color(rgb(p.border));
        let mut page = div().w_full().flex().flex_col().gap_6().child(
            div()
                .text_size(px(28.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(section.title()),
        );
        match section {
            Section::General => {
                let quick_replies_weak = cx.entity().downgrade();
                page = page.child(div().mt_4().font_weight(FontWeight::SEMIBOLD).child("会话"));
                card = card
                    .child(row(
                        "当前 Provider",
                        "用于新消息的模型连接。".into(),
                        if self.provider.enabled {
                            self.provider.name.clone()
                        } else {
                            "未连接".into()
                        },
                    ))
                    .child(div().border_t_1().border_color(rgb(p.border)))
                    .child(row(
                        "会话与草稿",
                        "保存在本机，启动时恢复上次会话。".into(),
                        "自动保存".into(),
                    ))
                    .child(div().border_t_1().border_color(rgb(p.border)))
                    .child(row(
                        "输入方式",
                        "通过 Shift + Enter 插入新行。".into(),
                        "Enter 发送".into(),
                    ));
                card = card
                    .child(div().border_t_1().border_color(rgb(p.border)))
                    .child(div().py_4().flex().items_center().gap_3()
                        .child(div().flex_1().flex().flex_col().gap_1()
                            .child("快捷回复建议")
                            .child(div().text_size(px(12.)).text_color(rgb(p.text_muted))
                                .child("回复结束后生成可点击发送的建议，也可忽略。会额外调用当前模型。")))
                        .child(Switch::new("启用")
                            .checked(self.history.quick_replies_enabled)
                            .theme(self.theme)
                            .on_change(move |enabled, _, cx| {
                                let _ = quick_replies_weak.update(cx, |this, cx| this.set_quick_replies_enabled(enabled, cx));
                            })
                            .render()
                            .debug_selector(|| "quick-replies-enabled".into())))
                    .child(div().border_t_1().border_color(rgb(p.border)))
                    .child(div().py_4().flex().items_center().gap_3()
                        .child(div().flex_1().flex().flex_col().gap_1()
                            .child("自动压缩会话")
                            .child(div().text_size(px(12.)).text_color(rgb(p.text_muted))
                                .child("发送前检查上下文占用；达到阈值时摘要旧消息，保留最近两轮。")))
                        .child(Button::new(if self.history.compaction_settings.enabled { "已开启" } else { "已关闭" })
                            .kind(ButtonKind::Plain).size(ButtonSize::Small).theme(self.theme).render()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.history.compaction_settings.enabled = !this.history.compaction_settings.enabled;
                                this.save(cx); cx.notify();
                            }))))
                    .child(div().py_4().flex().items_center().gap_3()
                        .child(div().flex_1().child("自动压缩阈值"))
                        .child(div().w(px(110.)).child(self.provider_settings.compaction_threshold.clone()))
                        .child("%")
                        .child(Button::new("保存").size(ButtonSize::Small).theme(self.theme).render()
                            .debug_selector(|| "save-compaction-settings".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                let value = this.provider_settings.compaction_threshold.read(cx).value().to_owned();
                                match crate::compaction::parse_threshold(&value) {
                                    Ok(value) => {
                                        this.history.compaction_settings.threshold_percent = value;
                                        this.provider_settings.compaction_message = Some("压缩设置已保存".into());
                                        this.save(cx);
                                    }
                                    Err(error) => this.provider_settings.compaction_message = Some(error),
                                }
                                cx.notify();
                            }))))
                    .child(div().pb_4().text_size(px(12.)).text_color(rgb(p.text_muted))
                        .child("原始消息保留在本机。压缩会调用当前模型，产生额外用量；上下文占用未知时不会自动压缩，可在会话顶部手动压缩。"))
                    .when_some(self.provider_settings.compaction_message.clone(), |card, message| {
                        card.child(div().pb_4().text_size(px(12.)).child(message))
                    });
                page = page.child(card);
                page = page.child(
                    div()
                        .mt_4()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("提交信息"),
                );
                let commit_card = div()
                    .w_full()
                    .px_5()
                    .rounded(px(14.))
                    .border_1()
                    .border_color(rgb(p.border))
                    .child(
                        div()
                            .py_4()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child("默认提示语")
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(p.text_muted))
                                    .child("提交信息输入框左下角的魔法棒会按这段提示语生成信息。留空或恢复默认时使用内置提示语。"),
                            ),
                    )
                    .child(
                        div()
                            .pb_3()
                            .rounded(px(10.))
                            .border_1()
                            .border_color(rgb(p.border))
                            .p_2()
                            .min_h(px(84.))
                            .child(self.provider_settings.commit_prompt.clone()),
                    )
                    .child(
                        div()
                            .pb_4()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("保存提示语")
                                    .size(ButtonSize::Small)
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(|| "save-commit-prompt".into())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let value = this
                                            .provider_settings
                                            .commit_prompt
                                            .read(cx)
                                            .value()
                                            .to_owned();
                                        this.history.commit_message.prompt =
                                            if value.trim() == commit_message::DEFAULT_PROMPT {
                                                String::new()
                                            } else {
                                                value
                                            };
                                        this.save(cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("恢复默认")
                                    .kind(ButtonKind::Plain)
                                    .size(ButtonSize::Small)
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(|| "reset-commit-prompt".into())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.history.commit_message.prompt.clear();
                                        let prompt = this.history.commit_message.prompt().to_owned();
                                        this.provider_settings
                                            .commit_prompt
                                            .update(cx, |editor, cx| editor.set_value(prompt, cx));
                                        this.save(cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(p.text_muted))
                                    .child(if self.history.commit_message.custom_prompt() {
                                        "当前使用自定义提示语"
                                    } else {
                                        "当前使用内置提示语"
                                    }),
                            ),
                    );
                page = page.child(commit_card);
            }
            Section::Appearance => {
                page = page.child(div().mt_4().font_weight(FontWeight::SEMIBOLD).child("主题"));
                card = card.child(
                    div()
                        .py_4()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child("窗口外观")
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(p.text_muted))
                                        .child(format!(
                                            "选择当前窗口使用的主题，当前为{}。",
                                            self.theme.name()
                                        )),
                                ),
                        )
                        .child({
                            let weak = cx.entity().downgrade();
                            SegmentedControl::new([
                                SegmentOption::new("浅色").debug_selector("theme-appearance-light"),
                                SegmentOption::new("深色").debug_selector("theme-appearance-dark"),
                            ])
                            .id("theme-appearance")
                            .selected(appearance_segment(self.theme))
                            .theme(self.theme)
                            .on_change(move |index, _, cx| {
                                let theme = if index == 1 {
                                    ThemePreset::DarkObsidian
                                } else {
                                    ThemePreset::Moonstone
                                };
                                let _ = weak.update(cx, |this, cx| {
                                    this.theme = theme;
                                    this.composer
                                        .update(cx, |composer, cx| composer.set_theme(theme, cx));
                                    this.search
                                        .update(cx, |entry, cx| entry.set_theme(theme, cx));
                                    this.provider_settings.set_theme(theme, cx);
                                    this.skills_ui.set_theme(theme, cx);
                                    this.mcp_ui.set_theme(theme, cx);
                                    this.sync_skill_context(cx);
                                    cx.notify();
                                });
                            })
                            .render()
                        }),
                );
                page = page.child(card)
                    .child(div().mt_4().font_weight(FontWeight::SEMIBOLD).child("Git 工作区"))
                    .child(div().py_4().flex().items_center().gap_3()
                        .child(div().flex_1().min_w_0().flex().flex_col().gap_1()
                            .child("文件展示形态")
                            .child(div().text_size(px(12.)).text_color(rgb(p.text_muted))
                                .child("列表显示完整路径；树形目录按文件夹分组，可展开或折叠。")))
                        .child({
                            let weak = cx.entity().downgrade();
                            SegmentedControl::new([
                                SegmentOption::new("列表").debug_selector("workspace-display-list"),
                                SegmentOption::new("树形目录").debug_selector("workspace-display-tree"),
                            ])
                            .id("workspace-display-mode")
                            .selected(usize::from(self.history.workspace_display == crate::workspace_tree::DisplayMode::Tree))
                            .theme(self.theme)
                            .on_change(move |index, _, cx| {
                                let mode = if index == 1 {
                                    crate::workspace_tree::DisplayMode::Tree
                                } else {
                                    crate::workspace_tree::DisplayMode::List
                                };
                                let _ = weak.update(cx, |this, cx| {
                                    this.history.workspace_display = mode;
                                    this.save(cx);
                                    cx.notify();
                                });
                            })
                            .render()
                        }))
                    .child(self.code_font_settings(cx));
            }
            Section::Layout => {
                page = page.child(
                    div()
                        .mt_4()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("终端面板"),
                );
                card = card.child(
                    div().py_4().flex().items_center().gap_3()
                        .child(div().flex_1().min_w_0().flex().flex_col().gap_1()
                            .child("默认位置")
                            .child(div().text_size(px(12.)).text_color(rgb(p.text_muted))
                                .child("启动时终端面板显示的位置。修改后立即生效，也可在面板中临时切换。")))
                        .child({
                            let weak = cx.entity().downgrade();
                            SegmentedControl::new([
                                SegmentOption::new("下方").debug_selector("terminal-position-bottom"),
                                SegmentOption::new("右侧").debug_selector("terminal-position-right"),
                            ])
                            .id("terminal-default-position")
                            .selected(usize::from(self.history.terminal_default_position == crate::terminal_ui::Position::Right))
                            .theme(self.theme)
                            .on_change(move |index, _, cx| {
                                let position = if index == 1 {
                                    crate::terminal_ui::Position::Right
                                } else {
                                    crate::terminal_ui::Position::Bottom
                                };
                                let _ = weak.update(cx, |this, cx| {
                                    this.history.terminal_default_position = position;
                                    this.set_terminal_position(position);
                                    this.save(cx);
                                    cx.notify();
                                });
                            })
                            .render()
                        }),
                );
                page = page.child(card);
            }
            Section::Storage => {
                page = page.child(
                    div()
                        .mt_4()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("本地数据"),
                );
                card = card
                    .child(row(
                        "会话记录",
                        "SQLite 保存会话、草稿、项目、供应商、日程与记忆。".into(),
                        format!("{} 个会话", self.history.sessions.len()),
                    ))
                    .child(div().border_t_1().border_color(rgb(p.border)))
                    .child(row(
                        "SQLite 数据库",
                        gpui_glint::paths::display_path(crate::database::path(&self.path)),
                        "WAL".into(),
                    ))
                    .child(row(
                        "数据目录",
                        gpui_glint::paths::display_path(
                            self.path.parent().unwrap_or(std::path::Path::new(".")),
                        ),
                        "".into(),
                    ))
                    .child(div().border_t_1().border_color(rgb(p.border)))
                    .child(row(
                        "API Key",
                        "密钥以明文保存在本机 SQLite 独立表中。".into(),
                        "本地 SQLite".into(),
                    ));
                page = page.child(card);
            }
            Section::Shortcuts => {
                let modifier = if cfg!(target_os = "macos") {
                    "⌘"
                } else {
                    "Ctrl"
                };
                let keycaps = |label: String| {
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .flex_shrink_0()
                        .children(
                            label
                                .split(" + ")
                                .map(|key| {
                                    div()
                                        .min_w(px(28.))
                                        .h(px(28.))
                                        .px_2()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(5.))
                                        .border_1()
                                        .border_color(rgb(p.border))
                                        .bg(rgb(p.panel))
                                        .text_size(px(12.))
                                        .text_color(rgb(p.text))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(key.to_owned())
                                })
                                .collect::<Vec<_>>(),
                        )
                };
                let shortcut_card = || {
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .flex_shrink_0()
                        .px_4()
                        .rounded(px(12.))
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.surface))
                };
                let group_title = |title: &str, caption: &str| {
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title.to_owned()),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child(caption.to_owned()),
                        )
                };
                // Keep shortcut rows independent of the generic settings row's description layout.
                let mut common = shortcut_card();
                for (index, (title, key)) in [
                    ("发送消息", "Enter".into()),
                    ("换行", "Shift + Enter".into()),
                    ("新建会话", format!("{modifier} + N")),
                    ("搜索会话", format!("{modifier} + K")),
                    ("打开设置", format!("{modifier} + ,")),
                ]
                .into_iter()
                .enumerate()
                {
                    common = common.child(
                        div()
                            .h(px(52.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_4()
                            .when(index > 0, |row| {
                                row.border_t_1().border_color(rgb(p.border))
                            })
                            .child(div().flex_1().min_w_0().text_size(px(13.)).child(title))
                            .child(keycaps(key)),
                    );
                }
                let mut project = shortcut_card();
                for action in crate::shortcuts::ProjectAction::ALL {
                    let recording = self.provider_settings.shortcut_recording == Some(action);
                    let shortcut_label = action.label(&self.history);
                    let record_selector = format!("record-{}-shortcut", action.id());
                    let reset_selector = format!("reset-{}-shortcut", action.id());
                    project = project.child(
                        div()
                            .py_4()
                            .when(action != crate::shortcuts::ProjectAction::Terminal, |row| {
                                row.border_t_1().border_color(rgb(p.border))
                            })
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(180.))
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(div().text_size(px(13.)).child(action.title()))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(rgb(p.text_muted))
                                            .child(action.description()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .id(SharedString::from(format!(
                                                "{}-shortcut-trigger",
                                                action.id()
                                            )))
                                            .debug_selector(move || record_selector.clone())
                                            .cursor_pointer()
                                            .rounded(px(6.))
                                            .p_1()
                                            .bg(rgb(if recording {
                                                p.accent_tint
                                            } else {
                                                p.surface
                                            }))
                                            .hover(|style| style.bg(rgb(p.accent_tint)))
                                            .child(if recording {
                                                div()
                                                    .h(px(28.))
                                                    .px_2()
                                                    .flex()
                                                    .items_center()
                                                    .text_size(px(12.))
                                                    .text_color(rgb(p.accent))
                                                    .child("请按组合键…")
                                            } else {
                                                keycaps(shortcut_label)
                                            })
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.provider_settings.shortcut_recording =
                                                    Some(action);
                                                this.provider_settings.shortcut_message = None;
                                                window.focus(&this.focus, cx);
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("恢复默认")
                                            .kind(ButtonKind::Plain)
                                            .size(ButtonSize::Small)
                                            .theme(self.theme)
                                            .render()
                                            .id(SharedString::from(format!(
                                                "reset-{}-shortcut",
                                                action.id()
                                            )))
                                            .debug_selector(move || reset_selector.clone())
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.reset_project_shortcut(action, cx)
                                            })),
                                    ),
                            ),
                    );
                }
                use crate::shortcuts::DefinitionModifier;
                let modifier = self.history.definition_modifier;
                let owner = cx.entity().downgrade();
                let project = project.child(
                    div()
                        .py_4()
                        .border_t_1()
                        .border_color(rgb(p.border))
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(180.))
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(div().text_size(px(13.)).child("跳转到 Go 定义"))
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(p.text_muted))
                                        .child("按住修饰键悬停查看可跳转符号，左键打开定义"),
                                ),
                        )
                        .child(
                            SegmentedControl::new(DefinitionModifier::ALL.map(|modifier| {
                                SegmentOption::new(modifier.label()).debug_selector(format!(
                                    "go-definition-modifier-{}",
                                    modifier.label()
                                ))
                            }))
                            .id("go-definition-modifier")
                            .theme(self.theme)
                            .selected(
                                DefinitionModifier::ALL
                                    .iter()
                                    .position(|key| *key == modifier)
                                    .unwrap_or_default(),
                            )
                            .on_change(move |index, _, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.set_definition_modifier(
                                        DefinitionModifier::ALL[index],
                                        cx,
                                    );
                                });
                            })
                            .render(),
                        )
                        .child(keycaps("左键".into()))
                        .child(
                            Button::new("恢复默认")
                                .kind(ButtonKind::Plain)
                                .size(ButtonSize::Small)
                                .theme(self.theme)
                                .render()
                                .id("reset-go-definition-shortcut")
                                .debug_selector(|| "reset-go-definition-shortcut".into())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_definition_modifier(DefinitionModifier::default(), cx);
                                })),
                        ),
                );
                let recording = self.provider_settings.shortcut_recording.is_some();
                let project = project
                    .child(
                        div()
                            .py_3()
                            .border_t_1()
                            .border_color(rgb(p.border))
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(if recording {
                                "按下包含 Ctrl、Alt 或 Command 的组合键，Esc 取消。"
                            } else {
                                "点击按键修改快捷键，更改后立即生效。"
                            }),
                    )
                    .when_some(
                        self.provider_settings.shortcut_message.clone(),
                        |card, message| {
                            card.child(
                                div()
                                    .pb_3()
                                    .text_size(px(11.))
                                    .text_color(rgb(self.theme.color(0xd9342b)))
                                    .child(message),
                            )
                        },
                    );
                page = page
                    .debug_selector(|| "shortcuts-settings-page".into())
                    .child(
                        div()
                            .mt(px(-12.))
                            .text_size(px(12.))
                            .text_color(rgb(p.text_muted))
                            .child("用键盘更快地发送消息、切换会话和进入项目。"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(group_title("消息与会话", "默认快捷键"))
                            .child(common),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(group_title("项目", "支持自定义"))
                            .child(project),
                    );
            }
            _ => {}
        }
        self.settings_container(page)
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;

    #[gpui::test]
    fn provider_response_timeout_setting_validates_persists_and_loads(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let config = provider::Config {
            name: "Timeout test".into(),
            model: "test-model".into(),
            models: vec!["test-model".into()],
            ..Default::default()
        };
        let id = config.id;
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut h = AgentWorkspace::with_path(directory.path().join("sessions.json"), cx);
            h.page = crate::Page::Settings;
            h.catalog.providers.push(config.clone());
            h.provider_settings.load(config, String::new(), false, cx);
            h.provider_settings.section = Section::Provider;
            h
        });
        window.simulate_resize(size(px(1440.), px(1200.)));
        window.run_until_parked();
        assert!(window.debug_bounds("provider-response-timeout").is_none());
        for selector in [
            "provider-auth-env",
            "provider-auth-key",
            "provider-protocol-anthropic",
            "provider-protocol-openai",
            "provider-tab-models",
            "provider-tab-connection",
        ] {
            let bounds = window.debug_bounds(selector).unwrap();
            window.simulate_click(bounds.center(), gpui::Modifiers::default());
            window.run_until_parked();
            host.read_with(window, |h, _| match selector {
                "provider-auth-env" => assert!(h.provider_settings.use_env),
                "provider-auth-key" => assert!(!h.provider_settings.use_env),
                "provider-protocol-anthropic" => {
                    assert!(h.provider_settings.protocol == provider::Protocol::Anthropic)
                }
                "provider-protocol-openai" => {
                    assert!(h.provider_settings.protocol == provider::Protocol::OpenAi)
                }
                "provider-tab-models" => assert!(h.provider_settings.tab == ProviderTab::Models),
                "provider-tab-connection" => {
                    assert!(h.provider_settings.tab == ProviderTab::Connection)
                }
                _ => unreachable!(),
            });
        }
        let trigger = window.debug_bounds("glass-collapse-panel-trigger").unwrap();
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        window.run_until_parked();
        assert!(window.debug_bounds("provider-response-timeout").is_some());
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        window.run_until_parked();
        assert!(window.debug_bounds("provider-response-timeout").is_none());
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        window.run_until_parked();
        host.update(window, |h, cx| {
            for invalid in ["", "0", "86401", "1.5", "abc"] {
                h.provider_settings
                    .response_timeout
                    .update(cx, |e, cx| e.set_value(invalid, cx));
                h.save_provider(false, cx);
                assert!(
                    h.provider_settings
                        .message
                        .as_deref()
                        .unwrap()
                        .contains("1–86400")
                );
                assert_eq!(
                    h.catalog
                        .providers
                        .iter()
                        .find(|p| p.id == id)
                        .unwrap()
                        .response_timeout_seconds,
                    180
                );
            }
            h.provider_settings
                .response_timeout
                .update(cx, |e, cx| e.set_value("600", cx));
            h.save_provider(false, cx);
            let catalog =
                crate::provider_catalog::ProviderCatalog::load(&h.catalog_path(), &[]).unwrap();
            let saved = catalog.providers.into_iter().find(|p| p.id == id).unwrap();
            assert_eq!(saved.response_timeout_seconds, 600);
            h.provider_settings
                .load(provider::Config::default(), String::new(), false, cx);
            assert_eq!(h.provider_settings.response_timeout.read(cx).value(), "180");
            h.provider_settings.load(saved, String::new(), false, cx);
            assert_eq!(h.provider_settings.response_timeout.read(cx).value(), "600");
            assert!(!h.provider_settings.more_settings.read(cx).is_expanded());
        });
    }

    struct ModelRows {
        added: Vec<String>,
    }
    impl Render for ModelRows {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(600.))
                .flex()
                .flex_col()
                .children(["gpt-a", "gpt-b"].into_iter().map(|model| {
                    available_model_row(
                        model,
                        self.added.iter().any(|added| added == model),
                        ThemePreset::Moonstone,
                        cx.listener(move |this, _, _, cx| {
                            this.added.push(model.into());
                            cx.notify();
                        }),
                    )
                }))
        }
    }
    #[gpui::test]
    fn compaction_settings_validate_and_persist_custom_percentage(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut h = AgentWorkspace::with_path(directory.path().join("sessions.json"), cx);
            h.page = crate::Page::Settings;
            h
        });
        window.simulate_resize(size(px(1440.), px(1200.)));
        window.run_until_parked();
        for (input, expected) in [("90", 90), ("100", 90), ("75%", 75)] {
            host.update(window, |h, cx| {
                h.provider_settings
                    .compaction_threshold
                    .update(cx, |e, cx| e.set_value(input, cx));
                cx.notify();
            });
            window.run_until_parked();
            let save = window.debug_bounds("save-compaction-settings").unwrap();
            window.simulate_click(save.center(), gpui::Modifiers::default());
            window.run_until_parked();
            host.read_with(window, |h, _| {
                assert_eq!(h.history.compaction_settings.threshold_percent, expected)
            });
        }
        host.read_with(window, |h, _| {
            h.history.save(&h.path).unwrap();
            assert_eq!(
                History::load(&h.path)
                    .unwrap()
                    .compaction_settings
                    .threshold_percent,
                75
            );
        });
    }

    #[test]
    fn appearance_segments_map_every_preset_to_light_or_dark() {
        for theme in ThemePreset::ALL {
            assert_eq!(
                appearance_segment(theme),
                usize::from(theme.is_dark()),
                "{}",
                theme.name()
            );
        }
        assert_eq!(appearance_segment(ThemePreset::Sage), 0);
        assert_eq!(appearance_segment(ThemePreset::Moonstone), 0);
        assert_eq!(appearance_segment(ThemePreset::DarkLinen), 1);
        assert_eq!(appearance_segment(ThemePreset::DarkObsidian), 1);
    }

    #[gpui::test]
    fn appearance_segments_switch_the_window_theme(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut h = AgentWorkspace::with_path(directory.path().join("sessions.json"), cx);
            h.theme = ThemePreset::Moonstone;
            h.page = crate::Page::Settings;
            h.provider_settings.section = Section::Appearance;
            h
        });
        window.simulate_resize(size(px(1440.), px(1200.)));
        window.run_until_parked();
        let dark = window.debug_bounds("theme-appearance-dark").unwrap();
        window.simulate_click(dark.center(), gpui::Modifiers::default());
        window.run_until_parked();
        host.read_with(window, |h, _| {
            assert!(h.theme == ThemePreset::DarkObsidian);
            assert!(h.theme.is_dark());
        });
        let light = window.debug_bounds("theme-appearance-light").unwrap();
        window.simulate_click(light.center(), gpui::Modifiers::default());
        window.run_until_parked();
        host.read_with(window, |h, _| {
            assert!(h.theme == ThemePreset::Moonstone);
            assert!(!h.theme.is_dark());
        });
    }

    #[gpui::test]
    fn repeated_add_labels_dispatch_to_the_correct_model_once(cx: &mut gpui::TestAppContext) {
        let (view, window) = cx.add_window_view(|_, _| ModelRows { added: Vec::new() });
        view.update(window, |_, cx| cx.notify());
        for model in ["add-model:gpt-b", "add-model:gpt-a"] {
            let button = window.debug_bounds(model).unwrap();
            window.simulate_click(button.center(), gpui::Modifiers::default());
        }
        let button = window.debug_bounds("add-model:gpt-b").unwrap();
        window.simulate_click(button.center(), gpui::Modifiers::default());
        view.read_with(window, |view, _| assert_eq!(view.added, ["gpt-b", "gpt-a"]));
    }

    #[test]
    fn manual_context_windows_validate_and_empty_means_auto() {
        assert_eq!(parse_context_window(""), Ok(None));
        assert_eq!(parse_context_window("1,048,576"), Ok(Some(1048576)));
        assert_eq!(parse_context_window("128_000"), Ok(Some(128000)));
        assert_eq!(parse_context_window("1M"), Ok(Some(1_000_000)));
        assert_eq!(parse_context_window("256K"), Ok(Some(256_000)));
        assert_eq!(parse_context_window("1.5m"), Ok(Some(1_500_000)));
        assert_eq!(parse_context_window("1.25k"), Ok(Some(1_250)));
        for invalid in [
            "0",
            "-1",
            "128G",
            "1.5",
            "unknown",
            "18446744073709551615M",
            "0.0000001M",
        ] {
            assert!(parse_context_window(invalid).is_err());
        }
    }
}
