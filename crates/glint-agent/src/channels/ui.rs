//! Native channel configuration and bridge into the existing partner request path.
use super::{Application, Platform, Repository, Worker};
use crate::*;
use gpui_glint::components::InputField;
use std::collections::HashMap;

pub struct State {
    repo: Repository,
    apps: Vec<Application>,
    selected: Option<Uuid>,
    editor_open: bool,
    name: Entity<TextEntry>,
    app_id: Entity<TextEntry>,
    secret: Entity<TextEntry>,
    project: Entity<TextEntry>,
    partner: Option<Uuid>,
    pub notice: Option<String>,
    pub credential_source: Option<crate::hosting::WpsCredentialSource>,
    workers: HashMap<Uuid, Worker>,
    status: HashMap<Uuid, String>,
    sender: async_channel::Sender<(Uuid, String)>,
    receiver: async_channel::Receiver<(Uuid, String)>,
    task: Option<Task<()>>,
    pub running: Option<(i64, Uuid, Uuid)>,
}
impl State {
    pub fn new(path: &std::path::Path, cx: &mut Context<AgentWorkspace>) -> Self {
        let repo = Repository::new(path);
        let loaded = repo.recover().and_then(|_| repo.applications());
        let notice = loaded.as_ref().err().cloned();
        let apps = loaded.unwrap_or_default();
        let name = cx.new(|cx| TextEntry::new(cx, "例如：团队助理", ""));
        let app_id = cx.new(|cx| TextEntry::new(cx, "WPS 开放平台 App ID（AK）", ""));
        let secret =
            cx.new(|cx| TextEntry::new(cx, "App Secret（SK）；编辑时留空保留", "").masked(true));
        let project = cx.new(|cx| TextEntry::new(cx, "伙伴工作的本地项目目录", ""));
        for field in [&name, &app_id, &secret, &project] {
            cx.observe(field, |_, _, cx| cx.notify()).detach();
        }
        let (sender, receiver) = async_channel::unbounded();
        Self {
            repo,
            apps,
            selected: None,
            editor_open: false,
            name,
            app_id,
            secret,
            project,
            partner: None,
            notice,
            credential_source: None,
            workers: HashMap::new(),
            status: HashMap::new(),
            sender,
            receiver,
            task: None,
            running: None,
        }
    }
    pub fn set_theme(&self, theme: ThemePreset, cx: &mut Context<AgentWorkspace>) {
        for field in [&self.name, &self.app_id, &self.secret, &self.project] {
            field.update(cx, |f, cx| f.set_theme(theme, cx));
        }
    }
    fn sync(&mut self) {
        self.workers
            .retain(|id, _| self.apps.iter().any(|a| a.id == *id && a.enabled));
        for app in self.apps.iter().filter(|a| a.enabled) {
            if self.workers.contains_key(&app.id) {
                continue;
            }
            match super::start(app.clone(), self.repo.clone(), self.sender.clone()) {
                Ok(worker) => {
                    self.workers.insert(app.id, worker);
                }
                Err(error) => {
                    self.status.insert(app.id, error);
                }
            }
        }
    }
}
impl AgentWorkspace {
    pub(crate) fn start_channels(&mut self, cx: &mut Context<Self>) {
        self.channels.sync();
        self.channels.task = Some(cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(500))
                    .await;
                if weak.update(cx, |this, cx| this.poll_channels(cx)).is_err() {
                    break;
                }
            }
        }));
    }
    fn poll_channels(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        while let Ok((id, status)) = self.channels.receiver.try_recv() {
            self.channels.status.insert(id, status);
            changed = true;
        }
        for (id, worker) in &self.channels.workers {
            if worker.0.is_finished()
                && self
                    .channels
                    .status
                    .get(id)
                    .is_none_or(|s| s == "正在连接…")
            {
                self.channels
                    .status
                    .insert(*id, "频道连接任务已退出，请停用后重新启用".into());
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
        if !self.channels.apps.iter().any(|a| a.enabled)
            || self.pending.is_some()
            || self.preparing.is_some()
            || !self.queued_requests.is_empty()
            || !self.provider.enabled
            || !self.storage_readable
        {
            return;
        }
        let job = match self.channels.repo.next() {
            Ok(Some(job)) => job,
            Ok(None) => return,
            Err(e) => {
                self.channels.notice = Some(e);
                return;
            }
        };
        let partners = match partners::Repository::new(self.path.clone()).list() {
            Ok(p) => p,
            Err(e) => {
                let _ = self.channels.repo.finish(job.row, Err(&e));
                return;
            }
        };
        if !partners.iter().any(|p| p.id == job.app.partner) || !job.app.project.is_dir() {
            let _ = self
                .channels
                .repo
                .finish(job.row, Err("绑定的伙伴或项目已不可用，请检查频道配置"));
            return;
        }
        if !self.history.sessions.iter().any(|s| s.id == job.session) {
            let mut session = model::Session::new();
            session.id = job.session;
            session.partner_id = Some(job.app.partner);
            session.project = Some(job.app.project.clone());
            session.title = format!("WPS · {}", job.app.name);
            self.history.sessions.insert(0, session);
        }
        self.channels.running = Some((job.row, job.session, job.app.id));
        self.send_to(
            job.session,
            ComposerSubmission::text(job.text),
            self.provider.clone(),
            false,
            cx,
        );
        if self.preparing.is_none() && self.pending.is_none() {
            self.finish_channel(job.session, None, Some("无法启动伙伴请求"));
        }
    }
    pub(crate) fn finish_channel(
        &mut self,
        session: Uuid,
        reply: Option<&str>,
        error: Option<&str>,
    ) {
        if let Some((row, id, _)) = self.channels.running {
            if id == session {
                let result = match reply.filter(|s| !s.trim().is_empty()) {
                    Some(reply) if error.is_none() => Ok(reply),
                    _ => Err(error.unwrap_or("伙伴未返回回复")),
                };
                if let Err(e) = self.channels.repo.finish(row, result) {
                    self.channels.notice = Some(e);
                }
                self.channels.running = None;
            }
        }
    }
    pub(crate) fn open_channels(&mut self, cx: &mut Context<Self>) {
        self.page = Page::Channels;
        match self.channels.repo.applications() {
            Ok(apps) => self.channels.apps = apps,
            Err(e) => self.channels.notice = Some(e),
        }
        cx.notify();
    }
    fn edit_channel(&mut self, app: Option<Application>, cx: &mut Context<Self>) {
        self.channels.editor_open = true;
        self.channels.selected = app.as_ref().map(|a| a.id);
        self.channels.partner = app.as_ref().map(|a| a.partner);
        let project = app
            .as_ref()
            .map(|a| a.project.clone())
            .or_else(|| self.history.active().project.clone());
        self.channels.name.update(cx, |f, cx| {
            f.set_value(app.as_ref().map(|a| a.name.as_str()).unwrap_or(""), cx)
        });
        self.channels.app_id.update(cx, |f, cx| {
            f.set_value(app.as_ref().map(|a| a.app_id.as_str()).unwrap_or(""), cx)
        });
        self.channels.secret.update(cx, |f, cx| f.set_value("", cx));
        self.channels.project.update(cx, |f, cx| {
            f.set_value(
                project.map(|p| p.display().to_string()).unwrap_or_default(),
                cx,
            )
        });
        self.channels.notice = None;
        cx.notify();
    }
    fn save_channel(&mut self, cx: &mut Context<Self>) {
        let result = (|| {
            let partner = self.channels.partner.ok_or("请选择绑定的伙伴")?;
            if !partners::Repository::new(self.path.clone())
                .list()?
                .iter()
                .any(|p| p.id == partner)
            {
                return Err("绑定的伙伴已不存在".into());
            }
            let existing = self
                .channels
                .apps
                .iter()
                .find(|a| Some(a.id) == self.channels.selected)
                .cloned();
            if self
                .channels
                .running
                .is_some_and(|(_, _, id)| Some(id) == self.channels.selected)
            {
                return Err("这个应用的伙伴正在回复，请完成后修改绑定".into());
            }
            let app = Application {
                id: existing.as_ref().map(|a| a.id).unwrap_or_else(Uuid::new_v4),
                platform: Platform::Wps,
                name: self.channels.name.read(cx).value().trim().into(),
                app_id: self.channels.app_id.read(cx).value().trim().into(),
                project: std::fs::canonicalize(self.channels.project.read(cx).value().trim())
                    .map_err(|_| "项目目录不存在")?,
                partner,
                enabled: false,
            };
            if existing.as_ref().is_some_and(|a| a.app_id != app.app_id) {
                return Err("已有应用的 App ID 不可修改，请新增应用".into());
            }
            let secret = self.channels.secret.read(cx).value().trim().to_owned();
            if secret.is_empty() {
                self.channels.repo.secret(app.id)?;
            }
            self.channels
                .repo
                .save_with_secret(&app, (!secret.is_empty()).then_some(secret.as_str()))?;
            self.channels.workers.remove(&app.id);
            self.channels.repo.interrupt_sends(app.id)?;
            self.channels.apps = self.channels.repo.applications()?;
            self.channels.selected = Some(app.id);
            self.channels.secret.update(cx, |f, cx| f.set_value("", cx));
            Ok::<_, String>(())
        })();
        self.channels.notice = Some(match result {
            Ok(()) => {
                self.channels.editor_open = false;
                "应用已保存，启用后开始接收消息。".into()
            }
            Err(e) => e,
        });
        cx.notify();
    }
    fn toggle_channel(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let result = (|| {
            let mut app = self
                .channels
                .apps
                .iter()
                .find(|a| a.id == id)
                .cloned()
                .ok_or("应用不存在")?;
            app.enabled = !app.enabled;
            if app.enabled {
                self.channels.repo.secret(app.id)?;
                if !self.provider.enabled {
                    return Err("请先连接模型 Provider".into());
                }
                if !partners::Repository::new(self.path.clone())
                    .list()?
                    .iter()
                    .any(|p| p.id == app.partner)
                {
                    return Err("绑定的伙伴已不存在".into());
                }
            }
            self.channels.repo.save(&app)?;
            self.channels.apps = self.channels.repo.applications()?;
            // Restart terminated workers explicitly, e.g. after connection_replaced.
            self.channels.workers.remove(&id);
            self.channels.repo.interrupt_sends(id)?;
            self.channels.status.remove(&id);
            self.channels.sync();
            Ok::<_, String>(())
        })();
        self.channels.notice = result.err();
        cx.notify();
    }
    fn choose_channel_directory(&mut self, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择伙伴的工作目录".into()),
        });
        cx.spawn(async move |weak, cx| {
            let result = picker.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            this.channels
                                .project
                                .update(cx, |f, cx| f.set_value(path.display().to_string(), cx));
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => this.channels.notice = Some("无法选择项目目录".into()),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn channels_page(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let theme = self.theme;
        div()
            .id("channels-page")
            .debug_selector(|| "channels-page".into())
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .w_full()
                    .max_w(px(1120.))
                    .mx_auto()
                    .p_8()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_4()
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(px(26.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("频道"),
                                    )
                                    .child(
                                        div()
                                            .text_color(rgb(p.text_muted))
                                            .child("让伙伴在你常用的地方，接住每一次对话。"),
                                    ),
                            )
                            .child(
                                Button::new("添加应用")
                                    .icon(Icon::Plus)
                                    .theme(theme)
                                    .render()
                                    .id("channel-add")
                                    .debug_selector(|| "channel-add".into())
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.edit_channel(None, cx)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .border_b_1()
                            .border_color(rgb(p.border))
                            .pb_3()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .px_3()
                                    .py_2()
                                    .rounded_lg()
                                    .bg(rgb(p.accent_tint))
                                    .text_color(rgb(p.accent))
                                    .font_weight(FontWeight::MEDIUM)
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(Icon::Radio.render(16.))
                                    .child("WPS 协作")
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .child(self.channels.apps.len().to_string()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_color(rgb(p.text_muted))
                                    .text_size(px(12.))
                                    .child("应用机器人"),
                            ),
                    )
                    .when_some(self.channels.notice.clone(), |d, n| {
                        d.child(
                            div()
                                .px_4()
                                .py_3()
                                .rounded_lg()
                                .bg(rgb(p.panel))
                                .text_color(rgb(p.text_muted))
                                .child(n),
                        )
                    })
                    .child(if self.channels.editor_open {
                        self.channel_editor(cx)
                    } else {
                        self.channel_applications(cx)
                    }),
            )
    }
    fn channel_applications(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let theme = self.theme;
        let partners = partners::Repository::new(self.path.clone())
            .list()
            .unwrap_or_default();
        div().flex().flex_col().gap_4()
            .when(self.channels.apps.is_empty(),|d|d.child(div().py_12().px_6().rounded_xl().border_1().border_color(rgb(p.border)).bg(rgb(p.surface)).flex().flex_col().items_center().gap_4()
                .child(div().size(px(60.)).rounded(px(18.)).bg(rgb(p.accent_tint)).text_color(rgb(p.accent)).flex().items_center().justify_center().child(Icon::Radio.render(28.)))
                .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child("把第一个应用交给伙伴"))
                .child(div().max_w(px(430.)).text_center().text_color(rgb(p.text_muted)).line_height(px(22.)).child("绑定 WPS 应用后，你可以直接和机器人对话，也能收到伙伴的任务进展与定时通知。"))
                .child(div().flex().items_center().gap_3().mt_2().text_size(px(12.)).text_color(rgb(p.text_muted)).child("WPS 应用").child(Icon::ChevronRight.render(12.)).child("你的伙伴").child(Icon::ChevronRight.render(12.)).child("与你对话"))
                .child(Button::new("连接 WPS 应用").kind(ButtonKind::Tinted).theme(theme).render().on_click(cx.listener(|this,_,_,cx|this.edit_channel(None,cx))))))
            .children(self.channels.apps.iter().map(|app|{
                let id=app.id;let edit=app.clone();
                let partner=partners.iter().find(|p|p.id==app.partner).map(|p|p.name.clone()).unwrap_or_else(||"伙伴已删除".into());
                let owner=self.channels.repo.owner(id).ok().flatten();
                let state=if app.enabled{self.channels.status.get(&id).cloned().unwrap_or_else(||"正在启动…".into())}else{"已停用".into()};
                div().p_5().rounded_xl().border_1().border_color(rgb(p.border)).bg(rgb(p.surface)).flex().flex_col().gap_4()
                    .child(div().flex().items_center().gap_3()
                        .child(div().size(px(40.)).rounded_lg().bg(rgb(p.accent_tint)).text_color(rgb(p.accent)).flex().items_center().justify_center().child(Icon::Radio.render(20.)))
                        .child(div().flex_1().flex().flex_col().gap_1().child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(app.name.clone())).child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child(app.app_id.clone())))
                        .child(div().rounded_full().px_3().py_1().bg(rgb(if state.starts_with("已连接"){p.green_tint}else{p.panel})).text_size(px(11.)).child(state))
                        .child(Button::new("配置").kind(ButtonKind::Plain).theme(theme).render().id(SharedString::from(format!("channel-edit-{id}"))).on_click(cx.listener(move|this,_,_,cx|this.edit_channel(Some(edit.clone()),cx))))
                        .child(Button::new(if app.enabled{"停用"}else{"启用"}).kind(if app.enabled{ButtonKind::Gray}else{ButtonKind::Tinted}).theme(theme).render().id(SharedString::from(format!("channel-toggle-{id}"))).on_click(cx.listener(move|this,_,_,cx|this.toggle_channel(id,cx)))))
                    .child(div().flex().flex_wrap().gap_6().text_size(px(12.))
                        .child(div().flex().items_center().gap_2().text_color(rgb(p.text_muted)).child(Icon::User.render(14.)).child(partner))
                        .child(div().flex().items_center().gap_2().text_color(rgb(p.text_muted)).child(Icon::Shield.render(14.)).child(owner.as_ref().map(|(_,sender,_)|format!("主人 · {sender}")).unwrap_or_else(||"等待主人认领".into())))
                        .child(div().text_color(rgb(p.text_muted)).child(self.channels.repo.summary(id).unwrap_or_else(|e|e))))
                    .when(owner.is_none(),|d|d.child(div().px_3().py_2().rounded_lg().bg(rgb(p.panel)).text_color(rgb(p.text_muted)).text_size(px(12.)).child("首次使用：在机器人单聊中发送「我是你的主人」，完成认领。")))
            }))
    }
    fn channel_editor(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let theme = self.theme;
        let partners = partners::Repository::new(self.path.clone())
            .list()
            .unwrap_or_default();
        let field = |label: &str, entry: &Entity<TextEntry>| {
            InputField::new(label.to_owned())
                .label(label.to_owned())
                .control(entry.clone())
                .theme(theme)
                .render()
                .w_full()
        };
        let section = |title: &str, description: &str| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(14.))
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child(description.to_owned()),
                )
        };
        div().flex().flex_wrap().items_start().gap_6()
            .child(div().debug_selector(||"channel-editor-form".into()).flex_1().min_w(px(360.)).p_6().rounded_xl().bg(rgb(p.surface)).border_1().border_color(rgb(p.border)).flex().flex_col().gap_5()
                .child(div().flex().items_center().child(div().flex_1().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(if self.channels.selected.is_some(){"配置应用"}else{"连接 WPS 应用"}))
                    .child(Button::new("取消").kind(ButtonKind::Plain).size(ButtonSize::Small).theme(theme).render().on_click(cx.listener(|this,_,_,cx|{this.channels.editor_open=false;this.channels.secret.update(cx,|e,cx|e.set_value("",cx));cx.notify();}))))
                .child(field("应用名称",&self.channels.name))
                .child(div().border_t_1().border_color(rgb(p.border)).pt_5().flex().items_center().gap_3()
                    .child(section("应用凭证","来自 WPS 开放平台的 App ID 与密钥。"))
                    .child(div().flex_1())
                    .when(self.channels.credential_source.is_some(),|d|d.child(Button::new("从 API 导入").icon(Icon::Download).kind(ButtonKind::Plain).size(ButtonSize::Small).theme(theme).render().id("channel-import-credentials").on_click(cx.listener(|this,_,_,cx|{
                        match this.channels.credential_source.as_ref().unwrap()(cx){Ok((id,key))=>{this.channels.app_id.update(cx,|f,cx|f.set_value(id,cx));this.channels.secret.update(cx,|f,cx|f.set_value(key,cx));this.channels.notice=Some("已填入当前 API 环境的应用凭证。".into());},Err(e)=>this.channels.notice=Some(e)}cx.notify();
                    })))))
                .child(field("App ID",&self.channels.app_id)).child(field("App Secret",&self.channels.secret))
                .child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).flex().items_center().gap_2().child(Icon::Key.render(12.)).child("密钥保存在本地 SQLite 中；编辑时留空即可保留。"))
                .child(div().border_t_1().border_color(rgb(p.border)).pt_5().child(section("交给哪位伙伴","这个应用的对话与通知，都由这位伙伴接手。")))
                .child(div().flex().flex_wrap().gap_2().children(partners.iter().map(|partner|{
                    let id=partner.id;let selected=self.channels.partner==Some(id);
                    Button::new(partner.name.clone()).icon(Icon::User).kind(if selected{ButtonKind::Tinted}else{ButtonKind::Gray}).theme(theme).render().id(SharedString::from(format!("channel-partner-{id}")))
                        .on_click(cx.listener(move|this,_,_,cx|{this.channels.partner=Some(id);cx.notify();}))
                })))
                .when(partners.is_empty(),|d|d.child(div().text_color(rgb(p.text_muted)).child("先到“伙伴”页面创建一位伙伴，再回来绑定。")))
                .child(div().flex().flex_col().gap_1()
                    .child(div().pl(px(2.)).text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(rgb(theme.color(0x555b59))).child("工作目录"))
                    .child(div().flex().items_center().gap_2()
                        .child(div().flex_1().min_w_0().child(InputField::new("伙伴工作的本地项目目录").control(self.channels.project.clone()).theme(theme).render().w_full().debug_selector(||"channel-directory-input".into())))
                        .child(Button::new("选择目录").icon(Icon::Folder).kind(ButtonKind::Gray).theme(theme).render().h(px(32.)).flex_shrink_0().debug_selector(||"channel-directory-button".into()).on_click(cx.listener(|this,_,_,cx|this.choose_channel_directory(cx))))))
                .child(div().border_t_1().border_color(rgb(p.border)).pt_4().flex().items_center().justify_end().gap_3()
                    .child(Button::new("保存应用").theme(theme).render().id("channel-save").debug_selector(||"channel-save".into()).on_click(cx.listener(|this,_,_,cx|this.save_channel(cx))))))
            .child(div().debug_selector(||"channel-editor-guide".into()).w(px(244.)).flex_shrink_0().flex().flex_col().gap_5().pt_2()
                .child(section("连接之后","让应用、伙伴和你，保持同一条对话。"))
                .child(self.channel_setup_step(Icon::Radio,"开启机器人","在 WPS 开放平台开启应用机器人、消息权限和消息事件。"))
                .child(self.channel_setup_step(Icon::Shield,"认领主人","启用应用后，在机器人单聊中发送："))
                .child(div().ml_6().px_3().py_2().rounded_lg().bg(rgb(p.accent_tint)).text_color(rgb(p.accent)).text_size(px(12.)).child("我是你的主人"))
                .child(self.channel_setup_step(Icon::Calendar,"随时通知你","伙伴可以主动发消息，定时任务也能把进展通知给主人。"))
                .child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).line_height(px(19.)).child("仅响应已认领主人的消息。应用运行时保持连接，退出后暂停收发。"))
                .child(Button::new("查看接入文档").icon(Icon::External).kind(ButtonKind::Plain).size(ButtonSize::Small).theme(theme).render().on_click(cx.listener(|_,_,_,cx|cx.open_url("https://365.kdocs.cn/3rd/open/documents/app-integration-dev/wps365/server/event-subscription/event-websocket"))))
                .when_some(self.channels.selected,|d,id|d.child(div().border_t_1().border_color(rgb(p.border)).pt_4().flex().flex_col().items_start().gap_2()
                    .child(Button::new("重试失败发送").icon(Icon::Refresh).kind(ButtonKind::Plain).size(ButtonSize::Small).theme(theme).render().on_click(cx.listener(move|this,_,_,cx|{this.channels.notice=Some(match this.channels.repo.retry_sends(id){Ok(())=>"已重新排队；此前送达状态不明的消息可能重复。".into(),Err(e)=>e});cx.notify();})))
                    .child(Button::new("解除主人绑定").kind(ButtonKind::Plain).size(ButtonSize::Small).theme(theme).render().on_click(cx.listener(move|this,_,_,cx|{this.channels.notice=Some(match this.channels.repo.clear_owner(id){Ok(())=>"已解除主人绑定，可在机器人单聊重新认领。".into(),Err(e)=>e});cx.notify();})))
                    .child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child("解除绑定前，请先停用应用。")))))
    }
    fn channel_setup_step(&self, icon: Icon, title: &str, text: &str) -> gpui::Div {
        let p = self.theme.palette();
        div()
            .flex()
            .gap_3()
            .child(
                div()
                    .pt_1()
                    .text_color(rgb(p.text_muted))
                    .child(icon.render(16.)),
            )
            .child(
                div()
                    .flex_1()
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
                            .line_height(px(20.))
                            .child(text.to_owned()),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn native_channel_page_keeps_binding_editor_and_owner_controls(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(dir.path().join("sessions.json"), cx);
            let partner = partners::Repository::new(dir.path().join("sessions.json"))
                .create()
                .unwrap();
            let app = Application {
                id: Uuid::new_v4(),
                platform: Platform::Wps,
                name: "WPS 测试应用".into(),
                app_id: "test-ak".into(),
                partner,
                project: dir.path().canonicalize().unwrap(),
                enabled: false,
            };
            host.channels.repo.save(&app).unwrap();
            host.open_channels(cx);
            host.edit_channel(Some(app), cx);
            host
        });
        window.simulate_resize(size(px(1440.), px(1000.)));
        window.run_until_parked();
        assert!(window.debug_bounds("channels-page").is_some());
        assert!(window.debug_bounds("channel-add").is_some());
        let form = window.debug_bounds("channel-editor-form").unwrap();
        let guide = window.debug_bounds("channel-editor-guide").unwrap();
        assert!(
            form.right() < guide.left(),
            "setup guide should sit beside the bounded form"
        );
        assert!(
            window.debug_bounds("channel-save").unwrap().size.width < px(180.),
            "save action should keep a natural button width"
        );
        let input = window.debug_bounds("channel-directory-input").unwrap();
        let button = window.debug_bounds("channel-directory-button").unwrap();
        assert_eq!(
            input.top(),
            button.top(),
            "directory controls should share a top edge"
        );
        assert_eq!(
            input.size.height, button.size.height,
            "directory controls should share a height"
        );
        host.read_with(window, |host, cx| {
            assert_eq!(host.channels.app_id.read(cx).value(), "test-ak");
            assert_eq!(host.channels.secret.read(cx).value(), "");
            assert!(host.channels.partner.is_some());
            assert_eq!(host.channels.apps.len(), 1);
        });
    }
}
