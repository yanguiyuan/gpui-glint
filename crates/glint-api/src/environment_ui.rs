use super::*;
use std::rc::Rc;

pub(super) struct ProxyInput {
    pub(super) url: Entity<TextEntry>,
    enabled: bool,
    skip_tls_verification: bool,
    _subscription: Subscription,
}
#[derive(Clone)]
pub(super) struct VariableInput {
    id: u64,
    pub(super) name: Entity<TextEntry>,
    pub(super) value: Entity<TextEntry>,
    pub(super) kind: Entity<Select>,
    _subscriptions: Rc<Vec<Subscription>>,
}
impl Workbench {
    fn variable_input(
        &mut self,
        name: &str,
        value: &str,
        secret: bool,
        cx: &mut Context<Self>,
    ) -> VariableInput {
        let id = self.next_variable_row;
        self.next_variable_row += 1;
        let name = Self::entry(name, "变量名称", cx);
        let value = Self::entry(value, "变量值", cx);
        let theme = self.theme;
        for entry in [&name, &value] {
            entry.update(cx, |entry, cx| entry.set_theme(theme, cx));
        }
        let kind = cx.new(|cx| {
            Select::new(
                [
                    SelectOption::new("text", "文本"),
                    SelectOption::new("secret", "敏感"),
                ],
                cx,
            )
            .selected(if secret { "secret" } else { "text" })
            .searchable(false)
            .width(78.)
            .min_width(60.)
            .theme(theme)
        });
        let mut subscriptions = vec![];
        for entry in [&name, &value] {
            subscriptions.push(cx.observe(entry, |this: &mut Self, _, cx| {
                this.variable_error = None;
                cx.notify();
            }));
        }
        subscriptions.push(cx.subscribe(
            &kind,
            |this: &mut Self, select, event: &SelectEvent, cx| {
                let SelectEvent::Changed(value) = event;
                select.update(cx, |select, cx| {
                    select.set_selected(Some(value.clone()), cx)
                });
                this.revealed_variables.clear();
                this.sync_variable_table(cx);
                cx.notify();
            },
        ));
        VariableInput {
            id,
            name,
            value,
            kind,
            _subscriptions: Rc::new(subscriptions),
        }
    }
    pub(super) fn reset_variable_rows(&mut self, cx: &mut Context<Self>) {
        while self.proxy_inputs.len() < self.environment_inputs.len() {
            self.add_proxy_input(cx);
        }
        for index in 0..self.environment_inputs.len() {
            self.reset_environment_rows(index, cx);
        }
    }
    pub(super) fn add_proxy_input(&mut self, cx: &mut Context<Self>) {
        let saved = &self.environment_inputs[self.proxy_inputs.len()].proxy;
        let enabled = saved.enabled;
        let skip_tls_verification = saved.skip_tls_verification;
        let url = Self::entry(&saved.url, "http://127.0.0.1:7890", cx);
        url.update(cx, |entry, cx| entry.set_theme(self.theme, cx));
        let subscription = cx.observe(&url, |this, _, cx| {
            this.variable_error = None;
            cx.notify();
        });
        self.proxy_inputs.push(ProxyInput {
            url,
            enabled,
            skip_tls_verification,
            _subscription: subscription,
        });
    }
    fn proxy_draft(&self, cx: &App) -> environment::ProxySettings {
        let input = &self.proxy_inputs[self.editing_environment];
        environment::ProxySettings {
            enabled: input.enabled,
            url: input.url.read(cx).value().trim().into(),
            skip_tls_verification: input.skip_tls_verification,
        }
    }
    fn reset_environment_rows(&mut self, index: usize, cx: &mut Context<Self>) {
        let saved = self.environment_inputs[index].proxy.clone();
        let input = &mut self.proxy_inputs[index];
        input.enabled = saved.enabled;
        input.skip_tls_verification = saved.skip_tls_verification;
        input
            .url
            .update(cx, |entry, cx| entry.set_value(saved.url, cx));
        let variables = self.environment_inputs[index].variables.clone();
        self.variable_rows[index] = variables
            .iter()
            .map(|v| self.variable_input(&v.name, &v.value, v.secret, cx))
            .collect();
        self.variable_delete = None;
        self.variable_error = None;
        self.revealed_variables.clear();
        self.sync_variable_table(cx);
    }
    fn variable_draft(&self, cx: &App) -> Vec<(String, String, bool)> {
        self.variable_rows[self.editing_environment]
            .iter()
            .map(|row| {
                (
                    row.name.read(cx).value().into(),
                    row.value.read(cx).value().into(),
                    row.kind
                        .read(cx)
                        .value()
                        .is_some_and(|value| value.as_ref() == "secret"),
                )
            })
            .collect()
    }
    fn environment_dirty(&self, cx: &App) -> bool {
        let saved: Vec<_> = self.environment_inputs[self.editing_environment]
            .variables
            .iter()
            .map(|v| (v.name.clone(), v.value.clone(), v.secret))
            .collect();
        self.variable_draft(cx) != saved
            || self.proxy_draft(cx) != self.environment_inputs[self.editing_environment].proxy
    }
    pub(super) fn has_unsaved_environment(&self, cx: &App) -> bool {
        self.environment_inputs
            .iter()
            .enumerate()
            .any(|(index, saved)| {
                let rows: Vec<_> = self.variable_rows[index]
                    .iter()
                    .map(|row| {
                        (
                            row.name.read(cx).value().to_owned(),
                            row.value.read(cx).value().to_owned(),
                            row.kind
                                .read(cx)
                                .value()
                                .is_some_and(|value| value.as_ref() == "secret"),
                        )
                    })
                    .collect();
                let variables: Vec<_> = saved
                    .variables
                    .iter()
                    .map(|v| (v.name.clone(), v.value.clone(), v.secret))
                    .collect();
                let proxy = &self.proxy_inputs[index];
                rows != variables
                    || proxy.enabled != saved.proxy.enabled
                    || proxy.skip_tls_verification != saved.proxy.skip_tls_verification
                    || proxy.url.read(cx).value().trim() != saved.proxy.url
            })
    }
    pub(super) fn add_variable(&mut self, cx: &mut Context<Self>) {
        if self.editing_environment == 0 {
            return;
        }
        let row = self.variable_input("", "", false, cx);
        self.variable_rows[self.editing_environment].push(row);
        self.variable_search
            .update(cx, |entry, cx| entry.set_value("", cx));
        self.variable_delete = None;
        self.sync_variable_table(cx);
        cx.notify();
    }
    pub(super) fn save_environment_variables(&mut self, cx: &mut Context<Self>) {
        let rows = self.variable_draft(cx);
        let index = self.editing_environment;
        let proxy = self.proxy_draft(cx);
        match proxy
            .validate()
            .and_then(|_| self.environment_inputs[index].replace(&rows))
        {
            Ok(()) => {
                self.environment_inputs[index].proxy = proxy;
                self.storage_dirty = true;
                self.reset_environment_rows(index, cx);
                self.sync_reference_inputs(cx);
                self.notice = format!(
                    "已保存{}的环境设置",
                    self.environment_name(index).to_owned()
                );
            }
            Err(error) => self.variable_error = Some(error),
        }
        cx.notify();
    }
    pub(super) fn sync_variable_table(&mut self, cx: &mut Context<Self>) {
        let theme = self.theme;
        let p = theme.palette();
        let environment = self.editing_environment;
        let query = self.variable_search.read(cx).value().trim().to_lowercase();
        let rows = self.variable_rows[environment]
            .iter()
            .filter(|row| row.name.read(cx).value().to_lowercase().contains(&query))
            .map(|row| {
                let id = row.id;
                let name = row.name.clone();
                let name_cell = TableCell::custom(move || {
                    div()
                        .w_full()
                        .h(px(32.))
                        .font_family("Menlo")
                        .child(name.clone())
                });
                let value = row.value.clone();
                let empty = value.read(cx).value().is_empty();
                let secret = row
                    .kind
                    .read(cx)
                    .value()
                    .is_some_and(|value| value.as_ref() == "secret");
                let revealed = self.revealed_variables.contains(&(environment, id));
                let owner = cx.entity().downgrade();
                let value_cell = TableCell::custom(move || {
                    let mut cell = div()
                        .w_full()
                        .h(px(32.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .font_family("Menlo");
                    if secret && !revealed {
                        cell = cell.child(
                            div()
                                .flex_1()
                                .text_size(px(12.))
                                .text_color(rgb(p.text_muted))
                                .child(if empty {
                                    "未填写"
                                } else {
                                    "••••••••••••"
                                }),
                        );
                    } else {
                        cell = cell.child(div().flex_1().min_w_0().child(value.clone()));
                    }
                    if secret {
                        let owner = owner.clone();
                        cell = cell.child(
                            IconButton::new(
                                if revealed { Icon::Shield } else { Icon::Eye },
                                "显示或隐藏变量值",
                            )
                            .plain()
                            .theme(theme)
                            .render()
                            .size(px(22.))
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                let _ = owner.update(cx, |this, cx| {
                                    if !this.revealed_variables.remove(&(environment, id)) {
                                        this.revealed_variables.insert((environment, id));
                                    }
                                    this.sync_variable_table(cx);
                                    cx.notify();
                                });
                            }),
                        );
                    }
                    cell
                });
                let kind = row.kind.clone();
                let type_cell = TableCell::custom(move || div().w_full().child(kind.clone()));
                let name = row.name.clone();
                let label = name.read(cx).value().to_owned();
                let open = self.variable_delete == Some((environment, id));
                let owner = cx.entity().downgrade();
                let actions = TableCell::custom(move || {
                    let copy_owner = owner.clone();
                    let name = name.clone();
                    let cancel_owner = owner.clone();
                    let confirm_owner = owner.clone();
                    let toggle_owner = owner.clone();
                    let content = div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(format!(
                            "删除 {}？",
                            if label.is_empty() {
                                "此变量"
                            } else {
                                &label
                            }
                        )))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child("保存后，仍引用此变量的请求将无法发送。"),
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
                                        .on_click(move |_, _, cx| {
                                            cx.stop_propagation();
                                            let _ = cancel_owner.update(cx, |this, cx| {
                                                this.variable_delete = None;
                                                this.sync_variable_table(cx);
                                                cx.notify();
                                            });
                                        }),
                                )
                                .child(
                                    Button::new("确认删除")
                                        .kind(ButtonKind::Danger)
                                        .size(ButtonSize::Mini)
                                        .theme(theme)
                                        .render()
                                        .on_click(move |_, _, cx| {
                                            cx.stop_propagation();
                                            let _ = confirm_owner.update(cx, |this, cx| {
                                                this.variable_rows[environment]
                                                    .retain(|row| row.id != id);
                                                this.variable_delete = None;
                                                this.sync_variable_table(cx);
                                                cx.notify();
                                            });
                                        }),
                                ),
                        );
                    let delete = Popover::new(
                        ("variable-delete-popover", id),
                        IconButton::new(Icon::Trash, "删除变量")
                            .plain()
                            .theme(theme)
                            .render()
                            .size(px(22.)),
                        content,
                    )
                    .theme(theme)
                    .width(260.)
                    .trigger_height(22.)
                    .open(open)
                    .on_open_change(move |open, _, cx| {
                        let _ = toggle_owner.update(cx, |this, cx| {
                            this.variable_delete = open.then_some((environment, id));
                            this.sync_variable_table(cx);
                            cx.notify();
                        });
                    });
                    div()
                        .w_full()
                        .flex()
                        .justify_end()
                        .gap_1()
                        .child(
                            IconButton::new(Icon::Copy, "复制变量引用")
                                .plain()
                                .theme(theme)
                                .render()
                                .size(px(22.))
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    let _ = copy_owner.update(cx, |this, cx| {
                                        let reference =
                                            format!("{{{{{}}}}}", name.read(cx).value());
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            reference.clone(),
                                        ));
                                        this.notice = format!("已复制 {reference}");
                                        cx.notify();
                                    });
                                }),
                        )
                        .child(delete)
                });
                TableRow::new(
                    format!("environment-{environment}-{id}"),
                    [name_cell, value_cell, type_cell, actions],
                )
            })
            .collect::<Vec<_>>();
        self.variable_table.update(cx, |table, cx| {
            table.set_theme(theme, cx);
            table.set_rows(rows, cx);
        });
    }
    fn environment_proxy_settings(&self, cx: &mut Context<Self>) -> gpui::Div {
        let index = self.editing_environment;
        let input = &self.proxy_inputs[index];
        let owner = cx.entity().downgrade();
        let tls_owner = owner.clone();
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("代理"),
            )
            .child(
                self.request_setting_row(
                    "启用代理",
                    "当前环境下的所有请求使用此代理，保存后生效。",
                    Switch::new("")
                        .checked(input.enabled)
                        .theme(self.theme)
                        .on_change(move |enabled, _, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                this.proxy_inputs[index].enabled = enabled;
                                this.variable_error = None;
                                cx.notify();
                            });
                        })
                        .render()
                        .id("environment-proxy-enabled")
                        .debug_selector(|| "environment-proxy-enabled".into()),
                ),
            )
            .child(self.request_setting_row(
                "代理地址",
                "支持 HTTP / HTTPS 代理，例如 http://127.0.0.1:7890。",
                input.url.clone(),
            ))
            .child(
                self.request_setting_row(
                    "跳过 TLS 证书校验",
                    "使用此代理时跳过目标服务及 HTTPS 代理的证书校验。",
                    Switch::new("")
                        .checked(input.skip_tls_verification)
                        .theme(self.theme)
                        .on_change(move |skip, _, cx| {
                            let _ = tls_owner.update(cx, |this, cx| {
                                this.proxy_inputs[index].skip_tls_verification = skip;
                                cx.notify();
                            });
                        })
                        .render()
                        .id("environment-proxy-skip-tls")
                        .debug_selector(|| "environment-proxy-skip-tls".into()),
                ),
            )
    }
    pub(super) fn environment_workspace(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        if self.editing_environment == 0 {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(p.text_muted))
                .child("暂无环境，请从左侧创建环境");
        }
        let index = self.editing_environment;
        let applied = self.selected_environment(cx) == index;
        let dirty = self.environment_dirty(cx);
        let mut body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.environment_name(index).to_owned()),
                    )
                    .when(applied, |el| {
                        el.child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(self.theme.color(0x278258)))
                                .child("当前请求使用中"),
                        )
                    })
                    .when(!applied, |el| {
                        el.child(
                            Button::new("用于当前请求")
                                .kind(ButtonKind::Gray)
                                .size(ButtonSize::Mini)
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.environment.update(cx, |select, cx| {
                                        select.set_selected(Some(index.to_string().into()), cx)
                                    });
                                    this.sync_reference_inputs(cx);
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("变量"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(self.variable_rows[index].len().to_string()),
                    )
                    .child(
                        SearchInput::new("搜索变量名称…")
                            .control(self.variable_search.clone())
                            .width(190.)
                            .theme(self.theme)
                            .render(),
                    ),
            )
            .child(self.variable_table.clone());
        body = body
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("点击单元格编辑 · 使用 {{变量名}} 引用 · 敏感值点击眼睛后编辑"),
            )
            .child(
                div().flex().flex_col().gap_2()
                    .child(div().text_size(px(11.)).child("模板方法 · 点击复制引用"))
                    .child(div().flex().flex_wrap().gap_2().children(
                        super::template::METHOD_EXAMPLES.iter().map(|(expression, label)| {
                            let expression = expression.to_string();
                            Button::new(*label).kind(ButtonKind::Gray).size(ButtonSize::Mini)
                                .theme(self.theme).render()
                                .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(expression.clone())))
                        })
                    ))
                    .child(div().text_size(px(10.)).text_color(rgb(p.text_muted))
                        .child("用于 URL、参数、请求头、Cookie、认证和请求体；同次发送复用相同表达式，时间为 UTC。"))
            )
            .child(self.environment_proxy_settings(cx))
            .when_some(self.variable_error.clone(), |el, error| {
                el.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(self.theme.color(0xd34545)))
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(10.))
                            .text_color(rgb(if dirty { p.accent } else { p.text_muted }))
                            .child(if dirty {
                                "有未保存的更改"
                            } else {
                                "所有更改已保存"
                            }),
                    )
                    .when(dirty, |el| {
                        el.child(
                            Button::new("放弃更改")
                                .kind(ButtonKind::Gray)
                                .size(ButtonSize::Mini)
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.reset_environment_rows(index, cx);
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        Button::new("保存更改")
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .opacity(if dirty { 1. } else { 0.45 })
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.environment_dirty(cx) {
                                    this.save_environment_variables(cx);
                                }
                            })),
                    ),
            );
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
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .child("环境管理"),
                    )
                    .child(
                        Button::new("返回请求")
                            .icon(Icon::ArrowLeft)
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_environments = false;
                                this.revealed_variables.clear();
                                this.variable_delete = None;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("environment-table-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(24.))
                    .child(body),
            )
            .child(
                div()
                    .h(px(32.))
                    .flex_shrink_0()
                    .px_5()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("敏感变量默认隐藏 · 已保存环境设置存储于本机 SQLite"),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn proxy_edits_save_atomically_discard_and_survive_restart(cx: &mut gpui::TestAppContext) {
        let path = std::env::temp_dir().join(format!(
            "glint-api-proxy-{}-{}.sqlite3",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateEnvironment);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("Development", cx));
            host.save_group(cx);
            host.show_environments = true;
            host
        });
        let bounds = window.debug_bounds("environment-proxy-enabled").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        let bounds = window.debug_bounds("environment-proxy-skip-tls").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert!(host.proxy_draft(cx).enabled && host.proxy_draft(cx).skip_tls_verification);
            assert!(
                !host.environment_inputs[1].proxy.enabled,
                "unsaved drafts must not affect requests"
            );
            host.add_variable(cx);
            host.variable_rows[1][0]
                .name
                .update(cx, |entry, cx| entry.set_value("name", cx));
            host.save_environment_variables(cx);
            assert!(host.variable_error.is_some());
            assert!(
                host.environment_inputs[1].variables.is_empty(),
                "invalid proxy must not partially save variables"
            );
            host.proxy_inputs[1].url.update(cx, |entry, cx| {
                entry.set_value(" http://127.0.0.1:7890 ", cx)
            });
            host.save_environment_variables(cx);
            assert!(host.variable_error.is_none());
            assert!(host.environment_inputs[1].proxy.enabled);
            assert_eq!(
                host.environment_inputs[1].proxy.url,
                "http://127.0.0.1:7890"
            );
            assert!(!host.environment_dirty(cx));
            host.proxy_inputs[1].enabled = false;
            host.reset_environment_rows(1, cx);
            assert!(host.proxy_draft(cx).enabled);
            host.group_edit = Some(GroupEdit::CreateEnvironment);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("Production", cx));
            host.save_group(cx);
            assert!(!host.proxy_draft(cx).enabled);
            host.editing_environment = 1;
            assert!(host.proxy_draft(cx).enabled);
            host.persist(cx);
        });
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(window, |host, _| {
            assert!(
                host.environment_inputs[1].proxy.enabled
                    && host.environment_inputs[1].proxy.skip_tls_verification
            );
            assert!(!host.environment_inputs[2].proxy.enabled);
        });
        let mut legacy = serde_json::to_value(EnvironmentVariables::default()).unwrap();
        legacy.as_object_mut().unwrap().remove("proxy");
        assert!(
            !serde_json::from_value::<EnvironmentVariables>(legacy)
                .unwrap()
                .proxy
                .enabled
        );
        std::fs::remove_file(path).unwrap();
    }
}
