use super::{ContentType, Icon};
use crate::theme::ThemePreset;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, deferred, div, px, rgb, rgba,
};
use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap},
    rc::Rc,
};

/// A fixed-width column, or a flexible column that consumes the remaining width.
#[derive(Clone)]
pub struct TableColumn {
    pub label: SharedString,
    width: f32,
    flexible: bool,
    right: bool,
    sortable: bool,
    control: Option<gpui::Entity<super::CodeCard>>,
}
impl TableColumn {
    pub fn new(label: impl Into<SharedString>, width: f32) -> Self {
        Self {
            label: label.into(),
            width: width.max(36.),
            flexible: false,
            right: false,
            sortable: false,
            control: None,
        }
    }
    pub fn control(mut self, editor: gpui::Entity<super::CodeCard>) -> Self {
        self.control = Some(editor);
        self
    }
    pub fn flexible(mut self) -> Self {
        self.flexible = true;
        self
    }
    pub fn right(mut self) -> Self {
        self.right = true;
        self
    }
    pub fn sortable(mut self) -> Self {
        self.sortable = true;
        self
    }
    fn container(&self, compact: bool) -> gpui::Div {
        let mut cell = div()
            .min_w_0()
            .px(px(if compact { 8. } else { 12. }))
            .flex()
            .items_center();
        if self.flexible {
            cell = cell.flex_1().min_w(px(self.width));
        } else {
            cell = cell.w(px(self.width)).flex_shrink_0();
        }
        if self.right {
            cell = cell.justify_end();
        }
        cell
    }
}

#[derive(Clone)]
enum CellContent {
    Text { mono: bool },
    Resource { color: u32, icon: Option<Icon> },
    Type(ContentType),
    Status(u32),
    Editor(gpui::Entity<super::CodeCard>),
    Custom(Rc<dyn Fn() -> gpui::AnyElement>),
}

/// Display content and an optional numeric sort value (seconds, bytes, etc.).
#[derive(Clone)]
pub struct TableCell {
    text: SharedString,
    content: CellContent,
    number: Option<f64>,
    search_input: Option<gpui::Entity<super::TextEntry>>,
}
impl TableCell {
    pub fn text(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            content: CellContent::Text { mono: false },
            number: None,
            search_input: None,
        }
    }
    pub fn mono(text: impl Into<SharedString>) -> Self {
        Self {
            content: CellContent::Text { mono: true },
            ..Self::text(text)
        }
    }
    pub fn resource(title: impl Into<SharedString>, thumbnail_color: u32) -> Self {
        Self {
            content: CellContent::Resource {
                color: thumbnail_color,
                icon: None,
            },
            ..Self::text(title)
        }
    }
    pub fn thumbnail_icon(mut self, icon: Icon) -> Self {
        if let CellContent::Resource { icon: current, .. } = &mut self.content {
            *current = Some(icon);
        }
        self
    }
    pub fn content_type(content_type: ContentType) -> Self {
        Self {
            content: CellContent::Type(content_type),
            ..Self::text(content_type.label())
        }
    }
    pub fn status(label: impl Into<SharedString>, color: u32) -> Self {
        Self {
            content: CellContent::Status(color),
            ..Self::text(label)
        }
    }
    pub fn editor(editor: gpui::Entity<super::CodeCard>) -> Self {
        Self {
            content: CellContent::Editor(editor),
            ..Self::text("")
        }
    }
    /// Rebuild an interactive cell while preserving state in its captured entities.
    /// The factory must return a fresh element on every render.
    pub fn custom<E: IntoElement>(render: impl Fn() -> E + 'static) -> Self {
        Self {
            content: CellContent::Custom(Rc::new(move || render().into_any_element())),
            ..Self::text("")
        }
    }
    pub fn sort_number(mut self, value: f64) -> Self {
        self.number = Some(value);
        self
    }
    /// Searchable text for a custom cell whose contents are otherwise opaque to the table.
    pub fn search_text(mut self, text: impl Into<SharedString>) -> Self {
        self.text = text.into();
        self
    }
    /// Include an editable cell in filtering and highlight its matches without replacing the input.
    /// Rebuild the row when its value changes to refresh the searchable text.
    pub fn search_input(mut self, input: gpui::Entity<super::TextEntry>, cx: &App) -> Self {
        self.text = input.read(cx).value().to_owned().into();
        self.search_input = Some(input);
        self
    }
    fn compare(&self, other: &Self) -> Ordering {
        match (self.number, other.number) {
            (Some(a), Some(b)) => a.total_cmp(&b),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => self.text.cmp(&other.text),
        }
    }
    fn render(&self, theme: ThemePreset, query: &SharedString, cx: &mut App) -> gpui::Div {
        if let Some(input) = &self.search_input {
            input.update(cx, |input, cx| input.set_search_query(query.clone(), cx));
        }
        let highlighted = || {
            gpui::StyledText::new(self.text.clone()).with_highlights(
                super::search_highlight::match_ranges(&self.text, query)
                    .into_iter()
                    .map(|range| {
                        (
                            range,
                            gpui::HighlightStyle {
                                background_color: Some(super::search_highlight::color(
                                    theme.is_dark(),
                                )),
                                ..Default::default()
                            },
                        )
                    }),
            )
        };
        let palette = theme.palette();
        let base = div()
            .min_w_0()
            .text_size(px(13.))
            .text_color(rgb(palette.text));
        match &self.content {
            CellContent::Custom(render) => base.w_full().child(render()),
            CellContent::Editor(editor) => base.w_full().child(editor.clone()),
            CellContent::Text { mono } => {
                let base = if *mono {
                    base.font_family("Menlo").text_size(px(12.))
                } else {
                    base
                };
                base.child(div().truncate().child(highlighted()))
            }
            CellContent::Resource { color, icon } => {
                let mut thumbnail = div()
                    .size(px(34.))
                    .flex_shrink_0()
                    .rounded(px(5.))
                    .bg(rgb(*color))
                    .flex()
                    .items_center()
                    .justify_center();
                if let Some(icon) = icon {
                    thumbnail = thumbnail.child(icon.render(15.).text_color(rgba(0xffffff55)));
                }
                base.flex()
                    .items_center()
                    .gap(px(10.))
                    .child(thumbnail)
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(highlighted()),
                    )
            }
            CellContent::Type(kind) => base
                .flex()
                .items_center()
                .gap(px(5.))
                .px(px(10.))
                .py(px(3.))
                .rounded(px(999.))
                .bg(if theme.is_dark() {
                    rgb(theme.palette().surface_subtle)
                } else {
                    rgba(0xffffffaa)
                })
                .text_color(rgb(theme.color(0x48484a)))
                .child(
                    kind.icon()
                        .render(12.)
                        .text_color(rgb(theme.color(0x6e6e73))),
                )
                .child(highlighted()),
            CellContent::Status(color) => base
                .flex()
                .items_center()
                .gap(px(6.))
                .text_color(rgb(*color))
                .child(div().size(px(8.)).rounded_full().bg(rgb(*color)))
                .child(highlighted()),
        }
    }
}

/// Menu actions are identifiers, not built-in mutations. The owner handles emitted events.
#[derive(Clone)]
pub struct TableAction {
    pub id: SharedString,
    pub label: SharedString,
    icon: Icon,
    color: Option<u32>,
    danger: bool,
}
impl TableAction {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>, icon: Icon) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon,
            color: None,
            danger: false,
        }
    }
    pub fn color(mut self, color: u32) -> Self {
        self.color = Some(color);
        self
    }
    pub fn danger(mut self) -> Self {
        self.danger = true;
        self
    }
}

/// Stable IDs preserve selection across paging, sorting, and row updates.
#[derive(Clone)]
pub struct TableRow {
    pub id: SharedString,
    pub cells: Vec<TableCell>,
    focus: Option<FocusHandle>,
    primary: Option<TableAction>,
    actions: Vec<TableAction>,
}
impl TableRow {
    pub fn new(id: impl Into<SharedString>, cells: impl IntoIterator<Item = TableCell>) -> Self {
        Self {
            id: id.into(),
            cells: cells.into_iter().collect(),
            focus: None,
            primary: None,
            actions: Vec::new(),
        }
    }
    /// Control to focus when this row is appended through the add-row action.
    pub fn focus_handle(mut self, focus: FocusHandle) -> Self {
        self.focus = Some(focus);
        self
    }
    pub fn primary_action(mut self, action: TableAction) -> Self {
        self.primary = Some(action);
        self
    }
    pub fn action(mut self, action: TableAction) -> Self {
        self.actions.push(action);
        self
    }
}

#[derive(Clone, Debug)]
pub enum TableEvent {
    /// The owner creates and appends an empty row.
    AddRow,
    SelectionChanged(Vec<SharedString>),
    SortChanged {
        column: usize,
        descending: bool,
    },
    PageChanged(usize),
    Action {
        row_id: SharedString,
        action_id: SharedString,
    },
}

/// A local-data table with sorting, multi-selection, pagination, and upward action menus.
/// Create as an entity, then subscribe to [`TableEvent`] for application integration.
pub struct DataTable {
    max_height: Option<f32>,
    row_list: gpui::ListState,
    list_width: Rc<std::cell::Cell<gpui::Pixels>>,
    list_dirty: bool,
    reset_scroll: bool,
    list_row_ids: Vec<SharedString>,
    row_focus: HashMap<SharedString, FocusHandle>,
    order_cache: Option<Rc<Vec<usize>>>,
    page_cache: Option<Rc<Vec<usize>>>,
    pending_reveal: Option<usize>,
    adding_row: bool,
    pending_focus: Option<SharedString>,
    search_query: SharedString,
    scrollbar_drag: Rc<std::cell::Cell<Option<gpui::Pixels>>>,
    document: bool,
    compact: bool,
    add_row_label: Option<SharedString>,
    empty_row_label: Option<SharedString>,
    columns: Vec<TableColumn>,
    rows: Vec<TableRow>,
    selected: BTreeSet<SharedString>,
    sort: Option<(usize, bool)>,
    page: usize,
    page_size: usize,
    theme: ThemePreset,
    menu: Option<SharedString>,
    hovered: Option<SharedString>,
    anchor: Option<SharedString>,
    focus: FocusHandle,
}
impl DataTable {
    pub fn new(
        cx: &mut Context<Self>,
        columns: impl IntoIterator<Item = TableColumn>,
        rows: impl IntoIterator<Item = TableRow>,
    ) -> Self {
        Self {
            max_height: None,
            row_list: gpui::ListState::new(0, gpui::ListAlignment::Top, px(100.)),
            list_width: Rc::new(std::cell::Cell::new(px(-1.))),
            list_dirty: true,
            reset_scroll: false,
            list_row_ids: Vec::new(),
            row_focus: HashMap::new(),
            order_cache: None,
            page_cache: None,
            pending_reveal: None,
            adding_row: false,
            pending_focus: None,
            search_query: "".into(),
            scrollbar_drag: Rc::new(std::cell::Cell::new(None)),
            document: false,
            compact: false,
            add_row_label: None,
            empty_row_label: None,
            columns: columns.into_iter().collect(),
            rows: unique_rows(rows),
            selected: BTreeSet::new(),
            sort: None,
            page: 0,
            page_size: 20,
            theme: ThemePreset::default(),
            menu: None,
            hovered: None,
            anchor: None,
            focus: cx.focus_handle(),
        }
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    /// Limit the full table height in logical pixels. Rows scroll vertically while
    /// the header and pagination stay visible; shorter tables keep their natural height.
    pub fn max_height(mut self, height: f32) -> Self {
        self.max_height = Some(height.max(0.));
        self
    }
    /// Reduce header height and row padding for dense editable tables.
    /// Taller cell contents can still grow rows to avoid clipping.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }
    /// Show a trailing action row, even when the table is empty.
    /// Activating it emits `TableEvent::AddRow`; it is not counted as data.
    pub fn add_row(mut self, label: impl Into<SharedString>) -> Self {
        self.add_row_label = Some(label.into());
        self
    }
    /// Show a single, left-aligned empty-state row instead of the large placeholder.
    /// The row spans all columns, matches normal row height, and is not selectable data.
    pub fn empty_row(mut self, label: impl Into<SharedString>) -> Self {
        self.empty_row_label = Some(label.into());
        self
    }
    /// Change the empty-state row text; `None` restores the default placeholder.
    pub fn set_empty_row(&mut self, label: Option<SharedString>, cx: &mut Context<Self>) {
        self.empty_row_label = label;
        cx.notify();
    }
    /// Document tables omit selection, row actions and pagination.
    pub fn document(mut self) -> Self {
        self.document = true;
        self.page_size = usize::MAX;
        self
    }
    pub fn page_size(mut self, size: usize) -> Self {
        self.page_size = size.max(1);
        self.page = self.page.min(self.page_count() - 1);
        self
    }
    /// Pages are zero-based.
    pub fn page(mut self, page: usize) -> Self {
        self.page = page.min(self.page_count() - 1);
        self
    }
    pub fn selected(mut self, ids: impl IntoIterator<Item = SharedString>) -> Self {
        self.selected = ids
            .into_iter()
            .filter(|id| self.rows.iter().any(|r| &r.id == id))
            .collect();
        self.anchor = self.selected.iter().next().cloned();
        self
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        self.row_list.remeasure();
        self.list_width.set(px(-1.));
        cx.notify();
    }
    pub fn set_rows(&mut self, rows: impl IntoIterator<Item = TableRow>, cx: &mut Context<Self>) {
        let previous_count = self.rows.len();
        self.rows = unique_rows(rows);
        if std::mem::take(&mut self.adding_row) && self.rows.len() > previous_count {
            if let Some(row) = self.rows.last() {
                self.pending_focus = Some(row.id.clone());
                // Keep a newly appended row at the end while it is being edited.
                self.sort = None;
                self.search_query = "".into();
                self.page = (self.rows.len() - 1) / self.page_size;
            }
        }
        self.order_cache = None;
        self.page_cache = None;
        self.list_dirty = true;
        let ids = self
            .rows
            .iter()
            .map(|row| &row.id)
            .collect::<std::collections::HashSet<_>>();
        self.row_focus.retain(|id, _| ids.contains(id));
        let before = self.selected.clone();
        self.selected.retain(|id| ids.contains(id));
        self.menu = None;
        self.hovered = None;
        if self.anchor.as_ref().is_some_and(|id| !ids.contains(id)) {
            self.anchor = self.selected.iter().next().cloned();
        }
        self.order();
        self.page = self.page.min(self.page_count() - 1);
        if before != self.selected {
            cx.emit(TableEvent::SelectionChanged(
                self.selected.iter().cloned().collect(),
            ));
        }
        cx.notify();
    }
    pub fn selected_ids(&self) -> impl Iterator<Item = &SharedString> {
        self.selected.iter()
    }
    /// Case-insensitive substring matching across all searchable cells, before sorting/paging.
    pub fn set_search_query(&mut self, query: impl Into<SharedString>, cx: &mut Context<Self>) {
        let query = query.into();
        let query: SharedString = query.trim().to_owned().into();
        if self.search_query == query {
            return;
        }
        self.search_query = query;
        self.order_cache = None;
        self.page_cache = None;
        self.list_dirty = true;
        self.reset_scroll = true;
        self.page = 0;
        self.order();
        cx.notify();
    }
    pub fn matching_row_count(&self) -> usize {
        self.order_cache
            .as_ref()
            .map_or(self.rows.len(), |order| order.len())
    }
    fn page_count(&self) -> usize {
        self.matching_row_count().div_ceil(self.page_size).max(1)
    }
    fn order(&mut self) -> Rc<Vec<usize>> {
        if let Some(order) = &self.order_cache {
            return order.clone();
        }
        let query = self.search_query.to_lowercase();
        let mut indices: Vec<_> = (0..self.rows.len())
            .filter(|&index| {
                query.is_empty()
                    || self.rows[index]
                        .cells
                        .iter()
                        .any(|cell| cell.text.to_lowercase().contains(&query))
            })
            .collect();
        if let Some((column, descending)) = self.sort {
            indices.sort_by(|&a, &b| {
                let comparison = match (
                    self.rows[a].cells.get(column),
                    self.rows[b].cells.get(column),
                ) {
                    (Some(a), Some(b)) => a.compare(b),
                    _ => Ordering::Equal,
                };
                if descending {
                    comparison.reverse()
                } else {
                    comparison
                }
            });
        }
        let indices = Rc::new(indices);
        self.order_cache = Some(indices.clone());
        indices
    }
    fn select(&mut self, id: SharedString, toggle: bool, range: bool, cx: &mut Context<Self>) {
        self.menu = None;
        if range && let Some(anchor) = self.anchor.clone() {
            let order = self.order();
            let a = order.iter().position(|&i| self.rows[i].id == anchor);
            let b = order.iter().position(|&i| self.rows[i].id == id);
            if let (Some(a), Some(b)) = (a, b) {
                if !toggle {
                    self.selected.clear();
                }
                for &i in &order[a.min(b)..=a.max(b)] {
                    self.selected.insert(self.rows[i].id.clone());
                }
            }
        } else {
            if !toggle {
                self.selected.clear();
            }
            if !self.selected.remove(&id) {
                self.selected.insert(id.clone());
            }
            self.anchor = Some(id);
        }
        cx.emit(TableEvent::SelectionChanged(
            self.selected.iter().cloned().collect(),
        ));
        cx.notify();
    }
    fn change_page(&mut self, page: usize, cx: &mut Context<Self>) {
        let page = page.min(self.page_count() - 1);
        if self.page == page {
            return;
        }
        self.page = page;
        self.page_cache = None;
        self.list_dirty = true;
        self.reset_scroll = true;
        self.menu = None;
        self.hovered = None;
        cx.emit(TableEvent::PageChanged(page));
        cx.notify();
    }
    fn sort_column(&mut self, column: usize, cx: &mut Context<Self>) {
        let descending = self.sort == Some((column, false));
        self.sort = Some((column, descending));
        self.page = 0;
        self.order_cache = None;
        self.page_cache = None;
        self.list_dirty = true;
        self.reset_scroll = true;
        self.menu = None;
        self.order();
        cx.emit(TableEvent::SortChanged { column, descending });
        cx.notify();
    }

    fn keyboard(&mut self, key: &str, cx: &mut Context<Self>) {
        match key {
            "escape" => {
                self.menu = None;
                cx.notify();
            }
            "left" => self.change_page(self.page.saturating_sub(1), cx),
            "right" => self.change_page(self.page + 1, cx),
            "up" | "down" => {
                let order = self.order();
                if order.is_empty() {
                    return;
                }
                let current = self
                    .anchor
                    .as_ref()
                    .and_then(|id| order.iter().position(|&i| &self.rows[i].id == id));
                let next = match (key, current) {
                    ("up", Some(index)) => index.saturating_sub(1),
                    (_, Some(index)) => (index + 1).min(order.len() - 1),
                    _ => self.page * self.page_size,
                };
                let id = self.rows[order[next]].id.clone();
                self.change_page(next / self.page_size, cx);
                self.select(id, false, false, cx);
                if self.max_height.is_some() {
                    self.pending_reveal = Some(next % self.page_size);
                }
            }
            "enter" => {
                if let Some(id) = &self.anchor
                    && let Some(action) = self
                        .rows
                        .iter()
                        .find(|row| &row.id == id)
                        .and_then(|row| row.primary.as_ref())
                {
                    self.action(id.clone(), action.id.clone(), cx);
                }
            }
            _ => {}
        }
    }

    fn action(&mut self, row_id: SharedString, action_id: SharedString, cx: &mut Context<Self>) {
        self.menu = None;
        cx.emit(TableEvent::Action { row_id, action_id });
        cx.notify();
    }
    fn menu_element(&self, row: &TableRow, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        let mut menu = div()
            .id("table-action-menu")
            .absolute()
            .bottom(px(38.))
            .right_0()
            .w(px(180.))
            .p(px(6.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .rounded(px(11.))
            .border_1()
            .border_color(rgb(self.theme.palette().border))
            .bg(rgb(self.theme.palette().surface))
            .shadow_lg()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.menu = None;
                cx.notify();
            }));
        for (index, action) in row.actions.iter().enumerate() {
            let row_id = row.id.clone();
            let action_id = action.id.clone();
            let danger = action.danger;
            if danger && index > 0 {
                menu = menu.child(
                    div()
                        .h(px(1.))
                        .my(px(3.))
                        .bg(rgb(self.theme.palette().border)),
                );
            }
            let color = action.color.unwrap_or(if danger {
                theme.color(0xff3b30)
            } else {
                self.theme.palette().text
            });
            menu = menu.child(
                div()
                    .id(("table-menu-action", index))
                    .h(px(30.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(5.))
                    .text_size(px(13.))
                    .text_color(rgb(color))
                    .cursor_pointer()
                    .hover(move |style| {
                        style
                            .bg(rgb(if danger {
                                theme.color(0xff3b30)
                            } else {
                                theme.color(0x007aff)
                            }))
                            .text_color(rgb(0xffffff))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        window.focus(&this.focus, cx);
                        this.action(row_id.clone(), action_id.clone(), cx);
                    }))
                    .child(action.icon.render(14.).text_color(rgb(color)))
                    .child(action.label.clone()),
            );
        }
        menu.child(
            div()
                .absolute()
                .bottom(px(-5.))
                .right(px(10.))
                .text_size(px(12.))
                .text_color(rgb(self.theme.palette().surface))
                .child("▼"),
        )
    }
    fn sync_list(&mut self, visible: &[usize], count: usize, cx: &mut Context<Self>) {
        if self.list_dirty {
            if self.scrollbar_drag.take().is_some() {
                self.row_list.scrollbar_drag_ended();
            }
            let old_top = self.row_list.logical_scroll_top();
            let anchor = self.list_row_ids.get(old_top.item_ix).cloned();
            let mut focus_handles = Vec::with_capacity(count);
            self.list_row_ids.clear();
            for &index in visible {
                let row = &self.rows[index];
                self.list_row_ids.push(row.id.clone());
                // Keep an edited row mounted when it leaves the viewport so IME,
                // selection, and keyboard focus survive manual scrolling.
                let editable = row.cells.iter().any(|cell| {
                    matches!(
                        cell.content,
                        CellContent::Editor(_) | CellContent::Custom(_)
                    )
                });
                focus_handles.push(editable.then(|| {
                    self.row_focus
                        .entry(row.id.clone())
                        .or_insert_with(|| cx.focus_handle())
                        .clone()
                }));
            }
            focus_handles.resize(count, None);
            self.row_list
                .splice_focusable(0..self.row_list.item_count(), focus_handles);
            self.row_list
                .clone()
                .with_uniform_item_height(px(if self.compact { 34. } else { 53. }));
            self.list_width.set(px(-1.));
            let top = if self.reset_scroll {
                gpui::ListOffset::default()
            } else {
                gpui::ListOffset {
                    item_ix: anchor
                        .and_then(|id| self.list_row_ids.iter().position(|row| *row == id))
                        .unwrap_or(old_top.item_ix)
                        .min(count.saturating_sub(1)),
                    offset_in_item: old_top.offset_in_item,
                }
            };
            self.row_list.scroll_to(top);
            self.list_dirty = false;
            self.reset_scroll = false;
        }
        if let Some(index) = self.pending_reveal.take() {
            self.row_list.scroll_to_reveal_item(index);
        }
    }

    fn render_empty(&self) -> gpui::AnyElement {
        let palette = self.theme.palette();
        let search_label: SharedString = "没有匹配的记录".into();
        let label = if self.search_query.is_empty() {
            self.empty_row_label.as_ref()
        } else {
            Some(&search_label)
        };
        let empty = if let Some(label) = label {
            div()
                .w_full()
                .h(px(if self.compact { 34. } else { 53. }))
                .flex_shrink_0()
                .flex()
                .items_center()
                .px(px(if self.compact { 8. } else { 12. }))
                .text_size(px(13.))
                .text_color(rgb(palette.text_muted))
                .child(div().min_w_0().truncate().child(label.clone()))
        } else {
            div()
                .h(px(120.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(rgb(palette.text_muted))
                .child("暂无数据")
        };
        empty
            .debug_selector(|| "table-empty-state".into())
            .into_any_element()
    }

    fn render_row(&self, index: usize, has_next: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        let row = &self.rows[index];
        let row_id = row.id.clone();
        let hover_id = row.id.clone();
        let selected = self.selected.contains(&row.id);
        let document = self.document;
        let mut element = div()
            .id(row.id.clone())
            .w_full()
            .group(row.id.clone())
            .when_some(self.row_focus.get(&row.id), |row, focus| {
                row.track_focus(focus)
            })
            .flex()
            .items_center()
            .min_h(px(if self.compact { 34. } else { 53. }))
            .flex_shrink_0()
            .bg(if selected {
                rgba(0x007aff21)
            } else {
                rgba(0xffffff00)
            })
            .cursor_pointer()
            .hover(move |style| {
                if selected {
                    style
                } else {
                    style.bg(rgba(0x7878800b))
                }
            })
            .on_hover(cx.listener(move |this, hovered, _, cx| {
                if this.document {
                    return;
                }
                let previous = this.hovered.clone();
                if *hovered {
                    this.hovered = Some(hover_id.clone());
                } else if this.hovered.as_ref() == Some(&hover_id) {
                    this.hovered = None;
                }
                if previous != this.hovered {
                    cx.notify();
                }
            }))
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    if document {
                        return;
                    }
                    window.focus(&this.focus, cx);
                    let modifiers = event.modifiers();
                    this.select(
                        row_id.clone(),
                        modifiers.platform || modifiers.control,
                        modifiers.shift,
                        cx,
                    );
                }),
            );
        if has_next {
            element = element.border_b_1().border_color(rgba(0x78788014));
        }
        for (cell_index, column) in self.columns.iter().enumerate() {
            let mut cell =
                column
                    .container(self.compact)
                    .py(px(if self.compact { 4. } else { 9. }));
            if let Some(content) = row.cells.get(cell_index) {
                cell = cell.child(content.render(self.theme, &self.search_query, cx));
            } else {
                cell = cell.child("—");
            }
            element = element.child(cell);
        }
        if !self.document {
            element = element.child(
                div()
                    .w(px(80.))
                    .flex_shrink_0()
                    .px(px(12.))
                    .child(self.row_actions(row, cx)),
            );
        }
        element.into_any_element()
    }

    fn render_add_row(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let label = self.add_row_label.as_ref().unwrap();
        let theme = self.theme;
        let palette = theme.palette();
        div()
            .id("table-add-row")
            .debug_selector(|| "table-add-row".into())
            .focusable()
            .w_full()
            .h(px(if self.compact { 34. } else { 44. }))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(if self.compact { 8. } else { 12. }))
            .border_t_1()
            .border_color(rgb(palette.border))
            .text_size(px(12.))
            .text_color(rgb(palette.accent))
            .cursor_pointer()
            .hover(move |style| style.bg(rgb(palette.accent_tint)))
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.adding_row = true;
                cx.emit(TableEvent::AddRow);
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                    cx.stop_propagation();
                    this.adding_row = true;
                    cx.emit(TableEvent::AddRow);
                }
            }))
            .child(Icon::Plus.render(12.))
            .child(label.clone())
            .into_any_element()
    }

    fn render_item(
        &self,
        position: usize,
        visible: &[usize],
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if visible.is_empty() && position == 0 {
            self.render_empty()
        } else {
            self.render_row(visible[position], position + 1 < visible.len(), cx)
        }
    }

    fn render_scrollbar(&self) -> impl IntoElement {
        let state = self.row_list.clone();
        let layout_state = state.clone();
        let drag = self.scrollbar_drag.clone();
        let dark = self.theme.is_dark();
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(px(12.))
            .debug_selector(|| "table-scrollbar".into())
            .child(
                gpui::canvas(
                    move |bounds, window, _| {
                        let maximum = layout_state.max_offset_for_scrollbar().y;
                        if maximum <= px(0.) || bounds.size.height <= px(0.) {
                            return None;
                        }
                        let track = bounds.inset(px(2.));
                        let height = (track.size.height
                            * (bounds.size.height / (bounds.size.height + maximum)))
                            .max(px(24.))
                            .min(track.size.height);
                        let travel = track.size.height - height;
                        let offset = -layout_state.scroll_px_offset_for_scrollbar().y;
                        let top = track.top() + travel * (offset / maximum).clamp(0., 1.);
                        let thumb = gpui::Bounds::new(
                            gpui::point(bounds.right() - px(8.), top),
                            gpui::size(px(5.), height),
                        );
                        let hitbox = window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal);
                        Some((hitbox, thumb, track, maximum, travel))
                    },
                    move |_, geometry, window, _| {
                        let Some((hitbox, thumb, track, maximum, travel)) = geometry else {
                            return;
                        };
                        window.paint_quad(gpui::quad(
                            thumb,
                            px(3.),
                            rgba(if dark { 0xffffff55 } else { 0x00000033 }),
                            px(0.),
                            rgba(0),
                            gpui::BorderStyle::default(),
                        ));
                        window.set_cursor_style(gpui::CursorStyle::Arrow, &hitbox);
                        let down_state = state.clone();
                        let down_drag = drag.clone();
                        window.on_mouse_event(
                            move |event: &gpui::MouseDownEvent, phase, window, cx| {
                                if phase != gpui::DispatchPhase::Bubble
                                    || event.button != MouseButton::Left
                                    || !hitbox.is_hovered(window)
                                {
                                    return;
                                }
                                cx.stop_propagation();
                                window.prevent_default();
                                down_state.scrollbar_drag_started();
                                let grab = if thumb.contains(&event.position) {
                                    event.position.y - thumb.top()
                                } else {
                                    thumb.size.height / 2.
                                };
                                down_drag.set(Some(grab));
                                Self::scrollbar_position(
                                    &down_state,
                                    event.position.y - grab - track.top(),
                                    travel,
                                    maximum,
                                );
                                window.refresh();
                            },
                        );
                        let move_state = state.clone();
                        let move_drag = drag.clone();
                        window.on_mouse_event(
                            move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                                if phase != gpui::DispatchPhase::Capture {
                                    return;
                                }
                                if let Some(grab) = move_drag.get() {
                                    if !event.dragging() {
                                        move_drag.set(None);
                                        move_state.scrollbar_drag_ended();
                                    } else {
                                        Self::scrollbar_position(
                                            &move_state,
                                            event.position.y - grab - track.top(),
                                            travel,
                                            maximum,
                                        );
                                        cx.stop_propagation();
                                    }
                                    window.refresh();
                                }
                            },
                        );
                        let up_state = state.clone();
                        let up_drag = drag.clone();
                        window.on_mouse_event(
                            move |event: &gpui::MouseUpEvent, phase, window, cx| {
                                if phase == gpui::DispatchPhase::Capture
                                    && event.button == MouseButton::Left
                                    && up_drag.take().is_some()
                                {
                                    up_state.scrollbar_drag_ended();
                                    cx.stop_propagation();
                                    window.refresh();
                                }
                            },
                        );
                    },
                )
                .size_full(),
            )
    }

    fn scrollbar_position(
        state: &gpui::ListState,
        position: gpui::Pixels,
        travel: gpui::Pixels,
        maximum: gpui::Pixels,
    ) {
        let ratio = if travel > px(0.) {
            (position / travel).clamp(0., 1.)
        } else {
            0.
        };
        if ratio >= 1. {
            state.scroll_to_end();
        } else {
            state.set_offset_from_scrollbar(gpui::point(px(0.), -maximum * ratio));
        }
    }

    fn row_actions(&self, row: &TableRow, cx: &mut Context<Self>) -> gpui::Div {
        let theme = self.theme;
        let visible = self.hovered.as_ref() == Some(&row.id)
            || self.selected.contains(&row.id)
            || self.menu.as_ref() == Some(&row.id);
        let mut actions = div()
            .relative()
            .w_full()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(4.));
        if let Some(action) = &row.primary {
            let row_id = row.id.clone();
            let action_id = action.id.clone();
            actions = actions.child(
                div()
                    .id((row.id.clone(), 0usize))
                    .size(px(24.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(if visible { 1. } else { 0. })
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgba(0x78788016)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        window.focus(&this.focus, cx);
                        this.action(row_id.clone(), action_id.clone(), cx);
                    }))
                    .child(
                        action
                            .icon
                            .render(12.)
                            .text_color(rgb(action.color.unwrap_or(theme.color(0x48484a)))),
                    ),
            );
        }
        if !row.actions.is_empty() {
            let row_id = row.id.clone();
            let mut trigger = div()
                .id((row.id.clone(), 1usize))
                .relative()
                .size(px(24.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(rgba(0x78788016)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|_, _, _, cx| cx.stop_propagation()),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    window.focus(&this.focus, cx);
                    this.menu = if this.menu.as_ref() == Some(&row_id) {
                        None
                    } else {
                        Some(row_id.clone())
                    };
                    cx.notify();
                }))
                .child(
                    Icon::More
                        .render(12.)
                        .opacity(if visible { 1. } else { 0. }),
                );
            if self.menu.as_ref() == Some(&row.id) {
                trigger = trigger.child(deferred(self.menu_element(row, cx)).with_priority(20));
            }
            actions = actions.child(trigger);
        }
        actions
    }
}
impl EventEmitter<TableEvent> for DataTable {}
impl Focusable for DataTable {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn unique_rows(rows: impl IntoIterator<Item = TableRow>) -> Vec<TableRow> {
    let mut ids = BTreeSet::new();
    rows.into_iter()
        .filter(|row| ids.insert(row.id.clone()))
        .collect()
}

/// Condensed page links with gaps; the current page and both endpoints stay reachable.
fn page_links(current: usize, count: usize) -> Vec<Option<usize>> {
    let mut links = Vec::new();
    let mut previous = None;
    for page in 0..count {
        if count <= 5 || page == 0 || page + 1 == count || page.abs_diff(current) <= 1 {
            if previous.is_some_and(|last| page > last + 1) {
                links.push(None);
            }
            links.push(Some(page));
            previous = Some(page);
        }
    }
    links
}

impl Render for DataTable {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let palette = self.theme.palette();
        let width = self.columns.iter().map(|column| column.width).sum::<f32>()
            + if self.document { 0. } else { 80. };
        let mut header = div()
            .debug_selector(|| "table-header".into())
            .h(px(if self.compact { 28. } else { 35. }))
            .flex()
            .flex_shrink_0()
            .bg(rgba(0x7878800b))
            .border_b_1()
            .border_color(rgb(palette.border))
            .text_size(px(11.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(theme.color(0x8e8e93)));
        for (index, column) in self.columns.iter().enumerate() {
            let mut cell = column
                .container(self.compact)
                .h_full()
                .debug_selector(move || format!("table-header-cell-{index}"));
            if let Some(control) = &column.control {
                cell = cell.child(control.clone());
            } else {
                cell = cell.child(column.label.clone());
            }
            if column.sortable {
                let mut sortable = cell
                    .id(("table-sort", index))
                    .focusable()
                    .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                        if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                            cx.stop_propagation();
                            this.sort_column(index, cx);
                        }
                    }))
                    .cursor_pointer()
                    .gap(px(4.))
                    .hover(move |style| style.text_color(rgb(theme.color(0x007aff))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sort_column(index, cx);
                    }));
                sortable = sortable.child(
                    if self.sort == Some((index, false)) {
                        Icon::ArrowUp
                    } else if self.sort == Some((index, true)) {
                        Icon::ChevronDown
                    } else {
                        Icon::Sort
                    }
                    .render(10.),
                );
                header = header.child(sortable);
            } else {
                cell = cell.cursor_default();
                header = header.child(cell);
            }
        }
        if !self.document {
            header = header.child(
                div()
                    .w(px(80.))
                    .flex_shrink_0()
                    .px(px(12.))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_end()
                    .child("操作"),
            );
        }
        let visible = if let Some(rows) = &self.page_cache {
            rows.clone()
        } else {
            let rows = Rc::new(
                self.order()
                    .iter()
                    .copied()
                    .skip(self.page.saturating_mul(self.page_size))
                    .take(self.page_size)
                    .collect::<Vec<_>>(),
            );
            self.page_cache = Some(rows.clone());
            rows
        };
        let item_count = visible.len().max(1);
        let mut body = div()
            .id("table-body-scroll")
            .debug_selector(|| "table-body-scroll".into())
            .w_full()
            .min_h_0()
            .flex()
            .flex_col();
        if self.max_height.is_some() {
            self.sync_list(&visible, item_count, cx);
            if self.pending_focus.is_some() {
                self.row_list.scroll_to_end();
            }
            let state = self.row_list.clone();
            let last_width = self.list_width.clone();
            let row_height = px(if self.compact { 34. } else { 53. });
            body = body
                .relative()
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(
                    gpui::list(
                        self.row_list.clone(),
                        cx.processor(move |this, position, _, cx| {
                            this.render_item(position, &visible, cx)
                        }),
                    )
                    .with_sizing_behavior(gpui::ListSizingBehavior::Infer)
                    .w_full()
                    .min_h_0(),
                )
                .child(
                    gpui::canvas(
                        move |_, _, _| {
                            let width = state.viewport_bounds().size.width;
                            if last_width.replace(width) != width {
                                // GPUI clears all size hints on the initial layout and width
                                // changes. Restore estimates after prepaint so a large wheel
                                // gesture can reach rows that have not been measured yet.
                                let measured_height = state
                                    .bounds_for_item(state.logical_scroll_top().item_ix)
                                    .map_or(row_height, |bounds| bounds.size.height);
                                state
                                    .clone()
                                    .with_uniform_item_height(measured_height.max(row_height));
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .inset_0(),
                )
                .child(self.render_scrollbar());
        } else {
            for position in 0..item_count {
                body = body.child(self.render_item(position, &visible, cx));
            }
        }
        if let Some(id) = self.pending_focus.take() {
            let focus = self
                .rows
                .iter()
                .find(|row| row.id == id)
                .and_then(|row| row.focus.clone())
                .or_else(|| self.row_focus.get(&id).cloned());
            if let Some(focus) = focus {
                window.focus(&focus, cx);
            }
        }
        let mut pager = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(7.))
            .border_1()
            .border_color(rgb(palette.border))
            .bg(rgba(0x7878800f));
        for link in page_links(self.page, self.page_count()) {
            if let Some(page) = link {
                let active = page == self.page;
                pager = pager.child(
                    div()
                        .id(("table-page", page))
                        .focusable()
                        .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                            if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                                cx.stop_propagation();
                                this.change_page(page, cx);
                            }
                        }))
                        .min_w(px(28.))
                        .h(px(24.))
                        .px(px(8.))
                        .rounded(px(6.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(if active {
                            rgb(palette.surface)
                        } else {
                            rgba(0xffffff00)
                        })
                        .text_color(rgb(palette.text))
                        .cursor_pointer()
                        .hover(move |style| {
                            style.bg(if theme.is_dark() {
                                rgb(theme.palette().surface_subtle)
                            } else {
                                rgba(0xffffffbb)
                            })
                        })
                        .on_click(cx.listener(move |this, _, _, cx| this.change_page(page, cx)))
                        .child((page + 1).to_string()),
                );
            } else {
                pager = pager.child(div().w(px(28.)).text_center().child("…"));
            }
        }
        let footer = div()
            .debug_selector(|| "table-footer".into())
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(12.))
            .px(px(14.))
            .py(px(10.))
            .border_t_1()
            .border_color(rgba(0x78788014))
            .text_size(px(12.))
            .text_color(rgb(theme.color(0x8e8e93)))
            .child(format!(
                "共 {} 项 · 已选 {} 项",
                self.rows.len(),
                self.selected.len()
            ))
            .child(div().flex_1())
            .child(pager);
        div()
            .id("glass-data-table")
            .debug_selector(|| "data-table".into())
            .w_full()
            .min_w_0()
            .when_some(self.max_height, |table, height| table.max_h(px(height)))
            .flex_shrink_0()
            .relative()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .tab_index(0)
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if !this.document {
                    this.keyboard(&event.keystroke.key, cx);
                }
            }))
            .rounded(px(16.))
            .border_1()
            .border_color(if self.document && self.theme.is_dark() {
                rgba(0xffffff14)
            } else {
                rgb(palette.surface)
            })
            .bg(if self.document && self.theme.is_dark() {
                rgba(0xffffff04)
            } else {
                rgb(palette.surface)
            })
            .shadow_sm()
            .child(
                div()
                    .id("table-horizontal-scroll")
                    .flex()
                    .flex_col()
                    .w_full()
                    .min_w_0()
                    .min_h_0()
                    .overflow_x_scroll()
                    .rounded_t(px(16.))
                    .child(
                        div()
                            .w_full()
                            .min_w(px(width.max(660.)))
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .child(header)
                            .child(body),
                    ),
            )
            .children(self.add_row_label.as_ref().map(|_| self.render_add_row(cx)))
            .children((!self.document).then_some(footer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext;

    struct TableView(gpui::Entity<DataTable>);
    impl Render for TableView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().flex().flex_col().child(self.0.clone())
        }
    }

    #[gpui::test]
    fn scrollbar_drags_to_both_ends_and_releases_outside_table(cx: &mut gpui::TestAppContext) {
        let table = cx.new(|cx| {
            DataTable::new(
                cx,
                [TableColumn::new("域名", 180.).flexible()],
                (0..10_000).map(|index| {
                    TableRow::new(format!("row-{index}"), [TableCell::text(index.to_string())])
                }),
            )
            .document()
            .compact()
            .max_height(200.)
            .add_row("添加")
        });
        let (_, window) = cx.add_window_view(|_, _| TableView(table.clone()));
        let rail = window.debug_bounds("table-scrollbar").unwrap();
        let add = window.debug_bounds("table-add-row").unwrap();
        let header = window.debug_bounds("table-header").unwrap();
        for (start, end, expected) in [
            (rail.top() + px(4.), rail.bottom() + px(80.), true),
            (rail.bottom() - px(4.), rail.top() - px(80.), false),
        ] {
            window.simulate_mouse_move(
                gpui::point(rail.center().x, start),
                None,
                gpui::Modifiers::default(),
            );
            window.simulate_event(gpui::MouseDownEvent {
                position: gpui::point(rail.center().x, start),
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            });
            table.read_with(window, |table, _| {
                assert!(table.row_list.is_scrollbar_dragging())
            });
            let target = gpui::point(rail.right() + px(100.), end);
            window.simulate_event(gpui::MouseMoveEvent {
                position: target,
                pressed_button: Some(MouseButton::Left),
                ..Default::default()
            });
            window.simulate_event(gpui::MouseUpEvent {
                position: target,
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            });
            table.read_with(window, |table, _| {
                assert!(!table.row_list.is_scrollbar_dragging());
                if expected {
                    assert!(table.row_list.logical_scroll_top().item_ix > 9_990);
                } else {
                    assert_eq!(table.row_list.logical_scroll_top().item_ix, 0);
                }
            });
            assert_eq!(window.debug_bounds("table-add-row").unwrap(), add);
            assert_eq!(window.debug_bounds("table-header").unwrap(), header);
        }
    }

    #[gpui::test]
    fn search_filters_before_sorting_and_paging_and_restores_all_rows(
        cx: &mut gpui::TestAppContext,
    ) {
        let table = cx.new(|cx| {
            DataTable::new(
                cx,
                [TableColumn::new("域名", 180.).sortable()],
                ["a.test", "other", "B.TEST", "中文.test", "last"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, text)| TableRow::new(index.to_string(), [TableCell::text(text)])),
            )
            .page_size(2)
            .max_height(200.)
        });
        let (_, window) = cx.add_window_view(|_, _| TableView(table.clone()));
        table.update(window, |table, cx| {
            table.set_search_query("  TeSt  ", cx);
            assert_eq!(table.order().as_ref(), &[0, 2, 3]);
            assert_eq!(table.page_count(), 2);
            table.change_page(1, cx);
        });
        table.read_with(window, |table, _| {
            assert_eq!(table.page_cache.as_deref().unwrap().as_slice(), &[3])
        });
        table.update(window, |table, cx| {
            table.sort_column(0, cx);
            assert_eq!(table.order().as_ref(), &[2, 0, 3]);
            table.set_search_query("中文", cx);
            assert_eq!(table.order().as_ref(), &[3]);
            table.set_search_query("missing", cx);
            assert_eq!(table.matching_row_count(), 0);
            assert_eq!(table.page_count(), 1);
            assert_eq!(table.page, 0);
        });
        assert!(window.debug_bounds("table-empty-state").is_some());
        table.update(window, |table, cx| {
            table.set_search_query("", cx);
            assert_eq!(table.matching_row_count(), 5);
            assert_eq!(table.rows.len(), 5);
        });
    }

    #[gpui::test]
    fn virtual_rows_bound_render_work_and_preserve_offscreen_editing(
        cx: &mut gpui::TestAppContext,
    ) {
        let calls = Rc::new(std::cell::Cell::new(0usize));
        let entry = cx.new(|cx| super::super::TextEntry::new(cx, "域名", "original"));
        let table = cx.new(|cx| {
            DataTable::new(
                cx,
                [TableColumn::new("域名", 180.).flexible()],
                (0..10_000).map(|index| {
                    let calls = calls.clone();
                    let entry = entry.clone();
                    TableRow::new(
                        format!("row-{index}"),
                        [TableCell::custom(move || {
                            calls.set(calls.get() + 1);
                            // Different heights must remain supported by the virtual list.
                            let cell = div().h(px(if index % 7 == 0 { 64. } else { 32. }));
                            if index == 0 {
                                cell.child(entry.clone())
                            } else {
                                cell.child(index.to_string())
                            }
                        })],
                    )
                }),
            )
            .document()
            .compact()
            .max_height(420.)
            .add_row("添加行")
        });
        let (_, window) = cx.add_window_view(|_, _| TableView(table.clone()));
        assert!(
            calls.get() < 100,
            "initial render built {} cells",
            calls.get()
        );
        let viewport = window.debug_bounds("table-body-scroll").unwrap();
        let header = window.debug_bounds("table-header").unwrap();
        window.update(|window, cx| window.focus(&entry.read(cx).focus_handle(cx), cx));
        calls.set(0);
        window.simulate_event(gpui::ScrollWheelEvent {
            position: viewport.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-1_000_000.))),
            ..Default::default()
        });
        assert!(calls.get() < 100, "scroll built {} cells", calls.get());
        assert_eq!(window.debug_bounds("table-header").unwrap(), header);
        let add = window.debug_bounds("table-add-row").unwrap();
        assert!(
            add.top() >= viewport.bottom()
                && add.bottom() <= window.debug_bounds("data-table").unwrap().bottom()
        );
        window.update(|window, cx| assert!(entry.read(cx).focus_handle(cx).is_focused(window)));
        window.simulate_input(" edited");
        assert_eq!(
            entry.read_with(window, |entry, _| entry.value().to_owned()),
            "original edited"
        );
        window.simulate_resize(gpui::size(px(800.), px(600.)));
        table.update(window, |table, cx| {
            assert!(table.row_list.logical_scroll_top().item_ix > 9_900);
            table.row_list.scroll_to(gpui::ListOffset::default());
            cx.notify();
        });
        window.run_until_parked();
        assert_eq!(
            entry.read_with(window, |entry, _| entry.value().to_owned()),
            "original edited"
        );
    }

    #[gpui::test]
    fn virtual_sort_paging_keyboard_and_row_removal_keep_correct_rows(
        cx: &mut gpui::TestAppContext,
    ) {
        let table = cx.new(|cx| {
            DataTable::new(
                cx,
                [TableColumn::new("数值", 180.).sortable()],
                (0..200).map(|index| {
                    TableRow::new(
                        format!("row-{index}"),
                        [TableCell::text(index.to_string()).sort_number(index as f64)],
                    )
                }),
            )
            .page_size(50)
            .max_height(200.)
        });
        let (_, window) = cx.add_window_view(|_, _| TableView(table.clone()));
        table.update(window, |table, cx| {
            table.change_page(1, cx);
            table.keyboard("down", cx);
        });
        table.update(window, |table, cx| {
            assert_eq!(table.anchor.as_deref(), Some("row-50"));
            assert_eq!(table.list_row_ids[0], "row-50");
            let old = table.order();
            assert!(Rc::ptr_eq(&old, &table.order()));
            table.sort_column(0, cx);
            table.sort_column(0, cx);
        });
        table.update(window, |table, cx| {
            assert_eq!(table.page, 0);
            assert_eq!(table.list_row_ids[0], "row-199");
            table.row_list.scroll_to(gpui::ListOffset {
                item_ix: 20,
                offset_in_item: px(0.),
            });
            cx.notify();
        });
        let (anchor, top) = table.read_with(window, |table, _| {
            let top = table.row_list.logical_scroll_top().item_ix;
            (table.list_row_ids[top].clone(), top)
        });
        assert!(top > 0);
        table.update(window, |table, cx| {
            let rows = table
                .rows
                .iter()
                .filter(|row| row.id.as_ref() != "row-199")
                .cloned()
                .collect::<Vec<_>>();
            table.set_rows(rows, cx);
        });
        table.read_with(window, |table, _| {
            assert_eq!(
                table.list_row_ids[table.row_list.logical_scroll_top().item_ix],
                anchor
            );
        });
    }

    #[gpui::test]
    fn max_height_scrolls_rows_with_fixed_header_and_footer(cx: &mut gpui::TestAppContext) {
        struct View(gpui::Entity<DataTable>);
        impl Render for View {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().flex().flex_col().child(self.0.clone())
            }
        }
        for document in [false, true] {
            for compact in [false, true] {
                let table = cx.new(|cx| {
                    let table = DataTable::new(
                        cx,
                        [TableColumn::new("名称", 180.).flexible()],
                        (0..100).map(|index| {
                            TableRow::new(
                                format!("row-{index}"),
                                [TableCell::text(index.to_string())],
                            )
                        }),
                    )
                    .page_size(100)
                    .add_row("添加行")
                    .max_height(200.);
                    let table = if document { table.document() } else { table };
                    if compact { table.compact() } else { table }
                });
                let (_, window) = cx.add_window_view(|_, _| View(table.clone()));
                let bounds = window.debug_bounds("data-table").unwrap();
                let header = window.debug_bounds("table-header").unwrap();
                let viewport = window.debug_bounds("table-body-scroll").unwrap();
                let footer = window.debug_bounds("table-footer");
                assert_eq!(bounds.size.height, px(200.));
                assert!(viewport.size.height > px(0.));
                assert!(viewport.bottom() <= bounds.bottom());
                assert_eq!(header.bottom(), viewport.top());
                assert_eq!(footer.is_none(), document);
                window.simulate_event(gpui::ScrollWheelEvent {
                    position: viewport.center(),
                    delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-10000.))),
                    ..Default::default()
                });
                table.update(window, |table, _| {
                    assert!(
                        table.row_list.scroll_px_offset_for_scrollbar().y < px(-1000.),
                        "offset={:?} viewport={:?} list={:?} max={:?}",
                        table.row_list.scroll_px_offset_for_scrollbar(),
                        viewport,
                        table.row_list.viewport_bounds(),
                        table.row_list.max_offset_for_scrollbar()
                    );
                });
                assert_eq!(window.debug_bounds("table-header").unwrap(), header);
                assert_eq!(window.debug_bounds("table-footer"), footer);
                let add = window.debug_bounds("table-add-row").unwrap();
                assert!(
                    add.top() >= viewport.bottom()
                        && add.bottom() <= window.debug_bounds("data-table").unwrap().bottom()
                );
                table.update(window, |table, cx| {
                    table.set_rows([TableRow::new("one", [TableCell::text("一行")])], cx);
                });
                assert!(window.debug_bounds("data-table").unwrap().size.height < px(200.));
                table.update(window, |table, _| {
                    assert_eq!(table.row_list.max_offset_for_scrollbar().y, px(0.));
                });
            }
        }
    }

    #[gpui::test]
    fn empty_state_row_matches_data_height_and_keeps_add_action(cx: &mut gpui::TestAppContext) {
        for compact in [false, true] {
            let (table, window) = cx.add_window_view(|_, cx| {
                let table = DataTable::new(cx, [TableColumn::new("名称", 180.).flexible()], [])
                    .empty_row("暂无 Cookie 字段覆盖。")
                    .add_row("添加 Cookie 字段覆盖");
                if compact { table.compact() } else { table }
            });
            let events = Rc::new(std::cell::RefCell::new(Vec::new()));
            let received = events.clone();
            let _subscription = window.update(|_, cx| {
                cx.subscribe(&table, move |_, event, _| {
                    received.borrow_mut().push(event.clone())
                })
            });
            let empty = window.debug_bounds("table-empty-state").unwrap();
            let add = window.debug_bounds("table-add-row").unwrap();
            assert_eq!(empty.size.height, px(if compact { 34. } else { 53. }));
            assert_eq!(empty.bottom(), add.top());
            assert_eq!(empty.size.width, add.size.width);
            window.simulate_click(empty.center(), gpui::Modifiers::default());
            assert!(
                events.borrow().is_empty(),
                "the empty state must not select a data row"
            );
            window.simulate_click(add.center(), gpui::Modifiers::default());
            assert!(matches!(events.borrow().as_slice(), [TableEvent::AddRow]));
            table.update(window, |table, cx| {
                assert!(table.rows.is_empty() && table.selected.is_empty());
                table.set_rows([TableRow::new("cookie", [TableCell::text("session")])], cx);
            });
            assert!(window.debug_bounds("table-empty-state").is_none());
            table.update(window, |table, cx| table.set_rows([], cx));
            assert!(window.debug_bounds("table-empty-state").is_some());
            table.update(window, |table, cx| table.set_empty_row(None, cx));
            assert_eq!(
                window
                    .debug_bounds("table-empty-state")
                    .unwrap()
                    .size
                    .height,
                px(120.)
            );
        }
    }

    #[test]
    fn numeric_values_sort_numerically() {
        assert_eq!(
            TableCell::mono("820M")
                .sort_number(820.)
                .compare(&TableCell::mono("1.8G").sort_number(1800.)),
            Ordering::Less
        );
        assert_eq!(
            TableCell::mono("—").compare(&TableCell::mono("1.8G").sort_number(1800.)),
            Ordering::Greater
        );
    }
    #[test]
    fn page_links_keep_endpoints_and_current_page() {
        assert_eq!(
            page_links(4, 10),
            vec![Some(0), None, Some(3), Some(4), Some(5), None, Some(9)]
        );
        assert_eq!(page_links(0, 1), vec![Some(0)]);
        assert_eq!(page_links(2, 3), vec![Some(0), Some(1), Some(2)]);
    }
    #[test]
    fn duplicate_ids_cannot_alias_selection() {
        let rows = unique_rows([
            TableRow::new("a", []),
            TableRow::new("a", []),
            TableRow::new("b", []),
        ]);
        assert_eq!(rows.len(), 2);
    }
}
