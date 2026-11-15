use gpui::{Context, EventEmitter, Window, prelude::*};
use gpui_glint::{components::Icon, theme::ThemePreset};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Api(glint_api::Page),
    Agent(glint_agent::Page),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Navigate(Route),
    NewSession,
    ToggleTheme,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, Entity, Subscription, div, px};

    struct Harness {
        navigation: Entity<Navigation>,
        last: Option<Action>,
        _subscription: Subscription,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(46.)).h(px(650.)).child(self.navigation.clone())
        }
    }

    #[gpui::test]
    fn unified_rail_dispatches_routes_and_shared_actions(cx: &mut gpui::TestAppContext) {
        let (host, window) = cx.add_window_view(|_, cx| {
            let navigation = cx.new(|_| Navigation::default());
            let subscription =
                cx.subscribe(&navigation, |this: &mut Harness, _, action: &Action, cx| {
                    this.last = Some(*action);
                    cx.notify();
                });
            Harness {
                navigation,
                last: None,
                _subscription: subscription,
            }
        });
        host.read_with(window, |host, cx| {
            assert_eq!(
                host.navigation.read(cx).route,
                Route::Agent(glint_agent::Page::Chat)
            );
        });
        let agent = window.debug_bounds("nav-agent").unwrap();
        let calendar = window.debug_bounds("nav-calendar").unwrap();
        let api = window.debug_bounds("nav-api").unwrap();
        assert!(agent.top() < calendar.top());
        assert!(calendar.top() < api.top());
        for (selector, action) in [
            (
                "nav-api",
                Action::Navigate(Route::Api(glint_api::Page::Requests)),
            ),
            (
                "nav-environments",
                Action::Navigate(Route::Api(glint_api::Page::Environments)),
            ),
            (
                "nav-proxies",
                Action::Navigate(Route::Api(glint_api::Page::Proxies)),
            ),
            (
                "nav-plugins",
                Action::Navigate(Route::Api(glint_api::Page::Plugins)),
            ),
            (
                "nav-history",
                Action::Navigate(Route::Api(glint_api::Page::History)),
            ),
            (
                "nav-mcp",
                Action::Navigate(Route::Api(glint_api::Page::Mcp)),
            ),
            (
                "nav-agent",
                Action::Navigate(Route::Agent(glint_agent::Page::Chat)),
            ),
            (
                "nav-partners",
                Action::Navigate(Route::Agent(glint_agent::Page::Partners)),
            ),
            (
                "nav-channels",
                Action::Navigate(Route::Agent(glint_agent::Page::Channels)),
            ),
            (
                "nav-calendar",
                Action::Navigate(Route::Agent(glint_agent::Page::Calendar)),
            ),
            (
                "nav-settings",
                Action::Navigate(Route::Agent(glint_agent::Page::Settings)),
            ),
            (
                "nav-vault",
                Action::Navigate(Route::Api(glint_api::Page::Vault)),
            ),
            ("nav-new-session", Action::NewSession),
            ("nav-theme", Action::ToggleTheme),
        ] {
            let bounds = window.debug_bounds(selector).unwrap();
            window.simulate_click(bounds.center(), gpui::Modifiers::default());
            host.read_with(window, |host, _| assert_eq!(host.last, Some(action)));
        }
    }
}

pub struct Navigation {
    route: Route,
    theme: ThemePreset,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            route: Route::Agent(glint_agent::Page::Chat),
            theme: ThemePreset::Moonstone,
        }
    }
}

impl EventEmitter<Action> for Navigation {}

impl Navigation {
    pub fn set_state(&mut self, route: Route, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.route != route || self.theme != theme {
            self.route = route;
            self.theme = theme;
            cx.notify();
        }
    }
}

impl Render for Navigation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use glint_agent::Page as Agent;
        use glint_api::Page as Api;
        use gpui_glint::components::{NavigationGroup, NavigationItem, NavigationRail};
        let routes = [
            (
                "nav-agent",
                Icon::MessageCircle,
                "Agent 会话",
                Route::Agent(Agent::Chat),
            ),
            (
                "nav-partners",
                Icon::Users,
                "伙伴",
                Route::Agent(Agent::Partners),
            ),
            (
                "nav-channels",
                Icon::Radio,
                "频道",
                Route::Agent(Agent::Channels),
            ),
            (
                "nav-calendar",
                Icon::Calendar,
                "日程与定时任务",
                Route::Agent(Agent::Calendar),
            ),
            (
                "nav-api",
                Icon::Folders,
                "API 请求",
                Route::Api(Api::Requests),
            ),
            (
                "nav-environments",
                Icon::Globe,
                "环境变量",
                Route::Api(Api::Environments),
            ),
            (
                "nav-proxies",
                Icon::Shuffle,
                "代理管理",
                Route::Api(Api::Proxies),
            ),
            (
                "nav-history",
                Icon::Clock,
                "请求历史",
                Route::Api(Api::History),
            ),
            (
                "nav-plugins",
                Icon::Puzzle,
                "插件管理",
                Route::Api(Api::Plugins),
            ),
            (
                "nav-mcp",
                Icon::Workflow,
                "MCP 调用历史",
                Route::Api(Api::Mcp),
            ),
            ("nav-vault", Icon::Key, "密码箱", Route::Api(Api::Vault)),
            (
                "nav-settings",
                Icon::Settings,
                "Agent 设置",
                Route::Agent(Agent::Settings),
            ),
        ];
        let item = |(id, icon, title, _): &(&'static str, Icon, &'static str, Route)| {
            NavigationItem::new(*id, *title, *icon)
        };
        let mut agent: Vec<_> = routes[..3].iter().map(item).collect();
        agent.push(NavigationItem::new(
            "nav-new-session",
            "新建 Agent 会话",
            Icon::Plus,
        ));
        agent.push(item(&routes[3]));
        let mut footer: Vec<_> = routes[8..].iter().map(item).collect();
        footer.push(NavigationItem::new(
            "nav-theme",
            "切换明暗主题",
            Icon::HalfCircle,
        ));
        let selected = routes
            .iter()
            .find(|(_, _, _, route)| *route == self.route)
            .map(|entry| entry.0)
            .unwrap_or("nav-agent");
        NavigationRail::new([])
            .groups([
                NavigationGroup::new("nav-agent", agent),
                NavigationGroup::new("nav-api-agent", routes[4..8].iter().map(item)),
            ])
            .footer_group(NavigationGroup::new("nav-common", footer))
            .selected(selected)
            .theme(self.theme)
            .on_select(
                cx.listener(move |_, id: &gpui::SharedString, _, cx| match id.as_ref() {
                    "nav-new-session" => cx.emit(Action::NewSession),
                    "nav-theme" => cx.emit(Action::ToggleTheme),
                    _ => {
                        if let Some((_, _, _, route)) =
                            routes.iter().find(|(entry, _, _, _)| *entry == id.as_ref())
                        {
                            cx.emit(Action::Navigate(*route));
                        }
                    }
                }),
            )
            .render(cx)
    }
}
