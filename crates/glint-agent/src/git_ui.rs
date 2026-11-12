use super::*;
use gpui::AnimationExt as _;
use gpui_glint::components::{
    Dialog, DiffCard, GitGraph, GitGraphEvent, InputField, MenuItem, Toast, ToastHost, ToastKind,
    Tooltip,
};
use std::time::{Duration, Instant};

pub struct State {
    pub graph: Entity<GitGraph>,
    pub toasts: Entity<ToastHost>,
    pub open: bool,
    pub picker_open: bool,
    worktree_actions: Option<PathBuf>,
    pub dialog: Option<WorktreeDialog>,
    branch_name: Entity<TextEntry>,
    dialog_focus: [gpui::FocusHandle; 2],
    _branch_observer: Subscription,
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
pub enum WorktreeDialog {
    Create {
        root: PathBuf,
        repo: git_project::Repository,
    },
    Remove {
        root: PathBuf,
        tree: git_project::Worktree,
    },
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
        let branch_name = cx.new(|cx| TextEntry::new(cx, "例如 feat/abc-ef", "").theme(theme));
        let branch_observer = cx.observe(&branch_name, |_, _, cx| cx.notify());
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
            worktree_actions: None,
            dialog: None,
            branch_name,
            dialog_focus: [cx.focus_handle(), cx.focus_handle()],
            _branch_observer: branch_observer,
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
fn compact_worktree_path(path: &str) -> String {
    let chars: Vec<_> = path.chars().collect();
    if chars.len() <= 46 {
        return path.to_owned();
    }
    format!(
        "{}…{}",
        chars[..14].iter().collect::<String>(),
        chars[chars.len() - 31..].iter().collect::<String>()
    )
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
            self.git_ui.worktree_actions = None;
            self.git_ui.dialog = None;
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
                if let (Some(repo), Some(path)) = (&repository, &this.git_ui.project) {
                    let metadata = project_grouping::metadata(repo);
                    if this.history.project_metadata.get(path) != Some(&metadata) {
                        this.history.project_metadata.insert(path.clone(), metadata);
                        this.save(cx);
                    }
                }
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
            .child(if self.git_ui.switching {
                Icon::Spinner
                    .render(15.)
                    .with_animation(
                        "git-branch-switch-spinner",
                        gpui::Animation::new(Duration::from_millis(900)).repeat(),
                        |spinner, delta| {
                            spinner.with_transformation(gpui::Transformation::rotate(
                                gpui::radians(std::f32::consts::TAU * delta),
                            ))
                        },
                    )
                    .into_any_element()
            } else {
                Icon::GitBranch.render(15.).into_any_element()
            })
            .child(div().max_w(px(200.)).truncate().child(repo.current.clone()))
            .child(Icon::ChevronDown.render(12.));
        let project_name = repo
            .worktrees
            .first()
            .map(|tree| ssh_project::label(&tree.path))
            .unwrap_or_else(|| ssh_project::label(&repo.worktree));
        let header = div()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .pt_1()
            .pb_2()
            .text_color(rgb(p.text_muted))
            .child(Icon::Folder.render(14.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(project_name),
            );
        let create = MenuItem::new("create-worktree-branch", "创建 worktree 分支")
            .icon(Icon::Plus)
            .theme(self.theme)
            .render()
            .debug_selector(|| "create-worktree-branch".into())
            .on_click(cx.listener(|this, _, window, cx| {
                if this.git_ui.switching {
                    return;
                }
                if let (Some(root), Some(repo)) =
                    (this.git_ui.project.clone(), this.git_ui.repository.clone())
                {
                    this.git_ui
                        .branch_name
                        .update(cx, |input, cx| input.set_value("", cx));
                    this.git_ui.dialog = Some(WorktreeDialog::Create { root, repo });
                    this.git_ui.picker_open = false;
                    this.git_ui.worktree_actions = None;
                    window.focus(&this.git_ui.branch_name.focus_handle(cx), cx);
                    cx.notify();
                }
            }));
        let mut list = div()
            .id("git-branch-options")
            .max_h(px(320.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        let trees: Vec<_> = repo.worktrees.iter().filter(|tree| !tree.bare).collect();
        list = list.child(
            div()
                .px_2()
                .pt_3()
                .pb_1()
                .text_size(px(10.))
                .text_color(rgb(p.text_muted))
                .child(format!("工作目录 · {}", trees.len())),
        );
        for tree in trees {
            let path = tree.path.clone();
            let current = path == repo.worktree;
            let main = repo.worktrees.first() == Some(tree);
            let removable = !current && !main && tree.locked.is_none();
            let actions_open = self.git_ui.worktree_actions.as_ref() == Some(&path);
            let label = tree
                .branch
                .as_deref()
                .and_then(|branch| branch.strip_prefix("refs/heads/"))
                .unwrap_or("分离 HEAD")
                .to_owned();
            let full_path = path.to_string_lossy().into_owned();
            let mut title = div().flex().items_center().gap_2().min_w_0().child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(label),
            );
            let badge = if current {
                Some("当前")
            } else if tree.locked.is_some() {
                Some("已锁定")
            } else if tree.prunable.is_some() {
                Some("目录不可用")
            } else if main {
                Some("主目录")
            } else {
                None
            };
            if let Some(badge) = badge {
                title = title.child(
                    div()
                        .flex_shrink_0()
                        .px_1()
                        .rounded(px(4.))
                        .text_size(px(10.))
                        .text_color(rgb(if current { p.accent } else { p.text_muted }))
                        .child(badge),
                );
            }
            let details = div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(title)
                .child(
                    Tooltip::new(
                        div()
                            .w_full()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(compact_worktree_path(&full_path)),
                        full_path,
                    )
                    .theme(self.theme)
                    .render(cx)
                    .min_w_0()
                    .w_full(),
                );
            let mut line = div()
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(
                    div()
                        .w(px(16.))
                        .flex_shrink_0()
                        .text_color(rgb(if current { p.accent } else { p.text_muted }))
                        .child(
                            if current {
                                Icon::CheckCircle
                            } else {
                                Icon::GitBranch
                            }
                            .render(14.),
                        ),
                )
                .child(details);
            if removable {
                let action_path = path.clone();
                let delete_tree = tree.clone();
                let trigger = IconButton::new(Icon::More, "工作目录操作")
                    .plain()
                    .theme(self.theme)
                    .render()
                    .size(px(24.))
                    .debug_selector(|| "worktree-more".into())
                    .when(actions_open, |button| button.bg(rgb(p.border)));
                let menu = MenuItem::new("delete-worktree", "删除 worktree…")
                    .icon(Icon::Trash)
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "delete-worktree".into())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        if this.git_ui.switching {
                            return;
                        }
                        if let Some(root) = this.history.active().project.clone() {
                            this.git_ui.dialog = Some(WorktreeDialog::Remove {
                                root,
                                tree: delete_tree.clone(),
                            });
                            this.git_ui.picker_open = false;
                            this.git_ui.worktree_actions = None;
                            window.focus(&this.git_ui.dialog_focus[0], cx);
                            cx.notify();
                        }
                    }));
                let weak = cx.entity().downgrade();
                line = line.child(
                    Popover::new(
                        SharedString::from(format!("worktree-actions:{}", path.display())),
                        trigger,
                        menu,
                    )
                    .open(actions_open)
                    .width(172.)
                    .trigger_height(24.)
                    .gap(4.)
                    .padding(4.)
                    .theme(self.theme)
                    .on_open_change(move |open, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            if this.git_ui.switching {
                                return;
                            }
                            if open {
                                this.git_ui.worktree_actions = Some(action_path.clone());
                            } else if this.git_ui.worktree_actions.as_ref() == Some(&action_path) {
                                this.git_ui.worktree_actions = None;
                            }
                            cx.notify();
                        });
                    }),
                );
            } else {
                line = line.child(div().w(px(24.)).flex_shrink_0());
            }
            let row = div()
                .id(SharedString::from(format!(
                    "open-worktree:{}",
                    path.display()
                )))
                .debug_selector(move || {
                    if current {
                        "current-worktree-row".into()
                    } else {
                        "worktree-row".into()
                    }
                })
                .px_2()
                .py_2()
                .rounded(px(6.))
                .min_w_0()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .gap_1()
                .when(current, |row| row.bg(rgb(p.accent_tint)))
                .when(!current, |row| {
                    row.cursor_pointer()
                        .hover(|style| style.bg(rgb(p.surface_subtle)))
                })
                .child(line)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !current {
                        this.open_git_worktree(path.clone(), cx);
                    }
                }));
            list = list.child(row);
        }
        // Occupied branches already appear as worktree rows; avoid duplicate targets.
        let branches: Vec<_> = repo
            .branches
            .iter()
            .filter(|branch| {
                let reference = format!("refs/heads/{branch}");
                !repo
                    .worktrees
                    .iter()
                    .any(|tree| !tree.bare && tree.branch.as_deref() == Some(&reference))
            })
            .collect();
        if !branches.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .pt_3()
                    .pb_1()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("其他本地分支"),
            );
            for branch in branches {
                let branch = branch.clone();
                list =
                    list.child(
                        MenuItem::new(
                            SharedString::from(format!("switch-branch:{branch}")),
                            branch.clone(),
                        )
                        .icon(Icon::GitBranch)
                        .theme(self.theme)
                        .render()
                        .min_w_0()
                        .overflow_hidden()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.switch_git_branch(branch.clone(), cx)
                        })),
                    );
            }
        }
        let content = div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(header)
            .child(create)
            .child(list)
            .child(
                div()
                    .mt_2()
                    .pt_1()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .child(
                        MenuItem::new("open-git-graph", "查看 GitGraph")
                            .icon(Icon::Workflow)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "open-git-graph".into())
                            .on_click(cx.listener(|this, _, _, cx| this.open_git_graph(cx))),
                    ),
            );
        let weak = cx.entity().downgrade();
        Some(
            Popover::new("composer-git-branch", trigger, content)
                .open(self.git_ui.picker_open)
                .above_trigger()
                .trigger_height(28.)
                .width(340.)
                .padding(6.)
                .theme(self.theme)
                .on_open_change(move |open, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        if this.git_ui.switching {
                            return;
                        }
                        this.git_ui.picker_open = open;
                        this.git_ui.worktree_actions = None;
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
        let origin_root = root.clone();
        let reference = format!("refs/heads/{branch}");
        let expected_worktree = self.git_ui.repository.as_ref().and_then(|repo| {
            repo.worktrees
                .iter()
                .find(|tree| {
                    tree.branch.as_deref() == Some(reference.as_str()) && tree.path != repo.worktree
                })
                .map(|tree| tree.path.clone())
        });
        let job = match provider::Job::start(async move {
            git_project::select_branch(&root, &branch, expected_worktree.as_deref()).await
        }) {
            Ok(job) => job,
            Err(error) => {
                self.git_error("切换分支失败", error, cx);
                return;
            }
        };
        self.await_git_selection(job, origin_root, "切换分支失败", cx);
    }

    fn open_git_worktree(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.git_ui.switching {
            return;
        }
        let Some(root) = self.history.active().project.clone() else {
            return;
        };
        let origin_root = root.clone();
        match provider::Job::start(async move { git_project::select_worktree(&root, &path).await })
        {
            Ok(job) => self.await_git_selection(job, origin_root, "打开 worktree 失败", cx),
            Err(error) => self.git_error("打开 worktree 失败", error, cx),
        }
    }

    pub(crate) fn worktree_dialog(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(dialog) = &self.git_ui.dialog else {
            return div().into_any_element();
        };
        let mut body = div().flex().flex_col().gap_3();
        let removing = matches!(dialog, WorktreeDialog::Remove { .. });
        match dialog {
            WorktreeDialog::Create { repo, .. } => {
                let branch = self.git_ui.branch_name.read(cx).value();
                let preview = git_project::worktree_path(
                    repo,
                    if branch.is_empty() {
                        "<分支名>"
                    } else {
                        branch
                    },
                )
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_else(|error| error);
                body = body
                    .child(div().text_size(px(18.)).child("创建 worktree 分支"))
                    .child(
                        InputField::new("分支名")
                            .label("分支名")
                            .control(self.git_ui.branch_name.clone())
                            .theme(self.theme)
                            .render(),
                    )
                    .child(format!(
                        "基于当前 HEAD 创建，完成后打开新工作目录。\n{preview}"
                    ));
            }
            WorktreeDialog::Remove { tree, .. } => {
                body = body.child(div().text_size(px(18.)).child("确认删除 worktree？"))
                    .child(tree.path.to_string_lossy().into_owned())
                    .child("将删除此工作目录并移除 worktree 登记，保留分支和历史会话。有未提交改动、未跟踪文件或已锁定时无法删除。");
            }
        }
        body = body.child(
            div()
                .flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("取消")
                        .kind(ButtonKind::Plain)
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "cancel-worktree-dialog".into())
                        .track_focus(&self.git_ui.dialog_focus[0])
                        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                            if event.keystroke.key == "enter" {
                                this.git_ui.dialog = None;
                                window.focus(&this.composer.focus_handle(cx), cx);
                                cx.stop_propagation();
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.git_ui.dialog = None;
                            window.focus(&this.composer.focus_handle(cx), cx);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(if removing {
                        "确认删除"
                    } else {
                        "创建并打开"
                    })
                    .kind(if removing {
                        ButtonKind::Danger
                    } else {
                        ButtonKind::Primary
                    })
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "confirm-worktree-dialog".into())
                    .track_focus(&self.git_ui.dialog_focus[1])
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        if event.keystroke.key == "enter" {
                            window.focus(&this.composer.focus_handle(cx), cx);
                            this.confirm_worktree_dialog(cx);
                            cx.stop_propagation();
                        }
                    }))
                    .on_click(cx.listener(|this, _, window, cx| {
                        window.focus(&this.composer.focus_handle(cx), cx);
                        this.confirm_worktree_dialog(cx);
                    })),
                ),
        );
        let weak = cx.entity().downgrade();
        Dialog::new("git-worktree-dialog", body)
            .width(560.)
            .theme(self.theme)
            .focus_handles(if removing {
                self.git_ui.dialog_focus.to_vec()
            } else {
                vec![
                    self.git_ui.branch_name.focus_handle(cx),
                    self.git_ui.dialog_focus[0].clone(),
                    self.git_ui.dialog_focus[1].clone(),
                ]
            })
            .on_close(move |_, window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.git_ui.dialog = None;
                    window.focus(&this.composer.focus_handle(cx), cx);
                    cx.notify();
                });
            })
            .render()
    }

    fn confirm_worktree_dialog(&mut self, cx: &mut Context<Self>) {
        if self.git_ui.switching {
            return;
        }
        let Some(dialog) = self.git_ui.dialog.take() else {
            return;
        };
        let root = match &dialog {
            WorktreeDialog::Create { root, .. } | WorktreeDialog::Remove { root, .. } => {
                root.clone()
            }
        };
        if self.history.active().project.as_ref() != Some(&root) {
            cx.notify();
            return;
        }
        match dialog {
            WorktreeDialog::Create { .. } => {
                let branch = self.git_ui.branch_name.read(cx).value().to_owned();
                let origin = root.clone();
                match provider::Job::start(async move {
                    git_project::create_worktree(&root, &branch).await
                }) {
                    Ok(job) => self.await_git_selection(job, origin, "创建 worktree 失败", cx),
                    Err(error) => self.git_error("创建 worktree 失败", error, cx),
                }
            }
            WorktreeDialog::Remove { tree, .. } => {
                if self.runs.iter().any(|(id, run)| {
                    run.busy()
                        && self.history.sessions.iter().any(|session| {
                            session.id == *id && session.project.as_ref() == Some(&tree.path)
                        })
                }) {
                    self.git_error(
                        "删除 worktree 失败",
                        "该工作目录有任务正在运行，请先停止任务".into(),
                        cx,
                    );
                    return;
                }
                let path = tree.path.clone();
                let job = match provider::Job::start(async move {
                    git_project::remove_worktree(&root, &tree).await
                }) {
                    Ok(job) => job,
                    Err(error) => {
                        self.git_error("删除 worktree 失败", error, cx);
                        return;
                    }
                };
                self.git_ui.switching = true;
                self.git_ui.revision += 1;
                self.git_ui.loading = false;
                cx.spawn(async move |weak, cx| {
                    let result = job.result().await;
                    let _ = weak.update(cx, |this, cx| {
                        this.git_ui.switching = false;
                        match result {
                            Ok(()) => {
                                this.history.projects.retain(|project| project != &path);
                                this.save(cx);
                            }
                            Err(error) => this.git_error("删除 worktree 失败", error, cx),
                        }
                        this.refresh_git_repository(true, cx);
                        cx.notify();
                    });
                })
                .detach();
            }
        }
        cx.notify();
    }

    fn await_git_selection(
        &mut self,
        job: provider::Job<git_project::BranchSelection>,
        origin_root: PathBuf,
        error_title: &'static str,
        cx: &mut Context<Self>,
    ) {
        let origin = self.history.active().id;
        self.git_ui.picker_open = false;
        self.git_ui.switching = true;
        // Invalidate reads started before the mutation.
        self.git_ui.revision += 1;
        self.git_ui.loading = false;
        self.git_ui.graph_revision += 1;
        cx.spawn(async move |weak, cx| {
            let result = job.result().await;
            let _ = weak.update(cx, |this, cx| {
                this.git_ui.switching = false;
                // A late result must never move a different conversation.
                if this.history.active().id != origin
                    || this.history.active().project.as_ref() != Some(&origin_root)
                {
                    this.refresh_git_repository(true, cx);
                    cx.notify();
                    return;
                }
                match result {
                    Ok(git_project::BranchSelection::Worktree(path)) => {
                        this.open_worktree_session(path, cx);
                        return;
                    }
                    Ok(git_project::BranchSelection::Switched) => {}
                    Err(error) => this.git_error(error_title, error, cx),
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

    fn open_worktree_session(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.refresh_project_metadata(vec![path.clone()], cx);
        // Preserve the source session even when it has only a draft/attachments.
        let partner = self.history.active().partner_id;
        let existing = self
            .history
            .sessions
            .iter()
            .find(|session| {
                session.project.as_ref() == Some(&path) && session.partner_id == partner
            })
            .map(|session| session.id);
        let id = existing.unwrap_or_else(|| {
            let mut session = model::Session::new();
            session.project = Some(path.clone());
            session.partner_id = partner;
            let id = session.id;
            self.history.sessions.insert(0, session);
            id
        });
        if !self.history.projects.contains(&path) {
            self.history.projects.push(path);
        }
        self.switch(id, cx);
        self.git_ui.open = false;
        self.edits_ui.open = false;
        self.refresh_git_repository(true, cx);
    }
    pub(crate) fn open_git_graph(&mut self, cx: &mut Context<Self>) {
        self.terminal_ui.open = false;
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
        self.git_ui
            .branch_name
            .update(cx, |input, cx| input.set_theme(theme, cx));
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
                            .child(
                                div()
                                    .truncate()
                                    .child(gpui_glint::paths::display_path(&diff.path)),
                            )
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
    fn deleting_worktree_requires_confirmation_and_cancel_preserves_directory(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let main = dir.path().join("main");
        let linked = dir
            .path()
            .join("worktrees/admin/a-long-worktree-directory/feat/界面优化/admin");
        std::fs::create_dir(&main).unwrap();
        std::fs::create_dir_all(&linked).unwrap();
        let tree = git_project::Worktree {
            path: linked.clone(),
            branch: Some("refs/heads/feature".into()),
            ..Default::default()
        };
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        window.simulate_resize(size(px(1440.), px(1000.)));
        host.update(window, |host, cx| {
            host.history.active_mut().project = Some(main.clone());
            host.git_ui.project = Some(main.clone());
            host.git_ui.repository = Some(git_project::Repository {
                current: "main".into(),
                branches: vec!["main".into()],
                worktree: main.clone(),
                common_dir: main.join(".git"),
                worktrees: vec![
                    git_project::Worktree {
                        path: main.clone(),
                        ..Default::default()
                    },
                    tree,
                ],
            });
            host.git_ui.refreshed = Some(Instant::now());
            host.git_ui.picker_open = true;
            cx.notify();
        });
        window.run_until_parked();
        assert!(window.debug_bounds("delete-worktree").is_none());
        let row = window.debug_bounds("worktree-row").unwrap();
        let footer = window.debug_bounds("open-git-graph").unwrap();
        assert!(
            row.size.height <= px(64.),
            "long paths must stay on one line"
        );
        let current = window.debug_bounds("current-worktree-row").unwrap();
        window.simulate_click(current.center(), Default::default());
        window.run_until_parked();
        let more = window.debug_bounds("worktree-more").unwrap();
        window.simulate_click(more.center(), Default::default());
        window.run_until_parked();
        assert_eq!(window.debug_bounds("worktree-row").unwrap(), row);
        assert_eq!(window.debug_bounds("open-git-graph").unwrap(), footer);
        assert!(window.debug_bounds("delete-worktree").is_some());
        // Clicking elsewhere in the parent closes only the child popup.
        window.simulate_click(current.center(), Default::default());
        window.run_until_parked();
        assert!(window.debug_bounds("delete-worktree").is_none());
        host.read_with(window, |host, _| assert!(host.git_ui.picker_open));
        window.simulate_click(more.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert_eq!(host.git_ui.worktree_actions.as_ref(), Some(&linked));
            assert!(host.git_ui.picker_open);
            assert!(!host.git_ui.switching);
        });
        let delete = window.debug_bounds("delete-worktree").unwrap();
        window.simulate_click(delete.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert!(matches!(
                host.git_ui.dialog,
                Some(WorktreeDialog::Remove { .. })
            ));
            assert!(!host.git_ui.switching);
            assert_eq!(host.history.active().project.as_ref(), Some(&main));
        });
        assert!(linked.exists());
        assert!(window.debug_bounds("confirm-worktree-dialog").is_some());
        // Enter initially activates Cancel; the destructive action needs explicit selection.
        window.simulate_keystrokes("enter");
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert!(host.git_ui.dialog.is_none());
            assert!(!host.git_ui.switching);
        });
        assert!(linked.exists());
        assert!(window.debug_bounds("confirm-worktree-dialog").is_none());
    }

    #[gpui::test]
    fn worktree_navigation_keeps_source_drafts_and_reuses_target_sessions(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        host.update(window, |host, cx| {
            host.history.active_mut().project = Some(source.clone());
            let source_id = host.history.active;
            host.composer.update(cx, |composer, cx| {
                composer.set_draft(
                    "source draft",
                    vec![gpui_glint::components::PastedImage::new(
                        gpui::ImageFormat::Png,
                        vec![1, 2, 3],
                    )],
                    cx,
                )
            });
            host.open_worktree_session(target.clone(), cx);
            let target_id = host.history.active;
            assert_ne!(source_id, target_id);
            assert_eq!(host.history.active().project.as_ref(), Some(&target));
            let old = host
                .history
                .sessions
                .iter()
                .find(|session| session.id == source_id)
                .unwrap();
            assert_eq!(old.project.as_ref(), Some(&source));
            assert_eq!(old.draft, "source draft");
            assert_eq!(old.draft_images.len(), 1);
            host.composer
                .update(cx, |composer, cx| composer.set_value("target draft", cx));
            host.open_worktree_session(source.clone(), cx);
            assert_eq!(host.history.active, source_id);
            assert_eq!(host.composer.read(cx).value(cx), "source draft");
            assert_eq!(host.composer.read(cx).attachments().len(), 1);
            host.open_worktree_session(target.clone(), cx);
            assert_eq!(host.history.active, target_id);
            assert_eq!(host.composer.read(cx).value(cx), "target draft");
        });
    }
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
