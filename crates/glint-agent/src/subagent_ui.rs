use super::*;
use crate::subagent::{Registry, Status, Trace};

#[derive(Default)]
pub struct State {
    pub live: Registry,
    pub selected: Option<(Uuid, Uuid)>,
    pub picker_open: bool,
    pub scrolls: std::cell::RefCell<std::collections::HashMap<Uuid, ScrollHandle>>,
    tasks: std::cell::RefCell<std::collections::HashMap<Uuid, TaskPanel>>,
}

struct TaskPanel {
    source: String,
    theme: ThemePreset,
    panel: Entity<gpui_glint::components::CollapsePanel>,
}

impl AgentWorkspace {
    pub fn all_subagents(&self) -> Vec<Trace> {
        let mut traces = Vec::new();
        for message in &self.history.active().messages {
            for record in &message.tools {
                if let Some(trace) = &record.subagent {
                    if !traces.iter().any(|other: &Trace| other.id == trace.id) {
                        traces.push(trace.clone());
                    }
                }
            }
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|(id, _)| *id == self.history.active)
        {
            if let Ok(live) = self.subagent_ui.live.lock() {
                for trace in live.iter() {
                    if !traces.iter().any(|other| other.id == trace.id) {
                        traces.push(trace.clone());
                    }
                }
            }
        }
        traces
    }

    pub fn selected_subagent(&self) -> Option<Trace> {
        let (session, id) = self.subagent_ui.selected?;
        (session == self.history.active)
            .then(|| {
                self.all_subagents()
                    .into_iter()
                    .find(|trace| trace.id == id)
            })
            .flatten()
    }

    pub fn selected_subagent_scroll(&self) -> Option<ScrollHandle> {
        let (_, id) = self.subagent_ui.selected?;
        Some(
            self.subagent_ui
                .scrolls
                .borrow_mut()
                .entry(id)
                .or_insert_with(ScrollHandle::new)
                .clone(),
        )
    }

    fn open_subagent(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        self.subagent_ui.selected = Some((self.history.active, id));
        self.subagent_ui.picker_open = false;
        window.blur();
        cx.notify();
    }

    pub fn trace_for_record(&self, record: &tools::ToolRecord) -> Option<Trace> {
        record.subagent.clone().or_else(|| {
            if record.call.name != "subagent" || record.result != "执行中…" {
                return None;
            }
            self.subagent_ui
                .live
                .lock()
                .ok()?
                .iter()
                .rev()
                .find(|trace| trace.call_id == record.call.id)
                .cloned()
        })
    }

    /// Merge directly from the request's registry, including events not yet drained
    /// by the UI. This keeps stopped/failed turns inspectable in saved history.
    pub fn finish_subagent_records(&mut self) {
        let traces = if let Ok(mut traces) = self.subagent_ui.live.lock() {
            for trace in traces.iter_mut() {
                trace.interrupt();
            }
            traces.clone()
        } else {
            Vec::new()
        };
        for trace in traces {
            if let Some(record) = self
                .tool_activity
                .iter_mut()
                .find(|record| record.call.id == trace.call_id)
            {
                record.subagent = Some(trace.clone());
                if record.result == "执行中…" {
                    record.result = trace
                        .error
                        .clone()
                        .unwrap_or_else(|| trace.status.label().into());
                    record.elapsed_ms = trace.elapsed_ms;
                    record.success = trace.status == Status::Completed;
                }
            } else {
                self.tool_activity.push(tools::ToolRecord {
                    call: tools::ToolCall {
                        id: trace.call_id.clone(),
                        name: "subagent".into(),
                        arguments: serde_json::json!({"task":trace.task,"title":trace.title}),
                    },
                    result: trace
                        .error
                        .clone()
                        .unwrap_or_else(|| trace.status.label().into()),
                    success: trace.status == Status::Completed,
                    elapsed_ms: trace.elapsed_ms,
                    live_output: None,
                    subagent: Some(trace.clone()),
                });
            }
            let record = self
                .tool_activity
                .iter()
                .find(|record| record.call.id == trace.call_id)
                .unwrap()
                .clone();
            if let Some(block) = self.transcript.iter_mut().find(|block| matches!(block, generation::ContentBlock::ToolResult(record) if record.call.id == trace.call_id)) {
                *block = generation::ContentBlock::ToolResult(record);
            } else { self.transcript.push(generation::ContentBlock::ToolResult(record)); }
        }
    }

    pub fn subagent_waiting_label(&self) -> &'static str {
        let traces = self.all_subagents();
        if traces
            .iter()
            .any(|trace| trace.status == Status::WaitingApproval)
        {
            "子代理正在等待批准…"
        } else if traces.iter().any(|trace| trace.status.active()) {
            "子代理正在执行任务…"
        } else {
            "正在等待模型回复…"
        }
    }

    pub fn subagent_revision(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::hash::DefaultHasher::new();
        if let Ok(traces) = self.subagent_ui.live.lock() {
            for trace in traces.iter() {
                trace.id.hash(&mut hash);
                trace.status.label().hash(&mut hash);
                trace.elapsed_ms.hash(&mut hash);
            }
        }
        hash.finish()
    }

    pub fn subagent_selector(&self, cx: &mut Context<Self>) -> gpui::Div {
        let traces = self.all_subagents();
        if traces.is_empty() {
            return div();
        }
        let p = self.theme.palette();
        let running = traces.iter().filter(|trace| trace.status.active()).count();
        let waiting = traces
            .iter()
            .filter(|trace| trace.status == Status::WaitingApproval)
            .count();
        let label = if waiting > 0 {
            format!("子代理 · {waiting} 待批准")
        } else if running > 0 {
            format!("子代理 · {running} 运行中")
        } else {
            format!("子代理 · {}", traces.len())
        };
        let trigger = div()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(px(6.))
            .text_color(rgb(if waiting > 0 { p.accent } else { p.text_muted }))
            .hover(|style| style.bg(rgb(p.surface_subtle)))
            .child(Icon::Workflow.render(15.))
            .child(label)
            .child(Icon::ChevronDown.render(12.));
        let mut menu = div()
            .id("subagent-menu-list")
            .max_h(px(360.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        for trace in traces {
            let id = trace.id;
            menu = menu.child(
                div()
                    .id(SharedString::from(format!("subagent-choice:{id}")))
                    .debug_selector(move || format!("subagent-choice:{id}"))
                    .p_2()
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(p.surface_subtle)))
                    .child(Icon::Workflow.render(15.).text_color(rgb(p.accent)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(trace.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(trace_caption(&trace)),
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.open_subagent(id, window, cx)),
                    ),
            );
        }
        let weak = cx.entity().downgrade();
        div().flex_shrink_0().child(
            div()
                .id("subagent-selector")
                .debug_selector(|| "subagent-selector".into())
                .child(
                    Popover::new("subagent-picker", trigger, menu)
                        .open(self.subagent_ui.picker_open)
                        .width(360.)
                        .padding(6.)
                        .theme(self.theme)
                        .on_open_change(move |open, _, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.subagent_ui.picker_open = open;
                                cx.notify();
                            });
                        }),
                ),
        )
    }

    pub fn subagent_summary(&self, trace: &Trace, cx: &mut Context<Self>) -> gpui::AnyView {
        let workspace = cx.entity().downgrade();
        let trace = trace.clone();
        cx.new(|_| Summary {
            workspace,
            trace,
            theme: self.theme,
        })
        .into()
    }

    fn subagent_task_panel(
        &self,
        trace: &Trace,
        cx: &mut Context<Self>,
    ) -> Entity<gpui_glint::components::CollapsePanel> {
        let mut tasks = self.subagent_ui.tasks.borrow_mut();
        let entry = tasks.entry(trace.id).or_insert_with(|| TaskPanel {
            source: String::new(),
            theme: self.theme,
            panel: cx.new(|_| {
                gpui_glint::components::CollapsePanel::new("委派任务")
                    .icon(Icon::Text)
                    .compact()
                    .theme(self.theme)
            }),
        });
        if entry.source != trace.task || entry.theme != self.theme {
            let task = trace.task.clone();
            let key = format!("subagent:{}:task", trace.id);
            let weak = cx.entity().downgrade();
            entry.panel.update(cx, |panel, cx| {
                panel.set_theme(self.theme, cx);
                panel.set_content(
                    move |_, cx| {
                        weak.update(cx, |this, cx| {
                            div().w_full().min_w_0().child(this.markdown_text(
                                key.clone(),
                                &task,
                                cx,
                            ))
                        })
                        .unwrap_or_else(|_| div())
                        .into_any_element()
                    },
                    cx,
                );
            });
            entry.source = trace.task.clone();
            entry.theme = self.theme;
        }
        entry.panel.clone()
    }

    pub fn subagent_page(&self, trace: &Trace, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let id = trace.id;
        let complete = !trace.status.active();
        let scroll = self.selected_subagent_scroll().unwrap_or_default();
        let header = div()
            .h(px(52.))
            .flex_shrink_0()
            .px_5()
            .border_b_1()
            .border_color(rgb(p.border))
            .flex()
            .items_center()
            .gap_3()
            .child(
                Button::new("返回主对话")
                    .kind(ButtonKind::Plain)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.subagent_ui.selected = None;
                        this.subagent_ui.picker_open = false;
                        cx.notify();
                    })),
            )
            .child(Icon::Workflow.render(16.).text_color(rgb(p.accent)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(trace.title.clone()),
            )
            .child(
                div()
                    .text_color(rgb(p.text_muted))
                    .child(trace_caption(trace)),
            )
            .child(self.subagent_selector(cx));
        let key = format!("{}:subagent:{id}", self.history.active);
        let rows = self.transcript_rows(
            &key,
            &trace.blocks,
            &trace.thinking_ms,
            complete,
            &trace.tools,
            cx,
        );
        let mut body = div()
            .w_full()
            .max_w(px(800.))
            .flex()
            .flex_col()
            .gap_4()
            .child(self.subagent_task_panel(trace, cx))
            .child(div().h(px(1.)).bg(rgb(p.border)))
            .children(rows);
        if let Some(error) = &trace.error {
            body = body.child(div().text_color(rgb(0xd9342b)).child(error.clone()));
        }
        if let Some(stats) = &trace.stats {
            body = body.child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child(stats.caption()),
            );
        }
        if trace.status.active() {
            body = body.child(
                ShimmerText::new(
                    SharedString::from(format!("subagent-working:{id}")),
                    if trace.status == Status::WaitingApproval {
                        "正在等待批准…"
                    } else {
                        "子代理正在工作…"
                    },
                )
                .theme(self.theme),
            );
        }
        let footer = div()
            .w_full()
            .flex_shrink_0()
            .px_5()
            .py_2()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .when(self.approval.is_some(), |body| {
                body.child(self.approval_panel(cx))
            })
            .when(
                self.pending
                    .as_ref()
                    .is_some_and(|(session, _)| *session == self.history.active),
                |body| {
                    body.child(
                        Button::new("停止主任务及子代理")
                            .kind(ButtonKind::Plain)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                    )
                },
            );
        div()
            .size_full()
            .child(page_layout(header, body, footer, &scroll))
    }
}

fn trace_caption(trace: &Trace) -> String {
    format!(
        "{} · {:.1}s",
        trace.status.label(),
        trace.elapsed_ms as f64 / 1000.
    )
}

fn page_layout(
    header: gpui::Div,
    body: gpui::Div,
    footer: gpui::Div,
    scroll: &ScrollHandle,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id("subagent-page")
        .debug_selector(|| "subagent-page".into())
        .size_full()
        .flex()
        .flex_col()
        .child(header)
        .child(
            div()
                .id("subagent-conversation")
                .debug_selector(|| "subagent-conversation".into())
                .track_scroll(scroll)
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(40.))
                .py_6()
                .flex()
                .flex_col()
                .items_center()
                .child(body),
        )
        .child(footer)
}

struct Summary {
    workspace: gpui::WeakEntity<AgentWorkspace>,
    trace: Trace,
    theme: ThemePreset,
}

impl Render for Summary {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let workspace = self.workspace.clone();
        let id = self.trace.id;
        div()
            .id(SharedString::from(format!("subagent-summary:{id}")))
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .py_2()
            .px_2()
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(p.surface_subtle)))
            .child(Icon::Workflow.render(16.).text_color(rgb(p.accent)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(format!("子代理 · {}", self.trace.title)),
            )
            .child(
                div()
                    .text_color(rgb(p.text_muted))
                    .child(trace_caption(&self.trace)),
            )
            .child(Icon::ChevronRight.render(12.).text_color(rgb(p.text_muted)))
            .on_click(move |_, window, cx| {
                let _ = workspace.update(cx, |this, cx| this.open_subagent(id, window, cx));
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct DetailLayout;
    impl Render for DetailLayout {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            page_layout(
                div().h(px(52.)),
                div().h(px(120.)),
                div().h(px(30.)),
                &ScrollHandle::new(),
            )
        }
    }
    #[gpui::test]
    fn detail_uses_full_conversation_space_without_a_composer(cx: &mut gpui::TestAppContext) {
        let (view, window) = cx.add_window_view(|_, _| DetailLayout);
        view.update(window, |_, cx| cx.notify());
        assert!(window.debug_bounds("subagent-page").is_some());
        let conversation = window.debug_bounds("subagent-conversation").unwrap();
        let size = window.update(|window, _| window.viewport_size());
        assert_eq!(conversation.size.height, size.height - px(82.));
        assert!(window.debug_bounds("new-session-composer").is_none());
    }
}
