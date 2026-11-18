//! Grouped change rows shared by the project explorer and the diff sidebar.
//! The host owns all Git I/O, tree building and staging; this only renders rows
//! and reports what the user did with them.
use super::{Checkbox, Icon, Scrollbar, ScrollbarState, Skeleton};
use crate::theme::ThemePreset;
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, ListAlignment, ListOffset, ListState,
    MouseButton, Render, SharedString, Window, div, prelude::*, px, rgb,
};

const ROW_HEIGHT: f32 = 34.;
const SKELETON_ROWS: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    Conflict,
}

impl ChangeStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Added => "新增",
            Self::Modified => "修改",
            Self::Deleted => "删除",
            Self::Renamed => "重命名",
            Self::Untracked => "未跟踪",
            Self::Conflict => "冲突",
        }
    }

    fn color(self, theme: ThemePreset) -> u32 {
        match self {
            Self::Added => theme.color(0x24864c),
            Self::Deleted | Self::Conflict => theme.color(0xd9342b),
            Self::Renamed => theme.color(0x087bfa),
            Self::Modified => theme.color(0xb37716),
            Self::Untracked => theme.palette().text_muted,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeRow {
    /// Group header, e.g. `已暂存 · 3`. `staged` drives the batch checkbox.
    Group {
        title: String,
        count: usize,
        staged: Option<bool>,
    },
    Directory {
        path: String,
        label: String,
        depth: usize,
        count: usize,
        collapsed: bool,
        /// Which group this directory belongs to, so the host can key collapse state.
        staged: bool,
    },
    File {
        path: String,
        label: String,
        depth: usize,
        status: ChangeStatus,
        /// Layer the row belongs to; it also drives the staging checkbox.
        staged: bool,
        /// Counts from `git diff --numstat`. `None` only means this layer has no
        /// numstat row, which is also how unknown-but-not-yet-loaded stats look,
        /// so it must never be rendered as if the file were binary.
        additions: Option<usize>,
        deletions: Option<usize>,
        /// The host loaded stats and still found no numstat row, i.e. Git inline
        /// data reported the file as binary. Only then is `二进制` shown.
        binary: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeNavigationEvent {
    Select {
        path: String,
        staged: bool,
    },
    ToggleDirectory {
        path: String,
        staged: bool,
    },
    /// Stage or unstage these files; the host resolves them against its own sources.
    Stage {
        paths: Vec<String>,
        staged: bool,
    },
    StageGroup {
        staged: bool,
    },
    Menu {
        path: String,
        position: gpui::Point<gpui::Pixels>,
    },
}

pub struct ChangeNavigation {
    rows: Vec<ChangeRow>,
    theme: ThemePreset,
    selected: Option<(String, bool)>,
    /// Staging is only offered where the host can actually move files between layers.
    stage_controls: bool,
    error: Option<String>,
    loading: bool,
    focus: FocusHandle,
    state: ListState,
    bar: ScrollbarState,
}

impl EventEmitter<ChangeNavigationEvent> for ChangeNavigation {}

impl ChangeNavigation {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            rows: Vec::new(),
            theme: ThemePreset::default(),
            selected: None,
            stage_controls: false,
            error: None,
            loading: false,
            focus: cx.focus_handle(),
            state: ListState::new(0, ListAlignment::Top, px(ROW_HEIGHT)),
            bar: ScrollbarState::default(),
        }
    }

    /// Files currently in the index, so a host footer can offer a commit.
    pub fn staged_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| matches!(row, ChangeRow::File { staged: true, .. }))
            .count()
    }

    pub fn stage_controls(mut self, enabled: bool) -> Self {
        self.stage_controls = enabled;
        self
    }

    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            cx.notify();
        }
    }

    /// Replace the rows. Common prefixes and suffixes reuse their list items so
    /// a background refresh does not jump the viewport.
    pub fn set_rows(&mut self, rows: Vec<ChangeRow>, cx: &mut Context<Self>) {
        let old_count = self.rows.len();
        let new_count = rows.len();
        if old_count != new_count {
            let common = self
                .rows
                .iter()
                .zip(rows.iter())
                .take_while(|(old, new)| old == new)
                .count();
            let suffix = self
                .rows
                .iter()
                .rev()
                .zip(rows.iter().rev())
                .take_while(|(old, new)| old == new)
                .count()
                .min(old_count.min(new_count).saturating_sub(common));
            let top = self.state.logical_scroll_top();
            let anchor = top
                .item_ix
                .checked_sub(common)
                .and_then(|index| self.rows.get(index))
                .and_then(|key| rows.iter().position(|candidate| candidate == key));
            if old_count == 0 {
                self.state.reset(new_count);
            } else {
                self.state.splice(
                    common..old_count.saturating_sub(suffix),
                    new_count.saturating_sub(common + suffix),
                );
            }
            self.state.clone().with_uniform_item_height(px(ROW_HEIGHT));
            if let Some(index) = anchor {
                self.state.scroll_to(ListOffset {
                    item_ix: index,
                    offset_in_item: top.offset_in_item,
                });
            }
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|key| !rows.iter().any(|row| row_key(row).as_ref() == Some(key)))
        {
            self.selected = None;
        }
        self.rows = rows;
        self.error = None;
        self.loading = false;
        cx.notify();
    }

    pub fn set_selected(&mut self, selected: Option<(String, bool)>, cx: &mut Context<Self>) {
        if self.selected != selected {
            self.selected = selected;
            cx.notify();
        }
    }

    pub fn set_loading(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        cx.notify();
    }

    pub fn set_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        self.loading = false;
        cx.notify();
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.rows.clear();
        self.selected = None;
        self.error = None;
        self.loading = true;
        self.state = ListState::new(0, ListAlignment::Top, px(ROW_HEIGHT));
        cx.notify();
    }

    fn file_rows(&self) -> Vec<(usize, &ChangeRow)> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, ChangeRow::File { .. }))
            .collect()
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let files = self.file_rows();
        if files.is_empty() {
            return;
        }
        let current = self.selected.as_ref().and_then(|key| {
            files
                .iter()
                .position(|(_, row)| row_key(row).as_ref() == Some(key))
        });
        let next = match (current, forward) {
            (Some(current), true) => (current + 1).min(files.len() - 1),
            (Some(current), false) => current.saturating_sub(1),
            (None, _) => 0,
        };
        let (index, row) = files[next];
        if let Some(key) = row_key(row) {
            self.selected = Some(key);
            self.state.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: px(0.),
            });
        }
        cx.notify();
    }

    fn group_row(
        &self,
        title: String,
        count: usize,
        staged: Option<bool>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let p = self.theme.palette();
        let mut row = div()
            .h(px(ROW_HEIGHT))
            .flex_shrink_0()
            .w_full()
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_size(px(12.))
            .child(format!("{title} · {count}"));
        if self.stage_controls
            && let Some(staged) = staged
        {
            let weak = cx.entity().downgrade();
            row = row.child(div().flex_1()).child(
                div()
                    .flex_shrink_0()
                    .p_1()
                    .id(SharedString::from(format!("stage-group:{staged}")))
                    .debug_selector(move || format!("stage-group:{staged}"))
                    .child(
                        Checkbox::new("")
                            .checked(staged)
                            .theme(self.theme)
                            .on_change(move |staged, _, cx| {
                                cx.stop_propagation();
                                let _ = weak.update(cx, |_, cx| {
                                    cx.emit(ChangeNavigationEvent::StageGroup { staged })
                                });
                            })
                            .render(),
                    )
                    .on_click(|_, _, cx| cx.stop_propagation()),
            );
        } else {
            row = row.child(div().flex_1()).child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child(count.to_string()),
            );
        }
        row
    }

    fn directory_row(
        &self,
        path: String,
        label: String,
        depth: usize,
        count: usize,
        collapsed: bool,
        staged: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let selector = format!("change-directory:{path}:{staged}");
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .h(px(ROW_HEIGHT))
            .flex_shrink_0()
            .w_full()
            .pr_3()
            .pl(px(12. + depth as f32 * 16.))
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .text_size(px(12.))
            .hover(|style| style.bg(rgb(p.surface_subtle)))
            .child(
                if collapsed {
                    Icon::ChevronRight
                } else {
                    Icon::ChevronDown
                }
                .render(12.),
            )
            .child(Icon::Folder.render(14.))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child(count.to_string()),
            )
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(ChangeNavigationEvent::ToggleDirectory {
                    path: path.clone(),
                    staged,
                });
            }))
    }

    fn file_row(
        &self,
        path: String,
        label: String,
        depth: usize,
        status: ChangeStatus,
        staged: bool,
        additions: Option<usize>,
        deletions: Option<usize>,
        binary: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let color = status.color(self.theme);
        let icon = Icon::for_file_name(&label);
        let selector = format!("change-file:{path}:{staged}");
        let active = self
            .selected
            .as_ref()
            .is_some_and(|(selected, layer)| selected == &path && *layer == staged);
        let mut row = div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .h(px(ROW_HEIGHT))
            .flex_shrink_0()
            .w_full()
            .pr_2()
            .pl(px(26. + depth as f32 * 16.))
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .text_size(px(12.))
            .when(active, |row| row.bg(rgb(p.accent_tint)))
            .hover(|row| row.bg(rgb(p.surface_subtle)))
            .child(icon.render(14.).text_color(rgb(color)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(rgb(color))
                    .child(label),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.))
                    .text_color(rgb(color))
                    .child(status.label()),
            );
        if binary && status != ChangeStatus::Untracked {
            row = row.child(
                div()
                    .debug_selector(|| "change-file-binary".into())
                    .flex_shrink_0()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("二进制"),
            );
        }
        for (count, sign, tint) in [
            (additions, "+", self.theme.color(0x24864c)),
            (deletions, "−", self.theme.color(0xd9342b)),
        ] {
            if let Some(count) = count {
                row = row.child(
                    div()
                        .flex_shrink_0()
                        .font_family("monospace")
                        .text_size(px(10.))
                        .text_color(rgb(tint))
                        .child(format!("{sign}{count}")),
                );
            }
        }
        if self.stage_controls {
            let weak = cx.entity().downgrade();
            let stage_path = path.clone();
            row = row.child(
                div()
                    .flex_shrink_0()
                    .p_1()
                    .id(SharedString::from(format!("stage-file:{path}:{staged}")))
                    .debug_selector({
                        let value = format!("stage-file:{path}:{staged}");
                        move || value.clone()
                    })
                    .child(
                        Checkbox::new("")
                            .checked(staged)
                            .theme(self.theme)
                            .on_change(move |next, _, cx| {
                                cx.stop_propagation();
                                let _ = weak.update(cx, |_, cx| {
                                    cx.emit(ChangeNavigationEvent::Stage {
                                        paths: vec![stage_path.clone()],
                                        staged: next,
                                    })
                                });
                            })
                            .render(),
                    )
                    .on_click(|_, _, cx| cx.stop_propagation()),
            );
        }
        let menu_path = path.clone();
        row.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |_, event: &gpui::MouseDownEvent, _, cx| {
                cx.emit(ChangeNavigationEvent::Menu {
                    path: menu_path.clone(),
                    position: event.position,
                });
            }),
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            window.focus(&this.focus, cx);
            this.selected = Some((path.clone(), staged));
            cx.emit(ChangeNavigationEvent::Select {
                path: path.clone(),
                staged,
            });
            cx.notify();
        }))
    }

    fn row(&self, index: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(row) = self.rows.get(index).cloned() else {
            return div().into_any_element();
        };
        match row {
            ChangeRow::Group {
                title,
                count,
                staged,
            } => self.group_row(title, count, staged, cx).into_any_element(),
            ChangeRow::Directory {
                path,
                label,
                depth,
                count,
                collapsed,
                staged,
            } => self
                .directory_row(path, label, depth, count, collapsed, staged, cx)
                .into_any_element(),
            ChangeRow::File {
                path,
                label,
                depth,
                status,
                staged,
                additions,
                deletions,
                binary,
            } => self
                .file_row(
                    path, label, depth, status, staged, additions, deletions, binary, cx,
                )
                .into_any_element(),
        }
    }

    fn empty_label(&self) -> &'static str {
        if self.rows.is_empty() {
            "工作区没有未提交的变更"
        } else {
            "没有匹配的变更"
        }
    }
}

fn row_key(row: &ChangeRow) -> Option<(String, bool)> {
    match row {
        ChangeRow::File { path, staged, .. } => Some((path.clone(), *staged)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Entity;

    fn row(label: &str, staged: bool) -> ChangeRow {
        ChangeRow::File {
            path: format!("/repo/{label}"),
            label: label.into(),
            depth: 0,
            status: ChangeStatus::Modified,
            staged,
            additions: Some(3),
            deletions: Some(1),
            binary: false,
        }
    }

    fn rows() -> Vec<ChangeRow> {
        vec![
            ChangeRow::Group {
                title: "已暂存".into(),
                count: 1,
                staged: Some(true),
            },
            row("src/staged.rs", true),
            ChangeRow::Group {
                title: "未暂存".into(),
                count: 1,
                staged: Some(false),
            },
            row("src/work.rs", false),
        ]
    }

    fn panel(
        cx: &mut gpui::TestAppContext,
    ) -> (Entity<ChangeNavigation>, &mut gpui::VisualTestContext) {
        cx.add_window_view(|_, cx| {
            let mut panel = ChangeNavigation::new(cx).stage_controls(true);
            panel.set_rows(rows(), cx);
            panel
        })
    }

    #[gpui::test]
    fn rows_keep_their_layer_and_offer_stage_checkboxes(cx: &mut gpui::TestAppContext) {
        let (_, window) = panel(cx);
        window.simulate_resize(gpui::size(px(280.), px(600.)));
        window.run_until_parked();
        // The same path can appear in both layers, so the layer is part of the row identity.
        assert!(
            window
                .debug_bounds("change-file:/repo/src/staged.rs:true")
                .is_some()
        );
        assert!(
            window
                .debug_bounds("change-file:/repo/src/work.rs:false")
                .is_some()
        );
        assert!(
            window
                .debug_bounds("stage-file:/repo/src/work.rs:false")
                .is_some()
        );
    }

    #[gpui::test]
    fn clicking_a_row_reports_the_layer_it_belongs_to(cx: &mut gpui::TestAppContext) {
        let (panel, window) = panel(cx);
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let received = events.clone();
        let _observer = window.new(|cx: &mut Context<()>| {
            cx.subscribe(&panel, move |_, _, event: &ChangeNavigationEvent, _| {
                received.borrow_mut().push(event.clone())
            })
            .detach();
        });
        window.run_until_parked();
        let row = window
            .debug_bounds("change-file:/repo/src/staged.rs:true")
            .unwrap();
        window.simulate_click(row.center(), Default::default());
        window.run_until_parked();
        assert_eq!(
            *events.borrow(),
            vec![ChangeNavigationEvent::Select {
                path: "/repo/src/staged.rs".into(),
                staged: true,
            }]
        );
    }

    #[gpui::test]
    fn the_group_checkbox_stages_the_whole_layer(cx: &mut gpui::TestAppContext) {
        let (panel, window) = panel(cx);
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let received = events.clone();
        let _observer = window.new(|cx: &mut Context<()>| {
            cx.subscribe(&panel, move |_, _, event: &ChangeNavigationEvent, _| {
                received.borrow_mut().push(event.clone())
            })
            .detach();
        });
        window.run_until_parked();
        let checkbox = window.debug_bounds("stage-group:false").unwrap();
        window.simulate_click(checkbox.center(), Default::default());
        window.run_until_parked();
        assert_eq!(
            *events.borrow(),
            vec![ChangeNavigationEvent::StageGroup { staged: true }]
        );
    }

    #[gpui::test]
    fn error_state_is_distinguishable_from_an_empty_list(cx: &mut gpui::TestAppContext) {
        let (panel, window) = cx.add_window_view(|_, cx| ChangeNavigation::new(cx));
        window.simulate_resize(gpui::size(px(280.), px(600.)));
        window.run_until_parked();
        assert!(window.debug_bounds("change-navigation-error").is_none());
        panel.update(window, |panel, cx| {
            panel.set_error("不在 Git 仓库中".into(), cx);
        });
        window.run_until_parked();
        assert!(window.debug_bounds("change-navigation-error").is_some());
        panel.update(window, |panel, cx| panel.set_rows(Vec::new(), cx));
        window.run_until_parked();
        assert!(window.debug_bounds("change-navigation-error").is_none());
    }

    #[gpui::test]
    fn unknown_statistics_never_render_as_binary(cx: &mut gpui::TestAppContext) {
        let (panel, window) = cx.add_window_view(|_, cx| {
            let mut panel = ChangeNavigation::new(cx);
            panel.set_rows(
                vec![ChangeRow::File {
                    path: "/repo/src/pending.rs".into(),
                    label: "src/pending.rs".into(),
                    depth: 0,
                    status: ChangeStatus::Modified,
                    staged: false,
                    // Counts were not loaded yet; that must not look like binary.
                    additions: None,
                    deletions: None,
                    binary: false,
                }],
                cx,
            );
            panel
        });
        window.simulate_resize(gpui::size(px(280.), px(400.)));
        window.run_until_parked();
        assert!(window.debug_bounds("change-file-binary").is_none());
        panel.update(window, |panel, cx| {
            panel.set_rows(
                vec![ChangeRow::File {
                    path: "/repo/logo.png".into(),
                    label: "logo.png".into(),
                    depth: 0,
                    status: ChangeStatus::Modified,
                    staged: false,
                    additions: None,
                    deletions: None,
                    binary: true,
                }],
                cx,
            )
        });
        window.run_until_parked();
        assert!(window.debug_bounds("change-file-binary").is_some());
    }
}

impl Focusable for ChangeNavigation {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ChangeNavigation {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let count = self.rows.len();
        let state = self.state.clone();
        let list = gpui::list(
            state.clone(),
            cx.processor(|this: &mut Self, index: usize, _, cx| this.row(index, cx)),
        )
        .flex_1()
        .min_h_0()
        .w_full();
        div()
            .id("change-navigation-panel")
            .debug_selector(|| "change-navigation".into())
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .text_size(px(12.))
            .text_color(rgb(p.text))
            .bg(rgb(p.surface))
            .track_focus(&self.focus)
            .tab_index(0)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                match event.keystroke.key.as_str() {
                    "up" => {
                        this.step(false, cx);
                        cx.stop_propagation();
                    }
                    "down" => {
                        this.step(true, cx);
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .pr(px(12.))
                    .flex()
                    .flex_col()
                    .children(self.error.as_ref().map(|error| {
                        div()
                            .debug_selector(|| "change-navigation-error".into())
                            .m_2()
                            .p_2()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(rgb(self.theme.color(0xffdeda)))
                            .bg(rgb(self.theme.color(0xffe9e7)))
                            .text_size(px(11.5))
                            .text_color(rgb(self.theme.color(0xd9342b)))
                            .child(error.clone())
                    }))
                    .when(self.loading && count == 0, |body| {
                        body.child(div().p_2().flex().flex_col().gap_3().children(
                            (0..SKELETON_ROWS).map(|index| {
                                Skeleton::new(("change-navigation-loading", index))
                                    .height(10.)
                                    .theme(self.theme)
                            }),
                        ))
                    })
                    .when(
                        count == 0 && !self.loading && self.error.is_none(),
                        |body| {
                            body.child(
                                div()
                                    .p_3()
                                    .text_size(px(11.5))
                                    .text_color(rgb(p.text_muted))
                                    .child(self.empty_label()),
                            )
                        },
                    )
                    .child(list)
                    .child(
                        Scrollbar::vertical_list(&state, &self.bar)
                            .theme(self.theme)
                            .render()
                            .absolute()
                            .right_0()
                            .top_0(),
                    ),
            )
    }
}
