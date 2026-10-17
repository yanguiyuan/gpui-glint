use super::*;

impl Workbench {
    pub(super) fn open_mcp_history(&mut self, cx: &mut Context<Self>) {
        self.show_mcp = true;
        self.show_plugins = false;
        self.show_environments = false;
        self.show_collection_menu = false;
        self.tree_menu = None;
        self.directory_settings = None;
        self.reload_mcp_history(cx);
    }
    pub(super) fn reload_mcp_history(&mut self, cx: &mut Context<Self>) {
        let success = match self.mcp_history_filter {
            1 => Some(true),
            2 => Some(false),
            _ => None,
        };
        match self.storage.mcp_call_page(
            self.mcp_history_search.read(cx).value(),
            success,
            self.mcp_history_before,
            100,
        ) {
            Ok(page) => {
                self.mcp_history_rows = page.rows;
                self.mcp_history_total = page.total;
                self.mcp_history_has_more = page.has_more;
                if !self.mcp_history_selected.as_ref().is_some_and(|selected| {
                    self.mcp_history_rows
                        .iter()
                        .any(|row| row.id == selected.summary.id)
                }) {
                    if let Some(row) = self.mcp_history_rows.first() {
                        self.select_mcp_history(row.id, cx);
                    } else {
                        self.mcp_history_selected = None;
                        self.sync_mcp_history_detail(cx);
                    }
                }
            }
            Err(error) => self.mcp_history_error = Some(error),
        }
        cx.notify();
    }
    fn select_mcp_history(&mut self, id: i64, cx: &mut Context<Self>) {
        match self.storage.mcp_call_record(id) {
            Ok(record) => {
                self.mcp_history_selected = Some(record);
                self.sync_mcp_history_detail(cx);
            }
            Err(error) => self.mcp_history_error = Some(error),
        }
        cx.notify();
    }
    fn sync_mcp_history_detail(&mut self, cx: &mut Context<Self>) {
        let content = self
            .mcp_history_selected
            .as_ref()
            .map(|record| {
                serde_json::to_string_pretty(if self.mcp_history_tab == 0 {
                    &record.arguments
                } else {
                    &record.result
                })
                .unwrap_or_default()
            })
            .unwrap_or_default();
        self.mcp_history_detail
            .update(cx, |editor, cx| editor.set_content(content, cx));
    }
    pub(super) fn mcp_history_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let mut list = div()
            .id("mcp-history-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_2()
            .pb_3();
        let mut day = String::new();
        for row in &self.mcp_history_rows {
            let date = row.date();
            if day != date {
                day = date.clone();
                let count = self
                    .mcp_history_rows
                    .iter()
                    .filter(|row| row.date() == date)
                    .count();
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .pt_4()
                        .pb_2()
                        .debug_selector(move || format!("mcp-history-day-{date}").into())
                        .text_size(px(10.))
                        .text_color(rgb(p.text_muted))
                        .font_weight(FontWeight::MEDIUM)
                        .child(day.clone())
                        .child(div().flex_1())
                        .child(format!("{count} 次")),
                );
            }
            let selected = self
                .mcp_history_selected
                .as_ref()
                .is_some_and(|record| record.summary.id == row.id);
            let id = row.id;
            let color = self
                .theme
                .color(if row.success { 0x24864c } else { 0xd9342b });
            list = list.child(
                div()
                    .id(("mcp-history-call", id as u64))
                    .debug_selector(move || format!("mcp-history-call-{id}").into())
                    .px_2()
                    .py_2()
                    .mb_1()
                    .rounded(px(7.))
                    .cursor_pointer()
                    .when(selected, |s| s.bg(rgb(p.accent_tint)))
                    .hover(|s| s.bg(rgb(p.panel)))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_color(rgb(color)).text_size(px(10.)).child("●"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family("Menlo")
                                    .text_size(px(11.))
                                    .child(
                                        row.tool
                                            .strip_prefix("glint_")
                                            .unwrap_or(&row.tool)
                                            .to_owned(),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .pl(px(15.))
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(row.time())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(row.client.clone()),
                            )
                            .child(format!("{} ms", row.duration_ms)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select_mcp_history(id, cx))),
            );
        }
        if self.mcp_history_rows.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_4()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(
                        if self.mcp_history_search.read(cx).value().is_empty()
                            && self.mcp_history_filter == 0
                        {
                            "暂无调用记录"
                        } else {
                            "没有匹配的调用"
                        },
                    ),
            );
        }
        let owner = cx.entity().downgrade();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_3()
                    .pt_4()
                    .pb_3()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("MCP 调用")
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.text_muted))
                                    .child(format!("{} 次", self.mcp_history_total)),
                            ),
                    )
                    .child(self.mcp_history_search.clone())
                    .child(
                        SegmentedControl::new([
                            SegmentOption::new("全部"),
                            SegmentOption::new("成功"),
                            SegmentOption::new("失败"),
                        ])
                        .id("mcp-history-outcome")
                        .selected(self.mcp_history_filter)
                        .theme(self.theme)
                        .on_change(move |index, _, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                this.mcp_history_filter = index;
                                this.mcp_history_before = None;
                                this.mcp_history_pages.clear();
                                this.reload_mcp_history(cx);
                            });
                        })
                        .render(),
                    ),
            )
            .child(list)
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .p_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("上一页")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .when(self.mcp_history_pages.is_empty(), |b| b.disabled())
                            .render()
                            .debug_selector(|| "mcp-history-previous".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(before) = this.mcp_history_pages.pop() {
                                    this.mcp_history_before = before;
                                    this.reload_mcp_history(cx);
                                }
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .text_center()
                            .child(format!("第 {} 页", self.mcp_history_pages.len() + 1)),
                    )
                    .child(
                        Button::new("下一页")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .when(!self.mcp_history_has_more, |b| b.disabled())
                            .render()
                            .debug_selector(|| "mcp-history-next".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.mcp_history_has_more {
                                    let before = this.mcp_history_rows.last().map(|row| row.id);
                                    this.mcp_history_pages.push(this.mcp_history_before);
                                    this.mcp_history_before = before;
                                    this.reload_mcp_history(cx);
                                }
                            })),
                    ),
            )
    }
    pub(super) fn mcp_history_workspace(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let connected = self.mcp_server.is_some();
        let mut body = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(p.surface))
            .debug_selector(|| "mcp-history-workspace".into())
            .child(
                div()
                    .flex_shrink_0()
                    .px_5()
                    .py_4()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("MCP"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.text_muted))
                                    .child("按日期查看 agent 的工具调用"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(if connected {
                                "● 已就绪"
                            } else {
                                "○ 未启动"
                            }),
                    )
                    .child(
                        Button::new("刷新")
                            .icon(Icon::Refresh)
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.reload_mcp_history(cx))),
                    )
                    .child(
                        Button::new("复制连接配置")
                            .icon(Icon::Copy)
                            .kind(ButtonKind::Tinted)
                            .size(ButtonSize::Regular)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "mcp-history-copy-config".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                match mcp::configuration() {
                                    Ok(config) => {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            serde_json::to_string_pretty(&config).unwrap(),
                                        ));
                                        this.notice = "已复制 MCP 连接配置".into();
                                    }
                                    Err(error) => this.notice = error,
                                };
                                cx.notify();
                            })),
                    ),
            )
            .when_some(
                self.mcp_error
                    .as_ref()
                    .or(self.mcp_history_error.as_ref())
                    .cloned(),
                |s, error| {
                    s.child(
                        div()
                            .px_5()
                            .py_2()
                            .text_size(px(11.))
                            .text_color(rgb(self.theme.color(0xd9342b)))
                            .child(format!("MCP：{error}")),
                    )
                },
            );
        if let Some(record) = &self.mcp_history_selected {
            let summary = &record.summary;
            body = body
                .child(
                    div()
                        .px_5()
                        .py_3()
                        .flex_shrink_0()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .border_b_1()
                        .border_color(rgb(p.border))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .font_family("Menlo")
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(summary.tool.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(self.theme.color(if summary.success {
                                            0x24864c
                                        } else {
                                            0xd9342b
                                        })))
                                        .child(if summary.success { "成功" } else { "失败" }),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_3()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child(summary.timestamp())
                                .child(summary.client.clone())
                                .child(format!("{} ms", summary.duration_ms))
                                .child(format!("请求 {}", summary.rpc_id)),
                        ),
                )
                .child(
                    div()
                        .h(px(40.))
                        .flex_shrink_0()
                        .px_5()
                        .flex()
                        .items_center()
                        .gap_5()
                        .child(
                            tab_button("参数", self.mcp_history_tab == 0, self.theme)
                                .debug_selector(|| "mcp-history-arguments-tab".into())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.mcp_history_tab = 0;
                                    this.sync_mcp_history_detail(cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            tab_button("结果", self.mcp_history_tab == 1, self.theme)
                                .debug_selector(|| "mcp-history-result-tab".into())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.mcp_history_tab = 1;
                                    this.sync_mcp_history_detail(cx);
                                    cx.notify();
                                })),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.text_muted))
                                .child("密钥字段已隐藏"),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .px_5()
                        .pb_4()
                        .child(self.mcp_history_detail.clone()),
                );
        } else {
            body = body.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .text_color(rgb(p.text_muted))
                    .child(Icon::Workflow.render(30.))
                    .child(div().text_size(px(14.)).text_color(rgb(p.text)).child(
                        if self.mcp_history_total == 0 {
                            "还没有 MCP 调用"
                        } else {
                            "选择一条调用查看详情"
                        },
                    ))
                    .child(
                        div()
                            .text_size(px(11.))
                            .child("连接 agent 后，工具调用会自动记录在这里"),
                    ),
            );
        }
        body
    }
}

impl Workbench {
    pub(super) fn execute_mcp_call(&mut self, call: mcp::Call, cx: &mut Context<Self>) {
        let mut secrets = self.mcp_log_secrets();
        let arguments = call.arguments.clone();
        let result = if Instant::now() >= call.deadline {
            Err("MCP 操作已过期，未执行".into())
        } else if let Some(error) = call.rejection {
            Err(error)
        } else {
            self.mcp_call(&call.name, call.arguments, cx)
        };
        secrets.extend(self.mcp_log_secrets());
        let mut arguments = arguments;
        if call.name == "glint_environment_variable_set" {
            if let Some(fields) = arguments.as_object_mut() {
                fields.insert("value".into(), serde_json::json!("[REDACTED]"));
            }
        }
        let mut output = match &result {
            Ok(value) => serde_json::json!({"result":value}),
            Err(error) => serde_json::json!({"error":error}),
        };
        super::mcp_ui::redact(&mut arguments);
        super::mcp_ui::redact(&mut output);
        scrub_values(&mut arguments, &secrets);
        scrub_values(&mut output, &secrets);
        let record = mcp_history::Record {
            summary: mcp_history::Summary {
                id: 0,
                started_at_ms: call.metadata.started_at_ms,
                client: call.metadata.client,
                rpc_id: call.metadata.rpc_id,
                tool: mcp_history::clip(&call.name, 256),
                duration_ms: call
                    .metadata
                    .started
                    .elapsed()
                    .as_millis()
                    .min(u64::MAX as u128) as u64,
                success: result.is_ok(),
            },
            arguments: mcp_history::bounded(arguments),
            result: mcp_history::bounded(output),
        };
        match self.storage.append_mcp_call(&record) {
            Ok(_) => {
                self.mcp_history_error = None;
                if self.show_mcp {
                    self.reload_mcp_history(cx);
                }
            }
            Err(error) => {
                self.mcp_history_error = Some(format!("调用记录保存失败：{error}"));
                cx.notify();
            }
        }
        let _ = call.reply.send(result);
    }
    fn mcp_log_secrets(&self) -> Vec<String> {
        let mut values = self
            .environment_inputs
            .iter()
            .chain(
                self.collections
                    .iter()
                    .flat_map(|c| c.environments.values()),
            )
            .flat_map(|env| {
                env.variables
                    .iter()
                    .filter(|v| v.secret)
                    .map(|v| v.value.clone())
            })
            .collect::<Vec<_>>();
        for request in &self.requests {
            values.push(request.draft.token.clone());
            values.extend(
                request
                    .draft
                    .cookies
                    .iter()
                    .map(|cookie| cookie.value.clone()),
            );
            values.extend(
                request
                    .draft
                    .headers
                    .iter()
                    .filter(|h| {
                        matches!(
                            h.key.to_ascii_lowercase().as_str(),
                            "authorization"
                                | "cookie"
                                | "x-kso-authorization"
                                | "proxy-authorization"
                        )
                    })
                    .map(|h| h.value.clone()),
            );
        }
        values.retain(|value| value.len() >= 4);
        values.sort_by_key(|value| std::cmp::Reverse(value.len()));
        values.dedup();
        values
    }
}
fn scrub_values(value: &mut serde_json::Value, secrets: &[String]) {
    match value {
        serde_json::Value::String(text) => {
            for secret in secrets {
                if text.contains(secret) {
                    *text = text.replace(secret, "[REDACTED]");
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                scrub_values(item, secrets);
            }
        }
        serde_json::Value::Object(fields) => {
            for item in fields.values_mut() {
                scrub_values(item, secrets);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    fn dispatch(
        host: &mut Workbench,
        tool: &str,
        arguments: Value,
        client: &str,
        cx: &mut Context<Workbench>,
    ) -> Result<Value, String> {
        let (reply, receiver) = std::sync::mpsc::channel();
        host.execute_mcp_call(
            mcp::Call {
                name: tool.into(),
                arguments,
                deadline: Instant::now() + std::time::Duration::from_secs(30),
                reply,
                metadata: mcp::CallMetadata {
                    client: client.into(),
                    rpc_id: "42".into(),
                    started_at_ms: chrono::Utc::now().timestamp_millis(),
                    started: Instant::now(),
                },
                rejection: None,
            },
            cx,
        );
        receiver.recv().unwrap()
    }
    #[gpui::test]
    fn mcp_calls_record_success_failure_and_redact_before_persisting(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let host = cx.new(|cx| Workbench::with_store(storage::Store::open(&path).unwrap(), cx));
        host.update(cx,|host,cx|{
            dispatch(host,"glint_environment_create",json!({"name":"Dev"}),"Agent",cx).unwrap();
            dispatch(host,"glint_environment_variable_set",json!({"environment":"Dev","name":"api_key","value":"long-secret-value","secret":true}),"Agent",cx).unwrap();
            let revealed=dispatch(host,"glint_environment_get",json!({"environment":"Dev","include_secrets":true}),"Agent",cx).unwrap();
            assert_eq!(revealed["variables"][0]["value"],"long-secret-value");
            assert!(dispatch(host,"glint_request_get",json!({"request_id":"missing"}),"Agent",cx).is_err());
            let page=host.storage.mcp_call_page("",None,None,100).unwrap();assert_eq!(page.total,4);
            assert!(!page.rows[0].success);assert_eq!(page.rows[0].client,"Agent");
            for row in page.rows{
                let record=host.storage.mcp_call_record(row.id).unwrap();
                assert!(!record.arguments.to_string().contains("long-secret-value"));
                assert!(!record.result.to_string().contains("long-secret-value"));
            }
        });
        assert_eq!(
            storage::Store::open(&path)
                .unwrap()
                .mcp_call_page("", None, None, 100)
                .unwrap()
                .total,
            4
        );
    }
    #[gpui::test]
    fn rail_opens_grouped_history_details_and_returns_to_requests(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            for (stamp, tool, success) in [
                ("2026-10-16T12:00:00Z", "glint_request_get", true),
                ("2026-10-17T12:00:00Z", "glint_request_send", false),
            ] {
                let time = chrono::DateTime::parse_from_rfc3339(stamp)
                    .unwrap()
                    .timestamp_millis();
                host.storage
                    .append_mcp_call(&mcp_history::Record {
                        summary: mcp_history::Summary {
                            id: 0,
                            started_at_ms: time,
                            client: "Agent".into(),
                            rpc_id: "1".into(),
                            tool: tool.into(),
                            duration_ms: 3,
                            success,
                        },
                        arguments: json!({"query":"hello"}),
                        result: json!({"error":"test failure"}),
                    })
                    .unwrap();
            }
            host
        });
        let bounds = window.debug_bounds("open-mcp-history").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        assert!(window.debug_bounds("mcp-history-workspace").is_some());
        assert!(window.debug_bounds("mcp-history-day-2026-10-17").is_some());
        assert!(window.debug_bounds("mcp-history-day-2026-10-16").is_some());
        host.update(window, |host, cx| {
            assert!(host.show_mcp);
            assert_eq!(host.mcp_history_rows.len(), 2);
            assert!(host.mcp_history_detail.read(cx).content().contains("hello"));
        });
        let bounds = window.debug_bounds("mcp-history-result-tab").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert!(
                host.mcp_history_detail
                    .read(cx)
                    .content()
                    .contains("test failure")
            )
        });
        host.update(window, |host, cx| {
            host.mcp_history_search
                .update(cx, |input, cx| input.set_value("request_get", cx))
        });
        window.run_until_parked();
        host.update(window, |host, _| {
            assert_eq!(host.mcp_history_rows.len(), 1);
            assert_eq!(host.mcp_history_rows[0].tool, "glint_request_get");
        });
        let bounds = window.debug_bounds("mcp-history-copy-config").unwrap();
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        let config: Value =
            serde_json::from_str(&window.read_from_clipboard().unwrap().text().unwrap()).unwrap();
        assert_eq!(config["mcpServers"]["glint-api"]["args"][0], "--mcp");
        host.update(window, |host, cx| {
            host.open_plugin_manager(cx);
            assert!(!host.show_mcp);
            host.open_mcp_history(cx);
            assert!(host.show_mcp);
            host.mcp_call("glint_ui_set", json!({"section":"requests"}), cx)
                .unwrap();
            assert!(!host.show_mcp);
        });
    }
}
