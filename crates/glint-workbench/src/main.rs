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
use gpui_glint::{components::GlintAssets, theme::ThemePreset};
use navigation::{Action, Navigation, Route};

struct Workbench {
    _api_tools_task: gpui::Task<()>,
    api: Entity<glint_api::Workbench>,
    agent: Entity<glint_agent::AgentWorkspace>,
    navigation: Entity<Navigation>,
    agent_active: bool,
    theme: ThemePreset,
    _subscriptions: Vec<Subscription>,
}

impl Workbench {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let navigation = cx.new(|_| Navigation::default());
        let api = cx.new(|cx| glint_api::Workbench::hosted(navigation.clone().into(), cx));
        let (tools, api_tools_task) = api_tools::ApiTools::connect(&api, cx);
        let agent = cx.new(|cx| {
            let mut agent = glint_agent::AgentWorkspace::hosted(navigation.clone().into(), cx);
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
            cx.observe(&api, |_, _, cx| cx.notify()),
            cx.observe(&agent, |_, _, cx| cx.notify()),
        ];
        Self {
            _api_tools_task: api_tools_task,
            api,
            agent,
            navigation,
            agent_active: false,
            theme: ThemePreset::Moonstone,
            _subscriptions: subscriptions,
        }
    }

    fn dispatch(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            Action::Navigate(Route::Api(page)) => {
                self.agent_active = false;
                self.api
                    .update(cx, |api, cx| api.navigate(page, window, cx));
            }
            Action::Navigate(Route::Agent(page)) => {
                if !self.agent_active {
                    self.api.update(cx, |api, cx| api.deactivate(window, cx));
                }
                self.agent_active = true;
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
        cx.notify();
    }
}

impl Render for Workbench {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Module commands can also change the theme; keep the host and inactive module in sync.
        let theme = if self.agent_active {
            self.agent.read(cx).host_theme()
        } else {
            self.api.read(cx).host_theme()
        };
        if theme != self.theme {
            self.theme = theme;
            if self.agent_active {
                self.api.update(cx, |api, cx| api.set_host_theme(theme, cx));
            } else {
                self.agent
                    .update(cx, |agent, cx| agent.set_host_theme(theme, cx));
            }
        }
        let route = if self.agent_active {
            Route::Agent(self.agent.read(cx).page())
        } else {
            Route::Api(self.api.read(cx).page())
        };
        self.navigation.update(cx, |navigation, cx| {
            navigation.set_state(route, self.theme, cx)
        });
        div().size_full().child(if self.agent_active {
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
