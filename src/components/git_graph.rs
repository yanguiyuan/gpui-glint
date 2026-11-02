//! Reusable Git history and branch comparison viewer. The host owns all Git I/O.
use super::{Icon, IconButton, Select, SelectEvent, SelectOption};
use crate::theme::ThemePreset;
use gpui::{
    AppContext, Context, Entity, EventEmitter, Render, ScrollHandle, SharedString, Window, div,
    point, prelude::*, px, rgb,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitCommit {
    pub id: String,
    pub parents: Vec<String>,
    pub summary: String,
    pub author: String,
    pub date: String,
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
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitGraphEvent {
    CompareBranch(String),
    SelectCommit(String),
    OpenFile { commit: String, path: String },
    Refresh,
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

pub struct GitGraph {
    data: GitGraphData,
    theme: ThemePreset,
    compare: Entity<Select>,
    selected: Option<String>,
    details: Option<String>,
    commit_details: Option<GitCommitDetails>,
    copied_hash: bool,
    error: Option<String>,
    loading: bool,
    comparison_tab: bool,
    scroll: ScrollHandle,
    _subscription: gpui::Subscription,
}
impl EventEmitter<GitGraphEvent> for GitGraph {}
impl GitGraph {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let compare = cx.new(|cx| Select::new([], cx).searchable(true).width(160.));
        let subscription = cx.subscribe(&compare, |_, _, event: &SelectEvent, cx| {
            let SelectEvent::Changed(branch) = event;
            cx.emit(GitGraphEvent::CompareBranch(branch.to_string()));
        });
        Self {
            data: GitGraphData::default(),
            theme: ThemePreset::default(),
            compare,
            selected: None,
            details: None,
            commit_details: None,
            copied_hash: false,
            error: None,
            loading: false,
            comparison_tab: false,
            scroll: ScrollHandle::new(),
            _subscription: subscription,
        }
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            self.compare.update(cx, |s, cx| s.set_theme(theme, cx));
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
        self.data = data;
        self.error = None;
        self.loading = false;
        cx.notify();
    }
    /// Clear previous-project state before an asynchronous reload.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.data = GitGraphData::default();
        self.selected = None;
        self.details = None;
        self.commit_details = None;
        self.copied_hash = false;
        self.compare.update(cx, |s, cx| {
            s.set_options([], cx);
            s.set_selected(None, cx);
        });
        self.error = None;
        self.loading = true;
        self.scroll = ScrollHandle::new();
        cx.notify();
    }
    pub fn set_loading(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        self.selected = None;
        self.details = None;
        self.commit_details = None;
        self.copied_hash = false;
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
            cx.notify();
        }
    }
    pub fn set_commit_details(&mut self, details: GitCommitDetails, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(&details.id) {
            self.commit_details = Some(details);
            self.details = None;
            self.copied_hash = false;
            cx.notify();
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
                div().mb_2().font_weight(gpui::FontWeight::SEMIBOLD).child(
                    details
                        .message
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_owned(),
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
                    .child(div().flex_1().min_w_0().truncate().child(path.clone()))
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
        div()
            .id(SharedString::from(format!("git-commit:{id}")))
            .debug_selector(move || format!("git-commit:{id}"))
            .h(px(68.))
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
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().truncate().child(commit.summary.clone()))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .truncate()
                            .child(format!(
                                "{} · {} · {}",
                                commit.id.chars().take(8).collect::<String>(),
                                commit.author,
                                commit.date
                            )),
                    )
                    .when(!commit.references.is_empty(), |d| {
                        d.child(
                            div()
                                .truncate()
                                .text_size(px(10.))
                                .text_color(rgb(p.accent))
                                .child(commit.references.clone()),
                        )
                    }),
            )
            .on_click({
                let id = commit.id.clone();
                cx.listener(move |this, _, _, cx| {
                    this.selected = Some(id.clone());
                    this.details = None;
                    this.commit_details = None;
                    this.copied_hash = false;
                    cx.emit(GitGraphEvent::SelectCommit(id.clone()));
                    cx.notify();
                })
            })
    }
}
impl Render for GitGraph {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let mut body = div()
            .id("git-graph-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .p_3()
            .flex()
            .flex_col()
            .gap_2();
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .text_color(rgb(self.theme.color(0xd9342b)))
                    .child(error.clone()),
            );
        }
        if self.loading {
            body = body.child(div().text_color(rgb(p.text_muted)).child("正在读取 Git…"));
        }
        if self.comparison_tab {
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
                        body = body.child(format!("仅显示前 {} 条", commits.len()));
                    }
                }
            } else if let Some(error) = &self.data.comparison_error {
                body = body.child(error.clone());
            } else if !self.loading {
                body = body.child("尚无可对比的提交");
            }
        } else {
            // Each canvas ends at its row boundary. Keep graph rows contiguous;
            // the outer body's gap is only for notices and the history footer.
            let mut history = div().flex().flex_col().flex_shrink_0();
            for (commit, lane) in self.data.commits.iter().zip(lanes(&self.data.commits)) {
                history = history.child(self.commit_row(commit, Some(lane), cx));
            }
            body = body.child(history);
            if self.data.commits.is_empty() && !self.loading && self.error.is_none() {
                body = body.child("尚无提交");
            }
            if self.data.commits.len() >= 100 {
                body = body.child("显示最近 100 条提交");
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
            .bg(rgb(p.surface))
            .child(
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
                    .child(
                        IconButton::new(Icon::Refresh, "刷新 Git")
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(GitGraphEvent::Refresh))),
                    )
                    .child(
                        IconButton::new(Icon::Close, "关闭 GitGraph")
                            .theme(self.theme)
                            .render()
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(GitGraphEvent::CloseRequested)),
                            ),
                    ),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .truncate()
                            .child(format!("当前：{}", self.data.current_branch)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child("对比")
                            .child(self.compare.clone()),
                    )
                    .child(div().flex().gap_2().children(
                        [(false, "提交历史"), (true, "分支差异")].into_iter().map(
                            |(compare, label)| {
                                div()
                                    .id(label)
                                    .px_2()
                                    .py_1()
                                    .rounded(px(5.))
                                    .cursor_pointer()
                                    .when(self.comparison_tab == compare, |d| {
                                        d.bg(rgb(p.surface_subtle)).text_color(rgb(p.accent))
                                    })
                                    .child(label)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.comparison_tab = compare;
                                        cx.notify();
                                    }))
                            },
                        ),
                    )),
            )
            .child(body)
            .when(self.selected.is_some(), |root| {
                root.child(self.details_panel(cx))
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
            assert_eq!(previous.size.height, px(68.));
        }
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
