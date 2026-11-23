//! Grouped change rows shared by the project explorer and the diff sidebar.
//! The host owns all Git I/O, tree building and staging; this only renders rows
//! and reports what the user did with them.
use super::{Checkbox, Icon, Scrollbar, ScrollbarState, Skeleton};
use crate::theme::ThemePreset;
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, ListAlignment, ListOffset, ListState,
    MouseButton, Render, SharedString, Window, div, prelude::*, px, rgb,
};
use std::collections::{HashMap, HashSet};

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
        /// Staging layer, if this directory belongs to a staging group.
        staged: Option<bool>,
    },
    File {
        path: String,
        label: String,
        depth: usize,
        status: Option<ChangeStatus>,
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
        staged: Option<bool>,
    },
    /// Stage or unstage these files; the host resolves them against its own sources.
    Stage {
        paths: Vec<String>,
        staged: bool,
    },
    StageGroup {
        layer: bool,
        staged: bool,
    },
    StageDirectory {
        path: String,
        layer: bool,
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
    blocked_files: HashSet<String>,
    stage_intents: HashMap<String, bool>,
    available_groups: [bool; 2],
    available_directories: [HashSet<String>; 2],
    background_visible: bool,
    empty_label: &'static str,
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
            blocked_files: HashSet::new(),
            stage_intents: HashMap::new(),
            available_groups: [true; 2],
            available_directories: Default::default(),
            background_visible: true,
            empty_label: "工作区没有未提交的变更",
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

    pub fn background_visible(mut self, visible: bool) -> Self {
        self.background_visible = visible;
        self
    }

    pub fn empty_label(mut self, label: &'static str) -> Self {
        self.empty_label = label;
        self
    }

    pub fn has_conflicts(&self) -> bool {
        self.rows.iter().any(|row| {
            matches!(
                row,
                ChangeRow::File {
                    status: Some(ChangeStatus::Conflict),
                    ..
                }
            )
        })
    }

    pub fn conflict_label(&self, path: &str) -> Option<&str> {
        self.rows.iter().find_map(|row| match row {
            ChangeRow::File {
                path: row_path,
                label,
                status: Some(ChangeStatus::Conflict),
                ..
            } if row_path == path => Some(label.as_str()),
            _ => None,
        })
    }

    /// Group availability also accounts for files in collapsed directories.
    pub fn set_stage_availability(
        &mut self,
        blocked_files: HashSet<String>,
        available_groups: [bool; 2],
        cx: &mut Context<Self>,
    ) {
        if self.blocked_files != blocked_files || self.available_groups != available_groups {
            self.blocked_files = blocked_files;
            self.available_groups = available_groups;
            cx.notify();
        }
    }

    pub fn set_stage_intents(&mut self, intents: HashMap<String, bool>, cx: &mut Context<Self>) {
        if self.stage_intents != intents {
            self.stage_intents = intents;
            cx.notify();
        }
    }

    pub fn set_stage_directory_availability(
        &mut self,
        available: [HashSet<String>; 2],
        cx: &mut Context<Self>,
    ) {
        if self.available_directories != available {
            self.available_directories = available;
            cx.notify();
        }
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
        if self.rows == rows && !self.loading && self.error.is_none() {
            return;
        }
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
        self.stage_intents.clear();
        self.selected = None;
        self.error = None;
        self.loading = true;
        self.state = ListState::new(0, ListAlignment::Top, px(ROW_HEIGHT));
        cx.notify();
    }

    pub fn scroll_to_row(&self, index: usize, cx: &mut Context<Self>) {
        self.state.scroll_to(ListOffset {
            item_ix: index,
            offset_in_item: px(0.),
        });
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
        if let Some((path, staged)) = row_key(row) {
            self.selected = Some((path.clone(), staged));
            self.state.scroll_to_reveal_item(index);
            cx.emit(ChangeNavigationEvent::Select { path, staged });
            cx.notify();
        }
    }

    pub fn group_element(title: &str, count: usize, conflict: bool) -> gpui::Div {
        div()
            .h(px(ROW_HEIGHT))
            .flex_shrink_0()
            .w_full()
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_size(px(12.))
            .when(conflict, |row| row.child(Icon::GitConflict.render(13.)))
            .child(format!("{title} · {count}"))
    }

    fn group_row(
        &self,
        title: String,
        count: usize,
        staged: Option<bool>,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let mut row = Self::group_element(&title, count, title == "待解决冲突");
        if self.stage_controls
            && let Some(staged) = staged
        {
            let checked = if self.stage_intents.is_empty() {
                staged
            } else {
                let mut files = self
                    .rows
                    .iter()
                    .filter_map(|row| match row {
                        ChangeRow::File {
                            path,
                            staged: layer,
                            status: Some(status),
                            ..
                        } if *layer == staged && *status != ChangeStatus::Conflict => {
                            Some(self.stage_intents.get(path).copied().unwrap_or(staged))
                        }
                        _ => None,
                    })
                    .peekable();
                if files.peek().is_some() {
                    files.all(|checked| checked)
                } else {
                    staged
                }
            };
            row = row.child(div().flex_1()).child(self.stage_checkbox(
                format!("stage-group:{staged}"),
                checked,
                self.available_groups[usize::from(staged)],
                ChangeNavigationEvent::StageGroup {
                    layer: staged,
                    staged: !checked,
                },
                cx,
            ));
        }
        row
    }

    pub fn directory_element(
        selector: String,
        label: String,
        depth: usize,
        count: usize,
        collapsed: bool,
        theme: ThemePreset,
    ) -> gpui::Stateful<gpui::Div> {
        let p = theme.palette();
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
            .child(Icon::disclosure(12., !collapsed))
            .child(Icon::Folder.render(14.))
            .child(div().flex_1().min_w_0().truncate().child(label))
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child(count.to_string()),
            )
    }

    fn directory_row(
        &self,
        path: String,
        label: String,
        depth: usize,
        count: usize,
        collapsed: bool,
        staged: Option<bool>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let selector = match staged {
            Some(layer) => format!("change-directory:{path}:{layer}"),
            None => format!("change-directory:{path}"),
        };
        let mut row = Self::directory_element(selector, label, depth, count, collapsed, self.theme);
        if self.stage_controls
            && let Some(layer) = staged
        {
            row = row.child(self.stage_checkbox(
                format!("stage-directory:{path}:{layer}"),
                layer,
                self.available_directories[usize::from(layer)].contains(&path),
                ChangeNavigationEvent::StageDirectory {
                    path: path.clone(),
                    layer,
                    staged: !layer,
                },
                cx,
            ));
        }
        row.on_click(cx.listener(move |_, _, _, cx| {
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
        status: Option<ChangeStatus>,
        staged: bool,
        additions: Option<usize>,
        deletions: Option<usize>,
        binary: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let color = status.map_or(p.text, |status| status.color(self.theme));
        let conflict = status == Some(ChangeStatus::Conflict);
        let icon = if conflict {
            Icon::GitConflict
        } else {
            Icon::for_file_name(&label)
        };
        let selector = format!("change-file:{path}:{staged}");
        let marker = format!("selected-conflict-file:{path}");
        let active = self
            .selected
            .as_ref()
            .is_some_and(|(selected, layer)| selected == &path && *layer == staged);
        let mut row = div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .relative()
            .h(px(ROW_HEIGHT))
            .flex_shrink_0()
            .w_full()
            .pr_3()
            .pl(px(26. + depth as f32 * 16.))
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .text_size(px(12.))
            .when(active, |row| row.bg(rgb(p.accent_tint)))
            .when(active && conflict, |row| {
                row.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(6.))
                        .bottom(px(6.))
                        .w(px(3.))
                        .rounded(px(2.))
                        .bg(rgb(p.accent))
                        .debug_selector(move || marker.clone()),
                )
            })
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
            .when_some(status, |row, status| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(10.))
                        .text_color(rgb(color))
                        .child(status.label()),
                )
            });
        if binary && !conflict && status != Some(ChangeStatus::Untracked) {
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
        if self.stage_controls && status.is_some() {
            let checked = self.stage_intents.get(&path).copied().unwrap_or(staged);
            row = row.child(self.stage_checkbox(
                format!("stage-file:{path}:{staged}"),
                checked,
                !self.blocked_files.contains(&path),
                ChangeNavigationEvent::Stage {
                    paths: vec![path.clone()],
                    staged: !checked,
                },
                cx,
            ));
        }
        let menu_path = path.clone();
        row.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |_, event: &gpui::MouseDownEvent, _, cx| {
                cx.stop_propagation();
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

    fn stage_checkbox(
        &self,
        selector: String,
        checked: bool,
        available: bool,
        event: ChangeNavigationEvent,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let mut checkbox = Checkbox::new("").checked(checked).theme(self.theme);
        if available {
            let weak = cx.entity().downgrade();
            checkbox = checkbox.on_change(move |_, _, cx| {
                cx.stop_propagation();
                let _ = weak.update(cx, |_, cx| cx.emit(event.clone()));
            });
        }
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .flex_shrink_0()
            .p_1()
            .when(!available, |row| row.opacity(0.4))
            .child(checkbox.render())
            .on_click(|_, _, cx| cx.stop_propagation())
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
            status: Some(ChangeStatus::Modified),
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
            vec![ChangeNavigationEvent::StageGroup {
                layer: false,
                staged: true
            }]
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
                    status: Some(ChangeStatus::Modified),
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
                    status: Some(ChangeStatus::Modified),
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
            .when(self.background_visible, |panel| panel.bg(rgb(p.surface)))
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
                                    .child(self.empty_label),
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
