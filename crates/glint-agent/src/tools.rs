use crate::schedules::{Repeat, Schedule, ScheduleStore};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolRecord {
    pub call: ToolCall,
    pub result: String,
    pub success: bool,
    pub elapsed_ms: u64,
}

pub enum ToolEvent {
    Transcript(Vec<crate::generation::ContentBlock>),
    Started(ToolCall),
    Approval {
        call: ToolCall,
        respond: tokio::sync::oneshot::Sender<bool>,
    },
    Completed(ToolRecord),
}

#[derive(Clone)]
pub struct ToolContext {
    pub root: Option<PathBuf>,
    pub schedules: Arc<Mutex<ScheduleStore>>,
    pub provider_id: Uuid,
    pub model: String,
    pub events: async_channel::Sender<ToolEvent>,
}

fn definition(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({"name":name,"description":description,"parameters":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
}

pub fn definitions(anthropic: bool) -> Value {
    let string = || json!({"type":"string"});
    let mut definitions = vec![
        definition(
            "list_files",
            "List project-relative files. Skips .git, target and node_modules.",
            json!({"path":string()}),
            &[],
        ),
        definition(
            "read_file",
            "Read UTF-8 project file, up to 500 lines. Paths are relative to the opened project.",
            json!({"path":string(),"start_line":{"type":"integer","minimum":1},"lines":{"type":"integer","minimum":1,"maximum":500}}),
            &["path"],
        ),
        definition(
            "search_files",
            "Search project text with ripgrep regex; returns at most 200 matches.",
            json!({"pattern":string(),"glob":string()}),
            &["pattern"],
        ),
        definition(
            "write_file",
            "Write a UTF-8 file in the project. Requires user approval.",
            json!({"path":string(),"content":string()}),
            &["path", "content"],
        ),
        definition(
            "replace_in_file",
            "Replace one exact occurrence in a project file. Requires user approval; fails if old text is missing or ambiguous.",
            json!({"path":string(),"old":string(),"new":string()}),
            &["path", "old", "new"],
        ),
        definition(
            "run_command",
            "Run a program with argv in the project. Requires user approval. Use program + args, not shell syntax. Has a 60 second timeout.",
            json!({"program":string(),"args":{"type":"array","items":string()}}),
            &["program", "args"],
        ),
        definition(
            "schedule_list",
            "List scheduled agent tasks. Application must be running for execution.",
            json!({}),
            &[],
        ),
        definition(
            "schedule_create",
            "Create a scheduled agent task. Requires user approval. run_at must be an ISO 8601 timestamp with timezone. repeat is once, daily or weekly.",
            json!({"title":string(),"prompt":string(),"run_at":string(),"repeat":{"type":"string","enum":["once","daily","weekly"]}}),
            &["title", "prompt", "run_at"],
        ),
        definition(
            "schedule_update",
            "Update an existing task by id. Requires user approval; omitted fields stay unchanged.",
            json!({"id":string(),"title":string(),"prompt":string(),"run_at":string(),"repeat":{"type":"string","enum":["once","daily","weekly"]},"enabled":{"type":"boolean"}}),
            &["id"],
        ),
        definition(
            "schedule_delete",
            "Delete a scheduled task by id. Requires user approval.",
            json!({"id":string()}),
            &["id"],
        ),
    ];
    if anthropic {
        Value::Array(definitions.drain(..).map(|value| json!({"name":value["name"],"description":value["description"],"input_schema":value["parameters"]})).collect())
    } else {
        Value::Array(
            definitions
                .into_iter()
                .map(|value| json!({"type":"function","function":value}))
                .collect(),
        )
    }
}

fn text<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value[name]
        .as_str()
        .ok_or_else(|| format!("缺少参数 {name}"))
}

fn safe_path(root: &Path, relative: &str, writing: bool) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::Prefix(_)))
    {
        return Err("路径必须位于项目目录内".into());
    }
    let root = root.canonicalize().map_err(|_| "项目目录不可用")?;
    let path = root.join(relative);
    let existing = if path.exists() {
        path.clone()
    } else if writing {
        path.parent().ok_or("文件路径无效")?.to_path_buf()
    } else {
        return Err("文件不存在".into());
    };
    let resolved = existing
        .canonicalize()
        .map_err(|_| "目录不存在，请先选择已有目录")?;
    if !resolved.starts_with(&root) {
        return Err("符号链接指向了项目之外".into());
    }
    Ok(path)
}

fn needs_approval(call: &ToolCall) -> bool {
    if matches!(
        call.name.as_str(),
        "write_file"
            | "replace_in_file"
            | "run_command"
            | "schedule_create"
            | "schedule_update"
            | "schedule_delete"
    ) {
        return true;
    }
    call.name == "read_file"
        && call.arguments["path"].as_str().is_some_and(|path| {
            path.split('/')
                .any(|part| part.starts_with(".env") || part.ends_with(".pem") || part == "id_rsa")
        })
}

pub async fn execute(call: ToolCall, context: &ToolContext) -> ToolRecord {
    let started = Instant::now();
    let _ = context.events.send(ToolEvent::Started(call.clone())).await;
    let result = async {
        if needs_approval(&call) {
            let (respond, response) = tokio::sync::oneshot::channel();
            context
                .events
                .send(ToolEvent::Approval {
                    call: call.clone(),
                    respond,
                })
                .await
                .map_err(|_| "界面已关闭")?;
            if !response.await.unwrap_or(false) {
                return Err("用户拒绝了此工具调用".into());
            }
        }
        execute_inner(&call, context).await
    }
    .await;
    let success = result.is_ok();
    let result = match result {
        Ok(result) | Err(result) => result,
    };
    let record = ToolRecord {
        call,
        result,
        success,
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    let _ = context
        .events
        .send(ToolEvent::Completed(record.clone()))
        .await;
    record
}

async fn execute_inner(call: &ToolCall, context: &ToolContext) -> Result<String, String> {
    let args = &call.arguments;
    if call.name.starts_with("schedule_") {
        return schedule_tool(call, context);
    }
    let root = context
        .root
        .as_deref()
        .ok_or("请先打开项目，再使用文件和命令工具")?;
    match call.name.as_str() {
        "read_file" => {
            let path = safe_path(root, text(args, "path")?, false)?;
            if std::fs::metadata(&path).map_err(|_| "无法读取文件")?.len() > 2 * 1024 * 1024 {
                return Err("文件超过 2MB，请使用搜索工具定位内容".into());
            }
            let content = std::fs::read_to_string(path).map_err(|_| "无法读取 UTF-8 文件")?;
            let start = args["start_line"].as_u64().unwrap_or(1).max(1) as usize - 1;
            let lines = args["lines"].as_u64().unwrap_or(200).clamp(1, 500) as usize;
            Ok(content
                .lines()
                .enumerate()
                .skip(start)
                .take(lines)
                .map(|(index, line)| format!("{}: {line}", index + 1))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "list_files" => {
            let path = safe_path(root, args["path"].as_str().unwrap_or("."), false)?;
            let mut pending = vec![(path, 0)];
            let mut paths = Vec::new();
            while let Some((directory, depth)) = pending.pop() {
                for entry in std::fs::read_dir(directory)
                    .map_err(|_| "无法列出目录")?
                    .flatten()
                {
                    if paths.len() >= 2000 {
                        break;
                    }
                    if matches!(
                        entry.file_name().to_str(),
                        Some(".git" | "node_modules" | "target")
                    ) {
                        continue;
                    }
                    let path = entry.path();
                    if entry.file_type().is_ok_and(|kind| kind.is_symlink()) {
                        continue;
                    }
                    paths.push(
                        path.strip_prefix(root)
                            .unwrap_or(&path)
                            .display()
                            .to_string(),
                    );
                    if path.is_dir() && depth < 3 {
                        pending.push((path, depth + 1));
                    }
                }
            }
            paths.sort();
            Ok(paths.join("\n"))
        }
        "search_files" => {
            let mut args_list = vec![
                "--line-number".to_owned(),
                "--max-count".into(),
                "30".into(),
                "--glob".into(),
                "!.env*".into(),
                "--glob".into(),
                "!*.pem".into(),
            ];
            if let Some(glob) = args["glob"].as_str() {
                args_list.extend(["--glob".into(), glob.into()]);
            }
            args_list.extend(["--".into(), text(args, "pattern")?.into(), ".".into()]);
            let result = command(root, "rg", &args_list).await?;
            Ok(result.lines().take(200).collect::<Vec<_>>().join("\n"))
        }
        "write_file" | "replace_in_file" => {
            let path = safe_path(root, text(args, "path")?, true)?;
            let content = if call.name == "write_file" {
                text(args, "content")?.to_owned()
            } else {
                let current = std::fs::read_to_string(&path).map_err(|_| "无法读取目标文件")?;
                let old = text(args, "old")?;
                if old.is_empty() || current.matches(old).count() != 1 {
                    return Err("待替换内容不存在或不唯一，未修改文件".into());
                }
                current.replacen(old, text(args, "new")?, 1)
            };
            if content.len() > 2 * 1024 * 1024 {
                return Err("单次写入超过 2MB".into());
            }
            std::fs::write(path, content).map_err(|_| "写入失败")?;
            Ok("文件已更新".into())
        }
        "run_command" => {
            let program = text(args, "program")?;
            let args = args["args"]
                .as_array()
                .ok_or("args 必须为字符串数组")?
                .iter()
                .map(|item| item.as_str().map(str::to_owned).ok_or("参数必须为字符串"))
                .collect::<Result<Vec<_>, _>>()?;
            command(root, program, &args).await
        }
        _ => Err("未知工具".into()),
    }
}

async fn capture(mut reader: impl tokio::io::AsyncRead + Unpin) -> std::io::Result<String> {
    let mut kept = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let remaining = (128 * 1024usize).saturating_sub(kept.len());
        kept.extend_from_slice(&buffer[..count.min(remaining)]);
    }
    Ok(String::from_utf8_lossy(&kept).into_owned())
}

async fn command(root: &Path, program: &str, args: &[String]) -> Result<String, String> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .current_dir(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    for (name, _) in std::env::vars() {
        if name.ends_with("_KEY")
            || name.ends_with("_TOKEN")
            || name.ends_with("_SECRET")
            || name == "deepseek_api_key"
        {
            command.env_remove(name);
        }
    }
    let mut child = command
        .spawn()
        .map_err(|_| "无法运行命令，请检查程序是否已安装")?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (out, err, status) = tokio::time::timeout(Duration::from_secs(60), async {
        tokio::join!(capture(stdout), capture(stderr), child.wait())
    })
    .await
    .map_err(|_| "命令超过 60 秒，已终止")?;
    let status = status.map_err(|_| "命令执行失败")?;
    Ok(format!(
        "exit: {}\n{}\n{}",
        status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".into()),
        out.map_err(|_| "无法读取输出")?,
        err.map_err(|_| "无法读取错误输出")?
    ))
}

fn schedule_tool(call: &ToolCall, context: &ToolContext) -> Result<String, String> {
    let mut store = context.schedules.lock().map_err(|_| "日程存储不可用")?;
    let args = &call.arguments;
    if call.name == "schedule_list" {
        return serde_json::to_string(&store.tasks).map_err(|_| "无法编码日程".into());
    }
    let id = if call.name == "schedule_create" {
        Uuid::new_v4()
    } else {
        Uuid::parse_str(text(args, "id")?).map_err(|_| "日程 ID 无效")?
    };
    if call.name == "schedule_delete" {
        store.remove(id)?;
        return Ok("日程已删除".into());
    }
    let mut task = if call.name == "schedule_create" {
        Schedule {
            id,
            title: String::new(),
            prompt: String::new(),
            run_at: chrono::Local::now(),
            repeat: Repeat::Once,
            enabled: true,
            project: context.root.clone(),
            provider_id: Some(context.provider_id),
            model: context.model.clone(),
            running: false,
            last_run: None,
            last_error: None,
        }
    } else {
        store
            .tasks
            .iter()
            .find(|task| task.id == id)
            .cloned()
            .ok_or("日程不存在")?
    };
    if task.running {
        return Err("任务正在执行，暂时不能修改".into());
    }
    if let Some(value) = args["title"].as_str() {
        task.title = value.into();
    }
    if let Some(value) = args["prompt"].as_str() {
        task.prompt = value.into();
    }
    if let Some(value) = args["run_at"].as_str() {
        task.run_at = chrono::DateTime::parse_from_rfc3339(value)
            .map_err(|_| "run_at 必须为带时区的 ISO 8601 时间")?
            .with_timezone(&chrono::Local);
    }
    if let Some(value) = args["repeat"].as_str() {
        task.repeat = match value {
            "once" => Repeat::Once,
            "daily" => Repeat::Daily,
            "weekly" => Repeat::Weekly,
            _ => return Err("repeat 无效".into()),
        };
    }
    if let Some(value) = args["enabled"].as_bool() {
        task.enabled = value;
    }
    store.upsert(task.clone())?;
    serde_json::to_string(&task).map_err(|_| "无法编码日程".into())
}

pub fn parse_calls(message: &Value, anthropic: bool) -> Result<Vec<ToolCall>, String> {
    let entries: Vec<_> = if anthropic {
        message
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item["type"] == "tool_use")
                    .collect()
            })
            .unwrap_or_default()
    } else {
        message["tool_calls"]
            .as_array()
            .map(|items| items.iter().collect())
            .unwrap_or_default()
    };
    entries
        .into_iter()
        .map(|item| {
            let (name, arguments) = if anthropic {
                (text(item, "name")?.to_owned(), item["input"].clone())
            } else {
                (
                    text(&item["function"], "name")?.to_owned(),
                    serde_json::from_str(text(&item["function"], "arguments")?)
                        .map_err(|_| "工具参数不是有效 JSON")?,
                )
            };
            Ok(ToolCall {
                id: text(item, "id")?.to_owned(),
                name,
                arguments,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_paths_cannot_escape_project_and_tool_schemas_cover_schedules() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file.txt"), "hello").unwrap();
        assert!(safe_path(root.path(), "../outside", true).is_err());
        assert!(safe_path(root.path(), "/etc/passwd", false).is_err());
        assert!(safe_path(root.path(), "file.txt", false).is_ok());
        let schemas = definitions(true);
        assert!(
            schemas
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "schedule_create")
        );
    }
}
