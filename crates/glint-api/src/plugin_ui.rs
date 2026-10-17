use super::*;
use gpui_glint::components::{CountBadge, CountBadgeKind, TagBadge, TagBadgeKind};

fn metadata_badge(label: impl Into<gpui::SharedString>, theme: ThemePreset) -> gpui::Div {
    CountBadge::new(label)
        .kind(CountBadgeKind::Muted)
        .theme(theme)
        .render()
}

fn origin_label(plugin: &gpui_glint::plugins::Plugin) -> &'static str {
    if plugin.origin.is_some() {
        "远程"
    } else if plugins::is_bundled(plugin) {
        "内置"
    } else {
        "本地"
    }
}

fn action_badge(label: &'static str, enabled: bool, theme: ThemePreset) -> gpui::Div {
    if enabled {
        TagBadge::new(label)
            .kind(TagBadgeKind::RealData)
            .theme(theme)
            .render()
    } else {
        metadata_badge(label, theme)
    }
}

#[derive(Clone)]
struct ActionDrag {
    owner: gpui::EntityId,
    request: usize,
    id: String,
    action_id: String,
    title: String,
    theme: ThemePreset,
}
impl gpui::Render for ActionDrag {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(self.theme.palette().panel))
            .shadow_md()
            .child(self.title.clone())
    }
}

impl Workbench {
    fn before_action_bindings(&self) -> Vec<model::BeforeAction> {
        let mut actions = self
            .requests
            .get(self.active)
            .map(|r| r.draft.plugins.clone())
            .unwrap_or_default();
        for plugin in &self.plugin_catalog {
            let family = plugin.family_identity();
            let chosen = actions.iter().find(|a| {
                self.plugin_catalog
                    .iter()
                    .any(|p| p.identity() == a.id && p.family_identity() == family)
            });
            if chosen.is_some_and(|a| a.id != plugin.identity()) {
                continue;
            }
            if chosen.is_none()
                && self.plugin_catalog.iter().any(|p| {
                    p.family_identity() == family
                        && semver::Version::parse(&p.manifest.version).ok()
                            > semver::Version::parse(&plugin.manifest.version).ok()
                })
            {
                continue;
            }
            let ids: Vec<_> = if plugin.manifest.contributions.is_empty() {
                vec![String::new()]
            } else {
                plugin
                    .manifest
                    .contributions
                    .iter()
                    .filter(|c| c.kind == plugins::HOOK)
                    .map(|c| c.id.clone())
                    .collect()
            };
            for (index, action_id) in ids.into_iter().enumerate() {
                if let Some(binding) = actions.iter_mut().find(|a| {
                    a.id == plugin.identity()
                        && (a.action_id == action_id || (index == 0 && a.action_id.is_empty()))
                }) {
                    binding.action_id = action_id;
                } else {
                    actions.push(model::BeforeAction {
                        approval: gpui_glint::plugins::Approval::for_plugin(plugin),
                        action_id,
                        enabled: false,
                    });
                }
            }
        }
        actions
    }
    fn reorder_before_action(
        &mut self,
        drag: &ActionDrag,
        target: &(String, String),
        after: bool,
        cx: &mut Context<Self>,
    ) {
        if drag.request != self.active
            || drag.owner != cx.entity_id()
            || (&drag.id, &drag.action_id) == (&target.0, &target.1)
        {
            return;
        }
        let mut actions = self.before_action_bindings();
        let Some(from) = actions
            .iter()
            .position(|a| a.id == drag.id && a.action_id == drag.action_id)
        else {
            return;
        };
        if !actions
            .iter()
            .any(|a| a.id == target.0 && a.action_id == target.1)
        {
            return;
        }
        let action = actions.remove(from);
        let to = actions
            .iter()
            .position(|a| a.id == target.0 && a.action_id == target.1)
            .unwrap();
        actions.insert(to + usize::from(after), action);
        let Some(request) = self.requests.get_mut(self.active) else {
            return;
        };
        request.draft.plugins = actions;
        self.storage_dirty = true;
        self.persist(cx);
        cx.notify();
    }
    pub(super) fn plugin_configuration(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let selected = self
            .requests
            .get(self.active)
            .map(|r| r.draft.plugins.as_slice())
            .unwrap_or(&[]);
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .child("发送前动作"),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("按启用顺序执行；动作失败或超时会停止发送。"),
            );
        let mut cards = Vec::new();
        for plugin in &self.plugin_catalog {
            let id = plugin.identity();
            let family = plugin.family_identity();
            // A request selects one installed version per provider, without showing version controls here.
            let chosen = selected.iter().find(|a| {
                self.plugin_catalog
                    .iter()
                    .any(|p| p.identity() == a.id && p.family_identity() == family)
            });
            if chosen.is_some_and(|a| a.id != id) {
                continue;
            }
            if chosen.is_none()
                && self.plugin_catalog.iter().any(|p| {
                    p.family_identity() == family
                        && semver::Version::parse(&p.manifest.version).ok()
                            > semver::Version::parse(&plugin.manifest.version).ok()
                })
            {
                continue;
            }
            let actions: Vec<_> = if plugin.manifest.contributions.is_empty() {
                vec![(String::new(), plugin.manifest.name.clone())]
            } else {
                plugin
                    .manifest
                    .contributions
                    .iter()
                    .filter(|c| c.kind == plugins::HOOK)
                    .map(|c| (c.id.clone(), c.title.clone()))
                    .collect()
            };
            for (action_index, (action_id, title)) in actions.into_iter().enumerate() {
                let binding = selected.iter().find(|a| {
                    a.id == id
                        && (a.action_id == action_id
                            || (action_index == 0 && a.action_id.is_empty()))
                });
                let changed = binding.is_some_and(|a| !a.matches(plugin));
                let enabled = binding.is_some_and(|a| a.enabled) && !changed;
                let approval = gpui_glint::plugins::Approval::for_plugin(plugin);
                let id = id.clone();
                let action_id = action_id.clone();
                let drag = ActionDrag {
                    owner: cx.entity_id(),
                    request: self.active,
                    id: id.clone(),
                    action_id: action_id.clone(),
                    title: title.clone(),
                    theme: self.theme,
                };
                let target = (id.clone(), action_id.clone());
                let drop_owner = cx.entity().downgrade();
                let scope = cx.entity_id();
                let request_index = self.active;
                let bounds =
                    std::rc::Rc::new(std::cell::Cell::new(gpui::Bounds::<gpui::Pixels>::default()));
                let drop_bounds = bounds.clone();
                let owner = cx.entity().downgrade();
                let environment_names = plugin
                    .manifest
                    .capabilities
                    .iter()
                    .filter_map(|c| c.strip_prefix("environment.read."))
                    .collect::<Vec<_>>()
                    .join("、");
                let selector = if action_index == 0 {
                    format!("plugin-toggle-{}", plugin.identity())
                } else {
                    format!("action-toggle-{}-{action_id}", plugin.identity())
                };
                let sort_action_id = action_id.clone();
                let card_id = format!("before-action-{}-{action_id}", plugin.identity());
                let switch = Switch::new("")
                    .theme(self.theme)
                    .checked(enabled)
                    .on_change(move |checked, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            let bindings = this.before_action_bindings();
                            if let Some(request) = this.requests.get_mut(this.active) {
                                request.draft.plugins = bindings;
                                if let Some(binding) = request.draft.plugins.iter_mut().find(|a| {
                                    a.id == id
                                        && (a.action_id == action_id
                                            || (action_index == 0 && a.action_id.is_empty()))
                                }) {
                                    binding.enabled = checked;
                                    binding.action_id = action_id.clone();
                                    if checked {
                                        binding.approval = approval.clone();
                                    }
                                } else if checked {
                                    request.draft.plugins.push(model::BeforeAction {
                                        approval: approval.clone(),
                                        action_id: action_id.clone(),
                                        enabled: true,
                                    });
                                }
                                this.storage_dirty = true;
                                cx.notify();
                            }
                        });
                    })
                    .render()
                    .debug_selector(|| selector);
                let handle = div()
                    .id(gpui::SharedString::from(format!(
                        "action-grip-{}-{}",
                        plugin.identity(),
                        drag.action_id
                    )))
                    .debug_selector(|| {
                        format!("action-grip-{}-{}", plugin.identity(), drag.action_id)
                    })
                    .cursor(gpui::CursorStyle::OpenHand)
                    .text_color(rgb(p.text_muted))
                    .p_1()
                    .child(Icon::Grip.render(16.))
                    .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()));
                let mut card = div()
                    .id(gpui::SharedString::from(card_id.clone()))
                    .relative()
                    .debug_selector(|| card_id.clone())
                    .drag_over::<ActionDrag>(move |style, drag, _, _| {
                        if drag.owner == scope && drag.request == request_index {
                            style.bg(rgb(p.accent_tint))
                        } else {
                            style
                        }
                    })
                    .child(
                        gpui::canvas(
                            move |rect, _, _| {
                                bounds.set(rect);
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    )
                    .on_drop(move |drag: &ActionDrag, window, cx| {
                        if drag.owner == scope && drag.request == request_index {
                            let after = window.mouse_position().y > drop_bounds.get().center().y;
                            let _ = drop_owner.update(cx, |this, cx| {
                                this.reorder_before_action(drag, &target, after, cx)
                            });
                        }
                    })
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(rgb(p.border))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(handle)
                            .child(div().flex_1().min_w_0().child(title))
                            .child(action_badge(
                                if changed {
                                    "需重新启用"
                                } else if enabled {
                                    "已启用"
                                } else {
                                    "未启用"
                                },
                                enabled,
                                self.theme,
                            ))
                            .child(switch),
                    );
                if !environment_names.is_empty() {
                    card = card.child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(format!("所需环境变量：{environment_names}")),
                    );
                }
                let order = selected
                    .iter()
                    .position(|a| {
                        a.id == plugin.identity()
                            && (a.action_id == sort_action_id
                                || (action_index == 0 && a.action_id.is_empty()))
                    })
                    .unwrap_or(usize::MAX);
                cards.push((order, card));
            }
        }
        cards.sort_by_key(|(order, _)| *order);
        panel = panel.children(cards.into_iter().map(|(_, card)| card));
        for binding in selected
            .iter()
            .filter(|a| !self.plugin_catalog.iter().any(|p| p.identity() == a.id))
        {
            let id = binding.id.clone();
            let action_id = binding.action_id.clone();
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child("动作不可用"))
                    .child(metadata_badge("不可用", self.theme))
                    .child(
                        Button::new("移除")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(r) = this.requests.get_mut(this.active) {
                                    r.draft
                                        .plugins
                                        .retain(|a| a.id != id || a.action_id != action_id);
                                    this.storage_dirty = true;
                                    cx.notify();
                                }
                            })),
                    ),
            );
        }
        panel
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn enabled_plugins_persist_per_request_and_restore_from_versions(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.config = 5;
            host.plugin_catalog.truncate(1);
            host
        });
        let bounds = window.debug_bounds("plugin-toggle-com.wps.kso1").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert_eq!(
                host.snapshot(cx)
                    .plugins
                    .iter()
                    .map(|a| a.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["com.wps.kso1"]
            );
            host.save(cx);
            host.add_request(cx);
            assert!(host.snapshot(cx).plugins.is_empty());
            host.choose(0, cx);
            assert_eq!(
                host.snapshot(cx)
                    .plugins
                    .iter()
                    .map(|a| a.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["com.wps.kso1"]
            );
            let baseline = Draft::default();
            host.load(&baseline, cx);
            assert!(host.snapshot(cx).plugins.is_empty());
            let saved = host.requests[0].versions[0].draft.clone();
            host.load(&saved, cx);
            host.persist(cx);
        });
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(window, |host, cx| {
            assert_eq!(
                host.snapshot(cx)
                    .plugins
                    .iter()
                    .map(|a| a.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["com.wps.kso1"]
            )
        });
        let mut legacy = serde_json::to_value(Draft::default()).unwrap();
        legacy.as_object_mut().unwrap().remove("plugins");
        assert!(
            serde_json::from_value::<Draft>(legacy)
                .unwrap()
                .plugins
                .is_empty()
        );
    }
}

#[cfg(test)]
mod action_tests {
    use super::*;
    #[gpui::test]
    fn registered_actions_have_independent_switches_and_retain_disabled_bindings(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.config = 5;
            let mut plugin = plugins::catalog().0.remove(0);
            plugin.manifest.capabilities.clear();
            plugin
                .manifest
                .contributions
                .push(gpui_glint::plugins::Contribution {
                    id: "second".into(),
                    kind: plugins::HOOK.into(),
                    title: "第二个动作".into(),
                    hook: plugins::HOOK.into(),
                    data: None,
                });
            host.plugin_catalog = vec![plugin];
            host
        });
        for selector in [
            "plugin-toggle-com.wps.kso1",
            "action-toggle-com.wps.kso1-second",
            "plugin-toggle-com.wps.kso1",
        ] {
            let bounds = window.debug_bounds(selector).unwrap();
            window.simulate_click(bounds.center(), gpui::Modifiers::default());
        }
        let source = window
            .debug_bounds("action-grip-com.wps.kso1-second")
            .unwrap()
            .center();
        let target = window
            .debug_bounds("before-action-com.wps.kso1-sign-request")
            .unwrap();
        let destination = gpui::point(target.center().x, target.top() + px(3.));
        window.simulate_event(gpui::MouseDownEvent {
            position: source,
            button: gpui::MouseButton::Left,
            click_count: 1,
            ..Default::default()
        });
        for _ in 0..2 {
            window.simulate_event(gpui::MouseMoveEvent {
                position: destination,
                pressed_button: Some(gpui::MouseButton::Left),
                ..Default::default()
            });
        }
        window.simulate_event(gpui::MouseUpEvent {
            position: destination,
            button: gpui::MouseButton::Left,
            click_count: 1,
            ..Default::default()
        });
        assert!(
            window
                .debug_bounds("before-action-com.wps.kso1-second")
                .unwrap()
                .top()
                < window
                    .debug_bounds("before-action-com.wps.kso1-sign-request")
                    .unwrap()
                    .top()
        );
        host.update(window, |host, cx| {
            let actions = host.snapshot(cx).plugins;
            assert_eq!(actions.len(), 2);
            assert!(actions[0].enabled);
            assert!(!actions[1].enabled);
            assert_eq!(actions[0].action_id, "second");
            host.persist(cx);
        });
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(window, |host, cx| {
            let actions = host.snapshot(cx).plugins;
            assert_eq!(actions.len(), 2);
            assert!(actions[0].enabled);
            assert!(!actions[1].enabled);
        });
    }
}

#[cfg(test)]
mod remote_tests {
    use super::*;
    #[gpui::test]
    fn remote_actions_choose_latest_and_preserve_source_binding(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.config = 5;
            let mut first = plugins::catalog().0.remove(0);
            first.origin = Some(gpui_glint::plugins::Origin {
                source_id: "a".repeat(64),
                package_sha256: "b".repeat(64),
            });
            let mut next = first.clone();
            next.manifest.version = "1.1.0".into();
            next.origin.as_mut().unwrap().package_sha256 = "c".repeat(64);
            let mut other = first.clone();
            other.origin.as_mut().unwrap().source_id = "d".repeat(64);
            host.plugin_catalog = vec![first, next, other];
            host
        });
        for (source, version) in [("a", "1.1.0"), ("d", "1.0.0")] {
            let selector = format!(
                "plugin-toggle-{}::com.wps.kso1::{version}",
                source.repeat(64)
            );
            let identity = format!("{}::com.wps.kso1::{version}", source.repeat(64));
            host.update(window, |host, cx| {
                host.plugin_catalog
                    .sort_by_key(|p| p.identity() != identity);
                cx.notify();
            });
            // The native test selector API requires a process-lifetime string.
            let selector: &'static str = Box::leak(selector.into_boxed_str());
            let bounds = window.debug_bounds(selector).unwrap();
            window.simulate_click(bounds.center(), gpui::Modifiers::default());
        }
        host.update(window, |host, cx| {
            let approvals = host.snapshot(cx).plugins;
            assert_eq!(approvals.len(), 2);
            assert!(approvals.iter().any(|a| a.id.starts_with(&"d".repeat(64))));
            assert!(approvals.iter().any(|a| a.id.ends_with("::1.1.0")));
        });
    }
}

impl Workbench {
    pub(super) fn open_plugin_manager(&mut self, cx: &mut Context<Self>) {
        self.show_mcp = false;
        self.show_plugins = true;
        self.show_environments = false;
        self.show_collection_menu = false;
        self.tree_menu = None;
        (self.plugin_catalog, self.plugin_errors) = plugins::catalog();
        cx.notify();
    }
    pub(super) fn plugin_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let mut sidebar = div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .bg(rgb(p.panel))
            .child(div().font_weight(FontWeight::MEDIUM).child("插件管理"));
        for (index, label) in ["已安装插件", "插件源"].into_iter().enumerate() {
            sidebar = sidebar.child(
                Button::new(label)
                    .kind(if self.plugin_manager_tab == index {
                        ButtonKind::Tinted
                    } else {
                        ButtonKind::Plain
                    })
                    .theme(self.theme)
                    .render()
                    .debug_selector(move || format!("plugin-manager-section-{index}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.plugin_manager_tab = index;
                        cx.notify();
                    })),
            );
        }
        sidebar.child(
            div()
                .mt_3()
                .text_size(px(10.))
                .text_color(rgb(p.text_muted))
                .child("插件属于应用资源；请求仅配置前置动作。"),
        )
    }
    pub(super) fn plugin_workspace(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let mut body = div().w_full().flex().flex_col().gap_3();
        if self.plugin_manager_tab == 1 {
            body = body.child(self.registry_configuration(cx));
        } else {
            body = body
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(format!("已安装 · {} 个版本", self.plugin_catalog.len()))
                        .child(
                            Button::new("刷新已安装插件")
                                .kind(ButtonKind::Plain)
                                .size(ButtonSize::Small)
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    (this.plugin_catalog, this.plugin_errors) = plugins::catalog();
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child(
                            "请求动作需在“前置动作”中启用；授权预设在“身份验证 → OAuth 2.0”中选择。",
                        ),
                );
            for plugin in &self.plugin_catalog {
                let id = plugin.identity();
                let uses = self
                    .requests
                    .iter()
                    .filter(|r| r.draft.plugins.iter().any(|a| a.id == id))
                    .count();
                let mut card = div()
                    .id(gpui::SharedString::from(format!("installed-{id}")))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(rgb(p.border))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(plugin.manifest.name.clone())
                            .child(metadata_badge(
                                format!("v{}", plugin.manifest.version),
                                self.theme,
                            ))
                            .child(metadata_badge(origin_label(plugin), self.theme))
                            .when(
                                plugin.manifest.contributions.is_empty()
                                    || plugin
                                        .manifest
                                        .contributions
                                        .iter()
                                        .any(|c| c.kind == plugins::HOOK),
                                |row| {
                                    row.child(metadata_badge(
                                        format!("绑定 {uses} 个请求"),
                                        self.theme,
                                    ))
                                },
                            )
                            .when(
                                plugin
                                    .manifest
                                    .contributions
                                    .iter()
                                    .any(|c| c.kind == plugins::OAUTH_PRESET),
                                |row| row.child(metadata_badge("授权预设", self.theme)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(plugin.manifest.id.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(format!(
                                "扩展点：{}",
                                if plugin.manifest.contributions.is_empty() {
                                    plugin.manifest.hooks.iter().cloned().collect::<Vec<_>>()
                                } else {
                                    plugin
                                        .manifest
                                        .contributions
                                        .iter()
                                        .map(|c| c.kind.clone())
                                        .collect::<std::collections::BTreeSet<_>>()
                                        .into_iter()
                                        .collect()
                                }
                                .join(" · ")
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(format!(
                                "权限：{}",
                                if plugin.manifest.capabilities.is_empty() {
                                    "无".to_owned()
                                } else {
                                    plugin
                                        .manifest
                                        .capabilities
                                        .iter()
                                        .cloned()
                                        .collect::<Vec<_>>()
                                        .join(" · ")
                                }
                            )),
                    );
                if let Some(origin) = &plugin.origin {
                    let origin = origin.clone();
                    let plugin_id = plugin.manifest.id.clone();
                    let version = plugin.manifest.version.clone();
                    card = card.child(
                        Button::new("卸载此版本")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let origin = origin.clone();
                                let plugin_id = plugin_id.clone();
                                let version = version.clone();
                                this.registry_operation(
                                    move || {
                                        let store = plugins::registry_store()?;
                                        let record = store
                                            .installed()?
                                            .into_iter()
                                            .find(|r| {
                                                r.source.id().ok().as_ref()
                                                    == Some(&origin.source_id)
                                                    && r.release.sha256 == origin.package_sha256
                                            })
                                            .ok_or("找不到已安装版本")?;
                                        store.uninstall(
                                            &record.source.name,
                                            &plugin_id,
                                            &version,
                                        )?;
                                        Ok(vec![])
                                    },
                                    true,
                                    cx,
                                );
                            })),
                    );
                } else {
                    card = card.child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(if plugins::is_bundled(plugin) {
                                "内置插件"
                            } else {
                                "本地开发插件 · 通过插件目录管理"
                            }),
                    );
                }
                body = body.child(card);
            }
            if let Ok(path) = plugins::directory() {
                body = body.child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(p.text_muted))
                        .child(format!("本地开发目录：{}", path.display())),
                );
            }
            for error in &self.plugin_errors {
                body = body.child(error.clone());
            }
            if let Some(message) = &self.registry_message {
                body = body.child(div().text_size(px(11.)).child(message.clone()));
            }
        }
        div()
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(rgb(p.surface))
            .child(
                div()
                    .h(px(52.))
                    .flex_shrink_0()
                    .px_5()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::MEDIUM)
                            .child("插件管理"),
                    )
                    .child(
                        Button::new("返回请求")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "close-plugin-manager".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_plugins = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("plugin-manager-scroll")
                    .debug_selector(|| "plugin-manager-scroll".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_5()
                    .child(body),
            )
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    #[gpui::test]
    fn plugin_management_is_global_and_does_not_require_a_request(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            Workbench::with_store(storage::Store::open(&path).unwrap(), cx)
        });
        assert!(host.read_with(window, |host, _| host.requests.is_empty()));
        let button = window.debug_bounds("open-plugin-manager").unwrap();
        window.simulate_click(button.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert!(host.show_plugins && !host.show_environments)
        });
        assert!(window.debug_bounds("plugin-manager-scroll").is_some());
        let sources = window.debug_bounds("plugin-manager-section-1").unwrap();
        window.simulate_click(sources.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert_eq!(host.plugin_manager_tab, 1);
            assert!(host.snapshot(cx).plugins.is_empty());
        });
        let back = window.debug_bounds("close-plugin-manager").unwrap();
        window.simulate_click(back.center(), gpui::Modifiers::default());
        host.update(window, |host, _| assert!(!host.show_plugins));
    }
}
