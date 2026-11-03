use crate::tools::{ToolCall, ToolContext, ToolRecord};
use gpui_glint::components::{DiffLine, DiffLineKind, diff_lines};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub id: uuid::Uuid,
    pub path: PathBuf,
    pub label: String,
    pub before: Option<Arc<str>>,
    pub after: Arc<str>,
    pub added: usize,
    pub removed: usize,
    #[serde(skip)]
    lines: Arc<OnceLock<Vec<DiffLine>>>,
}

impl Change {
    pub(super) fn preview(path: PathBuf, label: String, before: Option<Arc<str>>, after: Arc<str>, stats: Option<(usize, usize)>) -> Self {
        let mut change = Self { id: uuid::Uuid::new_v4(), path, label, before, after, added: 0, removed: 0, lines: Default::default() };
        if let Some((added, removed)) = stats { change.added = added; change.removed = removed; }
        else { change.count(); }
        change
    }
    pub fn lines(&self) -> Vec<DiffLine> {
        self.lines
            .get_or_init(|| diff_lines(self.before.as_deref().unwrap_or(""), &self.after))
            .clone()
    }
    fn count(&mut self) {
        let lines = self.lines();
        self.added = lines
            .iter()
            .filter(|line| line.kind == DiffLineKind::Added)
            .count();
        self.removed = lines
            .iter()
            .filter(|line| line.kind == DiffLineKind::Deleted)
            .count();
    }
}

pub fn apply(call: &ToolCall, context: &ToolContext) -> Result<(String, Option<Change>), String> {
    let argument = |key: &str| {
        call.arguments[key]
            .as_str()
            .ok_or_else(|| format!("缺少字符串参数 {key}"))
    };
    let label = argument("path")?;
    let path = crate::tools::resolve_file(context, label, true)?;
    let path = if path.exists() {
        path.canonicalize().map_err(|_| "无法解析目标文件")?
    } else {
        path.parent()
            .ok_or("目标目录无效")?
            .canonicalize()
            .map_err(|_| "目标目录不存在")?
            .join(path.file_name().ok_or("文件名无效")?)
    };
    let before = match std::fs::metadata(&path) {
        Ok(metadata) => {
            if metadata.len() > 2 * 1024 * 1024 {
                return Err("目标文件超过 2MB，无法安全记录修改前内容".into());
            }
            Some(std::fs::read_to_string(&path).map_err(|_| "无法读取目标 UTF-8 文件")?)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err("无法检查目标文件".into()),
    };
    if call
        .arguments
        .get("create_only")
        .is_some_and(|v| !v.is_boolean())
    {
        return Err("create_only 必须为布尔值".into());
    }
    if call.arguments["create_only"].as_bool() == Some(true) && before.is_some() {
        return Err("文件已存在，未覆盖".into());
    }
    if let Some(version) = call.arguments.get("expected_sha256") {
        let expected = version
            .as_str()
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("expected_sha256 必须为 64 位 SHA-256 字符串")?;
        if !before
            .as_deref()
            .map(crate::atomic_file::digest)
            .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
        {
            return Err("文件版本不匹配，请重新读取后重试".into());
        }
    }
    let after = if call.name == "write_file" {
        argument("content")?.to_owned()
    } else {
        let current = before.as_ref().ok_or("目标文件不存在")?;
        let old = argument("old")?;
        if old.is_empty() || current.matches(old).count() != 1 {
            return Err("待替换内容不存在或不唯一，未修改文件".into());
        }
        current.replacen(old, argument("new")?, 1)
    };
    if after.len() > 2 * 1024 * 1024 {
        return Err("单次写入超过 2MB".into());
    }
    if before.as_deref() == Some(after.as_str()) {
        return Ok(("文件内容未变化".into(), None));
    }
    let mut change = Change {
        id: uuid::Uuid::new_v4(),
        path,
        label: label.to_owned(),
        before: before.map(Arc::from),
        after: Arc::from(after),
        added: 0,
        removed: 0,
        lines: Default::default(),
    };
    change.count();
    crate::edit_journal::prepare(context, &change, &call.name)?;
    if let Err(error) =
        crate::atomic_file::replace(&change.path, change.before.as_deref(), &change.after)
    {
        let _ =
            crate::edit_journal::finish(&context.memory.anchor, change.id, "failed", Some(&error));
        return Err(error);
    }
    let journal_error =
        crate::edit_journal::finish(&context.memory.anchor, change.id, "committed", None).err();
    Ok((
        format!(
            "文件已更新，+{} -{} 行{}",
            change.added,
            change.removed,
            journal_error
                .map(|e| format!("；写前快照已保存，完成标记待核对：{e}"))
                .unwrap_or_default()
        ),
        Some(change),
    ))
}

/// Record a confirmed remote write using the same diff presentation as local edits.
pub(crate) fn remote_change(
    mut remote: crate::ssh_project::Remote,
    label: &str,
    value: serde_json::Value,
) -> Result<(String, Option<Change>), String> {
    let before = value["before"].as_str().map(Arc::from);
    let after: Arc<str> = Arc::from(value["after"].as_str().ok_or("远程写入响应无效")?);
    if before.as_deref() == Some(after.as_ref()) {
        return Ok(("文件内容未变化".into(), None));
    }
    remote.directory = value["path"].as_str().ok_or("远程写入路径无效")?.into();
    let mut change = Change {
        id: uuid::Uuid::new_v4(),
        path: remote.uri(),
        label: label.into(),
        before,
        after,
        added: 0,
        removed: 0,
        lines: Default::default(),
    };
    change.count();
    Ok((
        format!("远程文件已更新，+{} -{} 行", change.added, change.removed),
        Some(change),
    ))
}

pub(crate) async fn remote_workspace_changes(
    root: &Path,
) -> Result<(Vec<Change>, Vec<String>), String> {
    let remote = crate::ssh_project::Remote::from_path(root)?;
    let value = remote
        .action("workspace_changes", serde_json::json!({}), false)
        .await?;
    let rows = value["changes"].as_array().ok_or("远程 Git 响应无效")?;
    let mut changes = Vec::new();
    for row in rows {
        let label = row["label"].as_str().ok_or("远程 Git 文件名无效")?;
        if let Some(change) = remote_change(remote.clone(), label, row.clone())?.1 {
            changes.push(change);
        }
    }
    let notes = value["notes"]
        .as_array()
        .map(|notes| {
            notes
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    Ok((changes, notes))
}

/// Fold repeated writes into the net change from first content to last content.
/// Only these tools' observed edits are included; pre-existing Git changes are not.
#[cfg(test)]
fn aggregate(records: &[ToolRecord]) -> Vec<Change> {
    aggregate_records(records.iter().collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
    pub(crate) fn context(path: &Path) -> ToolContext {
        let (events, _) = async_channel::unbounded();
        ToolContext {
            steering: None,
            todos: Default::default(),
            subagents: Default::default(),
            delegation: Default::default(),
            registry: Default::default(),
            skills: Arc::new(crate::skills::Catalog::empty(path.join("sessions.json"))),
            memory: crate::memory::Repository::new(path.join("sessions.json")),
            access: Arc::new(std::sync::Mutex::new(crate::tools::AccessMode::AutoApprove)),
            root: Some(path.to_owned()),
            schedules: Arc::new(std::sync::Mutex::new(
                crate::schedules::ScheduleStore::open(path.join("schedules.json")).unwrap(),
            )),
            provider_id: uuid::Uuid::new_v4(),
            model: "test".into(),
            events,
            recall: None,
            session_id: None,
            changes: Default::default(),
        }
    }
    #[test]
    fn version_guard_refuses_stale_writes_and_journal_precedes_failed_commit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut context = context(&root);
        context.session_id = Some(uuid::Uuid::new_v4());
        let path = root.join("file.txt");
        std::fs::write(&path, "external").unwrap();
        let mut call = ToolCall {
            id: "write".into(),
            name: "write_file".into(),
            arguments: json!({"path":"file.txt","content":"next","expected_sha256":crate::atomic_file::digest("old")}),
        };
        assert!(apply(&call, &context).unwrap_err().contains("版本"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        call.arguments["expected_sha256"] = crate::atomic_file::digest("external").into();
        let (_, change) = apply(&call, &context).unwrap();
        let change = change.unwrap();
        let rows = crate::edit_journal::entries(
            &context.memory.anchor,
            &root,
            None,
            context.session_id.unwrap(),
            Some(change.id),
        )
        .unwrap();
        assert_eq!(rows[0].status, "committed");
        assert_eq!(rows[0].change.before.as_deref(), Some("external"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
            call.arguments["expected_sha256"] = crate::atomic_file::digest("next").into();
            call.arguments["content"] = "refused".into();
            assert!(apply(&call, &context).is_err());
            let rows = crate::edit_journal::entries(
                &context.memory.anchor,
                &root,
                None,
                context.session_id.unwrap(),
                None,
            )
            .unwrap();
            assert!(
                rows.iter()
                    .any(|r| r.status == "failed" && r.before_chars == Some(4))
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "next");
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
    }
    fn record(change: Change) -> ToolRecord {
        ToolRecord {
            call: ToolCall {
                id: uuid::Uuid::new_v4().to_string(),
                name: "write_file".into(),
                arguments: json!({"path":change.label}),
            },
            change: Some(change),
            subagent: None,
            result: "updated".into(),
            success: true,
            elapsed_ms: 0,
            live_output: None,
            nested_tools: Default::default(),
        }
    }
    #[test]
    fn observed_edits_coalesce_aliases_into_net_changes_and_survive_restart() {
        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path());
        let call = ToolCall {
            id: "first".into(),
            name: "write_file".into(),
            arguments: json!({"path":"demo.txt","content":"原始\n第二行\n"}),
        };
        let (_, first) = apply(&call, &context).unwrap();
        let first = first.unwrap();
        assert!(first.before.is_none());
        assert_eq!(first.added, 2);
        let replace = ToolCall {
            id: "second".into(),
            name: "replace_in_file".into(),
            arguments: json!({"path":"./demo.txt","old":"原始","new":"更新 😀"}),
        };
        let (_, second) = apply(&replace, &context).unwrap();
        let second = second.unwrap();
        assert_eq!(second.before.as_deref(), Some("原始\n第二行\n"));
        assert_eq!((second.added, second.removed), (1, 1));
        let records = vec![record(first), record(second)];
        let net = aggregate(&records);
        assert_eq!(net.len(), 1);
        assert!(net[0].before.is_none());
        assert_eq!(net[0].after.as_ref(), "更新 😀\n第二行\n");
        assert_eq!((net[0].added, net[0].removed), (2, 0));
        let mut history = crate::model::History::default();
        history
            .active_mut()
            .push(crate::model::Role::Agent, "done".into());
        history.active_mut().messages.last_mut().unwrap().tools = records;
        let path = directory.path().join("history/sessions.json");
        history.save(&path).unwrap();
        let loaded = crate::model::History::load(&path).unwrap();
        assert_eq!(
            aggregate(&loaded.active().messages[0].tools)[0]
                .after
                .as_ref(),
            "更新 😀\n第二行\n"
        );
        let no_op = ToolCall {
            arguments: json!({"path":"demo.txt","content":"更新 😀\n第二行\n"}),
            ..call.clone()
        };
        assert!(apply(&no_op, &context).unwrap().1.is_none());
        let invalid = ToolCall {
            arguments: json!({"path":"demo.txt","old":"missing","new":"bad"}),
            ..replace
        };
        assert!(apply(&invalid, &context).is_err());
        assert_eq!(
            std::fs::read_to_string(directory.path().join("demo.txt")).unwrap(),
            "更新 😀\n第二行\n"
        );
    }
    #[test]
    fn large_sparse_edits_keep_precise_line_counts_and_reverts_have_no_net_change() {
        let directory = tempfile::tempdir().unwrap();
        let context = context(directory.path());
        let initial = (0..1500)
            .map(|index| format!("line {index}\n"))
            .collect::<String>();
        std::fs::write(directory.path().join("large.txt"), &initial).unwrap();
        let replace = ToolCall {
            id: "replace".into(),
            name: "replace_in_file".into(),
            arguments: json!({"path":"large.txt","old":"line 750\n","new":"changed\n"}),
        };
        let (_, change) = apply(&replace, &context).unwrap();
        let change = change.unwrap();
        assert_eq!((change.added, change.removed), (1, 1));
        let restore = ToolCall {
            id: "restore".into(),
            name: "write_file".into(),
            arguments: json!({"path":"large.txt","content":initial}),
        };
        let (_, restored) = apply(&restore, &context).unwrap();
        assert!(aggregate(&[record(change), record(restored.unwrap())]).is_empty());
        let create = ToolCall {
            id: "empty".into(),
            name: "write_file".into(),
            arguments: json!({"path":"empty.txt","content":""}),
        };
        let (_, empty) = apply(&create, &context).unwrap();
        assert_eq!(aggregate(&[record(empty.unwrap())]).len(), 1);
    }

    #[test]
    fn workspace_diff_includes_staged_unstaged_deleted_renamed_and_untracked_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .current_dir(root)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&["config", "core.autocrlf", "false"]);
        for name in ["modify.txt", "delete.txt", "rename.txt"] {
            std::fs::write(root.join(name), "old\n").unwrap();
        }
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "baseline",
        ]);
        std::fs::write(root.join("modify.txt"), "staged\n").unwrap();
        git(&["add", "modify.txt"]);
        std::fs::write(root.join("modify.txt"), "current\n").unwrap();
        std::fs::remove_file(root.join("delete.txt")).unwrap();
        git(&["mv", "rename.txt", "renamed.txt"]);
        std::fs::write(root.join("new 中文.txt"), "新文件\n").unwrap();
        std::fs::write(root.join("binary.bin"), [0, 1, 2]).unwrap();
        let (files, notes) = workspace_changes(root).unwrap();
        assert_eq!(files.len(), 4);
        let modified = files
            .iter()
            .find(|file| file.label == "modify.txt")
            .unwrap();
        assert_eq!(modified.before.as_deref(), Some("old\n"));
        assert_eq!(modified.after.as_ref(), "current\n");
        assert_eq!((modified.added, modified.removed), (1, 1));
        let deleted = files
            .iter()
            .find(|file| file.label == "delete.txt")
            .unwrap();
        assert_eq!((deleted.added, deleted.removed), (0, 1));
        let renamed = files
            .iter()
            .find(|file| file.label == "renamed.txt")
            .unwrap();
        assert_eq!(renamed.before.as_deref(), Some("old\n"));
        assert_eq!((renamed.added, renamed.removed), (0, 0));
        assert!(
            files
                .iter()
                .any(|file| file.label == "new 中文.txt" && file.before.is_none())
        );
        assert!(notes.iter().any(|note| note.starts_with("binary.bin：")));
        let no_head = tempfile::tempdir().unwrap();
        assert!(
            std::process::Command::new("git")
                .current_dir(no_head.path())
                .args(["init", "-q"])
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(no_head.path().join("first.txt"), "first\n").unwrap();
        assert_eq!(workspace_changes(no_head.path()).unwrap().0[0].added, 1);
    }
}

pub fn ids(records: &[&ToolRecord]) -> Vec<uuid::Uuid> {
    let mut result = Vec::new();
    for record in records {
        result.extend(ids(&record.nested_tools.iter().collect::<Vec<_>>()));
        if let Some(trace) = &record.subagent {
            result.extend(ids(&trace.tools.iter().collect::<Vec<_>>()));
        }
        if let Some(change) = &record.change {
            result.push(change.id);
        }
    }
    result
}

#[cfg(test)]
pub fn aggregate_records(records: Vec<&ToolRecord>) -> Vec<Change> {
    aggregate_records_excluding(records, &Default::default())
}
pub fn aggregate_records_excluding(
    records: Vec<&ToolRecord>,
    excluded: &std::collections::BTreeSet<uuid::Uuid>,
) -> Vec<Change> {
    fn visit(
        records: Vec<&ToolRecord>,
        changes: &mut Vec<Change>,
        seen: &mut std::collections::HashSet<uuid::Uuid>,
        merged: &mut std::collections::HashSet<PathBuf>,
        excluded: &std::collections::BTreeSet<uuid::Uuid>,
    ) {
        for record in records {
            visit(
                record.nested_tools.iter().collect(),
                changes,
                seen,
                merged,
                excluded,
            );
            if let Some(trace) = &record.subagent {
                visit(
                    trace.tools.iter().collect(),
                    changes,
                    seen,
                    merged,
                    excluded,
                );
            }
            if let Some(change) = &record.change {
                if excluded.contains(&change.id) || !seen.insert(change.id) {
                    continue;
                }
                if let Some(existing) = changes
                    .iter_mut()
                    .find(|existing| existing.path == change.path)
                {
                    existing.after = change.after.clone();
                    existing.lines = Default::default();
                    existing.id = change.id;
                    merged.insert(change.path.clone());
                } else {
                    changes.push(change.clone());
                }
            }
        }
    }
    let mut changes = Vec::new();
    let mut merged = std::collections::HashSet::new();
    visit(
        records,
        &mut changes,
        &mut std::collections::HashSet::new(),
        &mut merged,
        excluded,
    );
    changes.retain(|change| change.before.as_deref() != Some(change.after.as_ref()));
    for change in &mut changes {
        if merged.contains(&change.path) {
            change.count();
        }
    }
    changes
}

pub fn language(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
    {
        "rs" => "rust",
        "js" => "javascript",
        "ts" | "tsx" => "typescript",
        "html" => "html",
        "css" => "css",
        "json" => "json",
        "py" => "python",
        "go" => "go",
        _ => "text",
    }
}

/// Read-only snapshot, executed on the background executor. It includes staged,
/// unstaged and untracked files without modifying the index or attributing
/// pre-existing changes to the agent.
#[cfg(test)]
pub fn workspace_changes(root: &Path) -> Result<(Vec<Change>, Vec<String>), String> {
    let git = |directory: &Path, arguments: &[&str]| {
        let mut command = crate::child_process::command("git");
        command
            .current_dir(directory)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .args(arguments);
        command
            .output()
            .map_err(|error| format!("无法运行 Git：{error}"))
    };
    let status = git(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if !status.status.success() {
        return Err("无法读取 Git 工作区，请确认项目属于 Git 仓库。".into());
    }
    let top = git(root, &["rev-parse", "--show-toplevel"])?;
    let top = PathBuf::from(
        String::from_utf8(top.stdout)
            .map_err(|_| "Git 路径不是 UTF-8")?
            .trim(),
    );
    // HEAD may be absent in a newly initialized repository.
    let has_head = git(&top, &["rev-parse", "--verify", "HEAD"])?
        .status
        .success();
    let mut fields = status
        .stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut files = Vec::new();
    let mut notes = Vec::new();
    while let Some(field) = fields.next() {
        if field.len() < 4 {
            return Err("Git 状态格式无效".into());
        }
        let name = std::str::from_utf8(&field[3..]).map_err(|_| "文件路径不是 UTF-8")?;
        let renamed = field[..2].iter().any(|code| *code == b'R' || *code == b'C');
        let source = if renamed {
            std::str::from_utf8(fields.next().ok_or("重命名记录不完整")?)
                .map_err(|_| "文件路径不是 UTF-8")?
        } else {
            name
        };
        if field[..2].contains(&b'U') || field[..2] == *b"AA" || field[..2] == *b"DD" {
            continue;
        }
        let path = top.join(name);
        // Do not follow symlinks outside the repository during preview.
        let after = match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() && meta.len() > 2 * 1024 * 1024 => {
                notes.push(format!("{name}：大文件（{:.1} MiB），为保持流畅已跳过全文 Diff。", meta.len() as f64 / (1024. * 1024.)));
                continue;
            }
            Ok(meta) if meta.is_file() && meta.len() <= 2 * 1024 * 1024 => {
                std::fs::read(&path).ok()
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(Vec::new()),
            _ => None,
        };
        let before = if field[..2] == *b"??" || !has_head {
            Some(Vec::new())
        } else {
            let spec = format!("HEAD:{source}");
            // Check blob size before requesting potentially large output.
            let output = git(&top, &["cat-file", "-s", &spec])?;
            let size = String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse::<usize>()
                .ok();
            if !output.status.success() {
                Some(Vec::new())
            } else if size.is_some_and(|size| size <= 2 * 1024 * 1024) {
                let output = git(&top, &["show", &spec])?;
                if output.status.success() {
                    Some(output.stdout)
                } else {
                    None
                }
            } else {
                notes.push(format!("{name}：HEAD 版本超过 2 MiB，为保持流畅已跳过全文 Diff。"));
                continue;
            }
        };
        let text = |bytes: Option<Vec<u8>>| {
            bytes
                .filter(|bytes| !bytes.contains(&0))
                .and_then(|bytes| String::from_utf8(bytes).ok())
        };
        let (Some(before), Some(after)) = (text(before), text(after)) else {
            notes.push(format!(
                "{name}：二进制、非 UTF-8、符号链接或超过 2MB，无法预览文本 diff。"
            ));
            continue;
        };
        let mut change = Change {
            id: uuid::Uuid::new_v4(),
            path,
            label: name.into(),
            before: if field[..2] == *b"??" {
                None
            } else {
                Some(Arc::from(before))
            },
            after: Arc::from(after),
            added: 0,
            removed: 0,
            lines: Default::default(),
        };
        change.count();
        files.push(change);
    }
    Ok((files, notes))
}

pub fn ids_by_path(
    records: Vec<&ToolRecord>,
    paths: &std::collections::HashSet<PathBuf>,
) -> std::collections::HashMap<PathBuf, Vec<uuid::Uuid>> {
    fn visit(
        records: Vec<&ToolRecord>,
        paths: &std::collections::HashSet<PathBuf>,
        result: &mut std::collections::HashMap<PathBuf, Vec<uuid::Uuid>>,
    ) {
        for record in records {
            visit(record.nested_tools.iter().collect(), paths, result);
            if let Some(trace) = &record.subagent {
                visit(trace.tools.iter().collect(), paths, result);
            }
            if let Some(change) = &record.change
                && paths.contains(&change.path)
            {
                result
                    .entry(change.path.clone())
                    .or_default()
                    .push(change.id);
            }
        }
    }
    let mut result = std::collections::HashMap::new();
    visit(records, paths, &mut result);
    result
}

/// Reject net reversals spanning unrecorded edits between tool writes.
pub fn verify_reversal_chain(
    records: Vec<&ToolRecord>,
    paths: &std::collections::HashSet<PathBuf>,
    excluded: &std::collections::BTreeSet<uuid::Uuid>,
) -> Result<(), String> {
    fn visit(
        records: Vec<&ToolRecord>,
        paths: &std::collections::HashSet<PathBuf>,
        excluded: &std::collections::BTreeSet<uuid::Uuid>,
        seen: &mut std::collections::HashSet<uuid::Uuid>,
        last: &mut std::collections::HashMap<PathBuf, Arc<str>>,
    ) -> Result<(), String> {
        for record in records {
            visit(
                record.nested_tools.iter().collect(),
                paths,
                excluded,
                seen,
                last,
            )?;
            if let Some(trace) = &record.subagent {
                visit(trace.tools.iter().collect(), paths, excluded, seen, last)?;
            }
            if let Some(change) = &record.change
                && paths.contains(&change.path)
                && !excluded.contains(&change.id)
                && seen.insert(change.id)
            {
                if last
                    .get(&change.path)
                    .is_some_and(|previous| change.before.as_deref() != Some(previous.as_ref()))
                {
                    return Err(format!(
                        "{} 在两次写入记录之间存在未记录修改，无法安全整体撤销",
                        change.label
                    ));
                }
                last.insert(change.path.clone(), change.after.clone());
            }
        }
        Ok(())
    }
    visit(
        records,
        paths,
        excluded,
        &mut Default::default(),
        &mut Default::default(),
    )
}

pub async fn apply_remote(
    call: &ToolCall,
    context: &ToolContext,
    remote: crate::ssh_project::Remote,
    full: bool,
) -> Result<(String, Option<Change>), String> {
    let mut args = call.arguments.clone();
    args["kind"] = call.name.clone().into();
    let plan = remote.action("prepare_edit", args, full).await?;
    let path = call.arguments["path"]
        .as_str()
        .ok_or("缺少 path")?
        .to_owned();
    let diff_remote = remote.clone();
    let diff_path = path.clone();
    let diff_plan = plan.clone();
    let change =
        tokio::task::spawn_blocking(move || remote_change(diff_remote, &diff_path, diff_plan))
            .await
            .map_err(|e| e.to_string())??
            .1;
    let Some(change) = change else {
        return Ok(("文件内容未变化".into(), None));
    };
    let scope = context.clone();
    let snapshot = change.clone();
    let name = call.name.clone();
    tokio::task::spawn_blocking(move || crate::edit_journal::prepare(&scope, &snapshot, &name))
        .await
        .map_err(|e| e.to_string())??;
    let result=remote.action("commit_edit",serde_json::json!({"path":path,"canonical_path":plan["path"],"expected_sha256":plan["sha256"],"expected_absent":plan["before"].is_null(),"content":plan["after"]}),full).await;
    let anchor = context.memory.anchor.clone();
    let id = change.id;
    let error = result.as_ref().err().cloned();
    let status = if result.is_ok() {
        "committed"
    } else {
        "uncertain"
    };
    let _ = tokio::task::spawn_blocking(move || {
        crate::edit_journal::finish(&anchor, id, status, error.as_deref())
    })
    .await;
    result?;
    Ok((
        format!("远程文件已更新，+{} -{} 行", change.added, change.removed),
        Some(change),
    ))
}
