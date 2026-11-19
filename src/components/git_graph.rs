//! Reusable Git history and branch comparison viewer. The host owns all Git I/O.
use super::{
    Button, ButtonKind, ButtonSize, ChangeNavigation, ChangeNavigationEvent, ChangeRow, Icon,
    IconButton, SearchInput, SegmentOption, SegmentedControl, Select, SelectEvent, SelectOption,
    Skeleton, TextEntry, search_highlight,
};
use crate::theme::ThemePreset;
use gpui::{
    AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, HighlightStyle, Render,
    ScrollHandle, SharedString, StyledText, Window, div, point, prelude::*, px, rgb,
};

/// Row height when the commit carries branch or tag decorations.
const ROW_HEIGHT: f32 = 68.;
/// Shorter row without a decoration line, so undecorated history stays dense.
const ROW_HEIGHT_COMPACT: f32 = 52.;
/// Decorations rendered before the remaining ones collapse into a `+N` chip.
const VISIBLE_REFERENCE_CHIPS: usize = 3;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitCommit {
    pub id: String,
    pub parents: Vec<String>,
    pub summary: String,
    pub author: String,
    pub date: String,
    /// Git's `%D` decoration, e.g. `HEAD -> main, origin/main, tag: v1`.
    pub references: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitCommitFile {
    pub path: String,
    /// None represents a binary change, not zero lines.
    pub additions: Option<usize>,
    pub deletions: Option<usize>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitCommitDetails {
    pub id: String,
    pub parents: Vec<String>,
    pub author: String,
    pub authored_at: String,
    pub committer: String,
    pub committed_at: String,
    pub message: String,
    /// Changes relative to the first parent; root commits compare against an empty tree.
    pub files: Vec<GitCommitFile>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitComparison {
    pub current_count: usize,
    pub target_count: usize,
    pub current_only: Vec<GitCommit>,
    pub target_only: Vec<GitCommit>,
    pub file_changes: Option<Result<GitBranchFiles, String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitBranchFiles {
    /// The compared branch as resolved when the graph loaded.
    pub base: String,
    pub files: Vec<GitCommitFile>,
    pub untracked: std::collections::BTreeSet<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitGraphData {
    pub current_branch: String,
    pub branches: Vec<String>,
    pub compare_branch: String,
    /// Topological order, children before parents. Hosts may supply a bounded window.
    pub commits: Vec<GitCommit>,
    pub comparison: Option<GitComparison>,
    pub comparison_error: Option<String>,
    /// True when the host truncated the history window; the viewer then offers `LoadMore`.
    pub has_more: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitGraphEvent {
    CompareBranch(String),
    SelectCommit(String),
    OpenFile {
        commit: String,
        path: String,
    },
    OpenComparedFile {
        base: String,
        path: String,
    },
    RefreshFiles,
    Refresh,
    /// Request a wider history window from the host.
    LoadMore,
    CloseRequested,
}

#[derive(Clone, Debug)]
struct LaneRow {
    node: usize,
    incoming: bool,
    edges: Vec<(usize, usize, bool)>, // true: node -> parent; false: passing lane
    width: usize,
}
fn lanes(commits: &[GitCommit]) -> Vec<LaneRow> {
    lanes_for(&commits.iter().collect::<Vec<_>>())
}
fn lanes_for(commits: &[&GitCommit]) -> Vec<LaneRow> {
    let mut active = Vec::<String>::new();
    commits
        .iter()
        .map(|commit| {
            let incoming = active.contains(&commit.id);
            let node = active
                .iter()
                .position(|id| id == &commit.id)
                .unwrap_or_else(|| {
                    active.push(commit.id.clone());
                    active.len() - 1
                });
            let before = active.clone();
            active.remove(node);
            for (offset, parent) in commit.parents.iter().enumerate() {
                if !active.contains(parent) {
                    active.insert((node + offset).min(active.len()), parent.clone());
                }
            }
            let mut edges = Vec::new();
            for (index, id) in before.iter().enumerate() {
                if index != node {
                    if let Some(to) = active.iter().position(|item| item == id) {
                        edges.push((index, to, false));
                    }
                }
            }
            for parent in &commit.parents {
                if let Some(to) = active.iter().position(|id| id == parent) {
                    edges.push((node, to, true));
                }
            }
            LaneRow {
                node,
                incoming,
                edges,
                width: before.len().max(active.len()),
            }
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefKind {
    Head,
    Local,
    Remote,
    Tag,
}

fn classify_reference(name: &str) -> RefKind {
    if name == "HEAD" || name.ends_with("/HEAD") {
        RefKind::Head
    } else if name.contains('/') {
        RefKind::Remote
    } else {
        RefKind::Local
    }
}

/// Split Git's `%D` decoration into one entry per reference, dropping the `->` arrow.
fn reference_chips(references: &str) -> Vec<(String, RefKind)> {
    let mut chips = Vec::new();
    for token in references
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        if let Some(tag) = token.strip_prefix("tag: ") {
            chips.push((tag.to_owned(), RefKind::Tag));
        } else if let Some((from, to)) = token.split_once("->") {
            chips.push((from.trim().to_owned(), RefKind::Head));
            let name = to.trim();
            if !name.is_empty() {
                chips.push((name.to_owned(), classify_reference(name)));
            }
        } else {
            chips.push((token.to_owned(), classify_reference(token)));
        }
    }
    chips
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// Decorate one text run with search matches while keeping truncation intact.
fn highlighted(text: &str, query: &str, theme: ThemePreset) -> StyledText {
    let highlights = search_highlight::match_ranges(text, query.trim())
        .into_iter()
        .map(|range| {
            (
                range,
                HighlightStyle {
                    background_color: Some(search_highlight::color(theme.is_dark())),
                    ..Default::default()
                },
            )
        });
    StyledText::new(SharedString::from(text.to_owned())).with_highlights(highlights)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GraphTab {
    History,
    Comparison,
    Files,
}

pub struct GitGraph {
    data: GitGraphData,
    lanes: Vec<LaneRow>,
    theme: ThemePreset,
    compare: Entity<Select>,
    search: Entity<TextEntry>,
    query: String,
    merge_only: bool,
    labeled_only: bool,
    selected: Option<String>,
    details: Option<String>,
    commit_details: Option<GitCommitDetails>,
    copied_hash: bool,
    copied_message: bool,
    copied_commit: Option<String>,
    error: Option<String>,
    loading: bool,
    tab: GraphTab,
    branch_files: Entity<ChangeNavigation>,
    header_visible: bool,
    background_visible: bool,
    focus: FocusHandle,
    scroll: ScrollHandle,
    _search_subscription: gpui::Subscription,
    _subscription: gpui::Subscription,
    _files_subscription: gpui::Subscription,
}
impl EventEmitter<GitGraphEvent> for GitGraph {}
impl GitGraph {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let compare = cx.new(|cx| Select::new([], cx).searchable(true).width(160.));
        let subscription = cx.subscribe(&compare, |_, _, event: &SelectEvent, cx| {
            let SelectEvent::Changed(branch) = event;
            cx.emit(GitGraphEvent::CompareBranch(branch.to_string()));
        });
        let search = cx.new(|cx| TextEntry::new(cx, "搜索提交、Hash、作者", "").font_size(11.));
        let search_subscription = cx.observe(&search, |this: &mut Self, search, cx| {
            let query = search.read(cx).value().to_owned();
            if this.query != query {
                this.query = query;
                cx.notify();
            }
        });
        let branch_files = cx.new(|cx| {
            ChangeNavigation::new(cx)
                .background_visible(false)
                .empty_label("当前工作区与对比分支没有文件内容差异")
        });
        let files_subscription = cx.subscribe(
            &branch_files,
            |this: &mut Self, _, event: &ChangeNavigationEvent, cx| {
                if let ChangeNavigationEvent::Select { path, .. } = event
                    && let Some(Ok(files)) = this
                        .data
                        .comparison
                        .as_ref()
                        .and_then(|comparison| comparison.file_changes.as_ref())
                {
                    cx.emit(GitGraphEvent::OpenComparedFile {
                        base: files.base.clone(),
                        path: path.clone(),
                    });
                }
            },
        );
        Self {
            data: GitGraphData::default(),
            lanes: Vec::new(),
            theme: ThemePreset::default(),
            compare,
            search,
            query: String::new(),
            merge_only: false,
            labeled_only: false,
            selected: None,
            details: None,
            commit_details: None,
            copied_hash: false,
            copied_message: false,
            copied_commit: None,
            error: None,
            loading: false,
            tab: GraphTab::History,
            branch_files,
            header_visible: true,
            background_visible: true,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _search_subscription: search_subscription,
            _subscription: subscription,
            _files_subscription: files_subscription,
        }
    }
    pub fn header_visible(mut self, visible: bool) -> Self {
        self.header_visible = visible;
        self
    }

    pub fn background_visible(mut self, visible: bool) -> Self {
        self.background_visible = visible;
        self
    }

    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            self.compare.update(cx, |s, cx| s.set_theme(theme, cx));
            self.search.update(cx, |s, cx| s.set_theme(theme, cx));
            self.branch_files
                .update(cx, |files, cx| files.set_theme(theme, cx));
            cx.notify();
        }
    }
    pub fn set_data(&mut self, data: GitGraphData, cx: &mut Context<Self>) {
        self.compare.update(cx, |s, cx| {
            s.set_options(
                data.branches
                    .iter()
                    .map(|b| SelectOption::new(b.clone(), b.clone())),
                cx,
            );
            s.set_selected(Some(data.compare_branch.clone().into()), cx);
        });
        self.lanes = lanes(&data.commits);
        if data.compare_branch.is_empty() {
            self.tab = GraphTab::History;
        }
        if let Some(Ok(files)) = data
            .comparison
            .as_ref()
            .and_then(|comparison| comparison.file_changes.as_ref())
        {
            self.branch_files.update(cx, |navigation, cx| {
                navigation.set_rows(
                    files
                        .files
                        .iter()
                        .map(|file| {
                            let untracked = files.untracked.contains(&file.path);
                            ChangeRow::File {
                                path: file.path.clone(),
                                label: crate::paths::display_path(&file.path),
                                depth: 0,
                                status: untracked.then_some(super::ChangeStatus::Untracked),
                                staged: false,
                                additions: file.additions,
                                deletions: file.deletions,
                                binary: !untracked && file.additions.is_none(),
                            }
                        })
                        .collect(),
                    cx,
                );
            });
        }
        self.data = data;
        self.error = None;
        self.loading = false;
        self.copied_commit = None;
        cx.notify();
    }
    /// Clear previous-project state before an asynchronous reload.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.data = GitGraphData::default();
        self.lanes.clear();
        self.selected = None;
        self.details = None;
        self.commit_details = None;
        self.copied_hash = false;
        self.copied_message = false;
        self.copied_commit = None;
        self.compare.update(cx, |s, cx| {
            s.set_options([], cx);
            s.set_selected(None, cx);
        });
        self.error = None;
        self.loading = true;
        self.scroll = ScrollHandle::new();
        self.branch_files.update(cx, |files, cx| files.reset(cx));
        cx.notify();
    }
    pub fn set_loading(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        self.selected = None;
        self.details = None;
        self.commit_details = None;
        self.copied_hash = false;
        self.copied_message = false;
        self.copied_commit = None;
        cx.notify();
    }
    pub fn set_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        self.loading = false;
        cx.notify();
    }
    pub fn set_details(&mut self, id: &str, text: String, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(id) {
            self.details = Some(text);
            self.commit_details = None;
            self.copied_hash = false;
            self.copied_message = false;
            cx.notify();
        }
    }
    pub fn set_commit_details(&mut self, details: GitCommitDetails, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(&details.id) {
            self.commit_details = Some(details);
            self.details = None;
            self.copied_hash = false;
            self.copied_message = false;
            cx.notify();
        }
    }
    fn comparison_available(&self) -> bool {
        !self.data.compare_branch.is_empty()
    }
    fn filter_active(&self) -> bool {
        self.merge_only || self.labeled_only || !self.query.trim().is_empty()
    }
    /// Client-side filtering keeps the host payload untouched.
    fn visible_commits(&self) -> Vec<&GitCommit> {
        let query = self.query.trim().to_lowercase();
        self.data
            .commits
            .iter()
            .filter(|commit| {
                (!self.merge_only || commit.parents.len() > 1)
                    && (!self.labeled_only || !commit.references.is_empty())
                    && (query.is_empty()
                        || commit.summary.to_lowercase().contains(&query)
                        || commit.id.to_lowercase().contains(&query)
                        || commit.author.to_lowercase().contains(&query))
            })
            .collect()
    }
    fn select_commit(&mut self, id: &str, cx: &mut Context<Self>) {
        self.selected = Some(id.to_owned());
        self.details = None;
        self.commit_details = None;
        self.copied_hash = false;
        self.copied_message = false;
        cx.emit(GitGraphEvent::SelectCommit(id.to_owned()));
        cx.notify();
    }
    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        self.details = None;
        self.commit_details = None;
        self.copied_hash = false;
        self.copied_message = false;
        cx.notify();
    }
    /// Move the selection through the filtered rows and reveal it.
    fn step_selection(&mut self, forward: bool, cx: &mut Context<Self>) {
        let rows = self.visible_commits();
        if rows.is_empty() {
            return;
        }
        let index = self
            .selected
            .as_ref()
            .and_then(|id| rows.iter().position(|commit| &commit.id == id));
        let next = match (index, forward) {
            (Some(current), true) => (current + 1).min(rows.len() - 1),
            (Some(current), false) => current.saturating_sub(1),
            (None, _) => 0,
        };
        let id = rows[next].id.clone();
        self.scroll.scroll_to_item(next);
        self.select_commit(&id, cx);
    }
    fn ref_chip(&self, label: &str, kind: RefKind) -> gpui::Div {
        let p = self.theme.palette();
        let (background, foreground, border) = match kind {
            RefKind::Head => (
                self.theme.color(0xdcecff),
                self.theme.color(0x087bfa),
                self.theme.color(0xdcecff),
            ),
            RefKind::Local => (
                self.theme.color(0xf0f2ee),
                self.theme.color(0x4e5451),
                self.theme.color(0xdfe4df),
            ),
            RefKind::Remote => (p.surface, p.text_muted, p.border),
            RefKind::Tag => (
                self.theme.color(0xf3f5f8),
                self.theme.color(0xb37716),
                p.border,
            ),
        };
        div()
            .h(px(16.))
            .max_w(px(132.))
            .px(px(5.))
            .flex()
            .items_center()
            .rounded(px(4.))
            .border_1()
            .border_color(rgb(border))
            .bg(rgb(background))
            .text_color(rgb(foreground))
            .text_size(px(9.5))
            .child(div().min_w_0().truncate().child(label.to_owned()))
    }
    fn reference_row(&self, chips: &[(String, RefKind)]) -> gpui::Div {
        let mut row = div().flex().items_center().gap_1().overflow_hidden();
        for (label, kind) in chips.iter().take(VISIBLE_REFERENCE_CHIPS) {
            row = row.child(self.ref_chip(label, *kind));
        }
        if chips.len() > VISIBLE_REFERENCE_CHIPS {
            row = row.child(self.ref_chip(
                &format!("+{}", chips.len() - VISIBLE_REFERENCE_CHIPS),
                RefKind::Remote,
            ));
        }
        row
    }
    fn filter_chip(
        &self,
        label: &'static str,
        active: bool,
        selector: &'static str,
        toggle: fn(&mut Self),
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        div()
            .id(SharedString::from(format!("git-filter:{selector}")))
            .debug_selector(move || selector.to_string())
            .h(px(24.))
            .px(px(8.))
            .flex()
            .items_center()
            .flex_shrink_0()
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(if active {
                self.theme.color(0x087bfa)
            } else {
                p.border
            }))
            .bg(rgb(if active {
                self.theme.color(0xdcecff)
            } else {
                p.surface
            }))
            .text_color(rgb(if active {
                self.theme.color(0x087bfa)
            } else {
                p.text_muted
            }))
            .text_size(px(11.))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(p.surface_subtle)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                toggle(this);
                cx.notify();
            }))
    }
    fn load_more_button(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        Button::new("加载更多")
            .kind(ButtonKind::Glass)
            .size(ButtonSize::Small)
            .theme(self.theme)
            .render()
            .debug_selector(|| "git-load-more".into())
            .on_click(cx.listener(|_, _, _, cx| cx.emit(GitGraphEvent::LoadMore)))
    }
    fn refresh_button(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let button = IconButton::new(Icon::Refresh, "刷新 Git")
            .theme(self.theme)
            .render()
            .debug_selector(|| "git-refresh".into());
        if self.loading {
            button.opacity(0.45)
        } else {
            button.on_click(cx.listener(|_, _, _, cx| cx.emit(GitGraphEvent::Refresh)))
        }
    }
    fn details_panel(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let mut panel = div()
            .id("git-commit-details")
            .debug_selector(|| "git-commit-details".into())
            .max_h(px(320.))
            .flex_shrink_0()
            .overflow_y_scroll()
            .p_3()
            .border_t_1()
            .border_color(rgb(p.border))
            .text_size(px(12.));
        let Some(details) = &self.commit_details else {
            return panel.child(
                self.details
                    .clone()
                    .unwrap_or_else(|| "正在读取提交详情…".into()),
            );
        };
        let id = details.id.clone();
        let message = details.message.clone();
        let subject = details
            .message
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned();
        panel = panel
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .mb_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family("monospace")
                            .text_color(rgb(p.text_muted))
                            .child(id.clone()),
                    )
                    .child(
                        IconButton::new(
                            if self.copied_hash {
                                Icon::CheckCircle
                            } else {
                                Icon::Copy
                            },
                            if self.copied_hash {
                                "已复制完整 Hash"
                            } else {
                                "复制提交 Hash"
                            },
                        )
                        .theme(self.theme)
                        .render()
                        .id("copy-commit-hash")
                        .debug_selector(|| "copy-commit-hash".into())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(id.clone()));
                            this.copied_hash = true;
                            cx.notify();
                        })),
                    ),
            )
            .child(
                // 复制信息的按钮跟随标题，避免和 Hash 复制按钮在两个图标并排时难以区分。
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .mb_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(subject),
                    )
                    .child(
                        IconButton::new(
                            if self.copied_message {
                                Icon::CheckCircle
                            } else {
                                Icon::Copy
                            },
                            if self.copied_message {
                                "已复制提交信息"
                            } else {
                                "复制提交信息"
                            },
                        )
                        .theme(self.theme)
                        .render()
                        .id("copy-commit-message")
                        .debug_selector(|| "copy-commit-message".into())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(message.clone()));
                            this.copied_message = true;
                            cx.notify();
                        })),
                    ),
            )
            .when_some(
                details
                    .message
                    .split_once('\n')
                    .map(|(_, body)| body.trim())
                    .filter(|body| !body.is_empty()),
                |panel, body| {
                    panel.child(
                        div()
                            .mb_2()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(body.to_owned()),
                    )
                },
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(format!("{} · {}", details.author, details.authored_at)),
            );
        if details.committer != details.author || details.committed_at != details.authored_at {
            panel = panel.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(format!(
                        "提交者：{} · {}",
                        details.committer, details.committed_at
                    )),
            );
        }
        let additions: usize = details.files.iter().filter_map(|f| f.additions).sum();
        let deletions: usize = details.files.iter().filter_map(|f| f.deletions).sum();
        panel = panel.child(
            div()
                .mt_3()
                .mb_1()
                .flex()
                .items_center()
                .gap_2()
                .child(format!("{} 个改动文件", details.files.len()))
                .child(
                    div()
                        .text_color(rgb(self.theme.color(0x24864c)))
                        .child(format!("+{additions}")),
                )
                .child(
                    div()
                        .text_color(rgb(self.theme.color(0xd9342b)))
                        .child(format!("−{deletions}")),
                ),
        );
        if details.parents.len() > 1 {
            panel = panel.child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("合并提交：以下改动相对第一父提交"),
            );
        }
        for file in &details.files {
            let path = file.path.clone();
            let commit = details.id.clone();
            panel = panel.child(
                div()
                    .id(SharedString::from(format!("git-file:{}", file.path)))
                    .debug_selector({
                        let path = path.clone();
                        move || format!("git-file:{path}")
                    })
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_2()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(p.surface_subtle)))
                    .child(Icon::Code.render(13.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(crate::paths::display_path(&path)),
                    )
                    .when(file.additions.is_none(), |d| {
                        d.child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.text_muted))
                                .child("二进制"),
                        )
                    })
                    .when_some(file.additions, |d, count| {
                        d.child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(self.theme.color(0x24864c)))
                                .child(format!("+{count}")),
                        )
                    })
                    .when_some(file.deletions, |d, count| {
                        d.child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(self.theme.color(0xd9342b)))
                                .child(format!("−{count}")),
                        )
                    })
                    .child(Icon::ChevronRight.render(12.))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(GitGraphEvent::OpenFile {
                            commit: commit.clone(),
                            path: path.clone(),
                        })
                    })),
            );
        }
        panel
    }
    fn commit_row(
        &self,
        commit: &GitCommit,
        lane: Option<LaneRow>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let id = commit.id.clone();
        let selected = self.selected.as_ref() == Some(&id);
        let chips = reference_chips(&commit.references);
        let height = if chips.is_empty() {
            ROW_HEIGHT_COMPACT
        } else {
            ROW_HEIGHT
        };
        let mut text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .truncate()
                    .child(highlighted(&commit.summary, &self.query, self.theme)),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .truncate()
                    .child(format!(
                        "{} · {} · {}",
                        short_id(&commit.id),
                        commit.author,
                        commit.date
                    )),
            );
        if !chips.is_empty() {
            text = text.child(self.reference_row(&chips));
        }
        let copy_hash = commit.id.clone();
        let copied = self.copied_commit.as_deref() == Some(commit.id.as_str());
        div()
            .id(SharedString::from(format!("git-commit:{id}")))
            .debug_selector(move || format!("git-commit:{id}"))
            .h(px(height))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .rounded(px(6.))
            .cursor_pointer()
            .when(selected, |d| d.bg(rgb(p.surface_subtle)))
            .hover(|d| d.bg(rgb(p.surface_subtle)))
            .when_some(lane, |d, lane| {
                let width = (lane.width as f32 * 12. + 12.).max(28.);
                d.child(
                    gpui::canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _| {
                            let x = |lane: usize| bounds.left() + px(12. + lane as f32 * 12.);
                            let middle = bounds.center().y;
                            let colors = [p.accent, 0x9b6ed3, 0x38a080, 0xd09a38];
                            for &(from, to, from_node) in &lane.edges {
                                let mut path = gpui::PathBuilder::stroke(px(1.5));
                                path.move_to(point(
                                    x(from),
                                    if from_node { middle } else { bounds.top() },
                                ));
                                path.line_to(point(x(from), middle));
                                path.line_to(point(x(to), bounds.bottom()));
                                if let Ok(path) = path.build() {
                                    window.paint_path(path, rgb(colors[from % colors.len()]));
                                }
                            }
                            if lane.incoming {
                                let mut path = gpui::PathBuilder::stroke(px(1.5));
                                path.move_to(point(x(lane.node), bounds.top()));
                                path.line_to(point(x(lane.node), middle));
                                if let Ok(path) = path.build() {
                                    window.paint_path(path, rgb(colors[lane.node % colors.len()]));
                                }
                            }
                            let mut circle = gpui::PathBuilder::fill();
                            for i in 0..=20 {
                                let angle = i as f32 * std::f32::consts::TAU / 20.;
                                let pt = point(
                                    x(lane.node) + px(3.5 * angle.cos()),
                                    middle + px(3.5 * angle.sin()),
                                );
                                if i == 0 {
                                    circle.move_to(pt);
                                } else {
                                    circle.line_to(pt);
                                }
                            }
                            if let Ok(path) = circle.build() {
                                window.paint_path(path, rgb(colors[lane.node % colors.len()]));
                            }
                        },
                    )
                    .w(px(width))
                    .h_full()
                    .flex_shrink_0(),
                )
            })
            .child(text)
            .child(
                div()
                    .id(SharedString::from(format!("git-copy:{}", commit.id)))
                    .debug_selector({
                        let copy = commit.id.clone();
                        move || format!("git-copy:{copy}")
                    })
                    .size(px(22.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .text_color(rgb(if copied {
                        self.theme.color(0x087bfa)
                    } else {
                        p.text_muted
                    }))
                    .hover(|s| s.bg(rgb(p.border)))
                    .child(Icon::Copy.render(12.).text_color(rgb(if copied {
                        self.theme.color(0x087bfa)
                    } else {
                        p.text_muted
                    })))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy_hash.clone()));
                        this.copied_commit = Some(copy_hash.clone());
                        cx.stop_propagation();
                        cx.notify();
                    })),
            )
            .on_click({
                let id = commit.id.clone();
                cx.listener(move |this, _, window, cx| {
                    window.focus(&this.focus, cx);
                    this.select_commit(&id, cx);
                })
            })
    }
}
impl Render for GitGraph {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let tab = if self.comparison_available() {
            self.tab
        } else {
            GraphTab::History
        };
        let comparison_tab = tab != GraphTab::History;
        let show_files = tab == GraphTab::Files;
        let file_changes = self
            .data
            .comparison
            .as_ref()
            .and_then(|comparison| comparison.file_changes.as_ref());
        let file_list_ready = show_files
            && !self.loading
            && self.error.is_none()
            && file_changes.is_some_and(Result::is_ok);
        let mut body = div()
            .id("git-graph-scroll")
            .debug_selector(|| "git-graph-scroll".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .track_focus(&self.focus)
            .tab_index(0)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if this.search.focus_handle(cx).is_focused(window) {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "up" => {
                        this.step_selection(false, cx);
                        cx.stop_propagation();
                    }
                    "down" => {
                        this.step_selection(true, cx);
                        cx.stop_propagation();
                    }
                    "escape" if this.selected.is_some() => {
                        this.clear_selection(cx);
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .p_3()
            .flex()
            .flex_col()
            .gap_2();
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .debug_selector(|| "git-error".into())
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_2()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(self.theme.color(0xffdeda)))
                    .bg(rgb(self.theme.color(0xffe9e7)))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(self.theme.color(0xd9342b)))
                            .child(error.clone()),
                    )
                    .child(self.refresh_button(cx)),
            );
        }
        let content_empty = if comparison_tab {
            self.data.comparison.is_none() && self.data.comparison_error.is_none()
        } else {
            self.data.commits.is_empty()
        };
        if self.loading && content_empty {
            body = body.child(
                div()
                    .debug_selector(|| "git-history-loading-skeleton".into())
                    .flex()
                    .flex_col()
                    .gap_3()
                    .children((0..6usize).map(|index| {
                        div()
                            .flex()
                            .gap_3()
                            .items_start()
                            .child(
                                Skeleton::new(("git-node-loading", index))
                                    .width(12.)
                                    .height(12.)
                                    .rounded(6.)
                                    .theme(self.theme),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(
                                        Skeleton::new(("git-title-loading", index))
                                            .height(12.)
                                            .theme(self.theme),
                                    )
                                    .child(
                                        Skeleton::new(("git-meta-loading", index))
                                            .width(140.)
                                            .height(10.)
                                            .theme(self.theme),
                                    ),
                            )
                    })),
            );
        } else if self.loading {
            body = body.child(div().text_color(rgb(p.text_muted)).child("正在读取 Git…"));
        }
        if show_files {
            if !self.loading && self.error.is_none() {
                let error = file_changes
                    .and_then(|result| result.as_ref().err())
                    .or(self.data.comparison_error.as_ref());
                body = body.child(
                    div().text_color(rgb(p.text_muted)).child(
                        error
                            .cloned()
                            .unwrap_or_else(|| "没有可比较的提交".to_owned()),
                    ),
                );
            }
        } else if comparison_tab {
            if let Some(comparison) = &self.data.comparison {
                for (title, count, commits) in [
                    (
                        "当前分支独有".to_owned(),
                        comparison.current_count,
                        &comparison.current_only,
                    ),
                    (
                        format!("{} 独有", self.data.compare_branch),
                        comparison.target_count,
                        &comparison.target_only,
                    ),
                ] {
                    body = body.child(
                        div()
                            .mt_2()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!("{title} · {count}")),
                    );
                    if count == 0 {
                        body = body.child(div().text_color(rgb(p.text_muted)).child("无独有提交"));
                    }
                    for commit in commits {
                        body = body.child(self.commit_row(commit, None, cx));
                    }
                    if count > commits.len() {
                        body = body.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child(format!("仅显示前 {} 条", commits.len())),
                        );
                    }
                }
            } else if let Some(error) = &self.data.comparison_error {
                body = body.child(error.clone());
            } else if !self.loading {
                body = body.child(
                    div()
                        .text_color(rgb(p.text_muted))
                        .child("选一个对比分支查看差异"),
                );
            }
        } else {
            // Each canvas ends at its row boundary. Keep graph rows contiguous;
            // the outer body's gap is only for notices and the history footer.
            let visible = self.visible_commits();
            let lanes = lanes_for(&visible);
            let mut history = div().flex().flex_col().flex_shrink_0();
            for (commit, lane) in visible.iter().copied().zip(lanes) {
                history = history.child(self.commit_row(commit, Some(lane), cx));
            }
            body = body.child(history);
            if visible.is_empty() && !self.loading && self.error.is_none() {
                body = body.child(div().text_color(rgb(p.text_muted)).child(
                    if self.data.commits.is_empty() {
                        "尚无提交"
                    } else {
                        "没有匹配的提交"
                    },
                ));
            }
            if self.data.has_more {
                body = body.child(
                    div()
                        .flex()
                        .justify_center()
                        .pt_1()
                        .child(self.load_more_button(cx)),
                );
            }
        }
        div()
            .id("git-graph")
            .debug_selector(|| "git-graph".into())
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .text_size(px(12.))
            .text_color(rgb(p.text))
            .when(self.background_visible, |graph| graph.bg(rgb(p.surface)))
            .when(self.header_visible, |graph| {
                graph.child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_2()
                        .p_3()
                        .border_b_1()
                        .border_color(rgb(p.border))
                        .child(Icon::GitBranch.render(16.))
                        .child(div().flex_1().child("GitGraph"))
                        .child(self.refresh_button(cx))
                        .child(
                            IconButton::new(Icon::Close, "关闭 GitGraph")
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.emit(GitGraphEvent::CloseRequested)
                                })),
                        ),
                )
            })
            .child(self.filter_bar(tab, cx))
            .child(if file_list_ready {
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.branch_files.clone())
                    .into_any_element()
            } else {
                body.into_any_element()
            })
            .when(!show_files && self.selected.is_some(), |root| {
                root.child(self.details_panel(cx))
            })
    }
}
impl GitGraph {
    fn filter_bar(&self, tab: GraphTab, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let history = tab == GraphTab::History;
        let weak = cx.entity().downgrade();
        div()
            .px_3()
            .py_2()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_2()
            .border_b_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(format!("当前：{}", self.data.current_branch)),
                    )
                    .when(history && self.filter_active(), |row| {
                        row.child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(p.text_muted))
                                .child(format!("{} 条", self.visible_commits().len())),
                        )
                    })
                    .when(history, |row| {
                        row.child(self.filter_chip(
                            "合并",
                            self.merge_only,
                            "git-filter-merges",
                            |this| this.merge_only = !this.merge_only,
                            cx,
                        ))
                        .child(self.filter_chip(
                            "有标记",
                            self.labeled_only,
                            "git-filter-labeled",
                            |this| this.labeled_only = !this.labeled_only,
                            cx,
                        ))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child("对比")
                    .child(self.compare.clone()),
            )
            .child(
                SegmentedControl::new([
                    SegmentOption::new("提交历史").debug_selector("git-tab-history"),
                    SegmentOption::new("分支差异")
                        .debug_selector("git-tab-comparison")
                        .disabled(!self.comparison_available()),
                    SegmentOption::new("文件差异")
                        .debug_selector("git-tab-files")
                        .disabled(!self.comparison_available()),
                ])
                .id("git-graph-tabs")
                .theme(self.theme)
                .selected(tab as usize)
                .on_change(move |index, _, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        let tab = match index {
                            1 => GraphTab::Comparison,
                            2 => GraphTab::Files,
                            _ => GraphTab::History,
                        };
                        if tab == GraphTab::Files && this.tab != tab && !this.loading {
                            cx.emit(GitGraphEvent::RefreshFiles);
                        }
                        this.tab = tab;
                        this.scroll = ScrollHandle::new();
                        cx.notify();
                    });
                }),
            )
            .when(history, |row| {
                row.child(
                    SearchInput::new("搜索提交、Hash、作者")
                        .theme(self.theme)
                        .full_width()
                        .control(self.search.clone())
                        .render(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn structured_details_copy_full_hash_and_emit_file_navigation(cx: &mut gpui::TestAppContext) {
        const ID: &str = "0123456789abcdef0123456789abcdef01234567";
        let (graph, window) = cx.add_window_view(|_, cx| {
            let mut graph = GitGraph::new(cx);
            graph.selected = Some(ID.into());
            graph.set_commit_details(
                GitCommitDetails {
                    id: ID.into(),
                    author: "Author".into(),
                    message: "A clear summary".into(),
                    files: vec![GitCommitFile {
                        path: "src/main.rs".into(),
                        additions: Some(2),
                        deletions: Some(1),
                    }],
                    ..Default::default()
                },
                cx,
            );
            graph
        });
        window.simulate_resize(gpui::size(px(420.), px(700.)));
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let received = events.clone();
        let _observer = window.new(|cx: &mut Context<()>| {
            cx.subscribe(&graph, move |_, _, event: &GitGraphEvent, _| {
                received.borrow_mut().push(event.clone())
            })
            .detach();
        });
        window.run_until_parked();
        let copy = window.debug_bounds("copy-commit-hash").unwrap();
        window.simulate_click(copy.center(), Default::default());
        window.run_until_parked();
        assert_eq!(window.read_from_clipboard().unwrap().text().unwrap(), ID);
        let message = window.debug_bounds("copy-commit-message").unwrap();
        // 复制信息的按钮跟随标题，不与 Hash 复制图标并排，避免看起来像重复按钮。
        assert!(message.origin.y > copy.origin.y);
        window.simulate_click(message.center(), Default::default());
        window.run_until_parked();
        assert_eq!(
            window.read_from_clipboard().unwrap().text().unwrap(),
            "A clear summary"
        );
        let file = window.debug_bounds("git-file:src/main.rs").unwrap();
        window.simulate_click(file.center(), Default::default());
        window.run_until_parked();
        assert_eq!(
            *events.borrow(),
            vec![GitGraphEvent::OpenFile {
                commit: ID.into(),
                path: "src/main.rs".into()
            }]
        );
        graph.update(window, |g, cx| {
            g.reset(cx);
            g.set_commit_details(
                GitCommitDetails {
                    id: ID.into(),
                    ..Default::default()
                },
                cx,
            );
        });
        graph.read_with(window, |g, _| assert!(g.commit_details.is_none()));
    }
    #[gpui::test]
    fn history_rows_touch_so_graph_edges_connect_across_commits(cx: &mut gpui::TestAppContext) {
        let commit = |id: &str, parents: &[&str]| GitCommit {
            id: id.into(),
            parents: parents.iter().map(|s| s.to_string()).collect(),
            summary: format!("Commit {id}"),
            references: "HEAD -> feature".into(),
            ..Default::default()
        };
        let (_, window) = cx.add_window_view(|_, cx| {
            let mut graph = GitGraph::new(cx);
            graph.set_data(
                GitGraphData {
                    commits: vec![
                        commit("merge", &["left", "right"]),
                        commit("left", &["base"]),
                        commit("right", &["base"]),
                        commit("base", &[]),
                    ],
                    ..Default::default()
                },
                cx,
            );
            graph
        });
        window.simulate_resize(gpui::size(px(400.), px(700.)));
        window.run_until_parked();
        for pair in [
            "git-commit:merge",
            "git-commit:left",
            "git-commit:right",
            "git-commit:base",
        ]
        .windows(2)
        {
            let previous = window.debug_bounds(pair[0]).unwrap();
            let next = window.debug_bounds(pair[1]).unwrap();
            assert_eq!(
                previous.bottom(),
                next.top(),
                "Graph rows must have no vertical gap"
            );
            assert_eq!(
                previous.left(),
                next.left(),
                "Graph lanes must share a horizontal origin"
            );
            assert_eq!(previous.size.height, px(ROW_HEIGHT));
        }
    }
    #[gpui::test]
    fn undecorated_rows_are_compact_and_still_touch(cx: &mut gpui::TestAppContext) {
        let plain = |id: &str, parents: &[&str]| GitCommit {
            id: id.into(),
            parents: parents.iter().map(|s| s.to_string()).collect(),
            summary: format!("Commit {id}"),
            ..Default::default()
        };
        let (_, window) = cx.add_window_view(|_, cx| {
            let mut graph = GitGraph::new(cx);
            graph.set_data(
                GitGraphData {
                    commits: vec![
                        GitCommit {
                            references: "HEAD -> main, tag: v1".into(),
                            ..plain("tagged", &["plain"])
                        },
                        plain("plain", &[]),
                    ],
                    ..Default::default()
                },
                cx,
            );
            graph
        });
        window.simulate_resize(gpui::size(px(400.), px(700.)));
        window.run_until_parked();
        let tagged = window.debug_bounds("git-commit:tagged").unwrap();
        let plain_row = window.debug_bounds("git-commit:plain").unwrap();
        assert_eq!(tagged.size.height, px(ROW_HEIGHT));
        assert_eq!(plain_row.size.height, px(ROW_HEIGHT_COMPACT));
        assert_eq!(tagged.bottom(), plain_row.top());
    }
    #[gpui::test]
    fn search_and_filters_hide_rows_without_changing_host_data(cx: &mut gpui::TestAppContext) {
        let commit = |id: &str, summary: &str, parents: &[&str], references: &str| GitCommit {
            id: id.into(),
            parents: parents.iter().map(|s| s.to_string()).collect(),
            summary: summary.into(),
            references: references.into(),
            ..Default::default()
        };
        let (graph, window) = cx.add_window_view(|_, cx| {
            let mut graph = GitGraph::new(cx);
            graph.set_data(
                GitGraphData {
                    current_branch: "main".into(),
                    branches: vec!["main".into()],
                    compare_branch: "main".into(),
                    commits: vec![
                        commit(
                            "ffffff",
                            "Merge release",
                            &["aaaaaa", "bbbbbb"],
                            "HEAD -> main",
                        ),
                        commit("aaaaaa", "Add search", &["cccccc"], ""),
                        commit("bbbbbb", "Add filters", &["cccccc"], "tag: v1"),
                        commit("cccccc", "Initial commit", &[], ""),
                    ],
                    ..Default::default()
                },
                cx,
            );
            graph
        });
        window.simulate_resize(gpui::size(px(420.), px(700.)));
        window.run_until_parked();
        assert!(window.debug_bounds("git-commit:aaaaaa").is_some());
        graph.update(window, |graph, cx| {
            graph.merge_only = true;
            cx.notify();
        });
        window.run_until_parked();
        assert!(window.debug_bounds("git-commit:ffffff").is_some());
        assert!(window.debug_bounds("git-commit:aaaaaa").is_none());
        graph.update(window, |graph, cx| {
            graph.merge_only = false;
            graph.labeled_only = true;
            cx.notify();
        });
        window.run_until_parked();
        assert!(window.debug_bounds("git-commit:bbbbbb").is_some());
        assert!(window.debug_bounds("git-commit:cccccc").is_none());
        graph.update(window, |graph, cx| {
            graph.labeled_only = false;
            graph.query = "search".into();
            cx.notify();
        });
        window.run_until_parked();
        assert!(window.debug_bounds("git-commit:aaaaaa").is_some());
        assert!(window.debug_bounds("git-commit:ffffff").is_none());
        graph.read_with(window, |graph, _| {
            assert_eq!(graph.visible_commits().len(), 1);
            assert_eq!(graph.data.commits.len(), 4);
        });
    }
    #[gpui::test]
    fn truncated_history_offers_load_more(cx: &mut gpui::TestAppContext) {
        let (graph, window) = cx.add_window_view(|_, cx| {
            let mut graph = GitGraph::new(cx);
            graph.set_data(
                GitGraphData {
                    current_branch: "main".into(),
                    commits: vec![GitCommit {
                        id: "abcdef12".into(),
                        summary: "A commit".into(),
                        ..Default::default()
                    }],
                    has_more: true,
                    ..Default::default()
                },
                cx,
            );
            graph
        });
        window.simulate_resize(gpui::size(px(400.), px(600.)));
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let received = events.clone();
        let _observer = window.new(|cx: &mut Context<()>| {
            cx.subscribe(&graph, move |_, _, event: &GitGraphEvent, _| {
                received.borrow_mut().push(event.clone())
            })
            .detach();
        });
        window.run_until_parked();
        let button = window.debug_bounds("git-load-more").unwrap();
        window.simulate_click(button.center(), Default::default());
        window.run_until_parked();
        assert_eq!(*events.borrow(), vec![GitGraphEvent::LoadMore]);
    }
    #[gpui::test]
    fn comparison_tab_requires_a_compare_branch(cx: &mut gpui::TestAppContext) {
        let (graph, window) = cx.add_window_view(|_, cx| GitGraph::new(cx));
        window.simulate_resize(gpui::size(px(400.), px(600.)));
        window.run_until_parked();
        let tab = window.debug_bounds("git-tab-comparison").unwrap();
        window.simulate_click(tab.center(), Default::default());
        window.run_until_parked();
        graph.read_with(window, |graph, _| assert!(graph.tab == GraphTab::History));
    }

    #[gpui::test]
    fn branch_files_and_unique_commits_have_separate_tabs(cx: &mut gpui::TestAppContext) {
        let (graph, window) = cx.add_window_view(|_, cx| {
            let mut graph = GitGraph::new(cx);
            graph.set_data(
                GitGraphData {
                    current_branch: "feature".into(),
                    branches: vec!["master".into(), "feature".into()],
                    compare_branch: "master".into(),
                    comparison: Some(GitComparison {
                        current_count: 1,
                        current_only: vec![GitCommit {
                            id: "feature-commit".into(),
                            ..Default::default()
                        }],
                        file_changes: Some(Ok(GitBranchFiles {
                            base: "base".into(),
                            files: vec![GitCommitFile {
                                path: "main.rs".into(),
                                additions: Some(1),
                                deletions: Some(0),
                            }],
                            ..Default::default()
                        })),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
            );
            graph
        });
        window.simulate_resize(gpui::size(px(400.), px(600.)));
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let received = events.clone();
        let _observer = window.new(|cx: &mut Context<()>| {
            cx.subscribe(&graph, move |_, _, event: &GitGraphEvent, _| {
                received.borrow_mut().push(event.clone())
            })
            .detach();
        });
        let click = |window: &mut gpui::VisualTestContext, selector| {
            window.run_until_parked();
            let position = window.debug_bounds(selector).unwrap().center();
            window.simulate_click(position, Default::default());
            window.run_until_parked();
        };
        click(window, "git-tab-comparison");
        assert!(window.debug_bounds("git-commit:feature-commit").is_some());
        click(window, "git-tab-files");
        assert!(window.debug_bounds("git-commit:feature-commit").is_none());
        click(window, "change-file:main.rs:false");
        assert_eq!(
            *events.borrow(),
            [
                GitGraphEvent::RefreshFiles,
                GitGraphEvent::OpenComparedFile {
                    base: "base".into(),
                    path: "main.rs".into(),
                }
            ]
        );
    }
    #[test]
    fn merge_lanes_join_existing_parent_without_duplicate_lanes() {
        let commit = |id: &str, parents: &[&str]| GitCommit {
            id: id.into(),
            parents: parents.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        let layout = lanes(&[
            commit("merge", &["left", "right"]),
            commit("left", &["base"]),
            commit("right", &["base"]),
            commit("base", &[]),
        ]);
        assert_eq!(layout[0].edges, vec![(0, 0, true), (0, 1, true)]);
        assert_eq!(layout[2].edges, vec![(0, 0, false), (1, 0, true)]);
        assert_eq!(layout[3].width, 1);
    }
    #[test]
    fn reference_chips_split_head_branches_remotes_and_tags() {
        assert_eq!(
            reference_chips("HEAD -> main, origin/main, origin/HEAD, tag: v1.2"),
            vec![
                ("HEAD".to_owned(), RefKind::Head),
                ("main".to_owned(), RefKind::Local),
                ("origin/main".to_owned(), RefKind::Remote),
                ("origin/HEAD".to_owned(), RefKind::Head),
                ("v1.2".to_owned(), RefKind::Tag),
            ]
        );
        assert_eq!(
            reference_chips("feature/x"),
            vec![("feature/x".to_owned(), RefKind::Remote)]
        );
        assert!(reference_chips("").is_empty());
        assert!(reference_chips(" , ").is_empty());
    }

    #[gpui::test]
    fn commit_selection_emits_event_and_project_reset_rejects_stale_details(
        cx: &mut gpui::TestAppContext,
    ) {
        let (graph, window) = cx.add_window_view(|_, cx| {
            let mut graph = GitGraph::new(cx);
            graph.set_data(
                GitGraphData {
                    current_branch: "feature".into(),
                    branches: vec!["master".into(), "feature".into()],
                    compare_branch: "master".into(),
                    commits: vec![GitCommit {
                        id: "abcdef12".into(),
                        summary: "A commit".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                cx,
            );
            graph
        });
        window.simulate_resize(gpui::size(px(400.), px(600.)));
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let received = events.clone();
        let _observer = window.new(|cx: &mut Context<()>| {
            cx.subscribe(&graph, move |_, _, event: &GitGraphEvent, _| {
                received.borrow_mut().push(event.clone())
            })
            .detach();
        });
        window.run_until_parked();
        let row = window.debug_bounds("git-commit:abcdef12").unwrap();
        window.simulate_click(row.center(), Default::default());
        window.run_until_parked();
        assert_eq!(
            *events.borrow(),
            vec![GitGraphEvent::SelectCommit("abcdef12".into())]
        );
        assert!(window.debug_bounds("git-commit-details").is_some());
        graph.update(window, |g, cx| {
            g.reset(cx);
            g.set_details("abcdef12", "stale".into(), cx);
        });
        graph.read_with(window, |g, _| {
            assert!(g.details.is_none());
            assert!(g.data.commits.is_empty());
        });
    }
}
