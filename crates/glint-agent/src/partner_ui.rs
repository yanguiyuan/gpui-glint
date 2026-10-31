//! Native companion management, shared by Workbench and standalone Agent.
use super::*;

mod avatar;
mod deletion;
mod memories;
mod overview;
mod settings;
mod transfer;
mod wake;
use crate::partners;
use gpui::ColorExt;
use gpui_glint::components::{Dialog, InputField, PromptEditor, Switch};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Overview,
    Memories,
    Wake,
    Settings,
}
impl Tab {
    fn label(self) -> &'static str {
        match self {
            Self::Overview => "近况",
            Self::Memories => "记忆",
            Self::Wake => "唤醒计划",
            Self::Settings => "设定",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Memories => "memories",
            Self::Wake => "wake",
            Self::Settings => "settings",
        }
    }
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum MemoryFilter {
    #[default]
    All,
    Active,
    Dormant,
    Pinned,
}
impl MemoryFilter {
    fn matches(self, e: &partners::Entry) -> bool {
        match self {
            Self::All => true,
            Self::Active => e.activation(chrono::Utc::now().timestamp()) >= 0.2,
            Self::Dormant => e.activation(chrono::Utc::now().timestamp()) < 0.2,
            Self::Pinned => e.pinned,
        }
    }
}
pub struct State {
    pub selected: Option<Uuid>,
    name: Entity<TextEntry>,
    avatar: Entity<TextEntry>,
    description: Entity<PromptEditor>,
    partner_search: Entity<TextEntry>,
    tab: Tab,
    filter: MemoryFilter,
    scroll: ScrollHandle,
    search: Entity<TextEntry>,
    pub notice: Option<String>,
    confirming_delete: Option<Uuid>,
    confirming_partner: Option<Uuid>,
    cancel_focus: gpui::FocusHandle,
    confirm_focus: gpui::FocusHandle,
    avatar_task: Option<Task<()>>,
    transfer_task: Option<Task<()>>,
    expanded: Option<Uuid>,
}
impl State {
    pub fn set_theme(&self, theme: ThemePreset, cx: &mut Context<AgentWorkspace>) {
        for entry in [&self.name, &self.avatar, &self.search, &self.partner_search] {
            entry.update(cx, |e, cx| e.set_theme(theme, cx));
        }
        self.description.update(cx, |e, cx| e.set_theme(theme, cx));
    }
    pub fn new(cx: &mut Context<AgentWorkspace>) -> Self {
        let search = cx.new(|cx| TextEntry::new(cx, "搜索历史，包括沉睡记忆…", ""));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        let partner_search = cx.new(|cx| TextEntry::new(cx, "查找伙伴…", ""));
        cx.observe(&partner_search, |_, _, cx| cx.notify()).detach();
        let name = cx.new(|cx| TextEntry::new(cx, "名字可以以后再起", ""));
        let avatar = cx.new(|cx| TextEntry::new(cx, "头像符号，例如 ◌", "◌"));
        let description =
            cx.new(|cx| PromptEditor::new(cx, "可以留空，在合作中逐渐形成习惯").min_rows(4));
        for editor in [&name, &avatar] {
            cx.observe(editor, |_, _, cx| cx.notify()).detach();
        }
        cx.observe(&description, |_, _, cx| cx.notify()).detach();
        Self {
            partner_search,
            tab: Tab::Overview,
            filter: MemoryFilter::All,
            scroll: ScrollHandle::new(),
            selected: None,
            name,
            avatar,
            description,
            search,
            notice: None,
            confirming_delete: None,
            confirming_partner: None,
            cancel_focus: cx.focus_handle(),
            confirm_focus: cx.focus_handle(),
            avatar_task: None,
            transfer_task: None,
            expanded: None,
        }
    }
}
impl AgentWorkspace {
    pub(crate) fn open_partners(&mut self, cx: &mut Context<Self>) {
        self.page = crate::Page::Partners;
        let target = self
            .partners_ui
            .selected
            .or(self.history.active().partner_id)
            .or(self.history.preferred_partner);
        match self.partner_repo().list() {
            Ok(ps) => {
                if let Some(p) = target
                    .and_then(|id| ps.iter().find(|p| p.id == id))
                    .or(ps.first())
                {
                    self.select_partner(p, cx);
                }
            }
            Err(error) => self.partners_ui.notice = Some(error),
        }
        cx.notify();
    }
    fn partner_repo(&self) -> partners::Repository {
        partners::Repository::new(self.path.clone())
    }
    fn select_partner(&mut self, p: &partners::Partner, cx: &mut Context<Self>) {
        self.partners_ui.selected = Some(p.id);
        self.partners_ui.confirming_delete = None;
        self.partners_ui.confirming_partner = None;
        self.partners_ui.tab = Tab::Overview;
        self.partners_ui.filter = MemoryFilter::All;
        self.partners_ui.expanded = None;
        self.partners_ui.scroll = ScrollHandle::new();
        self.partners_ui
            .name
            .update(cx, |e, cx| e.set_value(p.name.clone(), cx));
        self.partners_ui
            .avatar
            .update(cx, |e, cx| e.set_value(p.avatar.clone(), cx));
        self.partners_ui
            .description
            .update(cx, |e, cx| e.set_value(p.description.clone(), cx));
        self.partners_ui
            .search
            .update(cx, |e, cx| e.set_value("", cx));
        cx.notify();
    }
    pub(crate) fn start_partner_worker(&mut self, cx: &mut Context<Self>) {
        self.partner_worker = Some(cx.spawn(async move |weak, cx| {
            loop {
                let Ok((repo, config, readable)) = weak.update(cx, |this, _| {
                    (
                        this.partner_repo(),
                        this.provider.clone(),
                        this.storage_readable,
                    )
                }) else {
                    break;
                };
                if readable && config.enabled {
                    let result =
                        match provider::Job::start(async move { repo.process_one(config).await }) {
                            Ok(job) => job.result().await,
                            Err(error) => Err(error),
                        };
                    let did_work = matches!(result, Ok(true));
                    let _ = weak.update(cx, |this, cx| {
                        if let Err(error) = result {
                            this.partners_ui.notice =
                                Some(format!("经历整理暂未完成，将自动重试：{error}"));
                        } else if did_work {
                            this.partners_ui.notice = None;
                        }
                        if this.page == crate::Page::Partners {
                            cx.notify();
                        }
                    });
                    if did_work {
                        continue;
                    }
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(5))
                    .await;
            }
        }));
    }
    pub(crate) fn partner_switcher(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self
            .partner_repo()
            .list()
            .ok()
            .and_then(|ps| {
                ps.into_iter()
                    .find(|p| Some(p.id) == self.history.active().partner_id)
            })
            .map(|p| format!("{} {}", p.avatar, p.name))
            .unwrap_or_else(|| "选择伙伴".into());
        div()
            .id("chat-partner")
            .debug_selector(|| "chat-partner".into())
            .px_2()
            .py_1()
            .rounded(px(6.))
            .cursor_pointer()
            .text_color(rgb(self.theme.palette().accent))
            .child(name)
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_partners(cx);
            }))
    }
    fn partner_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.partners_ui.tab = tab;
        self.partners_ui.scroll = ScrollHandle::new();
        cx.notify();
    }
    fn create_partner(&mut self, cx: &mut Context<Self>) {
        match self.partner_repo().create().and_then(|id| {
            self.partner_repo()
                .list()?
                .into_iter()
                .find(|p| p.id == id)
                .ok_or("伙伴创建后无法读取".into())
        }) {
            Ok(p) => self.select_partner(&p, cx),
            Err(e) => self.partners_ui.notice = Some(e),
        }
        cx.notify();
    }
    fn chat_with_partner(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.capture_draft(cx);
        let mut session = model::Session::new();
        session.project = self.history.active().project.clone();
        session.partner_id = Some(id);
        self.history.preferred_partner = Some(id);
        self.history.active = session.id;
        self.history.sessions.insert(0, session);
        self.page = crate::Page::Chat;
        self.restore_draft(cx);
        self.save(cx);
        window.focus(&self.composer.focus_handle(cx), cx);
        cx.notify();
    }
    fn partner_chat_button(
        &self,
        id: Uuid,
        first: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        Button::new(if first {
            "开始第一次会话"
        } else {
            "开始会话"
        })
        .icon(if first {
            Icon::ChevronRight
        } else {
            Icon::Text
        })
        .size(ButtonSize::Regular)
        .theme(self.theme)
        .render()
        .id("chat-with-partner")
        .debug_selector(|| "chat-with-partner".into())
        .on_click(cx.listener(move |this, _, window, cx| this.chat_with_partner(id, window, cx)))
        .into_any_element()
    }
    pub(super) fn partner_avatar_model(
        &self,
        partner: &partners::Partner,
    ) -> gpui_glint::components::Avatar {
        use gpui_glint::components::{Avatar, AvatarShape};
        let mut avatar = Avatar::new(partner.name.clone()).shape(AvatarShape::Rectangle);
        if let Some(path) = self.partner_repo().avatar_path(&partner.avatar) {
            avatar = avatar.image(path);
        } else if !partners::is_avatar_reference(&partner.avatar)
            && !partner.avatar.trim().is_empty()
        {
            avatar = avatar.text(partner.avatar.clone());
        }
        avatar
    }

    pub(super) fn partner_avatar(&self, p: &partners::Partner, size: f32) -> gpui::Div {
        let palette = self.theme.palette();
        if let Some(path) = self.partner_repo().avatar_path(&p.avatar) {
            return div()
                .w(px(size))
                .h(px(size))
                .flex_shrink_0()
                .rounded(px(size * 0.3))
                .overflow_hidden()
                .border_1()
                .border_color(rgb(palette.border))
                .child(img(path).w_full().h_full().object_fit(ObjectFit::Cover));
        }
        let mut p = p.clone();
        if partners::is_avatar_reference(&p.avatar) {
            p.avatar = "◌".into();
        }
        div()
            .w(px(size))
            .h(px(size))
            .flex_shrink_0()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(size * 0.3))
            .bg(rgb(palette.accent_tint))
            .border_1()
            .border_color(rgb(self.theme.color(0xdfe8f8)))
            .text_color(rgb(palette.accent))
            .text_size(px(size * 0.38))
            .when(p.avatar == "◌" || p.avatar.trim().is_empty(), |d| {
                d.child(
                    div()
                        .absolute()
                        .w(px(size * 0.56))
                        .h(px(size * 0.56))
                        .rounded_full()
                        .border_1()
                        .border_color(rgb(self.theme.color(0xa4bde8))),
                )
                .child(
                    div()
                        .absolute()
                        .w(px(size * 0.28))
                        .h(px(size * 0.44))
                        .rounded_full()
                        .border_1()
                        .border_color(rgb(self.theme.color(0xa4bde8))),
                )
                .child(
                    div()
                        .w(px(4.))
                        .h(px(4.))
                        .rounded_full()
                        .bg(rgb(palette.accent)),
                )
            })
            .when(p.avatar != "◌" && !p.avatar.trim().is_empty(), |d| {
                d.child(p.avatar.clone())
            })
    }
    fn partner_badge(&self, text: impl Into<SharedString>, accent: bool) -> gpui::Div {
        let p = self.theme.palette();
        // The outer row can stretch; the colored label must keep its text width.
        div().flex().items_center().flex_shrink_0().child(
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .text_size(px(10.))
                .flex_shrink_0()
                .bg(rgb(if accent { p.accent_tint } else { p.panel }))
                .text_color(rgb(if accent { p.accent } else { p.text_muted }))
                .child(text.into()),
        )
    }
    fn partner_section(&self, label: &'static str) -> gpui::Div {
        div()
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(label)
    }
    fn partner_link(
        &self,
        label: &'static str,
        tab: Tab,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        div()
            .id(SharedString::from(format!(
                "partner-link-{}-{label}",
                tab.key()
            )))
            .self_start()
            .cursor_pointer()
            .text_size(px(11.))
            .text_color(rgb(self.theme.palette().text_muted))
            .hover(|s| s.text_color(rgb(self.theme.palette().accent)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.partner_tab(tab, cx)))
            .into_any_element()
    }
    fn partner_sidebar(
        &self,
        partners: &[partners::Partner],
        compact: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let p = self.theme.palette();
        let mut side = div()
            .w(px(if compact { 76. } else { 230. }))
            .h_full()
            .flex_shrink_0()
            .bg(rgb(p.panel))
            .border_r_1()
            .border_color(rgb(p.border))
            .flex()
            .flex_col()
            .py_5()
            .px_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .mb_4()
                    .when(compact, |d| d.flex_col())
                    .when(!compact, |d| {
                        d.child(self.partner_section("伙伴"))
                            .child(self.partner_badge(partners.len().to_string(), false))
                            .child(div().flex_1())
                    })
                    .child(
                        IconButton::new(Icon::Upload, "导入伙伴")
                            .theme(self.theme)
                            .render()
                            .id("import-partner")
                            .debug_selector(|| "import-partner".into())
                            .when(self.partners_ui.transfer_task.is_some(), |button| {
                                button.opacity(0.5)
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.import_partner_file(cx))),
                    )
                    .child(
                        IconButton::new(Icon::Plus, "创建空白伙伴")
                            .theme(self.theme)
                            .render()
                            .id("create-partner")
                            .debug_selector(|| "create-partner".into())
                            .on_click(cx.listener(|this, _, _, cx| this.create_partner(cx))),
                    ),
            );
        if !compact {
            side = side
                .child(
                    div().h(px(32.)).flex_shrink_0().px_2().mb_5().child(
                        InputField::new("查找伙伴")
                            .width(190.)
                            .control(self.partners_ui.partner_search.clone())
                            .theme(self.theme)
                            .render(),
                    ),
                )
                .child(
                    div()
                        .px_2()
                        .mb_2()
                        .text_size(px(10.))
                        .text_color(rgb(p.text_muted))
                        .child("一起工作的伙伴"),
                );
        }
        let query = self
            .partners_ui
            .partner_search
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let mut list = div()
            .id("partner-list")
            .debug_selector(|| "partner-list".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        for partner in partners
            .iter()
            .filter(|p| compact || p.name.to_lowercase().contains(&query))
        {
            let partner = partner.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("partner-{}", partner.id)))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_2()
                    .py_3()
                    .rounded(px(9.))
                    .cursor_pointer()
                    .when(self.partners_ui.selected == Some(partner.id), |d| {
                        d.bg(rgb(p.accent_tint))
                    })
                    .hover(|d| d.bg(rgb(p.accent_tint)))
                    .child(self.partner_avatar(&partner, 36.))
                    .when(!compact, |d| {
                        d.child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(partner.name.clone()),
                                )
                                .child(
                                    div()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .text_size(px(10.))
                                        .text_color(rgb(p.text_muted))
                                        .child(if partner.description.trim().is_empty() {
                                            "在相处中慢慢成长".into()
                                        } else {
                                            preview_line(&partner.description, 28)
                                        }),
                                ),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.select_partner(&partner, cx))),
            );
        }
        if partners.is_empty() && !compact {
            list = list.child(
                div()
                    .px_2()
                    .py_4()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("还没有伙伴，点击 + 开始。"),
            );
        }
        side.child(list).child(
            div()
                .mt_4()
                .pt_4()
                .border_t_1()
                .border_color(rgb(p.border))
                .child(
                    Button::new(if compact {
                        "＋"
                    } else {
                        "认识一位新伙伴"
                    })
                    .kind(ButtonKind::Plain)
                    .size(ButtonSize::Small)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| this.create_partner(cx))),
                )
                .when(!compact, |d| {
                    d.child(
                        div()
                            .pl_3()
                            .mt_1()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child("无需预设，慢慢长成。"),
                    )
                }),
        )
    }
    fn partner_header(
        &self,
        partner: &partners::Partner,
        entries: &[partners::Entry],
        tasks: &[schedules::Schedule],
        padding: f32,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let p = self.theme.palette();
        let id = partner.id;
        let blank = entries.is_empty();
        let days = ((chrono::Utc::now().timestamp() - partner.created_at).max(0) / 86400) + 1;
        let mut tabs = div()
            .flex()
            .items_center()
            .gap_6()
            .mt_6()
            .border_b_1()
            .border_color(rgb(p.border));
        for tab in [Tab::Overview, Tab::Memories, Tab::Wake, Tab::Settings] {
            let active = self.partners_ui.tab == tab;
            let count = match tab {
                Tab::Memories => Some(entries.len()),
                Tab::Wake => Some(tasks.iter().filter(|t| t.enabled || t.running).count()),
                _ => None,
            };
            let key = format!("partner-tab-{}", tab.key());
            tabs = tabs.child(
                div()
                    .id(SharedString::from(key.clone()))
                    .debug_selector(move || key.clone())
                    .pb_3()
                    .pt_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_b_2()
                    .border_color(if active {
                        rgb(p.accent)
                    } else {
                        rgb(p.border).opacity(0.)
                    })
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(if active { p.accent } else { p.text_muted }))
                    .child(tab.label())
                    .when_some(count, |d, n| {
                        d.child(div().text_size(px(10.)).child(n.to_string()))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.partner_tab(tab, cx))),
            );
        }
        div()
            .flex_shrink_0()
            .px(px(padding))
            .pt_7()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("COMPANION")
                    .child("/")
                    .child(if days == 1 {
                        "今天，第一次相遇".into()
                    } else {
                        format!(
                            "从 {} 开始，一起工作了 {days} 天",
                            local_date(partner.created_at, "%m 月 %d 日")
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .mt_5()
                    .child(self.partner_avatar(partner, 60.))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        div()
                                            .text_size(px(26.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(partner.name.clone()),
                                    )
                                    .child(self.partner_badge(
                                        if blank {
                                            "空白起步"
                                        } else {
                                            "在相处中成长"
                                        },
                                        false,
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(p.text_muted))
                                    .child(if !partner.description.trim().is_empty() {
                                        preview_line(&partner.description, 80)
                                    } else if blank {
                                        "名字、性格和擅长的事，都可以以后慢慢发现。".into()
                                    } else {
                                        "从真实的合作里积累经历，也在学习怎样与你配合。".into()
                                    }),
                            ),
                    )
                    .child(
                        IconButton::new(Icon::Download, "导出伙伴与头像")
                            .theme(self.theme)
                            .render()
                            .id("export-partner")
                            .debug_selector(|| "export-partner".into())
                            .when(self.partners_ui.transfer_task.is_some(), |button| {
                                button.opacity(0.5)
                            })
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.export_partner_file(id, cx)),
                            ),
                    )
                    .child(
                        IconButton::new(Icon::Settings, "编辑伙伴设定")
                            .theme(self.theme)
                            .render()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.partner_tab(Tab::Settings, cx)),
                            ),
                    )
                    .child(
                        IconButton::new(Icon::Trash, "删除伙伴")
                            .theme(self.theme)
                            .render()
                            .id("delete-partner")
                            .debug_selector(|| "delete-partner".into())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.request_partner_delete(id, window, cx)
                            })),
                    )
                    .when(!blank, |d| d.child(self.partner_chat_button(id, false, cx))),
            )
            .child(tabs)
    }
    pub(crate) fn partners_page(&self, window: &Window, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let repo = self.partner_repo();
        let list = repo.list();
        let mut error = list.as_ref().err().cloned();
        let partners = list.unwrap_or_default();
        let selected = self
            .partners_ui
            .selected
            .and_then(|id| partners.iter().find(|p| p.id == id));
        let width = f32::from(window.viewport_size().width);
        let compact = width < 740.;
        let padding = if width < 1080. { 24. } else { 36. };
        let mut workspace = div().flex_1().min_w_0().h_full().flex().flex_col();
        if let Some(partner) = selected {
            let result = repo.entries(partner.id, self.history.active().project.as_deref());
            if let Err(e) = &result {
                error = Some(e.clone());
            }
            let entries = result.unwrap_or_default();
            let mut tasks = self
                .schedules
                .lock()
                .map(|mut s| {
                    let _ = s.refresh();
                    s.tasks
                        .iter()
                        .filter(|t| t.partner_id == Some(partner.id))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            tasks.sort_by_key(|t| (!t.running, !t.enabled, t.run_at));
            workspace =
                workspace.child(self.partner_header(partner, &entries, &tasks, padding, cx));
            let mut body = div()
                .id("partner-body")
                .debug_selector(|| "partner-body".into())
                .track_scroll(&self.partners_ui.scroll)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(padding))
                .py_6();
            if let Some(e) = error {
                body = body.child(self.partner_notice(e));
            }
            if let Some(notice) = &self.partners_ui.notice {
                body = body.child(self.partner_notice(notice.clone()));
            }
            body = body.child(match self.partners_ui.tab {
                Tab::Overview => {
                    self.partner_overview(partner, &entries, &tasks, width >= 1120., cx)
                }
                Tab::Memories => self.partner_memories(partner, &entries, cx),
                Tab::Wake => self.partner_wakes(partner, &tasks, cx),
                Tab::Settings => self.partner_settings(partner, cx),
            });
            workspace = workspace.child(body).child(
                div()
                    .flex_shrink_0()
                    .px(px(padding))
                    .py_3()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child(Icon::Shield.render(12.))
                    .child(match repo.queue_status() {
                        Ok((n, _)) if n > 0 => format!("{n} 段经历待整理"),
                        _ => "记忆保存在本机".into(),
                    })
                    .child(div().flex_1())
                    .child("跨项目保留经历 · 项目资料按范围访问"),
            );
        } else {
            workspace = workspace
                .when_some(self.partners_ui.notice.as_ref(), |body, notice| {
                    body.child(
                        div()
                            .px(px(padding))
                            .pt_3()
                            .child(self.partner_notice(notice.clone())),
                    )
                })
                .child(
                    div()
                        .px(px(padding))
                        .py_7()
                        .child(self.partner_section("伙伴")),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .px(px(padding))
                        .gap_4()
                        .child(
                            div()
                                .text_size(px(28.))
                                .font_weight(FontWeight::MEDIUM)
                                .child("从一次合作开始"),
                        )
                        .child(
                            div()
                                .text_color(rgb(p.text_muted))
                                .child("创建一个空白伙伴，无需先填写名字、性格或背景。"),
                        )
                        .child(
                            div().child(
                                Button::new("创建空白伙伴")
                                    .icon(Icon::Plus)
                                    .theme(self.theme)
                                    .render()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.create_partner(cx)),
                                    ),
                            ),
                        )
                        .when_some(error, |d, e| d.child(self.partner_notice(e))),
                );
        }
        div()
            .relative()
            .size_full()
            .flex()
            .bg(rgb(p.surface))
            .text_color(rgb(p.text))
            .child(self.partner_sidebar(&partners, compact, cx))
            .child(workspace)
            .when_some(self.partners_ui.confirming_partner, |root, id| {
                root.child(self.partner_delete_dialog(id, cx))
            })
    }
    fn partner_notice(&self, text: String) -> gpui::Div {
        div()
            .mb_4()
            .px_3()
            .py_2()
            .rounded(px(7.))
            .bg(rgb(self.theme.palette().panel))
            .text_color(rgb(self.theme.palette().text_muted))
            .text_size(px(11.))
            .child(text)
    }
}

fn local_date(timestamp: i64, format: &str) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|d| d.with_timezone(&chrono::Local).format(format).to_string())
        .unwrap_or_else(|| "未知时间".into())
}
fn is_active_goal(kind: &str) -> bool {
    matches!(kind, "goal" | "goal_running" | "goal_review")
}
fn entry_status(e: &partners::Entry) -> &'static str {
    match e.kind.as_str() {
        "goal" => "待推进目标",
        "goal_running" => "已安排推进",
        "goal_review" => "结果待核对",
        "goal_done" => "已结束目标",
        "goal_failed" => "推进中断",
        "goal_paused" => "已暂停目标",
        "commitment" => "未完成承诺",
        "identity" => "名字提议",
        _ if e.pinned => "固定保留",
        _ if e.activation(chrono::Utc::now().timestamp()) < 0.2 => "沉睡",
        _ => "活跃",
    }
}
fn entry_icon(kind: &str) -> Icon {
    match kind {
        "preference" => Icon::Star,
        "commitment" => Icon::Clock,
        "identity" => Icon::User,
        k if k.starts_with("goal") => Icon::Compass,
        _ => Icon::Text,
    }
}
fn entry_scope(e: &partners::Entry) -> String {
    match &e.project {
        Some(path) => format!(
            "项目经历 · {}",
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
        ),
        None => "跨项目记忆".into(),
    }
}
fn recent_experiences(entries: &[partners::Entry]) -> Vec<&partners::Entry> {
    let mut entries: Vec<_> = entries
        .iter()
        .filter(|e| !e.kind.starts_with("goal") && e.kind != "commitment" && e.kind != "identity")
        .filter(|e| {
            e.kind != "overview"
                || !entries
                    .iter()
                    .any(|other| other.kind != "overview" && other.source == e.source)
        })
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.occurred_at));
    entries
}
fn schedule_status(task: &schedules::Schedule) -> &'static str {
    if task.running {
        "执行中"
    } else if task.last_error.is_some() {
        "执行失败"
    } else if task.enabled {
        "已安排"
    } else if task.last_run.is_some() {
        "已完成"
    } else {
        "已暂停"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(
        cx: &mut gpui::TestAppContext,
    ) -> (
        tempfile::TempDir,
        Entity<AgentWorkspace>,
        &mut gpui::VisualTestContext,
        Uuid,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut w = AgentWorkspace::with_path(dir.path().join("sessions.json"), cx);
            let repo = w.partner_repo();
            let id = repo.create().unwrap();
            let partner = repo
                .list()
                .unwrap()
                .into_iter()
                .find(|p| p.id == id)
                .unwrap();
            w.select_partner(&partner, cx);
            w.page = crate::Page::Partners;
            w
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.run_until_parked();
        let id = host.read_with(window, |h, _| h.partners_ui.selected.unwrap());
        (dir, host, window, id)
    }

    #[gpui::test]
    fn partner_transfer_file_pickers_include_picture_and_cancel_without_changes(
        cx: &mut gpui::TestAppContext,
    ) {
        let (dir, host, window, id) = fixture(cx);
        let picture = dir.path().join("avatar.png");
        image::RgbaImage::from_pixel(32, 32, image::Rgba([50, 100, 200, 255]))
            .save(&picture)
            .unwrap();
        host.update(window, |h, cx| {
            let repo = h.partner_repo();
            let reference = repo.import_avatar(id, &picture).unwrap();
            repo.update(id, "知微", &reference, "迁移后继续一起工作")
                .unwrap();
            let partner = repo
                .list()
                .unwrap()
                .into_iter()
                .find(|p| p.id == id)
                .unwrap();
            h.select_partner(&partner, cx);
            cx.notify();
        });
        window.run_until_parked();
        let file = dir.path().join("知微.glint-partner.json");
        let export = window.debug_bounds("export-partner").unwrap();
        window.simulate_click(export.center(), gpui::Modifiers::default());
        assert!(window.did_prompt_for_new_path());
        window.simulate_new_path_selection(|_| Some(file.clone()));
        window.run_until_parked();
        assert!(file.is_file());
        host.read_with(window, |h, _| {
            assert!(h.partners_ui.transfer_task.is_none())
        });
        let import = window.debug_bounds("import-partner").unwrap();
        window.simulate_click(import.center(), gpui::Modifiers::default());
        assert!(window.did_prompt_for_paths());
        window.simulate_path_prompt_response(|options| {
            assert!(options.files && !options.directories && !options.multiple);
            Some(vec![file.clone()])
        });
        window.run_until_parked();
        host.read_with(window, |h, _| {
            let repo = h.partner_repo();
            assert_eq!(repo.list().unwrap().len(), 2);
            let imported = repo
                .list()
                .unwrap()
                .into_iter()
                .find(|p| Some(p.id) == h.partners_ui.selected)
                .unwrap();
            assert_ne!(imported.id, id);
            assert_eq!(imported.name, "知微");
            assert!(repo.avatar_path(&imported.avatar).unwrap().is_file());
            assert!(h.partners_ui.notice.as_ref().unwrap().contains("已导入"));
            assert!(h.partners_ui.transfer_task.is_none());
        });
        let import = window.debug_bounds("import-partner").unwrap();
        window.simulate_click(import.center(), gpui::Modifiers::default());
        window.simulate_path_prompt_response(|_| None);
        window.run_until_parked();
        host.update(window, |h, cx| {
            assert_eq!(h.partner_repo().list().unwrap().len(), 2);
            h.partners_ui
                .name
                .update(cx, |entry, cx| entry.set_value("尚未保存的名字", cx));
            let selected = h.partners_ui.selected.unwrap();
            h.export_partner_file(selected, cx);
            assert!(h.partners_ui.transfer_task.is_none());
            assert!(h.partners_ui.notice.as_ref().unwrap().contains("先保存"));
        });
    }

    #[gpui::test]
    fn layout_separates_settings_and_stacks_overview_at_narrow_width(
        cx: &mut gpui::TestAppContext,
    ) {
        let (_dir, host, window, id) = fixture(cx);
        let main = window.debug_bounds("partner-overview-main").unwrap();
        let side = window.debug_bounds("partner-overview-side").unwrap();
        assert!(side.origin.x >= main.origin.x + main.size.width);
        assert!(window.debug_bounds("partner-name-field").is_none());
        for (tab, panel) in [
            ("partner-tab-memories", "partner-memories-page"),
            ("partner-tab-wake", "partner-wakes-page"),
            ("partner-tab-settings", "partner-settings-page"),
        ] {
            let b = window.debug_bounds(tab).unwrap();
            window.simulate_click(b.center(), gpui::Modifiers::default());
            window.run_until_parked();
            assert!(window.debug_bounds(panel).is_some());
        }
        assert!(
            window
                .debug_bounds("partner-name-field")
                .unwrap()
                .size
                .height
                < px(80.)
        );
        assert_eq!(
            window
                .debug_bounds("partner-description-editor")
                .unwrap()
                .size
                .height,
            px(150.)
        );
        host.update(window, |h, cx| {
            h.partners_ui
                .name
                .update(cx, |e, cx| e.set_value("知微", cx));
            h.partners_ui.description.update(cx, |e, cx| {
                e.set_value("先验证，再动手。\n保留真实的经历。", cx)
            });
        });
        window.run_until_parked();
        host.update(window, |h, cx| {
            h.partners_ui.scroll.scroll_to_bottom();
            cx.notify();
        });
        window.run_until_parked();
        let save = window.debug_bounds("save-partner-settings").unwrap();
        window.simulate_click(save.center(), gpui::Modifiers::default());
        window.run_until_parked();
        host.read_with(window, |h, _| {
            let partner = h
                .partner_repo()
                .list()
                .unwrap()
                .into_iter()
                .find(|p| p.id == id)
                .unwrap();
            assert_eq!(partner.name, "知微");
            assert!(partner.description.contains('\n'));
        });
        host.update(window, |h, cx| h.partner_tab(Tab::Overview, cx));
        window.simulate_resize(size(px(840.), px(900.)));
        window.run_until_parked();
        let main = window.debug_bounds("partner-overview-main").unwrap();
        let side = window.debug_bounds("partner-overview-side").unwrap();
        assert!(side.origin.y >= main.origin.y + main.size.height);
        assert!(side.origin.x + side.size.width <= px(840.));
    }
    #[gpui::test]
    fn picture_avatar_preview_saves_and_can_reset_to_default(cx: &mut gpui::TestAppContext) {
        let (dir, host, window, id) = fixture(cx);
        let image = dir.path().join("avatar.png");
        image::RgbaImage::from_pixel(32, 32, image::Rgba([50, 100, 200, 255]))
            .save(&image)
            .unwrap();
        let reference = host.update(window, |h, cx| {
            let reference = h.partner_repo().import_avatar(id, &image).unwrap();
            h.partners_ui
                .avatar
                .update(cx, |e, cx| e.set_value(reference.clone(), cx));
            h.partner_tab(Tab::Settings, cx);
            reference
        });
        window.run_until_parked();
        assert_eq!(
            window
                .debug_bounds("partner-avatar-preview")
                .unwrap()
                .size
                .width,
            px(72.)
        );
        host.update(window, |h, cx| {
            h.partners_ui.scroll.scroll_to_bottom();
            cx.notify();
        });
        window.run_until_parked();
        let save = window.debug_bounds("save-partner-settings").unwrap();
        window.simulate_click(save.center(), gpui::Modifiers::default());
        window.run_until_parked();
        host.read_with(window, |h, _| {
            assert_eq!(
                h.partner_repo()
                    .list()
                    .unwrap()
                    .into_iter()
                    .find(|p| p.id == id)
                    .unwrap()
                    .avatar,
                reference
            )
        });
        host.update(window, |h, cx| {
            h.partners_ui.scroll.set_offset(point(px(0.), px(0.)));
            cx.notify();
        });
        window.run_until_parked();
        let reset = window.debug_bounds("reset-partner-avatar").unwrap();
        window.simulate_click(reset.center(), gpui::Modifiers::default());
        window.run_until_parked();
        host.update(window, |h, cx| {
            h.partners_ui.scroll.scroll_to_bottom();
            cx.notify();
        });
        window.run_until_parked();
        let save = window.debug_bounds("save-partner-settings").unwrap();
        window.simulate_click(save.center(), gpui::Modifiers::default());
        window.run_until_parked();
        host.read_with(window, |h, _| {
            assert_eq!(
                h.partner_repo()
                    .list()
                    .unwrap()
                    .into_iter()
                    .find(|p| p.id == id)
                    .unwrap()
                    .avatar,
                "◌"
            )
        });
    }
    #[gpui::test]
    fn deletion_requires_confirmation_cancel_and_escape_preserve_data(
        cx: &mut gpui::TestAppContext,
    ) {
        let (_dir, host, window, id) = fixture(cx);
        host.update(window, |h, cx| {
            h.history.active_mut().partner_id = Some(id);
            h.history.preferred_partner = Some(id);
            h.history
                .active_mut()
                .push(model::Role::User, "保留这段历史会话".into());
            database::save_history(&h.path, &h.history).unwrap();
            h.save(cx);
        });
        for escape in [false, true] {
            let delete = window.debug_bounds("delete-partner").unwrap();
            window.simulate_click(delete.center(), gpui::Modifiers::default());
            window.run_until_parked();
            assert!(window.debug_bounds("confirm-delete-partner").is_some());
            host.read_with(window, |h, _| {
                assert_eq!(h.partner_repo().list().unwrap().len(), 1)
            });
            if escape {
                window.simulate_keystrokes("escape");
            } else {
                let cancel = window.debug_bounds("cancel-delete-partner").unwrap();
                window.simulate_click(cancel.center(), gpui::Modifiers::default());
            }
            window.run_until_parked();
            assert!(window.debug_bounds("confirm-delete-partner").is_none());
            host.read_with(window, |h, _| {
                assert!(h.partner_repo().list().unwrap().iter().any(|p| p.id == id))
            });
        }
        let stale = host.read_with(window, |h, _| h.history.clone());
        let delete = window.debug_bounds("delete-partner").unwrap();
        window.simulate_click(delete.center(), gpui::Modifiers::default());
        window.run_until_parked();
        let confirm = window.debug_bounds("confirm-delete-partner").unwrap();
        window.simulate_click(confirm.center(), gpui::Modifiers::default());
        window.run_until_parked();
        host.read_with(window, |h, _| {
            assert!(h.partner_repo().list().unwrap().is_empty());
            assert!(h.history.active().partner_id.is_none());
            assert!(h.history.preferred_partner.is_none());
            assert_eq!(h.history.active().messages[0].text, "保留这段历史会话");
            database::save_history(&h.path, &stale).unwrap();
            let loaded = model::History::load(&h.path).unwrap();
            assert!(loaded.active().partner_id.is_none());
            assert!(loaded.preferred_partner.is_none());
            assert_eq!(loaded.active().messages[0].text, "保留这段历史会话");
        });
    }

    #[gpui::test]
    fn create_companion_and_start_bound_chat_in_hosted_workspace(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut w = AgentWorkspace::with_path(dir.path().join("sessions.json"), cx);
            w.page = crate::Page::Partners;
            w
        });
        window.run_until_parked();
        let create = window.debug_bounds("create-partner").unwrap();
        window.simulate_click(create.center(), gpui::Modifiers::default());
        window.run_until_parked();
        let selected = host.read_with(window, |h, cx| {
            assert_eq!(h.partner_repo().list().unwrap().len(), 1);
            assert_eq!(h.partners_ui.name.read(cx).value(), "未命名伙伴");
            h.partners_ui.selected.unwrap()
        });
        let chat = window.debug_bounds("chat-with-partner").unwrap();
        window.simulate_click(chat.center(), gpui::Modifiers::default());
        window.run_until_parked();
        host.read_with(window, |h, _| {
            assert_eq!(h.page, crate::Page::Chat);
            assert_eq!(h.history.active().partner_id, Some(selected));
            assert_eq!(h.history.preferred_partner, Some(selected));
            assert!(h.history.active().messages.is_empty());
        });
        assert!(window.debug_bounds("chat-partner").is_some());
    }
}

impl AgentWorkspace {
    pub(crate) fn open_partner_settings(&mut self, id: Uuid, cx: &mut Context<Self>) {
        match self.partner_repo().list() {
            Ok(ps) => {
                if let Some(partner) = ps.iter().find(|p| p.id == id) {
                    self.select_partner(partner, cx);
                    self.partners_ui.tab = Tab::Settings;
                    self.page = crate::Page::Partners;
                    cx.notify();
                }
            }
            Err(error) => {
                self.provider_error = Some(error);
                cx.notify();
            }
        }
    }
}
