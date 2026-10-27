use super::*;

/// An owned snapshot: preview windows never retain the workbench or its editable entities.
struct RequestPreview {
    name: String,
    endpoint: Entity<TextEntry>,
    configuration_lines: Vec<Entity<TextEntry>>,
    theme: ThemePreset,
    request_tab: usize,
    response_tab: usize,
    request_cards: Vec<Entity<CodeCard>>,
    response_cards: Vec<Entity<CodeCard>>,
    status: String,
}

struct Snapshot {
    name: String,
    draft: Draft,
    resolver: TextReferenceResolver,
    response: Option<HttpResponse>,
    sent_request: Option<String>,
    events: Option<String>,
    error: Option<String>,
}

impl Workbench {
    fn request_window_snapshot(&self, index: usize, cx: &App) -> Option<Snapshot> {
        if let Some(editor) = self.shared_request_editor(index) {
            return editor.read(cx).request_window_snapshot(0, cx);
        }
        let request = self.requests.get(index)?;
        let active = index == self.active;
        let draft = if active {
            self.snapshot(cx)
        } else {
            request.draft.clone()
        };
        let events = (draft.method == "WS").then(|| {
            let log = if active {
                Some(&self.ws_log)
            } else {
                self.ws_background.get(&request.id).map(|state| &state.log)
            };
            log.map(|log| {
                log.iter()
                    .map(|event| event.log_text())
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
            .unwrap_or_default()
        });
        Some(Snapshot {
            name: request.name.clone(),
            resolver: self.request_reference_resolver(&draft, Some(&request.collection), cx),
            draft,
            response: if active {
                self.response_data.clone()
            } else {
                self.responses.get(index).cloned().flatten()
            },
            sent_request: self.sent_requests.get(index).cloned().flatten(),
            events,
            error: if active {
                self.error.clone()
            } else {
                self.ws_background
                    .get(&request.id)
                    .and_then(|state| state.error.clone())
            },
        })
    }

    pub(super) fn open_request_preview(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab_menu = None;
        cx.notify();
        let Some(snapshot) = self.request_window_snapshot(index, cx) else {
            return;
        };
        let theme = self.theme;
        let title = format!("{} · 只读预览 — Glint API", snapshot.name);
        let display_id = window.display(cx).map(|display| display.id());
        let bounds = Bounds::centered(display_id, size(px(960.), px(800.)), cx);
        if let Err(error) = cx.open_window(
            WindowOptions {
                display_id,
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(640.), px(480.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(title.into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |_, cx| cx.new(|cx| RequestPreview::new(snapshot, theme, cx)),
        ) {
            self.notice = format!("无法打开请求预览窗口：{error}");
            cx.notify();
        }
    }
}

fn configuration_text(draft: &Draft) -> String {
    let mut text = format!("{} {}\n", draft.method, draft.url);
    for (label, pairs) in [
        ("参数", &draft.params),
        ("请求头", &draft.headers),
        ("Cookie", &draft.cookies),
    ] {
        text.push_str(&format!("\n{label}\n"));
        if pairs.is_empty() {
            text.push_str("（无）\n");
        }
        for pair in pairs {
            text.push_str(&format!(
                "{}{}: {}{}\n",
                if pair.enabled { "" } else { "[已禁用] " },
                pair.key,
                pair.value,
                if pair.note.is_empty() {
                    String::new()
                } else {
                    format!("  // {}", pair.note)
                }
            ));
        }
    }
    text.push_str(&format!(
        "\n身份验证：{}\nTLS 证书校验：{}",
        if draft.oauth.is_some() {
            "OAuth 2.0"
        } else if draft.auth {
            "Bearer Token"
        } else {
            "无"
        },
        if draft.skip_tls_verification {
            "跳过"
        } else {
            "启用"
        }
    ));
    text
}

impl RequestPreview {
    fn new(snapshot: Snapshot, theme: ThemePreset, cx: &mut Context<Self>) -> Self {
        let card = |language, content: String, cx: &mut Context<Self>| {
            cx.new(|cx| {
                CodeCard::new(cx, language, content)
                    .theme(theme)
                    .read_only(true)
                    .show_header(false)
                    .viewport()
            })
        };
        let entry = |content: String, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut entry = TextEntry::new(cx, "", content)
                    .theme(theme)
                    .font_size(12.)
                    .read_only(true);
                entry.set_reference_resolver(Some(snapshot.resolver.clone()), cx);
                entry
            })
        };
        let endpoint = entry(
            format!("{} {}", snapshot.draft.method, snapshot.draft.url),
            cx,
        );
        let configuration_lines = configuration_text(&snapshot.draft)
            .split('\n')
            .map(|line| entry(line.to_owned(), cx))
            .collect();
        let body = if snapshot.draft.body_format == BodyFormat::UrlEncoded {
            snapshot
                .draft
                .form
                .iter()
                .map(|pair| {
                    format!(
                        "{}{}: {}",
                        if pair.enabled { "" } else { "[已禁用] " },
                        pair.key,
                        pair.value
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            snapshot.draft.body.clone()
        };
        let body_language = if serde_json::from_str::<serde_json::Value>(&body).is_ok() {
            "json"
        } else {
            "text"
        };
        let request_body = card(body_language, body, cx);
        request_body.update(cx, |body, cx| {
            body.set_reference_resolver(Some(snapshot.resolver.clone()), cx);
        });
        let request_cards = vec![
            request_body,
            card(
                "http",
                snapshot
                    .sent_request
                    .unwrap_or_else(|| "尚未发送请求".into()),
                cx,
            ),
        ];
        let status = snapshot
            .response
            .as_ref()
            .map(|response| {
                format!(
                    "{} · {} bytes",
                    status_label(response.status),
                    response.bytes
                )
            })
            .unwrap_or_else(|| {
                if snapshot.events.is_some() {
                    "WebSocket 消息快照".into()
                } else {
                    "暂无响应".into()
                }
            });
        let response_language = snapshot
            .response
            .as_ref()
            .map(HttpResponse::language)
            .unwrap_or("text");
        let raw = snapshot.events.unwrap_or_else(|| {
            snapshot
                .response
                .as_ref()
                .map(|response| response.text.clone())
                .unwrap_or_else(|| snapshot.error.unwrap_or_else(|| "暂无响应".into()))
        });
        let pretty = snapshot
            .response
            .as_ref()
            .and_then(|response| response.json.as_ref())
            .and_then(|json| serde_json::to_string_pretty(json).ok())
            .unwrap_or_else(|| raw.clone());
        let headers = snapshot
            .response
            .as_ref()
            .map(|response| {
                response
                    .headers
                    .iter()
                    .map(|(name, value)| format!("{name}: {value}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let response_body = card(response_language, pretty, cx);
        let folding = snapshot
            .response
            .as_ref()
            .is_some_and(|response| response.json.is_some());
        response_body.update(cx, |editor, cx| editor.set_json_folding(folding, cx));
        Self {
            name: snapshot.name,
            endpoint,
            configuration_lines,
            theme,
            request_tab: 0,
            response_tab: 0,
            request_cards,
            response_cards: vec![
                response_body,
                card(response_language, raw, cx),
                card("http", headers, cx),
            ],
            status,
        }
    }

    fn panel(&self, response: bool, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let (labels, selected, cards) = if response {
            (
                ["响应体", "原始响应", "响应头"],
                self.response_tab,
                &self.response_cards,
            )
        } else {
            (
                ["请求配置", "请求体", "最近发送报文"],
                self.request_tab,
                &self.request_cards,
            )
        };
        let mut tabs = div()
            .h(px(38.))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .gap_4()
            .border_b_1()
            .border_color(rgb(p.border));
        for (index, label) in labels.into_iter().enumerate() {
            tabs = tabs.child(
                tab_button(label, selected == index, self.theme)
                    .id((
                        if response {
                            "preview-response-tab"
                        } else {
                            "preview-request-tab"
                        },
                        index,
                    ))
                    .debug_selector(move || {
                        format!(
                            "preview-{}-tab-{index}",
                            if response { "response" } else { "request" }
                        )
                        .into()
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if response {
                            this.response_tab = index;
                        } else {
                            this.request_tab = index;
                        }
                        cx.notify();
                    })),
            );
        }
        let content = if !response && selected == 0 {
            div()
                .id("preview-configuration")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p_4()
                .children(
                    self.configuration_lines
                        .iter()
                        .enumerate()
                        .map(|(index, entry)| {
                            div()
                                .id(("preview-configuration-line", index))
                                .h(px(26.))
                                .w_full()
                                .child(entry.clone())
                        }),
                )
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .p_4()
                .child(cards[if response { selected } else { selected - 1 }].clone())
                .into_any_element()
        };
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .child(tabs)
            .child(content)
    }
}

impl Render for RequestPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(p.surface))
            .text_color(rgb(p.text))
            .text_size(px(12.))
            .child(
                div()
                    .px_4()
                    .py_3()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.name.clone()),
                    )
                    .child(
                        div()
                            .text_color(rgb(p.text_muted))
                            .child("只读 · 打开时快照"),
                    ),
            )
            .child(
                div()
                    .id("preview-endpoint")
                    .px_4()
                    .h(px(36.))
                    .flex_shrink_0()
                    .child(self.endpoint.clone()),
            )
            .child(self.panel(false, cx))
            .child(div().h(px(1.)).flex_shrink_0().bg(rgb(p.border)))
            .child(
                div()
                    .px_4()
                    .pt_3()
                    .flex_shrink_0()
                    .text_color(rgb(p.text_muted))
                    .child(self.status.clone()),
            )
            .child(self.panel(true, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn request_preview_json_folds_toggle_and_survive_tab_switches(cx: &mut gpui::TestAppContext) {
        cx.update(bind_code_card_keys);
        let json = serde_json::json!({"items": [{"name": "hidden value"}], "total": 1});
        let pretty = serde_json::to_string_pretty(&json).unwrap();
        let snapshot = Snapshot {
            name: "JSON preview".into(),
            draft: Draft::default(),
            resolver: std::rc::Rc::new(|_| Err("undefined".into())),
            response: Some(HttpResponse {
                status: 200,
                headers: vec![],
                text: json.to_string(),
                bytes: json.to_string().len(),
                json: Some(json),
            }),
            sent_request: None,
            events: None,
            error: None,
        };
        let (preview, window) =
            cx.add_window_view(|_, cx| RequestPreview::new(snapshot, ThemePreset::default(), cx));
        assert!(
            window.debug_bounds("json-fold-3").is_some(),
            "nested object has a fold toggle"
        );
        let array = window.debug_bounds("json-fold-2").unwrap();
        window.simulate_click(array.center(), gpui::Modifiers::default());
        assert!(
            window.debug_bounds("json-fold-3").is_none(),
            "folding the array hides its children"
        );
        let raw = window.debug_bounds("preview-response-tab-1").unwrap();
        window.simulate_click(raw.center(), gpui::Modifiers::default());
        assert!(
            window.debug_bounds("json-fold-1").is_none(),
            "raw response stays unfolded"
        );
        let formatted = window.debug_bounds("preview-response-tab-0").unwrap();
        window.simulate_click(formatted.center(), gpui::Modifiers::default());
        assert!(
            window.debug_bounds("json-fold-3").is_none(),
            "tab switching preserves folds"
        );
        window.update(|window, cx| {
            let editor = preview.read(cx).response_cards[0].clone();
            window.focus(&editor.focus_handle(cx), cx);
        });
        window.simulate_keystrokes("cmd-a cmd-c");
        assert_eq!(
            window.read_from_clipboard().unwrap().text().unwrap(),
            pretty,
            "copy includes folded content"
        );
        let array = window.debug_bounds("json-fold-2").unwrap();
        window.simulate_click(array.center(), gpui::Modifiers::default());
        assert!(
            window.debug_bounds("json-fold-3").is_some(),
            "expanding restores nested objects"
        );
    }

    #[gpui::test]
    fn request_preview_template_rendering_uses_the_clicked_collection(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(bind_text_entry_keys);
        let directory = tempfile::tempdir().unwrap();
        let host = cx.new(|cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&directory.path().join("templates.sqlite3")).unwrap(),
                cx,
            );
            for name in ["A", "B"] {
                host.group_edit = Some(GroupEdit::CreateCollection);
                host.group_name
                    .update(cx, |entry, cx| entry.set_value(name, cx));
                host.save_group(cx);
                host.add_request(cx);
            }
            host.group_edit = Some(GroupEdit::CreateEnvironment);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("Development", cx));
            host.save_group(cx);
            host.environment.update(cx, |environment, cx| {
                environment.set_selected(Some("1".into()), cx)
            });
            host.environment_inputs[1]
                .save(None, "openapi_baseurl", "https://global.test", false)
                .unwrap();
            for (collection, url) in [("A", "https://a.test"), ("B", "https://b.test")] {
                host.scoped_environment_mut(1, Some(collection))
                    .unwrap()
                    .save(None, "openapi_baseurl", url, false)
                    .unwrap();
            }
            host.requests[0].draft.url = "{{openapi_baseurl}}/v7/recent_chats/tags".into();
            host.requests[0].draft.params = vec![Pair::new("next", "{{openapi_baseurl}}/next", "")];
            host
        });
        let snapshot = host.read_with(cx, |host, cx| {
            assert_eq!(
                (host.reference_resolver(cx))("{{openapi_baseurl}}").unwrap(),
                "https://b.test"
            );
            host.request_window_snapshot(0, cx).unwrap()
        });
        let resolver = snapshot.resolver.clone();
        assert_eq!(resolver("{{openapi_baseurl}}").unwrap(), "https://a.test");
        assert!(resolver("{{missing}}").is_err());
        host.update(cx, |host, cx| {
            host.choose(0, cx);
            assert_eq!(
                (host.reference_resolver(cx))("{{openapi_baseurl}}"),
                resolver("{{openapi_baseurl}}")
            );
            host.scoped_environment_mut(1, Some("A"))
                .unwrap()
                .replace(&[(
                    "openapi_baseurl".into(),
                    "https://changed.test".into(),
                    false,
                )])
                .unwrap();
        });
        let (preview, window) =
            cx.add_window_view(|_, cx| RequestPreview::new(snapshot, ThemePreset::default(), cx));
        window.run_until_parked();
        let endpoint = preview.read_with(window, |preview, cx| {
            assert_eq!(
                preview.endpoint.read(cx).value(),
                "GET {{openapi_baseurl}}/v7/recent_chats/tags"
            );
            assert!(
                preview
                    .configuration_lines
                    .iter()
                    .any(|line| line.read(cx).value() == "next: {{openapi_baseurl}}/next")
            );
            preview.endpoint.clone()
        });
        window.update(|window, cx| window.focus(&endpoint.focus_handle(cx), cx));
        window.simulate_input("cannot edit");
        endpoint.read_with(window, |entry, _| {
            assert_eq!(
                entry.value(),
                "GET {{openapi_baseurl}}/v7/recent_chats/tags"
            )
        });
        assert_eq!(
            resolver("{{openapi_baseurl}}").unwrap(),
            "https://a.test",
            "preview keeps the opening environment snapshot"
        );
    }

    #[gpui::test]
    fn request_preview_windows_snapshot_clicked_tab_and_isolate_edits(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&directory.path().join("preview.sqlite3")).unwrap(),
                cx,
            );
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.add_request(cx);
            host.requests[0].name = "First".into();
            host.requests[0].draft.url = "https://example.com/first".into();
            let draft = Draft {
                url: "https://example.com/unsaved".into(),
                body: "unsaved body".into(),
                ..Draft::default()
            };
            host.load(&draft, cx);
            host.response_data = Some(HttpResponse {
                status: 201,
                headers: vec![("x-snapshot".into(), "yes".into())],
                text: "{\"before\":true}".into(),
                bytes: 15,
                json: Some(serde_json::json!({"before": true})),
            });
            host
        });
        let active = host.read_with(window, |host, _| host.active);
        host.update(window, |host, cx| {
            host.tab_menu = Some((0, point(px(200.), px(100.))));
            cx.notify();
        });
        window.run_until_parked();
        let menu_item = window.debug_bounds("tab-open-snapshot").unwrap();
        window.simulate_click(menu_item.center(), gpui::Modifiers::default());
        window.update(|window, cx| {
            host.update(cx, |host, cx| {
                host.open_request_preview(active, window, cx);
                assert_eq!(host.active, active);
                assert_eq!(host.snapshot(cx).url, "https://example.com/unsaved");
            })
        });
        window.run_until_parked();
        let previews = window
            .windows()
            .into_iter()
            .filter_map(|window| window.downcast::<RequestPreview>())
            .collect::<Vec<_>>();
        assert_eq!(previews.len(), 2);
        let current = previews
            .iter()
            .copied()
            .find(|preview| {
                preview
                    .read_with(window, |preview, cx| {
                        preview.endpoint.read(cx).value().contains("unsaved")
                    })
                    .unwrap()
            })
            .unwrap();
        let first = previews
            .iter()
            .copied()
            .find(|preview| {
                preview
                    .read_with(window, |preview, _| preview.name == "First")
                    .unwrap()
            })
            .unwrap();
        first
            .read_with(window, |preview, cx| {
                assert!(preview.endpoint.read(cx).value().contains("/first"));
                assert_eq!(preview.status, "暂无响应");
                assert!(
                    preview.request_cards[1]
                        .read(cx)
                        .content()
                        .contains("尚未发送")
                );
            })
            .unwrap();
        let body = current
            .read_with(window, |preview, cx| {
                assert_eq!(preview.request_cards[0].read(cx).content(), "unsaved body");
                assert!(
                    preview.response_cards[0]
                        .read(cx)
                        .content()
                        .contains("before")
                );
                assert_eq!(
                    preview.response_cards[2].read(cx).content(),
                    "x-snapshot: yes"
                );
                preview.request_cards[0].clone()
            })
            .unwrap();
        host.update(window, |host, cx| {
            host.body
                .update(cx, |body, cx| body.set_content("changed", cx));
            host.response_data = None;
            host.requests[active].draft.body = "changed stored draft".into();
        });
        current
            .update(window, |preview, child_window, cx| {
                preview.request_tab = 1;
                child_window.focus(&body.focus_handle(cx), cx);
                cx.notify();
            })
            .unwrap();
        let mut child = gpui::VisualTestContext::from_window(current.into(), window);
        child.simulate_input("should not edit");
        current
            .read_with(window, |preview, cx| {
                assert_eq!(body.read(cx).content(), "unsaved body");
                assert!(
                    preview.response_cards[0]
                        .read(cx)
                        .content()
                        .contains("before")
                );
            })
            .unwrap();
        current
            .update(window, |_, window, _| window.remove_window())
            .unwrap();
        first
            .read_with(window, |preview, _| assert_eq!(preview.name, "First"))
            .unwrap();
        host.update(window, |host, _| assert_eq!(host.active, active));
    }
}
