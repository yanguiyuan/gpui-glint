use super::*;
use crate::go_definition::ProcessState;
use gpui_glint::components::{BackgroundTask, BackgroundTaskState as State, BackgroundTasksMenu};

const GO_TASK_PREFIX: &str = "language-server:go:";

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    #[ignore = "requires Go and gopls on PATH"]
    fn go_background_task_reopens_the_project_and_keeps_the_process(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("go-project");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("go.mod"), "module example.com/fixture\ngo 1.21\n").unwrap();
        let file = root.join("main.go");
        std::fs::write(&file, "package fixture\n").unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (agent, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        let id = format!("{GO_TASK_PREFIX}{}", root.to_string_lossy());
        agent.update_in(window, |agent, window, cx| {
            agent.navigate(crate::Page::Projects, window, cx);
            agent.choose_project(root.clone(), cx);
            agent.project_files.update(cx, |files, cx| {
                files.open_at(file.to_string_lossy().into_owned(), None, cx);
            });
            agent.navigate(crate::Page::Chat, window, cx);
        });
        let task = |window: &mut gpui::VisualTestContext| {
            agent.read_with(window, |agent, cx| {
                agent
                    .background_tasks(cx)
                    .into_iter()
                    .find(|task| task.id == id)
                    .unwrap()
            })
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let detail = loop {
            window.run_until_parked();
            let current = task(window);
            assert_eq!(current.state, State::Running, "{}", current.detail);
            if current.detail.starts_with("PID ") {
                break current.detail;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "gopls did not initialize"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };

        let trigger = window.debug_bounds("background-tasks-trigger").unwrap();
        window.simulate_click(trigger.center(), Default::default());
        let selector = Box::leak(format!("background-task-{id}").into_boxed_str());
        let row = window.debug_bounds(selector).unwrap();
        window.simulate_click(row.center(), Default::default());
        agent.read_with(window, |agent, cx| {
            assert_eq!(agent.page(), crate::Page::Projects);
            assert_eq!(agent.project_files.read(cx).root(), Some(root.as_path()));
        });
        assert_eq!(task(window).detail, detail);
        assert!(window.debug_bounds("background-tasks-trigger").is_some());
    }

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
        agent.update(window, |agent, cx| {
            agent.run_mut(id).pending.take();
            agent
                .history
                .active_mut()
                .push(Role::Agent, "任务结果".into());
            cx.notify();
        });
        agent.read_with(window, |agent, cx| {
            assert_eq!(
                agent
                    .background_tasks(cx)
                    .iter()
                    .find(|t| t.id == format!("session:{id}"))
                    .unwrap()
                    .state,
                State::Completed
            );
        });
        let trigger = window.debug_bounds("background-tasks-trigger").unwrap();
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        let row = window.debug_bounds(selector).unwrap();
        window.simulate_click(row.center(), gpui::Modifiers::default());
        agent.read_with(window, |agent, cx| {
            assert_eq!(
                agent.history.active().messages.last().unwrap().text,
                "任务结果"
            );
            assert!(
                agent
                    .background_tasks(cx)
                    .iter()
                    .all(|t| t.id != format!("session:{id}"))
            );
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
                        .map(|task| (task.id, task.state, task.detail))
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
                state: gpui_glint::components::BackgroundTaskState::Running,
            })
            .collect();
        tasks.extend(
            self.running_project_action_tasks(cx)
                .into_iter()
                .map(|(root, label)| BackgroundTask {
                    id: format!("action:{}", root.to_string_lossy()),
                    label,
                    detail: gpui_glint::paths::display_path(&root),
                    group: "项目操作".into(),
                    icon: Icon::Play,
                    state: gpui_glint::components::BackgroundTaskState::Running,
                }),
        );
        tasks.extend(
            self.running_terminal_tasks(cx)
                .into_iter()
                .map(|(root, _status)| BackgroundTask {
                    id: format!("terminal:{}", root.to_string_lossy()),
                    label: ssh_project::label(&root),
                    detail: gpui_glint::paths::display_path(&root),
                    group: "项目终端".into(),
                    icon: Icon::Code,
                    state: gpui_glint::components::BackgroundTaskState::Running,
                }),
        );
        tasks.extend(
            command_runner::background_commands(&self.memory.anchor)
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
                    state: run.state,
                }),
        );
        if let Some((root, process)) = self.project_files.read(cx).go_process() {
            let (state, status) = match process {
                ProcessState::Starting => (State::Running, "启动中".into()),
                ProcessState::Running(pid) => (State::Running, format!("PID {pid}")),
                ProcessState::Failed(error) => (State::Failed, error),
                ProcessState::Finished => (State::Finished, "进程已退出".into()),
            };
            tasks.push(BackgroundTask {
                id: format!("{GO_TASK_PREFIX}{}", root.to_string_lossy()),
                label: "Go 语言服务器 (gopls)".into(),
                detail: format!("{status} · {}", gpui_glint::paths::display_path(root)),
                group: "语言服务器".into(),
                icon: Icon::Code,
                state,
            });
        }
        self.background_task_inbox
            .borrow_mut()
            .update(tasks, |task| self.background_task_outcome(&task.id, cx))
    }
    fn background_task_outcome(&self, id: &str, cx: &App) -> State {
        if let Some(id) = id
            .strip_prefix("session:")
            .and_then(|s| Uuid::parse_str(s).ok())
        {
            return self
                .history
                .sessions
                .iter()
                .find(|s| s.id == id)
                .and_then(|s| s.messages.last())
                .map(|message| {
                    if message.role == Role::Agent {
                        State::Completed
                    } else if message.role == Role::Notice && message.text.starts_with("请求失败")
                    {
                        State::Failed
                    } else if message.role == Role::Notice && message.text.contains("已停止") {
                        State::Cancelled
                    } else {
                        State::Finished
                    }
                })
                .unwrap_or(State::Finished);
        }
        if let Some(root) = id.strip_prefix("action:") {
            return self.project_action_task_outcome(std::path::Path::new(root), cx);
        }
        if let Some(root) = id.strip_prefix("terminal:") {
            return self.terminal_task_outcome(std::path::Path::new(root), cx);
        }
        if let Some(id) = id
            .strip_prefix("command:")
            .and_then(|s| s.rsplit(':').next())
            .and_then(|s| Uuid::parse_str(s).ok())
        {
            return command_runner::background_command_outcome(&self.memory.anchor, id);
        }
        State::Finished
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
    pub fn focus_background_task(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.background_tasks(cx);
        let go_root = id.strip_prefix(GO_TASK_PREFIX);
        if self.host_background_tasks && go_root.is_none() {
            self.navigate(crate::Page::Chat, window, cx);
        }
        let mut opened = false;
        self.show_background_tasks = false;
        if let Some(root) = go_root {
            self.focus_go_server_project(PathBuf::from(root), window, cx);
            opened = true;
        } else if let Some(id) = id.strip_prefix("session:") {
            if let Ok(id) = Uuid::parse_str(id) {
                if self.history.sessions.iter().any(|s| s.id == id) {
                    self.switch(id, cx);
                    opened = true;
                }
            }
        } else if let Some(root) = id.strip_prefix("action:") {
            self.focus_background_project(std::path::Path::new(root), cx);
            if let Some((_, terminal)) = self.project_action_terminal(std::path::Path::new(root)) {
                self.select_terminal_tab(root.into(), terminal.entity_id());
            }
            opened = true;
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
            self.restore_completed_terminal(root);
            self.open_project_terminal(window, cx);
            opened = true;
        } else if let Some(id) = id.strip_prefix("command:") {
            if let Some((session, _)) = id.split_once(':') {
                if let Ok(session) = Uuid::parse_str(session) {
                    if self.history.sessions.iter().any(|s| s.id == session) {
                        self.switch(session, cx);
                        self.background_task_inbox
                            .borrow_mut()
                            .acknowledge(&format!("command:{id}"));
                        cx.notify();
                        return true;
                    }
                }
            }
            if let Some(run) = command_runner::background_commands(&self.memory.anchor)
                .into_iter()
                .find(|run| run.id.to_string() == id)
            {
                if let Some(id) = run
                    .session
                    .filter(|id| self.history.sessions.iter().any(|s| s.id == *id))
                {
                    self.switch(id, cx);
                    opened = true;
                } else if let Some(root) = run.root {
                    self.focus_background_project(&root, cx);
                    opened = true;
                }
            }
        }
        if opened {
            self.background_task_inbox.borrow_mut().acknowledge(id);
        }
        cx.notify();
        opened
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
