//! A session-local ledger over saved history and the current tool transcript.
//! Rendering summaries never changes the JSON inspected or copied by the user.
use super::*;
use generation::ContentBlock;
use gpui_glint::components::CodeCard;
use serde_json::{Value, json};
use std::{cell::RefCell, collections::HashSet, rc::Rc};

#[derive(Clone, Copy, Default, PartialEq)]
enum Detail {
    #[default]
    Arguments,
    Output,
    Record,
}

#[derive(Default)]
pub struct State {
    pub open: bool,
    detail_open: bool,
    selected: Option<(Uuid, String)>,
    detail: Detail,
    scroll: gpui::UniformListScrollHandle,
    editor: RefCell<Option<RawEditor>>,
}

struct RawEditor {
    source: String,
    theme: ThemePreset,
    language: &'static str,
    view: Entity<CodeCard>,
}

impl State {
    pub(super) fn close_detail(&mut self) {
        self.detail_open = false;
    }
}

fn preview(text: &str) -> String {
    let end = text.char_indices().nth(220).map(|(index, _)| index);
    let mut result = text[..end.unwrap_or(text.len())]
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect::<String>();
    if end.is_some() {
        result.push('…');
    }
    result
}

fn result_preview(record: &tools::ToolRecord) -> String {
    let text = record.live_output.as_deref().unwrap_or(&record.result);
    if text.len() <= 65536 {
        if let Ok(value) = serde_json::from_str::<Value>(text) {
            for key in if record.success {
                ["stdout", "content", "message", "error", "stderr"]
            } else {
                ["error", "stderr", "message", "stdout", "content"]
            } {
                if let Some(text) = value[key].as_str().filter(|s| !s.trim().is_empty()) {
                    return preview(text);
                }
            }
            if let Some(text) = value["page"]["content"].as_str() {
                return preview(text);
            }
        }
    }
    preview(text)
}

enum Payload<'a> {
    Message(&'a model::Message),
    Block(&'a ContentBlock),
    Tool {
        call: &'a tools::ToolCall,
        record: Option<&'a tools::ToolRecord>,
    },
    Agent(&'a subagent::Trace),
}

struct Entry<'a> {
    id: String,
    title: String,
    status: &'static str,
    depth: usize,
    payload: Payload<'a>,
}

impl Entry<'_> {
    fn row(&self) -> Row {
        let (icon, title, description) = match &self.payload {
            Payload::Tool { call, record } => {
                let (icon, title) = app_tools_ui::tool_summary(call, *record);
                (
                    icon,
                    title,
                    record
                        .map(result_preview)
                        .unwrap_or_else(|| "等待工具执行结果".into()),
                )
            }
            Payload::Block(ContentBlock::Thinking(text)) => {
                (Icon::Workflow, "思考".into(), preview(text))
            }
            Payload::Block(ContentBlock::Text(text)) => {
                (Icon::Text, "助手输出".into(), preview(text))
            }
            Payload::Block(_) => (Icon::Workflow, self.title.clone(), String::new()),
            Payload::Message(message) => (
                Icon::Text,
                self.title.clone(),
                if message.text.trim().is_empty() {
                    format!(
                        "{} 个过程片段 · {} 个附件",
                        message.blocks.len(),
                        message.images.len()
                    )
                } else {
                    preview(&message.text)
                },
            ),
            Payload::Agent(agent) => (
                Icon::Workflow,
                format!("子代理 · {}", agent.title),
                preview(&agent.task),
            ),
        };
        Row {
            id: self.id.clone(),
            title: preview(&title),
            preview: description,
            icon,
            status: self.status,
            depth: self.depth,
        }
    }
    fn record(&self) -> Value {
        match &self.payload {
            Payload::Message(message) => json!({"kind":"message","message":message}),
            Payload::Block(block) => json!({"kind":"content","block":block}),
            Payload::Tool { call, record } => {
                json!({"kind":"tool","status":self.status,"call":call,"record":record,"live_output":record.and_then(|r|r.live_output.as_deref())})
            }
            Payload::Agent(agent) => json!({"kind":"subagent","trace":agent}),
        }
    }
    fn source(&self, detail: Detail) -> (String, &'static str) {
        match (&self.payload, detail) {
            (Payload::Tool { call, .. }, Detail::Arguments) => (pretty(&call.arguments), "json"),
            (Payload::Tool { record, .. }, Detail::Output) => (
                record
                    .map(|r| r.live_output.as_deref().unwrap_or(&r.result))
                    .unwrap_or("尚未返回结果")
                    .to_owned(),
                "text",
            ),
            _ => (pretty(&self.record()), "json"),
        }
    }
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "无法编码记录".into())
}

fn add_tool<'a>(
    entries: &mut Vec<Entry<'a>>,
    scope: &str,
    call: &'a tools::ToolCall,
    records: &'a [tools::ToolRecord],
    agents: &'a [subagent::Trace],
    depth: usize,
    live: bool,
    seen: &mut HashSet<String>,
) {
    if !seen.insert(call.id.clone()) {
        return;
    }
    let record = records.iter().find(|r| r.call.id == call.id);
    let call = record.map(|r| &r.call).unwrap_or(call);
    let status = match record {
        Some(r) if r.result == "执行中…" => "执行中",
        Some(r) if r.success => "成功",
        Some(r) if r.result.starts_with("未执行：") => "未执行",
        Some(_) => "失败",
        None if live => "待执行",
        None => "未完成",
    };
    let id = format!("{scope}/tool:{}", call.id);
    entries.push(Entry {
        id: id.clone(),
        title: call.name.clone(),
        status,
        depth,
        payload: Payload::Tool { call, record },
    });
    if let Some(record) = record {
        let mut nested = HashSet::new();
        for child in record.nested_tools.iter() {
            add_tool(
                entries,
                &id,
                &child.call,
                &record.nested_tools,
                &[],
                depth + 1,
                live,
                &mut nested,
            );
        }
    }
    if let Some(agent) = agents
        .iter()
        .find(|a| a.call_id == call.id)
        .or_else(|| record.and_then(|r| r.subagent.as_ref()))
    {
        let scope = format!("{id}/agent:{}", agent.id);
        entries.push(Entry {
            id: scope.clone(),
            title: agent.title.clone(),
            status: agent.status.label(),
            depth: depth + 1,
            payload: Payload::Agent(agent),
        });
        add_blocks(
            entries,
            &scope,
            &agent.blocks,
            &agent.tools,
            &[],
            depth + 2,
            agent.status.active(),
        );
    }
}

fn add_blocks<'a>(
    entries: &mut Vec<Entry<'a>>,
    scope: &str,
    blocks: &'a [ContentBlock],
    records: &'a [tools::ToolRecord],
    agents: &'a [subagent::Trace],
    depth: usize,
    live: bool,
) {
    let mut seen = HashSet::new();
    // Result blocks are also authoritative when a legacy message lacks tools[].
    let find_records = |call: &tools::ToolCall| records.iter().find(|r| r.call.id == call.id);
    for (index, block) in blocks.iter().enumerate() {
        match block {
            ContentBlock::Thinking(text) if text.trim().is_empty() => continue,
            ContentBlock::ToolCall(call) => {
                if find_records(call).is_some() {
                    add_tool(
                        entries, scope, call, records, agents, depth, live, &mut seen,
                    );
                } else if let Some(record) = blocks.iter().find_map(|b| match b {
                    ContentBlock::ToolResult(r) if r.call.id == call.id => Some(r),
                    _ => None,
                }) {
                    add_tool(
                        entries,
                        scope,
                        &record.call,
                        std::slice::from_ref(record),
                        agents,
                        depth,
                        live,
                        &mut seen,
                    );
                } else {
                    add_tool(entries, scope, call, &[], agents, depth, live, &mut seen);
                }
            }
            ContentBlock::ToolResult(record) => add_tool(
                entries,
                scope,
                &record.call,
                if find_records(&record.call).is_some() {
                    records
                } else {
                    std::slice::from_ref(record)
                },
                agents,
                depth,
                live,
                &mut seen,
            ),
            ContentBlock::Text(_) | ContentBlock::Thinking(_) => entries.push(Entry {
                id: format!("{scope}/block:{index}"),
                title: if matches!(block, ContentBlock::Thinking(_)) {
                    "思考"
                } else {
                    "正文"
                }
                .into(),
                status: if live { "生成中" } else { "已记录" },
                depth,
                payload: Payload::Block(block),
            }),
        }
    }
    for record in records {
        add_tool(
            entries,
            scope,
            &record.call,
            records,
            agents,
            depth,
            live,
            &mut seen,
        );
    }
}

fn entries<'a>(
    session: &'a model::Session,
    blocks: &'a [ContentBlock],
    activity: &'a [tools::ToolRecord],
    agents: &'a [subagent::Trace],
    running: bool,
) -> Vec<Entry<'a>> {
    let mut entries = Vec::new();
    for (index, message) in session.messages.iter().enumerate() {
        let scope = format!("message:{index}");
        entries.push(Entry {
            id: scope.clone(),
            title: format!(
                "{} · {}",
                match message.role {
                    Role::User => "用户",
                    Role::Agent => "助手",
                    Role::Notice => "系统通知",
                },
                message.time.format("%H:%M:%S")
            ),
            status: "已记录",
            depth: 0,
            payload: Payload::Message(message),
        });
        add_blocks(
            &mut entries,
            &scope,
            &message.blocks,
            &message.tools,
            &[],
            1,
            false,
        );
    }
    if running {
        add_blocks(
            &mut entries,
            &format!("message:{}", session.messages.len()),
            blocks,
            activity,
            agents,
            0,
            true,
        );
    }
    entries
}

#[derive(Clone)]
struct Row {
    id: String,
    title: String,
    status: &'static str,
    depth: usize,
    preview: String,
    icon: Icon,
}

impl AgentWorkspace {
    pub(super) fn session_view_tabs(&self, cx: &mut Context<Self>) -> gpui::Div {
        let weak = cx.entity().downgrade();
        div().flex_shrink_0().child(
            SegmentedControl::new([SegmentOption::new("聊天"), SegmentOption::new("轨迹")])
                .id("session-view-tabs")
                .selected(usize::from(self.trajectory.open))
                .theme(self.theme)
                .on_change(move |index, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.trajectory.open = index == 1;
                        cx.notify();
                    });
                })
                .render()
                .debug_selector(|| "session-view-tabs".into()),
        )
    }

    fn trajectory_running(&self) -> bool {
        self.session_busy(self.history.active)
    }

    fn copy_trajectory(&self, cx: &mut Context<Self>) {
        let agents = self.active_run().subagents.lock().ok();
        let entries = entries(
            self.history.active(),
            &self.active_run().transcript,
            &self.active_run().tool_activity,
            agents.as_deref().map(Vec::as_slice).unwrap_or_default(),
            self.trajectory_running(),
        );
        let value = json!({"session_id":self.history.active,"entries":entries.iter().map(Entry::record).collect::<Vec<_>>()});
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(pretty(&value)));
    }

    pub(super) fn trajectory_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let session = self.history.active();
        let agents = self.active_run().subagents.lock().ok();
        let mut entries = entries(
            session,
            &self.active_run().transcript,
            &self.active_run().tool_activity,
            agents.as_deref().map(Vec::as_slice).unwrap_or_default(),
            self.trajectory_running(),
        );
        if let Some(approval) = &self.active_run().approval {
            for entry in &mut entries {
                if matches!(&entry.payload,Payload::Tool{call,..} if call.id==approval.call.id) {
                    entry.status = "待批准";
                }
            }
        }
        let selected = self
            .trajectory
            .detail_open
            .then(|| self.trajectory.selected.as_ref())
            .flatten()
            .filter(|(id, _)| *id == session.id)
            .and_then(|(_, id)| entries.iter().find(|e| e.id == *id));
        let selected_id = selected.map(|e| e.id.clone());
        let rows = Rc::new(entries.iter().map(Entry::row).collect::<Vec<_>>());
        let toolbar = div()
            .flex_shrink_0()
            .h(px(42.))
            .px_4()
            .flex()
            .items_center()
            .gap_3()
            .border_b_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .flex_1()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(format!("{} 条记录 · 点击记录查看详情", entries.len())),
            )
            .child(
                Button::new("复制全部轨迹")
                    .kind(ButtonKind::Plain)
                    .size(ButtonSize::Small)
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "trajectory-copy-all".into())
                    .on_click(cx.listener(|this, _, _, cx| this.copy_trajectory(cx))),
            );
        let detail = if let Some(entry) = selected {
            let (source, language) = entry.source(self.trajectory.detail);
            let mut editor = self.trajectory.editor.borrow_mut();
            let raw = editor.get_or_insert_with(|| RawEditor {
                source: source.clone(),
                theme: self.theme,
                language,
                view: cx.new(|cx| {
                    CodeCard::new(cx, language, source.clone())
                        .read_only(true)
                        .show_header(false)
                        .viewport()
                        .theme(self.theme)
                }),
            });
            if raw.source != source || raw.theme != self.theme || raw.language != language {
                raw.view.update(cx, |card, cx| {
                    card.set_theme(self.theme, cx);
                    card.set_language(language, cx);
                    if raw.source != source {
                        card.set_content(source.clone(), cx);
                    }
                });
                raw.source = source.clone();
                raw.theme = self.theme;
                raw.language = language;
            }
            let tool = matches!(entry.payload, Payload::Tool { .. });
            let current = if tool {
                self.trajectory.detail
            } else {
                Detail::Record
            };
            let mut tabs = div().flex().items_center().gap_1();
            for (label, mode) in [
                ("参数", Detail::Arguments),
                ("结果", Detail::Output),
                ("完整记录", Detail::Record),
            ] {
                if tool || mode == Detail::Record {
                    tabs = tabs.child(
                        Button::new(label)
                            .kind(if current == mode {
                                ButtonKind::Tinted
                            } else {
                                ButtonKind::Plain
                            })
                            .size(ButtonSize::Small)
                            .theme(self.theme)
                            .render()
                            .debug_selector(move || format!("trajectory-detail-{label}"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.trajectory.detail = mode;
                                cx.notify();
                            })),
                    );
                }
            }
            Some(
                div()
                    .debug_selector(|| "trajectory-details-sidebar".into())
                    .w(gpui::relative(0.5))
                    .min_w(px(320.))
                    .max_w(px(640.))
                    .flex_shrink_0()
                    .border_l_1()
                    .border_color(rgb(p.border))
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .min_h(px(42.))
                            .py_1()
                            .flex_shrink_0()
                            .px_3()
                            .flex()
                            .items_center()
                            .flex_wrap()
                            .gap_2()
                            .border_b_1()
                            .border_color(rgb(p.border))
                            .child(tabs)
                            .child(div().flex_1())
                            .child(
                                Button::new("复制")
                                    .size(ButtonSize::Small)
                                    .kind(ButtonKind::Plain)
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(|| "trajectory-copy-detail".into())
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                            source.clone(),
                                        ))
                                    }),
                            )
                            .child(
                                Button::new("关闭详情")
                                    .icon(Icon::Close)
                                    .kind(ButtonKind::Plain)
                                    .size(ButtonSize::Small)
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(|| "trajectory-close-detail".into())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.trajectory.close_detail();
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "trajectory-raw-detail".into())
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .child(raw.view.clone()),
                    ),
            )
        } else {
            None
        };
        let session_id = session.id;
        let empty = rows.is_empty();
        let count = rows.len();
        let list = gpui::uniform_list(
            "trajectory-events",
            count,
            cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|index| {
                        let row = &rows[index];
                        let id = row.id.clone();
                        let selected = selected_id.as_deref() == Some(&row.id);
                        div()
                            .id(SharedString::from(format!("trajectory:{}", row.id)))
                            .debug_selector(move || format!("trajectory-row-{index}"))
                            .h(px(72.))
                            .w_full()
                            .min_w_0()
                            .pl(px(16. + row.depth.min(5) as f32 * 12.))
                            .pr_4()
                            .py_3()
                            .border_b_1()
                            .border_color(rgb(p.border))
                            .cursor_pointer()
                            .bg(rgb(if selected { p.surface } else { p.panel }))
                            .hover(|s| s.bg(rgb(p.surface_subtle)))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(28.))
                                            .flex_shrink_0()
                                            .text_size(px(10.))
                                            .text_color(rgb(p.text_muted))
                                            .child(format!("{:03}", index + 1)),
                                    )
                                    .child(row.icon.render(14.).text_color(rgb(p.text_muted)))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(row.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_size(px(10.))
                                            .text_color(rgb(
                                                if matches!(
                                                    row.status,
                                                    "失败" | "未执行" | "未完成"
                                                ) {
                                                    0xd9342b
                                                } else {
                                                    p.text_muted
                                                },
                                            ))
                                            .child(row.status),
                                    )
                                    .child(
                                        Icon::ChevronRight
                                            .render(12.)
                                            .text_color(rgb(p.text_muted)),
                                    ),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .pl(px(50.))
                                    .truncate()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.text_muted))
                                    .child(row.preview.clone()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.trajectory.selected = Some((session_id, id.clone()));
                                this.trajectory.detail_open = true;
                                cx.notify();
                            }))
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .size_full()
        .track_scroll(&self.trajectory.scroll);
        div()
            .debug_selector(|| "session-trajectory".into())
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .debug_selector(|| "trajectory-record-list".into())
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .when(!empty, |body| body.child(list))
                            .when(empty, |body| {
                                body.child(
                                    div()
                                        .size_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_color(rgb(p.text_muted))
                                        .child("此会话尚无轨迹记录"),
                                )
                            }),
                    )
                    .when_some(detail, |body, detail| body.child(detail)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_historical_and_live_thinking_is_hidden_but_kept_in_full_message_record() {
        let mut session = model::Session::new();
        session.push(Role::Agent, "answer".into());
        session.messages[0].blocks = vec![
            ContentBlock::Thinking(String::new()),
            ContentBlock::Thinking(" \n\t".into()),
            ContentBlock::Thinking("actual thought".into()),
            ContentBlock::Text("answer".into()),
        ];
        let rows = entries(&session, &[], &[], &[], false);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].record()["message"]["blocks"][0]["Thinking"], "");
        assert!(rows[1].id.ends_with("block:2"));
        let empty = model::Session::new();
        let live = entries(&empty, &session.messages[0].blocks, &[], &[], true);
        assert_eq!(live.len(), 2);
    }
    #[test]
    fn list_summaries_match_chat_and_elide_only_previews() {
        let record = failed("summary");
        let session = session(&record);
        let entries = entries(&session, &[], &[], &[], false);
        let entry = entries
            .iter()
            .find(|e| matches!(e.payload, Payload::Tool { .. }))
            .unwrap();
        let row = entry.row();
        assert_eq!(
            row.title,
            preview(&app_tools_ui::tool_summary(&record.call, Some(&record)).1)
        );
        assert!(row.title.contains("git status --porcelain=v1 -b"));
        assert!(row.preview.contains("路径必须位于项目目录内"));
        let long = "多行说明\n".repeat(500);
        let block = ContentBlock::Thinking(long.clone());
        let entry = Entry {
            id: "long".into(),
            title: "思考".into(),
            status: "已记录",
            depth: 0,
            payload: Payload::Block(&block),
        };
        let row = entry.row();
        assert!(row.preview.ends_with('…'));
        assert!(row.preview.chars().count() <= 221);
        assert!(!row.preview.contains('\n'));
        assert_eq!(entry.record()["block"]["Thinking"], long);
    }
    fn failed(id: &str) -> tools::ToolRecord {
        tools::rejected(
            tools::ToolCall {
                id: id.into(),
                name: "run_command".into(),
                arguments: json!({"program":"git","args":["status","--porcelain=v1","-b"],"cwd":"D:\\项目\\my repo","timeout_seconds":60,"background":false,"extra":{"quote":"\"hello\"\n第二行"}}),
            },
            "路径必须位于项目目录内".into(),
        )
    }
    fn session(record: &tools::ToolRecord) -> model::Session {
        let mut session = model::Session::new();
        session.push(Role::User, "检查工作区".into());
        session.push(Role::Agent, "reply".into());
        let message = session.messages.last_mut().unwrap();
        message.tools = vec![record.clone()];
        message.blocks = vec![
            ContentBlock::Thinking("plan".into()),
            ContentBlock::ToolCall(record.call.clone()),
            ContentBlock::ToolResult(record.clone()),
            ContentBlock::Text("reply".into()),
        ];
        session
    }
    #[test]
    fn failed_call_keeps_all_arguments_and_one_row_after_history_round_trip() {
        let record = failed("call");
        let session = session(&record);
        let mut history = History::default();
        history.active = session.id;
        history.sessions = vec![session];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        history.save(&path).unwrap();
        let loaded = History::load(&path).unwrap();
        let rows = entries(loaded.active(), &[], &[], &[], false);
        let tools: Vec<_> = rows
            .iter()
            .filter(|e| matches!(e.payload, Payload::Tool { .. }))
            .collect();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].status, "失败");
        let (raw, _) = tools[0].source(Detail::Arguments);
        assert_eq!(
            serde_json::from_str::<Value>(&raw).unwrap(),
            record.call.arguments
        );
        assert_eq!(tools[0].source(Detail::Output).0, record.result);
        let raw = tools[0].record();
        assert_eq!(raw["call"]["arguments"]["cwd"], "D:\\项目\\my repo");
    }
    #[test]
    fn legacy_result_only_and_large_output_are_complete() {
        let mut record = failed("legacy");
        record.result = "原始输出\n".repeat(2000);
        let mut session = session(&record);
        let msg = session.messages.last_mut().unwrap();
        msg.tools.clear();
        msg.blocks = vec![ContentBlock::ToolResult(record.clone())];
        let rows = entries(&session, &[], &[], &[], false);
        let row = rows.last().unwrap();
        assert_eq!(row.source(Detail::Output).0, record.result);
        assert_eq!(row.record()["record"]["result"], record.result);
    }
    #[test]
    fn live_calls_keep_ids_when_completed_and_never_leak_into_other_sessions() {
        let mut session = model::Session::new();
        session.push(Role::User, "run".into());
        let mut record = failed("live");
        record.result = "执行中…".into();
        record.live_output = Some("live stdout".into());
        let blocks = vec![ContentBlock::ToolCall(record.call.clone())];
        let activity = vec![record.clone()];
        let running = entries(&session, &blocks, &activity, &[], true);
        let row = running.last().unwrap();
        let id = row.id.clone();
        assert_eq!(row.status, "执行中");
        assert_eq!(row.source(Detail::Output).0, "live stdout");
        assert_eq!(entries(&session, &blocks, &activity, &[], false).len(), 1);
        session.push(Role::Agent, "done".into());
        session.messages.last_mut().unwrap().tools = activity;
        let done = entries(&session, &[], &[], &[], false);
        assert_eq!(done.last().unwrap().id, id);
    }
    #[test]
    fn nested_calls_and_subagent_blocks_remain_inspectable_with_distinct_ids() {
        let mut parent = failed("outer");
        parent.call.name = "code_mode".into();
        let child = failed("inner");
        parent.nested_tools = vec![child.clone()].into();
        let agent = subagent::Trace {
            partner_id: None,
            todos: vec![],
            id: Uuid::new_v4(),
            call_id: parent.call.id.clone(),
            title: "检查仓库".into(),
            task: "task".into(),
            status: subagent::Status::Running,
            blocks: vec![ContentBlock::Thinking("child plan".into())],
            tools: vec![child.clone()],
            thinking_ms: Default::default(),
            stats: None,
            elapsed_ms: 10,
            error: None,
        };
        let s = model::Session::new();
        let records = vec![parent];
        let agents = vec![agent];
        let rows = entries(&s, &[], &records, &agents, true);
        assert!(rows.iter().any(|e| matches!(e.payload, Payload::Agent(_))));
        assert!(rows.iter().any(|e| e.title == "思考"));
        let ids: HashSet<_> = rows.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids.len(), rows.len());
        let children: Vec<_> = rows
            .iter()
            .filter(|e| matches!(&e.payload,Payload::Tool{call,..} if call.id=="inner"))
            .collect();
        assert_eq!(children.len(), 2);
        for child_row in children {
            assert_eq!(
                serde_json::from_str::<Value>(&child_row.source(Detail::Arguments).0).unwrap(),
                child.call.arguments
            );
        }
    }
    #[gpui::test]
    fn tabs_show_full_json_copy_and_preserve_composer_and_session(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let record = failed("inspect");
        let expected = record.call.arguments.clone();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(dir.path().join("sessions.json"), cx);
            host.history.sessions = vec![session(&record)];
            host.history.active = host.history.sessions[0].id;
            host.composer
                .update(cx, |c, cx| c.set_value("保留草稿", cx));
            host
        });
        window.simulate_resize(size(px(1440.), px(1000.)));
        window.run_until_parked();
        let id = host.read_with(window, |h, _| h.history.active);
        let composer_bounds = window.debug_bounds("agent-composer-panel").unwrap();
        let tabs = window.debug_bounds("session-view-tabs").unwrap();
        window.simulate_click(
            point(tabs.right() - px(24.), tabs.center().y),
            gpui::Modifiers::default(),
        );
        window.run_until_parked();
        assert!(window.debug_bounds("session-trajectory").is_some());
        assert!(window.debug_bounds("agent-composer-panel").is_none());
        assert!(window.debug_bounds("trajectory-details-sidebar").is_none());
        assert!(window.debug_bounds("trajectory-raw-detail").is_none());
        let list_bounds = window.debug_bounds("trajectory-record-list").unwrap();
        assert!(list_bounds.bottom() > composer_bounds.top());
        assert_eq!(
            list_bounds.size.width,
            window
                .debug_bounds("session-trajectory")
                .unwrap()
                .size
                .width
        );
        assert!(window.debug_bounds("agent-chat-conversation").is_none());
        // User, assistant, thinking, tool, answer. Select the failed tool.
        let row = window.debug_bounds("trajectory-row-3").unwrap();
        window.simulate_click(row.center(), gpui::Modifiers::default());
        window.run_until_parked();
        assert!(window.debug_bounds("trajectory-details-sidebar").is_some());
        assert!(
            window
                .debug_bounds("trajectory-raw-detail")
                .unwrap()
                .bottom()
                > composer_bounds.top()
        );
        assert!(
            window
                .debug_bounds("trajectory-record-list")
                .unwrap()
                .size
                .width
                < list_bounds.size.width
        );
        let copy = window.debug_bounds("trajectory-copy-detail").unwrap();
        window.simulate_click(copy.center(), gpui::Modifiers::default());
        window.run_until_parked();
        let copied = window.read_from_clipboard().unwrap().text().unwrap();
        assert_eq!(serde_json::from_str::<Value>(&copied).unwrap(), expected);
        let all = window.debug_bounds("trajectory-copy-all").unwrap();
        window.simulate_click(all.center(), gpui::Modifiers::default());
        window.run_until_parked();
        let all: Value =
            serde_json::from_str(&window.read_from_clipboard().unwrap().text().unwrap()).unwrap();
        assert_eq!(all["session_id"], id.to_string());
        assert!(
            all["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["call"]["id"] == "inspect")
        );
        window.simulate_resize(size(px(1000.), px(800.)));
        window.run_until_parked();
        let sidebar = window.debug_bounds("trajectory-details-sidebar").unwrap();
        let close = window.debug_bounds("trajectory-close-detail").unwrap();
        assert!(close.right() <= sidebar.right());
        assert!(
            window
                .debug_bounds("trajectory-record-list")
                .unwrap()
                .size
                .width
                > px(100.)
        );
        window.simulate_resize(size(px(1440.), px(1000.)));
        window.run_until_parked();
        let close = window.debug_bounds("trajectory-close-detail").unwrap();
        window.simulate_click(close.center(), gpui::Modifiers::default());
        window.run_until_parked();
        assert!(window.debug_bounds("trajectory-details-sidebar").is_none());
        assert!(window.debug_bounds("trajectory-raw-detail").is_none());
        assert_eq!(
            window
                .debug_bounds("trajectory-record-list")
                .unwrap()
                .size
                .width,
            list_bounds.size.width
        );
        let tabs = window.debug_bounds("session-view-tabs").unwrap();
        window.simulate_click(
            point(tabs.left() + px(24.), tabs.center().y),
            gpui::Modifiers::default(),
        );
        window.run_until_parked();
        assert!(window.debug_bounds("session-trajectory").is_none());
        assert!(window.debug_bounds("agent-chat-conversation").is_some());
        assert!(window.debug_bounds("agent-composer-panel").is_some());
        host.read_with(window, |h, cx| {
            assert_eq!(h.history.active, id);
            assert_eq!(h.composer.read(cx).value(cx).as_ref(), "保留草稿");
        });
    }
}
