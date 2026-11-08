//! User questions are independent of tool approval, including in full-access mode.
use crate::tools::ToolEvent;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub struct Request {
    pub id: Uuid,
    pub agent_id: Option<Uuid>,
    pub question: String,
    pub options: Vec<String>,
    pub respond: tokio::sync::oneshot::Sender<Option<String>>,
}

struct Cleanup {
    id: Uuid,
    events: async_channel::Sender<ToolEvent>,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = self.events.try_send(ToolEvent::QuestionResolved(self.id));
    }
}

pub async fn execute(
    args: &Value,
    events: &async_channel::Sender<ToolEvent>,
) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        question: String,
        #[serde(default)]
        options: Vec<String>,
    }
    let args: Args =
        serde_json::from_value(args.clone()).map_err(|e| format!("提问参数无效：{e}"))?;
    if args.question.trim().is_empty()
        || args.question.len() > 8000
        || args.options.len() > 6
        || args
            .options
            .iter()
            .any(|s| s.trim().is_empty() || s.len() > 1000)
    {
        return Err("问题不能为空且最多 8000 字节；最多 6 个非空选项，每项最多 1000 字节".into());
    }
    let id = Uuid::new_v4();
    let (respond, response) = tokio::sync::oneshot::channel();
    let _cleanup = Cleanup {
        id,
        events: events.clone(),
    };
    events
        .send(ToolEvent::Question(Request {
            id,
            agent_id: None,
            question: args.question,
            options: args.options,
            respond,
        }))
        .await
        .map_err(|_| "界面已关闭")?;
    let answer = response.await.map_err(|_| "提问已取消或界面已关闭")?;
    Ok(
        json!({"status":if answer.is_some() { "answered" } else { "skipped" },"answer":answer})
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn waits_for_custom_answer_or_skip_and_cleans_up() {
        for answer in [Some("自定义回答".to_owned()), None] {
            let (events, receiver) = async_channel::unbounded();
            let args = json!({"question":"选择方案？","options":["方案一","方案二"]});
            let execution = execute(&args, &events);
            tokio::pin!(execution);
            let request = tokio::select! {
                result = &mut execution => panic!("question did not wait: {result:?}"),
                event = receiver.recv() => match event.unwrap() {
                    ToolEvent::Question(request) => request,
                    _ => panic!("expected question"),
                }
            };
            assert_eq!(request.options.len(), 2);
            let id = request.id;
            request.respond.send(answer.clone()).unwrap();
            let result: Value = serde_json::from_str(&execution.await.unwrap()).unwrap();
            assert_eq!(result["answer"], json!(answer));
            assert_eq!(
                result["status"],
                if answer.is_some() {
                    "answered"
                } else {
                    "skipped"
                }
            );
            assert!(
                matches!(receiver.recv().await.unwrap(), ToolEvent::QuestionResolved(found) if found == id)
            );
        }
    }

    #[tokio::test]
    async fn tool_waits_for_answer_in_every_access_mode_and_records_it() {
        use crate::tools::{AccessMode, ToolCall, execute as execute_tool};
        for mode in [
            AccessMode::Ask,
            AccessMode::AutoApprove,
            AccessMode::FullAccess,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut context = crate::edits::tests::context(dir.path());
            *context.access.lock().unwrap() = mode;
            let (events, receiver) = async_channel::unbounded();
            context.events = events;
            let execution = execute_tool(
                ToolCall {
                    id: "question-tool".into(),
                    name: "ask_question".into(),
                    arguments: json!({"question":"Q"}),
                },
                &context,
            );
            tokio::pin!(execution);
            loop {
                tokio::select! {
                    result = &mut execution => panic!("tool did not wait: {result:?}"),
                    event = receiver.recv() => match event.unwrap() {
                        ToolEvent::Question(request) => {
                            request.respond.send(Some("answer".into())).unwrap();
                            break;
                        }
                        ToolEvent::Approval { .. } => panic!("question must not require tool approval"),
                        _ => {}
                    }
                }
            }
            let record = execution.await;
            assert!(record.success, "{}", record.result);
            assert_eq!(
                serde_json::from_str::<Value>(&record.result).unwrap()["answer"],
                "answer"
            );
        }
        for anthropic in [false, true] {
            assert!(
                crate::tools::definitions(anthropic)
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|tool| {
                        if anthropic {
                            tool["name"] == "ask_question"
                        } else {
                            tool["function"]["name"] == "ask_question"
                        }
                    })
            );
        }
    }

    #[tokio::test]
    async fn invalid_input_does_not_prompt_and_cancel_cleans_up() {
        let (events, receiver) = async_channel::unbounded();
        for args in [
            json!({"question":" "}),
            json!({"question":"Q","options":[""]}),
            json!({"question":"Q","options":vec!["a"; 7]}),
        ] {
            assert!(execute(&args, &events).await.is_err());
            assert!(receiver.try_recv().is_err());
        }
        let args = json!({"question":"Q"});
        let mut execution = Box::pin(execute(&args, &events));
        let request = tokio::select! {
            _ = &mut execution => panic!("did not wait"),
            event = receiver.recv() => match event.unwrap() { ToolEvent::Question(q) => q, _ => panic!() }
        };
        drop(execution);
        assert!(request.respond.is_closed());
        assert!(
            matches!(receiver.recv().await.unwrap(), ToolEvent::QuestionResolved(id) if id == request.id)
        );
    }
}
