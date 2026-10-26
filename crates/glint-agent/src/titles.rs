use super::*;
use std::{collections::HashMap, future::Future};

pub struct TitleGenerator {
    jobs: HashMap<Uuid, Task<()>>,
}

pub struct GeneratedTitle {
    pub session: Uuid,
    pub previous: String,
    pub title: String,
}

impl gpui::EventEmitter<GeneratedTitle> for TitleGenerator {}

impl TitleGenerator {
    pub fn new() -> Self {
        Self {
            jobs: HashMap::new(),
        }
    }

    pub fn cancel(&mut self, session: Uuid) {
        self.jobs.remove(&session);
    }

    pub fn generate(
        &mut self,
        session: Uuid,
        previous: String,
        config: provider::Config,
        prompt: String,
        reply: String,
        cx: &mut Context<Self>,
    ) {
        self.start(session, previous, async move {
            let key = config.key().ok()?;
            let messages = vec![
                serde_json::json!({"role":"system","content":"你是会话标题生成器。根据这段对话给出不超过 12 个字、概括任务意图的标题。只回复标题本身：不要引号、句号或任何解释。"}),
                serde_json::json!({"role":"user","content":format!("用户：{}\n\n助手：{}", truncate_chars(&prompt, 400), truncate_chars(&reply, 400))}),
            ];
            let request = provider::Request::start(config, key, messages, provider::Thinking::Default).ok()?;
            let completion = request.response().await.ok()?;
            session_title(&completion.text)
        }, cx);
    }

    fn start(
        &mut self,
        session: Uuid,
        previous: String,
        future: impl Future<Output = Option<String>> + 'static,
        cx: &mut Context<Self>,
    ) {
        let task = cx.spawn(async move |weak, cx| {
            let title = future.await;
            let _ = weak.update(cx, |this, cx| {
                // There are no awaits after removing this job's own handle.
                if let Some(job) = this.jobs.remove(&session) {
                    job.detach();
                }
                if let Some(title) = title {
                    cx.emit(GeneratedTitle {
                        session,
                        previous,
                        title,
                    });
                }
            });
        });
        self.jobs.insert(session, task);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn title_job_survives_reply_cleanup_and_cancellation_blocks_deleted_sessions(
        cx: &mut gpui::TestAppContext,
    ) {
        use std::{cell::RefCell, rc::Rc};
        let generator = cx.new(|_| TitleGenerator::new());
        let results = Rc::new(RefCell::new(Vec::new()));
        let received = results.clone();
        let _observer = cx.new(|cx: &mut Context<()>| {
            cx.subscribe(&generator, move |_, _, event: &GeneratedTitle, _| {
                received
                    .borrow_mut()
                    .push((event.session, event.title.clone()));
            })
            .detach();
        });
        let session = Uuid::new_v4();
        let (send, receive) = tokio::sync::oneshot::channel();
        let generator_for_reply = generator.clone();
        let mut pending = Some(cx.spawn(async move |mut cx| {
            generator_for_reply.update(&mut cx, |generator, cx| {
                generator.start(
                    session,
                    "fallback".into(),
                    async move { receive.await.ok() },
                    cx,
                );
            });
        }));
        cx.run_until_parked();
        pending.take();
        send.send("自动标题".into()).unwrap();
        cx.run_until_parked();
        assert_eq!(&*results.borrow(), &[(session, "自动标题".into())]);
        let (send, receive) = tokio::sync::oneshot::channel();
        generator.update(cx, |generator, cx| {
            generator.start(
                session,
                "fallback".into(),
                async move { receive.await.ok() },
                cx,
            );
        });
        cx.run_until_parked();
        generator.update(cx, |generator, _| generator.cancel(session));
        cx.run_until_parked();
        assert!(send.send("late".into()).is_err());
        assert_eq!(results.borrow().len(), 1);
    }
}
