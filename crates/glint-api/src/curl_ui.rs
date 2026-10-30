use super::*;

impl Workbench {
    pub(super) fn open_curl_import(&mut self, cx: &mut Context<Self>) {
        self.show_collection_menu = false;
        self.tree_menu = None;
        self.curl_error = None;
        self.show_curl_import = true;
        self.curl_import_needs_focus = true;
        cx.notify();
    }
    fn import_curl(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = match curl::parse(self.curl_input.read(cx).content()) {
            Ok(draft) => draft,
            Err(error) => {
                self.curl_error = Some(error);
                cx.notify();
                return;
            }
        };
        // Parse first: failed imports must not create collections or requests.
        if self.workspace.collections.is_empty() {
            self.workspace
                .collections
                .push(RequestGroups::create_collection("导入的请求", &[]).unwrap());
            self.current_collection = 0;
            self.current_directory.clear();
        }
        self.add_request(cx);
        let name = reqwest::Url::parse(&draft.url)
            .ok()
            .map(|url| format!("{} {}", draft.method, url.path()))
            .unwrap_or_else(|| format!("{} 导入请求", draft.method));
        let request = &mut self.workspace.requests[self.active];
        request.name = name;
        request.draft = draft.clone();
        request.versions[0].draft = draft.clone();
        request.versions[0].detail = "从 cURL 导入".into();
        // Imported fields replace directory defaults; no implicit inherited auth/actions.
        self.load(&draft, cx);
        self.config = if matches!(draft.method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE") {
            3
        } else {
            0
        };
        self.sync_groups(cx);
        self.sync_commands(cx);
        self.show_curl_import = false;
        self.curl_error = None;
        self.curl_input.update(cx, |e, cx| e.set_content("", cx));
        self.workspace.storage_dirty = true;
        self.persist(cx);
        window.focus(&self.focus, cx);
        self.notice = "已从 cURL 导入请求".into();
        cx.notify();
    }
    pub(super) fn copy_response_curl(&mut self, cx: &mut Context<Self>) {
        if let Some(command) = self
            .workspace
            .sent_curls
            .get(self.active)
            .and_then(|c| c.clone())
        {
            cx.write_to_clipboard(ClipboardItem::new_string(curl::filter_response_resolves(
                &command,
            )));
            self.toasts.update(cx, |host, cx| {
                host.push(Toast::new("已复制 cURL 命令").kind(ToastKind::Success), cx)
            });
        }
    }
    pub(super) fn curl_import_dialog(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.show_curl_import {
            return None;
        }
        let p = self.theme.palette();
        let destination = self
            .workspace
            .collections
            .get(self.current_collection)
            .map(|c| {
                if self.current_directory.is_empty() {
                    c.collection.clone()
                } else {
                    format!("{} / {}", c.collection, self.current_directory)
                }
            })
            .unwrap_or("导入的请求（新建集合）".into());
        let form = div().id("curl-import-dialog").debug_selector(|| "curl-import-dialog".into()).flex().flex_col().gap_3()
            .on_mouse_down(gpui::MouseButton::Left,|_,_,cx| cx.stop_propagation())
            .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child("从 cURL 导入请求"))
            .child(div().text_color(rgb(p.text_muted)).child(format!("导入到 {destination}")))
            .child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child("粘贴 Bash / POSIX 格式的 cURL 命令，支持多行、请求头、鉴权、Cookie 和正文。不支持的参数会自动忽略。导入后可编辑，不会自动发送。"))
            .child(div().h(px(280.)).min_h_0().child(self.curl_input.clone()))
            .when_some(self.curl_error.clone(),|s,error| s.child(div().text_size(px(11.)).text_color(rgb(self.theme.color(0xd9342b))).child(error)))
            .child(div().flex().justify_end().gap_2()
                .child(Button::new("取消").kind(ButtonKind::Plain).theme(self.theme).render().debug_selector(|| "cancel-curl-import".into()).on_click(cx.listener(|this,_,window,cx| { this.show_curl_import=false; window.focus(&this.focus,cx); cx.notify(); })))
                .child(Button::new("导入请求").theme(self.theme).render().debug_selector(|| "submit-curl-import".into()).on_click(cx.listener(|this,_,window,cx| this.import_curl(window,cx)))));
        Some(
            Dialog::new("curl_ui-modal", form)
                .focus_handles([self.curl_input.read(cx).focus_handle(cx)])
                .theme(self.theme)
                .width(680.)
                .priority(110)
                .on_close(cx.listener(|this, _, window, cx| {
                    this.show_curl_import = false;
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
    fn click(window: &mut gpui::VisualTestContext, selector: &'static str) {
        let rect = window.debug_bounds(selector).unwrap();
        window.simulate_click(rect.center(), gpui::Modifiers::default());
    }
    #[gpui::test]
    fn import_menu_palette_and_response_export_preserve_snapshots(cx: &mut gpui::TestAppContext) {
        cx.update(bind_text_entry_keys);
        cx.update(bind_code_card_keys);
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            Workbench::with_store(storage::Store::open(&path).unwrap(), cx)
        });
        assert!(window.debug_bounds("open-curl-import").is_none());
        click(window, "collection-more");
        click(window, "open-curl-import");
        assert!(window.debug_bounds("curl-import-dialog").is_some());
        host.update(window, |h, cx| {
            h.curl_input.update(cx, |e, cx| {
                e.set_content("curl https://example.test -X", cx)
            })
        });
        click(window, "submit-curl-import");
        host.update(window, |h, _| {
            assert!(h.show_curl_import);
            assert!(h.curl_error.is_some());
            assert!(h.workspace.requests.is_empty() && h.workspace.collections.is_empty());
        });
        let source = "curl 'https://example.test/v1?q=two%20words' -H 'Authorization: Bearer synthetic' --json '{\"name\":\"导入\"}'";
        let pasted = format!("{source} --proxy http://127.0.0.1:7890 --location");
        window.update(|_, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(pasted.clone()));
        });
        window.simulate_keystrokes("ctrl-a ctrl-v");
        host.update(window, |h, cx| {
            assert_eq!(h.curl_input.read(cx).content(), pasted);
        });
        click(window, "submit-curl-import");
        host.update(window, |h, cx| {
            assert!(!h.show_curl_import);
            assert_eq!(h.workspace.requests.len(), 1);
            assert_eq!(h.workspace.collections.len(), 1);
            assert_eq!(h.snapshot(cx), curl::parse(source).unwrap());
            assert_eq!(h.workspace.requests[0].versions[0].draft, h.snapshot(cx));
            assert!(h.sending.is_none());
            assert!(h.workspace.sent_curls[0].is_none());
            h.workspace.collections[0].add_child("", "Folder").unwrap();
            h.workspace.collections[0].defaults.insert(
                "Folder".into(),
                model::DirectoryDefaults {
                    sections: [model::DefaultSection::Auth].into_iter().collect(),
                    draft: Draft {
                        auth: true,
                        token: "{{inherited_token}}".into(),
                        ..Draft::default()
                    },
                },
            );
            h.current_directory = "Folder".into();
        });
        // Discover and execute the importer through the actual command palette.
        window.update(|window, cx| host.update(cx, |h, cx| h.open_commands(window, cx)));
        host.update(window, |h, cx| {
            h.commands
                .update(cx, |palette, cx| palette.set_query("curl", cx))
        });
        window.simulate_keystrokes("enter");
        assert!(window.debug_bounds("curl-import-dialog").is_some());
        window.simulate_keystrokes("escape");
        host.update(window, |h, _| assert!(!h.show_curl_import));
        window.update(|window, cx| host.update(cx, |h, cx| h.open_commands(window, cx)));
        host.update(window, |h, cx| {
            h.commands
                .update(cx, |palette, cx| palette.set_query("导入", cx))
        });
        window.simulate_keystrokes("enter");
        let second = "curl https://example.test/token --data-urlencode 'name=张 三' --data-urlencode 'name=a+b'";
        host.update(window, |h, cx| {
            h.curl_input.update(cx, |e, cx| e.set_content(second, cx))
        });
        click(window, "submit-curl-import");
        let (old, new) = host.update(window, |h, cx| {
            assert_eq!(h.workspace.requests[1].group, "Folder");
            assert_eq!(h.snapshot(cx), curl::parse(second).unwrap());
            assert!(!h.snapshot(cx).auth);
            assert_eq!(h.config, 3);
            assert_eq!(h.selected_environment(cx), 0);
            let mut old_request =
                PreparedRequest::new(&h.snapshot(cx), &EnvironmentVariables::default().values())
                    .unwrap();
            old_request
                .apply_plugin_headers(
                    &serde_json::json!({"setHeaders":{"X-Signature":"signature-before-edit"}}),
                    &std::collections::BTreeSet::from(["request.header.x-signature".into()]),
                )
                .unwrap();
            let old = old_request.to_curl().unwrap();
            let new = PreparedRequest::new(
                &Draft {
                    url: "https://example.test/newer".into(),
                    ..Draft::default()
                },
                &EnvironmentVariables::default().values(),
            )
            .unwrap()
            .to_curl()
            .unwrap();
            for (i, curl) in [new.clone(), old.clone()].into_iter().enumerate() {
                let mut record = h.workspace.requests[1].versions[0].clone();
                record.id = model::new_id();
                record.label = "200 OK".into();
                record.exchange = Some(model::Exchange {
                    websocket_connected: false,
                    websocket_events: vec![],
                    curl: Some(curl),
                    response: Some(HttpResponse {
                        status: 200,
                        text: format!("response-{i}"),
                        bytes: 10,
                        headers: vec![],
                        json: None,
                    }),
                    error: None,
                    preview: Some(format!("request-{i}")),
                    traces: vec![],
                });
                h.workspace.requests[1].history.push(record);
            }
            h.workspace.sent_curls[1] = Some(new.clone());
            h.activity = 1;
            h.show_activity = true;
            h.workspace.storage_dirty = true;
            cx.notify();
            (old, new)
        });
        click(window, "export-response-curl");
        assert_eq!(window.read_from_clipboard().unwrap().text().unwrap(), new);
        click(window, "history-record-1");
        host.update(window, |h, cx| {
            h.endpoint.update(cx, |field, cx| {
                field.set_text("https://example.test/edited-after-send", cx)
            })
        });
        click(window, "export-response-curl");
        assert_eq!(window.read_from_clipboard().unwrap().text().unwrap(), old);
        host.update(window, |h, cx| h.persist(cx));
        let stored = storage::Store::open(&path).unwrap().load().unwrap();
        assert_eq!(stored.curls[1].as_deref(), Some(old.as_str()));
        assert_eq!(
            stored.requests[1].history[1]
                .exchange
                .as_ref()
                .unwrap()
                .curl
                .as_deref(),
            Some(old.as_str())
        );
        assert_eq!(stored.requests[0].draft, curl::parse(source).unwrap());
        assert_eq!(
            stored.requests[1].draft.url,
            "https://example.test/edited-after-send"
        );
        host.update(window, |h, cx| h.choose(0, cx));
        host.update(window, |h, _| assert!(h.workspace.sent_curls[0].is_none()));
        host.update(window, |h, cx| h.choose(1, cx));
        click(window, "export-response-curl");
        assert_eq!(window.read_from_clipboard().unwrap().text().unwrap(), old);
    }
}
