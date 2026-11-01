//! JavaScript orchestration over the existing, request-scoped tool executor.
mod worker;
use crate::tools::{ToolCall, ToolContext, ToolEvent, ToolRecord};
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashSet, VecDeque},
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
pub use worker::main as worker_main;
pub struct Outcome {
    pub result: String,
    pub success: bool,
    pub tools: Vec<ToolRecord>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    code: String,
    timeout_seconds: Option<u64>,
}
fn readonly(call: &ToolCall) -> bool {
    matches!(
        call.name.as_str(),
        "list_files"
            | "read_file"
            | "search"
            | "search_files"
            | "session_read"
            | "session_changes"
            | "memory_read"
            | "memory_search"
            | "skill_list"
            | "skill_read"
            | "todo_read"
            | "partner_list"
            | "schedule_list"
            | "command_poll"
    ) && !crate::tools::needs_approval(call)
}
fn output(record: &ToolRecord, id: u64) -> Value {
    if record.success {
        json!({"id":id,"success":true,"value":serde_json::from_str::<Value>(&record.result).unwrap_or_else(|_|Value::String(record.result.clone()))})
    } else {
        json!({"id":id,"success":false,"error":record.result})
    }
}
async fn relay(event: ToolEvent, parent: &ToolContext, id: &str) {
    match event {
        ToolEvent::Started(call) => {
            let _ = parent
                .events
                .send(ToolEvent::Output {
                    call_id: id.into(),
                    output: format!("正在执行 {}…", call.name),
                })
                .await;
        }
        ToolEvent::Output { .. } | ToolEvent::Completed(_) => {}
        other => {
            let _ = parent.events.send(other).await;
        }
    }
}
async fn nested(context: ToolContext, call: ToolCall, parent_id: String) -> ToolRecord {
    let (events, receiver) = async_channel::bounded(32);
    let mut child = context.clone();
    child.events = events;
    let future = crate::tools::execute(call, &child);
    tokio::pin!(future);
    loop {
        tokio::select! {record=&mut future=>{while let Ok(event)=receiver.try_recv(){relay(event,&context,&parent_id).await;}return record;},event=receiver.recv()=>{if let Ok(event)=event{relay(event,&context,&parent_id).await;}}}
    }
}
#[cfg(target_os = "macos")]
fn resident(pid: u32) -> u64 {
    unsafe {
        let mut info: libc::proc_taskinfo = std::mem::zeroed();
        let size = std::mem::size_of_val(&info);
        let n = libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTASKINFO,
            0,
            &mut info as *mut _ as *mut _,
            size as i32,
        );
        if n == size as i32 {
            info.pti_resident_size
        } else {
            0
        }
    }
}
#[cfg(target_os = "linux")]
fn resident(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|line| line.starts_with("VmRSS:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|s| s.parse::<u64>().ok())
        })
        .unwrap_or(0)
        * 1024
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn resident(_: u32) -> u64 {
    0
}
async fn protocol(
    context: &ToolContext,
    call: &ToolCall,
    args: Args,
    records: &mut Vec<ToolRecord>,
    executable: std::path::PathBuf,
) -> Result<Value, String> {
    let names: HashSet<String> = context
        .definitions(true)
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .filter(|name| !matches!(*name, "code_mode" | "subagent"))
        .map(str::to_owned)
        .collect();
    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(args.timeout_seconds.unwrap_or(120));
    let mut command = tokio::process::Command::new(executable);
    command
        .arg("--glint-code-mode-worker")
        .env_clear()
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        if let Some(value) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", value);
        }
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("无法启动 Code Mode 执行进程：{e}"))?;
    let pid = child.id().ok_or("执行进程未启动")?;
    let mut input = child.stdin.take().unwrap();
    let mut lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
    tokio::time::timeout_at(deadline, async {
        input
            .write_all((json!({"code":args.code,"names":names}).to_string() + "\n").as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        input.flush().await.map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "Code Mode 启动超时")??;
    let timeout = tokio::time::sleep_until(deadline);
    tokio::pin!(timeout);
    let mut memory = tokio::time::interval(Duration::from_millis(200));
    let mut queue = VecDeque::<(u64, ToolCall)>::new();
    let mut ids = HashSet::new();
    let mut running = FuturesUnordered::new();
    let mut exclusive = false;
    loop {
        while let Some((_, next)) = queue.front() {
            let read = readonly(next);
            if exclusive || running.len() >= 4 || (!read && !running.is_empty()) {
                break;
            }
            let (id, next) = queue.pop_front().unwrap();
            exclusive = !read;
            let context = context.clone();
            let parent = call.id.clone();
            running.push(async move { (id, nested(context, next, parent).await) }.boxed());
            if exclusive {
                break;
            }
        }
        tokio::select! {
             _=&mut timeout=>return Err("Code Mode 已超时，已停止脚本及未完成的调用；已完成的修改保留，可查看变更并撤销".into()),
             _=memory.tick()=>{if resident(pid)>512*1024*1024{return Err("Code Mode 超过 512MiB 内存限制，已终止执行进程".into());}},
             completed=running.next(),if !running.is_empty()=>{
              let (id,mut record)=completed.unwrap();exclusive=false;
              let reply=output(&record,id);let success=record.success;bound_trace(&mut record);records.push(record);
        let _=context.events.send(ToolEvent::BatchProgress{call_id:call.id.clone(),tools:records.clone()}).await;
              let _=context.events.send(ToolEvent::Output{call_id:call.id.clone(),output:format!("Code Mode：已执行 {} 个工具调用",records.len())}).await;
              // Every accepted write remains in records even when the script or a later step fails.
              if !success{return Err(format!("第 {} 个工具调用失败：{}；后续调用已停止，已完成的操作不会自动回滚",records.len(),records.last().unwrap().result.chars().take(1000).collect::<String>()));}
              let text=reply.to_string();if text.len()>4*1024*1024{return Err("工具结果超过 4MiB，请缩小工具查询范围".into());}
              input.write_all((text+"\n").as_bytes()).await.map_err(|e|e.to_string())?;input.flush().await.map_err(|e|e.to_string())?;
             },
             line=lines.next_line()=>{
              let line=line.map_err(|e|e.to_string())?.ok_or("Code Mode 执行进程中断，请核对已执行操作")?;if line.len()>4*1024*1024{return Err("Code Mode RPC 超出限制".into());}let frame:Value=serde_json::from_str(&line).map_err(|_|"Code Mode RPC 无效")?;
              match frame["type"].as_str(){
               Some("call")=>{let id=frame["id"].as_u64().ok_or("无效的调用 ID")?;if ids.len()>=64||!ids.insert(id){return Err("调用数量超限或 ID 重复".into());}let name=frame["name"].as_str().ok_or("缺少工具名")?;if !names.contains(name)||!frame["arguments"].is_object(){return Err("工具不可用、不可嵌套调用，或参数不是对象".into());}queue.push_back((id,ToolCall{id:format!("{}:{id}",call.id),name:name.into(),arguments:frame["arguments"].clone()}));},
               Some("result")=>{if !queue.is_empty()||!running.is_empty(){return Err("存在未完成的工具调用，请 await 所有操作".into());}let value=frame["value"].clone();if value.to_string().len()>65536{return Err("最终结果超过 64KiB，请在脚本内筛选数据".into());}drop(input);let status=tokio::time::timeout(Duration::from_secs(2),child.wait()).await.map_err(|_|"执行进程未及时结束")?.map_err(|e|e.to_string())?;if !status.success(){return Err("执行进程异常退出".into());}return Ok(value);},
               Some("error")=>return Err(frame["error"].as_str().unwrap_or("JavaScript 执行失败").chars().take(2048).collect()),
               _=>return Err("未知 Code Mode RPC 帧".into())
              }
             }
            }
    }
}
fn bound_trace(record: &mut ToolRecord) {
    const LIMIT: usize = 64 * 1024;
    if record.result.len() <= LIMIT {
        return;
    }
    let original = record.result.len();
    let mut end = LIMIT - 256;
    while !record.result.is_char_boundary(end) {
        end -= 1;
    }
    record.result.truncate(end);
    record.result.push_str(&format!("\n…执行轨迹仅保留前 64KiB（原输出 {original} 字节）；脚本收到完整结果，文件快照另行完整保存。"));
}
struct ApprovalCleanup(async_channel::Sender<ToolEvent>);
impl Drop for ApprovalCleanup {
    fn drop(&mut self) {
        let _ = self.0.try_send(ToolEvent::ApprovalResolved);
    }
}
pub async fn execute(context: &ToolContext, call: &ToolCall) -> Outcome {
    let mut records = Vec::new();
    let _cleanup = ApprovalCleanup(context.events.clone());
    let result = async {
        let args: Args = serde_json::from_value(call.arguments.clone())
            .map_err(|e| format!("Code Mode 参数无效：{e}"))?;
        if args.code.len() > 65536 || !(1..=600).contains(&args.timeout_seconds.unwrap_or(120)) {
            return Err("code 最大 64KiB，timeout_seconds 必须为 1..600".into());
        }
        static SLOTS: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
            std::sync::OnceLock::new();
        let _permit = SLOTS
            .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)))
            .clone()
            .try_acquire_owned()
            .map_err(|_| "同时执行的 Code Mode 已达到 2 个")?;
        protocol(
            context,
            call,
            args,
            &mut records,
            std::env::current_exe().map_err(|e| e.to_string())?,
        )
        .await
    }
    .await;
    let success = result.is_ok();
    let steps: Vec<_> = records
        .iter()
        .map(|r| json!({"id":r.call.id,"tool":r.call.name,"success":r.success}))
        .collect();
    let result = match result {
        Ok(value) => json!({"status":"completed","value":value,"calls":steps}),
        Err(error) => {
            json!({"status":"failed","error":error,"calls":steps,"partial_changes_retained":records.iter().any(|r|r.change.is_some())})
        }
    };
    Outcome {
        result: result.to_string(),
        success,
        tools: records,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::Path;
    fn mock(root: &Path, source: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = root.join("worker.py");
        std::fs::write(&path,format!("#!/usr/bin/env python3\nimport json,sys\nstart=json.loads(sys.stdin.readline())\ndef send(v):\n print(json.dumps(v),flush=True)\ndef receive():\n return json.loads(sys.stdin.readline())\n{source}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    fn call() -> ToolCall {
        ToolCall {
            id: "batch".into(),
            name: "code_mode".into(),
            arguments: json!({"code":"return 1;"}),
        }
    }
    #[tokio::test]
    async fn raw_pages_preserve_newlines_and_report_completeness() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let context = crate::edits::tests::context(&root);
        let text = "甲\r\n乙\n丙";
        std::fs::write(root.join("raw.txt"), text).unwrap();
        for (lines, complete, expected) in [(1, false, "甲\r\n"), (3, true, text)] {
            let call = ToolCall {
                id: "raw".into(),
                name: "read_file".into(),
                arguments: json!({"path":"raw.txt","line_numbers":false,"lines":lines}),
            };
            let record = crate::tools::execute(call, &context).await;
            assert!(record.success);
            let value: Value = serde_json::from_str(&record.result).unwrap();
            assert_eq!(value["content"], expected);
            assert_eq!(value["is_complete"], complete);
            assert_eq!(value["expected_sha256"], crate::atomic_file::digest(text));
        }
    }
    #[test]
    fn nested_data_survives_storage_but_is_excluded_from_model_messages() {
        let mut outer =
            crate::tools::rejected(call(), "{\"status\":\"completed\",\"value\":42}".into());
        outer.success = true;
        let mut inner = crate::tools::rejected(
            ToolCall {
                id: "child".into(),
                name: "read_file".into(),
                arguments: json!({}),
            },
            "INTERMEDIATE_MARKER".into(),
        );
        inner.success = true;
        outer.nested_tools = vec![inner].into();
        let cloned = outer.clone();
        assert!(std::sync::Arc::ptr_eq(
            &outer.nested_tools,
            &cloned.nested_tools
        ));
        let encoded = serde_json::to_value(&outer).unwrap();
        assert!(encoded.to_string().contains("INTERMEDIATE_MARKER"));
        let restored: ToolRecord = serde_json::from_value(encoded).unwrap();
        assert_eq!(restored.nested_tools.len(), 1);
        let mut session = crate::model::Session::new();
        session.push(crate::model::Role::Agent, "FINAL_SUMMARY".into());
        session.messages[0].tools = vec![outer.clone()];
        session.messages[0].blocks = vec![crate::generation::ContentBlock::ToolResult(outer)];
        for protocol in [
            crate::provider::Protocol::OpenAi,
            crate::provider::Protocol::Anthropic,
        ] {
            let messages = crate::provider::messages(&session.messages, protocol);
            assert!(
                !serde_json::to_string(&messages)
                    .unwrap()
                    .contains("INTERMEDIATE_MARKER")
            );
        }
    }
    #[tokio::test]
    async fn controller_reuses_versions_records_edits_and_supports_undo() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let context = crate::edits::tests::context(&root);
        std::fs::write(root.join("a"), "old").unwrap();
        let executable = mock(
            &root,
            r#"
for n in [1,2]: send({'type':'call','id':n,'name':'read_file','arguments':{'path':'a'}})
replies=[receive(),receive()]
sha=replies[0]['value']['expected_sha256']
send({'type':'call','id':3,'name':'write_file','arguments':{'path':'a','content':'new','expected_sha256':sha}})
assert receive()['success']
send({'type':'result','value':{'changed':1}})
"#,
        );
        let mut records = Vec::new();
        let result = protocol(
            &context,
            &call(),
            Args {
                code: "return 1;".into(),
                timeout_seconds: Some(10),
            },
            &mut records,
            executable,
        )
        .await
        .unwrap();
        assert_eq!(result["changed"], 1);
        assert_eq!(records.len(), 3);
        assert_eq!(std::fs::read_to_string(root.join("a")).unwrap(), "new");
        let mut parent = crate::tools::rejected(call(), "result".into());
        parent.nested_tools = records.into();
        let net = crate::edits::aggregate_records(vec![&parent]);
        assert_eq!(net.len(), 1);
        assert_eq!(crate::edits::ids(&[&parent]).len(), 1);
        let paths = net.iter().map(|c| c.path.clone()).collect();
        assert_eq!(crate::edits::ids_by_path(vec![&parent], &paths).len(), 1);
        crate::edits::verify_reversal_chain(vec![&parent], &paths, &Default::default()).unwrap();
        let undone = crate::undo::apply(net, Some(root.clone())).await;
        assert!(undone.error.is_none(), "{:?}", undone.error);
        assert_eq!(std::fs::read_to_string(root.join("a")).unwrap(), "old");
    }
    #[tokio::test]
    async fn failure_and_rejection_stop_queued_mutations_and_preserve_completed_edits() {
        for reject in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let mut context = crate::edits::tests::context(&root);
            std::fs::write(root.join("a"), "old").unwrap();
            let source = if reject {
                r#"
send({'type':'call','id':1,'name':'write_file','arguments':{'path':'a','content':'new'}})
send({'type':'call','id':2,'name':'write_file','arguments':{'path':'untouched','content':'wrong'}})
receive()
"#
            } else {
                r#"
send({'type':'call','id':1,'name':'write_file','arguments':{'path':'a','content':'new'}})
receive()
send({'type':'call','id':2,'name':'read_file','arguments':{'path':'missing'}})
send({'type':'call','id':3,'name':'write_file','arguments':{'path':'untouched','content':'wrong'}})
receive()
"#
            };
            let executable = mock(&root, source);
            let approval = if reject {
                *context.access.lock().unwrap() = crate::tools::AccessMode::Ask;
                let (tx, rx) = async_channel::unbounded();
                context.events = tx;
                Some(tokio::spawn(async move {
                    while let Ok(event) = rx.recv().await {
                        if let ToolEvent::Approval { respond, .. } = event {
                            let _ = respond.send(false);
                            break;
                        }
                    }
                }))
            } else {
                None
            };
            let mut records = Vec::new();
            let error = protocol(
                &context,
                &call(),
                Args {
                    code: "return 1;".into(),
                    timeout_seconds: Some(5),
                },
                &mut records,
                executable,
            )
            .await
            .unwrap_err();
            assert!(error.contains("失败"));
            assert!(!root.join("untouched").exists());
            assert_eq!(
                std::fs::read_to_string(root.join("a")).unwrap(),
                if reject { "old" } else { "new" }
            );
            assert_eq!(records.len(), if reject { 1 } else { 2 });
            if let Some(task) = approval {
                task.await.unwrap();
            }
        }
    }
    #[tokio::test]
    async fn timeout_kills_worker_and_unknown_or_recursive_tools_are_not_executed() {
        for name in ["code_mode", "subagent", "unknown"] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let context = crate::edits::tests::context(&root);
            let executable = mock(
                &root,
                &format!(
                    "send({{'type':'call','id':1,'name':'{name}','arguments':{{}}}})\nreceive()"
                ),
            );
            let mut records = Vec::new();
            assert!(
                protocol(
                    &context,
                    &call(),
                    Args {
                        code: "".into(),
                        timeout_seconds: Some(3)
                    },
                    &mut records,
                    executable
                )
                .await
                .is_err()
            );
            assert!(records.is_empty());
        }
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let context = crate::edits::tests::context(&root);
        let executable = mock(
            &root,
            &format!(
                "import os,time\nopen({},'w').write(str(os.getpid()))\ntime.sleep(30)",
                serde_json::to_string(root.join("pid").to_str().unwrap()).unwrap()
            ),
        );
        let mut records = Vec::new();
        let result = protocol(
            &context,
            &call(),
            Args {
                code: "".into(),
                timeout_seconds: Some(1),
            },
            &mut records,
            executable,
        )
        .await;
        assert!(result.unwrap_err().contains("超时"));
        let pid: i32 = std::fs::read_to_string(root.join("pid"))
            .unwrap()
            .parse()
            .unwrap();
        for _ in 0..30 {
            if unsafe { libc::kill(pid, 0) } != 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0);
    }
}
