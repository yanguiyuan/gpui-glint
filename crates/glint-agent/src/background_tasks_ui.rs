use super::*;
use gpui_glint::components::{BackgroundTask, BackgroundTasksMenu};

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn background_tasks_switch_running_sessions_and_preserve_drafts(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (agent, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        let id = agent.update(window, |agent, cx| {
            let mut session = model::Session::new();
            session.project = Some(dir.path().join("other"));
            session.title = "另一个项目的任务".into();
            let id = session.id;
            agent.history.sessions.push(session);
            agent.run_mut(id).pending = Some((
                Uuid::new_v4(),
                cx.spawn(async move |_, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(3600))
                        .await;
                }),
            ));
            agent
                .composer
                .update(cx, |composer, cx| composer.set_value("当前草稿", cx));
            cx.notify();
            id
        });
        let trigger = window.debug_bounds("background-tasks-trigger").unwrap();
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        let selector: &'static str =
            Box::leak(format!("background-task-session:{id}").into_boxed_str());
        let row = window.debug_bounds(selector).unwrap();
        window.simulate_click(row.center(), gpui::Modifiers::default());
        agent.read_with(window, |agent, _| {
            assert_eq!(agent.history.active, id);
            assert!(agent.history.sessions.iter().any(|s| s.draft == "当前草稿"));
            assert!(agent.session_busy(id));
            assert!(!agent.show_background_tasks);
        });
    }
}

impl AgentWorkspace {
    pub(super) fn start_background_tasks_refresh(&mut self, cx: &mut Context<Self>) {
        self.background_tasks_refresh = Some(cx.spawn(async move |this, cx| {
            let mut previous = Vec::new();
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                let result = this.update(cx, |this, cx| {
                    let current: Vec<_> = this
                        .background_tasks(cx)
                        .into_iter()
                        .map(|task| task.id)
                        .collect();
                    if current != previous {
                        previous = current;
                        cx.notify();
                    }
                });
                if result.is_err() {
                    break;
                }
            }
        }));
    }
    pub fn set_host_background_tasks(&mut self, hosted: bool) {
        self.host_background_tasks = hosted;
    }
    pub fn background_tasks(&self, cx: &App) -> Vec<BackgroundTask> {
        let mut tasks: Vec<_> = self
            .history
            .sessions
            .iter()
            .filter(|s| self.session_busy(s.id))
            .map(|s| BackgroundTask {
                id: format!("session:{}", s.id),
                label: s.title.clone(),
                detail: s
                    .project
                    .as_deref()
                    .map(ssh_project::label)
                    .unwrap_or_default(),
                group: "Agent 会话".into(),
                icon: Icon::Workflow,
            })
            .collect();
        tasks.extend(
            self.running_project_action_tasks(cx)
                .into_iter()
                .map(|(root, label)| BackgroundTask {
                    id: format!("action:{}", root.to_string_lossy()),
                    label,
                    detail: root.to_string_lossy().into_owned(),
                    group: "项目操作".into(),
                    icon: Icon::Play,
                }),
        );
        tasks.extend(
            self.running_terminal_tasks(cx)
                .into_iter()
                .map(|(root, status)| BackgroundTask {
                    id: format!("terminal:{}", root.to_string_lossy()),
                    label: ssh_project::label(&root),
                    detail: status,
                    group: "项目终端".into(),
                    icon: Icon::Code,
                }),
        );
        tasks.extend(
            command_runner::active_commands(&self.memory.anchor)
                .into_iter()
                .map(|run| BackgroundTask {
                    id: run
                        .session
                        .map(|session| format!("command:{session}:{}", run.id))
                        .unwrap_or_else(|| format!("command:{}", run.id)),
                    label: run.label,
                    detail: run
                        .root
                        .as_deref()
                        .map(ssh_project::label)
                        .unwrap_or_default(),
                    group: "命令进程".into(),
                    icon: Icon::Code,
                }),
        );
        tasks
    }
    fn focus_background_project(&mut self, root: &std::path::Path, cx: &mut Context<Self>) {
        if self.history.active().project.as_deref() == Some(root) {
            return;
        }
        let id = self
            .history
            .sessions
            .iter()
            .find(|s| s.project.as_deref() == Some(root))
            .map(|s| s.id);
        let id = id.unwrap_or_else(|| {
            let mut session = model::Session::new();
            session.project = Some(root.into());
            if !self.history.projects.iter().any(|p| p == root) {
                self.history.projects.push(root.into());
            }
            let id = session.id;
            self.history.sessions.push(session);
            id
        });
        self.switch(id, cx);
    }
    pub fn focus_background_task(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.show_background_tasks = false;
        if let Some(id) = id.strip_prefix("session:") {
            if let Ok(id) = Uuid::parse_str(id) {
                if self.history.sessions.iter().any(|s| s.id == id) {
                    self.switch(id, cx);
                }
            }
        } else if let Some(root) = id.strip_prefix("action:") {
            self.focus_background_project(std::path::Path::new(root), cx);
            self.page = crate::Page::Chat;
            self.project_actions.panel = true;
            self.terminal_ui.open = false;
            self.file_preview.open = false;
            self.edits_ui.open = false;
        } else if let Some(root) = id.strip_prefix("terminal:") {
            let root = std::path::Path::new(root);
            if self
                .adopt_panel_repository(
                    crate::panel_repositories::Panel::Terminal,
                    &root.to_string_lossy(),
                    cx,
                )
                .as_deref()
                != Some(root)
            {
                self.focus_background_project(root, cx);
            }
            self.open_project_terminal(window, cx);
        } else if let Some(id) = id.strip_prefix("command:") {
            if let Some((session, _)) = id.split_once(':') {
                if let Ok(session) = Uuid::parse_str(session) {
                    if self.history.sessions.iter().any(|s| s.id == session) {
                        self.switch(session, cx);
                        cx.notify();
                        return;
                    }
                }
            }
            if let Some(run) = command_runner::active_commands(&self.memory.anchor)
                .into_iter()
                .find(|run| run.id.to_string() == id)
            {
                if let Some(id) = run
                    .session
                    .filter(|id| self.history.sessions.iter().any(|s| s.id == *id))
                {
                    self.switch(id, cx);
                } else if let Some(root) = run.root {
                    self.focus_background_project(&root, cx);
                }
            }
        }
        cx.notify();
    }
    pub(super) fn background_tasks_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.entity().downgrade();
        let toggle = owner.clone();
        BackgroundTasksMenu::new(self.background_tasks(cx))
            .theme(self.theme)
            .open(self.show_background_tasks)
            .on_select(move |id, window, cx| {
                let _ = owner.update(cx, |this, cx| this.focus_background_task(id, window, cx));
            })
            .on_open_change(move |open, _, cx| {
                let _ = toggle.update(cx, |this, cx| {
                    this.show_background_tasks = open;
                    cx.notify();
                });
            })
            .render()
    }
}
