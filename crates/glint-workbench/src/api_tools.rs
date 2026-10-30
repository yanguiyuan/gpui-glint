//! Asynchronous model-runtime to GPUI bridge. No MCP server or socket is involved.
use glint_agent::host_tools::{HostToolFuture, HostTools};
use gpui::{Context, Entity, Task};
use serde_json::Value;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(30);

struct Invocation {
    id: String,
    name: String,
    arguments: Value,
    deadline: Instant,
    reply: async_channel::Sender<Result<Value, String>>,
}

pub struct ApiTools {
    sender: async_channel::Sender<Invocation>,
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
                if call.reply.is_closed() {
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
        (Arc::new(Self { sender }), task)
    }
}

impl HostTools for ApiTools {
    fn catalog(&self) -> Vec<Value> {
        glint_api::Workbench::tool_catalog()
    }

    fn call(&self, id: String, name: String, arguments: Value) -> HostToolFuture {
        let sender = self.sender.clone();
        Box::pin(async move {
            let (reply, result) = async_channel::bounded(1);
            let call = Invocation {
                id,
                name,
                arguments,
                deadline: Instant::now() + TIMEOUT,
                reply,
            };
            tokio::time::timeout(TIMEOUT, async move {
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
            .map_err(|_| "API 工具调用超时".to_owned())?
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bridge_returns_results_and_cancelled_call_closes_its_reply() {
        let (sender, receiver) = async_channel::bounded::<Invocation>(2);
        let tools = ApiTools { sender };
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
}
