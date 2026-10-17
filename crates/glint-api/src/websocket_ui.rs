use super::*;

impl Workbench {
    fn ws_append(&mut self, text: String, cx: &mut Context<Self>) {
        let text = if text.len() > 64 * 1024 {
            let mut end = 64 * 1024;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}\n[消息显示已截断]", &text[..end])
        } else {
            text
        };
        self.ws_sequence = self.ws_sequence.saturating_add(1);
        self.ws_log.push_back(format!(
            "[{}] {text}",
            chrono::Local::now().format("%H:%M:%S")
        ));
        while self.ws_log.len() > 500
            || self.ws_log.iter().map(String::len).sum::<usize>() > 1024 * 1024
        {
            self.ws_log.pop_front();
        }
        let content = self.ws_log.iter().cloned().collect::<Vec<_>>().join("\n\n");
        self.ws_output
            .update(cx, |output, cx| output.set_content(content, cx));
        cx.notify();
    }
    pub(super) fn connect_websocket(&mut self, cx: &mut Context<Self>) {
        let draft = self.snapshot(cx);
        let environment_index = self.selected_environment(cx);
        let inputs = &self.environment_inputs[environment_index];
        let variables = self.active_environment_values(cx);
        let prepared = PreparedRequest::new(&draft, &variables)
            .and_then(|request| self.oauth_attach(request, &draft, cx))
            .and_then(|request| request.with_proxy(&inputs.proxy))
            .and_then(|request| {
                if !matches!(request.url.scheme(), "ws" | "wss") {
                    return Err("WebSocket 地址必须使用 ws:// 或 wss://".into());
                }
                plugins::validate_variables(&draft.plugins, &self.plugin_catalog, &variables)?;
                Ok(request)
            });
        let request = match prepared {
            Ok(request) => request,
            Err(error) => {
                self.error = Some(error.clone());
                self.notice = error.clone();
                self.ws_append(format!("错误 · {error}"), cx);
                return;
            }
        };
        self.capture_draft(cx);
        self.config = 3;
        self.sent_curls[self.active] = None;
        self.sent_requests[self.active] = Some(request.preview.clone());
        self.sync_request_preview(cx);
        self.ws_log.clear();
        self.error = None;
        self.responses[self.active] = None;
        self.response_data = None;
        self.response_json = None;
        self.status = None;
        self.ws_connected = false;
        self.ws_active = true;
        self.notice = "正在连接 WebSocket…".into();
        self.ws_append(self.notice.clone(), cx);
        let ids = draft.plugins;
        let catalog = self.plugin_catalog.clone();
        self.sending = Some(cx.spawn(async move |this, cx| {
            let prepared = cx
                .background_executor()
                .spawn(async move { plugins::apply(request, &ids, &catalog, &variables) })
                .await;
            if let Ok(request) = &prepared {
                let _ = this.update(cx, |this, cx| {
                    this.sent_requests[this.active] = Some(request.preview.clone());
                    this.sync_request_preview(cx);
                });
            }
            let result = prepared.and_then(websocket::start);
            match result {
                Ok(session) => {
                    let _ = this.update(cx, |this, cx| {
                        this.ws_commands = Some(session.commands.clone());
                        cx.notify();
                    });
                    while let Ok(event) = session.events.recv().await {
                        if this
                            .update(cx, |this, cx| {
                                let text = match event {
                                    websocket::Event::Connected => {
                                        this.ws_connected = true;
                                        this.notice = "WebSocket 已连接".into();
                                        "已连接 · 在请求体中编辑文本，点击发送消息".into()
                                    }
                                    websocket::Event::Sent(text) => format!("发送 → {text}"),
                                    websocket::Event::Text(text) => format!("接收 ← {text}"),
                                    websocket::Event::Binary(bytes) => {
                                        use base64::Engine;
                                        format!(
                                            "接收 ← 二进制 · {} B · Base64: {}",
                                            bytes.len(),
                                            base64::engine::general_purpose::STANDARD.encode(bytes)
                                        )
                                    }
                                    websocket::Event::Closed(reason) => {
                                        this.notice = reason.clone();
                                        reason
                                    }
                                    websocket::Event::Error(error) => {
                                        this.error = Some(error.clone());
                                        this.notice = format!("WebSocket 失败：{error}");
                                        this.notice.clone()
                                    }
                                };
                                this.ws_append(text, cx);
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.error = Some(error.clone());
                        this.notice = format!("WebSocket 失败：{error}");
                        this.ws_append(this.notice.clone(), cx);
                    });
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.ws_connected = false;
                this.ws_active = false;
                this.ws_commands = None;
                this.sending = None;
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(super) fn disconnect_websocket(&mut self, cx: &mut Context<Self>) {
        if self.ws_commands.is_none() || !self.ws_connected {
            self.sending = None;
            self.ws_active = false;
            self.ws_commands = None;
            self.ws_connected = false;
            self.notice = "已取消 WebSocket 连接".into();
            cx.notify();
            return;
        }
        if let Some(sender) = &self.ws_commands {
            if sender.try_send(websocket::Command::Close).is_err() {
                self.sending = None;
                self.ws_commands = None;
                self.ws_active = false;
            }
        }
        self.ws_connected = false;
        self.notice = "正在断开 WebSocket…".into();
        cx.notify();
    }
    pub(super) fn send_websocket_message(&mut self, cx: &mut Context<Self>) {
        if !self.ws_connected {
            return;
        }
        let draft = self.snapshot(cx);
        let values = self.active_environment_values(cx);
        let message = template::TemplateResolver::for_request(&values, &draft)
            .and_then(|resolver| resolver.resolve(&draft.body))
            .and_then(|text| {
                if text.len() > 1024 * 1024 {
                    return Err("发送消息不能超过 1 MiB".into());
                }
                self.ws_commands
                    .as_ref()
                    .ok_or("WebSocket 尚未连接")?
                    .try_send(websocket::Command::Text(text))
                    .map_err(|_| "消息队列已满或连接已断开".to_owned())
            });
        if let Err(error) = message {
            self.ws_append(format!("错误 · {error}"), cx);
        }
    }
    pub(super) fn websocket_panel(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(42.))
                    .flex_shrink_0()
                    .px_5()
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .child("WebSocket 消息")
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(if self.ws_connected {
                                "已连接"
                            } else if self.sending.is_some() {
                                "连接中 / 断开中"
                            } else {
                                "未连接"
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("清空消息")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ws_log.clear();
                                this.ws_output
                                    .update(cx, |output, cx| output.set_content("", cx));
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("发送消息")
                            .when(!self.ws_connected, |button| button.disabled())
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "websocket-send-message".into())
                            .on_click(
                                cx.listener(|this, _, _, cx| this.send_websocket_message(cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .px_5()
                    .py_2()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("请求体作为待发送的文本消息 · 支持环境变量 · 消息仅保留在当前会话中"),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px_5()
                    .child(self.ws_output.clone()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn websocket_draft_messages_validation_and_switch_cleanup(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.load(
                &Draft {
                    method: "WS".into(),
                    url: "ws://localhost:1234/echo".into(),
                    body: "你好".into(),
                    ..Draft::default()
                },
                cx,
            );
            host.config = 3;
            host
        });
        window.update(|window, _| window.refresh());
        assert!(window.debug_bounds("websocket-send-message").is_some());
        host.update(window, |host, cx| {
            assert_eq!(host.snapshot(cx).method, "WS");
            let saved = host.snapshot(cx);
            let (sender, receiver) = async_channel::bounded(16);
            host.ws_commands = Some(sender);
            host.ws_connected = true;
            host.send_websocket_message(cx);
            assert!(matches!(receiver.try_recv().unwrap(), websocket::Command::Text(text) if text == "你好"));
            host.disconnect_websocket(cx);
            assert!(matches!(receiver.try_recv().unwrap(), websocket::Command::Close));
            assert!(!host.ws_connected);
            host.capture_draft(cx);
            host.persist(cx);
            host.load(&Draft::default(), cx);
            assert!(host.ws_commands.is_none());
            assert!(receiver.is_closed());
            assert!(host.ws_log.is_empty());
            host.load(&Draft { method: "WS".into(), url: "https://localhost/".into(), ..Draft::default() }, cx);
            host.send(cx);
            assert!(host.sending.is_none());
            assert!(host.error.as_ref().unwrap().contains("ws://"));
            host.load(&saved, cx);
            host.capture_draft(cx);
            host.persist(cx);
        });
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(window, |host, cx| {
            assert_eq!(host.snapshot(cx).method, "WS");
            assert_eq!(host.snapshot(cx).url, "ws://localhost:1234/echo");
        });
    }
}
