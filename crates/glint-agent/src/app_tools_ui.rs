use super::*;
use gpui::{StyledText, TextRun};
use gpui_glint::components::ChainScroll;
use std::hash::{Hash, Hasher};

pub struct MarkdownView {
    source: SharedString,
    theme: ThemePreset,
    editor: Entity<gpui_glint::components::DocumentEditor>,
    root: Option<PathBuf>,
    last_used: std::time::Instant,
    follow_tail: bool,
    _link_subscription: Subscription,
}

pub struct ToolPanel {
    active: bool,
    output_panel: Option<Entity<gpui_glint::components::CollapsePanel>>,
    output_scroll: ScrollHandle,
    diff_scroll: ScrollHandle,
    signature: String,
    theme: ThemePreset,
    panel: Entity<gpui_glint::components::CollapsePanel>,
}

pub struct ProcessPanel {
    rows: std::rc::Rc<std::cell::RefCell<Option<Entity<crate::history_rows::HistoryRows>>>>,
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

fn command_description(call: &tools::ToolCall) -> Option<String> {
    call.arguments["description"]
        .as_str()
        .or_else(|| call.arguments["summary"].as_str())
        .or_else(|| call.arguments["toolSummary"].as_str())
        .or_else(|| call.arguments["title"].as_str())
        .or_else(|| call.arguments["explanation"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn command_text(call: &tools::ToolCall) -> String {
    if let Some(cmd) = call.arguments["command"]
        .as_str()
        .or_else(|| call.arguments["CommandLine"].as_str())
        .or_else(|| call.arguments["cmd"].as_str())
    {
        let trimmed = cmd.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    command_summary(call)
}

fn format_command_prompt(call: &tools::ToolCall, preview_root: Option<&std::path::Path>) -> String {
    let cwd_val = call.arguments["cwd"].as_str().filter(|s| !s.trim().is_empty());
    let cwd = cwd_val.map(std::path::Path::new).or(preview_root);
    let folder_name = cwd
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            cwd.and_then(|p| p.to_str()).unwrap_or("project")
        });
    let folder_name = folder_name.rsplit(['/', '\\']).next().unwrap_or(folder_name);
    format!("~/.../{folder_name} $ ")
}

fn highlight_command_line(prompt: &str, cmd: &str, theme: ThemePreset) -> gpui::StyledText {
    let p = theme.palette();
    let dark = theme.is_dark();
    let cmd_color = if dark { 0xe06c75 } else { 0xd9342b };
    let flag_color = if dark { 0xd19a66 } else { 0xb37716 };
    let string_color = if dark { 0x98c379 } else { 0x24864c };
    let op_color = if dark { 0x56b6c2 } else { 0x007aff };
    let plain_color = p.text;
    let prompt_color = p.text_muted;

    let full_text = format!("{prompt}{cmd}");
    let mut runs = Vec::new();

    if !prompt.is_empty() {
        runs.push(TextRun {
            len: prompt.len(),
            font: gpui::Font::default(),
            color: gpui::rgb_to_hsla(rgb(prompt_color)),
            background_color: None,
            underline: None,
            strikethrough: None,
            letter_spacing: None,
        });
    }

    let mut is_command_start = true;
    let mut expect_subcommand = false;
    let mut i = 0;
    let bytes = cmd.as_bytes();

    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            runs.push(TextRun {
                len: i - start,
                font: gpui::Font::default(),
                color: gpui::rgb_to_hsla(rgb(plain_color)),
                background_color: None,
                underline: None,
                strikethrough: None,
                letter_spacing: None,
            });
            continue;
        }

        if bytes[i..].starts_with(b"&&") || bytes[i..].starts_with(b"||") {
            runs.push(TextRun {
                len: 2,
                font: gpui::Font::default(),
                color: gpui::rgb_to_hsla(rgb(op_color)),
                background_color: None,
                underline: None,
                strikethrough: None,
                letter_spacing: None,
            });
            i += 2;
            is_command_start = true;
            expect_subcommand = false;
            continue;
        }
        if bytes[i] == b'|' || bytes[i] == b';' || bytes[i] == b'&' {
            runs.push(TextRun {
                len: 1,
                font: gpui::Font::default(),
                color: gpui::rgb_to_hsla(rgb(op_color)),
                background_color: None,
                underline: None,
                strikethrough: None,
                letter_spacing: None,
            });
            i += 1;
            is_command_start = true;
            expect_subcommand = false;
            continue;
        }

        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            let start = i;
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i < bytes.len() {
                i += 1;
            }
            runs.push(TextRun {
                len: i - start,
                font: gpui::Font::default(),
                color: gpui::rgb_to_hsla(rgb(string_color)),
                background_color: None,
                underline: None,
                strikethrough: None,
                letter_spacing: None,
            });
            is_command_start = false;
            expect_subcommand = false;
            continue;
        }

        let start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'|'
            && bytes[i] != b';'
            && bytes[i] != b'&'
        {
            if bytes[i] == b'"' || bytes[i] == b'\'' {
                break;
            }
            i += 1;
        }
        let word = &cmd[start..i];

        if word.starts_with('-') {
            runs.push(TextRun {
                len: word.len(),
                font: gpui::Font::default(),
                color: gpui::rgb_to_hsla(rgb(flag_color)),
                background_color: None,
                underline: None,
                strikethrough: None,
                letter_spacing: None,
            });
            is_command_start = false;
        } else if is_command_start {
            runs.push(TextRun {
                len: word.len(),
                font: gpui::Font::default(),
                color: gpui::rgb_to_hsla(rgb(cmd_color)),
                background_color: None,
                underline: None,
                strikethrough: None,
                letter_spacing: None,
            });
            is_command_start = false;
            if matches!(
                word,
                "cargo" | "git" | "npm" | "pnpm" | "bun" | "yarn" | "docker" | "kubectl" | "go" | "rustup"
            ) {
                expect_subcommand = true;
            }
        } else if expect_subcommand {
            runs.push(TextRun {
                len: word.len(),
                font: gpui::Font::default(),
                color: gpui::rgb_to_hsla(rgb(cmd_color)),
                background_color: None,
                underline: None,
                strikethrough: None,
                letter_spacing: None,
            });
            expect_subcommand = false;
        } else {
            runs.push(TextRun {
                len: word.len(),
                font: gpui::Font::default(),
                color: gpui::rgb_to_hsla(rgb(plain_color)),
                background_color: None,
                underline: None,
                strikethrough: None,
                letter_spacing: None,
            });
        }
    }

    StyledText::new(full_text).with_runs(runs)
}

fn command_summary(call: &tools::ToolCall) -> String {
    if let Some(cmd) = call.arguments["command"]
        .as_str()
        .or_else(|| call.arguments["CommandLine"].as_str())
        .or_else(|| call.arguments["cmd"].as_str())
    {
        let trimmed = cmd.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
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

fn tool_succeeded(record: &tools::ToolRecord) -> bool {
    record.success
        && !(record.call.name == "run_command"
            && record
                .result
                .strip_prefix("exit: ")
                .and_then(|result| result.lines().next())
                .is_some_and(|status| status.trim() != "0"))
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
    let success = tool_succeeded(record);
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
        "run_command" => {
            let target = command_description(&record.call)
                .unwrap_or_else(|| command_summary(&record.call));
            (
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
                } else if serde_json::from_str::<serde_json::Value>(&record.result)
                    .ok()
                    .is_some_and(|value| value["status"] == "cancelled")
                {
                    "命令已终止"
                } else {
                    "命令失败"
                },
                target,
            )
        }
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
    let mut summary = if action == "已运行" {
        if let Some(stripped) = target.strip_prefix("已运行 ") {
            format!("{action} {stripped}")
        } else if let Some(stripped) = target.strip_prefix("Ran ") {
            format!("{action} {stripped}")
        } else {
            format!("{action} {target}")
        }
    } else if action == "正在运行" {
        if let Some(stripped) = target.strip_prefix("正在运行 ") {
            format!("{action} {stripped}")
        } else if let Some(stripped) = target.strip_prefix("Running ") {
            format!("{action} {stripped}")
        } else {
            format!("{action} {target}")
        }
    } else {
        format!("{action} {target}")
    };
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

fn file_icon_color(path: &str) -> Option<u32> {
    gpui_glint::components::Icon::color_for_file_name(path)
}

fn read_file_line_range(record: &tools::ToolRecord) -> Option<(usize, usize)> {
    let (res_start, res_count) = serde_json::from_str::<serde_json::Value>(&record.result)
        .ok()
        .map(|v| {
            let start = v["start_line"].as_u64().map(|n| n as usize);
            let count = v["content"].as_str().map(|c| c.lines().count());
            (start, count)
        })
        .unwrap_or((None, None));

    let args = &record.call.arguments;
    let arg_start = args["start_line"]
        .as_u64()
        .or_else(|| args["StartLine"].as_u64())
        .or_else(|| args["offset"].as_u64())
        .map(|n| n as usize);
    let arg_end = args["EndLine"].as_u64().map(|n| n as usize);
    let arg_lines = args["lines"]
        .as_u64()
        .or_else(|| args["limit"].as_u64())
        .map(|n| n as usize);

    let start = res_start.or(arg_start);
    if let Some(start) = start {
        if let Some(end) = arg_end {
            Some((start, end.max(start)))
        } else if let Some(count) = res_count {
            if count > 0 {
                Some((start, start + count - 1))
            } else {
                Some((start, start))
            }
        } else if let Some(lines) = arg_lines {
            Some((start, start + lines.saturating_sub(1)))
        } else {
            Some((start, start))
        }
    } else {
        None
    }
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
                                tools::ToolEvent::SteeringApplied(blocks) => {
                                    this.steering_applied(id, blocks);
                                }
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
                                    if run
                                        .approval
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
                                    if id == this.history.active
                                        && this
                                            .selected_subagent()
                                            .is_some_and(|trace| trace.status.active())
                                    {
                                        if let Some(scroll) = this.selected_subagent_scroll() {
                                            scroll_follow::follow_if_at_bottom(&scroll);
                                        }
                                    }
                                }
                                tools::ToolEvent::BatchProgress { call_id, tools } => {
                                    if let Some(record) = this
                                        .run_mut(id)
                                        .tool_activity
                                        .iter_mut()
                                        .find(|r| r.call.id == call_id)
                                    {
                                        record.nested_tools = tools.into();
                                    }
                                }
                                tools::ToolEvent::Question(request) => {
                                    if !request.respond.is_closed() {
                                        let theme = this.theme;
                                        let input = cx.new(|cx| {
                                            TextEntry::new(cx, "输入回答…", "").theme(theme)
                                        });
                                        this.run_mut(id)
                                            .questions
                                            .push(Question { request, input });
                                    }
                                }
                                tools::ToolEvent::QuestionResolved(question_id) => {
                                    this.run_mut(id)
                                        .questions
                                        .retain(|q| q.request.id != question_id);
                                }
                                tools::ToolEvent::ApprovalResolved => {
                                    if this
                                        .run_mut(id)
                                        .approval
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
                                        session.context = Some(context_usage::ContextSnapshot {
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
                                    this.refresh_steering_turn(id);
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
                                    if record.call.name.starts_with("schedule_") {
                                        this.refresh_calendar(cx);
                                    }

                                    if let Some(existing) = this
                                        .run_mut(id)
                                        .tool_activity
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
        let is_edit = matches!(record.call.name.as_str(), "write_file" | "replace_in_file")
            || record.change.is_some();
        let is_read = record.call.name == "read_file";
        let is_file_op = is_edit || is_read;
        let entry = cache
            .entry(format!("{key}:tool:{}", record.call.id))
            .or_insert_with(|| {
                let mut panel = CollapsePanel::new("").expandable(!is_file_op).theme(theme);
                if !is_file_op {
                    panel = panel.icon(tool_presentation(record).icon);
                }
                let panel = cx.new(|_| panel);
                // A tool toggle also changes the layout of the surrounding conversation.
                cx.subscribe(
                    &panel,
                    |_, _, _: &gpui_glint::components::CollapsePanelEvent, cx| cx.notify(),
                )
                .detach();
                ToolPanel {
                    active: false,
                    output_panel: None,
                    output_scroll: ScrollHandle::new(),
                    diff_scroll: ScrollHandle::new(),
                    signature: String::new(),
                    theme,
                    panel,
                }
            });
        if entry.signature != signature || entry.theme != theme {
            let presentation = tool_presentation(record);
            let result: SharedString = presentation.body.into();
            let command_output = matches!(
                record.call.name.as_str(),
                "run_command" | "command_poll" | "command_stop"
            )
            .then(|| crate::command_output::Output::parse(&result, theme));
            let result_scroll = entry.output_scroll.clone();
            let diff_scroll = entry.diff_scroll.clone();
            if record.result == "执行中…" {
                scroll_follow::follow_if_at_bottom(&result_scroll);
            }
            let change = record.change.clone();
            let nested = record.nested_tools.clone();
            // A running call with nothing to show yet renders a skeleton where
            // the "waiting for the tool" placeholder sentence used to sit.
            let pending_body = record.result == "执行中…"
                && record
                    .live_output
                    .as_ref()
                    .is_none_or(|output| output.trim().is_empty());
            let pending_id = format!("tool-pending:{}", record.call.id);
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
            let output_cards = (record.call.name == "code_mode" && !pending_body).then(|| {
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
                        move |_, _| {
                            code_mode_result_view(&cards, theme, &output_scroll).into_any_element()
                        },
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
            let is_run_command = record.call.name == "run_command";
            let raw_cmd = command_text(&record.call);
            let cmd_prompt = format_command_prompt(&record.call, preview_root.as_deref());
            let operation_title = match record.call.name.as_str() {
                "run_command" | "command_poll" | "command_stop" => "Shell",
                "code_mode" => "JavaScript",
                name => name,
            }
            .to_owned();
            let status_animation_id = SharedString::from(format!("tool-status:{}", record.call.id));
            use gpui_glint::components::StatusTone;
            let (status_icon, status_label, status_tone) = if record.result == "执行中…" {
                (Icon::Spinner, "执行中", StatusTone::Accent)
            } else if record.result == "正在生成参数…" {
                (Icon::Clock, "准备中", StatusTone::Accent)
            } else if record.result.starts_with("未执行：") {
                (Icon::Pause, "未执行", StatusTone::Warning)
            } else if record.result.starts_with("调用已中断：") {
                (Icon::Pause, "已中断", StatusTone::Warning)
            } else if tool_succeeded(record) {
                (Icon::CheckCircle, "成功", StatusTone::Success)
            } else {
                (Icon::Close, "失败", StatusTone::Danger)
            };
            // A tool call is one result block, so its body lines up flush with
            // the trigger icon and skips the indent guide. That holds for tool
            // orchestration too: the script and its return value read as one
            // answer, not as a list to be grouped.
            entry.panel.update(cx, |panel, cx| {
                panel.set_theme(theme, cx);
                panel.set_expandable(!is_file_op, cx);
                panel.set_icon(if is_file_op { None } else { Some(presentation.icon) }, cx);
                panel.set_summary(presentation.summary.clone(), cx);
                if is_edit {
                    let path_str = record.call.arguments["path"]
                        .as_str()
                        .or_else(|| record.call.arguments["file"].as_str())
                        .or_else(|| record.change.as_ref().map(|c| c.label.as_str()))
                        .unwrap_or("")
                        .to_owned();
                    let path_buf = record
                        .change
                        .as_ref()
                        .map(|c| c.path.clone())
                        .unwrap_or_else(|| PathBuf::from(&path_str));
                    let file_display_name = std::path::Path::new(&path_str)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&path_str)
                        .to_owned();
                    let file_icon = gpui_glint::components::Icon::for_file_name(&path_str);
                    let icon_color = file_icon_color(&path_str);
                    let root = preview_root.clone();
                    let workspace = preview_workspace.clone();

                    let status_label = if record.result == "执行中…" {
                        "正在编辑"
                    } else if !tool_succeeded(record) {
                        "编辑失败"
                    } else {
                        "已编辑"
                    };

                    let diff_stats = record.change.as_ref().map(|change| {
                        (change.added, change.removed)
                    });
                    let record_change = record.change.clone();

                    let diff_colors = if theme.is_dark() {
                        gpui_glint::components::DiffColors::DARK
                    } else {
                        gpui_glint::components::DiffColors::LIGHT
                    };

                    panel.set_summary_content(
                        move |_, _| {
                            let path_buf = path_buf.clone();
                            let root = root.clone();
                            let workspace = workspace.clone();
                            let record_change = record_change.clone();
                            div()
                                .w_full()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_size(px(13.))
                                        .text_color(rgb(p.text_muted))
                                        .child(status_label),
                                )
                                .child(
                                    div()
                                        .id("tool-file-preview-link")
                                        .debug_selector(|| "tool-file-preview-link".into())
                                        .flex()
                                        .items_center()
                                        .gap(px(5.))
                                        .min_w_0()
                                        .px(px(6.))
                                        .py(px(2.))
                                        .rounded(px(6.))
                                        .cursor_pointer()
                                        .hover(move |style| style.bg(rgb(p.surface_subtle)))
                                        .child(
                                            div()
                                                .flex_shrink_0()
                                                .when_some(icon_color, |d, c| d.text_color(rgb(c)))
                                                .child(file_icon.render(14.)),
                                        )
                                        .child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(rgb(p.text_muted))
                                                .child(file_display_name.clone()),
                                        )
                                        .on_click({
                                            let path = path_buf.clone();
                                            let root = root.clone();
                                            let workspace = workspace.clone();
                                            move |_, _, cx| {
                                                cx.stop_propagation();
                                                let _ = workspace.update(cx, |this, cx| {
                                                    this.preview_changed_file(
                                                        &path,
                                                        root.clone(),
                                                        cx,
                                                    )
                                                });
                                            }
                                        }),
                                )
                                .when_some(diff_stats, |row, (adds, dels)| {
                                    row.child(
                                        div()
                                            .id("tool-file-diff-link")
                                            .debug_selector(|| "tool-file-diff-link".into())
                                            .flex()
                                            .items_center()
                                            .gap(px(4.))
                                            .flex_shrink_0()
                                            .cursor_pointer()
                                            .hover(|style| style.opacity(0.75))
                                            .when(adds > 0 || dels == 0, |r| {
                                                r.child(
                                                    div()
                                                        .text_size(px(13.))
                                                        .font_weight(gpui::FontWeight::MEDIUM)
                                                        .text_color(rgb(diff_colors.added_marker))
                                                        .child(format!("+{adds}")),
                                                )
                                            })
                                            .when(dels > 0, |r| {
                                                r.child(
                                                    div()
                                                        .text_size(px(13.))
                                                        .font_weight(gpui::FontWeight::MEDIUM)
                                                        .text_color(rgb(diff_colors.deleted_marker))
                                                        .child(format!("-{dels}")),
                                                )
                                            })
                                            .on_click(move |_, _, cx| {
                                                cx.stop_propagation();
                                                let _ = workspace.update(cx, |this, cx| {
                                                    if let Some(change) = record_change.clone() {
                                                        this.open_single_edit(change, cx);
                                                    } else {
                                                        this.open_edits(Some(path_buf.clone()), cx);
                                                    }
                                                });
                                            }),
                                    )
                                })
                                .into_any_element()
                        },
                        cx,
                    );
                } else if is_read {
                    let path_str = record.call.arguments["path"]
                        .as_str()
                        .or_else(|| record.call.arguments["file"].as_str())
                        .unwrap_or("")
                        .to_owned();
                    let file_display_name = std::path::Path::new(&path_str)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&path_str)
                        .to_owned();
                    let file_icon = gpui_glint::components::Icon::for_file_name(&path_str);
                    let icon_color = file_icon_color(&path_str);
                    let root = preview_root.clone();
                    let workspace = preview_workspace.clone();

                    let status_label = if record.result == "执行中…" {
                        "正在分析"
                    } else if !tool_succeeded(record) {
                        "分析失败"
                    } else {
                        "已分析"
                    };

                    let line_range = read_file_line_range(record);
                    let (target_str, line_label) = if let Some((start, end)) = line_range {
                        let label = if start == end {
                            format!("#L{start}")
                        } else {
                            format!("#L{start}-{end}")
                        };
                        (format!("{path_str}{label}"), Some(label))
                    } else {
                        (path_str.clone(), None)
                    };

                    panel.set_summary_content(
                        move |_, _| {
                            let target_str = target_str.clone();
                            let root = root.clone();
                            let workspace = workspace.clone();
                            div()
                                .w_full()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_size(px(13.))
                                        .text_color(rgb(p.text_muted))
                                        .child(status_label),
                                )
                                .child(
                                    div()
                                        .id("tool-file-read-link")
                                        .debug_selector(|| "tool-file-read-link".into())
                                        .flex()
                                        .items_center()
                                        .gap(px(5.))
                                        .min_w_0()
                                        .px(px(6.))
                                        .py(px(2.))
                                        .rounded(px(6.))
                                        .cursor_pointer()
                                        .hover(move |style| style.bg(rgb(p.surface_subtle)))
                                        .child(
                                            div()
                                                .flex_shrink_0()
                                                .when_some(icon_color, |d, c| d.text_color(rgb(c)))
                                                .child(file_icon.render(14.)),
                                        )
                                        .child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(rgb(p.text_muted))
                                                .child(file_display_name.clone()),
                                        )
                                        .when_some(line_label.clone(), |row, label| {
                                            row.child(
                                                div()
                                                    .flex_shrink_0()
                                                    .text_color(rgb(p.text_muted))
                                                    .child(label),
                                            )
                                        })
                                        .on_click(move |_, _, cx| {
                                            cx.stop_propagation();
                                            let _ = workspace.update(cx, |this, cx| {
                                                this.preview_file_target(
                                                    &target_str,
                                                    root.clone(),
                                                    cx,
                                                );
                                            });
                                        }),
                                )
                                .into_any_element()
                        },
                        cx,
                    );
                } else if record.call.name == "run_command" && record.result == "执行中…" {
                    let anchor = self.memory.anchor.clone();
                    let session = self.history.active;
                    let call_id = record.call.id.clone();
                    let summary = presentation.summary.clone();
                    let host = weak.clone();
                    panel.set_summary_content(
                        move |_, _| {
                            let cancelling =
                                crate::command_runner::call_cancelling(&anchor, session, &call_id);
                            let anchor = anchor.clone();
                            let call_id = call_id.clone();
                            let host = host.clone();
                            div()
                                .w_full()
                                .min_w_0()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().flex_1().min_w_0().truncate().child(summary.clone()))
                                .child(
                                    div()
                                        .id("stop-running-command")
                                        .debug_selector(|| "stop-running-command".into())
                                        .flex_shrink_0()
                                        .px_2()
                                        .py_1()
                                        .rounded(px(5.))
                                        .cursor_pointer()
                                        .text_size(px(11.))
                                        .text_color(rgb(p.text_muted))
                                        .child(if cancelling {
                                            "正在终止…"
                                        } else {
                                            "终止"
                                        })
                                        .on_click(move |_, _, cx| {
                                            cx.stop_propagation();
                                            let accepted = crate::command_runner::cancel_call(
                                                &anchor, session, &call_id,
                                            );
                                            let _ = host.update(cx, |this, cx| {
                                                if !accepted {
                                                    this.git_ui.toasts.update(cx, |toasts, cx| {
                                                        toasts.push(
                                                            gpui_glint::components::Toast::new(
                                                                "命令尚未启动或已经结束",
                                                            )
                                                            .description(
                                                                "若命令刚开始，请稍后重试。",
                                                            ),
                                                            cx,
                                                        );
                                                    });
                                                }
                                                cx.notify();
                                            });
                                        }),
                                )
                                .into_any_element()
                        },
                        cx,
                    );
                }
                panel.set_content(
                    move |_, cx| {
                        if is_run_command {
                            let styled_cmd =
                                highlight_command_line(&cmd_prompt, &raw_cmd, theme);
                            let cmd_to_copy = raw_cmd.clone();
                            let host = weak.clone();
                            let (header_bg, body_bg) = if theme.is_dark() {
                                (p.panel, p.canvas)
                            } else {
                                (p.panel, p.surface)
                            };
                            return div()
                                .id("command-card")
                                .debug_selector(|| "command-card".into())
                                .w_full()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .rounded(px(8.))
                                .border_1()
                                .border_color(rgb(p.border))
                                .bg(rgb(body_bg))
                                .overflow_hidden()
                                .child(
                                    div()
                                        .id("command-card-header")
                                        .debug_selector(|| "command-card-header".into())
                                        .w_full()
                                        .min_w_0()
                                        .flex()
                                        .items_start()
                                        .justify_between()
                                        .gap(px(8.))
                                        .px_3()
                                        .py(px(8.))
                                        .border_b_1()
                                        .border_color(rgb(p.border))
                                        .bg(rgb(header_bg))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .font_family("monospace")
                                                .text_size(px(12.))
                                                .line_height(px(19.))
                                                .child(styled_cmd),
                                        )
                                        .child(
                                            div()
                                                .id("command-card-copy-btn")
                                                .debug_selector(|| {
                                                    "command-card-copy-btn".into()
                                                })
                                                .flex_shrink_0()
                                                .p(px(3.))
                                                .rounded(px(4.))
                                                .cursor_pointer()
                                                .text_color(rgb(p.text_muted))
                                                .hover(|s| {
                                                    s.text_color(rgb(p.text))
                                                        .bg(rgb(p.surface_subtle))
                                                })
                                                .child(Icon::Copy.render(13.))
                                                .on_click(move |_, _, cx| {
                                                    cx.stop_propagation();
                                                    cx.write_to_clipboard(
                                                        gpui::ClipboardItem::new_string(
                                                            cmd_to_copy.clone(),
                                                        ),
                                                    );
                                                    let _ = host.update(cx, |this, cx| {
                                                        this.git_ui.toasts.update(
                                                            cx,
                                                            |toasts, cx| {
                                                                toasts.push(
                                                                    gpui_glint::components::Toast::new(
                                                                        "已复制命令",
                                                                    ),
                                                                    cx,
                                                                );
                                                            },
                                                        );
                                                    });
                                                }),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("tool-output")
                                        .debug_selector(|| "tool-output".into())
                                        .w_full()
                                        .min_w_0()
                                        .flex_shrink_0()
                                        .max_h(px(320.))
                                        .relative()
                                        .overflow_y_scroll()
                                        .track_scroll(&result_scroll)
                                        .text_size(px(12.))
                                        .line_height(px(20.))
                                        .font_family("monospace")
                                        .text_color(rgb(p.text_muted))
                                        .child(
                                            div()
                                                .w_full()
                                                .min_w_0()
                                                .px_3()
                                                .py(px(10.))
                                                .child(
                                                    if pending_body {
                                                        div()
                                                            .py_1()
                                                            .child(pending_skeleton(
                                                                pending_id.clone(),
                                                                theme,
                                                            ))
                                                            .into_any_element()
                                                    } else if let Some(output) = &command_output {
                                                        output.render().into_any_element()
                                                    } else {
                                                        div()
                                                            .child(result.clone())
                                                            .into_any_element()
                                                    },
                                                ),
                                        )
                                        .child(crate::command_output::scroll_guard(
                                            &result_scroll,
                                            20.,
                                        )),
                                )
                                .into_any_element();
                        }
                        let body = (|| {
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
                                                        .debug_selector(|| {
                                                            "code-mode-source".into()
                                                        })
                                                        .h(px(source_height))
                                                        .flex_shrink_0()
                                                        .overflow_hidden()
                                                        .rounded(px(8.))
                                                        .child(card),
                                                )
                                            })
                                            .child(if let Some(output_panel) = &output_panel {
                                                div()
                                                    .debug_selector(|| {
                                                        "code-mode-result-toggle".into()
                                                    })
                                                    .child(output_panel.clone())
                                                    .into_any_element()
                                            } else if pending_body {
                                                div()
                                                    .w_full()
                                                    .py_1()
                                                    .child(pending_skeleton(
                                                        pending_id.clone(),
                                                        theme,
                                                    ))
                                                    .into_any_element()
                                            } else {
                                                div()
                                                    .text_size(px(12.))
                                                    .child(result.clone())
                                                    .into_any_element()
                                            })
                                            .children(
                                                nested.iter().map(|record| {
                                                    this.tool_panel(&scope, record, cx)
                                                }),
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
                                                    .track_scroll(&diff_scroll)
                                                    .chain_scroll_y(&diff_scroll)
                                                    .child(this.edit_diff(&scope, change, cx)),
                                            )
                                            .into_any_element()
                                    })
                                    .unwrap_or_else(|_| div().into_any_element());
                            }
                            div()
                                .id("tool-output")
                                .debug_selector(|| "tool-output".into())
                                .w_full()
                                .min_w_0()
                                .flex_shrink_0()
                                .max_h(px(320.))
                                .relative()
                                .overflow_y_scroll()
                                .track_scroll(&result_scroll)
                                .text_size(px(12.))
                                .line_height(px(20.))
                                .font_family("monospace")
                                .text_color(rgb(p.text_muted))
                                .child(div().w_full().min_w_0().child(if pending_body {
                                    div()
                                        .py_1()
                                        .child(pending_skeleton(pending_id.clone(), theme))
                                        .into_any_element()
                                } else if let Some(output) = &command_output {
                                    output.render().into_any_element()
                                } else {
                                    div().child(result.clone()).into_any_element()
                                }))
                                .child(crate::command_output::scroll_guard(&result_scroll, 20.))
                                .into_any_element()
                        })();
                        gpui_glint::components::OperationPanel::new(operation_title.clone())
                            .theme(theme)
                            .child(body)
                            .status(status_icon, status_label)
                            .status_tone(status_tone)
                            .status_animation_id(status_animation_id.clone())
                            .render()
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

    /// Thinking text that is still streaming: keep the newest line in view while
    /// older lines scroll up.
    pub fn streaming_markdown_text(
        &self,
        key: String,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Entity<gpui_glint::components::DocumentEditor> {
        self.markdown_preview_with(key, text, true, true, cx)
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
        self.markdown_preview_with(key, text, scrollable, false, cx)
    }

    fn markdown_preview_with(
        &self,
        key: String,
        text: &str,
        scrollable: bool,
        follow_tail: bool,
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
                    .link_root(root.clone())
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
                source,
                theme,
                editor,
                root: root.clone(),
                last_used: std::time::Instant::now(),
                follow_tail: false,
                _link_subscription: subscription,
            }
        });
        view.last_used = std::time::Instant::now();
        if view.follow_tail != follow_tail {
            view.follow_tail = follow_tail;
            view.editor.update(cx, |editor, cx| {
                editor.set_follow_preview_tail(follow_tail, cx)
            });
        }
        if view.source.as_ref() != text || view.theme != theme {
            let changed = view.source.as_ref() != text;
            view.editor.update(cx, |editor, cx| {
                editor.set_theme(theme, cx);
                if changed {
                    editor.sync_preview_markdown(text, cx);
                }
            });
            if changed {
                view.source = SharedString::from(text.to_owned());
            }
            view.theme = theme;
        }
        let editor = view.editor.clone();
        if prune {
            let mut bytes: usize = cache.values().map(|v| v.source.len()).sum();
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
                    bytes = bytes.saturating_sub(old.source.len());
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
                if complete {
                    &[]
                } else {
                    self.active_run().visible_tools()
                },
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
                            active: false,
                            output_panel: None,
                            output_scroll: ScrollHandle::new(),
                            diff_scroll: ScrollHandle::new(),
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
                        // Only the streaming action follows its newest line; a
                        // reopened history panel keeps its own scroll position.
                        let streaming = active;
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
                                        let editor = if streaming {
                                            this.streaming_markdown_text(
                                                editor_key.clone(),
                                                &source,
                                                cx,
                                            )
                                        } else {
                                            this.markdown_text(editor_key.clone(), &source, cx)
                                        };
                                        div().w_full().min_w_0().h(px(height)).child(editor)
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
            let panel_key = match block {
                ContentBlock::Thinking(_) => Some(format!("{key}:{index}:thinking")),
                ContentBlock::ToolCall(call) if !completed.contains(call.id.as_str()) => {
                    Some(format!("{key}:tool:{}", call.id))
                }
                ContentBlock::ToolResult(record) => Some(format!("{key}:tool:{}", record.call.id)),
                _ => None,
            };
            if let Some(panel_key) = panel_key
                && let Some(entry) = self.tool_panels.borrow_mut().get_mut(&panel_key)
            {
                let active = !complete && index + 1 == blocks.len();
                // Change the default only on action boundaries. Streaming deltas
                // must never override a user's manual expansion/collapse.
                if entry.active != active {
                    entry.active = active;
                    entry
                        .panel
                        .update(cx, |panel, cx| panel.set_expanded(active, cx));
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
                    if complete {
                        &[]
                    } else {
                        self.active_run().visible_tools()
                    },
                )
            });
        let signature = source_signature ^ self.subagent_revision();
        let entry = cache.entry(key.into()).or_insert_with(|| ProcessPanel {
            rows: Default::default(),
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
            // Keep the ancestor entity stable across deltas: replacing it also
            // replaces GPUI's element paths and loses in-flight mouse clicks.
            let rows = entry.rows.clone();
            let refreshed = std::cell::Cell::new(false);
            let weak = cx.entity().downgrade();
            let process = blocks[..split].to_vec();
            let thinking_ms = thinking_ms.clone();
            let activity = if complete {
                Vec::new()
            } else {
                self.active_run().visible_tools().to_vec()
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
                        let scroll = weak
                            .update(cx, |this, _| this.scroll.clone())
                            .unwrap_or_default();
                        let mut rows = rows.borrow_mut();
                        if !refreshed.replace(true) {
                            let children = weak
                                .update(cx, |this, cx| {
                                    this.transcript_rows(
                                        &content_key,
                                        &process,
                                        &thinking_ms,
                                        complete,
                                        &activity,
                                        cx,
                                    )
                                })
                                .unwrap_or_default();
                            if let Some(rows) = rows.as_ref() {
                                rows.update(cx, |rows, cx| rows.set_rows(children, cx));
                            } else {
                                *rows = Some(cx.new(|_| {
                                    crate::history_rows::HistoryRows::new(children, scroll.clone())
                                }));
                            }
                        }
                        let rows = rows.as_ref().unwrap();
                        rows.update(cx, |rows, _| rows.set_scroll(scroll));
                        div()
                            .w_full()
                            .min_w_0()
                            .child(rows.clone())
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

    fn answer_question(
        &mut self,
        session: Uuid,
        id: Uuid,
        answer: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let questions = &mut self.run_mut(session).questions;
        if let Some(index) = questions.iter().position(|q| q.request.id == id) {
            let question = questions.remove(index);
            let _ = question.request.respond.send(answer);
        }
        cx.notify();
    }

    pub fn question_panel(&self, cx: &mut Context<Self>) -> gpui::Div {
        const MAX_WIDTH: f32 = 800.;
        // Bound text during intrinsic measurement as well as final layout.
        // The card's padding and border leave 26 px less for wrapped content.
        const CONTENT_WIDTH: f32 = MAX_WIDTH - 26.;
        let p = self.theme.palette();
        let session = self.history.active;
        let mut panel = div()
            .debug_selector(|| "agent-question-panel".into())
            .w_full()
            .max_w(px(MAX_WIDTH))
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_3();
        for question in self
            .active_run()
            .questions
            .iter()
            .filter(|q| !q.request.respond.is_closed())
        {
            let id = question.request.id;
            let input = question.input.clone();
            let source = question
                .request
                .agent_id
                .and_then(|id| {
                    self.active_run()
                        .subagents
                        .lock()
                        .ok()?
                        .iter()
                        .find(|trace| trace.id == id)
                        .map(|trace| format!("子 Agent「{}」提问", trace.title))
                })
                .unwrap_or_else(|| "Agent 提问".into());
            let mut choices = div()
                .max_w(px(CONTENT_WIDTH))
                .min_w_0()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .gap_2();
            for (index, option) in question.request.options.iter().enumerate() {
                let answer = option.clone();
                choices = choices.child(
                    div()
                        .id(("question-option", index))
                        .debug_selector(move || format!("question-option-{id}-{index}"))
                        .max_w(px(CONTENT_WIDTH))
                        .min_w_0()
                        .flex_shrink_0()
                        .whitespace_normal()
                        .px_3()
                        .py_2()
                        .rounded(px(7.))
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.surface))
                        .text_size(px(12.))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(p.accent_tint)).border_color(rgb(p.accent)))
                        .child(format!("{}. {option}", index + 1))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.answer_question(session, id, Some(answer.clone()), cx)
                        })),
                );
            }
            panel = panel.child(
                div()
                    .id(SharedString::from(format!("agent-question-{id}")))
                    .debug_selector(move || format!("agent-question-{id}"))
                    .max_w(px(MAX_WIDTH))
                    .min_w_0()
                    .flex_shrink_0()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(p.border))
                    .bg(rgb(p.panel))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(Icon::Text.render(14.).text_color(rgb(p.accent)))
                            .child(source)
                            .child("· 等待你的回答"),
                    )
                    .child(
                        div()
                            .max_w(px(CONTENT_WIDTH))
                            .min_w_0()
                            .flex_shrink_0()
                            .whitespace_normal()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(question.request.question.clone()),
                    )
                    .when(!question.request.options.is_empty(), |card| {
                        card.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child("点击一项直接回答，也可以在下方自行填写"),
                        )
                        .child(choices)
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .h(px(38.))
                            .px_3()
                            .rounded(px(7.))
                            .border_1()
                            .border_color(rgb(p.border))
                            .bg(rgb(p.surface))
                            .child(input.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("提交回答")
                                    .theme(self.theme)
                                    .render()
                                    .debug_selector(move || format!("question-submit-{id}"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let answer = input.read(cx).value().trim().to_owned();
                                        if !answer.is_empty() {
                                            this.answer_question(session, id, Some(answer), cx);
                                        }
                                    })),
                            )
                            .child(
                                Button::new("跳过")
                                    .kind(ButtonKind::Plain)
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.answer_question(session, id, None, cx)
                                    })),
                            ),
                    ),
            );
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
                    self.active_run()
                        .subagents
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
                .child(approval_arguments_view(approval, self.theme))
                .when(approval.call.name == "run_command", |panel| panel.child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child("命令在项目目录运行，批准前请检查程序和参数；命令自身可以访问项目外部。")))
                .child(div().flex().gap_3()
                    .child(Button::new("允许本次").theme(self.theme).render().on_click(cx.listener(move |this, _, _, cx| { if let Some(approval) = this.runs.get_mut(&session).and_then(|run| run.approval.take()) { let _=approval.respond.send(true); } cx.notify(); })))
                    .child(Button::new("拒绝").kind(ButtonKind::Glass).theme(self.theme).render().on_click(cx.listener(move |this, _, _, cx| { if let Some(approval) = this.runs.get_mut(&session).and_then(|run| run.approval.take()) { let _=approval.respond.send(false); } cx.notify(); }))));
        }
        panel
    }
}

/// Approval arguments scroll on their own; the dialogue must stay put while the
/// reader inspects a long call, then take over once the arguments hit an edge.
fn approval_arguments_view(approval: &Approval, theme: ThemePreset) -> gpui::Stateful<gpui::Div> {
    let p = theme.palette();
    use std::cell::RefCell;
    thread_local! {
        static HANDLES: RefCell<std::collections::HashMap<String, ScrollHandle>> =
            RefCell::new(std::collections::HashMap::new());
    }
    let scroll = HANDLES.with(|handles| {
        handles
            .borrow_mut()
            .entry(approval.call.id.clone())
            .or_default()
            .clone()
    });
    div()
        .id("approval-arguments")
        .max_h(px(180.))
        .overflow_y_scroll()
        .track_scroll(&scroll)
        .chain_scroll_y(&scroll)
        .text_size(px(11.))
        .text_color(rgb(p.text_muted))
        .child(serde_json::to_string_pretty(&approval.call.arguments).unwrap_or_default())
}

type OutputCard = (
    Option<String>,
    Option<Entity<gpui_glint::components::CodeCard>>,
    f32,
);

fn code_mode_result_view(
    cards: &[OutputCard],
    theme: ThemePreset,
    scroll: &ScrollHandle,
) -> gpui::Stateful<gpui::Div> {
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
        .child(crate::command_output::scroll_guard(&scroll, 21.))
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

/// A running tool call has no result yet; a pulsing placeholder reads as
/// "content is coming" where a sentence of placeholder copy reads as output.
fn pending_skeleton(id: String, theme: ThemePreset) -> gpui::AnyElement {
    div()
        .debug_selector(|| "tool-pending".into())
        .w_full()
        .min_w_0()
        .child(
            gpui_glint::components::Skeleton::new(id)
                .lines(2)
                .height(11.)
                .gap(10.)
                .theme(theme),
        )
        .into_any_element()
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
    fn thinking_toggle_survives_a_stream_update_during_click(cx: &mut gpui::TestAppContext) {
        struct Preview {
            host: Entity<AgentWorkspace>,
            blocks: Vec<generation::ContentBlock>,
        }
        impl Render for Preview {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div().w(px(800.)).child(self.host.update(cx, |host, cx| {
                    host.response_view(
                        "stream-click",
                        &self.blocks,
                        None,
                        &Default::default(),
                        false,
                        cx,
                    )
                }))
            }
        }
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (preview, window) = cx.add_window_view(|_, cx| Preview {
            host: cx
                .new(|cx| AgentWorkspace::with_path(directory.path().join("sessions.json"), cx)),
            blocks: vec![Thinking("检查文件".into())],
        });
        window.run_until_parked();
        let host = preview.read_with(window, |view, _| view.host.clone());
        let panel = host.read_with(window, |host, _| {
            host.tool_panels.borrow()["stream-click:0:thinking"]
                .panel
                .clone()
        });
        panel.read_with(window, |panel, _| {
            assert!(panel.is_expanded(), "the latest action starts expanded")
        });
        panel.update(window, |panel, cx| panel.set_expanded(true, cx));
        window.run_until_parked();
        let position = window.debug_bounds("streaming-summary").unwrap().center();
        window.simulate_mouse_down(position, gpui::MouseButton::Left, Default::default());
        preview.update(window, |view, cx| {
            view.blocks[0] = Thinking("继续检查文件".into());
            cx.notify();
        });
        window.run_until_parked();
        window.simulate_mouse_up(position, gpui::MouseButton::Left, Default::default());
        window.run_until_parked();
        panel.read_with(window, |panel, _| {
            assert!(
                !panel.is_expanded(),
                "streaming must preserve the pending click"
            )
        });
        preview.update(window, |view, cx| {
            view.blocks[0] = Thinking("继续检查文件，保留手动折叠".into());
            cx.notify();
        });
        window.run_until_parked();
        panel.read_with(window, |panel, _| assert!(!panel.is_expanded()));
        let call = tools::ToolCall {
            id: "next-command".into(),
            name: "run_command".into(),
            arguments: serde_json::json!({"program":"cargo","args":["check"]}),
        };
        preview.update(window, |view, cx| {
            view.blocks
                .push(generation::ContentBlock::ToolCall(call.clone()));
            cx.notify();
        });
        window.run_until_parked();
        let command = host.read_with(window, |host, _| {
            host.tool_panels.borrow()["stream-click:tool:next-command"]
                .panel
                .clone()
        });
        command.read_with(window, |panel, _| assert!(panel.is_expanded()));
        preview.update(window, |view, cx| {
            view.blocks
                .push(generation::ContentBlock::ToolResult(tools::rejected(
                    call,
                    "finished".into(),
                )));
            cx.notify();
        });
        window.run_until_parked();
        command.read_with(window, |panel, _| {
            assert!(
                panel.is_expanded(),
                "keep the result visible until the next action"
            )
        });
        preview.update(window, |view, cx| {
            view.blocks.push(Thinking("检查结果".into()));
            cx.notify();
        });
        window.run_until_parked();
        command.read_with(window, |panel, _| assert!(!panel.is_expanded()));
        host.read_with(window, |host, cx| {
            assert!(
                host.tool_panels.borrow()["stream-click:3:thinking"]
                    .panel
                    .read(cx)
                    .is_expanded()
            )
        });
    }

    #[gpui::test]
    fn streaming_thinking_body_follows_its_newest_line(cx: &mut gpui::TestAppContext) {
        struct Preview {
            host: Entity<AgentWorkspace>,
            blocks: Vec<generation::ContentBlock>,
        }
        impl Render for Preview {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div().w(px(800.)).child(self.host.update(cx, |host, cx| {
                    host.response_view(
                        "stream-follow",
                        &self.blocks,
                        None,
                        &Default::default(),
                        false,
                        cx,
                    )
                }))
            }
        }
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let reasoning = |count: usize| {
            (0..count)
                .map(|index| format!("第 {index} 步：核对文件与依赖"))
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        let (preview, window) = cx.add_window_view(|_, cx| Preview {
            host: cx
                .new(|cx| AgentWorkspace::with_path(directory.path().join("sessions.json"), cx)),
            blocks: vec![Thinking(reasoning(4))],
        });
        window.simulate_resize(gpui::size(px(900.), px(600.)));
        window.run_until_parked();
        for count in [16, 32, 48] {
            preview.update(window, |view, cx| {
                view.blocks = vec![Thinking(reasoning(count))];
                cx.notify();
            });
            window.run_until_parked();
            let editor = preview.read_with(window, |view, cx| {
                view.host.read(cx).markdown_views.borrow()["stream-follow:0:thinking"]
                    .editor
                    .clone()
            });
            editor.read_with(window, |editor, _| {
                let scroll = editor.preview_scroll();
                assert!(
                    scroll.max_offset().y > px(0.),
                    "the thinking body must have its own scroll range"
                );
                assert!(
                    scroll.max_offset().y + scroll.offset().y <= px(2.),
                    "the newest reasoning line must stay visible"
                );
            });
        }
    }

    #[gpui::test]
    fn command_output_scrolls_before_the_outer_conversation(cx: &mut gpui::TestAppContext) {
        struct Preview {
            host: Entity<AgentWorkspace>,
            panel: Entity<gpui_glint::components::CollapsePanel>,
            outer: ScrollHandle,
        }
        impl Render for Preview {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .id("command-scroll-outer")
                    .h(px(400.))
                    .w(px(600.))
                    .overflow_y_scroll()
                    .track_scroll(&self.outer)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(div().h(px(300.)).flex_shrink_0())
                            .child(self.panel.clone())
                            .child(div().h(px(500.)).flex_shrink_0()),
                    )
            }
        }
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (preview, window) = cx.add_window_view(|_, cx| {
            let host =
                cx.new(|cx| AgentWorkspace::with_path(directory.path().join("sessions.json"), cx));
            let record = tools::rejected(
                tools::ToolCall {
                    id: "scroll".into(),
                    name: "run_command".into(),
                    arguments: serde_json::json!({"program":"cargo"}),
                },
                "\x1b[32mChecking crate\x1b[0m\n".repeat(100),
            );
            let panel = host.update(cx, |host, cx| host.tool_panel("scroll", &record, cx));
            panel.update(cx, |panel, cx| panel.set_expanded(true, cx));
            Preview {
                host,
                panel,
                outer: ScrollHandle::new(),
            }
        });
        preview.update(window, |view, cx| {
            view.outer.set_offset(point(px(0.), px(-250.)));
            cx.notify();
        });
        window.run_until_parked();
        let (outer, inner) = preview.read_with(window, |view, cx| {
            (
                view.outer.clone(),
                view.host.read(cx).tool_panels.borrow()["scroll:tool:scroll"]
                    .output_scroll
                    .clone(),
            )
        });
        let bounds = window.debug_bounds("tool-output").unwrap();
        assert!(bounds.size.height <= px(320.));
        let position = point(bounds.left() + px(10.), bounds.top() + px(10.));
        let before = outer.offset().y;
        window.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-80.))),
            ..Default::default()
        });
        window.run_until_parked();
        assert!(inner.offset().y < px(0.));
        assert_eq!(
            outer.offset().y,
            before,
            "one wheel event must move only the output"
        );
        preview.update(window, |_, cx| {
            inner.scroll_to_bottom();
            cx.notify();
        });
        window.run_until_parked();
        window.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-80.))),
            ..Default::default()
        });
        window.run_until_parked();
        assert!(
            outer.offset().y < before,
            "at the output boundary the conversation can take over"
        );
    }

    #[gpui::test]
    fn long_questions_scroll_without_displacing_the_composer(cx: &mut gpui::TestAppContext) {
        struct Preview {
            host: Entity<AgentWorkspace>,
            _subscription: Subscription,
        }
        impl Render for Preview {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                self.host.update(cx, |host, cx| host.content(cx))
            }
        }
        cx.executor().allow_parking();
        cx.update(crate::initialize);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (respond, mut response) = tokio::sync::oneshot::channel();
        let id = Uuid::new_v4();
        let submit_selector: &'static str =
            Box::leak(format!("question-submit-{id}").into_boxed_str());
        let (preview, window) = cx.add_window_view(|_, cx| {
            let host = cx.new(|cx| AgentWorkspace::with_path(dir.path().join("sessions.json"), cx));
            host.update(cx, |host, cx| {
                host.history
                    .active_mut()
                    .push(Role::User, "请选择目标文件".into());
                let session = host.history.active;
                let input = cx.new(|cx| TextEntry::new(cx, "输入回答…", "自定义目标"));
                host.run_mut(session).questions.push(Question {
                    request: crate::ask_question::Request {
                        id,
                        agent_id: None,
                        question: "具体要覆盖哪个文件？".repeat(20),
                        options: (0..6)
                            .map(|index| {
                                format!(
                                    "{index}：覆盖 xidl/apis/spec/open/im/api_chat_control.xidl {}",
                                    "请确认目标文件及写入范围。".repeat(12)
                                )
                            })
                            .collect(),
                        respond,
                    },
                    input,
                });
            });
            let subscription = cx.observe(&host, |_, _, cx| cx.notify());
            Preview {
                host,
                _subscription: subscription,
            }
        });
        let host = preview.read_with(window, |preview, _| preview.host.clone());
        for (width, height) in [(520., 420.), (1440., 900.), (900., 480.)] {
            window.simulate_resize(size(px(width), px(height)));
            preview.update(window, |_, cx| cx.notify());
            window.run_until_parked();
            host.update(window, |host, cx| {
                host.scroll.scroll_to_bottom();
                cx.notify();
            });
            preview.update(window, |_, cx| cx.notify());
            window.run_until_parked();
            let composer = window.debug_bounds("agent-composer-panel").unwrap();
            let conversation = window.debug_bounds("agent-chat-conversation").unwrap();
            let submit = window.debug_bounds(submit_selector).unwrap();
            assert!(composer.top() >= px(0.) && composer.bottom() <= px(height));
            assert!(composer.size.height > px(60.));
            assert!(conversation.size.height > px(0.) && conversation.bottom() <= composer.top());
            assert!(
                submit.top() >= conversation.top() && submit.bottom() <= conversation.bottom(),
                "{width}x{height}: submit={submit:?}, conversation={conversation:?}, composer={composer:?}"
            );
        }
        let submit = window.debug_bounds(submit_selector).unwrap();
        window.simulate_click(submit.center(), Default::default());
        assert_eq!(response.try_recv().unwrap(), Some("自定义目标".into()));
    }

    #[gpui::test]
    fn question_answers_are_scoped_and_preserve_other_pending_questions(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let host = cx.new(|cx| AgentWorkspace::with_path(dir.path().join("sessions.json"), cx));
        let (respond, mut response) = tokio::sync::oneshot::channel();
        let (other, mut other_response) = tokio::sync::oneshot::channel();
        host.update(cx, |host, cx| {
            let session = host.history.active;
            let id = Uuid::new_v4();
            for (question_id, sender) in [(id, respond), (Uuid::new_v4(), other)] {
                let input = cx.new(|cx| TextEntry::new(cx, "回答", ""));
                host.run_mut(session).questions.push(Question {
                    request: crate::ask_question::Request {
                        id: question_id,
                        agent_id: None,
                        question: "问题".into(),
                        options: vec![],
                        respond: sender,
                    },
                    input,
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
        struct Preview {
            _host: Entity<AgentWorkspace>,
            panel: Entity<gpui_glint::components::CollapsePanel>,
        }
        impl Render for Preview {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(px(600.)).child(self.panel.clone())
            }
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (preview, window) = cx.add_window_view(|_, cx| {
            let host = cx.new(|cx| AgentWorkspace::with_path(dir.path().join("sessions.json"), cx));
            let record = tools::rejected(
                tools::ToolCall {
                    id: "stop-test".into(),
                    name: "run_command".into(),
                    arguments: serde_json::json!({"program":"rg","args":["search"]}),
                },
                "执行中…".into(),
            );
            let panel = host.update(cx, |host, cx| host.tool_panel("stop-test", &record, cx));
            Preview { _host: host, panel }
        });
        let button = window
            .debug_bounds("stop-running-command")
            .expect("active command stop button");
        window.simulate_click(button.center(), Default::default());
        preview.read_with(window, |preview, cx| {
            assert!(!preview.panel.read(cx).is_expanded())
        });
    }

    #[gpui::test]
    fn orchestration_output_scroll_chains_at_its_boundary(cx: &mut gpui::TestAppContext) {
        struct Nested {
            outer: ScrollHandle,
            inner: ScrollHandle,
        }
        impl Render for Nested {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let cards = (0..12)
                    .map(|i| (Some(format!("section {i}")), None, 50.))
                    .collect::<Vec<_>>();
                div()
                    .id("outer-output")
                    .h(px(400.))
                    .w(px(400.))
                    .track_scroll(&self.outer)
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(div().h(px(300.)).flex_shrink_0())
                            .child(code_mode_result_view(
                                &cards,
                                ThemePreset::default(),
                                &self.inner,
                            ))
                            .child(div().h(px(500.)).flex_shrink_0()),
                    )
            }
        }
        let (host, window) = cx.add_window_view(|_, _| Nested {
            outer: ScrollHandle::new(),
            inner: ScrollHandle::new(),
        });
        host.update(window, |host, cx| {
            host.outer.set_offset(point(px(0.), px(-250.)));
            cx.notify();
        });
        window.run_until_parked();
        let bounds = window.debug_bounds("code-mode-result").unwrap();
        let position = point(bounds.left() + px(8.), bounds.top() + px(8.));
        let initial = host.read_with(window, |host, _| host.outer.offset().y);
        window.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-80.))),
            ..Default::default()
        });
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert!(host.inner.offset().y < px(0.));
            assert_eq!(host.outer.offset().y, initial);
        });
        host.update(window, |host, cx| {
            host.inner.scroll_to_bottom();
            cx.notify();
        });
        window.run_until_parked();
        window.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-80.))),
            ..Default::default()
        });
        window.run_until_parked();
        assert!(
            host.read_with(window, |host, _| host.outer.offset().y) < initial,
            "exhausted output must release the outer conversation"
        );
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
            let host = cx.new(|cx| {
                let mut host = AgentWorkspace::with_path(dir.path().join("sessions.json"), cx);
                host.history.active_mut().project = Some(dir.path().to_path_buf());
                host
            });
            let panel = host.update(cx, |host, cx| host.tool_panel("test", &record, cx));
            Root { host, panel }
        });
        window.run_until_parked();
        let link = window.debug_bounds("tool-file-preview-link").unwrap();
        window.simulate_click(link.center(), Default::default());
        window.run_until_parked();
        root.read_with(window, |root, cx| {
            assert!(!root.panel.read(cx).is_expanded());
            assert!(!root.panel.read(cx).is_expandable());
            assert!(root.host.read(cx).file_preview.open);
            assert!(!root.host.read(cx).edits_ui.open);
        });

        let diff_link = window.debug_bounds("tool-file-diff-link").unwrap();
        window.simulate_click(diff_link.center(), Default::default());
        window.run_until_parked();
        root.read_with(window, |root, cx| {
            assert!(!root.panel.read(cx).is_expanded());
            assert!(!root.host.read(cx).file_preview.open);
            assert!(root.host.read(cx).edits_ui.open);
            assert_eq!(
                root.host.read(cx).edits_ui.selected.as_deref(),
                Some(path.as_path())
            );
        });
    }

    #[gpui::test]
    fn read_file_tool_panel_opens_preview_at_line_anchor(cx: &mut gpui::TestAppContext) {
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
        let path = dir.path().join("test.rs");
        std::fs::write(&path, "line1\nline2\nline3\n").unwrap();
        let record: tools::ToolRecord = serde_json::from_value(serde_json::json!({
            "call": {"id":"read1", "name":"read_file", "arguments":{"path":"test.rs","start_line":2,"lines":2}},
            "result": serde_json::json!({"start_line":2,"content":"2: line2\n3: line3","total_lines":3}).to_string(),
            "success":true, "elapsed_ms":10,
        })).unwrap();
        let (root, window) = cx.add_window_view(|_, cx| {
            let host = cx.new(|cx| {
                let mut host = AgentWorkspace::with_path(dir.path().join("sessions.json"), cx);
                host.history.active_mut().project = Some(dir.path().to_path_buf());
                host
            });
            let panel = host.update(cx, |host, cx| host.tool_panel("test", &record, cx));
            Root { host, panel }
        });
        window.run_until_parked();
        root.read_with(window, |root, cx| {
            assert!(!root.panel.read(cx).is_expandable());
            assert!(!root.panel.read(cx).is_expanded());
        });
        let link = window.debug_bounds("tool-file-read-link").unwrap();
        window.simulate_click(link.center(), Default::default());
        window.run_until_parked();
        root.read_with(window, |root, cx| {
            assert!(root.host.read(cx).file_preview.open);
            let target = root.host.read(cx).file_preview.target.as_ref().unwrap();
            assert_eq!(target.line, Some(2));
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
            host.history
                .active_mut()
                .push(Role::User, "查询记录".into());
            let mut memory = tools::rejected(
                tools::ToolCall {
                    id: "memory-overlap".into(),
                    name: "memory_search".into(),
                    arguments: serde_json::json!({"query": "查询记录"}),
                },
                serde_json::json!({"matches": vec!["很长的记忆结果。".repeat(30); 20]}).to_string(),
            );
            memory.success = true;
            host.run_mut(host.history.active).transcript =
                vec![generation::ContentBlock::ToolResult(memory)];
            host.run_mut(host.history.active)
                .transcript
                .extend((0..6).map(|_| Thinking("继续查询".into())));
            host.run_mut(host.history.active).pending = Some((
                host.history.active,
                cx.spawn(async move |_, _| std::future::pending::<()>().await),
            ));
            host
        });
        window.simulate_resize(size(px(1200.), px(700.)));
        window.run_until_parked();
        let panel = host.read_with(window, |host, _| {
            host.tool_panels
                .borrow()
                .iter()
                .find(|(key, _)| key.ends_with(":tool:memory-overlap"))
                .unwrap()
                .1
                .panel
                .clone()
        });
        let before = window.debug_bounds("waiting-for-model").unwrap();
        panel.update(window, |panel, cx| panel.set_expanded(true, cx));
        window.run_until_parked();
        let live = window.debug_bounds("live-response").unwrap();
        let waiting = window.debug_bounds("waiting-for-model").unwrap();
        assert!(
            waiting.top() >= live.bottom(),
            "{waiting:?} overlaps {live:?}"
        );
        // A conversation resting at its newest message follows the growth by
        // scrolling, so the waiting row may stay put instead of moving down;
        // an overlap would pull it up over the response above.
        assert!(
            waiting.top() >= before.top(),
            "expanding must never pull the waiting row up: {before:?} -> {waiting:?}"
        );
    }

    #[gpui::test]
    fn expanded_skill_read_keeps_waiting_below_the_live_response(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(directory.path().join("sessions.json"), cx);
            host.history
                .active_mut()
                .push(Role::User, "用 ego-browser 查页面".into());
            let guide = "1: ---\n2: name: ego-browser\n3: description: browse pages\n".repeat(160);
            let mut skill = tools::rejected(
                tools::ToolCall {
                    id: "skill-overlap".into(),
                    name: "skill_read".into(),
                    arguments: serde_json::json!({"name": "ego-browser"}),
                },
                serde_json::json!({"name": "ego-browser", "content": guide}).to_string(),
            );
            skill.success = true;
            let mut command = tools::rejected(
                tools::ToolCall {
                    id: "skill-overlap-command".into(),
                    name: "run_command".into(),
                    arguments: serde_json::json!({"program": "ego-browser", "args": ["skill"]}),
                },
                "usage guide output".into(),
            );
            command.success = true;
            host.run_mut(host.history.active).transcript =
                vec![generation::ContentBlock::ToolResult(skill)];
            host.run_mut(host.history.active)
                .transcript
                .push(generation::ContentBlock::Thinking("继续读取".into()));
            host.run_mut(host.history.active)
                .transcript
                .push(generation::ContentBlock::ToolResult(command));
            host.run_mut(host.history.active).pending = Some((
                host.history.active,
                cx.spawn(async move |_, _| std::future::pending::<()>().await),
            ));
            host
        });
        window.simulate_resize(size(px(1200.), px(700.)));
        window.run_until_parked();
        let panel = host.read_with(window, |host, _| {
            host.tool_panels
                .borrow()
                .iter()
                .find(|(key, _)| key.ends_with(":tool:skill-overlap"))
                .expect("skill panel")
                .1
                .panel
                .clone()
        });
        panel.update(window, |panel, cx| panel.set_expanded(true, cx));
        window.run_until_parked();
        if host.read_with(window, |host, _| host.scroll.offset().y) != px(0.) {
            host.update(window, |host, cx| {
                host.scroll.scroll_to_bottom();
                cx.notify();
            });
            window.run_until_parked();
        }
        let live = window.debug_bounds("live-response").unwrap();
        let waiting = window.debug_bounds("waiting-for-model").unwrap();
        let output = window.debug_bounds("tool-output").unwrap();
        assert!(
            output.size.height <= px(320.),
            "expanded tool output must clamp to its scroll viewport: {output:?}"
        );
        assert!(
            output.bottom() <= live.bottom() + px(1.),
            "expanded tool output escapes the live response: {output:?}, {live:?}"
        );
        assert!(
            waiting.top() >= live.bottom(),
            "waiting label overlaps the expanded skill card: {waiting:?}, {live:?}"
        );
        // A running command row that settles while the skill panel stays expanded
        // must still push the waiting row down instead of drawing underneath it.
        host.update(window, |host, cx| {
            let run = host.run_mut(host.history.active);
            if let Some(generation::ContentBlock::ToolResult(r)) = run
                .transcript
                .iter_mut()
                .find(|block| matches!(block, generation::ContentBlock::ToolResult(r) if r.call.id == "skill-overlap-command"))
            {
                r.result = "exit: 0".into();
                r.success = true;
                r.elapsed_ms = 600;
            }
            cx.notify();
        });
        window.run_until_parked();
        host.update(window, |host, cx| {
            host.scroll.scroll_to_bottom();
            cx.notify();
        });
        window.run_until_parked();
        let live = window.debug_bounds("live-response").unwrap();
        let waiting = window.debug_bounds("waiting-for-model").unwrap();
        assert!(
            waiting.top() >= live.bottom(),
            "waiting label overlaps the settled command row: {waiting:?}, {live:?}"
        );
    }

    #[gpui::test]
    fn clicking_a_message_image_opens_the_viewer_and_escape_closes_it(
        cx: &mut gpui::TestAppContext,
    ) {
        use base64::Engine as _;
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        // 300x200: the 160x120 thumbnail crops it, the viewer must not.
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(300, 200, image::Rgba([40, 90, 160, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let data: std::sync::Arc<str> = base64::engine::general_purpose::STANDARD
            .encode(&png)
            .into();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(directory.path().join("sessions.json"), cx);
            host.history.active_mut().push_message(
                Role::User,
                "看看这张图".into(),
                vec![crate::model::MessageImage {
                    media_type: "image/png".into(),
                    data,
                    file: None,
                }],
            );
            host
        });
        window.simulate_resize(size(px(1000.), px(760.)));
        window.run_until_parked();
        assert!(
            window.debug_bounds("agent-image-lightbox").is_none(),
            "the viewer must stay closed until the image is clicked"
        );
        let thumbnail = window
            .debug_bounds("message-image:0:0")
            .expect("message image thumbnail");
        window.simulate_click(thumbnail.center(), Default::default());
        window.run_until_parked();
        let viewer = window
            .debug_bounds("agent-image-lightbox")
            .expect("image viewer after click");
        assert!(
            viewer.size.width >= px(990.) && viewer.size.height >= px(750.),
            "the viewer must cover the window: {viewer:?}"
        );
        assert!(host.read_with(window, |host, cx| host.image_preview.read(cx).is_open()));
        // The source is 300x200; the viewer must show it whole (the 160x120
        // thumbnail crops it), hence both sides grow and the 3:2 ratio holds.
        let shown = window
            .debug_bounds("agent-image-viewer")
            .expect("viewer image bounds");
        assert!(
            shown.size.width > px(280.) && shown.size.height > px(180.),
            "the viewer must enlarge the image: {shown:?}"
        );
        let ratio = f32::from(shown.size.width) / f32::from(shown.size.height);
        assert!(
            (ratio - 1.5).abs() < 0.05,
            "the viewer must preserve the aspect ratio: {shown:?}"
        );
        window.simulate_keystrokes("escape");
        window.run_until_parked();
        assert!(
            window.debug_bounds("agent-image-lightbox").is_none(),
            "Escape must close the viewer"
        );
        assert!(host.read_with(window, |host, cx| !host.image_preview.read(cx).is_open()));
        // Reopening and clicking the scrim also dismisses it.
        window.simulate_click(thumbnail.center(), Default::default());
        window.run_until_parked();
        assert!(window.debug_bounds("agent-image-lightbox").is_some());
        window.simulate_click(point(px(8.), px(8.)), Default::default());
        window.run_until_parked();
        assert!(
            window.debug_bounds("agent-image-lightbox").is_none(),
            "clicking the scrim must close the viewer"
        );
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
            host.message_layout.pin = None;
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
        assert!(
            waiting.bottom() <= viewport.bottom() + px(1.),
            "the last row must be reachable at the true scroll bottom: {waiting:?}, {viewport:?}"
        );
        assert!(
            viewport.bottom() <= composer.top() + px(1.),
            "the composer must not cover the chat viewport"
        );
        assert!(
            waiting.top() >= live.bottom(),
            "waiting label overlaps expanded live response: {waiting:?}, {live:?}"
        );
        // The newest thinking row now opens automatically. The older tool may
        // be outside the virtualized viewport at the bottom; scroll back to it.
        host.update(window, |host, cx| {
            host.scroll.scroll_to_top_of_item(1);
            cx.notify();
        });
        window.run_until_parked();
        let live = window.debug_bounds("live-response").unwrap();
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

        first_sender
            .try_send(tools::ToolEvent::Transcript(vec![
                generation::ContentBlock::Text("first".into()),
            ]))
            .unwrap();
        second_sender
            .try_send(tools::ToolEvent::Transcript(vec![
                generation::ContentBlock::Text("second".into()),
            ]))
            .unwrap();
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
            let blocks =
                std::mem::take(&mut workspace.run_mut(workspace.history.active).transcript);
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
    fn expanded_history_process_culls_rows_after_scroll_and_session_switch(
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
            workspace
                .history
                .active_mut()
                .push(Role::Agent, "done".into());
            workspace.history.active_mut().messages[0].blocks = (0..300)
                .map(|i| {
                    let record = tools::rejected(
                        tools::ToolCall {
                            id: format!("row-{i}"),
                            name: "read_file".into(),
                            arguments: serde_json::json!({"path":format!("file-{i}.rs")}),
                        },
                        "example output".into(),
                    );
                    generation::ContentBlock::ToolResult(record)
                })
                .collect();
            workspace
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.run_until_parked();
        let original = workspace.read_with(window, |workspace, _| workspace.history.active);
        let key = format!("{original}:0");
        workspace.update(window, |workspace, cx| {
            workspace.process_panels.borrow()[&key]
                .panel
                .update(cx, |panel, cx| panel.set_expanded(true, cx));
            // Explicitly read the top; opening a process otherwise follows the end.
            workspace.message_layout.pin = None;
            workspace.scroll.scroll_to_top_of_item(0);
            cx.notify();
        });
        window.run_until_parked();
        assert!(window.debug_bounds("history-row-0").is_some());
        assert!(window.debug_bounds("history-spacer-299").is_some());
        let height = workspace.read_with(window, |w, _| w.scroll.max_offset().y);
        workspace.update(window, |w, cx| {
            w.scroll.scroll_to_bottom();
            cx.notify();
        });
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
        assert!(window.debug_bounds("history-spacer-0").is_some());
        assert!(window.debug_bounds("history-row-299").is_some());
        window.simulate_resize(size(px(1000.), px(700.)));
        window.run_until_parked();
        assert!(window.debug_bounds("history-row-299").is_some());
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
        workspace.update(window, |this, cx| {
            this.switch(original, cx);
            assert_eq!(
                this.message_layout.context.unwrap().0,
                original,
                "warm switch must reuse measured geometry before its first frame"
            );
            assert_eq!(this.message_layout.measure_passes, 0);
            assert!(this.scroll.bounds_for_item(0).is_some());
            assert!(this.scroll.max_offset().y > px(400.));
        });
        window.run_until_parked();
        workspace.read_with(window, |this, _| {
            assert!(this.scroll.max_offset().y > px(400.))
        });
    }

    #[gpui::test]
    fn switching_to_a_conversation_opens_it_at_the_latest_message(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (workspace, window) = cx.add_window_view(|_, cx| {
            let mut workspace =
                AgentWorkspace::with_path(directory.path().join("sessions.json"), cx);
            // Keep the GPUI test scheduler independent of disk writes.
            workspace.storage_readable = false;
            for index in 0..30 {
                workspace.history.active_mut().push(
                    Role::User,
                    format!("History {index} with variable wrapping and paragraphs.\n\n").repeat(6),
                );
            }
            workspace
        });
        let view = workspace.clone();
        let assert_at_latest = move |window: &mut gpui::VisualTestContext| {
            view.read_with(window, |this, _| {
                assert!(
                    this.scroll.max_offset().y > px(400.),
                    "the conversation needs more history than the viewport"
                );
                assert!(
                    this.scroll.max_offset().y + this.scroll.offset().y <= px(2.),
                    "a conversation must open at its newest message, not its first task row"
                );
            });
        };

        // The session restored on launch opens at its newest message.
        window.simulate_resize(size(px(1000.), px(700.)));
        window.run_until_parked();
        assert_at_latest(window);
        assert!(
            window.debug_bounds("message-spacer-0").is_some(),
            "the oldest rows should stay virtualized above the newest message"
        );

        let second = workspace.update(window, |this, _| {
            let mut session = model::Session::new();
            for index in 0..30 {
                session.push(
                    Role::User,
                    format!("Second {index} with variable wrapping and paragraphs.\n\n").repeat(6),
                );
            }
            let second = session.id;
            this.history.sessions.insert(0, session);
            second
        });
        let first = workspace.read_with(window, |this, _| this.history.active);
        workspace.update(window, |this, cx| this.switch(second, cx));
        window.run_until_parked();
        assert_at_latest(window);

        // Scrolling away keeps the reader's position instead of pulling them
        // back to the end.
        let viewport = workspace.read_with(window, |this, _| this.scroll.bounds());
        window.simulate_event(gpui::ScrollWheelEvent {
            position: viewport.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(600.))),
            ..Default::default()
        });
        window.run_until_parked();
        workspace.read_with(window, |this, _| {
            assert!(
                this.message_layout.pin.is_none(),
                "scrolling cancels the pin"
            );
            assert!(
                this.scroll.max_offset().y + this.scroll.offset().y > px(2.),
                "the reader must keep the position they scrolled to"
            );
        });

        // Reading the top of a session and then switching away must not leak
        // that position into the next conversation.
        workspace.update(window, |this, cx| {
            this.scroll.scroll_to_top_of_item(0);
            cx.notify();
        });
        window.run_until_parked();
        workspace.read_with(window, |this, _| {
            assert!(
                this.scroll.max_offset().y + this.scroll.offset().y > px(400.),
                "the reader is at the top of the conversation"
            );
        });
        workspace.update(window, |this, cx| this.switch(first, cx));
        window.run_until_parked();
        assert_at_latest(window);

        // A new, still empty session has nothing to scroll and must not drag
        // the conversation that is opened next.
        workspace.update(window, |this, cx| {
            this.history.new_session_for(None);
            let id = this.history.active;
            this.switch(id, cx);
        });
        window.run_until_parked();
        workspace.read_with(window, |this, _| {
            assert_eq!(
                this.scroll.max_offset().y,
                px(0.),
                "an empty session has nothing to scroll"
            );
        });
        workspace.update(window, |this, cx| this.switch(second, cx));
        window.run_until_parked();
        assert_at_latest(window);
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

    #[gpui::test]
    fn command_panel_header_and_body_and_description_compatibility(cx: &mut gpui::TestAppContext) {
        // 1. Description in arguments
        let mut with_desc = tools::ToolRecord {
            call: tools::ToolCall {
                id: "c1".into(),
                name: "run_command".into(),
                arguments: serde_json::json!({
                    "program": "cargo",
                    "args": ["test", "--features", "webview", "grouped_options"],
                    "description": "run grouped_options test",
                    "cwd": "/Users/ygy/Workspace/gpui-glint"
                }),
            },
            result: "running 1 test\ntest ok".into(),
            success: true,
            elapsed_ms: 200,
            live_output: None,
            nested_tools: Default::default(),
            subagent: None,
            change: None,
        };
        let view = tool_presentation(&with_desc);
        assert_eq!(view.summary, "已运行 run grouped_options test · 0.2s");

        // 2. Running state with description
        with_desc.result = "执行中…".into();
        let running_view = tool_presentation(&with_desc);
        assert_eq!(running_view.summary, "正在运行 run grouped_options test");

        // 3. toolSummary alias
        let with_tool_summary = tools::ToolRecord {
            call: tools::ToolCall {
                id: "c2".into(),
                name: "run_command".into(),
                arguments: serde_json::json!({
                    "program": "git",
                    "args": ["status"],
                    "toolSummary": "check git status"
                }),
            },
            result: "On branch main".into(),
            success: true,
            elapsed_ms: 100,
            live_output: None,
            nested_tools: Default::default(),
            subagent: None,
            change: None,
        };
        assert_eq!(
            tool_presentation(&with_tool_summary).summary,
            "已运行 check git status · 0.1s"
        );

        // 4. Historical data without description falls back to command_summary
        let historical = tools::ToolRecord {
            call: tools::ToolCall {
                id: "c3".into(),
                name: "run_command".into(),
                arguments: serde_json::json!({
                    "program": "cargo",
                    "args": ["check", "-p", "glint-agent"]
                }),
            },
            result: "Finished dev".into(),
            success: true,
            elapsed_ms: 500,
            live_output: None,
            nested_tools: Default::default(),
            subagent: None,
            change: None,
        };
        assert_eq!(
            tool_presentation(&historical).summary,
            "已运行 cargo check -p glint-agent · 0.5s"
        );

        // 5. CommandLine / command compatibility
        let cmd_line_record = tools::ToolRecord {
            call: tools::ToolCall {
                id: "c4".into(),
                name: "run_command".into(),
                arguments: serde_json::json!({
                    "CommandLine": "git diff --stat"
                }),
            },
            result: "1 file changed".into(),
            success: true,
            elapsed_ms: 150,
            live_output: None,
            nested_tools: Default::default(),
            subagent: None,
            change: None,
        };
        assert_eq!(
            tool_presentation(&cmd_line_record).summary,
            "已运行 git diff --stat · 0.1s"
        );
        assert_eq!(command_text(&cmd_line_record.call), "git diff --stat");

        // 6. UI rendering: Command Card with header, copy button and body
        struct CommandPreview {
            _host: Entity<AgentWorkspace>,
            panel: Entity<gpui_glint::components::CollapsePanel>,
        }
        impl Render for CommandPreview {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().child(self.panel.clone())
            }
        }
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let project_dir = directory.path().join("gpui-glint");
        std::fs::create_dir(&project_dir).unwrap();
        with_desc.result = "running 1 test\ntest ok".into();
        let (_preview, window) = cx.add_window_view(|_, cx| {
            let host = cx.new(|cx| {
                let mut host =
                    AgentWorkspace::with_path(directory.path().join("sessions.json"), cx);
                host.history.active_mut().project = Some(project_dir.clone());
                host
            });
            let panel = host.update(cx, |h, cx| h.tool_panel("test-key", &with_desc, cx));
            panel.update(cx, |p, cx| p.set_expanded(true, cx));
            CommandPreview {
                _host: host,
                panel,
            }
        });
        window.run_until_parked();

        assert!(window.debug_bounds("command-card").is_some());
        assert!(window.debug_bounds("command-card-header").is_some());
        assert!(window.debug_bounds("command-card-copy-btn").is_some());
        assert!(window.debug_bounds("tool-output").is_some());

        // 7. Prompt formatting
        let prompt = format_command_prompt(&with_desc.call, Some(&project_dir));
        assert_eq!(prompt, "~/.../gpui-glint $ ");

        // 8. Copy button click
        let copy_btn = window.debug_bounds("command-card-copy-btn").unwrap();
        window.simulate_click(copy_btn.center(), Default::default());
        window.run_until_parked();
        let clipboard = cx.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(
            clipboard.as_deref(),
            Some("cargo test --features webview grouped_options")
        );
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
