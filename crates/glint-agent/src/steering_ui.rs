//! Keep visible conversation turns aligned with steering applied at model boundaries.
use super::*;
use generation::ContentBlock;

impl SessionRun {
    pub(super) fn visible_transcript(&self) -> &[ContentBlock] {
        if self.steering_waiting {
            &[]
        } else {
            &self.transcript[self.segment_start.min(self.transcript.len())..]
        }
    }
    pub(super) fn visible_tools(&self) -> &[tools::ToolRecord] {
        if self.steering_waiting {
            &[]
        } else {
            &self.tool_activity[self.segment_tool_start.min(self.tool_activity.len())..]
        }
    }
    pub(super) fn segment_timings(&self) -> std::collections::BTreeMap<usize, u64> {
        self.thinking_timings
            .snapshot()
            .into_iter()
            .filter(|(index, _)| *index < self.transcript.len())
            .filter_map(|(index, ms)| {
                index
                    .checked_sub(self.segment_start)
                    .map(|index| (index, ms))
            })
            .collect()
    }
}

impl AgentWorkspace {
    pub(super) fn begin_steering_turn(&mut self, id: Uuid) {
        if self.run(id).steering_waiting {
            return;
        }
        let run = self.run(id);
        let blocks = run.visible_transcript().to_vec();
        let tools = run.visible_tools().to_vec();
        let timings = run.segment_timings();
        let anchor = self
            .history
            .sessions
            .iter()
            .find(|session| session.id == id)
            .map(|session| session.messages.len());
        let mut message_index = None;
        if !blocks.is_empty() || !tools.is_empty() {
            if let Some(session) = self
                .history
                .sessions
                .iter_mut()
                .find(|session| session.id == id)
            {
                session.push(Role::Agent, block_text(&blocks));
                let index = session.messages.len() - 1;
                let message = &mut session.messages[index];
                message.blocks = blocks;
                message.tools = tools;
                message.thinking_ms = timings;
                message_index = Some(index);
            }
        }
        let run = self.run_mut(id);
        run.steering_message = message_index;
        run.steering_anchor = anchor;
        run.steering_waiting = true;
    }

    pub(super) fn refresh_steering_turn(&mut self, id: Uuid) {
        let run = self.run(id);
        let Some(index) = run.steering_message.or(run.steering_anchor) else {
            return;
        };
        let insert = run.steering_message.is_none();
        let blocks = run.transcript[run.segment_start.min(run.transcript.len())..].to_vec();
        let tools =
            run.tool_activity[run.segment_tool_start.min(run.tool_activity.len())..].to_vec();
        let timings = run.segment_timings();
        if insert && blocks.is_empty() && tools.is_empty() {
            return;
        }
        if let Some(session) = self.history.sessions.iter_mut().find(|s| s.id == id) {
            if insert {
                session.push(Role::Agent, String::new());
                let message = session.messages.pop().unwrap();
                session.messages.insert(index, message);
            }
            if let Some(message) = session.messages.get_mut(index) {
                message.text = block_text(&blocks);
                message.blocks = blocks;
                message.tools = tools;
                message.thinking_ms = timings;
                self.process_panels
                    .borrow_mut()
                    .remove(&format!("{id}:{index}"));
            }
            self.run_mut(id).steering_message = Some(index);
        }
    }

    pub(super) fn steering_applied(&mut self, id: Uuid, blocks: Vec<ContentBlock>) {
        self.run_mut(id).thinking_timings.observe(&blocks);
        self.run_mut(id).thinking_timings.finish();
        self.run_mut(id).transcript = blocks;
        self.refresh_steering_turn(id);
        let run = self.run_mut(id);
        run.segment_start = run.transcript.len();
        run.segment_tool_start = run.tool_activity.len();
        run.steering_message = None;
        run.steering_anchor = None;
        run.steering_waiting = false;
    }

    pub(super) fn finish_pending_steering(&mut self, id: Uuid) {
        if self.run(id).steering_waiting {
            let blocks = self.run(id).transcript.clone();
            self.steering_applied(id, blocks);
        }
    }
}

fn block_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| {
            if let ContentBlock::Text(text) = block {
                Some(text.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn steering_before_first_output_still_places_preceding_work_before_the_new_user(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let host =
            cx.new(|cx| AgentWorkspace::with_path(directory.path().join("sessions.json"), cx));
        host.update(cx, |host, _| {
            let id = host.history.active;
            host.history
                .active_mut()
                .push(Role::User, "first task".into());
            host.begin_steering_turn(id);
            host.history
                .active_mut()
                .push(Role::User, "new instruction".into());
            host.steering_applied(id, vec![ContentBlock::Text("first response".into())]);
            let messages = &host.history.active().messages;
            assert_eq!(messages.len(), 3);
            assert_eq!(messages[1].text, "first response");
            assert!(matches!(messages[1].role, Role::Agent));
            assert_eq!(messages[2].text, "new instruction");
        });
    }

    #[gpui::test]
    fn steering_creates_distinct_turns_and_retains_tool_records_after_reload(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let host =
            cx.new(|cx| AgentWorkspace::with_path(directory.path().join("sessions.json"), cx));
        host.update(cx, |host, cx| {
            host.storage_readable = false;
            let id = host.history.active;
            host.history.active_mut().push(Role::User, "first task".into());
            let old = tools::rejected(tools::ToolCall { id: "old-tool".into(), name: "read_file".into(), arguments: serde_json::json!({"path":"old.rs"}) }, "old result".into());
            let new = tools::rejected(tools::ToolCall { id: "new-tool".into(), name: "read_file".into(), arguments: serde_json::json!({"path":"new.rs"}) }, "new result".into());
            let prefix = vec![ContentBlock::Thinking("old thinking".into()), ContentBlock::ToolResult(old.clone()), ContentBlock::Text("old answer".into())];
            host.run_mut(id).transcript = prefix.clone();
            host.run_mut(id).tool_activity = vec![old.clone()];
            host.begin_steering_turn(id);
            host.history.active_mut().push(Role::User, "steered task".into());
            assert!(host.run(id).visible_transcript().is_empty(), "pending steering must not duplicate the preceding work below the new user message");
            assert_eq!(host.history.active().messages.len(), 3);
            assert_eq!(host.history.active().messages[1].tools[0].call.id, "old-tool");
            host.steering_applied(id, prefix.clone());
            let mut blocks = prefix;
            blocks.extend([ContentBlock::Thinking("new thinking".into()), ContentBlock::ToolResult(new.clone()), ContentBlock::Text("new answer".into())]);
            host.run_mut(id).transcript = blocks.clone();
            host.run_mut(id).tool_activity.push(new);
            assert!(matches!(&host.run(id).visible_transcript()[0], ContentBlock::Thinking(text) if text == "new thinking"));
            assert_eq!(host.run(id).visible_tools().len(), 1);
            assert_eq!(host.run(id).visible_tools()[0].call.id, "new-tool");
            host.begin_steering_turn(id);
            host.history.active_mut().push(Role::User, "third task".into());
            host.steering_applied(id, blocks);
            let messages = &host.history.active().messages;
            assert_eq!(messages.len(), 5);
            assert!(matches!(messages[0].role, Role::User));
            assert!(matches!(messages[1].role, Role::Agent));
            assert!(matches!(messages[2].role, Role::User));
            assert!(matches!(messages[3].role, Role::Agent));
            assert!(matches!(messages[4].role, Role::User));
            assert_eq!(messages[3].text, "new answer");
            assert_eq!(messages[3].tools.len(), 1);
            assert_eq!(messages[3].tools[0].call.id, "new-tool");
            let path = directory.path().join("saved/sessions.json");
            host.history.save(&path).unwrap();
            let history = History::load(&path).unwrap();
            assert_eq!(history.active().messages.len(), 5);
            assert_eq!(history.active().messages[1].text, "old answer");
            assert_eq!(history.active().messages[3].text, "new answer");
            let _ = cx;
        });
    }
}
