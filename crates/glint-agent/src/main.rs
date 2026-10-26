#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app_icon;
mod app_tools_ui;
mod chatgpt;
mod discovery;
mod generation;
mod model;
mod project_ui;
mod provider;
mod provider_catalog;
mod provider_ui;
mod schedule_ui;
mod schedules;
mod tools;

use gpui::{
    App, AppContext, Bounds, Context, Entity, Focusable, FontWeight, KeyBinding, ScrollHandle,
    Subscription, Task, TitlebarOptions, Window, WindowBounds, WindowOptions, div, img, point,
    prelude::*, px, rgb, size,
};
use gpui_glint::{
    components::{
        AppLayout, Button, ButtonKind, ButtonSize, Calendar, CalendarEvent, Composer, ComposerChip,
        ComposerEvent, GlintAssets, GroupList, GroupListEvent, Icon, IconButton, SearchInput,
        SegmentOption, SegmentedControl, TextEntry, bind_composer_keys, bind_prompt_editor_keys,
        bind_text_entry_keys, window_drag_region,
    },
    theme::ThemePreset,
};
use model::{History, Role};
use std::path::PathBuf;
use uuid::Uuid;

gpui::actions!(agent_workspace, [NewSession, SearchSessions, OpenSettings]);

struct AgentWorkspace {
    group_list: Entity<GroupList>,
    calendar: Entity<Calendar>,
    show_calendar: bool,
    schedules: std::sync::Arc<std::sync::Mutex<schedules::ScheduleStore>>,
    schedule_editor: schedule_ui::ScheduleEditor,
    scheduler_task: Option<Task<()>>,
    running_schedule: Option<Uuid>,
    tool_listener: Option<Task<()>>,
    tool_activity: Vec<tools::ToolRecord>,
    transcript: Vec<generation::ContentBlock>,
    markdown_views:
        std::cell::RefCell<std::collections::HashMap<String, app_tools_ui::MarkdownView>>,
    process_panels:
        std::cell::RefCell<std::collections::HashMap<String, app_tools_ui::ProcessPanel>>,
    tool_panels: std::cell::RefCell<std::collections::HashMap<String, app_tools_ui::ToolPanel>>,
    approval: Option<app_tools_ui::Approval>,
    catalog: provider_catalog::ProviderCatalog,
    history: History,
    provider: provider::Config,
    provider_settings: provider_ui::Settings,
    provider_error: Option<String>,
    show_settings: bool,
    path: PathBuf,
    storage_error: Option<String>,
    // Do not overwrite a file that failed to load.
    storage_readable: bool,
    theme: ThemePreset,
    composer: Entity<Composer>,
    search: Entity<TextEntry>,
    _subscriptions: Vec<Subscription>,
    scroll: ScrollHandle,
    show_sessions: bool,
    pending: Option<(Uuid, Task<()>)>,
}

impl AgentWorkspace {
    fn new(cx: &mut Context<Self>) -> Self {
        let path = history_path();
        let (history, storage_error, storage_readable) = match History::load(&path) {
            Ok(history) => (history, None, true),
            Err(error) => (
                History::default(),
                Some(format!(
                    "无法读取会话记录：{error}。当前修改仅保留在内存中。"
                )),
                false,
            ),
        };
        let discovered = discovery::DiscoveryRegistry::default().discover();
        let (catalog, provider_error) = match provider_catalog::ProviderCatalog::load(
            &path.with_file_name("providers.json"),
            &discovered,
        ) {
            Ok(catalog) => (catalog, None),
            Err(error) => (
                provider_catalog::ProviderCatalog {
                    providers: vec![],
                    active: None,
                    active_model: String::new(),
                },
                Some(error),
            ),
        };
        let provider = catalog.current();
        let editing = catalog
            .providers
            .iter()
            .find(|profile| Some(profile.id) == catalog.active)
            .or(catalog.providers.first());
        let mut provider_settings = provider_ui::Settings::new(editing, discovered, cx);
        provider_settings.message = provider_error.clone();
        let theme = ThemePreset::Moonstone;
        let group_list =
            cx.new(|_| GroupList::new(project_ui::session_groups(&history, "")).theme(theme));
        let calendar = cx.new(|cx| Calendar::new(cx).theme(theme));
        let schedule_path = path.with_file_name("schedules.json");
        let schedules = std::sync::Arc::new(std::sync::Mutex::new(
            schedules::ScheduleStore::open(schedule_path.clone())
                .unwrap_or_else(|_| schedules::ScheduleStore::unavailable(schedule_path)),
        ));
        let schedule_editor = schedule_ui::ScheduleEditor::new(cx);
        let draft = history.active().draft.clone();
        let composer = cx.new(|cx| {
            Composer::new("描述任务，或继续追问…", cx)
                .clear_on_submit(false)
                .menu_chip(model_chip(&catalog))
                .disabled(!provider.enabled)
                .theme(theme)
        });
        composer.update(cx, |editor, cx| editor.set_value(draft, cx));
        let search = cx.new(|cx| TextEntry::new(cx, "搜索会话…", "").theme(theme));
        let subscriptions = vec![
            cx.subscribe(&group_list, |this, _, event: &GroupListEvent, cx| {
                if let GroupListEvent::ItemSelected { item, .. } = event {
                    if let Ok(id) = Uuid::parse_str(item) {
                        this.switch(id, cx);
                    }
                }
            }),
            cx.subscribe(&calendar, |this, _, event: &CalendarEvent, cx| {
                match event {
                    CalendarEvent::DateSelected(_) => {
                        this.schedule_editor.open = false;
                    }
                    CalendarEvent::MonthChanged(_) => {}
                }
                cx.notify();
            }),
            cx.subscribe(
                &composer,
                |this, _, event: &ComposerEvent, cx| match event {
                    ComposerEvent::Changed(value) => {
                        this.history.active_mut().draft = value.to_string();
                        this.save();
                        cx.notify();
                    }
                    ComposerEvent::Submitted(text) => this.send(text.to_string(), cx),
                    ComposerEvent::ChipSelected { chip, value } if chip.as_ref() == "model" => {
                        this.choose_model(value, cx)
                    }
                    ComposerEvent::ChipClicked(chip) if chip.as_ref() == "model" => {
                        this.show_settings = true;
                        this.provider_settings.section = provider_ui::Section::Provider;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
            cx.observe(&search, |_, _, cx| cx.notify()),
            cx.observe(&provider_settings.search, |_, _, cx| cx.notify()),
        ];
        let mut workspace = Self {
            group_list,
            calendar,
            schedules,
            schedule_editor,
            show_calendar: false,
            scheduler_task: None,
            running_schedule: None,
            tool_listener: None,
            tool_activity: vec![],
            transcript: vec![],
            markdown_views: std::cell::RefCell::new(std::collections::HashMap::new()),
            process_panels: std::cell::RefCell::new(std::collections::HashMap::new()),
            tool_panels: std::cell::RefCell::new(std::collections::HashMap::new()),
            approval: None,
            history,
            catalog,
            provider,
            provider_settings,
            provider_error,
            show_settings: false,
            path,
            storage_error,
            storage_readable,
            theme,
            composer,
            search,
            _subscriptions: subscriptions,
            scroll: ScrollHandle::new(),
            show_sessions: true,
            pending: None,
        };
        workspace.refresh_calendar(cx);
        workspace.start_scheduler(cx);
        if workspace.provider.enabled
            && workspace.provider.kind == provider::Kind::Compatible
            && workspace
                .provider_settings
                .discovered
                .iter()
                .any(|found| found.config.base_url == workspace.provider.base_url)
        {
            workspace.refresh_models(cx);
        }
        workspace
    }

    fn save(&mut self) {
        if self.storage_readable {
            self.storage_error = self
                .history
                .save(&self.path)
                .err()
                .map(|error| format!("会话保存失败：{error}。修改尚未写入磁盘。"));
        }
    }

    fn switch(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.show_settings = false;
        self.show_calendar = false;
        self.provider_error = None;
        self.history.active_mut().draft = self.composer.read(cx).value(cx).to_string();
        self.history.active = id;
        let draft = self.history.active().draft.clone();
        self.composer
            .update(cx, |editor, cx| editor.set_value(draft, cx));
        self.scroll = ScrollHandle::new();
        self.save();
        cx.notify();
    }

    fn new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_settings = false;
        self.provider_error = None;
        self.history.active_mut().draft = self.composer.read(cx).value(cx).to_string();
        self.show_calendar = false;
        self.history
            .new_session_for(self.history.active().project.clone());
        let draft = self.history.active().draft.clone();
        self.composer
            .update(cx, |editor, cx| editor.set_value(draft, cx));
        self.scroll = ScrollHandle::new();
        self.save();
        window.focus(&self.composer.focus_handle(cx), cx);
        cx.notify();
    }

    fn send(&mut self, text: String, cx: &mut Context<Self>) {
        self.send_to(self.history.active, text, self.provider.clone(), true, cx);
    }

    pub fn send_to(
        &mut self,
        id: Uuid,
        text: String,
        config: provider::Config,
        clear_composer: bool,
        cx: &mut Context<Self>,
    ) {
        if self.pending.is_some() {
            return;
        }
        let text = text.trim().to_owned();
        if text.is_empty() {
            return;
        }
        if !config.enabled {
            self.provider_error = Some("请先在设置中连接 Provider。".into());
            cx.notify();
            return;
        }
        let Some(index) = self
            .history
            .sessions
            .iter()
            .position(|session| session.id == id)
        else {
            return;
        };
        let (events, receiver) = async_channel::unbounded();
        let tool_context = tools::ToolContext {
            root: self.history.sessions[index].project.clone(),
            schedules: self.schedules.clone(),
            provider_id: config.id,
            model: config.model.clone(),
            events,
        };
        let result = (|| {
            let key = if config.kind == provider::Kind::ChatGpt {
                String::new()
            } else {
                config.key()?
            };
            let mut messages = provider::messages(&self.history.sessions[index].messages);
            messages.insert(0,serde_json::json!({"role":"system","content":format!("你是 Glint Agent。当前本地时间：{}。当前项目：{}。使用工具完成实际工作；只有工具返回成功后才能声称操作已完成。日程使用带时区的 ISO 8601 时间。文件工具只作用于已打开的项目，写入、命令和日程修改需要界面确认。",chrono::Local::now().to_rfc3339(),self.history.sessions[index].project.as_ref().map(|path|path.display().to_string()).unwrap_or_else(||"未打开项目".into()))}));
            messages.push(serde_json::json!({"role":"user", "content":text}));
            provider::Request::start_with_tools(config, key, messages, Some(tool_context))
        })();
        let request = match result {
            Ok(request) => request,
            Err(error) => {
                self.provider_error = Some(error);
                cx.notify();
                return;
            }
        };
        self.provider_error = None;
        self.watch_tools(receiver, cx);
        self.history.sessions[index].push(Role::User, text);
        self.history.sessions[index].draft.clear();
        if clear_composer {
            self.composer
                .update(cx, |editor, cx| editor.set_value("", cx));
        }
        self.save();
        self.scroll.scroll_to_bottom();
        let task = cx.spawn(async move |weak, cx| {
            let result = request.response().await;
            let _ = weak.update(cx, |this, cx| {
                let error = result.as_ref().err().cloned();
                if let Some(session) = this.history.sessions.iter_mut().find(|s| s.id == id) {
                    match result {
                        Ok(completion) => {
                            session.push(Role::Agent, completion.text);
                            session.messages.last_mut().unwrap().blocks = completion.blocks;
                            session.messages.last_mut().unwrap().stats = Some(completion.stats);
                            session.messages.last_mut().unwrap().tools = completion.tool_records;
                        }
                        Err(error) => {
                            session.push(Role::Notice, format!("请求失败：{error}"));
                            session.messages.last_mut().unwrap().tools = this.tool_activity.clone();
                            session.messages.last_mut().unwrap().blocks = this.transcript.clone();
                        }
                    }
                }
                this.pending = None;
                this.tool_listener = None;
                this.approval = None;
                this.tool_activity.clear();
                this.transcript.clear();
                if let Some(task) = this.running_schedule.take() {
                    let _ = this
                        .schedules
                        .lock()
                        .map(|mut store| store.finish(task, error));
                }
                this.refresh_calendar(cx);
                this.composer.update(cx, |composer, cx| {
                    composer.set_disabled(false, cx);
                    composer.set_busy(false, cx);
                });
                this.save();
                if this.history.active == id {
                    this.scroll.scroll_to_bottom();
                }
                cx.notify();
            });
        });
        self.pending = Some((id, task));
        self.composer.update(cx, |composer, cx| {
            composer.set_busy(true, cx);
            composer.set_disabled(true, cx);
        });
        cx.notify();
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some((id, _task)) = self.pending.take() {
            self.tool_listener = None;
            self.approval = None;
            if let Some(task) = self.running_schedule.take() {
                let _ = self
                    .schedules
                    .lock()
                    .map(|mut store| store.finish(task, Some("用户已停止任务".into())));
            }
            self.composer.update(cx, |composer, cx| {
                composer.set_disabled(false, cx);
                composer.set_busy(false, cx);
            });
            if let Some(session) = self.history.sessions.iter_mut().find(|s| s.id == id) {
                session.push(Role::Notice, "已停止回复。".into());
                session.messages.last_mut().unwrap().blocks = std::mem::take(&mut self.transcript);
                session.messages.last_mut().unwrap().tools =
                    std::mem::take(&mut self.tool_activity);
            }
            self.save();
            cx.notify();
        }
    }

    fn search_sessions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_settings {
            window.focus(&self.provider_settings.search.focus_handle(cx), cx);
            return;
        }
        self.show_sessions = true;
        window.focus(&self.search.focus_handle(cx), cx);
        cx.notify();
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_settings = true;
        self.show_calendar = false;
        window.focus(&self.provider_settings.search.focus_handle(cx), cx);
        cx.notify();
    }

    fn topbar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        div()
            .size_full()
            .flex()
            .items_center()
            .gap_3()
            .pl(px(if cfg!(target_os = "macos") { 80. } else { 16. }))
            .pr_3()
            .text_size(px(12.))
            .child(img(app_icon::image()).size(px(22.)).flex_shrink_0())
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Glint Agent"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(rgb(p.text_muted))
                    .child(Icon::Folder.render(14.))
                    .id("open-project")
                    .cursor_pointer()
                    .child(
                        self.history
                            .active()
                            .project
                            .as_ref()
                            .map(|path| {
                                path.file_name()
                                    .unwrap_or(path.as_os_str())
                                    .to_string_lossy()
                                    .into_owned()
                            })
                            .unwrap_or_else(|| "打开项目".into()),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.open_project(cx)))
                    .child("›"),
            )
            .child(
                div()
                    .max_w(px(260.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(if self.show_settings {
                        format!("设置  ›  {}", self.provider_settings.section.title())
                    } else {
                        self.history.active().title.clone()
                    }),
            )
            .child(window_drag_region())
            .child(
                div()
                    .id("topbar-search")
                    .w(px(240.))
                    .h(px(30.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(p.border))
                    .bg(rgb(p.panel))
                    .text_color(rgb(p.text_muted))
                    .text_size(px(11.))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(p.surface)))
                    .child(Icon::Search.render(15.))
                    .child(div().flex_1().child(if self.show_settings {
                        "搜索设置"
                    } else {
                        "搜索会话"
                    }))
                    .child(if cfg!(target_os = "macos") {
                        "⌘ K"
                    } else {
                        "Ctrl K"
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.search_sessions(window, cx))),
            )
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded(px(5.))
                    .bg(rgb(p.accent_tint))
                    .text_size(px(10.))
                    .text_color(rgb(p.accent))
                    .child(if self.provider.enabled {
                        self.provider.name.clone()
                    } else {
                        "未连接 Provider".into()
                    }),
            )
            .when(!self.show_settings, |bar| {
                bar.child(
                    IconButton::new(Icon::Sidebar, "展开或收起会话记录")
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_sessions = !this.show_sessions;
                            cx.notify();
                        })),
                )
            })
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
                IconButton::new(Icon::Plus, "新建会话")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, window, cx| this.new_session(window, cx))),
            )
            .child(
                IconButton::new(Icon::Clock, "会话记录")
                    .theme(self.theme)
                    .render()
                    .when(self.show_sessions, |s| {
                        s.bg(rgb(self.theme.palette().accent_tint))
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_sessions = !this.show_sessions;
                        cx.notify();
                    })),
            )
            .child(
                IconButton::new(Icon::Search, "搜索会话")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, window, cx| this.search_sessions(window, cx))),
            )
            .child(
                IconButton::new(Icon::Calendar, "日程与定时任务")
                    .theme(self.theme)
                    .render()
                    .when(self.show_calendar, |button| {
                        button.bg(rgb(self.theme.palette().accent_tint))
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_settings = false;
                        this.show_calendar = true;
                        this.refresh_calendar(cx);
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                IconButton::new(Icon::Settings, "设置 Provider")
                    .theme(self.theme)
                    .render()
                    .when(self.show_settings, |s| {
                        s.bg(rgb(self.theme.palette().accent_tint))
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
            )
            .child(
                IconButton::new(Icon::HalfCircle, "切换明暗主题")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.theme = if this.theme.is_dark() {
                            ThemePreset::Moonstone
                        } else {
                            ThemePreset::DarkObsidian
                        };
                        this.composer
                            .update(cx, |editor, cx| editor.set_theme(this.theme, cx));
                        this.search
                            .update(cx, |editor, cx| editor.set_theme(this.theme, cx));
                        this.provider_settings.set_theme(this.theme, cx);
                        cx.notify();
                    })),
            )
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let groups = project_ui::session_groups(&self.history, self.search.read(cx).value());
        self.group_list.update(cx, |list, cx| {
            list.set_groups(groups, cx);
            list.set_selected(Some(self.history.active.to_string().into()), cx);
            list.set_theme(self.theme, cx);
        });
        let weak = cx.entity().downgrade();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(p.panel))
            .child(
                div()
                    .h(px(44.))
                    .px_3()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("会话"),
                    )
                    .child(
                        IconButton::new(Icon::Folder, "打开项目")
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.open_project(cx))),
                    )
                    .child(
                        IconButton::new(Icon::Plus, "新建会话")
                            .theme(self.theme)
                            .render()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.new_session(window, cx)),
                            ),
                    ),
            )
            .child(
                div().px_3().pb_3().child(
                    SearchInput::new("搜索会话…")
                        .control(self.search.clone())
                        .theme(self.theme)
                        .render()
                        .w_full(),
                ),
            )
            .child(
                div().px_3().pb_3().child(
                    SegmentedControl::new([
                        SegmentOption::new("按项目"),
                        SegmentOption::new("按日期"),
                    ])
                    .id("session-grouping")
                    .selected(if self.history.group_by_project { 0 } else { 1 })
                    .theme(self.theme)
                    .on_change(move |index, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.history.group_by_project = index == 0;
                            this.save();
                            cx.notify();
                        });
                    })
                    .render()
                    .w_full(),
                ),
            )
            .child(
                div()
                    .id("session-groups")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.group_list.clone()),
            )
            .child(
                div()
                    .p_3()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child(format!(
                        "{} 个会话 · 自动保存到本机",
                        self.history.sessions.len()
                    )),
            )
    }

    fn welcome(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let mut welcome = div()
            .w_full()
            .max_w(px(720.))
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_color(rgb(p.accent))
                    .child(Icon::Workflow.render(32.)),
            )
            .child(
                div()
                    .text_size(px(28.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("一起完成下一件事。"),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(p.text_muted))
                    .child("从一个问题、一项改动，或一个想法开始。"),
            )
            .child(
                div()
                    .mt_4()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("从这里开始"),
            );
        for (index, (icon, title, prompt)) in [
            (
                Icon::Compass,
                "了解这个项目",
                "梳理这个项目的目录结构，说明各模块的职责。",
            ),
            (
                Icon::Code,
                "实现一个功能",
                "参考现有组件，为应用实现一个新的功能。",
            ),
            (
                Icon::Search,
                "检查代码问题",
                "检查最近的代码改动，找出潜在问题并提出修复建议。",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            welcome = welcome.child(
                div()
                    .id(("suggestion", index))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_3()
                    .rounded(px(9.))
                    .border_1()
                    .border_color(rgb(p.border))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(p.surface_subtle)))
                    .child(icon.render(17.).text_color(rgb(p.text_muted)))
                    .child(div().flex_1().child(title))
                    .child(Icon::ArrowUp.render(14.).text_color(rgb(p.text_muted)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.composer
                            .update(cx, |editor, cx| editor.set_value(prompt, cx));
                        this.history.active_mut().draft = prompt.into();
                        this.save();
                        window.focus(&this.composer.focus_handle(cx), cx);
                        cx.notify();
                    })),
            );
        }
        welcome
    }

    fn content(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let session = self.history.active();
        let running = self
            .pending
            .as_ref()
            .is_some_and(|(id, _)| *id == session.id);
        let mut messages = div()
            .id("conversation")
            .track_scroll(&self.scroll)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .px(px(40.))
            .py_6()
            .gap_6();
        if session.messages.is_empty() {
            messages = messages.justify_center().child(self.welcome(cx));
        } else {
            for (index, message) in session.messages.iter().enumerate() {
                let user = message.role == Role::User;
                messages = messages.child(
                    div()
                        .id(("message", index))
                        .w_full()
                        .max_w(px(800.))
                        .flex()
                        .gap_3()
                        .child(
                            div()
                                .size(px(28.))
                                .flex_shrink_0()
                                .rounded(px(8.))
                                .bg(rgb(if user {
                                    p.surface_subtle
                                } else {
                                    p.accent_tint
                                }))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(if user { Icon::User } else { Icon::Workflow }.render(16.)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_3()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(if user {
                                            "你"
                                        } else if message.role == Role::Notice {
                                            "提示"
                                        } else {
                                            "Glint Agent"
                                        })
                                        .child(
                                            div()
                                                .text_size(px(10.))
                                                .font_weight(FontWeight::NORMAL)
                                                .text_color(rgb(p.text_muted))
                                                .child(message.time.format("%H:%M").to_string()),
                                        ),
                                )
                                .child(if message.blocks.is_empty() {
                                    div()
                                        .child(self.markdown_text(
                                            format!("{}:{index}:legacy", self.history.active),
                                            &message.text,
                                            cx,
                                        ))
                                        .when(!message.tools.is_empty(), |body| {
                                            body.child(self.tool_records(&message.tools, cx))
                                        })
                                } else {
                                    div()
                                        .child(self.response_view(
                                            &format!("{}:{index}", self.history.active),
                                            &message.blocks,
                                            message.stats.as_ref(),
                                            true,
                                            cx,
                                        ))
                                        .when(message.role == Role::Notice, |body| {
                                            body.child(self.markdown_text(
                                                format!("{}:{index}:notice", self.history.active),
                                                &message.text,
                                                cx,
                                            ))
                                        })
                                })
                                .when_some(message.stats.as_ref(), |body, stats| {
                                    body.child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(rgb(p.text_muted))
                                            .child(stats.caption()),
                                    )
                                }),
                        ),
                );
            }
        }
        if running {
            if !self.transcript.is_empty() {
                let key = format!(
                    "{}:{}",
                    self.history.active,
                    self.history
                        .sessions
                        .iter()
                        .find(|session| session.id == self.history.active)
                        .map(|session| session.messages.len())
                        .unwrap_or(0)
                );
                messages =
                    messages.child(div().w_full().max_w(px(800.)).child(self.response_view(
                        &key,
                        &self.transcript,
                        None,
                        false,
                        cx,
                    )));
            }
            messages = messages.child(
                div()
                    .w_full()
                    .max_w(px(800.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(rgb(p.text_muted))
                    .child(Icon::Spinner.render(15.))
                    .child("正在等待模型回复…"),
            );
        }
        let composer = div()
            .w_full()
            .max_w(px(800.))
            .child(self.composer.clone())
            .when(self.pending.is_some(), |panel| {
                panel.child(
                    div().mt_2().flex().justify_end().child(
                        Button::new("停止回复")
                            .icon(Icon::Pause)
                            .kind(ButtonKind::Gray)
                            .size(ButtonSize::Small)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                    ),
                )
            });
        div()
            .size_full()
            .flex()
            .flex_col()
            .when_some(self.provider_error.clone(), |root, error| {
                root.child(div().px_5().py_2().text_color(rgb(0xd9342b)).child(error))
            })
            .child(
                div()
                    .h(px(52.))
                    .flex_shrink_0()
                    .px_5()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(Icon::Text.render(16.).text_color(rgb(p.text_muted)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_ellipsis()
                            .font_weight(FontWeight::MEDIUM)
                            .child(session.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(format!("{} 条消息", session.messages.len())),
                    ),
            )
            .when_some(self.storage_error.clone(), |root, error| {
                root.child(div().px_5().py_2().text_color(rgb(0xd9342b)).child(error))
            })
            .child(messages)
            .child(
                div()
                    .flex_shrink_0()
                    .w_full()
                    .px(px(40.))
                    .pb_3()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child(composer)
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(if self.provider.enabled {
                                format!(
                                    "{} · {} · 仅文本对话",
                                    self.provider.name, self.provider.model
                                )
                            } else {
                                "请先在设置中连接 Provider".into()
                            }),
                    ),
            )
    }
}

impl Render for AgentWorkspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .key_context("AgentWorkspace")
            .text_size(px(12.))
            .on_action(cx.listener(|this, _: &NewSession, window, cx| this.new_session(window, cx)))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SearchSessions, window, cx| {
                    this.search_sessions(window, cx)
                }),
            )
            .child(
                AppLayout::new(
                    self.topbar(cx),
                    if self.show_settings {
                        self.settings_sidebar(cx)
                    } else if self.show_calendar {
                        self.calendar_sidebar(cx)
                    } else {
                        self.sidebar(cx)
                    },
                    if self.show_settings {
                        self.settings_page(cx)
                    } else if self.show_calendar {
                        self.calendar_page(cx)
                    } else {
                        self.content(cx)
                    },
                )
                .navigation_rail(self.rail(cx))
                .navigation_rail_width(46.)
                .sidebar_width(if self.show_settings { 280. } else { 260. })
                .sidebar_visible(self.show_settings || self.show_calendar || self.show_sessions)
                .theme(self.theme),
            )
            .when(self.approval.is_some(), |root| {
                root.child(
                    div()
                        .absolute()
                        .bottom(px(20.))
                        .left(px(64.))
                        .right(px(24.))
                        .flex()
                        .justify_center()
                        .child(self.approval_panel(cx)),
                )
            })
    }
}

fn model_chip(catalog: &provider_catalog::ProviderCatalog) -> ComposerChip {
    let mut chip = ComposerChip::new("model").label("选择模型").chevron();
    for profile in catalog
        .providers
        .iter()
        .filter(|profile| profile.enabled && profile.validate().is_ok())
    {
        let models = if profile.models.is_empty() && profile.kind == provider::Kind::ChatGpt {
            vec![String::new()]
        } else {
            profile.models.clone()
        };
        for model in models {
            chip = chip.option(
                format!("{}:{model}", profile.id),
                format!(
                    "{} · {}",
                    profile.name,
                    if model.is_empty() {
                        "默认模型"
                    } else {
                        &model
                    }
                ),
            );
        }
    }
    if let Some(id) = catalog.active {
        chip = chip.default_selected(format!("{id}:{}", catalog.active_model));
    }
    chip
}

fn history_path() -> PathBuf {
    if let Some(path) = std::env::var_os("GLINT_AGENT_DATA_DIR") {
        return PathBuf::from(path).join("sessions.json");
    }
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")));
    base.unwrap_or_else(|| PathBuf::from("."))
        .join("Glint Agent/sessions.json")
}

fn main() {
    gpui_platform::application()
        .with_assets(GlintAssets)
        .run(|cx: &mut App| {
            app_icon::install();
            bind_text_entry_keys(cx);
            bind_prompt_editor_keys(cx);
            bind_composer_keys(cx);
            for modifier in ["cmd", "ctrl"] {
                cx.bind_keys([
                    KeyBinding::new(
                        &format!("{modifier}-,"),
                        OpenSettings,
                        Some("AgentWorkspace"),
                    ),
                    KeyBinding::new(&format!("{modifier}-n"), NewSession, Some("AgentWorkspace")),
                    KeyBinding::new(
                        &format!("{modifier}-k"),
                        SearchSessions,
                        Some("AgentWorkspace"),
                    ),
                ]);
            }
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(1360.), px(860.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(1000.), px(680.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Glint Agent".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(16.), px(14.))),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let app = cx.new(AgentWorkspace::new);
                    window.focus(&app.read(cx).composer.focus_handle(cx), cx);
                    app
                },
            )
            .expect("无法打开 Glint Agent 窗口");
            cx.activate(true);
        });
}
