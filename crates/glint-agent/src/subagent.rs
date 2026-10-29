use crate::{
    provider,
    tools::{ToolCall, ToolContext, ToolEvent},
};
use serde::Deserialize;
use serde_json::json;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

/// This capability is request-local and is never serialized into conversation history.
#[derive(Clone, Default)]
pub enum Delegation {
    #[default]
    Unavailable,
    Root(Arc<Runner>),
    Child,
}

pub struct Runner {
    config: provider::Config,
    key: String,
    thinking: provider::Thinking,
    instructions: Vec<serde_json::Value>,
    calls: AtomicUsize,
}

impl Runner {
    pub fn new(
        config: provider::Config,
        key: String,
        thinking: provider::Thinking,
        messages: &[serde_json::Value],
    ) -> Arc<Self> {
        Arc::new(Self {
            config,
            key,
            thinking,
            instructions: messages
                .iter()
                .filter(|message| message["role"] == "system")
                .cloned()
                .collect(),
            calls: AtomicUsize::new(0),
        })
    }

    // Erase the future type at the recursive provider/tool boundary.
    pub fn run<'a>(
        &'a self,
        call: &'a ToolCall,
        context: &'a ToolContext,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
        Box::pin(async move {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                task: String,
            }
            let args: Args = serde_json::from_value(call.arguments.clone())
                .map_err(|_| "subagent 参数无效，需要 task")?;
            if args.task.trim().is_empty() || args.task.len() > 64 * 1024 {
                return Err("子任务必须为非空文本，且不超过 64KB".into());
            }
            if self.calls.fetch_add(1, Ordering::Relaxed) >= 8 {
                return Err("每次用户请求最多执行 8 个子任务".into());
            }
            let (events, receiver) = async_channel::unbounded();
            let mut child = context.clone();
            child.delegation = Delegation::Child;
            child.events = events;
            child.recall = None;
            let mut messages = self.instructions.clone();
            messages.push(json!({"role":"system","content":"You are a subagent executing a single delegated task. Work independently using the provided tools and return a concise result with evidence and any limitations to the parent agent. You cannot delegate work or create further subagents. Follow the inherited project boundaries and approval policy. You share the project files with the parent; avoid unrelated changes."}));
            messages.push(json!({"role":"user","content":args.task}));
            let request = provider::Request::start_with_tools(
                self.config.clone(),
                self.key.clone(),
                messages,
                self.thinking,
                Some(child),
            )?;
            let response = tokio::time::timeout(Duration::from_secs(120), request.response());
            tokio::pin!(response);
            let mut output = String::new();
            let mut events_open = true;
            loop {
                tokio::select! {
                    result = &mut response => {
                        let completion = result.map_err(|_| "子代理执行超时，已取消")??;
                        if completion.text.trim().is_empty() {
                            return Err("子代理未返回文本结果".into());
                        }
                        return Ok(completion.text);
                    }
                    event = receiver.recv(), if events_open => {
                        let Ok(event) = event else {
                            events_open = false;
                            continue;
                        };
                        match event {
                            // Forward the original responder so approval and live access-mode changes apply normally.
                            ToolEvent::Approval { call, respond } => {
                                context.events.send(ToolEvent::Approval { call, respond }).await.map_err(|_| "界面已关闭")?;
                            }
                            ToolEvent::Started(child_call) => {
                                output.push_str(&format!("子代理调用 {}\n", child_call.name));
                                let _ = context.events.try_send(ToolEvent::Output { call_id: call.id.clone(), output: output.clone() });
                            }
                            ToolEvent::Completed(record) => {
                                output.push_str(&format!("{}: {}\n", record.call.name, if record.success { "完成" } else { "失败" }));
                                let _ = context.events.try_send(ToolEvent::Output { call_id: call.id.clone(), output: output.clone() });
                            }
                            // Child transcripts and token occupancy must not replace the parent's UI state.
                            _ => {}
                        }
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        schedules::ScheduleStore,
        tools::{self, AccessMode},
    };
    use serde_json::Value;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::Mutex,
    };

    fn context(path: &std::path::Path, events: async_channel::Sender<ToolEvent>) -> ToolContext {
        ToolContext {
            delegation: Delegation::default(),
            mcp: Default::default(),
            skills: Arc::new(crate::skills::Catalog::empty(path.join("sessions.json"))),
            memory: crate::memory::Repository::new(path.join("sessions.json")),
            access: Arc::new(Mutex::new(AccessMode::Ask)),
            root: Some(path.to_owned()),
            schedules: Arc::new(Mutex::new(
                ScheduleStore::open(path.join("schedules.json")).unwrap(),
            )),
            provider_id: uuid::Uuid::new_v4(),
            model: "test".into(),
            events,
            recall: None,
        }
    }

    #[tokio::test]
    async fn subagent_is_hidden_and_rejected_for_children_in_every_access_mode() {
        let directory = tempfile::tempdir().unwrap();
        let (events, _receiver) = async_channel::unbounded();
        let mut context = context(directory.path(), events);
        context.delegation = Delegation::Child;
        for access in [
            AccessMode::Ask,
            AccessMode::AutoApprove,
            AccessMode::FullAccess,
        ] {
            *context.access.lock().unwrap() = access;
            for anthropic in [false, true] {
                assert!(
                    !context
                        .definitions(anthropic)
                        .to_string()
                        .contains("\"subagent\"")
                );
            }
            let result = tools::execute(
                ToolCall {
                    id: "nested".into(),
                    name: "subagent".into(),
                    arguments: json!({"task":"delegate again"}),
                },
                &context,
            )
            .await;
            assert!(!result.success);
            assert!(result.result.contains("不允许"));
        }
    }

    #[tokio::test]
    async fn both_protocols_delegate_with_isolated_history_and_enforce_child_approvals_and_depth() {
        for anthropic in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let config = provider::Config {
                model: "test-model".into(),
                base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
                protocol: if anthropic {
                    provider::Protocol::Anthropic
                } else {
                    provider::Protocol::OpenAi
                },
                ..Default::default()
            };
            let server = std::thread::spawn(move || {
                let mut requests = Vec::new();
                for round in 0..4 {
                    let start = std::time::Instant::now();
                    let mut stream = loop {
                        match listener.accept() {
                            Ok((stream, _)) => break stream,
                            Err(error)
                                if error.kind() == std::io::ErrorKind::WouldBlock
                                    && start.elapsed() < Duration::from_secs(10) =>
                            {
                                std::thread::sleep(Duration::from_millis(10))
                            }
                            Err(error) => panic!("accept failed: {error}"),
                        }
                    };
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut bytes = Vec::new();
                    while !bytes.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        stream.read_exact(&mut byte).unwrap();
                        bytes.push(byte[0]);
                    }
                    let headers = String::from_utf8(bytes).unwrap().to_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    let mut bytes = vec![0; length];
                    stream.read_exact(&mut bytes).unwrap();
                    requests.push(serde_json::from_slice::<Value>(&bytes).unwrap());
                    let calls = match round {
                        0 => vec![(
                            "delegate",
                            "subagent",
                            json!({"task":"Inspect the project"}),
                        )],
                        1 => vec![
                            (
                                "write",
                                "write_file",
                                json!({"path":"denied.txt","content":"must not write"}),
                            ),
                            ("nested", "subagent", json!({"task":"must not run"})),
                        ],
                        _ => vec![],
                    };
                    let text = if round == 2 {
                        "child-result"
                    } else if round == 3 {
                        "parent-result"
                    } else {
                        ""
                    };
                    let body = if anthropic {
                        let mut content: Vec<Value> = calls.iter().map(|(id, name, args)| json!({"type":"tool_use","id":id,"name":name,"input":args})).collect();
                        if !text.is_empty() { content.push(json!({"type":"text","text":text})); }
                        json!({"content":content})
                    } else {
                        json!({"choices":[{"message":{"content":text,"tool_calls":calls.iter().map(|(id, name, args)| json!({"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}})).collect::<Vec<_>>()}}]})
                    }.to_string();
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
                requests
            });
            let (events, receiver) = async_channel::unbounded();
            let context = context(directory.path(), events);
            let request = provider::Request::start_with_tools(
                config,
                "test-key".into(),
                vec![
                    json!({"role":"system","content":"project-instructions"}),
                    json!({"role":"user","content":"private-parent-history"}),
                ],
                provider::Thinking::Default,
                Some(context),
            )
            .unwrap();
            let approvals = async {
                while let Ok(event) = receiver.recv().await {
                    if let ToolEvent::Approval { call, respond } = event {
                        assert_eq!(call.name, "write_file");
                        respond.send(false).unwrap();
                    }
                }
            };
            let (result, _) = tokio::join!(request.response(), approvals);
            assert_eq!(result.unwrap().text, "parent-result");
            assert!(!directory.path().join("denied.txt").exists());
            let requests = server.join().unwrap();
            assert!(requests[0]["tools"].to_string().contains("\"subagent\""));
            for request in &requests[1..3] {
                assert!(!request["tools"].to_string().contains("\"subagent\""));
                assert!(!request.to_string().contains("private-parent-history"));
                assert!(request.to_string().contains("project-instructions"));
            }
            assert!(requests[2].to_string().contains("用户拒绝"));
            assert!(requests[2].to_string().contains("不允许继续派生"));
            assert!(requests[3].to_string().contains("child-result"));
            assert!(!requests[3].to_string().contains("must not write"));
        }
    }
}
