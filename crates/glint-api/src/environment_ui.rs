use super::*;
use std::rc::Rc;

pub(super) struct ProxyBindingInput {
    pub(super) binding: Option<String>,
    pub(super) select: Entity<Select>,
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
    pub(super) fn environment_values(
        &self,
        index: usize,
        collection: Option<&str>,
    ) -> environment::EnvironmentValues {
        let mut values = self.workspace.environment_inputs[index].values();
        if let Some(local) = collection
            .and_then(|name| {
                self.workspace
                    .collections
                    .iter()
                    .find(|c| c.collection == name)
            })
            .and_then(|c| c.environments.get(self.environment_name(index)))
        {
            values.0.extend(local.values().0);
        }
        values
    }
    pub(super) fn active_environment_values(&self, cx: &App) -> environment::EnvironmentValues {
        self.environment_values(
            self.selected_environment(cx),
            self.workspace
                .requests
                .get(self.active)
                .map(|r| r.collection.as_str()),
        )
    }
    pub(super) fn environment_completions(&self, cx: &App) -> Vec<TextCompletion> {
        self.environment_completions_for(
            cx,
            self.workspace
                .requests
                .get(self.active)
                .map(|r| r.collection.as_str()),
        )
    }
    pub(super) fn environment_completions_for(
        &self,
        cx: &App,
        collection: Option<&str>,
    ) -> Vec<TextCompletion> {
        let index = self.selected_environment(cx);
        let mut entries = std::collections::BTreeMap::new();
        let local = collection
            .and_then(|name| {
                self.workspace
                    .collections
                    .iter()
                    .find(|c| c.collection == name)
            })
            .and_then(|c| c.environments.get(self.environment_name(index)));
        for (variables, scope) in [
            (Some(&self.workspace.environment_inputs[index]), "全局变量"),
            (local, "集合变量"),
        ] {
            if let Some(variables) = variables {
                for variable in &variables.variables {
                    entries.insert(
                        variable.name.clone(),
                        TextCompletion::new(
                            &variable.name,
                            if variable.secret {
                                format!("{scope} · 敏感")
                            } else {
                                scope.into()
                            },
                        ),
                    );
                }
            }
        }
        let mut completions: Vec<_> = entries.into_values().collect();
        completions.extend(
            template::METHOD_EXAMPLES
                .iter()
                .map(|(expression, description)| {
                    TextCompletion::new(
                        expression.trim_start_matches("{{").trim_end_matches("}}"),
                        format!("内置函数 · {description}"),
                    )
                }),
        );
        completions
    }
    pub(super) fn editor_environment(&self, index: usize) -> EnvironmentVariables {
        if let Some(name) = &self.editing_variable_collection {
            self.workspace
                .collections
                .iter()
                .find(|c| &c.collection == name)
                .and_then(|c| c.environments.get(self.environment_name(index)))
                .cloned()
                .unwrap_or_default()
        } else {
            self.workspace.environment_inputs[index].clone()
        }
    }
    pub(super) fn scoped_environment_mut(
        &mut self,
        index: usize,
        collection: Option<&str>,
    ) -> Result<&mut EnvironmentVariables, String> {
        if let Some(name) = collection {
            let environment = self.environment_name(index).to_owned();
            let collection = self
                .workspace
                .collections
                .iter_mut()
                .find(|c| c.collection == name)
                .ok_or("集合不存在")?;
            Ok(collection.environments.entry(environment).or_default())
        } else {
            Ok(&mut self.workspace.environment_inputs[index])
        }
    }
    pub(super) fn set_variable_scope(
        &mut self,
        collection: Option<String>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.editing_variable_collection == collection {
            return Ok(());
        }
        if self.has_unsaved_environment(cx) {
            return Err("请先保存或放弃当前作用域的修改".into());
        }
        if collection.as_ref().is_some_and(|name| {
            !self
                .workspace
                .collections
                .iter()
                .any(|c| &c.collection == name)
        }) {
            return Err("集合不存在".into());
        }
        self.editing_variable_collection = collection;
        self.reset_variable_rows(cx);
        Ok(())
    }
    pub(super) fn mcp_environment_editor(&self, index: usize, cx: &App) -> serde_json::Value {
        let rows = self.variable_rows[index]
            .iter()
            .map(|row| {
                serde_json::json!({
                    "name":row.name.read(cx).value(),"value":row.value.read(cx).value(),
                    "secret":row.kind.read(cx).value().is_some_and(|kind| kind.as_ref() == "secret")
                })
            })
            .collect::<Vec<_>>();
        let binding = &self.proxy_inputs[index].binding;
        let proxy = binding
            .as_ref()
            .and_then(|id| self.workspace.proxies.iter().find(|p| &p.id == id))
            .map(|p| p.settings.clone())
            .unwrap_or_default();
        serde_json::json!({"variables":rows,"proxy_id":binding,"proxy":proxy})
    }

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
        while self.proxy_inputs.len() < self.workspace.environment_inputs.len() {
            self.add_proxy_input(cx);
        }
        for index in 0..self.workspace.environment_inputs.len() {
            self.reset_environment_rows(index, cx);
        }
    }
    pub(super) fn add_proxy_input(&mut self, cx: &mut Context<Self>) {
        let binding = self.workspace.environment_inputs[self.proxy_inputs.len()]
            .proxy_id
            .clone();
        let options = self.proxy_binding_options();
        let theme = self.theme;
        let select = cx.new(|cx| {
            Select::new(options, cx)
                .selected(binding.clone().unwrap_or_default())
                .width(180.)
                .theme(theme)
        });
        let subscription = cx.subscribe(&select, |this, select, event: &SelectEvent, cx| {
            let SelectEvent::Changed(value) = event;
            if !value.is_empty()
                && !this
                    .workspace
                    .proxies
                    .iter()
                    .any(|p| p.id == value.as_ref())
            {
                return;
            }
            if let Some(input) = this
                .proxy_inputs
                .iter_mut()
                .find(|input| input.select == select)
            {
                input.binding = (!value.is_empty()).then(|| value.to_string());
                select.update(cx, |select, cx| {
                    select.set_selected(Some(value.clone()), cx)
                });
                this.variable_error = None;
                cx.notify();
            }
        });
        self.proxy_inputs.push(ProxyBindingInput {
            binding,
            select,
            _subscription: subscription,
        });
    }
    pub(super) fn merge_extracted_variables(
        &mut self,
        index: usize,
        collection: &str,
        names: &[String],
        cx: &mut Context<Self>,
    ) {
        if self.editing_variable_collection.as_deref() != Some(collection) {
            return;
        }
        let variables = self.editor_environment(index).variables;
        for variable in variables.iter().filter(|v| names.contains(&v.name)) {
            if let Some(row) = self.variable_rows[index]
                .iter()
                .find(|row| row.name.read(cx).value() == variable.name)
            {
                row.value
                    .update(cx, |entry, cx| entry.set_value(variable.value.clone(), cx));
            } else {
                let row = self.variable_input(&variable.name, &variable.value, variable.secret, cx);
                self.variable_rows[index].push(row);
            }
        }
        self.sync_variable_table(cx);
    }
    pub(super) fn reset_environment_rows(&mut self, index: usize, cx: &mut Context<Self>) {
        self.proxy_inputs[index].binding =
            self.workspace.environment_inputs[index].proxy_id.clone();
        let binding = self.proxy_inputs[index].binding.clone().unwrap_or_default();
        self.proxy_inputs[index].select.update(cx, |select, cx| {
            select.set_selected(Some(binding.into()), cx);
            select.close(cx);
        });
        let variables = self.editor_environment(index).variables;
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
    pub(super) fn environment_dirty(&self, cx: &App) -> bool {
        let saved: Vec<_> = self
            .editor_environment(self.editing_environment)
            .variables
            .iter()
            .map(|v| (v.name.clone(), v.value.clone(), v.secret))
            .collect();
        self.variable_draft(cx) != saved
            || self.proxy_inputs[self.editing_environment].binding
                != self.workspace.environment_inputs[self.editing_environment].proxy_id
    }
    pub(super) fn has_unsaved_environment(&self, cx: &App) -> bool {
        self.workspace
            .environment_inputs
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
                let variables: Vec<_> = self
                    .editor_environment(index)
                    .variables
                    .iter()
                    .map(|v| (v.name.clone(), v.value.clone(), v.secret))
                    .collect();
                let proxy = &self.proxy_inputs[index];
                rows != variables || proxy.binding != saved.proxy_id
            })
    }
    pub(super) fn add_variable(&mut self, cx: &mut Context<Self>) {
        if self.editing_environment == 0 && self.editing_variable_collection.is_none() {
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
    /// Validate all editor drafts before changing saved workspace variables.
    pub(super) fn save_workspace_environment_drafts(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let editing = self.editing_environment;
        let scope = self.editing_variable_collection.clone();
        let drafts = (|| {
            let mut drafts = Vec::new();
            for index in 0..self.workspace.environment_inputs.len() {
                self.editing_environment = index;
                let mut environment = self.editor_environment(index);
                let result = environment.replace(&self.variable_draft(cx)).and_then(|_| {
                    if scope.is_none() {
                        self.validate_proxy_binding(index)?;
                        environment.proxy_id = self.proxy_inputs[index].binding.clone();
                    }
                    Ok(())
                });
                result.map_err(|error| {
                    format!("请先修正环境“{}”：{error}", self.environment_name(index))
                })?;
                drafts.push(environment);
            }
            Ok::<_, String>(drafts)
        })();
        self.editing_environment = editing;
        for (index, environment) in drafts?.into_iter().enumerate() {
            *self.scoped_environment_mut(index, scope.as_deref())? = environment;
        }
        self.workspace.storage_dirty = true;
        self.variable_error = None;
        self.reset_variable_rows(cx);
        self.sync_reference_inputs(cx);
        Ok(())
    }
    pub(super) fn save_environment_variables(&mut self, cx: &mut Context<Self>) {
        let rows = self.variable_draft(cx);
        let index = self.editing_environment;
        let scope = self.editing_variable_collection.clone();
        match self.validate_proxy_binding(index).and_then(|_| {
            self.scoped_environment_mut(index, scope.as_deref())?
                .replace(&rows)
        }) {
            Ok(()) => {
                if scope.is_none() {
                    self.workspace.environment_inputs[index].proxy_id =
                        self.proxy_inputs[index].binding.clone();
                }
                self.workspace.storage_dirty = true;
                self.reset_environment_rows(index, cx);
                self.sync_reference_inputs(cx);
                self.ws_invalidate_titles();
                self.ws_select_sync(cx);
                self.notice = format!(
                    "已保存{}的环境设置",
                    self.environment_name(index).to_owned()
                );
                self.persist(cx);
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
                let expires_at = self
                    .editor_environment(environment)
                    .variables
                    .iter()
                    .find(|v| v.name == name.read(cx).value())
                    .and_then(|v| v.expires_at);
                let expiry_label = expires_at
                    .map(|expires| {
                        let time = chrono::DateTime::from_timestamp(expires, 0)
                            .map(|t| {
                                t.with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M:%S")
                                    .to_string()
                            })
                            .unwrap_or_else(|| expires.to_string());
                        if expires <= chrono::Utc::now().timestamp() {
                            format!("已过期 · {time}")
                        } else {
                            time
                        }
                    })
                    .unwrap_or_else(|| "不过期".into());
                let expiry_cell = TableCell::text(expiry_label);
                let name_cell = TableCell::custom(move || {
                    div()
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
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
                        .min_w_0()
                        .overflow_hidden()
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
                            .flex_shrink_0()
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
                    [name_cell, value_cell, type_cell, expiry_cell, actions],
                )
                .focus_handle(row.name.read(cx).focus_handle(cx))
            })
            .collect::<Vec<_>>();
        self.variable_table.update(cx, |table, cx| {
            table.set_theme(theme, cx);
            table.set_rows(rows, cx);
        });
    }
    fn variable_scope_tabs(&self, cx: &mut Context<Self>) -> gpui::Div {
        let mut tabs = div().flex().items_center().gap_2();
        let collection = self.editing_variable_collection.clone().or_else(|| {
            self.workspace
                .collections
                .get(self.current_collection)
                .map(|c| c.collection.clone())
        });
        for (local, scope, title) in [
            (false, None, "全局变量".to_owned()),
            (
                true,
                collection.clone(),
                format!(
                    "集合变量 · {}",
                    collection.as_deref().unwrap_or("未选择集合")
                ),
            ),
        ] {
            if local && scope.is_none() || !local && self.workspace.environment_names.len() <= 1 {
                continue;
            }
            let active = self.editing_variable_collection == scope;
            tabs = tabs.child(
                Button::new(title)
                    .kind(if active {
                        ButtonKind::Primary
                    } else {
                        ButtonKind::Gray
                    })
                    .size(ButtonSize::Mini)
                    .theme(self.theme)
                    .render()
                    .debug_selector({
                        let local = scope.is_some();
                        move || {
                            if local {
                                "variable-scope-collection".into()
                            } else {
                                "variable-scope-global".into()
                            }
                        }
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Err(error) = this.set_variable_scope(scope.clone(), cx) {
                            this.variable_error = Some(error);
                        }
                        cx.notify();
                    })),
            );
        }
        tabs.when(
            self.workspace.environment_names.len() > 1 && collection.is_some(),
            |tabs| {
                tabs.child(
                    div()
                        .debug_selector(|| "variable-scope-hint".into())
                        .text_size(px(10.))
                        .text_color(rgb(self.theme.palette().text_muted))
                        .child("集合优先于全局，仅在所属集合生效"),
                )
            },
        )
    }

    pub(super) fn begin_create_environment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.group_name
            .update(cx, |entry, cx| entry.set_value("", cx));
        self.group_edit = Some(GroupEdit::CreateEnvironment);
        self.group_error = None;
        window.focus(&self.group_name.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn environment_workspace(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        if self.workspace.environment_names.len() <= 1 && self.workspace.collections.is_empty() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .debug_selector(|| "environment-empty".into())
                .text_color(rgb(p.text_muted))
                .child("暂无环境")
                .child(
                    Button::new("创建环境")
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "empty-create-environment".into())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.begin_create_environment(window, cx)
                        })),
                );
        }
        if self.editing_environment == 0 && self.editing_variable_collection.is_none() {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .gap_4()
                .child(self.variable_scope_tabs(cx))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(p.text_muted))
                        .child(if self.workspace.environment_names.len() <= 1 {
                            "选择集合变量来管理当前集合的变量，或创建请求环境。"
                        } else {
                            "请选择环境来管理全局变量；集合变量可单独设置。"
                        }),
                );
        }
        let index = self.editing_environment;
        let applied = self.selected_environment(cx) == index;
        let dirty = self.environment_dirty(cx);
        let mut body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(self.variable_scope_tabs(cx))
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
                            .child(format!(
                                "{} · {}",
                                self.environment_name(index),
                                self.editing_variable_collection
                                    .as_ref()
                                    .map(|name| format!("集合 {name}"))
                                    .unwrap_or_else(|| "全局".into())
                            )),
                    )
                    .when(applied, |el| {
                        el.child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(self.theme.color(0x278258)))
                                .child("全局使用中"),
                        )
                    })
                    .when(!applied, |el| {
                        el.child(
                            Button::new("设为全局环境")
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
            .when(self.editing_variable_collection.is_none() && index != 0, |body| body.child(self.environment_proxy_binding(cx)))
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
                                if this.page == crate::Page::Environments {
                                    this.page = crate::Page::Requests;
                                }
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

impl Workbench {
    pub(super) fn environment_name(&self, index: usize) -> &str {
        self.workspace
            .environment_names
            .get(index)
            .map(String::as_str)
            .unwrap_or("不使用环境")
    }

    pub(super) fn reference_resolver(&self, cx: &App) -> TextReferenceResolver {
        self.request_reference_resolver(
            &self.snapshot(cx),
            self.workspace
                .requests
                .get(self.active)
                .map(|request| request.collection.as_str()),
            cx,
        )
    }

    pub(super) fn request_reference_resolver(
        &self,
        draft: &Draft,
        collection: Option<&str>,
        cx: &App,
    ) -> TextReferenceResolver {
        let values = self.environment_values(self.selected_environment(cx), collection);
        let resolver = template::TemplateResolver::for_preview(&values, draft);
        std::rc::Rc::new(move |expression| resolver.resolve(expression))
    }

    pub(super) fn sync_reference_inputs(&mut self, cx: &mut Context<Self>) {
        let resolver = self.reference_resolver(cx);
        let mut completions = self.environment_completions(cx);
        let oauth_completions = completions.clone();
        let draft = self.snapshot(cx);
        for cookie in draft
            .cookies
            .iter()
            .filter(|pair| pair.enabled && !pair.key.trim().is_empty())
        {
            completions.push(TextCompletion::new(
                format!(
                    "cookie('{}')",
                    cookie.key.replace('\\', "\\\\").replace('\'', "\\'")
                ),
                "读取请求 Cookie",
            ));
        }
        for row in self
            .params
            .iter()
            .chain(self.headers.iter())
            .chain(self.cookies.iter())
            .chain(self.form.iter())
        {
            for entry in [&row.key, &row.value] {
                entry.update(cx, |entry, cx| {
                    entry.set_reference_resolver(Some(resolver.clone()), cx);
                    entry.set_completions(completions.clone(), cx)
                });
            }
        }
        let oauth_resolver = template::TemplateResolver::new(&self.active_environment_values(cx));
        let oauth_resolver: TextReferenceResolver =
            std::rc::Rc::new(move |expression| oauth_resolver.resolve(expression));
        for entry in self
            .oauth
            .fields
            .iter()
            .chain(std::iter::once(&self.oauth.secret))
        {
            entry.update(cx, |entry, cx| {
                entry.set_reference_resolver(Some(oauth_resolver.clone()), cx);
                entry.set_completions(oauth_completions.clone(), cx)
            });
        }
        for field in [&self.endpoint, &self.authentication] {
            field.update(cx, |field, cx| {
                field.set_reference_resolver(Some(resolver.clone()), cx);
                field.set_completions(completions.clone(), cx)
            });
        }
        self.body.update(cx, |body, cx| {
            body.set_reference_resolver(Some(resolver.clone()), cx);
            body.set_completions(completions.clone(), cx);
        });
        if let Some(editor) = &self.directory_settings {
            let collection = self
                .workspace
                .collections
                .get(editor.read(cx).collection)
                .map(|c| c.collection.as_str());
            let completions = self.environment_completions_for(cx, collection);
            editor.update(cx, |editor, cx| {
                editor.set_context(self.theme, completions, cx)
            });
        }
    }

    pub(super) fn selected_environment(&self, cx: &App) -> usize {
        self.environment
            .read(cx)
            .value()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0)
    }

    pub(super) fn environment_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let applied = self.selected_environment(cx);
        let mut list = div().flex().flex_col().gap_2().px_2().py_3();
        for (index, name) in self.workspace.environment_names.iter().enumerate().skip(1) {
            let selected = self.editing_environment == index;
            let scoped = self.editor_environment(index);
            let base = scoped
                .variables
                .iter()
                .find(|variable| variable.name == "base_url")
                .map(|variable| variable.value.as_str())
                .unwrap_or("");
            let host = reqwest::Url::parse(base)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_else(|| "未配置服务地址".into());
            let caption = format!("{} 个变量", scoped.variables.len());
            list = list.child(
                div()
                    .id(("environment-profile", index))
                    .px_3()
                    .py_3()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(if selected { p.accent } else { p.panel }))
                    .bg(rgb(if selected { p.accent_tint } else { p.panel }))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(p.accent_tint)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(if index == 2 {
                                Icon::Code.render(15.)
                            } else {
                                Icon::Globe.render(15.)
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(name.clone()),
                            )
                            .when(index == applied, |row| {
                                row.child(
                                    div()
                                        .text_size(px(9.))
                                        .text_color(rgb(p.accent))
                                        .child("使用中"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .mt_2()
                            .font_family("Menlo")
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .truncate()
                            .child(host),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(caption),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.editing_environment = index;
                        this.revealed_variables.clear();
                        this.variable_delete = None;
                        this.variable_error = None;
                        this.variable_search
                            .update(cx, |entry, cx| entry.set_value("", cx));
                        cx.notify();
                    })),
            );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(p.panel))
            .child(
                div()
                    .h(px(44.))
                    .flex_shrink_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("请求环境"))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(format!(
                                "{} 个环境",
                                self.workspace.environment_names.len() - 1
                            )),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .id("environment-list-scroll")
                    .overflow_y_scroll()
                    .child(list)
                    .when(
                        self.workspace.environment_names.len() > 1
                            || !self.workspace.collections.is_empty(),
                        |list| {
                            list.child(
                                Button::new("创建环境")
                                    .icon(Icon::Plus)
                                    .kind(ButtonKind::Gray)
                                    .size(ButtonSize::Mini)
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(|| "sidebar-create-environment".into())
                                    .mx_2()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.begin_create_environment(window, cx);
                                    })),
                            )
                        },
                    ),
            )
            .child(
                div()
                    .p_3()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("全局当前环境")
                    .child(
                        div()
                            .mt_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_color(rgb(p.text))
                            .child(div().size(px(6.)).rounded_full().bg(rgb(p.accent)))
                            .child(self.environment_name(applied).to_owned()),
                    ),
            )
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    #[gpui::test]
    fn scoped_variables_edit_override_extract_and_persist_without_cross_collection_leaks(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("scopes.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            for name in ["A", "B"] {
                host.workspace.collections.push(
                    RequestGroups::create_collection(name, &host.workspace.collections).unwrap(),
                );
            }
            host.current_collection = 0;
            host.add_request(cx);
            host.workspace.environment_names.push("Dev".into());
            host.workspace.environment_inputs.push(Default::default());
            host.variable_rows.push(vec![]);
            host.add_proxy_input(cx);
            host.workspace.environment_inputs[1]
                .save(None, "host", "https://global.test", false)
                .unwrap();
            host.workspace.environment_inputs[1]
                .save(None, "token", "global-secret", true)
                .unwrap();
            host.scoped_environment_mut(1, Some("A"))
                .unwrap()
                .save(None, "host", "https://a.test", false)
                .unwrap();
            host.scoped_environment_mut(1, Some("A"))
                .unwrap()
                .save(None, "only_a", "A", false)
                .unwrap();
            host.reset_variable_rows(cx);
            host.page = crate::Page::Environments;
            host.editing_environment = 1;
            host
        });
        let local = window.debug_bounds("variable-scope-collection").unwrap();
        window.simulate_click(local.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert_eq!(host.editing_variable_collection.as_deref(), Some("A"));
            assert_eq!(host.editor_environment(1).variables.len(), 2);
            host.variable_rows[1][0]
                .value
                .update(cx, |entry, cx| entry.set_value("https://unsaved.test", cx));
            assert_eq!(
                host.environment_values(1, Some("A")).0["host"],
                "https://a.test"
            );
            assert!(host.set_variable_scope(None, cx).is_err());
            host.save_environment_variables(cx);
            assert_eq!(
                host.environment_values(1, Some("A")).0["host"],
                "https://unsaved.test"
            );
            assert_eq!(
                host.environment_values(1, Some("B")).0["host"],
                "https://global.test"
            );
            assert!(
                !host
                    .environment_values(1, Some("B"))
                    .0
                    .contains_key("only_a")
            );
            host.set_variable_scope(None, cx).unwrap();
            // A response from A must still write A/Dev after the UI moves to B/no environment.
            host.current_collection = 1;
            host.add_request(cx);
            let draft = Draft {
                extractors: vec![Pair::new("token", "$.token", "")],
                ..Draft::default()
            };
            let response = HttpResponse {
                status: 200,
                headers: vec![],
                text: String::new(),
                bytes: 0,
                json: Some(serde_json::json!({"token":"local-secret"})),
            };
            host.apply_extractors(&draft, 1, "A", &response, cx)
                .unwrap();
            assert_eq!(
                host.environment_values(1, Some("A")).0["token"],
                "local-secret"
            );
            assert_eq!(
                host.environment_values(1, Some("B")).0["token"],
                "global-secret"
            );
            assert_eq!(
                host.workspace.environment_inputs[1].values().0["token"],
                "global-secret"
            );
            let token = host.workspace.collections[0].environments["Dev"]
                .variables
                .iter()
                .find(|v| v.name == "token")
                .unwrap();
            assert!(token.secret);
            assert!(host.workspace.collections[1].environments.is_empty());
            let bad = Draft {
                extractors: vec![
                    Pair::new("token", "$.token", ""),
                    Pair::new("missing", "$.missing", ""),
                ],
                ..Draft::default()
            };
            assert!(host.apply_extractors(&bad, 1, "A", &response, cx).is_err());
            assert_eq!(
                host.environment_values(1, Some("A")).0["token"],
                "local-secret"
            );
            host.apply_extractors(&draft, 0, "A", &response, cx)
                .unwrap();
            assert!(host.workspace.environment_inputs[0].variables.is_empty());
            assert!(
                !host
                    .environment_values(0, Some("B"))
                    .0
                    .contains_key("token")
            );
            RequestGroups::rename_collection(
                0,
                "Renamed",
                &mut host.workspace.collections,
                &mut host.workspace.requests,
            )
            .unwrap();
            assert_eq!(host.environment_values(1, Some("Renamed")).0["only_a"], "A");
            let origin_id = host.workspace.collections[0].id.clone();
            host.workspace
                .collections
                .push(RequestGroups::create_collection("A", &host.workspace.collections).unwrap());
            host.apply_extractors_for_id(&draft, 1, &origin_id, &response, cx)
                .unwrap();
            assert!(host.workspace.collections[2].environments.is_empty());
            assert_eq!(
                host.environment_values(1, Some("Renamed")).0["token"],
                "local-secret"
            );

            host.persist(cx);
        });
        let mut store = storage::Store::open(&path).unwrap();
        let saved = store.load().unwrap();
        saved.validate().unwrap();
        assert_eq!(
            saved.collections[0].environments["Dev"].values().0["token"],
            "local-secret"
        );
        assert!(saved.collections[1].environments.is_empty());
        // Legacy collection JSON loads with an empty collection scope.
        let mut legacy = serde_json::to_value(&saved.collections[0]).unwrap();
        legacy.as_object_mut().unwrap().remove("environments");
        legacy.as_object_mut().unwrap().remove("id");
        let legacy: RequestGroups = serde_json::from_value(legacy).unwrap();
        assert!(legacy.environments.is_empty());
    }
}
