//! Response display, search, HTML preview, traces, and sent-request snapshots.

use super::*;

impl Workbench {
    pub(super) fn trace_view(&self) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let entries = &self.workspace.traces[self.active];
        let mut body = div()
            .id("execution-trace-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .text_size(px(11.));
        if entries.is_empty() {
            return body
                .text_color(rgb(p.text_muted))
                .child("发送请求后显示执行轨迹。");
        }
        body = body.child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(p.text))
                .child(format!(
                    "{} 条记录 · {}",
                    entries.len(),
                    entries.last().unwrap().message
                )),
        );
        for entry in entries {
            body = body.child(
                div()
                    .flex()
                    .gap_3()
                    .items_start()
                    .flex_shrink_0()
                    .child(
                        div()
                            .w(px(64.))
                            .flex_shrink_0()
                            .font_family("Menlo")
                            .text_color(rgb(p.text_muted))
                            .child(format!("+{} ms", entry.elapsed_ms)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(rgb(p.text))
                            .child(entry.message.clone()),
                    ),
            );
        }
        body
    }

    pub(super) fn response_content(&self) -> String {
        let Some(response) = &self.response_data else {
            return String::new();
        };
        if self.pretty {
            if let Some(value) = &response.json {
                return serde_json::to_string_pretty(value).unwrap();
            }
        }
        response.text.clone()
    }

    pub(super) fn sync_response_search(&mut self, force: bool, cx: &mut Context<Self>) {
        let query = if self.show_response_search {
            self.response_search.read(cx).value().to_owned()
        } else {
            String::new()
        };
        if !force && self.response.read(cx).search_query() == query {
            return;
        }
        self.response_search_count = self.response.update(cx, |editor, cx| {
            editor.set_search_query(query, cx);
            let count = editor.search_match_count();
            if count > 0 {
                editor.reveal_search_match(0, cx);
            }
            count
        });
        self.response_search_index = (self.response_search_count > 0).then_some(0);
        cx.notify();
    }

    pub(super) fn step_response_search(&mut self, previous: bool, cx: &mut Context<Self>) {
        let count = self.response_search_count;
        if count == 0 {
            return;
        }
        let current = self.response_search_index.unwrap_or(0);
        let index = if previous {
            (current + count - 1) % count
        } else {
            (current + 1) % count
        };
        self.response_search_index = Some(index);
        self.response.update(cx, |editor, cx| {
            editor.reveal_search_match(index, cx);
        });
        cx.notify();
    }

    pub(super) fn toggle_response_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_json_pick(cx);
        self.show_response_search = !self.show_response_search;
        if self.show_response_search {
            self.html_preview = false;
            window.focus(&self.response_search.focus_handle(cx), cx);
        } else {
            window.focus(&self.focus, cx);
        }
        self.sync_response_search(true, cx);
    }

    pub(super) fn update_response(&mut self, cx: &mut Context<Self>) {
        self.cancel_json_pick(cx);
        self.json_menu = None;
        let headers = self
            .response_data
            .as_ref()
            .map(|response| response.headers.clone())
            .unwrap_or_default();
        self.response_header_entries = headers
            .iter()
            .map(|(key, value)| {
                [key, value].map(|text| {
                    cx.new(|cx| {
                        TextEntry::new(cx, "", text.clone())
                            .read_only(true)
                            .fit_content()
                            .font_size(11.)
                            .theme(self.theme)
                    })
                })
            })
            .collect();
        let rows = self
            .response_header_entries
            .iter()
            .enumerate()
            .map(|(index, entries)| {
                TableRow::new(
                    format!("response-header-{index}"),
                    entries.iter().enumerate().map(|(column, entry)| {
                        let entry = entry.clone();
                        TableCell::custom(move || {
                            div()
                                .id(("response-header-cell", index * 2 + column))
                                .debug_selector(move || {
                                    format!("response-header-{index}-{column}").into()
                                })
                                .w_full()
                                .min_w_0()
                                .h(px(26.))
                                .overflow_x_scroll()
                                .font_family("Menlo")
                                .text_size(px(11.))
                                .child(entry.clone())
                        })
                    }),
                )
            })
            .collect::<Vec<_>>();
        self.response_headers_table
            .update(cx, |table, cx| table.set_rows(rows, cx));
        self.response_headers_scroll
            .set_offset(point(px(0.), px(0.)));
        let content = self.response_content();
        self.json_nodes = if self.response_json.is_some() {
            json_picker::index(&content)
        } else {
            vec![]
        };
        self.html_preview = false;
        let language = self
            .response_data
            .as_ref()
            .map(HttpResponse::language)
            .unwrap_or("text");
        let folding = self.pretty && self.response_json.is_some();
        self.response.update(cx, |editor, cx| {
            editor.set_json_folding(folding, cx);
            editor.set_language(language, cx);
            editor.set_content(content, cx);
        });
        self.sync_response_search(true, cx);
    }

    #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
    pub(super) fn activate_html_window(&mut self, window: &Window, cx: &App) {
        let id = window.window_handle().window_id();
        let Some(previous) = self.html_window.replace(id) else {
            return;
        };
        if previous == id {
            return;
        }
        let open = cx
            .windows()
            .into_iter()
            .map(|window| window.window_id())
            .collect::<HashSet<_>>();
        self.html_windows.retain(|id, _| open.contains(id));
        let mut native = self.html_windows.remove(&id).unwrap_or_default();
        std::mem::swap(&mut self.html_webview, &mut native.0);
        std::mem::swap(&mut self.html_webview_source, &mut native.1);
        if open.contains(&previous) {
            self.html_windows.insert(previous, native);
        }
    }

    pub(super) fn preview_html(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
        {
            self.activate_html_window(window, cx);
            let Some(response) = self
                .response_data
                .as_ref()
                .filter(|r| r.language() == "html")
            else {
                return;
            };
            if self.html_webview_source.as_deref() == Some(response.text.as_str())
                && self.html_webview.is_some()
            {
                self.html_preview = true;
                self.show_response_search = false;
                self.sync_response_search(true, cx);
                cx.notify();
                return;
            }
            let text = response.text.clone();
            self.html_webview_source = Some(text.clone());
            let builder = gpui_glint::components::wry::WebViewBuilder::new()
                .with_html(&text)
                .with_incognito(true)
                .with_javascript_disabled()
                .with_navigation_handler(|url| url == "about:blank")
                .with_new_window_req_handler(|_, _| {
                    gpui_glint::components::wry::NewWindowResponse::Deny
                })
                .with_download_started_handler(|_, _| false);
            use gpui_glint::components::wry::raw_window_handle::HasWindowHandle;
            let result = window
                .window_handle()
                .map_err(|e| e.to_string())
                .and_then(|handle| {
                    builder
                        .with_visible(false)
                        .build_as_child(&handle)
                        .map_err(|e| e.to_string())
                });
            match result {
                Ok(native) => {
                    self.html_webview_source = Some(text);
                    self.html_webview =
                        Some(cx.new(|cx| gpui_glint::components::WebView::from_native(native, cx)));
                    self.html_preview = true;
                }
                Err(error) => self.notice = format!("无法打开 HTML 预览：{error}"),
            }
        }
        #[cfg(not(all(feature = "webview", any(target_os = "macos", target_os = "windows"))))]
        {
            let _ = window;
            self.notice =
                "HTML 预览需运行 cargo run -p glint-api --features webview（macOS / Windows）"
                    .into();
        }
        if self.html_preview {
            self.show_response_search = false;
            self.sync_response_search(true, cx);
        }
        cx.notify();
    }

    pub(super) fn sync_request_preview(&mut self, cx: &mut Context<Self>) {
        let content = self
            .workspace
            .sent_requests
            .get(self.active)
            .and_then(|s| s.as_ref())
            .cloned()
            .unwrap_or_default();
        self.request_preview
            .update(cx, |editor, cx| editor.set_content(content, cx));
    }

    pub(super) fn request_view(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let content = self
            .workspace
            .sent_requests
            .get(self.active)
            .and_then(|s| s.clone());
        let mut view = div().flex_1().min_h_0().flex().flex_col();
        if let Some(content) = content {
            view = view
                .child(
                    div()
                        .h(px(36.))
                        .flex_shrink_0()
                        .px_5()
                        .flex()
                        .items_center()
                        .text_size(px(10.))
                        .text_color(rgb(p.text_muted))
                        .child("最近一次发送的 HTTP 请求 · 后续编辑不会改变此快照")
                        .child(div().flex_1())
                        .child(
                            Button::new("复制请求")
                                .kind(ButtonKind::Plain)
                                .size(ButtonSize::Mini)
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        content.clone(),
                                    ))
                                })),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .px_5()
                        .child(self.request_preview.clone()),
                );
        } else {
            view = view
                .items_center()
                .justify_center()
                .text_color(rgb(p.text_muted))
                .child("发送请求后，在这里查看最终 URL、请求头和请求体");
        }
        view
    }

    pub(super) fn response_search_controls(
        &self,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        div()
            .id("response-search-bar")
            .debug_selector(|| "response-search-bar".into())
            .w(px(340.))
            .min_w(px(180.))
            .h(px(28.))
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .rounded(px(5.))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface_subtle))
            .child(
                Icon::Search
                    .render(12.)
                    .flex_shrink_0()
                    .text_color(rgb(p.text_muted)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.response_search.clone()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child(format!(
                        "{}/{}",
                        self.response_search_index.map_or(0, |i| i + 1),
                        self.response_search_count
                    )),
            )
            .child(
                IconButton::new(Icon::ArrowUp, "上一处（Shift+Enter）")
                    .plain()
                    .theme(self.theme)
                    .render()
                    .size(px(22.))
                    .on_click(cx.listener(|this, _, _, cx| this.step_response_search(true, cx))),
            )
            .child(
                IconButton::new(Icon::ChevronDown, "下一处（Enter）")
                    .plain()
                    .theme(self.theme)
                    .render()
                    .size(px(22.))
                    .on_click(cx.listener(|this, _, _, cx| this.step_response_search(false, cx))),
            )
            .child(
                IconButton::new(Icon::Close, "关闭搜索（Esc）")
                    .plain()
                    .theme(self.theme)
                    .render()
                    .size(px(22.))
                    .debug_selector(|| "response-search-close".into())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.toggle_response_search(window, cx)),
                    ),
            )
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if this.response_search.read(cx).is_composing() {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "enter" => this.step_response_search(event.keystroke.modifiers.shift, cx),
                    "escape" => this.toggle_response_search(window, cx),
                    _ => return,
                }
                cx.stop_propagation();
                window.prevent_default();
            }))
    }

    pub(super) fn response_panel(&self, cx: &mut Context<Self>) -> gpui::Div {
        if self.snapshot(cx).method == "WS" || self.ws_active {
            return self.websocket_panel(cx);
        }
        let p = self.theme.palette();
        let mut head = div()
            .h(px(42.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_4()
            .px_5()
            .border_b_1()
            .border_color(rgb(p.border))
            .child(div().font_weight(FontWeight::MEDIUM).child("响应"));
        for (i, label) in ["响应体", "响应头", "测试", "轨迹", "请求"]
            .iter()
            .enumerate()
        {
            head = head.child(
                tab_button(label, i == self.response_tab, self.theme).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.response_tab = i;
                        cx.notify();
                    },
                )),
            );
        }
        head = head.child(div().flex_1()).child(
            Button::new("复制 cURL")
                .icon(Icon::Copy)
                .kind(ButtonKind::Plain)
                .size(ButtonSize::Mini)
                .theme(self.theme)
                .when(
                    self.workspace
                        .sent_curls
                        .get(self.active)
                        .and_then(|c| c.as_ref())
                        .is_none(),
                    |button| button.disabled(),
                )
                .render()
                .debug_selector(|| "export-response-curl".into())
                .on_click(cx.listener(|this, _, _, cx| this.copy_response_curl(cx))),
        );
        if let Some(status) = self.status {
            let bytes = self
                .response_data
                .as_ref()
                .map_or(0, |response| response.bytes);
            head = head.child(
                div()
                    .font_family("Menlo")
                    .text_size(px(10.))
                    .text_color(rgb(self.theme.color(if status < 400 {
                        0x24864c
                    } else {
                        0xd9342b
                    })))
                    .child(format!(
                        "● {}   {} ms   {} B{}",
                        status_label(status),
                        self.trace_started
                            .filter(|_| self.sending.is_some())
                            .map(|started| started.elapsed().as_millis())
                            .or_else(|| self.workspace.requests[self.active]
                                .history
                                .get(
                                    self.selected_history
                                        .filter(|(request, _)| *request == self.active)
                                        .map_or(0, |(_, index)| index)
                                )
                                .and_then(|r| r.elapsed_ms))
                            .unwrap_or(0),
                        bytes,
                        if self.sending.is_some()
                            && self
                                .response_data
                                .as_ref()
                                .is_some_and(HttpResponse::is_event_stream)
                        {
                            " · SSE 接收中"
                        } else {
                            ""
                        }
                    )),
            );
        }
        let content = if self.response_tab == 4 {
            self.request_view(cx)
        } else if self.response_tab == 3 {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(self.trace_view())
        } else if self.response_data.is_none() {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .text_color(rgb(p.text_muted))
                .child(Icon::Play.render(28.))
                .child(div().text_size(px(13.)).text_color(rgb(p.text)).child(
                    if self.sending.is_some() {
                        "正在等待服务器响应"
                    } else if self.error.is_some() {
                        "无法发送请求"
                    } else {
                        "发送请求，查看响应"
                    },
                ))
                .child(
                    div()
                        .text_size(px(11.))
                        .child(self.error.clone().unwrap_or_else(|| {
                            if self.sending.is_some() {
                                "点击取消可终止本次请求".into()
                            } else {
                                "配置参数后点击发送，或按 ⌘ / Ctrl + Enter".into()
                            }
                        })),
                )
        } else if self.response_tab == 1 {
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .flex()
                .flex_col()
                .overflow_hidden()
                .child(
                    div()
                        .id("response-headers-scroll")
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .overflow_y_scroll()
                        .track_scroll(&self.response_headers_scroll)
                        .px_5()
                        .py_3()
                        .child(self.response_headers_table.clone()),
                )
        } else if self.response_tab == 2 {
            let success = self.status.is_some_and(|s| s < 400);
            div()
                .p_5()
                .text_size(px(12.))
                .text_color(rgb(self.theme.color(if success {
                    0x24864c
                } else {
                    0xd9342b
                })))
                .child(format!(
                    "{} 状态码小于 400\n{} 响应为有效 JSON",
                    if success { "✓" } else { "✕" },
                    if self.response_json.is_some() {
                        "✓"
                    } else {
                        "✕"
                    }
                ))
        } else {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .id("response-body-toolbar")
                        .debug_selector(|| "response-body-toolbar".into())
                        .h(px(36.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_5()
                        .child({
                            let html = self
                                .response_data
                                .as_ref()
                                .is_some_and(|response| response.language() == "html");
                            let mut options = vec![
                                SegmentOption::new(if self.response_json.is_some() {
                                    "格式化"
                                } else {
                                    "源码"
                                }),
                                SegmentOption::new("原始"),
                            ];
                            if html {
                                options.push(SegmentOption::new("预览"));
                            }
                            let owner = cx.entity().downgrade();
                            SegmentedControl::new(options)
                                .id("response-format-segments")
                                .selected(if self.html_preview && html {
                                    2
                                } else if self.pretty {
                                    0
                                } else {
                                    1
                                })
                                .theme(self.theme)
                                .on_change(move |selected, window, cx| {
                                    let _ = owner.update(cx, |this, cx| {
                                        if selected == 2 {
                                            this.preview_html(window, cx);
                                        } else {
                                            this.pretty = selected == 0;
                                            this.update_response(cx);
                                        }
                                        cx.notify();
                                    });
                                })
                                .render()
                                .flex_shrink_0()
                        })
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.text_muted))
                                .max_w(px(280.))
                                .truncate()
                                .child(if self.json_pick.is_some() {
                                    self.json_hover
                                        .and_then(|index| self.json_nodes.get(index))
                                        .map(|node| node.path.clone())
                                        .unwrap_or_else(|| "点击字段选取 · Esc 取消".into())
                                } else if self
                                    .response_data
                                    .as_ref()
                                    .is_some_and(|r| r.language() == "html")
                                {
                                    "HTML 响应".to_owned()
                                } else if self.response_json.is_some() {
                                    "JSON 响应".to_owned()
                                } else {
                                    "文本响应".to_owned()
                                }),
                        )
                        .when(self.json_pick.is_some(), |toolbar| {
                            toolbar.child(
                                Button::new("取消选取")
                                    .kind(ButtonKind::Plain)
                                    .size(ButtonSize::Mini)
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(|| "cancel-json-pick".into())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel_json_pick(cx);
                                        cx.notify();
                                    })),
                            )
                        })
                        .child(div().flex_1())
                        .when(self.show_response_search, |toolbar| {
                            toolbar.child(self.response_search_controls(cx))
                        })
                        .when(!self.show_response_search, |toolbar| {
                            toolbar.child(
                                IconButton::new(Icon::Search, "搜索响应体（⌘ / Ctrl + F）")
                                    .plain()
                                    .theme(self.theme)
                                    .render()
                                    .size(px(26.))
                                    .id("response-search-toggle")
                                    .debug_selector(|| "response-search-toggle".into())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_response_search(window, cx)
                                    })),
                            )
                        })
                        .child(
                            IconButton::new(Icon::Copy, "复制响应")
                                .plain()
                                .theme(self.theme)
                                .render()
                                .size(px(26.))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if this.response_data.is_some() {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            this.response_content(),
                                        ));
                                        this.notice = "已复制响应".into();
                                        cx.notify();
                                    }
                                })),
                        ),
                )
                .child(
                    div()
                        .id("response-scroll")
                        .debug_selector(|| "response-scroll".into())
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px_3()
                        .pb_3()
                        .child(self.response_view(cx)),
                )
        };
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(head)
            .child(content)
    }

    pub(super) fn response_view(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
        if self.html_preview {
            if let Some(view) = &self.html_webview {
                return div().size_full().child(view.clone()).into_any_element();
            }
        }
        self.json_response_view(cx).into_any_element()
    }
}
