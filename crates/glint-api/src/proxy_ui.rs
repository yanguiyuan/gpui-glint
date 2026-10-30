use super::*;

pub(super) struct ProxyInput {
    pub(super) url: Entity<TextEntry>,
    enabled: bool,
    built_in: bool,
    skip_tls_verification: bool,
    pub(super) mappings: Vec<MappingInput>,
    pub(super) table: Entity<DataTable>,
    delete_mapping: Option<u64>,
    delete_profile: bool,
    pub(super) search: Entity<TextEntry>,
    _search_subscription: Subscription,
    _table_subscription: Subscription,
    _subscription: Subscription,
    pub(super) listener_port: Entity<TextEntry>,
    pub(super) allow_lan: bool,
    _listener_subscription: Subscription,
}
pub(super) struct MappingInput {
    id: u64,
    pub(super) name: Entity<TextEntry>,
    pub(super) value: Entity<TextEntry>,
    _subscriptions: Vec<Subscription>,
}

impl Workbench {
    pub(super) fn validate_proxy_binding(&self, index: usize) -> Result<(), String> {
        if let Some(id) = &self.proxy_inputs[index].binding {
            if index == 0 {
                return Err("不使用环境时不能绑定代理".into());
            }
            if !self.workspace.proxies.iter().any(|p| &p.id == id) {
                return Err("绑定的代理不存在".into());
            }
        }
        Ok(())
    }
    fn import_hosts_from_clipboard(&mut self, cx: &mut Context<Self>) {
        let result = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .ok_or_else(|| "剪贴板中没有文本，请先复制 Hosts 配置".to_owned())
            .and_then(|text| environment::HostMapping::parse_hosts(&text));
        match result {
            Ok(mappings) => {
                let index = self.editing_proxy;
                let mut added = 0;
                let mut updated = 0;
                for mapping in mappings {
                    if let Some(row) = self.profile_inputs[index].mappings.iter().find(|row| {
                        row.name
                            .read(cx)
                            .value()
                            .trim()
                            .trim_end_matches('.')
                            .eq_ignore_ascii_case(&mapping.domain)
                    }) {
                        row.name
                            .update(cx, |entry, cx| entry.set_value(mapping.domain, cx));
                        row.value
                            .update(cx, |entry, cx| entry.set_value(mapping.ip, cx));
                        updated += 1;
                    } else {
                        let row = self.mapping_input(&mapping.domain, &mapping.ip, cx);
                        self.profile_inputs[index].mappings.push(row);
                        added += 1;
                    }
                }
                // Replace empty placeholders left by the Add mapping button.
                self.profile_inputs[index].mappings.retain(|row| {
                    !row.name.read(cx).value().trim().is_empty()
                        || !row.value.read(cx).value().trim().is_empty()
                });
                self.sync_proxy_table(index, cx);
                self.variable_error = None;
                self.notice =
                    format!("已导入 Hosts：新增 {added} 条，更新 {updated} 条；保存代理后生效");
            }
            Err(error) => self.variable_error = Some(error),
        }
        cx.notify();
    }
    fn mapping_input(&mut self, domain: &str, ip: &str, cx: &mut Context<Self>) -> MappingInput {
        let id = self.next_variable_row;
        self.next_variable_row += 1;
        let name = Self::entry(domain, "api.example.com", cx);
        let value = Self::entry(ip, "127.0.0.1 或 ::1", cx);
        let mut subscriptions = vec![];
        for entry in [&name, &value] {
            entry.update(cx, |entry, cx| entry.set_theme(self.theme, cx));
            let mut previous = entry.read(cx).value().to_owned();
            subscriptions.push(cx.observe(entry, move |this, entry, cx| {
                let value = entry.read(cx).value();
                if previous == value {
                    return;
                }
                previous = value.to_owned();
                this.variable_error = None;
                for index in 0..this.profile_inputs.len() {
                    if !this.profile_inputs[index]
                        .search
                        .read(cx)
                        .value()
                        .trim()
                        .is_empty()
                    {
                        this.sync_proxy_table(index, cx);
                    }
                }
                cx.notify();
            }));
        }
        MappingInput {
            id,
            name,
            value,
            _subscriptions: subscriptions,
        }
    }
    fn new_proxy_input(
        &mut self,
        saved: &environment::ProxySettings,
        cx: &mut Context<Self>,
    ) -> ProxyInput {
        let url = Self::entry(&saved.url, "http://127.0.0.1:7890", cx);
        url.update(cx, |entry, cx| entry.set_theme(self.theme, cx));
        let subscription = cx.observe(&url, |this, _, cx| {
            this.variable_error = None;
            cx.notify();
        });
        let mappings = saved
            .mappings
            .iter()
            .map(|m| self.mapping_input(&m.domain, &m.ip, cx))
            .collect::<Vec<_>>();
        let theme = self.theme;
        let table = cx.new(|cx| {
            DataTable::new(
                cx,
                [
                    TableColumn::new("域名", 140.).flexible(),
                    TableColumn::new("IP 地址", 140.).flexible(),
                    TableColumn::new("", 38.),
                ],
                [],
            )
            .document()
            .compact()
            .max_height(420.)
            .theme(theme)
            .empty_row("暂无映射，添加一行或导入 Hosts 文本。")
            .add_row("添加映射")
        });
        let rows = self.proxy_table_rows(&mappings, table.entity_id(), None, cx);
        table.update(cx, |table, cx| table.set_rows(rows, cx));
        let search = Self::entry("", "搜索域名或 IP", cx);
        search.update(cx, |entry, cx| entry.set_theme(theme, cx));
        let search_table = table.clone();
        let mut previous_query = String::new();
        let search_subscription = cx.observe(&search, move |this, search, cx| {
            let query = search.read(cx).value().to_owned();
            if previous_query == query {
                return;
            }
            previous_query = query.clone();
            if let Some(index) = this.profile_inputs.iter().position(|p| p.search == search) {
                this.sync_proxy_table(index, cx);
            }
            search_table.update(cx, |table, cx| table.set_search_query(query, cx));
            cx.notify();
        });
        let table_subscription = cx.subscribe(&table, |this, table, event, cx| {
            if let TableEvent::AddRow = event {
                if let Some(index) = this.profile_inputs.iter().position(|p| p.table == table) {
                    this.profile_inputs[index]
                        .search
                        .update(cx, |entry, cx| entry.set_value("", cx));
                    let row = this.mapping_input("", "", cx);
                    this.profile_inputs[index].mappings.push(row);
                    this.sync_proxy_table(index, cx);
                    this.variable_error = None;
                    cx.notify();
                }
            }
        });
        let listener_port = Self::entry(&saved.listener.port.to_string(), "8899", cx);
        listener_port.update(cx, |entry, cx| entry.set_theme(theme, cx));
        let listener_subscription = cx.observe(&listener_port, |this, _, cx| {
            this.variable_error = None;
            cx.notify();
        });
        ProxyInput {
            listener_port,
            _listener_subscription: listener_subscription,
            allow_lan: saved.listener.allow_lan,
            table,
            delete_mapping: None,
            delete_profile: false,
            search,
            _search_subscription: search_subscription,
            _table_subscription: table_subscription,
            url,
            enabled: saved.enabled,
            built_in: saved.built_in,
            skip_tls_verification: saved.skip_tls_verification,
            mappings,
            _subscription: subscription,
        }
    }
    fn proxy_table_rows(
        &self,
        mappings: &[MappingInput],
        table_id: gpui::EntityId,
        delete_mapping: Option<u64>,
        cx: &Context<Self>,
    ) -> Vec<TableRow> {
        let theme = self.theme;
        mappings
            .iter()
            .map(|row| {
                let id = row.id;
                let name = row.name.clone();
                let value = row.value.clone();
                let owner = cx.entity().downgrade();
                let label = row.name.read(cx).value().to_owned();
                let open = delete_mapping == Some(id);
                let group = gpui::SharedString::from(format!("proxy-mapping-{id}"));
                TableRow::new(
                    group.clone(),
                    [
                        TableCell::custom(move || {
                            div()
                                .id(("proxy-domain-cell", id))
                                .debug_selector(move || format!("proxy-domain-cell-{id}"))
                                .w_full()
                                .min_w_0()
                                .h(px(32.))
                                .child(name.clone())
                        })
                        .search_input(row.name.clone(), cx),
                        TableCell::custom(move || {
                            div()
                                .id(("proxy-ip-cell", id))
                                .debug_selector(move || format!("proxy-ip-cell-{id}"))
                                .w_full()
                                .min_w_0()
                                .h(px(32.))
                                .child(value.clone())
                        })
                        .search_input(row.value.clone(), cx),
                        TableCell::custom(move || {
                            let toggle_owner = owner.clone();
                            let confirm_owner = owner.clone();
                            div().flex().justify_end().w_full().child(
                                pair_ui::delete_popover(
                                    format!("proxy-mapping-delete-{id}"),
                                    label.clone(),
                                    open,
                                    theme,
                                    move |open, _, cx| {
                                        let _ = toggle_owner.update(cx, |this, cx| {
                                            if let Some(index) = this
                                                .profile_inputs
                                                .iter()
                                                .position(|p| p.table.entity_id() == table_id)
                                            {
                                                this.profile_inputs[index].delete_mapping =
                                                    open.then_some(id);
                                                this.sync_proxy_table(index, cx);
                                                cx.notify();
                                            }
                                        });
                                    },
                                    move |_, _, cx| {
                                        let _ = confirm_owner.update(cx, |this, cx| {
                                            if let Some(index) = this
                                                .profile_inputs
                                                .iter()
                                                .position(|p| p.table.entity_id() == table_id)
                                            {
                                                if this.profile_inputs[index].delete_mapping
                                                    != Some(id)
                                                {
                                                    return;
                                                }
                                                this.profile_inputs[index]
                                                    .mappings
                                                    .retain(|row| row.id != id);
                                                this.profile_inputs[index].delete_mapping = None;
                                                this.sync_proxy_table(index, cx);
                                                this.variable_error = None;
                                                cx.notify();
                                            }
                                        });
                                    },
                                )
                                .into_element()
                                .opacity(if open { 1. } else { 0. })
                                .group_hover(group.clone(), |s| s.opacity(1.)),
                            )
                        }),
                    ],
                )
                .focus_handle(row.name.read(cx).focus_handle(cx))
            })
            .collect()
    }
    pub(super) fn sync_proxy_table(&mut self, index: usize, cx: &mut Context<Self>) {
        let input = &self.profile_inputs[index];
        let rows = self.proxy_table_rows(
            &input.mappings,
            input.table.entity_id(),
            input.delete_mapping,
            cx,
        );
        input.table.update(cx, |table, cx| table.set_rows(rows, cx));
    }
    pub(super) fn effective_proxy(&self, index: usize) -> environment::ProxySettings {
        let variables = &self.workspace.environment_inputs[index];
        variables
            .proxy_id
            .as_ref()
            .and_then(|id| {
                self.workspace
                    .proxies
                    .iter()
                    .find(|profile| &profile.id == id)
            })
            .map(|profile| profile.settings.clone())
            .unwrap_or_else(|| variables.proxy.clone())
    }
    pub(super) fn reset_proxy_profiles(&mut self, cx: &mut Context<Self>) {
        self.profile_inputs.clear();
        self.profile_names.clear();
        for profile in self.workspace.proxies.clone() {
            let input = self.new_proxy_input(&profile.settings, cx);
            self.profile_inputs.push(input);
            let name = Self::entry(&profile.name, "代理名称", cx);
            name.update(cx, |entry, cx| entry.set_theme(self.theme, cx));
            self.profile_names.push(name);
        }
        self.editing_proxy = self
            .editing_proxy
            .min(self.workspace.proxies.len().saturating_sub(1));
        self.sync_proxy_binding_options(cx);
    }
    pub(super) fn profile_draft(&self, index: usize, cx: &App) -> environment::ProxySettings {
        let input = &self.profile_inputs[index];
        environment::ProxySettings {
            listener: environment::ProxyListenerSettings {
                port: input
                    .listener_port
                    .read(cx)
                    .value()
                    .trim()
                    .parse()
                    .unwrap_or(0),
                allow_lan: input.allow_lan,
            },
            enabled: input.enabled,
            built_in: input.built_in,
            url: input.url.read(cx).value().trim().into(),
            skip_tls_verification: input.skip_tls_verification,
            mappings: input
                .mappings
                .iter()
                .map(|row| environment::HostMapping {
                    domain: row.name.read(cx).value().trim().into(),
                    ip: row.value.read(cx).value().trim().into(),
                })
                .collect(),
        }
    }
    pub(super) fn proxy_drafts(&self, cx: &App) -> Result<Vec<environment::ProxyProfile>, String> {
        let mut names = HashSet::new();
        self.workspace
            .proxies
            .iter()
            .enumerate()
            .map(|(index, saved)| {
                let name = self.profile_names[index].read(cx).value().trim().to_owned();
                if name.is_empty() || !names.insert(name.clone()) {
                    return Err("代理名称不能为空或重复".into());
                }
                let settings = self.profile_draft(index, cx);
                settings.listener.validate()?;
                settings.validate()?;
                Ok(environment::ProxyProfile {
                    id: saved.id.clone(),
                    name,
                    settings,
                })
            })
            .collect()
    }
    pub(super) fn has_unsaved_proxies(&self, cx: &App) -> bool {
        self.workspace.proxies.iter().enumerate().any(|(i, p)| {
            self.profile_names[i].read(cx).value().trim() != p.name
                || self.profile_draft(i, cx) != p.settings
        })
    }
    fn save_proxy_profile(&mut self, cx: &mut Context<Self>) {
        let index = self.editing_proxy;
        let name = self.profile_names[index].read(cx).value().trim().to_owned();
        let settings = self.profile_draft(index, cx);
        let result = settings
            .listener
            .validate()
            .and_then(|_| settings.validate())
            .and_then(|_| {
                if name.is_empty()
                    || self
                        .workspace
                        .proxies
                        .iter()
                        .enumerate()
                        .any(|(i, p)| i != index && p.name == name)
                {
                    return Err("代理名称不能为空或重复".into());
                }
                Ok(())
            });
        match result {
            Ok(()) => {
                self.workspace.proxies[index].name = name;
                self.workspace.proxies[index].settings = settings;
                self.sync_proxy_binding_options(cx);
                self.variable_error = None;
                self.workspace.storage_dirty = true;
                self.persist(cx);
                self.notice = "代理已保存，所有绑定环境立即生效".into();
            }
            Err(error) => self.variable_error = Some(error),
        }
        cx.notify();
    }
    fn delete_proxy_profile(&mut self, cx: &mut Context<Self>) {
        let index = self.editing_proxy;
        let id = self.workspace.proxies.remove(index).id;
        self.proxy_services.remove(&id);
        self.proxy_captures.remove(&id);
        self.profile_inputs.remove(index);
        self.profile_names.remove(index);
        for environment in &mut self.workspace.environment_inputs {
            if environment.proxy_id.as_ref() == Some(&id) {
                environment.proxy_id = None;
            }
        }
        for input in &mut self.proxy_inputs {
            if input.binding.as_ref() == Some(&id) {
                input.binding = None;
            }
        }
        self.editing_proxy = index.min(self.workspace.proxies.len().saturating_sub(1));
        self.sync_proxy_binding_options(cx);
        self.variable_error = None;
        self.workspace.storage_dirty = true;
        self.persist(cx);
        cx.notify();
    }
    fn proxy_delete_popover(&self, cx: &mut Context<Self>) -> Popover {
        let index = self.editing_proxy;
        let id = self.workspace.proxies[index].id.clone();
        let confirm_id = id.clone();
        let cancel_id = id.clone();
        let theme = self.theme;
        let owner = cx.entity().downgrade();
        Popover::new(
            "proxy-delete-popover",
            Button::new("删除代理")
                .kind(ButtonKind::Danger)
                .size(ButtonSize::Small)
                .theme(theme)
                .render()
                .debug_selector(|| "proxy-delete".into()),
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(format!(
                    "删除代理“{}”？",
                    self.workspace.proxies[index].name
                )))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(theme.palette().text_muted))
                        .child("删除后，所有绑定此代理的环境将解除绑定。"),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("取消")
                                .kind(ButtonKind::Gray)
                                .size(ButtonSize::Mini)
                                .theme(theme)
                                .render()
                                .debug_selector(|| "proxy-delete-cancel".into())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(index) = this
                                        .workspace
                                        .proxies
                                        .iter()
                                        .position(|p| p.id == cancel_id)
                                    {
                                        this.profile_inputs[index].delete_profile = false;
                                        cx.notify();
                                    }
                                })),
                        )
                        .child(
                            Button::new("确认删除")
                                .kind(ButtonKind::Danger)
                                .size(ButtonSize::Mini)
                                .theme(theme)
                                .render()
                                .debug_selector(|| "proxy-delete-confirm".into())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(index) = this
                                        .workspace
                                        .proxies
                                        .iter()
                                        .position(|p| p.id == confirm_id)
                                    {
                                        if this.profile_inputs[index].delete_profile {
                                            this.editing_proxy = index;
                                            this.delete_proxy_profile(cx);
                                        }
                                    }
                                })),
                        ),
                ),
        )
        .theme(theme)
        .width(280.)
        .padding(12.)
        .trigger_height(28.)
        .open(self.profile_inputs[index].delete_profile)
        .on_open_change(move |open, _, cx| {
            let _ = owner.update(cx, |this, cx| {
                if let Some(index) = this.workspace.proxies.iter().position(|p| p.id == id) {
                    this.profile_inputs[index].delete_profile = open;
                    cx.notify();
                }
            });
        })
    }
    pub(super) fn proxy_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let mut list = div().flex().flex_col().gap_2().p_3().child("代理管理");
        for (index, profile) in self.workspace.proxies.iter().enumerate() {
            let count = self
                .workspace
                .environment_inputs
                .iter()
                .filter(|v| v.proxy_id.as_ref() == Some(&profile.id))
                .count();
            list = list.child(
                Button::new(format!("{} · {} 个环境", profile.name, count))
                    .kind(ButtonKind::Gray)
                    .theme(self.theme)
                    .render()
                    .id(("proxy-profile", index))
                    .when(index == self.editing_proxy, |button| {
                        button.bg(rgb(self.theme.palette().accent_tint))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.editing_proxy = index;
                        this.variable_error = None;
                        cx.notify();
                    })),
            );
        }
        list.child(
            Button::new("新建代理")
                .kind(ButtonKind::Gray)
                .theme(self.theme)
                .render()
                .debug_selector(|| "proxy-create".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    let mut n = this.workspace.proxies.len() + 1;
                    while this
                        .workspace
                        .proxies
                        .iter()
                        .any(|p| p.name == format!("代理 {n}"))
                    {
                        n += 1;
                    }
                    let settings = environment::ProxySettings {
                        built_in: true,
                        ..Default::default()
                    };
                    let input = this.new_proxy_input(&settings, cx);
                    let name = format!("代理 {n}");
                    this.profile_inputs.push(input);
                    let entry = Self::entry(&name, "代理名称", cx);
                    entry.update(cx, |entry, cx| entry.set_theme(this.theme, cx));
                    this.profile_names.push(entry);
                    this.editing_proxy = this.workspace.proxies.len();
                    this.workspace.proxies.push(environment::ProxyProfile {
                        id: uuid::Uuid::new_v4().to_string(),
                        name,
                        settings,
                    });
                    this.sync_proxy_binding_options(cx);
                    this.variable_error = None;
                    this.workspace.storage_dirty = true;
                    this.persist(cx);
                    cx.notify();
                })),
        )
    }
    pub(super) fn proxy_settings_workspace(&self, cx: &mut Context<Self>) -> gpui::Div {
        if self.workspace.proxies.is_empty() {
            return div()
                .p_6()
                .child("新建代理后，可在环境设置中绑定；多个环境可以共用同一代理。");
        }
        let index = self.editing_proxy;
        let profile = &self.workspace.proxies[index];
        let input = &self.profile_inputs[index];
        let p = self.theme.palette();
        let bound = self
            .workspace
            .environment_inputs
            .iter()
            .enumerate()
            .filter(|(_, v)| v.proxy_id.as_ref() == Some(&profile.id))
            .map(|(i, _)| self.workspace.environment_names[i].clone())
            .collect::<Vec<_>>();
        let owner = cx.entity().downgrade();
        div().flex_1().min_w_0().min_h_0().flex().flex_col().child(
            div()
                .id("proxy-workspace")
                .debug_selector(|| "proxy-settings-scroll".into())
                .w_full()
                .flex_1()
                .min_h_0()
                .track_scroll(&self.proxy_settings_scroll)
                .overflow_y_scroll()
                .p_6()
                .child(
                    div()
                        .id("proxy-settings-content")
                        .debug_selector(|| "proxy-settings-content".into())
                        .w_full()
                        .max_w(px(840.))
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .gap_5()
                        .text_size(px(12.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_3()
                                .child(
                                    div()
                                        .text_size(px(18.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("代理设置"),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(div().text_color(rgb(p.text_muted)).child(
                                            if input.enabled {
                                                "已启用"
                                            } else {
                                                "已停用"
                                            },
                                        ))
                                        .child(
                                            Switch::new("")
                                                .checked(input.enabled)
                                                .theme(self.theme)
                                                .on_change(move |enabled, _, cx| {
                                                    let _ = owner.update(cx, |this, cx| {
                                                        this.profile_inputs[index].enabled =
                                                            enabled;
                                                        this.variable_error = None;
                                                        cx.notify();
                                                    });
                                                })
                                                .render()
                                                .id("environment-proxy-enabled")
                                                .debug_selector(|| {
                                                    "environment-proxy-enabled".into()
                                                }),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_3()
                                        .child(
                                            div()
                                                .w(px(52.))
                                                .text_color(rgb(p.text_muted))
                                                .child("名称"),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .max_w(px(360.))
                                                .child(self.profile_names[index].clone()),
                                        ),
                                )
                                .child(
                                    div()
                                        .pl(px(64.))
                                        .text_size(px(11.))
                                        .text_color(rgb(p.text_muted))
                                        .child(if bound.is_empty() {
                                            "尚未绑定环境".into()
                                        } else {
                                            format!("已绑定 · {}", bound.join("、"))
                                        }),
                                ),
                        )
                        .child(self.proxy_profile_settings(cx))
                        .child(self.proxy_listener_settings(cx))
                        .when_some(self.variable_error.clone(), |body, error| {
                            body.child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(0xd9342b))
                                    .child(error),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .pt_3()
                                .border_t_1()
                                .border_color(rgb(p.border))
                                .child(
                                    Button::new("保存代理")
                                        .size(ButtonSize::Small)
                                        .theme(self.theme)
                                        .render()
                                        .debug_selector(|| "proxy-save".into())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.save_proxy_profile(cx)
                                        })),
                                )
                                .child(
                                    Button::new("丢弃修改")
                                        .kind(ButtonKind::Plain)
                                        .size(ButtonSize::Small)
                                        .theme(self.theme)
                                        .render()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            let profile = this.workspace.proxies[index].clone();
                                            this.profile_inputs[index] =
                                                this.new_proxy_input(&profile.settings, cx);
                                            this.profile_names[index].update(cx, |entry, cx| {
                                                entry.set_value(profile.name, cx)
                                            });
                                            this.variable_error = None;
                                            cx.notify();
                                        })),
                                )
                                .child(div().flex_1())
                                .child(self.proxy_delete_popover(cx)),
                        ),
                ),
        )
    }
    pub(super) fn proxy_binding_options(&self) -> Vec<SelectOption> {
        std::iter::once(SelectOption::new("", "不使用代理"))
            .chain(
                self.workspace
                    .proxies
                    .iter()
                    .map(|p| SelectOption::new(p.id.clone(), p.name.clone())),
            )
            .collect()
    }
    pub(super) fn sync_proxy_binding_options(&mut self, cx: &mut Context<Self>) {
        let options = self.proxy_binding_options();
        for input in &self.proxy_inputs {
            input.select.update(cx, |select, cx| {
                select.set_options(options.clone(), cx);
                select.set_selected(Some(input.binding.clone().unwrap_or_default().into()), cx);
                select.close(cx);
            });
        }
    }
    pub(super) fn environment_proxy_binding(&self, _: &mut Context<Self>) -> gpui::Div {
        self.request_setting_row(
            "绑定代理",
            "代理在左侧代理管理中配置，可由多个环境共用；保存环境后生效。",
            div()
                .id("environment-proxy-binding")
                .debug_selector(|| "environment-proxy-binding".into())
                .child(self.proxy_inputs[self.editing_environment].select.clone()),
        )
    }
    pub(super) fn proxy_profile_settings(&self, cx: &mut Context<Self>) -> gpui::Div {
        let index = self.editing_proxy;
        let input = &self.profile_inputs[index];
        let p = self.theme.palette();
        let owner = cx.entity().downgrade();
        let tls_owner = owner.clone();
        let table = div()
            .id("proxy-mapping-table")
            .debug_selector(|| "proxy-mapping-table".into())
            .w_full()
            .min_w_0()
            .child(input.table.clone());
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().w(px(52.)).text_color(rgb(p.text_muted)).child("类型"))
                    .child(
                        SegmentedControl::new([
                            SegmentOption::new("内置映射"),
                            SegmentOption::new("外部代理"),
                        ])
                        .id("environment-proxy-built-in")
                        .selected(usize::from(!input.built_in))
                        .theme(self.theme)
                        .on_change(move |mode, _, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                this.profile_inputs[index].built_in = mode == 0;
                                this.variable_error = None;
                                cx.notify();
                            });
                        })
                        .render()
                        .debug_selector(|| "environment-proxy-built-in".into()),
                    ),
            )
            .when(input.built_in, |body| {
                body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(div().flex_shrink_0().font_weight(FontWeight::MEDIUM).child(
                                    if input.search.read(cx).value().trim().is_empty() {
                                        format!("域名映射 · {} 条", input.mappings.len())
                                    } else {
                                        format!(
                                            "域名映射 · {} / {} 条",
                                            input.table.read(cx).matching_row_count(),
                                            input.mappings.len()
                                        )
                                    },
                                ))
                                .child(
                                    div()
                                        .min_w_0()
                                        .w(px(260.))
                                        .id("proxy-mapping-search")
                                        .debug_selector(|| "proxy-mapping-search".into())
                                        .child(
                                            SearchInput::new("搜索域名或 IP")
                                                .control(input.search.clone())
                                                .theme(self.theme)
                                                .render()
                                                .w_full(),
                                        ),
                                )
                                .child(div().flex_1())
                                .child(
                                    Button::new("导入 Hosts")
                                        .icon(Icon::Upload)
                                        .kind(ButtonKind::Plain)
                                        .size(ButtonSize::Mini)
                                        .theme(self.theme)
                                        .render()
                                        .debug_selector(|| "proxy-hosts-import".into())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.import_hosts_from_clipboard(cx)
                                        })),
                                ),
                        )
                        .child(table)
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child("连接指定 IP，保留原域名。复制 Hosts 文本后可直接导入。"),
                        ),
                )
            })
            .when(!input.built_in, |body| {
                body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().text_color(rgb(p.text_muted)).child("代理地址"))
                        .child(input.url.clone())
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child("支持 HTTP / HTTPS，例如 http://127.0.0.1:7890"),
                        ),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pt_1()
                    .child(
                        Switch::new("")
                            .checked(input.skip_tls_verification)
                            .theme(self.theme)
                            .on_change(move |skip, _, cx| {
                                let _ = tls_owner.update(cx, |this, cx| {
                                    this.profile_inputs[index].skip_tls_verification = skip;
                                    cx.notify();
                                });
                            })
                            .render()
                            .id("environment-proxy-skip-tls")
                            .debug_selector(|| "environment-proxy-skip-tls".into()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child("跳过 TLS 证书校验"),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reveal(
        window: &mut gpui::VisualTestContext,
        selector: impl Into<String>,
    ) -> Bounds<gpui::Pixels> {
        let selector = Box::leak(selector.into().into_boxed_str());
        let mut bounds = window
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        if let Some(viewport) = window.debug_bounds("proxy-settings-scroll") {
            let delta = if bounds.bottom() > viewport.bottom() {
                viewport.bottom() - bounds.bottom() - px(8.)
            } else if bounds.top() < viewport.top() {
                viewport.top() - bounds.top() + px(8.)
            } else {
                px(0.)
            };
            if delta != px(0.) {
                window.simulate_event(gpui::ScrollWheelEvent {
                    position: gpui::point(viewport.right() - px(4.), viewport.center().y),
                    delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), delta)),
                    ..Default::default()
                });
                bounds = window.debug_bounds(selector).unwrap();
            }
        }
        bounds
    }
    fn click(window: &mut gpui::VisualTestContext, selector: impl Into<String>) {
        let bounds = reveal(window, selector);
        window.simulate_mouse_move(bounds.center(), None, gpui::Modifiers::default());
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
    }

    #[gpui::test]
    fn proxy_settings_scroll_inside_panel_and_keep_tabs_fixed(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&dir.path().join("scroll.sqlite3")).unwrap(),
                cx,
            );
            host.workspace.proxies.push(environment::ProxyProfile {
                id: uuid::Uuid::new_v4().to_string(),
                name: "滚动测试".into(),
                settings: environment::ProxySettings {
                    built_in: true,
                    mappings: (0..442)
                        .map(|i| environment::HostMapping {
                            domain: format!("host-{i}.test"),
                            ip: "127.0.0.1".into(),
                        })
                        .collect(),
                    ..Default::default()
                },
            });
            host.reset_proxy_profiles(cx);
            host.page = crate::Page::Proxies;
            host
        });
        window.simulate_resize(gpui::size(px(1000.), px(650.)));
        let tabs = window.debug_bounds("proxy-workspace-tabs").unwrap();
        let apps = window.debug_bounds("proxy-open-application").unwrap();
        let viewport = window.debug_bounds("proxy-settings-scroll").unwrap();
        assert!(viewport.bottom() <= px(650.));
        assert!(window.debug_bounds("proxy-ca-install").unwrap().bottom() > viewport.bottom());
        window.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(viewport.right() - px(4.), viewport.center().y),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))),
            ..Default::default()
        });
        host.read_with(window, |host, _| {
            assert!(host.proxy_settings_scroll.offset().y < px(-100.))
        });
        for selector in ["proxy-ca-install", "proxy-save", "proxy-delete"] {
            let bounds = window.debug_bounds(selector).unwrap();
            assert!(
                bounds.top() >= viewport.top() && bounds.bottom() <= viewport.bottom(),
                "{selector}: {bounds:?}, {viewport:?}"
            );
        }
        assert_eq!(window.debug_bounds("proxy-workspace-tabs").unwrap(), tabs);
        assert_eq!(window.debug_bounds("proxy-open-application").unwrap(), apps);
    }
    #[gpui::test]
    fn deleting_proxy_requires_confirmation_and_persists_only_on_confirm(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&dir.path().join("delete.sqlite3")).unwrap(),
                cx,
            );
            for name in ["A", "B"] {
                host.workspace.proxies.push(environment::ProxyProfile {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: name.into(),
                    settings: environment::ProxySettings {
                        built_in: true,
                        ..Default::default()
                    },
                });
            }
            host.reset_proxy_profiles(cx);
            host.page = crate::Page::Proxies;
            host.workspace.storage_dirty = true;
            host.persist(cx);
            host
        });
        click(window, "proxy-delete");
        host.update(window, |host, _| {
            assert_eq!(host.workspace.proxies.len(), 2);
            assert_eq!(host.workspace.storage.load().unwrap().proxies.len(), 2);
        });
        click(window, "proxy-delete-cancel");
        assert!(window.debug_bounds("proxy-delete-confirm").is_none());
        host.read_with(window, |host, _| {
            assert_eq!(host.workspace.proxies.len(), 2)
        });
        click(window, "proxy-delete");
        window.simulate_keystrokes("escape");
        assert!(window.debug_bounds("proxy-delete-confirm").is_none());
        click(window, "proxy-delete");
        click(window, "proxy-delete-confirm");
        host.update(window, |host, _| {
            assert_eq!(host.workspace.proxies.len(), 1);
            assert_eq!(host.workspace.proxies[0].name, "B");
            assert_eq!(host.workspace.storage.load().unwrap().proxies[0].name, "B");
        });
    }

    #[gpui::test]
    fn mapping_search_filters_domain_and_ip_and_keeps_row_actions_correct(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&dir.path().join("search.sqlite3")).unwrap(),
                cx,
            );
            host.workspace.proxies.push(environment::ProxyProfile {
                id: uuid::Uuid::new_v4().to_string(),
                name: "搜索".into(),
                settings: environment::ProxySettings {
                    built_in: true,
                    mappings: (0..442)
                        .map(|index| environment::HostMapping {
                            domain: format!("Host-{index}.test"),
                            ip: if index == 441 {
                                "10.20.30.40"
                            } else {
                                "127.0.0.1"
                            }
                            .into(),
                        })
                        .collect(),
                    ..Default::default()
                },
            });
            host.reset_proxy_profiles(cx);
            host.page = crate::Page::Proxies;
            host
        });
        let (search, table, id) = host.read_with(window, |host, _| {
            let input = &host.profile_inputs[0];
            (
                input.search.clone(),
                input.table.clone(),
                input.mappings.last().unwrap().id,
            )
        });
        let edited = host.read_with(window, |host, _| {
            host.profile_inputs[0].mappings[0].name.clone()
        });
        edited.update(window, |entry, cx| {
            entry.set_value("edited-before-search.test", cx)
        });
        search.update(window, |entry, cx| {
            entry.set_value("edited-before-search", cx)
        });
        table.read_with(window, |table, _| assert_eq!(table.matching_row_count(), 1));
        edited.update(window, |entry, cx| entry.set_value("Host-0.test", cx));
        search.update(window, |entry, cx| entry.set_value("", cx));
        click(window, "proxy-mapping-search");
        window.simulate_input("HOST-441");
        table.read_with(window, |table, _| assert_eq!(table.matching_row_count(), 1));
        assert!(
            window
                .debug_bounds(Box::leak(
                    format!("proxy-domain-cell-{id}").into_boxed_str()
                ))
                .is_some()
        );
        search.update(window, |entry, cx| entry.set_value("10.20.30.40", cx));
        table.read_with(window, |table, _| assert_eq!(table.matching_row_count(), 1));
        click(window, format!("proxy-mapping-delete-{id}"));
        host.read_with(window, |host, _| {
            assert_eq!(host.profile_inputs[0].mappings.len(), 442)
        });
        click(window, format!("proxy-mapping-delete-{id}-confirm"));
        table.read_with(window, |table, _| assert_eq!(table.matching_row_count(), 0));
        host.read_with(window, |host, _| {
            assert_eq!(host.profile_inputs[0].mappings.len(), 441);
            assert_eq!(host.workspace.proxies[0].settings.mappings.len(), 442);
        });
        assert!(window.debug_bounds("table-empty-state").is_some());
        search.update(window, |entry, cx| entry.set_value("host-0.test", cx));
        let name = host.read_with(window, |host, _| {
            host.profile_inputs[0].mappings[0].name.clone()
        });
        name.update(window, |entry, cx| entry.set_value("changed.test", cx));
        table.read_with(window, |table, _| assert_eq!(table.matching_row_count(), 0));
        click(window, "table-add-row");
        search.read_with(window, |entry, _| assert_eq!(entry.value(), ""));
        table.read_with(window, |table, _| {
            assert_eq!(table.matching_row_count(), 442)
        });
        let name = host.read_with(window, |host, _| {
            host.profile_inputs[0].mappings.last().unwrap().name.clone()
        });
        window.update(|window, cx| assert!(name.read(cx).focus_handle(cx).is_focused(window)));
        window.simulate_input("new.test");
        name.read_with(window, |entry, _| assert_eq!(entry.value(), "new.test"));
    }
    #[gpui::test]
    fn shared_proxy_binding_save_discard_restart_and_delete(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proxy.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            for name in ["Dev", "Test"] {
                host.group_edit = Some(GroupEdit::CreateEnvironment);
                host.group_name.update(cx, |e, cx| e.set_value(name, cx));
                host.save_group(cx);
            }
            host.page = crate::Page::Proxies;
            host
        });
        let bounds = window.debug_bounds("proxy-create").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        assert!(window.debug_bounds("environment-proxy-built-in").is_some());
        let bounds = window.debug_bounds("table-add-row").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            host.profile_inputs[0].enabled = true;
            host.save_proxy_profile(cx);
            assert!(host.variable_error.is_some());
            assert!(host.workspace.proxies[0].settings.mappings.is_empty());
            let row = &host.profile_inputs[0].mappings[0];
            row.name.update(cx, |e, cx| e.set_value("api.test", cx));
            row.value.update(cx, |e, cx| e.set_value("127.0.0.1", cx));
            host.save_proxy_profile(cx);
            assert!(host.variable_error.is_none());
            let id = host.workspace.proxies[0].id.clone();
            for index in [1, 2] {
                host.editing_environment = index;
                host.proxy_inputs[index].binding = Some(id.clone());
                assert!(host.environment_dirty(cx));
                host.save_environment_variables(cx);
                assert!(!host.environment_dirty(cx));
                assert!(host.effective_proxy(index).built_in);
            }
            host.profile_names[0].update(cx, |e, cx| e.set_value("共享代理", cx));
            host.save_proxy_profile(cx);
            assert_eq!(
                host.workspace.environment_inputs[1].proxy_id,
                host.workspace.environment_inputs[2].proxy_id
            );
            host.proxy_inputs[2].binding = None;
            host.reset_environment_rows(2, cx);
            assert_eq!(host.proxy_inputs[2].binding.as_ref(), Some(&id));
            host.persist(cx);
        });
        host.update(window, |host, cx| {
            host.page = crate::Page::Environments;
            host.editing_environment = 2;
            cx.notify();
        });
        let bounds = window.debug_bounds("environment-proxy-binding").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        window.simulate_keystrokes("home");
        window.simulate_keystrokes("enter");
        host.update(window, |host, cx| {
            assert!(host.proxy_inputs[2].binding.is_none());
            assert!(host.workspace.environment_inputs[2].proxy_id.is_some());
            host.reset_environment_rows(2, cx);
        });
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(window, |host, cx| {
            assert_eq!(host.workspace.proxies[0].name, "共享代理");
            assert_eq!(host.effective_proxy(1), host.effective_proxy(2));
            assert_eq!(host.effective_proxy(1).mappings[0].domain, "api.test");
            host.delete_proxy_profile(cx);
            assert!(host.workspace.proxies.is_empty());
            assert!(
                host.workspace
                    .environment_inputs
                    .iter()
                    .all(|e| e.proxy_id.is_none())
            );
            assert!(!host.effective_proxy(1).enabled);
            let saved = host.workspace.storage.load().unwrap();
            assert!(saved.proxies.is_empty());
        });
    }
    #[gpui::test]
    fn hosts_clipboard_import_merges_atomically_and_saves_explicitly(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("import.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.workspace.proxies.push(environment::ProxyProfile {
                id: uuid::Uuid::new_v4().to_string(),
                name: "Hosts".into(),
                settings: environment::ProxySettings {
                    enabled: true,
                    built_in: true,
                    mappings: vec![
                        environment::HostMapping {
                            domain: "API.test.".into(),
                            ip: "10.0.0.1".into(),
                        },
                        environment::HostMapping {
                            domain: "keep.test".into(),
                            ip: "10.0.0.2".into(),
                        },
                    ],
                    ..Default::default()
                },
            });
            host.reset_proxy_profiles(cx);
            let empty = host.mapping_input("", "", cx);
            host.profile_inputs[0].mappings.push(empty);
            host.page = crate::Page::Proxies;
            host
        });
        host.update(window, |_, cx| cx.write_to_clipboard(ClipboardItem::new_string(
            "# copied hosts\n127.0.0.1 api.test alias.test\n::1 ipv6.test # local\n10.0.0.3 API.test".into())));
        let bounds = window.debug_bounds("proxy-hosts-import").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        let draft = host.update(window, |host, cx| {
            let draft = host.profile_draft(0, cx);
            assert_eq!(draft.mappings.len(), 4);
            assert_eq!(draft.mappings[0].domain, "api.test");
            assert_eq!(draft.mappings[0].ip, "10.0.0.3");
            assert_eq!(draft.mappings[1].domain, "keep.test");
            assert_eq!(host.workspace.proxies[0].settings.mappings.len(), 2);
            assert!(host.notice.contains("新增 2 条，更新 1 条"));
            cx.write_to_clipboard(ClipboardItem::new_string(
                "127.0.0.1 new.test\nbad-ip broken.test".into(),
            ));
            draft
        });
        let row_ids = host.read_with(window, |host, _| {
            host.profile_inputs[0]
                .mappings
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>()
        });
        for id in row_ids {
            assert!(
                window
                    .debug_bounds(Box::leak(
                        format!("proxy-domain-cell-{id}").into_boxed_str()
                    ))
                    .is_some()
            );
        }
        let bounds = window.debug_bounds("proxy-hosts-import").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert!(host.variable_error.as_ref().unwrap().contains("第 2 行"));
            assert_eq!(host.profile_draft(0, cx), draft);
            host.save_proxy_profile(cx);
            assert!(host.variable_error.is_none());
            assert_eq!(
                host.workspace.storage.load().unwrap().proxies[0].settings,
                draft
            );
        });
    }
    #[gpui::test]
    fn compact_proxy_layout_aligns_columns_and_switches_modes(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&dir.path().join("layout.sqlite3")).unwrap(),
                cx,
            );
            host.workspace.proxies.push(environment::ProxyProfile {
                id: uuid::Uuid::new_v4().to_string(),
                name: "布局".into(),
                settings: environment::ProxySettings {
                    built_in: true,
                    mappings: vec![environment::HostMapping {
                        domain: "api.test".into(),
                        ip: "127.0.0.1".into(),
                    }],
                    ..Default::default()
                },
            });
            host.reset_proxy_profiles(cx);
            host.page = crate::Page::Proxies;
            host
        });
        let row_id = host.read_with(window, |host, _| host.profile_inputs[0].mappings[0].id);
        let domain = window
            .debug_bounds(Box::leak(
                format!("proxy-domain-cell-{row_id}").into_boxed_str(),
            ))
            .unwrap();
        let ip = window
            .debug_bounds(Box::leak(
                format!("proxy-ip-cell-{row_id}").into_boxed_str(),
            ))
            .unwrap();
        assert!(domain.left() < ip.left());
        assert_eq!(domain.top(), ip.top());
        assert_eq!(domain.size.width, ip.size.width);
        let domain_header = window.debug_bounds("table-header-cell-0").unwrap();
        let ip_header = window.debug_bounds("table-header-cell-1").unwrap();
        assert_eq!(domain.left(), domain_header.left() + px(8.));
        assert_eq!(ip.left(), ip_header.left() + px(8.));
        assert_eq!(domain.size.width, domain_header.size.width - px(16.));
        assert_eq!(ip.size.width, ip_header.size.width - px(16.));
        let content = window.debug_bounds("proxy-settings-content").unwrap();
        assert!(content.size.width <= px(840.));
        let table = window.debug_bounds("proxy-mapping-table").unwrap();
        assert!(window.debug_bounds("table-add-row").unwrap().size.width <= table.size.width);
        assert!(
            window.debug_bounds("table-add-row").unwrap().size.width > table.size.width - px(4.)
        );
        assert!(window.debug_bounds("proxy-save").unwrap().top() > table.bottom());
        host.update(window, |host, cx| {
            host.profile_inputs[0].mappings = (0..60)
                .map(|index| host.mapping_input(&format!("host-{index}.test"), "127.0.0.1", cx))
                .collect();
            host.sync_proxy_table(0, cx);
        });
        reveal(window, "proxy-mapping-table");
        let table = window.debug_bounds("proxy-mapping-table").unwrap();
        assert!(table.size.height <= px(420.));
        let viewport = window.debug_bounds("table-body-scroll").unwrap();
        let header = window.debug_bounds("table-header").unwrap();
        window.simulate_event(gpui::ScrollWheelEvent {
            position: viewport.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))),
            ..Default::default()
        });
        let add = window.debug_bounds("table-add-row").unwrap();
        assert!(add.top() >= viewport.bottom() && add.bottom() <= table.bottom());
        assert_eq!(window.debug_bounds("table-header").unwrap(), header);
        // The rail jumps directly to the end, independent of estimated offscreen row heights.
        let rail = window.debug_bounds("table-scrollbar").unwrap();
        window.simulate_click(
            gpui::point(rail.center().x, rail.bottom() - px(2.)),
            gpui::Modifiers::default(),
        );
        let last_id = host.read_with(window, |host, _| {
            host.profile_inputs[0].mappings.last().unwrap().id
        });
        let domain = window
            .debug_bounds(Box::leak(
                format!("proxy-domain-cell-{last_id}").into_boxed_str(),
            ))
            .unwrap();
        let ip = window
            .debug_bounds(Box::leak(
                format!("proxy-ip-cell-{last_id}").into_boxed_str(),
            ))
            .unwrap();
        assert_eq!(
            domain.left(),
            window.debug_bounds("table-header-cell-0").unwrap().left() + px(8.)
        );
        assert_eq!(
            ip.left(),
            window.debug_bounds("table-header-cell-1").unwrap().left() + px(8.)
        );
        let mode = reveal(window, "environment-proxy-built-in");
        window.simulate_click(
            gpui::point(mode.right() - px(16.), mode.center().y),
            gpui::Modifiers::default(),
        );
        host.update(window, |host, _| assert!(!host.profile_inputs[0].built_in));
        assert!(window.debug_bounds("proxy-mapping-table").is_none());
    }
    #[gpui::test]
    fn add_mapping_is_fixed_and_focuses_new_last_row_in_a_large_table(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&dir.path().join("add-row.sqlite3")).unwrap(),
                cx,
            );
            host.workspace.proxies.push(environment::ProxyProfile {
                id: uuid::Uuid::new_v4().to_string(),
                name: "大量映射".into(),
                settings: environment::ProxySettings {
                    built_in: true,
                    mappings: (0..442)
                        .map(|index| environment::HostMapping {
                            domain: format!("host-{index}.test"),
                            ip: "127.0.0.1".into(),
                        })
                        .collect(),
                    ..Default::default()
                },
            });
            host.reset_proxy_profiles(cx);
            host.page = crate::Page::Proxies;
            host
        });
        reveal(window, "proxy-mapping-table");
        let add = window.debug_bounds("table-add-row").unwrap();
        let viewport = window.debug_bounds("table-body-scroll").unwrap();
        assert!(add.top() >= viewport.bottom());
        window.simulate_event(gpui::ScrollWheelEvent {
            position: viewport.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-4000.))),
            ..Default::default()
        });
        assert_eq!(window.debug_bounds("table-add-row").unwrap(), add);
        window.simulate_click(add.center(), gpui::Modifiers::default());
        let (id, entry) = host.read_with(window, |host, _| {
            assert_eq!(host.profile_inputs[0].mappings.len(), 443);
            let row = host.profile_inputs[0].mappings.last().unwrap();
            (row.id, row.name.clone())
        });
        let last = window
            .debug_bounds(Box::leak(
                format!("proxy-domain-cell-{id}").into_boxed_str(),
            ))
            .unwrap();
        assert!(last.top() >= viewport.top() && last.bottom() <= viewport.bottom());
        assert_eq!(window.debug_bounds("table-add-row").unwrap(), add);
        window.update(|window, cx| assert!(entry.read(cx).focus_handle(cx).is_focused(window)));
        window.simulate_input("added.test");
        entry.read_with(window, |entry, _| assert_eq!(entry.value(), "added.test"));
    }

    #[gpui::test]
    fn standard_table_add_and_delete_follow_proxy_and_row_identity(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&dir.path().join("table.sqlite3")).unwrap(),
                cx,
            );
            for name in ["A", "B"] {
                host.workspace.proxies.push(environment::ProxyProfile {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: name.into(),
                    settings: environment::ProxySettings {
                        built_in: true,
                        mappings: vec![environment::HostMapping {
                            domain: "api.test".into(),
                            ip: "127.0.0.1".into(),
                        }],
                        ..Default::default()
                    },
                });
            }
            host.reset_proxy_profiles(cx);
            host.page = crate::Page::Proxies;
            host
        });
        let (table, row_id) = host.read_with(window, |host, _| {
            (
                host.profile_inputs[0].table.clone(),
                host.profile_inputs[0].mappings[0].id,
            )
        });
        host.update(window, |host, cx| {
            host.editing_proxy = 1;
            cx.notify();
        });
        table.update(window, |_, cx| cx.emit(TableEvent::AddRow));
        host.update(window, |host, cx| {
            assert_eq!(host.profile_inputs[0].mappings.len(), 2);
            assert_eq!(host.profile_inputs[1].mappings.len(), 1);
            host.editing_proxy = 0;
            cx.notify();
        });
        let bounds = window
            .debug_bounds(Box::leak(
                format!("proxy-mapping-delete-{row_id}").into_boxed_str(),
            ))
            .unwrap();
        window.simulate_mouse_move(bounds.center(), None, gpui::Modifiers::default());
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        host.read_with(window, |host, _| {
            assert_eq!(host.profile_inputs[0].mappings.len(), 2);
            assert_eq!(host.profile_inputs[0].delete_mapping, Some(row_id));
        });
        let cancel = window
            .debug_bounds(Box::leak(
                format!("proxy-mapping-delete-{row_id}-cancel").into_boxed_str(),
            ))
            .unwrap();
        window.simulate_click(cancel.center(), gpui::Modifiers::default());
        host.read_with(window, |host, _| {
            assert_eq!(host.profile_inputs[0].mappings.len(), 2)
        });
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        let confirm = window
            .debug_bounds(Box::leak(
                format!("proxy-mapping-delete-{row_id}-confirm").into_boxed_str(),
            ))
            .unwrap();
        window.simulate_click(confirm.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert_eq!(host.profile_inputs[0].mappings.len(), 1);
            assert_ne!(host.profile_inputs[0].mappings[0].id, row_id);
            assert_eq!(host.profile_inputs[1].mappings.len(), 1);
        });
    }
}
