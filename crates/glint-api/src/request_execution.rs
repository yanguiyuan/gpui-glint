//! HTTP request execution, cancellation, streaming, and trace recording.

use super::*;

impl Workbench {
    pub(super) fn send(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.shared_request_editor(self.active) {
            editor.update(cx, |editor, cx| editor.send(cx));
            return;
        }
        if self.show_curl_import {
            return;
        }
        if self.directory_settings.is_some() {
            return;
        }
        if self.requests.is_empty() {
            return;
        }
        if self.ws_active {
            self.disconnect_websocket(cx);
            return;
        }
        if self.sending.take().is_some() {
            self.push_trace("已取消请求", cx);
            self.trace_started = None;
            self.notice = "已取消请求".into();
            cx.notify();
            return;
        }
        if self.oauth_before_send(cx) {
            return;
        }
        if self.snapshot(cx).method == "WS" {
            self.connect_websocket(cx);
            return;
        }
        self.sent_curls[self.active] = None;
        let draft = self.snapshot(cx);
        let extraction_collection = self
            .collections
            .iter()
            .find(|c| c.collection == self.requests[self.active].collection)
            .map(|c| c.id.clone())
            .unwrap_or_default();
        let environment_index = self.selected_environment(cx);
        let environment_name = self.environment_name(environment_index).to_owned();
        self.trace_started = Some(Instant::now());
        self.traces[self.active].clear();
        self.push_trace(
            format!("准备 {} 请求 · {}", draft.method, environment_name),
            cx,
        );
        self.cancel_json_pick(cx);
        self.json_menu = None;
        self.json_nodes.clear();
        self.response_json = None;
        self.response_data = None;
        self.status = None;
        self.error = None;
        let proxy = self.effective_proxy(environment_index);
        let variables = self.active_environment_values(cx);
        let request = match PreparedRequest::new(&draft, &variables)
            .and_then(|request| self.oauth_attach(request, &draft, cx))
            .and_then(|request| request.with_proxy(&proxy))
            .and_then(|request| {
                plugins::validate_variables(
                    &self.enabled_before_actions(&draft.plugins),
                    &self.plugin_catalog,
                    &variables,
                )?;
                Ok(request)
            }) {
            Ok(request) => request,
            Err(error) => {
                self.push_trace(format!("校验失败：{error}"), cx);
                self.trace_started = None;
                self.request_error_toast("无法发送请求", &environment_name, &error, cx);
                self.error = Some(error);
                self.notice = "请求配置有误".into();
                cx.notify();
                return;
            }
        };
        self.sent_requests[self.active] = Some(request.preview.clone());
        self.sync_request_preview(cx);
        self.push_trace("URL、环境变量与请求头校验通过", cx);
        if draft
            .cookies
            .iter()
            .any(|p| p.enabled && !p.key.trim().is_empty())
        {
            self.push_trace("已合并 Cookie 覆盖表格到请求头", cx);
        }
        self.push_trace(
            if draft.plugins.iter().any(|action| action.enabled) {
                "执行发送前动作"
            } else {
                "准备发送 HTTP 请求"
            },
            cx,
        );
        self.requests[self.active].draft = draft.clone();
        self.notice = "正在发送 HTTP 请求".into();
        let index = self.active;
        let request_id = self.requests[index].id.clone();
        let started = Instant::now();
        let plugin_ids = self.enabled_before_actions(&draft.plugins);
        let plugin_catalog = self.plugin_catalog.clone();
        let plugin_variables = variables.clone();
        self.sending = Some(cx.spawn(async move |this, cx| {
            let prepared = cx
                .background_executor()
                .spawn(async move {
                    plugins::apply(request, &plugin_ids, &plugin_catalog, &plugin_variables)
                })
                .await;
            let failure_title = if prepared.is_ok() {
                "请求失败"
            } else {
                "无法发送请求"
            };
            let result = match prepared {
                Ok(request) => {
                    let _ = this.update(cx, |this, cx| {
                        this.with_request_executor(&request_id, cx, |this, cx| {
                            let index = this.active;
                            this.push_trace("发送 HTTP 请求，等待服务器响应", cx);
                            this.sent_requests[index] = Some(request.preview.clone());
                            this.sent_curls[index] = request.to_curl().ok();
                            if this.active == index {
                                this.sync_request_preview(cx);
                            }
                            cx.notify();
                        });
                    });
                    let (progress, updates) = async_channel::bounded(1);
                    let execution = cx
                        .background_executor()
                        .spawn(http::execute_stream(request, Some(progress)));
                    while let Ok(response) = updates.recv().await {
                        if this
                            .update(cx, |this, cx| {
                                this.with_request_executor(&request_id, cx, |this, cx| {
                                    this.apply_stream_response(this.active, response, cx)
                                })
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                    execution.await
                }
                Err(error) => Err(error),
            };
            let elapsed_ms = started.elapsed().as_millis();
            let _ = this.update(cx, |this, cx| {
                this.with_request_executor(&request_id, cx, |this, cx| {
                    let index = this.active;
                    let (label, detail) = match &result {
                        Ok(response) => (
                            status_label(response.status),
                            format!(
                                "{elapsed_ms} ms · {} · {} B",
                                environment_name, response.bytes
                            ),
                        ),
                        Err(error) => ("请求失败".into(), format!("{elapsed_ms} ms · {error}")),
                    };
                    if let Ok(response) = &result {
                        this.push_trace(
                            format!(
                                "收到响应 · {} · {} B",
                                status_label(response.status),
                                response.bytes
                            ),
                            cx,
                        );
                    }
                    let extraction = result
                        .as_ref()
                        .ok()
                        .filter(|response| !response.is_event_stream())
                        .map(|response| {
                            this.apply_extractors_for_id(
                                &draft,
                                environment_index,
                                &extraction_collection,
                                response,
                                cx,
                            )
                        });
                    this.storage_dirty = true;
                    this.prepend_history(
                        index,
                        Record {
                            id: crate::model::new_id(),
                            tag: String::new(),
                            source: Default::default(),
                            environment_name: Some(environment_name),
                            draft,
                            label,
                            detail,
                            time: record_time(),
                            elapsed_ms: Some(elapsed_ms),
                            exchange: Some(model::Exchange {
                                websocket_connected: false,
                                websocket_events: vec![],
                                curl: this.sent_curls[index].clone(),
                                response: result.as_ref().ok().cloned().or_else(|| {
                                    this.response_data
                                        .clone()
                                        .filter(HttpResponse::is_event_stream)
                                }),
                                error: result.as_ref().err().cloned(),
                                preview: this.sent_requests[index].clone(),
                                traces: vec![],
                            }),
                        },
                    );
                    match result {
                        Ok(response) => {
                            this.push_trace(
                                if response.status < 400 {
                                    "请求完成"
                                } else {
                                    "请求完成 · HTTP 错误状态"
                                },
                                cx,
                            );
                            this.responses[index] = Some(response.clone());
                            this.status = Some(response.status);
                            this.response_json = response.json.clone();
                            this.response_data = Some(response);
                            this.update_response(cx);
                            this.notice = match extraction {
                                Some(Ok(count)) if count > 0 => {
                                    format!("请求完成 · 已提取 {count} 个环境变量")
                                }
                                Some(Err(error)) => format!("请求完成 · 后置动作失败：{error}"),
                                _ => "请求完成".into(),
                            };
                        }
                        Err(error) => {
                            let environment = this.requests[index].history[0]
                                .environment_name
                                .clone()
                                .unwrap_or_default();
                            this.request_error_toast(failure_title, &environment, &error, cx);
                            this.push_trace(format!("请求失败：{error}"), cx);
                            this.error = Some(error);
                            if !this
                                .response_data
                                .as_ref()
                                .is_some_and(HttpResponse::is_event_stream)
                            {
                                this.responses[index] = None;
                                this.response_data = None;
                                this.response_json = None;
                                this.status = None;
                            }
                            this.update_response(cx);
                            this.notice = "请求失败".into();
                        }
                    }
                    this.requests[index].history[0]
                        .exchange
                        .as_mut()
                        .unwrap()
                        .traces = this.traces[index].clone();
                    this.trace_started = None;
                    this.sending = None;
                    cx.notify();
                });
            });
        }));
        cx.notify();
    }

    pub(super) fn request_error_toast(
        &mut self,
        title: &str,
        environment: &str,
        error: &str,
        cx: &mut Context<Self>,
    ) {
        self.toasts.update(cx, |host, cx| {
            host.push(
                Toast::new(title.to_owned())
                    .kind(ToastKind::Error)
                    .description(format!("环境：{environment}\n{error}"))
                    .duration(Some(std::time::Duration::from_secs(10))),
                cx,
            );
        });
    }

    pub(super) fn push_trace(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.storage_dirty = true;
        self.traces[self.active].push(TraceEntry {
            elapsed_ms: self
                .trace_started
                .map(|start| start.elapsed().as_millis())
                .unwrap_or(0),
            message: message.into(),
        });
        cx.notify();
    }

    pub(super) fn apply_stream_response(
        &mut self,
        index: usize,
        response: HttpResponse,
        cx: &mut Context<Self>,
    ) {
        let first = !self
            .response_data
            .as_ref()
            .is_some_and(HttpResponse::is_event_stream);
        self.responses[index] = Some(response.clone());
        self.status = Some(response.status);
        self.response_json = None;
        self.response_data = Some(response);
        if first {
            self.push_trace("收到 SSE 响应，开始持续接收事件", cx);
            self.update_response(cx);
        } else {
            let content = self.response_content();
            self.response
                .update(cx, |editor, cx| editor.set_content(content, cx));
            self.sync_response_search(true, cx);
        }
        self.notice = "SSE 接收中 · 点击取消可断开 · 显示最近 1 MiB".into();
        cx.notify();
    }
}
