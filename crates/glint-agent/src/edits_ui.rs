use super::*;
use crate::edits::Change;
use gpui::{Animation, AnimationExt, Priority};
use gpui_glint::components::ChangeNavigationEvent;
use gpui_glint::components::{
    ChainScroll, ChangeNavigation, ChangeRow, ChangeStatus, CodeCard, CodeCardEvent, Dialog,
    DiffCard, DiffCardEvent, DiffColors, MenuItem, PromptEditor, Scrollbar, ScrollbarState,
    Skeleton, Tooltip,
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};
use tokio::sync::oneshot;

type ListLink = Rc<RefCell<Option<(gpui::ListState, usize)>>>;

const DIFF_PREFETCH_DELAY: Duration = Duration::from_millis(150);
const DIFF_PREFETCH_CAPACITY: usize = 24;
const DIFF_ON_DEMAND_DIRECTORIES: &[&str] = &[
    "vendor",
    "node_modules",
    "third_party",
    "target",
    "dist",
    "build",
    "generated",
    ".next",
];

fn diff_on_demand(label: &str) -> bool {
    let Some((directory, _)) = label.rsplit_once('/') else {
        return false;
    };
    directory.split('/').any(|part| {
        DIFF_ON_DEMAND_DIRECTORIES
            .iter()
            .any(|directory| part.eq_ignore_ascii_case(directory))
    })
}

type DiffKey = (Uuid, bool, bool);
struct DiffPreview {
    change: Change,
    prepared: gpui_glint::components::PreparedDiff,
    cacheable: bool,
}
type DiffLoadResult = Result<DiffPreview, crate::workspace_diff::LoadError>;

fn prepare_workspace_diff(
    source: &crate::workspace_diff::Source,
    manual: bool,
    side_by_side: bool,
) -> DiffLoadResult {
    let mut change = crate::workspace_diff::load_with_options(source, manual)?;
    let lines = change.take_lines();
    let cacheable = lines.len() <= 4_000
        && change.before.as_deref().map_or(0, str::len) + change.after.len() <= 256 * 1024;
    let prepared = DiffCard::prepare(lines, side_by_side);
    Ok(DiffPreview {
        change,
        prepared,
        cacheable,
    })
}

#[derive(Default)]
struct DiffPrefetch {
    files: Rc<Vec<Change>>,
    side_by_side: bool,
    next: usize,
    attempted: HashSet<DiffKey>,
    ready: HashMap<DiffKey, DiffPreview>,
    // A click on this same version can take over the result of the current read.
    active: Option<(DiffKey, Option<oneshot::Sender<DiffLoadResult>>)>,
}

impl DiffPrefetch {
    fn update_inventory(&mut self, files: &Rc<Vec<Change>>, side_by_side: bool) {
        if self.side_by_side == side_by_side && Rc::ptr_eq(&self.files, files) {
            return;
        }
        let ids: HashSet<_> = files.iter().map(|file| file.id).collect();
        let valid = |(id, _, layout): &DiffKey| *layout == side_by_side && ids.contains(id);
        self.ready.retain(|key, _| valid(key));
        self.attempted.retain(valid);
        self.files = files.clone();
        self.side_by_side = side_by_side;
        self.next = 0;
    }

    fn has_pending(&self) -> bool {
        self.next < self.files.len() * 2 && self.ready.len() < DIFF_PREFETCH_CAPACITY
    }

    fn next_source(
        &mut self,
        sources: &HashMap<PathBuf, crate::workspace_diff::Source>,
    ) -> Option<(DiffKey, crate::workspace_diff::Source)> {
        while self.has_pending() {
            let file = &self.files[self.next / 2];
            let staged = self.next % 2 == 1;
            self.next += 1;
            let Some(source) = sources.get(&file.path) else {
                continue;
            };
            let changed = if staged {
                source.staged
            } else {
                source.unstaged
            };
            let key = (file.id, staged, self.side_by_side);
            if changed && !diff_on_demand(&source.label) && self.attempted.insert(key) {
                self.active = Some((key, None));
                return Some((key, source.for_layer(staged)));
            }
        }
        None
    }

    fn finish(&mut self, key: DiffKey, result: DiffLoadResult) {
        let Some((_, waiter)) = self.active.take() else {
            return;
        };
        if let Some(waiter) = waiter {
            let _ = waiter.send(result);
        } else if self.attempted.contains(&key)
            && let Ok(preview) = result
            && preview.cacheable
        {
            self.ready.insert(key, preview);
        }
    }
}

fn change_file_label(label: &str, tree: bool) -> String {
    if tree {
        label.rsplit('/').next().unwrap_or(label).to_owned()
    } else {
        label.to_owned()
    }
}

fn change_directory(row: &crate::workspace_tree::Row, staged: Option<bool>) -> ChangeRow {
    let crate::workspace_tree::Row::Directory {
        path,
        label,
        depth,
        count,
        collapsed,
    } = row
    else {
        unreachable!("expected a change directory")
    };
    ChangeRow::Directory {
        path: path.clone(),
        label: label.clone(),
        depth: *depth,
        count: *count,
        collapsed: *collapsed,
        staged,
    }
}

fn file_status_style(
    status: crate::workspace_diff::FileStatus,
    theme: ThemePreset,
) -> (&'static str, u32) {
    use crate::workspace_diff::FileStatus;
    let dark = theme.is_dark();
    match status {
        FileStatus::Added => ("新增", if dark { 0x4bc26b } else { 0x257235 }),
        FileStatus::Modified => ("修改", if dark { 0xe5c07b } else { 0x966300 }),
        FileStatus::Deleted => ("删除", if dark { 0xff7b72 } else { 0xb42318 }),
        FileStatus::Renamed => ("重命名", if dark { 0x79b8ff } else { 0x0969da }),
        FileStatus::Untracked => ("未跟踪", if dark { 0xa8b0bd } else { 0x6e7781 }),
    }
}

fn conflict_status_style(
    kind: crate::merge_conflicts::ConflictKind,
    theme: ThemePreset,
) -> (&'static str, u32) {
    use crate::merge_conflicts::ConflictKind;
    let dark = theme.is_dark();
    match kind {
        ConflictKind::BothModified => ("双方修改", if dark { 0xfb923c } else { 0xb45309 }),
        ConflictKind::BothAdded => ("双方新增", if dark { 0xc4b5fd } else { 0x7c3aed }),
        ConflictKind::DeletedByUs => ("当前侧删除", if dark { 0xff7b72 } else { 0xb42318 }),
        ConflictKind::DeletedByThem => ("传入侧删除", if dark { 0xfda4af } else { 0xbe185d }),
        ConflictKind::BothDeleted => ("双方删除", if dark { 0xf87171 } else { 0x991b1b }),
        ConflictKind::AddedByUs => ("当前侧新增", if dark { 0x4bc26b } else { 0x257235 }),
        ConflictKind::AddedByThem => ("传入侧新增", if dark { 0x5eead4 } else { 0x0f766e }),
        ConflictKind::Other => ("冲突", if dark { 0xfb923c } else { 0xb45309 }),
    }
}
#[derive(Clone)]
enum GitRow {
    Group(bool, usize),
    Directory(bool, crate::workspace_tree::Row),
    File(bool, usize, usize),
}

enum StageTarget {
    Files(Vec<crate::workspace_diff::Source>),
    Group(bool),
    Directory(String, bool),
}

enum FileAction {
    DeleteUntracked(crate::workspace_diff::Source),
    DiscardTracked(crate::workspace_diff::Source),
}
impl FileAction {
    fn source(&self) -> &crate::workspace_diff::Source {
        match self {
            Self::DeleteUntracked(source) | Self::DiscardTracked(source) => source,
        }
    }
    fn label(&self) -> &'static str {
        match self {
            Self::DeleteUntracked(_) => "删除文件",
            Self::DiscardTracked(_) => "撤销更改",
        }
    }
}

struct StageRequest {
    sources: Vec<crate::workspace_diff::Source>,
    staged: bool,
    project: Option<PathBuf>,
}

impl GitRow {
    fn key(&self, files: &[Change]) -> String {
        match self {
            Self::Group(staged, _) => format!("group:{staged}"),
            Self::Directory(staged, crate::workspace_tree::Row::Directory { path, .. }) => {
                format!("dir:{staged}:{path}")
            }
            Self::File(staged, index, _) => {
                format!("file:{staged}:{}", files[*index].path.display())
            }
            _ => unreachable!(),
        }
    }
}

struct VirtualFiles {
    files: Rc<Vec<Change>>,
    rows: Option<Rc<Vec<crate::workspace_tree::Row>>>,
    scope: String,
    state: gpui::ListState,
    expanded: bool,
    measured_width: gpui::Pixels,
    prefix: usize,
    conflict_rows: Rc<Vec<crate::workspace_tree::Row>>,
}
fn watch_row<V: 'static>(view: &Entity<V>, link: ListLink, cx: &mut Context<AgentWorkspace>) {
    cx.observe(view, move |_, _, cx| {
        if let Some((list, index)) = &*link.borrow() {
            if *index < list.item_count() {
                list.remeasure_items(*index..*index + 1);
            }
            cx.notify();
        }
    })
    .detach();
}

#[derive(Default)]
pub struct State {
    pub open: bool,
    pub(super) expanded: bool,
    pub selected: Option<PathBuf>,
    pub undo_task: Option<Task<()>>,
    undo_notice: Option<(Uuid, String)>,
    aggregates: RefCell<HashMap<String, (Vec<Uuid>, Rc<Vec<Change>>)>>,
    summaries: RefCell<HashMap<String, (Vec<Uuid>, Entity<Summary>)>>,
    diffs: RefCell<HashMap<String, (Uuid, Entity<DiffCard>)>>,
    pub(super) workspace_mode: bool,
    selected_staged: bool,
    stage_task: Option<Task<()>>,
    commit_task: Option<Task<()>>,
    commit_drafts: RefCell<HashMap<PathBuf, Entity<PromptEditor>>>,
    commit_notice: Option<(PathBuf, String)>,
    /// One in-flight commit-message request per repository.
    pub(super) commit_generation: HashMap<PathBuf, Task<()>>,
    /// Whether the footer's model menu is open.
    pub(super) commit_model_open: bool,
    stage_queue: std::collections::VecDeque<StageRequest>,
    stage_pending: std::collections::HashSet<PathBuf>,
    stage_refresh_pending: bool,
    grouped_rows: RefCell<Option<(Rc<Vec<Change>>, bool, Rc<Vec<GitRow>>)>>,
    grouped_list: RefCell<
        Option<(
            Rc<Vec<GitRow>>,
            Vec<gpui::EntityId>,
            gpui::ListState,
            Rc<Vec<Change>>,
        )>,
    >,
    grouped_width: Cell<gpui::Pixels>,
    pub(super) workspace_root: Option<PathBuf>,
    workspace_files: Rc<Vec<Change>>,
    workspace_sources: Rc<HashMap<PathBuf, crate::workspace_diff::Source>>,
    virtual_files: RefCell<Option<VirtualFiles>>,
    tree_cache: RefCell<Option<(String, Rc<Vec<Change>>, Rc<Vec<crate::workspace_tree::Row>>)>>,
    collapsed_directories: HashMap<String, std::collections::HashSet<String>>,
    workspace_notes: Vec<String>,
    workspace_error: Option<String>,
    workspace_task: Option<Task<()>>,
    workspace_background_refresh: bool,
    workspace_resync_pending: bool,
    workspace_watch_root: Option<PathBuf>,
    workspace_stamp: Option<crate::workspace_diff::RepositoryStamp>,
    workspace_conflict_versions: Vec<(Vec<u8>, Option<crate::workspace_diff::FileStamp>)>,
    workspace_watch_task: Option<Task<()>>,
    workspace_probe_task: Option<Task<()>>,
    workspace_sync_debounce: Option<Task<()>>,
    workspace_conflicts_task: Option<Task<()>>,
    workspace_stats_task: Option<Task<()>>,
    diff_prefetch: Rc<RefCell<DiffPrefetch>>,
    diff_prefetch_task: Option<Task<()>>,
    workspace_revision: u64,
    conflicts: Vec<Entity<ConflictView>>,
    file_views: RefCell<HashMap<String, Entity<FileDiff>>>,
    sidebar_scroll: ScrollHandle,
    navigation_bar: ScrollbarState,
    workspace_branch: Option<String>,
    file_menu: Option<(PathBuf, String, gpui::WindowId, gpui::Point<gpui::Pixels>)>,
    file_menu_focus: Option<gpui::FocusHandle>,
    file_action: Option<FileAction>,
    file_action_focus: Vec<gpui::FocusHandle>,
    file_action_task: Option<Task<()>>,
    detail_scrolls: RefCell<HashMap<String, ScrollHandle>>,
    detail_scrollbars: RefCell<HashMap<String, ScrollbarState>>,
    navigation: RefCell<Option<(String, Entity<ChangeNavigation>)>>,
    pub(super) navigation_width: Option<f32>,
    pub(super) navigation_resize: Option<(gpui::Pixels, f32)>,
    pub(super) navigation_maximum: Cell<f32>,
    conflict_tree_cache: RefCell<
        Option<(
            String,
            Vec<gpui::EntityId>,
            Rc<Vec<crate::workspace_tree::Row>>,
        )>,
    >,
    reveal_revision: u64,
    reveal_pending: Cell<bool>,
}

impl AgentWorkspace {
    fn open_diff_file_menu(
        &mut self,
        path: PathBuf,
        label: String,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.edits_ui.file_menu_focus = Some(focus);
        self.edits_ui.file_menu = Some((path, label, window.window_handle().window_id(), position));
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn diff_file_overlays(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut overlays = div().absolute().inset_0().size_full();
        if let Some((path, label, owner, position)) = &self.edits_ui.file_menu
            && *owner == window.window_handle().window_id()
            && self.edits_ui.open
        {
            let p = self.theme.palette();
            let absolute = path.to_string_lossy().into_owned();
            let relative = label.clone();
            let source = self
                .edits_ui
                .workspace_sources
                .get(path)
                .filter(|_| {
                    self.edits_ui.workspace_mode
                        && self.edits_ui.workspace_root == self.diff_repository_root()
                })
                .cloned();
            let menu = div()
                .id("diff-file-context-menu")
                .track_focus(self.edits_ui.file_menu_focus.as_ref().unwrap())
                .w(px(200.))
                .p_1()
                .rounded(px(7.))
                .bg(rgb(p.surface))
                .border_1()
                .border_color(rgb(p.border))
                .shadow_lg()
                .occlude()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.edits_ui.file_menu = None;
                    cx.notify();
                }))
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                    if event.keystroke.key == "escape" {
                        this.edits_ui.file_menu = None;
                        cx.stop_propagation();
                        cx.notify();
                    }
                }))
                .child(
                    MenuItem::new("diff-copy-relative-path", "复制相对路径")
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                relative.clone(),
                            ));
                            this.edits_ui.file_menu = None;
                            cx.notify();
                        })),
                )
                .child(
                    MenuItem::new("diff-copy-absolute-path", "复制绝对路径")
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                absolute.clone(),
                            ));
                            this.edits_ui.file_menu = None;
                            cx.notify();
                        })),
                )
                .when_some(source, |menu, source| {
                    let untracked = source.status == crate::workspace_diff::FileStatus::Untracked;
                    menu.child(
                        MenuItem::new(
                            if untracked {
                                "diff-delete-untracked"
                            } else {
                                "diff-discard-tracked"
                            },
                            if untracked {
                                "删除文件…"
                            } else {
                                "撤销更改…"
                            },
                        )
                        .theme(self.theme)
                        .render()
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.edits_ui.file_menu = None;
                                this.edits_ui.file_action = Some(if untracked {
                                    FileAction::DeleteUntracked(source.clone())
                                } else {
                                    FileAction::DiscardTracked(source.clone())
                                });
                                this.edits_ui.file_action_focus =
                                    vec![cx.focus_handle(), cx.focus_handle()];
                                window.focus(&this.edits_ui.file_action_focus[0], cx);
                                cx.notify();
                            },
                        )),
                    )
                });
            overlays = overlays.child(
                gpui::deferred(
                    gpui::anchored()
                        .position(*position)
                        .snap_to_window_with_margin(px(8.))
                        .child(menu),
                )
                .with_priority(80),
            );
        }
        if let Some(action) = &self.edits_ui.file_action {
            let source = action.source();
            let body = div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("{}？", action.label())),
                )
                .child(div().text_size(px(12.)).child(source.label.clone()))
                .child(match action {
                    FileAction::DeleteUntracked(_) => "文件将从磁盘永久删除，无法通过 Git 恢复。",
                    FileAction::DiscardTracked(_) => "将同时撤销此文件的已暂存和未暂存更改，恢复到 HEAD。新增文件将被删除，重命名将恢复原路径。此操作无法撤回。",
                })
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("取消")
                                .kind(ButtonKind::Plain)
                                .theme(self.theme)
                                .render()
                                .track_focus(&self.edits_ui.file_action_focus[0])
                                .on_key_down(cx.listener(
                                    |this, event: &gpui::KeyDownEvent, window, cx| {
                                        if event.keystroke.key == "enter" {
                                            this.edits_ui.file_action = None;
                                            window.focus(&this.composer.focus_handle(cx), cx);
                                            cx.stop_propagation();
                                            cx.notify();
                                        }
                                    },
                                ))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.edits_ui.file_action = None;
                                    window.focus(&this.composer.focus_handle(cx), cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new(action.label())
                                .theme(self.theme)
                                .render()
                                .debug_selector(|| "confirm-diff-file-action".into())
                                .track_focus(&self.edits_ui.file_action_focus[1])
                                .on_key_down(cx.listener(
                                    |this, event: &gpui::KeyDownEvent, window, cx| {
                                        if event.keystroke.key == "enter" {
                                            this.confirm_diff_file_action(cx);
                                            window.focus(&this.composer.focus_handle(cx), cx);
                                            cx.stop_propagation();
                                        }
                                    },
                                ))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_diff_file_action(cx);
                                    window.focus(&this.composer.focus_handle(cx), cx);
                                })),
                        ),
                );
            let weak = cx.entity().downgrade();
            overlays = overlays.child(
                Dialog::new("diff-file-action-dialog", body)
                    .width(460.)
                    .theme(self.theme)
                    .focus_handles(self.edits_ui.file_action_focus.clone())
                    .on_close(move |_, window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.edits_ui.file_action = None;
                            window.focus(&this.composer.focus_handle(cx), cx);
                            cx.notify();
                        });
                    })
                    .render(),
            );
        }
        overlays.into_any_element()
    }

    fn confirm_diff_file_action(&mut self, cx: &mut Context<Self>) {
        let Some(action) = self.edits_ui.file_action.take() else {
            return;
        };
        if self.edits_ui.file_action_task.is_some()
            || self.repository_change_busy(cx)
            || self.edits_ui.workspace_root != self.diff_repository_root()
        {
            self.edits_ui.workspace_error = Some("仓库状态正在变化，请刷新后重试。".into());
            cx.notify();
            return;
        }
        let root = self.diff_repository_root();
        let label = action.label();
        let work = cx.background_executor().spawn(async move {
            match action {
                FileAction::DeleteUntracked(source) => {
                    crate::workspace_diff::delete_untracked(&source)
                }
                FileAction::DiscardTracked(source) => {
                    crate::workspace_diff::discard_tracked(&source)
                }
            }
        });
        self.edits_ui.file_action_task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                this.edits_ui.file_action_task = None;
                if this.diff_repository_root() == root {
                    match result {
                        Ok(()) => this.refresh_workspace_edits(cx),
                        Err(error) => {
                            this.edits_ui.workspace_error = Some(format!("{label}失败：{error}"))
                        }
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn conflict_rows(
        &self,
        conflicts: &[Entity<ConflictView>],
        scope: &str,
        cx: &App,
    ) -> Rc<Vec<crate::workspace_tree::Row>> {
        if self.history.workspace_display != crate::workspace_tree::DisplayMode::Tree {
            return Rc::new(
                (0..conflicts.len())
                    .map(|index| crate::workspace_tree::Row::File { index, depth: 0 })
                    .collect(),
            );
        }
        let ids = conflicts
            .iter()
            .map(|view| view.entity_id())
            .collect::<Vec<_>>();
        let mut cache = self.edits_ui.conflict_tree_cache.borrow_mut();
        if cache
            .as_ref()
            .is_none_or(|(key, cached, _)| key != scope || *cached != ids)
        {
            let files = conflicts
                .iter()
                .map(|view| {
                    let file = &view.read(cx).file;
                    Change::preview(
                        file.path.clone(),
                        file.label.clone(),
                        None,
                        "".into(),
                        Some((0, 0)),
                    )
                })
                .collect::<Vec<_>>();
            let empty = Default::default();
            let collapsed = self
                .edits_ui
                .collapsed_directories
                .get(scope)
                .unwrap_or(&empty);
            *cache = Some((
                scope.into(),
                ids,
                Rc::new(crate::workspace_tree::rows(&files, collapsed)),
            ));
        }
        cache.as_ref().unwrap().2.clone()
    }

    fn conflict_directory_row(
        &self,
        row: &crate::workspace_tree::Row,
        scope: &str,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let crate::workspace_tree::Row::Directory {
            path,
            label,
            depth,
            count,
            collapsed,
        } = row
        else {
            unreachable!()
        };
        let path = path.clone();
        let scope = scope.to_owned();
        ChangeNavigation::directory_element(
            format!("conflict-directory:{path}"),
            label.clone(),
            *depth,
            *count,
            *collapsed,
            self.theme,
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            let collapsed = this
                .edits_ui
                .collapsed_directories
                .entry(scope.clone())
                .or_default();
            if !collapsed.remove(&path) {
                collapsed.insert(path.clone());
            }
            this.edits_ui.conflict_tree_cache.borrow_mut().take();
            this.edits_ui.grouped_list.borrow_mut().take();
            cx.notify();
        }))
        .into_any_element()
    }

    pub(super) fn open_workspace_diff(&mut self, cx: &mut Context<Self>) {
        if self.history.active().project.is_none() {
            return;
        }
        self.page = crate::Page::Chat;
        self.git_ui.open = false;
        if !self.terminal_ui.bottom {
            self.terminal_ui.open = false;
        }
        self.open_edits(None, cx);
        self.edits_ui.workspace_mode = true;
        self.right_sidebar_width = self.right_sidebar_width.max(400.);
        self.refresh_workspace_edits(cx);
    }
    fn undo_session_edits(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.any_run_busy()
            || !self.queued_requests.is_empty()
            || self.edits_ui.undo_task.is_some()
        {
            return;
        }
        let files: Vec<_> = self
            .session_edits()
            .iter()
            .filter(|file| path.as_ref().is_none_or(|p| p == &file.path))
            .cloned()
            .collect();
        if files.is_empty() {
            return;
        }
        let session = self.history.active;
        let root = self.history.active().project.clone();
        let paths = files
            .iter()
            .map(|c| c.path.clone())
            .collect::<std::collections::HashSet<_>>();
        let records: Vec<_> = self
            .history
            .active()
            .messages
            .iter()
            .flat_map(|m| m.tools.iter())
            .collect();
        if let Err(error) = edits::verify_reversal_chain(
            records.clone(),
            &paths,
            &self.history.active().reverted_changes,
        ) {
            self.edits_ui.undo_notice = Some((session, error));
            cx.notify();
            return;
        }
        let by_path = edits::ids_by_path(records, &paths);
        self.edits_ui.undo_notice = Some((session, "正在撤销文件改动…".into()));
        self.composer.update(cx, |c, cx| c.set_disabled(true, cx));
        self.edits_ui.undo_task = Some(cx.spawn(async move |weak, cx| {
            let work_root = root.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    if work_root.as_deref().is_some_and(ssh_project::is_remote) {
                        provider::Job::start(async move {
                            Ok(crate::undo::apply(files, work_root).await)
                        })
                        .map_err(|e| e.to_string())?
                        .result()
                        .await
                    } else {
                        Ok(crate::undo::apply(files, work_root).await)
                    }
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.edits_ui.undo_task.take() {
                    task.detach();
                }
                let outcome = result.unwrap_or_else(|error| crate::undo::Outcome {
                    error: Some(error),
                    ..Default::default()
                });
                if let Some(session) = this.history.sessions.iter_mut().find(|s| s.id == session) {
                    for path in &outcome.restored {
                        if let Some(ids) = by_path.get(path) {
                            session.reverted_changes.extend(ids);
                        }
                    }
                    session.updated = chrono::Local::now();
                }
                let message = match outcome.error {
                    Some(error) => format!("已撤销 {} 个文件；{error}", outcome.restored.len()),
                    None => format!("已撤销 {} 个文件的会话改动", outcome.restored.len()),
                };
                this.edits_ui.undo_notice = Some((session, message));
                this.edits_ui.aggregates.borrow_mut().clear();
                this.reload_preview_after_undo(&outcome.restored, cx);
                if this.edits_ui.open
                    && this.edits_ui.workspace_mode
                    && this.history.active().project == root
                {
                    this.refresh_workspace_edits(cx);
                }
                this.sync_skill_context(cx);
                this.save(cx);
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn cached_edits(&self, key: &str, records: Vec<&tools::ToolRecord>) -> Rc<Vec<Change>> {
        let excluded = &self.history.active().reverted_changes;
        let ids = edits::ids(&records)
            .into_iter()
            .filter(|id| !excluded.contains(id))
            .collect::<Vec<_>>();
        let mut cache = self.edits_ui.aggregates.borrow_mut();
        let entry = cache
            .entry(key.into())
            .or_insert_with(|| (Vec::new(), Rc::new(Vec::new())));
        if entry.0 != ids {
            *entry = (
                ids,
                Rc::new(edits::aggregate_records_excluding(records, excluded)),
            );
        }
        entry.1.clone()
    }

    fn session_edits(&self) -> Rc<Vec<Change>> {
        let mut records: Vec<_> = self
            .history
            .active()
            .messages
            .iter()
            .flat_map(|message| message.tools.iter())
            .collect();
        let running = self.session_busy(self.history.active);
        if running {
            records.extend(self.active_run().tool_activity.iter());
        }
        let live = self.active_run().subagents.lock().ok();
        if running {
            if let Some(live) = &live {
                for trace in live.iter() {
                    records.extend(trace.tools.iter());
                }
            }
        }
        self.filter_repository_edits(
            self.cached_edits(&format!("session:{}", self.history.active), records),
        )
    }

    pub fn open_edits(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if let Some(path) = &path {
            self.adopt_panel_repository(
                crate::panel_repositories::Panel::Diff,
                &path.to_string_lossy(),
                cx,
            );
        }
        self.file_preview.open = false;
        if !self.terminal_ui.bottom {
            self.terminal_ui.open = false;
        }
        self.edits_ui.open = true;
        self.edits_ui.workspace_mode = false;
        self.edits_ui.selected = path;
        self.edits_ui.reveal_revision += 1;
        self.edits_ui.reveal_pending.set(true);
        cx.notify();
    }

    pub fn edit_diff(
        &self,
        scope: &str,
        change: &Change,
        cx: &mut Context<Self>,
    ) -> Entity<DiffCard> {
        let key = format!("{scope}:{}", change.path.display());
        let mut cache = self.edits_ui.diffs.borrow_mut();
        let entry = cache.entry(key).or_insert_with(|| {
            let card = cx.new(|cx| {
                DiffCard::from_lines(cx, edits::language(&change.path), change.lines())
                    .name(change.label.clone())
                    .title_clickable(true)
                    .theme(self.theme)
            });
            let path = change.path.clone();
            cx.subscribe(&card, move |this, _, _: &DiffCardEvent, cx| {
                this.open_edits(Some(path.clone()), cx)
            })
            .detach();
            (change.id, card)
        });
        if entry.0 != change.id {
            entry
                .1
                .update(cx, |card, cx| card.set_lines(change.lines(), cx));
            entry.0 = change.id;
        }
        let card = &entry.1;
        card.update(cx, |card, cx| card.set_theme(self.theme, cx));
        card.clone()
    }

    pub fn edits_summary(
        &self,
        key: &str,
        records: &[tools::ToolRecord],
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let files = self.cached_edits(key, records.iter().collect());
        if files.is_empty() {
            self.edits_ui.summaries.borrow_mut().remove(key);
            return div();
        }
        let ids = files.iter().map(|file| file.id).collect::<Vec<_>>();
        let mut cache = self.edits_ui.summaries.borrow_mut();
        let workspace = cx.entity().downgrade();
        let entry = cache.entry(key.into()).or_insert_with(|| {
            (
                Vec::new(),
                cx.new(|_| Summary {
                    files: Rc::new(Vec::new()),
                    workspace,
                    theme: self.theme,
                    expanded: false,
                }),
            )
        });
        if entry.0 != ids || entry.1.read(cx).theme != self.theme {
            entry.1.update(cx, |summary, cx| {
                summary.files = files;
                summary.theme = self.theme;
                cx.notify();
            });
            entry.0 = ids;
        }
        div().w_full().child(entry.1.clone())
    }

    pub fn edits_button(&self, cx: &mut Context<Self>) -> gpui::Div {
        let files = self.session_edits();
        let p = self.theme.palette();
        // Quiet unless there is something to look at; hover carries the accent.
        let foreground = if files.is_empty() {
            p.text_muted
        } else {
            p.text
        };
        div().flex_shrink_0().child(
            Button::new(format!("变更 {}", files.len()))
                .icon(Icon::Pencil)
                .kind(ButtonKind::Ghost)
                .size(ButtonSize::Compact)
                .foreground(foreground)
                .theme(self.theme)
                .render()
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.edits_ui.open {
                        this.edits_ui.open = false;
                        cx.notify();
                    } else {
                        this.open_edits(None, cx);
                    }
                })),
        )
    }

    fn file_row(
        &self,
        file: &Change,
        scope: &str,
        link: Option<(gpui::ListState, usize)>,
        tree_depth: Option<usize>,
        can_undo: bool,
        reveal: bool,
        cx: &mut Context<Self>,
    ) -> Entity<FileDiff> {
        let key = format!("{scope}:{}", file.path.display());
        let mut views = self.edits_ui.file_views.borrow_mut();
        let view = views
            .entry(key)
            .or_insert_with(|| {
                let view = cx.new(|_| FileDiff::new(file.clone(), self.theme));
                watch_row(&view, view.read(cx).list_link.clone(), cx);
                view
            })
            .clone();
        drop(views);
        let workspace = cx.entity().downgrade();
        view.update(cx, |view, cx| {
            *view.list_link.borrow_mut() = link;
            let mut display_file = file.clone();
            if let Some(source) = self.edits_ui.workspace_sources.get(&file.path) {
                let stats = if scope.ends_with(":staged") {
                    source.staged_stats
                } else if scope.ends_with(":unstaged") {
                    source.unstaged_stats
                } else {
                    source.stats
                };
                if let Some((a, r)) = stats {
                    display_file.added = a;
                    display_file.removed = r;
                }
            }
            view.set_file(&display_file, self.theme, cx);
            if view.detail != self.edits_ui.expanded {
                view.detail = self.edits_ui.expanded;
                cx.notify();
            }
            if view.tree_depth != tree_depth {
                view.tree_depth = tree_depth;
                cx.notify();
            }
            view.source = if self.edits_ui.workspace_mode {
                self.edits_ui
                    .workspace_sources
                    .get(&file.path)
                    .map(|source| {
                        if scope.ends_with(":staged") {
                            source.for_layer(true)
                        } else if scope.ends_with(":unstaged") {
                            source.for_layer(false)
                        } else {
                            source.clone()
                        }
                    })
            } else {
                None
            };
            let status = view.source.as_ref().map(|source| source.status);
            if view.status != status {
                view.status = status;
                cx.notify();
            }
            if view.side_by_side != self.edits_ui.expanded {
                view.side_by_side = self.edits_ui.expanded;
                if let Some(card) = &view.card {
                    card.update(cx, |card, cx| card.set_side_by_side(view.side_by_side, cx));
                }
                cx.notify();
            }
            view.workspace = Some(workspace);
            view.prefetch = view
                .source
                .as_ref()
                .filter(|source| source.layer.is_some() && !diff_on_demand(&source.label))
                .map(|_| self.edits_ui.diff_prefetch.clone());
            if view.undo_visible != !self.edits_ui.workspace_mode || view.undo_enabled != can_undo {
                view.undo_visible = !self.edits_ui.workspace_mode;
                view.undo_enabled = can_undo;
                cx.notify();
            }
            if reveal {
                view.reveal(self.edits_ui.reveal_revision, cx);
            }
        });
        view
    }

    fn virtual_workspace_files(
        &self,
        files: Rc<Vec<Change>>,
        scope: String,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let rows = if self.history.workspace_display == crate::workspace_tree::DisplayMode::Tree {
            let mut cache = self.edits_ui.tree_cache.borrow_mut();
            if cache
                .as_ref()
                .is_none_or(|(cached_scope, cached_files, _)| {
                    cached_scope != &scope || !Rc::ptr_eq(cached_files, &files)
                })
            {
                let empty = Default::default();
                let collapsed = self
                    .edits_ui
                    .collapsed_directories
                    .get(&scope)
                    .unwrap_or(&empty);
                *cache = Some((
                    scope.clone(),
                    files.clone(),
                    Rc::new(crate::workspace_tree::rows(&files, collapsed)),
                ));
            }
            Some(cache.as_ref().unwrap().2.clone())
        } else {
            None
        };
        let row_count = rows.as_ref().map_or(files.len(), |rows| rows.len());
        let conflicts = self.edits_ui.conflicts.clone();
        let conflict_scope = format!("{scope}:conflicts");
        let conflict_rows = self.conflict_rows(&conflicts, &conflict_scope, cx);
        let notes = self.edits_ui.workspace_notes.clone();
        let prefix = if conflicts.is_empty() {
            0
        } else {
            conflict_rows.len() + 1
        };
        let count = prefix + row_count + notes.len();
        let mut cached = self.edits_ui.virtual_files.borrow_mut();
        let same_rows = cached.as_ref().is_some_and(|cache| cache.rows == rows);
        if let Some(cache) = cached.as_mut() {
            if cache.scope == scope && cache.conflict_rows != conflict_rows {
                cache.state.splice(0..cache.prefix, prefix);
                cache.prefix = prefix;
                cache.conflict_rows = conflict_rows.clone();
            }
            if cache.scope == scope
                && !same_rows
                && cache.files.len() == files.len()
                && cache
                    .files
                    .iter()
                    .zip(files.iter())
                    .all(|(a, b)| a.id == b.id)
            {
                if let (Some(old), Some(new)) = (&cache.rows, &rows) {
                    if cache.state.item_count() == prefix + old.len() + notes.len() {
                        let common = old
                            .iter()
                            .zip(new.iter())
                            .take_while(|(a, b)| a == b)
                            .count();
                        let suffix = old[common..]
                            .iter()
                            .rev()
                            .zip(new[common..].iter().rev())
                            .take_while(|(a, b)| a == b)
                            .count();
                        cache.state.splice(
                            prefix + common..prefix + old.len() - suffix,
                            new.len() - common - suffix,
                        );
                        cache.state.clone().with_uniform_item_height(px(49.));
                        cache.rows = rows.clone();
                        cache.files = files.clone();
                    }
                }
            }
            if cache.scope == scope
                && same_rows
                && !Rc::ptr_eq(&cache.files, &files)
                && cache.state.item_count() == count
                && cache.files.len() == files.len()
                && cache
                    .files
                    .iter()
                    .zip(files.iter())
                    .all(|(a, b)| a.id == b.id)
            {
                // Background statistics must not reset scroll position or disclosures.
                cache.files = files.clone();
            }
        }
        if cached.as_ref().is_none_or(|cache| {
            cache.rows != rows || cache.scope != scope || !Rc::ptr_eq(&cache.files, &files)
        }) {
            let state = gpui::ListState::new(count, gpui::ListAlignment::Top, px(96.));
            state.reset_with_uniform_height(count, px(49.));
            *cached = Some(VirtualFiles {
                files: files.clone(),
                rows: rows.clone(),
                scope: scope.clone(),
                state,
                expanded: self.edits_ui.expanded,
                measured_width: px(0.),
                prefix,
                conflict_rows: conflict_rows.clone(),
            });
        }
        let cached = cached.as_mut().unwrap();
        if cached.expanded != self.edits_ui.expanded {
            cached.expanded = self.edits_ui.expanded;
            cached.state.remeasure();
        }
        let state = cached.state.clone();
        let link_state = state.clone();
        let weak = cx.entity().downgrade();
        let layout_weak = weak.clone();
        let list = gpui::list(state.clone(), move |index, _, cx| {
            weak.update(cx, |this, cx| {
                if prefix > 0 && index == 0 {
                    return div()
                        .px_3()
                        .py_2()
                        .flex()
                        .items_center()
                        .gap_1()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(Icon::GitConflict.render(13.))
                        .child(format!("待解决冲突 · {}", conflicts.len()))
                        .into_any_element();
                }
                if index < prefix {
                    let (file_index, depth) = match &conflict_rows[index - 1] {
                        crate::workspace_tree::Row::Directory { .. } => {
                            return this.conflict_directory_row(
                                &conflict_rows[index - 1],
                                &conflict_scope,
                                cx,
                            );
                        }
                        crate::workspace_tree::Row::File { index, depth } => (*index, *depth),
                    };
                    let view = &conflicts[file_index];
                    view.update(cx, |view, cx| {
                        let depth = (this.history.workspace_display
                            == crate::workspace_tree::DisplayMode::Tree)
                            .then_some(depth);
                        if view.tree_depth != depth {
                            view.tree_depth = depth;
                            cx.notify();
                        }
                        *view.list_link.borrow_mut() = Some((link_state.clone(), index));
                        if view.theme != this.theme {
                            view.theme = this.theme;
                            view.editor
                                .update(cx, |editor, cx| editor.set_theme(this.theme, cx));
                            cx.notify();
                        }
                    });
                    return view.clone().into_any_element();
                }
                let row_index = index - prefix;
                let (file_index, depth) = match rows.as_ref().and_then(|rows| rows.get(row_index)) {
                    Some(crate::workspace_tree::Row::Directory {
                        path,
                        label,
                        depth,
                        count,
                        collapsed,
                    }) => {
                        let path = path.clone();
                        let directory_scope = scope.clone();
                        let selector = format!("workspace-directory:{path}");
                        let p = this.theme.palette();
                        return div()
                            .id(SharedString::from(format!(
                                "workspace-directory:{scope}:{path}"
                            )))
                            .debug_selector(move || selector.clone())
                            .h(px(34.))
                            .pr_3()
                            .pl(px(12. + *depth as f32 * 16.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(p.surface_subtle)))
                            .child(
                                Icon::disclosure(13., !*collapsed)
                                    .flex_shrink_0()
                                    .text_color(rgb(p.text_muted)),
                            )
                            .child(
                                Icon::Folder
                                    .render(15.)
                                    .flex_shrink_0()
                                    .text_color(rgb(p.text_muted)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .child(label.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.text_muted))
                                    .child(count.to_string()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let collapsed = this
                                    .edits_ui
                                    .collapsed_directories
                                    .entry(directory_scope.clone())
                                    .or_default();
                                if !collapsed.remove(&path) {
                                    collapsed.insert(path.clone());
                                }
                                this.edits_ui.tree_cache.borrow_mut().take();
                                cx.notify();
                            }))
                            .into_any_element();
                    }
                    Some(crate::workspace_tree::Row::File { index, depth }) => {
                        (*index, Some(*depth))
                    }
                    None => (row_index, None),
                };
                if row_index < row_count {
                    if let Some(file) = files.get(file_index) {
                        return this
                            .file_row(
                                file,
                                &scope,
                                Some((link_state.clone(), index)),
                                depth,
                                false,
                                false,
                                cx,
                            )
                            .into_any_element();
                    }
                }
                div()
                    .px_3()
                    .text_size(px(11.))
                    .text_color(rgb(this.theme.palette().text_muted))
                    .child(
                        notes
                            .get(row_index.saturating_sub(row_count))
                            .cloned()
                            .unwrap_or_default(),
                    )
                    .into_any_element()
            })
            .unwrap_or_else(|_| div().into_any_element())
        })
        .flex_1()
        .min_h_0()
        .w_full();
        div()
            .flex_1()
            .min_h_0()
            .w_full()
            .flex()
            .flex_col()
            .on_children_prepainted(move |_, _, cx| {
                let _ = layout_weak.update(cx, |this, cx| {
                    let mut cache = this.edits_ui.virtual_files.borrow_mut();
                    let Some(cache) = cache.as_mut() else {
                        return;
                    };
                    let width = cache.state.viewport_bounds().size.width;
                    if width > px(0.) && cache.measured_width != width {
                        cache.measured_width = width;
                        // GPUI clears all off-screen height hints on the first paint
                        // and on width changes. Restore estimates after that pass,
                        // retaining actual heights for measured/open Diff rows.
                        cache.state.clone().with_uniform_item_height(px(49.));
                        let id = cx.entity_id();
                        cx.defer(move |cx| cx.notify(id));
                    }
                });
            })
            .child(list)
    }

    pub fn edits_sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let workspace_mode = self.edits_ui.workspace_mode;
        let root_matches = self.edits_ui.workspace_root == self.diff_repository_root();
        let files = if workspace_mode {
            if root_matches {
                self.edits_ui.workspace_files.clone()
            } else {
                Rc::new(Vec::new())
            }
        } else {
            self.session_edits()
        };
        let reveal_index = if workspace_mode {
            None
        } else {
            files
                .iter()
                .position(|file| Some(&file.path) == self.edits_ui.selected.as_ref())
        };
        let workspace = cx.entity();
        let mut list = div()
            .on_children_prepainted(move |_, _, cx| {
                workspace.update(cx, |this, cx| {
                    if this.edits_ui.reveal_pending.replace(false) {
                        if let Some(index) = reveal_index {
                            this.edits_ui.sidebar_scroll.scroll_to_item(index);
                        } else {
                            this.edits_ui
                                .sidebar_scroll
                                .set_offset(point(px(0.), px(0.)));
                        }
                        let id = cx.entity_id();
                        cx.defer(move |cx| cx.notify(id));
                    }
                });
            })
            .id("edited-files-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.edits_ui.sidebar_scroll)
            .flex()
            .flex_col();
        let scope = if workspace_mode {
            format!("workspace:{:?}", self.edits_ui.workspace_root)
        } else {
            format!("session:{}", self.history.active)
        };
        let grouped = workspace_mode && root_matches && self.edits_ui.workspace_sources.len() > 0;
        let virtualized = !grouped
            && !self.edits_ui.expanded
            && workspace_mode
            && (files.len() + self.edits_ui.conflicts.len() > 128
                || ((!files.is_empty() || !self.edits_ui.conflicts.is_empty())
                    && self.history.workspace_display == crate::workspace_tree::DisplayMode::Tree));
        if virtualized {
            list = div()
                .id("edited-files-scroll")
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(self.virtual_workspace_files(files.clone(), scope.clone(), cx));
        }
        if !grouped
            && !self.edits_ui.expanded
            && !virtualized
            && workspace_mode
            && root_matches
            && !self.edits_ui.conflicts.is_empty()
        {
            for view in &self.edits_ui.conflicts {
                view.update(cx, |view, cx| {
                    if view.tree_depth.take().is_some() {
                        cx.notify();
                    }
                    view.list_link.borrow_mut().take();
                    if view.theme != self.theme {
                        view.theme = self.theme;
                        view.editor
                            .update(cx, |editor, cx| editor.set_theme(self.theme, cx));
                        cx.notify();
                    }
                });
            }
            list = list
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .flex()
                        .items_center()
                        .gap_1()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(Icon::GitConflict.render(13.))
                        .child(format!("待解决冲突 · {}", self.edits_ui.conflicts.len())),
                )
                .children(self.edits_ui.conflicts.iter().cloned());
        }
        let can_undo = !workspace_mode
            && !self.any_run_busy()
            && self.edits_ui.undo_task.is_none()
            && self.queued_requests.is_empty();
        if !grouped && !self.edits_ui.expanded && !virtualized {
            for (index, file) in files.iter().enumerate() {
                let view = self.file_row(
                    file,
                    &scope,
                    None,
                    None,
                    can_undo,
                    reveal_index == Some(index),
                    cx,
                );
                list = list.child(view);
            }
        }
        if grouped && !self.edits_ui.expanded {
            list = self.compact_workspace_files(files.clone(), scope.clone(), cx);
        }
        let workspace_status = if !workspace_mode {
            None
        } else if !root_matches {
            Some("项目已切换，请刷新工作区变更。".to_owned())
        } else if self.edits_ui.stage_task.is_some() {
            Some("正在更新暂存区…".to_owned())
        } else if self.edits_ui.workspace_task.is_some() {
            Some("正在读取 Git 工作区文件清单…".to_owned())
        } else if self.edits_ui.workspace_conflicts_task.is_some() {
            Some("文件清单已就绪，正在读取冲突详情…".to_owned())
        } else if self.edits_ui.workspace_stats_task.is_some() {
            Some("文件清单已就绪，正在后台统计改动行数…".to_owned())
        } else {
            self.edits_ui.workspace_error.clone()
        };
        let source_weak = cx.entity().downgrade();
        let mut body = div()
            .when(self.edits_ui.expanded, |body| body.w_full())
            .when(!self.edits_ui.expanded, |body| {
                body.w(px(self.right_sidebar_width))
            })
            .flex_shrink_0()
            .h_full()
            .min_h_0()
            .border_l_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.panel))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(52.))
                    .flex_shrink_0()
                    .px_3()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(
                                if workspace_mode
                                    && files.is_empty()
                                    && (self.edits_ui.workspace_task.is_some()
                                        || (self.edits_ui.workspace_conflicts_task.is_some()
                                            && self.edits_ui.conflicts.is_empty()))
                                {
                                    "文件变更 · 读取中".into()
                                } else {
                                    format!(
                                        "文件变更 · {}",
                                        files.len()
                                            + if workspace_mode && root_matches {
                                                self.edits_ui.conflicts.len()
                                            } else {
                                                0
                                            }
                                    )
                                },
                            ),
                    )
                    .children(self.repository_select(crate::panel_repositories::Panel::Diff, cx))
                    .when(!workspace_mode, |header| {
                        header.child(
                            Button::new("全部撤销")
                                .kind(ButtonKind::Plain)
                                .when(!can_undo || files.is_empty(), |button| button.disabled())
                                .theme(self.theme)
                                .render()
                                .debug_selector(|| "undo-session-changes".into())
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.undo_session_edits(None, cx)),
                                ),
                        )
                    })
                    .child(
                        IconButton::new(
                            if self.edits_ui.expanded {
                                Icon::Collapse
                            } else {
                                Icon::Expand
                            },
                            if self.edits_ui.expanded {
                                "恢复侧栏 Diff"
                            } else {
                                "展开 Diff，左右对照"
                            },
                        )
                        .plain()
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "edits-panel-expand".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.edits_ui.expanded = !this.edits_ui.expanded;
                            cx.notify();
                        })),
                    )
                    .child(
                        Button::new("关闭")
                            .icon(Icon::Close)
                            .kind(ButtonKind::Plain)
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.edits_ui.open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "diff-source-toolbar".into())
                    .h(px(48.))
                    .flex_shrink_0()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        SegmentedControl::new([
                            SegmentOption::new("会话记录"),
                            SegmentOption::new("Git 工作区"),
                        ])
                        .id("edit-diff-source")
                        .selected(usize::from(workspace_mode))
                        .theme(self.theme)
                        .on_change(move |index, _, cx| {
                            let _ = source_weak.update(cx, |this, cx| {
                                this.edits_ui.workspace_mode = index == 1;
                                this.edits_ui.reveal_pending.set(true);
                                if index == 1 {
                                    this.refresh_workspace_edits(cx);
                                }
                                cx.notify();
                            });
                        })
                        .render(),
                    )
                    .when(workspace_mode && root_matches, |row| {
                        row.children(self.edits_ui.workspace_branch.as_ref().map(|branch| {
                            Tooltip::new(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .min_w_0()
                                    .max_w(px(200.))
                                    .text_size(px(11.))
                                    .text_color(rgb(p.text_muted))
                                    .child(Icon::GitBranch.render(13.))
                                    .child(div().truncate().child(branch.clone())),
                                format!("当前分支：{branch}"),
                            )
                            .theme(self.theme)
                            .render(cx)
                        }))
                    })
                    .child(div().flex_1().min_w_0().overflow_hidden().when_some(
                        workspace_status,
                        |slot, status| {
                            slot.child(
                                Tooltip::new(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(11.))
                                        .text_color(rgb(p.text_muted))
                                        .child(status.clone()),
                                    status,
                                )
                                .theme(self.theme)
                                .render(cx)
                                .w_full()
                                .min_w_0(),
                            )
                        },
                    ))
                    .when(workspace_mode, |row| {
                        row.child(
                            Button::new("刷新")
                                .kind(ButtonKind::Plain)
                                .theme(self.theme)
                                .render()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.refresh_workspace_edits(cx)),
                                ),
                        )
                    }),
            );
        if let Some((id, message)) = &self.edits_ui.undo_notice
            && *id == self.history.active
        {
            body = body.child(
                div()
                    .px_3()
                    .py_2()
                    .text_size(px(12.))
                    .text_color(rgb(p.text_muted))
                    .child(message.clone()),
            );
        }
        if workspace_mode {
            if root_matches && !virtualized {
                for note in &self.edits_ui.workspace_notes {
                    list = list.child(
                        div()
                            .px_3()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(note.clone()),
                    );
                }
            }
        }
        if files.is_empty()
            && (!workspace_mode
                || self.edits_ui.conflicts.is_empty()
                    && self.edits_ui.workspace_task.is_none()
                    && self.edits_ui.workspace_conflicts_task.is_none()
                    && self.edits_ui.workspace_error.is_none())
        {
            list = list.child(
                div()
                    .p_4()
                    .text_color(rgb(p.text_muted))
                    .child(if workspace_mode {
                        "暂无可显示的文本变更。"
                    } else {
                        "当前会话暂无文件工具记录的修改。"
                    }),
            );
        }
        let loading_files = workspace_mode
            && root_matches
            && files.is_empty()
            && self.edits_ui.conflicts.is_empty()
            && self.edits_ui.workspace_task.is_some();
        if self.edits_ui.expanded && !loading_files {
            list = self.expanded_edit_files(files, scope, can_undo, root_matches, window, cx);
        }
        if loading_files {
            list = list.child(
                div()
                    .p_3()
                    .debug_selector(|| "workspace-files-loading-skeleton".into())
                    .child(
                        Skeleton::new("workspace-files-loading")
                            .lines(8)
                            .height(12.)
                            .gap(20.)
                            .theme(self.theme),
                    ),
            );
        }
        body.child(list)
            .when(workspace_mode && !self.edits_ui.expanded, |body| {
                body.children(self.git_commit_footer(cx))
            })
            .child(
                div()
                    .debug_selector(|| "diff-panel-footer".into())
                    .flex_shrink_0()
                    .when(!workspace_mode, |footer| {
                        footer
                            .px_3()
                            .py_2()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child("会话内文件工具的累计净变更，含子代理。")
                    }),
            )
    }

    fn expanded_edit_files(
        &self,
        files: Rc<Vec<Change>>,
        scope: String,
        can_undo: bool,
        root_matches: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        use crate::workspace_tree::Row;
        let p = self.theme.palette();
        let tree = self.history.workspace_display == crate::workspace_tree::DisplayMode::Tree;
        let grouped = self.edits_ui.workspace_mode
            && root_matches
            && (!self.edits_ui.workspace_sources.is_empty() || !self.edits_ui.conflicts.is_empty());
        let conflicts = if grouped {
            self.edits_ui.conflicts.as_slice()
        } else {
            &[]
        };
        let mut rows = Vec::new();
        if !conflicts.is_empty() {
            rows.push(ChangeRow::Group {
                title: "待解决冲突".into(),
                count: conflicts.len(),
                staged: None,
            });
            for row in self
                .conflict_rows(conflicts, &format!("{scope}:conflicts"), cx)
                .iter()
            {
                match row {
                    Row::File { index, depth } => {
                        let file = &conflicts[*index].read(cx).file;
                        rows.push(ChangeRow::File {
                            path: file.path.to_string_lossy().into_owned(),
                            label: change_file_label(&file.label, tree),
                            depth: *depth,
                            status: Some(ChangeStatus::Conflict),
                            staged: false,
                            additions: None,
                            deletions: None,
                            binary: false,
                        });
                    }
                    directory => rows.push(change_directory(directory, None)),
                }
            }
        }
        if grouped {
            for row in self.git_group_rows(files.clone(), &scope).iter() {
                rows.push(match row {
                    GitRow::Group(staged, count) => ChangeRow::Group {
                        title: if *staged { "已暂存" } else { "未暂存" }.into(),
                        count: *count,
                        staged: Some(*staged),
                    },
                    GitRow::Directory(staged, row) => change_directory(row, Some(*staged)),
                    GitRow::File(staged, index, depth) => {
                        let file = &files[*index];
                        let source = &self.edits_ui.workspace_sources[&file.path];
                        let stats = source.layer_stats(*staged);
                        ChangeRow::File {
                            path: file.path.to_string_lossy().into_owned(),
                            label: change_file_label(&file.label, tree),
                            depth: *depth,
                            status: Some(source.layer_status(*staged).into()),
                            staged: *staged,
                            additions: stats.map(|(added, _)| added),
                            deletions: stats.map(|(_, removed)| removed),
                            binary: source.stats_ready && stats.is_none(),
                        }
                    }
                });
            }
        } else {
            let tree_rows = if tree {
                let mut cache = self.edits_ui.tree_cache.borrow_mut();
                if cache
                    .as_ref()
                    .is_none_or(|(key, cached, _)| key != &scope || !Rc::ptr_eq(cached, &files))
                {
                    let empty = Default::default();
                    let collapsed = self
                        .edits_ui
                        .collapsed_directories
                        .get(&scope)
                        .unwrap_or(&empty);
                    *cache = Some((
                        scope.clone(),
                        files.clone(),
                        Rc::new(crate::workspace_tree::rows(&files, collapsed)),
                    ));
                }
                cache.as_ref().unwrap().2.clone()
            } else {
                Rc::new(
                    (0..files.len())
                        .map(|index| Row::File { index, depth: 0 })
                        .collect(),
                )
            };
            rows.extend(tree_rows.iter().map(|row| match row {
                Row::File { index, depth } => {
                    let file = &files[*index];
                    ChangeRow::File {
                        path: file.path.to_string_lossy().into_owned(),
                        label: change_file_label(&file.label, tree),
                        depth: *depth,
                        status: None,
                        staged: false,
                        additions: Some(file.added),
                        deletions: Some(file.removed),
                        binary: false,
                    }
                }
                directory => change_directory(directory, None),
            }));
        }
        let selected = self
            .edits_ui
            .selected
            .as_ref()
            .filter(|path| {
                conflicts
                    .iter()
                    .any(|view| &view.read(cx).file.path == *path)
                    || files.iter().any(|file| &file.path == *path)
            })
            .cloned()
            .or_else(|| {
                conflicts
                    .first()
                    .map(|view| view.read(cx).file.path.clone())
            })
            .or_else(|| {
                files
                    .iter()
                    .find(|file| !grouped || !diff_on_demand(&file.label))
                    .map(|file| file.path.clone())
            });
        let selected_layer = selected
            .as_ref()
            .filter(|_| grouped)
            .and_then(|path| self.edits_ui.workspace_sources.get(path))
            .is_some_and(|source| {
                if self.edits_ui.selected_staged {
                    source.staged
                } else {
                    !source.unstaged
                }
            });
        let navigation = self.expanded_change_navigation(
            &scope,
            rows,
            selected
                .as_ref()
                .map(|path| (path.to_string_lossy().into_owned(), selected_layer)),
            grouped,
            window,
            cx,
        );
        let mut detail_bar = None;
        let mut conflict_header = None;
        let mut detail = div()
            .id("expanded-diff-content")
            .debug_selector(|| "expanded-diff-content".into())
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scroll();
        if let Some(path) = selected.as_ref() {
            if let Some(view) = conflicts
                .iter()
                .find(|view| &view.read(cx).file.path == path)
            {
                let scroll_key = format!("{scope}:{}", path.display());
                let scroll = self
                    .edits_ui
                    .detail_scrolls
                    .borrow_mut()
                    .entry(scroll_key.clone())
                    .or_default()
                    .clone();
                let state = self
                    .edits_ui
                    .detail_scrollbars
                    .borrow_mut()
                    .entry(scroll_key)
                    .or_default()
                    .clone();
                detail_bar = Some(
                    Scrollbar::vertical(&scroll, &state)
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "diff-detail-vertical-scrollbar".into())
                        .absolute()
                        .right_0()
                        .top_0(),
                );
                detail = detail.track_scroll(&scroll).pr(px(12.));
                view.update(cx, |view, cx| {
                    view.reveal(scroll.clone(), self.edits_ui.reveal_revision, cx);
                    if view.tree_depth.take().is_some()
                        || !view.expanded
                        || view.theme != self.theme
                    {
                        cx.notify();
                    }
                    view.list_link.borrow_mut().take();
                    view.expanded = true;
                    view.theme = self.theme;
                    view.editor
                        .update(cx, |editor, cx| editor.set_theme(self.theme, cx));
                });
                conflict_header = Some(view.read(cx).file_header());
                detail = detail.child(view.clone());
            } else if let Some(file) = files.iter().find(|file| &file.path == path) {
                let group_scope = if grouped {
                    format!(
                        "{scope}:{}",
                        if selected_layer { "staged" } else { "unstaged" }
                    )
                } else {
                    scope.clone()
                };
                detail = detail
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(self.file_row(file, &group_scope, None, None, can_undo, false, cx));
            }
        }
        if selected.is_none() {
            detail = detail.child(
                div()
                    .p_4()
                    .text_color(rgb(p.text_muted))
                    .child("选择左侧文件查看 Diff。"),
            );
        }
        if self.edits_ui.workspace_mode {
            detail = detail.children(
                self.edits_ui
                    .workspace_notes
                    .iter()
                    .map(|note| div().px_3().text_size(px(11.)).child(note.clone())),
            );
        }
        let weak = cx.entity().downgrade();
        div()
            .on_children_prepainted(move |bounds, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    let width: f32 = bounds
                        .iter()
                        .map(|bounds| f32::from(bounds.size.width))
                        .sum();
                    let maximum = (width - 320.).clamp(180., 800.);
                    this.edits_ui.navigation_maximum.set(maximum);
                    if this.edits_ui.navigation_width.unwrap_or(260.) > maximum {
                        this.edits_ui.navigation_width = Some(maximum);
                        let id = cx.entity_id();
                        cx.defer(move |cx| cx.notify(id));
                    }
                });
            })
            .id("expanded-diff-layout")
            .debug_selector(|| "expanded-diff-layout".into())
            .flex_1()
            .min_h_0()
            .flex()
            .child(self.diff_navigation(navigation, cx))
            .child(
                div()
                    .id("diff-navigation-resize")
                    .debug_selector(|| "diff-navigation-resize".into())
                    .w(px(5.))
                    .h_full()
                    .flex_shrink_0()
                    .cursor(gpui::CursorStyle::ResizeLeftRight)
                    .bg(rgb(p.border))
                    .hover(|style| style.bg(rgb(p.accent)))
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            this.edits_ui.navigation_resize = Some((
                                event.position.x,
                                this.edits_ui.navigation_width.unwrap_or(260.),
                            ));
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .children(conflict_header)
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .flex()
                            .child(detail)
                            .children(detail_bar),
                    ),
            )
    }
    fn expanded_change_navigation(
        &self,
        scope: &str,
        rows: Vec<ChangeRow>,
        selected: Option<(String, bool)>,
        grouped: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Entity<ChangeNavigation> {
        let mut cached = self.edits_ui.navigation.borrow_mut();
        let navigation = match cached.as_ref() {
            Some((key, navigation)) if key == scope => navigation.clone(),
            _ => {
                let navigation = cx.new(|cx| {
                    ChangeNavigation::new(cx)
                        .stage_controls(true)
                        .background_visible(false)
                });
                let scope = scope.to_owned();
                let event_scope = scope.clone();
                cx.subscribe_in(&navigation, window, move |this, _, event, window, cx| {
                    this.handle_change_navigation(event, &event_scope, window, cx);
                })
                .detach();
                *cached = Some((scope, navigation.clone()));
                navigation
            }
        };
        let mut blocked = HashSet::new();
        let mut available = [false; 2];
        let mut directories = [HashSet::new(), HashSet::new()];
        if grouped {
            let busy = self.workspace_stage_busy();
            let tree = self.history.workspace_display == crate::workspace_tree::DisplayMode::Tree;
            for source in self.edits_ui.workspace_sources.values() {
                if busy || source.overlaps(&self.edits_ui.stage_pending) {
                    blocked.insert(source.path.to_string_lossy().into_owned());
                } else {
                    available[0] |= source.unstaged;
                    available[1] |= source.staged;
                    if tree {
                        let mut directory = source.label.as_str();
                        while let Some((parent, _)) = directory.rsplit_once('/') {
                            if source.unstaged {
                                directories[0].insert(parent.to_owned());
                            }
                            if source.staged {
                                directories[1].insert(parent.to_owned());
                            }
                            directory = parent;
                        }
                    }
                }
            }
        }
        navigation.update(cx, |navigation, cx| {
            navigation.set_theme(self.theme, cx);
            navigation.set_rows(rows, cx);
            navigation.set_selected(selected, cx);
            navigation.set_stage_availability(blocked, available, cx);
            navigation.set_stage_directory_availability(directories, cx);
        });
        navigation
    }

    fn handle_change_navigation(
        &mut self,
        event: &ChangeNavigationEvent,
        scope: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ChangeNavigationEvent::Select { path, staged } => {
                self.edits_ui.selected = Some(PathBuf::from(path));
                self.edits_ui.selected_staged = *staged;
                self.edits_ui.reveal_revision += 1;
                cx.notify();
            }
            ChangeNavigationEvent::ToggleDirectory { path, staged } => {
                let group_scope = match staged {
                    Some(true) => format!("{scope}:staged"),
                    Some(false) => format!("{scope}:unstaged"),
                    None if self.edits_ui.workspace_mode && !self.edits_ui.conflicts.is_empty() => {
                        format!("{scope}:conflicts")
                    }
                    None => scope.to_owned(),
                };
                let collapsed = self
                    .edits_ui
                    .collapsed_directories
                    .entry(group_scope)
                    .or_default();
                if !collapsed.remove(path) {
                    collapsed.insert(path.clone());
                }
                self.edits_ui.tree_cache.borrow_mut().take();
                self.edits_ui.grouped_rows.borrow_mut().take();
                self.edits_ui.conflict_tree_cache.borrow_mut().take();
                self.edits_ui.grouped_list.borrow_mut().take();
                cx.notify();
            }
            ChangeNavigationEvent::Stage { paths, staged } => {
                if self.edits_ui.workspace_root != self.diff_repository_root() {
                    return;
                }
                let sources = paths
                    .iter()
                    .filter_map(|path| {
                        self.edits_ui
                            .workspace_sources
                            .get(std::path::Path::new(path))
                    })
                    .cloned()
                    .collect();
                self.stage_workspace_files(sources, *staged, cx);
            }
            ChangeNavigationEvent::StageGroup { staged, .. } => {
                if self.edits_ui.workspace_root != self.diff_repository_root() {
                    return;
                }
                let sources = self
                    .edits_ui
                    .workspace_sources
                    .values()
                    .filter(|source| {
                        if *staged {
                            source.unstaged
                        } else {
                            source.staged
                        }
                    })
                    .cloned()
                    .collect();
                self.stage_workspace_files(sources, *staged, cx);
            }
            ChangeNavigationEvent::StageDirectory {
                path,
                layer,
                staged,
            } => {
                if self.edits_ui.workspace_root != self.diff_repository_root() {
                    return;
                }
                let sources = self
                    .workspace_directory_sources(path, *layer)
                    .cloned()
                    .collect();
                self.stage_workspace_files(sources, *staged, cx);
            }
            ChangeNavigationEvent::Menu { path, position } => {
                let path = PathBuf::from(path);
                let label = if self.edits_ui.workspace_mode {
                    self.edits_ui
                        .conflicts
                        .iter()
                        .find_map(|view| {
                            let file = &view.read(cx).file;
                            (file.path == path).then(|| file.label.clone())
                        })
                        .or_else(|| {
                            self.edits_ui
                                .workspace_files
                                .iter()
                                .find(|file| file.path == path)
                                .map(|file| file.label.clone())
                        })
                } else {
                    self.session_edits()
                        .iter()
                        .find(|file| file.path == path)
                        .map(|file| file.label.clone())
                }
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
                self.open_diff_file_menu(path, label, *position, window, cx);
            }
        }
    }

    fn diff_navigation(
        &self,
        navigation: Entity<ChangeNavigation>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div()
            .debug_selector(|| "expanded-diff-navigation".into())
            .w(px(self.edits_ui.navigation_width.unwrap_or(260.)))
            .flex_shrink_0()
            .h_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .debug_selector(|| "diff-navigation-tree".into())
                    .flex_1()
                    .min_h_0()
                    .child(navigation),
            )
            .when(self.edits_ui.workspace_mode, |column| {
                column.children(self.git_commit_footer(cx))
            })
    }

    /// The wand's latest notice for one repository, if it belongs to `root`.
    pub(super) fn commit_notice_for(&self, root: &std::path::Path) -> Option<String> {
        self.edits_ui
            .commit_notice
            .as_ref()
            .filter(|(project, _)| project == root)
            .map(|(_, notice)| notice.clone())
    }

    /// Writes a commit message from the staged diff with the configured model.
    pub(super) fn generate_commit_message(
        &mut self,
        root: PathBuf,
        entry: Entity<PromptEditor>,
        cx: &mut Context<Self>,
    ) {
        if self.edits_ui.commit_generation.contains_key(&root) {
            return;
        }
        let config = match self.commit_model_config() {
            Ok(config) => config,
            Err(error) => {
                self.edits_ui.commit_notice = Some((root, error));
                cx.notify();
                return;
            }
        };
        self.edits_ui.commit_notice = None;
        let prompt = self.history.commit_message.prompt().to_owned();
        let finished = root.clone();
        let task = self.spawn_commit_message(
            root.clone(),
            entry,
            config,
            prompt,
            cx,
            move |this, root, result, cx| {
                if let Some(task) = this.edits_ui.commit_generation.remove(&root) {
                    task.detach();
                }
                if let Err(error) = result {
                    this.edits_ui.commit_notice =
                        Some((root, format!("生成提交信息失败：{error}")));
                }
                cx.notify();
            },
        );
        self.edits_ui.commit_generation.insert(finished, task);
        cx.notify();
    }

    fn git_commit_footer(&self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let root = self.diff_repository_root()?;
        if crate::ssh_project::is_remote(&root) {
            return None;
        }
        let p = self.theme.palette();
        let entry = self
            .edits_ui
            .commit_drafts
            .borrow_mut()
            .entry(root.clone())
            .or_insert_with(|| {
                let entry = cx.new(|cx| {
                    PromptEditor::new(cx, "输入提交信息…")
                        .theme(self.theme)
                        .min_rows(2)
                });
                cx.observe(&entry, |_, _, cx| cx.notify()).detach();
                entry
            })
            .clone();
        entry.update(cx, |entry, cx| entry.set_theme(self.theme, cx));
        let matches = self.edits_ui.workspace_root.as_ref() == Some(&root);
        let staged = if matches {
            self.edits_ui
                .workspace_sources
                .values()
                .filter(|source| source.staged)
                .count()
        } else {
            0
        };
        let disabled = !self.can_commit_workspace(cx);
        let submitting = self.edits_ui.commit_task.is_some();
        let generating = self.edits_ui.commit_generation.contains_key(&root);
        // Anything the repository can read counts; the wand stays live either
        // way so a click can explain what is missing instead of doing nothing.
        let has_changes = matches
            && (self
                .edits_ui
                .workspace_sources
                .values()
                .any(|source| source.staged || source.unstaged)
                || !self.edits_ui.conflicts.is_empty());
        let wand = {
            // Always clickable: with nothing to read the request answers with a
            // notice instead of leaving the button silently dead.
            let enabled = !generating && !submitting;
            let mut icon = div()
                .id("diff-commit-generate")
                .debug_selector(|| "diff-commit-generate".into())
                .size(px(24.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .text_color(rgb(if enabled && has_changes {
                    p.accent
                } else {
                    p.text_muted
                }))
                .when(enabled, |button| {
                    button
                        .cursor_pointer()
                        .hover(move |style| style.bg(rgb(p.accent_tint)))
                        .on_click(cx.listener({
                            let root = root.clone();
                            let entry = entry.clone();
                            move |this, _, _, cx| {
                                this.generate_commit_message(root.clone(), entry.clone(), cx)
                            }
                        }))
                });
            if generating {
                icon = icon.child(
                    Icon::Spinner
                        .render(14.)
                        .with_animation(
                            "diff-commit-generation-spinner",
                            Animation::new(std::time::Duration::from_millis(900)).repeat(),
                            |spinner, delta| {
                                spinner.with_transformation(gpui::Transformation::rotate(
                                    gpui::radians(std::f32::consts::TAU * delta),
                                ))
                            },
                        )
                        .into_any_element(),
                );
            } else {
                icon = icon.child(Icon::Wand.render(15.).into_any_element());
            }
            icon
        };
        let submit = div()
            .id("diff-commit-submit")
            .debug_selector(|| "diff-commit-submit".into())
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
                    .on_click(cx.listener(|this, _, _, cx| this.commit_workspace(cx)))
            })
            .child(if submitting {
                Icon::Spinner
                    .render(14.)
                    .with_animation(
                        "diff-commit-spinner",
                        Animation::new(std::time::Duration::from_millis(900)).repeat(),
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
        let model = commit_message::model_picker(
            "diff-commit-model",
            self.theme,
            self.commit_model_label(),
            self.commit_model_options(),
            self.commit_model_value(),
            self.edits_ui.commit_model_open,
            cx.entity().downgrade(),
            |this, open| this.edits_ui.commit_model_open = open,
            |this, value, cx| this.choose_commit_model(value, cx),
        );
        Some(
            div()
                .debug_selector(|| "diff-commit-footer".into())
                .flex_shrink_0()
                .border_t_1()
                .border_color(rgb(p.border))
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .id("diff-commit-message")
                        .key_context("DiffCommitMessage")
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_2()
                        .rounded(px(14.))
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.surface))
                        .debug_selector(|| "diff-commit-message".into())
                        .child(
                            div()
                                .id("diff-commit-message-scroll")
                                .max_h(px(117.))
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
                .when_some(
                    if matches && !self.edits_ui.conflicts.is_empty() {
                        Some("请先解决合并冲突".to_owned())
                    } else if staged > 0 {
                        Some(format!("提交 {staged} 个已暂存文件"))
                    } else {
                        None
                    },
                    |footer, status| {
                        footer.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child(status),
                        )
                    },
                )
                .when_some(
                    self.edits_ui
                        .commit_notice
                        .as_ref()
                        .filter(|(project, _)| project == &root),
                    |footer, (_, notice)| {
                        footer.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child(notice.clone()),
                        )
                    },
                ),
        )
    }

    fn can_commit_workspace(&self, cx: &App) -> bool {
        let Some(root) = self.diff_repository_root() else {
            return false;
        };
        self.edits_ui.workspace_mode
            && !crate::ssh_project::is_remote(&root)
            && self.edits_ui.workspace_root.as_ref() == Some(&root)
            && self.edits_ui.workspace_task.is_none()
            && self.edits_ui.workspace_conflicts_task.is_none()
            && self.edits_ui.workspace_error.is_none()
            && !self.repository_change_busy(cx)
            && self.edits_ui.conflicts.is_empty()
            && self
                .edits_ui
                .workspace_sources
                .values()
                .any(|source| source.staged)
            && self
                .edits_ui
                .commit_drafts
                .borrow()
                .get(&root)
                .is_some_and(|entry| {
                    let entry = entry.read(cx);
                    !entry.is_composing() && !entry.value().trim().is_empty()
                })
    }

    fn commit_workspace(&mut self, cx: &mut Context<Self>) {
        if !self.can_commit_workspace(cx) {
            return;
        }
        let root = self.diff_repository_root().unwrap();
        let entry = self
            .edits_ui
            .commit_drafts
            .borrow()
            .get(&root)
            .unwrap()
            .clone();
        let message = entry.read(cx).value().to_owned();
        let project = root.clone();
        let submitted = message.clone();
        self.edits_ui.commit_notice = None;
        let work = cx
            .background_executor()
            .spawn(async move { crate::workspace_diff::commit_staged(&root, &message) });
        self.edits_ui.commit_task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.edits_ui.commit_task.take() {
                    task.detach();
                }
                let notice = match result {
                    Ok(()) => {
                        entry.update(cx, |entry, cx| {
                            if !entry.is_composing() && entry.value() == submitted {
                                entry.set_value("", cx);
                            }
                        });
                        "提交成功".to_owned()
                    }
                    Err(error) => format!("提交失败：{error}"),
                };
                this.edits_ui.commit_notice = Some((project.clone(), notice));
                if this.diff_repository_root().as_ref() == Some(&project) {
                    this.refresh_workspace_edits(cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn stage_workspace_files(
        &mut self,
        sources: Vec<crate::workspace_diff::Source>,
        staged: bool,
        cx: &mut Context<Self>,
    ) {
        if self.workspace_stage_busy() {
            return;
        }
        if self.edits_ui.workspace_task.is_some() {
            self.edits_ui.workspace_task = None;
            self.edits_ui.workspace_revision += 1;
            self.edits_ui.workspace_background_refresh = false;
            self.edits_ui.workspace_resync_pending = true;
        }
        let sources: Vec<_> = sources
            .into_iter()
            .filter(|source| !source.overlaps(&self.edits_ui.stage_pending))
            .collect();
        if sources.is_empty() {
            return;
        }
        for source in &sources {
            self.edits_ui.stage_pending.extend(source.affected_paths());
        }
        self.edits_ui.stage_queue.push_back(StageRequest {
            sources,
            staged,
            project: self.diff_repository_root(),
        });
        self.edits_ui.workspace_error = None;
        self.start_next_stage(cx);
        cx.notify();
    }

    fn start_next_stage(&mut self, cx: &mut Context<Self>) {
        if self.edits_ui.stage_task.is_some() {
            return;
        }
        let Some(request) = self.edits_ui.stage_queue.pop_front() else {
            if std::mem::take(&mut self.edits_ui.stage_refresh_pending) {
                self.refresh_workspace_edits(cx);
            } else if std::mem::take(&mut self.edits_ui.workspace_resync_pending) {
                self.load_workspace_edits(true, cx);
            }
            return;
        };
        let paths: Vec<_> = request
            .sources
            .iter()
            .flat_map(|source| source.affected_paths())
            .collect();
        let work = cx.background_executor().spawn(async move {
            crate::workspace_diff::stage_and_refresh(&request.sources, request.staged)
        });
        self.edits_ui.stage_task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.edits_ui.stage_task.take() {
                    task.detach();
                }
                for path in &paths {
                    this.edits_ui.stage_pending.remove(path);
                }
                if this.diff_repository_root() == request.project
                    && this.edits_ui.workspace_root == request.project
                {
                    match result {
                        Ok(update) => this.apply_stage_update(update, cx),
                        Err(error) => {
                            this.edits_ui.workspace_error =
                                Some(format!("暂存操作失败：{error}；可刷新后重试"))
                        }
                    }
                }
                this.start_next_stage(cx);
                cx.notify();
            });
        }));
    }

    fn apply_stage_update(
        &mut self,
        update: crate::workspace_diff::StageUpdate,
        cx: &mut Context<Self>,
    ) {
        // Acknowledge our own index write, but never swallow an external change
        // that happened before this request's initial snapshot.
        if self.edits_ui.workspace_stamp == update.before_stamp {
            self.edits_ui.workspace_stamp = update.inventory.stamp.clone();
        }
        let mut replacements: HashMap<_, _> = update
            .inventory
            .files
            .into_iter()
            .map(|file| (file.path.clone(), file))
            .collect();
        let mut files: Vec<_> = self
            .edits_ui
            .workspace_files
            .iter()
            .filter_map(|file| {
                if update.affected.contains(&file.path) {
                    replacements.remove(&file.path)
                } else {
                    Some(file.clone())
                }
            })
            .collect();
        let mut additions: Vec<_> = replacements.into_values().collect();
        additions.sort_by(|a, b| a.label.cmp(&b.label));
        files.extend(additions);
        let sources = Rc::make_mut(&mut self.edits_ui.workspace_sources);
        sources.retain(|path, _| !update.affected.contains(path));
        sources.extend(update.inventory.sources);
        self.edits_ui.workspace_files = Rc::new(files);
        self.edits_ui.grouped_rows.borrow_mut().take();
        self.edits_ui.file_views.borrow_mut().retain(|_, view| {
            let path = &view.read(cx).file.path;
            !update.affected.contains(path) || sources.contains_key(path)
        });
    }

    fn stage_checkbox(
        &self,
        sources: Vec<crate::workspace_diff::Source>,
        checked: bool,
        selector: String,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        self.stage_control(StageTarget::Files(sources), checked, selector, cx)
    }

    fn workspace_stage_busy(&self) -> bool {
        self.edits_ui.commit_task.is_some()
            || self.edits_ui.file_action_task.is_some()
            || self.edits_ui.workspace_task.is_some() && !self.edits_ui.workspace_background_refresh
    }

    fn workspace_directory_sources<'a>(
        &'a self,
        directory: &'a str,
        layer: bool,
    ) -> impl Iterator<Item = &'a crate::workspace_diff::Source> {
        self.edits_ui
            .workspace_sources
            .values()
            .filter(move |source| {
                (if layer {
                    source.staged
                } else {
                    source.unstaged
                }) && source
                    .label
                    .strip_prefix(directory)
                    .is_some_and(|suffix| suffix.starts_with('/'))
            })
    }

    fn stage_control(
        &self,
        target: StageTarget,
        checked: bool,
        selector: String,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let weak = cx.entity().downgrade();
        let available =
            |source: &crate::workspace_diff::Source| !source.overlaps(&self.edits_ui.stage_pending);
        let any_available = match &target {
            StageTarget::Files(sources) => sources.iter().any(available),
            StageTarget::Directory(path, layer) => self
                .workspace_directory_sources(path, *layer)
                .any(available),
            StageTarget::Group(staged) => self.edits_ui.workspace_sources.values().any(|source| {
                (if *staged {
                    source.staged
                } else {
                    source.unstaged
                }) && available(source)
            }),
        };
        let busy = self.workspace_stage_busy() || !any_available;
        let mut checkbox = gpui_glint::components::Checkbox::new("")
            .checked(checked)
            .theme(self.theme);
        if !busy {
            checkbox = checkbox.on_change(move |staged, _, cx| {
                cx.stop_propagation();
                let _ = weak.update(cx, |this, cx| {
                    let sources = match &target {
                        StageTarget::Files(sources) => sources.clone(),
                        StageTarget::Directory(path, layer) => this
                            .workspace_directory_sources(path, *layer)
                            .cloned()
                            .collect(),
                        StageTarget::Group(group) => this
                            .edits_ui
                            .workspace_sources
                            .values()
                            .filter(|source| {
                                if *group {
                                    source.staged
                                } else {
                                    source.unstaged
                                }
                            })
                            .cloned()
                            .collect(),
                    };
                    this.stage_workspace_files(sources, staged, cx);
                });
            });
        }
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .flex_shrink_0()
            .p_1()
            .when(busy, |row| row.opacity(0.4))
            .child(checkbox.render())
            .on_click(|_, _, cx| cx.stop_propagation())
    }

    fn git_group_rows(&self, files: Rc<Vec<Change>>, scope: &str) -> Rc<Vec<GitRow>> {
        let tree = self.history.workspace_display == crate::workspace_tree::DisplayMode::Tree;
        let mut cache = self.edits_ui.grouped_rows.borrow_mut();
        if let Some((cached, mode, rows)) = cache.as_mut() {
            if *mode == tree
                && (Rc::ptr_eq(cached, &files)
                    || cached.len() == files.len()
                        && cached.iter().zip(files.iter()).all(|(a, b)| a.id == b.id))
            {
                *cached = files.clone();
                return rows.clone();
            }
        }
        let mut result = Vec::new();
        for staged in [false, true] {
            let indices: Vec<_> = files
                .iter()
                .enumerate()
                .filter_map(|(i, file)| {
                    self.edits_ui
                        .workspace_sources
                        .get(&file.path)
                        .filter(|source| {
                            if staged {
                                source.staged
                            } else {
                                source.unstaged
                            }
                        })
                        .map(|_| i)
                })
                .collect();
            if indices.is_empty() && self.edits_ui.workspace_sources.is_empty() {
                continue;
            }
            result.push(GitRow::Group(staged, indices.len()));
            if tree {
                let group_scope = format!("{scope}:{}", if staged { "staged" } else { "unstaged" });
                let empty = Default::default();
                let collapsed = self
                    .edits_ui
                    .collapsed_directories
                    .get(&group_scope)
                    .unwrap_or(&empty);
                let subset: Vec<_> = indices.iter().map(|i| files[*i].clone()).collect();
                result.extend(
                    crate::workspace_tree::rows(&subset, collapsed)
                        .into_iter()
                        .map(|row| match row {
                            crate::workspace_tree::Row::File { index, depth } => {
                                GitRow::File(staged, indices[index], depth)
                            }
                            row => GitRow::Directory(staged, row),
                        }),
                );
            } else {
                result.extend(indices.into_iter().map(|i| GitRow::File(staged, i, 0)));
            }
        }
        let rows = Rc::new(result);
        *cache = Some((files, tree, rows.clone()));
        rows
    }

    fn git_group_row(
        &self,
        row: &GitRow,
        files: &[Change],
        scope: &str,
        link: Option<(gpui::ListState, usize)>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match row {
            GitRow::Group(staged, count) => ChangeNavigation::group_element(
                if *staged { "已暂存" } else { "未暂存" },
                *count,
                false,
            )
            .child(div().flex_1())
            .child(self.stage_control(
                StageTarget::Group(*staged),
                *staged,
                format!("stage-group:{staged}"),
                cx,
            ))
            .into_any_element(),
            GitRow::Directory(staged, row) => {
                let crate::workspace_tree::Row::Directory {
                    path,
                    label,
                    depth,
                    count,
                    collapsed,
                } = row
                else {
                    unreachable!()
                };
                let path = path.clone();
                let group_scope =
                    format!("{scope}:{}", if *staged { "staged" } else { "unstaged" });
                ChangeNavigation::directory_element(
                    format!("git-directory:{group_scope}:{path}"),
                    label.clone(),
                    *depth,
                    *count,
                    *collapsed,
                    self.theme,
                )
                .child(self.stage_control(
                    StageTarget::Directory(path.clone(), *staged),
                    *staged,
                    format!("stage-directory:{path}:{staged}"),
                    cx,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let collapsed = this
                        .edits_ui
                        .collapsed_directories
                        .entry(group_scope.clone())
                        .or_default();
                    if !collapsed.remove(&path) {
                        collapsed.insert(path.clone());
                    }
                    this.edits_ui.grouped_rows.borrow_mut().take();
                    cx.notify();
                }))
                .into_any_element()
            }
            GitRow::File(staged, index, depth) => {
                let file = &files[*index];
                let group_scope =
                    format!("{scope}:{}", if *staged { "staged" } else { "unstaged" });
                self.file_row(
                    file,
                    &group_scope,
                    link,
                    (self.history.workspace_display == crate::workspace_tree::DisplayMode::Tree)
                        .then_some(*depth),
                    false,
                    false,
                    cx,
                )
                .into_any_element()
            }
        }
    }

    fn compact_workspace_files(
        &self,
        files: Rc<Vec<Change>>,
        scope: String,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let rows = self.git_group_rows(files.clone(), &scope);
        let conflicts = self.edits_ui.conflicts.clone();
        let conflict_scope = format!("{scope}:conflicts");
        let conflict_rows = self.conflict_rows(&conflicts, &conflict_scope, cx);
        let prefix = if conflicts.is_empty() {
            0
        } else {
            conflict_rows.len() + 1
        };
        let count = prefix + rows.len();
        let render_row = {
            let files = files.clone();
            let scope = scope.clone();
            let rows = rows.clone();
            let conflicts = conflicts.clone();
            move |this: &AgentWorkspace,
                  index: usize,
                  link: Option<(gpui::ListState, usize)>,
                  cx: &mut Context<AgentWorkspace>| {
                if prefix > 0 && index == 0 {
                    return ChangeNavigation::group_element("待解决冲突", conflicts.len(), true)
                        .into_any_element();
                }
                if index < prefix {
                    let (i, depth) = match &conflict_rows[index - 1] {
                        crate::workspace_tree::Row::Directory { .. } => {
                            return this.conflict_directory_row(
                                &conflict_rows[index - 1],
                                &conflict_scope,
                                cx,
                            );
                        }
                        crate::workspace_tree::Row::File { index, depth } => (*index, *depth),
                    };
                    let view = &conflicts[i];
                    view.update(cx, |view, cx| {
                        *view.list_link.borrow_mut() = link;
                        let depth = (this.history.workspace_display
                            == crate::workspace_tree::DisplayMode::Tree)
                            .then_some(depth);
                        if view.tree_depth != depth {
                            view.tree_depth = depth;
                            cx.notify();
                        }
                        if view.theme != this.theme {
                            view.theme = this.theme;
                            view.editor
                                .update(cx, |editor, cx| editor.set_theme(this.theme, cx));
                            cx.notify();
                        }
                    });
                    return view.clone().into_any_element();
                }
                this.git_group_row(&rows[index - prefix], &files, &scope, link, cx)
            }
        };
        let ids = conflicts
            .iter()
            .map(|view| view.entity_id())
            .collect::<Vec<_>>();
        let mut cache = self.edits_ui.grouped_list.borrow_mut();
        if cache.as_ref().is_none_or(|(_, cached_ids, state, _)| {
            *cached_ids != ids || state.item_count() < prefix
        }) {
            let state = gpui::ListState::new(count, gpui::ListAlignment::Top, px(96.));
            state.reset_with_uniform_height(count, px(49.));
            self.edits_ui.grouped_width.set(px(0.));
            *cache = Some((rows.clone(), ids, state, files.clone()));
        } else if let Some((old_rows, _, state, old_files)) = cache.as_mut() {
            if !Rc::ptr_eq(old_rows, &rows) {
                let old_keys: Vec<_> = old_rows.iter().map(|row| row.key(old_files)).collect();
                let new_keys: Vec<_> = rows.iter().map(|row| row.key(&files)).collect();
                let common = old_keys
                    .iter()
                    .zip(&new_keys)
                    .take_while(|(a, b)| a == b)
                    .count();
                let suffix = old_keys[common..]
                    .iter()
                    .rev()
                    .zip(new_keys[common..].iter().rev())
                    .take_while(|(a, b)| a == b)
                    .count();
                if common + suffix < old_keys.len() || common + suffix < new_keys.len() {
                    let top = state.logical_scroll_top();
                    let anchor = top
                        .item_ix
                        .checked_sub(prefix)
                        .and_then(|index| old_keys.get(index))
                        .and_then(|key| new_keys.iter().position(|candidate| candidate == key));
                    state.splice(
                        prefix + common..prefix + old_keys.len() - suffix,
                        new_keys.len() - common - suffix,
                    );
                    state.clone().with_uniform_item_height(px(49.));
                    if let Some(index) = anchor {
                        state.scroll_to(gpui::ListOffset {
                            item_ix: prefix + index,
                            offset_in_item: top.offset_in_item,
                        });
                    }
                }
                *old_rows = rows.clone();
            }
            *old_files = files.clone();
        }
        let state = cache.as_ref().unwrap().2.clone();
        let link = state.clone();
        let weak = cx.entity().downgrade();
        let list = gpui::list(state.clone(), move |index, _, cx| {
            weak.update(cx, |this, cx| {
                render_row(this, index, Some((link.clone(), index)), cx)
            })
            .unwrap_or_else(|_| div().into_any_element())
        })
        .flex_1()
        .min_h_0()
        .w_full();
        let weak = cx.entity().downgrade();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .on_children_prepainted(move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    if let Some((_, _, state, _)) = &*this.edits_ui.grouped_list.borrow() {
                        let width = state.viewport_bounds().size.width;
                        if width > px(0.) && this.edits_ui.grouped_width.replace(width) != width {
                            state.clone().with_uniform_item_height(px(49.));
                            let id = cx.entity_id();
                            cx.defer(move |cx| cx.notify(id));
                        }
                    }
                });
            })
            .id("grouped-workspace-files")
            .relative()
            .pr(px(12.))
            .child(list)
            .child(
                Scrollbar::vertical_list(&state, &self.edits_ui.navigation_bar)
                    .theme(self.theme)
                    .render()
                    .absolute()
                    .right_0()
                    .top_0(),
            )
    }
    fn workspace_watch_active(&self) -> bool {
        self.edits_ui.open
            && self.edits_ui.workspace_mode
            && self.edits_ui.workspace_root == self.diff_repository_root()
            && self.edits_ui.workspace_watch_root.is_some()
    }

    fn diff_prefetch_active(&self) -> bool {
        self.page == crate::Page::Chat
            && self.edits_ui.open
            && self.edits_ui.workspace_mode
            && !self.edits_ui.workspace_sources.is_empty()
            && self.edits_ui.workspace_root == self.diff_repository_root()
    }

    pub(super) fn sync_workspace_diff_prefetch(&mut self, cx: &mut Context<Self>) {
        let mut prefetch = self.edits_ui.diff_prefetch.borrow_mut();
        prefetch.update_inventory(&self.edits_ui.workspace_files, self.edits_ui.expanded);
        if !self.diff_prefetch_active() {
            return;
        }
        if self.edits_ui.diff_prefetch_task.is_some() || !prefetch.has_pending() {
            return;
        }
        drop(prefetch);
        self.edits_ui.diff_prefetch_task = Some(cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor().timer(DIFF_PREFETCH_DELAY).await;
                let Ok(next) = weak.update(cx, |this, cx| {
                    if !this.diff_prefetch_active() {
                        return None;
                    }
                    let mut prefetch = this.edits_ui.diff_prefetch.borrow_mut();
                    if !prefetch.has_pending() {
                        return None;
                    }
                    if this.edits_ui.workspace_task.is_some()
                        || this.edits_ui.stage_task.is_some()
                        || this.edits_ui.workspace_stats_task.is_some()
                        || this
                            .edits_ui
                            .file_views
                            .borrow()
                            .values()
                            .any(|view| view.read(cx).load_task.is_some())
                    {
                        return Some(None);
                    }
                    prefetch
                        .next_source(&this.edits_ui.workspace_sources)
                        .map(|(key, source)| {
                            Some((
                                key,
                                cx.background_executor()
                                    .spawn_with_priority(Priority::Low, async move {
                                        prepare_workspace_diff(&source, false, key.2)
                                    }),
                            ))
                        })
                }) else {
                    return;
                };
                let Some(next) = next else { break };
                let Some((key, work)) = next else { continue };
                let result = work.await;
                if weak
                    .update(cx, |this, _| {
                        this.edits_ui.diff_prefetch.borrow_mut().finish(key, result)
                    })
                    .is_err()
                {
                    return;
                }
            }
            let _ = weak.update(cx, |this, _| {
                if let Some(task) = this.edits_ui.diff_prefetch_task.take() {
                    task.detach();
                }
            });
        }));
    }

    pub(super) fn sync_workspace_watch(&mut self, cx: &mut Context<Self>) {
        if !self.workspace_watch_active() {
            self.edits_ui.workspace_watch_task = None;
            self.edits_ui.workspace_probe_task = None;
            self.edits_ui.workspace_sync_debounce = None;
            return;
        }
        if self.edits_ui.workspace_watch_task.is_some() {
            return;
        }
        self.edits_ui.workspace_watch_task = Some(cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                if !weak
                    .update(cx, |this, cx| {
                        if !this.workspace_watch_active() {
                            return false;
                        }
                        this.probe_workspace_repository(cx);
                        true
                    })
                    .unwrap_or(false)
                {
                    break;
                }
            }
        }));
    }

    fn probe_workspace_repository(&mut self, cx: &mut Context<Self>) {
        if !self.workspace_watch_active()
            || self.edits_ui.workspace_probe_task.is_some()
            || self.edits_ui.workspace_task.is_some()
            || self.edits_ui.stage_task.is_some()
            || self.edits_ui.commit_task.is_some()
        {
            return;
        }
        let root = self.edits_ui.workspace_watch_root.clone().unwrap();
        let project = self.diff_repository_root();
        let observed = self.edits_ui.workspace_stamp.clone();
        let work = cx
            .background_executor()
            .spawn(async move { crate::workspace_diff::repository_stamp(&root) });
        self.edits_ui.workspace_probe_task = Some(cx.spawn(async move |weak, cx| {
            let stamp = work.await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.edits_ui.workspace_probe_task.take() {
                    task.detach();
                }
                if !this.workspace_watch_active()
                    || this.diff_repository_root() != project
                    || this.edits_ui.stage_task.is_some()
                    || this.edits_ui.commit_task.is_some()
                    || this.edits_ui.workspace_stamp != observed
                {
                    return;
                }
                if this.edits_ui.workspace_stamp != stamp {
                    this.edits_ui.workspace_stamp = stamp;
                    this.load_workspace_edits(true, cx);
                }
            });
        }));
    }

    /// Coalesce terminal output bursts (including a final shell prompt). Silent
    /// commands still trigger once on Enter; long commands trigger again on exit.
    pub(super) fn workspace_terminal_activity(
        &mut self,
        project: &PathBuf,
        cx: &mut Context<Self>,
    ) {
        if !self.edits_ui.open
            || !self.edits_ui.workspace_mode
            || self.diff_repository_root().as_ref() != Some(project)
            || crate::ssh_project::is_remote(project)
        {
            return;
        }
        let project = project.clone();
        self.edits_ui.workspace_sync_debounce = Some(cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(900))
                .await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.edits_ui.workspace_sync_debounce.take() {
                    task.detach();
                }
                if this.edits_ui.open
                    && this.edits_ui.workspace_mode
                    && this.diff_repository_root().as_ref() == Some(&project)
                {
                    this.load_workspace_edits(true, cx);
                }
            });
        }));
    }

    pub(super) fn repository_change_busy(&self, cx: &App) -> bool {
        self.edits_ui.stage_task.is_some()
            || self.edits_ui.commit_task.is_some()
            || self.edits_ui.file_action_task.is_some()
            || !self.edits_ui.stage_queue.is_empty()
            || self.edits_ui.undo_task.is_some()
            || self
                .edits_ui
                .conflicts
                .iter()
                .any(|view| view.read(cx).task.is_some())
    }

    pub(super) fn refresh_workspace_edits(&mut self, cx: &mut Context<Self>) {
        self.load_workspace_edits(false, cx);
    }

    fn load_workspace_edits(&mut self, background: bool, cx: &mut Context<Self>) {
        if self.edits_ui.commit_task.is_some() {
            return;
        }
        let root = self.diff_repository_root();
        if self.edits_ui.stage_task.is_some() && self.edits_ui.workspace_root == root {
            if background {
                self.edits_ui.workspace_resync_pending = true;
            } else {
                self.edits_ui.stage_refresh_pending = true;
            }
            return;
        }
        if self.edits_ui.workspace_root != root {
            self.edits_ui.stage_refresh_pending = false;
            self.edits_ui.workspace_resync_pending = false;
        }
        if self.edits_ui.workspace_task.is_some() && self.edits_ui.workspace_root == root {
            if background {
                self.edits_ui.workspace_resync_pending = true;
            }
            return;
        }
        self.edits_ui.workspace_resync_pending = false;
        // This snapshot also consumes terminal changes already observed before
        // it started; output arriving during the read schedules the next pass.
        self.edits_ui.workspace_sync_debounce = None;
        self.edits_ui.workspace_background_refresh = background;
        self.edits_ui.workspace_revision += 1;
        let revision = self.edits_ui.workspace_revision;
        self.edits_ui.workspace_stats_task = None;
        self.edits_ui.workspace_conflicts_task = None;
        if self.edits_ui.workspace_root != root {
            self.edits_ui.conflicts.clear();
            self.edits_ui
                .file_views
                .borrow_mut()
                .retain(|key, _| !key.starts_with("workspace:"));
            self.edits_ui.workspace_files = Rc::new(Vec::new());
            self.edits_ui.workspace_sources = Rc::new(HashMap::new());
            self.edits_ui.workspace_branch = None;
            self.edits_ui.file_menu = None;
            self.edits_ui.file_action = None;
            self.edits_ui.workspace_watch_root = None;
            self.edits_ui.workspace_stamp = None;
            self.edits_ui.workspace_conflict_versions.clear();
            self.edits_ui.virtual_files.borrow_mut().take();
            self.edits_ui.workspace_notes.clear();
        }
        self.edits_ui.workspace_root = root.clone();
        self.edits_ui.workspace_error = None;
        let Some(root) = root else {
            self.edits_ui.workspace_files = Rc::new(Vec::new());
            self.edits_ui.workspace_notes.clear();
            self.edits_ui.workspace_error = Some("请先选择项目。".into());
            self.edits_ui.workspace_task = None;
            cx.notify();
            return;
        };
        let requested_root = root.clone();
        let work = cx.background_executor().spawn(async move {
            if crate::ssh_project::is_remote(&root) {
                provider::Job::start(async move { edits::remote_workspace_changes(&root).await })?
                    .result()
                    .await
                    .map(|(files, notes)| (files, notes, HashMap::new(), None))
            } else {
                let inventory = crate::workspace_diff::inventory(&root)?;
                Ok((
                    inventory.files,
                    Vec::new(),
                    inventory.sources,
                    Some((
                        inventory.branch,
                        inventory.root,
                        inventory.head,
                        !inventory.conflict_labels.is_empty(),
                        inventory.conflict_versions,
                        inventory.stamp,
                    )),
                ))
            }
        });
        self.edits_ui.workspace_task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if this.diff_repository_root().as_ref() != Some(&requested_root)
                    || this.edits_ui.workspace_root.as_ref() != Some(&requested_root)
                    || this.edits_ui.workspace_revision != revision
                {
                    return;
                }
                if let Some(task) = this.edits_ui.workspace_task.take() {
                    task.detach();
                }
                match result {
                    Ok((mut files, notes, mut sources, statistics)) => {
                        let old_files: HashMap<_, _> = this
                            .edits_ui
                            .workspace_files
                            .iter()
                            .map(|file| (&file.path, file))
                            .collect();
                        for file in &mut files {
                            if let (Some(old), Some(source), Some(previous)) = (
                                old_files.get(&file.path),
                                sources.get_mut(&file.path),
                                this.edits_ui.workspace_sources.get(&file.path),
                            ) {
                                if source.same_version(previous) {
                                    *file = (*old).clone();
                                    source.staged_stats = previous.staged_stats;
                                    source.unstaged_stats = previous.unstaged_stats;
                                    source.stats_ready = previous.stats_ready;
                                }
                            }
                        }
                        let valid: std::collections::HashSet<_> =
                            files.iter().map(|file| file.path.clone()).collect();
                        this.edits_ui.file_views.borrow_mut().retain(|key, view| {
                            !key.starts_with("workspace:")
                                || valid.contains(&view.read(cx).file.path)
                        });
                        this.edits_ui.workspace_files = Rc::new(files);
                        this.edits_ui.workspace_sources = Rc::new(sources);
                        if !background || this.edits_ui.conflicts.is_empty() {
                            this.edits_ui.workspace_notes = notes;
                        }
                        if let Some((branch, root, head, has_conflicts, conflict_versions, stamp)) =
                            statistics
                        {
                            this.edits_ui.workspace_branch = branch;
                            this.edits_ui.workspace_watch_root = Some(root.clone());
                            this.edits_ui.workspace_stamp = stamp;
                            let changed =
                                conflict_versions != this.edits_ui.workspace_conflict_versions;
                            this.edits_ui.workspace_conflict_versions = conflict_versions;
                            if has_conflicts && (!background || changed) {
                                this.start_workspace_conflicts(root.clone(), revision, cx);
                            } else if !has_conflicts {
                                this.edits_ui.conflicts.clear();
                                this.edits_ui.workspace_notes.clear();
                            }
                            this.start_workspace_statistics(root, head, revision, cx);
                        } else {
                            this.edits_ui.conflicts.clear();
                        }
                    }
                    Err(error) => {
                        this.edits_ui.workspace_error = Some(error);
                    }
                }
                if std::mem::take(&mut this.edits_ui.workspace_resync_pending) {
                    this.load_workspace_edits(true, cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn start_workspace_conflicts(&mut self, root: PathBuf, revision: u64, cx: &mut Context<Self>) {
        let work_root = root.clone();
        let drafts: Vec<_> = self
            .edits_ui
            .conflicts
            .iter()
            .filter_map(|view| {
                let view = view.read(cx);
                (view.editor.read(cx).content() != view.file.original || view.task.is_some())
                    .then(|| view.file.path.clone())
            })
            .collect();
        let work = cx.background_executor().spawn(async move {
            let (files, mut notes) = crate::merge_conflicts::load(&work_root)?;
            let mut pending = Vec::new();
            let mut resolved = false;
            for file in files {
                if file.limitation.is_none()
                    && !drafts.contains(&file.path)
                    && !crate::merge_conflicts::has_markers(&file.original)
                {
                    match crate::merge_conflicts::save(&file, &file.original) {
                        Ok(()) => {
                            resolved = true;
                            continue;
                        }
                        Err(error) => notes.push(format!("{}：自动标记失败：{error}", file.label)),
                    }
                }
                pending.push(file);
            }
            Ok::<_, String>((pending, notes, resolved))
        });
        self.edits_ui.workspace_conflicts_task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if this.edits_ui.workspace_revision != revision
                    || this.diff_repository_root() != this.edits_ui.workspace_root
                {
                    return;
                }
                if let Some(task) = this.edits_ui.workspace_conflicts_task.take() {
                    task.detach();
                }
                match result {
                    Ok((files, notes, resolved)) => {
                        let workspace = cx.entity().downgrade();
                        if resolved {
                            this.load_workspace_edits(true, cx);
                        }
                        let existing: HashMap<_, _> = this
                            .edits_ui
                            .conflicts
                            .iter()
                            .map(|view| (view.read(cx).file.path.clone(), view.clone()))
                            .collect();
                        this.edits_ui.conflicts = files
                            .into_iter()
                            .map(|file| {
                                existing
                                    .get(&file.path)
                                    .filter(|view| view.read(cx).file == file)
                                    .cloned()
                                    .unwrap_or_else(|| {
                                        let view = cx.new(|cx| {
                                            ConflictView::new(
                                                file,
                                                root.clone(),
                                                Some(workspace.clone()),
                                                this.theme,
                                                cx,
                                            )
                                        });
                                        watch_row(&view, view.read(cx).list_link.clone(), cx);
                                        view
                                    })
                            })
                            .collect();
                        this.edits_ui.workspace_notes.extend(notes);
                    }
                    Err(error) => {
                        this.edits_ui.workspace_error =
                            Some(format!("冲突详情读取失败，可继续查看其他文件：{error}"))
                    }
                }
                cx.notify();
            });
        }));
    }

    fn start_workspace_statistics(
        &mut self,
        root: PathBuf,
        _head: Option<String>,
        revision: u64,
        cx: &mut Context<Self>,
    ) {
        // Large vendor updates stay metadata-only until a file is opened.
        if self.edits_ui.workspace_files.is_empty()
            || self.edits_ui.workspace_files.len() > crate::workspace_diff::EAGER_STATS_LIMIT
        {
            return;
        }
        let paths: Vec<_> = self
            .edits_ui
            .workspace_sources
            .values()
            .filter(|source| {
                !source.stats_ready
                    && source.status != crate::workspace_diff::FileStatus::Untracked
                    && source.status != crate::workspace_diff::FileStatus::Renamed
            })
            .map(|source| source.label.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        let versions: std::collections::HashSet<_> = self
            .edits_ui
            .workspace_files
            .iter()
            .map(|file| file.id)
            .collect();
        let work = cx
            .background_executor()
            .spawn(async move { crate::workspace_diff::scoped_statistics(&root, &paths) });
        self.edits_ui.workspace_stats_task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if this.edits_ui.workspace_revision != revision
                    || this.diff_repository_root() != this.edits_ui.workspace_root
                {
                    return;
                }
                if let Some(task) = this.edits_ui.workspace_stats_task.take() {
                    task.detach();
                }
                match result {
                    Ok((staged, unstaged)) => {
                        let unchanged: std::collections::HashSet<_> = this
                            .edits_ui
                            .workspace_files
                            .iter()
                            .filter(|file| versions.contains(&file.id))
                            .map(|file| file.path.clone())
                            .collect();
                        for source in Rc::make_mut(&mut this.edits_ui.workspace_sources)
                            .values_mut()
                            .filter(|source| unchanged.contains(&source.path))
                        {
                            source.stats_ready = true;
                            source.staged_stats = (source.status
                                != crate::workspace_diff::FileStatus::Renamed)
                                .then(|| staged.get(&source.label).copied())
                                .flatten();
                            source.unstaged_stats = unstaged.get(&source.label).copied();
                        }
                    }
                    Err(error) => {
                        this.edits_ui.workspace_error =
                            Some(format!("行数统计失败，可展开文件按需查看：{error}"))
                    }
                }
                cx.notify();
            });
        }));
    }
}

/// Drafts save and stage automatically once all conflict markers are removed.
pub(super) struct ConflictView {
    tree_depth: Option<usize>,
    list_link: ListLink,
    file: crate::merge_conflicts::Conflict,
    project: PathBuf,
    workspace: Option<gpui::WeakEntity<AgentWorkspace>>,
    theme: ThemePreset,
    language: &'static str,
    expanded: bool,
    manual_edit: bool,
    /// Conflict hunks in the pristine file, shown in the collapsed badge.
    count: usize,
    /// Editable merged result with line numbers and syntax highlighting.
    editor: Entity<CodeCard>,
    /// Scroll chain for the capped merge-result viewport inside the list.
    result_scroll: gpui::ScrollHandle,
    detail_scroll: Option<gpui::ScrollHandle>,
    reveal_revision: Option<u64>,
    reveal_pending: Rc<Cell<bool>>,
    auto_resolve_task: Option<Task<()>>,
    /// Read-only highlighted previews keyed by (hunk index, side).
    sides: RefCell<HashMap<(usize, usize), Entity<CodeCard>>>,
    notice: Option<String>,
    task: Option<Task<()>>,
}

pub(super) struct ConflictResolved;

impl gpui::EventEmitter<ConflictResolved> for ConflictView {}

/// Build the conflict request: instruction text plus the file snapshot as a
/// separate reference attachment, so history shows a compact highlighted diff
/// reference with its own icon instead of an inline JSON blob.
pub(crate) fn conflict_submission(
    file: &crate::merge_conflicts::Conflict,
) -> gpui_glint::components::ComposerSubmission {
    let (mut text, snapshot) = crate::merge_conflicts::agent_request(file);
    let mut references = Vec::new();
    if let Some((name, content)) = snapshot {
        let mut reference =
            gpui_glint::components::ComposerReference::conflict_snapshot(name, &content);
        reference.token = reference.label();
        text.push('\n');
        text.push_str(&reference.token);
        references.push(reference);
    }
    gpui_glint::components::ComposerSubmission {
        text: text.into(),
        references,
        images: Vec::new(),
    }
}

impl ConflictView {
    fn file_header(&self) -> gpui::Div {
        let p = self.theme.palette();
        div()
            .debug_selector(|| "current-conflict-file".into())
            .h(px(38.))
            .flex_shrink_0()
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .text_size(px(12.))
            .text_color(rgb(p.text))
            .child(Icon::GitConflict.render(14.).text_color(rgb(p.accent)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.file.label.clone()),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("正在解决冲突"),
            )
    }

    fn ask_agent(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.as_ref() else {
            return;
        };
        if self.task.is_some() {
            return;
        }
        if self.editor.read(cx).content() != self.file.original {
            self.notice =
                Some("存在未保存的合并草稿，请先保存或重置草稿，避免 Agent 覆盖手动修改。".into());
            self.expanded = true;
            cx.notify();
            return;
        }
        let submission = conflict_submission(&self.file);
        let result = workspace.update(cx, |workspace, cx| -> Result<(), String> {
            if workspace.diff_repository_root().as_ref() != Some(&self.project) {
                return Err("项目已切换，请回到原项目后重试".into());
            }
            if workspace.any_run_busy()
                || workspace.edits_ui.undo_task.is_some()
                || !workspace.queued_requests.is_empty()
            {
                return Err("Agent 正在处理任务，请完成当前任务后重试".into());
            }
            if !workspace.active_model_enabled() {
                return Err("请先连接并启用一个 Agent 模型".into());
            }
            workspace.send_to(
                workspace.history.active,
                submission,
                workspace.provider.clone(),
                false,
                cx,
            );
            if !workspace.session_busy(workspace.history.active) {
                return Err(workspace
                    .provider_error
                    .clone()
                    .unwrap_or_else(|| "Agent 请求未启动，请检查当前模型配置".into()));
            }
            workspace.page = crate::Page::Chat;
            workspace.edits_ui.expanded = false;
            cx.notify();
            Ok(())
        });
        self.notice = Some(match result {
            Ok(Ok(())) => "已请求 Agent 解决此文件；完成后刷新工作区审阅。".into(),
            Ok(Err(error)) => error,
            Err(_) => "会话已关闭".into(),
        });
        self.expanded = true;
        cx.notify();
    }

    pub(super) fn new(
        file: crate::merge_conflicts::Conflict,
        project: PathBuf,
        workspace: Option<gpui::WeakEntity<AgentWorkspace>>,
        theme: ThemePreset,
        cx: &mut Context<Self>,
    ) -> Self {
        let count = crate::merge_conflicts::hunks(&file.original)
            .map(|hunks| hunks.len())
            .unwrap_or(0);
        let language = crate::edits::language(&file.path);
        let editor = cx.new(|cx| {
            CodeCard::new(cx, language, file.original.clone())
                .theme(theme)
                .show_header(false)
        });
        cx.subscribe(&editor, |this, _, event: &CodeCardEvent, cx| {
            if matches!(event, CodeCardEvent::Changed(_)) {
                this.schedule_auto_resolve(cx);
                cx.notify();
            }
        })
        .detach();
        Self {
            list_link: Default::default(),
            tree_depth: None,
            file,
            project,
            workspace,
            theme,
            language,
            expanded: false,
            manual_edit: false,
            count,
            editor,
            result_scroll: gpui::ScrollHandle::new(),
            detail_scroll: None,
            reveal_revision: None,
            reveal_pending: Rc::new(Cell::new(false)),
            auto_resolve_task: None,
            sides: Default::default(),
            notice: None,
            task: None,
        }
    }

    pub(super) fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            self.editor
                .update(cx, |editor, cx| editor.set_theme(theme, cx));
            cx.notify();
        }
    }

    pub(super) fn reveal(
        &mut self,
        scroll: gpui::ScrollHandle,
        revision: u64,
        cx: &mut Context<Self>,
    ) {
        self.detail_scroll = Some(scroll);
        if self.reveal_revision != Some(revision) {
            self.reveal_revision = Some(revision);
            self.reveal_pending.set(true);
            self.expanded = true;
            cx.notify();
        }
    }

    fn schedule_auto_resolve(&mut self, cx: &mut Context<Self>) {
        self.auto_resolve_task = None;
        if self.file.limitation.is_some()
            || crate::merge_conflicts::has_markers(self.editor.read(cx).content())
        {
            return;
        }
        let timer = cx
            .background_executor()
            .timer(std::time::Duration::from_millis(600));
        self.auto_resolve_task = Some(cx.spawn(async move |weak, cx| {
            timer.await;
            let _ = weak.update(cx, |this, cx| {
                if !crate::merge_conflicts::has_markers(this.editor.read(cx).content()) {
                    this.save(cx);
                }
            });
        }));
    }

    fn choose(&mut self, index: usize, choice: usize, cx: &mut Context<Self>) {
        if self.task.is_some() {
            return;
        }
        let text = self.editor.read(cx).content().to_owned();
        if let Ok(hunks) = crate::merge_conflicts::hunks(&text) {
            if let Some(hunk) = hunks.get(index) {
                let replacement = match choice {
                    0 => hunk.ours.clone(),
                    1 => hunk.theirs.clone(),
                    _ => format!("{}{}", hunk.ours, hunk.theirs),
                };
                let mut result = text;
                result.replace_range(hunk.range.clone(), &replacement);
                self.editor
                    .update(cx, |editor, cx| editor.set_content(result, cx));
                self.notice = None;
                cx.notify();
            }
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.task.is_some() {
            return;
        }
        let file = self.file.clone();
        let result = self.editor.read(cx).content().to_owned();
        let work = cx
            .background_executor()
            .spawn(async move { crate::merge_conflicts::save(&file, &result) });
        self.task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.task.take() {
                    task.detach();
                }
                match result {
                    Ok(()) => {
                        this.notice = Some("已保存并暂存，冲突已解决".into());
                        cx.emit(ConflictResolved);
                        if let Some(workspace) = &this.workspace {
                            let _ = workspace.update(cx, |workspace, cx| {
                                if workspace.diff_repository_root().as_ref() == Some(&this.project)
                                    && workspace
                                        .edits_ui
                                        .conflicts
                                        .iter()
                                        .any(|view| view.entity_id() == weak.entity_id())
                                {
                                    workspace.refresh_workspace_edits(cx);
                                }
                            });
                        }
                    }
                    Err(error) => this.notice = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Actions and both colored sides live in the same source stream.
    fn hunk_card(
        &self,
        index: usize,
        hunk: &crate::merge_conflicts::Hunk,
        text: &str,
        first: usize,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let raw = &text[hunk.range.clone()];
        let split = raw
            .split_inclusive('\n')
            .scan(0, |offset, line| {
                let start = *offset;
                *offset += line.len();
                Some((start, line))
            })
            .find(|(_, line)| line.starts_with("======="))
            .map_or(raw.len(), |(offset, _)| offset);
        let p = self.theme.palette();
        let mut actions = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .px_3()
            .py_2()
            .bg(rgb(p.surface_subtle))
            .border_b_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .mr_2()
                    .child(format!("冲突 {} · L{first}", index + 1)),
            );
        for (choice, label) in ["采用当前侧", "采用传入侧", "保留双方"]
            .into_iter()
            .enumerate()
        {
            actions = actions.child(
                Button::new(label)
                    .size(ButtonSize::Small)
                    .kind(ButtonKind::Tinted)
                    .theme(self.theme)
                    .when(busy, |button| button.disabled())
                    .render()
                    .debug_selector(move || format!("merge-choice:{index}:{choice}"))
                    .on_click(cx.listener(move |this, _, _, cx| this.choose(index, choice, cx))),
            );
        }
        actions = actions.child(div().flex_1()).child(
            Button::new("Agent 解决")
                .icon(Icon::Sparkles)
                .size(ButtonSize::Small)
                .kind(ButtonKind::Plain)
                .theme(self.theme)
                .when(busy, |button| button.disabled())
                .render()
                .on_click(cx.listener(|this, _, _, cx| this.ask_agent(cx))),
        );
        let ours = self.side_preview(index, 0, Self::preview_text(&raw[..split]), cx);
        let theirs = self.side_preview(index, 1, Self::preview_text(&raw[split..]), cx);
        ours.update(cx, |card, cx| {
            card.set_inline_preview(
                first,
                Some(if self.theme.is_dark() {
                    0x193d32
                } else {
                    0xd9efe3
                }),
                cx,
            )
        });
        theirs.update(cx, |card, cx| {
            card.set_inline_preview(
                first + raw[..split].lines().count(),
                Some(if self.theme.is_dark() {
                    0x20384f
                } else {
                    0xdcedfc
                }),
                cx,
            )
        });
        let scroll = self.detail_scroll.clone();
        let pending = self.reveal_pending.clone();
        let id = cx.entity_id();
        div()
            .on_children_prepainted(move |bounds, _, cx| {
                if index == 0 && pending.get() {
                    if let (Some(scroll), Some(target)) = (&scroll, bounds.first()) {
                        if scroll.bounds().size.height > px(0.) {
                            pending.set(false);
                            let offset = scroll.offset();
                            scroll.set_offset(point(
                                offset.x,
                                (offset.y + scroll.bounds().origin.y - target.origin.y + px(8.))
                                    .min(px(0.)),
                            ));
                            cx.defer(move |cx| cx.notify(id));
                        }
                    }
                }
            })
            .debug_selector(move || format!("merge-hunk:{index}"))
            .my_2()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(p.border))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(actions)
            .child(
                div()
                    .debug_selector(move || format!("merge-side:{index}:0"))
                    .child(ours),
            )
            .child(
                div()
                    .debug_selector(move || format!("merge-side:{index}:1"))
                    .child(theirs),
            )
    }

    fn preview_text(text: &str) -> &str {
        text.strip_suffix("\r\n")
            .or_else(|| text.strip_suffix('\n'))
            .unwrap_or(text)
    }

    fn context_preview(
        &self,
        index: usize,
        text: &str,
        first: usize,
        cx: &mut Context<Self>,
    ) -> Entity<CodeCard> {
        let card = self.side_preview(usize::MAX, index, Self::preview_text(text), cx);
        card.update(cx, |card, cx| card.set_inline_preview(first, None, cx));
        card
    }

    /// Cached read-only highlighted preview for one hunk side.
    fn side_preview(
        &self,
        index: usize,
        side: usize,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Entity<CodeCard> {
        let cached = self.sides.borrow_mut().get(&(index, side)).cloned();
        if let Some(card) = cached {
            card.update(cx, |card, cx| card.set_theme(self.theme, cx));
            if card.read(cx).content() != text {
                card.update(cx, |card, cx| card.sync_content(text, cx));
            }
            return card;
        }
        let card = cx.new(|cx| {
            CodeCard::new(cx, self.language, text)
                .theme(self.theme)
                .show_header(false)
                .read_only(true)
        });
        self.sides.borrow_mut().insert((index, side), card.clone());
        card
    }

    /// Styled single-line status message.
    fn notice_banner(&self, notice: String) -> gpui::AnyElement {
        let p = self.theme.palette();
        div()
            .px_2()
            .py_1()
            .rounded(px(6.))
            .bg(rgb(p.surface_subtle))
            .text_size(px(12.))
            .child(notice)
            .into_any_element()
    }
}

impl Render for ConflictView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let colors = if self.theme.is_dark() {
            DiffColors::DARK
        } else {
            DiffColors::LIGHT
        };
        let (status_label, status_color) = conflict_status_style(self.file.kind(), self.theme);
        let busy = self.task.is_some();
        let remaining = crate::merge_conflicts::hunks(self.editor.read(cx).content())
            .map(|hunks| hunks.len())
            .unwrap_or(self.count);
        let badge = if remaining > 0 {
            format!("{status_label} · {remaining} 处待解决")
        } else if crate::merge_conflicts::has_markers(self.editor.read(cx).content()) {
            "冲突标记待检查".to_owned()
        } else {
            "无冲突标记".to_owned()
        };
        let mut row = div()
            .w_full()
            .flex_shrink_0()
            .border_b_1()
            .border_color(rgb(p.border))
            .flex()
            .flex_col()
            .child(
                div()
                    .id("conflict-disclosure")
                    .w_full()
                    .debug_selector(|| "merge-conflict-disclosure".into())
                    .px_3()
                    .pl(px(12. + self.tree_depth.unwrap_or(0) as f32 * 16.))
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .child(Icon::disclosure(13., self.expanded))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .text_color(rgb(status_color))
                            .child(if self.tree_depth.is_some() {
                                self.file
                                    .label
                                    .rsplit('/')
                                    .next()
                                    .unwrap_or(&self.file.label)
                                    .to_owned()
                            } else {
                                self.file.label.clone()
                            }),
                    )
                    .child(
                        div()
                            .px(px(6.))
                            .py(px(2.))
                            .rounded(px(5.))
                            .bg(rgb(p.surface_subtle))
                            .text_size(px(11.))
                            .text_color(rgb(status_color))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(Icon::GitConflict.render(11.))
                            .child(badge),
                    )
                    .when(self.workspace.is_some(), |row| {
                        row.child(
                            IconButton::new(Icon::Sparkles, "让 Agent 解决此文件冲突")
                                .plain()
                                .theme(self.theme)
                                .render()
                                .debug_selector(|| "agent-resolve-conflict".into())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.ask_agent(cx);
                                })),
                        )
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.expanded = !this.expanded;
                        cx.notify();
                    })),
            );
        if self.expanded {
            if let Some(reason) = &self.file.limitation {
                let notice = self.notice.clone();
                return row.child(
                    div()
                        .px_3()
                        .pb_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child(reason.clone())
                        .child(div().child("可点击星芒图标让 Agent 按需读取并解决冲突。"))
                        .when_some(notice, |content, notice| {
                            content.child(self.notice_banner(notice))
                        }),
                );
            }
            let text = self.editor.read(cx).content().to_owned();
            let mut content = div().px_3().pb_3().flex().flex_col().gap_2();
            let mut show_editor = self.manual_edit;
            match crate::merge_conflicts::hunks(&text) {
                Ok(hunks) => {
                    self.sides.borrow_mut().retain(|(hunk, context), _| {
                        !hunks.is_empty()
                            && ((*hunk == usize::MAX && *context <= hunks.len())
                                || *hunk < hunks.len())
                    });
                    if hunks.is_empty() && crate::merge_conflicts::has_markers(&text) {
                        show_editor = true;
                        content = content
                            .child(self.notice_banner("冲突标记不完整，请手动编辑结果".into()));
                    } else if hunks.is_empty() {
                        show_editor = true;
                        content = content.child(
                            div()
                                .debug_selector(|| "merge-resolved".into())
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_2()
                                .py_1()
                                .rounded(px(8.))
                                .bg(rgb(colors.added_row))
                                .text_size(px(12.))
                                .text_color(rgb(colors.added_marker))
                                .child(Icon::CheckCircle.render(13.))
                                .child("已无冲突标记，将自动保存并标记解决。"),
                        );
                    } else {
                        let mut offset = 0;
                        let mut first = 1;
                        for (index, hunk) in hunks.iter().enumerate() {
                            if hunk.range.start > offset {
                                let context = &text[offset..hunk.range.start];
                                content =
                                    content.child(self.context_preview(index, context, first, cx));
                                first += context.bytes().filter(|b| *b == b'\n').count();
                            }
                            content =
                                content.child(self.hunk_card(index, hunk, &text, first, busy, cx));
                            first += text[hunk.range.clone()]
                                .bytes()
                                .filter(|b| *b == b'\n')
                                .count();
                            offset = hunk.range.end;
                        }
                        if offset < text.len() {
                            content = content.child(self.context_preview(
                                hunks.len(),
                                &text[offset..],
                                first,
                                cx,
                            ));
                        }
                    }
                }
                Err(error) => {
                    show_editor = true;
                    content = content.child(self.notice_banner(error));
                }
            }
            let dirty = !busy && text != self.file.original;
            let mut result_header = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("合并结果"),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child("冲突消除后自动保存并暂存"),
                )
                .child(
                    Button::new(if self.manual_edit {
                        "收起编辑"
                    } else {
                        "手动编辑"
                    })
                    .size(ButtonSize::Small)
                    .kind(ButtonKind::Plain)
                    .theme(self.theme)
                    .when(busy, |button| button.disabled())
                    .render()
                    .debug_selector(|| "toggle-merge-editor".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.manual_edit = !this.manual_edit;
                        cx.notify();
                    })),
                );
            if dirty {
                result_header = result_header.child(div().flex_1()).child(
                    Button::new("重置草稿")
                        .kind(ButtonKind::Plain)
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "reset-merge-draft".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.editor.update(cx, |editor, cx| {
                                editor.set_content(this.file.original.clone(), cx)
                            });
                            this.notice = None;
                            cx.notify();
                        })),
                );
            }
            content = content
                .child(result_header)
                .when(!busy && show_editor, |content| {
                    content.child(
                        div()
                            .debug_selector(|| "merge-result-editor".into())
                            .max_h(px(360.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(rgb(p.border))
                            .id("merge-result-scroll")
                            .overflow_y_scroll()
                            .track_scroll(&self.result_scroll)
                            .chain_scroll_y(&self.result_scroll)
                            .child(self.editor.clone()),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child(
                                    "当前侧 = ours · 传入侧 = theirs（rebase 时当前侧为变基目标）",
                                ),
                        )
                        .child(
                            Button::new(if busy {
                                "正在保存…"
                            } else {
                                "保存并标记已解决"
                            })
                            .size(ButtonSize::Small)
                            .theme(self.theme)
                            .when(
                                busy || crate::merge_conflicts::has_markers(&text),
                                |button| button.disabled(),
                            )
                            .render()
                            .debug_selector(|| "save-merge-conflict".into())
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                        ),
                );
            if let Some(notice) = self.notice.clone() {
                content = content.child(self.notice_banner(notice));
            }
            row = row.child(content);
        }
        row
    }
}

/// Each file owns its disclosure state. Closed files never construct a diff;
/// opening another file leaves all existing disclosures and scroll positions intact.
struct FileDiff {
    status: Option<crate::workspace_diff::FileStatus>,
    detail: bool,
    tree_depth: Option<usize>,
    list_link: ListLink,
    source: Option<crate::workspace_diff::Source>,
    prefetch: Option<Rc<RefCell<DiffPrefetch>>>,
    loaded: Option<Change>,
    load_task: Option<Task<()>>,
    load_error: Option<String>,
    manual_load_available: bool,
    prepared: Option<gpui_glint::components::PreparedDiff>,
    workspace: Option<gpui::WeakEntity<AgentWorkspace>>,
    undo_visible: bool,
    undo_enabled: bool,
    file: Change,
    theme: ThemePreset,
    expanded: bool,
    side_by_side: bool,
    card: Option<Entity<DiffCard>>,
    reveal_revision: u64,
}

impl FileDiff {
    fn new(file: Change, theme: ThemePreset) -> Self {
        Self {
            detail: false,
            status: None,
            tree_depth: None,
            list_link: Default::default(),
            source: None,
            prefetch: None,
            loaded: None,
            load_task: None,
            load_error: None,
            manual_load_available: false,
            prepared: None,
            workspace: None,
            undo_visible: false,
            undo_enabled: false,
            file,
            theme,
            expanded: false,
            side_by_side: false,
            card: None,
            reveal_revision: 0,
        }
    }

    fn set_file(&mut self, file: &Change, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.file.id != file.id {
            self.file = file.clone();
            self.card = None;
            self.loaded = None;
            self.prepared = None;
            self.manual_load_available = false;
            self.load_error = None;
            self.load_task = None;
            cx.notify();
        } else if self.file.added != file.added || self.file.removed != file.removed {
            self.file.added = file.added;
            self.file.removed = file.removed;
            cx.notify();
        }
        if self.theme != theme {
            self.theme = theme;
            if let Some(card) = &self.card {
                card.update(cx, |card, cx| card.set_theme(theme, cx));
            }
            cx.notify();
        }
    }

    fn reveal(&mut self, revision: u64, cx: &mut Context<Self>) {
        if self.reveal_revision != revision {
            self.reveal_revision = revision;
            self.expanded = true;
            cx.notify();
        }
    }

    fn load_diff(&mut self, manual: bool, cx: &mut Context<Self>) {
        if self.load_task.is_some() {
            return;
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        let id = self.file.id;
        let side_by_side = self.side_by_side;
        self.load_error = None;
        self.manual_load_available = false;
        let mut pending = None;
        if !manual
            && let Some(prefetch) = &self.prefetch
            && let Some(layer) = source.layer
        {
            let key = (id, layer, side_by_side);
            let mut prefetch = prefetch.borrow_mut();
            prefetch.attempted.insert(key);
            if let Some(DiffPreview {
                change, prepared, ..
            }) = prefetch.ready.remove(&key)
            {
                self.loaded = Some(change);
                self.prepared = Some(prepared);
                if let Some(workspace) = self.workspace.clone() {
                    cx.defer(move |cx| {
                        let _ =
                            workspace.update(cx, |this, cx| this.sync_workspace_diff_prefetch(cx));
                    });
                }
                return;
            }
            if let Some((active, waiter)) = &mut prefetch.active
                && *active == key
            {
                let (send, receive) = oneshot::channel();
                *waiter = Some(send);
                pending = Some(receive);
            }
        }
        self.load_task = Some(cx.spawn(async move |weak, cx| {
            let result = async {
                if let Some(pending) = pending
                    && let Ok(result) = pending.await
                {
                    return result;
                }
                cx.background_executor()
                    .spawn_with_priority(Priority::High, async move {
                        prepare_workspace_diff(&source, manual, side_by_side)
                    })
                    .await
            }
            .await;
            let _ = weak.update(cx, |this, cx| {
                if this.file.id != id {
                    return;
                }
                if let Some(task) = this.load_task.take() {
                    task.detach();
                }
                match result {
                    Ok(DiffPreview {
                        change, prepared, ..
                    }) => {
                        if this.side_by_side != side_by_side {
                            this.load_diff(manual, cx);
                            cx.notify();
                            return;
                        }
                        this.loaded = Some(change);
                        this.prepared = Some(prepared);
                    }
                    Err(error) => {
                        this.manual_load_available =
                            matches!(error, crate::workspace_diff::LoadError::LargeFile(_));
                        this.load_error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        }));
    }
}

impl Render for FileDiff {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let expanded = self.expanded || self.detail;
        if expanded
            && self.source.is_some()
            && self.loaded.is_none()
            && self.load_task.is_none()
            && self.load_error.is_none()
        {
            self.load_diff(false, cx);
        }
        let selector = format!("edit-file:{}", self.file.label);
        let path = self.file.path.clone();
        let undo_selector = format!("undo-file:{}", path.display());
        let workspace = self.workspace.clone();
        let card = if expanded && self.load_task.is_none() && self.load_error.is_none() {
            Some(
                self.card
                    .get_or_insert_with(|| {
                        cx.new(|cx| {
                            let language = edits::language(&self.file.path);
                            let card = if let Some(prepared) = self.prepared.take() {
                                DiffCard::from_prepared(cx, language, prepared)
                            } else {
                                DiffCard::from_lines(
                                    cx,
                                    language,
                                    self.loaded.as_ref().unwrap_or(&self.file).lines(),
                                )
                            };
                            card.side_by_side(self.side_by_side)
                                .fill_height(self.detail)
                                .header_visible(false)
                                .theme(self.theme)
                        })
                    })
                    .clone(),
            )
        } else {
            None
        };
        if let Some(card) = &card {
            card.update(cx, |card, cx| card.set_fill_height(self.detail, cx));
        }
        let whitespace = card
            .as_ref()
            .map(|card| {
                let card = card.read(cx);
                (
                    card.whitespace_changed_pairs(),
                    card.hide_whitespace_enabled(),
                )
            })
            .filter(|(count, _)| *count > 0);
        let whitespace_selector = format!("whitespace-toggle:{}", self.file.label);
        let status = self
            .status
            .map(|status| file_status_style(status, self.theme));
        let mut row = div()
            .w_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .when(self.detail, |row| row.flex_1().min_h_0())
            .border_b_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .id("file-disclosure")
                    .on_mouse_down(
                        gpui::MouseButton::Right,
                        cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                            if let Some(workspace) = &this.workspace {
                                let _ = workspace.update(cx, |workspace, cx| {
                                    workspace.open_diff_file_menu(
                                        this.file.path.clone(),
                                        this.file.label.clone(),
                                        event.position,
                                        window,
                                        cx,
                                    );
                                });
                            }
                        }),
                    )
                    .debug_selector(move || selector.clone())
                    .h(px(48.))
                    .flex_shrink_0()
                    .px_3()
                    .pl(px(12. + self.tree_depth.unwrap_or(0) as f32 * 16.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(p.surface_subtle)))
                    .when(!self.detail, |header| {
                        header.child(
                            Icon::disclosure(13., self.expanded).text_color(rgb(p.text_muted)),
                        )
                    })
                    .child(
                        Icon::for_language(edits::language(&self.file.path))
                            .render(15.)
                            .text_color(rgb(status.map_or(p.text_muted, |(_, color)| color))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .when_some(status, |label, (_, color)| label.text_color(rgb(color)))
                            .child(if self.tree_depth.is_some() {
                                self.file
                                    .label
                                    .rsplit('/')
                                    .next()
                                    .unwrap_or(&self.file.label)
                                    .to_owned()
                            } else {
                                self.file.label.clone()
                            }),
                    )
                    .when_some(status, |header, (label, color)| {
                        header.child(div().text_size(px(11.)).text_color(rgb(color)).child(label))
                    })
                    .child(if let Some(loaded) = &self.loaded {
                        counts(loaded.added, loaded.removed)
                    } else if self
                        .source
                        .as_ref()
                        .is_some_and(|source| source.stats.is_none())
                    {
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child("按需预览")
                    } else {
                        counts(self.file.added, self.file.removed)
                    })
                    .when_some(whitespace, |header, (count, hidden)| {
                        header.child(
                            div()
                                .id("file-whitespace-toggle")
                                .debug_selector(move || whitespace_selector.clone())
                                .h(px(20.))
                                .px(px(8.))
                                .flex()
                                .items_center()
                                .rounded(px(10.))
                                .border_1()
                                .border_color(rgb(if hidden { p.accent } else { p.border }))
                                .text_size(px(11.))
                                .text_color(rgb(if hidden { p.accent } else { p.text_muted }))
                                .cursor_pointer()
                                .hover(|style| style.text_color(rgb(p.accent)))
                                .child(if hidden {
                                    format!("显示空白 {count}")
                                } else {
                                    format!("隐藏空白 {count}")
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(card) = &this.card {
                                        let enabled = card.read(cx).hide_whitespace_enabled();
                                        card.update(cx, |card, cx| {
                                            card.set_hide_whitespace(!enabled, cx)
                                        });
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .when_some(
                        self.source.clone().filter(|source| source.layer.is_some()),
                        |header, source| {
                            let staged = source.layer.unwrap();
                            let workspace = self.workspace.clone();
                            let selector =
                                format!("stage-file:{}:{staged}", self.file.path.display());
                            let checkbox = workspace.and_then(|workspace| {
                                workspace
                                    .update(cx, |workspace, cx| {
                                        workspace.stage_checkbox(vec![source], staged, selector, cx)
                                    })
                                    .ok()
                            });
                            header.children(checkbox)
                        },
                    )
                    .when(self.undo_visible, |header| {
                        header.child(
                            Button::new("撤销")
                                .kind(ButtonKind::Plain)
                                .when(!self.undo_enabled, |button| button.disabled())
                                .theme(self.theme)
                                .render()
                                .debug_selector(move || undo_selector)
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(workspace) = &workspace {
                                        let _ = workspace.update(cx, |this, cx| {
                                            this.undo_session_edits(Some(path.clone()), cx)
                                        });
                                    }
                                }),
                        )
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.detail {
                            return;
                        }
                        this.expanded = !this.expanded;
                        if !this.expanded {
                            this.card = None;
                            this.loaded = None;
                            this.prepared = None;
                            this.manual_load_available = false;
                            this.load_task = None;
                            this.load_error = None;
                        }
                        cx.notify();
                    })),
            );
        if expanded {
            if self.load_task.is_some() {
                let pane = |id: &'static str| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .p_4()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(
                            Skeleton::new((id, 0usize))
                                .width(72.)
                                .height(10.)
                                .theme(self.theme),
                        )
                        .child(
                            Skeleton::new((id, 1usize))
                                .lines(12)
                                .height(12.)
                                .gap(14.)
                                .theme(self.theme),
                        )
                };
                return row.child(
                    div()
                        .px_2()
                        .pb_2()
                        .when(self.detail, |body| {
                            body.flex_1().min_h_0().flex().flex_col()
                        })
                        .child(
                            div()
                                .id("file-diff-loading")
                                .debug_selector(|| "file-diff-loading-skeleton".into())
                                .w_full()
                                .min_w_0()
                                .overflow_hidden()
                                .flex()
                                .when(self.detail, |body| body.flex_1().min_h_0())
                                .rounded(px(8.))
                                .border_1()
                                .border_color(rgb(p.border))
                                .bg(rgb(p.surface))
                                .child(pane("diff-loading-before"))
                                .when(self.side_by_side, |body| {
                                    body.child(div().w(px(1.)).self_stretch().bg(rgb(p.border)))
                                        .child(pane("diff-loading-after"))
                                }),
                        ),
                );
            }
            if let Some(error) = &self.load_error {
                return row.child(
                    div()
                        .w_full()
                        .min_h(px(180.))
                        .px_6()
                        .py_8()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .when(self.detail, |body| body.flex_1().min_h_0())
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child(div().text_center().child(error.clone()))
                        .child(
                            Button::new(if self.manual_load_available {
                                "加载 Diff"
                            } else {
                                "重试"
                            })
                            .kind(ButtonKind::Primary)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "load-file-diff-manually".into())
                            .on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.load_diff(true, cx);
                                    cx.notify();
                                },
                            )),
                        ),
                );
            }
            let card = card.expect("diff card is ready when expanded without load errors");
            row = row.child(
                div()
                    .px_2()
                    .pb_2()
                    .when(self.detail, |body| {
                        body.flex_1().min_h_0().flex().flex_col()
                    })
                    .child(card),
            );
        }
        row
    }
}

fn counts(added: usize, removed: usize) -> gpui::Div {
    div()
        .flex()
        .gap_1()
        .text_size(px(11.))
        .child(div().text_color(rgb(0x34a853)).child(format!("+{added}")))
        .child(div().text_color(rgb(0xd9342b)).child(format!("-{removed}")))
}

struct Summary {
    files: Rc<Vec<Change>>,
    workspace: gpui::WeakEntity<AgentWorkspace>,
    theme: ThemePreset,
    expanded: bool,
}
impl Render for Summary {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let weak = self.workspace.clone();
        let total_add = self.files.iter().map(|file| file.added).sum();
        let total_remove = self.files.iter().map(|file| file.removed).sum();
        let mut panel = div()
            .w_full()
            .min_w_0()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .flex()
            .flex_col()
            .child(
                div()
                    .px_3()
                    .py_3()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::Pencil.render(18.).text_color(rgb(p.text_muted)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .debug_selector(|| "edit-summary-title".into())
                                    .child(format!("已编辑 {} 个文件", self.files.len())),
                            )
                            .child(
                                counts(total_add, total_remove)
                                    .flex_shrink_0()
                                    .debug_selector(|| "edit-summary-totals".into()),
                            ),
                    )
                    .child(
                        Button::new("查看变更")
                            .kind(ButtonKind::Glass)
                            .size(ButtonSize::Small)
                            .pill()
                            .theme(self.theme)
                            .render()
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |this, cx| this.open_edits(None, cx));
                            }),
                    ),
            );
        for file in self
            .files
            .iter()
            .take(if self.expanded { usize::MAX } else { 3 })
        {
            let weak = self.workspace.clone();
            let path = file.path.clone();
            panel = panel.child(
                div()
                    .id(SharedString::from(format!("edit-summary:{}", file.id)))
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(p.surface_subtle)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(rgb(p.text_muted))
                            .child(file.label.clone()),
                    )
                    .child(counts(file.added, file.removed))
                    .on_click(move |_, _, cx| {
                        let _ = weak.update(cx, |this, cx| this.open_edits(Some(path.clone()), cx));
                    }),
            );
        }
        if self.files.len() > 3 {
            // A quiet text toggle sits inside the card like one more row: the
            // filled button read as a separate action and split the list.
            panel = panel.child(
                div()
                    .id("edit-summary-more")
                    .debug_selector(|| "edit-summary-more".into())
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .cursor_pointer()
                    .rounded_b(px(10.))
                    .hover(|style| style.bg(rgb(p.surface_subtle)))
                    .child(div().text_size(px(12.)).text_color(rgb(p.text)).child(
                        if self.expanded {
                            "收起".to_owned()
                        } else {
                            format!("再显示 {} 个文件", self.files.len() - 3)
                        },
                    ))
                    .child(Icon::disclosure(12., self.expanded))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.expanded = !this.expanded;
                        cx.notify();
                    })),
            );
        }
        panel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn commit_footer_keeps_drafts_and_requires_message_and_staged_files(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        cx.update(crate::initialize);
        let repo = tempfile::tempdir().unwrap();
        crate::workspace_diff::tests::git_ok(repo.path(), &["init"]);
        std::fs::write(repo.path().join("new.txt"), "staged\n").unwrap();
        crate::workspace_diff::tests::git_ok(repo.path(), &["add", "new.txt"]);
        let inventory = crate::workspace_diff::inventory(repo.path()).unwrap();
        let root = inventory.root.clone();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.history.active_mut().project = Some(root.clone());
            host.edits_ui.open = true;
            host.edits_ui.workspace_mode = true;
            host.edits_ui.workspace_root = Some(root.clone());
            host.edits_ui.workspace_files = Rc::new(inventory.files);
            host.edits_ui.workspace_sources = Rc::new(inventory.sources);
            host
        });
        window.simulate_resize(size(px(1440.), px(1000.)));
        window.run_until_parked();
        let editor = window.debug_bounds("diff-commit-message").unwrap();
        let submit = window.debug_bounds("diff-commit-submit").unwrap();
        assert!(editor.contains(&submit.center()));
        assert!((editor.right() - submit.right() - px(9.)).abs() <= px(1.));
        assert!((editor.bottom() - submit.bottom() - px(9.)).abs() <= px(1.));
        let message = "feat: sidebar commit\n\nDescribe the staged changes.";
        window.simulate_click(editor.origin + point(px(12.), px(12.)), Default::default());
        window.simulate_keystrokes("enter");
        host.update(window, |host, cx| {
            assert!(!host.can_commit_workspace(cx));
            let entry = host
                .edits_ui
                .commit_drafts
                .borrow()
                .get(&root)
                .unwrap()
                .clone();
            assert_eq!(entry.read(cx).value(), "\n");
            entry.update(cx, |entry, cx| entry.set_value("  ", cx));
            assert!(!host.can_commit_workspace(cx));
            entry.update(cx, |entry, cx| entry.set_value(message, cx));
            assert!(host.can_commit_workspace(cx));
            let sources = host.edits_ui.workspace_sources.clone();
            host.edits_ui.workspace_sources = Rc::new(HashMap::new());
            assert!(!host.can_commit_workspace(cx));
            host.edits_ui.workspace_sources = sources;
            host.edits_ui.workspace_mode = false;
            assert!(!host.can_commit_workspace(cx));
            host.edits_ui.workspace_mode = true;
            host.edits_ui.expanded = true;
            assert_eq!(entry.read(cx).value(), message);
            assert!(host.can_commit_workspace(cx));
            cx.notify();
        });
        for viewport in [size(px(1440.), px(1000.)), size(px(900.), px(500.))] {
            window.simulate_resize(viewport);
            window.run_until_parked();
            let navigation = window.debug_bounds("expanded-diff-navigation").unwrap();
            let tree = window.debug_bounds("diff-navigation-tree").unwrap();
            let footer = window.debug_bounds("diff-commit-footer").unwrap();
            let detail = window.debug_bounds("expanded-diff-content").unwrap();
            assert!(footer.left() >= navigation.left());
            assert!(footer.right() <= navigation.right() + px(1.));
            assert!((footer.bottom() - navigation.bottom()).abs() <= px(1.));
            assert!(tree.bottom() <= footer.top() + px(1.));
            assert!(detail.left() >= footer.right());
            assert!((detail.bottom() - footer.bottom()).abs() <= px(1.));
        }
        // Empty inventories use the alternate navigation renderer too.
        host.update(window, |host, cx| {
            host.edits_ui.workspace_sources = Rc::new(HashMap::new());
            host.edits_ui.workspace_files = Rc::new(Vec::new());
            cx.notify();
        });
        window.run_until_parked();
        let navigation = window.debug_bounds("expanded-diff-navigation").unwrap();
        let footer = window.debug_bounds("diff-commit-footer").unwrap();
        assert!(footer.right() <= navigation.right() + px(1.));
        assert!((footer.bottom() - navigation.bottom()).abs() <= px(1.));
    }

    #[gpui::test]
    fn workspace_tree_folders_keep_diff_state_and_settings_switch_back_to_list(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        cx.update(crate::initialize);
        let data = tempfile::tempdir().unwrap();
        let path = data.path().join("sessions.json");
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(path.clone(), cx);
            host.edits_ui.open = true;
            host.edits_ui.expanded = false;
            host.edits_ui.workspace_mode = true;
            host.history.workspace_display = crate::workspace_tree::DisplayMode::Tree;
            host.edits_ui.workspace_files = Rc::new(vec![
                change("root.rs", "new\n"),
                change("src/a.rs", "new\n"),
                change("src/nested/b.rs", "new\n"),
                change("tests/a.rs", "new\n"),
            ]);
            host
        });
        window.simulate_resize(size(px(1440.), px(1000.)));
        window.run_until_parked();
        let file = window.debug_bounds("edit-file:src/a.rs").unwrap();
        window.simulate_click(file.center(), Default::default());
        window.run_until_parked();
        let view = host.read_with(window, |host, cx| {
            let views = host.edits_ui.file_views.borrow();
            let view = views
                .values()
                .find(|view| view.read(cx).file.label == "src/a.rs")
                .unwrap()
                .clone();
            assert!(view.read(cx).expanded && view.read(cx).card.is_some());
            assert_eq!(view.read(cx).tree_depth, Some(1));
            view
        });
        let directory = window.debug_bounds("workspace-directory:src").unwrap();
        window.simulate_click(directory.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, cx| {
            assert_eq!(
                host.edits_ui
                    .virtual_files
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .state
                    .item_count(),
                4
            );
            assert!(view.read(cx).expanded);
        });
        let directory = window.debug_bounds("workspace-directory:src").unwrap();
        window.simulate_click(directory.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, cx| {
            assert_eq!(
                host.edits_ui
                    .virtual_files
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .state
                    .item_count(),
                7
            );
            assert!(view.read(cx).expanded && view.read(cx).card.is_some());
        });
        host.update(window, |host, cx| {
            host.page = crate::Page::Settings;
            host.provider_settings.section = provider_ui::Section::Appearance;
            cx.notify();
        });
        window.run_until_parked();
        let list = window.debug_bounds("workspace-display-list").unwrap();
        window.simulate_click(list.center(), Default::default());
        host.update(window, |host, cx| {
            assert!(host.history.workspace_display == crate::workspace_tree::DisplayMode::List);
            host.history.save(&path).unwrap();
            assert!(
                model::History::load(&path).unwrap().workspace_display
                    == crate::workspace_tree::DisplayMode::List
            );
            host.page = crate::Page::Chat;
            cx.notify();
        });
        window.run_until_parked();
        host.read_with(window, |_, cx| {
            assert_eq!(view.read(cx).tree_depth, None);
            assert!(view.read(cx).expanded && view.read(cx).card.is_some());
        });
    }

    #[gpui::test]
    fn workspace_tree_virtualizes_thousands_of_nested_files(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.edits_ui.open = true;
            host.edits_ui.expanded = true;
            host.edits_ui.workspace_mode = true;
            host.history.workspace_display = crate::workspace_tree::DisplayMode::Tree;
            host.edits_ui.workspace_files = Rc::new(
                (0..5000)
                    .map(|index| change(&format!("vendor/{index:04}.rs"), "new\n"))
                    .collect(),
            );
            host
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.run_until_parked();
        let list = host.read_with(window, |host, _| {
            assert_eq!(host.edits_ui.file_views.borrow().len(), 1);
            assert_eq!(
                host.edits_ui.tree_cache.borrow().as_ref().unwrap().2.len(),
                5001
            );
            host.edits_ui
                .navigation
                .borrow()
                .as_ref()
                .unwrap()
                .1
                .clone()
        });
        list.update(window, |list, cx| list.scroll_to_row(4001, cx));
        window.run_until_parked();
        let file = window
            .debug_bounds("change-file:vendor/4000.rs:false")
            .unwrap();
        window.simulate_click(file.center(), Default::default());
        window.run_until_parked();
        assert!(window.debug_bounds("edit-file:vendor/4000.rs").is_some());
        host.read_with(window, |host, cx| {
            let views = host.edits_ui.file_views.borrow();
            assert_eq!(views.len(), 2, "only selected files create Diff views");
            assert!(views.values().all(|view| view.read(cx).card.is_some()));
        });
    }

    #[test]
    fn conflict_requests_attach_the_snapshot_as_a_highlighted_diff_reference() {
        let repo = crate::merge_conflicts::tests::conflicted();
        let file = crate::merge_conflicts::load(repo.path())
            .unwrap()
            .0
            .remove(0);
        let submission = conflict_submission(&file);
        let reference = &submission.references[0];
        assert_eq!(reference.icon(), gpui_glint::components::Icon::GitConflict);
        assert!(submission.text.ends_with(&reference.token));
        // History keeps the compact token; the raw snapshot is not inlined.
        let display = crate::model::MessageDisplay::from_submission(&submission).unwrap();
        assert_eq!(display.attachments.len(), 1);
        assert_eq!(
            display.attachments[0].excerpt.as_deref(),
            Some(file.original.as_str())
        );
        assert!(!display.text.contains("<<<<<<<"));
        let markdown = display.markdown(uuid::Uuid::new_v4(), 0);
        assert!(markdown.contains("glint-paste://"));
        assert!(!markdown.contains("<<<<<<<"));
        // The model still receives the full snapshot through the reference prompt.
        let prompt = submission.prompt_text();
        assert!(prompt.contains("冲突文件快照"));
        assert!(prompt.contains(&file.original));
    }

    #[gpui::test]
    fn thousands_of_workspace_files_only_render_visible_rows_and_load_disclosed_diff(
        cx: &mut gpui::TestAppContext,
    ) {
        let repo = crate::workspace_diff::tests::many_files(3089);
        let inventory = crate::workspace_diff::inventory(repo.path()).unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let started = std::time::Instant::now();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.edits_ui.open = true;
            host.edits_ui.workspace_mode = true;
            host.edits_ui.expanded = false;
            host.edits_ui.workspace_files = Rc::new(inventory.files);
            host.edits_ui.workspace_sources = Rc::new(inventory.sources);
            host
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.run_until_parked();
        let initial = started.elapsed();
        let list = host.read_with(window, |host, cx| {
            let views = host.edits_ui.file_views.borrow();
            assert!(views.len() < 64, "created {} views", views.len());
            assert!(views.values().all(
                    |view| view.read(cx).loaded.is_none() && view.read(cx).load_task.is_none()
                ));
            host.edits_ui
                .grouped_list
                .borrow()
                .as_ref()
                .unwrap()
                .2
                .clone()
        });
        // Leave room below the target so the following row remains visible
        // after opening the Diff; off-screen rows are intentionally not rendered.
        host.update(window, |_, cx| {
            list.scroll_to(gpui::ListOffset {
                item_ix: 2499,
                offset_in_item: px(0.),
            });
            cx.notify();
        });
        window.run_until_parked();
        let row = window.debug_bounds("edit-file:vendor/2500.rs").unwrap();
        window.simulate_click(row.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, cx| {
            let views = host.edits_ui.file_views.borrow();
            assert!(views.len() < 96);
            let open = views
                .values()
                .find(|view| view.read(cx).file.label == "vendor/2500.rs")
                .unwrap()
                .read(cx);
            assert!(
                open.expanded && open.card.is_some() && open.loaded.is_some(),
                "{:?}",
                open.load_error
            );
        });
        assert!(
            window
                .debug_bounds("edit-file:vendor/2501.rs")
                .unwrap()
                .top()
                > row.top() + px(70.)
        );
        // Adding statistics changes neither list identity nor scroll position.
        host.update(window, |host, cx| {
            let files = Rc::make_mut(&mut host.edits_ui.workspace_files);
            files[2500].added = 1;
            cx.notify();
        });
        window.run_until_parked();
        host.read_with(window, |host, _| {
            let list_after = host
                .edits_ui
                .grouped_list
                .borrow()
                .as_ref()
                .unwrap()
                .2
                .logical_scroll_top();
            assert!(list_after.item_ix > 2400);
        });
        host.update(window, |_, cx| {
            list.scroll_to_end();
            cx.notify();
        });
        window.run_until_parked();
        assert!(window.debug_bounds("edit-file:vendor/3088.rs").is_some());
        eprintln!(
            "3089 files: first visible frame={initial:?}; only visible file views created; one Diff loaded on disclosure"
        );
    }

    #[gpui::test]
    fn conflict_tree_keeps_drafts_and_full_width_rows_in_both_layouts(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        let repo = crate::merge_conflicts::tests::vendor_update(1, 4);
        let inventory = crate::workspace_diff::inventory(repo.path()).unwrap();
        let (mut conflicts, _) = crate::merge_conflicts::load(repo.path()).unwrap();
        for file in &mut conflicts {
            file.original = format!(
                "{}{}{}",
                "// context\n".repeat(300),
                file.original,
                "// after\n".repeat(100)
            );
            std::fs::write(&file.path, &file.original).unwrap();
        }
        let root = inventory.root.clone();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.history.active_mut().project = Some(root.clone());
            host.history.workspace_display = crate::workspace_tree::DisplayMode::Tree;
            host.edits_ui.workspace_root = Some(root.clone());
            host.edits_ui.open = true;
            host.edits_ui.expanded = true;
            host.edits_ui.workspace_mode = true;
            host.edits_ui.workspace_files = Rc::new(inventory.files);
            host.edits_ui.workspace_sources = Rc::new(inventory.sources);
            let weak = cx.entity().downgrade();
            host.edits_ui.conflicts = conflicts
                .into_iter()
                .map(|file| {
                    cx.new(|cx| {
                        ConflictView::new(file, root.clone(), Some(weak.clone()), host.theme, cx)
                    })
                })
                .collect();
            host
        });
        window.simulate_resize(size(px(1440.), px(1000.)));
        window.run_until_parked();
        let first_path = host.read_with(window, |host, cx| {
            host.edits_ui.conflicts[0].read(cx).file.path.clone()
        });
        let first_marker: &'static str =
            Box::leak(format!("selected-conflict-file:{}", first_path.display()).into_boxed_str());
        assert!(
            window.debug_bounds(first_marker).is_some(),
            "default file must be highlighted without a click"
        );
        let path = root.join("vendor/group_1/conflict-1.go");
        let selector = Box::leak(format!("change-file:{}:false", path.display()).into_boxed_str());
        let row = window.debug_bounds(selector).unwrap();
        let navigation = window.debug_bounds("expanded-diff-navigation").unwrap();
        assert_eq!(
            row.size.width, navigation.size.width,
            "hover and selection cover the complete navigation row"
        );
        window.simulate_click(row.center(), Default::default());
        window.run_until_parked();
        let selected_marker: &'static str =
            Box::leak(format!("selected-conflict-file:{}", path.display()).into_boxed_str());
        assert!(window.debug_bounds(selected_marker).is_some());
        assert!(window.debug_bounds(first_marker).is_none());
        let header = window.debug_bounds("current-conflict-file").unwrap();
        let viewport = window.debug_bounds("expanded-diff-content").unwrap();
        assert!(
            header.bottom() <= viewport.top(),
            "file identity stays outside the scrolled content"
        );
        let hunk = window.debug_bounds("merge-hunk:0").unwrap();
        let detail = window.debug_bounds("expanded-diff-content").unwrap();
        assert!(
            hunk.origin.y >= detail.origin.y && hunk.origin.y < detail.origin.y + px(40.),
            "selecting a conflict reveals the first hunk: {hunk:?}, viewport={detail:?}"
        );
        // Re-selecting must return to the conflict after scrolling elsewhere.
        host.update(window, |host, cx| {
            for scroll in host.edits_ui.detail_scrolls.borrow().values() {
                scroll.set_offset(point(px(0.), px(0.)));
            }
            cx.notify();
        });
        window.run_until_parked();
        window.simulate_click(row.center(), Default::default());
        window.run_until_parked();
        let hunk = window.debug_bounds("merge-hunk:0").unwrap();
        assert!(hunk.origin.y >= detail.origin.y && hunk.origin.y < detail.origin.y + px(40.));
        let draft = host.read_with(window, |host, cx| {
            host.edits_ui
                .conflicts
                .iter()
                .find(|view| view.read(cx).file.path == path)
                .unwrap()
                .read(cx)
                .editor
                .clone()
        });
        draft.update(window, |editor, cx| {
            editor.sync_content("draft survives folder collapse", cx)
        });
        for expanded in [true, false] {
            let (folder_selector, child_selector) = if expanded {
                ("change-directory:vendor", "change-directory:vendor/group_1")
            } else {
                (
                    "conflict-directory:vendor",
                    "conflict-directory:vendor/group_1",
                )
            };
            host.update(window, |host, cx| {
                host.edits_ui.expanded = expanded;
                if !expanded {
                    for view in &host.edits_ui.conflicts {
                        view.update(cx, |view, cx| {
                            view.expanded = false;
                            cx.notify();
                        });
                    }
                }
                cx.notify();
            });
            window.run_until_parked();
            let folder = window.debug_bounds(folder_selector).unwrap();
            window.simulate_click(folder.center(), Default::default());
            window.run_until_parked();
            assert!(window.debug_bounds(child_selector).is_none());
            draft.read_with(window, |editor, _| {
                assert_eq!(editor.content(), "draft survives folder collapse")
            });
            let folder = window.debug_bounds(folder_selector).unwrap();
            window.simulate_click(folder.center(), Default::default());
            window.run_until_parked();
            assert!(window.debug_bounds(child_selector).is_some());
        }
    }

    #[gpui::test]
    fn expanded_diff_navigation_switches_one_file_and_restores_compact_layout(
        cx: &mut gpui::TestAppContext,
    ) {
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.edits_ui.open = true;
            host.edits_ui.expanded = true;
            host.edits_ui.workspace_mode = true;
            host.history.workspace_display = crate::workspace_tree::DisplayMode::Tree;
            host.edits_ui.workspace_files = Rc::new(vec![
                change("src/first.rs", "first\n"),
                change("src/second.rs", "second\n"),
            ]);
            host
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.run_until_parked();
        let navigation = window.debug_bounds("expanded-diff-navigation").unwrap();
        let detail = window.debug_bounds("expanded-diff-content").unwrap();
        assert!(window.debug_bounds("change-navigation").is_some());
        assert!(navigation.right() <= detail.left());
        assert!(detail.size.width > navigation.size.width);
        let resize = window.debug_bounds("diff-navigation-resize").unwrap();
        let target = point(resize.center().x + px(180.), resize.center().y + px(60.));
        window.simulate_mouse_down(resize.center(), gpui::MouseButton::Left, Default::default());
        window.simulate_mouse_move(target, gpui::MouseButton::Left, Default::default());
        window.simulate_mouse_up(target, gpui::MouseButton::Left, Default::default());
        window.run_until_parked();
        let wider = window.debug_bounds("expanded-diff-navigation").unwrap();
        assert!(wider.size.width > navigation.size.width + px(100.));
        assert!(
            window
                .debug_bounds("expanded-diff-content")
                .unwrap()
                .size
                .width
                >= px(300.)
        );
        host.read_with(window, |host, _| {
            assert!(host.edits_ui.navigation_resize.is_none())
        });
        assert!(window.debug_bounds("edit-file:src/first.rs").is_some());
        assert!(window.debug_bounds("edit-file:src/second.rs").is_none());
        host.read_with(window, |host, _| {
            assert_eq!(
                host.edits_ui.file_views.borrow().len(),
                1,
                "only the selected file loads its Diff"
            )
        });
        let second = window
            .debug_bounds("change-file:src/second.rs:false")
            .unwrap();
        window.simulate_click(second.center(), Default::default());
        window.run_until_parked();
        assert!(window.debug_bounds("edit-file:src/first.rs").is_none());
        assert!(window.debug_bounds("edit-file:src/second.rs").is_some());
        window.simulate_keystrokes("up");
        window.run_until_parked();
        assert!(window.debug_bounds("edit-file:src/first.rs").is_some());
        window.simulate_keystrokes("down");
        window.run_until_parked();
        assert!(window.debug_bounds("edit-file:src/second.rs").is_some());
        window.simulate_mouse_down(
            second.center(),
            gpui::MouseButton::Right,
            Default::default(),
        );
        window.run_until_parked();
        host.read_with(window, |host, _| {
            let (_, label, _, _) = host.edits_ui.file_menu.as_ref().unwrap();
            assert_eq!(label, "src/second.rs");
        });
        window.simulate_keystrokes("escape");
        window.run_until_parked();
        let directory = window.debug_bounds("change-directory:src").unwrap();
        window.simulate_click(directory.center(), Default::default());
        window.run_until_parked();
        assert!(
            window
                .debug_bounds("change-file:src/second.rs:false")
                .is_none()
        );
        assert!(
            window.debug_bounds("edit-file:src/second.rs").is_some(),
            "collapsing navigation must keep the selected Diff visible"
        );
        host.update(window, |host, cx| {
            host.edits_ui.expanded = false;
            host.history.workspace_display = crate::workspace_tree::DisplayMode::List;
            cx.notify();
        });
        window.run_until_parked();
        assert!(window.debug_bounds("expanded-diff-layout").is_none());
        assert!(window.debug_bounds("edit-file:src/first.rs").is_some());
        assert!(window.debug_bounds("edit-file:src/second.rs").is_some());
        host.read_with(window, |host, cx| {
            for view in host.edits_ui.file_views.borrow().values() {
                assert!(
                    !view.read(cx).expanded && !view.read(cx).detail,
                    "detail mode must preserve compact disclosures"
                );
            }
        });
    }

    #[gpui::test]
    fn diff_panel_expands_and_restores_sidebar_with_existing_file_disclosures(
        cx: &mut gpui::TestAppContext,
    ) {
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.edits_ui.open = true;
            host.edits_ui.workspace_mode = true;
            host.edits_ui.workspace_files = Rc::new(vec![change("first.rs", "new\n")]);
            host
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.run_until_parked();
        let file = window.debug_bounds("edit-file:first.rs").unwrap();
        window.simulate_click(file.center(), Default::default());
        window.run_until_parked();
        for expanded in [true, false] {
            let button = window.debug_bounds("edits-panel-expand").unwrap();
            window.simulate_click(button.center(), Default::default());
            window.run_until_parked();
            host.read_with(window, |host, cx| {
                assert_eq!(host.edits_ui.expanded, expanded);
                let views = host.edits_ui.file_views.borrow();
                let view = views.values().next().unwrap().read(cx);
                assert!(view.expanded && view.card.is_some());
                assert_eq!(view.side_by_side, expanded);
            });
            let width = window
                .debug_bounds("glass-diff-card-body")
                .unwrap()
                .size
                .width;
            assert_eq!(width > px(700.), expanded);
        }
    }

    #[gpui::test]
    fn workspace_file_rows_expose_whitespace_toggle_and_hide_pairs(cx: &mut gpui::TestAppContext) {
        let change: Change = serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::new_v4(), "path": "fmt.rs", "label": "fmt.rs",
            "before": "alpha = 1;\nbeta = 2;\n",
            "after": "alpha =  1;\ngamma = 3;\n",
            "added": 2, "removed": 2
        }))
        .unwrap();
        let (stack, window) = cx.add_window_view(|_, cx| {
            FileStack(vec![
                cx.new(|_| FileDiff::new(change, ThemePreset::default())),
            ])
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.run_until_parked();
        // No pill before disclosure: the card and its pair scan do not exist yet.
        assert!(window.debug_bounds("whitespace-toggle:fmt.rs").is_none());
        let row = window.debug_bounds("edit-file:fmt.rs").unwrap();
        window.simulate_click(row.center(), Default::default());
        window.run_until_parked();
        let pill = window.debug_bounds("whitespace-toggle:fmt.rs").unwrap();
        window.simulate_click(pill.center(), Default::default());
        window.run_until_parked();
        stack.read_with(window, |stack, cx| {
            let view = stack.0[0].read(cx);
            assert!(view.expanded);
            let card = view.card.as_ref().unwrap().read(cx);
            assert_eq!(card.whitespace_changed_pairs(), 1);
            assert!(card.hide_whitespace_enabled());
        });
        // The pill click must not collapse the disclosure.
        assert!(window.debug_bounds("glass-diff-card-body").is_some());
    }

    #[gpui::test]
    fn clean_disk_conflicts_are_automatically_staged_on_refresh(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        let repo = crate::merge_conflicts::tests::conflicted();
        let file = crate::merge_conflicts::load(repo.path())
            .unwrap()
            .0
            .remove(0);
        std::fs::write(&file.path, "resolved externally\n").unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let root = file.root.clone();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.history.active_mut().project = Some(root.clone());
            host.edits_ui.workspace_root = Some(root.clone());
            host
        });
        host.update(window, |host, cx| {
            host.start_workspace_conflicts(root, host.edits_ui.workspace_revision, cx);
        });
        window.run_until_parked();
        assert!(
            crate::merge_conflicts::load(repo.path())
                .unwrap()
                .0
                .is_empty()
        );
        assert_eq!(
            std::fs::read_to_string(file.path).unwrap(),
            "resolved externally\n"
        );
    }

    #[gpui::test]
    fn conflict_choices_auto_save_after_the_last_marker_is_removed(cx: &mut gpui::TestAppContext) {
        let repo = crate::merge_conflicts::tests::conflicted();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(data.path().join("sessions.json"), cx)
        });
        host.update(window, |host, cx| {
            host.composer
                .update(cx, |composer, cx| composer.set_value("保留输入框草稿", cx))
        });
        let file = crate::merge_conflicts::load(repo.path())
            .unwrap()
            .0
            .remove(0);
        let original = file.original.clone();
        let path = file.path.clone();
        let (view, window) = window.add_window_view(|_, cx| {
            ConflictView::new(
                file,
                repo.path().into(),
                Some(host.downgrade()),
                ThemePreset::default(),
                cx,
            )
        });
        window.simulate_resize(size(px(700.), px(900.)));
        window.run_until_parked();
        let agent = window.debug_bounds("agent-resolve-conflict").unwrap();
        window.simulate_click(agent.center(), Default::default());
        window.run_until_parked();
        view.read_with(window, |view, _| {
            assert!(view.expanded);
            assert!(view.notice.as_deref().unwrap().contains("项目已切换"));
        });
        host.read_with(window, |host, cx| {
            assert_eq!(host.composer.read(cx).value(cx), "保留输入框草稿")
        });
        let button = window.debug_bounds("merge-choice:0:2").unwrap();
        window.simulate_click(button.center(), Default::default());
        window.run_until_parked();
        view.read_with(window, |view, cx| {
            assert_eq!(view.editor.read(cx).content(), "ours\ntheirs\n")
        });
        view.update(window, |view, cx| {
            view.ask_agent(cx);
            assert!(view.notice.as_deref().unwrap().contains("未保存"));
        });
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        window
            .executor()
            .advance_clock(std::time::Duration::from_millis(600));
        window.run_until_parked();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "ours\ntheirs\n");
        assert!(
            crate::merge_conflicts::load(repo.path())
                .unwrap()
                .0
                .is_empty()
        );
    }

    #[gpui::test]
    fn expanded_diff_fits_its_slot_and_only_source_rows_scroll(cx: &mut gpui::TestAppContext) {
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.edits_ui.open = true;
            host.edits_ui.expanded = true;
            host.edits_ui.workspace_mode = true;
            host.edits_ui.workspace_files = Rc::new(vec![change(
                "long.rs",
                &(0..300)
                    .map(|i| format!("let row_{i} = \"{}\";\n", "value".repeat(80)))
                    .collect::<String>(),
            )]);
            host
        });
        for viewport in [size(px(1440.), px(900.)), size(px(900.), px(500.))] {
            window.simulate_resize(viewport);
            window.run_until_parked();
            let detail = window.debug_bounds("expanded-diff-content").unwrap();
            let body = window.debug_bounds("glass-diff-card-body").unwrap();
            let horizontal = window
                .debug_bounds("diff-card-horizontal-scrollbar")
                .unwrap();
            let footer = window.debug_bounds("diff-panel-footer").unwrap();
            assert!(body.size.height > px(40.));
            assert!(horizontal.bottom() <= detail.bottom() + px(1.));
            assert!(
                horizontal.bottom() <= footer.top() + px(1.),
                "horizontal rail stays visible above fixed footer"
            );
            assert!(
                window
                    .debug_bounds("diff-detail-vertical-scrollbar")
                    .is_none(),
                "no duplicate outer vertical rail for file diffs"
            );
            host.read_with(window, |host, _| {
                assert!(
                    host.edits_ui
                        .detail_scrolls
                        .borrow()
                        .values()
                        .all(|scroll| scroll.max_offset().y <= px(1.))
                );
            });
            window.simulate_event(gpui::ScrollWheelEvent {
                position: body.center(),
                delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-1000.))),
                ..Default::default()
            });
            window.run_until_parked();
            assert_eq!(window.debug_bounds("diff-panel-footer").unwrap(), footer);
            assert_eq!(
                window
                    .debug_bounds("diff-card-horizontal-scrollbar")
                    .unwrap(),
                horizontal
            );
        }
    }

    #[gpui::test]
    fn external_git_add_syncs_groups_without_reloading_unchanged_diff(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        let repo = crate::workspace_diff::tests::many_files(3);
        let root = repo.path().canonicalize().unwrap();
        let path = root.join("vendor/0000.rs");
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.history.active_mut().project = Some(root.clone());
            host.edits_ui.open = true;
            host.edits_ui.workspace_mode = true;
            host.edits_ui.expanded = true;
            host.edits_ui.selected = Some(path.clone());
            host.refresh_workspace_edits(cx);
            host
        });
        window.simulate_resize(size(px(1440.), px(1000.)));
        window.run_until_parked();
        host.update(window, |host, cx| {
            let source = host.edits_ui.workspace_sources[&path].clone();
            host.stage_workspace_files(vec![source], true, cx);
        });
        window.run_until_parked();
        let (id, card, revision) = host.read_with(window, |host, cx| {
            let view = host
                .edits_ui
                .file_views
                .borrow()
                .values()
                .find(|view| {
                    view.read(cx).file.path == path
                        && view
                            .read(cx)
                            .source
                            .as_ref()
                            .is_some_and(|s| s.layer == Some(true))
                })
                .unwrap()
                .clone();
            (
                view.read(cx).file.id,
                view.read(cx).card.as_ref().unwrap().entity_id(),
                host.edits_ui.workspace_revision,
            )
        });
        host.update(window, |host, cx| host.probe_workspace_repository(cx));
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert_eq!(
                host.edits_ui.workspace_revision, revision,
                "own index writes do not start full scans"
            )
        });
        // This is the same index mutation as `git add .` in the PTY terminal.
        crate::workspace_diff::tests::git_ok(&root, &["add", "."]);
        host.update(window, |host, cx| host.probe_workspace_repository(cx));
        window.run_until_parked();
        host.read_with(window, |host, cx| {
            assert!(
                host.edits_ui.workspace_error.is_none(),
                "{:?}",
                host.edits_ui.workspace_error
            );
            assert!(
                host.edits_ui
                    .workspace_sources
                    .values()
                    .all(|source| source.staged && !source.unstaged)
            );
            let views = host.edits_ui.file_views.borrow();
            let view = views
                .values()
                .find(|view| {
                    view.read(cx).file.path == path
                        && view
                            .read(cx)
                            .source
                            .as_ref()
                            .is_some_and(|s| s.layer == Some(true))
                })
                .unwrap()
                .read(cx);
            assert_eq!(view.file.id, id);
            assert_eq!(view.card.as_ref().unwrap().entity_id(), card);
        });
    }

    #[gpui::test]
    fn terminal_vendor_bursts_coalesce_and_keep_thousands_of_files_metadata_only(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        let repo = crate::workspace_diff::tests::many_files(1);
        let root = repo.path().canonicalize().unwrap();
        let path = root.join("vendor/0000.rs");
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.history.active_mut().project = Some(root.clone());
            host.history.workspace_display = crate::workspace_tree::DisplayMode::Tree;
            host.edits_ui.open = true;
            host.edits_ui.workspace_mode = true;
            host.refresh_workspace_edits(cx);
            host
        });
        window.simulate_resize(size(px(1440.), px(1000.)));
        window.run_until_parked();
        let original = host.read_with(window, |host, _| host.edits_ui.workspace_files[0].id);
        let revision = host.read_with(window, |host, _| host.edits_ui.workspace_revision);
        for group in 0..32 {
            let directory = root.join(format!("vendor/generated/group-{group}"));
            std::fs::create_dir_all(&directory).unwrap();
            for file in 0..128 {
                std::fs::write(
                    directory.join(format!("file-{file}.go")),
                    "package generated\n",
                )
                .unwrap();
            }
        }
        host.update(window, |host, cx| {
            for _ in 0..100 {
                host.workspace_terminal_activity(&root, cx);
            }
        });
        window.run_until_parked();
        window
            .executor()
            .advance_clock(std::time::Duration::from_millis(899));
        host.read_with(window, |host, _| {
            assert_eq!(host.edits_ui.workspace_revision, revision)
        });
        let started = std::time::Instant::now();
        window
            .executor()
            .advance_clock(std::time::Duration::from_millis(1));
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert_eq!(
                host.edits_ui.workspace_revision,
                revision + 1,
                "one inventory for the entire output burst"
            );
            assert_eq!(host.edits_ui.workspace_files.len(), 4097);
            assert!(
                host.edits_ui
                    .workspace_files
                    .iter()
                    .all(|file| file.before.is_none() && file.after.is_empty())
            );
            assert_eq!(
                host.edits_ui
                    .workspace_files
                    .iter()
                    .find(|file| file.path == path)
                    .unwrap()
                    .id,
                original
            );
            assert!(
                host.edits_ui.workspace_stats_task.is_none(),
                "no eager whole-repository numstat for vendor updates"
            );
            assert!(
                host.edits_ui.file_views.borrow().len() < 96,
                "only visible rows construct file views"
            );
        });
        eprintln!(
            "4096 generated vendor files: coalesced metadata sync + visible tree frame={:?}",
            started.elapsed()
        );
    }

    #[gpui::test]
    fn workspace_diff_prefetch_yields_to_clicks_and_leaves_vendor_on_demand(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        let repo = crate::workspace_diff::tests::many_files(1);
        for label in ["first.rs", "second.rs"] {
            std::fs::write(repo.path().join(label), "// untracked\n").unwrap();
        }
        let inventory = crate::workspace_diff::inventory(repo.path()).unwrap();
        let root = inventory.root.clone();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.history.active_mut().project = Some(root.clone());
            host.edits_ui.open = true;
            host.edits_ui.workspace_mode = true;
            host.edits_ui.workspace_root = Some(root.clone());
            host.edits_ui.workspace_files = Rc::new(inventory.files);
            host.edits_ui.workspace_sources = Rc::new(inventory.sources);
            host
        });
        window.simulate_resize(size(px(1440.), px(1000.)));
        window.run_until_parked();

        let first = window.debug_bounds("edit-file:first.rs").unwrap();
        window.simulate_click(first.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, cx| {
            assert!(host.edits_ui.diff_prefetch.borrow().ready.is_empty());
            assert!(host.edits_ui.file_views.borrow().values().any(|view| {
                let view = view.read(cx);
                view.file.label == "first.rs" && view.loaded.is_some()
            }));
        });
        window.simulate_click(first.center(), Default::default());

        for _ in 0..2 {
            window.executor().advance_clock(DIFF_PREFETCH_DELAY);
            window.run_until_parked();
        }
        host.read_with(window, |host, _| {
            let prefetch = host.edits_ui.diff_prefetch.borrow();
            assert_eq!(prefetch.ready.len(), 1);
            assert_eq!(
                prefetch.ready.values().next().unwrap().change.label,
                "second.rs"
            );
            assert!(!prefetch.has_pending());
        });

        let second = window.debug_bounds("edit-file:second.rs").unwrap();
        window.simulate_click(second.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, cx| {
            assert!(host.edits_ui.diff_prefetch.borrow().ready.is_empty());
            assert!(host.edits_ui.file_views.borrow().values().any(|view| {
                let view = view.read(cx);
                view.file.label == "second.rs" && view.loaded.is_some()
            }));
        });

        let vendor = root.join("vendor/0000.rs");
        host.update(window, |host, cx| {
            Rc::make_mut(&mut host.edits_ui.workspace_files).retain(|file| file.path == vendor);
            Rc::make_mut(&mut host.edits_ui.workspace_sources).retain(|path, _| *path == vendor);
            host.edits_ui.grouped_rows.borrow_mut().take();
            host.edits_ui.expanded = true;
            cx.notify();
        });
        window.run_until_parked();
        assert!(window.debug_bounds("edit-file:vendor/0000.rs").is_none());
        let selector =
            Box::leak(format!("change-file:{}:false", vendor.display()).into_boxed_str());
        let row = window.debug_bounds(selector).unwrap();
        window.simulate_click(row.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, cx| {
            assert!(host.edits_ui.file_views.borrow().values().any(|view| {
                let view = view.read(cx);
                view.file.path == vendor && view.loaded.is_some()
            }));
        });
    }

    #[gpui::test]
    fn directory_checkbox_stages_collapsed_descendants_in_both_diff_layouts(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        let repo = crate::workspace_diff::tests::many_files(1);
        std::fs::create_dir(repo.path().join("vendor/nested")).unwrap();
        std::fs::create_dir(repo.path().join("vendor-other")).unwrap();
        for file in ["vendor/nested/new.rs", "vendor-other/unrelated.rs"] {
            std::fs::write(repo.path().join(file), "// untracked\n").unwrap();
        }
        let inventory = crate::workspace_diff::inventory(repo.path()).unwrap();
        let root = inventory.root.clone();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.history.active_mut().project = Some(root.clone());
            host.history.workspace_display = crate::workspace_tree::DisplayMode::Tree;
            host.edits_ui.workspace_root = Some(root.clone());
            host.edits_ui.open = true;
            host.edits_ui.workspace_mode = true;
            host.edits_ui.workspace_files = Rc::new(inventory.files);
            host.edits_ui.workspace_sources = Rc::new(inventory.sources);
            host
        });
        window.simulate_resize(size(px(1440.), px(1000.)));
        for expanded in [false, true] {
            host.update(window, |host, cx| {
                host.edits_ui.expanded = expanded;
                cx.notify();
            });
            window.run_until_parked();
            let checkbox = window.debug_bounds("stage-directory:vendor:false").unwrap();
            if window
                .debug_bounds("stage-directory:vendor/nested:false")
                .is_some()
            {
                let directory = point(checkbox.left() - px(16.), checkbox.center().y);
                window.simulate_click(directory, Default::default());
                window.run_until_parked();
            }
            assert!(
                window
                    .debug_bounds("stage-directory:vendor/nested:false")
                    .is_none()
            );
            window.simulate_click(checkbox.center(), Default::default());
            window.run_until_parked();
            let sources = crate::workspace_diff::inventory(repo.path())
                .unwrap()
                .sources;
            assert!(sources[&root.join("vendor/0000.rs")].staged);
            assert!(sources[&root.join("vendor/nested/new.rs")].staged);
            assert!(!sources[&root.join("vendor-other/unrelated.rs")].staged);

            let checkbox = window.debug_bounds("stage-directory:vendor:true").unwrap();
            window.simulate_click(checkbox.center(), Default::default());
            window.run_until_parked();
            assert!(
                crate::workspace_diff::inventory(repo.path())
                    .unwrap()
                    .sources
                    .values()
                    .all(|source| !source.staged)
            );
            assert!(
                window
                    .debug_bounds("stage-directory:vendor/nested:false")
                    .is_none()
            );
        }
    }

    #[gpui::test]
    fn staging_queues_other_files_and_preserves_unrelated_diff(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        let repo = crate::workspace_diff::tests::many_files(3);
        let inventory = crate::workspace_diff::inventory(repo.path()).unwrap();
        let root = inventory.root.clone();
        let first = inventory.sources[&root.join("vendor/0000.rs")].clone();
        let second = inventory.sources[&root.join("vendor/0001.rs")].clone();
        let untouched = root.join("vendor/0002.rs");
        let untouched_id = inventory
            .files
            .iter()
            .find(|file| file.path == untouched)
            .unwrap()
            .id;
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.history.active_mut().project = Some(root.clone());
            host.edits_ui.workspace_root = Some(root.clone());
            host.edits_ui.open = true;
            host.edits_ui.workspace_mode = true;
            host.edits_ui.workspace_files = Rc::new(inventory.files);
            host.edits_ui.workspace_sources = Rc::new(inventory.sources);
            host
        });
        window.simulate_resize(size(px(1440.), px(1000.)));
        window.run_until_parked();
        let row = window.debug_bounds("edit-file:vendor/0002.rs").unwrap();
        window.simulate_click(row.center(), Default::default());
        window.run_until_parked();
        let (view, card) = host.read_with(window, |host, cx| {
            let view = host
                .edits_ui
                .file_views
                .borrow()
                .values()
                .find(|view| view.read(cx).file.path == untouched)
                .unwrap()
                .clone();
            let card = view.read(cx).card.as_ref().unwrap().entity_id();
            (view, card)
        });
        let toolbar = window.debug_bounds("diff-source-toolbar").unwrap();
        host.update(window, |host, cx| {
            host.stage_workspace_files(vec![first], true, cx);
            host.stage_workspace_files(vec![second], true, cx);
            assert!(host.edits_ui.workspace_task.is_none());
            assert!(host.edits_ui.stage_task.is_some());
            assert_eq!(host.edits_ui.stage_queue.len(), 1);
            assert_eq!(host.edits_ui.stage_pending.len(), 2);
            assert!(!host.edits_ui.stage_pending.contains(&untouched));
        });
        window.run_until_parked();
        assert_eq!(window.debug_bounds("diff-source-toolbar").unwrap(), toolbar);
        host.read_with(window, |host, cx| {
            assert!(host.edits_ui.stage_pending.is_empty());
            assert!(
                host.edits_ui.workspace_error.is_none(),
                "{:?}",
                host.edits_ui.workspace_error
            );
            assert_eq!(
                host.edits_ui.workspace_revision, 0,
                "checkboxes do not start full inventory refreshes"
            );
            assert_eq!(
                host.edits_ui
                    .workspace_sources
                    .values()
                    .filter(|source| source.staged)
                    .count(),
                2
            );
            assert_eq!(
                host.edits_ui
                    .workspace_files
                    .iter()
                    .find(|file| file.path == untouched)
                    .unwrap()
                    .id,
                untouched_id
            );
            let view = view.read(cx);
            assert!(view.expanded);
            assert_eq!(
                view.card.as_ref().unwrap().entity_id(),
                card,
                "unrelated Diff remains mounted"
            );
        });
    }

    #[gpui::test]
    fn grouped_diff_keeps_partial_staging_previews_separate(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        let repo = crate::workspace_diff::tests::many_files(1);
        crate::workspace_diff::tests::git_ok(repo.path(), &["add", "vendor/0000.rs"]);
        std::fs::write(
            repo.path().join("vendor/0000.rs"),
            "fn before() {}\n// changed 0\n// unstaged\n",
        )
        .unwrap();
        let inventory = crate::workspace_diff::inventory(repo.path()).unwrap();
        let root = inventory.root.clone();
        let path = root.join("vendor/0000.rs");
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.history.active_mut().project = Some(root.clone());
            host.edits_ui.workspace_root = Some(root.clone());
            host.edits_ui.open = true;
            host.edits_ui.workspace_mode = true;
            host.edits_ui.expanded = true;
            host.edits_ui.selected = Some(path.clone());
            host.edits_ui.workspace_files = Rc::new(inventory.files);
            host.edits_ui.workspace_sources = Rc::new(inventory.sources);
            host
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.run_until_parked();
        assert!(window.debug_bounds("stage-group:false").is_some());
        assert!(window.debug_bounds("stage-group:true").is_some());
        let selector = Box::leak(format!("change-file:{}:true", path.display()).into_boxed_str());
        let staged_row = window.debug_bounds(selector).unwrap();
        window.simulate_click(staged_row.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, cx| {
            assert!(host.edits_ui.selected_staged);
            let views = host.edits_ui.file_views.borrow();
            assert_eq!(views.len(), 2);
            for view in views.values() {
                let view = view.read(cx);
                let source = view.source.as_ref().unwrap();
                let loaded = view.loaded.as_ref().unwrap();
                assert_eq!((loaded.added, loaded.removed), (1, 0));
                assert_eq!(
                    loaded.after.contains("unstaged"),
                    source.layer == Some(false)
                );
            }
        });
        // A checkbox in the navigation must stage without selecting its row.
        let selector = Box::leak(format!("stage-file:{}:false", path.display()).into_boxed_str());
        let checkbox = window.debug_bounds(selector).unwrap();
        window.simulate_click(checkbox.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert!(host.edits_ui.selected_staged);
            assert!(
                host.edits_ui
                    .workspace_sources
                    .values()
                    .all(|s| s.staged && !s.unstaged)
            );
        });
    }

    #[gpui::test]
    fn conflict_card_renders_highlighted_side_previews(cx: &mut gpui::TestAppContext) {
        let repo = crate::merge_conflicts::tests::conflicted();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(data.path().join("sessions.json"), cx)
        });
        let mut file = crate::merge_conflicts::load(repo.path())
            .unwrap()
            .0
            .remove(0);
        file.original = format!("before\n\n{}\n// after\n", file.original);
        let (view, window) = window.add_window_view(|_, cx| {
            ConflictView::new(
                file,
                repo.path().into(),
                Some(host.downgrade()),
                ThemePreset::default(),
                cx,
            )
        });
        window.simulate_resize(size(px(700.), px(900.)));
        window.run_until_parked();
        view.update(window, |view, cx| {
            view.expanded = true;
            cx.notify();
        });
        window.run_until_parked();
        assert!(window.debug_bounds("merge-side:0:0").is_some());
        assert!(window.debug_bounds("merge-side:0:1").is_some());
        assert!(window.debug_bounds("merge-result-editor").is_none());
        let toggle = window.debug_bounds("toggle-merge-editor").unwrap();
        window.simulate_click(toggle.center(), Default::default());
        window.run_until_parked();
        assert!(window.debug_bounds("merge-result-editor").is_some());
        view.read_with(window, |view, cx| {
            assert_eq!(view.count, 1);
            let sides = view.sides.borrow();
            assert_eq!(
                sides.get(&(usize::MAX, 0)).unwrap().read(cx).content(),
                "before\n"
            );
            assert_eq!(
                sides.get(&(usize::MAX, 1)).unwrap().read(cx).content(),
                "\n// after"
            );
            assert_eq!(
                sides.get(&(0, 0)).unwrap().read(cx).content(),
                "<<<<<<< HEAD\nours"
            );
            assert_eq!(
                sides.get(&(0, 1)).unwrap().read(cx).content(),
                "=======\ntheirs\n>>>>>>> feature"
            );
        });
        let button = window.debug_bounds("merge-choice:0:0").unwrap();
        window.simulate_click(button.center(), Default::default());
        window.run_until_parked();
        view.read_with(window, |view, cx| {
            assert_eq!(
                view.editor.read(cx).content(),
                "before\n\nours\n\n// after\n"
            );
            assert!(view.sides.borrow().is_empty());
        });
        assert!(window.debug_bounds("merge-resolved").is_some());
    }

    #[gpui::test]
    fn command_opens_project_workspace_diff_instead_of_session_changes(
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
        window.update(|window, cx| {
            host.update(cx, |host, cx| {
                host.history.active_mut().project = Some(dir.path().into());
                host.git_ui.open = true;
                host.file_preview.open = true;
                host.edits_ui.selected = Some(dir.path().join("old.txt"));
                host.commands.pending = Some("workspace-diff".into());
                host.execute_pending_command(window, cx);
                assert!(host.edits_ui.open && host.edits_ui.workspace_mode);
                assert_eq!(host.edits_ui.workspace_root.as_deref(), Some(dir.path()));
                assert!(host.edits_ui.selected.is_none());
                assert!(!host.git_ui.open && !host.file_preview.open && !host.terminal_ui.open);
            })
        });
    }

    fn change(name: &str, content: &str) -> Change {
        serde_json::from_value(serde_json::json!({
            "id": Uuid::new_v4(), "path": name, "label": name,
            "before": "old\n", "after": content, "added": 1, "removed": 1
        }))
        .unwrap()
    }

    struct FileStack(Vec<Entity<FileDiff>>);
    impl Render for FileStack {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .children(self.0.iter().cloned())
        }
    }

    #[gpui::test]
    fn native_undo_buttons_restore_files_and_persist_reverted_history(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let anchor = root.join("sessions.json");
        std::fs::write(
            root.join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let a = root.join("a.rs");
        let b = root.join("b.rs");
        std::fs::write(&a, "after-a").unwrap();
        std::fs::write(&b, "after-b").unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut h = AgentWorkspace::with_path(anchor.clone(), cx);
            h.history.active_mut().project = Some(root.clone());
            h.history
                .active_mut()
                .push(model::Role::Agent, "changed files".into());
            h.history.active_mut().messages.last_mut().unwrap().tools = [
                (a.clone(), "before-a", "after-a"),
                (b.clone(), "before-b", "after-b"),
            ]
            .into_iter()
            .map(|(path, before, after)| tools::ToolRecord {
                call: tools::ToolCall {
                    id: Uuid::new_v4().to_string(),
                    name: "write_file".into(),
                    arguments: serde_json::json!({"path":path}),
                },
                result: "ok".into(),
                change: Some(crate::undo::tests::change(path, Some(before), after)),
                subagent: None,
                success: true,
                elapsed_ms: 0,
                live_output: None,
                nested_tools: Default::default(),
            })
            .collect();
            h.open_edits(None, cx);
            h
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.run_until_parked();
        let selector = Box::leak(format!("undo-file:{}", a.display()).into_boxed_str());
        let button = window.debug_bounds(selector).unwrap();
        window.simulate_click(button.center(), Default::default());
        window.run_until_parked();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "before-a");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "after-b");
        let button = window.debug_bounds("undo-session-changes").unwrap();
        window.simulate_click(button.center(), Default::default());
        window.run_until_parked();
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "before-b");
        host.read_with(window, |h, _| {
            assert!(h.session_edits().is_empty());
            assert_eq!(h.history.active().reverted_changes.len(), 2);
            h.history.save(&anchor).unwrap();
        });
        let history = model::History::load(&anchor).unwrap();
        assert_eq!(history.active().reverted_changes.len(), 2);
        let result: serde_json::Value = serde_json::from_str(
            &crate::session_changes::read(
                &anchor,
                Some(&root),
                None,
                Some(history.active),
                &serde_json::json!({"path":""}),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(result["active_changes"], 0);
        assert!(
            result["records"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["reverted"] == true)
        );
    }
    #[gpui::test]
    fn large_file_waits_for_manual_click_then_installs_background_prepared_diff(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        let repo = tempfile::tempdir().unwrap();
        crate::workspace_diff::tests::git_ok(repo.path(), &["init"]);
        std::fs::write(
            repo.path().join("large.yaml"),
            "key: original value\n".repeat(120_000),
        )
        .unwrap();
        let inventory = crate::workspace_diff::inventory(repo.path()).unwrap();
        let file = inventory.files[0].clone();
        let source = inventory.sources[&file.path].for_layer(false);
        let (view, window) = cx.add_window_view(|_, _| {
            let mut view = FileDiff::new(file, ThemePreset::default());
            view.source = Some(source);
            view.expanded = true;
            view.side_by_side = true;
            view
        });
        window.run_until_parked();
        view.read_with(window, |view, _| {
            assert!(view.manual_load_available);
            assert!(view.loaded.is_none() && view.card.is_none() && view.load_task.is_none());
        });
        let button = window.debug_bounds("load-file-diff-manually").unwrap();
        window.simulate_click(button.center(), gpui::Modifiers::default());
        window.run_until_parked();
        view.read_with(window, |view, cx| {
            assert!(view.load_error.is_none(), "{:?}", view.load_error);
            assert!(view.loaded.as_ref().unwrap().after.len() > 2 * 1024 * 1024);
            assert!(view.card.as_ref().unwrap().read(cx).stats().0 >= 120_000);
        });
        // Explicit loading is not carried over to a later file revision.
        view.update(window, |view, cx| {
            let mut next = view.file.clone();
            next.id = Uuid::new_v4();
            view.set_file(&next, view.theme, cx);
        });
        window.run_until_parked();
        view.read_with(window, |view, _| {
            assert!(view.manual_load_available);
            assert!(view.loaded.is_none() && view.card.is_none());
        });
    }

    #[gpui::test]
    fn files_expand_independently_and_closed_diffs_are_lazy(cx: &mut gpui::TestAppContext) {
        let (stack, window) = cx.add_window_view(|_, cx| {
            FileStack(vec![
                cx.new(|_| FileDiff::new(change("first.rs", "first\n"), ThemePreset::default())),
                cx.new(|_| FileDiff::new(change("second.rs", "second\n"), ThemePreset::default())),
            ])
        });
        window.run_until_parked();
        let files = stack.update(window, |stack, _| stack.0.clone());
        for file in &files {
            file.update(window, |file, _| assert!(file.card.is_none()));
        }
        let first = window.debug_bounds("edit-file:first.rs").unwrap();
        window.simulate_click(first.center(), gpui::Modifiers::default());
        window.run_until_parked();
        let second = window.debug_bounds("edit-file:second.rs").unwrap();
        assert!(
            second.top() > first.bottom() + px(30.),
            "first file diff must sit directly below its header"
        );
        window.simulate_click(second.center(), gpui::Modifiers::default());
        window.run_until_parked();
        for file in &files {
            file.update(window, |file, _| {
                assert!(file.expanded && file.card.is_some())
            });
        }
        assert!(
            window.debug_bounds("glass-diff-card-header").is_none(),
            "file disclosure is the only header"
        );
        window.simulate_click(first.center(), gpui::Modifiers::default());
        window.run_until_parked();
        files[0].update(window, |file, cx| {
            assert!(!file.expanded);
            file.set_file(&change("first.rs", "new contents\n"), file.theme, cx);
            assert!(
                file.card.is_none(),
                "closed files must not rebuild hidden diff content"
            );
            file.reveal(1, cx);
        });
        window.run_until_parked();
        files[0].update(window, |file, cx| {
            assert!(file.expanded && file.card.is_some());
            file.expanded = false;
            file.reveal(1, cx);
            assert!(
                !file.expanded,
                "ordinary re-renders must not override a user's collapse"
            );
            file.reveal(2, cx);
            assert!(
                file.expanded,
                "a new summary click reopens the requested file"
            );
        });
        files[1].update(window, |file, _| assert!(file.expanded));
    }
}
