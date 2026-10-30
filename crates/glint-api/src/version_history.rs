use super::*;
use request_fingerprint::Fingerprint;

pub(super) struct DuplicateVersion {
    pub request_id: String,
    pub version_id: String,
    pub fingerprint: Fingerprint,
    pub focus: FocusHandle,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(window: &mut gpui::VisualTestContext, selector: &'static str) {
        let bounds = window.debug_bounds(selector).unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
    }

    #[gpui::test]
    fn duplicate_versions_cancel_confirm_and_restore_after_restart(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("versions.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name.update(cx, |e, cx| e.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.endpoint
                .update(cx, |e, cx| e.set_text("https://example.test/v2", cx));
            host.save(cx);
            host
        });
        let original = host.read_with(window, |h, _| {
            h.workspace.requests[0].versions[1].draft.clone()
        });
        for dismissal in ["cancel", "escape", "outside"] {
            host.update(window, |h, cx| {
                h.load(&original, cx);
                h.save(cx);
                assert_eq!(h.workspace.requests[0].versions.len(), 2);
                assert_eq!(h.current_versions[0], 0);
            });
            assert!(window.debug_bounds("duplicate-version-dialog").is_some());
            match dismissal {
                "cancel" => click(window, "cancel-duplicate-version"),
                "escape" => window.simulate_keystrokes("escape"),
                _ => window.simulate_click(point(px(10.), px(10.)), gpui::Modifiers::default()),
            }
            host.update(window, |h, cx| {
                assert!(h.duplicate_version.is_none());
                assert_eq!(h.current_versions[0], 0);
                assert_eq!(h.snapshot(cx), original);
            });
        }
        host.update(window, |h, cx| h.save(cx));
        click(window, "confirm-duplicate-version");
        host.update(window, |h, cx| {
            assert!(h.duplicate_version.is_none());
            assert_eq!(h.current_versions[0], 1);
            assert_eq!(h.snapshot(cx), original);
            assert_eq!(h.workspace.requests[0].versions.len(), 2);
        });
        let reopened = window.update(|_, cx| {
            cx.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx))
        });
        reopened.update(window, |h, cx| {
            assert_eq!(h.current_versions[0], 1);
            assert_eq!(h.snapshot(cx), original);
            assert_eq!(h.workspace.requests[0].versions.len(), 2);
        });
    }

    #[gpui::test]
    fn duplicate_confirmation_revalidates_edits_and_uses_persistent_ids(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let host = cx.new(|cx| {
            let mut h = Workbench::with_store(
                storage::Store::open(&directory.path().join("versions.sqlite3")).unwrap(),
                cx,
            );
            h.group_edit = Some(GroupEdit::CreateCollection);
            h.group_name.update(cx, |e, cx| e.set_value("API", cx));
            h.save_group(cx);
            h.add_request(cx);
            h
        });
        host.update(cx, |h, cx| {
            h.save(cx);
            h.endpoint
                .update(cx, |e, cx| e.set_text("https://example.test/edited", cx));
            h.confirm_duplicate_version(cx);
            assert_eq!(h.snapshot(cx).url, "https://example.test/edited");
            assert_eq!(h.workspace.requests[0].versions.len(), 1);
            h.save(cx);
            let original = h.workspace.requests[0].versions[1].draft.clone();
            h.load(&original, cx);
            h.save(cx);
            // Legacy/imported histories can contain duplicates; a selected match wins.
            let mut duplicate = h.workspace.requests[0].versions[1].clone();
            duplicate.id = model::new_id();
            h.workspace.requests[0].versions.insert(0, duplicate);
            h.confirm_duplicate_version(cx);
            assert_eq!(h.current_versions[0], 2);
            h.save(cx);
            assert_eq!(
                h.duplicate_version.as_ref().unwrap().version_id,
                h.workspace.requests[0].versions[2].id
            );
            let result = h
                .mcp_call(
                    "glint_version_save",
                    serde_json::json!({"request_id":h.workspace.requests[0].id,"tag":"must not overwrite"}),
                    cx,
                )
                .unwrap();
            assert_eq!(result["duplicate"], true);
            assert_eq!(result["confirmation_required"], false);
            assert!(h.duplicate_version.is_none());
            assert_eq!(result["id"], h.workspace.requests[0].versions[2].id);
            assert!(h.workspace.requests[0].versions.iter().all(|r| r.tag.is_empty()));
        });
    }
}

impl Workbench {
    pub(super) fn matching_version(&self, request: usize, draft: &Draft) -> Option<usize> {
        let fingerprint = Fingerprint::new(draft);
        let versions = &self.workspace.requests[request].versions;
        let selected = self.current_versions[request];
        if versions
            .get(selected)
            .is_some_and(|r| Fingerprint::new(&r.draft) == fingerprint)
        {
            Some(selected)
        } else {
            versions
                .iter()
                .position(|r| Fingerprint::new(&r.draft) == fingerprint)
        }
    }

    pub(super) fn version_modified(&self, request: usize, draft: &Draft) -> bool {
        self.workspace.requests[request]
            .versions
            .get(self.current_versions[request])
            .is_none_or(|r| Fingerprint::new(&r.draft) != Fingerprint::new(draft))
    }

    pub(super) fn append_version(
        &mut self,
        request: usize,
        draft: Draft,
        source: model::VersionSource,
    ) {
        self.workspace.append_version(request, draft, source);
        self.current_versions[request] = 0;
        self.workspace.storage_dirty = true;
    }

    /// MCP writes are saved without opening a GUI confirmation or interrupting
    /// the active request. Reusing a version preserves its original provenance.
    pub(super) fn save_mcp_version(&mut self, request: usize) -> (usize, bool) {
        if self
            .duplicate_version
            .as_ref()
            .is_some_and(|pending| pending.request_id == self.workspace.requests[request].id)
        {
            self.duplicate_version = None;
        }
        let draft = self.workspace.requests[request].draft.clone();
        if let Some(version) = self.matching_version(request, &draft) {
            self.current_versions[request] = version;
            (version, false)
        } else {
            self.append_version(request, draft, model::VersionSource::Mcp);
            (0, true)
        }
    }

    pub(super) fn restore_version(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(editor) = self.shared_request_editor(self.active) {
            editor.update(cx, |editor, cx| editor.restore_version(index, cx));
            return;
        }
        let Some(record) = self
            .workspace
            .requests
            .get(self.active)
            .and_then(|r| r.versions.get(index))
            .cloned()
        else {
            return;
        };
        if self.sending.take().is_some() {
            self.push_trace("恢复版本配置，已取消请求", cx);
        }
        self.trace_started = None;
        self.duplicate_version = None;
        self.selected_history = None;
        self.history_delete = None;
        self.current_versions[self.active] = index;
        self.load(&record.draft, cx);
        self.notice = format!("已恢复 {} 的请求配置", record.label);
        self.response_json = None;
        self.response_data = None;
        self.status = None;
        self.error = None;
        self.workspace.responses[self.active] = None;
        self.update_response(cx);
        self.workspace.storage_dirty = true;
        self.persist(cx);
        cx.notify();
    }

    fn confirm_duplicate_version(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.duplicate_version.take() else {
            return;
        };
        let index = self
            .workspace
            .requests
            .get(self.active)
            .filter(|request| request.id == pending.request_id)
            .and_then(|request| {
                request
                    .versions
                    .iter()
                    .position(|r| r.id == pending.version_id)
            });
        if let Some(index) = index.filter(|&index| {
            Fingerprint::new(&self.snapshot(cx)) == pending.fingerprint
                && Fingerprint::new(&self.workspace.requests[self.active].versions[index].draft)
                    == pending.fingerprint
        }) {
            self.restore_version(index, cx);
            self.activity = 0;
            self.show_activity = true;
        } else {
            self.notice = "请求配置已变化，请重新保存后检查版本".into();
        }
        cx.notify();
    }

    pub(super) fn duplicate_version_dialog(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let pending = self.duplicate_version.as_ref()?;
        let request = self
            .workspace
            .requests
            .get(self.active)
            .filter(|r| r.id == pending.request_id)?;
        let record = request
            .versions
            .iter()
            .find(|r| r.id == pending.version_id)?;
        let form = div()
            .debug_selector(|| "duplicate-version-dialog".into())
            .track_focus(&pending.focus)
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("已存在相同参数的版本"),
            )
            .child(format!(
                "当前请求与 {} 的参数一致，可直接切换到该历史版本。",
                record.label
            ))
            .child(
                div()
                    .font_family("Menlo")
                    .text_color(rgb(self.theme.palette().text_muted))
                    .child(format!(
                        "{} · #{} · {}",
                        record.label, pending.fingerprint.hash, record.time
                    )),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("取消")
                            .kind(ButtonKind::Plain)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "cancel-duplicate-version".into())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.duplicate_version = None;
                                window.focus(&this.focus, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("切换到历史版本")
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "confirm-duplicate-version".into())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_duplicate_version(cx);
                                window.focus(&this.focus, cx);
                            })),
                    ),
            );
        Some(
            Dialog::new("duplicate-version-modal", form)
                .focus_handles([pending.focus.clone()])
                .theme(self.theme)
                .width(440.)
                .on_close(cx.listener(|this, _, window, cx| {
                    this.duplicate_version = None;
                    window.focus(&this.focus, cx);
                    cx.notify();
                }))
                .render(),
        )
    }
}
