use super::*;
use crate::edits::Change;
use gpui_glint::components::DiffCard;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
};

#[derive(Default)]
pub struct State {
    pub open: bool,
    pub selected: Option<PathBuf>,
    aggregates: RefCell<HashMap<String, (Vec<Uuid>, Rc<Vec<Change>>)>>,
    summaries: RefCell<HashMap<String, (Vec<Uuid>, Entity<Summary>)>>,
    diffs: RefCell<HashMap<String, (Uuid, Entity<DiffCard>)>>,
    workspace_mode: bool,
    workspace_root: Option<PathBuf>,
    workspace_files: Rc<Vec<Change>>,
    workspace_notes: Vec<String>,
    workspace_error: Option<String>,
    workspace_task: Option<Task<()>>,
    file_views: RefCell<HashMap<String, Entity<FileDiff>>>,
    sidebar_scroll: ScrollHandle,
    reveal_revision: u64,
    reveal_pending: Cell<bool>,
}

impl AgentWorkspace {
    fn cached_edits(&self, key: &str, records: Vec<&tools::ToolRecord>) -> Rc<Vec<Change>> {
        let ids = edits::ids(&records);
        let mut cache = self.edits_ui.aggregates.borrow_mut();
        let entry = cache
            .entry(key.into())
            .or_insert_with(|| (Vec::new(), Rc::new(Vec::new())));
        if entry.0 != ids {
            *entry = (ids, Rc::new(edits::aggregate_records(records)));
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
        let running = self
            .pending
            .as_ref()
            .is_some_and(|(id, _)| *id == self.history.active);
        if running {
            records.extend(self.tool_activity.iter());
        }
        let live = self.subagent_ui.live.lock().ok();
        if running {
            if let Some(live) = &live {
                for trace in live.iter() {
                    records.extend(trace.tools.iter());
                }
            }
        }
        self.cached_edits(&format!("session:{}", self.history.active), records)
    }

    pub fn open_edits(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
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
            (
                change.id,
                cx.new(|cx| {
                    DiffCard::from_lines(cx, edits::language(&change.path), change.lines())
                        .name(change.label.clone())
                        .theme(self.theme)
                }),
            )
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
        div().flex_shrink_0().child(
            Button::new(format!("变更 {}", files.len()))
                .icon(Icon::Pencil)
                .kind(ButtonKind::Plain)
                .theme(self.theme)
                .render()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.edits_ui.open = !this.edits_ui.open;
                    cx.notify();
                })),
        )
    }

    pub fn edits_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let workspace_mode = self.edits_ui.workspace_mode;
        let root_matches = self.edits_ui.workspace_root == self.history.active().project;
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
        let mut views = self.edits_ui.file_views.borrow_mut();
        for (index, file) in files.iter().enumerate() {
            let key = format!("{scope}:{}", file.path.display());
            let view = views
                .entry(key)
                .or_insert_with(|| cx.new(|_| FileDiff::new(file.clone(), self.theme)));
            view.update(cx, |view, cx| {
                view.set_file(file, self.theme, cx);
                if reveal_index == Some(index) {
                    view.reveal(self.edits_ui.reveal_revision, cx);
                }
            });
            list = list.child(view.clone());
        }
        drop(views);
        let source_weak = cx.entity().downgrade();
        let mut body = div()
            .w(px(480.))
            .max_w(gpui::relative(0.5))
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
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("文件变更 · {}", files.len())),
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
                    .px_2()
                    .py_2()
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
                    .child(div().flex_1())
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
        if workspace_mode {
            let status = if !root_matches {
                Some("项目已切换，请刷新工作区变更。".to_owned())
            } else if self.edits_ui.workspace_task.is_some() {
                Some("正在读取 Git 工作区…".to_owned())
            } else {
                self.edits_ui.workspace_error.clone()
            };
            if let Some(status) = status {
                body = body.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_color(rgb(p.text_muted))
                        .child(status),
                );
            }
            if root_matches {
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
        if files.is_empty() {
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
        body.child(list).child(
            div()
                .px_3()
                .py_2()
                .text_size(px(10.))
                .text_color(rgb(p.text_muted))
                .child(if workspace_mode {
                    "当前文件相对 HEAD 的变更，包含已有修改和未跟踪文件；点击刷新更新。"
                } else {
                    "会话内文件工具的累计净变更，含子代理。"
                }),
        )
    }

    fn refresh_workspace_edits(&mut self, cx: &mut Context<Self>) {
        let root = self.history.active().project.clone();
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
        let work = cx
            .background_executor()
            .spawn(async move { edits::workspace_changes(&root) });
        self.edits_ui.workspace_task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.edits_ui.workspace_task.take() {
                    task.detach();
                }
                match result {
                    Ok((files, notes)) => {
                        this.edits_ui.workspace_files = Rc::new(files);
                        this.edits_ui.workspace_notes = notes;
                    }
                    Err(error) => {
                        this.edits_ui.workspace_files = Rc::new(Vec::new());
                        this.edits_ui.workspace_notes.clear();
                        this.edits_ui.workspace_error = Some(error);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

/// Each file owns its disclosure state. Closed files never construct a diff;
/// opening another file leaves all existing disclosures and scroll positions intact.
struct FileDiff {
    file: Change,
    theme: ThemePreset,
    expanded: bool,
    card: Option<Entity<DiffCard>>,
    reveal_revision: u64,
}

impl FileDiff {
    fn new(file: Change, theme: ThemePreset) -> Self {
        Self {
            file,
            theme,
            expanded: false,
            card: None,
            reveal_revision: 0,
        }
    }

    fn set_file(&mut self, file: &Change, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.file.id != file.id {
            self.file = file.clone();
            if self.expanded {
                if let Some(card) = &self.card {
                    card.update(cx, |card, cx| card.set_lines(file.lines(), cx));
                }
            } else {
                self.card = None;
            }
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
}

impl Render for FileDiff {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let selector = format!("edit-file:{}", self.file.label);
        let mut row = div()
            .w_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .id("file-disclosure")
                    .debug_selector(move || selector.clone())
                    .h(px(48.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(p.surface_subtle)))
                    .child(
                        if self.expanded {
                            Icon::ChevronDown
                        } else {
                            Icon::ChevronRight
                        }
                        .render(13.)
                        .text_color(rgb(p.text_muted)),
                    )
                    .child(
                        Icon::for_language(edits::language(&self.file.path))
                            .render(15.)
                            .text_color(rgb(p.text_muted)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .child(self.file.label.clone()),
                    )
                    .child(counts(self.file.added, self.file.removed))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.expanded = !this.expanded;
                        cx.notify();
                    })),
            );
        if self.expanded {
            let card = self.card.get_or_insert_with(|| {
                cx.new(|cx| {
                    DiffCard::from_lines(cx, edits::language(&self.file.path), self.file.lines())
                        .header_visible(false)
                        .theme(self.theme)
                })
            });
            row = row.child(div().px_2().pb_2().child(card.clone()));
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
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(format!("已编辑 {} 个文件", self.files.len()))
                            .child(counts(total_add, total_remove)),
                    )
                    .child(
                        Button::new("查看变更")
                            .kind(ButtonKind::Plain)
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
            panel = panel.child(
                Button::new(if self.expanded {
                    "收起".into()
                } else {
                    format!("再显示 {} 个文件", self.files.len() - 3)
                })
                .kind(ButtonKind::Plain)
                .theme(self.theme)
                .render()
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
