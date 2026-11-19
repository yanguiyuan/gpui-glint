//! Project browsing shares the persisted catalog with chat, but never selects a session.
use super::*;
use gpui_glint::components::{Select, SelectEvent, SelectOption, Tooltip};

#[derive(Default)]
pub(super) struct Repositories {
    home: Option<PathBuf>,
    selected: Option<PathBuf>,
    options: Vec<(String, PathBuf)>,
    select: Option<Entity<Select>>,
    task: Option<Task<()>>,
    error: Option<String>,
    head_root: Option<PathBuf>,
    head: Option<String>,
    head_task: Option<Task<()>>,
}

impl Repositories {
    pub(super) fn selected_root(&self) -> Option<PathBuf> {
        self.selected.clone()
    }
}

impl AgentWorkspace {
    pub(super) fn open_browser_project(&mut self, cx: &mut Context<Self>) {
        self.project_files.update(cx, |files, cx| {
            files.set_definition_modifier(self.history.definition_modifier, cx);
        });
        let root = self
            .browser_project
            .clone()
            .filter(|root| self.history.projects.contains(root))
            .or_else(|| self.history.active().project.clone())
            .or_else(|| self.history.projects.first().cloned());
        self.browser_project = root.clone();
        let Some(root) = root else { return };
        if self.browser_repositories.home.as_ref() != Some(&root) {
            self.browser_repositories = Repositories {
                home: Some(root.clone()),
                ..Default::default()
            };
        }
        let selected = self
            .browser_repositories
            .selected
            .clone()
            .unwrap_or_else(|| root.clone());
        self.open_browser_directory(selected, cx);
        self.refresh_browser_repositories(root, cx);
    }

    fn refresh_browser_repositories(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        let work_root = root.clone();
        let work = cx.background_executor().spawn(async move {
            agentspace::Space::load(&work_root).map(|space| {
                space.map(|space| {
                    let mut options = vec![("空间目录".to_owned(), work_root)];
                    options.extend(
                        space
                            .repositories()
                            .map(|(name, path)| (name.to_owned(), path.to_owned())),
                    );
                    options
                })
            })
        });
        self.browser_repositories.task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if this.browser_repositories.home.as_ref() != Some(&root) {
                    return;
                }
                if let Some(task) = this.browser_repositories.task.take() {
                    task.detach();
                }
                this.browser_repositories.error = None;
                let options = match result {
                    Ok(Some(options)) => options,
                    Ok(None) => {
                        this.browser_repositories.select = None;
                        this.browser_repositories.options.clear();
                        cx.notify();
                        return;
                    }
                    Err(error) => {
                        this.browser_repositories.error = Some(error);
                        cx.notify();
                        return;
                    }
                };
                let selected = this
                    .browser_repositories
                    .selected
                    .clone()
                    .filter(|selected| options.iter().any(|(_, path)| path == selected))
                    .unwrap_or_else(|| root.clone());
                let select = cx.new(|cx| {
                    Select::new([], cx)
                        .searchable(true)
                        .width(200.)
                        .theme(this.theme)
                });
                cx.subscribe(&select, |this, _, event: &SelectEvent, cx| {
                    let SelectEvent::Changed(value) = event;
                    if let Some((_, root)) = this
                        .browser_repositories
                        .options
                        .iter()
                        .find(|(_, root)| root.to_string_lossy().as_ref() == value.as_ref())
                    {
                        let root = root.clone();
                        this.browser_repositories.selected = Some(root.clone());
                        this.open_browser_directory(root, cx);
                    }
                })
                .detach();
                select.update(cx, |select, cx| {
                    select.set_options(
                        options
                            .iter()
                            .map(|(name, path)| {
                                SelectOption::new(path.to_string_lossy().into_owned(), name.clone())
                            })
                            .collect::<Vec<_>>(),
                        cx,
                    );
                    select.set_selected(Some(selected.to_string_lossy().into_owned().into()), cx);
                });
                let changed = this
                    .browser_repositories
                    .selected
                    .as_ref()
                    .is_some_and(|old| old != &selected);
                this.browser_repositories.selected = Some(selected.clone());
                this.browser_repositories.options = options;
                this.browser_repositories.select = Some(select);
                if changed && this.page == crate::Page::Projects {
                    this.open_browser_directory(selected, cx);
                }
                cx.notify();
            });
        }));
    }

    fn open_browser_directory(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.project_files
            .update(cx, |files, cx| files.set_root(root.clone(), cx));
        self.refresh_browser_head(root, cx);
        cx.notify();
    }

    pub(super) fn focus_go_server_project(
        &mut self,
        root: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate(crate::Page::Projects, window, cx);
        if self.project_files.read(cx).root() == Some(root.as_path()) {
            return;
        }
        if self
            .browser_repositories
            .options
            .iter()
            .any(|(_, path)| path == &root)
        {
            self.browser_repositories.selected = Some(root.clone());
            if let Some(select) = &self.browser_repositories.select {
                select.update(cx, |select, cx| {
                    select.set_selected(Some(root.to_string_lossy().into_owned().into()), cx);
                });
            }
            self.open_browser_directory(root, cx);
        } else {
            self.choose_project(root, cx);
        }
    }

    fn refresh_browser_head(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        let repositories = &mut self.browser_repositories;
        if repositories.head_root.as_ref() != Some(&root) {
            repositories.head_root = Some(root.clone());
            repositories.head = None;
        }
        repositories.head_task = None;
        let Ok(job) = provider::Job::start(async move { git_project::current_head(&root).await })
        else {
            return;
        };
        repositories.head_task = Some(cx.spawn(async move |weak, cx| {
            let head = job.result().await.ok().flatten();
            let _ = weak.update(cx, |this, cx| {
                this.browser_repositories.head = head;
                cx.notify();
            });
        }));
    }

    pub(super) fn projects_topbar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        div()
            .size_full()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .debug_selector(|| "projects-topbar".into())
            .child(self.project_selector_at(project_ui::WorkspacePicker::BrowserProject, cx))
            .when_some(self.browser_repositories.select.clone(), |bar, select| {
                if select.read(cx).theme_preset() != self.theme {
                    select.update(cx, |select, cx| select.set_theme(self.theme, cx));
                }
                bar.child(div().text_color(rgb(p.text_muted)).child("关联项目"))
                    .child(
                        div()
                            .debug_selector(|| "browser-repository-select".into())
                            .child(select),
                    )
            })
            .when_some(
                self.browser_project
                    .as_ref()
                    .and(self.browser_repositories.head.clone()),
                |bar, head| {
                    bar.child(
                        Tooltip::new(
                            div()
                                .flex()
                                .min_w_0()
                                .items_center()
                                .gap_1()
                                .text_color(rgb(p.text_muted))
                                .debug_selector(|| "project-topbar-git-branch".into())
                                .child(div().flex_shrink_0().child(Icon::GitBranch.render(14.)))
                                .child(
                                    div()
                                        .min_w_0()
                                        .max_w(px(200.))
                                        .truncate()
                                        .child(head.clone()),
                                ),
                            head,
                        )
                        .theme(self.theme)
                        .render(cx)
                        .min_w_0(),
                    )
                },
            )
            .when_some(self.browser_repositories.error.clone(), |bar, error| {
                bar.child(div().truncate().text_color(rgb(p.text_muted)).child(error))
            })
            .child(div().flex_1())
            .child(
                IconButton::new(Icon::Shell, "打开项目终端")
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "project-topbar-terminal".into())
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_project_terminal(window, cx)),
                    ),
            )
            .child(
                div()
                    .id("project-command-trigger")
                    .debug_selector(|| "project-topbar-search".into())
                    .w(px(240.))
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded(px(9.))
                    .border_1()
                    .border_color(rgb(p.border))
                    .bg(rgb(p.panel))
                    .text_color(rgb(p.text_muted))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(p.surface)))
                    .child(Icon::Search.render(15.))
                    .child(div().flex_1().child("命令面板"))
                    .child(if cfg!(target_os = "macos") {
                        "⌘ K"
                    } else {
                        "Ctrl K"
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.open_commands(window, cx))),
            )
            .child(
                IconButton::new(Icon::Refresh, "刷新项目")
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_browser_project(cx);
                        this.project_files.update(cx, |files, cx| {
                            files.refresh(cx);
                            files.refresh_graph(cx);
                        });
                    })),
            )
            .when(!self.host_background_tasks, |bar| {
                bar.child(self.background_tasks_menu(cx))
            })
    }

    pub(super) fn projects_page(&self, cx: &mut Context<Self>) -> gpui::Div {
        let body = div().size_full().min_w_0().min_h_0();
        if self.browser_project.is_some() {
            self.project_files
                .update(cx, |files, cx| files.set_theme(self.theme, cx));
            body.child(self.project_files.clone())
        } else {
            body.flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .child("选择项目以浏览目录和文件")
                .child(
                    Button::new("打开项目目录…")
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| this.open_project(cx))),
                )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn browsing_projects_keeps_chat_session_and_preview(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        host.update(window, |host, cx| {
            host.choose_project(first.clone(), cx);
        });
        host.update_in(window, |host, window, cx| {
            let session = host.history.active;
            let count = host.history.sessions.len();
            let chat_preview = host.file_preview.view.entity_id();
            host.navigate(crate::Page::Projects, window, cx);
            assert_eq!(host.file_preview.view.entity_id(), chat_preview);
            host.choose_project(second.clone(), cx);
            assert_eq!(host.browser_project.as_ref(), Some(&second));
            assert!(host.history.projects.contains(&second));
            assert_eq!(host.history.active, session);
            assert_eq!(host.history.sessions.len(), count);
            assert_eq!(host.history.active().project.as_ref(), Some(&first));
            host.navigate(crate::Page::Chat, window, cx);
            assert_eq!(host.file_preview.view.entity_id(), chat_preview);
            assert!(!host.file_preview.open);
            host.navigate(crate::Page::Projects, window, cx);
            assert_eq!(host.browser_project.as_ref(), Some(&second));
        });
    }
    #[gpui::test]
    fn agentspace_select_browses_repository_without_changing_session(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let space = dir.path().join("space");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&space).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(
            space.join("agentspace.toml"),
            r#"[[repositories]]
name = "App"
path = "../repo"
"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        host.update_in(window, |host, window, cx| {
            host.choose_project(space.clone(), cx);
            host.navigate(crate::Page::Projects, window, cx);
        });
        window.run_until_parked();
        let selected = repo.canonicalize().unwrap();
        let session = host.read_with(window, |host, _| host.history.active);
        host.update(window, |host, cx| {
            let select = host
                .browser_repositories
                .select
                .clone()
                .expect("AgentSpace select");
            assert_eq!(host.browser_repositories.options.len(), 2);
            select.update(cx, |_, cx| {
                cx.emit(SelectEvent::Changed(
                    selected.to_string_lossy().into_owned().into(),
                ))
            });
        });
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert_eq!(host.browser_repositories.selected.as_ref(), Some(&selected));
            assert_eq!(host.browser_project.as_ref(), Some(&space));
            assert_eq!(host.history.active, session);
            assert_eq!(host.history.active().project.as_ref(), Some(&space));
        });
    }
}
