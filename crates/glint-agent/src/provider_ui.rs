use super::*;
use gpui_glint::components::InputField;
use std::collections::HashMap;

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
    Storage,
    Shortcuts,
    Memory,
    Skills,
    Mcp,
    Provider,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Self::General => "常规",
            Self::Appearance => "外观",
            Self::Storage => "存储",
            Self::Shortcuts => "键盘快捷键",
            Self::Memory => "记忆",
            Self::Skills => "技能",
            Self::Mcp => "MCP",
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
    tab: ProviderTab,
    pub search: Entity<TextEntry>,
    pub kind: provider::Kind,
    protocol: provider::Protocol,
    pub codex_path: Entity<TextEntry>,
    pub name: Entity<TextEntry>,
    pub url: Entity<TextEntry>,
    pub model: Entity<TextEntry>,
    pub key: Entity<TextEntry>,
    pub key_env: Entity<TextEntry>,
    use_env: bool,
    pub message: Option<String>,
    pub testing: Option<Task<()>>,
    pub auth_task: Option<Task<()>>,
}

impl Settings {
    pub fn new(
        config: Option<&provider::Config>,
        discovered: Vec<discovery::DiscoveredProvider>,
        cx: &mut Context<AgentWorkspace>,
    ) -> Self {
        let selected = config.map(|config| config.id);
        let config = config.cloned().unwrap_or_default();
        let mut entry = |hint: &str, value: &str| {
            cx.new(|cx| {
                TextEntry::new(cx, hint.to_owned(), value.to_owned()).theme(ThemePreset::Moonstone)
            })
        };
        Self {
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
            tab: ProviderTab::Connection,
            kind: config.kind,
            protocol: config.protocol,
            search: entry("搜索设置或供应商…", ""),
            name: entry("供应商名称", &config.name),
            codex_path: entry("Codex CLI 路径", &config.codex_path),
            url: entry("https://your-provider.example/v1", &config.base_url),
            model: entry("输入模型 ID", ""),
            key: entry("输入或替换 API Key", ""),
            key_env: entry(
                "例如 DEEPSEEK_API_KEY",
                config.api_key_env.as_deref().unwrap_or(""),
            ),
            use_env: config.api_key_env.is_some(),
            original: config,
            message: None,
            testing: None,
            auth_task: None,
        }
    }

    fn load(
        &mut self,
        config: provider::Config,
        key: String,
        use_env: bool,
        cx: &mut Context<AgentWorkspace>,
    ) {
        self.discovering = None;
        self.testing = None;
        self.auth_task = None;
        self.selected = Some(config.id);
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
        self.codex_path.update(cx, |entry, cx| {
            entry.set_value(config.codex_path.clone(), cx)
        });
        self.key_env.update(cx, |entry, cx| {
            entry.set_value(config.api_key_env.clone().unwrap_or_default(), cx)
        });
        self.key.update(cx, |entry, cx| entry.set_value(key, cx));
        self.model.update(cx, |entry, cx| entry.set_value("", cx));
        self.original = config;
    }

    pub fn set_theme(&self, theme: ThemePreset, cx: &mut Context<AgentWorkspace>) {
        for entry in self.window_inputs.values() {
            entry.update(cx, |entry, cx| entry.set_theme(theme, cx));
        }
        for entry in [
            &self.search,
            &self.name,
            &self.url,
            &self.model,
            &self.new_model_window,
            &self.key,
            &self.key_env,
            &self.codex_path,
        ] {
            entry.update(cx, |entry, cx| entry.set_theme(theme, cx));
        }
    }
}

impl AgentWorkspace {
    fn catalog_path(&self) -> PathBuf {
        self.path.with_file_name("providers.json")
    }

    pub(super) fn sync_provider(&mut self, cx: &mut Context<Self>) {
        self.provider = self.catalog.current();
        let chip = model_chip(&self.catalog);
        self.composer.update(cx, |composer, cx| {
            composer.set_menu_chip(chip, cx);
            composer.set_disabled(
                !self.provider.enabled || self.pending.is_some() || self.preparing.is_some(),
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
            self.sync_context_usage(cx);
        }
        cx.notify();
    }

    pub(super) fn choose_model(&mut self, value: &str, cx: &mut Context<Self>) {
        let result = (|| {
            if self.pending.is_some() || self.preparing.is_some() {
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
        config.kind = settings.kind;
        config.protocol = settings.protocol;
        config.enabled = true;
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
        config.codex_path = settings.codex_path.read(cx).value().trim().to_owned();
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

    fn save_provider(&mut self, activate: bool, cx: &mut Context<Self>) {
        let config = self.edited_provider(cx);
        let result = (|| {
            for model in &self.provider_settings.models {
                if let Some(input) = self.provider_settings.window_inputs.get(model) {
                    parse_context_window(input.read(cx).value())
                        .map_err(|error| format!("{model}：{error}"))?;
                }
            }
            if (self.pending.is_some() || self.preparing.is_some())
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
            let path = config.codex_path.clone();
            provider::Job::start(async move {
                Ok(provider::ModelCatalog {
                    models: chatgpt::models(&path).await?,
                    ..Default::default()
                })
            })
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

    fn chatgpt_auth(&mut self, login: bool, cx: &mut Context<Self>) {
        if self.provider_settings.auth_task.is_some() {
            self.provider_settings.auth_task = None;
            cx.notify();
            return;
        }
        let id = self.provider_settings.selected;
        let path = self
            .provider_settings
            .codex_path
            .read(cx)
            .value()
            .trim()
            .to_owned();
        self.provider_settings.message = Some("正在连接本机 Codex…".into());
        self.provider_settings.auth_task = Some(cx.spawn(async move |weak, cx| {
            let result = async {
                if login {
                    let (connection, url) =
                        provider::Job::start(async move { chatgpt::start_login(&path).await })?
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
                } else {
                    provider::Job::start(async move { chatgpt::account(&path).await })?
                        .result()
                        .await
                }
            }
            .await;
            let _ = weak.update(cx, |this, cx| {
                this.provider_settings.auth_task = None;
                if this.provider_settings.selected == id {
                    this.provider_settings.message = Some(match result {
                        Ok(message) | Err(message) => message,
                    });
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub fn settings_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        if self.provider_settings.section == Section::Mcp {
            return self.mcp_page(cx);
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
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(p.text_muted))
                    .child(if chatgpt {
                        "ChatGPT 订阅 · 本机 Codex 登录"
                    } else {
                        if settings.protocol == provider::Protocol::Anthropic {
                            "Anthropic Messages 协议"
                        } else {
                            "OpenAI Chat Completions 协议"
                        }
                    }),
            )
            .child(
                div().flex().gap_2().children(
                    [
                        (ProviderTab::Connection, "连接".to_owned()),
                        (
                            ProviderTab::Models,
                            format!("模型 · {}", settings.models.len()),
                        ),
                    ]
                    .into_iter()
                    .map(|(tab, label)| {
                        Button::new(label)
                            .kind(if settings.tab == tab {
                                ButtonKind::Tinted
                            } else {
                                ButtonKind::Plain
                            })
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.provider_settings.tab = tab;
                                this.provider_settings.message = None;
                                cx.notify();
                            }))
                    }),
                ),
            );
        let field = |label: &str, entry: &Entity<TextEntry>| {
            InputField::new("")
                .label(label.to_owned())
                .control(entry.clone())
                .theme(self.theme)
                .render()
                .w_full()
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
                .child(field("名称", &settings.name));
            if chatgpt {
                card = card
                    .child(field("Codex CLI 路径", &settings.codex_path))
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(
                                Button::new(if settings.auth_task.is_some() {
                                    "取消登录 / 检查"
                                } else {
                                    "登录 ChatGPT"
                                })
                                .kind(ButtonKind::Glass)
                                .theme(self.theme)
                                .render()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.chatgpt_auth(true, cx)),
                                ),
                            )
                            .child(
                                Button::new("检查登录状态")
                                    .kind(ButtonKind::Plain)
                                    .theme(self.theme)
                                    .render()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.chatgpt_auth(false, cx)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child("登录与令牌续期由官方 Codex 管理，无需 API Key。"),
                    );
            } else {
                card = card
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_size(px(12.)).child("协议"))
                            .children(
                                [
                                    (provider::Protocol::Anthropic, "Anthropic"),
                                    (provider::Protocol::OpenAi, "OpenAI 兼容"),
                                ]
                                .into_iter()
                                .map(|(protocol, label)| {
                                    Button::new(label)
                                        .kind(if settings.protocol == protocol {
                                            ButtonKind::Tinted
                                        } else {
                                            ButtonKind::Plain
                                        })
                                        .size(ButtonSize::Small)
                                        .theme(self.theme)
                                        .render()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.provider_settings.protocol = protocol;
                                            let address =
                                                this.provider_settings.url.read(cx).value();
                                            if matches!(
                                                address.trim_end_matches('/'),
                                                "https://api.deepseek.com"
                                                    | "https://api.deepseek.com/v1"
                                                    | "https://api.deepseek.com/anthropic"
                                            ) {
                                                let url =
                                                    if protocol == provider::Protocol::Anthropic {
                                                        "https://api.deepseek.com/anthropic"
                                                    } else {
                                                        "https://api.deepseek.com"
                                                    };
                                                this.provider_settings
                                                    .url
                                                    .update(cx, |entry, cx| {
                                                        entry.set_value(url, cx)
                                                    });
                                                this.provider_settings.original.models_url =
                                                    Some("https://api.deepseek.com/models".into());
                                            }
                                            cx.notify();
                                        }))
                                }),
                            ),
                    )
                    .child(field("Base URL", &settings.url))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_size(px(12.)).child("认证方式"))
                            .children([(true, "环境变量"), (false, "API Key")].into_iter().map(
                                |(use_env, label)| {
                                    Button::new(label)
                                        .kind(if settings.use_env == use_env {
                                            ButtonKind::Tinted
                                        } else {
                                            ButtonKind::Plain
                                        })
                                        .size(ButtonSize::Small)
                                        .theme(self.theme)
                                        .render()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.provider_settings.use_env = use_env;
                                            cx.notify();
                                        }))
                                },
                            )),
                    )
                    .child(if settings.use_env {
                        field("环境变量名", &settings.key_env)
                    } else {
                        div()
                            .flex()
                            .items_end()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .child(field("API Key（留空保留已保存的密钥）", &settings.key)),
                            )
                            .child(
                                IconButton::new(Icon::Trash, "清除已保存的密钥")
                                    .plain()
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.provider_settings.message = Some(
                                            match this.provider_settings.original.remove_key() {
                                                Ok(()) => "已清除系统凭据库中的密钥。".into(),
                                                Err(error) => error,
                                            },
                                        );
                                        cx.notify();
                                    })),
                            )
                    });
            }
            card = card.child(
                div()
                    .pt_2()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new(if settings.testing.is_some() {
                            "取消测试"
                        } else {
                            "测试连接"
                        })
                        .kind(ButtonKind::Glass)
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| this.test_provider(cx))),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child("使用默认模型发送一条简短请求。"),
                    ),
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
                        "未指定模型时使用 Codex 默认模型。"
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
                        .child(div().flex_1().min_w_0().child(model.clone()).when_some(
                            settings.context_windows.get(model),
                            |label, capacity| {
                                label.child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(p.text_muted))
                                        .child(format!("接口窗口：{capacity} tokens")),
                                )
                            },
                        ))
                        .when_some(settings.window_inputs.get(model), |row, input| {
                            row.child(
                                div()
                                    .w(px(180.))
                                    .flex_shrink_0()
                                    .child(field("上下文窗口（tokens）", input)),
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
            (Section::Storage, Icon::Folder),
            (Section::Memory, Icon::Database),
            (Section::Skills, Icon::Workflow),
            (Section::Mcp, Icon::Workflow),
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
                .child(
                    IconButton::new(Icon::Plus, "添加 API 供应商")
                        .plain()
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.add_provider(provider::Kind::Compatible, window, cx)
                        })),
                ),
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
                                if detected { " · 自动发现" } else { "" }
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
            div().px_2().pt_3().flex().flex_col().gap_2().child(
                Button::new("连接 ChatGPT")
                    .icon(Icon::User)
                    .kind(ButtonKind::Plain)
                    .size(ButtonSize::Small)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.add_provider(provider::Kind::ChatGpt, window, cx)
                    })),
            ),
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
                page = page.child(card).child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child("在左侧选择供应商，分别配置连接和模型。"),
                );
            }
            Section::Appearance => {
                page = page.child(div().mt_4().font_weight(FontWeight::SEMIBOLD).child("主题"));
                card = card
                    .child(row(
                        "窗口外观",
                        "选择当前窗口使用的主题。".into(),
                        self.theme.name().into(),
                    ))
                    .child(div().border_t_1().border_color(rgb(p.border)))
                    .child(
                        div().py_4().flex().gap_3().children(
                            [
                                ("浅色", ThemePreset::Moonstone),
                                ("深色", ThemePreset::DarkObsidian),
                            ]
                            .into_iter()
                            .map(|(label, theme)| {
                                Button::new(label)
                                    .kind(if self.theme == theme {
                                        ButtonKind::Tinted
                                    } else {
                                        ButtonKind::Glass
                                    })
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.theme = theme;
                                        this.composer.update(cx, |composer, cx| {
                                            composer.set_theme(theme, cx)
                                        });
                                        this.search
                                            .update(cx, |entry, cx| entry.set_theme(theme, cx));
                                        this.provider_settings.set_theme(theme, cx);
                                        this.skills_ui.set_theme(theme, cx);
                                        this.mcp_ui.set_theme(theme, cx);
                                        this.sync_skill_context(cx);
                                        cx.notify();
                                    }))
                            }),
                        ),
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
                        crate::database::path(&self.path).display().to_string(),
                        "WAL".into(),
                    ))
                    .child(row(
                        "数据目录",
                        self.path
                            .parent()
                            .unwrap_or(std::path::Path::new("."))
                            .display()
                            .to_string(),
                        "".into(),
                    ))
                    .child(div().border_t_1().border_color(rgb(p.border)))
                    .child(row(
                        "API Key",
                        "密钥由操作系统凭据库管理。".into(),
                        "不写入 SQLite".into(),
                    ));
                page = page.child(card);
            }
            Section::Shortcuts => {
                let modifier = if cfg!(target_os = "macos") {
                    "⌘"
                } else {
                    "Ctrl"
                };
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
                    if index > 0 {
                        card = card.child(div().border_t_1().border_color(rgb(p.border)));
                    }
                    card = card.child(row(title, "".into(), key));
                }
                page = page.child(card);
            }
            _ => {}
        }
        self.settings_container(page)
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;
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
