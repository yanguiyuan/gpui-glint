use crate::{
    provider,
    tools::{ToolCall, ToolContext, ToolEvent},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

pub type Registry = Arc<Mutex<Vec<Trace>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Cancelled,
}

impl Status {
    pub fn active(self) -> bool {
        matches!(self, Self::Running | Self::WaitingApproval)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "运行中",
            Self::WaitingApproval => "待批准",
            Self::Completed => "已完成",
            Self::Failed => "失败",
            Self::Cancelled => "已取消",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trace {
    #[serde(default)]
    pub partner_id: Option<uuid::Uuid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todos: Vec<crate::todo::Item>,
    pub id: uuid::Uuid,
    pub call_id: String,
    pub title: String,
    pub task: String,
    pub status: Status,
    pub blocks: Vec<crate::generation::ContentBlock>,
    pub tools: Vec<crate::tools::ToolRecord>,
    pub thinking_ms: std::collections::BTreeMap<usize, u64>,
    pub stats: Option<crate::generation::GenerationStats>,
    pub elapsed_ms: u64,
    pub error: Option<String>,
}

impl Trace {
    pub fn interrupt(&mut self) {
        if !self.status.active() {
            return;
        }
        self.status = Status::Cancelled;
        self.error = Some("执行已中断".into());
        self.close_tools();
    }

    fn close_tools(&mut self) {
        crate::generation::close_unfinished_tools(
            &mut self.blocks,
            &mut self.tools,
            self.error.as_deref().unwrap_or("子任务已中断"),
        );
    }
}

/// Persists each event before notifying the UI; the final record is also available
/// when cancellation drops the provider future before the UI drains its channel.
struct Recording {
    trace: Trace,
    timings: crate::generation::ThinkingTimings,
    started: std::time::Instant,
    registry: Registry,
    events: async_channel::Sender<ToolEvent>,
}

impl Recording {
    fn publish(&mut self) {
        self.trace.elapsed_ms = self.started.elapsed().as_millis() as u64;
        self.trace.thinking_ms = self.timings.snapshot();
        if let Ok(mut traces) = self.registry.lock() {
            if let Some(trace) = traces.iter_mut().find(|trace| trace.id == self.trace.id) {
                *trace = self.trace.clone();
            } else {
                traces.push(self.trace.clone());
            }
        }
        let _ = self.events.try_send(ToolEvent::SubagentUpdated);
    }

    fn observe(&mut self, event: &ToolEvent) {
        use crate::generation::ContentBlock;
        match event {
            ToolEvent::TodoUpdated(todos) => self.trace.todos = todos.clone(),
            ToolEvent::Transcript(blocks) => {
                self.timings.observe(blocks);
                self.trace.blocks = blocks.clone();
            }
            ToolEvent::Started(call) => {
                self.trace.status = Status::Running;
                self.trace.tools.push(crate::tools::ToolRecord {
                    call: call.clone(),
                    result: "执行中…".into(),
                    success: false,
                    elapsed_ms: 0,
                    live_output: None,
                    subagent: None,
                    change: None,
                });
            }
            ToolEvent::Output { call_id, output } => {
                if let Some(tool) = self
                    .trace
                    .tools
                    .iter_mut()
                    .find(|tool| &tool.call.id == call_id)
                {
                    tool.live_output = Some(output.clone());
                }
            }
            ToolEvent::Completed(record) => {
                self.trace.status = Status::Running;
                if let Some(tool) = self
                    .trace
                    .tools
                    .iter_mut()
                    .find(|tool| tool.call.id == record.call.id)
                {
                    *tool = record.clone();
                } else {
                    self.trace.tools.push(record.clone());
                }
                if !self.trace.blocks.iter().any(|block| matches!(block, ContentBlock::ToolResult(tool) if tool.call.id == record.call.id)) { self.trace.blocks.push(ContentBlock::ToolResult(record.clone())); }
            }
            ToolEvent::Approval { .. } => self.trace.status = Status::WaitingApproval,
            ToolEvent::ApprovalResolved => self.trace.status = Status::Running,
            _ => {}
        }
        self.publish();
    }

    fn finish(&mut self, result: &Result<crate::generation::Completion, String>) {
        match result {
            Ok(completion) => {
                self.timings.observe(&completion.blocks);
                self.trace.blocks = completion.blocks.clone();
                self.trace.tools = completion.tool_records.clone();
                self.trace.stats = Some(completion.stats.clone());
                self.trace.status = Status::Completed;
            }
            Err(error) => {
                self.trace.status = Status::Failed;
                self.trace.error = Some(error.clone());
                self.trace.close_tools();
            }
        }
        self.timings.finish();
        self.publish();
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        if self.trace.status.active() {
            self.trace.interrupt();
            self.timings.finish();
            self.publish();
        }
    }
}

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
                title: Option<String>,
                partner_id: Option<uuid::Uuid>,
            }
            let args: Args = serde_json::from_value(call.arguments.clone())
                .map_err(|_| "subagent 参数无效，需要 task")?;
            if args.task.trim().is_empty() || args.task.len() > 64 * 1024 {
                return Err("子任务必须为非空文本，且不超过 64KB".into());
            }
            if self.calls.fetch_add(1, Ordering::Relaxed) >= 8 {
                return Err("每次用户请求最多执行 8 个子任务".into());
            }
            let mut recording = Recording {
                trace: Trace {
                    partner_id: args.partner_id,
                    todos: Vec::new(),
                    id: uuid::Uuid::new_v4(),
                    call_id: call.id.clone(),
                    title: args
                        .title
                        .filter(|title| !title.trim().is_empty())
                        .unwrap_or_else(|| {
                            args.task.split_whitespace().collect::<Vec<_>>().join(" ")
                        })
                        .chars()
                        .take(28)
                        .collect(),
                    task: args.task.clone(),
                    status: Status::Running,
                    blocks: Vec::new(),
                    tools: Vec::new(),
                    thinking_ms: Default::default(),
                    stats: None,
                    elapsed_ms: 0,
                    error: None,
                },
                timings: Default::default(),
                started: std::time::Instant::now(),
                registry: context.subagents.clone(),
                events: context.events.clone(),
            };
            recording.publish();
            let result = self
                .complete_child(&args.task, args.partner_id, context, &mut recording)
                .await;
            recording.finish(&result);
            result.map(|completion| completion.text)
        })
    }

    async fn complete_child(
        &self,
        task: &str,
        partner_id: Option<uuid::Uuid>,
        context: &ToolContext,
        recording: &mut Recording,
    ) -> Result<crate::generation::Completion, String> {
        let (events, receiver) = async_channel::unbounded();
        let mut child = context.clone();
        child.delegation = Delegation::Child;
        child.events = events;
        child.recall = None;
        child.steering = None;
        child.todos = Default::default();
        let mut messages = self.instructions.clone();
        if let Some(partner) = partner_id {
            // A peer sees only its own identity, not the parent's resident memories.
            messages.retain(|m| {
                !m["content"].as_str().is_some_and(|s| {
                    s.starts_with("你正在以一个持续存在的伙伴身份工作")
                        || s.starts_with("自动回忆的候选记忆")
                })
            });
            child.memory = child.memory.for_partner(Some(partner));
            if let Some(identity) = crate::partners::Repository::new(child.memory.anchor.clone())
                .context(Some(partner), child.root.as_deref())?
            {
                messages.push(json!({"role":"system","content":identity}));
            }
        }
        messages.push(json!({"role":"system","content":"You are a subagent executing a single delegated task. Work independently using the provided tools and return a concise result with evidence and any limitations to the parent agent. You cannot delegate work or create further subagents. Follow the inherited project boundaries and approval policy. You share the project files with the parent; avoid unrelated changes."}));
        messages.push(json!({"role":"user","content":task}));
        let request = provider::Request::start_with_tools(
            self.config.clone(),
            self.key.clone(),
            messages,
            self.thinking,
            Some(child),
        )?;
        let response = tokio::time::timeout(Duration::from_secs(120), request.response());
        tokio::pin!(response);
        let mut events_open = true;
        loop {
            tokio::select! {
                biased;
                event = receiver.recv(), if events_open => {
                    let Ok(event) = event else {
                        events_open = false;
                        continue;
                    };
                    recording.observe(&event);
                    if let ToolEvent::Approval { call, respond, .. } = event {
                        context.events.send(ToolEvent::Approval { call, respond, agent_id: Some(recording.trace.id) }).await.map_err(|_| "界面已关闭")?;
                    }
                }
                result = &mut response => {
                    let completion = result.map_err(|_| "子代理执行超时，已取消")??;
                    if completion.text.trim().is_empty() {
                        return Err("子代理未返回文本结果".into());
                    }
                    return Ok(completion);
                }
            }
        }
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
            subagents: Default::default(),
            todos: Default::default(),
            steering: None,
            registry: Default::default(),
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
                                "todo",
                                "todo_write",
                                json!({"todos":[{"content":"child-plan-sentinel","status":"pending"}]}),
                            ),
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
            let parent_todos = context.todos.clone();
            *parent_todos.lock().unwrap() = vec![crate::todo::Item {
                content: "parent-plan-sentinel".into(),
                status: crate::todo::Status::Pending,
            }];
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
                    assert!(
                        !matches!(&event, ToolEvent::TodoUpdated(_)),
                        "child checklist must not update the parent panel"
                    );
                    if let ToolEvent::Approval {
                        call,
                        respond,
                        agent_id,
                    } = event
                    {
                        assert_eq!(call.name, "write_file");
                        assert!(agent_id.is_some());
                        respond.send(false).unwrap();
                    }
                }
            };
            let (result, _) = tokio::join!(request.response(), approvals);
            let completion = result.unwrap();
            assert_eq!(completion.text, "parent-result");
            let trace = completion.tool_records[0].subagent.as_ref().unwrap();
            let trace_id = trace.id;
            assert_eq!(trace.status, Status::Completed);
            assert_eq!(trace.task, "Inspect the project");
            assert_eq!(trace.tools.len(), 3);
            assert_eq!(trace.todos[0].content, "child-plan-sentinel");
            assert_eq!(
                parent_todos.lock().unwrap()[0].content,
                "parent-plan-sentinel"
            );
            assert!(trace.tools.iter().all(|tool| tool.subagent.is_none()));
            assert!(trace.blocks.iter().any(|block| matches!(block, crate::generation::ContentBlock::Text(text) if text == "child-result")));
            let mut history = crate::model::History::default();
            history
                .active_mut()
                .push(crate::model::Role::Agent, completion.text);
            history.active_mut().messages.last_mut().unwrap().tools = completion.tool_records;
            history.active_mut().messages.last_mut().unwrap().blocks = completion.blocks;
            let path = directory.path().join("roundtrip/sessions.json");
            history.save(&path).unwrap();
            let loaded = crate::model::History::load(&path).unwrap();
            let saved = loaded.active().messages[0].tools[0]
                .subagent
                .as_ref()
                .unwrap();
            assert_eq!(saved.id, trace_id);
            assert_eq!(saved.status, Status::Completed);
            assert_eq!(saved.tools.len(), 3);
            assert_eq!(saved.todos[0].content, "child-plan-sentinel");
            assert!(!directory.path().join("denied.txt").exists());
            let requests = server.join().unwrap();
            assert!(requests[0]["tools"].to_string().contains("\"subagent\""));
            for request in &requests[1..3] {
                assert!(!request["tools"].to_string().contains("\"subagent\""));
                assert!(!request.to_string().contains("private-parent-history"));
                assert!(!request.to_string().contains("parent-plan-sentinel"));
                assert!(request.to_string().contains("project-instructions"));
            }
            assert!(requests[2].to_string().contains("用户拒绝"));
            assert!(requests[2].to_string().contains("不允许继续派生"));
            assert!(requests[3].to_string().contains("child-result"));
            assert!(!requests[3].to_string().contains("must not write"));
        }
    }

    #[test]
    fn cancellation_keeps_streamed_blocks_output_and_approval_state_without_ui_events() {
        use crate::generation::ContentBlock;
        let registry = Registry::default();
        let (events, _receiver) = async_channel::unbounded();
        let id = uuid::Uuid::new_v4();
        let mut recording = Recording {
            trace: Trace {
                partner_id: None,
                todos: Vec::new(),
                id,
                call_id: "parent-call".into(),
                title: "验证".into(),
                task: "Run verification".into(),
                status: Status::Running,
                blocks: Vec::new(),
                tools: Vec::new(),
                thinking_ms: Default::default(),
                stats: None,
                elapsed_ms: 0,
                error: None,
            },
            timings: Default::default(),
            started: std::time::Instant::now(),
            registry: registry.clone(),
            events,
        };
        let call = ToolCall {
            id: "same-provider-call-id".into(),
            name: "run_command".into(),
            arguments: json!({"program":"cargo","args":["test"]}),
        };
        recording.observe(&ToolEvent::Transcript(vec![ContentBlock::Thinking(
            "Inspect first".into(),
        )]));
        recording.observe(&ToolEvent::Transcript(vec![
            ContentBlock::Thinking("Inspect first".into()),
            ContentBlock::ToolCall(call.clone()),
        ]));
        recording.observe(&ToolEvent::Started(call.clone()));
        let (respond, _response) = tokio::sync::oneshot::channel();
        recording.observe(&ToolEvent::Approval {
            call: call.clone(),
            respond,
            agent_id: None,
        });
        assert_eq!(registry.lock().unwrap()[0].status, Status::WaitingApproval);
        recording.observe(&ToolEvent::ApprovalResolved);
        assert_eq!(registry.lock().unwrap()[0].status, Status::Running);
        recording.observe(&ToolEvent::Output {
            call_id: call.id,
            output: "Compiling glint-agent\n".into(),
        });
        drop(recording);
        let trace = registry.lock().unwrap()[0].clone();
        assert_eq!(trace.id, id);
        assert_eq!(trace.status, Status::Cancelled);
        assert!(trace.error.is_some());
        assert!(trace.thinking_ms.contains_key(&0));
        assert!(trace.tools[0].result.contains("Compiling glint-agent"));
        assert!(
            matches!(trace.blocks.last(), Some(ContentBlock::ToolResult(record)) if !record.success)
        );
        let persisted: Trace =
            serde_json::from_str(&serde_json::to_string(&trace).unwrap()).unwrap();
        assert!(persisted.tools[0].result.contains("Compiling glint-agent"));
        let mut legacy = serde_json::to_value(&persisted.tools[0]).unwrap();
        legacy.as_object_mut().unwrap().remove("subagent");
        assert!(
            serde_json::from_value::<tools::ToolRecord>(legacy)
                .unwrap()
                .subagent
                .is_none()
        );
    }

    #[tokio::test]
    async fn dropping_the_parent_tool_cancels_the_child_and_keeps_its_trace() {
        let directory = tempfile::tempdir().unwrap();
        // Keep the port open without returning an HTTP response.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let config = provider::Config {
            model: "test-model".into(),
            base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
            ..Default::default()
        };
        let runner = Runner::new(
            config,
            "test-only-key".into(),
            provider::Thinking::Default,
            &[],
        );
        let (events, _receiver) = async_channel::unbounded();
        let context = context(directory.path(), events);
        let call = ToolCall {
            id: "cancel-parent".into(),
            name: "subagent".into(),
            arguments: json!({"task":"Wait for response","title":"等待验证"}),
        };
        assert!(
            tokio::time::timeout(Duration::from_millis(50), runner.run(&call, &context))
                .await
                .is_err()
        );
        let trace = context.subagents.lock().unwrap()[0].clone();
        assert_eq!(trace.status, Status::Cancelled);
        assert_eq!(trace.title, "等待验证");
        assert_eq!(trace.call_id, "cancel-parent");
        assert!(trace.error.is_some());
    }
}
