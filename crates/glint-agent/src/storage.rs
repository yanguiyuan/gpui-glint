//! A serialized background writer, independent from the view's lifetime.
use super::*;
use gpui_glint::components::{ComposerReference, PastedImage};

pub(super) struct DraftAttachments {
    pub images: Vec<PastedImage>,
    pub references: Vec<ComposerReference>,
}

#[derive(Default)]
pub struct SaveRequest {
    pub history: Option<History>,
    pub dirty_sessions: Vec<(usize, crate::model::Session)>,
    pub session_order: Vec<Uuid>,
    pub drafts: Vec<(Uuid, String)>,
    draft_attachments: std::collections::HashMap<Uuid, DraftAttachments>,
    pub navigation: Option<(Uuid, Option<Uuid>)>,
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
                request.dirty_sessions.clear();
                request.session_order.clear();
                request.drafts.clear();
                request.draft_attachments.clear();
                request.navigation = None;
            } else if !next.dirty_sessions.is_empty() {
                request.session_order = next.session_order;
                for (ordinal, session) in next.dirty_sessions {
                    request
                        .dirty_sessions
                        .retain(|(_, existing)| existing.id != session.id);
                    request.dirty_sessions.push((ordinal, session));
                }
            }
            request.drafts.extend(next.drafts);
            request.draft_attachments.extend(next.draft_attachments);
            if next.navigation.is_some() {
                request.navigation = next.navigation;
            }
            request.revision = next.revision;
        }
        let result = if let Some(mut history) = request.history {
            for (id, draft) in request.drafts {
                if let Some(session) = history.sessions.iter_mut().find(|s| s.id == id) {
                    session.draft = draft;
                }
            }
            for (id, attachments) in request.draft_attachments {
                if let Some(session) = history.sessions.iter_mut().find(|s| s.id == id) {
                    session.draft_images = attachments.images;
                    session.draft_references = attachments.references;
                }
            }
            if let Some((active, partner)) = request.navigation {
                history.active = active;
                history.preferred_partner = partner;
            }
            history.save(&path).map_err(|error| error.to_string())
        } else if !request.dirty_sessions.is_empty() {
            let mut prep_err = None;
            for (_, session) in &mut request.dirty_sessions {
                if let Err(err) = crate::model::prepare_session_for_save(session, &path) {
                    prep_err = Some(err.to_string());
                    break;
                }
            }
            if let Some(err) = prep_err {
                Err(err)
            } else {
                database::save_dirty_sessions(
                    &path,
                    &request.session_order,
                    &request.dirty_sessions,
                    &request.drafts,
                    request.draft_attachments,
                    request.navigation,
                )
            }
        } else {
            database::save_ui_state(
                &path,
                &request.drafts,
                request.draft_attachments,
                request.navigation,
            )
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
            let sessions: Vec<_> = this.runs.keys().copied().collect();
            for id in sessions {
                this.stop_session(id, false, cx);
            }
            this.save_delay = None;
            let (send, receive) = tokio::sync::oneshot::channel();
            if this.storage_readable {
                this.capture_draft(cx);
                this.save_full_pending = true;
                this.enqueue_save(Some(send));
            }
            let anchor = this.path.clone();
            async move {
                crate::command_runner::shutdown(anchor).await;
                let _ = receive.await;
            }
        }));
        self._subscriptions.push(cx.on_release(|this, _| {
            crate::command_runner::cancel_anchor(&this.path);
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

    pub fn save_session(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.dirty_sessions.insert(id);
        self.save_draft(cx);
    }

    pub fn save_draft(&mut self, cx: &mut Context<Self>) {
        if !self.storage_readable {
            return;
        }
        self.save_drafts_pending
            .insert(self.history.active, self.history.active().draft.clone());
        self.save_revision += 1;
        self.save_delay = Some(cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(300))
                .await;
            let _ = weak.update(cx, |this, _| this.enqueue_save(None));
        }));
    }

    pub(super) fn save_draft_attachments(&mut self, cx: &mut Context<Self>) {
        if !self.storage_readable {
            return;
        }
        let session = self.history.active();
        self.draft_attachments_pending.insert(
            session.id,
            DraftAttachments {
                images: session.draft_images.clone(),
                references: session.draft_references.clone(),
            },
        );
        self.save_draft(cx);
    }

    fn enqueue_save(&mut self, complete: Option<tokio::sync::oneshot::Sender<()>>) {
        if let Some(sender) = &self.save_sender {
            let (history, dirty_sessions, session_order) = if self.save_full_pending {
                self.dirty_sessions.clear();
                self.draft_attachments_pending.clear();
                (Some(self.history.clone()), Vec::new(), Vec::new())
            } else if !self.dirty_sessions.is_empty() {
                let dirty: Vec<_> = self
                    .history
                    .sessions
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| self.dirty_sessions.contains(&s.id))
                    .map(|(ordinal, s)| (ordinal, s.clone()))
                    .collect();
                let order: Vec<_> = self.history.sessions.iter().map(|s| s.id).collect();
                self.dirty_sessions.clear();
                (None, dirty, order)
            } else {
                (None, Vec::new(), Vec::new())
            };
            let _ = sender.try_send(SaveRequest {
                history,
                dirty_sessions,
                session_order,
                drafts: self.save_drafts_pending.drain().collect(),
                draft_attachments: std::mem::take(&mut self.draft_attachments_pending),
                navigation: Some((self.history.active, self.history.preferred_partner)),
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

    #[gpui::test]
    fn adding_and_removing_composer_images_only_queues_the_active_draft(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let workspace =
            cx.new(|cx| AgentWorkspace::with_path(directory.path().join("sessions.json"), cx));
        let (sender, receiver) = async_channel::unbounded();
        let (id, composer) = workspace.update(cx, |workspace, _| {
            workspace.save_full_pending = false;
            workspace.save_sender = Some(sender);
            (workspace.history.active, workspace.composer.clone())
        });
        let image = PastedImage::new(gpui::ImageFormat::Png, vec![1, 2, 3]);
        composer.update(cx, |composer, cx| {
            composer.set_draft("draft", vec![image.clone()], cx);
            cx.emit(ComposerEvent::AttachmentsChanged);
        });
        workspace.update(cx, |workspace, _| workspace.enqueue_save(None));
        let request = receiver.try_recv().unwrap();
        assert!(request.history.is_none());
        assert_eq!(request.draft_attachments[&id].images, vec![image]);

        composer.update(cx, |composer, cx| composer.clear_attachments(cx));
        workspace.update(cx, |workspace, _| workspace.enqueue_save(None));
        let request = receiver.try_recv().unwrap();
        assert!(request.history.is_none());
        assert!(request.draft_attachments[&id].images.is_empty());
    }

    #[gpui::test]
    fn switching_saves_only_navigation_and_all_pending_drafts(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let workspace =
            cx.new(|cx| AgentWorkspace::with_path(directory.path().join("sessions.json"), cx));
        let (sender, receiver) = async_channel::unbounded();
        workspace.update(cx, |workspace, cx| {
            let first = workspace.history.active;
            workspace
                .history
                .active_mut()
                .push(Role::User, "large history".repeat(10000));
            workspace.history.new_session_for(None);
            let second = workspace.history.active;
            workspace.history.active = first;
            workspace
                .composer
                .update(cx, |composer, cx| composer.set_value("outgoing draft", cx));
            workspace.save_full_pending = false;
            workspace.save_sender = Some(sender);
            workspace.switch(second, cx);
            assert!(
                !workspace.save_full_pending,
                "navigation must not request a historical snapshot"
            );
            workspace.save_delay = None;
            workspace.enqueue_save(None);
            let request = receiver.try_recv().unwrap();
            assert!(request.history.is_none());
            assert_eq!(request.navigation, Some((second, None)));
            assert!(request.drafts.contains(&(first, "outgoing draft".into())));
            assert!(request.drafts.contains(&(second, String::new())));
        });
    }

    #[tokio::test]
    async fn lightweight_navigation_preserves_messages_and_coalesces_with_snapshots() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sessions.json");
        let mut history = History::default();
        history
            .active_mut()
            .push(Role::User, "large history".repeat(10000));
        let first = history.active;
        history.new_session_for(None);
        history
            .active_mut()
            .push(Role::Agent, "second message".into());
        let second = history.active;
        history.active = first;
        history.save(&path).unwrap();
        for full in [false, true] {
            let (sender, receiver) = async_channel::unbounded();
            let (results, completed) = async_channel::unbounded();
            if full {
                sender
                    .try_send(SaveRequest {
                        history: Some(history.clone()),
                        revision: 1,
                        ..Default::default()
                    })
                    .unwrap();
            }
            sender
                .try_send(SaveRequest {
                    drafts: vec![
                        (first, "first draft".into()),
                        (second, "second draft".into()),
                    ],
                    navigation: Some((second, None)),
                    revision: 2,
                    ..Default::default()
                })
                .unwrap();
            drop(sender);
            write_requests(receiver, path.clone(), results).await;
            completed.recv().await.unwrap().1.unwrap();
            let saved = History::load(&path).unwrap();
            assert_eq!(saved.active, second);
            assert_eq!(saved.active().draft, "second draft");
            let previous = saved.sessions.iter().find(|s| s.id == first).unwrap();
            assert_eq!(previous.draft, "first draft");
            assert_eq!(previous.messages[0].text, "large history".repeat(10000));
            assert_eq!(saved.active().messages[0].text, "second message");
        }
    }

    #[tokio::test]
    async fn draft_updates_preserve_messages_and_follow_full_snapshots() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sessions.json");
        let mut history = History::default();
        history
            .active_mut()
            .push(Role::User, "large history".repeat(1000));
        let id = history.active;
        let image = PastedImage::new(gpui::ImageFormat::Png, vec![1, 2, 3]);
        let reference = ComposerReference::file("src/main.rs", false);
        let (sender, receiver) = async_channel::unbounded();
        let (results, completed) = async_channel::unbounded();
        sender
            .try_send(SaveRequest {
                history: Some(history),
                revision: 1,
                ..Default::default()
            })
            .unwrap();
        sender
            .try_send(SaveRequest {
                drafts: vec![(id, "new draft".into())],
                draft_attachments: [(
                    id,
                    DraftAttachments {
                        images: vec![image.clone()],
                        references: vec![reference.clone()],
                    },
                )]
                .into(),
                revision: 2,
                ..Default::default()
            })
            .unwrap();
        drop(sender);
        write_requests(receiver, path.clone(), results).await;
        completed.recv().await.unwrap().1.unwrap();
        let saved = History::load(&path).unwrap();
        assert_eq!(saved.active().draft, "new draft");
        assert_eq!(saved.active().draft_images, vec![image]);
        assert_eq!(saved.active().draft_references, vec![reference]);
        assert_eq!(
            saved.active().messages[0].text,
            "large history".repeat(1000)
        );

        // A later standalone draft uses the UPDATE path, without any history snapshot.
        let (sender, receiver) = async_channel::unbounded();
        let (results, completed) = async_channel::unbounded();
        sender
            .try_send(SaveRequest {
                drafts: vec![(id, "latest".into())],
                draft_attachments: [(
                    id,
                    DraftAttachments {
                        images: vec![],
                        references: vec![],
                    },
                )]
                .into(),
                revision: 3,
                ..Default::default()
            })
            .unwrap();
        drop(sender);
        write_requests(receiver, path.clone(), results).await;
        completed.recv().await.unwrap().1.unwrap();
        let saved = History::load(&path).unwrap();
        assert_eq!(saved.active().draft, "latest");
        assert!(saved.active().draft_images.is_empty());
        assert!(saved.active().draft_references.is_empty());
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
                revision: 1,
                complete: Some(done_one),
                ..Default::default()
            })
            .unwrap();
        history.active_mut().draft = "latest".into();
        let (done_two, flushed_two) = tokio::sync::oneshot::channel();
        sender
            .try_send(SaveRequest {
                history: Some(history),
                revision: 2,
                complete: Some(done_two),
                ..Default::default()
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

    #[gpui::test]
    fn incremental_session_save_only_clones_dirty_session_and_persists_to_sqlite(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sessions.json");
        let mut initial = History::default();
        initial
            .active_mut()
            .push(Role::User, "existing large history".repeat(500));
        let first_id = initial.active;
        initial.save(&path).unwrap();

        let workspace = cx.new(|cx| AgentWorkspace::with_path(path.clone(), cx));
        let (sender, receiver) = async_channel::unbounded();
        let (new_id, composer) = workspace.update(cx, |workspace, cx| {
            assert!(!workspace.save_full_pending);
            workspace.save_sender = Some(sender);
            workspace.history.new_default_session();
            let new_id = workspace.history.active;
            workspace.save_session(new_id, cx);
            (new_id, workspace.composer.clone())
        });
        composer.update(cx, |composer, cx| {
            composer.set_draft("hello", Vec::new(), cx);
            cx.emit(ComposerEvent::Changed("hello".into()));
        });
        workspace.update(cx, |workspace, _| workspace.enqueue_save(None));
        let request = receiver.try_recv().unwrap();
        assert!(
            request.history.is_none(),
            "save_session must not clone full history on the main thread"
        );
        assert_eq!(request.dirty_sessions.len(), 1);
        assert_eq!(request.dirty_sessions[0].1.id, new_id);

        let (results, completed) = async_channel::unbounded();
        let (tx, rx) = async_channel::unbounded();
        tx.try_send(request).unwrap();
        drop(tx);
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(write_requests(rx, path.clone(), results));
        completed.try_recv().unwrap().1.unwrap();

        let loaded = History::load(&path).unwrap();
        assert_eq!(loaded.active, new_id);
        assert_eq!(loaded.sessions.len(), 2);
        let first = loaded.sessions.iter().find(|s| s.id == first_id).unwrap();
        assert_eq!(first.messages[0].text, "existing large history".repeat(500));
    }
}
