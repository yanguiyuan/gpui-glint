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

struct ToolPresentation {
    icon: Icon,
    summary: String,
    body: String,
}

fn tool_signature(record: &tools::ToolRecord) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::hash::DefaultHasher::new();
    record.call.name.hash(&mut hash);
    record.call.arguments.to_string().hash(&mut hash);
    record.result.hash(&mut hash);
    record.success.hash(&mut hash);
    record.elapsed_ms.hash(&mut hash);
    hash.finish().to_string()
}

fn command_summary(call: &tools::ToolCall) -> String {
    fn quote(value: &str) -> String {
        let value = value.replace(['\r', '\n'], " ");
        if !value.is_empty()
            && value
                .chars()
                .all(|ch| ch.is_alphanumeric() || "_./:@%+=,-".contains(ch))
        {
            value
        } else {
            format!("'{}'", value.replace('\'', "'\\''"))
        }
    }
    let mut command = vec![quote(
        call.arguments["program"].as_str().unwrap_or("run_command"),
    )];
    if let Some(args) = call.arguments["args"].as_array() {
        command.extend(args.iter().filter_map(|arg| arg.as_str()).map(quote));
    }
    command.join(" ")
}

fn tool_presentation(record: &tools::ToolRecord) -> ToolPresentation {
    let running = record.result == "执行中…";
    let command_failed = record.call.name == "run_command"
        && record
            .result
            .strip_prefix("exit: ")
            .and_then(|result| result.lines().next())
            .is_some_and(|status| status.trim() != "0");
    let success = record.success && !command_failed;
    let args = &record.call.arguments;
    let path = args["path"].as_str().unwrap_or(".");
    let (icon, action, target) = match record.call.name.as_str() {
        "run_command" => (
            Icon::Shell,
            if running {
                "正在运行"
            } else if success {
                "已运行"
            } else {
                "命令失败"
            },
            command_summary(&record.call),
        ),
        "read_file" => (
            Icon::Text,
            if running {
                "正在读取"
            } else if success {
                "已读取"
            } else {
                "读取失败"
            },
            path.into(),
        ),
        "list_files" => (
            Icon::Folder,
            if running {
                "正在列出目录"
            } else if success {
                "已列出目录"
            } else {
                "列出目录失败"
            },
            path.into(),
        ),
        "search_files" => (
            Icon::Search,
            if running {
                "正在搜索"
            } else if success {
                "已搜索"
            } else {
                "搜索失败"
            },
            args["pattern"].as_str().unwrap_or("").into(),
        ),
        "write_file" => (
            Icon::Pencil,
            if running {
                "正在写入"
            } else if success {
                "已写入"
            } else {
                "写入失败"
            },
            path.into(),
        ),
        "replace_in_file" => (
            Icon::Pencil,
            if running {
                "正在更新"
            } else if success {
                "已更新"
            } else {
                "更新失败"
            },
            path.into(),
        ),
        "skill_list" | "skill_read" => (
            Icon::Workflow,
            if running {
                "正在读取技能"
            } else if success {
                "已读取技能"
            } else {
                "技能读取失败"
            },
            args["name"]
                .as_str()
                .or(args["query"].as_str())
                .unwrap_or("技能目录")
                .into(),
        ),
        "session_read" => (
            Icon::Text,
            if running {
                "正在读取会话"
            } else if success {
                "已读取会话"
            } else {
                "读取会话失败"
            },
            args["session_id"].as_str().unwrap_or("").into(),
        ),
        name if name.starts_with("memory_") => (
            Icon::Text,
            if running {
                "正在处理记忆"
            } else if !success {
                "记忆处理失败"
            } else {
                match name {
                    "memory_search" => "已搜索记忆",
                    "memory_read" => "已读取记忆",
                    "memory_save" => "已保存记忆",
                    _ => "已删除记忆",
                }
            },
            args["title"]
                .as_str()
                .or(args["query"].as_str())
                .or(args["id"].as_str())
                .unwrap_or("")
                .into(),
        ),
        name if name.starts_with("schedule_") => (
            Icon::Calendar,
            if running {
                "正在处理日程"
            } else if success {
                "日程已处理"
            } else {
                "日程处理失败"
            },
            args["title"]
                .as_str()
                .or(args["id"].as_str())
                .unwrap_or("")
                .into(),
        ),
        _ => (
            Icon::Workflow,
            if running {
                "正在调用"
            } else if success {
                "已完成"
            } else {
                "调用失败"
            },
            record.call.name.clone(),
        ),
    };
    let mut summary = format!("{action} {target}");
    if record.call.name == "run_command" && !running {
        summary.push_str(&format!(" · {:.1}s", record.elapsed_ms as f64 / 1000.));
    }
    let body = if running {
        "等待工具返回结果…".into()
    } else if record.call.name == "run_command" && record.result.trim().is_empty() {
        "命令已结束，无输出。".into()
    } else {
        record.result.clone()
    };
    ToolPresentation {
        icon,
        summary,
        body,
    }
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
                            tools::ToolEvent::ContextUsage {
                                provider_id,
                                model,
                                used_tokens,
                                capacity_tokens,
                            } => {
                                if let Some((id, _)) = &this.pending {
                                    if let Some(session) = this
                                        .history
                                        .sessions
                                        .iter_mut()
                                        .find(|session| session.id == *id)
                                    {
                                        session.context = Some(context_usage::ContextSnapshot {
                                            provider_id,
                                            model,
                                            used_tokens,
                                            capacity_tokens,
                                        });
                                    }
                                }
                                this.sync_context_usage(cx);
                            }
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
                                if *this.access.lock().unwrap() != tools::AccessMode::Ask {
                                    let _ = respond.send(true);
                                } else {
                                    this.approval = Some(Approval { call, respond });
                                }
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
            let signature = tool_signature(record);
            let entry = cache
                .entry(record.call.id.clone())
                .or_insert_with(|| ToolPanel {
                    signature: String::new(),
                    theme,
                    panel: cx.new(|_| {
                        CollapsePanel::new("")
                            .icon(tool_presentation(record).icon)
                            .theme(theme)
                    }),
                });
            if entry.signature != signature || entry.theme != theme {
                let presentation = tool_presentation(record);
                let result: SharedString = presentation.body.into();
                entry.panel.update(cx, |panel, cx| {
                    panel.set_theme(theme, cx);
                    panel.set_summary(presentation.summary, cx);
                    panel.set_content(
                        move |_, _| {
                            div()
                                .id("tool-output")
                                .w_full()
                                .min_w_0()
                                .max_h(px(320.))
                                .overflow_y_scroll()
                                .text_size(px(12.))
                                .line_height(px(20.))
                                .font_family("monospace")
                                .text_color(rgb(p.text_muted))
                                .child(result.clone())
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
        let mut body = div().w_full().min_w_0().flex().flex_col().gap_2();
        let completed: std::collections::HashSet<_> = blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolResult(record) => Some(record.call.id.as_str()),
                _ => None,
            })
            .collect();
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
                    // Completed calls use their result row and the same panel
                    // identity, so streaming never leaves a duplicate call row.
                    if !completed.contains(call.id.as_str()) {
                        let running = tools::ToolRecord {
                            call: call.clone(),
                            result: "执行中…".into(),
                            success: false,
                            elapsed_ms: 0,
                        };
                        body = body.child(self.tool_records(std::slice::from_ref(&running), cx));
                    }
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
    fn builtin_summaries_describe_commands_and_files_and_bodies_are_only_results() {
        let mut record = tools::ToolRecord {
            call: tools::ToolCall {
                id: "one".into(),
                name: "run_command".into(),
                arguments: serde_json::json!({"program":"git","args":["diff","--cached","--name-status"]}),
            },
            result: "exit: 0\nM src/main.rs".into(),
            success: true,
            elapsed_ms: 1200,
        };
        let view = tool_presentation(&record);
        assert_eq!(
            view.summary,
            "已运行 git diff --cached --name-status · 1.2s"
        );
        assert_eq!(view.body, record.result);
        record.result = "执行中…".into();
        assert!(
            tool_presentation(&record)
                .summary
                .starts_with("正在运行 git diff")
        );
        record.result = "exit: 2\nerror".into();
        assert!(tool_presentation(&record).summary.starts_with("命令失败"));
        record.call.name = "read_file".into();
        record.call.arguments = serde_json::json!({"path":"src/main.rs","start_line":1});
        record.result = "1: fn main() {}".into();
        assert_eq!(tool_presentation(&record).summary, "已读取 src/main.rs");
        assert_eq!(tool_presentation(&record).body, "1: fn main() {}");
        record.success = false;
        assert_eq!(tool_presentation(&record).summary, "读取失败 src/main.rs");
        record.call.arguments =
            serde_json::json!({"program":"sh","args":["-c","printf '%s' hello"]});
        assert_eq!(
            command_summary(&record.call),
            "sh -c 'printf '\\''%s'\\'' hello'"
        );
        record.result = "x".repeat(12000);
        assert_eq!(tool_presentation(&record).body.len(), 12000);
    }

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
