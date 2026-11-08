use super::*;
use std::hash::{Hash, Hasher};

pub struct MarkdownView {
    source: SharedString,
    theme: ThemePreset,
    editor: Entity<gpui_glint::components::DocumentEditor>,
    pub(super) root: Option<PathBuf>,
    pub(super) paths: std::collections::HashSet<String>,
    last_used: std::time::Instant,
    pub(super) links_source: Option<SharedString>,
    render_source: SharedString,
    pub(super) links_revision: u64,
    pub(super) links_task: Option<Task<()>>,
    _link_subscription: Subscription,
}

pub struct ToolPanel {
    output_panel: Option<Entity<gpui_glint::components::CollapsePanel>>,
    output_scroll: ScrollHandle,
    signature: String,
    theme: ThemePreset,
    panel: Entity<gpui_glint::components::CollapsePanel>,
}

pub struct ProcessPanel {
    signature: u64,
    completed_source: Option<(usize, u64)>,
    theme: ThemePreset,
    complete: bool,
    panel: Entity<gpui_glint::components::CollapsePanel>,
}

pub struct Question {
    pub request: crate::ask_question::Request,
    pub input: Entity<TextEntry>,
}

pub struct Approval {
    pub agent_id: Option<Uuid>,
    pub call: tools::ToolCall,
    pub respond: tokio::sync::oneshot::Sender<bool>,
}

struct ToolPresentation {
    icon: Icon,
    summary: String,
    body: String,
}

fn tool_signature(record: &tools::ToolRecord) -> String {
    let mut hash = std::hash::DefaultHasher::new();
    record.call.name.hash(&mut hash);
    record.call.arguments.hash(&mut hash);
    record.result.hash(&mut hash);
    record.success.hash(&mut hash);
    record.elapsed_ms.hash(&mut hash);
    record.live_output.hash(&mut hash);
    for child in record.nested_tools.iter() {
        child.call.id.hash(&mut hash);
        child.success.hash(&mut hash);
    }
    record
        .change
        .as_ref()
        .map(|change| change.id)
        .hash(&mut hash);
    hash.finish().to_string()
}

fn process_signature(
    blocks: &[generation::ContentBlock],
    thinking_ms: &std::collections::BTreeMap<usize, u64>,
    summary: &str,
    activity: &[tools::ToolRecord],
) -> u64 {
    use generation::ContentBlock;
    let mut hash = std::hash::DefaultHasher::new();
    summary.hash(&mut hash);
    thinking_ms.hash(&mut hash);
    for block in blocks {
        std::mem::discriminant(block).hash(&mut hash);
        match block {
            ContentBlock::Text(text) | ContentBlock::Thinking(text) => text.hash(&mut hash),
            ContentBlock::ToolCall(call) => {
                call.id.hash(&mut hash);
                call.name.hash(&mut hash);
                call.arguments.hash(&mut hash);
            }
            ContentBlock::ToolResult(record) => tool_signature(record).hash(&mut hash),
        }
    }
    for record in activity {
        record.call.id.hash(&mut hash);
        tool_signature(record).hash(&mut hash);
    }
    hash.finish()
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

fn unresolved_tool_record(call: &tools::ToolCall, complete: bool) -> tools::ToolRecord {
    tools::rejected(
        call.clone(),
        if complete {
            "调用已中断：此回复未保存该工具的执行结果，请先核对实际状态。"
        } else {
            "正在生成参数…"
        }
        .into(),
    )
}

fn tool_presentation(record: &tools::ToolRecord) -> ToolPresentation {
    tool_presentation_with_body(record, true)
}

pub(super) fn tool_summary(
    call: &tools::ToolCall,
    record: Option<&tools::ToolRecord>,
) -> (Icon, String) {
    let pending;
    let record = if let Some(record) = record {
        record
    } else {
        pending = unresolved_tool_record(call, false);
        &pending
    };
    let presentation = tool_presentation_with_body(record, false);
    (presentation.icon, presentation.summary)
}

fn tool_presentation_with_body(record: &tools::ToolRecord, include_body: bool) -> ToolPresentation {
    if record.result == "正在生成参数…"
        || record.result.starts_with("未执行：")
        || record.result.starts_with("调用已中断：")
    {
        let generating = record.result == "正在生成参数…";
        return ToolPresentation {
            icon: if record.call.name == "run_command" {
                Icon::Shell
            } else {
                Icon::Workflow
            },
            summary: format!(
                "{} {}",
                if generating {
                    "正在生成参数"
                } else if record.result.starts_with("未执行：") {
                    "未执行"
                } else {
                    "调用已中断"
                },
                record.call.name
            ),
            body: if !include_body {
                String::new()
            } else if generating {
                "正在接收工具参数，尚未执行。".into()
            } else {
                record.result.clone()
            },
        };
    }
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
        name if name.starts_with("glint_") => (
            Icon::ApiDoc,
            if running {
                "正在操作 API 工作区"
            } else if success {
                "API 工具已完成"
            } else {
                "API 工具执行失败"
            },
            name.trim_start_matches("glint_").into(),
        ),
        "todo_read" | "todo_write" => (
            Icon::Workflow,
            if running {
                "正在更新任务清单"
            } else if success {
                "任务清单"
            } else {
                "任务清单更新失败"
            },
            if record.call.name == "todo_read" {
                "读取进度".into()
            } else {
                format!("{} 项", args["todos"].as_array().map_or(0, Vec::len))
            },
        ),
        "subagent" => (
            Icon::Workflow,
            if running {
                "子代理执行中"
            } else if success {
                "子代理已完成"
            } else {
                "子代理失败"
            },
            args["task"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(80)
                .collect(),
        ),
        "code_mode" => (
            Icon::Shell,
            if running {
                "正在编排工具"
            } else if success {
                "已完成工具编排"
            } else {
                "工具编排未完成"
            },
            format!("{} 个调用", record.nested_tools.len()),
        ),
        "run_command" => (
            Icon::Shell,
            if running {
                "正在运行"
            } else if serde_json::from_str::<serde_json::Value>(&record.result)
                .ok()
                .is_some_and(|v| v["status"] == "running")
            {
                "已启动后台命令"
            } else if success {
                "已运行"
            } else if serde_json::from_str::<serde_json::Value>(&record.result).ok()
                .is_some_and(|value| value["status"] == "cancelled") {
                "命令已终止"
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
        "command_poll" | "command_stop" => (
            Icon::Shell,
            if running {
                "正在查询命令"
            } else if success {
                "已查询命令状态"
            } else {
                "查询命令失败"
            },
            args["run_id"].as_str().unwrap_or("").into(),
        ),
        "session_changes" => (
            Icon::Pencil,
            if running {
                "正在查询会话变更"
            } else if success {
                "已查询会话变更"
            } else {
                "查询变更失败"
            },
            args["path"].as_str().unwrap_or("当前会话").into(),
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
    let body = if !include_body {
        String::new()
    } else if running {
        record
            .live_output
            .clone()
            .filter(|output| !output.is_empty())
            .unwrap_or_else(|| "等待工具返回结果…".into())
    } else if record.call.name == "run_command" && record.result.trim().is_empty() {
        "命令已结束，无输出。".into()
    } else {
        if record.call.name == "read_file" {
            serde_json::from_str::<serde_json::Value>(&record.result)
                .ok()
                .and_then(|v| v["content"].as_str().map(str::to_owned))
                .unwrap_or_else(|| record.result.clone())
        } else if record.call.name == "code_mode" {
            serde_json::from_str::<serde_json::Value>(&record.result)
                .ok()
                .map(|v| {
                    if v["status"] == "completed" {
                        serde_json::to_string_pretty(&v["value"]).unwrap_or_default()
                    } else {
                        v["error"].as_str().unwrap_or("工具编排未完成").into()
                    }
                })
                .unwrap_or_else(|| record.result.clone())
        } else if matches!(
            record.call.name.as_str(),
            "run_command" | "command_poll" | "command_stop"
        ) {
            serde_json::from_str::<serde_json::Value>(&record.result)
                .ok()
                .map(|v| {
                    let mut parts = Vec::new();
                    if let Some(page) = v["page"]["content"].as_str() {
                        parts.push(page.to_owned());
                    } else {
                        for key in ["stdout", "stderr"] {
                            if let Some(text) = v[key].as_str().filter(|s| !s.is_empty()) {
                                parts.push(text.into());
                            }
                        }
                    }
                    if let Some(code) = v["exit_code"].as_i64().filter(|n| *n != 0) {
                        parts.push(format!("退出码：{code}"));
                    }
                    if v["status"] == "running" {
                        parts.push("后台命令已启动，可查询进度或停止。".into());
                    }
                    if v["status"] == "timed_out" {
                        parts.push("命令超时，已停止本地进程组。".into());
                    }
                    if v["status"] == "cancelled" {
                        parts.push("命令已取消。".into());
                    }
                    for key in ["error", "remote_state"] {
                        if let Some(text) = v[key].as_str() {
                            parts.push(text.into());
                        }
                    }
                    if parts.is_empty() {
                        "命令已结束，无输出。".into()
                    } else {
                        parts.join("\n")
                    }
                })
                .unwrap_or_else(|| record.result.clone())
        } else {
            record.result.clone()
        }
    };
    ToolPresentation {
        icon,
        summary,
        body,
    }
}

/// Keep lifecycle/approval events; superseded full snapshots need no UI work.
fn coalesce_tool_snapshots(events: Vec<tools::ToolEvent>) -> Vec<tools::ToolEvent> {
    let mut seen_transcript = false;
    let mut seen_outputs = std::collections::HashSet::new();
    let mut seen_batches = std::collections::HashSet::new();
    let mut latest: Vec<_> = events
        .into_iter()
        .rev()
        .filter(|event| match event {
            tools::ToolEvent::Transcript(_) => !std::mem::replace(&mut seen_transcript, true),
            tools::ToolEvent::BatchProgress { call_id, .. } => seen_batches.insert(call_id.clone()),
            tools::ToolEvent::Output { call_id, .. } => seen_outputs.insert(call_id.clone()),
            _ => true,
        })
        .collect();
    latest.reverse();
    latest
}

impl AgentWorkspace {
    pub fn watch_tools(
        &mut self,
        id: Uuid,
        receiver: async_channel::Receiver<tools::ToolEvent>,
        cx: &mut Context<Self>,
    ) {
        let run = self.run_mut(id);
        run.tool_activity.clear();
        run.transcript.clear();
        run.thinking_timings = Default::default();
        run.tool_listener = Some(cx.spawn(async move |weak, cx| {
            while let Ok(event) = receiver.recv().await {
                let mut batch = vec![event];
                for _ in 0..255 {
                    match receiver.try_recv() {
                        Ok(event) => batch.push(event),
                        Err(_) => break,
                    }
                }
                for event in coalesce_tool_snapshots(batch) {
                    if weak
                        .update(cx, |this, cx| {
                            if !this.session_busy(id) {
                                return;
                            }
                            // Follow on the next layout only if the reader is still
                            // at the bottom of the previous layout. Scrolling back
                            // through history detaches until they return to the end.
                            if !matches!(
                                &event,
                                tools::ToolEvent::ContextUsage { .. }
                                    | tools::ToolEvent::SubagentUpdated
                            ) && this.subagent_ui.selected.is_none()
                                && id == this.history.active
                            {
                                scroll_follow::follow_if_at_bottom(&this.scroll);
                            }
                            match event {
                                tools::ToolEvent::SteeringFailed(error) => {
                                    if this.history.active == id {
                                        this.provider_error = Some(format!(
                                            "引导未生效：{error}。补充要求已保存到会话。"
                                        ));
                                    }
                                }
                                tools::ToolEvent::TodoUpdated(todos) => {
                                    if let Some(session) = this
                                        .history
                                        .sessions
                                        .iter_mut()
                                        .find(|session| session.id == id)
                                    {
                                        session.todos = todos;
                                    }
                                    this.sync_todos(cx);
                                    this.save(cx);
                                }
                                tools::ToolEvent::SubagentUpdated => {
                                    let run = this.run_mut(id);
                                    if run.approval
                                        .as_ref()
                                        .and_then(|approval| approval.agent_id)
                                        .is_some_and(|agent_id| {
                                            run.subagents.lock().ok().is_some_and(|traces| {
                                                traces.iter().any(|trace| {
                                                    trace.id == agent_id && !trace.status.active()
                                                })
                                            })
                                        })
                                    {
                                        run.approval = None;
                                    }
                                    if id == this.history.active && this
                                        .selected_subagent()
                                        .is_some_and(|trace| trace.status.active())
                                    {
                                        if let Some(scroll) = this.selected_subagent_scroll() {
                                            scroll_follow::follow_if_at_bottom(&scroll);
                                        }
                                    }
                                }
                                tools::ToolEvent::BatchProgress { call_id, tools } => {
                                    let previous = this.run(id).tool_activity
                                        .iter()
                                        .find(|r| r.call.id == call_id)
                                        .map_or(0, |r| r.nested_tools.len());
                                    for child in tools.iter().skip(previous) {
                                        this.invalidate_preview_paths(id, child);
                                    }
                                    if let Some(record) =
                                        this.run_mut(id).tool_activity.iter_mut().find(|r| r.call.id == call_id)
                                    {
                                        record.nested_tools = tools.into();
                                    }
                                }
                                tools::ToolEvent::Question(request) => {
                                    if !request.respond.is_closed() {
                                        let theme = this.theme;
                                        let input = cx.new(|cx| TextEntry::new(cx, "输入回答…", "").theme(theme));
                                        this.run_mut(id).questions.push(Question { request, input });
                                    }
                                }
                                tools::ToolEvent::QuestionResolved(question_id) => {
                                    this.run_mut(id).questions.retain(|q| q.request.id != question_id);
                                }
                                tools::ToolEvent::ApprovalResolved => {
                                    if this.run_mut(id).approval
                                        .as_ref()
                                        .is_some_and(|a| a.respond.is_closed())
                                    {
                                        this.run_mut(id).approval = None;
                                    }
                                }
                                tools::ToolEvent::ContextUsage {
                                    provider_id,
                                    model,
                                    used_tokens,
                                    capacity_tokens,
                                } => {
                                    if let Some(session) = this
                                        .history
                                        .sessions
                                        .iter_mut()
                                        .find(|session| session.id == id)
                                    {
                                        session.context =
                                            Some(context_usage::ContextSnapshot {
                                                provider_id,
                                                model,
                                                used_tokens,
                                                capacity_tokens,
                                            });
                                    }
                                    this.sync_context_usage(cx);
                                }
                                tools::ToolEvent::Transcript(blocks) => {
                                    let run = this.run_mut(id);
                                    run.thinking_timings.observe(&blocks);
                                    run.transcript = blocks;
                                }
                                tools::ToolEvent::Started(call) => {
                                    this.run_mut(id).tool_activity.push(tools::ToolRecord {
                                        call,
                                        result: "执行中…".into(),
                                        success: false,
                                        elapsed_ms: 0,
                                        live_output: None,
                                        nested_tools: Default::default(),
                                        subagent: None,
                                        change: None,
                                    })
                                }
                                tools::ToolEvent::Output { call_id, output } => {
                                    if let Some(record) =
                                        this.run_mut(id).tool_activity.iter_mut().find(|record| {
                                            record.call.id == call_id && record.result == "执行中…"
                                        })
                                    {
                                        record.live_output = Some(output);
                                    }
                                }
                                tools::ToolEvent::Approval {
                                    call,
                                    respond,
                                    agent_id,
                                } => {
                                    if *this.access.lock().unwrap() != tools::AccessMode::Ask {
                                        let _ = respond.send(true);
                                    } else {
                                        this.run_mut(id).approval = Some(Approval {
                                            call,
                                            respond,
                                            agent_id,
                                        });
                                    }
                                }
                                tools::ToolEvent::Completed(record) => {
                                    if record.success
                                        || matches!(
                                            record.call.name.as_str(),
                                            "run_command"
                                                | "command_poll"
                                                | "command_stop"
                                                | "code_mode"
                                        )
                                    {
                                        this.invalidate_preview_paths(id, &record);
                                    }
                                    if record.call.name.starts_with("schedule_") {
                                        this.refresh_calendar(cx);
                                    }

                                    if let Some(existing) = this.run_mut(id).tool_activity
                                        .iter_mut()
                                        .find(|item| item.call.id == record.call.id)
                                    {
                                        *existing = record;
                                    } else {
                                        this.run_mut(id).tool_activity.push(record);
                                    }
                                }
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }));
    }

    pub fn tool_records(
        &self,
        key: &str,
        records: &[tools::ToolRecord],
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div().w_full().min_w_0().flex().flex_col().gap_1().children(
            records
                .iter()
                .map(|record| self.tool_panel(key, record, cx)),
        )
    }

    fn tool_panel(
        &self,
        key: &str,
        record: &tools::ToolRecord,
        cx: &mut Context<Self>,
    ) -> Entity<gpui_glint::components::CollapsePanel> {
        use gpui_glint::components::CollapsePanel;
        let theme = self.theme;
        let p = theme.palette();
        let mut cache = self.tool_panels.borrow_mut();
        let signature = tool_signature(record);
        let entry = cache
            .entry(format!("{key}:tool:{}", record.call.id))
            .or_insert_with(|| {
                let panel = cx.new(|_| {
                    CollapsePanel::new("")
                        .icon(tool_presentation(record).icon)
                        .theme(theme)
                });
                // A tool toggle also changes the layout of the surrounding conversation.
                cx.subscribe(
                    &panel,
                    |_, _, _: &gpui_glint::components::CollapsePanelEvent, cx| cx.notify(),
                )
                .detach();
                ToolPanel {
                    output_panel: None,
                    output_scroll: ScrollHandle::new(),
                    signature: String::new(),
                    theme,
                    panel,
                }
            });
        if entry.signature != signature || entry.theme != theme {
            let presentation = tool_presentation(record);
            let result: SharedString = presentation.body.into();
            let change = record.change.clone();
            let nested = record.nested_tools.clone();
            let script = if record.call.name == "code_mode" {
                record.call.arguments["code"].as_str().map(|source| {
                    let text: SharedString = source.to_owned().into();
                    cx.new(|cx| {
                        gpui_glint::components::CodeCard::new(cx, "javascript", text)
                            .show_header(false)
                            .read_only(true)
                            .viewport()
                            .theme(theme)
                    })
                })
            } else {
                None
            };
            let output_cards = (record.call.name == "code_mode").then(|| {
                code_mode_output_sections(&record.result)
                    .into_iter()
                    .map(|(label, text, language)| {
                        let height =
                            (text.lines().take(10).count().max(1) as f32 * 21. + 16.).min(226.);
                        let card = (!text.is_empty()).then(|| {
                            cx.new(|cx| {
                                gpui_glint::components::CodeCard::new(cx, language, text)
                                    .show_header(false)
                                    .read_only(true)
                                    .viewport()
                                    .theme(theme)
                            })
                        });
                        (label, card, height)
                    })
                    .collect::<Vec<_>>()
            });
            let output_panel = output_cards.map(|cards| {
                let output_scroll = entry.output_scroll.clone();
                let output = entry.output_panel.get_or_insert_with(|| {
                    cx.new(|_| CollapsePanel::new("返回值").compact().theme(theme))
                });
                output.update(cx, |panel, cx| {
                    panel.set_theme(theme, cx);
                    panel.set_content(
                        move |_, _| code_mode_result_view(&cards, theme, &output_scroll).into_any_element(),
                        cx,
                    );
                });
                output.clone()
            });
            let source_height = record.call.arguments["code"]
                .as_str()
                .map(|code| (code.lines().take(10).count().max(1) as f32 * 21. + 16.).min(240.))
                .unwrap_or(64.);
            let weak = cx.entity().downgrade();
            let scope = format!("{key}:tool:{}", record.call.id);
            let preview_root = self.history.active().project.clone();
            let preview_workspace = cx.entity().downgrade();
            entry.panel.update(cx, |panel, cx| {
                panel.set_theme(theme, cx);
                panel.set_summary(presentation.summary.clone(), cx);
                if record.call.name == "run_command" && record.result == "执行中…" {
                    let anchor = self.memory.anchor.clone();
                    let session = self.history.active;
                    let call_id = record.call.id.clone();
                    let summary = presentation.summary.clone();
                    let host = weak.clone();
                    panel.set_summary_content(move |_, _| {
                        let cancelling = crate::command_runner::call_cancelling(&anchor, session, &call_id);
                        let anchor = anchor.clone();
                        let call_id = call_id.clone();
                        let host = host.clone();
                        div().w_full().min_w_0().flex().items_center().gap_2()
                            .child(div().flex_1().min_w_0().truncate().child(summary.clone()))
                            .child(div().id("stop-running-command").debug_selector(|| "stop-running-command".into())
                                .flex_shrink_0().px_2().py_1().rounded(px(5.)).cursor_pointer().text_size(px(11.)).text_color(rgb(p.text_muted))
                                .child(if cancelling { "正在终止…" } else { "终止" })
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    let accepted = crate::command_runner::cancel_call(&anchor, session, &call_id);
                                    let _ = host.update(cx, |this, cx| {
                                        if !accepted {
                                            this.git_ui.toasts.update(cx, |toasts, cx| {
                                                toasts.push(gpui_glint::components::Toast::new("命令尚未启动或已经结束")
                                                    .description("若命令刚开始，请稍后重试。"), cx);
                                            });
                                        }
                                        cx.notify();
                                    });
                                }))
                            .into_any_element()
                    }, cx);
                }
                if let Some(change) = &record.change {
                    let path = change.path.clone();
                    let root = preview_root.clone();
                    let workspace = preview_workspace.clone();
                    let label = record.call.arguments["path"]
                        .as_str()
                        .unwrap_or(&change.label)
                        .to_owned();
                    let summary = presentation.summary.clone();
                    if let Some(index) = summary.find(&label) {
                        let prefix = summary[..index].to_owned();
                        let suffix = summary[index + label.len()..].to_owned();
                        panel.set_summary_content(
                            move |_, _cx| {
                                let path = path.clone();
                                let root = root.clone();
                                let workspace = workspace.clone();
                                div()
                                    .flex()
                                    .items_center()
                                    .min_w_0()
                                    .child(prefix.clone())
                                    .child(
                                        div()
                                            .id("tool-file-preview-link")
                                            .debug_selector(|| "tool-file-preview-link".into())
                                            .min_w_0()
                                            .truncate()
                                            .cursor_pointer()
                                            .hover(|style| {
                                                style.text_color(rgb(p.accent)).underline()
                                            })
                                            .child(label.clone())
                                            .on_click(move |_, _, cx| {
                                                cx.stop_propagation();
                                                let _ = workspace.update(cx, |this, cx| {
                                                    this.preview_changed_file(
                                                        &path,
                                                        root.clone(),
                                                        cx,
                                                    )
                                                });
                                            }),
                                    )
                                    .child(suffix.clone())
                                    .into_any_element()
                            },
                            cx,
                        );
                    }
                }
                panel.set_content(
                    move |_, cx| {
                        if script.is_some() || output_panel.is_some() || !nested.is_empty() {
                            return weak
                                .update(cx, |this, cx| {
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap_3()
                                        .when_some(script.clone(), |body, card| {
                                            body.child(
                                                div()
                                                    .id("code-mode-source")
                                                    .debug_selector(|| "code-mode-source".into())
                                                    .h(px(source_height))
                                                    .flex_shrink_0()
                                                    .overflow_hidden()
                                                    .rounded(px(8.))
                                                    .child(card),
                                            )
                                        })
                                        .child(if let Some(output_panel) = &output_panel {
                                            div()
                                                .debug_selector(|| "code-mode-result-toggle".into())
                                                .child(output_panel.clone())
                                                .into_any_element()
                                        } else {
                                            div()
                                                .text_size(px(12.))
                                                .child(result.clone())
                                                .into_any_element()
                                        })
                                        .children(
                                            nested
                                                .iter()
                                                .map(|record| this.tool_panel(&scope, record, cx)),
                                        )
                                        .into_any_element()
                                })
                                .unwrap_or_else(|_| div().into_any_element());
                        }
                        if let Some(change) = &change {
                            return weak
                                .update(cx, |this, cx| {
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .child(
                                            div()
                                                .id("tool-edit-diff")
                                                .max_h(px(360.))
                                                .overflow_y_scroll()
                                                .child(this.edit_diff(&scope, change, cx)),
                                        )
                                        .into_any_element()
                                })
                                .unwrap_or_else(|_| div().into_any_element());
                        }
                        div()
                            .id("tool-output")
                            .w_full()
                            .min_w_0()
                            .flex_shrink_0()
                            .max_h(px(320.))
                            .overflow_y_scroll()
                            .text_size(px(12.))
                            .line_height(px(20.))
                            .font_family("monospace")
                            .text_color(rgb(p.text_muted))
                            .child(div().w_full().min_w_0().child(result.clone()))
                            .into_any_element()
                    },
                    cx,
                );
            });
            entry.signature = signature;
            entry.theme = theme;
        }
        entry.panel.clone()
    }

    pub fn markdown_text(
        &self,
        key: String,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Entity<gpui_glint::components::DocumentEditor> {
        let scrollable = key.ends_with(":thinking");
        self.markdown_preview(key, text, scrollable, cx)
    }

    pub fn scrollable_markdown_text(
        &self,
        key: String,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Entity<gpui_glint::components::DocumentEditor> {
        self.markdown_preview(key, text, true, cx)
    }

    fn markdown_preview(
        &self,
        key: String,
        text: &str,
        scrollable: bool,
        cx: &mut Context<Self>,
    ) -> Entity<gpui_glint::components::DocumentEditor> {
        let mut cache = self.markdown_views.borrow_mut();
        let root = self.history.active().project.clone();
        if cache.get(&key).is_some_and(|view| view.root != root) {
            cache.remove(&key);
        }
        let theme = self.theme;
        let prune = cache.get(&key).is_none_or(|v| v.source.as_ref() != text);
        let view = cache.entry(key.clone()).or_insert_with(|| {
            let editor = cx.new(|cx| {
                let editor = gpui_glint::components::DocumentEditor::new(cx, text)
                    .theme(theme)
                    .activate_preview_links(true);
                if scrollable {
                    editor.scrollable_preview()
                } else {
                    editor.embedded_preview()
                }
            });
            let origin = root.clone();
            let subscription = cx.subscribe(
                &editor,
                move |this, _, event: &gpui_glint::components::DocumentEvent, cx| {
                    if let gpui_glint::components::DocumentEvent::LinkActivated(link) = event {
                        this.preview_link(link, &origin, cx);
                    }
                },
            );
            let source = SharedString::from(text.to_owned());
            MarkdownView {
                source: source.clone(),
                theme,
                editor,
                root: root.clone(),
                paths: Default::default(),
                last_used: std::time::Instant::now(),
                links_source: None,
                render_source: source,
                links_revision: 0,
                links_task: None,
                _link_subscription: subscription,
            }
        });
        view.last_used = std::time::Instant::now();
        if view.source.as_ref() != text || view.theme != theme {
            view.editor.update(cx, |editor, cx| {
                editor.set_theme(theme, cx);
                if view.source.as_ref() != text {
                    editor.set_markdown(text, cx);
                }
            });
            if view.source.as_ref() != text {
                view.source = SharedString::from(text.to_owned());
                view.render_source = view.source.clone();
            }
            view.theme = theme;
        }
        if view.links_source.as_deref() != Some(text) {
            view.links_source = Some(view.source.clone());
            view.links_revision += 1;
            let revision = view.links_revision;
            let source = view.source.clone();
            let paths = self.file_preview.paths.clone();
            let task_key = key.clone();
            view.links_task = Some(cx.spawn(async move |weak, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(250))
                    .await;
                let work_source = source.to_string();
                let work_root = root.clone();
                let work = cx.background_executor().spawn(async move {
                    if work_root.as_deref().is_some_and(ssh_project::is_remote) {
                        provider::Job::start(file_preview::decorate_cached(
                            work_source,
                            work_root,
                            paths,
                        ))?
                        .result()
                        .await
                    } else {
                        file_preview::decorate_cached(work_source, work_root, paths).await
                    }
                });
                let result = work.await;
                let _ = weak.update(cx, |this, cx| {
                    let mut cache = this.markdown_views.borrow_mut();
                    if let Some(view) = cache.get_mut(&task_key) {
                        if view.links_revision != revision
                            || view.source != source
                            || view.root != root
                        {
                            return;
                        }
                        if let Some(task) = view.links_task.take() {
                            task.detach();
                        }
                        if let Ok(result) = result {
                            view.paths = result.paths;
                            let markdown = result.markdown;
                            if view.render_source.as_ref() != markdown {
                                view.editor
                                    .update(cx, |editor, cx| editor.set_markdown(&markdown, cx));
                                view.render_source = markdown.into();
                            }
                        }
                    }
                });
            }));
        }
        let editor = view.editor.clone();
        if prune {
            let mut bytes: usize = cache
                .values()
                .map(|v| v.source.len() + v.render_source.len())
                .sum();
            while cache.len() > 256 || bytes > 16 * 1024 * 1024 && cache.len() > 1 {
                let oldest = cache
                    .iter()
                    .filter(|(k, _)| *k != &key)
                    .min_by_key(|(_, v)| v.last_used)
                    .map(|(k, _)| k.clone());
                let Some(oldest) = oldest else {
                    break;
                };
                if let Some(old) = cache.remove(&oldest) {
                    bytes = bytes.saturating_sub(old.source.len() + old.render_source.len());
                }
            }
        }
        editor
    }

    pub fn transcript_view(
        &self,
        key: &str,
        blocks: &[generation::ContentBlock],
        thinking_ms: &std::collections::BTreeMap<usize, u64>,
        complete: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .children(self.transcript_rows(
                key,
                blocks,
                thinking_ms,
                complete,
                if complete { &[] } else { &self.active_run().tool_activity },
                cx,
            ))
    }

    pub fn transcript_rows(
        &self,
        key: &str,
        blocks: &[generation::ContentBlock],
        thinking_ms: &std::collections::BTreeMap<usize, u64>,
        complete: bool,
        activity: &[tools::ToolRecord],
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyView> {
        use generation::ContentBlock;
        use gpui_glint::components::CollapsePanel;
        let theme = self.theme;
        let mut rows = Vec::with_capacity(blocks.len());
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
                    rows.push(
                        self.markdown_text(format!("{key}:{index}"), text, cx)
                            .into(),
                    );
                }
                ContentBlock::Thinking(text) if text.trim().is_empty() => {}
                ContentBlock::Thinking(text) => {
                    let active = !complete && index + 1 == blocks.len();
                    let summary = thinking_summary(text, active, thinking_ms.get(&index).copied());
                    let signature = format!("{active}:{summary}:{text}");
                    let mut cache = self.tool_panels.borrow_mut();
                    let entry = cache
                        .entry(format!("{key}:{index}:thinking"))
                        .or_insert_with(|| ToolPanel {
                            output_panel: None,
                            output_scroll: ScrollHandle::new(),
                            signature: String::new(),
                            theme,
                            panel: cx
                                .new(|_| CollapsePanel::new("思考").icon(Icon::Pulse).theme(theme)),
                        });
                    if entry.signature != signature || entry.theme != theme {
                        // A collapsed thinking row needs only its summary. Do not
                        // parse its Markdown or allocate editors until expanded.
                        let weak = cx.entity().downgrade();
                        let editor_key = format!("{key}:{index}:thinking");
                        let source: SharedString = text.clone().into();
                        let height =
                            ((text.lines().count() + text.len() / 100).clamp(2, 15) as f32 * 24.
                                + 16.)
                                .min(376.);
                        entry.panel.update(cx, |panel, cx| {
                            panel.set_theme(theme, cx);
                            if active {
                                panel.set_streaming_summary(summary, cx);
                            } else {
                                panel.set_summary(summary, cx);
                            }
                            panel.set_content(
                                move |_, cx| {
                                    weak.update(cx, |this, cx| {
                                        div().w_full().min_w_0().h(px(height)).child(
                                            this.markdown_text(editor_key.clone(), &source, cx),
                                        )
                                    })
                                    .unwrap_or_else(|_| div())
                                    .into_any_element()
                                },
                                cx,
                            );
                        });
                        entry.signature = signature;
                        entry.theme = theme;
                    }
                    rows.push(entry.panel.clone().into());
                }
                ContentBlock::ToolCall(call) => {
                    // Completed calls use their result row and the same panel
                    // identity, so streaming never leaves a duplicate call row.
                    if !completed.contains(call.id.as_str()) {
                        let running = unresolved_tool_record(call, complete);
                        let running = if !complete {
                            activity
                                .iter()
                                .find(|record| record.call.id == call.id)
                                .unwrap_or(&running)
                        } else {
                            &running
                        };
                        if let Some(trace) = self.trace_for_record(running) {
                            rows.push(self.subagent_summary(&trace, cx));
                        } else {
                            rows.push(self.tool_panel(key, running, cx).into());
                        }
                    }
                }
                ContentBlock::ToolResult(record) => {
                    if let Some(trace) = self.trace_for_record(record) {
                        rows.push(self.subagent_summary(&trace, cx));
                    } else {
                        rows.push(self.tool_panel(key, record, cx).into());
                    }
                }
            }
        }
        rows
    }

    pub fn response_view(
        &self,
        key: &str,
        blocks: &[generation::ContentBlock],
        stats: Option<&generation::GenerationStats>,
        thinking_ms: &std::collections::BTreeMap<usize, u64>,
        complete: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        // Stored replies are immutable. Only live generation needs to scan and
        // hash tool payloads again; theme/subagent updates still invalidate UI.
        let cached_source = if complete {
            self.process_panels
                .borrow()
                .get(key)
                .and_then(|entry| entry.completed_source)
        } else {
            None
        };
        let split = cached_source
            .map(|(split, _)| split)
            .unwrap_or_else(|| process_prefix_len(blocks, complete));
        if split == 0 {
            return self.transcript_view(key, blocks, thinking_ms, complete, cx);
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
        let source_signature = cached_source
            .map(|(_, signature)| signature)
            .unwrap_or_else(|| {
                process_signature(
                    &blocks[..split],
                    thinking_ms,
                    &summary,
                    if complete { &[] } else { &self.active_run().tool_activity },
                )
            });
        let signature = source_signature ^ self.subagent_revision();
        let entry = cache.entry(key.into()).or_insert_with(|| ProcessPanel {
            signature: 0,
            completed_source: None,
            theme,
            complete,
            panel: cx.new(|_| {
                gpui_glint::components::CollapsePanel::new("")
                    .compact()
                    .expanded(!complete)
                    .theme(theme)
            }),
        });
        entry.completed_source = complete.then_some((split, source_signature));
        if entry.signature != signature || entry.theme != theme || entry.complete != complete {
            // Build child entities once on expansion, then reuse them while the
            // source is unchanged. Scrolling must not hash every tool result or
            // repeat Markdown/cache lookups; collapsed history stays lazy.
            let rows = std::cell::RefCell::new(None::<Entity<crate::history_rows::HistoryRows>>);
            let weak = cx.entity().downgrade();
            let process = blocks[..split].to_vec();
            let thinking_ms = thinking_ms.clone();
            let activity = if complete {
                Vec::new()
            } else {
                self.active_run().tool_activity.clone()
            };
            let content_key = key.to_owned();
            entry.panel.update(cx, |panel, cx| {
                panel.set_theme(theme, cx);
                panel.set_summary(summary, cx);
                if complete && !entry.complete {
                    panel.set_expanded(false, cx);
                }
                panel.set_content(
                    move |_, cx| {
                        let scroll = weak.update(cx, |this, _| this.scroll.clone()).unwrap_or_default();
                        let mut rows = rows.borrow_mut();
                        let rows = rows.get_or_insert_with(|| {
                            let children = weak.update(cx, |this, cx| {
                                this.transcript_rows(
                                    &content_key,
                                    &process,
                                    &thinking_ms,
                                    complete,
                                    &activity,
                                    cx,
                                )
                            }).unwrap_or_default();
                            cx.new(|_| crate::history_rows::HistoryRows::new(children, scroll.clone()))
                        });
                        rows.update(cx, |rows, _| rows.set_scroll(scroll));
                        div().w_full().min_w_0().child(rows.clone()).into_any_element()
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
        for (index, block) in blocks[split..]
            .iter()
            .enumerate()
            .map(|(index, block)| (index + split, block))
        {
            if let generation::ContentBlock::Text(text) = block {
                body = body.child(self.markdown_text(format!("{key}:{index}"), text, cx));
            }
        }
        body
    }

    fn answer_question(&mut self, session: Uuid, id: Uuid, answer: Option<String>, cx: &mut Context<Self>) {
        let questions = &mut self.run_mut(session).questions;
        if let Some(index) = questions.iter().position(|q| q.request.id == id) {
            let question = questions.remove(index);
            let _ = question.request.respond.send(answer);
        }
        cx.notify();
    }

    pub fn question_panel(&self, cx: &mut Context<Self>) -> gpui::Div {
        let session = self.history.active;
        let mut panel = div().flex().flex_col().gap_2();
        for question in self.active_run().questions.iter().filter(|q| !q.request.respond.is_closed()) {
            let id = question.request.id;
            let input = question.input.clone();
            let source = question.request.agent_id.and_then(|id| {
                self.active_run().subagents.lock().ok()?.iter().find(|trace| trace.id == id).map(|trace| format!("子 Agent「{}」提问", trace.title))
            }).unwrap_or_else(|| "Agent 提问".into());
            let mut choices = div().flex().flex_wrap().gap_2();
            for option in &question.request.options {
                let answer = option.clone();
                choices = choices.child(Button::new(option.clone()).theme(self.theme).render()
                    .on_click(cx.listener(move |this, _, _, cx| this.answer_question(session, id, Some(answer.clone()), cx))));
            }
            panel = panel.child(div().id(SharedString::from(format!("agent-question-{id}"))).p_3().rounded_lg().bg(rgb(self.theme.palette().surface_subtle))
                .flex().flex_col().gap_2()
                .child(div().text_size(px(11.)).child(source))
                .child(question.request.question.clone())
                .child(choices)
                .child(input.clone())
                .child(div().flex().gap_2()
                    .child(Button::new("提交回答").theme(self.theme).render().on_click(cx.listener(move |this, _, _, cx| {
                        let answer = input.read(cx).value().trim().to_owned();
                        if !answer.is_empty() { this.answer_question(session, id, Some(answer), cx); }
                    })))
                    .child(Button::new("跳过").kind(ButtonKind::Glass).theme(self.theme).render()
                        .on_click(cx.listener(move |this, _, _, cx| this.answer_question(session, id, None, cx))))));
        }
        panel
    }

    pub fn approval_panel(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let mut panel = div();
        let session = self.history.active;
        if let Some(approval) = &self.active_run().approval {
            let source = approval
                .agent_id
                .and_then(|id| {
                    self.active_run().subagents
                        .lock()
                        .ok()?
                        .iter()
                        .find(|trace| trace.id == id)
                        .cloned()
                })
                .map(|trace| format!("子代理「{}」 · ", trace.title))
                .unwrap_or_default();
            panel = panel.w_full().max_w(px(800.)).p_4().flex().flex_col().gap_3().rounded(px(10.)).border_1().border_color(rgb(p.accent)).bg(rgb(p.panel))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("{source}确认工具调用：{}", approval.call.name)))
                .child(div().id("approval-arguments").max_h(px(180.)).overflow_y_scroll().text_size(px(11.)).child(serde_json::to_string_pretty(&approval.call.arguments).unwrap_or_default()))
                .when(approval.call.name == "run_command", |panel| panel.child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child("命令在项目目录运行，批准前请检查程序和参数；命令自身可以访问项目外部。")))
                .child(div().flex().gap_3()
                    .child(Button::new("允许本次").theme(self.theme).render().on_click(cx.listener(move |this, _, _, cx| { if let Some(approval) = this.runs.get_mut(&session).and_then(|run| run.approval.take()) { let _=approval.respond.send(true); } cx.notify(); })))
                    .child(Button::new("拒绝").kind(ButtonKind::Glass).theme(self.theme).render().on_click(cx.listener(move |this, _, _, cx| { if let Some(approval) = this.runs.get_mut(&session).and_then(|run| run.approval.take()) { let _=approval.respond.send(false); } cx.notify(); }))));
        }
        panel
    }
}

type OutputCard = (
    Option<String>,
    Option<Entity<gpui_glint::components::CodeCard>>,
    f32,
);

fn code_mode_result_view(cards: &[OutputCard], theme: ThemePreset, scroll: &ScrollHandle) -> gpui::Stateful<gpui::Div> {
    let p = theme.palette();
    let scroll = scroll.clone();
    div()
        .id("code-mode-result")
        .debug_selector(|| "code-mode-result".into())
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .max_h(px(320.))
        .relative()
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .child(div().flex().flex_col().gap_3().children(cards.iter().map(
            |(label, card, height)| {
                div()
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(p.border))
                    .overflow_hidden()
                    .when_some(label.clone(), |section, label| {
                        section.child(
                            div()
                                .px_3()
                                .py_2()
                                .text_size(px(11.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(p.text_muted))
                                .child(label),
                        )
                    })
                    .when_some(card.clone(), |section, card| {
                        section.child(
                            div()
                                .h(px(*height))
                                .flex_shrink_0()
                                .overflow_hidden()
                                .child(card),
                        )
                    })
                    .when(card.is_none(), |section| {
                        section.child(
                            div()
                                .px_3()
                                .pb_2()
                                .text_size(px(12.))
                                .text_color(rgb(p.text_muted))
                                .child("无输出"),
                        )
                    })
            },
        )))
        .child(gpui::canvas(
            |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
            move |_, hitbox: gpui::Hitbox, window, _| {
                let view = window.current_view();
                window.on_mouse_event(move |event: &gpui::ScrollWheelEvent, phase, window, cx| {
                    if phase != gpui::DispatchPhase::Capture || !hitbox.should_handle_scroll(window) { return; }
                    let delta = event.delta.pixel_delta(px(21.));
                    let offset = scroll.offset();
                    let next = (offset.y+delta.y).clamp(-scroll.max_offset().y, px(0.));
                    if next != offset.y {
                        scroll.set_offset(point(offset.x, next));
                        cx.stop_propagation();
                        cx.notify(view);
                    }
                });
            },
        ).absolute().top_0().left_0().size_full())
}

fn code_mode_output_sections(raw: &str) -> Vec<(Option<String>, String, &'static str)> {
    use serde_json::Value;
    let value = match serde_json::from_str::<Value>(raw) {
        Ok(value) if value["status"] == "completed" => value["value"].clone(),
        Ok(value) if value.get("error").is_some() => value["error"].clone(),
        Ok(value) => value,
        Err(_) => Value::String(raw.into()),
    };
    let format_value = |label, value: &Value| match value {
        Value::String(text) => (label, text.clone(), "text"),
        value => (
            label,
            serde_json::to_string_pretty(value).unwrap_or_default(),
            "json",
        ),
    };
    if let Value::Object(fields) = &value
        && !fields.is_empty()
        && fields.len() <= 32
    {
        fields
            .iter()
            .map(|(key, value)| format_value(Some(key.clone()), value))
            .collect()
    } else {
        vec![format_value(Some("输出结果".into()), &value)]
    }
}

fn thinking_summary(text: &str, active: bool, elapsed_ms: Option<u64>) -> String {
    if active {
        text.lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("正在思考…")
            .to_owned()
    } else if let Some(elapsed_ms) = elapsed_ms {
        format!("已思考 {:.1} s", elapsed_ms as f64 / 1000.)
    } else {
        // Old history has no per-block timing. Do not substitute total work time.
        "已思考".into()
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

    #[gpui::test]
    fn question_answers_are_scoped_and_preserve_other_pending_questions(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("providers.json"), r#"{"providers":[],"active":null,"active_model":""}"#).unwrap();
        let host = cx.new(|cx| AgentWorkspace::with_path(dir.path().join("sessions.json"), cx));
        let (respond, mut response) = tokio::sync::oneshot::channel();
        let (other, mut other_response) = tokio::sync::oneshot::channel();
        host.update(cx, |host, cx| {
            let session = host.history.active;
            let id = Uuid::new_v4();
            for (question_id, sender) in [(id, respond), (Uuid::new_v4(), other)] {
                let input = cx.new(|cx| TextEntry::new(cx, "回答", ""));
                host.run_mut(session).questions.push(Question {
                    request: crate::ask_question::Request { id: question_id, agent_id: None, question: "问题".into(), options: vec![], respond: sender }, input,
                });
            }
            host.answer_question(Uuid::new_v4(), id, Some("错误会话".into()), cx);
            assert!(response.try_recv().is_err());
            host.answer_question(session, id, Some("用户回答".into()), cx);
            assert_eq!(response.try_recv().unwrap(), Some("用户回答".into()));
            assert_eq!(host.run(session).questions.len(), 1);
            assert!(other_response.try_recv().is_err());
            let remaining = host.run(session).questions[0].request.id;
            host.answer_question(session, remaining, None, cx);
            assert_eq!(other_response.try_recv().unwrap(), None);
        });
    }

    #[gpui::test]
    fn running_command_stop_button_does_not_toggle_tool_details(cx: &mut gpui::TestAppContext) {
        struct Preview { _host: Entity<AgentWorkspace>, panel: Entity<gpui_glint::components::CollapsePanel> }
        impl Render for Preview {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { div().w(px(600.)).child(self.panel.clone()) }
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("providers.json"), r#"{"providers":[],"active":null,"active_model":""}"#).unwrap();
        let (preview, window) = cx.add_window_view(|_, cx| {
            let host = cx.new(|cx| AgentWorkspace::with_path(dir.path().join("sessions.json"), cx));
            let record = tools::rejected(tools::ToolCall { id: "stop-test".into(), name: "run_command".into(), arguments: serde_json::json!({"program":"rg","args":["search"]}) }, "执行中…".into());
            let panel = host.update(cx, |host, cx| host.tool_panel("stop-test", &record, cx));
            Preview { _host: host, panel }
        });
        let button = window.debug_bounds("stop-running-command").expect("active command stop button");
        window.simulate_click(button.center(), Default::default());
        preview.read_with(window, |preview, cx| assert!(!preview.panel.read(cx).is_expanded()));
    }

    #[gpui::test]
    fn orchestration_output_scroll_chains_at_its_boundary(cx: &mut gpui::TestAppContext) {
        struct Nested { outer: ScrollHandle, inner: ScrollHandle }
        impl Render for Nested {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let cards = (0..12).map(|i| (Some(format!("section {i}")), None, 50.)).collect::<Vec<_>>();
                div().id("outer-output").h(px(400.)).w(px(400.)).track_scroll(&self.outer).overflow_y_scroll()
                    .child(div().flex().flex_col().child(div().h(px(300.)).flex_shrink_0())
                        .child(code_mode_result_view(&cards, ThemePreset::default(), &self.inner))
                        .child(div().h(px(500.)).flex_shrink_0()))
            }
        }
        let (host, window) = cx.add_window_view(|_, _| Nested { outer: ScrollHandle::new(), inner: ScrollHandle::new() });
        host.update(window, |host, cx| { host.outer.set_offset(point(px(0.), px(-250.))); cx.notify(); });
        window.run_until_parked();
        let bounds = window.debug_bounds("code-mode-result").unwrap();
        let position = point(bounds.left()+px(8.), bounds.top()+px(8.));
        let initial = host.read_with(window, |host, _| host.outer.offset().y);
        window.simulate_event(gpui::ScrollWheelEvent { position, delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-80.))), ..Default::default() });
        window.run_until_parked();
        host.read_with(window, |host, _| { assert!(host.inner.offset().y < px(0.)); assert_eq!(host.outer.offset().y, initial); });
        host.update(window, |host, cx| { host.inner.scroll_to_bottom(); cx.notify(); });
        window.run_until_parked();
        window.simulate_event(gpui::ScrollWheelEvent { position, delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-80.))), ..Default::default() });
        window.run_until_parked();
        assert!(host.read_with(window, |host, _| host.outer.offset().y) < initial, "exhausted output must release the outer conversation");
    }

    #[gpui::test]
    fn tool_file_name_opens_preview_without_toggling_panel(cx: &mut gpui::TestAppContext) {
        struct Root {
            host: Entity<AgentWorkspace>,
            panel: Entity<gpui_glint::components::CollapsePanel>,
        }
        impl Render for Root {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().child(self.panel.clone())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let path = dir.path().join("test.md");
        std::fs::write(&path, "# Preview").unwrap();
        let record: tools::ToolRecord = serde_json::from_value(serde_json::json!({
            "call": {"id":"preview", "name":"write_file", "arguments":{"path":"test.md"}},
            "result":"written", "success":true, "elapsed_ms":10,
            "change": {"id":uuid::Uuid::new_v4(), "path":path, "label":"test.md", "before":"", "after":"# Preview", "added":1, "removed":0}
        })).unwrap();
        let (root, window) = cx.add_window_view(|_, cx| {
            let host = cx.new(|cx| AgentWorkspace::with_path(dir.path().join("sessions.json"), cx));
            let panel = host.update(cx, |host, cx| host.tool_panel("test", &record, cx));
            Root { host, panel }
        });
        window.run_until_parked();
        let link = window.debug_bounds("tool-file-preview-link").unwrap();
        window.simulate_click(link.center(), Default::default());
        window.run_until_parked();
        root.read_with(window, |root, cx| {
            assert!(!root.panel.read(cx).is_expanded());
            assert!(root.host.read(cx).file_preview.open);
            assert!(!root.host.read(cx).edits_ui.open);
        });
    }

    #[test]
    fn orchestration_outputs_decode_string_values_without_corrupting_literal_escapes() {
        let raw = serde_json::json!({"status":"completed","value":{
            "library":"170 GB\t/Library\n60 GB\t/Library/Containers",
            "literal":r"keep\nunchanged",
            "nested":{"count":2},
        }})
        .to_string();
        let sections = code_mode_output_sections(&raw);
        let library = sections
            .iter()
            .find(|(key, _, _)| key.as_deref() == Some("library"))
            .unwrap();
        assert_eq!(library.1, "170 GB\t/Library\n60 GB\t/Library/Containers");
        assert_eq!(library.2, "text");
        assert!(
            sections
                .iter()
                .any(|(key, text, _)| key.as_deref() == Some("literal")
                    && text == r"keep\nunchanged")
        );
        let nested = sections
            .iter()
            .find(|(key, _, _)| key.as_deref() == Some("nested"))
            .unwrap();
        assert_eq!(nested.2, "json");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&nested.1).unwrap(),
            serde_json::json!({"count":2})
        );
        for value in [
            serde_json::json!(null),
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!(42),
        ] {
            let sections = code_mode_output_sections(
                &serde_json::json!({"status":"completed","value":value}).to_string(),
            );
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&sections[0].1).unwrap(),
                value
            );
        }
        assert_eq!(code_mode_output_sections("执行中…")[0].1, "执行中…");
        assert_eq!(
            code_mode_output_sections(r#"{"status":"failed","error":"失败原因"}"#)[0].1,
            "失败原因"
        );
    }

    #[gpui::test]
    fn expanded_memory_search_keeps_waiting_below_the_live_response(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(directory.path().join("sessions.json"), cx);
            host.history.active_mut().push(Role::User, "查询记录".into());
            let mut memory = tools::rejected(
                tools::ToolCall {
                    id: "memory-overlap".into(),
                    name: "memory_search".into(),
                    arguments: serde_json::json!({"query": "查询记录"}),
                },
                serde_json::json!({"matches": vec!["很长的记忆结果。".repeat(30); 20]}).to_string(),
            );
            memory.success = true;
            host.run_mut(host.history.active).transcript = vec![generation::ContentBlock::ToolResult(memory)];
            host.run_mut(host.history.active).transcript.extend((0..6).map(|_| Thinking("继续查询".into())));
            host.run_mut(host.history.active).pending = Some((host.history.active, cx.spawn(async move |_, _| std::future::pending::<()>().await)));
            host
        });
        window.simulate_resize(size(px(1200.), px(700.)));
        window.run_until_parked();
        let panel = host.read_with(window, |host, _| {
            host.tool_panels.borrow().iter().find(|(key, _)| key.ends_with(":tool:memory-overlap")).unwrap().1.panel.clone()
        });
        let before = window.debug_bounds("waiting-for-model").unwrap();
        panel.update(window, |panel, cx| panel.set_expanded(true, cx));
        window.run_until_parked();
        let live = window.debug_bounds("live-response").unwrap();
        let waiting = window.debug_bounds("waiting-for-model").unwrap();
        assert!(waiting.top() >= live.bottom(), "{waiting:?} overlaps {live:?}");
        assert!(waiting.top() > before.top(), "waiting row did not move after expansion");
    }

    #[gpui::test]
    fn expanded_orchestration_stays_above_waiting_indicator(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(directory.path().join("sessions.json"), cx);
            host.history.active_mut().push(Role::User, "检查磁盘".into());
            let mut record = tools::rejected(tools::ToolCall {
                id: "overlap-test".into(), name: "code_mode".into(),
                arguments: serde_json::json!({"code":"const result = await tools.run_command({program: 'df', args: ['-h']});\nreturn result;"}),
            }, "长输出内容。".repeat(1000));
            record.result = serde_json::json!({"status":"completed","value":{
                "library":"170 GB\t/Library\n60 GB\t/Library/Containers\n".repeat(40),
                "workspace":"510 GB\t/Workspace\n".repeat(20),
            }}).to_string();
            record.nested_tools = vec![tools::rejected(tools::ToolCall {
                id: "nested-command".into(), name: "run_command".into(), arguments: serde_json::json!({"program":"df","args":["-h"]}),
            }, "输出".into())].into();
            host.run_mut(host.history.active).transcript = vec![generation::ContentBlock::ToolResult(record)];
            for _ in 0..18 {
                host.run_mut(host.history.active).transcript.push(Thinking("继续检查".into()));
            }
            host.run_mut(host.history.active).pending = Some((host.history.active, cx.spawn(async move |_, _| std::future::pending::<()>().await)));
            host
        });
        window.simulate_resize(size(px(1200.), px(700.)));
        window.run_until_parked();
        let panel = host.read_with(window, |host, _| {
            host.tool_panels
                .borrow()
                .iter()
                .find(|(key, _)| key.ends_with(":tool:overlap-test"))
                .unwrap()
                .1
                .panel
                .clone()
        });
        panel.update(window, |panel, cx| panel.set_expanded(true, cx));
        window.run_until_parked();
        assert!(
            window.debug_bounds("code-mode-result").is_none(),
            "return value starts collapsed"
        );
        host.update(window, |host, cx| {
            host.scroll.scroll_to_top_of_item(1);
            cx.notify();
        });
        window.run_until_parked();
        let toggle = window.debug_bounds("code-mode-result-toggle").unwrap();
        window.simulate_click(toggle.center(), gpui::Modifiers::default());
        window.run_until_parked();
        host.update(window, |host, cx| {
            host.scroll.scroll_to_bottom();
            cx.notify();
        });
        window.run_until_parked();
        let live = window.debug_bounds("live-response").unwrap();
        let waiting = window.debug_bounds("waiting-for-model").unwrap();
        let viewport = host.read_with(window, |host, _| host.scroll.bounds());
        let composer = window.debug_bounds("agent-composer-panel").unwrap();
        assert!(waiting.bottom() <= viewport.bottom()+px(1.), "the last row must be reachable at the true scroll bottom: {waiting:?}, {viewport:?}");
        assert!(viewport.bottom() <= composer.top()+px(1.), "the composer must not cover the chat viewport");
        assert!(
            waiting.top() >= live.bottom(),
            "waiting label overlaps expanded live response: {waiting:?}, {live:?}"
        );
        let output = window.debug_bounds("code-mode-result").unwrap();
        let source = window.debug_bounds("code-mode-source").unwrap();
        assert!(
            output.top() >= source.bottom() + px(12.),
            "source and output must have a full gap between them"
        );
        assert!(
            output.size.height > px(40.),
            "output cards must reserve visible height"
        );
        assert!(
            live.bottom() >= output.bottom(),
            "expanded tool output escapes live response bounds"
        );
        panel.update(window, |panel, cx| panel.set_expanded(false, cx));
        window.run_until_parked();
        panel.update(window, |panel, cx| panel.set_expanded(true, cx));
        window.run_until_parked();
        assert!(
            window.debug_bounds("code-mode-result").is_some(),
            "outer toggle preserves return-value expansion"
        );
        let output_panel = host.read_with(window, |host, _| {
            host.tool_panels
                .borrow()
                .iter()
                .find(|(key, _)| key.ends_with(":tool:overlap-test"))
                .unwrap()
                .1
                .output_panel
                .clone()
                .unwrap()
        });
        output_panel.update(window, |panel, cx| panel.set_expanded(false, cx));
        window.run_until_parked();
        assert!(window.debug_bounds("code-mode-result").is_none());
        assert!(window.debug_bounds("code-mode-source").is_some());
    }

    #[gpui::test]
    fn markdown_cache_is_bounded_and_preserves_recent_editors(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        host.update(window, |h, cx| {
            for i in 0..300 {
                h.markdown_preview(format!("bounded-{i}"), "A cached message", false, cx);
            }
            let last = h.markdown_preview("bounded-299".into(), "A cached message", false, cx);
            let cache = h.markdown_views.borrow();
            assert_eq!(cache.len(), 256);
            assert!(!cache.contains_key("bounded-0"));
            assert_eq!(cache["bounded-299"].editor.entity_id(), last.entity_id());
        });
    }
    #[gpui::test]
    fn background_project_streams_and_stops_independently(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let first_project = directory.path().join("first");
        let second_project = directory.path().join("second");
        std::fs::create_dir(&first_project).unwrap();
        std::fs::create_dir(&second_project).unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (first_sender, first_receiver) = async_channel::unbounded();
        let (second_sender, second_receiver) = async_channel::unbounded();
        let (workspace, window) = cx.add_window_view(|_, cx| {
            let mut workspace =
                AgentWorkspace::with_path(directory.path().join("sessions.json"), cx);
            // Keep the GPUI test scheduler independent of disk writes.
            workspace.storage_readable = false;
            workspace
        });
        let (first, second) = workspace.update(window, |workspace, cx| {
            workspace.history.active_mut().project = Some(first_project);
            let first = workspace.history.active;
            workspace.run_mut(first).pending = Some((
                first,
                cx.spawn(async move |_, _| std::future::pending::<()>().await),
            ));
            workspace.watch_tools(first, first_receiver, cx);

            let mut session = model::Session::new();
            session.project = Some(second_project);
            let second = session.id;
            workspace.history.sessions.insert(0, session);
            workspace.switch(second, cx);
            assert!(workspace.session_busy(first));
            assert!(!workspace.session_busy(second));

            workspace.run_mut(second).pending = Some((
                second,
                cx.spawn(async move |_, _| std::future::pending::<()>().await),
            ));
            workspace.watch_tools(second, second_receiver, cx);
            (first, second)
        });

        first_sender.try_send(tools::ToolEvent::Transcript(vec![
            generation::ContentBlock::Text("first".into()),
        ])).unwrap();
        second_sender.try_send(tools::ToolEvent::Transcript(vec![
            generation::ContentBlock::Text("second".into()),
        ])).unwrap();
        window.run_until_parked();
        workspace.update(window, |workspace, cx| {
            assert!(matches!(
                workspace.active_run().transcript.as_slice(),
                [generation::ContentBlock::Text(text)] if text == "second"
            ));
            workspace.stop(cx);
            assert!(!workspace.session_busy(second));
            assert!(workspace.session_busy(first));
            workspace.switch(first, cx);
            assert!(matches!(
                workspace.active_run().transcript.as_slice(),
                [generation::ContentBlock::Text(text)] if text == "first"
            ));
        });
    }

    #[gpui::test]
    fn streaming_follows_the_end_detaches_on_wheel_and_resumes_when_reader_returns(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (sender, receiver) = async_channel::unbounded();
        let (workspace, window) = cx.add_window_view(|_, cx| {
            let mut workspace =
                AgentWorkspace::with_path(directory.path().join("sessions.json"), cx);
            for index in 0..20 {
                workspace.history.active_mut().push(
                    Role::User,
                    format!("历史记录 {index}，阅读时不应被新回复打断。\n\n").repeat(8),
                );
            }
            workspace.run_mut(workspace.history.active).pending = Some((
                workspace.history.active,
                cx.spawn(async move |_, _| std::future::pending::<()>().await),
            ));
            workspace.watch_tools(workspace.history.active, receiver, cx);
            workspace
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        workspace.update(window, |workspace, cx| {
            workspace.scroll.scroll_to_bottom();
            cx.notify();
        });
        window.run_until_parked();
        let scroll = workspace.read_with(window, |workspace, _| workspace.scroll.clone());
        let at_bottom = || scroll.max_offset().y + scroll.offset().y <= px(2.);
        assert!(scroll.max_offset().y > px(400.));
        assert!(at_bottom());
        assert!(
            window.debug_bounds("message-spacer-0").is_some(),
            "distant messages should keep only a measured spacer"
        );
        let measured_height = scroll.max_offset().y;
        workspace.update(window, |_, cx| cx.notify());
        window.run_until_parked();
        assert_eq!(
            scroll.max_offset().y,
            measured_height,
            "replacing history with spacers must preserve scroll geometry"
        );
        let update = |paragraphs| {
            tools::ToolEvent::Transcript(vec![Text("正在输出结果。\n\n".repeat(paragraphs))])
        };
        let old_max = scroll.max_offset().y;
        sender.try_send(update(20)).unwrap();
        window.run_until_parked();
        assert!(scroll.max_offset().y > old_max);
        assert!(
            at_bottom(),
            "new content should stay pinned while reading the end"
        );

        // Use the conversation's left padding so nested Markdown controls cannot consume the wheel.
        let wheel_position = gpui::point(
            scroll.bounds().left() + px(8.),
            scroll.bounds().top() + px(80.),
        );
        window.simulate_event(gpui::ScrollWheelEvent {
            position: wheel_position,
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(360.))),
            ..Default::default()
        });
        window.run_until_parked();
        assert!(
            !at_bottom(),
            "the reader must be able to scroll back during generation"
        );
        let reading_offset = scroll.offset();
        sender.try_send(update(50)).unwrap();
        window.run_until_parked();
        assert_eq!(
            scroll.offset(),
            reading_offset,
            "streaming must preserve the history position"
        );

        window.simulate_event(gpui::ScrollWheelEvent {
            position: wheel_position,
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-100000.))),
            ..Default::default()
        });
        window.run_until_parked();
        assert!(at_bottom());
        let old_max = scroll.max_offset().y;
        sender.try_send(update(80)).unwrap();
        window.run_until_parked();
        assert!(scroll.max_offset().y > old_max);
        assert!(
            at_bottom(),
            "returning to the bottom should resume following automatically"
        );

        window.simulate_event(gpui::ScrollWheelEvent {
            position: wheel_position,
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(360.))),
            ..Default::default()
        });
        window.run_until_parked();
        let reading_offset = scroll.offset();
        workspace.update(window, |workspace, cx| {
            let blocks = std::mem::take(&mut workspace.run_mut(workspace.history.active).transcript);
            workspace
                .history
                .active_mut()
                .push(Role::Agent, "正在输出结果。\n\n".repeat(80));
            workspace
                .history
                .active_mut()
                .messages
                .last_mut()
                .unwrap()
                .blocks = blocks;
            workspace.runs.remove(&workspace.history.active);
            scroll_follow::follow_if_at_bottom(&workspace.scroll);
            cx.notify();
        });
        window.run_until_parked();
        assert_eq!(
            scroll.offset(),
            reading_offset,
            "finishing the reply must not pull the reader down"
        );
    }

    #[gpui::test]
    fn expanded_history_process_culls_rows_after_scroll_and_session_switch(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("providers.json"), r#"{"providers":[],"active":null,"active_model":""}"#).unwrap();
        let (workspace, window) = cx.add_window_view(|_, cx| {
            let mut workspace = AgentWorkspace::with_path(directory.path().join("sessions.json"), cx);
            workspace.history.active_mut().push(Role::Agent, "done".into());
            workspace.history.active_mut().messages[0].blocks = (0..300).map(|i| {
                let record = tools::rejected(tools::ToolCall { id: format!("row-{i}"), name: "read_file".into(), arguments: serde_json::json!({"path":format!("file-{i}.rs")}) }, "example output".into());
                generation::ContentBlock::ToolResult(record)
            }).collect();
            workspace
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.run_until_parked();
        let original = workspace.read_with(window, |workspace, _| workspace.history.active);
        let key = format!("{original}:0");
        workspace.update(window, |workspace, cx| {
            workspace.process_panels.borrow()[&key].panel.update(cx, |panel, cx| panel.set_expanded(true, cx));
            workspace.scroll.scroll_to_top_of_item(0);
            cx.notify();
        });
        window.run_until_parked();
        assert!(window.debug_bounds("history-row-0").is_some());
        assert!(window.debug_bounds("history-spacer-299").is_some());
        let height = workspace.read_with(window, |w, _| w.scroll.max_offset().y);
        workspace.update(window, |w, cx| { w.scroll.scroll_to_bottom(); cx.notify(); });
        window.run_until_parked();
        assert!(window.debug_bounds("history-spacer-0").is_some());
        assert!(window.debug_bounds("history-row-299").is_some());
        workspace.read_with(window, |w, _| assert_eq!(w.scroll.max_offset().y, height));
        workspace.update(window, |w, cx| {
            w.history.new_session_for(None);
            w.switch(w.history.active, cx);
        });
        window.run_until_parked();
        workspace.update(window, |w, cx| w.switch(original, cx));
        window.run_until_parked();
        assert!(window.debug_bounds("history-row-0").is_some());
        assert!(window.debug_bounds("history-spacer-299").is_some());
        window.simulate_resize(size(px(1000.), px(700.)));
        window.run_until_parked();
        assert!(window.debug_bounds("history-row-0").is_some());
    }

    #[gpui::test]
    fn measured_history_reappears_after_jump_resize_and_session_switch(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (workspace, window) = cx.add_window_view(|_, cx| {
            let mut workspace =
                AgentWorkspace::with_path(directory.path().join("sessions.json"), cx);
            for index in 0..30 {
                workspace.history.active_mut().push(
                    Role::User,
                    format!("History {index} with variable wrapping and paragraphs.\n\n").repeat(6),
                );
            }
            workspace
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        workspace.update(window, |this, cx| {
            this.scroll.scroll_to_bottom();
            cx.notify();
        });
        window.run_until_parked();
        assert!(window.debug_bounds("message-spacer-0").is_some());
        workspace.update(window, |this, cx| {
            this.scroll.scroll_to_top_of_item(0);
            cx.notify();
        });
        window.run_until_parked();
        assert!(window.debug_bounds("message-spacer-0").is_none());
        let before = workspace.read_with(window, |this, _| this.scroll.max_offset().y);
        window.simulate_resize(size(px(1000.), px(700.)));
        window.run_until_parked();
        workspace.read_with(window, |this, _| {
            assert_eq!(this.message_layout.measure_passes, 0);
            assert_eq!(this.message_layout.width, this.scroll.bounds().size.width);
            assert!(this.scroll.max_offset().y > before);
        });
        let original = workspace.read_with(window, |this, _| this.history.active);
        workspace.update(window, |this, cx| {
            this.history.new_session_for(None);
            let id = this.history.active;
            this.history
                .active_mut()
                .push(Role::User, "short session".into());
            this.switch(id, cx);
        });
        window.run_until_parked();
        workspace.read_with(window, |this, _| {
            assert_eq!(this.scroll.max_offset().y, px(0.))
        });
        workspace.update(window, |this, cx| this.switch(original, cx));
        window.run_until_parked();
        workspace.read_with(window, |this, _| {
            assert!(this.scroll.max_offset().y > px(400.))
        });
    }

    #[test]
    fn historical_call_without_result_is_closed_while_streaming_call_is_pending() {
        let call = tools::ToolCall {
            id: "orphan".into(),
            name: "run_command".into(),
            arguments: serde_json::json!({}),
        };
        let pending = tool_presentation(&unresolved_tool_record(&call, false));
        assert_eq!(pending.summary, "正在生成参数 run_command");
        let ended = tool_presentation(&unresolved_tool_record(&call, true));
        assert_eq!(ended.summary, "调用已中断 run_command");
        assert!(!ended.body.contains("等待工具返回结果"));
    }

    #[test]
    fn generating_rejected_and_interrupted_tools_never_present_as_running() {
        for (result, label) in [
            ("正在生成参数…", "正在生成参数"),
            ("未执行：JSON 错误", "未执行"),
            ("调用已中断：结果未知", "调用已中断"),
        ] {
            let record = tools::rejected(
                tools::ToolCall {
                    id: "test".into(),
                    name: "run_command".into(),
                    arguments: serde_json::json!({}),
                },
                result.into(),
            );
            let presentation = tool_presentation(&record);
            assert!(presentation.summary.starts_with(label));
            assert!(!presentation.body.contains("等待工具返回结果"));
        }
    }

    #[test]
    fn process_cache_tracks_live_output_timings_and_tool_results() {
        use generation::ContentBlock::{ToolCall, ToolResult};
        let mut record = tools::ToolRecord {
            call: tools::ToolCall {
                id: "command".into(),
                name: "run_command".into(),
                arguments: serde_json::json!({"program":"cargo", "args":["test"]}),
            },
            result: "执行中…".into(),
            success: false,
            elapsed_ms: 0,
            live_output: None,
            nested_tools: Default::default(),
            subagent: None,
            change: None,
        };
        let blocks = vec![Thinking("checking".into()), ToolCall(record.call.clone())];
        let mut timings = std::collections::BTreeMap::new();
        let signature = process_signature(&blocks, &timings, "正在工作…", &[record.clone()]);
        assert_eq!(
            signature,
            process_signature(&blocks.clone(), &timings, "正在工作…", &[record.clone()])
        );
        record.live_output = Some("Compiling".into());
        assert_ne!(
            signature,
            process_signature(&blocks, &timings, "正在工作…", &[record.clone()])
        );
        let signature = process_signature(&blocks, &timings, "正在工作…", &[record.clone()]);
        timings.insert(0, 1234);
        assert_ne!(
            signature,
            process_signature(&blocks, &timings, "正在工作…", &[record.clone()])
        );
        record.result = "exit: 0\ntest passed".into();
        record.success = true;
        record.live_output = None;
        let mut completed = blocks;
        completed.push(ToolResult(record));
        let signature = process_signature(&completed, &timings, "工作过程", &[]);
        if let Some(ToolResult(record)) = completed.last_mut() {
            record.result.push_str("\nextra output");
        }
        assert_ne!(
            signature,
            process_signature(&completed, &timings, "工作过程", &[])
        );
    }
    #[test]
    fn thinking_summary_follows_latest_nonempty_line_then_shows_duration() {
        assert_eq!(
            thinking_summary("第一步\n正在检查文件\n  ", true, Some(1200)),
            "正在检查文件"
        );
        assert_eq!(
            thinking_summary("第一步\n新的内容持续到达", true, Some(1500)),
            "新的内容持续到达"
        );
        assert_eq!(thinking_summary("", true, None), "正在思考…");
        assert_eq!(
            thinking_summary("不再展示预览", false, Some(12345)),
            "已思考 12.3 s"
        );
        assert_eq!(thinking_summary("旧历史", false, None), "已思考");
    }
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
            live_output: None,
            nested_tools: Default::default(),
            subagent: None,
            change: None,
        };
        let view = tool_presentation(&record);
        assert_eq!(
            view.summary,
            "已运行 git diff --cached --name-status · 1.2s"
        );
        assert_eq!(view.body, record.result);
        record.result = "执行中…".into();
        assert_eq!(tool_presentation(&record).body, "等待工具返回结果…");
        let signature = tool_signature(&record);
        record.live_output = Some("Compiling glint-agent\ntest in progress".into());
        assert_ne!(tool_signature(&record), signature);
        assert_eq!(
            tool_presentation(&record).body,
            "Compiling glint-agent\ntest in progress"
        );
        assert!(
            tool_presentation(&record)
                .summary
                .starts_with("正在运行 git diff")
        );
        record.result = "exit: 2\nerror".into();
        record.live_output = None;
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

#[cfg(test)]
mod event_burst_tests {
    use super::*;
    #[test]
    fn tool_burst_keeps_lifecycle_and_latest_full_transcript() {
        let mut events = vec![];
        for i in 0..1000 {
            events.push(tools::ToolEvent::Transcript(vec![
                generation::ContentBlock::Text(i.to_string()),
            ]));
            events.push(tools::ToolEvent::Started(tools::ToolCall {
                id: i.to_string(),
                name: "read_file".into(),
                arguments: serde_json::json!({}),
            }));
        }
        for i in 0..1000 {
            events.push(tools::ToolEvent::Output {
                call_id: "command".into(),
                output: i.to_string(),
            });
        }
        let events = coalesce_tool_snapshots(events);
        assert_eq!(events.len(), 1002);
        assert!(matches!(events.last(),Some(tools::ToolEvent::Output{output,..}) if output=="999"));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, tools::ToolEvent::Started(_)))
                .count(),
            1000
        );
        assert!(events.iter().any(|e|matches!(e,tools::ToolEvent::Transcript(blocks) if matches!(&blocks[0],generation::ContentBlock::Text(text) if text=="999"))));
    }
}
