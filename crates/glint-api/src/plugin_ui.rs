use super::*;
use gpui_glint::components::{CountBadge, CountBadgeKind, TagBadge, TagBadgeKind};

pub(super) type EnvironmentSettingsCache =
    std::cell::RefCell<HashMap<(String, String, String), EnvironmentSettings>>;

pub(super) struct EnvironmentSettings {
    panel: Entity<gpui_glint::components::CollapsePanel>,
    fields: Vec<(String, Entity<TextEntry>)>,
    saved: std::collections::BTreeMap<String, String>,
    _subscriptions: Vec<Subscription>,
}

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
    fn action_environment_settings(
        &self,
        plugin: &gpui_glint::plugins::Plugin,
        binding: &model::BeforeAction,
        first_action: bool,
        cx: &mut Context<Self>,
    ) -> Entity<gpui_glint::components::CollapsePanel> {
        let request_id = self.workspace.requests[self.active].id.clone();
        let key = (
            request_id.clone(),
            plugin.identity(),
            binding.action_id.clone(),
        );
        let mut cache = self.plugin_environment_settings.borrow_mut();
        // Remove panels for deleted requests while keeping independent state per request/action.
        cache.retain(|(request, _, _), _| self.workspace.requests.iter().any(|r| &r.id == request));
        let settings = cache.entry(key).or_insert_with(|| {
            let mut subscriptions = Vec::new();
            let fields: Vec<_> = plugin
                .manifest
                .capabilities
                .iter()
                .filter_map(|capability| capability.strip_prefix("environment.read."))
                .map(|name| {
                    let entry = Self::entry(
                        binding
                            .environment_bindings
                            .get(name)
                            .map(String::as_str)
                            .unwrap_or(""),
                        name,
                        cx,
                    );
                    let request_id = request_id.clone();
                    let id = plugin.identity();
                    let approval = gpui_glint::plugins::Approval::for_plugin(plugin);
                    let action_id = binding.action_id.clone();
                    let name_owned = name.to_owned();
                    subscriptions.push(cx.observe(&entry, move |this, entry, cx| {
                        let value = entry.read(cx).value().trim().to_owned();
                        let Some(request) = this
                            .workspace
                            .requests
                            .iter_mut()
                            .find(|r| r.id == request_id)
                        else {
                            return;
                        };
                        let position = request.draft.plugins.iter().position(|a| {
                            a.id == id
                                && (a.action_id == action_id
                                    || (first_action && a.action_id.is_empty()))
                        });
                        let action = if let Some(position) = position {
                            &mut request.draft.plugins[position]
                        } else {
                            if value.is_empty() {
                                return;
                            }
                            request.draft.plugins.push(model::BeforeAction {
                                approval: approval.clone(),
                                action_id: action_id.clone(),
                                enabled: false,
                                ..Default::default()
                            });
                            request.draft.plugins.last_mut().unwrap()
                        };
                        if action
                            .environment_bindings
                            .get(&name_owned)
                            .map(String::as_str)
                            .unwrap_or("")
                            == value
                        {
                            return;
                        }
                        if value.is_empty() {
                            action.environment_bindings.remove(&name_owned);
                        } else {
                            action
                                .environment_bindings
                                .insert(name_owned.clone(), value);
                        }
                        this.workspace.storage_dirty = true;
                        cx.notify();
                    }));
                    (name.to_owned(), entry)
                })
                .collect();
            let content_fields = fields.clone();
            EnvironmentSettings {
                panel: cx.new(|_| {
                    gpui_glint::components::CollapsePanel::new("更多设置")
                        .compact()
                        .theme(self.theme)
                        .content(move |_, _| {
                            div()
                                .w_full()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(div().text_size(px(11.)).child(
                                    "填写优先读取的环境变量名；留空或名称不存在时回退到默认变量。",
                                ))
                                .children(content_fields.iter().map(|(name, entry)| {
                                    div()
                                        .w_full()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .child(format!("{name} · 优先读取")),
                                        )
                                        .child(entry.clone())
                                }))
                                .into_any_element()
                        })
                }),
                fields,
                saved: binding.environment_bindings.clone(),
                _subscriptions: subscriptions,
            }
        });
        // Restore versions/drafts into existing editors without rebuilding focus or collapse state.
        let changed = settings.saved != binding.environment_bindings;
        for (name, entry) in &settings.fields {
            entry.update(cx, |entry, cx| {
                entry.set_theme(self.theme, cx);
                let value = binding
                    .environment_bindings
                    .get(name)
                    .map(String::as_str)
                    .unwrap_or("");
                if changed && entry.value().trim() != value {
                    entry.set_value(value, cx);
                }
            });
        }
        settings.saved = binding.environment_bindings.clone();
        settings
            .panel
            .update(cx, |panel, cx| panel.set_theme(self.theme, cx));
        settings.panel.clone()
    }

    pub(super) fn plugin_enabled(&self, plugin: &gpui_glint::plugins::Plugin) -> bool {
        !self.disabled_plugins.contains(&plugin.family_identity())
    }
    pub(super) fn enabled_plugin_catalog(&self) -> Vec<gpui_glint::plugins::Plugin> {
        self.plugin_catalog
            .iter()
            .filter(|p| self.plugin_enabled(p))
            .cloned()
            .collect()
    }
    pub(super) fn plugin_identity_disabled(&self, identity: &str) -> bool {
        self.disabled_plugins.contains(identity)
            || identity
                .rsplit_once("::")
                .is_some_and(|(family, _)| self.disabled_plugins.contains(family))
    }
    /// Keep bindings in drafts so re-enabling restores them; skip disabled providers at execution.
    pub(super) fn enabled_before_actions(
        &self,
        actions: &[model::BeforeAction],
    ) -> Vec<model::BeforeAction> {
        actions
            .iter()
            .filter(|binding| !self.plugin_identity_disabled(&binding.id))
            .cloned()
            .collect()
    }
    pub(super) fn set_plugin_enabled(
        &mut self,
        identity: &str,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let plugin = self
            .plugin_catalog
            .iter()
            .find(|p| p.identity() == identity)
            .ok_or("插件不存在，请刷新列表")?;
        let family = plugin.family_identity();
        let name = plugin.manifest.name.clone();
        self.workspace
            .storage
            .set_plugin_enabled(&family, enabled)?;
        if enabled {
            self.disabled_plugins.remove(&family);
        } else {
            self.disabled_plugins.insert(family);
        }
        self.ws_refresh_decoder_select(cx);
        self.ws_invalidate_titles();
        self.ws_select_sync(cx);
        self.notice = format!("{name}已{}", if enabled { "启用" } else { "禁用" });
        cx.notify();
        Ok(())
    }

    fn before_action_bindings(&self) -> Vec<model::BeforeAction> {
        let mut actions = self
            .workspace
            .requests
            .get(self.active)
            .map(|r| r.draft.plugins.clone())
            .unwrap_or_default();
        for plugin in self
            .plugin_catalog
            .iter()
            .filter(|p| self.plugin_enabled(p))
        {
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
                        environment_bindings: Default::default(),
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
        let Some(request) = self.workspace.requests.get_mut(self.active) else {
            return;
        };
        request.draft.plugins = actions;
        self.workspace.storage_dirty = true;
        self.persist(cx);
        cx.notify();
    }
    pub(super) fn plugin_configuration(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let selected = self
            .workspace
            .requests
            .get(self.active)
            .map(|r| r.draft.plugins.as_slice())
            .unwrap_or(&[]);
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_size(px(12.)).child("规则触发器"))
            .child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child("条件与动作独立配置，按行顺序执行。变量缺失、为空或过期时触发；动作失败停止发送。目标支持普通 HTTP 请求。"))
            .child(self.triggers_table.clone())
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
                    .child("按启用顺序执行；全局禁用的插件不执行，动作失败或超时会停止发送。"),
            );
        let mut cards = Vec::new();
        for plugin in self
            .plugin_catalog
            .iter()
            .filter(|p| self.plugin_enabled(p))
        {
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
                let environment_settings = if environment_names.is_empty() {
                    None
                } else {
                    let mut settings_binding =
                        binding.cloned().unwrap_or_else(|| model::BeforeAction {
                            approval: approval.clone(),
                            enabled: false,
                            ..Default::default()
                        });
                    settings_binding.action_id = action_id.clone();
                    Some(self.action_environment_settings(
                        plugin,
                        &settings_binding,
                        action_index == 0,
                        cx,
                    ))
                };
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
                            if let Some(request) = this.workspace.requests.get_mut(this.active) {
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
                                        environment_bindings: Default::default(),
                                        approval: approval.clone(),
                                        action_id: action_id.clone(),
                                        enabled: true,
                                    });
                                }
                                this.workspace.storage_dirty = true;
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
                if let Some(settings) = environment_settings {
                    card = card.child(settings);
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
                                if let Some(r) = this.workspace.requests.get_mut(this.active) {
                                    r.draft
                                        .plugins
                                        .retain(|a| a.id != id || a.action_id != action_id);
                                    this.workspace.storage_dirty = true;
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
    fn environment_settings_start_collapsed_and_persist_with_the_request(
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
        let (panel, field) = host.read_with(window, |host, cx| {
            let settings = host.plugin_environment_settings.borrow();
            let settings = settings.values().next().unwrap();
            assert!(!settings.panel.read(cx).is_expanded());
            (
                settings.panel.clone(),
                settings
                    .fields
                    .iter()
                    .find(|(name, _)| name == "wps_app_id")
                    .unwrap()
                    .1
                    .clone(),
            )
        });
        panel.update(window, |panel, cx| panel.set_expanded(true, cx));
        field.update(window, |field, cx| field.set_value("wps_app_id1", cx));
        window.run_until_parked();
        host.read_with(window, |host, _| {
            let action = &host.workspace.requests[host.active].draft.plugins[0];
            assert_eq!(action.environment_bindings["wps_app_id"], "wps_app_id1");
            assert!(!action.enabled);
        });
        let bounds = window.debug_bounds("plugin-toggle-com.wps.kso1").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert_eq!(
                host.snapshot(cx).plugins[0].environment_bindings["wps_app_id"],
                "wps_app_id1"
            );
            host.save(cx);
            host.add_request(cx);
            assert!(host.snapshot(cx).plugins.is_empty());
            host.choose(0, cx);
            host.persist(cx);
        });
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.read_with(window, |host, cx| {
            assert_eq!(
                host.snapshot(cx).plugins[0].environment_bindings["wps_app_id"],
                "wps_app_id1"
            );
        });
    }

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
            let saved = host.workspace.requests[0].versions[0].draft.clone();
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
        self.page = crate::Page::Plugins;
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
                .child("插件开关对所有工作区生效；禁用后保留绑定配置。"),
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
                            "请求动作在“前置动作”中启用；事件解密在“WebSocket 消息 → 解密后”中选择；授权预设在 OAuth 2.0 中选择。",
                        ),
                );
            for plugin in &self.plugin_catalog {
                let id = plugin.identity();
                let enabled = self.plugin_enabled(plugin);
                let toggle_id = id.clone();
                let toggle_owner = cx.entity().downgrade();
                let uses = self
                    .workspace
                    .requests
                    .iter()
                    .filter(|r| {
                        r.draft.plugins.iter().any(|a| a.id == id)
                            || r.draft.event_decoder.as_ref().is_some_and(|a| a.id == id)
                    })
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
                                    || plugin.manifest.contributions.iter().any(|c| {
                                        c.kind == plugins::HOOK || c.kind == plugins::EVENT_DECODE
                                    }),
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
                                    .any(|c| c.kind == plugins::EVENT_DECODE),
                                |row| row.child(metadata_badge("事件解密", self.theme)),
                            )
                            .when(
                                plugin
                                    .manifest
                                    .contributions
                                    .iter()
                                    .any(|c| c.kind == plugins::OAUTH_PRESET),
                                |row| row.child(metadata_badge("授权预设", self.theme)),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.text_muted))
                                    .child(if enabled { "已启用" } else { "已禁用" }),
                            )
                            .child(
                                Switch::new("")
                                    .checked(enabled)
                                    .theme(self.theme)
                                    .on_change(move |enabled, _, cx| {
                                        let _ = toggle_owner.update(cx, |this, cx| {
                                            if let Err(error) =
                                                this.set_plugin_enabled(&toggle_id, enabled, cx)
                                            {
                                                this.notice = error.clone();
                                                this.plugin_errors.push(error);
                                                cx.notify();
                                            }
                                        });
                                    })
                                    .render()
                                    .id(format!("plugin-enabled-{id}"))
                                    .debug_selector({
                                        let id = id.clone();
                                        move || format!("plugin-enabled-{id}")
                                    }),
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
                                if this.page == crate::Page::Plugins {
                                    this.page = crate::Page::Requests;
                                }
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
        assert!(host.read_with(window, |host, _| host.workspace.requests.is_empty()));
        let button = window.debug_bounds("open-plugin-manager").unwrap();
        window.simulate_click(button.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert!(host.page == crate::Page::Plugins && host.page != crate::Page::Environments)
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
        host.update(window, |host, _| assert!(host.page != crate::Page::Plugins));
    }
}

#[cfg(test)]
mod enable_tests {
    use super::*;
    #[gpui::test]
    fn builtin_plugin_switch_persists_and_skips_bound_actions_until_enabled(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("enabled.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name.update(cx, |e, cx| e.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            let plugin = host
                .plugin_catalog
                .iter()
                .find(|p| p.manifest.id == "com.wps.kso1")
                .unwrap();
            host.workspace.requests[0].draft.plugins = vec![model::BeforeAction {
                environment_bindings: Default::default(),
                approval: gpui_glint::plugins::Approval::for_plugin(plugin),
                action_id: "sign-request".into(),
                enabled: true,
            }];
            host.workspace.storage_dirty = true;
            host.persist(cx);
            host.open_plugin_manager(cx);
            host
        });
        let bounds = window.debug_bounds("plugin-enabled-com.wps.kso1").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            let actions = host.enabled_before_actions(&host.snapshot(cx).plugins);
            assert!(actions.is_empty());
            assert_eq!(host.workspace.requests[0].draft.plugins.len(), 1);
            assert!(
                plugins::validate_variables(
                    &actions,
                    &host.plugin_catalog,
                    &environment::EnvironmentValues(Default::default())
                )
                .is_ok()
            );
            let prepared = PreparedRequest::new(
                &Draft {
                    url: "https://example.test".into(),
                    ..Default::default()
                },
                &environment::EnvironmentValues(Default::default()),
            )
            .unwrap();
            assert!(
                plugins::apply(
                    prepared,
                    &actions,
                    &host.plugin_catalog,
                    &environment::EnvironmentValues(Default::default())
                )
                .is_ok()
            );
            host.open_plugin_manager(cx);
            assert!(host.disabled_plugins.contains("com.wps.kso1"));
        });
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(window, |host, cx| {
            assert!(host.disabled_plugins.contains("com.wps.kso1"));
            assert_eq!(host.workspace.requests[0].draft.plugins.len(), 1);
            host.set_plugin_enabled("com.wps.kso1", true, cx).unwrap();
            assert_eq!(
                host.enabled_before_actions(&host.snapshot(cx).plugins)
                    .len(),
                1
            );
            assert!(
                !host
                    .workspace
                    .storage
                    .disabled_plugins()
                    .unwrap()
                    .contains("com.wps.kso1")
            );
        });
    }
    #[gpui::test]
    fn disabled_presets_and_decoder_are_unavailable_and_save_failure_is_atomic(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            Workbench::with_store(
                storage::Store::open(&dir.path().join("plugins.sqlite3")).unwrap(),
                cx,
            )
        });
        host.update(window,|host,cx| {
            let preset_count=plugins::oauth_presets(&host.enabled_plugin_catalog()).len();
            host.set_plugin_enabled("com.wps.oauth",false,cx).unwrap();
            assert_eq!(plugins::oauth_presets(&host.enabled_plugin_catalog()).len(),preset_count-1);
            assert!(plugins::oauth_presets(&host.enabled_plugin_catalog()).iter().all(|p|!p.title.contains("WPS")));
            host.group_edit=Some(GroupEdit::CreateCollection);
            host.group_name.update(cx,|e,cx|e.set_value("WS",cx));
            host.save_group(cx);host.add_request(cx);
            let decoder=host.plugin_catalog.iter().find(|p|p.manifest.id=="com.wps.event-decrypt").unwrap();
            host.workspace.requests[0].draft.event_decoder=Some(model::BeforeAction { environment_bindings: Default::default(),approval:gpui_glint::plugins::Approval::for_plugin(decoder),action_id:"decrypt-event".into(),enabled:true});
            host.ws_detail_tab=1;
            host.set_plugin_enabled("com.wps.event-decrypt",false,cx).unwrap();
            assert!(host.ws_decrypted.as_ref().unwrap().as_ref().unwrap_err().contains("已禁用"));
            assert!(host.workspace.requests[0].draft.event_decoder.is_some());
            assert!(websocket_ui::decoder_bindings(&host.enabled_plugin_catalog()).is_empty());
            assert_eq!(host.plugin_catalog.len(),plugins::catalog().0.len());
            host.workspace.storage.connection.execute_batch("CREATE TRIGGER reject_enable BEFORE DELETE ON disabled_plugins BEGIN SELECT RAISE(FAIL,'test failure'); END;").unwrap();
            assert!(host.set_plugin_enabled("com.wps.event-decrypt",true,cx).is_err());
            assert!(host.disabled_plugins.contains("com.wps.event-decrypt"));
        });
    }
    #[gpui::test]
    fn disabling_provider_covers_versions_but_preserves_other_sources(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            Workbench::with_store(
                storage::Store::open(&dir.path().join("families.sqlite3")).unwrap(),
                cx,
            )
        });
        host.update(window, |host, cx| {
            let mut first = host.plugin_catalog[0].clone();
            first.origin = Some(gpui_glint::plugins::Origin {
                source_id: "a".repeat(64),
                package_sha256: "b".repeat(64),
            });
            let mut next = first.clone();
            next.manifest.version = "2.0.0".into();
            let mut other = first.clone();
            other.origin.as_mut().unwrap().source_id = "c".repeat(64);
            host.plugin_catalog
                .extend([first.clone(), next.clone(), other.clone()]);
            host.set_plugin_enabled(&first.identity(), false, cx)
                .unwrap();
            assert!(!host.plugin_enabled(&first));
            assert!(!host.plugin_enabled(&next));
            assert!(host.plugin_enabled(&other));
            host.plugin_catalog
                .retain(|p| p.identity() != first.identity());
            let old_binding = model::BeforeAction {
                environment_bindings: Default::default(),
                approval: gpui_glint::plugins::Approval::for_plugin(&first),
                action_id: "sign-request".into(),
                enabled: true,
            };
            assert!(host.enabled_before_actions(&[old_binding]).is_empty());
            assert!(host.plugin_enabled(&host.plugin_catalog[0]));
            let id = host.workspace.storage.create_workspace("Other").unwrap();
            let (store, _) = host.workspace.storage.prepare_workspace(&id).unwrap();
            assert!(
                store
                    .disabled_plugins()
                    .unwrap()
                    .contains(&first.family_identity())
            );
        });
    }
}
