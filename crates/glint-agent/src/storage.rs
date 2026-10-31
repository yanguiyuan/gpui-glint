//! A serialized background writer, independent from the view's lifetime.
use super::*;

pub struct SaveRequest {
    pub history: Option<History>,
    pub drafts: Vec<(Uuid, String)>,
    pub revision: u64,
    pub complete: Option<tokio::sync::oneshot::Sender<()>>,
}

async fn write_requests(
    receiver: async_channel::Receiver<SaveRequest>,
    path: PathBuf,
    results: async_channel::Sender<(u64, Result<(), String>)>,
) {
    while let Ok(mut request) = receiver.recv().await {
        let mut barriers = Vec::new();
        if let Some(complete) = request.complete.take() {
            barriers.push(complete);
        }
        while let Ok(mut next) = receiver.try_recv() {
            if let Some(complete) = next.complete.take() {
                barriers.push(complete);
            }
            if let Some(history) = next.history.take() {
                request.history = Some(history);
                request.drafts.clear();
            }
            request.drafts.extend(next.drafts);
            request.revision = next.revision;
        }
        let result = if let Some(mut history) = request.history {
            for (id, draft) in request.drafts {
                if let Some(session) = history.sessions.iter_mut().find(|s| s.id == id) {
                    session.draft = draft;
                }
            }
            history.save(&path).map_err(|error| error.to_string())
        } else {
            database::save_drafts(&path, &request.drafts)
        };
        let _ = results.try_send((request.revision, result));
        for complete in barriers {
            let _ = complete.send(());
        }
    }
}

impl AgentWorkspace {
    pub fn start_storage(&mut self, cx: &mut Context<Self>) {
        let (sender, receiver) = async_channel::unbounded::<SaveRequest>();
        let (results, completed) = async_channel::unbounded();
        self.save_sender = Some(sender);
        let path = self.path.clone();
        cx.background_executor()
            .spawn(async move {
                write_requests(receiver, path, results).await;
            })
            .detach();
        self.save_worker = Some(cx.spawn(async move |weak, cx| {
            while let Ok((revision, result)) = completed.recv().await {
                let _ = weak.update(cx, |this, cx| {
                    // A failed full snapshot must be retried as a full save,
                    // even when a newer draft revision is already pending.
                    if result.is_err() {
                        this.save_full_pending = true;
                    }
                    if revision == this.save_revision {
                        let error = result
                            .err()
                            .map(|error| format!("会话保存失败：{error}。修改尚未写入磁盘。"));
                        if this.storage_error != error {
                            this.storage_error = error;
                            cx.notify();
                        }
                    }
                });
            }
        }));
        self._subscriptions.push(cx.on_app_quit(|this, cx| {
            this.save_delay = None;
            let (send, receive) = tokio::sync::oneshot::channel();
            if this.storage_readable {
                this.capture_draft(cx);
                this.save_full_pending = true;
                this.enqueue_save(Some(send));
            }
            async move {
                let _ = receive.await;
            }
        }));
        self._subscriptions.push(cx.on_release(|this, _| {
            if this.storage_readable {
                this.save_full_pending = true;
                this.enqueue_save(None);
            }
        }));
    }

    pub fn save(&mut self, cx: &mut Context<Self>) {
        self.save_full_pending = true;
        self.save_draft(cx);
    }

    pub fn save_draft(&mut self, cx: &mut Context<Self>) {
        if !self.storage_readable {
            return;
        }
        self.save_revision += 1;
        self.save_delay = Some(cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(300))
                .await;
            let _ = weak.update(cx, |this, _| this.enqueue_save(None));
        }));
    }

    fn enqueue_save(&mut self, complete: Option<tokio::sync::oneshot::Sender<()>>) {
        if let Some(sender) = &self.save_sender {
            let _ = sender.try_send(SaveRequest {
                history: self.save_full_pending.then(|| self.history.clone()),
                drafts: vec![(self.history.active, self.history.active().draft.clone())],
                revision: self.save_revision,
                complete,
            });
            self.save_full_pending = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn draft_updates_preserve_messages_and_follow_full_snapshots() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sessions.json");
        let mut history = History::default();
        history
            .active_mut()
            .push(Role::User, "large history".repeat(1000));
        let id = history.active;
        let (sender, receiver) = async_channel::unbounded();
        let (results, completed) = async_channel::unbounded();
        sender
            .try_send(SaveRequest {
                history: Some(history),
                drafts: vec![],
                revision: 1,
                complete: None,
            })
            .unwrap();
        sender
            .try_send(SaveRequest {
                history: None,
                drafts: vec![(id, "new draft".into())],
                revision: 2,
                complete: None,
            })
            .unwrap();
        drop(sender);
        write_requests(receiver, path.clone(), results).await;
        completed.recv().await.unwrap().1.unwrap();
        let saved = History::load(&path).unwrap();
        assert_eq!(saved.active().draft, "new draft");
        assert_eq!(
            saved.active().messages[0].text,
            "large history".repeat(1000)
        );

        // A later standalone draft uses the UPDATE path, without any history snapshot.
        let (sender, receiver) = async_channel::unbounded();
        let (results, completed) = async_channel::unbounded();
        sender
            .try_send(SaveRequest {
                history: None,
                drafts: vec![(id, "latest".into())],
                revision: 3,
                complete: None,
            })
            .unwrap();
        drop(sender);
        write_requests(receiver, path.clone(), results).await;
        completed.recv().await.unwrap().1.unwrap();
        let saved = History::load(&path).unwrap();
        assert_eq!(saved.active().draft, "latest");
        assert_eq!(
            saved.active().messages[0].text,
            "large history".repeat(1000)
        );
    }

    #[tokio::test]
    async fn writer_coalesces_snapshots_and_flushes_the_latest_revision() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sessions.json");
        let (sender, receiver) = async_channel::unbounded();
        let (results, completed) = async_channel::unbounded();
        let mut history = History::default();
        history.active_mut().draft = "old".into();
        let (done_one, flushed_one) = tokio::sync::oneshot::channel();
        sender
            .try_send(SaveRequest {
                history: Some(history.clone()),
                drafts: Vec::new(),
                revision: 1,
                complete: Some(done_one),
            })
            .unwrap();
        history.active_mut().draft = "latest".into();
        let (done_two, flushed_two) = tokio::sync::oneshot::channel();
        sender
            .try_send(SaveRequest {
                history: Some(history),
                drafts: Vec::new(),
                revision: 2,
                complete: Some(done_two),
            })
            .unwrap();
        drop(sender);
        write_requests(receiver, path.clone(), results).await;
        flushed_one.await.unwrap();
        flushed_two.await.unwrap();
        let (revision, result) = completed.recv().await.unwrap();
        assert_eq!(revision, 2);
        result.unwrap();
        assert!(completed.try_recv().is_err());
        assert_eq!(History::load(&path).unwrap().active().draft, "latest");
    }
}
