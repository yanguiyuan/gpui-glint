use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum WsMenu {
    Content,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MessageKind {
    Status,
    Sent,
    Received,
    Binary,
    Error,
}
impl MessageKind {
    fn label(self) -> &'static str {
        match self {
            Self::Status => "状态",
            Self::Sent => "发送",
            Self::Received => "接收",
            Self::Binary => "二进制",
            Self::Error => "错误",
        }
    }
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct WsMessage {
    pub(super) id: u64,
    time: String,
    kind: MessageKind,
    payload: String,
    summary: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(skip)]
    title_decoded: bool,
}
impl WsMessage {
    pub(super) fn mcp_value(&self) -> serde_json::Value {
        serde_json::json!({"sequence": self.id, "message": self.log_text(), "title": self.title, "payload": self.payload, "direction": self.kind.label(), "time": self.time})
    }
    pub(super) fn was_connected(&self) -> bool {
        self.kind == MessageKind::Status && self.payload == "已连接"
    }
    fn byte_len(&self) -> usize {
        if self.kind == MessageKind::Binary {
            self.payload.len() / 4 * 3
                - self
                    .payload
                    .bytes()
                    .rev()
                    .take_while(|&ch| ch == b'=')
                    .count()
        } else {
            self.payload.len()
        }
    }
    pub(super) fn log_text(&self) -> String {
        let prefix = match self.kind {
            MessageKind::Sent => "发送 → ",
            MessageKind::Received => "接收 ← ",
            MessageKind::Binary => "接收 ← 二进制 · Base64: ",
            _ => "",
        };
        format!("[{}] {prefix}{}", self.time, self.payload)
    }
}

pub(super) fn decoder_bindings(
    catalog: &[gpui_glint::plugins::Plugin],
) -> Vec<(String, model::BeforeAction, String)> {
    catalog
        .iter()
        .flat_map(|plugin| {
            plugin
                .manifest
                .contributions
                .iter()
                .filter(|c| c.kind == plugins::EVENT_DECODE)
                .map(move |action| {
                    (
                        format!("{}#{}", plugin.identity(), action.id),
                        model::BeforeAction {
                            approval: gpui_glint::plugins::Approval::for_plugin(plugin),
                            action_id: action.id.clone(),
                            enabled: true,
                        },
                        action.title.clone(),
                    )
                })
        })
        .collect()
}
pub(super) fn decoder_options(catalog: &[gpui_glint::plugins::Plugin]) -> Vec<SelectOption> {
    std::iter::once(SelectOption::new("", "不使用解密插件"))
        .chain(
            decoder_bindings(catalog)
                .into_iter()
                .map(|(value, _, label)| SelectOption::new(value, label)),
        )
        .collect()
}

impl Workbench {
    pub(super) fn ws_refresh_decoder_select(&mut self, cx: &mut Context<Self>) {
        let mut options = decoder_options(&self.enabled_plugin_catalog());
        if let Some(binding) = self
            .requests
            .get(self.active)
            .and_then(|r| r.draft.event_decoder.as_ref())
        {
            if let Some(plugin) = self
                .plugin_catalog
                .iter()
                .find(|p| p.identity() == binding.id && !self.plugin_enabled(p))
            {
                options.push(
                    SelectOption::new(
                        format!("{}#{}", binding.id, binding.action_id),
                        format!("{}（已禁用）", plugin.manifest.name),
                    )
                    .disabled(true),
                );
            }
        }
        let selected = self
            .requests
            .get(self.active)
            .and_then(|r| r.draft.event_decoder.as_ref())
            .map(|b| format!("{}#{}", b.id, b.action_id))
            .unwrap_or_default();
        self.ws_decoder_select.update(cx, |select, cx| {
            select.set_options(options, cx);
            select.set_selected(Some(selected.into()), cx);
        });
    }
    pub(super) fn ws_select_sync(&mut self, cx: &mut Context<Self>) {
        self.ws_decode_generation = self.ws_decode_generation.saturating_add(1);
        self.ws_decrypted = None;
        if self.ws_detail_tab == 1
            || self
                .requests
                .get(self.active)
                .is_some_and(|request| request.draft.event_decoder.is_some())
        {
            self.ws_start_decode(cx);
        } else {
            self.ws_sync_detail(cx);
        }
    }
    fn ws_start_decode(&mut self, cx: &mut Context<Self>) {
        self.ws_decode_generation = self.ws_decode_generation.saturating_add(1);
        self.ws_decrypted = None;
        if self.ws_decode_task.is_some() {
            self.ws_sync_detail(cx);
            return;
        }
        let Some(binding) = self
            .requests
            .get(self.active)
            .and_then(|r| r.draft.event_decoder.clone())
        else {
            self.ws_decrypted = Some(Err(
                "请选择事件解密插件；WPS 插件读取当前环境的 wps_app_id、wps_app_key。".into(),
            ));
            self.ws_sync_detail(cx);
            return;
        };
        if !binding.enabled || self.plugin_identity_disabled(&binding.id) {
            self.ws_decrypted = Some(Err("事件解密插件已禁用，请在插件管理中启用。".into()));
            self.ws_sync_detail(cx);
            return;
        }
        let Some(event) = self
            .ws_log
            .iter()
            .find(|event| Some(event.id) == self.ws_selected)
        else {
            self.ws_decrypted = Some(Err("选择接收事件后查看解密结果。".into()));
            self.ws_sync_detail(cx);
            return;
        };
        if event.kind != MessageKind::Received {
            self.ws_decrypted = Some(Err(
                "仅对接收到的文本事件执行解密；状态、发送及二进制消息请查看原始事件。".into(),
            ));
            self.ws_sync_detail(cx);
            return;
        }
        self.ws_decode_message(event.id, cx);
    }
    pub(super) fn ws_invalidate_titles(&mut self) {
        for event in &mut self.ws_log {
            event.title = None;
            event.title_decoded = false;
        }
    }
    fn ws_decode_next(&mut self, cx: &mut Context<Self>) {
        if self.ws_decode_task.is_some() {
            return;
        }
        if let Some(id) = self
            .ws_log
            .iter()
            .find(|event| event.kind == MessageKind::Received && !event.title_decoded)
            .map(|event| event.id)
        {
            self.ws_decode_message(id, cx);
        }
    }
    fn ws_decode_message(&mut self, event_id: u64, cx: &mut Context<Self>) {
        if self.ws_decode_task.is_some() {
            return;
        }
        let Some(binding) = self
            .requests
            .get(self.active)
            .and_then(|request| request.draft.event_decoder.clone())
        else {
            return;
        };
        if !binding.enabled || self.plugin_identity_disabled(&binding.id) {
            return;
        }
        let Some(event) = self.ws_log.iter().find(|event| event.id == event_id) else {
            return;
        };
        let payload = event.payload.clone();
        let generation = self.ws_decode_generation;
        let catalog = self.enabled_plugin_catalog();
        let variables = self.websocket_variables(cx);
        let request_id = self.requests[self.active].id.clone();
        let expected_binding = binding.clone();
        let expected_variables = variables.0.clone();
        self.ws_sync_detail(cx);
        self.ws_decode_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    plugins::decode_event_presentation(&binding, &catalog, &variables, &payload)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.with_ws_request(&request_id, cx, |this, cx| {
                    if this
                        .requests
                        .get(this.active)
                        .and_then(|request| request.draft.event_decoder.as_ref())
                        != Some(&expected_binding)
                        || this.plugin_identity_disabled(&expected_binding.id)
                        || this.websocket_variables(cx).0 != expected_variables
                    {
                        this.ws_decode_task = None;
                        this.ws_invalidate_titles();
                        this.ws_decode_next(cx);
                        return;
                    }
                    this.ws_finish_decode(generation, event_id, result, cx);
                });
            });
        }));
    }

    fn ws_finish_decode(
        &mut self,
        generation: u64,
        event_id: u64,
        result: Result<plugins::DecodedEvent, String>,
        cx: &mut Context<Self>,
    ) {
        self.ws_decode_task = None;
        if let Some(event) = self.ws_log.iter_mut().find(|event| event.id == event_id) {
            event.title_decoded = true;
            event.title = result.as_ref().ok().and_then(|event| event.title.clone());
        }
        if let Some((request_id, record_id, _)) = &self.ws_history {
            if let Some(event) = self
                .requests
                .iter_mut()
                .find(|request| &request.id == request_id)
                .and_then(|request| {
                    request
                        .history
                        .iter_mut()
                        .find(|record| &record.id == record_id)
                })
                .and_then(|record| record.exchange.as_mut())
                .and_then(|exchange| {
                    exchange
                        .websocket_events
                        .iter_mut()
                        .find(|event| event.id == event_id)
                })
            {
                event.title = result.as_ref().ok().and_then(|result| result.title.clone());
                self.storage_dirty = true;
            }
        }
        if self.ws_decode_generation == generation && self.ws_selected == Some(event_id) {
            self.ws_decrypted = Some(result.map(|event| event.data));
            self.ws_sync_detail(cx);
        } else if self.ws_detail_tab == 1 && self.ws_decrypted.is_none() {
            self.ws_start_decode(cx);
        }
        self.ws_decode_next(cx);
        cx.notify();
    }

    fn ws_detail_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity().downgrade();
        SegmentedControl::new([SegmentOption::new("原始事件"), SegmentOption::new("解密后")])
            .id("websocket-detail-tab")
            .selected(self.ws_detail_tab)
            .theme(self.theme)
            .on_change(move |tab, _, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.ws_menu = None;
                    this.ws_detail_tab = tab;
                    this.ws_refresh_decoder_select(cx);
                    this.ws_select_sync(cx);
                    cx.notify();
                });
            })
            .render()
            .debug_selector(|| "websocket-detail-tab".into())
    }

    fn ws_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let item = |label: &str, selector: &'static str| {
            let disabled = selector == "websocket-copy-payload"
                && (self.ws_selected.is_none()
                    || (self.ws_detail_tab == 1 && !matches!(self.ws_decrypted, Some(Ok(_)))));
            let icon = match selector {
                "websocket-clear" => Icon::Trash,
                "websocket-decode-retry" => Icon::Refresh,
                _ => Icon::Copy,
            };
            MenuItem::new(selector, label.to_owned())
                .icon(icon)
                .disabled(disabled)
                .theme(self.theme)
                .render()
                .debug_selector(move || selector.into())
        };
        let separator = || div().my_1().h(px(1.)).bg(rgb(p.border));
        let content = {
            let copy_label = if self.ws_detail_tab == 1 {
                "复制解密结果"
            } else if self
                .ws_log
                .iter()
                .find(|event| Some(event.id) == self.ws_selected)
                .is_some_and(|event| event.kind == MessageKind::Binary)
            {
                "复制 Base64"
            } else {
                "复制原文"
            };
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    item(copy_label, "websocket-copy-payload").on_click(cx.listener(
                        |this, _, _, cx| {
                            let text = if this.ws_detail_tab == 1 {
                                match &this.ws_decrypted {
                                    Some(Ok(text)) => Some(text.clone()),
                                    _ => None,
                                }
                            } else {
                                this.ws_log
                                    .iter()
                                    .find(|event| Some(event.id) == this.ws_selected)
                                    .map(|event| event.payload.clone())
                            };
                            if let Some(text) = text {
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                                this.notice = if this.ws_detail_tab == 1 {
                                    "已复制解密结果"
                                } else {
                                    "已复制事件原文"
                                }
                                .into();
                            }
                            this.ws_menu = None;
                            cx.notify();
                        },
                    )),
                )
                .when(self.ws_detail_tab == 1, |menu| {
                    menu.child(separator()).child(
                        item("重新解密", "websocket-decode-retry").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.ws_menu = None;
                                this.ws_start_decode(cx);
                                cx.notify();
                            },
                        )),
                    )
                })
        }
        .child(separator())
        .child(
            item("清空消息", "websocket-clear")
                .on_click(cx.listener(|this, _, _, cx| this.clear_websocket_messages(cx))),
        );
        let kind = WsMenu::Content;
        let open = self.ws_menu == Some(kind);
        let owner = cx.entity().downgrade();
        DropdownMenu::new(
            "websocket-actions-menu",
            Button::new("操作 ▾")
                .kind(ButtonKind::Plain)
                .size(ButtonSize::Mini)
                .theme(self.theme)
                .render()
                .debug_selector(|| "websocket-content-trigger".into()),
            content.occlude(),
        )
        .open(open)
        .width(180.)
        .theme(self.theme)
        .on_open_change(move |open, _, cx| {
            let _ = owner.update(cx, |this, cx| {
                // An outside click from the other menu must not close the newly opened one.
                if open {
                    this.ws_menu = Some(kind);
                } else if this.ws_menu == Some(kind) {
                    this.ws_menu = None;
                }
                cx.notify();
            });
        })
        .render()
    }

    pub(super) fn ws_append(&mut self, text: String, cx: &mut Context<Self>) {
        self.ws_push(MessageKind::Status, text, cx);
    }
    pub(super) fn ws_push(&mut self, kind: MessageKind, payload: String, cx: &mut Context<Self>) {
        self.ws_sequence = self.ws_sequence.saturating_add(1);
        let summary = if payload.is_empty() {
            "（空消息）".into()
        } else {
            let mut text = payload
                .chars()
                .take(100)
                .map(|ch| if ch.is_whitespace() { ' ' } else { ch })
                .collect::<String>();
            if payload.chars().take(101).count() > 100 {
                text.push('…');
            }
            text
        };
        self.ws_log.push_back(WsMessage {
            id: self.ws_sequence,
            time: chrono::Local::now().format("%H:%M:%S").to_string(),
            kind,
            payload,
            summary,
            title: None,
            title_decoded: false,
        });
        while self.ws_log.len() > 500
            || self
                .ws_log
                .iter()
                .map(|event| event.payload.len())
                .sum::<usize>()
                > 8 * 1024 * 1024
        {
            self.ws_log.pop_front();
        }
        self.ws_record_event();
        if self.ws_follow
            || !self
                .ws_log
                .iter()
                .any(|event| Some(event.id) == self.ws_selected)
        {
            self.ws_selected = self.ws_log.back().map(|event| event.id);
            self.ws_select_sync(cx);
            if self.ws_follow {
                self.ws_scroll
                    .scroll_to_item(self.ws_log.len().saturating_sub(1));
            }
        }
        self.ws_decode_next(cx);
        cx.notify();
    }
    pub(super) fn ws_sync_detail(&mut self, cx: &mut Context<Self>) {
        if !self.ws_background_update {
            self.ws_format_select.update(cx, |select, cx| {
                select.set_selected(Some(if self.ws_raw { "raw" } else { "pretty" }.into()), cx)
            });
        }
        self.ws_detail_scroll
            .set_offset(gpui::point(px(0.), px(0.)));
        let original = self
            .ws_log
            .iter()
            .find(|event| Some(event.id) == self.ws_selected)
            .map(|event| event.payload.as_str())
            .unwrap_or("");
        let payload = if self.ws_detail_tab == 1 {
            match &self.ws_decrypted {
                Some(Ok(text)) | Some(Err(text)) => text.as_str(),
                None => "正在解密事件…",
            }
        } else {
            original
        };
        let formatted = if self.ws_raw {
            None
        } else {
            serde_json::from_str::<serde_json::Value>(payload)
                .ok()
                .and_then(|json| serde_json::to_string_pretty(&json).ok())
        };
        self.ws_output.update(cx, |output, cx| {
            output.set_prose(formatted.is_none(), cx);
            output.set_json_folding(formatted.is_some(), cx);
            output.set_language(if formatted.is_some() { "json" } else { "text" }, cx);
            output.set_content(formatted.as_deref().unwrap_or(payload).to_owned(), cx);
        });
    }
    pub(super) fn clear_websocket_messages(&mut self, cx: &mut Context<Self>) {
        self.ws_decode_generation = self.ws_decode_generation.saturating_add(1);
        self.ws_decode_task = None;
        self.ws_decrypted = None;
        self.ws_menu = None;
        self.ws_log.clear();
        self.ws_selected = None;
        self.ws_follow = self.ws_follow_latest;
        self.ws_output
            .update(cx, |output, cx| output.set_content("", cx));
        cx.notify();
    }
    fn ws_begin_history(&mut self, draft: &Draft, cx: &mut Context<Self>) {
        self.ws_finish_history("WS 已结束");
        self.ws_history = None;
        self.clear_websocket_messages(cx);
        if let Some(preview) = self.sent_requests.get_mut(self.active) {
            *preview = None;
        }
        let Some(request) = self.requests.get(self.active) else {
            return;
        };
        let request_id = request.id.clone();
        let record_id = model::new_id();
        self.prepend_history(
            self.active,
            Record {
                id: record_id.clone(),
                tag: String::new(),
                source: Default::default(),
                environment_name: Some(
                    self.environment_name(self.selected_environment(cx))
                        .to_owned(),
                ),
                draft: draft.clone(),
                label: "WS 连接中".into(),
                time: record_time(),
                detail: "WebSocket 会话 · 0 条事件".into(),
                elapsed_ms: None,
                exchange: Some(model::Exchange {
                    websocket_connected: false,
                    websocket_events: vec![],
                    curl: None,
                    response: None,
                    error: None,
                    preview: None,
                    traces: vec![],
                }),
            },
        );
        self.ws_history = Some((request_id, record_id, Instant::now()));
        self.storage_dirty = true;
    }
    fn ws_record_event(&mut self) {
        let Some((request_id, record_id, _)) = &self.ws_history else {
            return;
        };
        let Some(event) = self.ws_log.back().cloned() else {
            return;
        };
        let Some(index) = self.requests.iter().position(|r| &r.id == request_id) else {
            return;
        };
        let Some(record) = self.requests[index]
            .history
            .iter_mut()
            .find(|r| &r.id == record_id)
        else {
            return;
        };
        let Some(exchange) = &mut record.exchange else {
            return;
        };
        exchange.preview = self.sent_requests[index].clone();
        if event.kind == MessageKind::Error {
            exchange.error = Some(event.payload.clone());
            record.label = "WS 失败".into();
        } else if self.ws_connected {
            record.label = "WS 已连接".into();
        }
        exchange.websocket_connected |= event.was_connected();
        exchange.websocket_events.push(event);
        while exchange.websocket_events.len() > 500
            || exchange
                .websocket_events
                .iter()
                .map(|event| event.payload.len())
                .sum::<usize>()
                > 8 * 1024 * 1024
        {
            exchange.websocket_events.remove(0);
        }
        record.detail = format!(
            "WebSocket 会话 · {} 条事件",
            exchange.websocket_events.len()
        );
        self.storage_dirty = true;
    }
    pub(super) fn ws_finish_history(&mut self, label: &str) {
        let Some((request_id, record_id, started)) = &self.ws_history else {
            return;
        };
        if let Some(record) = self
            .requests
            .iter_mut()
            .find(|request| &request.id == request_id)
            .and_then(|request| {
                request
                    .history
                    .iter_mut()
                    .find(|record| &record.id == record_id)
            })
        {
            record.elapsed_ms = Some(started.elapsed().as_millis());
            if record
                .exchange
                .as_ref()
                .is_none_or(|exchange| exchange.error.is_none())
            {
                record.label = label.into();
            }
            self.storage_dirty = true;
        }
    }
    pub(super) fn restore_ws_history(&mut self, events: &[WsMessage], cx: &mut Context<Self>) {
        self.ws_history = None;
        self.ws_log = events.iter().cloned().collect();
        self.ws_sequence = self
            .ws_sequence
            .max(events.iter().map(|event| event.id).max().unwrap_or(0));
        self.ws_selected = self.ws_log.back().map(|event| event.id);
        self.ws_select_sync(cx);
    }

    pub(super) fn connect_websocket(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.shared_request_editor(self.active) {
            editor.update(cx, |editor, cx| editor.connect_websocket(cx));
            return;
        }
        let draft = self.snapshot(cx);
        self.ws_begin_history(&draft, cx);
        let environment_index = self.selected_environment(cx);
        let proxy = self.effective_proxy(environment_index);
        let variables = self.active_environment_values(cx);
        self.ws_environment = Some(variables.clone());
        let request_id = self.requests[self.active].id.clone();
        let prepared = PreparedRequest::new(&draft, &variables)
            .and_then(|request| self.oauth_attach(request, &draft, cx))
            .and_then(|request| request.with_proxy(&proxy))
            .and_then(|request| {
                if !matches!(request.url.scheme(), "ws" | "wss") {
                    return Err("WebSocket 地址必须使用 ws:// 或 wss://".into());
                }
                plugins::validate_variables(
                    &self.enabled_before_actions(&draft.plugins),
                    &self.plugin_catalog,
                    &variables,
                )?;
                Ok(request)
            });
        let request = match prepared {
            Ok(request) => request,
            Err(error) => {
                self.error = Some(error.clone());
                self.notice = error.clone();
                self.ws_push(MessageKind::Error, error, cx);
                self.ws_finish_history("WS 失败");
                self.ws_history = None;
                return;
            }
        };
        self.capture_draft(cx);
        self.config = 3;
        self.sent_curls[self.active] = None;
        self.sent_requests[self.active] = Some(request.preview.clone());
        self.sync_request_preview(cx);
        self.error = None;
        self.responses[self.active] = None;
        self.response_data = None;
        self.response_json = None;
        self.status = None;
        self.ws_connected = false;
        self.ws_active = true;
        self.notice = "正在连接 WebSocket…".into();
        self.ws_append(self.notice.clone(), cx);
        let ids = self.enabled_before_actions(&draft.plugins);
        let catalog = self.plugin_catalog.clone();
        self.sending = Some(cx.spawn(async move |this, cx| {
            let prepared = cx
                .background_executor()
                .spawn(async move { plugins::apply(request, &ids, &catalog, &variables) })
                .await;
            if let Ok(request) = &prepared {
                let _ = this.update(cx, |this, cx| {
                    this.with_ws_request(&request_id, cx, |this, cx| {
                        this.sent_requests[this.active] = Some(request.preview.clone());
                        if !this.ws_background_update {
                            this.sync_request_preview(cx);
                        }
                    });
                });
            }
            let result = prepared.and_then(websocket::start);
            match result {
                Ok(session) => {
                    let _ = this.update(cx, |this, cx| {
                        this.with_ws_request(&request_id, cx, |this, cx| {
                            this.ws_commands = Some(session.commands.clone());
                            cx.notify();
                        });
                    });
                    while let Ok(event) = session.events.recv().await {
                        if this
                            .update(cx, |this, cx| {
                                this.with_ws_request(&request_id, cx, |this, cx| {
                                    let (kind, payload) = match event {
                                        websocket::Event::Connected => {
                                            this.ws_connected = true;
                                            this.notice = "WebSocket 已连接".into();
                                            (MessageKind::Status, "已连接".into())
                                        }
                                        websocket::Event::Sent(text) => (MessageKind::Sent, text),
                                        websocket::Event::Text(text) => {
                                            (MessageKind::Received, text)
                                        }
                                        websocket::Event::Binary(bytes) => {
                                            use base64::Engine;
                                            (
                                                MessageKind::Binary,
                                                base64::engine::general_purpose::STANDARD
                                                    .encode(bytes),
                                            )
                                        }
                                        websocket::Event::Closed(reason) => {
                                            this.notice = reason.clone();
                                            (MessageKind::Status, reason)
                                        }
                                        websocket::Event::Error(error) => {
                                            this.error = Some(error.clone());
                                            this.notice = format!("WebSocket 失败：{error}");
                                            (MessageKind::Error, this.notice.clone())
                                        }
                                    };
                                    this.ws_push(kind, payload, cx);
                                })
                            })
                            .ok()
                            .flatten()
                            .is_none()
                        {
                            return;
                        }
                    }
                }
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.with_ws_request(&request_id, cx, |this, cx| {
                            this.error = Some(error.clone());
                            this.notice = format!("WebSocket 失败：{error}");
                            this.ws_push(MessageKind::Error, this.notice.clone(), cx);
                        });
                    });
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.with_ws_request(&request_id, cx, |this, cx| {
                    this.ws_finish_history("WS 已断开");
                    this.ws_history = None;
                    this.ws_connected = false;
                    this.ws_active = false;
                    this.ws_commands = None;
                    this.sending = None;
                    cx.notify();
                });
            });
        }));
        cx.notify();
    }
    pub(super) fn disconnect_websocket(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.shared_request_editor(self.active) {
            editor.update(cx, |editor, cx| editor.disconnect_websocket(cx));
            return;
        }
        if self.ws_commands.is_none() || !self.ws_connected {
            self.sending = None;
            self.ws_active = false;
            self.ws_commands = None;
            self.ws_connected = false;
            self.ws_finish_history("WS 已取消");
            self.ws_history = None;
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
        if let Some(editor) = self.shared_request_editor(self.active) {
            editor.update(cx, |editor, cx| editor.send_websocket_message(cx));
            return;
        }
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
            self.ws_push(MessageKind::Error, error, cx);
        }
    }
    pub(super) fn websocket_panel(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let code_view = !self.ws_raw && self.ws_output.read(cx).json_valid() == Some(true);
        let selected = self
            .ws_log
            .iter()
            .find(|event| Some(event.id) == self.ws_selected);
        let mut list = div()
            .id("websocket-event-list")
            .debug_selector(|| "websocket-event-list".into())
            .w(px(250.))
            .min_w(px(160.))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .track_scroll(&self.ws_scroll)
            .border_r_1()
            .border_color(rgb(p.border));
        for event in &self.ws_log {
            let id = event.id;
            let active = self.ws_selected == Some(id);
            let color = match event.kind {
                MessageKind::Received | MessageKind::Binary => p.accent,
                MessageKind::Error => 0xd9342b,
                _ => p.text_muted,
            };
            list = list.child(
                div()
                    .id(("websocket-event", id))
                    .debug_selector(move || format!("websocket-event-{id}"))
                    .px_3()
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .w_full()
                    .overflow_hidden()
                    .bg(rgb(if active { p.accent_tint } else { p.surface }))
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(p.accent_tint)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(event.time.clone())
                            .child(div().text_color(rgb(color)).child(event.kind.label()))
                            .child(div().flex_1())
                            .child(format!("{} B", event.byte_len())),
                    )
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_size(px(11.))
                            .child(event.title.as_ref().unwrap_or(&event.summary).clone()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ws_selected = Some(id);
                        this.ws_follow = false;
                        this.ws_select_sync(cx);
                        cx.notify();
                    })),
            );
        }
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .min_h(px(42.))
                    .py_1()
                    .flex_shrink_0()
                    .px_5()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .child("WebSocket 消息")
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(format!(
                                "{} · {} 条",
                                if self.ws_connected {
                                    "已连接"
                                } else if self.sending.is_some() {
                                    "连接中 / 断开中"
                                } else {
                                    "未连接"
                                },
                                self.ws_log.len()
                            )),
                    )
                    .child(div().flex_1())
                    .child(self.ws_menu(cx))
                    .child(
                        Button::new("独立窗口")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "websocket-open-window".into())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_request_window(this.active, window, cx)
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
            .child(if self.ws_log.is_empty() {
                div()
                    .flex_1()
                    .px_5()
                    .py_4()
                    .text_size(px(12.))
                    .text_color(rgb(p.text_muted))
                    .child("连接后，收发事件会显示在这里。请求体可用于编辑待发送消息。")
            } else {
                div().flex_1().min_h_0().min_w_0().flex().child(list).child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .h(px(38.))
                                .flex_shrink_0()
                                .px_3()
                                .flex()
                                .items_center()
                                .gap_2()
                                .border_b_1()
                                .border_color(rgb(p.border))
                                .child(self.ws_detail_tabs(cx))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(11.))
                                        .text_color(rgb(p.text_muted))
                                        .child(
                                            selected
                                                .map(|event| {
                                                    format!(
                                                        "{} · {} · {} B{}",
                                                        event.kind.label(),
                                                        event.time,
                                                        event.byte_len(),
                                                        if event.kind == MessageKind::Binary {
                                                            " · Base64"
                                                        } else {
                                                            ""
                                                        }
                                                    )
                                                })
                                                .unwrap_or_else(|| "选择事件查看消息体".into()),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .id("websocket-event-detail")
                                .debug_selector(|| "websocket-event-detail".into())
                                .flex_1()
                                .min_h_0()
                                .min_w_0()
                                .when(!code_view, |body| {
                                    body.overflow_y_scroll()
                                        .track_scroll(&self.ws_detail_scroll)
                                })
                                .when(code_view, |body| body.flex().flex_col().overflow_hidden())
                                .font_family("Menlo")
                                .child(
                                    div()
                                        .id("websocket-editor-area")
                                        .debug_selector(|| "websocket-editor-area".into())
                                        .w_full()
                                        .min_w_0()
                                        .when(code_view, |area| {
                                            area.flex_1().min_h_0().flex().flex_col()
                                        })
                                        .child(self.ws_output.clone()),
                                ),
                        ),
                )
            })
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
    #[gpui::test]
    fn event_detail_formats_json_preserves_raw_and_keeps_selection(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let raw = r#"{"event":"message","data":{"text":"你好","ids":[1,2]}}"#;
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&directory.path().join("events.sqlite3")).unwrap(),
                cx,
            );
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("WS", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.load(
                &Draft {
                    method: "WS".into(),
                    url: "ws://localhost/echo".into(),
                    ..Default::default()
                },
                cx,
            );
            host.ws_push(MessageKind::Received, raw.into(), cx);
            host.ws_push(MessageKind::Sent, "ping".into(), cx);
            host
        });
        let bounds = window.debug_bounds("websocket-event-1").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert!(!host.ws_follow);
            assert!(host.ws_output.read(cx).content().contains("\n  \"event\""));
            host.ws_push(MessageKind::Received, "later".into(), cx);
            assert_eq!(host.ws_selected, Some(1));
            assert!(host.ws_output.read(cx).content().contains("你好"));
        });
        let list = window.debug_bounds("websocket-event-list").unwrap();
        let detail = window.debug_bounds("websocket-event-detail").unwrap();
        assert!(list.right() <= detail.left());
        let editor = window.debug_bounds("websocket-editor-area").unwrap();
        assert_eq!(editor.left(), detail.left());
        assert_eq!(editor.top(), detail.top());
        assert_eq!(editor.right(), detail.right());
        let tabs = window.debug_bounds("websocket-detail-tab").unwrap();
        assert_eq!(tabs.left(), detail.left() + px(12.));
        assert!(tabs.bottom() <= detail.top());
        assert!(tabs.top() >= list.top());
        assert!(window.debug_bounds("websocket-copy-payload").is_none());
        assert!(window.debug_bounds("websocket-clear").is_none());

        let more = window.debug_bounds("websocket-content-trigger").unwrap();
        window.simulate_click(more.center(), gpui::Modifiers::default());
        let bounds = window.debug_bounds("websocket-copy-payload").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        assert_eq!(window.read_from_clipboard().unwrap().text().unwrap(), raw);
        let format = host.read_with(window, |host, _| host.ws_format_select.clone());
        format.update(window, |_, cx| cx.emit(SelectEvent::Changed("raw".into())));
        host.update(window, |host, cx| {
            assert_eq!(host.ws_output.read(cx).content(), raw)
        });
        let more = window.debug_bounds("websocket-content-trigger").unwrap();
        window.simulate_click(more.center(), gpui::Modifiers::default());
        let bounds = window.debug_bounds("websocket-clear").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert!(host.ws_log.is_empty());
            assert_eq!(host.ws_selected, None);
            assert_eq!(host.ws_sequence, 3);
            assert!(host.ws_output.read(cx).content().is_empty());
        });
    }
    #[gpui::test]
    fn plugin_titles_replace_previews_for_unselected_events_and_keep_raw_messages(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            Workbench::with_store(
                storage::Store::open(&dir.path().join("titles.sqlite3")).unwrap(),
                cx,
            )
        });
        host.update(window, |host, cx| {
            let raw = r#"{"encrypted_data":"cipher"}"#;
            host.ws_push(MessageKind::Received, raw.into(), cx);
            host.ws_push(MessageKind::Received, raw.into(), cx);
            let selected = host.ws_selected;
            let generation = host.ws_decode_generation;
            host.ws_finish_decode(
                generation,
                1,
                Ok(plugins::DecodedEvent {
                    data: "{}".into(),
                    title: Some("kso.app_chat · enter · 80545490".into()),
                }),
                cx,
            );
            assert_eq!(
                host.ws_log[0].title.as_deref(),
                Some("kso.app_chat · enter · 80545490")
            );
            assert_eq!(host.ws_log[0].payload, raw);
            assert_eq!(host.ws_selected, selected);
            host.ws_finish_decode(generation, 2, Ok("{}".to_string().into()), cx);
            assert!(host.ws_log[1].title.is_none());
            assert!(host.ws_log[1].summary.contains("cipher"));
            host.ws_invalidate_titles();
            assert!(
                host.ws_log
                    .iter()
                    .all(|event| event.title.is_none() && !event.title_decoded)
            );
            host.clear_websocket_messages(cx);
            host.ws_finish_decode(
                generation,
                1,
                Ok(plugins::DecodedEvent {
                    data: "{}".into(),
                    title: Some("late".into()),
                }),
                cx,
            );
            assert!(host.ws_log.is_empty());
        });
    }

    #[gpui::test]
    fn websocket_session_history_keeps_events_across_clear_restart_and_replay(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ws-history.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name.update(cx, |e, cx| e.set_value("WS", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.load(
                &Draft {
                    method: "WS".into(),
                    url: "ws://localhost/events".into(),
                    ..Default::default()
                },
                cx,
            );
            let draft = host.snapshot(cx);
            host.ws_begin_history(&draft, cx);
            host.ws_push(MessageKind::Status, "已连接".into(), cx);
            host.ws_push(MessageKind::Received, r#"{"event":"first"}"#.into(), cx);
            host.ws_push(MessageKind::Sent, "ping".into(), cx);
            host.clear_websocket_messages(cx);
            host.ws_push(MessageKind::Received, "after clear".into(), cx);
            host.ws_finish_history("WS 已断开");
            host.ws_history = None;
            let record = &host.requests[0].history[0];
            assert_eq!(record.exchange.as_ref().unwrap().websocket_events.len(), 4);
            assert_eq!(record.label, "WS 已断开");
            assert!(record.elapsed_ms.is_some());
            assert!(record.environment_name.is_some());
            host.persist(cx);
            host.show_activity = true;
            host.activity = 1;
            host
        });
        let record = window.debug_bounds("history-record-0").unwrap();
        window.simulate_click(record.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert_eq!(host.ws_log.len(), 4);
            assert!(!host.ws_connected && !host.ws_active);
            assert!(host.ws_history.is_none());
            assert!(host.ws_log[1].payload.contains("first"));
            assert_eq!(host.ws_output.read(cx).content(), "after clear");
            let draft = host.snapshot(cx);
            host.ws_begin_history(&draft, cx);
            host.ws_push(MessageKind::Error, "connection failed".into(), cx);
            host.ws_finish_history("WS 已断开");
            assert_eq!(host.requests[0].history[0].label, "WS 失败");
            assert_eq!(
                host.requests[0].history[1]
                    .exchange
                    .as_ref()
                    .unwrap()
                    .websocket_events
                    .len(),
                4
            );
            host.persist(cx);
            host.requests[0].history.remove(0);
            host.ws_push(MessageKind::Received, "ignored deleted record".into(), cx);
            assert_eq!(host.requests[0].history.len(), 1);
        });
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(window, |host, _| {
            assert_eq!(host.requests[0].history.len(), 2);
            let events = &host.requests[0].history[1]
                .exchange
                .as_ref()
                .unwrap()
                .websocket_events;
            assert_eq!(events.len(), 4);
            assert_eq!(events[2].kind, MessageKind::Sent);
            assert_eq!(events[2].payload, "ping");
        });
    }

    #[gpui::test]
    fn websocket_preferences_persist_and_history_selection_only_pauses_follow(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name.update(cx, |e, cx| e.set_value("WS", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.load(
                &Draft {
                    method: "WS".into(),
                    ..Default::default()
                },
                cx,
            );
            host.ws_push(MessageKind::Received, "first".into(), cx);
            host.ws_push(MessageKind::Received, "second".into(), cx);
            host.config = 4;
            host
        });
        let first = window.debug_bounds("websocket-event-1").unwrap();
        window.simulate_click(first.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert!(!host.ws_follow);
            assert!(host.snapshot(cx).websocket_follow_latest);
        });
        let scroll = window.debug_bounds("request-config-scroll").unwrap();
        window.simulate_event(gpui::ScrollWheelEvent {
            position: scroll.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-100.))),
            ..Default::default()
        });
        let follow = window.debug_bounds("websocket-follow-setting").unwrap();
        assert!(
            scroll.contains(&follow.center()),
            "follow {follow:?}, scroll {scroll:?}"
        );
        window.simulate_click(follow.center(), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert!(
                !host.ws_follow_latest,
                "follow click {follow:?}, scroll {scroll:?}"
            )
        });
        let format = window.debug_bounds("websocket-body-format").unwrap();
        window.simulate_click(format.center(), gpui::Modifiers::default());
        window.simulate_keystrokes("end");
        window.simulate_keystrokes("enter");
        host.update(window, |host, cx| {
            assert!(!host.ws_follow_latest);
            assert!(host.ws_raw);
            host.clear_websocket_messages(cx);
            assert!(!host.ws_follow);
            host.persist(cx);
            let saved = host.snapshot(cx);
            host.load(&Draft::default(), cx);
            assert!(host.ws_follow);
            assert!(!host.ws_raw);
            host.load(&saved, cx);
            assert!(!host.ws_follow);
            assert!(host.ws_raw);
        });
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(window, |host, cx| {
            assert!(!host.snapshot(cx).websocket_follow_latest);
            assert!(host.snapshot(cx).websocket_raw);
        });
        let mut old = serde_json::to_value(Draft::default()).unwrap();
        old.as_object_mut()
            .unwrap()
            .remove("websocket_follow_latest");
        old.as_object_mut().unwrap().remove("websocket_raw");
        let old: Draft = serde_json::from_value(old).unwrap();
        assert!(old.websocket_follow_latest);
        assert!(!old.websocket_raw);
    }

    #[gpui::test]
    fn full_payload_retention_and_eviction_keep_stable_event_ids(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            Workbench::with_store(
                storage::Store::open(&directory.path().join("retention.sqlite3")).unwrap(),
                cx,
            )
        });
        host.update(window, |host, cx| {
            let text = "文".repeat(30_000);
            host.ws_push(MessageKind::Received, text.clone(), cx);
            assert_eq!(host.ws_log[0].payload, text);
            assert_eq!(host.ws_output.read(cx).content(), text);
            host.ws_follow = false;
            for _ in 0..500 {
                host.ws_push(MessageKind::Status, "status".into(), cx);
            }
            assert_eq!(host.ws_log.len(), 500);
            assert_eq!(host.ws_log.front().unwrap().id, 2);
            assert!(
                host.ws_log
                    .iter()
                    .any(|event| Some(event.id) == host.ws_selected)
            );
            host.clear_websocket_messages(cx);
            for _ in 0..9 {
                host.ws_push(MessageKind::Received, "x".repeat(1024 * 1024), cx);
            }
            assert_eq!(host.ws_log.len(), 8);
            assert!(
                host.ws_log
                    .iter()
                    .map(|event| event.payload.len())
                    .sum::<usize>()
                    <= 8 * 1024 * 1024
            );
        });
    }
    #[gpui::test]
    fn decrypted_tab_preserves_original_ignores_stale_results_and_persists_plugin(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("decoder.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name
                .update(cx, |entry, cx| entry.set_value("WS", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.load(
                &Draft {
                    method: "WS".into(),
                    url: "ws://localhost/".into(),
                    ..Default::default()
                },
                cx,
            );
            host.ws_push(
                MessageKind::Received,
                r#"{"encrypted_data":"ciphertext"}"#.into(),
                cx,
            );
            host
        });
        // Bind through the same controlled Select event used by pointer/keyboard selection.
        let (select, value) = host.read_with(window, |host, _| {
            (
                host.ws_decoder_select.clone(),
                decoder_bindings(&host.plugin_catalog)
                    .into_iter()
                    .find(|(_, b, _)| b.id == "com.wps.event-decrypt")
                    .unwrap()
                    .0,
            )
        });
        select.update(window, |_, cx| cx.emit(SelectEvent::Changed(value.into())));
        host.update(window, |host, cx| {
            assert!(host.requests[0].draft.event_decoder.is_some());
            host.ws_detail_tab = 1;
            host.ws_decrypted = Some(Ok(r#"{"message":"decrypted"}"#.into()));
            host.ws_sync_detail(cx);
            assert!(host.ws_output.read(cx).content().contains("decrypted"));
            assert!(host.ws_log[0].payload.contains("ciphertext"));
            host.ws_decode_generation = 9;
            host.ws_detail_tab = 0;
            host.ws_sync_detail(cx);
            host.ws_finish_decode(
                8,
                host.ws_selected.unwrap(),
                Ok("stale".to_string().into()),
                cx,
            );
            assert!(host.ws_output.read(cx).content().contains("ciphertext"));
            assert!(!host.ws_output.read(cx).content().contains("stale"));
            host.ws_detail_tab = 1;
            host.ws_sync_detail(cx);
            host.persist(cx);
        });
        let more = window.debug_bounds("websocket-content-trigger").unwrap();
        window.simulate_click(more.center(), gpui::Modifiers::default());
        assert!(window.debug_bounds("websocket-event-decoder").is_none());
        let bounds = window.debug_bounds("websocket-copy-payload").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        assert_eq!(
            window.read_from_clipboard().unwrap().text().unwrap(),
            r#"{"message":"decrypted"}"#
        );
        // Decoder selection belongs to request settings and persists with the draft.
        let settings = window.debug_bounds("request-config-4").unwrap();
        window.simulate_click(settings.center(), gpui::Modifiers::default());
        assert!(window.debug_bounds("websocket-decoder-settings").is_none());
        let decoder = window.debug_bounds("websocket-event-decoder").unwrap();
        window.simulate_click(decoder.center(), gpui::Modifiers::default());
        window.simulate_click(
            gpui::point(decoder.left() + px(40.), decoder.top() + px(58.)),
            gpui::Modifiers::default(),
        );
        host.update(window, |host, cx| {
            assert!(host.snapshot(cx).event_decoder.is_none());
            assert!(host.config == 4);
        });
        let decoder = window.debug_bounds("websocket-event-decoder").unwrap();
        window.simulate_click(decoder.center(), gpui::Modifiers::default());
        window.simulate_keystrokes("end");
        window.simulate_keystrokes("enter");
        window.simulate_keystrokes("escape");
        host.update(window, |host, cx| {
            assert!(host.ws_menu.is_none());
            assert!(host.snapshot(cx).event_decoder.is_some());
            host.persist(cx);
        });
        let reopened =
            window.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        reopened.update(window, |host, cx| {
            assert_eq!(
                host.snapshot(cx).event_decoder.unwrap().id,
                "com.wps.event-decrypt"
            );
            assert!(host.ws_decrypted.is_none());
            assert!(host.ws_log.is_empty());
        });
    }
    #[gpui::test]
    fn both_event_tabs_fold_nested_json_without_changing_copy_or_raw(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let payload = r#"{"data":{"items":[{"id":1},{"id":2}]},"ok":true}"#;
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(
                storage::Store::open(&dir.path().join("fold.sqlite3")).unwrap(),
                cx,
            );
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name.update(cx, |e, cx| e.set_value("WS", cx));
            host.save_group(cx);
            host.add_request(cx);
            host.load(
                &Draft {
                    method: "WS".into(),
                    url: "ws://localhost/".into(),
                    ..Default::default()
                },
                cx,
            );
            host.ws_push(MessageKind::Received, payload.into(), cx);
            host
        });
        for tab in [0, 1] {
            host.update(window, |host, cx| {
                host.ws_detail_tab = tab;
                host.ws_raw = false;
                host.ws_decrypted = Some(Ok(payload.into()));
                host.ws_sync_detail(cx);
            });
            assert!(
                window.debug_bounds("json-fold-3").is_some(),
                "tab {tab}, detail {:?}",
                window.debug_bounds("websocket-event-detail")
            );
            let fold = window.debug_bounds("json-fold-2").unwrap();
            window.simulate_click(fold.center(), gpui::Modifiers::default());
            assert!(window.debug_bounds("json-fold-3").is_none());
            host.update(window, |host, cx| {
                let json: serde_json::Value =
                    serde_json::from_str(host.ws_output.read(cx).content()).unwrap();
                assert_eq!(json["data"]["items"].as_array().unwrap().len(), 2);
            });
            let more = window.debug_bounds("websocket-content-trigger").unwrap();
            window.simulate_click(more.center(), gpui::Modifiers::default());
            let copy = window.debug_bounds("websocket-copy-payload").unwrap();
            window.simulate_click(copy.center(), gpui::Modifiers::default());
            assert_eq!(
                window.read_from_clipboard().unwrap().text().unwrap(),
                payload
            );
            let fold = window.debug_bounds("json-fold-2").unwrap();
            window.simulate_click(fold.center(), gpui::Modifiers::default());
            assert!(window.debug_bounds("json-fold-3").is_some());
            let format = host.read_with(window, |host, _| host.ws_format_select.clone());
            format.update(window, |_, cx| cx.emit(SelectEvent::Changed("raw".into())));
            assert!(window.debug_bounds("json-fold-2").is_none());
            host.update(window, |host, cx| {
                assert_eq!(host.ws_output.read(cx).content(), payload)
            });
        }
    }
}
