use super::{ContentType, Icon};
use crate::theme::ThemePreset;
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, deferred, div, px, rgb, rgba,
};
use std::{cmp::Ordering, collections::BTreeSet, rc::Rc};

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
}
impl TableCell {
    pub fn text(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            content: CellContent::Text { mono: false },
            number: None,
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
    fn compare(&self, other: &Self) -> Ordering {
        match (self.number, other.number) {
            (Some(a), Some(b)) => a.total_cmp(&b),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => self.text.cmp(&other.text),
        }
    }
    fn render(&self, theme: ThemePreset) -> gpui::Div {
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
                base.child(div().truncate().child(self.text.clone()))
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
                            .child(self.text.clone()),
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
                .child(self.text.clone()),
            CellContent::Status(color) => base
                .flex()
                .items_center()
                .gap(px(6.))
                .text_color(rgb(*color))
                .child(div().size(px(8.)).rounded_full().bg(rgb(*color)))
                .child(self.text.clone()),
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
    primary: Option<TableAction>,
    actions: Vec<TableAction>,
}
impl TableRow {
    pub fn new(id: impl Into<SharedString>, cells: impl IntoIterator<Item = TableCell>) -> Self {
        Self {
            id: id.into(),
            cells: cells.into_iter().collect(),
            primary: None,
            actions: Vec::new(),
        }
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
    document: bool,
    compact: bool,
    add_row_label: Option<SharedString>,
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
            document: false,
            compact: false,
            add_row_label: None,
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
        cx.notify();
    }
    pub fn set_rows(&mut self, rows: impl IntoIterator<Item = TableRow>, cx: &mut Context<Self>) {
        self.rows = unique_rows(rows);
        let before = self.selected.clone();
        self.selected
            .retain(|id| self.rows.iter().any(|row| &row.id == id));
        self.page = self.page.min(self.page_count() - 1);
        self.menu = None;
        self.hovered = None;
        if self
            .anchor
            .as_ref()
            .is_some_and(|id| !self.rows.iter().any(|row| &row.id == id))
        {
            self.anchor = self.selected.iter().next().cloned();
        }
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
    fn page_count(&self) -> usize {
        self.rows.len().div_ceil(self.page_size).max(1)
    }
    fn order(&self) -> Vec<usize> {
        let mut indices: Vec<_> = (0..self.rows.len()).collect();
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
        indices
    }
    fn select(&mut self, id: SharedString, toggle: bool, range: bool, cx: &mut Context<Self>) {
        self.menu = None;
        if range && let Some(anchor) = &self.anchor {
            let order = self.order();
            let a = order.iter().position(|&i| &self.rows[i].id == anchor);
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
        self.menu = None;
        self.hovered = None;
        cx.emit(TableEvent::PageChanged(page));
        cx.notify();
    }
    fn sort_column(&mut self, column: usize, cx: &mut Context<Self>) {
        let descending = self.sort == Some((column, false));
        self.sort = Some((column, descending));
        self.page = 0;
        self.menu = None;
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let palette = self.theme.palette();
        let width = self.columns.iter().map(|column| column.width).sum::<f32>()
            + if self.document { 0. } else { 80. };
        let mut header = div()
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
            let mut cell = column.container(self.compact).h_full();
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
        let mut body = div()
            .w_full()
            .min_w(px(width.max(660.)))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .child(header);
        let order = self.order();
        let visible: Vec<_> = order
            .into_iter()
            .skip(self.page * self.page_size)
            .take(self.page_size)
            .collect();
        if visible.is_empty() {
            body = body.child(
                div()
                    .h(px(120.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(13.))
                    .text_color(rgb(palette.text_muted))
                    .child("暂无数据"),
            );
        }
        let document = self.document;
        for (position, &index) in visible.iter().enumerate() {
            let row = &self.rows[index];
            let row_id = row.id.clone();
            let hover_id = row.id.clone();
            let selected = self.selected.contains(&row.id);
            let mut element = div()
                .id(row.id.clone())
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
                    if *hovered {
                        this.hovered = Some(hover_id.clone());
                    } else if this.hovered.as_ref() == Some(&hover_id) {
                        this.hovered = None;
                    }
                    cx.notify();
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
            if position + 1 < visible.len() {
                element = element.border_b_1().border_color(rgba(0x78788014));
            }
            for (cell_index, column) in self.columns.iter().enumerate() {
                let mut cell =
                    column
                        .container(self.compact)
                        .py(px(if self.compact { 4. } else { 9. }));
                if let Some(content) = row.cells.get(cell_index) {
                    cell = cell.child(content.render(self.theme));
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
            body = body.child(element);
        }
        if let Some(label) = &self.add_row_label {
            body = body.child(
                div()
                    .id("table-add-row")
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
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.stop_propagation();
                        cx.emit(TableEvent::AddRow);
                    }))
                    .on_key_down(cx.listener(|_, event: &gpui::KeyDownEvent, _, cx| {
                        if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                            cx.stop_propagation();
                            cx.emit(TableEvent::AddRow);
                        }
                    }))
                    .child(Icon::Plus.render(12.))
                    .child(label.clone()),
            );
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
            .flex()
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
            .w_full()
            .min_w_0()
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
                    .w_full()
                    .min_w_0()
                    .overflow_x_scroll()
                    .rounded_t(px(16.))
                    .child(body),
            )
            .children((!self.document).then_some(footer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
