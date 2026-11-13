use super::*;
use crate::edits::Change;
use gpui_glint::components::{
    ChainScroll, CodeCard, CodeCardEvent, DiffCard, DiffCardEvent, DiffColors,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
};

type ListLink = Rc<RefCell<Option<(gpui::ListState, usize)>>>;
struct VirtualFiles {
    files: Rc<Vec<Change>>,
    rows: Option<Rc<Vec<crate::workspace_tree::Row>>>,
    scope: String,
    state: gpui::ListState,
    expanded: bool,
    measured_width: gpui::Pixels,
}
fn watch_row<V: 'static>(view: &Entity<V>, link: ListLink, cx: &mut Context<AgentWorkspace>) {
    cx.observe(view, move |_, _, cx| {
        if let Some((list, index)) = &*link.borrow() {
            if *index < list.item_count() { list.remeasure_items(*index..*index + 1); }
            cx.notify();
        }
    }).detach();
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
    workspace_mode: bool,
    workspace_root: Option<PathBuf>,
    workspace_files: Rc<Vec<Change>>,
    workspace_sources: Rc<HashMap<PathBuf, crate::workspace_diff::Source>>,
    virtual_files: RefCell<Option<VirtualFiles>>,
    tree_cache: RefCell<Option<(String, Rc<Vec<Change>>, Rc<Vec<crate::workspace_tree::Row>>)>>,
    collapsed_directories: HashMap<String, std::collections::HashSet<String>>,
    workspace_notes: Vec<String>,
    workspace_error: Option<String>,
    workspace_task: Option<Task<()>>,
    workspace_stats_task: Option<Task<()>>,
    workspace_revision: u64,
    conflicts: Vec<Entity<ConflictView>>,
    file_views: RefCell<HashMap<String, Entity<FileDiff>>>,
    sidebar_scroll: ScrollHandle,
    detail_scrolls: RefCell<HashMap<String, ScrollHandle>>,
    navigation_scroll: gpui::UniformListScrollHandle,
    reveal_revision: u64,
    reveal_pending: Cell<bool>,
}

impl AgentWorkspace {
    pub(super) fn open_workspace_diff(&mut self, cx: &mut Context<Self>) {
        if self.history.active().project.is_none() { return; }
        self.page = crate::Page::Chat;
        self.git_ui.open = false;
        if !self.terminal_ui.bottom { self.terminal_ui.open = false; }
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
        self.cached_edits(&format!("session:{}", self.history.active), records)
    }

    pub fn open_edits(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        self.file_preview.open = false;
        if !self.terminal_ui.bottom { self.terminal_ui.open = false; }
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

    fn file_row(&self, file: &Change, scope: &str, link: Option<(gpui::ListState, usize)>, tree_depth: Option<usize>, can_undo: bool, reveal: bool, cx: &mut Context<Self>) -> Entity<FileDiff> {
        let key = format!("{scope}:{}", file.path.display());
        let mut views = self.edits_ui.file_views.borrow_mut();
        let view = views.entry(key).or_insert_with(|| {
            let view = cx.new(|_| FileDiff::new(file.clone(), self.theme));
            watch_row(&view, view.read(cx).list_link.clone(), cx);
            view
        }).clone();
        drop(views);
        let workspace = cx.entity().downgrade();
        view.update(cx, |view, cx| {
            *view.list_link.borrow_mut() = link;
            view.set_file(file, self.theme, cx);
            if view.detail != self.edits_ui.expanded { view.detail = self.edits_ui.expanded; cx.notify(); }
            if view.tree_depth != tree_depth { view.tree_depth = tree_depth; cx.notify(); }
            view.source = if self.edits_ui.workspace_mode { self.edits_ui.workspace_sources.get(&file.path).cloned() } else { None };
            if view.side_by_side != self.edits_ui.expanded {
                view.side_by_side = self.edits_ui.expanded;
                if let Some(card) = &view.card { card.update(cx, |card, cx| card.set_side_by_side(view.side_by_side, cx)); }
                cx.notify();
            }
            view.workspace = Some(workspace);
            if view.undo_visible != !self.edits_ui.workspace_mode || view.undo_enabled != can_undo {
                view.undo_visible = !self.edits_ui.workspace_mode;
                view.undo_enabled = can_undo;
                cx.notify();
            }
            if reveal { view.reveal(self.edits_ui.reveal_revision, cx); }
        });
        view
    }

    fn virtual_workspace_files(&self, files: Rc<Vec<Change>>, scope: String, cx: &mut Context<Self>) -> gpui::Div {
        let rows = if self.history.workspace_display == crate::workspace_tree::DisplayMode::Tree {
            let mut cache = self.edits_ui.tree_cache.borrow_mut();
            if cache.as_ref().is_none_or(|(cached_scope, cached_files, _)| cached_scope != &scope || !Rc::ptr_eq(cached_files, &files)) {
                let empty = Default::default();
                let collapsed = self.edits_ui.collapsed_directories.get(&scope).unwrap_or(&empty);
                *cache = Some((scope.clone(), files.clone(), Rc::new(crate::workspace_tree::rows(&files, collapsed))));
            }
            Some(cache.as_ref().unwrap().2.clone())
        } else { None };
        let row_count = rows.as_ref().map_or(files.len(), |rows| rows.len());
        let conflicts = self.edits_ui.conflicts.clone();
        let notes = self.edits_ui.workspace_notes.clone();
        let prefix = if conflicts.is_empty() { 0 } else { conflicts.len() + 1 };
        let count = prefix + row_count + notes.len();
        let mut cached = self.edits_ui.virtual_files.borrow_mut();
        let same_rows = cached.as_ref().is_some_and(|cache| cache.rows == rows);
        if let Some(cache) = cached.as_mut() {
            if cache.scope == scope && !same_rows && cache.files.len() == files.len()
                && cache.files.iter().zip(files.iter()).all(|(a,b)| a.id == b.id) {
                if let (Some(old), Some(new)) = (&cache.rows, &rows) {
                    if cache.state.item_count() == prefix + old.len() + notes.len() {
                        let common = old.iter().zip(new.iter()).take_while(|(a,b)| a == b).count();
                        let suffix = old[common..].iter().rev().zip(new[common..].iter().rev()).take_while(|(a,b)| a == b).count();
                        cache.state.splice(prefix + common..prefix + old.len() - suffix, new.len() - common - suffix);
                        cache.state.clone().with_uniform_item_height(px(49.));
                        cache.rows = rows.clone();
                        cache.files = files.clone();
                    }
                }
            }
            if cache.scope == scope && same_rows && !Rc::ptr_eq(&cache.files, &files) && cache.state.item_count() == count
                && cache.files.len() == files.len() && cache.files.iter().zip(files.iter()).all(|(a,b)| a.id == b.id) {
                // Background statistics must not reset scroll position or disclosures.
                cache.files = files.clone();
            }
        }
        if cached.as_ref().is_none_or(|cache| cache.rows != rows || cache.scope != scope || !Rc::ptr_eq(&cache.files, &files)) {
            let state = gpui::ListState::new(count, gpui::ListAlignment::Top, px(96.));
            state.reset_with_uniform_height(count, px(49.));
            *cached = Some(VirtualFiles { files: files.clone(), rows: rows.clone(), scope: scope.clone(), state, expanded: self.edits_ui.expanded, measured_width: px(0.) });
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
        let list = gpui::list(state, move |index, _, cx| {
            weak.update(cx, |this, cx| {
                if prefix > 0 && index == 0 {
                    return div().px_3().py_2().flex().items_center().gap_1()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(Icon::GitConflict.render(13.))
                        .child(format!("待解决冲突 · {}", conflicts.len()))
                        .into_any_element();
                }
                if index < prefix {
                    let view = &conflicts[index - 1];
                    view.update(cx, |view, cx| {
                        *view.list_link.borrow_mut() = Some((link_state.clone(), index));
                        if view.theme != this.theme {
                            view.theme = this.theme;
                            view.editor.update(cx, |editor, cx| editor.set_theme(this.theme, cx));
                            cx.notify();
                        }
                    });
                    return view.clone().into_any_element();
                }
                let row_index = index - prefix;
                let (file_index, depth) = match rows.as_ref().and_then(|rows| rows.get(row_index)) {
                    Some(crate::workspace_tree::Row::Directory { path, label, depth, count, collapsed }) => {
                        let path = path.clone();
                        let directory_scope = scope.clone();
                        let selector = format!("workspace-directory:{path}");
                        let p = this.theme.palette();
                        return div().id(SharedString::from(format!("workspace-directory:{scope}:{path}")))
                            .debug_selector(move || selector.clone()).h(px(34.)).pr_3().pl(px(12. + *depth as f32 * 16.))
                            .flex().items_center().gap_2().cursor_pointer().hover(|style| style.bg(rgb(p.surface_subtle)))
                            .child(if *collapsed { Icon::ChevronRight } else { Icon::ChevronDown }.render(13.).flex_shrink_0().text_color(rgb(p.text_muted)))
                            .child(Icon::Folder.render(15.).flex_shrink_0().text_color(rgb(p.text_muted)))
                            .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).child(label.clone()))
                            .child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child(count.to_string()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let collapsed = this.edits_ui.collapsed_directories.entry(directory_scope.clone()).or_default();
                                if !collapsed.remove(&path) { collapsed.insert(path.clone()); }
                                this.edits_ui.tree_cache.borrow_mut().take();
                                cx.notify();
                            })).into_any_element();
                    }
                    Some(crate::workspace_tree::Row::File { index, depth }) => (*index, Some(*depth)),
                    None => (row_index, None),
                };
                if row_index < row_count {
                    if let Some(file) = files.get(file_index) {
                        return this.file_row(file, &scope, Some((link_state.clone(), index)), depth, false, false, cx).into_any_element();
                    }
                }
                div().px_3().text_size(px(11.)).text_color(rgb(this.theme.palette().text_muted))
                    .child(notes.get(row_index.saturating_sub(row_count)).cloned().unwrap_or_default()).into_any_element()
            }).unwrap_or_else(|_| div().into_any_element())
        }).flex_1().min_h_0().w_full();
        div().flex_1().min_h_0().w_full().flex().flex_col()
            .on_children_prepainted(move |_, _, cx| {
                let _ = layout_weak.update(cx, |this, cx| {
                    let mut cache = this.edits_ui.virtual_files.borrow_mut();
                    let Some(cache) = cache.as_mut() else { return; };
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
        let virtualized = !self.edits_ui.expanded && workspace_mode && (files.len() > 128 || (!files.is_empty() && self.history.workspace_display == crate::workspace_tree::DisplayMode::Tree));
        if virtualized {
            list = div().id("edited-files-scroll").flex_1().min_h_0().flex().flex_col()
                .child(self.virtual_workspace_files(files.clone(), scope.clone(), cx));
        }
        if !self.edits_ui.expanded && !virtualized && workspace_mode && root_matches && !self.edits_ui.conflicts.is_empty() {
            for view in &self.edits_ui.conflicts {
                view.update(cx, |view, cx| {
                    view.list_link.borrow_mut().take();
                    if view.theme != self.theme {
                        view.theme = self.theme;
                        view.editor.update(cx, |editor, cx| editor.set_theme(self.theme, cx));
                        cx.notify();
                    }
                });
            }
            list = list.child(div().px_3().py_2().flex().items_center().gap_1()
                .font_weight(FontWeight::SEMIBOLD)
                .child(Icon::GitConflict.render(13.))
                .child(format!("待解决冲突 · {}", self.edits_ui.conflicts.len())))
                .children(self.edits_ui.conflicts.iter().cloned());
        }
        let can_undo = !workspace_mode
            && !self.any_run_busy()
            && self.edits_ui.undo_task.is_none()
            && self.queued_requests.is_empty();
        if !self.edits_ui.expanded && !virtualized {
            for (index, file) in files.iter().enumerate() {
                let view = self.file_row(file, &scope, None, None, can_undo, reveal_index == Some(index), cx);
                list = list.child(view);
            }
        }
        let source_weak = cx.entity().downgrade();
        let mut body = div()
            .when(self.edits_ui.expanded, |body| body.w_full())
            .when(!self.edits_ui.expanded, |body| body.w(px(self.right_sidebar_width)))
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
                            .child(if workspace_mode && self.edits_ui.workspace_task.is_some() && files.is_empty() { "文件变更 · 读取中".into() } else { format!("文件变更 · {}", files.len() + if workspace_mode && root_matches { self.edits_ui.conflicts.len() } else { 0 }) }),
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
                        IconButton::new(if self.edits_ui.expanded { Icon::Collapse } else { Icon::Expand },
                            if self.edits_ui.expanded { "恢复侧栏 Diff" } else { "展开 Diff，左右对照" })
                            .plain().theme(self.theme).render()
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
                Some("正在读取 Git 工作区文件清单…".to_owned())
            } else if self.edits_ui.workspace_stats_task.is_some() {
                Some("文件清单已就绪，正在后台统计改动行数…".to_owned())
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
        if files.is_empty() && (!workspace_mode || self.edits_ui.conflicts.is_empty() && self.edits_ui.workspace_task.is_none() && self.edits_ui.workspace_error.is_none()) {
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
        if self.edits_ui.expanded {
            list = self.expanded_edit_files(files, scope, can_undo, root_matches, cx);
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

    fn expanded_edit_files(&self, files: Rc<Vec<Change>>, scope: String, can_undo: bool, root_matches: bool, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        use crate::workspace_tree::Row;
        let p = self.theme.palette();
        let tree = self.history.workspace_display == crate::workspace_tree::DisplayMode::Tree;
        let rows = if tree {
            let mut cache = self.edits_ui.tree_cache.borrow_mut();
            if cache.as_ref().is_none_or(|(key, cached, _)| key != &scope || !Rc::ptr_eq(cached, &files)) {
                let empty = Default::default();
                let collapsed = self.edits_ui.collapsed_directories.get(&scope).unwrap_or(&empty);
                *cache = Some((scope.clone(), files.clone(), Rc::new(crate::workspace_tree::rows(&files, collapsed))));
            }
            cache.as_ref().unwrap().2.clone()
        } else {
            Rc::new((0..files.len()).map(|index| Row::File { index, depth: 0 }).collect())
        };
        let conflicts = if self.edits_ui.workspace_mode && root_matches { self.edits_ui.conflicts.clone() } else { Vec::new() };
        let selected = self.edits_ui.selected.as_ref()
            .filter(|path| files.iter().any(|file| &file.path == *path) || conflicts.iter().any(|view| &view.read(cx).file.path == *path))
            .cloned()
            .or_else(|| conflicts.first().map(|view| view.read(cx).file.path.clone()))
            .or_else(|| files.first().map(|file| file.path.clone()));
        let mut detail = div().id("expanded-diff-content").debug_selector(|| "expanded-diff-content".into())
            .flex_1().min_w_0().min_h_0().overflow_y_scroll();
        if let Some(path) = &selected {
            let scroll_key = format!("{scope}:{}", path.display());
            let scroll = self.edits_ui.detail_scrolls.borrow_mut().entry(scroll_key).or_default().clone();
            detail = detail.track_scroll(&scroll);
            if let Some(view) = conflicts.iter().find(|view| &view.read(cx).file.path == path) {
                view.update(cx, |view, cx| {
                    view.list_link.borrow_mut().take();
                    if !view.expanded { view.expanded = true; cx.notify(); }
                    if view.theme != self.theme {
                        view.theme = self.theme;
                        view.editor.update(cx, |editor, cx| editor.set_theme(self.theme, cx));
                    }
                });
                detail = detail.child(view.clone());
            } else if let Some(file) = files.iter().find(|file| &file.path == path) {
                detail = detail.child(self.file_row(file, &scope, None, None, can_undo, false, cx));
            }
        } else {
            detail = detail.child(div().p_4().text_color(rgb(p.text_muted)).child("选择左侧文件查看 Diff。"));
        }
        if self.edits_ui.workspace_mode {
            detail = detail.children(self.edits_ui.workspace_notes.iter().map(|note| div().px_3().py_1().text_size(px(11.)).text_color(rgb(p.text_muted)).child(note.clone())));
        }
        let prefix = conflicts.len();
        let count = prefix + rows.len();
        let navigation_files = files.clone();
        let navigation = gpui::uniform_list("expanded-diff-files", count, cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
            range.map(|index| {
                let (path, label, depth, icon, counts) = if index < prefix {
                    let file = &conflicts[index].read(cx).file;
                    (file.path.clone(), file.label.clone(), 0, Icon::GitConflict, "冲突".to_owned())
                } else {
                    match &rows[index - prefix] {
                        Row::Directory { path, label, depth, count, collapsed } => {
                            let path = path.clone();
                            let directory_scope = scope.clone();
                            let selector = format!("diff-navigation-directory:{path}");
                            return div().id(SharedString::from(format!("diff-directory:{scope}:{path}")))
                                .debug_selector(move || selector.clone()).h(px(34.)).pr_3().pl(px(10. + *depth as f32 * 16.))
                                .flex().items_center().gap_2().cursor_pointer().hover(|style| style.bg(rgb(p.surface_subtle)))
                                .child(if *collapsed { Icon::ChevronRight } else { Icon::ChevronDown }.render(12.))
                                .child(Icon::Folder.render(14.))
                                .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).child(label.clone()))
                                .child(div().text_size(px(10.)).text_color(rgb(p.text_muted)).child(count.to_string()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let collapsed = this.edits_ui.collapsed_directories.entry(directory_scope.clone()).or_default();
                                    if !collapsed.remove(&path) { collapsed.insert(path.clone()); }
                                    this.edits_ui.tree_cache.borrow_mut().take();
                                    cx.notify();
                                })).into_any_element();
                        }
                        Row::File { index, depth } => {
                            let file = &navigation_files[*index];
                            let label = if tree { file.label.rsplit('/').next().unwrap_or(&file.label).to_owned() } else { file.label.clone() };
                            (file.path.clone(), label, *depth, Icon::for_language(edits::language(&file.path)), format!("+{} −{}", file.added, file.removed))
                        }
                    }
                };
                let selector = format!("diff-navigation-file:{}", path.display());
                let active = selected.as_ref() == Some(&path);
                div().id(SharedString::from(selector.clone())).debug_selector(move || selector.clone())
                    .h(px(34.)).pr_3().pl(px(26. + depth as f32 * 16.))
                    .flex().items_center().gap_2().cursor_pointer()
                    .when(active, |row| row.bg(rgb(p.accent_tint)).text_color(rgb(p.accent)))
                    .hover(|style| style.bg(rgb(p.surface_subtle)))
                    .child(icon.render(14.).flex_shrink_0())
                    .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).child(label))
                    .child(div().text_size(px(10.)).text_color(rgb(p.text_muted)).child(counts))
                    .on_click(cx.listener(move |this, _, _, cx| { this.edits_ui.selected = Some(path.clone()); cx.notify(); }))
                    .into_any_element()
            }).collect::<Vec<_>>()
        })).size_full().track_scroll(&self.edits_ui.navigation_scroll);
        div().id("expanded-diff-layout").debug_selector(|| "expanded-diff-layout".into()).flex_1().min_h_0().flex()
            .child(div().debug_selector(|| "expanded-diff-navigation".into()).w(px(260.)).flex_shrink_0().h_full().border_r_1().border_color(rgb(p.border)).child(navigation))
            .child(detail)
    }

    fn refresh_workspace_edits(&mut self, cx: &mut Context<Self>) {
        self.edits_ui.workspace_revision += 1;
        let revision = self.edits_ui.workspace_revision;
        self.edits_ui.workspace_stats_task = None;
        let root = self.history.active().project.clone();
        if self.edits_ui.workspace_root != root {
            self.edits_ui.conflicts.clear();
            self.edits_ui.file_views.borrow_mut().retain(|key, _| !key.starts_with("workspace:"));
        }
        self.edits_ui.workspace_root = root.clone();
        self.edits_ui.workspace_error = None;
        self.edits_ui.workspace_files = Rc::new(Vec::new());
        self.edits_ui.workspace_sources = Rc::new(HashMap::new());
        self.edits_ui.virtual_files.borrow_mut().take();
        self.edits_ui.workspace_notes.clear();
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
                    .await.map(|(files, notes)| (files, notes, Vec::new(), HashMap::new(), None))
            } else {
                let inventory = crate::workspace_diff::inventory(&root)?;
                let (conflicts, notes) = if inventory.has_conflicts { crate::merge_conflicts::load(&root)? } else { (Vec::new(), Vec::new()) };
                Ok((inventory.files, notes, conflicts, inventory.sources, Some((inventory.root, inventory.head))))
            }
        });
        self.edits_ui.workspace_task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if this.history.active().project.as_ref() != Some(&requested_root)
                    || this.edits_ui.workspace_root.as_ref() != Some(&requested_root)
                    || this.edits_ui.workspace_revision != revision { return; }
                if let Some(task) = this.edits_ui.workspace_task.take() {
                    task.detach();
                }
                match result {
                    Ok((files, notes, conflicts, sources, statistics)) => {
                        let valid: std::collections::HashSet<_> = files.iter().map(|file| file.path.clone()).collect();
                        this.edits_ui.file_views.borrow_mut().retain(|key, view| !key.starts_with("workspace:") || valid.contains(&view.read(cx).file.path));
                        this.edits_ui.workspace_files = Rc::new(files);
                        this.edits_ui.workspace_sources = Rc::new(sources);
                        this.edits_ui.workspace_notes = notes;
                        let workspace = cx.entity().downgrade();
                        this.edits_ui.conflicts = conflicts.into_iter().map(|file| {
                            this.edits_ui.conflicts.iter().find(|view| view.read(cx).file == file).cloned()
                                .unwrap_or_else(|| {
                                    let view = cx.new(|cx| ConflictView::new(file, requested_root.clone(), workspace.clone(), this.theme, cx));
                                    watch_row(&view, view.read(cx).list_link.clone(), cx);
                                    view
                                })
                        }).collect();
                        if let Some((root, head)) = statistics {
                            this.start_workspace_statistics(root, head, revision, cx);
                        }
                    }
                    Err(error) => {
                        this.edits_ui.conflicts.clear();
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

    fn start_workspace_statistics(&mut self, root: PathBuf, head: Option<String>, revision: u64, cx: &mut Context<Self>) {
        if self.edits_ui.workspace_files.is_empty() { return; }
        let work = cx.background_executor().spawn(async move { crate::workspace_diff::statistics(&root, head.as_deref()) });
        self.edits_ui.workspace_stats_task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if this.edits_ui.workspace_revision != revision || this.history.active().project != this.edits_ui.workspace_root { return; }
                if let Some(task) = this.edits_ui.workspace_stats_task.take() { task.detach(); }
                match result {
                    Ok(counts) => crate::workspace_diff::apply_statistics(Rc::make_mut(&mut this.edits_ui.workspace_files).as_mut_slice(), Rc::make_mut(&mut this.edits_ui.workspace_sources), &counts),
                    Err(error) => this.edits_ui.workspace_error = Some(format!("行数统计失败，可展开文件按需查看：{error}")),
                }
                cx.notify();
            });
        }));
    }
}

/// Drafts stay in memory until the user explicitly saves and stages the file.
struct ConflictView {
    list_link: ListLink,
    file: crate::merge_conflicts::Conflict,
    project: PathBuf,
    workspace: gpui::WeakEntity<AgentWorkspace>,
    theme: ThemePreset,
    language: &'static str,
    expanded: bool,
    /// Conflict hunks in the pristine file, shown in the collapsed badge.
    count: usize,
    /// Editable merged result with line numbers and syntax highlighting.
    editor: Entity<CodeCard>,
    /// Scroll chain for the capped merge-result viewport inside the list.
    result_scroll: gpui::ScrollHandle,
    /// Read-only highlighted previews keyed by (hunk index, side).
    sides: RefCell<HashMap<(usize, usize), Entity<CodeCard>>>,
    notice: Option<String>,
    task: Option<Task<()>>,
}

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
    fn ask_agent(&mut self, cx: &mut Context<Self>) {
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
        let result = self
            .workspace
            .update(cx, |workspace, cx| -> Result<(), String> {
                if workspace.history.active().project.as_ref() != Some(&self.project) {
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

    fn new(
        file: crate::merge_conflicts::Conflict,
        project: PathBuf,
        workspace: gpui::WeakEntity<AgentWorkspace>,
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
        cx.subscribe(&editor, |_, _, event: &CodeCardEvent, cx| {
            if matches!(event, CodeCardEvent::Changed(_)) {
                cx.notify();
            }
        })
        .detach();
        Self {
            list_link: Default::default(),
            file,
            project,
            workspace,
            theme,
            language,
            expanded: false,
            count,
            editor,
            result_scroll: gpui::ScrollHandle::new(),
            sides: Default::default(),
            notice: None,
            task: None,
        }
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
                        let _ = this.workspace.update(cx, |workspace, cx| {
                            if workspace.history.active().project.as_ref() == Some(&this.project)
                                && workspace
                                    .edits_ui
                                    .conflicts
                                    .iter()
                                    .any(|view| view.entity_id() == weak.entity_id())
                            {
                                workspace.refresh_workspace_edits(cx);
                                workspace
                                    .file_preview
                                    .paths
                                    .invalidate(&Some(this.project.clone()), None);
                            }
                        });
                    }
                    Err(error) => this.notice = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// One conflict hunk: header with adopt actions, then highlighted side previews.
    fn hunk_card(
        &self,
        index: usize,
        hunk: &crate::merge_conflicts::Hunk,
        colors: &DiffColors,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let p = self.theme.palette();
        let mut actions = div().flex().flex_wrap().gap_1();
        for (choice, label) in ["采用当前侧", "采用传入侧", "保留双方"]
            .into_iter()
            .enumerate()
        {
            actions = actions.child(
                Button::new(label)
                    .kind(ButtonKind::Plain)
                    .theme(self.theme)
                    .when(busy, |button| button.disabled())
                    .render()
                    .debug_selector(move || format!("merge-choice:{index}:{choice}"))
                    .on_click(cx.listener(move |this, _, _, cx| this.choose(index, choice, cx))),
            );
        }
        div()
            .debug_selector(move || format!("merge-hunk:{index}"))
            .flex()
            .flex_col()
            .gap(px(6.))
            .p_2()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.panel))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("冲突 {}", index + 1)),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(format!(
                                "当前 {} 行 · 传入 {} 行",
                                hunk.ours.lines().count(),
                                hunk.theirs.lines().count()
                            )),
                    )
                    .child(div().flex_1())
                    .child(actions),
            )
            .child(self.side_section(
                index,
                0,
                "当前侧",
                "ours",
                &hunk.ours,
                colors.deleted_marker,
                cx,
            ))
            .child(self.side_section(
                index,
                1,
                "传入侧",
                "theirs",
                &hunk.theirs,
                colors.added_marker,
                cx,
            ))
    }

    /// One side of a hunk: colored label row plus a syntax-highlighted preview
    /// with a matching gutter stripe, mirroring diff row semantics.
    fn side_section(
        &self,
        index: usize,
        side: usize,
        label: &str,
        hint: &str,
        source: &str,
        marker: u32,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let p = self.theme.palette();
        div()
            .debug_selector(move || format!("merge-side:{index}:{side}"))
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().size(px(7.)).rounded_full().bg(rgb(marker)))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(format!("{label}（{hint}）")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgb(p.border))
                    .overflow_hidden()
                    .child(div().w(px(3.)).flex_shrink_0().bg(rgb(marker)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(self.side_preview(index, side, source, cx)),
                    ),
            )
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
        let busy = self.task.is_some();
        let badge = if self.count > 0 {
            format!("冲突 · {}", self.count)
        } else {
            "冲突".to_owned()
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
                    .debug_selector(|| "merge-conflict-disclosure".into())
                    .px_3()
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .child(
                        if self.expanded {
                            Icon::ChevronDown
                        } else {
                            Icon::ChevronRight
                        }
                        .render(13.),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .child(self.file.label.clone()),
                    )
                    .child(
                        div()
                            .px(px(6.))
                            .py(px(2.))
                            .rounded(px(5.))
                            .bg(rgb(colors.deleted_row))
                            .text_size(px(11.))
                            .text_color(rgb(colors.deleted_marker))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(Icon::GitConflict.render(11.))
                            .child(badge),
                    )
                    .child(
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
            let mut content = div().px_3().pb_3().flex().flex_col().gap_2()
                .child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).child("当前侧 / 传入侧对应 Git ours / theirs；rebase 时当前侧通常为变基目标。保存会暂存此文件，外部文件变化后刷新会重新读取。"));
            match crate::merge_conflicts::hunks(&text) {
                Ok(hunks) => {
                    self.sides
                        .borrow_mut()
                        .retain(|(hunk, _), _| *hunk < hunks.len());
                    if hunks.is_empty() {
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
                                .child("已无冲突标记，检查无误后保存并标记已解决。"),
                        );
                    } else {
                        for (index, hunk) in hunks.iter().enumerate() {
                            content = content.child(self.hunk_card(index, hunk, &colors, busy, cx));
                        }
                    }
                }
                Err(error) => content = content.child(self.notice_banner(error)),
            }
            let dirty = !busy && text != self.file.original;
            let mut result_header = div()
                .flex()
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
                        .child("可直接编辑，保存会暂存此文件"),
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
                .when(!busy, |content| {
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
                    Button::new(if busy {
                        "正在保存…"
                    } else {
                        "保存并标记已解决"
                    })
                    .theme(self.theme)
                    .when(busy, |button| button.disabled())
                    .render()
                    .debug_selector(|| "save-merge-conflict".into())
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
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
    detail: bool,
    tree_depth: Option<usize>,
    list_link: ListLink,
    source: Option<crate::workspace_diff::Source>,
    loaded: Option<Change>,
    load_task: Option<Task<()>>,
    load_error: Option<String>,
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
            tree_depth: None,
            list_link: Default::default(),
            source: None,
            loaded: None,
            load_task: None,
            load_error: None,
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

    fn load_diff(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else { return; };
        let id = self.file.id;
        let work = cx.background_executor().spawn(async move { crate::workspace_diff::load(&source) });
        self.load_task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if this.file.id != id { return; }
                if let Some(task) = this.load_task.take() { task.detach(); }
                match result {
                    Ok(change) => this.loaded = Some(change),
                    Err(error) => this.load_error = Some(error),
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
        if expanded && self.source.is_some() && self.loaded.is_none() && self.load_task.is_none() && self.load_error.is_none() {
            self.load_diff(cx);
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
                            DiffCard::from_lines(
                                cx,
                                edits::language(&self.file.path),
                                self.loaded.as_ref().unwrap_or(&self.file).lines(),
                            )
                            .side_by_side(self.side_by_side)
                            .header_visible(false)
                            .theme(self.theme)
                        })
                    })
                    .clone(),
            )
        } else {
            None
        };
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
                    .pl(px(12. + self.tree_depth.unwrap_or(0) as f32 * 16.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(p.surface_subtle)))
                    .when(!self.detail, |header| header.child(
                        if self.expanded {
                            Icon::ChevronDown
                        } else {
                            Icon::ChevronRight
                        }
                        .render(13.)
                        .text_color(rgb(p.text_muted)),
                    ))
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
                            .child(if self.tree_depth.is_some() {
                                self.file.label.rsplit('/').next().unwrap_or(&self.file.label).to_owned()
                            } else { self.file.label.clone() }),
                    )
                    .child(if let Some(loaded) = &self.loaded {
                        counts(loaded.added, loaded.removed)
                    } else if self.source.as_ref().is_some_and(|source| source.stats.is_none()) {
                        div().text_size(px(11.)).text_color(rgb(p.text_muted)).child("按需预览")
                    } else { counts(self.file.added, self.file.removed) })
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
                        if this.detail { return; }
                        this.expanded = !this.expanded;
                        if !this.expanded {
                            this.card = None;
                            this.loaded = None;
                            this.load_task = None;
                            this.load_error = None;
                        }
                        cx.notify();
                    })),
            );
        if expanded {
            if self.load_task.is_some() {
                return row.child(div().px_3().py_2().text_color(rgb(p.text_muted)).child("正在加载此文件的 Diff…"));
            }
            if let Some(error) = &self.load_error {
                return row.child(div().px_3().py_2().text_size(px(12.)).text_color(rgb(p.text_muted)).child(error.clone())
                    .child(Button::new("重试").kind(ButtonKind::Plain).theme(self.theme).render()
                        .on_click(cx.listener(|this, _, _, cx| { this.load_error = None; cx.notify(); }))));
            }
            let card = card.expect("diff card is ready when expanded without load errors");
            row = row.child(div().px_2().pb_2().child(card));
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

    #[gpui::test]
    fn workspace_tree_folders_keep_diff_state_and_settings_switch_back_to_list(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        cx.update(crate::initialize);
        let data = tempfile::tempdir().unwrap();
        let path = data.path().join("sessions.json");
        std::fs::write(data.path().join("providers.json"), r#"{"providers":[],"active":null,"active_model":""}"#).unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(path.clone(), cx);
            host.edits_ui.open = true;
            host.edits_ui.expanded = false;
            host.edits_ui.workspace_mode = true;
            host.history.workspace_display = crate::workspace_tree::DisplayMode::Tree;
            host.edits_ui.workspace_files = Rc::new(vec![change("root.rs", "new\n"), change("src/a.rs", "new\n"), change("src/nested/b.rs", "new\n"), change("tests/a.rs", "new\n")]);
            host
        });
        window.simulate_resize(size(px(1440.), px(1000.)));
        window.run_until_parked();
        let file = window.debug_bounds("edit-file:src/a.rs").unwrap();
        window.simulate_click(file.center(), Default::default());
        window.run_until_parked();
        let view = host.read_with(window, |host, cx| {
            let views = host.edits_ui.file_views.borrow();
            let view = views.values().find(|view| view.read(cx).file.label == "src/a.rs").unwrap().clone();
            assert!(view.read(cx).expanded && view.read(cx).card.is_some());
            assert_eq!(view.read(cx).tree_depth, Some(1));
            view
        });
        let directory = window.debug_bounds("workspace-directory:src").unwrap();
        window.simulate_click(directory.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, cx| {
            assert_eq!(host.edits_ui.virtual_files.borrow().as_ref().unwrap().state.item_count(), 4);
            assert!(view.read(cx).expanded);
        });
        let directory = window.debug_bounds("workspace-directory:src").unwrap();
        window.simulate_click(directory.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, cx| {
            assert_eq!(host.edits_ui.virtual_files.borrow().as_ref().unwrap().state.item_count(), 7);
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
            assert!(model::History::load(&path).unwrap().workspace_display == crate::workspace_tree::DisplayMode::List);
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
        std::fs::write(data.path().join("providers.json"), r#"{"providers":[],"active":null,"active_model":""}"#).unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.edits_ui.open = true;
            host.edits_ui.expanded = true;
            host.edits_ui.workspace_mode = true;
            host.history.workspace_display = crate::workspace_tree::DisplayMode::Tree;
            host.edits_ui.workspace_files = Rc::new((0..5000).map(|index| change(&format!("vendor/{index:04}.rs"), "new\n")).collect());
            host
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.run_until_parked();
        let list = host.read_with(window, |host, _| {
            assert_eq!(host.edits_ui.file_views.borrow().len(), 1);
            assert_eq!(host.edits_ui.tree_cache.borrow().as_ref().unwrap().2.len(), 5001);
            host.edits_ui.navigation_scroll.clone()
        });
        host.update(window, |_, cx| { list.scroll_to_item(4001, gpui::ScrollStrategy::Top); cx.notify(); });
        window.run_until_parked();
        let file = window.debug_bounds("diff-navigation-file:vendor/4000.rs").unwrap();
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
    fn thousands_of_workspace_files_only_render_visible_rows_and_load_disclosed_diff(cx: &mut gpui::TestAppContext) {
        let repo = crate::workspace_diff::tests::many_files(3089);
        let inventory = crate::workspace_diff::inventory(repo.path()).unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(data.path().join("providers.json"), r#"{"providers":[],"active":null,"active_model":""}"#).unwrap();
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
            assert!(views.values().all(|view| view.read(cx).loaded.is_none() && view.read(cx).load_task.is_none()));
            host.edits_ui.virtual_files.borrow().as_ref().unwrap().state.clone()
        });
        // Leave room below the target so the following row remains visible
        // after opening the Diff; off-screen rows are intentionally not rendered.
        host.update(window, |_, cx| { list.scroll_to(gpui::ListOffset { item_ix: 2499, offset_in_item: px(0.) }); cx.notify(); });
        window.run_until_parked();
        let row = window.debug_bounds("edit-file:vendor/2500.rs").unwrap();
        window.simulate_click(row.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, cx| {
            let views = host.edits_ui.file_views.borrow();
            assert!(views.len() < 96);
            let open = views.values().find(|view| view.read(cx).file.label == "vendor/2500.rs").unwrap().read(cx);
            assert!(open.expanded && open.card.is_some() && open.loaded.is_some(), "{:?}", open.load_error);
        });
        assert!(window.debug_bounds("edit-file:vendor/2501.rs").unwrap().top() > row.top() + px(70.));
        // Adding statistics changes neither list identity nor scroll position.
        host.update(window, |host, cx| {
            let files = Rc::make_mut(&mut host.edits_ui.workspace_files);
            files[2500].added = 1;
            cx.notify();
        });
        window.run_until_parked();
        host.read_with(window, |host, _| {
            let list_after = host.edits_ui.virtual_files.borrow().as_ref().unwrap().state.logical_scroll_top();
            assert!(list_after.item_ix > 2400);
        });
        host.update(window, |_, cx| { list.scroll_to_end(); cx.notify(); });
        window.run_until_parked();
        assert!(window.debug_bounds("edit-file:vendor/3088.rs").is_some());
        eprintln!("3089 files: first visible frame={initial:?}; only visible file views created; one Diff loaded on disclosure");
    }

    #[gpui::test]
    fn expanded_diff_navigation_switches_one_file_and_restores_compact_layout(cx: &mut gpui::TestAppContext) {
        let data = tempfile::tempdir().unwrap();
        std::fs::write(data.path().join("providers.json"), r#"{"providers":[],"active":null,"active_model":""}"#).unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.edits_ui.open = true;
            host.edits_ui.expanded = true;
            host.edits_ui.workspace_mode = true;
            host.history.workspace_display = crate::workspace_tree::DisplayMode::Tree;
            host.edits_ui.workspace_files = Rc::new(vec![change("src/first.rs", "first\n"), change("src/second.rs", "second\n")]);
            host
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.run_until_parked();
        let navigation = window.debug_bounds("expanded-diff-navigation").unwrap();
        let detail = window.debug_bounds("expanded-diff-content").unwrap();
        assert!(navigation.right() <= detail.left());
        assert!(detail.size.width > navigation.size.width);
        assert!(window.debug_bounds("edit-file:src/first.rs").is_some());
        assert!(window.debug_bounds("edit-file:src/second.rs").is_none());
        host.read_with(window, |host, _| assert_eq!(host.edits_ui.file_views.borrow().len(), 1, "only the selected file loads its Diff"));
        let second = window.debug_bounds("diff-navigation-file:src/second.rs").unwrap();
        window.simulate_click(second.center(), Default::default());
        window.run_until_parked();
        assert!(window.debug_bounds("edit-file:src/first.rs").is_none());
        assert!(window.debug_bounds("edit-file:src/second.rs").is_some());
        let directory = window.debug_bounds("diff-navigation-directory:src").unwrap();
        window.simulate_click(directory.center(), Default::default());
        window.run_until_parked();
        assert!(window.debug_bounds("diff-navigation-file:src/second.rs").is_none());
        assert!(window.debug_bounds("edit-file:src/second.rs").is_some(), "collapsing navigation must keep the selected Diff visible");
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
                assert!(!view.read(cx).expanded && !view.read(cx).detail, "detail mode must preserve compact disclosures");
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
    fn conflict_choices_edit_a_draft_before_explicit_save(cx: &mut gpui::TestAppContext) {
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
        host.update(window, |host, cx| host.composer.update(cx, |composer, cx| composer.set_value("保留输入框草稿", cx)));
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
                host.downgrade(),
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
        host.read_with(window, |host, cx| assert_eq!(host.composer.read(cx).value(cx), "保留输入框草稿"));
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
        let button = window.debug_bounds("save-merge-conflict").unwrap();
        window.simulate_click(button.center(), Default::default());
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
        let file = crate::merge_conflicts::load(repo.path())
            .unwrap()
            .0
            .remove(0);
        let (view, window) = window.add_window_view(|_, cx| {
            ConflictView::new(
                file,
                repo.path().into(),
                host.downgrade(),
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
        assert!(window.debug_bounds("merge-result-editor").is_some());
        view.read_with(window, |view, cx| {
            assert_eq!(view.count, 1);
            let sides = view.sides.borrow();
            assert_eq!(sides.get(&(0, 0)).unwrap().read(cx).content(), "ours\n");
            assert_eq!(sides.get(&(0, 1)).unwrap().read(cx).content(), "theirs\n");
        });
        let button = window.debug_bounds("merge-choice:0:0").unwrap();
        window.simulate_click(button.center(), Default::default());
        window.run_until_parked();
        view.read_with(window, |view, cx| {
            assert_eq!(view.editor.read(cx).content(), "ours\n");
            assert!(view.sides.borrow().is_empty());
        });
        assert!(window.debug_bounds("merge-resolved").is_some());
    }

    #[gpui::test]
    fn command_opens_project_workspace_diff_instead_of_session_changes(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("providers.json"), r#"{"providers":[],"active":null,"active_model":""}"#).unwrap();
        let (host, window) = cx.add_window_view(|_, cx| AgentWorkspace::with_path(dir.path().join("sessions.json"), cx));
        window.update(|window, cx| host.update(cx, |host, cx| {
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
        }));
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
