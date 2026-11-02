use super::*;
use gpui_glint::components::{DiffCard, GitGraph, GitGraphEvent, Toast, ToastHost, ToastKind};
use std::time::{Duration, Instant};

pub struct State {
    pub graph: Entity<GitGraph>,
    pub toasts: Entity<ToastHost>,
    pub open: bool,
    pub picker_open: bool,
    project: Option<PathBuf>,
    repository: Option<git_project::Repository>,
    refreshed: Option<Instant>,
    loading: bool,
    switching: bool,
    revision: u64,
    graph_revision: u64,
    target: String,
    diff: Option<CommitDiff>,
    diff_revision: u64,
    _subscription: Subscription,
}
struct CommitDiff {
    commit: String,
    path: String,
    card: Option<Entity<DiffCard>>,
    error: Option<String>,
}
impl State {
    pub fn new(theme: ThemePreset, cx: &mut Context<AgentWorkspace>) -> Self {
        let graph = cx.new(GitGraph::new);
        graph.update(cx, |g, cx| g.set_theme(theme, cx));
        let toasts = cx.new(|_| ToastHost::new().theme(theme));
        let subscription = cx.subscribe(&graph, |this, _, event: &GitGraphEvent, cx| {
            match event {
                GitGraphEvent::CompareBranch(branch) => {
                    this.git_ui.target = branch.clone();
                    this.refresh_git_graph(cx);
                }
                GitGraphEvent::SelectCommit(id) => this.load_git_commit(id.clone(), cx),
                GitGraphEvent::OpenFile { commit, path } => {
                    this.open_git_file(commit.clone(), path.clone(), cx)
                }
                GitGraphEvent::Refresh => {
                    this.refresh_git_repository(true, cx);
                    this.refresh_git_graph(cx);
                }
                GitGraphEvent::CloseRequested => this.git_ui.open = false,
            }
            cx.notify();
        });
        Self {
            graph,
            toasts,
            open: false,
            picker_open: false,
            project: None,
            repository: None,
            refreshed: None,
            loading: false,
            switching: false,
            revision: 0,
            graph_revision: 0,
            target: "master".into(),
            diff: None,
            diff_revision: 0,
            _subscription: subscription,
        }
    }
}
impl AgentWorkspace {
    fn git_error(&mut self, title: &str, error: String, cx: &mut Context<Self>) {
        self.git_ui.toasts.update(cx, |toasts, cx| {
            toasts.push(
                Toast::new(title.to_owned())
                    .description(error)
                    .kind(ToastKind::Error)
                    .duration(Some(Duration::from_secs(10))),
                cx,
            );
        });
        cx.notify();
    }
    pub(crate) fn refresh_git_repository(&mut self, force: bool, cx: &mut Context<Self>) {
        let project = self.history.active().project.clone();
        let changed = project != self.git_ui.project;
        if changed {
            self.git_ui.diff = None;
            self.git_ui.diff_revision += 1;
            self.git_ui.project = project.clone();
            self.git_ui.repository = None;
            self.git_ui.picker_open = false;
            self.git_ui.loading = false;
            self.git_ui.refreshed = None;
            self.git_ui.revision += 1;
            self.git_ui.target = "master".into();
            self.git_ui.graph_revision += 1;
            self.git_ui.graph.update(cx, |g, cx| g.reset(cx));
            if self.git_ui.open {
                self.refresh_git_graph(cx);
            }
        }
        if self.git_ui.loading || self.git_ui.switching {
            return;
        }
        if !force
            && self
                .git_ui
                .refreshed
                .is_some_and(|t| t.elapsed() < Duration::from_secs(10))
        {
            return;
        }
        self.git_ui.refreshed = Some(Instant::now());
        let Some(root) = project else {
            return;
        };
        self.git_ui.revision += 1;
        let revision = self.git_ui.revision;
        let job = match provider::Job::start(async move { git_project::repository(&root).await }) {
            Ok(job) => job,
            Err(_) => return,
        };
        self.git_ui.loading = true;
        cx.spawn(async move |weak, cx| {
            let result = job.result().await;
            let _ = weak.update(cx, |this, cx| {
                if this.git_ui.revision != revision {
                    return;
                }
                this.git_ui.loading = false;
                let repository = result.ok().flatten();
                let changed = repository != this.git_ui.repository;
                this.git_ui.repository = repository;
                if changed && this.git_ui.open {
                    this.refresh_git_graph(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn git_branch_selector(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if self.git_ui.project != self.history.active().project {
            return None;
        }
        let repo = self.git_ui.repository.as_ref()?;
        let p = self.theme.palette();
        let trigger = div()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(px(8.))
            .text_color(rgb(p.text_muted))
            .hover(|s| s.bg(rgb(p.border)))
            .child(
                if self.git_ui.switching {
                    Icon::Spinner
                } else {
                    Icon::GitBranch
                }
                .render(15.),
            )
            .child(div().max_w(px(200.)).truncate().child(repo.current.clone()))
            .child(Icon::ChevronDown.render(12.));
        let mut content = div()
            .id("git-branch-options")
            .max_h(px(300.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        for branch in &repo.branches {
            let branch = branch.clone();
            content = content.child(
                div()
                    .id(SharedString::from(format!("switch-branch:{branch}")))
                    .p_2()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(p.surface_subtle)))
                    .when(branch == repo.current, |d| d.text_color(rgb(p.accent)))
                    .child(branch.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.switch_git_branch(branch.clone(), cx)
                    })),
            );
        }
        if repo.branches.is_empty() {
            content = content.child(div().p_2().child("尚无本地分支提交"));
        }
        content = content.child(
            Button::new("查看 GitGraph")
                .kind(ButtonKind::Plain)
                .theme(self.theme)
                .render()
                .on_click(cx.listener(|this, _, _, cx| this.open_git_graph(cx))),
        );
        let weak = cx.entity().downgrade();
        Some(
            Popover::new("composer-git-branch", trigger, content)
                .open(self.git_ui.picker_open)
                .above_trigger()
                .trigger_height(28.)
                .width(280.)
                .padding(6.)
                .theme(self.theme)
                .on_open_change(move |open, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        if this.git_ui.switching {
                            return;
                        }
                        this.git_ui.picker_open = open;
                        if open {
                            this.project_picker_open = false;
                            this.refresh_git_repository(true, cx);
                        }
                        cx.notify();
                    });
                })
                .into_any_element(),
        )
    }
    fn switch_git_branch(&mut self, branch: String, cx: &mut Context<Self>) {
        if self.git_ui.switching {
            return;
        }
        self.git_ui.picker_open = false;
        let Some(root) = self.history.active().project.clone() else {
            return;
        };
        if self
            .git_ui
            .repository
            .as_ref()
            .is_some_and(|r| r.current == branch)
        {
            cx.notify();
            return;
        }
        let job =
            match provider::Job::start(async move { git_project::switch(&root, &branch).await }) {
                Ok(job) => job,
                Err(error) => {
                    self.git_error("切换分支失败", error, cx);
                    return;
                }
            };
        self.git_ui.switching = true;
        // Invalidate reads started before the mutation.
        self.git_ui.revision += 1;
        self.git_ui.loading = false;
        self.git_ui.graph_revision += 1;
        cx.spawn(async move |weak, cx| {
            let result = job.result().await;
            let _ = weak.update(cx, |this, cx| {
                this.git_ui.switching = false;
                if let Err(error) = result {
                    this.git_error("切换分支失败", error, cx);
                }
                this.refresh_git_repository(true, cx);
                if this.git_ui.open {
                    this.refresh_git_graph(cx);
                }
                this.reset_project_context(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn open_git_graph(&mut self, cx: &mut Context<Self>) {
        self.back_from_git_diff(cx);
        self.page = crate::Page::Chat;
        self.file_preview.open = false;
        self.edits_ui.open = false;
        self.git_ui.picker_open = false;
        self.git_ui.open = true;
        self.right_sidebar_width = self.right_sidebar_width.max(400.);
        self.refresh_git_repository(true, cx);
        self.refresh_git_graph(cx);
        cx.notify();
    }
    fn refresh_git_graph(&mut self, cx: &mut Context<Self>) {
        self.git_ui.graph_revision += 1;
        let revision = self.git_ui.graph_revision;
        let Some(root) = self.history.active().project.clone() else {
            self.git_ui
                .graph
                .update(cx, |g, cx| g.set_error("请先选择项目".into(), cx));
            return;
        };
        let target = self.git_ui.target.clone();
        let job = match provider::Job::start(async move { git_project::graph(&root, target).await })
        {
            Ok(job) => job,
            Err(error) => {
                self.git_error("读取 Git 失败", error, cx);
                return;
            }
        };
        self.git_ui.graph.update(cx, |g, cx| g.set_loading(cx));
        cx.spawn(async move |weak, cx| {
            let result = job.result().await;
            let _ = weak.update(cx, |this, cx| {
                if this.git_ui.graph_revision != revision {
                    return;
                }
                this.git_ui.graph.update(cx, |graph, cx| match result {
                    Ok(data) => graph.set_data(data, cx),
                    Err(error) => graph.set_error(error, cx),
                });
            });
        })
        .detach();
    }
    fn load_git_commit(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(root) = self.history.active().project.clone() else {
            return;
        };
        let revision = self.git_ui.graph_revision;
        let commit = id.clone();
        let job =
            match provider::Job::start(async move { git_project::details(&root, &commit).await }) {
                Ok(job) => job,
                Err(error) => {
                    self.git_error("读取提交失败", error, cx);
                    return;
                }
            };
        cx.spawn(async move |weak, cx| {
            let result = job.result().await;
            let _ = weak.update(cx, |this, cx| {
                if this.git_ui.graph_revision != revision {
                    return;
                }
                this.git_ui.graph.update(cx, |graph, cx| match result {
                    Ok(details) => graph.set_commit_details(details, cx),
                    Err(error) => graph.set_details(&id, error, cx),
                });
            });
        })
        .detach();
    }
    pub(crate) fn set_git_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.git_ui.graph.update(cx, |g, cx| g.set_theme(theme, cx));
        self.git_ui
            .toasts
            .update(cx, |t, cx| t.set_theme(theme, cx));
        if let Some(card) = self.git_ui.diff.as_ref().and_then(|d| d.card.as_ref()) {
            card.update(cx, |card, cx| card.set_theme(theme, cx));
        }
    }
    fn back_from_git_diff(&mut self, cx: &mut Context<Self>) {
        self.git_ui.diff = None;
        self.git_ui.diff_revision += 1;
        cx.notify();
    }
    fn open_git_file(&mut self, commit: String, path: String, cx: &mut Context<Self>) {
        let Some(root) = self.history.active().project.clone() else {
            return;
        };
        self.git_ui.diff_revision += 1;
        let revision = self.git_ui.diff_revision;
        self.git_ui.diff = Some(CommitDiff {
            commit: commit.clone(),
            path: path.clone(),
            card: None,
            error: None,
        });
        let job = match provider::Job::start(async move {
            git_project::file_diff(&root, &commit, &path).await
        }) {
            Ok(job) => job,
            Err(error) => {
                self.git_ui.diff.as_mut().unwrap().error = Some(error);
                cx.notify();
                return;
            }
        };
        cx.spawn(async move |weak, cx| {
            let result = job.result().await;
            let _ = weak.update(cx, |this, cx| {
                if this.git_ui.diff_revision != revision {
                    return;
                }
                let Some(diff) = &mut this.git_ui.diff else {
                    return;
                };
                match result {
                    Ok(lines) => {
                        let language = match std::path::Path::new(&diff.path)
                            .extension()
                            .and_then(|s| s.to_str())
                        {
                            Some("rs") => "rust",
                            Some("go") => "go",
                            Some("js" | "jsx") => "javascript",
                            Some("ts" | "tsx") => "typescript",
                            Some("py") => "python",
                            Some("json") => "json",
                            Some("html") => "html",
                            Some("css") => "css",
                            _ => "text",
                        };
                        diff.card = Some(cx.new(|cx| {
                            DiffCard::from_lines(cx, language, lines)
                                .path(diff.path.clone())
                                .header_visible(false)
                                .theme(this.theme)
                        }));
                    }
                    Err(error) => diff.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn git_sidebar_content(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(diff) = &self.git_ui.diff else {
            return div()
                .size_full()
                .child(self.git_ui.graph.clone())
                .into_any_element();
        };
        let p = self.theme.palette();
        div()
            .id("git-commit-diff")
            .debug_selector(|| "git-commit-diff".into())
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(rgb(p.surface))
            .text_color(rgb(p.text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_3()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .child(
                        IconButton::new(Icon::ArrowLeft, "返回提交详情")
                            .theme(self.theme)
                            .render()
                            .id("back-to-git-graph")
                            .debug_selector(|| "back-to-git-graph".into())
                            .on_click(cx.listener(|this, _, _, cx| this.back_from_git_diff(cx))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().truncate().child(diff.path.clone()))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.text_muted))
                                    .child(format!(
                                        "{} · 提交改动",
                                        diff.commit.chars().take(12).collect::<String>()
                                    )),
                            ),
                    )
                    .child(
                        IconButton::new(Icon::Close, "关闭侧栏")
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.git_ui.open = false;
                                this.back_from_git_diff(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .id("git-file-diff-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when_some(diff.card.clone(), |body, card| body.child(card))
                    .when_some(diff.error.clone(), |body, error| {
                        body.child(div().p_3().child(error))
                    })
                    .when(diff.card.is_none() && diff.error.is_none(), |body| {
                        body.child(div().p_3().child("正在读取提交文件 Diff…"))
                    }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn back_from_commit_diff_restores_existing_graph_and_details(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        let graph_id = host.update(window, |host, cx| {
            host.git_ui.open = true;
            host.git_ui.graph.update(cx, |graph, cx| {
                graph.set_data(
                    gpui_glint::components::GitGraphData {
                        commits: vec![gpui_glint::components::GitCommit {
                            id: "abcdef1234".into(),
                            summary: "Selected commit".into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                    cx,
                );
            });
            cx.notify();
            host.git_ui.graph.entity_id()
        });
        window.run_until_parked();
        let commit = window.debug_bounds("git-commit:abcdef1234").unwrap();
        window.simulate_click(commit.center(), Default::default());
        window.run_until_parked();
        host.update(window, |host, cx| {
            host.git_ui.graph.update(cx, |g, cx| {
                g.set_commit_details(
                    gpui_glint::components::GitCommitDetails {
                        id: "abcdef1234".into(),
                        message: "Keep this selected commit".into(),
                        ..Default::default()
                    },
                    cx,
                )
            });
            host.git_ui.diff = Some(CommitDiff {
                commit: "abcdef1234".into(),
                path: "src/main.rs".into(),
                card: None,
                error: Some("binary preview unavailable".into()),
            });
            cx.notify();
        });
        window.run_until_parked();
        assert!(window.debug_bounds("git-commit-diff").is_some());
        let back = window.debug_bounds("back-to-git-graph").unwrap();
        window.simulate_click(back.center(), Default::default());
        window.run_until_parked();
        assert!(window.debug_bounds("git-commit-diff").is_none());
        assert!(window.debug_bounds("git-graph").is_some());
        assert!(window.debug_bounds("copy-commit-hash").is_some());
        host.read_with(window, |h, _| {
            assert_eq!(h.git_ui.graph.entity_id(), graph_id);
            assert!(h.git_ui.diff.is_none());
            assert!(h.git_ui.diff_revision > 0);
        });
    }
    #[gpui::test]
    fn non_git_picker_is_hidden_errors_use_toast_and_command_opens_sidebar(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        host.update(window, |host, cx| {
            assert!(host.git_branch_selector(cx).is_none());
            host.git_error(
                "切换分支失败",
                "local changes would be overwritten".into(),
                cx,
            );
            let toast = host.git_ui.toasts.read(cx).toasts().next().unwrap();
            assert_eq!(toast.kind, ToastKind::Error);
            assert_eq!(
                toast.description.as_deref(),
                Some("local changes would be overwritten")
            );
            host.commands.pending = Some("gitgraph".into());
            cx.notify();
        });
        window.run_until_parked();
        assert!(window.debug_bounds("git-graph").is_some());
        host.update(window, |host, cx| {
            host.git_ui
                .graph
                .update(cx, |_, cx| cx.emit(GitGraphEvent::CloseRequested));
        });
        window.run_until_parked();
        assert!(window.debug_bounds("git-graph").is_none());
    }
}
