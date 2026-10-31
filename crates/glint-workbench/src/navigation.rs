use gpui::{Context, EventEmitter, Window, div, prelude::*, px, rgb};
use gpui_glint::{
    components::{Icon, IconButton, Tooltip},
    theme::ThemePreset,
};

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
    use gpui::{AppContext, Entity, Subscription};

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
            route: Route::Api(glint_api::Page::Requests),
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

    fn route_button(
        &self,
        id: &'static str,
        icon: Icon,
        title: &'static str,
        route: Route,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let button = IconButton::new(icon, title)
            .theme(self.theme)
            .render()
            .id(id)
            .debug_selector(move || id.into())
            .when(self.route == route, |button| {
                button
                    .bg(rgb(self.theme.palette().accent_tint))
                    .text_color(rgb(self.theme.palette().accent))
            })
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(Action::Navigate(route))));
        Tooltip::new(button, title).theme(self.theme).render(cx)
    }
}

impl Render for Navigation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use glint_agent::Page as Agent;
        use glint_api::Page as Api;
        let separator = |id: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .w(px(24.))
                .my_1()
                .h(px(1.))
                .flex_shrink_0()
                .bg(rgb(self.theme.palette().border))
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .py_3()
            .child(self.route_button(
                "nav-api",
                Icon::Folders,
                "API 请求",
                Route::Api(Api::Requests),
                cx,
            ))
            .child(self.route_button(
                "nav-environments",
                Icon::Globe,
                "环境变量",
                Route::Api(Api::Environments),
                cx,
            ))
            .child(self.route_button(
                "nav-proxies",
                Icon::Shuffle,
                "代理管理",
                Route::Api(Api::Proxies),
                cx,
            ))
            .child(self.route_button(
                "nav-history",
                Icon::Clock,
                "请求历史",
                Route::Api(Api::History),
                cx,
            ))
            .child(separator("nav-api-agent-divider"))
            .child(self.route_button(
                "nav-agent",
                Icon::MessageCircle,
                "Agent 会话",
                Route::Agent(Agent::Chat),
                cx,
            ))
            .child(self.route_button(
                "nav-partners",
                Icon::Users,
                "伙伴",
                Route::Agent(Agent::Partners),
                cx,
            ))
            .child(
                Tooltip::new(
                    IconButton::new(Icon::Plus, "新建 Agent 会话")
                        .theme(self.theme)
                        .render()
                        .id("nav-new-session")
                        .debug_selector(|| "nav-new-session".into())
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(Action::NewSession))),
                    "新建 Agent 会话",
                )
                .theme(self.theme)
                .render(cx),
            )
            .child(self.route_button(
                "nav-calendar",
                Icon::Calendar,
                "日程与定时任务",
                Route::Agent(Agent::Calendar),
                cx,
            ))
            .child(div().flex_1().min_h_0())
            .child(separator("nav-common-divider"))
            .child(self.route_button(
                "nav-plugins",
                Icon::Script,
                "插件管理",
                Route::Api(Api::Plugins),
                cx,
            ))
            .child(self.route_button(
                "nav-mcp",
                Icon::Workflow,
                "MCP 调用历史",
                Route::Api(Api::Mcp),
                cx,
            ))
            .child(self.route_button("nav-vault", Icon::Key, "密码箱", Route::Api(Api::Vault), cx))
            .child(self.route_button(
                "nav-settings",
                Icon::Settings,
                "Agent 设置",
                Route::Agent(Agent::Settings),
                cx,
            ))
            .child(
                Tooltip::new(
                    IconButton::new(Icon::HalfCircle, "切换明暗主题")
                        .theme(self.theme)
                        .render()
                        .id("nav-theme")
                        .debug_selector(|| "nav-theme".into())
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(Action::ToggleTheme))),
                    "切换明暗主题",
                )
                .theme(self.theme)
                .render(cx),
            )
    }
}
