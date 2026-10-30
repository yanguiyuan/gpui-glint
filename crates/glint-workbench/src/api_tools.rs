//! Asynchronous model-runtime to GPUI bridge. No MCP server or socket is involved.
use glint_agent::host_tools::{HostToolFuture, HostTools};
use gpui::{Context, Entity, Task};
use serde_json::Value;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(30);
const QUEUED: u8 = 0;
const STARTED: u8 = 1;
const CANCELLED: u8 = 2;

struct Invocation {
    id: String,
    name: String,
    arguments: Value,
    deadline: Instant,
    state: Arc<AtomicU8>,
    reply: async_channel::Sender<Result<Value, String>>,
}

// Dropping a model turn atomically cancels only calls which have not started.
// Running API operations retain their own explicit cancel/status tools.
struct Pending(Arc<AtomicU8>);
impl Drop for Pending {
    fn drop(&mut self) {
        let _ = self
            .0
            .compare_exchange(QUEUED, CANCELLED, Ordering::SeqCst, Ordering::SeqCst);
    }
}
impl Invocation {
    fn try_start(&self) -> bool {
        !self.reply.is_closed()
            && Instant::now() < self.deadline
            && self
                .state
                .compare_exchange(QUEUED, STARTED, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
    }
}

pub struct ApiTools {
    sender: async_channel::Sender<Invocation>,
    timeout: Duration,
}

impl ApiTools {
    pub fn connect(
        api: &Entity<glint_api::Workbench>,
        cx: &mut Context<super::Workbench>,
    ) -> (Arc<dyn HostTools>, Task<()>) {
        let (sender, receiver) = async_channel::bounded::<Invocation>(32);
        let api = api.downgrade();
        let task = cx.spawn(async move |_, cx| {
            while let Ok(call) = receiver.recv().await {
                // A cancelled model turn must not execute mutations still waiting in the queue.
                if !call.try_start() {
                    if Instant::now() >= call.deadline {
                        let _ = call
                            .reply
                            .try_send(Err("API 工具排队超时，未开始执行".into()));
                    }
                    continue;
                }
                let result = api
                    .update(cx, |api, cx| {
                        api.execute_host_tool(call.id, call.name, call.arguments, call.deadline, cx)
                    })
                    .unwrap_or_else(|_| Err("API 工作区已关闭".into()));
                let _ = call.reply.try_send(result);
            }
        });
        (
            Arc::new(Self {
                sender,
                timeout: TIMEOUT,
            }),
            task,
        )
    }
}

impl HostTools for ApiTools {
    fn catalog(&self) -> Vec<Value> {
        glint_api::Workbench::tool_catalog()
    }

    fn call(&self, id: String, name: String, arguments: Value) -> HostToolFuture {
        let sender = self.sender.clone();
        let timeout = self.timeout;
        Box::pin(async move {
            let (reply, result) = async_channel::bounded(1);
            let state = Arc::new(AtomicU8::new(QUEUED));
            let _pending = Pending(state.clone());
            let call = Invocation {
                state: state.clone(),
                id,
                name,
                arguments,
                deadline: Instant::now() + timeout,
                reply,
            };
            tokio::time::timeout(timeout, async move {
                sender
                    .send(call)
                    .await
                    .map_err(|_| "API 工作区已关闭".to_owned())?;
                let value = result
                    .recv()
                    .await
                    .map_err(|_| "API 工具调用已取消或工作区已关闭".to_owned())??;
                serde_json::to_string(&value).map_err(|error| error.to_string())
            })
            .await
            .map_err(|_| {
                if state.compare_exchange(QUEUED, CANCELLED, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                    "API 工具排队超时，未开始执行".to_owned()
                } else {
                    "API 工具等待结果超时，调用已开始，结果尚未确认；请查询状态或调用历史，勿直接重试写操作".to_owned()
                }
            })?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bridge_returns_results_and_cancelled_call_closes_its_reply() {
        let (sender, receiver) = async_channel::bounded::<Invocation>(2);
        let tools = ApiTools {
            sender,
            timeout: TIMEOUT,
        };
        let first = tokio::spawn(tools.call(
            "first".into(),
            "glint_workspace_get".into(),
            serde_json::json!({}),
        ));
        let call = receiver.recv().await.unwrap();
        assert_eq!(call.id, "first");
        call.reply
            .send(Ok(serde_json::json!({"live":true})))
            .await
            .unwrap();
        assert_eq!(first.await.unwrap().unwrap(), r#"{"live":true}"#);
        let pending = tokio::spawn(tools.call(
            "cancelled".into(),
            "glint_request_send".into(),
            serde_json::json!({}),
        ));
        let call = receiver.recv().await.unwrap();
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());
        assert!(call.reply.is_closed());
        assert!(!call.try_start());
        drop(receiver);
        assert!(
            tools
                .call(
                    "closed".into(),
                    "glint_workspace_get".into(),
                    serde_json::json!({})
                )
                .await
                .unwrap_err()
                .contains("已关闭")
        );
    }
    #[tokio::test]
    async fn timeout_distinguishes_queued_from_started_calls() {
        let (sender, receiver) = async_channel::bounded::<Invocation>(2);
        let tools = ApiTools {
            sender,
            timeout: Duration::from_millis(100),
        };
        let queued = tokio::spawn(tools.call(
            "queued".into(),
            "glint_request_send".into(),
            serde_json::json!({}),
        ));
        let call = receiver.recv().await.unwrap();
        assert!(queued.await.unwrap().unwrap_err().contains("未开始执行"));
        assert!(!call.try_start());
        let started = tokio::spawn(tools.call(
            "started".into(),
            "glint_request_send".into(),
            serde_json::json!({}),
        ));
        let call = receiver.recv().await.unwrap();
        assert!(call.try_start());
        assert!(!call.try_start());
        assert!(started.await.unwrap().unwrap_err().contains("勿直接重试"));
    }
}
