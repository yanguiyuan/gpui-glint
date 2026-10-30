use super::*;

impl Workbench {
    pub(super) fn workspace_label(&self) -> String {
        self.workspace_list
            .iter()
            .find(|w| w.id == self.workspace.storage.workspace_id)
            .map(|w| w.name.clone())
            .unwrap_or_else(|| "工作区".into())
    }
    pub(super) fn workspace_trigger(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        div()
            .id("workspace-switcher")
            .debug_selector(|| "workspace-switcher".into())
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(px(6.))
            .max_w(px(180.))
            .flex_shrink_0()
            .overflow_hidden()
            .cursor_pointer()
            .hover(|s| s.bg(rgb(self.theme.palette().accent_tint)))
            .child(div().min_w_0().truncate().child(self.workspace_label()))
            .child(Icon::ChevronDown.render(12.).flex_shrink_0())
            .on_click(cx.listener(|this, _, window, cx| {
                match this.workspace.storage.workspaces() {
                    Ok(list) => {
                        this.workspace_list = list;
                        this.workspace_error = None;
                    }
                    Err(error) => this.workspace_error = Some(error),
                }
                this.show_workspaces = true;
                this.workspace_edit = None;
                this.workspace_delete = None;
                this.workspace_name_input
                    .update(cx, |e, cx| e.set_value("", cx));
                window.focus(&this.workspace_name_input.read(cx).focus_handle(cx), cx);
                cx.notify();
            }))
    }
    pub(super) fn workspace_switch_ready(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        if self.workspace_transfer_task.is_some() {
            return Err("工作区导入或导出完成后可切换工作区".into());
        }
        self.flush_request_editors(cx);
        if self.request_editors.values().any(|shared| shared.running) {
            return Err("请先结束独立窗口中的请求或 WebSocket 连接，再切换工作区".into());
        }
        if self
            .ws_background
            .values()
            .any(|state| state.active || state.connected)
            || self.sending.is_some()
            || self.ws_active
            || self.ws_connected
            || self.oauth.is_busy()
        {
            return Err("请先结束当前请求、WebSocket 连接或授权，再切换工作区".into());
        }
        if self.registry_task.is_some() {
            return Err("插件操作完成后可切换工作区".into());
        }
        if self.proxy_certificate_task.is_some() {
            return Err("代理证书操作完成后可切换工作区".into());
        }
        if self.directory_settings.is_some()
            || self.group_edit.is_some()
            || self.editing_request_name
        {
            return Err("请先完成或关闭当前编辑，再切换工作区".into());
        }
        let proxies = self.proxy_drafts(cx)?;
        if self.has_unsaved_environment(cx) {
            self.save_workspace_environment_drafts(cx)?;
        }
        if self.workspace.proxies != proxies {
            self.workspace.proxies = proxies;
            self.workspace.storage_dirty = true;
        }
        self.persist(cx);
        if let Some(error) = &self.workspace.storage_error {
            return Err(format!("当前工作区保存失败：{error}"));
        }
        Ok(())
    }
    pub(super) fn switch_workspace(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.switch_workspace_with_settings(id, false, window, cx)
    }
    pub(super) fn switch_workspace_with_settings(
        &mut self,
        id: &str,
        restore_settings: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if id == self.workspace.storage.workspace_id {
            self.show_workspaces = false;
            window.focus(&self.focus, cx);
            cx.notify();
            return Ok(());
        }
        self.workspace_switch_ready(cx)?;
        let (store, data) = self.workspace.storage.prepare_workspace(id)?;
        store.select_workspace()?;
        let mut next = Self::with_workspace(store, data, cx);
        // Keep application-level services alive; request/editor entities belong to the workspace.
        next.host_shell = self.host_shell.clone();
        next.mcp_server = self.mcp_server.take();
        next.mcp_task = self.mcp_task.take();
        next.mcp_error = self.mcp_error.take();
        next.update_task = self.update_task.take();
        next.update_state = std::mem::replace(&mut self.update_state, update_ui::UpdateState::Idle);
        next.show_update_details = self.show_update_details;
        next.update_error = self.update_error.take();
        if restore_settings {
            next.update_state = update_ui::UpdateState::Idle;
            next.update_error = None;
        } else {
            next.workspace.update_source = self.workspace.update_source.clone();
            next.update_source_url.update(cx, |e, cx| {
                e.set_value(self.update_source_url.read(cx).value().to_owned(), cx)
            });
            next.update_source_key.update(cx, |e, cx| {
                e.set_value(self.update_source_key.read(cx).value().to_owned(), cx)
            });
        }
        next.sidebar_width = self.sidebar_width;
        next.show_activity = self.show_activity;
        next.set_theme(self.theme, cx);
        next.workspace.storage_dirty = true;
        *self = next;
        self.persist(cx);
        window.focus(&self.focus, cx);
        cx.notify();
        Ok(())
    }
    pub(super) fn submit_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.workspace_name_input.read(cx).value().to_owned();
        let result = if let Some(id) = self.workspace_edit.clone() {
            self.workspace
                .storage
                .rename_workspace(&id, &name)
                .and_then(|_| {
                    self.workspace_list = self.workspace.storage.workspaces()?;
                    self.workspace_edit = None;
                    self.workspace_name_input
                        .update(cx, |e, cx| e.set_value("", cx));
                    Ok(())
                })
        } else {
            self.workspace_switch_ready(cx)
                .and_then(|_| self.workspace.storage.create_workspace(&name))
                .and_then(|id| self.switch_workspace(&id, window, cx))
        };
        self.workspace_error = result.err();
        cx.notify();
    }
    fn confirm_workspace_deletion(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let list = self.workspace.storage.workspaces()?;
        if list.len() <= 1 {
            return Err("至少保留一个工作区，无法删除最后一个工作区".into());
        }
        if !list.iter().any(|w| w.id == id) {
            return Err("工作区不存在".into());
        }
        if id == self.workspace.storage.workspace_id {
            let next = list.iter().find(|w| w.id != id).unwrap();
            self.switch_workspace(&next.id, window, cx)?;
            self.show_workspaces = true;
        }
        self.workspace.storage.delete_workspace(id)?;
        self.workspace_list = self.workspace.storage.workspaces()?;
        self.workspace_delete = None;
        if self.workspace_edit.as_deref() == Some(id) {
            self.workspace_edit = None;
            self.workspace_name_input
                .update(cx, |e, cx| e.set_value("", cx));
        }
        self.notice = "已删除工作区".into();
        Ok(())
    }

    pub(super) fn workspace_dialog(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.show_workspaces {
            return None;
        }
        let p = self.theme.palette();
        let mut list = div()
            .id("workspace-list")
            .max_h(px(260.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2();
        for workspace in &self.workspace_list {
            let id = workspace.id.clone();
            let edit_id = id.clone();
            let delete_id = id.clone();
            let name = workspace.name.clone();
            let selected = id == self.workspace.storage.workspace_id;
            let mut row = div()
                .id(format!("workspace-row-{id}"))
                .flex()
                .items_center()
                .gap_2()
                .p_2()
                .rounded(px(8.))
                .bg(rgb(if selected { p.accent_tint } else { p.panel }))
                .child(
                    div()
                        .id(format!("switch-{id}"))
                        .debug_selector({
                            let id = id.clone();
                            move || format!("switch-{id}")
                        })
                        .flex_1()
                        .min_w_0()
                        .cursor_pointer()
                        .child(div().text_size(px(12.)).truncate().child(name.clone()))
                        .when(selected || workspace.id == "default", |row| {
                            row.child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.text_muted))
                                    .child(if selected && workspace.id == "default" {
                                        "当前 · 默认工作区"
                                    } else if selected {
                                        "当前工作区"
                                    } else {
                                        "默认工作区"
                                    }),
                            )
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.workspace_error = this.switch_workspace(&id, window, cx).err();
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("重命名")
                        .kind(ButtonKind::Plain)
                        .size(ButtonSize::Small)
                        .theme(self.theme)
                        .render()
                        .debug_selector({
                            let id = edit_id.clone();
                            move || format!("rename-workspace-{id}")
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.workspace_edit = Some(edit_id.clone());
                            this.workspace_delete = None;
                            this.workspace_error = None;
                            this.workspace_name_input
                                .update(cx, |e, cx| e.set_value(name.clone(), cx));
                            window.focus(&this.workspace_name_input.read(cx).focus_handle(cx), cx);
                            cx.notify();
                        })),
                );
            {
                row = row.child(
                    Button::new("删除")
                        .kind(ButtonKind::Plain)
                        .size(ButtonSize::Small)
                        .theme(self.theme)
                        .when(self.workspace_list.len() <= 1, |button| button.disabled())
                        .render()
                        .debug_selector({
                            let id = delete_id.clone();
                            move || format!("delete-workspace-{id}")
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.workspace_list.len() <= 1 {
                                return;
                            }
                            this.workspace_delete = Some(delete_id.clone());
                            this.workspace_error = None;
                            cx.notify();
                        })),
                );
            }
            list = list.child(row);
        }
        let mut form = div()
            .id("workspace-dialog")
            .debug_selector(|| "workspace-dialog".into())
            .flex()
            .flex_col()
            .gap_4()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::MEDIUM)
                            .child("工作区"),
                    )
                    .child(
                        Button::new("关闭")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Small)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.show_workspaces = false;
                                window.focus(&this.focus, cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("按项目隔离请求、环境、版本与历史，切换前自动保存。"),
            )
            .child(list);
        form = form.child(
            div()
                .flex()
                .gap_2()
                .child(
                    Button::new("导入工作区")
                        .kind(ButtonKind::Plain)
                        .size(ButtonSize::Small)
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "open-workspace-import".into())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_workspace_transfer(
                                workspace_transfer_ui::Mode::Import,
                                window,
                                cx,
                            )
                        })),
                )
                .child(
                    Button::new("导出当前工作区")
                        .kind(ButtonKind::Plain)
                        .size(ButtonSize::Small)
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "open-workspace-export".into())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_workspace_transfer(
                                workspace_transfer_ui::Mode::Export,
                                window,
                                cx,
                            )
                        })),
                ),
        );
        if let Some(id) = self.workspace_delete.clone() {
            let name = self
                .workspace_list
                .iter()
                .find(|w| w.id == id)
                .map(|w| w.name.as_str())
                .unwrap_or("");
            form = form.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .bg(rgb(p.panel))
                    .rounded(px(8.))
                    .child(format!(
                        "删除“{name}”及其中所有请求、环境和历史？此操作无法撤销。"
                    ))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("确认删除")
                                    .size(ButtonSize::Small)
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(|| "confirm-delete-workspace".into())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        let result =
                                            this.confirm_workspace_deletion(&id, window, cx);
                                        this.workspace_error = result.err();
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("取消")
                                    .kind(ButtonKind::Plain)
                                    .size(ButtonSize::Small)
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(|| "cancel-delete-workspace".into())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.workspace_delete = None;
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        form = form
            .child(
                div()
                    .text_size(px(12.))
                    .child(if self.workspace_edit.is_some() {
                        "重命名工作区"
                    } else {
                        "新建工作区"
                    }),
            )
            .child(self.workspace_name_input.clone());
        if let Some(error) = &self.workspace_error {
            form = form.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(error.clone()),
            );
        }
        form = form.child(
            div()
                .flex()
                .justify_end()
                .gap_2()
                .when(self.workspace_edit.is_some(), |row| {
                    row.child(
                        Button::new("取消重命名")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Small)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.workspace_edit = None;
                                this.workspace_name_input
                                    .update(cx, |e, cx| e.set_value("", cx));
                                cx.notify();
                            })),
                    )
                })
                .child(
                    Button::new(if self.workspace_edit.is_some() {
                        "保存名称"
                    } else {
                        "创建并切换"
                    })
                    .size(ButtonSize::Small)
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "submit-workspace".into())
                    .on_click(cx.listener(|this, _, window, cx| this.submit_workspace(window, cx))),
                ),
        );
        Some(
            Dialog::new("workspace_ui-modal", form)
                .focus_handles([self.workspace_name_input.read(cx).focus_handle(cx)])
                .theme(self.theme)
                .width(520.)
                .priority(120)
                .on_close(cx.listener(|this, _, window, cx| {
                    this.show_workspaces = false;
                    window.focus(&this.focus, cx);
                    cx.notify();
                }))
                .render(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn switcher_creates_isolates_and_restores_unsaved_request_drafts(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |e, cx| e.set_value("原项目", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.body
                .update(cx, |e, cx| e.set_content("unsaved body", cx));
            host
        });
        let trigger = window.debug_bounds("workspace-switcher").unwrap();
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        assert!(window.debug_bounds("workspace-dialog").is_some());
        host.update(window, |host, cx| {
            host.workspace_name_input
                .update(cx, |e, cx| e.set_value("项目 B", cx))
        });
        let submit = window.debug_bounds("submit-workspace").unwrap();
        window.simulate_click(submit.center(), gpui::Modifiers::default());
        assert!(window.debug_bounds("workspace-dialog").is_none());
        let (second_id, second_namespace) = host.update(window, |host, cx| {
            assert!(host.workspace.requests.is_empty());
            assert!(host.workspace.collections.is_empty());
            assert_eq!(host.workspace.environment_names, ["不使用环境"]);
            assert_eq!(host.workspace_label(), "项目 B");
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |e, cx| e.set_value("另一个项目", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.body.update(cx, |e, cx| e.set_content("B body", cx));
            (
                host.workspace.storage.workspace_id.clone(),
                host.oauth_namespace.clone(),
            )
        });
        let trigger = window.debug_bounds("workspace-switcher").unwrap();
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        let original = window.debug_bounds("switch-default").unwrap();
        window.simulate_click(original.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert_eq!(host.workspace.storage.workspace_id, "default");
            assert_eq!(host.workspace.collections[0].collection, "原项目");
            assert_eq!(host.snapshot(cx).body, "unsaved body");
            assert_ne!(host.oauth_namespace, second_namespace);
            // A live transport must prevent replacement of its workspace/editor state.
            host.ws_active = true;
        });
        let trigger = window.debug_bounds("workspace-switcher").unwrap();
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        let second = window
            .debug_bounds(Box::leak(format!("switch-{second_id}").into_boxed_str()))
            .unwrap();
        window.simulate_click(second.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert_eq!(host.workspace.storage.workspace_id, "default");
            assert!(host.workspace_error.as_ref().unwrap().contains("先结束"));
            assert!(host.show_workspaces);
            host.ws_active = false;
        });
        let second = window
            .debug_bounds(Box::leak(format!("switch-{second_id}").into_boxed_str()))
            .unwrap();
        window.simulate_click(second.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert_eq!(host.workspace.storage.workspace_id, second_id);
            assert_eq!(host.snapshot(cx).body, "B body");
            assert_eq!(host.workspace.collections[0].collection, "另一个项目");
        });
        let mut reopened = storage::Store::open(&path).unwrap();
        assert_eq!(reopened.workspace_id, second_id);
        assert_eq!(reopened.load().unwrap().requests[0].draft.body, "B body");
    }

    #[gpui::test]
    fn hidden_environment_drafts_validate_atomically_and_save_all_environments(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite3");
        let host = cx.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        host.update(cx, |host, cx| {
            for name in ["A", "B"] {
                let mut variables = EnvironmentVariables::default();
                variables.save(None, "host", "original", false).unwrap();
                host.workspace.environment_names.push(name.into());
                host.workspace.environment_inputs.push(variables);
                host.variable_rows.push(vec![]);
                host.add_proxy_input(cx);
            }
            host.reset_variable_rows(cx);
            host.variable_rows[1][0]
                .value
                .update(cx, |e, cx| e.set_value("edited A", cx));
            host.variable_rows[2][0]
                .name
                .update(cx, |e, cx| e.set_value("", cx));
            host.variable_rows[2][0]
                .value
                .update(cx, |e, cx| e.set_value("edited B", cx));
            assert!(host.page != crate::Page::Environments);
            assert!(host.save_workspace_environment_drafts(cx).is_err());
            assert_eq!(host.editing_environment, 0);
            assert_eq!(
                host.workspace.environment_inputs[1].variables[0].value,
                "original"
            );
            host.variable_rows[2][0]
                .name
                .update(cx, |e, cx| e.set_value("host", cx));
            host.save_workspace_environment_drafts(cx).unwrap();
            assert!(!host.has_unsaved_environment(cx));
            assert_eq!(
                host.workspace.environment_inputs[1].variables[0].value,
                "edited A"
            );
            assert_eq!(
                host.workspace.environment_inputs[2].variables[0].value,
                "edited B"
            );
            host.persist(cx);
            assert!(host.workspace.storage_error.is_none());
        });
        let data = storage::Store::open(&path).unwrap().load().unwrap();
        assert_eq!(data.variables[1].variables[0].value, "edited A");
        assert_eq!(data.variables[2].variables[0].value, "edited B");
    }

    #[gpui::test]
    fn save_failure_keeps_current_workspace_and_does_not_create_another(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            Workbench::with_store(
                storage::Store::open(&dir.path().join("test.sqlite3")).unwrap(),
                cx,
            )
        });
        host.update(window,|host,cx| {
            host.workspace.storage_dirty=true;
            host.persist(cx);
            host.workspace.storage.connection.execute_batch("CREATE TRIGGER reject_workspace_save BEFORE UPDATE ON workspace BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
            host.workspace.environment_names.push("unsaved".into());
            host.workspace.environment_inputs.push(EnvironmentVariables::default());
            host.variable_rows.push(vec![]);
            host.add_proxy_input(cx);
            host.reset_variable_rows(cx);
            host.workspace.storage_dirty=true;
        });
        let trigger = window.debug_bounds("workspace-switcher").unwrap();
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            host.workspace_name_input
                .update(cx, |e, cx| e.set_value("B", cx))
        });
        let submit = window.debug_bounds("submit-workspace").unwrap();
        window.simulate_click(submit.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert_eq!(host.workspace.storage.workspace_id, "default");
            assert_eq!(host.workspace.storage.workspaces().unwrap().len(), 1);
            assert!(host.workspace_error.as_ref().unwrap().contains("保存失败"));
            assert!(host.workspace.storage_dirty);
        });
    }

    #[gpui::test]
    fn workspace_management_renames_and_confirms_deletion(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite3");
        let mut store = storage::Store::open(&path).unwrap();
        let other_id = store.create_workspace("B").unwrap();
        let (host, window) = cx.add_window_view(|_, cx| Workbench::with_store(store, cx));
        let trigger = window.debug_bounds("workspace-switcher").unwrap();
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        let rename = window.debug_bounds("rename-workspace-default").unwrap();
        window.simulate_click(rename.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            host.workspace_name_input
                .update(cx, |e, cx| e.set_value("我的项目", cx))
        });
        let submit = window.debug_bounds("submit-workspace").unwrap();
        window.simulate_click(submit.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert_eq!(host.workspace_label(), "我的项目")
        });
        let rename = window
            .debug_bounds(Box::leak(
                format!("rename-workspace-{other_id}").into_boxed_str(),
            ))
            .unwrap();
        window.simulate_click(rename.center(), gpui::Modifiers::default());
        let delete = window
            .debug_bounds(Box::leak(
                format!("delete-workspace-{other_id}").into_boxed_str(),
            ))
            .unwrap();
        window.simulate_click(delete.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert_eq!(host.workspace.storage.workspaces().unwrap().len(), 2);
            assert_eq!(host.workspace_delete.as_deref(), Some(other_id.as_str()));
        });
        let confirm = window.debug_bounds("confirm-delete-workspace").unwrap();
        window.simulate_click(confirm.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert_eq!(host.workspace.storage.workspaces().unwrap().len(), 1);
            assert!(host.workspace_edit.is_none());
            assert!(host.workspace_delete.is_none());
            assert!(host.workspace_error.is_none());
        });
    }

    #[gpui::test]
    fn deleting_current_default_requires_confirmation_and_preserves_last_workspace(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite3");
        let mut store = storage::Store::open(&path).unwrap();
        let other_id = store.create_workspace("保留工作区").unwrap();
        let (host, window) = cx.add_window_view(|_, cx| Workbench::with_store(store, cx));
        let trigger = window.debug_bounds("workspace-switcher").unwrap();
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        for confirm in [false, true] {
            let delete = window.debug_bounds("delete-workspace-default").unwrap();
            window.simulate_click(delete.center(), gpui::Modifiers::default());
            host.update(window, |h, _| {
                assert_eq!(h.workspace.storage.workspace_id, "default");
                assert_eq!(h.workspace.storage.workspaces().unwrap().len(), 2);
            });
            let action = window
                .debug_bounds(if confirm {
                    "confirm-delete-workspace"
                } else {
                    "cancel-delete-workspace"
                })
                .unwrap();
            window.simulate_click(action.center(), gpui::Modifiers::default());
        }
        host.update(window, |h, _| {
            assert_eq!(h.workspace.storage.workspace_id, other_id);
            assert_eq!(h.workspace_list.len(), 1);
            assert!(h.workspace_error.is_none());
            assert!(h.workspace_delete.is_none());
        });
        let selector: &'static str =
            Box::leak(format!("delete-workspace-{other_id}").into_boxed_str());
        let delete = window.debug_bounds(selector).unwrap();
        window.simulate_click(delete.center(), gpui::Modifiers::default());
        host.update(window, |h, cx| {
            assert!(h.workspace_delete.is_none());
            // Also reject a stale confirmation and a direct storage call.
            assert!(h.workspace.storage.delete_workspace(&other_id).is_err());
            h.workspace_delete = Some(other_id.clone());
            cx.notify();
        });
        let confirm = window.debug_bounds("confirm-delete-workspace").unwrap();
        window.simulate_click(confirm.center(), gpui::Modifiers::default());
        host.update(window, |h, _| {
            assert!(
                h.workspace_error
                    .as_deref()
                    .unwrap()
                    .contains("至少保留一个工作区")
            );
            assert_eq!(h.workspace.storage.workspaces().unwrap().len(), 1);
        });
        let reopened = storage::Store::open(&path).unwrap();
        assert_eq!(reopened.workspace_id, other_id);
        assert_eq!(reopened.workspaces().unwrap().len(), 1);
        assert!(
            reopened
                .workspaces()
                .unwrap()
                .iter()
                .all(|w| w.id != "default")
        );
    }

    #[gpui::test]
    fn invalid_names_and_escape_keep_workspace_unchanged(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            Workbench::with_store(
                storage::Store::open(&dir.path().join("test.sqlite3")).unwrap(),
                cx,
            )
        });
        let trigger = window.debug_bounds("workspace-switcher").unwrap();
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        let submit = window.debug_bounds("submit-workspace").unwrap();
        window.simulate_click(submit.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert_eq!(host.workspace.storage.workspace_id, "default");
            assert!(host.workspace_error.is_some());
            assert_eq!(host.workspace_list.len(), 1);
        });
        window.simulate_keystrokes("escape");
        assert!(window.debug_bounds("workspace-dialog").is_none());
    }
}
