//! Per-message recall jobs outlive a completed reply, but never a deleted session.
use crate::{memory, provider};
use gpui::{Context, Task};
use std::{collections::HashMap, future::Future, path::PathBuf};
use uuid::Uuid;

#[derive(Default)]
pub struct RecallRunner {
    jobs: HashMap<(Uuid, usize), Task<()>>,
}

pub struct Recalled {
    pub session: Uuid,
    pub message: usize,
    pub recall: memory::Recall,
}

impl gpui::EventEmitter<Recalled> for RecallRunner {}

impl RecallRunner {
    pub fn cancel_all(&mut self) {
        self.jobs.clear();
    }

    pub fn cancel(&mut self, session: Uuid) {
        self.jobs.retain(|(id, _), _| *id != session);
    }

    pub fn generate(
        &mut self,
        session: Uuid,
        message: usize,
        repository: memory::Repository,
        config: provider::Config,
        project: PathBuf,
        query: String,
        sender: async_channel::Sender<memory::Recall>,
        cx: &mut Context<Self>,
    ) {
        self.start(
            session,
            message,
            sender,
            async move {
                let job = provider::Job::start(async move {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        repository.recall(config, &project, &query),
                    )
                    .await
                    .map_err(|_| "记忆召回超时".to_owned())?
                });
                match job {
                    Ok(job) => job.result().await,
                    Err(error) => Err(error),
                }
            },
            cx,
        );
    }

    fn start(
        &mut self,
        session: Uuid,
        message: usize,
        sender: async_channel::Sender<memory::Recall>,
        future: impl Future<Output = Result<memory::Recall, String>> + 'static,
        cx: &mut Context<Self>,
    ) {
        let task = cx.spawn(async move |weak, cx| {
            let recall = future.await.unwrap_or_else(|error| memory::Recall {
                notice: Some(format!("记忆召回暂不可用：{error}")),
                ..Default::default()
            });
            let _ = weak.update(cx, |this, cx| {
                if let Some(job) = this.jobs.remove(&(session, message)) {
                    job.detach();
                }
                // A finished reply has no receiver. Persist its result anyway,
                // so the next user turn can include it without waiting now.
                let _ = sender.try_send(recall.clone());
                cx.emit(Recalled {
                    session,
                    message,
                    recall,
                });
            });
        });
        self.jobs.insert((session, message), task);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gpui::AppContext;
    use std::{cell::RefCell, rc::Rc};

    pub fn sample() -> memory::Recall {
        memory::Recall {
            intent: "安排饮食".into(),
            matches: vec![memory::Card {
                id: Uuid::new_v4(),
                title: "recall-sentinel".into(),
                summary: "偏好清淡".into(),
                category: "饮食偏好".into(),
                project: None,
                updated_at: String::new(),
                reason: "用于选择菜品".into(),
            }],
            ..Default::default()
        }
    }

    #[gpui::test]
    fn late_recall_outlives_reply_and_keeps_its_original_message(cx: &mut gpui::TestAppContext) {
        let runner = cx.new(|_| RecallRunner::default());
        let results = Rc::new(RefCell::new(Vec::new()));
        let received = results.clone();
        let _observer = cx.new(|cx: &mut Context<()>| {
            cx.subscribe(&runner, move |_, _, event: &Recalled, _| {
                received.borrow_mut().push((event.session, event.message));
            })
            .detach();
        });
        let session = Uuid::new_v4();
        let (send, receive) = tokio::sync::oneshot::channel();
        let (sender, inbox) = async_channel::bounded(1);
        let runner_for_reply = runner.clone();
        let mut reply = Some(cx.spawn(async move |mut cx| {
            runner_for_reply.update(&mut cx, |runner, cx| {
                runner.start(
                    session,
                    2,
                    sender,
                    async move { receive.await.map_err(|_| "cancelled".into()) },
                    cx,
                );
            });
        }));
        cx.run_until_parked();
        assert!(inbox.try_recv().is_err());
        assert!(results.borrow().is_empty());
        reply.take();
        send.send(sample()).ok().unwrap();
        cx.run_until_parked();
        assert_eq!(&*results.borrow(), &[(session, 2)]);
        assert_eq!(inbox.try_recv().unwrap().matches.len(), 1);
        assert!(inbox.try_recv().is_err());
        runner.read_with(cx, |runner, _| assert!(runner.jobs.is_empty()));
    }

    #[gpui::test]
    fn stopping_deleting_or_disabling_cancels_recall(cx: &mut gpui::TestAppContext) {
        let runner = cx.new(|_| RecallRunner::default());
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let mut sends = Vec::new();
        for session in [a, b] {
            let (send, receive) = tokio::sync::oneshot::channel();
            let (sender, _inbox) = async_channel::bounded(1);
            runner.update(cx, |runner, cx| {
                runner.start(
                    session,
                    0,
                    sender,
                    async move { receive.await.map_err(|_| "cancelled".into()) },
                    cx,
                )
            });
            sends.push(send);
        }
        cx.run_until_parked();
        runner.update(cx, |runner, _| runner.cancel(a));
        cx.run_until_parked();
        runner.read_with(cx, |runner, _| {
            assert!(!runner.jobs.contains_key(&(a, 0)));
            assert!(runner.jobs.contains_key(&(b, 0)));
        });
        assert!(sends.remove(0).send(sample()).is_err());
        runner.update(cx, |runner, _| runner.cancel_all());
        cx.run_until_parked();
        assert!(sends.remove(0).send(sample()).is_err());
    }

    #[gpui::test]
    fn recall_errors_return_a_notice_without_matches(cx: &mut gpui::TestAppContext) {
        let runner = cx.new(|_| RecallRunner::default());
        let (sender, inbox) = async_channel::bounded(1);
        runner.update(cx, |runner, cx| {
            runner.start(
                Uuid::new_v4(),
                0,
                sender,
                async { Err("timeout".into()) },
                cx,
            )
        });
        cx.run_until_parked();
        let recall = inbox.try_recv().unwrap();
        assert!(recall.matches.is_empty());
        assert!(recall.notice.unwrap().contains("timeout"));
    }
}
