//! Project-page layout owns a persistent explorer and independently loaded documents.
use super::*;
use crate::edits_ui::{ConflictResolved, ConflictView};
use crate::file_preview::{Loaded, Target};
use crate::git_ui::CommitDiff;
use crate::go_definition::{GoDefinitions, ProcessState};
use crate::project_changes::{ChangeSnapshot, ProjectChanges, StageSelection};
use crate::shortcuts::DefinitionModifier;
use gpui::{Animation, AnimationExt, EventEmitter};
use gpui_glint::components::{
    ChangeNavigation, ChangeNavigationEvent, CodeSelection, FilePreview, FilePreviewEvent,
    GitGraph, GitGraphEvent, MenuItem, PromptEditor, Skeleton, Tooltip,
};
use std::{ops::Range, path::Path, time::Duration};

const EXPLORER_MIN_WIDTH: f32 = 240.;
const EXPLORER_MAX_WIDTH: f32 = 720.;
const DOCUMENT_MIN_WIDTH: f32 = 320.;
const EXPLORER_DIVIDER_WIDTH: f32 = 5.;

pub(super) struct ProjectExcerpt {
    pub project: PathBuf,
    pub path: String,
    pub selection: CodeSelection,
}

struct Document {
    id: u64,
    path: String,
    view: DocumentView,
    _task: Option<Task<()>>,
}

enum DocumentView {
    File(Entity<FilePreview>),
    Diff(CommitDiff),
    Conflict {
        editor: Option<Entity<ConflictView>>,
        error: Option<String>,
        scroll: ScrollHandle,
    },
}

struct DocumentTabMenu {
    tab: u64,
    window: gpui::WindowId,
    position: gpui::Point<gpui::Pixels>,
    focus: gpui::FocusHandle,
}

#[derive(Clone, Copy)]
enum CloseTabs {
    Others,
    Left,
    Right,
}

struct GoSymbol {
    view: Entity<FilePreview>,
    path: String,
    source: SharedString,
    range: Range<usize>,
}

struct DefinitionHover {
    view: Entity<FilePreview>,
    range: Range<usize>,
    target: Option<(String, usize)>,
    _task: Task<()>,
}

/// The explorer column shows exactly one panel at a time.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ExplorerMode {
    Tree,
    Graph,
    Changes,
}

impl ExplorerMode {
    fn index(self) -> usize {
        match self {
            Self::Tree => 0,
            Self::Graph => 1,
            Self::Changes => 2,
        }
    }

    fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Graph,
            2 => Self::Changes,
            _ => Self::Tree,
        }
    }
}

pub(super) struct ProjectFiles {
    root: Option<PathBuf>,
    tree: Entity<FilePreview>,
    graph: Option<Entity<GitGraph>>,
    changes: Option<Entity<ChangeNavigation>>,
    mode: ExplorerMode,
    compare_branch: String,
    history_limit: usize,
    graph_task: Option<Task<()>>,
    changes_task: Option<Task<()>>,
    changes_by_root: std::collections::HashMap<PathBuf, ProjectChanges>,
    commit_task: Option<Task<()>>,
    /// Git Graph commit-details loads. Kept apart from `commit_task` so a
    /// details request cannot make the commit button look busy, and so a graph
    /// refresh cannot drop (and thereby cancel) a real commit in flight.
    details_task: Option<Task<()>>,
    commit_entry: Option<Entity<PromptEditor>>,
    commit_notice: Option<String>,
    /// The workspace that owns provider and prompt settings for the wand.
    owner: Option<gpui::WeakEntity<AgentWorkspace>>,
    go_definitions: Option<GoDefinitions>,
    definition_modifier: DefinitionModifier,
    definition_hover: Option<DefinitionHover>,
    definition_task: Option<Task<()>>,
    definition_error: Option<String>,
    documents: Vec<Document>,
    active: Option<usize>,
    tab_menu: Option<DocumentTabMenu>,
    explorer_width: f32,
    explorer_maximum: std::cell::Cell<f32>,
    explorer_resize: Option<(gpui::Pixels, f32)>,
    task: Option<Task<()>>,
    directories: std::collections::HashMap<u64, Task<()>>,
    theme: ThemePreset,
    visible: bool,
    next_document_id: u64,
}

impl EventEmitter<ProjectExcerpt> for ProjectFiles {}

async fn read(target: Target) -> Result<(String, Option<String>, Loaded), String> {
    if target.root.as_deref().is_some_and(ssh_project::is_remote) {
        provider::Job::start(file_preview::load(target))?
            .result()
            .await
    } else {
        file_preview::load(target).await
    }
}

impl ProjectFiles {
    pub(super) fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub(super) fn go_process(&self) -> Option<(&Path, ProcessState)> {
        Some((self.root()?, self.go_definitions.as_ref()?.process_state()))
    }

    pub fn new(cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| {
            let mut view = FilePreview::new(cx);
            view.set_standalone(true, cx);
            view.set_expanded(true, cx);
            view.set_content_only(true, cx);
            view
        });
        cx.subscribe(&tree, |this, _, event: &FilePreviewEvent, cx| match event {
            FilePreviewEvent::OpenRequested(path) => this.open(path.clone(), cx),
            FilePreviewEvent::DirectoryRequested { path, request } => {
                this.expand(path.clone(), *request, cx)
            }
            _ => {}
        })
        .detach();
        Self {
            root: None,
            tree,
            graph: None,
            changes: None,
            mode: ExplorerMode::Tree,
            compare_branch: "master".into(),
            history_limit: git_project::DEFAULT_HISTORY_LIMIT,
            graph_task: None,
            changes_task: None,
            changes_by_root: Default::default(),
            commit_task: None,
            details_task: None,
            commit_entry: None,
            commit_notice: None,
            owner: None,
            go_definitions: None,
            definition_modifier: Default::default(),
            definition_hover: None,
            definition_task: None,
            definition_error: None,
            documents: vec![],
            active: None,
            tab_menu: None,
            explorer_width: 260.,
            explorer_maximum: std::cell::Cell::new(EXPLORER_MAX_WIDTH),
            explorer_resize: None,
            task: None,
            directories: Default::default(),
            theme: ThemePreset::Moonstone,
            visible: true,
            next_document_id: 0,
        }
    }

    /// Points the commit footer at the workspace that owns model and prompt
    /// preferences. Set once, when the workspace is created. The footer reads
    /// live workspace state (busy flag, menu, notice), so it must repaint
    /// whenever that state changes.
    pub fn set_owner(&mut self, owner: gpui::WeakEntity<AgentWorkspace>, cx: &mut Context<Self>) {
        if let Some(owner) = owner.upgrade() {
            cx.observe(&owner, |_, _, cx| cx.notify()).detach();
        }
        self.owner = Some(owner);
    }

    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        if self.root.as_ref() == Some(&root) {
            return;
        }
        self.root = Some(root);
        self.graph = None;
        self.changes = None;
        self.graph_task = None;
        self.changes_task = None;
        self.commit_task = None;
        self.details_task = None;
        self.commit_entry = None;
        self.commit_notice = None;
        self.definition_task = None;
        self.definition_error = None;
        self.compare_branch = "master".into();
        self.history_limit = git_project::DEFAULT_HISTORY_LIMIT;
        for document in &self.documents {
            if let DocumentView::File(view) = &document.view {
                view.update(cx, |view, cx| view.set_active(false, cx));
            }
        }
        self.documents.clear();
        self.active = None;
        self.tab_menu = None;
        self.refresh(cx);
        match self.mode {
            ExplorerMode::Graph => self.show_git_graph(cx),
            ExplorerMode::Changes => self.show_changes(cx),
            ExplorerMode::Tree => {}
        }
    }

    pub(super) fn show_git_graph(&mut self, cx: &mut Context<Self>) {
        self.mode = ExplorerMode::Graph;
        if self.graph.is_none() {
            let graph = cx.new(|cx| {
                let mut graph = GitGraph::new(cx)
                    .header_visible(false)
                    .background_visible(false);
                graph.set_theme(self.theme, cx);
                graph
            });
            cx.subscribe(&graph, |this, _, event: &GitGraphEvent, cx| match event {
                GitGraphEvent::CompareBranch(branch) => {
                    this.compare_branch = branch.clone();
                    this.refresh_graph(cx);
                }
                GitGraphEvent::SelectCommit(id) => this.load_commit(id.clone(), cx),
                GitGraphEvent::OpenFile { commit, path } => {
                    this.open_diff(CommitDiff::new(commit.clone(), path.clone()), cx)
                }
                GitGraphEvent::OpenComparedFile { base, path } => {
                    this.open_diff(CommitDiff::against_worktree(base.clone(), path.clone()), cx)
                }
                GitGraphEvent::RefreshFiles => this.load_graph(false, cx),
                GitGraphEvent::Refresh => this.refresh_graph(cx),
                GitGraphEvent::LoadMore => {
                    this.history_limit += git_project::DEFAULT_HISTORY_LIMIT;
                    this.load_graph(false, cx);
                }
                GitGraphEvent::CloseRequested => {
                    this.mode = ExplorerMode::Tree;
                    cx.notify();
                }
            })
            .detach();
            self.graph = Some(graph);
        }
        self.refresh_graph(cx);
        cx.notify();
    }

    pub(super) fn refresh_graph(&mut self, cx: &mut Context<Self>) {
        self.load_graph(true, cx);
    }

    /// Extend the history window without clearing rows that are already visible.
    fn load_graph(&mut self, show_loading: bool, cx: &mut Context<Self>) {
        let (Some(root), Some(graph)) = (self.root.clone(), self.graph.clone()) else {
            return;
        };
        self.graph_task = None;
        let branch = self.compare_branch.clone();
        let limit = self.history_limit;
        if show_loading {
            graph.update(cx, |graph, cx| graph.set_loading(cx));
        }
        let job = match provider::Job::start(async move {
            git_project::graph_window(&root, branch, limit).await
        }) {
            Ok(job) => job,
            Err(error) => {
                graph.update(cx, |graph, cx| graph.set_error(error, cx));
                return;
            }
        };
        let graph = graph.downgrade();
        self.graph_task = Some(cx.spawn(async move |_, cx| {
            let result = job.result().await;
            let _ = graph.update(cx, |graph, cx| match result {
                Ok(data) => graph.set_data(data, cx),
                Err(error) => graph.set_error(error, cx),
            });
        }));
    }

    pub(super) fn show_changes(&mut self, cx: &mut Context<Self>) {
        self.mode = ExplorerMode::Changes;
        if self.changes.is_none() {
            let changes = cx.new(|cx| {
                let mut panel = ChangeNavigation::new(cx)
                    .stage_controls(true)
                    .background_visible(false);
                panel.set_theme(self.theme, cx);
                panel
            });
            cx.subscribe(
                &changes,
                |this, changes, event: &ChangeNavigationEvent, cx| match event {
                    ChangeNavigationEvent::Select { path, staged } => {
                        if let Some(label) = changes.read(cx).conflict_label(path) {
                            this.open_conflict(path.clone(), label.to_owned(), cx);
                        } else {
                            this.open_worktree_diff(path.clone(), *staged, cx);
                        }
                    }
                    ChangeNavigationEvent::Stage { paths, staged } => {
                        this.queue_staging(StageSelection::Files(paths), *staged, cx)
                    }
                    ChangeNavigationEvent::StageGroup { layer, staged } => {
                        this.queue_staging(StageSelection::Group(*layer), *staged, cx)
                    }
                    ChangeNavigationEvent::StageDirectory { .. }
                    | ChangeNavigationEvent::ToggleDirectory { .. }
                    | ChangeNavigationEvent::Menu { .. } => {}
                },
            )
            .detach();
            let entry = cx.new(|cx| {
                PromptEditor::new(cx, "输入提交信息…")
                    .theme(self.theme)
                    .min_rows(2)
            });
            cx.observe(&entry, |_, _, cx| cx.notify()).detach();
            self.commit_entry = Some(entry);
            self.changes = Some(changes);
            if let Some(root) = self.root.clone() {
                self.publish_changes(&root, true, cx);
            }
        }
        // Re-read on every switch: the worktree changes while the page stays open.
        self.refresh_changes(cx);
        cx.notify();
    }

    pub(super) fn refresh_changes(&mut self, cx: &mut Context<Self>) {
        let (Some(root), Some(changes)) = (self.root.clone(), self.changes.clone()) else {
            return;
        };
        if ssh_project::is_remote(&root) {
            changes.update(cx, |changes, cx| {
                changes.set_error(
                    "SSH 项目暂不支持工作区变更列表，请在终端执行 git status".into(),
                    cx,
                )
            });
            return;
        }
        let state = self.changes_by_root.entry(root.clone()).or_default();
        if state.busy() {
            state.refresh_pending = true;
            return;
        }
        state.refresh_pending = false;
        changes.update(cx, |changes, cx| changes.set_loading(cx));
        let work_root = root.clone();
        let work = cx
            .background_executor()
            .spawn(async move { ChangeSnapshot::read(&work_root) });
        self.changes_task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.changes_task.take() {
                    task.detach();
                }
                match result {
                    Ok(snapshot) => {
                        this.changes_by_root
                            .entry(root.clone())
                            .or_default()
                            .snapshot = Some(snapshot);
                        this.publish_changes(&root, true, cx);
                    }
                    Err(error) => changes.update(cx, |changes, cx| changes.set_error(error, cx)),
                }
                cx.notify();
            });
        }));
    }

    fn publish_changes(&self, root: &Path, rows_changed: bool, cx: &mut Context<Self>) {
        if self.root.as_deref() != Some(root) {
            return;
        }
        let (Some(changes), Some(state)) = (self.changes.as_ref(), self.changes_by_root.get(root))
        else {
            return;
        };
        changes.update(cx, |changes, cx| {
            if rows_changed && let Some(snapshot) = &state.snapshot {
                changes.set_rows(snapshot.rows(), cx);
            }
            changes.set_stage_intents(state.intents(), cx);
        });
    }

    fn queue_staging(
        &mut self,
        selection: StageSelection<'_>,
        staged: bool,
        cx: &mut Context<Self>,
    ) {
        if self.commit_task.is_some() {
            return;
        }
        let Some(root) = self.root.clone() else {
            return;
        };
        let Some(state) = self.changes_by_root.get_mut(&root) else {
            return;
        };
        state.enqueue(selection, staged);
        if state.busy() {
            state.error = None;
            state.refresh_pending |= self.changes_task.take().is_some();
            if state.debounce.is_none() && state.task.is_none() {
                let timer = cx.background_executor().timer(Duration::from_millis(80));
                let queued_root = root.clone();
                state.debounce = Some(cx.spawn(async move |weak, cx| {
                    timer.await;
                    let _ = weak.update(cx, |this, cx| {
                        if let Some(task) = this
                            .changes_by_root
                            .get_mut(&queued_root)
                            .and_then(|state| state.debounce.take())
                        {
                            task.detach();
                        }
                        this.start_stage_batch(queued_root, cx);
                    });
                }));
            }
        } else if state.refresh_pending {
            self.refresh_changes(cx);
        }
        self.publish_changes(&root, false, cx);
        cx.notify();
    }

    fn start_stage_batch(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        let Some(state) = self.changes_by_root.get_mut(&root) else {
            return;
        };
        let Some((staged, sources)) = state.take_batch() else {
            if state.refresh_pending && self.root.as_ref() == Some(&root) {
                self.refresh_changes(cx);
            }
            self.publish_changes(&root, false, cx);
            cx.notify();
            return;
        };
        let work = cx
            .background_executor()
            .spawn(async move { workspace_diff::stage_and_refresh(&sources, staged) });
        state.task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                let state = this
                    .changes_by_root
                    .get_mut(&root)
                    .expect("staging project state");
                if let Some(task) = state.task.take() {
                    task.detach();
                }
                state.complete(result);
                this.publish_changes(&root, true, cx);
                this.start_stage_batch(root, cx);
                cx.notify();
            });
        }));
    }

    fn can_commit_changes(&self, cx: &App) -> bool {
        let Some(root) = self.root.as_ref() else {
            return false;
        };
        self.mode == ExplorerMode::Changes
            && !ssh_project::is_remote(root)
            && !self
                .changes_by_root
                .get(root)
                .is_some_and(ProjectChanges::busy)
            && self.commit_task.is_none()
            && self.changes_task.is_none()
            && self.changes.as_ref().is_some_and(|changes| {
                let changes = changes.read(cx);
                changes.staged_count() > 0 && !changes.has_conflicts()
            })
            && self.commit_entry.as_ref().is_some_and(|entry| {
                let entry = entry.read(cx);
                !entry.is_composing() && !entry.value().trim().is_empty()
            })
    }

    /// Commit the index from the changes panel, mirroring the diff sidebar footer.
    fn commit_changes(&mut self, cx: &mut Context<Self>) {
        if !self.can_commit_changes(cx) {
            return;
        }
        let (Some(root), Some(entry)) = (self.root.clone(), self.commit_entry.clone()) else {
            return;
        };
        let message = entry.read(cx).value().to_owned();
        let submitted = message.clone();
        self.commit_notice = None;
        let work = cx
            .background_executor()
            .spawn(async move { workspace_diff::commit_staged(&root, &message) });
        self.commit_task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.commit_task.take() {
                    task.detach();
                }
                this.commit_notice = Some(match result {
                    Ok(()) => {
                        entry.update(cx, |entry, cx| {
                            if !entry.is_composing() && entry.value() == submitted {
                                entry.set_value("", cx);
                            }
                        });
                        "提交成功".to_owned()
                    }
                    Err(error) => format!("提交失败：{error}"),
                });
                this.refresh_changes(cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Asks the workspace to fill the footer's draft from the repository diff.
    fn request_commit_message(&mut self, cx: &mut Context<Self>) {
        let (Some(owner), Some(root), Some(entry)) = (
            self.owner.clone(),
            self.root.clone(),
            self.commit_entry.clone(),
        ) else {
            return;
        };
        let _ = owner.update(cx, |workspace, cx| {
            workspace.generate_commit_message(root, entry, cx);
        });
        cx.notify();
    }

    fn commit_footer(&self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let root = self.root.clone()?;
        if ssh_project::is_remote(&root) {
            return None;
        }
        let entry = self.commit_entry.clone()?;
        entry.update(cx, |entry, cx| entry.set_theme(self.theme, cx));
        let p = self.theme.palette();
        let (staged, has_conflicts) = self.changes.as_ref().map_or((0, false), |changes| {
            let changes = changes.read(cx);
            (changes.staged_count(), changes.has_conflicts())
        });
        let stage_state = self.changes_by_root.get(&root);
        let sync_notice = stage_state.and_then(|state| {
            if state.busy() {
                Some("正在同步暂存区…".to_owned())
            } else {
                state.error.clone()
            }
        });
        let disabled = !self.can_commit_changes(cx);
        let submitting = self.commit_task.is_some();
        let generating = self
            .owner
            .as_ref()
            .and_then(|owner| {
                owner
                    .read_with(cx, |workspace: &AgentWorkspace, _| {
                        workspace.edits_ui.commit_generation.contains_key(&root)
                    })
                    .ok()
            })
            .unwrap_or(false);
        let submit = div()
            .id("project-commit-submit")
            .debug_selector(|| "project-commit-submit".into())
            .size(px(24.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .text_color(rgb(if disabled { p.text_muted } else { p.accent }))
            .when(!disabled, |button| {
                button
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(p.accent_tint)))
                    .on_click(cx.listener(|this, _, _, cx| this.commit_changes(cx)))
            })
            .child(if submitting {
                Icon::Spinner
                    .render(14.)
                    .with_animation(
                        "project-commit-spinner",
                        Animation::new(Duration::from_millis(900)).repeat(),
                        |spinner, delta| {
                            spinner.with_transformation(gpui::Transformation::rotate(
                                gpui::radians(std::f32::consts::TAU * delta),
                            ))
                        },
                    )
                    .into_any_element()
            } else {
                Icon::ArrowUp.render(16.).into_any_element()
            });
        // The wand reads the index, then the working tree, so any change is
        // worth a click; it stays live when idle so a click can explain why
        // nothing can be written.
        let has_changes = staged > 0 || has_conflicts;
        let mut wand = div()
            .id("project-commit-generate")
            .debug_selector(|| "project-commit-generate".into())
            .size(px(24.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .text_color(rgb(if has_changes || generating {
                p.accent
            } else {
                p.text_muted
            }));
        if !generating {
            wand = wand
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(p.accent_tint)))
                .on_click(cx.listener(|this, _, _, cx| this.request_commit_message(cx)));
        }
        wand = wand.child(if generating {
            Icon::Spinner
                .render(14.)
                .with_animation(
                    "project-commit-generation-spinner",
                    Animation::new(Duration::from_millis(900)).repeat(),
                    |spinner, delta| {
                        spinner.with_transformation(gpui::Transformation::rotate(gpui::radians(
                            std::f32::consts::TAU * delta,
                        )))
                    },
                )
                .into_any_element()
        } else {
            Icon::Wand.render(15.).into_any_element()
        });
        let owner = self.owner.clone()?;
        let (label, options, value, open, generation_notice) = owner
            .read_with(cx, |workspace: &AgentWorkspace, _| {
                (
                    workspace.commit_model_label(),
                    workspace.commit_model_options(),
                    workspace.commit_model_value(),
                    workspace.edits_ui.commit_model_open,
                    workspace.commit_notice_for(&root),
                )
            })
            .ok()?;
        let model = commit_message::model_picker(
            "project-commit-model",
            self.theme,
            label,
            options,
            value,
            open,
            owner.clone(),
            |this: &mut AgentWorkspace, open| this.edits_ui.commit_model_open = open,
            |this: &mut AgentWorkspace, value, cx| this.choose_commit_model(value, cx),
        );
        Some(
            div()
                .debug_selector(|| "project-commit-footer".into())
                .flex_shrink_0()
                .border_t_1()
                .border_color(rgb(p.border))
                .p_2()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .id("project-commit-message")
                        .key_context("DiffCommitMessage")
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_2()
                        .rounded(px(12.))
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.workspace))
                        .debug_selector(|| "project-commit-message".into())
                        .child(
                            div()
                                .id("project-commit-message-scroll")
                                .max_h(px(96.))
                                .overflow_y_scroll()
                                .child(entry),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(wand)
                                .child(model)
                                .child(div().flex_1())
                                .child(
                                    Tooltip::new(
                                        submit,
                                        if submitting { "提交中…" } else { "提交" },
                                    )
                                    .theme(self.theme)
                                    .render(cx),
                                ),
                        ),
                )
                .when(has_conflicts || (staged > 0 && !submitting), |footer| {
                    footer.child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(if has_conflicts {
                                "请先解决所有合并冲突".to_owned()
                            } else {
                                format!("提交 {staged} 个已暂存文件")
                            }),
                    )
                })
                .when_some(sync_notice, |footer, notice| {
                    footer.child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(notice),
                    )
                })
                .when_some(
                    self.commit_notice.clone().or(generation_notice),
                    |footer, notice| {
                        footer.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child(notice),
                        )
                    },
                ),
        )
    }

    fn load_commit(&mut self, id: String, cx: &mut Context<Self>) {
        let (Some(root), Some(graph)) = (self.root.clone(), self.graph.as_ref()) else {
            return;
        };
        let commit = id.clone();
        let job =
            match provider::Job::start(async move { git_project::details(&root, &commit).await }) {
                Ok(job) => job,
                Err(error) => {
                    graph.update(cx, |graph, cx| graph.set_details(&id, error, cx));
                    return;
                }
            };
        let graph = graph.downgrade();
        self.details_task = Some(cx.spawn(async move |_, cx| {
            let result = job.result().await;
            let _ = graph.update(cx, |graph, cx| match result {
                Ok(details) => graph.set_commit_details(details, cx),
                Err(error) => graph.set_details(&id, error, cx),
            });
        }));
    }

    fn open_diff(&mut self, mut diff: CommitDiff, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let existing = self.documents.iter().position(|document| {
            matches!(&document.view, DocumentView::Diff(open) if open.commit == diff.commit && open.base == diff.base && open.path == diff.path)
        });
        if let Some(index) = existing
            && diff.base.is_none()
        {
            self.activate(index, cx);
            return;
        }
        let path = diff.path.clone();
        let id = existing.map_or_else(
            || {
                self.next_document_id += 1;
                self.next_document_id
            },
            |index| self.documents[index].id,
        );
        let task = match diff.load(root) {
            Ok(job) => Some(cx.spawn(async move |weak, cx| {
                let result = job.result().await;
                let _ = weak.update(cx, |this, cx| {
                    let Some(document) =
                        this.documents.iter_mut().find(|document| document.id == id)
                    else {
                        return;
                    };
                    if let DocumentView::Diff(diff) = &mut document.view {
                        diff.set_result(result, this.theme, true, cx);
                        cx.notify();
                    }
                });
            })),
            Err(error) => {
                diff.error = Some(error);
                None
            }
        };
        let document = Document {
            id,
            path,
            view: DocumentView::Diff(diff),
            _task: task,
        };
        if let Some(index) = existing {
            self.documents[index] = document;
            self.activate(index, cx);
        } else {
            self.documents.push(document);
            self.activate(self.documents.len() - 1, cx);
        }
    }

    /// Open one worktree layer of a changed file without leaving the explorer.
    fn open_worktree_diff(&mut self, path: String, staged: bool, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let note = if staged { "已暂存" } else { "工作区" };
        if let Some(index) = self.documents.iter().position(|document| {
            matches!(
                &document.view,
                DocumentView::Diff(diff) if diff.commit == note && diff.path == path
            )
        }) {
            self.activate(index, cx);
            return;
        }
        let mut diff = CommitDiff::new(note.to_owned(), path.clone());
        self.next_document_id += 1;
        let id = self.next_document_id;
        let lookup = path.clone();
        let work = provider::Job::start(async move {
            let inventory = workspace_diff::inventory(&root)?;
            let source = inventory
                .sources
                .values()
                .find(|source| source.path.to_string_lossy() == lookup)
                .ok_or_else(|| "文件已不在变更列表中，请刷新后重试".to_owned())?;
            let change = workspace_diff::load_with_options(&source.for_layer(staged), false)
                .map_err(|error| error.to_string())?;
            Ok(change.lines())
        });
        let task = match work {
            Ok(job) => Some(cx.spawn(async move |weak, cx| {
                let result = job.result().await;
                let _ = weak.update(cx, |this, cx| {
                    let Some(document) =
                        this.documents.iter_mut().find(|document| document.id == id)
                    else {
                        return;
                    };
                    if let DocumentView::Diff(diff) = &mut document.view {
                        diff.set_result(result, this.theme, true, cx);
                        cx.notify();
                    }
                });
            })),
            Err(error) => {
                diff.error = Some(error);
                None
            }
        };
        self.documents.push(Document {
            id,
            path,
            view: DocumentView::Diff(diff),
            _task: task,
        });
        self.activate(self.documents.len() - 1, cx);
    }

    fn open_conflict(&mut self, path: String, label: String, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if let Some(index) = self.documents.iter().position(|document| {
            document.path == path && matches!(document.view, DocumentView::Conflict { .. })
        }) {
            if matches!(
                self.documents[index].view,
                DocumentView::Conflict { error: None, .. }
            ) {
                self.activate(index, cx);
                return;
            }
            self.close(index, cx);
        }

        self.next_document_id += 1;
        let id = self.next_document_id;
        let scroll = ScrollHandle::new();
        let work_root = root.clone();
        let work = cx
            .background_executor()
            .spawn(async move { crate::merge_conflicts::load_file(&work_root, &label) });
        let task = cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                let Some(document) = this.documents.iter_mut().find(|document| document.id == id)
                else {
                    return;
                };
                let DocumentView::Conflict {
                    editor,
                    error,
                    scroll,
                } = &mut document.view
                else {
                    return;
                };
                match result {
                    Ok(file) => {
                        let view = cx.new(|cx| {
                            let mut view = ConflictView::new(file, root, None, this.theme, cx);
                            view.reveal(scroll.clone(), 0, cx);
                            view
                        });
                        cx.subscribe(&view, move |this, _, _: &ConflictResolved, cx| {
                            if let Some(index) = this.documents.iter().position(|doc| doc.id == id)
                            {
                                this.close(index, cx);
                            }
                            this.refresh_changes(cx);
                        })
                        .detach();
                        *editor = Some(view);
                    }
                    Err(reason) => *error = Some(reason),
                }
                cx.notify();
            });
        });
        self.documents.push(Document {
            id,
            path,
            view: DocumentView::Conflict {
                editor: None,
                error: None,
                scroll,
            },
            _task: Some(task),
        });
        self.activate(self.documents.len() - 1, cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        self.definition_task = None;
        self.clear_definition_hover(cx);
        self.definition_error = None;
        self.go_definitions = None;
        self.warm_go_definitions();
        self.directories.clear();
        let path = if ssh_project::is_remote(&root) {
            match ssh_project::Remote::from_path(&root) {
                Ok(remote) => remote.directory,
                Err(_) => return,
            }
        } else {
            root.to_string_lossy().into_owned()
        };
        self.tree.update(cx, |view, cx| {
            view.cancel_directory_requests(cx);
            view.set_theme(self.theme, cx);
            view.set_loading(path.clone(), cx);
        });
        let target = Target {
            root: Some(root.clone()),
            path,
            line: None,
        };
        let work = cx.background_executor().spawn(read(target));
        self.task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if this.root.as_ref() != Some(&root) {
                    return;
                }
                if let Some(task) = this.task.take() {
                    task.detach();
                }
                this.tree.update(cx, |view, cx| match result {
                    Ok((path, parent, Loaded::Directory(entries, _))) => {
                        view.set_directory(path, parent, entries, cx)
                    }
                    Err(error) => view.set_error(error, cx),
                    _ => view.set_error("项目目录不可用".into(), cx),
                });
                if let Some(index) = this
                    .active
                    .filter(|index| matches!(this.documents[*index].view, DocumentView::File(_)))
                {
                    this.tree.update(cx, |view, cx| {
                        view.set_selected_path(this.documents[index].path.clone(), cx)
                    });
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn expand(&mut self, path: String, request: u64, cx: &mut Context<Self>) {
        let target = Target {
            root: self.root.clone(),
            path: path.clone(),
            line: None,
        };
        let work = cx.background_executor().spawn(read(target));
        let task = cx.spawn(async move |weak, cx| {
            let result = work.await.and_then(|(_, _, data)| match data {
                Loaded::Directory(entries, truncated) => Ok((entries, truncated)),
                _ => Err("目录不可用".into()),
            });
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.directories.remove(&request) {
                    task.detach();
                }
                this.tree.update(cx, |view, cx| {
                    view.set_directory_children(path, request, result, cx)
                });
            });
        });
        self.directories.insert(request, task);
    }

    fn open(&mut self, path: String, cx: &mut Context<Self>) {
        self.open_at(path, None, cx);
    }

    fn go_definitions(&mut self, first_file: PathBuf) -> Result<&GoDefinitions, String> {
        if self.go_definitions.is_none() {
            let root = self.root.clone().ok_or("项目目录不可用")?;
            self.go_definitions = Some(GoDefinitions::start(root, first_file)?);
        }
        Ok(self.go_definitions.as_ref().unwrap())
    }

    fn warm_go_definitions(&mut self) {
        if self
            .root
            .as_deref()
            .is_some_and(|root| !ssh_project::is_remote(root))
            && let Some(index) = self.active
            && let document = &self.documents[index]
            && matches!(document.view, DocumentView::File(_))
            && Path::new(&document.path)
                .extension()
                .is_some_and(|ext| ext == "go")
        {
            let _ = self.go_definitions(PathBuf::from(&document.path));
        }
    }

    fn go_symbol_at(&self, position: gpui::Point<gpui::Pixels>, cx: &App) -> Option<GoSymbol> {
        if self.root.as_deref().is_none_or(ssh_project::is_remote) {
            return None;
        }
        let DocumentView::File(view) = &self.documents[self.active?].view else {
            return None;
        };
        let (path, source, offset) = view.read(cx).go_source_at(position, cx)?;
        let identifier = |ch: char| ch == '_' || ch.is_alphanumeric();
        let start = source[..offset]
            .char_indices()
            .rev()
            .take_while(|(_, ch)| identifier(*ch))
            .last()
            .map_or(offset, |(index, _)| index);
        let end = offset
            + source[offset..]
                .find(|ch| !identifier(ch))
                .unwrap_or(source.len() - offset);
        Some(GoSymbol {
            view: view.clone(),
            path,
            source,
            range: start..end,
        })
    }

    pub(super) fn set_definition_modifier(
        &mut self,
        modifier: DefinitionModifier,
        cx: &mut Context<Self>,
    ) {
        if self.definition_modifier != modifier {
            self.definition_modifier = modifier;
            self.clear_definition_hover(cx);
        }
    }

    fn clear_definition_hover(&mut self, cx: &mut Context<Self>) {
        if let Some(hover) = self.definition_hover.take() {
            hover
                .view
                .update(cx, |view, cx| view.set_source_underline(None, cx));
        }
    }

    pub(super) fn update_definition_hover(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        modifiers: gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        if !self.definition_modifier.matches(modifiers) {
            self.clear_definition_hover(cx);
            return;
        }
        let Some(symbol) = self.go_symbol_at(position, cx) else {
            self.clear_definition_hover(cx);
            return;
        };
        if self
            .definition_hover
            .as_ref()
            .is_some_and(|hover| hover.view == symbol.view && hover.range == symbol.range)
        {
            return;
        };
        self.clear_definition_hover(cx);
        let GoSymbol {
            view,
            path,
            source,
            range,
        } = symbol;
        let offset = range.start;
        let task = cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(75))
                .await;
            let job = weak.update(cx, |this, _| {
                this.go_definitions(PathBuf::from(&path))
                    .and_then(|session| session.lookup(path, source, offset))
            });
            let Ok(Ok(job)) = job else { return };
            if let Ok(target) = job.result().await {
                let _ = weak.update(cx, |this, cx| {
                    if let Some(hover) = &mut this.definition_hover {
                        hover.target = Some(target);
                        let range = hover.range.clone();
                        hover
                            .view
                            .update(cx, |view, cx| view.set_source_underline(Some(range), cx));
                    }
                });
            }
        });
        self.definition_hover = Some(DefinitionHover {
            view,
            range,
            target: None,
            _task: task,
        });
    }

    fn jump_to_go_definition(&mut self, event: &gpui::MouseDownEvent, cx: &mut Context<Self>) {
        if !self.definition_modifier.matches(event.modifiers)
            || event.button != gpui::MouseButton::Left
        {
            return;
        }
        let Some(symbol) = self.go_symbol_at(event.position, cx) else {
            return;
        };
        cx.stop_propagation();
        self.definition_task = None;
        self.definition_error = None;
        let target = self
            .definition_hover
            .as_ref()
            .filter(|hover| hover.view == symbol.view && hover.range == symbol.range)
            .and_then(|hover| hover.target.clone());
        self.clear_definition_hover(cx);
        if let Some((path, line)) = target {
            self.open_at(path, Some(line), cx);
            return;
        }
        let GoSymbol {
            path,
            source,
            range,
            ..
        } = symbol;
        match self
            .go_definitions(PathBuf::from(&path))
            .and_then(|session| session.lookup(path, source, range.start))
        {
            Ok(job) => {
                self.definition_task = Some(cx.spawn(async move |weak, cx| {
                    let result = job.result().await;
                    let _ = weak.update(cx, |this, cx| {
                        if let Some(task) = this.definition_task.take() {
                            task.detach();
                        }
                        match result {
                            Ok((path, line)) => this.open_at(path, Some(line), cx),
                            Err(error) => this.definition_error = Some(error),
                        }
                        cx.notify();
                    });
                }));
            }
            Err(error) => self.definition_error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn open_search_result(
        &mut self,
        path: String,
        line: Option<usize>,
        query: String,
        cx: &mut Context<Self>,
    ) {
        self.open_at(path, line, cx);
        if let Some(index) = self.active
            && let DocumentView::File(view) = &self.documents[index].view
        {
            view.update(cx, |view, cx| view.set_document_search(query, line, cx));
        }
    }

    pub(super) fn open_at(&mut self, path: String, line: Option<usize>, cx: &mut Context<Self>) {
        if let Some(index) = self.documents.iter().position(|document| {
            document.path == path && matches!(document.view, DocumentView::File(_))
        }) {
            self.activate(index, cx);
            if let Some(line) = line
                && let DocumentView::File(view) = &self.documents[index].view
            {
                view.update(cx, |view, cx| view.reveal_line(line, cx));
            }
            return;
        }
        let view = cx.new(|cx| FilePreview::new(cx).quote_label("交给 Agent"));
        cx.observe(&view, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&view, |this, preview, event: &FilePreviewEvent, cx| {
            if let FilePreviewEvent::QuoteRequested { path, selection } = event
                && let Some(project) = &this.root
                && let Some(index) = this.active
                && matches!(&this.documents[index].view, DocumentView::File(view) if *view == preview)
            {
                cx.emit(ProjectExcerpt {
                    project: project.clone(),
                    path: path.clone(),
                    selection: selection.clone(),
                });
            }
        })
        .detach();
        view.update(cx, |view, cx| {
            view.set_theme(self.theme, cx);
            view.set_loading(path.clone(), cx);
            view.set_standalone(true, cx);
            view.set_content_only(true, cx);
            view.set_expanded(true, cx);
            if let Some(line) = line {
                view.reveal_line(line, cx);
            }
        });
        let target = Target {
            root: self.root.clone(),
            path: path.clone(),
            line: None,
        };
        let work = cx.background_executor().spawn(read(target));
        let document_view = view.downgrade();
        let task = cx.spawn(async move |_, cx| {
            let result = work.await;
            let _ = document_view.update(cx, |view, cx| match result {
                Ok((path, parent, Loaded::File(content))) => {
                    let language = std::path::Path::new(&path)
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("text")
                        .to_owned();
                    view.set_file(path, parent, language, content, cx);
                }
                Ok((path, parent, Loaded::Image(bytes))) => view.set_image(
                    path,
                    parent,
                    std::sync::Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, bytes)),
                    cx,
                ),
                Err(error) => view.set_error(error, cx),
                _ => view.set_error("请从目录树选择文件".into(), cx),
            });
        });
        self.next_document_id += 1;
        self.documents.push(Document {
            id: self.next_document_id,
            path,
            view: DocumentView::File(view),
            _task: Some(task),
        });
        self.activate(self.documents.len() - 1, cx);
    }

    fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        self.tab_menu = None;
        self.clear_definition_hover(cx);
        self.definition_task = None;
        self.definition_error = None;
        self.active = Some(index);
        self.warm_go_definitions();
        if matches!(self.documents[index].view, DocumentView::File(_)) {
            self.tree.update(cx, |view, cx| {
                view.set_selected_path(self.documents[index].path.clone(), cx)
            });
        }
        self.update_active_previews(cx);
        cx.notify();
    }

    fn close(&mut self, index: usize, cx: &mut Context<Self>) {
        self.close_documents(|candidate| candidate == index, index, cx);
    }

    fn close_documents(
        &mut self,
        mut should_close: impl FnMut(usize) -> bool,
        fallback: usize,
        cx: &mut Context<Self>,
    ) {
        self.tab_menu = None;
        self.clear_definition_hover(cx);
        let selected_id = self.active.map(|index| self.documents[index].id);
        let mut index = 0;
        self.documents.retain(|document| {
            let close = should_close(index);
            index += 1;
            if close && let DocumentView::File(view) = &document.view {
                view.update(cx, |view, cx| view.set_active(false, cx));
            }
            !close
        });
        self.active = selected_id.and_then(|id| {
            let last = self.documents.len().checked_sub(1)?;
            Some(
                self.documents
                    .iter()
                    .position(|doc| doc.id == id)
                    .unwrap_or(fallback.min(last)),
            )
        });
        if let Some(index) = self.active {
            self.activate(index, cx);
        }
        cx.notify();
    }

    fn close_tabs_around(&mut self, tab: u64, mode: CloseTabs, cx: &mut Context<Self>) {
        let Some(anchor) = self
            .documents
            .iter()
            .position(|document| document.id == tab)
        else {
            return;
        };
        let fallback = match mode {
            CloseTabs::Others | CloseTabs::Left => 0,
            CloseTabs::Right => anchor,
        };
        self.close_documents(
            |index| match mode {
                CloseTabs::Others => index != anchor,
                CloseTabs::Left => index < anchor,
                CloseTabs::Right => index > anchor,
            },
            fallback,
            cx,
        );
    }

    fn tab_context_menu(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let state = self.tab_menu.as_ref()?;
        if state.window != window.window_handle().window_id() {
            return None;
        }
        let tab = state.tab;
        let anchor = self
            .documents
            .iter()
            .position(|document| document.id == tab)?;
        let p = self.theme.palette();
        let menu = div()
            .id("project-document-tab-menu")
            .track_focus(&state.focus)
            .w(px(192.))
            .p_1()
            .rounded(px(7.))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.tab_menu = None;
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.tab_menu = None;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .children(
                [
                    (
                        "project-tab-close-others",
                        "关闭其他标签页",
                        CloseTabs::Others,
                        self.documents.len() > 1,
                    ),
                    (
                        "project-tab-close-left",
                        "关闭左侧标签页",
                        CloseTabs::Left,
                        anchor > 0,
                    ),
                    (
                        "project-tab-close-right",
                        "关闭右侧标签页",
                        CloseTabs::Right,
                        anchor + 1 < self.documents.len(),
                    ),
                ]
                .into_iter()
                .map(|(id, label, mode, enabled)| {
                    MenuItem::new(id, label)
                        .theme(self.theme)
                        .disabled(!enabled)
                        .render()
                        .debug_selector(move || id.into())
                        .when(enabled, |item| {
                            item.on_click(cx.listener(move |this, _, _, cx| {
                                this.close_tabs_around(tab, mode, cx);
                            }))
                        })
                }),
            );
        Some(
            gpui::deferred(
                gpui::anchored()
                    .position(state.position)
                    .snap_to_window_with_margin(px(8.))
                    .child(menu),
            )
            .with_priority(80)
            .into_any_element(),
        )
    }

    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if !visible {
            self.tab_menu = None;
            self.explorer_resize = None;
            self.clear_definition_hover(cx);
            self.definition_task = None;
            self.definition_error = None;
        }
        self.update_active_previews(cx);
    }

    fn update_active_previews(&self, cx: &mut Context<Self>) {
        for (index, document) in self.documents.iter().enumerate() {
            if let DocumentView::File(view) = &document.view {
                view.update(cx, |view, cx| {
                    view.set_active(self.visible && self.active == Some(index), cx)
                });
            }
        }
    }

    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme == theme {
            return;
        }
        self.theme = theme;
        self.tree.update(cx, |view, cx| view.set_theme(theme, cx));
        if let Some(graph) = &self.graph {
            graph.update(cx, |graph, cx| graph.set_theme(theme, cx));
        }
        if let Some(changes) = &self.changes {
            changes.update(cx, |changes, cx| changes.set_theme(theme, cx));
        }
        for document in &self.documents {
            match &document.view {
                DocumentView::File(view) => view.update(cx, |view, cx| view.set_theme(theme, cx)),
                DocumentView::Diff(diff) => {
                    if let Some(card) = &diff.card {
                        card.update(cx, |card, cx| card.set_theme(theme, cx));
                    }
                }
                DocumentView::Conflict { editor, .. } => {
                    if let Some(editor) = editor {
                        editor.update(cx, |editor, cx| editor.set_theme(theme, cx));
                    }
                }
            }
        }
        cx.notify();
    }
}

impl Render for ProjectFiles {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let weak_select = cx.entity().downgrade();
        let weak_close = cx.entity().downgrade();
        let weak_context = cx.entity().downgrade();
        let tabs =
            gpui_glint::components::ClosableTabs::new(self.documents.iter().map(|document| {
                let label = std::path::Path::new(&document.path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let file_icon = Icon::for_file_name(&label);
                let (label, icon) = match &document.view {
                    DocumentView::File(_) => (label, file_icon),
                    DocumentView::Conflict { .. } => (label, Icon::GitConflict),
                    DocumentView::Diff(diff) => (
                        if diff.base.is_some() {
                            format!("{label} · 分支差异")
                        } else {
                            format!(
                                "{label} · {}",
                                diff.commit.chars().take(7).collect::<String>()
                            )
                        },
                        file_icon,
                    ),
                };
                gpui_glint::components::ClosableTab::new(document.id, label).icon(icon)
            }))
            .selected(self.active.map(|index| self.documents[index].id))
            .theme(self.theme)
            .fill_height()
            .content_padding(0.)
            .on_select(move |id, _, cx| {
                let _ = weak_select.update(cx, |this, cx| {
                    if let Some(index) =
                        this.documents.iter().position(|document| document.id == id)
                    {
                        this.activate(index, cx);
                    }
                });
            })
            .on_close(move |id, _, cx| {
                let _ = weak_close.update(cx, |this, cx| {
                    if let Some(index) =
                        this.documents.iter().position(|document| document.id == id)
                    {
                        this.close(index, cx);
                    }
                });
            })
            .on_context(move |tab, position, window, cx| {
                let _ = weak_context.update(cx, |this, cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    this.tab_menu = Some(DocumentTabMenu {
                        tab,
                        window: window.window_handle().window_id(),
                        position,
                        focus,
                    });
                    cx.notify();
                });
            });
        let document_pane = if let Some(index) = self.active {
            let document = &self.documents[index];
            match &document.view {
                DocumentView::File(view) => {
                    let controls = view.update(cx, |view, cx| view.document_controls(cx));
                    let status = self
                        .definition_error
                        .as_deref()
                        .or(self.definition_task.as_ref().map(|_| "正在查找 Go 定义…"));
                    tabs.trailing(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when_some(status, |row, status| {
                                row.child(
                                    gpui_glint::components::Tooltip::new(
                                        div()
                                            .max_w(px(240.))
                                            .truncate()
                                            .text_size(px(12.))
                                            .text_color(rgb(p.text_muted))
                                            .child(status.to_owned()),
                                        status.to_owned(),
                                    )
                                    .theme(self.theme)
                                    .render(cx),
                                )
                            })
                            .child(controls),
                    )
                    .content(
                        document.id,
                        div()
                            .size_full()
                            .min_h_0()
                            .id("project-file-preview")
                            .debug_selector(|| "project-file-preview".into())
                            .on_mouse_move(cx.listener(
                                |this, event: &gpui::MouseMoveEvent, _, cx| {
                                    if event.dragging() {
                                        this.clear_definition_hover(cx);
                                    } else {
                                        this.update_definition_hover(
                                            event.position,
                                            event.modifiers,
                                            cx,
                                        );
                                    }
                                },
                            ))
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                if !hovered {
                                    this.clear_definition_hover(cx);
                                }
                            }))
                            .on_scroll_wheel(
                                cx.listener(|this, _, _, cx| this.clear_definition_hover(cx)),
                            )
                            .capture_any_mouse_down(cx.listener(|this, event, _, cx| {
                                this.jump_to_go_definition(event, cx);
                            }))
                            .child(view.clone()),
                    )
                    .render()
                }
                DocumentView::Conflict {
                    editor,
                    error,
                    scroll,
                } => {
                    let content = div()
                        .id("project-conflict-editor")
                        .size_full()
                        .min_h_0()
                        .overflow_y_scroll()
                        .track_scroll(scroll)
                        .children(editor.clone())
                        .when_some(error.clone(), |body, error| {
                            body.child(div().p_3().child(error))
                        })
                        .when(editor.is_none() && error.is_none(), |body| {
                            body.child(
                                div().p_4().child(
                                    Skeleton::new("project-conflict-loading")
                                        .lines(5)
                                        .theme(self.theme),
                                ),
                            )
                        });
                    tabs.content(document.id, content).render()
                }
                DocumentView::Diff(diff) => {
                    let content = div()
                        .size_full()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .when_some(diff.card.clone(), |body, card| body.child(card))
                        .when_some(diff.error.clone(), |body, error| {
                            body.child(div().p_3().child(error))
                        })
                        .when(diff.card.is_none() && diff.error.is_none(), |body| {
                            body.child(
                                div().p_4().child(
                                    Skeleton::new("project-commit-diff-loading")
                                        .lines(12)
                                        .height(12.)
                                        .gap(14.)
                                        .theme(self.theme),
                                ),
                            )
                        });
                    tabs.content(document.id, content).render()
                }
            }
        } else {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(p.text_muted))
                .child(match self.mode {
                    ExplorerMode::Graph => "选择提交中的文件以查看差异",
                    ExplorerMode::Changes => "点击左侧文件查看未提交的差异",
                    ExplorerMode::Tree => "从左侧目录树打开文件",
                })
        };
        let weak = cx.entity().downgrade();
        let segments = SegmentedControl::new([
            SegmentOption::new("文件目录").debug_selector("project-directory-segment"),
            SegmentOption::new("Git Graph").debug_selector("project-git-graph-segment"),
            SegmentOption::new("变更").debug_selector("project-git-changes-segment"),
        ])
        .id("project-explorer-mode")
        .compact()
        .selected(self.mode.index())
        .theme(self.theme)
        .on_change(move |index, _, cx| {
            let _ = weak.update(cx, |this, cx| match ExplorerMode::from_index(index) {
                ExplorerMode::Graph => this.show_git_graph(cx),
                ExplorerMode::Changes => this.show_changes(cx),
                ExplorerMode::Tree => {
                    this.mode = ExplorerMode::Tree;
                    cx.notify();
                }
            });
        });
        let navigation = match self.mode {
            ExplorerMode::Graph => self.graph.clone().map(IntoElement::into_any_element),
            ExplorerMode::Changes => self.changes.clone().map(IntoElement::into_any_element),
            ExplorerMode::Tree => Some(self.tree.clone().into_any_element()),
        };
        let weak = cx.entity().downgrade();
        let explorer_width = self
            .explorer_width
            .clamp(EXPLORER_MIN_WIDTH, self.explorer_maximum.get());
        div()
            .size_full()
            .flex()
            .min_h_0()
            .min_w_0()
            .on_children_prepainted(move |bounds, _, cx| {
                let available: f32 = bounds
                    .iter()
                    .take(3)
                    .map(|bounds| f32::from(bounds.size.width))
                    .sum();
                let maximum = (available - DOCUMENT_MIN_WIDTH - EXPLORER_DIVIDER_WIDTH)
                    .clamp(EXPLORER_MIN_WIDTH, EXPLORER_MAX_WIDTH);
                let _ = weak.update(cx, |this, cx| {
                    if this.explorer_maximum.replace(maximum) != maximum {
                        let id = cx.entity_id();
                        cx.defer(move |cx| cx.notify(id));
                    }
                });
            })
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if let Some((start, width)) = this.explorer_resize {
                    if event.dragging() {
                        this.explorer_width = (width + f32::from(event.position.x - start))
                            .clamp(EXPLORER_MIN_WIDTH, this.explorer_maximum.get());
                    } else {
                        this.explorer_resize = None;
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.explorer_resize = None;
                }),
            )
            .child(
                div()
                    .debug_selector(|| "project-explorer-sidebar".into())
                    .w(px(explorer_width))
                    .h_full()
                    .min_h_0()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .bg(rgb(p.panel))
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .justify_between()
                            .px_3()
                            .py_2()
                            .border_b_1()
                            .border_color(rgb(p.border))
                            .child(segments)
                            .when(self.mode != ExplorerMode::Tree, |header| {
                                header.child(
                                    IconButton::new(Icon::Refresh, "刷新 Git")
                                        .theme(self.theme)
                                        .render()
                                        .debug_selector(|| "project-explorer-refresh".into())
                                        .on_click(cx.listener(|this, _, _, cx| match this.mode {
                                            ExplorerMode::Changes => this.refresh_changes(cx),
                                            _ => this.refresh_graph(cx),
                                        })),
                                )
                            }),
                    )
                    .child(div().flex_1().min_h_0().children(navigation))
                    .when(self.mode == ExplorerMode::Changes, |column| {
                        column.children(self.commit_footer(cx))
                    }),
            )
            .child(
                div()
                    .id("project-explorer-resize")
                    .debug_selector(|| "project-explorer-resize".into())
                    .w(px(EXPLORER_DIVIDER_WIDTH))
                    .h_full()
                    .flex_shrink_0()
                    .cursor(gpui::CursorStyle::ResizeLeftRight)
                    .bg(rgb(p.border))
                    .hover(|style| style.bg(rgb(p.accent)))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                            this.explorer_resize = Some((event.position.x, explorer_width));
                            cx.stop_propagation();
                        }),
                    ),
            )
            .child(div().flex_1().min_w_0().h_full().child(document_pane))
            .children(self.tab_context_menu(window, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn project_changes_checkboxes_update_before_git_and_cancel_a_queued_click(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        let repo = workspace_diff::tests::many_files(2);
        let root = repo.path().canonicalize().unwrap();
        let path = root.join("vendor/0000.rs").to_string_lossy().into_owned();
        let checkbox = Box::leak(format!("stage-file:{path}:false").into_boxed_str());
        let (files, window) = cx.add_window_view(|_, cx| ProjectFiles::new(cx));
        window.simulate_resize(size(px(1100.), px(720.)));
        files.update(window, |files, cx| {
            files.set_root(root.clone(), cx);
            files.show_changes(cx);
        });
        let until = |window: &mut gpui::VisualTestContext,
                     done: &dyn Fn(&mut gpui::VisualTestContext) -> bool| {
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            loop {
                window.run_until_parked();
                if done(window) {
                    break;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        until(window, &|window| window.debug_bounds(checkbox).is_some());
        let click = |window: &mut gpui::VisualTestContext, selector| {
            let bounds = window.debug_bounds(selector).unwrap();
            window.simulate_click(bounds.center(), Default::default());
            window.run_until_parked();
        };
        click(window, checkbox);
        files.read_with(window, |files, cx| {
            let changes = &files.changes_by_root[&root];
            assert_eq!(changes.intents().get(&path), Some(&true));
            assert!(changes.task.is_none());
            assert_eq!(files.changes.as_ref().unwrap().read(cx).staged_count(), 0);
        });
        click(window, checkbox);
        files.read_with(window, |files, _| {
            assert!(!files.changes_by_root[&root].busy())
        });
        window.executor().advance_clock(Duration::from_millis(100));
        window.run_until_parked();
        assert!(
            workspace_diff::inventory(&root)
                .unwrap()
                .sources
                .values()
                .all(|file| !file.staged)
        );

        click(window, "stage-group:false");
        files.read_with(window, |files, _| {
            assert_eq!(files.changes_by_root[&root].intents().len(), 2)
        });
        window.executor().advance_clock(Duration::from_millis(100));
        until(window, &|window| {
            files.read_with(window, |files, cx| {
                !files.changes_by_root[&root].busy()
                    && files.changes.as_ref().unwrap().read(cx).staged_count() == 2
            })
        });
        assert!(
            workspace_diff::inventory(&root)
                .unwrap()
                .sources
                .values()
                .all(|file| file.staged)
        );
    }

    #[gpui::test]
    fn document_tab_context_closes_around_the_clicked_tab(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let paths = ["first.rs", "keep.toml", "third.go", "fourth.txt"].map(|name| {
            let path = root.join(name);
            std::fs::write(&path, "content").unwrap();
            path.to_string_lossy().into_owned()
        });
        let (files, window) = cx.add_window_view(|_, cx| ProjectFiles::new(cx));
        window.simulate_resize(size(px(1200.), px(720.)));
        files.update(window, |files, cx| {
            files.set_root(root.to_owned(), cx);
            for path in &paths {
                files.open(path.clone(), cx);
            }
        });
        let tab = files.read_with(window, |files, _| files.documents[1].id);
        let tab_selector: &'static str =
            Box::leak(format!("glass-closable-tab:{tab}").into_boxed_str());
        let action = |window: &mut gpui::VisualTestContext, selector| {
            window.run_until_parked();
            let position = window.debug_bounds(tab_selector).unwrap().center();
            window.simulate_mouse_down(position, gpui::MouseButton::Right, Default::default());
            window.run_until_parked();
            let menu = window.debug_bounds(selector).unwrap();
            window.simulate_click(menu.center(), Default::default());
            window.run_until_parked();
        };
        action(window, "project-tab-close-right");
        files.read_with(window, |files, _| {
            assert_eq!(files.documents.len(), 2);
            assert_eq!(files.documents[files.active.unwrap()].id, tab);
        });
        files.update(window, |files, cx| files.open(paths[2].clone(), cx));
        action(window, "project-tab-close-left");
        files.read_with(window, |files, _| {
            assert_eq!(files.documents.len(), 2);
            assert_eq!(files.documents[0].id, tab);
            assert_eq!(files.documents[files.active.unwrap()].path, paths[2]);
        });
        action(window, "project-tab-close-others");
        files.read_with(window, |files, _| {
            assert_eq!(files.documents.len(), 1);
            assert_eq!(files.documents[files.active.unwrap()].id, tab);
        });
    }

    #[gpui::test]
    fn project_explorer_divider_resizes_the_sidebar(cx: &mut gpui::TestAppContext) {
        let (files, window) = cx.add_window_view(|_, cx| ProjectFiles::new(cx));
        window.simulate_resize(size(px(1100.), px(720.)));
        window.run_until_parked();
        let width = |window: &mut gpui::VisualTestContext| {
            window
                .debug_bounds("project-explorer-sidebar")
                .unwrap()
                .size
                .width
        };
        let before = width(window);
        let start = window
            .debug_bounds("project-explorer-resize")
            .unwrap()
            .center();
        let end = gpui::point(start.x + px(150.), start.y);
        window.simulate_mouse_down(start, gpui::MouseButton::Left, Default::default());
        window.simulate_mouse_move(end, gpui::MouseButton::Left, Default::default());
        window.simulate_mouse_up(end, gpui::MouseButton::Left, Default::default());
        window.run_until_parked();
        assert_eq!(width(window), before + px(150.));
        files.read_with(window, |files, _| assert!(files.explorer_resize.is_none()));
        window.simulate_resize(size(px(650.), px(720.)));
        window.run_until_parked();
        assert!(width(window) <= px(650. - DOCUMENT_MIN_WIDTH - EXPLORER_DIVIDER_WIDTH));
    }

    #[gpui::test]
    #[ignore = "requires Go and gopls on PATH"]
    fn go_definition_shortcut_resolves_hover_and_jumps(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let router = root.join("router.go");
        let handler = root.join("handler.go");
        let name = "handleItem";
        // Unicode and a tab before the click exercise mapping from shaped text to byte offsets.
        let call = format!(
            "package fixture\nfunc route(h *Handler) {{\n\t_ = \"中文\"; _ = h.{name}\n}}\n"
        );
        let definition = format!(
            "package fixture\ntype Handler struct{{}}\n{}func (*Handler) {name}() {{}}\n{}",
            "\n".repeat(100),
            "\n".repeat(60)
        );
        std::fs::write(root.join("go.mod"), "module example.com/fixture\ngo 1.21\n").unwrap();
        std::fs::write(&router, &call).unwrap();
        std::fs::write(&handler, &definition).unwrap();

        let history_path = root.join("sessions.json");
        std::fs::write(
            root.join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (workspace, window) =
            cx.add_window_view(|_, cx| AgentWorkspace::with_path(history_path.clone(), cx));
        window.simulate_resize(gpui::size(px(1440.), px(1100.)));
        let files = workspace.read_with(window, |workspace, _| workspace.project_files.clone());
        workspace.update_in(window, |workspace, window, cx| {
            workspace.navigate(crate::Page::Projects, window, cx);
            workspace.choose_project(root.clone(), cx);
            files.update(cx, |files, cx| {
                files.open(router.to_string_lossy().into_owned(), cx)
            });
        });

        let find_symbol = |path: &Path, source: &str, window: &mut gpui::VisualTestContext| {
            let start = source.find(name).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(65);
            loop {
                window.run_until_parked();
                let bounds = window.debug_bounds("project-file-preview").unwrap();
                let target = files.read_with(window, |files, cx| {
                    assert!(
                        files.definition_error.is_none(),
                        "{:?}",
                        files.definition_error
                    );
                    let document = &files.documents[files.active?];
                    let DocumentView::File(view) = &document.view else {
                        return None;
                    };
                    if Path::new(&document.path) != path {
                        return None;
                    }
                    for y in (2..100).step_by(4) {
                        for x in (25..420).step_by(4) {
                            let position = gpui::point(
                                bounds.left() + px(x as f32),
                                bounds.top() + px(y as f32),
                            );
                            if view.read(cx).go_source_at(position, cx).is_some_and(
                                |(_, _, offset)| (start..start + name.len()).contains(&offset),
                            ) {
                                return Some(position);
                            }
                        }
                    }
                    None
                });
                if let Some(position) = target {
                    return position;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "symbol not visible in {path:?}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };

        let position = find_symbol(&router, &call, window);
        let shift = gpui::Modifiers {
            shift: true,
            ..Default::default()
        };
        window.simulate_click(position, shift);
        window.run_until_parked();
        files.read_with(window, |files, _| assert!(files.definition_task.is_none()));
        window.simulate_click(
            position,
            gpui::Modifiers {
                control: true,
                ..Default::default()
            },
        );
        find_symbol(&handler, &definition, window);
        files.read_with(window, |files, _| assert_eq!(files.documents.len(), 2));

        workspace.update_in(window, |workspace, window, cx| {
            workspace.navigate(crate::Page::Settings, window, cx);
            workspace.provider_settings.section = crate::provider_ui::Section::Shortcuts;
            cx.notify();
        });
        window.run_until_parked();
        let option = window.debug_bounds("go-definition-modifier-Shift").unwrap();
        window.simulate_click(option.center(), Default::default());
        workspace.update_in(window, |workspace, window, cx| {
            assert_eq!(
                workspace.history.definition_modifier,
                DefinitionModifier::Shift
            );
            workspace.history.save(&history_path).unwrap();
            assert_eq!(
                History::load(&history_path).unwrap().definition_modifier,
                DefinitionModifier::Shift
            );
            workspace.navigate(crate::Page::Projects, window, cx);
            files.update(cx, |files, cx| files.activate(0, cx));
        });
        let position = find_symbol(&router, &call, window);
        window.simulate_mouse_move(position, None, Default::default());
        let symbol_start = call.find(name).unwrap();
        let symbol_range = symbol_start..symbol_start + name.len();
        let resolve_hover = |window: &mut gpui::VisualTestContext| {
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            window.simulate_modifiers_change(shift);
            window.executor().advance_clock(Duration::from_millis(100));
            loop {
                window.run_until_parked();
                if files.read_with(window, |files, _| {
                    files
                        .definition_hover
                        .as_ref()
                        .is_some_and(|hover| hover.range == symbol_range && hover.target.is_some())
                }) {
                    return;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "hover definition did not resolve"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        resolve_hover(window);
        window.simulate_modifiers_change(Default::default());
        files.read_with(window, |files, _| assert!(files.definition_hover.is_none()));
        resolve_hover(window);
        window.simulate_click(position, shift);
        files.read_with(window, |files, _| {
            assert_eq!(files.active, Some(1));
            assert!(
                files.definition_task.is_none(),
                "click should reuse the hovered target"
            );
        });
    }

    #[gpui::test]
    fn project_graph_opens_commit_files_in_reusable_diff_tabs(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .current_dir(&root)
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "core.hooksPath=disabled-hooks",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        git(&["init", "-q", "-b", "master"]);
        std::fs::write(root.join("main.rs"), "fn first() {}\n").unwrap();
        git(&["add", "main.rs"]);
        git(&["commit", "-qm", "First"]);
        let first = git(&["rev-parse", "HEAD"]);
        std::fs::write(root.join("main.rs"), "fn second() {}\n").unwrap();
        git(&["add", "main.rs"]);
        git(&["commit", "-qm", "Second"]);
        let second = git(&["rev-parse", "HEAD"]);
        std::fs::write(root.join("main.rs"), "fn working_copy() {}\n").unwrap();

        let (files, window) = cx.add_window_view(|_, cx| ProjectFiles::new(cx));
        window.simulate_resize(gpui::size(px(900.), px(720.)));
        files.update(window, |files, cx| {
            files.set_root(root.clone(), cx);
            files.open(root.join("main.rs").to_string_lossy().into_owned(), cx);
        });
        let click = |selector: &'static str, window: &mut gpui::VisualTestContext| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                window.run_until_parked();
                if let Some(bounds) = window.debug_bounds(selector) {
                    window.simulate_click(bounds.center(), Default::default());
                    return;
                }
                assert!(std::time::Instant::now() < deadline, "missing {selector}");
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        click("project-git-graph-segment", window);
        window.run_until_parked();
        assert!(window.debug_bounds("git-graph").is_some());
        click(
            Box::leak(format!("git-commit:{second}").into_boxed_str()),
            window,
        );
        click("git-file:main.rs", window);
        click("git-file:main.rs", window);
        files.read_with(window, |files, _| {
            assert_eq!(files.documents.len(), 2);
            assert_eq!(files.active, Some(1));
        });
        click("project-directory-segment", window);
        click("project-git-graph-segment", window);
        click("git-file:main.rs", window);
        click(
            Box::leak(format!("git-commit:{first}").into_boxed_str()),
            window,
        );
        click("git-file:main.rs", window);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            window.run_until_parked();
            let loaded = files.read_with(window, |files, cx| {
                assert_eq!(files.documents.len(), 3);
                assert_eq!(files.active, Some(2));
                files.documents[1..]
                    .iter()
                    .zip([(&second, (1, 1)), (&first, (1, 0))])
                    .all(|(document, (commit, stats))| {
                        let DocumentView::Diff(diff) = &document.view else {
                            panic!("expected diff tab")
                        };
                        assert_eq!(&diff.commit, commit);
                        assert!(diff.error.is_none(), "{:?}", diff.error);
                        diff.card.as_ref().is_some_and(|card| {
                            assert_eq!(card.read(cx).stats(), stats);
                            true
                        })
                    })
            });
            if loaded {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "diffs failed to load");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(window.debug_bounds("git-graph").is_some());
        assert!(window.debug_bounds("glass-diff-card-body").is_some());
    }

    #[gpui::test]
    fn tabs_reuse_document_views_close_neighbors_and_keep_explorer(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let first = root.join("first.rs").to_string_lossy().into_owned();
        let second = root.join("second.rs").to_string_lossy().into_owned();
        std::fs::write(&first, "fn first() {}\n").unwrap();
        std::fs::write(&second, "fn second() {}\n").unwrap();
        let (files, window) = cx.add_window_view(|_, cx| ProjectFiles::new(cx));
        files.update(window, |files, cx| files.set_root(root.clone(), cx));
        window.run_until_parked();
        files.update(window, |files, cx| {
            let tree = files.tree.entity_id();
            files.open(first.clone(), cx);
            let DocumentView::File(first_view) = &files.documents[0].view else {
                panic!("expected file")
            };
            let first_view = first_view.entity_id();
            files.open(second.clone(), cx);
            assert_eq!(files.active, Some(1));
            files.open(first.clone(), cx);
            assert_eq!(files.documents.len(), 2);
            assert_eq!(files.active, Some(0));
            let DocumentView::File(view) = &files.documents[0].view else {
                panic!("expected file")
            };
            assert_eq!(view.entity_id(), first_view);
            files.close(0, cx);
            assert_eq!(files.active, Some(0));
            assert_eq!(files.documents[0].path, second);
            files.close(0, cx);
            assert!(files.active.is_none());
            assert!(files.documents.is_empty());
            assert_eq!(files.tree.entity_id(), tree);
            files.open(first.clone(), cx);
            files.set_root(root.join("another-project"), cx);
            assert!(files.documents.is_empty());
            assert!(files.active.is_none());
        });
    }

    #[gpui::test]
    fn project_changes_tab_reuses_the_shared_change_rows(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .current_dir(&root)
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "core.hooksPath=disabled-hooks",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-q", "-b", "master"]);
        std::fs::write(root.join("main.rs"), "fn first() {}\n").unwrap();
        git(&["add", "main.rs"]);
        git(&["commit", "-qm", "First"]);
        std::fs::write(root.join("main.rs"), "fn changed() {}\n").unwrap();
        std::fs::write(root.join("new.rs"), "fn added() {}\n").unwrap();
        // Rows carry the canonical path Git reported, which differs inside /var on macOS.
        let canonical = root.canonicalize().unwrap();
        let changed = canonical.join("main.rs").to_string_lossy().into_owned();
        let added = canonical.join("new.rs").to_string_lossy().into_owned();

        let (files, window) = cx.add_window_view(|_, cx| ProjectFiles::new(cx));
        window.simulate_resize(gpui::size(px(900.), px(720.)));
        files.update(window, |files, cx| files.set_root(root.clone(), cx));
        let click = |selector: &'static str, window: &mut gpui::VisualTestContext| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                window.run_until_parked();
                if let Some(bounds) = window.debug_bounds(selector) {
                    window.simulate_click(bounds.center(), Default::default());
                    return;
                }
                assert!(std::time::Instant::now() < deadline, "missing {selector}");
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        click("project-git-changes-segment", window);
        // Both layers come from the shared component, not a bespoke list.
        click(
            Box::leak(format!("change-file:{changed}:false").into_boxed_str()),
            window,
        );
        window.run_until_parked();
        assert!(
            window
                .debug_bounds(Box::leak(
                    format!("change-file:{added}:false").into_boxed_str()
                ))
                .is_some()
        );
        assert!(window.debug_bounds("stage-group:false").is_some());
        // Counts are read for the whole list, so a text file is never labelled binary.
        assert!(window.debug_bounds("change-file-binary").is_none());
        // The panel owns the commit box, like the diff sidebar footer.
        assert!(window.debug_bounds("project-commit-footer").is_some());
        assert!(window.debug_bounds("project-commit-submit").is_some());

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            window.run_until_parked();
            let opened = files.read_with(window, |files, _| {
                files.documents.iter().any(|document| {
                    matches!(&document.view, DocumentView::Diff(diff) if diff.path == changed)
                })
            });
            if opened {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "worktree diff failed to open"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[gpui::test]
    fn project_changes_opens_and_resolves_one_conflict(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        let repo = crate::merge_conflicts::tests::vendor_update(1, 2);
        let root = repo.path().canonicalize().unwrap();
        let path = root.join("冲突 [1].txt");
        let path = path.to_string_lossy();
        let git = workspace_diff::tests::git_ok;
        git(&root, &["add", "vendor/tracked/0.go"]);

        let (files, window) = cx.add_window_view(|_, cx| ProjectFiles::new(cx));
        window.simulate_resize(gpui::size(px(1100.), px(900.)));
        files.update(window, |files, cx| {
            files.set_root(root.clone(), cx);
            files.show_changes(cx);
        });
        let wait_for = |window: &mut gpui::VisualTestContext, selector: &'static str| {
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            loop {
                window.run_until_parked();
                if let Some(bounds) = window.debug_bounds(selector) {
                    return bounds.center();
                }
                assert!(std::time::Instant::now() < deadline, "missing {selector}");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let selector = Box::leak(format!("change-file:{path}:false").into_boxed_str());
        let row = wait_for(window, selector);
        let checkbox = Box::leak(format!("stage-file:{path}:false").into_boxed_str());
        assert!(window.debug_bounds(checkbox).is_none());
        files.update(window, |files, cx| {
            files
                .commit_entry
                .as_ref()
                .unwrap()
                .update(cx, |entry, cx| {
                    entry.set_value("Resolve merge", cx);
                });
            assert!(!files.can_commit_changes(cx));
        });
        window.simulate_click(row, Default::default());
        let theirs = wait_for(window, "merge-choice:0:1");
        window.simulate_click(theirs, Default::default());
        let save = wait_for(window, "save-merge-conflict");
        window.simulate_click(save, Default::default());

        wait_for(
            window,
            Box::leak(format!("change-file:{path}:true").into_boxed_str()),
        );
        assert!(window.debug_bounds(selector).is_none());
        assert_eq!(
            std::fs::read_to_string(root.join("冲突 [1].txt")).unwrap(),
            "theirs\n"
        );
        let inventory = workspace_diff::inventory(&root).unwrap();
        assert_eq!(inventory.conflict_labels, ["vendor/group_1/conflict-1.go"]);
    }

    #[gpui::test]
    fn project_changes_tab_commits_the_index(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let git = |args: &[&str]| -> String {
            let output = std::process::Command::new("git")
                .current_dir(&root)
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "core.hooksPath=disabled-hooks",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        git(&["init", "-q", "-b", "master"]);
        git(&["config", "user.name", "Test"]);
        git(&["config", "user.email", "test@example.invalid"]);
        std::fs::write(root.join("main.rs"), "fn first() {}\n").unwrap();
        git(&["add", "main.rs"]);
        git(&["commit", "-qm", "First"]);
        std::fs::write(root.join("new.rs"), "fn added() {}\n").unwrap();
        git(&["add", "new.rs"]);

        let (files, window) = cx.add_window_view(|_, cx| ProjectFiles::new(cx));
        window.simulate_resize(gpui::size(px(900.), px(720.)));
        files.update(window, |files, cx| files.set_root(root.clone(), cx));
        let click = |selector: &'static str, window: &mut gpui::VisualTestContext| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                window.run_until_parked();
                if let Some(bounds) = window.debug_bounds(selector) {
                    window.simulate_click(bounds.center(), Default::default());
                    return;
                }
                assert!(std::time::Instant::now() < deadline, "missing {selector}");
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        click("project-git-changes-segment", window);
        // Rows carry the canonical absolute path, so wait for the staged checkbox.
        let staged = Box::leak(
            format!(
                "stage-file:{}:true",
                root.canonicalize()
                    .unwrap()
                    .join("new.rs")
                    .to_string_lossy()
            )
            .into_boxed_str(),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            window.run_until_parked();
            if window.debug_bounds(staged).is_some() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "staged change row never rendered"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        files.update(window, |files, cx| {
            let entry = files.commit_entry.clone().expect("commit editor");
            // A staged file alone is not enough; the message gates the button.
            assert!(!files.can_commit_changes(cx));
            entry.update(cx, |entry, cx| entry.set_value("feat: project commit", cx));
            assert!(files.can_commit_changes(cx));
            cx.notify();
        });
        window.run_until_parked();
        click("project-commit-submit", window);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            window.run_until_parked();
            if git(&["log", "--format=%s", "-1"]) == "feat: project commit" {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "project commit never landed"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        files.read_with(window, |files, cx| {
            // The draft clears and the index is empty again.
            let entry = files.commit_entry.clone().unwrap();
            assert_eq!(entry.read(cx).value(), "");
            assert!(!files.can_commit_changes(cx));
        });
    }

    #[test]
    fn change_rows_tell_binary_files_apart_from_loaded_counts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .current_dir(&root)
                .args([
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "core.hooksPath=disabled-hooks",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "-q", "-b", "master"]);
        std::fs::write(root.join("main.rs"), "fn first() {}\n").unwrap();
        git(&["add", "main.rs"]);
        git(&["commit", "-qm", "First"]);
        std::fs::write(root.join("main.rs"), "fn changed() {}\n").unwrap();
        std::fs::write(root.join("logo.png"), [0u8, 1, 2, 3]).unwrap();
        git(&["add", "logo.png"]);

        let rows = ChangeSnapshot::read(&root).unwrap().rows();
        let find = |label: &str| {
            rows.iter().find_map(|row| match row {
                gpui_glint::components::ChangeRow::File {
                    label: row_label,
                    additions,
                    deletions,
                    binary,
                    ..
                } if row_label == label => Some((*additions, *deletions, *binary)),
                _ => None,
            })
        };
        // A modified text file reports real counts and no binary flag.
        assert_eq!(find("main.rs"), Some((Some(1), Some(1), false)));
        // Git reports binary content as "-\t-\t", so the row is flagged instead.
        assert_eq!(find("logo.png"), Some((None, None, true)));
    }
}
