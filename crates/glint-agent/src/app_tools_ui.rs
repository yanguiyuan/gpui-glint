use super::*;

pub struct MarkdownView {
    source: String,
    theme: ThemePreset,
    editor: Entity<gpui_glint::components::DocumentEditor>,
}

pub struct ToolPanel {
    signature: String,
    theme: ThemePreset,
    panel: Entity<gpui_glint::components::CollapsePanel>,
}

pub struct ProcessPanel {
    signature: String,
    theme: ThemePreset,
    complete: bool,
    panel: Entity<gpui_glint::components::CollapsePanel>,
}

pub struct Approval {
    pub call: tools::ToolCall,
    pub respond: tokio::sync::oneshot::Sender<bool>,
}

impl AgentWorkspace {
    pub fn watch_tools(
        &mut self,
        receiver: async_channel::Receiver<tools::ToolEvent>,
        cx: &mut Context<Self>,
    ) {
        self.tool_activity.clear();
        self.transcript.clear();
        self.tool_listener = Some(cx.spawn(async move |weak, cx| {
            while let Ok(event) = receiver.recv().await {
                if weak
                    .update(cx, |this, cx| {
                        match event {
                            tools::ToolEvent::Transcript(blocks) => this.transcript = blocks,
                            tools::ToolEvent::Started(call) => {
                                this.tool_activity.push(tools::ToolRecord {
                                    call,
                                    result: "执行中…".into(),
                                    success: false,
                                    elapsed_ms: 0,
                                })
                            }
                            tools::ToolEvent::Approval { call, respond } => {
                                this.approval = Some(Approval { call, respond })
                            }
                            tools::ToolEvent::Completed(record) => {
                                if let Some(existing) = this
                                    .tool_activity
                                    .iter_mut()
                                    .find(|item| item.call.id == record.call.id)
                                {
                                    *existing = record;
                                } else {
                                    this.tool_activity.push(record);
                                }
                            }
                        }
                        this.refresh_calendar(cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    pub fn tool_records(&self, records: &[tools::ToolRecord], cx: &mut Context<Self>) -> gpui::Div {
        use gpui_glint::components::CollapsePanel;
        let theme = self.theme;
        let p = theme.palette();
        let mut cache = self.tool_panels.borrow_mut();
        let mut list = div().w_full().min_w_0().flex().flex_col().gap_1();
        for record in records {
            let signature = serde_json::to_string(record).unwrap_or_default();
            let entry = cache
                .entry(record.call.id.clone())
                .or_insert_with(|| ToolPanel {
                    signature: String::new(),
                    theme,
                    panel: cx.new(|_| CollapsePanel::new("").icon(Icon::Workflow).theme(theme)),
                });
            if entry.signature != signature || entry.theme != theme {
                let status = if record.result == "执行中…" {
                    "执行中".into()
                } else {
                    format!(
                        "{} · {:.2}s",
                        if record.success {
                            "完成"
                        } else {
                            "未完成"
                        },
                        record.elapsed_ms as f64 / 1000.
                    )
                };
                let summary = format!("{} · {}", record.call.name, status);
                let arguments =
                    serde_json::to_string_pretty(&record.call.arguments).unwrap_or_default();
                let result: String = record.result.chars().take(8000).collect();
                entry.panel.update(cx, |panel, cx| {
                    panel.set_theme(theme, cx);
                    panel.set_summary(summary, cx);
                    panel.set_content(
                        move |_, _| {
                            div()
                                .min_w_0()
                                .w_full()
                                .text_size(px(12.))
                                .line_height(px(20.))
                                .text_color(rgb(p.text_muted))
                                .child(format!("参数\n{arguments}\n\n结果\n{result}"))
                                .into_any_element()
                        },
                        cx,
                    );
                });
                entry.signature = signature;
                entry.theme = theme;
            }
            list = list.child(entry.panel.clone());
        }
        list
    }

    pub fn markdown_text(
        &self,
        key: String,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Entity<gpui_glint::components::DocumentEditor> {
        let mut cache = self.markdown_views.borrow_mut();
        let theme = self.theme;
        let view = cache.entry(key).or_insert_with(|| MarkdownView {
            source: text.into(),
            theme,
            editor: cx.new(|cx| {
                gpui_glint::components::DocumentEditor::new(cx, text)
                    .theme(theme)
                    .embedded_preview()
            }),
        });
        if view.source != text || view.theme != theme {
            view.editor.update(cx, |editor, cx| {
                editor.set_theme(theme, cx);
                if view.source != text {
                    editor.set_markdown(text, cx);
                }
            });
            view.source = text.into();
            view.theme = theme;
        }
        view.editor.clone()
    }

    pub fn transcript_view(
        &self,
        key: &str,
        blocks: &[generation::ContentBlock],
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        use generation::ContentBlock;
        use gpui_glint::components::CollapsePanel;
        let theme = self.theme;
        let p = theme.palette();
        let mut body = div().w_full().min_w_0().flex().flex_col().gap_2();
        for (index, block) in blocks.iter().enumerate() {
            match block {
                ContentBlock::Text(text) => {
                    body = body.child(self.markdown_text(format!("{key}:{index}"), text, cx));
                }
                ContentBlock::Thinking(text) => {
                    let editor = self.markdown_text(format!("{key}:{index}:thinking"), text, cx);
                    let mut cache = self.tool_panels.borrow_mut();
                    let entry = cache
                        .entry(format!("{key}:{index}:thinking"))
                        .or_insert_with(|| ToolPanel {
                            signature: String::new(),
                            theme,
                            panel: cx
                                .new(|_| CollapsePanel::new("思考").icon(Icon::Pulse).theme(theme)),
                        });
                    if entry.signature != *text || entry.theme != theme {
                        entry.panel.update(cx, |panel, cx| {
                            panel.set_theme(theme, cx);
                            panel.set_content(
                                move |_, _| {
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .child(editor.clone())
                                        .into_any_element()
                                },
                                cx,
                            );
                        });
                        entry.signature = text.clone();
                        entry.theme = theme;
                    }
                    body = body.child(entry.panel.clone());
                }
                ContentBlock::ToolCall(call) => {
                    let mut cache = self.tool_panels.borrow_mut();
                    let signature = serde_json::to_string(call).unwrap_or_default();
                    let entry =
                        cache
                            .entry(format!("call:{}", call.id))
                            .or_insert_with(|| ToolPanel {
                                signature: String::new(),
                                theme,
                                panel: cx.new(|_| {
                                    CollapsePanel::new("").icon(Icon::Workflow).theme(theme)
                                }),
                            });
                    if entry.signature != signature || entry.theme != theme {
                        let arguments =
                            serde_json::to_string_pretty(&call.arguments).unwrap_or_default();
                        entry.panel.update(cx, |panel, cx| {
                            panel.set_theme(theme, cx);
                            panel.set_summary(format!("调用 {}", call.name), cx);
                            panel.set_content(
                                move |_, _| {
                                    div()
                                        .text_size(px(12.))
                                        .line_height(px(20.))
                                        .text_color(rgb(p.text_muted))
                                        .child(arguments.clone())
                                        .into_any_element()
                                },
                                cx,
                            );
                        });
                        entry.signature = signature;
                        entry.theme = theme;
                    }
                    body = body.child(entry.panel.clone());
                }
                ContentBlock::ToolResult(record) => {
                    body = body.child(self.tool_records(std::slice::from_ref(record), cx))
                }
            }
        }
        body
    }

    pub fn response_view(
        &self,
        key: &str,
        blocks: &[generation::ContentBlock],
        stats: Option<&generation::GenerationStats>,
        complete: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let split = process_prefix_len(blocks, complete);
        if split == 0 {
            return self.transcript_view(key, blocks, cx);
        }
        let theme = self.theme;
        let mut cache = self.process_panels.borrow_mut();
        let summary = if !complete {
            "正在工作…".into()
        } else if let Some(stats) = stats {
            let seconds = stats.elapsed_ms / 1000;
            if seconds >= 60 {
                format!("已工作 {} 分 {} 秒", seconds / 60, seconds % 60)
            } else {
                format!("已工作 {seconds} 秒")
            }
        } else {
            "工作过程".into()
        };
        let signature = format!(
            "{}:{summary}",
            serde_json::to_string(&blocks[..split]).unwrap_or_default()
        );
        let entry = cache.entry(key.into()).or_insert_with(|| ProcessPanel {
            signature: String::new(),
            theme,
            complete,
            panel: cx.new(|_| {
                gpui_glint::components::CollapsePanel::new("")
                    .compact()
                    .expanded(!complete)
                    .theme(theme)
            }),
        });
        if entry.signature != signature || entry.theme != theme || entry.complete != complete {
            let weak = cx.entity().downgrade();
            let process = blocks[..split].to_vec();
            let content_key = key.to_owned();
            entry.panel.update(cx, |panel, cx| {
                panel.set_theme(theme, cx);
                panel.set_summary(summary, cx);
                if complete && !entry.complete {
                    panel.set_expanded(false, cx);
                }
                panel.set_content(
                    move |_, cx| {
                        weak.update(cx, |this, cx| {
                            this.transcript_view(&content_key, &process, cx)
                        })
                        .unwrap_or_else(|_| div())
                        .into_any_element()
                    },
                    cx,
                );
            });
            entry.signature = signature;
            entry.theme = theme;
            entry.complete = complete;
        }
        let mut body = div().w_full().min_w_0().flex().flex_col().gap_3().child(
            div()
                .w_full()
                .pb_2()
                .border_b_1()
                .border_color(rgb(theme.palette().border))
                .child(entry.panel.clone()),
        );
        // Keep original indices stable as the final reply streams and the process collapses.
        for (index, block) in blocks.iter().enumerate().skip(split) {
            if let generation::ContentBlock::Text(text) = block {
                body = body.child(self.markdown_text(format!("{key}:{index}"), text, cx));
            }
        }
        body
    }

    pub fn approval_panel(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let mut panel = div();
        if let Some(approval) = &self.approval {
            panel = panel.w_full().max_w(px(800.)).p_4().flex().flex_col().gap_3().rounded(px(10.)).border_1().border_color(rgb(p.accent)).bg(rgb(p.panel))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("确认工具调用：{}", approval.call.name)))
                .child(div().id("approval-arguments").max_h(px(180.)).overflow_y_scroll().text_size(px(11.)).child(serde_json::to_string_pretty(&approval.call.arguments).unwrap_or_default()))
                .when(approval.call.name == "run_command", |panel| panel.child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child("命令在项目目录运行，批准前请检查程序和参数；命令自身可以访问项目外部。")))
                .child(div().flex().gap_3()
                    .child(Button::new("允许本次").theme(self.theme).render().on_click(cx.listener(|this, _, _, cx| { if let Some(approval) = this.approval.take() { let _=approval.respond.send(true); } cx.notify(); })))
                    .child(Button::new("拒绝").kind(ButtonKind::Glass).theme(self.theme).render().on_click(cx.listener(|this, _, _, cx| { if let Some(approval) = this.approval.take() { let _=approval.respond.send(false); } cx.notify(); }))));
        }
        panel
    }
}

/// The process is a contiguous prefix; only trailing answer text stays outside.
fn process_prefix_len(blocks: &[generation::ContentBlock], complete: bool) -> usize {
    let Some(last_process) = blocks
        .iter()
        .rposition(|block| !matches!(block, generation::ContentBlock::Text(_)))
    else {
        return 0;
    };
    if complete {
        last_process + 1
    } else {
        blocks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use generation::ContentBlock::{Text, Thinking};
    #[test]
    fn folding_preserves_intermediate_text_and_only_exposes_the_final_answer() {
        let blocks = vec![
            Thinking("plan".into()),
            Text("before tool".into()),
            Thinking("next".into()),
            Text("**answer**".into()),
        ];
        assert_eq!(process_prefix_len(&blocks, false), 4);
        assert_eq!(process_prefix_len(&blocks, true), 3);
        assert_eq!(process_prefix_len(&[Text("answer".into())], true), 0);
    }
}
