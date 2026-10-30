//! Tools supplied by an embedding application, independent of external MCP servers.
use serde_json::Value;
use std::{future::Future, pin::Pin};

pub type HostToolFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;

/// Definitions use MCP's name/description/inputSchema/annotations shape.
/// Calls run on the model runtime; hosts must marshal GUI work to their UI thread.
pub trait HostTools: Send + Sync {
    fn catalog(&self) -> Vec<Value>;
    fn call(&self, id: String, name: String, arguments: Value) -> HostToolFuture;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{AccessMode, ToolCall, ToolContext, ToolEvent};
    use serde_json::json;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    struct FakeTools(Arc<AtomicUsize>);
    impl HostTools for FakeTools {
        fn catalog(&self) -> Vec<Value> {
            [("glint_workspace_get", true), ("glint_collection_create", false)].into_iter().map(|(name, read_only)| {
                json!({"name":name,"description":"Operate the live API workspace","inputSchema":{"type":"object","properties":{}},"annotations":{"readOnlyHint":read_only}})
            }).collect()
        }
        fn call(&self, id: String, name: String, arguments: Value) -> HostToolFuture {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move {
                if arguments["fail"] == true {
                    return Err("API validation failed".into());
                }
                Ok(json!({"id":id,"tool":name,"arguments":arguments}).to_string())
            })
        }
    }

    #[tokio::test]
    async fn hosted_tools_use_agent_permissions_events_and_both_model_schemas_without_a_project() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sessions.json");
        let config = directory.path().join("mcp.json");
        std::fs::write(&config, "invalid external MCP config").unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let mcp = crate::mcp::Client::new(config)
            .with_host_tools(Some(Arc::new(FakeTools(count.clone()))));
        assert!(mcp.prepare(None).await.is_err());
        assert!(mcp.instructions().contains("内置 Glint API"));
        let (events, receiver) = async_channel::unbounded();
        let mut context = ToolContext {
            steering: None,
            todos: Default::default(),
            subagents: Default::default(),
            delegation: Default::default(),
            mcp,
            skills: Arc::new(crate::skills::Catalog::empty(path.clone())),
            memory: crate::memory::Repository::new(path),
            access: Arc::new(Mutex::new(AccessMode::Ask)),
            root: None,
            schedules: Arc::new(Mutex::new(
                crate::schedules::ScheduleStore::open(directory.path().join("schedules.json"))
                    .unwrap(),
            )),
            provider_id: uuid::Uuid::new_v4(),
            model: "test".into(),
            events,
            recall: None,
        };
        for anthropic in [false, true] {
            let definitions = context.definitions(anthropic);
            let tool = definitions
                .as_array()
                .unwrap()
                .iter()
                .find(|tool| {
                    if anthropic {
                        tool["name"] == "glint_workspace_get"
                    } else {
                        tool["function"]["name"] == "glint_workspace_get"
                    }
                })
                .unwrap();
            assert_eq!(
                if anthropic {
                    &tool["input_schema"]
                } else {
                    &tool["function"]["parameters"]
                },
                &json!({"type":"object","properties":{}})
            );
        }
        let read = ToolCall {
            id: "read".into(),
            name: "glint_workspace_get".into(),
            arguments: json!({}),
        };
        let record = crate::tools::execute(read.clone(), &context).await;
        assert!(record.success);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        let mut completed = false;
        while let Ok(event) = receiver.try_recv() {
            assert!(!matches!(event, ToolEvent::Approval { .. }));
            completed |= matches!(event, ToolEvent::Completed(_));
        }
        assert!(completed);
        let mutation = ToolCall {
            id: "write".into(),
            name: "glint_collection_create".into(),
            arguments: json!({"name":"API"}),
        };
        for call in [
            mutation.clone(),
            ToolCall {
                arguments: json!({"include_secrets":true}),
                ..read.clone()
            },
        ] {
            let reject = async {
                while let Ok(event) = receiver.recv().await {
                    if let ToolEvent::Approval { respond, .. } = event {
                        respond.send(false).unwrap();
                        break;
                    }
                }
            };
            let (record, _) = tokio::join!(crate::tools::execute(call, &context), reject);
            assert!(!record.success);
            assert_eq!(count.load(Ordering::SeqCst), 1);
        }
        let approve = async {
            while let Ok(event) = receiver.recv().await {
                if let ToolEvent::Approval { respond, .. } = event {
                    respond.send(true).unwrap();
                    break;
                }
            }
        };
        let (record, _) = tokio::join!(crate::tools::execute(mutation.clone(), &context), approve);
        assert!(record.success);
        assert_eq!(count.load(Ordering::SeqCst), 2);
        for mode in [AccessMode::AutoApprove, AccessMode::FullAccess] {
            *context.access.lock().unwrap() = mode;
            assert!(
                crate::tools::execute(mutation.clone(), &context)
                    .await
                    .success
            );
        }
        let failed = crate::tools::execute(
            ToolCall {
                arguments: json!({"fail":true}),
                ..read
            },
            &context,
        )
        .await;
        assert!(!failed.success);
        assert_eq!(failed.result, "API validation failed");
        context.delegation = crate::subagent::Delegation::Child;
        assert!(
            context
                .definitions(true)
                .as_array()
                .unwrap()
                .iter()
                .any(|tool| tool["name"] == "glint_workspace_get")
        );
        assert!(!crate::mcp::Client::default().has_host_tool("glint_workspace_get"));
    }
}
