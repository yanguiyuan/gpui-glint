//! Request drafts, naming, selection, persistence, and request management.

use super::*;

impl Workbench {
    pub(super) fn edit_request_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.requests.get(self.active) else {
            return;
        };
        let name = request.name.clone();
        self.request_name
            .update(cx, |entry, cx| entry.set_value(name, cx));
        self.editing_request_name = true;
        self.request_name_error = None;
        window.focus(&self.request_name.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn cancel_request_name(&mut self, cx: &mut Context<Self>) {
        self.editing_request_name = false;
        self.request_name_error = None;
        cx.notify();
    }

    pub(super) fn save_request_name(&mut self, cx: &mut Context<Self>) {
        if !self.editing_request_name {
            return;
        }
        let name = self.request_name.read(cx).value().trim().to_owned();
        if name.is_empty() {
            self.request_name_error = Some("请输入请求名称".into());
            cx.notify();
            return;
        }
        if let Some(request) = self.requests.get_mut(self.active) {
            if request.name != name {
                request.name = name;
                self.storage_dirty = true;
            }
        }
        self.cancel_request_name(cx);
        self.sync_groups(cx);
        self.sync_commands(cx);
    }

    pub(super) fn request_name_view(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let p = self.theme.palette();
        if self.editing_request_name {
            return div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "enter" => {
                            this.save_request_name(cx);
                            cx.stop_propagation();
                        }
                        "escape" => {
                            this.cancel_request_name(cx);
                            cx.stop_propagation();
                        }
                        _ => return,
                    }
                    if !this.editing_request_name {
                        window.focus(&this.focus, cx);
                    }
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h(px(30.))
                                .px_2()
                                .rounded_md()
                                .border_1()
                                .border_color(rgb(p.accent))
                                .child(self.request_name.clone()),
                        )
                        .child(
                            IconButton::new(Icon::CheckCircle, "保存请求名称")
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_request_name(cx);
                                    if !this.editing_request_name {
                                        window.focus(&this.focus, cx);
                                    }
                                })),
                        )
                        .child(
                            IconButton::new(Icon::Close, "取消重命名")
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.cancel_request_name(cx);
                                    window.focus(&this.focus, cx);
                                })),
                        ),
                )
                .when_some(self.request_name_error.clone(), |view, error| {
                    view.child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(0xd9342b))
                            .child(error),
                    )
                })
                .into_any_element();
        }
        div()
            .id("edit-request-name")
            .group("request-name")
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .hover(|style| style.text_color(rgb(p.accent)))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(17.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.requests[self.active].name.clone()),
            )
            .child(
                Icon::Pencil
                    .render(14.)
                    .flex_shrink_0()
                    .opacity(0.)
                    .group_hover("request-name", |style| style.opacity(1.)),
            )
            .on_click(cx.listener(|this, _, window, cx| this.edit_request_name(window, cx)))
            .into_any_element()
    }

    pub(super) fn capture_draft(&mut self, cx: &App) {
        if !self.requests.is_empty() {
            let draft = self.snapshot(cx);
            if self.requests[self.active].draft != draft {
                self.requests[self.active].draft = draft;
                self.storage_dirty = true;
            }
        }
    }

    pub(super) fn persist(&mut self, cx: &mut Context<Self>) {
        if self.request_binding.is_some() {
            self.capture_draft(cx);
            return;
        }
        self.flush_request_editors(cx);
        self.refresh_request_editors(cx);
        self.capture_draft(cx);
        self.storage_dirty |= self.selected_environment(cx) != self.saved_environment;
        if !self.storage_dirty {
            return;
        }
        let workspace = storage::Workspace {
            update_source: self.update_source.clone(),
            selected_environment: self.selected_environment(cx),
            requests: self.requests.clone(),
            collections: self.collections.clone(),
            environments: self.environment_names.clone(),
            variables: self.environment_inputs.clone(),
            proxies: self.proxies.clone(),
            traces: self.traces.clone(),
            responses: self.responses.clone(),
            previews: self.sent_requests.clone(),
            curls: self.sent_curls.clone(),
        };
        let error = self.storage.save(&workspace).err();
        if error.is_none() {
            self.storage_dirty = false;
            self.saved_environment = workspace.selected_environment;
        }
        if error != self.storage_error {
            self.storage_error = error;
            cx.notify();
        }
    }

    pub(super) fn restore_response(&mut self, cx: &mut Context<Self>) {
        self.response_data = self.responses.get(self.active).cloned().flatten();
        self.status = self.response_data.as_ref().map(|r| r.status);
        self.response_json = self.response_data.as_ref().and_then(|r| r.json.clone());
        self.sync_request_preview(cx);
        self.update_response(cx);
    }

    pub(super) fn snapshot(&self, cx: &App) -> Draft {
        if let Some(editor) = self.shared_request_editor(self.active) {
            return editor.read(cx).snapshot(cx);
        }
        let pairs = |rows: &[PairInput]| {
            rows.iter()
                .map(|r| Pair {
                    enabled: r.enabled,
                    key: r.key.read(cx).value().into(),
                    value: r.value.read(cx).value().into(),
                    note: r.note.read(cx).value().into(),
                })
                .collect()
        };
        Draft {
            websocket_follow_latest: self.ws_follow_latest,
            websocket_raw: self.ws_raw,
            event_decoder: self
                .requests
                .get(self.active)
                .and_then(|r| r.draft.event_decoder.clone()),
            plugins: self
                .requests
                .get(self.active)
                .map(|r| r.draft.plugins.clone())
                .unwrap_or_default(),
            method: self
                .endpoint
                .read(cx)
                .value()
                .selected
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or("GET".into()),
            url: self.endpoint.read(cx).value().text.to_string(),
            body: self.body.read(cx).content().into(),
            body_format: self.body_format,
            form: pairs(&self.form),
            params: pairs(&self.params),
            headers: pairs(&self.headers),
            cookies: pairs(&self.cookies),
            extractors: pairs(&self.extractors),
            oauth: (self.authentication.read(cx).value().selected.as_deref() == Some("oauth2"))
                .then(|| self.oauth.snapshot(cx)),
            token: self.authentication.read(cx).value().text.to_string(),
            auth: self.authentication.read(cx).value().selected.as_deref() == Some("bearer"),
            skip_tls_verification: self.skip_tls_verification,
        }
    }

    pub(super) fn load(&mut self, draft: &Draft, cx: &mut Context<Self>) {
        self.pair_delete = None;
        if self.ws_active || self.ws_commands.is_some() {
            self.sending = None;
            self.ws_commands = None;
            self.ws_active = false;
        }
        self.ws_finish_history("WS 已结束");
        self.ws_history = None;
        self.ws_connected = false;
        self.ws_environment = None;
        self.ws_follow_latest = draft.websocket_follow_latest;
        self.ws_raw = draft.websocket_raw;
        self.clear_websocket_messages(cx);
        self.ws_format_select.update(cx, |select, cx| {
            select.close(cx);
            select.set_selected(Some(if self.ws_raw { "raw" } else { "pretty" }.into()), cx);
        });
        if let Some(request) = self.requests.get_mut(self.active) {
            request.draft.plugins = draft.plugins.clone();
            request.draft.event_decoder = draft.event_decoder.clone();
        }
        self.ws_refresh_decoder_select(cx);
        self.oauth.load(draft.oauth.clone().unwrap_or_default(), cx);
        self.skip_tls_verification = draft.skip_tls_verification;
        self.authentication.update(cx, |field, cx| {
            field.set_selected(
                Some(
                    if draft.oauth.is_some() {
                        "oauth2"
                    } else if draft.auth {
                        "bearer"
                    } else {
                        "none"
                    }
                    .into(),
                ),
                cx,
            );
            field.set_text(draft.token.clone(), cx);
            field.set_input_disabled(!draft.auth, cx);
        });
        self.endpoint.update(cx, |field, cx| {
            field.set_selected(Some(draft.method.clone().into()), cx);
            field.set_text(draft.url.clone(), cx);
        });
        self.body
            .update(cx, |editor, cx| editor.set_content(draft.body.clone(), cx));
        self.row_subscriptions.clear();
        self.params = draft.params.iter().map(|p| self.pair(p, cx)).collect();
        self.headers = draft.headers.iter().map(|p| self.pair(p, cx)).collect();
        self.cookies = draft.cookies.iter().map(|p| self.pair(p, cx)).collect();
        self.body_format = draft.body_format;
        self.form = draft.form.iter().map(|pair| self.pair(pair, cx)).collect();
        self.extractors = draft
            .extractors
            .iter()
            .map(|p| self.extractor_input(p, cx))
            .collect();
        self.sync_pair_tables(cx);
        self.sync_reference_inputs(cx);
        cx.notify();
    }

    pub(super) fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        self.request_menu = None;
        self.duplicate_version = None;
        self.history_delete = None;
        self.version_tag_edit = None;
        self.tab_menu = None;
        self.directory_settings = None;
        self.tree_delete = None;
        self.show_proxies = false;
        self.show_mcp = false;
        self.show_plugins = false;
        self.show_environments = false;
        if index >= self.requests.len() {
            return;
        }
        if !self.requests.is_empty() {
            self.capture_draft(cx);
        }
        self.park_websocket(cx);
        if self.sending.take().is_some() {
            self.push_trace("切换请求，已取消等待中的请求", cx);
        }
        self.trace_started = None;
        self.tree_menu = None;
        self.cancel_request_name(cx);
        self.active = index;
        if let Some(collection) = self
            .collections
            .iter()
            .position(|collection| collection.collection == self.requests[index].collection)
        {
            if self.current_collection != collection {
                self.group_edit = None;
                self.group_error = None;
            }
            self.current_collection = collection;
            self.current_directory = self.requests[index].group.clone();
            let mut path = self.current_directory.as_str();
            loop {
                self.collapsed_directories
                    .remove(&(collection, path.to_owned()));
                if path.is_empty() {
                    break;
                }
                path = RequestGroups::parent(path);
            }
        }
        self.sync_request_preview(cx);
        self.sync_groups(cx);
        if !self.tabs.contains(&index) {
            self.tabs.push(index);
        }
        let draft = self.requests[index].draft.clone();
        self.config = if matches!(draft.method.as_str(), "POST" | "PUT" | "PATCH" | "WS") {
            3
        } else {
            0
        };
        self.load(&draft, cx);
        self.restore_response(cx);
        self.error = None;
        self.unpark_websocket(cx);
    }

    pub(super) fn save(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.shared_request_editor(self.active) {
            editor.update(cx, |editor, cx| editor.save(cx));
            return;
        }
        if self.show_curl_import {
            return;
        }
        if self.version_tag_edit.is_some() {
            self.save_version_tag(cx);
            return;
        }
        if let Some(editor) = self.directory_settings.clone() {
            editor.update(cx, |editor, cx| editor.save(cx));
            return;
        }
        if self.requests.is_empty() {
            return;
        }
        let draft = self.snapshot(cx);
        self.duplicate_version = None;
        let request = &self.requests[self.active];
        if let Some(index) = self.matching_version(self.active, &draft) {
            let record = &request.versions[index];
            self.duplicate_version = Some(version_history::DuplicateVersion {
                request_id: request.id.clone(),
                version_id: record.id.clone(),
                fingerprint: request_fingerprint::Fingerprint::new(&draft),
                focus: cx.focus_handle(),
            });
            cx.notify();
            return;
        }
        self.append_version(self.active, draft, model::VersionSource::Manual);
        self.notice = "已保存新版本".into();
        self.activity = 0;
        self.show_activity = true;
        cx.notify();
    }

    pub(super) fn add_request(&mut self, cx: &mut Context<Self>) {
        self.add_request_to(self.current_directory.clone(), cx);
    }

    pub(super) fn add_request_to(&mut self, group: String, cx: &mut Context<Self>) {
        if self.collections.is_empty() {
            self.group_edit = Some(GroupEdit::CreateCollection);
            self.group_error = None;
            cx.notify();
            return;
        }
        let draft = self.collections[self.current_collection].request_defaults(&group);
        if !self.collections[self.current_collection].contains(&group) {
            return;
        }
        let i = self.requests.len();
        self.storage_dirty = true;
        self.requests.push(Request {
            id: crate::model::new_id(),
            tree_order: 0,
            name: format!("新建请求 {}", i + 1),
            collection: self.collections[self.current_collection].collection.clone(),
            group,
            draft: draft.clone(),
            versions: vec![Record {
                id: crate::model::new_id(),
                tag: String::new(),
                source: Default::default(),
                environment_name: None,
                draft,
                label: "v1".into(),
                time: record_time(),
                detail: "初始版本".into(),
                elapsed_ms: None,
                exchange: None,
            }],
            history: vec![],
        });
        let target = model::TreeNode::Directory {
            collection: self.current_collection,
            path: self.requests[i].group.clone(),
        };
        let _ = model::move_node(
            &mut self.collections,
            &mut self.requests,
            &model::TreeNode::Request(i),
            &target,
            model::Placement::Inside,
        );
        self.responses.push(None);
        self.traces.push(vec![]);
        self.sent_requests.push(None);
        self.sent_curls.push(None);
        self.current_versions.push(0);
        if i == 0 {
            self.load(&self.requests[0].draft.clone(), cx);
        }
        self.choose(i, cx);
    }

    pub(super) fn delete_request(&mut self, index: usize, cx: &mut Context<Self>) {
        self.request_menu = None;
        if let Some(request) = self.requests.get(index) {
            if let Some(shared) = self.request_editors.remove(&request.id) {
                shared
                    .editor
                    .update(cx, |editor, cx| editor.detach_request_window(cx));
            }
        }
        self.duplicate_version = None;
        self.history_delete = None;
        self.version_tag_edit = None;
        self.tree_delete = None;
        if index >= self.requests.len() {
            return;
        }
        self.cancel_request_name(cx);
        self.capture_draft(cx);
        if index != self.active {
            self.park_websocket(cx);
        }
        let deleted_id = self.requests[index].id.clone();
        self.ws_background.remove(&deleted_id);
        // Any removal changes captured indices in a pending response task.
        if self.sending.take().is_some() {
            self.push_trace("删除请求，已取消等待中的请求", cx);
        }
        self.trace_started = None;
        let removed_active = index == self.active;
        self.storage_dirty = true;
        let name = self.requests.remove(index).name;
        self.responses.remove(index);
        self.traces.remove(index);
        self.sent_requests.remove(index);
        self.sent_curls.remove(index);
        self.current_versions.remove(index);
        self.tabs = self
            .tabs
            .iter()
            .filter_map(|&i| {
                if i == index {
                    None
                } else {
                    Some(i - usize::from(i > index))
                }
            })
            .collect();
        self.selected_history = self
            .selected_history
            .and_then(|(i, record)| (i != index).then(|| (i - usize::from(i > index), record)));
        if self.requests.is_empty() {
            self.active = 0;
        } else if removed_active {
            self.active = self
                .tabs
                .first()
                .copied()
                .unwrap_or(index.min(self.requests.len() - 1));
            if !self.tabs.contains(&self.active) {
                self.tabs.push(self.active);
            }
            self.load(&self.requests[self.active].draft.clone(), cx);
            self.config = 0;
        } else if self.active > index {
            self.active -= 1;
        }
        if removed_active {
            self.restore_response(cx);
            self.error = None;
        }
        self.unpark_websocket(cx);
        self.sync_request_preview(cx);
        self.sync_groups(cx);
        self.notice = format!("已删除请求 {name}");
        cx.notify();
    }
}
