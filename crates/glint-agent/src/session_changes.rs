//! Recovery view of file writes recorded by this agent, read back from the
//! saved tool log. Every `write_file`/`replace_in_file` keeps the complete
//! content from before the write, which is what makes recovering from an
//! accidental overwrite possible without a version control reset.
use crate::database;
use crate::edits::Change;
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;
use uuid::Uuid;

const PAGE_CHARS: usize = 4_000;
const MAX_PAGE_CHARS: usize = 20_000;
const MAX_PATHS: usize = 100;
const MAX_RECORDS: usize = 200;
const MAX_ALERTS: usize = 20;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    /// Defaults to the session executing this tool.
    session_id: Option<Uuid>,
    /// Case-insensitive path or label substring. Omit to scan every path.
    path: Option<String>,
    /// Return the full before/after content of one recorded write.
    change_id: Option<Uuid>,
    #[serde(default)]
    start_record: u32,
    limit: Option<u32>,
    #[serde(default)]
    offset: u32,
    chars: Option<u32>,
}

#[derive(Deserialize)]
struct Payload {
    #[serde(default)]
    tools: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    #[serde(default)]
    nested_tools: Vec<Entry>,
    call: Call,
    #[serde(default)]
    change: Option<Change>,
    #[serde(default = "successful")]
    success: bool,
    #[serde(default)]
    subagent: Option<Trace>,
}

#[derive(Deserialize)]
struct Trace {
    #[serde(default)]
    tools: Vec<Entry>,
}

#[derive(Deserialize)]
struct Call {
    name: String,
}

fn successful() -> bool {
    true
}
struct Found {
    message: u32,
    tool: String,
    subagent: bool,
    change: Change,
    before_chars: Option<usize>,
    after_chars: usize,
    warning: Option<&'static str>,
    live: bool,
    reverted: bool,
    journal_status: String,
}

#[derive(Clone)]
pub struct LiveChange {
    pub change: Change,
    pub tool: String,
    pub subagent: bool,
}

struct PathSummary {
    path: String,
    label: String,
    records: usize,
    before_chars: Option<usize>,
    after_chars: usize,
    alerted: usize,
    reverted: usize,
    last_message: u32,
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

/// Flag the two shapes worth double-checking: creating a file, and replacing a
/// file with drastically less content. The second is how accidental full-file
/// overwrites show up in this log.
fn flag(before: Option<usize>, after: usize) -> Option<&'static str> {
    match before {
        None => Some("此前文件不存在：恢复应删除文件，而不是写回内容"),
        Some(before) if before > after.saturating_mul(4).max(400) => {
            Some("写入后内容大幅缩小，可能是误覆盖：用 change_id 读取 before 内容恢复")
        }
        _ => None,
    }
}

fn record(found: &Found) -> Value {
    json!({
        "change_id": found.change.id,
        "message": (!found.live).then_some(found.message),
        "tool": found.tool,
        "subagent": found.subagent,
        "path": found.change.path.to_string_lossy(),
        "label": found.change.label,
        "before_chars": found.before_chars,
        "after_chars": found.after_chars,
        "live": found.live,
        "reverted":found.reverted,
        "journal_status":found.journal_status,
        "needs_verification":matches!(found.journal_status.as_str(),"prepared"|"uncertain"|"failed"),
        "added": found.change.added,
        "removed": found.change.removed,
        "flag": found.warning,
    })
}

/// `current` is the session running this request, so an agent recovering from
/// its own mistake does not need to know its own session ID first.
#[cfg(test)]
pub fn read(
    anchor: &Path,
    project: Option<&Path>,
    partner: Option<Uuid>,
    current: Option<Uuid>,
    args: &Value,
) -> Result<String, String> {
    read_live(anchor, project, partner, current, args, &[])
}
pub fn read_live(
    anchor: &Path,
    project: Option<&Path>,
    partner: Option<Uuid>,
    current: Option<Uuid>,
    args: &Value,
    live: &[LiveChange],
) -> Result<String, String> {
    let args: Args = serde_json::from_value(args.clone())
        .map_err(|error| format!("变更查询参数无效：{error}"))?;
    let project = project.ok_or("请先选择项目，再查看会话变更")?;
    let session = args
        .session_id
        .or(current)
        .ok_or("无法确定当前会话，请提供 session_id")?;
    if args.change_id.is_none() && (args.offset != 0 || args.chars.is_some()) {
        return Err("offset/chars 只在提供了 change_id 时使用".into());
    }
    let chars_limit = args.chars.unwrap_or(PAGE_CHARS as u32) as usize;
    if !(1..=MAX_PAGE_CHARS).contains(&chars_limit) {
        return Err(format!("chars 必须在 1 到 {MAX_PAGE_CHARS} 之间"));
    }
    let limit = args.limit.unwrap_or(MAX_RECORDS as u32) as usize;
    if !(1..=MAX_RECORDS).contains(&limit) {
        return Err("limit 必须在 1 到 200 之间".into());
    }
    if (args.start_record != 0 || args.limit.is_some())
        && (args.path.is_none() || args.change_id.is_some())
    {
        return Err("start_record/limit 只用于 path 写入记录分页".into());
    }
    let mut database = database::open(anchor)?;
    let connection = database.transaction().map_err(|error| error.to_string())?;
    let (title,session_payload): (String,String) = connection
        .query_row(
            "SELECT title,payload FROM sessions WHERE id=?1 AND project=?2 AND (?3 IS NULL OR json_extract(payload,'$.partner_id') IS NULL OR json_extract(payload,'$.partner_id')=?3)",
            params![session.to_string(), project.to_string_lossy(), partner.map(|id| id.to_string())],
            |row| Ok((row.get(0)?,row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or("会话不存在、已删除或不属于当前项目")?;
    let payload: Value = serde_json::from_str(&session_payload).map_err(|e| e.to_string())?;
    let reverted: std::collections::BTreeSet<Uuid> =
        serde_json::from_value(payload["reverted_changes"].clone()).unwrap_or_default();
    let total: u32 = connection
        .query_row(
            "SELECT count(*) FROM messages WHERE session_id=?1",
            [session.to_string()],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    // Only payloads mentioning a change record need parsing; tool logs without
    // writes are skipped in SQL instead of being deserialized here.
    let mut found = Vec::new();
    let mut statement = connection
        .prepare(
            "SELECT ordinal,payload FROM messages WHERE session_id=?1 AND instr(payload,?2)>0 AND (?3 IS NULL OR instr(payload,?3)>0) ORDER BY ordinal",
        )
        .map_err(|error| error.to_string())?;
    let mut rows = statement
        .query(params![
            session.to_string(),
            "\"change\"",
            args.change_id.map(|id| id.to_string())
        ])
        .map_err(|error| error.to_string())?;
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let ordinal: u32 = row.get(0).map_err(|error| error.to_string())?;
        let payload: String = row.get(1).map_err(|error| error.to_string())?;
        let Ok(payload) = serde_json::from_str::<Payload>(&payload) else {
            continue;
        };
        collect(ordinal, &payload.tools, false, args.change_id, &mut found);
    }
    drop(rows);
    drop(statement);
    if Some(session) == current {
        let mut ids: std::collections::HashSet<_> = found.iter().map(|f| f.change.id).collect();
        for item in live {
            if ids.insert(item.change.id) && args.change_id.is_none_or(|id| id == item.change.id) {
                found.push(found_change(
                    total,
                    item.tool.clone(),
                    item.subagent,
                    item.change.clone(),
                    args.change_id.is_some(),
                    true,
                ));
            }
        }
    }
    let journal = crate::edit_journal::entries(anchor, project, partner, session, args.change_id)?;
    let mut ids: std::collections::HashSet<_> = found.iter().map(|f| f.change.id).collect();
    for entry in journal {
        if !ids.insert(entry.change.id) {
            continue;
        }
        let mut item = found_change(
            total,
            entry.tool,
            entry.subagent,
            entry.change,
            args.change_id.is_some(),
            false,
        );
        item.before_chars = entry.before_chars;
        item.after_chars = entry.after_chars;
        item.warning = flag(item.before_chars, item.after_chars);
        item.journal_status = entry.status;
        found.push(item);
    }
    for found in &mut found {
        found.reverted = reverted.contains(&found.change.id);
    }
    if let Some(id) = args.change_id {
        if let Some(path) = args.path.as_deref() {
            let query = path.to_lowercase();
            if !found.iter().any(|f| {
                f.change.id == id
                    && (f.change.label.to_lowercase().contains(&query)
                        || f.change
                            .path
                            .to_string_lossy()
                            .to_lowercase()
                            .contains(&query))
            }) {
                return Err("change_id 不属于指定路径".into());
            }
        }
        return content(
            session,
            total,
            id,
            &found,
            args.offset as usize,
            chars_limit,
        );
    }
    let query = args.path.as_deref().map(str::to_lowercase);
    let matched: Vec<&Found> = found
        .iter()
        .filter(|found| match &query {
            None => true,
            Some(query) => {
                found.change.label.to_lowercase().contains(query)
                    || found
                        .change
                        .path
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(query)
            }
        })
        .collect();
    let mut summaries: Vec<PathSummary> = Vec::new();
    let mut path_indices = std::collections::HashMap::new();
    for item in &matched {
        let key = item.change.path.to_string_lossy().into_owned();
        let index = match path_indices.get(&key).copied() {
            Some(index) => index,
            None => {
                path_indices.insert(key.clone(), summaries.len());
                summaries.push(PathSummary {
                    path: key,
                    label: item.change.label.clone(),
                    records: 0,
                    before_chars: item.before_chars,
                    after_chars: 0,
                    alerted: 0,
                    reverted: 0,
                    last_message: item.message,
                });
                summaries.len() - 1
            }
        };
        let summary = &mut summaries[index];
        summary.records += 1;
        summary.reverted += usize::from(item.reverted);
        summary.after_chars = item.after_chars;
        summary.alerted += usize::from(item.warning.is_some());
        summary.last_message = item.message;
        summary.label = item.change.label.clone();
    }
    let truncated = summaries.len() > MAX_PATHS;
    let paths: Vec<Value> = summaries
        .iter()
        .take(MAX_PATHS)
        .map(|summary| {
            json!({
                "path": summary.path,
                "label": summary.label,
                "records": summary.records,
                "active_records":summary.records-summary.reverted,
                "reverted_records":summary.reverted,
                "first_before_chars": summary.before_chars,
                "last_after_chars": summary.after_chars,
                "delta_chars": summary.after_chars as i64 - summary.before_chars.unwrap_or(0) as i64,
                "flagged": summary.alerted,
                "last_message": (summary.last_message<total).then_some(summary.last_message),
            })
        })
        .collect();
    let alerts: Vec<Value> = matched
        .iter()
        .filter(|found| found.warning.is_some())
        .take(MAX_ALERTS)
        .map(|found| record(found))
        .collect();
    // A path-scoped request is the follow-up to a scan, so it lists the writes.
    let records: Option<Vec<Value>> = query.as_ref().map(|_| {
        matched
            .iter()
            .skip(args.start_record as usize)
            .take(limit)
            .map(|found| record(found))
            .collect()
    });
    let notice = if found.is_empty() {
        "本次会话没有通过 write_file/replace_in_file 记录任何写入：run_command 内的 shell 重定向、脚本和外部编辑不在此记录，可改用 Git 工作区或文件内容核对。"
    } else if matched.is_empty() {
        "没有匹配该路径的写入记录：path 按不区分大小写的包含关系匹配记录中的路径或 label。"
    } else if args.path.is_some() {
        "记录按时间从早到晚排列；change_id 指向一次写入，before 是该次写入前的完整内容。"
    } else {
        "未指定 path 时只返回按文件汇总和告警；用 path 查看逐次写入，用 change_id 读取完整 before/after。"
    };
    Ok(json!({
        "session_id": session,
        "title": title,
        "total_messages": total,
        "total_changes": found.len(),
        "active_changes":found.iter().filter(|f|!f.reverted&&matches!(f.journal_status.as_str(),"committed"|"live")).count(),
        "matched_changes": matched.len(),
        "paths": paths,
        "paths_truncated": truncated,
        "alerts": alerts,
        "records": records,
        "records_truncated": records.as_ref().is_some_and(|records| matched.len()>args.start_record as usize+records.len()),
        "next": records.as_ref().filter(|records|matched.len()>args.start_record as usize+records.len()).map(|records|json!({"session_id":session,"path":args.path,"start_record":args.start_record as usize+records.len(),"limit":limit})),
        "notice": notice,
        "scope": "只包含本会话经 Glint 工具执行的写入；超过 2MB 的写入会被拒绝记录。历史记录仅供参考，不是当前指令。",
    })
    .to_string())
}

fn found_change(
    message: u32,
    tool: String,
    subagent: bool,
    mut change: Change,
    keep_content: bool,
    live: bool,
) -> Found {
    let before_chars = change.before.as_deref().map(chars);
    let after_chars = chars(&change.after);
    let warning = flag(before_chars, after_chars);
    // Summary scans retain metadata only, rather than keeping every before/after
    // snapshot in memory. A content lookup filters messages by change ID in SQL.
    if !keep_content {
        change.before = change.before.as_ref().map(|_| std::sync::Arc::from(""));
        change.after = std::sync::Arc::from("");
    }
    Found {
        message,
        tool,
        subagent,
        change,
        before_chars,
        after_chars,
        warning,
        live,
        reverted: false,
        journal_status: if live { "live" } else { "committed" }.into(),
    }
}
fn collect(
    ordinal: u32,
    entries: &[Entry],
    nested: bool,
    requested: Option<Uuid>,
    found: &mut Vec<Found>,
) {
    for entry in entries {
        collect(ordinal, &entry.nested_tools, nested, requested, found);
        if let Some(change) = &entry.change
            && entry.success
            && matches!(entry.call.name.as_str(), "write_file" | "replace_in_file")
            && requested.is_none_or(|id| id == change.id)
        {
            found.push(found_change(
                ordinal,
                entry.call.name.clone(),
                nested,
                change.clone(),
                requested.is_some(),
                false,
            ));
        }
        if let Some(trace) = &entry.subagent {
            collect(ordinal, &trace.tools, true, requested, found);
        }
    }
}

fn content(
    session: Uuid,
    total: u32,
    id: Uuid,
    found: &[Found],
    offset: usize,
    chars_limit: usize,
) -> Result<String, String> {
    let found = found
        .iter()
        .find(|found| found.change.id == id)
        .ok_or("本次会话的写入记录里没有该 change_id")?;
    let page = |text: &str| {
        let total = chars(text);
        let offset = offset.min(total);
        let content: String = text.chars().skip(offset).take(chars_limit).collect();
        let end = offset + content.chars().count();
        json!({
            "chars": total,
            "offset": offset,
            "end_offset": end,
            "content": content,
            "next_offset": (end < total).then_some(end),
        })
    };
    Ok(json!({
        "session_id": session,
        "total_messages": total,
        "change_id": id,
        "message": (!found.live).then_some(found.message),
        "tool": found.tool,
        "subagent": found.subagent,
        "live":found.live,
        "reverted":found.reverted,
        "journal_status":found.journal_status,
        "needs_verification":matches!(found.journal_status.as_str(),"prepared"|"uncertain"|"failed"),
        "path": found.change.path.to_string_lossy(),
        "label": found.change.label,
        "added": found.change.added,
        "removed": found.change.removed,
        "before": found.change.before.as_deref().map(page),
        "after": page(&found.change.after),
        "notice": "prepared/uncertain/failed 只表示写入意图或未确认结果，须先核对当前文件。before 是该次写入前的完整内容；恢复时按当前权限、用户意图及文件版本校验写回 before；before 为 null 表示写入前文件不存在。offset/chars 同时作用于 before 与 after，next_offset 非空时继续读取该片段。",
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{History, Role, Session};
    use crate::tools::{ToolCall, ToolContext, ToolRecord};
    use std::sync::{Arc, Mutex};

    fn context(root: &Path, anchor: &Path) -> ToolContext {
        let (events, _) = async_channel::unbounded();
        ToolContext {
            steering: None,
            todos: Default::default(),
            subagents: Default::default(),
            delegation: Default::default(),
            registry: Default::default(),
            skills: Arc::new(crate::skills::Catalog::empty(anchor.to_owned())),
            memory: crate::memory::Repository::new(anchor.to_owned()),
            access: Arc::new(Mutex::new(crate::tools::AccessMode::AutoApprove)),
            root: Some(root.to_owned()),
            schedules: Arc::new(Mutex::new(
                crate::schedules::ScheduleStore::open(anchor.with_file_name("schedules.json"))
                    .unwrap(),
            )),
            provider_id: Uuid::new_v4(),
            model: "test".into(),
            events,
            recall: None,
            session_id: None,
            changes: Default::default(),
        }
    }

    fn record(call: &ToolCall, change: Change) -> ToolRecord {
        ToolRecord {
            call: call.clone(),
            result: "ok".into(),
            change: Some(change),
            subagent: None,
            success: true,
            elapsed_ms: 0,
            live_output: None,
            nested_tools: Default::default(),
        }
    }

    /// Writes a file, edits one line, then replaces the whole file with almost
    /// nothing - the shape of the accident this tool exists to undo.
    fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Uuid, Change, String) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        let root = root.canonicalize().unwrap();
        let anchor = directory.path().join("sessions.json");
        let context = context(&root, &anchor);
        let original = (0..300)
            .map(|index| format!("fn line_{index}() {{}}\n"))
            .collect::<String>();
        let create = ToolCall {
            id: "create".into(),
            name: "write_file".into(),
            arguments: json!({"path":"src/demo.rs","content":original}),
        };
        let (_, created) = crate::edits::apply(&create, &context).unwrap();
        let edit = ToolCall {
            id: "edit".into(),
            name: "replace_in_file".into(),
            arguments: json!({"path":"src/demo.rs","old":"fn line_7() {}","new":"fn line_7() { edited(); }"}),
        };
        let (_, edited) = crate::edits::apply(&edit, &context).unwrap();
        let overwrite = ToolCall {
            id: "overwrite".into(),
            name: "write_file".into(),
            arguments: json!({"path":"src/demo.rs","content":"// 误覆盖\n"}),
        };
        let (_, overwrote) = crate::edits::apply(&overwrite, &context).unwrap();
        let overwrote = overwrote.unwrap();
        let original = overwrote.before.as_deref().unwrap().to_owned();
        let mut session = Session::new();
        session.project = Some(root.clone());
        session.push(Role::User, "改动 src/demo.rs".into());
        session.push(
            Role::Agent,
            "已写入，并提取了行区间：a.rs:1:5、a.rs:5".into(),
        );
        session.messages[1].tools = vec![
            record(&create, created.unwrap()),
            record(&edit, edited.unwrap()),
            record(&overwrite, overwrote.clone()),
        ];
        let id = session.id;
        let mut other = Session::new();
        other.project = Some(root.join("elsewhere"));
        other.push(Role::User, "另一个项目".into());
        let history = History {
            sessions: vec![session, other],
            active: id,
            projects: vec![root.clone()],
            ..Default::default()
        };
        database::save_history(&anchor, &history).unwrap();
        (directory, anchor, id, overwrote, original)
    }

    fn read(anchor: &Path, root: &Path, session: Option<Uuid>, args: Value) -> Value {
        serde_json::from_str(
            &super::read(anchor, Some(root), None, session, &args)
                .unwrap_or_else(|error| panic!("{error}")),
        )
        .unwrap()
    }

    #[test]
    fn reads_write_ahead_snapshots_without_saved_tool_result_and_marks_uncertain() {
        let (_dir, anchor, id, mut change, _) = fixture();
        let root = anchor
            .parent()
            .unwrap()
            .join("project")
            .canonicalize()
            .unwrap();
        let mut context = context(&root, &anchor);
        context.session_id = Some(id);
        change.id = Uuid::new_v4();
        change.before = Some(Arc::from("尚未保存的原文"));
        change.after = Arc::from("new");
        crate::edit_journal::prepare(&context, &change, "write_file").unwrap();
        let summary = read(&anchor, &root, Some(id), json!({"path":"demo.rs"}));
        assert_eq!(summary["total_changes"], 4);
        let details = read(&anchor, &root, Some(id), json!({"change_id":change.id}));
        assert_eq!(details["journal_status"], "prepared");
        assert_eq!(details["needs_verification"], true);
        assert_eq!(details["before"]["content"], "尚未保存的原文");
        crate::edit_journal::finish(&anchor, change.id, "committed", None).unwrap();
        let summary = read(&anchor, &root, Some(id), json!({}));
        assert_eq!(summary["active_changes"], 4);
    }
    #[tokio::test]
    async fn registered_tool_reads_live_writes_pages_records_and_enforces_scopes() {
        let (_dir, anchor, id, overwrote, _) = fixture();
        let root = anchor
            .parent()
            .unwrap()
            .join("project")
            .canonicalize()
            .unwrap();
        let mut context = context(&root, &anchor);
        context.session_id = Some(id);
        for anthropic in [true, false] {
            let defs = crate::tools::definitions(anthropic);
            assert!(defs.as_array().unwrap().iter().any(|d| if anthropic {
                d["name"] == "session_changes"
            } else {
                d["function"]["name"] == "session_changes"
            }));
        }
        let call = ToolCall {
            id: "history".into(),
            name: "session_changes".into(),
            arguments: json!({}),
        };
        let scan = crate::tools::execute(call.clone(), &context).await;
        assert!(scan.success, "{}", scan.result);
        assert_eq!(
            serde_json::from_str::<Value>(&scan.result).unwrap()["total_changes"],
            3
        );
        let write = crate::tools::execute(
            ToolCall {
                id: "live-write".into(),
                name: "write_file".into(),
                arguments: json!({"path":"src/demo.rs","content":"甲乙丙丁\nnext"}),
            },
            &context,
        )
        .await;
        assert!(write.success, "{}", write.result);
        let change = write.change.unwrap();
        let scan = crate::tools::execute(
            ToolCall {
                arguments: json!({"path":"demo.rs","limit":2}),
                ..call.clone()
            },
            &context,
        )
        .await;
        let page: Value = serde_json::from_str(&scan.result).unwrap();
        assert_eq!(page["total_changes"], 4);
        assert_eq!(page["records"].as_array().unwrap().len(), 2);
        let next = crate::tools::execute(
            ToolCall {
                arguments: page["next"].clone(),
                ..call.clone()
            },
            &context,
        )
        .await;
        let next: Value = serde_json::from_str(&next.result).unwrap();
        assert_eq!(next["records"].as_array().unwrap().len(), 2);
        assert_eq!(next["records"][1]["live"], true);
        assert!(next["next"].is_null());
        for mode in [
            crate::tools::AccessMode::Ask,
            crate::tools::AccessMode::AutoApprove,
            crate::tools::AccessMode::FullAccess,
        ] {
            *context.access.lock().unwrap() = mode;
            let result = crate::tools::execute(
                ToolCall {
                    arguments: json!({"change_id":change.id,"offset":1,"chars":2}),
                    ..call.clone()
                },
                &context,
            )
            .await;
            assert!(result.success, "{}", result.result);
            let content: Value = serde_json::from_str(&result.result).unwrap();
            assert_eq!(content["after"]["content"], "乙丙");
            assert_eq!(content["live"], true);
            let foreign = Uuid::parse_str(
                &database::open(&anchor)
                    .unwrap()
                    .query_row(
                        "SELECT id FROM sessions WHERE id<>?1",
                        [id.to_string()],
                        |r| r.get::<_, String>(0),
                    )
                    .unwrap(),
            )
            .unwrap();
            assert!(
                !crate::tools::execute(
                    ToolCall {
                        arguments: json!({"session_id":foreign}),
                        ..call.clone()
                    },
                    &context
                )
                .await
                .success
            );
        }
        assert!(
            !crate::tools::execute(
                ToolCall {
                    arguments: json!({"path":"unrelated.rs","change_id":overwrote.id}),
                    ..call.clone()
                },
                &context
            )
            .await
            .success
        );
        assert_eq!(
            std::fs::read_to_string(root.join("src/demo.rs")).unwrap(),
            "甲乙丙丁\nnext"
        );
    }
    #[test]
    fn companion_scope_and_live_deduplication_are_preserved() {
        let (_dir, anchor, id, change, _) = fixture();
        let root = anchor
            .parent()
            .unwrap()
            .join("project")
            .canonicalize()
            .unwrap();
        let live = vec![LiveChange {
            change: change.clone(),
            tool: "write_file".into(),
            subagent: true,
        }];
        let value: Value = serde_json::from_str(
            &super::read_live(&anchor, Some(&root), None, Some(id), &json!({}), &live).unwrap(),
        )
        .unwrap();
        assert_eq!(
            value["total_changes"], 3,
            "a write already saved is not counted twice"
        );
        let owner = Uuid::new_v4();
        let other = Uuid::new_v4();
        database::open(&anchor)
            .unwrap()
            .execute(
                "UPDATE sessions SET payload=json_set(payload,'$.partner_id',?1) WHERE id=?2",
                params![owner.to_string(), id.to_string()],
            )
            .unwrap();
        assert!(
            super::read_live(
                &anchor,
                Some(&root),
                Some(other),
                Some(id),
                &json!({}),
                &live
            )
            .is_err()
        );
        assert!(
            super::read_live(
                &anchor,
                Some(&root),
                Some(owner),
                Some(id),
                &json!({}),
                &live
            )
            .is_ok()
        );
    }
    #[test]
    fn nested_subagent_writes_are_included_without_parsing_tool_output_as_records() {
        let (_dir, anchor, id, _, _) = fixture();
        let root = anchor
            .parent()
            .unwrap()
            .join("project")
            .canonicalize()
            .unwrap();
        let db = database::open(&anchor).unwrap();
        let payload: String = db
            .query_row(
                "SELECT payload FROM messages WHERE session_id=?1 AND ordinal=1",
                [id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        let mut payload: Value = serde_json::from_str(&payload).unwrap();
        let tools = payload["tools"].take();
        payload["tools"] = json!([{"call":{"name":"subagent"},"subagent":{"tools":tools},"result":"a.rs:5 or directory/ is not a write"}]);
        db.execute(
            "UPDATE messages SET payload=?1 WHERE session_id=?2 AND ordinal=1",
            params![payload.to_string(), id.to_string()],
        )
        .unwrap();
        let value = read(&anchor, &root, Some(id), json!({"path":"demo.rs"}));
        assert_eq!(value["total_changes"], 3);
        assert!(
            value["records"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["subagent"] == true)
        );
    }
    #[test]
    fn scans_writes_and_stops_matching_on_references() {
        let (_directory, anchor, id, overwrote, original) = fixture();
        let root = anchor.parent().unwrap().join("project");
        let root = root.canonicalize().unwrap();
        let scan = read(&anchor, &root, Some(id), json!({}));
        assert_eq!(scan["session_id"], id.to_string());
        assert_eq!(scan["total_changes"], 3);
        assert_eq!(scan["paths"].as_array().unwrap().len(), 1);
        let path = &scan["paths"][0];
        assert_eq!(path["records"], 3);
        assert_eq!(path["flagged"], 2);
        assert!(path["path"].as_str().unwrap().ends_with("src/demo.rs"));
        let alerts = scan["alerts"].as_array().unwrap();
        assert_eq!(alerts.len(), 2);
        let overwritten = alerts
            .iter()
            .find(|alert| alert["change_id"] == overwrote.id.to_string())
            .unwrap();
        assert_eq!(overwritten["tool"], "write_file");
        assert_eq!(overwritten["before_chars"], original.chars().count());
        assert!(overwritten["flag"].as_str().unwrap().contains("大幅缩小"));
        let scoped = read(&anchor, &root, Some(id), json!({"path":"SRC/demo.RS"}));
        let records = scoped["records"].as_array().unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["before_chars"], Value::Null);
        assert!(records[0]["flag"].as_str().unwrap().contains("删除文件"));
        let missing = read(&anchor, &root, Some(id), json!({"path":"README.md"}));
        assert_eq!(missing["records"].as_array().unwrap().len(), 0);
        assert!(missing["notice"].as_str().unwrap().contains("没有匹配"));
    }

    #[test]
    fn content_recovers_previous_bytes_and_pages_them() {
        let (_directory, anchor, id, overwrote, original) = fixture();
        let root = anchor.parent().unwrap().join("project");
        let root = root.canonicalize().unwrap();
        let value = read(
            &anchor,
            &root,
            Some(id),
            json!({"change_id":overwrote.id,"chars":MAX_PAGE_CHARS}),
        );
        let before = &value["before"];
        assert_eq!(before["chars"], original.chars().count());
        assert_eq!(before["content"], original);
        assert!(before["next_offset"].is_null());
        assert_eq!(value["after"]["content"], "// 误覆盖\n");
        let page = read(
            &anchor,
            &root,
            Some(id),
            json!({"change_id":overwrote.id,"offset":0,"chars":16}),
        );
        assert_eq!(
            page["before"]["content"],
            original.chars().take(16).collect::<String>()
        );
        assert_eq!(page["before"]["next_offset"], 16);
        let rest = read(
            &anchor,
            &root,
            Some(id),
            json!({"change_id":overwrote.id,"offset":16,"chars":16}),
        );
        assert_eq!(
            rest["before"]["content"],
            original.chars().skip(16).take(16).collect::<String>()
        );
    }

    #[test]
    fn stays_project_scoped_and_reports_unusable_requests() {
        let (_directory, anchor, id, overwrote, _) = fixture();
        let root = anchor.parent().unwrap().join("project");
        let root = root.canonicalize().unwrap();
        let foreign = read(&anchor, &root, Some(id), json!({}));
        assert!(foreign["title"].as_str().is_some());
        // Another project's session must not be readable even by explicit id.
        let database = database::open(&anchor).unwrap();
        let other: String = database
            .query_row(
                "SELECT id FROM sessions WHERE project<>?1 LIMIT 1",
                [root.to_string_lossy()],
                |row| row.get(0),
            )
            .unwrap();
        let other = Uuid::parse_str(&other).unwrap();
        assert!(
            super::read(&anchor, Some(&root), None, Some(other), &json!({}))
                .unwrap_err()
                .contains("不属于当前项目")
        );
        assert!(
            super::read(&anchor, Some(&root), None, None, &json!({}))
                .unwrap_err()
                .contains("请提供 session_id")
        );
        assert!(
            super::read(&anchor, Some(&root), None, Some(id), &json!({"offset":4}))
                .unwrap_err()
                .contains("change_id")
        );
        assert!(
            super::read(
                &anchor,
                Some(&root),
                None,
                Some(id),
                &json!({"change_id":Uuid::new_v4()})
            )
            .unwrap_err()
            .contains("没有该 change_id")
        );
        assert!(
            super::read(
                &anchor,
                Some(&root),
                None,
                Some(id),
                &json!({"change_id":overwrote.id,"chars":MAX_PAGE_CHARS as u32+1})
            )
            .unwrap_err()
            .contains("chars")
        );
    }
}
