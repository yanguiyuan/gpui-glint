use super::workspace_transfer::{Archive, Contents, Section};
use super::*;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Import,
    Export,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn click(window: &mut gpui::VisualTestContext, selector: &'static str) {
        let bounds = window.debug_bounds(selector).unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
    }
    #[gpui::test]
    fn dialogs_export_live_drafts_and_import_into_an_isolated_workspace(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&database).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |e, cx| e.set_value("迁移项目", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.group_edit = Some(GroupEdit::CreateEnvironment);
            host.group_name.update(cx, |e, cx| e.set_value("beta", cx));
            host.save_group(cx);
            host.add_variable(cx);
            host.variable_rows[1][0]
                .name
                .update(cx, |e, cx| e.set_value("secret", cx));
            host.variable_rows[1][0]
                .value
                .update(cx, |e, cx| e.set_value("unsaved environment value", cx));
            host.body
                .update(cx, |e, cx| e.set_content("unsaved migration body", cx));
            host
        });
        click(window, "workspace-switcher");
        click(window, "open-workspace-export");
        assert!(window.debug_bounds("workspace-transfer-dialog").is_some());
        host.read_with(window, |host, _| {
            assert_eq!(
                host.workspace_transfer.as_ref().unwrap().selected,
                Contents::all()
            )
        });
        click(window, "transfer-select-all");
        host.read_with(window, |host, _| {
            assert!(
                host.workspace_transfer
                    .as_ref()
                    .unwrap()
                    .selected
                    .0
                    .is_empty()
            )
        });
        click(window, "transfer-select-all");
        let archive = host.update(window, |host, cx| {
            host.workspace_archive(Contents::all(), cx).unwrap()
        });
        assert_eq!(
            archive.data.requests[0].draft.body,
            "unsaved migration body"
        );
        assert_eq!(
            archive.data.variables[1].variables[0].value,
            "unsaved environment value"
        );
        let file = directory.path().join("workspace.glint-workspace.json");
        archive.write(&file).unwrap();
        click(window, "cancel-workspace-transfer");
        for query in ["导出当前工作区", "导入工作区"] {
            window.update(|window, cx| host.update(cx, |host, cx| host.open_commands(window, cx)));
            host.update(window, |host, cx| {
                host.commands
                    .update(cx, |palette, cx| palette.set_query(query, cx));
            });
            window.simulate_keystrokes("enter");
            assert!(window.debug_bounds("workspace-transfer-dialog").is_some());
            host.read_with(window, |host, _| {
                assert!(
                    host.workspace_transfer.as_ref().unwrap().mode
                        == if query.starts_with("导出") {
                            Mode::Export
                        } else {
                            Mode::Import
                        }
                )
            });
            if query.starts_with("导出") {
                click(window, "cancel-workspace-transfer");
            }
        }
        let loaded = Archive::read(&file).unwrap();
        window.update(|window, cx| {
            host.update(cx, |host, cx| {
                host.set_workspace_import(file, loaded, cx);
                // Test that replacing a live workspace is rejected without creating one.
                host.ws_active = true;
                host.import_workspace_archive(window, cx);
                assert_eq!(host.workspace.storage.workspaces().unwrap().len(), 1);
                assert!(
                    host.workspace_transfer
                        .as_ref()
                        .unwrap()
                        .error
                        .as_ref()
                        .unwrap()
                        .contains("先结束")
                );
                host.ws_active = false;
                assert_ne!(
                    host.workspace_transfer_name.read(cx).value(),
                    host.workspace_label()
                );
            })
        });
        click(window, "transfer-section-Requests");
        host.read_with(window, |host, _| {
            let selected = host
                .workspace_transfer
                .as_ref()
                .unwrap()
                .selected
                .effective();
            assert!(!selected.has(Section::Versions) && !selected.has(Section::History));
        });
        click(window, "transfer-section-Requests");
        click(window, "submit-workspace-transfer");
        host.read_with(window, |host, cx| {
            assert_ne!(host.workspace.storage.workspace_id, "default");
            assert!(host.workspace_transfer.is_none());
            assert_eq!(host.snapshot(cx).body, "unsaved migration body");
            assert_eq!(
                host.workspace.environment_inputs[1].variables[0].value,
                "unsaved environment value"
            );
        });
        window.update(|window, cx| {
            host.update(cx, |host, cx| {
                host.switch_workspace("default", window, cx).unwrap()
            })
        });
        host.read_with(window, |host, cx| {
            assert_eq!(host.snapshot(cx).body, "unsaved migration body")
        });
        assert_eq!(
            storage::Store::open(&database)
                .unwrap()
                .workspaces()
                .unwrap()
                .len(),
            2
        );
    }
}
pub(super) struct TransferDialog {
    mode: Mode,
    selected: Contents,
    archive: Option<Arc<Archive>>,
    path: Option<PathBuf>,
    error: Option<String>,
}

impl Workbench {
    pub(super) fn open_workspace_transfer(
        &mut self,
        mode: Mode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prepare_workspace_transfer(mode, cx);
        window.focus(&self.focus, cx);
    }
    pub(super) fn prepare_workspace_transfer(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.request_binding.is_some() || self.workspace_transfer_task.is_some() {
            return;
        }
        self.show_workspaces = false;
        self.workspace_transfer = Some(TransferDialog {
            mode,
            selected: Contents::all(),
            archive: None,
            path: None,
            error: None,
        });
        self.workspace_transfer_name
            .update(cx, |entry, cx| entry.set_value("", cx));
        cx.notify();
    }
    fn transfer_error(&mut self, error: String, cx: &mut Context<Self>) {
        if let Some(dialog) = &mut self.workspace_transfer {
            dialog.error = Some(error);
        }
        cx.notify();
    }
    pub(super) fn workspace_archive(
        &mut self,
        selected: Contents,
        cx: &mut Context<Self>,
    ) -> Result<Archive, String> {
        if self.directory_settings.is_some()
            || self.group_edit.is_some()
            || self.editing_request_name
        {
            return Err("请先完成当前集合、目录或请求名称的编辑，再导出".into());
        }
        // Include unsaved request editors, environment rows and proxy forms.
        self.flush_request_editors(cx);
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
        let data = self.workspace.storage.load()?;
        let calls = if selected.has(Section::McpHistory) {
            self.workspace.storage.export_mcp_calls()?
        } else {
            vec![]
        };
        Archive::new(self.workspace_label(), &data, selected, calls)
    }
    fn choose_workspace_archive(&mut self, cx: &mut Context<Self>) {
        if self.workspace_transfer_task.is_some() {
            return;
        }
        let picker = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("选择 Glint API 工作区迁移文件".into()),
        });
        if let Some(dialog) = &mut self.workspace_transfer {
            dialog.error = None;
        }
        self.workspace_transfer_task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let paths = picker
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                let Some(path) = paths.and_then(|paths| paths.into_iter().next()) else {
                    return Ok(None);
                };
                let input = path.clone();
                let archive = cx
                    .background_executor()
                    .spawn(async move { Archive::read(&input) })
                    .await?;
                Ok::<_, String>(Some((path, archive)))
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                this.workspace_transfer_task = None;
                match result {
                    Ok(Some((path, archive))) => this.set_workspace_import(path, archive, cx),
                    Ok(None) => {}
                    Err(error) => {
                        if let Some(dialog) = &mut this.workspace_transfer {
                            dialog.archive = None;
                            dialog.path = None;
                        }
                        this.transfer_error(error, cx);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(super) fn set_workspace_import(
        &mut self,
        path: PathBuf,
        archive: Archive,
        cx: &mut Context<Self>,
    ) {
        let base: String = archive.name.trim().chars().take(48).collect();
        let base = if base.is_empty() {
            "导入的工作区".to_owned()
        } else {
            base
        };
        let mut name = base.clone();
        let mut suffix = 1;
        let workspaces = match self.workspace.storage.workspaces() {
            Ok(workspaces) => workspaces,
            Err(error) => {
                self.transfer_error(error, cx);
                return;
            }
        };
        while workspaces.iter().any(|workspace| workspace.name == name) {
            name = format!("{base}（导入 {suffix}）");
            suffix += 1;
        }
        self.workspace_transfer_name
            .update(cx, |entry, cx| entry.set_value(name, cx));
        if let Some(dialog) = &mut self.workspace_transfer {
            dialog.selected = archive.contents.clone();
            dialog.archive = Some(Arc::new(archive));
            dialog.path = Some(path);
            dialog.error = None;
        }
    }
    fn export_workspace_archive(&mut self, cx: &mut Context<Self>) {
        if self.workspace_transfer_task.is_some() {
            return;
        }
        let Some(dialog) = &self.workspace_transfer else {
            return;
        };
        let archive = match self.workspace_archive(dialog.selected.clone(), cx) {
            Ok(archive) => archive,
            Err(error) => {
                self.transfer_error(error, cx);
                return;
            }
        };
        let safe_name: String = archive
            .name
            .chars()
            .map(|c| {
                if c.is_control() || "<>:\"/\\|?*".contains(c) {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        let filename = format!(
            "{}.glint-workspace.json",
            safe_name.trim().trim_end_matches('.')
        );
        let directory = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .unwrap_or_default();
        let picker = cx.prompt_for_new_path(&directory, Some(&filename));
        if let Some(dialog) = &mut self.workspace_transfer {
            dialog.error = None;
        }
        self.workspace_transfer_task = Some(cx.spawn(async move |this, cx| {
            let result = async {
                let path = picker
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                let Some(path) = path else {
                    return Ok(None);
                };
                let output = path.clone();
                cx.background_executor()
                    .spawn(async move { archive.write(&output) })
                    .await?;
                Ok::<_, String>(Some(path))
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                this.workspace_transfer_task = None;
                match result {
                    Ok(Some(path)) => {
                        this.workspace_transfer = None;
                        this.notice = format!("工作区已导出至 {}", path.display());
                        this.toasts.update(cx, |host, cx| {
                            host.push(Toast::new("工作区导出成功").kind(ToastKind::Success), cx)
                        });
                    }
                    Ok(None) => {}
                    Err(error) => this.transfer_error(error, cx),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(super) fn import_workspace_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace_transfer_task.is_some() {
            return;
        }
        let Some(dialog) = &self.workspace_transfer else {
            return;
        };
        let Some(archive) = dialog.archive.clone() else {
            self.transfer_error("请先选择迁移文件".into(), cx);
            return;
        };
        let selected = dialog.selected.clone();
        let name = self.workspace_transfer_name.read(cx).value().to_owned();
        let result = (|| {
            // Validate before saving/switching; import is one SQLite transaction.
            let mut chosen = archive.selected(&selected)?;
            if selected.has(Section::Preferences) {
                if self.update_state.busy()
                    || matches!(self.update_state, update_ui::UpdateState::Ready { .. })
                {
                    return Err(
                        "请先完成或取消应用更新，再导入工作区设置；也可取消勾选工作区设置".into(),
                    );
                }
                if let Some(source) = &mut chosen.data.update_source {
                    source.validate()?;
                    if let Some(previous) =
                        self.workspace.update_source.as_ref().filter(|old| {
                            old.url == source.url && old.public_key == source.public_key
                        })
                    {
                        source.minimum_sequence =
                            source.minimum_sequence.max(previous.minimum_sequence);
                    }
                }
            }
            self.workspace.storage.workspace_name(&name, "")?;
            self.workspace_switch_ready(cx)?;
            let id = self
                .workspace
                .storage
                .import_archive(&name, &chosen, &chosen.contents)?;
            if let Err(error) = self.switch_workspace_with_settings(
                &id,
                selected.has(Section::Preferences),
                window,
                cx,
            ) {
                self.workspace.storage.delete_workspace(&id)?;
                return Err(error);
            }
            Ok::<_, String>(())
        })();
        match result {
            Ok(()) => {
                self.notice = format!("已导入并切换到工作区“{name}”");
                self.toasts.update(cx, |host, cx| {
                    host.push(Toast::new("工作区导入成功").kind(ToastKind::Success), cx)
                });
            }
            Err(error) => self.transfer_error(error, cx),
        }
        cx.notify();
    }
    pub(super) fn workspace_transfer_dialog(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let dialog = self.workspace_transfer.as_ref()?;
        let importing = dialog.mode == Mode::Import;
        let busy = self.workspace_transfer_task.is_some();
        let available = dialog
            .archive
            .as_ref()
            .map(|a| a.contents.clone())
            .unwrap_or_else(Contents::all);
        let effective = dialog.selected.effective();
        let p = self.theme.palette();
        let mut form = div()
            .id("workspace-transfer-dialog")
            .debug_selector(|| "workspace-transfer-dialog".into())
            .flex()
            .flex_col()
            .gap_3()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if importing {
                        "导入工作区"
                    } else {
                        "导出工作区"
                    }),
            )
            .child(div().text_color(rgb(p.text_muted)).child(if importing {
                "导入为新的工作区，保留现有工作区；完成后自动切换。".to_owned()
            } else {
                format!("导出当前工作区：{}", self.workspace_label())
            }));
        if importing {
            form = form.child(
                Button::new(if busy {
                    "正在读取…"
                } else {
                    "选择迁移文件…"
                })
                .kind(ButtonKind::Plain)
                .theme(self.theme)
                .render()
                .debug_selector(|| "choose-workspace-archive".into())
                .on_click(cx.listener(|this, _, _, cx| this.choose_workspace_archive(cx))),
            );
            if let Some(path) = &dialog.path {
                form = form.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child(path.display().to_string()),
                );
            }
            if let Some(archive) = &dialog.archive {
                form = form
                    .child(div().text_size(px(11.)).child(archive.summary()))
                    .child(div().child("新工作区名称"))
                    .child(self.workspace_transfer_name.clone());
            }
        }
        let mut options = div()
            .id("workspace-transfer-options")
            .max_h(px(300.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3();
        for section in Section::ALL {
            let enabled = !busy
                && available.has(section)
                && (!section.requires_requests() || dialog.selected.has(Section::Requests));
            let owner = cx.weak_entity();
            let mut checkbox = Checkbox::new(section.label())
                .checked(effective.has(section) && available.has(section))
                .theme(self.theme);
            if enabled {
                checkbox = checkbox.on_change(move |checked, _, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        if let Some(dialog) = &mut this.workspace_transfer {
                            dialog.selected.set(section, checked);
                            dialog.error = None;
                        }
                        cx.notify();
                    });
                });
            }
            options = options.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(!enabled, |el| el.opacity(0.45))
                    .child(
                        checkbox
                            .render()
                            .debug_selector(move || format!("transfer-section-{section:?}")),
                    )
                    .child(
                        div()
                            .pl_5()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(if available.has(section) {
                                section.description()
                            } else {
                                "迁移文件未包含此项"
                            }),
                    ),
            );
        }
        let all = effective == available.effective();
        form = form.child(div().flex().justify_between().items_center()
            .child("选择迁移内容")
            .child(Button::new(if all { "取消全选" } else { "全选" }).kind(ButtonKind::Plain).size(ButtonSize::Mini).theme(self.theme).render()
                .debug_selector(|| "transfer-select-all".into())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.workspace_transfer_task.is_none() && let Some(dialog) = &mut this.workspace_transfer {
                        dialog.selected = if all { Contents::default() } else { available.clone() }; cx.notify();
                    }
                }))))
            .child(options)
            .child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child("文件保留请求与环境中的 Token、密钥原值。OAuth 系统凭据、已安装插件和本机 CA 证书不随工作区迁移；集合设置与变量会保留必要的目录结构。"));
        if let Some(error) = &dialog.error {
            form = form.child(
                div()
                    .text_color(rgb(self.theme.color(0xd9342b)))
                    .child(error.clone()),
            );
        }
        form = form.child(
            div()
                .flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("取消")
                        .kind(ButtonKind::Plain)
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "cancel-workspace-transfer".into())
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.workspace_transfer_task.is_none() {
                                this.workspace_transfer = None;
                                window.focus(&this.focus, cx);
                                cx.notify();
                            }
                        })),
                )
                .child(
                    Button::new(if busy {
                        "处理中…"
                    } else if importing {
                        "导入并切换"
                    } else {
                        "导出文件…"
                    })
                    .theme(self.theme)
                    .when(
                        busy || effective.0.is_empty() || (importing && dialog.archive.is_none()),
                        |button| button.disabled(),
                    )
                    .render()
                    .debug_selector(|| "submit-workspace-transfer".into())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if importing {
                            this.import_workspace_archive(window, cx);
                        } else {
                            this.export_workspace_archive(cx);
                        }
                    })),
                ),
        );
        Some(
            Dialog::new("workspace-transfer-modal", form)
                .theme(self.theme)
                .width(640.)
                .priority(130)
                .dismissible(!busy)
                .focus_handles(if importing && dialog.archive.is_some() {
                    vec![self.workspace_transfer_name.read(cx).focus_handle(cx)]
                } else {
                    vec![]
                })
                .on_close(cx.listener(|this, _, window, cx| {
                    if this.workspace_transfer_task.is_none() {
                        this.workspace_transfer = None;
                        window.focus(&this.focus, cx);
                        cx.notify();
                    }
                }))
                .render(),
        )
    }
}
