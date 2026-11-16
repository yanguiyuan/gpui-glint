use super::*;
use std::sync::{Arc, atomic::Ordering};

pub(super) struct Inputs {
    pub inheritance: Entity<Select>,
    pub inherited_mode: usize,
    pub config: oauth::Config,
    pub fields: [Entity<TextEntry>; 5],
    pub secret: Entity<TextEntry>,
    pub callback: Entity<TextEntry>,
    pub ready: Option<(String, oauth::Access)>,
    pub message: String,
    pub error: Option<String>,
    pub pending: Option<Arc<oauth::Session>>,
    pending_key: Option<String>,
    task: Option<Task<()>>,
    generation: u64,
    show_advanced: bool,
    show_presets: bool,
    form_scroll: gpui::ScrollHandle,
}
impl Inputs {
    pub fn new(cx: &mut Context<Workbench>) -> Self {
        Self {
            inheritance: cx.new(|cx| Select::new([], cx).placeholder("选择目录授权方案")),
            inherited_mode: 2,
            config: oauth::Config::default(),
            fields: std::array::from_fn(|index| {
                Workbench::entry(
                    "",
                    [
                        "https://…/authorize",
                        "https://…/token",
                        "Client ID 或 {{client_id}}",
                        "与服务商后台登记的地址一致",
                        "填写需要授权的权限",
                    ][index],
                    cx,
                )
            }),
            secret: Workbench::entry("", "Client Secret 或 {{client_secret}}", cx),
            callback: Workbench::entry("", "粘贴含 code 与 state 的完整回调地址", cx),
            ready: None,
            message: "尚未授权".into(),
            error: None,
            pending: None,
            pending_key: None,
            task: None,
            generation: 0,
            show_advanced: false,
            show_presets: false,
            form_scroll: gpui::ScrollHandle::new(),
        }
    }
    pub fn snapshot(&self, cx: &App) -> oauth::Config {
        let mut config = self.config.clone();
        config.authorization_url = self.fields[0].read(cx).value().trim().into();
        config.token_url = self.fields[1].read(cx).value().trim().into();
        config.client_id = self.fields[2].read(cx).value().trim().into();
        config.redirect_uri = self.fields[3].read(cx).value().trim().into();
        config.scope = self.fields[4].read(cx).value().trim().into();
        config.client_secret = self.secret.read(cx).value().into();
        config
    }
    pub fn is_busy(&self) -> bool {
        self.task.is_some() || self.pending.is_some()
    }
    pub fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(session) = self.pending.take() {
            session.cancelled.store(true, Ordering::Release);
        }
        self.task = None;
        self.pending_key = None;
    }
    pub fn load(&mut self, config: oauth::Config, cx: &mut Context<Workbench>) {
        self.cancel();
        for (field, value) in self.fields.iter().zip([
            &config.authorization_url,
            &config.token_url,
            &config.client_id,
            &config.redirect_uri,
            &config.scope,
        ]) {
            field.update(cx, |entry, cx| entry.set_value(value.clone(), cx));
        }
        let secret = config.client_secret.clone();
        self.config = config;
        self.show_advanced = false;
        self.show_presets = false;
        self.form_scroll.set_offset(gpui::point(px(0.), px(0.)));
        self.ready = None;
        self.secret
            .update(cx, |entry, cx| entry.set_value(secret, cx));
        self.callback
            .update(cx, |entry, cx| entry.set_value("", cx));
        self.message = "发送时自动读取系统凭据；未授权时请先授权".into();
        self.error = None;
    }
}
impl Drop for Inputs {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod auth_profile_tests {
    use super::*;

    #[gpui::test]
    fn auth_profile_switches_modes_and_persists_secret_in_sqlite(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profiles.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            let mut groups = RequestGroups::create_collection("API", &[]).unwrap();
            groups.defaults.insert(
                String::new(),
                model::DirectoryDefaults {
                    auth_profiles: vec![
                        model::AuthProfile {
                            id: "application".into(),
                            name: "应用授权".into(),
                            token: "{{app_token}}".into(),
                            oauth: None,
                        },
                        model::AuthProfile {
                            id: "user".into(),
                            name: "用户授权".into(),
                            token: String::new(),
                            oauth: Some(oauth::Config {
                                client_id: "app".into(),
                                client_secret: "stored-secret".into(),
                                ..Default::default()
                            }),
                        },
                    ],
                    ..Default::default()
                },
            );
            host.workspace.collections.push(groups);
            host.add_request(cx);
            host.config = 1;
            host.change_auth_mode(1, false, cx);
            host
        });
        assert!(window.debug_bounds("bearer-token-input").is_some());
        let switch = window.debug_bounds("auth-inherit-switch").unwrap();
        window.simulate_click(switch.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            let mut draft = host.snapshot(cx);
            assert_eq!(draft.auth_profile.as_deref(), Some("application"));
            host.resolve_request_auth(&mut draft).unwrap();
            assert_eq!(draft.token, "{{app_token}}");
            host.change_auth_mode(2, true, cx);
            assert_eq!(host.snapshot(cx).auth_profile.as_deref(), Some("user"));
            assert_eq!(host.oauth.secret.read(cx).value(), "stored-secret");
            host.oauth
                .secret
                .update(cx, |entry, cx| entry.set_value("edited-secret", cx));
        });
        window.run_until_parked();
        host.update(window, |host, cx| {
            host.persist(cx);
        });
        let mut store = storage::Store::open(&path).unwrap();
        let workspace = store.load().unwrap();
        assert_eq!(
            workspace.collections[0].defaults[""].auth_profiles[1]
                .oauth
                .as_ref()
                .unwrap()
                .client_secret,
            "edited-secret"
        );
        host.update(window, |host, cx| {
            let draft = host.snapshot(cx);
            host.load(&draft, cx);
            assert_eq!(host.oauth.secret.read(cx).value(), "edited-secret");
            host.change_auth_mode(0, false, cx);
            assert!(host.snapshot(cx).auth_profile.is_none());
            assert!(!host.snapshot(cx).auth);
        });
        assert!(window.debug_bounds("bearer-token-input").is_none());
        assert!(window.debug_bounds("auth-inherit-switch").is_some());
    }
}

impl Workbench {
    pub(super) fn oauth_secret_changed(&mut self, cx: &mut Context<Self>) {
        let secret = self.oauth.secret.read(cx).value().to_owned();
        if let Some(id) = self.snapshot(cx).auth_profile {
            if let Some(request) = self.workspace.requests.get(self.active) {
                if let Some(groups) = self
                    .workspace
                    .collections
                    .iter_mut()
                    .find(|g| g.collection == request.collection)
                {
                    let path = groups
                        .auth_profiles(&request.group)
                        .into_iter()
                        .find(|(_, p)| p.id == id)
                        .map(|(path, _)| path.to_owned());
                    if let Some(path) = path {
                        if let Some(config) = groups
                            .defaults
                            .get_mut(&path)
                            .and_then(|d| d.auth_profiles.iter_mut().find(|p| p.id == id))
                            .and_then(|p| p.oauth.as_mut())
                        {
                            if config.client_secret != secret {
                                config.client_secret = secret;
                                self.workspace.storage_dirty = true;
                            }
                        }
                    }
                }
            }
        }
        self.schedule_persist(cx);
    }
    pub(super) fn refresh_auth_profiles(&mut self, cx: &mut Context<Self>) {
        let selected = self.authentication.read(cx).value().selected.clone();
        let options = self.request_auth_options();
        self.authentication
            .update(cx, |field, cx| field.set_options(options, cx));
        let mode = self.oauth.inherited_mode;
        let mut options = Vec::new();
        if let Some(request) = self.workspace.requests.get(self.active) {
            if let Some(groups) = self
                .workspace
                .collections
                .iter()
                .find(|g| g.collection == request.collection)
            {
                options.extend(
                    groups
                        .auth_profiles(&request.group)
                        .into_iter()
                        .filter(|(_, p)| {
                            if mode == 2 {
                                p.oauth.is_some()
                            } else if mode == 1 {
                                p.oauth.is_none()
                            } else {
                                false
                            }
                        })
                        .map(|(path, p)| {
                            SelectOption::new(
                                format!("profile:{}", p.id),
                                format!(
                                    "{} / {}",
                                    if path.is_empty() {
                                        &groups.collection
                                    } else {
                                        path
                                    },
                                    p.name
                                ),
                            )
                        }),
                );
            }
        }
        self.oauth.inheritance.update(cx, |field, cx| {
            field.set_options(options, cx);
            field.set_selected(selected.filter(|v| v.starts_with("profile:")), cx);
        });
    }
    fn change_auth_mode(&mut self, mode: usize, inherit: bool, cx: &mut Context<Self>) {
        let mode = if inherit && mode == 0 {
            self.workspace
                .requests
                .get(self.active)
                .and_then(|r| {
                    let groups = self
                        .workspace
                        .collections
                        .iter()
                        .find(|g| g.collection == r.collection)?;
                    let profiles = groups.auth_profiles(&r.group);
                    let default = groups.request_defaults(&r.group).auth_profile;
                    let profile = profiles
                        .iter()
                        .find(|(_, p)| Some(&p.id) == default.as_ref())
                        .or_else(|| profiles.first())?
                        .1;
                    Some(if profile.oauth.is_some() { 2 } else { 1 })
                })
                .unwrap_or(2)
        } else {
            mode
        };
        self.oauth.cancel();
        self.oauth.ready = None;
        self.oauth.error = None;
        self.oauth.inherited_mode = mode;
        let mut selected = ["none", "bearer", "oauth2"][mode].to_owned();
        if inherit {
            selected = "profile:".into();
            if let Some(r) = self.workspace.requests.get(self.active) {
                if let Some(g) = self
                    .workspace
                    .collections
                    .iter()
                    .find(|g| g.collection == r.collection)
                {
                    if let Some((_, p)) = g.auth_profiles(&r.group).into_iter().find(|(_, p)| {
                        mode == 2 && p.oauth.is_some() || mode == 1 && p.oauth.is_none()
                    }) {
                        selected = format!("profile:{}", p.id);
                    }
                }
            }
        }
        self.authentication.update(cx, |f, cx| {
            f.set_selected(Some(selected.into()), cx);
            f.set_input_disabled(inherit || mode != 1, cx);
        });
        self.sync_inherited_auth(cx);
        self.refresh_auth_profiles(cx);
        self.schedule_persist(cx);
        cx.notify();
    }
    pub(super) fn auth_is_oauth(&self, cx: &App) -> bool {
        let selected = self.authentication.read(cx).value().selected.as_deref();
        selected == Some("oauth2")
            || selected.is_some_and(|s| s.starts_with("profile:")) && self.oauth.inherited_mode == 2
    }
    pub(super) fn request_auth_options(&self) -> Vec<SelectOption> {
        let mut options = vec![
            SelectOption::new("none", "No Auth"),
            SelectOption::new("bearer", "Bearer（独立配置）"),
            SelectOption::new("oauth2", "OAuth 2.0（独立配置）"),
        ];
        if let Some(request) = self.workspace.requests.get(self.active) {
            if let Some(groups) = self
                .workspace
                .collections
                .iter()
                .find(|g| g.collection == request.collection)
            {
                options.extend(groups.auth_profiles(&request.group).into_iter().map(
                    |(path, profile)| {
                        SelectOption::new(
                            format!("profile:{}", profile.id),
                            format!(
                                "继承 · {} / {}",
                                if path.is_empty() {
                                    &groups.collection
                                } else {
                                    path
                                },
                                profile.name
                            ),
                        )
                    },
                ));
            }
        }
        options
    }
    pub(super) fn resolve_request_auth(&self, draft: &mut Draft) -> Result<(), String> {
        if draft.auth_profile.is_none() {
            return Ok(());
        }
        let request = self
            .workspace
            .requests
            .get(self.active)
            .ok_or("请求不存在")?;
        self.workspace
            .collections
            .iter()
            .find(|g| g.collection == request.collection)
            .ok_or("授权所属集合不存在")?
            .resolve_auth(&request.group, draft)
    }
    pub(super) fn sync_inherited_auth(&mut self, cx: &mut Context<Self>) {
        let mut draft = self.snapshot(cx);
        if draft.auth_profile.is_some() {
            match self.resolve_request_auth(&mut draft) {
                Ok(()) => {
                    self.oauth.inherited_mode = if draft.oauth.is_some() { 2 } else { 1 };
                    let config = draft.oauth.unwrap_or_default();
                    if self.oauth.snapshot(cx) != config {
                        self.oauth.load(config, cx);
                    }
                }
                Err(error) => {
                    self.oauth.load(oauth::Config::default(), cx);
                    self.oauth.error = Some(error);
                }
            }
        }
    }
    fn oauth_resolve(&self, config: &oauth::Config, cx: &App) -> Result<oauth::Config, String> {
        let values = self.active_environment_values(cx);
        config.resolve(&template::TemplateResolver::new(&values))
    }
    fn oauth_secret(&self, cx: &App) -> Result<String, String> {
        let values = self.active_environment_values(cx);
        template::TemplateResolver::new(&values)
            .resolve(self.oauth.secret.read(cx).value())
            .map_err(|_| "Client Secret 模板解析失败，请检查当前环境变量".into())
    }
    pub(super) fn oauth_key(&self, config: &oauth::Config, cx: &App) -> String {
        // Different workspace paths and environments must never share a credential implicitly.
        config.key(&format!(
            "{}\n{}",
            self.oauth_namespace,
            self.environment_name(self.selected_environment(cx))
        ))
    }
    pub(super) fn oauth_before_send(&mut self, cx: &mut Context<Self>) -> bool {
        let draft = self.snapshot(cx);
        let mut resolved = draft.clone();
        if let Err(error) = self.resolve_request_auth(&mut resolved) {
            self.error = Some(error.clone());
            self.oauth.error = Some(error);
            cx.notify();
            return true;
        }
        let Some(config) = resolved.oauth.clone() else {
            return false;
        };
        let config = match self.oauth_resolve(&config, cx) {
            Ok(config) => config,
            Err(error) => {
                self.oauth.error = Some(error.clone());
                self.error = Some(error);
                cx.notify();
                return true;
            }
        };
        let key = self.oauth_key(&config, cx);
        if self
            .oauth
            .ready
            .as_ref()
            .is_some_and(|(stored, access)| stored == &key && access.valid())
        {
            return false;
        }
        if self.oauth.pending.is_some() || self.oauth.task.is_some() {
            self.error = Some("请先完成或取消正在进行的 OAuth 授权".into());
            cx.notify();
            return true;
        }
        let request_id = self.workspace.requests[self.active].id.clone();
        let environment = self.selected_environment(cx);
        let proxy = self.effective_proxy(environment);
        self.notice = "检查 OAuth 授权及有效期…".into();
        self.sending = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    oauth::access(&config, &key, false, &proxy).map(|access| (key, access))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.sending = None;
                if this.workspace.requests.get(this.active).map(|r| &r.id) != Some(&request_id)
                    || this.selected_environment(cx) != environment
                    || this.snapshot(cx) != draft
                {
                    return;
                }
                match result {
                    Ok(ready) => {
                        this.oauth.ready = Some(ready);
                        this.oauth.error = None;
                        this.oauth.message = "授权有效".into();
                        this.send(cx);
                    }
                    Err(error) => {
                        this.oauth.error = Some(error.clone());
                        this.error = Some(error.clone());
                        this.notice = error.clone();
                        this.request_error_toast(
                            "OAuth 授权不可用",
                            this.environment_name(environment).to_owned().as_str(),
                            &error,
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
        true
    }
    pub(super) fn oauth_attach(
        &self,
        request: PreparedRequest,
        draft: &Draft,
        cx: &App,
    ) -> Result<PreparedRequest, String> {
        if let Some(config) = &draft.oauth {
            let config = self.oauth_resolve(config, cx)?;
            let key = self.oauth_key(&config, cx);
            let access = self
                .oauth
                .ready
                .as_ref()
                .filter(|(saved, access)| saved == &key && access.valid())
                .ok_or("OAuth 授权不可用，请重新发送以刷新")?;
            request.with_oauth_token(&access.1.token)
        } else {
            Ok(request)
        }
    }
    fn oauth_authorize(&mut self, cx: &mut Context<Self>) {
        if self.sending.is_some() || self.oauth.task.is_some() || self.oauth.pending.is_some() {
            return;
        }
        let mut draft = self.snapshot(cx);
        if let Err(error) = self.resolve_request_auth(&mut draft) {
            self.oauth.error = Some(error);
            cx.notify();
            return;
        }
        let config = match self.oauth_resolve(&self.oauth.snapshot(cx), cx) {
            Ok(config) => config,
            Err(error) => {
                self.oauth.error = Some(error);
                cx.notify();
                return;
            }
        };
        let secret = match self.oauth_secret(cx) {
            Ok(secret) => secret,
            Err(error) => {
                self.oauth.error = Some(error);
                cx.notify();
                return;
            }
        };
        let session = match oauth::Session::start(config.clone()) {
            Ok(session) => Arc::new(session),
            Err(error) => {
                self.oauth.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.oauth.cancel();
        self.oauth.error = None;
        self.oauth.ready = None;
        self.oauth.pending = Some(session.clone());
        self.oauth.pending_key = Some(self.oauth_key(&config, cx));
        self.oauth.message = "等待浏览器授权，10 分钟后超时".into();
        self.capture_draft(cx);
        self.persist(cx);
        cx.open_url(&session.url);
        if !config.manual_callback {
            let key = self.oauth_key(&config, cx);
            let proxy = self.effective_proxy(self.selected_environment(cx));
            let generation = self.oauth.generation;
            self.oauth.task = Some(cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let code = session.wait()?;
                        session
                            .exchange(&key, &code, &secret, &proxy)
                            .map(|access| (key, access))
                    })
                    .await;
                let _ = this.update(cx, |this, cx| this.oauth_finished(generation, result, cx));
            }));
        }
        cx.notify();
    }
    fn oauth_exchange_manual(&mut self, cx: &mut Context<Self>) {
        if self.oauth.task.is_some() || self.sending.is_some() {
            return;
        }
        let Some(session) = self.oauth.pending.clone() else {
            self.oauth.error = Some("请先点击授权，再粘贴本次回调地址".into());
            cx.notify();
            return;
        };
        if self.oauth_resolve(&self.oauth.snapshot(cx), cx).as_ref() != Ok(&session.config)
            || self.oauth.pending_key.as_deref()
                != Some(self.oauth_key(&session.config, cx).as_str())
        {
            self.oauth.cancel();
            self.oauth.error = Some("授权配置或环境已变更，请重新授权".into());
            cx.notify();
            return;
        }
        let secret = match self.oauth_secret(cx) {
            Ok(secret) => secret,
            Err(error) => {
                self.oauth.error = Some(error);
                cx.notify();
                return;
            }
        };
        let code = match session.code(self.oauth.callback.read(cx).value()) {
            Ok(code) => code,
            Err(error) => {
                self.oauth.error = Some(error);
                cx.notify();
                return;
            }
        };
        let key = self.oauth_key(&session.config, cx);
        self.oauth
            .callback
            .update(cx, |entry, cx| entry.set_value("", cx));
        self.oauth.message = "正在换取 Token…".into();
        let proxy = self.effective_proxy(self.selected_environment(cx));
        let generation = self.oauth.generation;
        self.oauth.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    session
                        .exchange(&key, &code, &secret, &proxy)
                        .map(|access| (key, access))
                })
                .await;
            let _ = this.update(cx, |this, cx| this.oauth_finished(generation, result, cx));
        }));
        cx.notify();
    }
    fn oauth_finished(
        &mut self,
        generation: u64,
        result: Result<(String, oauth::Access), String>,
        cx: &mut Context<Self>,
    ) {
        if self.oauth.generation != generation {
            return;
        }
        self.oauth.pending = None;
        self.oauth.pending_key = None;
        self.oauth.task = None;
        match result {
            Ok(ready) => {
                self.oauth.message = format!(
                    "已授权 · 到期时间 {}",
                    chrono::DateTime::from_timestamp(ready.1.expires_at as i64, 0)
                        .map(|date| date
                            .with_timezone(&chrono::Local)
                            .format("%Y-%m-%d %H:%M:%S")
                            .to_string())
                        .unwrap_or_default()
                );
                self.oauth.ready = Some(ready);
                self.oauth.error = None;
            }
            Err(error) => {
                self.oauth.ready = None;
                self.oauth.error = Some(error);
                self.oauth.message = "授权未完成".into();
            }
        }
        cx.notify();
    }
    fn oauth_manage(&mut self, forget: bool, cx: &mut Context<Self>) {
        if self.sending.is_some() || self.oauth.task.is_some() {
            return;
        }
        self.oauth.cancel();
        self.oauth.ready = None;
        let config = match self.oauth_resolve(&self.oauth.snapshot(cx), cx) {
            Ok(config) => config,
            Err(error) => {
                self.oauth.error = Some(error);
                cx.notify();
                return;
            }
        };
        let key = self.oauth_key(&config, cx);
        let proxy = self.effective_proxy(self.selected_environment(cx));
        let generation = self.oauth.generation;
        self.oauth.message = if forget {
            "正在清除本地授权…"
        } else {
            "正在刷新授权…"
        }
        .into();
        self.oauth.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if forget {
                        oauth::forget(&key).map(|_| None)
                    } else {
                        oauth::access(&config, &key, true, &proxy).map(|access| Some((key, access)))
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.oauth.generation != generation {
                    return;
                }
                match result {
                    Ok(None) => {
                        this.oauth.task = None;
                        this.oauth.error = None;
                        this.oauth.message =
                            "已清除本地凭据；服务商端授权可到其账号设置中撤销".into();
                    }
                    Ok(Some(ready)) => this.oauth_finished(generation, Ok(ready), cx),
                    Err(error) => this.oauth_finished(generation, Err(error), cx),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn oauth_apply_preset(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.sending.is_some() || self.oauth.task.is_some() || self.oauth.pending.is_some() {
            return;
        }
        let preset = plugins::oauth_presets(&self.enabled_plugin_catalog())
            .into_iter()
            .find(|entry| entry.key == key);
        let result = preset
            .ok_or("预设插件已移除或更新，请重新选择".to_owned())
            .and_then(|entry| entry.preset.config());
        match result {
            Ok(config) => self.oauth.load(config, cx),
            Err(error) => {
                self.oauth.error = Some(error);
                self.oauth.show_presets = false;
            }
        }
        cx.notify();
    }
    fn oauth_preset_menu(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let presets = plugins::oauth_presets(&self.enabled_plugin_catalog());
        if presets.is_empty() {
            return None;
        }
        let p = self.theme.palette();
        let owner = cx.entity().downgrade();
        let mut menu = div()
            .id("oauth-preset-options")
            .max_h(px(280.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        for preset in presets {
            let key = preset.key.clone();
            let selector = format!("oauth-preset-{}", preset.key);
            menu = menu.child(
                div()
                    .id(gpui::SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .px_2()
                    .h(px(28.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(p.surface_subtle)))
                    .flex()
                    .items_center()
                    .child(div().text_size(px(12.)).truncate().child(preset.title))
                    .on_click(cx.listener(move |this, _, _, cx| this.oauth_apply_preset(&key, cx))),
            );
        }
        let trigger = Button::new("授权预设")
            .icon(Icon::ChevronDown)
            .kind(ButtonKind::Plain)
            .size(ButtonSize::Mini)
            .theme(self.theme)
            .render()
            .debug_selector(|| "oauth-preset-trigger".into());
        Some(
            Popover::new("oauth-presets-popover", trigger, menu)
                .theme(self.theme)
                .width(180.)
                .padding(4.)
                .trigger_height(21.)
                .open(self.oauth.show_presets)
                .on_open_change(move |open, _, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.oauth.show_presets = open;
                        cx.notify();
                    });
                })
                .render()
                .into_any_element(),
        )
    }
    fn oauth_input(
        &self,
        entry: &Entity<TextEntry>,
        selector: &'static str,
    ) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        div()
            .id(selector)
            .debug_selector(move || selector.into())
            .h(px(32.))
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .px_3()
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .child(div().flex_1().min_w_0().child(entry.clone()))
    }
    fn oauth_field_row(
        &self,
        label: &'static str,
        index: usize,
        selector: &'static str,
    ) -> gpui::Div {
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .w(px(76.))
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .text_color(rgb(self.theme.palette().text_muted))
                    .child(label),
            )
            .child(self.oauth_input(&self.oauth.fields[index], selector))
    }
    pub(super) fn authentication_configuration(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let selected = self.authentication.read(cx).value().selected.as_deref();
        let inherited = selected.is_some_and(|s| s.starts_with("profile:"));
        let mode = if inherited {
            self.oauth.inherited_mode
        } else {
            match selected {
                Some("oauth2") => 2,
                Some("bearer") => 1,
                _ => 0,
            }
        };
        let owner = cx.entity().downgrade();
        let switch_owner = owner.clone();
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .flex_shrink_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        SegmentedControl::new([
                            SegmentOption::new("No Auth"),
                            SegmentOption::new("Bearer"),
                            SegmentOption::new("OAuth 2.0"),
                        ])
                        .id("oauth-auth-mode")
                        .selected(mode)
                        .theme(self.theme)
                        .on_change(move |index, _, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                this.change_auth_mode(index, inherited && index != 0, cx)
                            });
                        })
                        .render(),
                    )
                    .child(
                        Switch::new("继承")
                            .checked(inherited)
                            .theme(self.theme)
                            .on_change(move |value, _, cx| {
                                let _ = switch_owner
                                    .update(cx, |this, cx| this.change_auth_mode(mode, value, cx));
                            })
                            .render()
                            .debug_selector(|| "auth-inherit-switch".into()),
                    ),
            )
            .when(mode == 2 && !inherited, |header| {
                header.when_some(self.oauth_preset_menu(cx), |header, menu| {
                    header.child(menu)
                })
            });
        let inherited_picker = || {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_size(px(11.)).child("目录授权方案"))
                .child(self.oauth.inheritance.clone())
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child(
                            "配置跟随目录更新；同一环境复用授权。切换类型可选择对应的授权方案。",
                        ),
                )
        };
        if mode != 2 {
            let mut view = div().flex().flex_col().gap_3().child(header);
            if inherited {
                view = view
                    .child(inherited_picker())
                    .children(self.oauth.error.clone());
            } else if mode == 1 {
                view = view
                    .child(div().text_size(px(11.)).child("Token"))
                    .child(
                        div()
                            .id("bearer-token-input")
                            .debug_selector(|| "bearer-token-input".into())
                            .h(px(32.))
                            .px_3()
                            .border_1()
                            .border_color(rgb(p.border))
                            .rounded_md()
                            .child(self.authentication.read(cx).text_input()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child("支持直接输入 Token 或使用 {{access_token}} 等环境变量。"),
                    );
            } else {
                view = view.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child("此请求不自动生成 Authorization 请求头。"),
                );
            }
            return view;
        }
        let mut form = div()
            .id("oauth-form-scroll")
            .track_scroll(&self.oauth.form_scroll)
            .debug_selector(|| "oauth-form-scroll".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .gap_3()
                    .pb_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("Client ID"),
                            )
                            .child(self.oauth_input(&self.oauth.fields[2], "oauth-client-id")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child("Client Secret"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(rgb(p.text_muted))
                                            .child("选填"),
                                    ),
                            )
                            .child(self.oauth_input(&self.oauth.secret, "oauth-client-secret")),
                    ),
            )
            .child(self.oauth_field_row("授权地址", 0, "oauth-authorization-url"))
            .child(self.oauth_field_row("Token 地址", 1, "oauth-token-url"))
            .child(self.oauth_field_row("回调地址", 3, "oauth-redirect-uri"))
            .child(self.oauth_field_row("Scope", 4, "oauth-scope"));
        let protocol = format!(
            "{}{} · {}",
            if self.oauth.config.pkce {
                "PKCE S256 · "
            } else {
                ""
            },
            if self.oauth.config.basic_auth {
                "Basic 认证"
            } else {
                "表单认证"
            },
            if self.oauth.config.manual_callback {
                "手动回调"
            } else {
                "自动接收回调"
            }
        );
        form = form.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .pt_1()
                .child(
                    Button::new("高级设置")
                        .icon(if self.oauth.show_advanced {
                            Icon::ChevronDown
                        } else {
                            Icon::ChevronRight
                        })
                        .kind(ButtonKind::Plain)
                        .size(ButtonSize::Mini)
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "oauth-advanced-toggle".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.oauth.show_advanced = !this.oauth.show_advanced;
                            if this.oauth.show_advanced {
                                this.oauth.form_scroll.scroll_to_bottom();
                            } else {
                                this.oauth
                                    .form_scroll
                                    .set_offset(gpui::point(px(0.), px(0.)));
                            }
                            cx.notify();
                        })),
                )
                .when(!self.oauth.show_advanced, |row| {
                    row.child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(protocol),
                    )
                }),
        );
        if self.oauth.show_advanced {
            let pkce_owner = cx.entity().downgrade();
            let basic_owner = cx.entity().downgrade();
            let callback_owner = cx.entity().downgrade();
            let advanced = div()
                .id("oauth-advanced-panel")
                .debug_selector(|| "oauth-advanced-panel".into())
                .rounded(px(6.))
                .bg(rgb(p.surface_subtle))
                .px_3()
                .py_2()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    Checkbox::new("使用 PKCE S256")
                        .checked(self.oauth.config.pkce)
                        .theme(self.theme)
                        .on_change(move |value, _, cx| {
                            let _ = pkce_owner.update(cx, |this, cx| {
                                this.oauth.cancel();
                                this.oauth.ready = None;
                                this.oauth.config.pkce = value;
                                cx.notify();
                            });
                        })
                        .render()
                        .text_size(px(11.)),
                )
                .child(
                    Checkbox::new("使用 Basic 发送客户端凭据")
                        .checked(self.oauth.config.basic_auth)
                        .theme(self.theme)
                        .on_change(move |value, _, cx| {
                            let _ = basic_owner.update(cx, |this, cx| {
                                this.oauth.cancel();
                                this.oauth.ready = None;
                                this.oauth.config.basic_auth = value;
                                cx.notify();
                            });
                        })
                        .render()
                        .text_size(px(11.)),
                )
                .child(
                    Checkbox::new("手动粘贴回调地址")
                        .checked(self.oauth.config.manual_callback)
                        .theme(self.theme)
                        .on_change(move |value, _, cx| {
                            let _ = callback_owner.update(cx, |this, cx| {
                                this.oauth.cancel();
                                this.oauth.ready = None;
                                this.oauth.config.manual_callback = value;
                                cx.notify();
                            });
                        })
                        .render()
                        .text_size(px(11.)),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(p.text_muted))
                        .child(
                            "回调地址需在服务商后台登记。Secret 随授权配置保存，打开时直接回填。",
                        ),
                )
                .child(
                    div().flex().justify_start().child(
                        Button::new("清除本地授权")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.oauth_manage(true, cx))),
                    ),
                );
            form = form.child(advanced);
        }
        if inherited {
            let config = self.oauth.snapshot(cx);
            form = div()
                .id("oauth-inherited-form")
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .gap_3()
                .child(inherited_picker())
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child(format!(
                            "Client ID：{} · Scope：{}",
                            config.client_id, config.scope
                        )),
                )
                .child(div().text_size(px(11.)).child("Client Secret（选填）"))
                .child(self.oauth_input(&self.oauth.secret, "oauth-client-secret"));
        }
        let pending = self.oauth.pending.is_some();
        let busy = self.oauth.task.is_some() || self.sending.is_some();
        if let Some(error) = &self.oauth.error {
            form = form.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(self.theme.color(0xd9342b)))
                    .child(error.clone()),
            );
        }
        let connected = self.oauth.ready.as_ref().is_some_and(|(key, access)| {
            access.valid()
                && self
                    .oauth_resolve(&self.oauth.snapshot(cx), cx)
                    .is_ok_and(|config| *key == self.oauth_key(&config, cx))
        });
        let (status, color) = if self.oauth.error.is_some() {
            ("需要处理", self.theme.color(0xd9342b))
        } else if pending || busy {
            ("授权进行中", p.accent)
        } else if connected {
            ("已授权", self.theme.color(0x25845b))
        } else {
            ("授权待检查", p.text_muted)
        };
        let details = if pending {
            if self.oauth.config.manual_callback {
                "在上方粘贴浏览器的回调地址".to_owned()
            } else {
                "请在浏览器中完成授权".to_owned()
            }
        } else if connected {
            self.oauth
                .message
                .trim_start_matches("已授权 · ")
                .to_owned()
        } else {
            "发送时自动检查并刷新凭据".to_owned()
        };
        let footer = div()
            .id("oauth-action-bar")
            .debug_selector(|| "oauth-action-bar".into())
            .flex_shrink_0()
            .border_t_1()
            .border_color(rgb(p.border))
            .pt_3()
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
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().size(px(6.)).rounded_full().bg(rgb(color)))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(status),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .truncate()
                            .child(details),
                    ),
            )
            .when(!pending, |row| {
                row.child(
                    IconButton::new(Icon::Refresh, "刷新 OAuth Token")
                        .plain()
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "oauth-refresh".into())
                        .on_click(cx.listener(|this, _, _, cx| this.oauth_manage(false, cx))),
                )
            })
            .when(pending, |row| {
                row.child(
                    Button::new("取消")
                        .kind(ButtonKind::Plain)
                        .size(ButtonSize::Small)
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "oauth-cancel".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.oauth.cancel();
                            this.oauth.message = "已取消授权".into();
                            this.oauth.callback.update(cx, |e, cx| e.set_value("", cx));
                            cx.notify();
                        })),
                )
            })
            .child(
                Button::new(if pending && self.oauth.config.manual_callback {
                    "完成授权"
                } else if pending {
                    "等待浏览器…"
                } else if connected {
                    "重新授权"
                } else {
                    "浏览器授权"
                })
                .when(
                    busy || pending && !self.oauth.config.manual_callback,
                    |button| button.disabled(),
                )
                .size(ButtonSize::Small)
                .theme(self.theme)
                .render()
                .debug_selector(|| "oauth-authorize".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.oauth.pending.is_some() && this.oauth.config.manual_callback {
                        this.oauth_exchange_manual(cx);
                    } else {
                        this.oauth_authorize(cx);
                    }
                })),
            );
        div()
            .h_full()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(header)
            .child(form)
            .when(self.oauth.config.manual_callback && pending, |view| {
                view.child(
                    div()
                        .id("oauth-manual-callback")
                        .debug_selector(|| "oauth-manual-callback".into())
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(11.))
                                .child("粘贴浏览器跳转后的完整回调地址"),
                        )
                        .child(self.oauth_input(&self.oauth.callback, "oauth-callback-input")),
                )
            })
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn oauth_environment_templates_apply_to_all_ui_paths_and_isolate_changed_clients(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&directory.path().join("templates.sqlite3")).unwrap(),
                cx,
            );
            host.workspace
                .collections
                .push(RequestGroups::create_collection("OAuth", &[]).unwrap());
            host.add_request(cx);
            host.authentication.update(cx, |field, cx| {
                field.set_selected(Some("oauth2".into()), cx)
            });
            host.endpoint.update(cx, |field, cx| {
                field.set_text("https://example.test/api", cx)
            });
            host.oauth.load(
                oauth::Config {
                    client_id: "{{wps_app_id}}".into(),
                    manual_callback: true,
                    ..plugins::test_oauth_config()
                },
                cx,
            );
            host
        });
        host.update(window, |host, cx| {
            let environment = host.selected_environment(cx);
            let id = host.workspace.environment_inputs[environment]
                .save(None, "wps_app_id", "app-one", false)
                .unwrap();
            host.workspace.environment_inputs[environment]
                .save(None, "wps_app_secret", "secret-one", true)
                .unwrap();
            host.oauth
                .secret
                .update(cx, |entry, cx| entry.set_value("{{wps_app_secret}}", cx));
            assert_eq!(host.oauth_secret(cx).unwrap(), "secret-one");
            let config = host.oauth_resolve(&host.oauth.snapshot(cx), cx).unwrap();
            assert_eq!(config.client_id, "app-one");
            let key = host.oauth_key(&config, cx);
            host.oauth.ready = Some((
                key.clone(),
                oauth::Access {
                    token: "runtime-access".into(),
                    expires_at: u64::MAX,
                },
            ));
            assert!(!host.oauth_before_send(cx));
            let draft = host.snapshot(cx);
            let request = PreparedRequest::new(
                &draft,
                &host.workspace.environment_inputs[environment].values(),
            )
            .unwrap();
            assert!(host.oauth_attach(request, &draft, cx).is_ok());
            assert_eq!(draft.oauth.as_ref().unwrap().client_id, "{{wps_app_id}}");
            // Manual callbacks compare the resolved snapshot, rather than the literal template.
            let session = Arc::new(oauth::Session::start(config).unwrap());
            host.oauth.pending = Some(session.clone());
            host.oauth.pending_key = Some(key.clone());
            host.oauth_exchange_manual(cx); // No callback entered yet: retains this session.
            assert!(host.oauth.pending.is_some());
            host.workspace.environment_inputs[environment]
                .save(Some(id), "wps_app_id", "app-two", false)
                .unwrap();
            let changed = host.oauth_resolve(&host.oauth.snapshot(cx), cx).unwrap();
            assert_ne!(host.oauth_key(&changed, cx), key);
            let request = PreparedRequest::new(
                &draft,
                &host.workspace.environment_inputs[environment].values(),
            )
            .unwrap();
            assert!(host.oauth_attach(request, &draft, cx).is_err());
            host.oauth_exchange_manual(cx);
            assert!(host.oauth.pending.is_none());
            assert!(session.cancelled.load(Ordering::Acquire));
            // Fail before opening a browser, starting a callback listener or vault task.
            host.oauth
                .secret
                .update(cx, |entry, cx| entry.set_value("{{missing_secret}}", cx));
            host.oauth_authorize(cx);
            assert!(host.oauth.error.as_ref().unwrap().contains("Client Secret"));
            assert!(host.oauth.pending.is_none() && host.oauth.task.is_none());
            host.workspace.environment_inputs[environment].remove(id);
            assert!(host.oauth_before_send(cx));
            assert!(host.oauth.error.as_ref().unwrap().contains("wps_app_id"));
            host.oauth_manage(false, cx);
            assert!(host.oauth.task.is_none());
            host.persist(cx);
            let saved: String = host
                .workspace
                .storage
                .connection
                .query_row("SELECT data FROM workspace", [], |row| row.get(0))
                .unwrap();
            assert!(saved.contains("{{wps_app_id}}"));
            assert!(!saved.contains("runtime-access"));
            assert!(saved.contains("{{missing_secret}}"));
        });
    }

    #[gpui::test]
    fn oauth_native_configuration_persists_secret_and_preserves_legacy_auth(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("oauth.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.workspace
                .collections
                .push(RequestGroups::create_collection("OAuth", &[]).unwrap());
            host.add_request(cx);
            host.config = 1;
            host.authentication.update(cx, |field, cx| {
                field.set_selected(Some("oauth2".into()), cx)
            });
            host.oauth.load(plugins::test_oauth_config(), cx);
            host
        });
        assert!(window.debug_bounds("oauth-preset-trigger").is_some());
        host.update(window, |host, cx| {
            host.oauth.fields[2].update(cx, |entry, cx| entry.set_value("test-app-id", cx));
            host.oauth.secret.update(cx, |entry, cx| {
                entry.set_value("transient-private-secret", cx)
            });
            host.persist(cx);
            let snapshot = host.snapshot(cx);
            assert!(!snapshot.auth);
            assert!(snapshot.oauth.is_some());
            let saved: String = host
                .workspace
                .storage
                .connection
                .query_row("SELECT data FROM workspace", [], |row| row.get(0))
                .unwrap();
            assert!(saved.contains("test-app-id"));
            assert!(saved.contains("transient-private-secret"));
            host.open_directory_settings(0, String::new(), cx);
            assert!(host.directory_settings.is_some());
            host.directory_settings = None;
            let session = Arc::new(
                oauth::Session::start(oauth::Config {
                    manual_callback: true,
                    ..snapshot.oauth.clone().unwrap()
                })
                .unwrap(),
            );
            let cancelled = session.cancelled.clone();
            host.oauth.pending = Some(session);
            host.add_request(cx);
            assert!(cancelled.load(Ordering::Acquire));
            let legacy = Draft {
                auth: true,
                token: "old-bearer".into(),
                ..Draft::default()
            };
            host.load(&legacy, cx);
            assert_eq!(host.snapshot(cx), legacy);
            assert!(host.oauth.secret.read(cx).value().is_empty());
            host.choose(0, cx);
            assert_eq!(host.snapshot(cx).oauth, snapshot.oauth);
        });
        let mut store = storage::Store::open(&path).unwrap();
        let data = store.load().unwrap();
        assert!(data.requests[0].draft.oauth.is_some());
    }
    #[gpui::test]
    fn oauth_compact_layout_keeps_actions_fixed_and_advanced_collapsed(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&directory.path().join("layout.sqlite3")).unwrap(),
                cx,
            );
            host.workspace
                .collections
                .push(RequestGroups::create_collection("OAuth", &[]).unwrap());
            host.add_request(cx);
            host.config = 1;
            host.authentication.update(cx, |field, cx| {
                field.set_selected(Some("oauth2".into()), cx)
            });
            host.oauth.load(plugins::test_oauth_config(), cx);
            host
        });
        assert!(window.debug_bounds("oauth-advanced-panel").is_none());
        let client = window.debug_bounds("oauth-client-id").unwrap();
        let secret = window.debug_bounds("oauth-client-secret").unwrap();
        assert_eq!(client.top(), secret.top());
        assert!(client.right() < secret.left());
        let form = window.debug_bounds("oauth-form-scroll").unwrap();
        let actions = window.debug_bounds("oauth-action-bar").unwrap();
        assert!(form.bottom() <= actions.top());
        let toggle = window.debug_bounds("oauth-advanced-toggle").unwrap();
        window.simulate_click(toggle.center(), gpui::Modifiers::default());
        assert!(window.debug_bounds("oauth-advanced-panel").is_some());
        assert_eq!(window.debug_bounds("oauth-action-bar").unwrap(), actions);
        host.update(window, |host, cx| {
            host.set_theme(ThemePreset::DarkObsidian, cx)
        });
        let dark_actions = window.debug_bounds("oauth-action-bar").unwrap();
        assert_eq!(dark_actions.size.height, actions.size.height);
        assert!(dark_actions.top() <= actions.top() + px(2.));
        host.update(window, |host, cx| {
            host.oauth.config.manual_callback = true;
            host.oauth.pending = Some(Arc::new(
                oauth::Session::start(oauth::Config {
                    client_id: "test-app".into(),
                    manual_callback: true,
                    ..plugins::test_oauth_config()
                })
                .unwrap(),
            ));
            cx.notify();
        });
        let callback = window.debug_bounds("oauth-manual-callback").unwrap();
        let actions = window.debug_bounds("oauth-action-bar").unwrap();
        assert!(callback.bottom() <= actions.top());
        assert!(window.debug_bounds("oauth-form-scroll").unwrap().bottom() <= callback.top());
    }
    #[test]
    fn oauth_request_preview_curl_and_plugin_patch_hide_the_actual_token() {
        let draft = Draft {
            url: "https://api.example.test/resource".into(),
            oauth: Some(plugins::test_oauth_config()),
            ..Draft::default()
        };
        let mut request = PreparedRequest::new(&draft, &EnvironmentVariables::default().values())
            .unwrap()
            .with_oauth_token("private-access-token")
            .unwrap();
        assert!(!request.preview.contains("private-access-token"));
        assert!(!request.to_curl().unwrap().contains("private-access-token"));
        assert_eq!(
            request
                .headers
                .iter()
                .find(|(name, _)| name == "authorization")
                .unwrap()
                .1,
            "Bearer private-access-token"
        );
        request
            .apply_plugin_headers(
                &serde_json::json!({"setHeaders":{"Authorization":"Bearer replacement-secret"}}),
                &std::collections::BTreeSet::from(["request.header.authorization".into()]),
            )
            .unwrap();
        assert!(!request.preview.contains("replacement-secret"));
        assert!(!request.to_curl().unwrap().contains("replacement-secret"));
        let mut inherited = Draft::default();
        model::DefaultSection::Auth.apply(&draft, &mut inherited);
        assert_eq!(inherited.oauth, draft.oauth);
        let old: Draft = serde_json::from_value(serde_json::json!({"method":"GET","url":"","body":"","params":[],"headers":[],"cookies":[],"token":"old","auth":true})).unwrap();
        assert!(old.oauth.is_none());
        assert!(old.auth);
    }
}

#[cfg(test)]
mod preset_tests {
    use super::*;
    #[gpui::test]
    fn oauth_plugin_menu_applies_configuration_and_removes_only_selected_provider(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&directory.path().join("workspace.sqlite3")).unwrap(),
                cx,
            );
            host.workspace
                .collections
                .push(RequestGroups::create_collection("OAuth", &[]).unwrap());
            host.add_request(cx);
            host.config = 1;
            host.authentication.update(cx, |field, cx| {
                field.set_selected(Some("oauth2".into()), cx)
            });
            host.oauth
                .secret
                .update(cx, |entry, cx| entry.set_value("transient-secret", cx));
            host
        });
        let trigger = window.debug_bounds("oauth-preset-trigger").unwrap();
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        let row = window
            .debug_bounds("oauth-preset-com.wps.oauth::user-authorization")
            .unwrap();
        window.simulate_click(row.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            let config = host.snapshot(cx).oauth.unwrap();
            assert_eq!(
                config.authorization_url,
                "https://openapi.wps.cn/oauth2/auth"
            );
            assert!(!config.pkce);
            assert!(config.client_id.is_empty());
            assert!(host.oauth.secret.read(cx).value().is_empty());
            assert!(!host.oauth.show_presets);
            host.plugin_catalog
                .retain(|p| p.manifest.id != "com.wps.oauth");
            assert_eq!(host.snapshot(cx).oauth.as_ref(), Some(&config));
            cx.notify();
        });
        let trigger = window.debug_bounds("oauth-preset-trigger").unwrap();
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        assert!(
            window
                .debug_bounds("oauth-preset-com.wps.oauth::user-authorization")
                .is_none()
        );
        let feishu = window
            .debug_bounds("oauth-preset-com.feishu.oauth::user-authorization")
            .unwrap();
        window.simulate_click(feishu.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            let config = host.snapshot(cx).oauth.unwrap();
            assert_eq!(
                config.authorization_url,
                "https://accounts.feishu.cn/open-apis/authen/v1/authorize"
            );
            assert_eq!(
                config.token_url,
                "https://accounts.feishu.cn/oauth/v3/token"
            );
            assert_eq!(config.scope, "offline_access");
            assert!(config.rolling_refresh_expiry);
            assert!(!config.pkce && !config.basic_auth);
            assert!(config.client_id.is_empty());
            let saved = serde_json::to_string(&config).unwrap();
            assert_eq!(
                serde_json::from_str::<oauth::Config>(&saved).unwrap(),
                config
            );
        });
    }
}
