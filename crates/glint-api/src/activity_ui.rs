//! Request version and execution history panels.

use super::*;

impl Workbench {
    pub(super) fn activity_panel(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let Some(request) = self.requests.get(self.active) else {
            return div();
        };
        let records = if self.activity == 0 {
            &request.versions
        } else {
            &request.history
        };
        let query = self.history_search.read(cx).value().to_lowercase();
        let mut list = div()
            .id("activity-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_3();
        let mut count = 0;
        let mut previous_date = "";
        let visible_history: Vec<_> = records
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r.draft.url.to_lowercase().contains(&query)
                    && (self.history_filter != 1 || r.is_success())
                    && (self.history_filter != 2 || !r.is_success())
            })
            .collect();
        if self.activity == 1 {
            list = list.child(
                div()
                    .pb_3()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("按日期分组 · 新到旧"),
            );
        }
        for (i, r) in records.iter().enumerate() {
            if self.activity == 1
                && (!r.draft.url.to_lowercase().contains(&query)
                    || (self.history_filter == 1 && !r.is_success())
                    || (self.history_filter == 2 && r.is_success()))
            {
                continue;
            }
            count += 1;
            let record = r.clone();
            if self.activity == 1 {
                let date = r.time.split(' ').next().unwrap_or(&r.time);
                if previous_date != date {
                    previous_date = date;
                    let day_count = visible_history
                        .iter()
                        .filter(|(_, r)| r.time.starts_with(date))
                        .count();
                    list = list.child(
                        div()
                            .flex()
                            .items_center()
                            .pt_3()
                            .pb_2()
                            .border_t_1()
                            .border_color(rgb(p.border))
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(date.to_owned())
                            .child(div().flex_1())
                            .child(day_count.to_string()),
                    );
                }
                let status = r
                    .label
                    .split(' ')
                    .next()
                    .and_then(|s| s.parse::<u16>().ok())
                    .unwrap_or(0);
                let color = self
                    .theme
                    .color(if r.is_success() || (200..400).contains(&status) {
                        0x24864c
                    } else if (400..500).contains(&status) {
                        0xb87508
                    } else {
                        0xd9342b
                    });
                let selected = self.selected_history == Some((self.active, i));
                list = list.child(
                    div()
                        .id(("history-record", i))
                        .group(gpui::SharedString::from(format!("history-record-{i}")))
                        .debug_selector(move || format!("history-record-{i}").into())
                        .rounded_md()
                        .p_3()
                        .mb_2()
                        .cursor_pointer()
                        .when(selected, |s| s.bg(rgb(p.accent_tint)))
                        .hover(|s| s.bg(rgb(p.accent_tint)))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .px_2()
                                        .py_1()
                                        .rounded_md()
                                        .bg(gpui::rgba((color << 8) | 0x18))
                                        .text_color(rgb(color))
                                        .text_size(px(10.))
                                        .font_family("Menlo")
                                        .child(div().size(px(5.)).rounded_full().bg(rgb(color)))
                                        .child(r.label.clone()),
                                )
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .text_size(px(9.))
                                        .text_color(rgb(p.text_muted))
                                        .child(r.time.split(' ').nth(1).unwrap_or("").to_owned()),
                                )
                                .child(self.history_delete_popover(i, r, cx)),
                        )
                        .child(
                            div()
                                .mt_2()
                                .flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .font_family("Menlo")
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_size(px(10.))
                                        .text_color(rgb(self
                                            .theme
                                            .color(method_color(&r.draft.method))))
                                        .child(r.draft.method.clone()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .font_family("Menlo")
                                        .text_size(px(10.))
                                        .child(r.draft.url.clone()),
                                ),
                        )
                        .child(
                            div()
                                .mt_2()
                                .text_size(px(9.))
                                .text_color(rgb(p.text_muted))
                                .child(format!(
                                    "{} ms · {}",
                                    r.elapsed_ms.unwrap_or(0),
                                    r.environment_name.as_deref().unwrap_or("未记录环境")
                                )),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.sending.take().is_some() {
                                this.push_trace("恢复历史配置，已取消请求", cx);
                            }
                            this.trace_started = None;
                            this.selected_history = Some((this.active, i));
                            this.load(&record.draft, cx);
                            let exchange = record.exchange.as_ref();
                            this.sent_curls[this.active] = exchange.and_then(|e| e.curl.clone());
                            this.response_data = exchange.and_then(|e| e.response.clone());
                            this.response_json =
                                this.response_data.as_ref().and_then(|r| r.json.clone());
                            this.status = this.response_data.as_ref().map(|r| r.status);
                            this.error = exchange.and_then(|e| e.error.clone());
                            this.sent_requests[this.active] =
                                exchange.and_then(|e| e.preview.clone());
                            this.traces[this.active] =
                                exchange.map(|e| e.traces.clone()).unwrap_or_default();
                            this.responses[this.active] = this.response_data.clone();
                            this.sync_request_preview(cx);
                            this.update_response(cx);
                            if record.draft.method == "WS" {
                                this.restore_ws_history(
                                    exchange
                                        .map(|e| e.websocket_events.as_slice())
                                        .unwrap_or_default(),
                                    cx,
                                );
                            }
                            this.notice = if exchange.is_some() {
                                format!("已恢复 {} 的请求与响应", record.label)
                            } else {
                                "已恢复历史请求配置，此旧记录未保存响应内容".into()
                            };
                            cx.notify();
                        })),
                );
                continue;
            }
            let current = self.current_versions[self.active] == i;
            let params = r
                .draft
                .params
                .iter()
                .filter(|pair| pair.enabled && !pair.key.trim().is_empty())
                .map(|pair| format!("{}={}", pair.key, pair.value))
                .collect::<Vec<_>>()
                .join(" · ");
            let time = if r.time.len() >= 16 && r.time.starts_with("20") {
                r.time[5..16].to_owned()
            } else {
                r.time.clone()
            };
            list = list.child(
                div()
                    .id(("version-record", i))
                    .group(gpui::SharedString::from(format!("version-record-{i}")))
                    .debug_selector(move || format!("version-record-{i}").into())
                    .h(px(92.))
                    .flex_shrink_0()
                    .flex()
                    .gap_2()
                    .px_2()
                    .rounded_lg()
                    .cursor_pointer()
                    .when(current, |s| s.bg(rgb(p.accent_tint)))
                    .hover(|s| s.bg(rgb(p.accent_tint)))
                    .child(
                        div()
                            .relative()
                            .w(px(16.))
                            .h_full()
                            .flex_shrink_0()
                            .child(
                                div()
                                    .absolute()
                                    .left(px(7.))
                                    .top(px(if i == 0 { 26. } else { 0. }))
                                    .bottom(px(if i + 1 == records.len() { 66. } else { 0. }))
                                    .w(px(1.))
                                    .bg(rgb(p.border)),
                            )
                            .when(current, |rail| {
                                rail.child(
                                    div()
                                        .absolute()
                                        .top(px(16.))
                                        .left(px(-2.))
                                        .size(px(20.))
                                        .rounded_full()
                                        .bg(gpui::rgba(0x007aff20)),
                                )
                            })
                            .child(
                                div()
                                    .absolute()
                                    .top(px(20.))
                                    .left(px(2.))
                                    .size(px(12.))
                                    .rounded_full()
                                    .border_2()
                                    .border_color(rgb(if current {
                                        p.accent
                                    } else {
                                        p.text_muted
                                    }))
                                    .bg(rgb(if current { p.accent } else { p.panel })),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .py_3()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_size(px(12.))
                                            .child(r.label.clone()),
                                    )
                                    .child(
                                        div()
                                            .px_2()
                                            .py(px(2.))
                                            .rounded_full()
                                            .text_size(px(9.))
                                            .text_color(rgb(self.theme.color(if current {
                                                0x24864c
                                            } else {
                                                0x007aff
                                            })))
                                            .bg(gpui::rgba(if current {
                                                0x24864c18
                                            } else {
                                                0x007aff14
                                            }))
                                            .child(if current { "当前" } else { "基线" }),
                                    )
                                    .child(div().flex_1().min_w_0())
                                    .child(self.version_tag_popover(i, r, cx))
                                    .child(
                                        div()
                                            .font_family("Menlo")
                                            .text_size(px(9.))
                                            .text_color(rgb(p.text_muted))
                                            .child(time),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .font_family("Menlo")
                                    .text_size(px(10.))
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(rgb(self
                                                .theme
                                                .color(method_color(&r.draft.method))))
                                            .child(r.draft.method.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .child(r.draft.url.clone()),
                                    )
                                    .child(
                                        Button::new("对比")
                                            .kind(ButtonKind::Plain)
                                            .size(ButtonSize::Mini)
                                            .theme(self.theme)
                                            .render()
                                            .debug_selector(move || {
                                                format!("version-diff-{i}").into()
                                            })
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.open_version_diff(i, window, cx);
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .font_family("Menlo")
                                    .text_size(px(9.))
                                    .text_color(rgb(p.text_muted))
                                    .when(r.source == model::VersionSource::Mcp, |s| {
                                        s.child(
                                            div()
                                                .debug_selector(move || {
                                                    format!("version-source-{i}").into()
                                                })
                                                .flex_shrink_0()
                                                .px_1()
                                                .rounded_md()
                                                .text_color(rgb(p.accent))
                                                .bg(rgb(p.accent_tint))
                                                .child("MCP"),
                                        )
                                    })
                                    .child(div().min_w_0().truncate().child(
                                        if params.is_empty() {
                                            format!(
                                                "#{} · 无查询参数",
                                                request_fingerprint::Fingerprint::new(&r.draft)
                                                    .hash
                                            )
                                        } else {
                                            format!(
                                                "#{} · {params}",
                                                request_fingerprint::Fingerprint::new(&r.draft)
                                                    .hash
                                            )
                                        },
                                    )),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.restore_version(i, cx);
                    })),
            );
        }
        if count == 0 {
            list = list.child(
                div()
                    .p_4()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(if self.activity == 0 {
                        "保存请求以创建版本"
                    } else {
                        "暂无匹配记录，发送请求后在此查看"
                    }),
            );
        }
        let mut panel = div()
            .w(px(270.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(rgb(p.panel))
            .border_l_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .h(px(44.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .child(
                        tab_button(
                            &format!("版本历史 {}", request.versions.len()),
                            self.activity == 0,
                            self.theme,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.activity = 0;
                            cx.notify();
                        })),
                    )
                    .child(
                        tab_button(
                            &format!("请求历史 {}", request.history.len()),
                            self.activity == 1,
                            self.theme,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.activity = 1;
                            cx.notify();
                        })),
                    )
                    .child(div().flex_1())
                    .child(
                        IconButton::new(Icon::Close, "收起记录")
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_activity = false;
                                cx.notify();
                            })),
                    ),
            );
        if self.activity == 1 {
            let mut filters = div().flex().gap_3().pt_2();
            for (i, label) in ["全部", "成功", "失败"].iter().enumerate() {
                filters = filters.child(
                    tab_button(label, self.history_filter == i, self.theme).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.history_filter = i;
                            cx.notify();
                        },
                    )),
                );
            }
            panel = panel.child(
                div()
                    .p_3()
                    .child(
                        SearchInput::new("搜索历史路径…")
                            .width(240.)
                            .control(self.history_search.clone())
                            .theme(self.theme)
                            .render(),
                    )
                    .child(filters),
            );
        } else {
            panel = panel.child(
                div()
                    .p_3()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("提交记录 · 新到旧"),
            );
        }
        panel.child(list).child(
            div()
                .p_3()
                .border_t_1()
                .border_color(rgb(p.border))
                .text_size(px(10.))
                .text_color(rgb(p.text_muted))
                .child(format!("共 {count} 条记录")),
        )
    }
}
