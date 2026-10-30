#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod api_tools;
mod navigation;

use gpui::{
    App, AppContext, Bounds, Context, Entity, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowOptions, div, point, prelude::*, px, size,
};
use gpui_glint::{
    components::{AppLayout, GlintAssets, ShellRenderer},
    theme::ThemePreset,
};
use navigation::{Action, Navigation, Route};

struct Workbench {
    _api_tools_task: gpui::Task<()>,
    api: Entity<glint_api::Workbench>,
    agent: Entity<glint_agent::AgentWorkspace>,
    navigation: Entity<Navigation>,
    route: Route,
    theme: ThemePreset,
    _subscriptions: Vec<Subscription>,
}

impl Workbench {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let navigation = cx.new(|_| Navigation::default());
        let rail = navigation.clone();
        let shell: ShellRenderer = std::rc::Rc::new(move |mut slots| {
            slots.topbar = div()
                .size_full()
                .flex()
                .items_center()
                .gap_3()
                .pl(px(if cfg!(target_os = "macos") { 80. } else { 16. }))
                .text_size(px(12.))
                .child(
                    div()
                        .flex_shrink_0()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child("Glint Workbench"),
                )
                .child(div().flex_1().min_w_0().h_full().child(slots.topbar))
                .into_any_element();
            AppLayout::from_slots(slots)
                .navigation_rail(rail.clone())
                .navigation_rail_width(46.)
                .into_any_element()
        });
        let api = cx.new(|cx| glint_api::Workbench::hosted(shell.clone(), cx));
        let (tools, api_tools_task) = api_tools::ApiTools::connect(&api, cx);
        let agent = cx.new(|cx| {
            let mut agent = glint_agent::AgentWorkspace::hosted(shell.clone(), cx);
            agent.set_host_tools(tools);
            agent
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &navigation,
                window,
                |this, _, action: &Action, window, cx| {
                    this.dispatch(*action, window, cx);
                },
            ),
            cx.observe(&api, |this, _, cx| {
                if matches!(this.route, Route::Api(_)) {
                    this.route = Route::Api(this.api.read(cx).page());
                }
                this.sync_theme(this.api.read(cx).host_theme(), cx);
                this.sync_navigation(cx);
                cx.notify();
            }),
            cx.observe(&agent, |this, _, cx| {
                if matches!(this.route, Route::Agent(_)) {
                    this.route = Route::Agent(this.agent.read(cx).page());
                }
                this.sync_theme(this.agent.read(cx).host_theme(), cx);
                this.sync_navigation(cx);
                cx.notify();
            }),
        ];
        Self {
            _api_tools_task: api_tools_task,
            api,
            agent,
            navigation,
            route: Route::Api(glint_api::Page::Requests),
            theme: ThemePreset::Moonstone,
            _subscriptions: subscriptions,
        }
    }

    fn dispatch(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            Action::Navigate(Route::Api(page)) => {
                self.route = Route::Api(page);
                self.api
                    .update(cx, |api, cx| api.navigate(page, window, cx));
            }
            Action::Navigate(Route::Agent(page)) => {
                if matches!(self.route, Route::Api(_)) {
                    self.api.update(cx, |api, cx| api.deactivate(window, cx));
                }
                self.route = Route::Agent(page);
                self.agent
                    .update(cx, |agent, cx| agent.navigate(page, window, cx));
            }
            Action::NewSession => {
                self.dispatch(
                    Action::Navigate(Route::Agent(glint_agent::Page::Chat)),
                    window,
                    cx,
                );
                self.agent
                    .update(cx, |agent, cx| agent.create_session(window, cx));
            }
            Action::ToggleTheme => {
                self.theme = if self.theme.is_dark() {
                    ThemePreset::Moonstone
                } else {
                    ThemePreset::DarkObsidian
                };
                self.api
                    .update(cx, |api, cx| api.set_host_theme(self.theme, cx));
                self.agent
                    .update(cx, |agent, cx| agent.set_host_theme(self.theme, cx));
            }
        }
        self.sync_navigation(cx);
        cx.notify();
    }

    fn sync_navigation(&self, cx: &mut Context<Self>) {
        self.navigation.update(cx, |navigation, cx| {
            navigation.set_state(self.route, self.theme, cx)
        });
    }

    fn sync_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if theme == self.theme {
            return;
        }
        self.theme = theme;
        self.api.update(cx, |api, cx| {
            if api.host_theme() != theme {
                api.set_host_theme(theme, cx);
            }
        });
        self.agent.update(cx, |agent, cx| {
            if agent.host_theme() != theme {
                agent.set_host_theme(theme, cx);
            }
        });
    }
}

impl Render for Workbench {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(if matches!(self.route, Route::Agent(_)) {
                self.agent.clone().into_any_element()
            } else {
                self.api.clone().into_any_element()
            })
    }
}

fn main() {
    if glint_api::handle_background_command() {
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("Glint Workbench {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let _instance = match glint_api::workbench_guard() {
        Ok(Some(guard)) => guard,
        Ok(None) => {
            eprintln!("API 工作区已由 Glint API 或 Glint Workbench 打开。");
            return;
        }
        Err(error) => {
            eprintln!("无法锁定工作区：{error}");
            std::process::exit(1);
        }
    };
    gpui_platform::application()
        .with_assets(GlintAssets)
        .run(|cx: &mut App| {
            glint_api::initialize(cx);
            glint_agent::initialize(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(1100.), px(700.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Glint Workbench".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(16.), px(14.))),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let app = cx.new(|cx| Workbench::new(window, cx));
                    let api = app.read(cx).api.downgrade();
                    window.on_window_should_close(cx, move |_, cx| {
                        let _ = api.update(cx, |api, cx| api.flush(cx));
                        true
                    });
                    let entity = app.read(cx).api.clone();
                    entity.update(cx, |api, cx| {
                        api.navigate(glint_api::Page::Requests, window, cx)
                    });
                    app
                },
            )
            .expect("无法打开 Glint Workbench 窗口");
            cx.activate(true);
        });
}
