use super::*;
use crate::edits::Change;
use gpui_glint::components::{DiffCard, DiffCardEvent};
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
    pub undo_task: Option<Task<()>>,
    undo_notice: Option<(Uuid, String)>,
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
    fn undo_session_edits(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.pending.is_some()
            || self.preparing.is_some()
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
                this.file_preview.paths.invalidate(&root, None);
                for view in this.markdown_views.borrow_mut().values_mut() {
                    if view.root == root {
                        view.links_source = None;
                        view.links_revision += 1;
                        view.links_task = None;
                    }
                }
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
        self.file_preview.open = false;
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
        div().flex_shrink_0().child(
            Button::new(format!("变更 {}", files.len()))
                .icon(Icon::Pencil)
                .kind(ButtonKind::Plain)
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
        let undo_workspace = workspace.downgrade();
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
        let can_undo = !workspace_mode
            && self.pending.is_none()
            && self.preparing.is_none()
            && self.edits_ui.undo_task.is_none()
            && self.queued_requests.is_empty();
        let mut views = self.edits_ui.file_views.borrow_mut();
        for (index, file) in files.iter().enumerate() {
            let key = format!("{scope}:{}", file.path.display());
            let view = views
                .entry(key)
                .or_insert_with(|| cx.new(|_| FileDiff::new(file.clone(), self.theme)));
            view.update(cx, |view, cx| {
                view.set_file(file, self.theme, cx);
                view.workspace = Some(undo_workspace.clone());
                if view.undo_visible != !workspace_mode || view.undo_enabled != can_undo {
                    view.undo_visible = !workspace_mode;
                    view.undo_enabled = can_undo;
                    cx.notify();
                }
                if reveal_index == Some(index) {
                    view.reveal(self.edits_ui.reveal_revision, cx);
                }
            });
            list = list.child(view.clone());
        }
        drop(views);
        let source_weak = cx.entity().downgrade();
        let mut body = div()
            .w(px(self.right_sidebar_width))
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
                            .child(format!("文件变更 · {}", files.len())),
                    )
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
        let work = cx.background_executor().spawn(async move {
            if crate::ssh_project::is_remote(&root) {
                provider::Job::start(async move { edits::remote_workspace_changes(&root).await })?
                    .result()
                    .await
            } else {
                edits::workspace_changes(&root)
            }
        });
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
    workspace: Option<gpui::WeakEntity<AgentWorkspace>>,
    undo_visible: bool,
    undo_enabled: bool,
    file: Change,
    theme: ThemePreset,
    expanded: bool,
    card: Option<Entity<DiffCard>>,
    reveal_revision: u64,
}

impl FileDiff {
    fn new(file: Change, theme: ThemePreset) -> Self {
        Self {
            workspace: None,
            undo_visible: false,
            undo_enabled: false,
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
        let path = self.file.path.clone();
        let undo_selector = format!("undo-file:{}", path.display());
        let workspace = self.workspace.clone();
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
