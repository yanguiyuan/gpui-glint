use super::*;

pub(super) struct ProxyInput {
    pub(super) url: Entity<TextEntry>,
    enabled: bool,
    built_in: bool,
    skip_tls_verification: bool,
    pub(super) mappings: Vec<MappingInput>,
    pub(super) table: Entity<DataTable>,
    _table_subscription: Subscription,
    _subscription: Subscription,
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
            if !self.proxies.iter().any(|p| &p.id == id) {
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
            subscriptions.push(cx.observe(entry, |this, _, cx| {
                this.variable_error = None;
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
            .theme(theme)
            .empty_row("暂无映射，添加一行或导入 Hosts 文本。")
            .add_row("添加映射")
        });
        let rows = self.proxy_table_rows(&mappings, table.entity_id(), cx);
        table.update(cx, |table, cx| table.set_rows(rows, cx));
        let table_subscription = cx.subscribe(&table, |this, table, event, cx| {
            if let TableEvent::AddRow = event {
                if let Some(index) = this.profile_inputs.iter().position(|p| p.table == table) {
                    let row = this.mapping_input("", "", cx);
                    this.profile_inputs[index].mappings.push(row);
                    this.sync_proxy_table(index, cx);
                    this.variable_error = None;
                    cx.notify();
                }
            }
        });
        ProxyInput {
            table,
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
                        }),
                        TableCell::custom(move || {
                            div()
                                .id(("proxy-ip-cell", id))
                                .debug_selector(move || format!("proxy-ip-cell-{id}"))
                                .w_full()
                                .min_w_0()
                                .h(px(32.))
                                .child(value.clone())
                        }),
                        TableCell::custom(move || {
                            let owner = owner.clone();
                            div().flex().justify_end().w_full().child(
                                IconButton::new(Icon::Close, "删除映射")
                                    .plain()
                                    .theme(theme)
                                    .render()
                                    .id(("proxy-mapping-delete", id))
                                    .debug_selector(move || format!("proxy-mapping-delete-{id}"))
                                    .size(px(22.))
                                    .opacity(0.)
                                    .group_hover(group.clone(), |s| s.opacity(1.))
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        let _ = owner.update(cx, |this, cx| {
                                            if let Some(index) = this
                                                .profile_inputs
                                                .iter()
                                                .position(|p| p.table.entity_id() == table_id)
                                            {
                                                this.profile_inputs[index]
                                                    .mappings
                                                    .retain(|row| row.id != id);
                                                this.sync_proxy_table(index, cx);
                                                this.variable_error = None;
                                                cx.notify();
                                            }
                                        });
                                    }),
                            )
                        }),
                    ],
                )
            })
            .collect()
    }
    pub(super) fn sync_proxy_table(&mut self, index: usize, cx: &mut Context<Self>) {
        let input = &self.profile_inputs[index];
        let rows = self.proxy_table_rows(&input.mappings, input.table.entity_id(), cx);
        input.table.update(cx, |table, cx| table.set_rows(rows, cx));
    }
    pub(super) fn effective_proxy(&self, index: usize) -> environment::ProxySettings {
        let variables = &self.environment_inputs[index];
        variables
            .proxy_id
            .as_ref()
            .and_then(|id| self.proxies.iter().find(|profile| &profile.id == id))
            .map(|profile| profile.settings.clone())
            .unwrap_or_else(|| variables.proxy.clone())
    }
    pub(super) fn reset_proxy_profiles(&mut self, cx: &mut Context<Self>) {
        self.profile_inputs.clear();
        self.profile_names.clear();
        for profile in self.proxies.clone() {
            let input = self.new_proxy_input(&profile.settings, cx);
            self.profile_inputs.push(input);
            let name = Self::entry(&profile.name, "代理名称", cx);
            name.update(cx, |entry, cx| entry.set_theme(self.theme, cx));
            self.profile_names.push(name);
        }
        self.editing_proxy = self.editing_proxy.min(self.proxies.len().saturating_sub(1));
        self.sync_proxy_binding_options(cx);
    }
    fn profile_draft(&self, index: usize, cx: &App) -> environment::ProxySettings {
        let input = &self.profile_inputs[index];
        environment::ProxySettings {
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
        self.proxies
            .iter()
            .enumerate()
            .map(|(index, saved)| {
                let name = self.profile_names[index].read(cx).value().trim().to_owned();
                if name.is_empty() || !names.insert(name.clone()) {
                    return Err("代理名称不能为空或重复".into());
                }
                let settings = self.profile_draft(index, cx);
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
        self.proxies.iter().enumerate().any(|(i, p)| {
            self.profile_names[i].read(cx).value().trim() != p.name
                || self.profile_draft(i, cx) != p.settings
        })
    }
    fn save_proxy_profile(&mut self, cx: &mut Context<Self>) {
        let index = self.editing_proxy;
        let name = self.profile_names[index].read(cx).value().trim().to_owned();
        let settings = self.profile_draft(index, cx);
        let result = settings.validate().and_then(|_| {
            if name.is_empty()
                || self
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
                self.proxies[index].name = name;
                self.proxies[index].settings = settings;
                self.sync_proxy_binding_options(cx);
                self.variable_error = None;
                self.storage_dirty = true;
                self.persist(cx);
                self.notice = "代理已保存，所有绑定环境立即生效".into();
            }
            Err(error) => self.variable_error = Some(error),
        }
        cx.notify();
    }
    fn delete_proxy_profile(&mut self, cx: &mut Context<Self>) {
        let index = self.editing_proxy;
        let id = self.proxies.remove(index).id;
        self.profile_inputs.remove(index);
        self.profile_names.remove(index);
        for environment in &mut self.environment_inputs {
            if environment.proxy_id.as_ref() == Some(&id) {
                environment.proxy_id = None;
            }
        }
        for input in &mut self.proxy_inputs {
            if input.binding.as_ref() == Some(&id) {
                input.binding = None;
            }
        }
        self.editing_proxy = index.min(self.proxies.len().saturating_sub(1));
        self.sync_proxy_binding_options(cx);
        self.variable_error = None;
        self.storage_dirty = true;
        self.persist(cx);
        cx.notify();
    }
    pub(super) fn proxy_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let mut list = div().flex().flex_col().gap_2().p_3().child("代理管理");
        for (index, profile) in self.proxies.iter().enumerate() {
            let count = self
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
                    let mut n = this.proxies.len() + 1;
                    while this.proxies.iter().any(|p| p.name == format!("代理 {n}")) {
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
                    this.editing_proxy = this.proxies.len();
                    this.proxies.push(environment::ProxyProfile {
                        id: uuid::Uuid::new_v4().to_string(),
                        name,
                        settings,
                    });
                    this.sync_proxy_binding_options(cx);
                    this.variable_error = None;
                    this.storage_dirty = true;
                    this.persist(cx);
                    cx.notify();
                })),
        )
    }
    pub(super) fn proxy_workspace(&self, cx: &mut Context<Self>) -> gpui::Div {
        if self.proxies.is_empty() {
            return div()
                .p_6()
                .child("新建代理后，可在环境设置中绑定；多个环境可以共用同一代理。");
        }
        let index = self.editing_proxy;
        let profile = &self.proxies[index];
        let input = &self.profile_inputs[index];
        let p = self.theme.palette();
        let bound = self
            .environment_inputs
            .iter()
            .enumerate()
            .filter(|(_, v)| v.proxy_id.as_ref() == Some(&profile.id))
            .map(|(i, _)| self.environment_names[i].clone())
            .collect::<Vec<_>>();
        let owner = cx.entity().downgrade();
        div().flex_1().min_w_0().child(
            div()
                .id("proxy-workspace")
                .size_full()
                .overflow_y_scroll()
                .p_6()
                .child(
                    div()
                        .id("proxy-settings-content")
                        .debug_selector(|| "proxy-settings-content".into())
                        .w_full()
                        .max_w(px(840.))
                        .flex()
                        .flex_col()
                        .gap_5()
                        .text_size(px(12.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
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
                                            let profile = this.proxies[index].clone();
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
                                .child(
                                    Button::new("删除代理")
                                        .kind(ButtonKind::Plain)
                                        .size(ButtonSize::Small)
                                        .theme(self.theme)
                                        .render()
                                        .text_color(rgb(p.text_muted))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.delete_proxy_profile(cx)
                                        })),
                                ),
                        ),
                ),
        )
    }
    pub(super) fn proxy_binding_options(&self) -> Vec<SelectOption> {
        std::iter::once(SelectOption::new("", "不使用代理"))
            .chain(
                self.proxies
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
                                .justify_between()
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(format!("域名映射 · {} 条", input.mappings.len())),
                                )
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
            host.show_proxies = true;
            host.show_environments = false;
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
            assert!(host.proxies[0].settings.mappings.is_empty());
            let row = &host.profile_inputs[0].mappings[0];
            row.name.update(cx, |e, cx| e.set_value("api.test", cx));
            row.value.update(cx, |e, cx| e.set_value("127.0.0.1", cx));
            host.save_proxy_profile(cx);
            assert!(host.variable_error.is_none());
            let id = host.proxies[0].id.clone();
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
                host.environment_inputs[1].proxy_id,
                host.environment_inputs[2].proxy_id
            );
            host.proxy_inputs[2].binding = None;
            host.reset_environment_rows(2, cx);
            assert_eq!(host.proxy_inputs[2].binding.as_ref(), Some(&id));
            host.persist(cx);
        });
        host.update(window, |host, cx| {
            host.show_proxies = false;
            host.show_environments = true;
            host.editing_environment = 2;
            cx.notify();
        });
        let bounds = window.debug_bounds("environment-proxy-binding").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        window.simulate_keystrokes("home");
        window.simulate_keystrokes("enter");
        host.update(window, |host, cx| {
            assert!(host.proxy_inputs[2].binding.is_none());
            assert!(host.environment_inputs[2].proxy_id.is_some());
            host.reset_environment_rows(2, cx);
        });
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(window, |host, cx| {
            assert_eq!(host.proxies[0].name, "共享代理");
            assert_eq!(host.effective_proxy(1), host.effective_proxy(2));
            assert_eq!(host.effective_proxy(1).mappings[0].domain, "api.test");
            host.delete_proxy_profile(cx);
            assert!(host.proxies.is_empty());
            assert!(host.environment_inputs.iter().all(|e| e.proxy_id.is_none()));
            assert!(!host.effective_proxy(1).enabled);
            let saved = host.storage.load().unwrap();
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
            host.proxies.push(environment::ProxyProfile {
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
            host.show_proxies = true;
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
            assert_eq!(host.proxies[0].settings.mappings.len(), 2);
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
            assert_eq!(host.storage.load().unwrap().proxies[0].settings, draft);
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
            host.proxies.push(environment::ProxyProfile {
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
            host.show_proxies = true;
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
        let content = window.debug_bounds("proxy-settings-content").unwrap();
        assert!(content.size.width <= px(840.));
        let table = window.debug_bounds("proxy-mapping-table").unwrap();
        assert!(window.debug_bounds("table-add-row").unwrap().size.width <= table.size.width);
        assert!(
            window.debug_bounds("table-add-row").unwrap().size.width > table.size.width - px(4.)
        );
        assert!(window.debug_bounds("proxy-save").unwrap().top() > table.bottom());
        let mode = window.debug_bounds("environment-proxy-built-in").unwrap();
        window.simulate_click(
            gpui::point(mode.right() - px(16.), mode.center().y),
            gpui::Modifiers::default(),
        );
        host.update(window, |host, _| assert!(!host.profile_inputs[0].built_in));
        assert!(window.debug_bounds("proxy-mapping-table").is_none());
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
                host.proxies.push(environment::ProxyProfile {
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
            host.show_proxies = true;
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
        host.update(window, |host, _| {
            assert_eq!(host.profile_inputs[0].mappings.len(), 1);
            assert_ne!(host.profile_inputs[0].mappings[0].id, row_id);
            assert_eq!(host.profile_inputs[1].mappings.len(), 1);
        });
    }
}
