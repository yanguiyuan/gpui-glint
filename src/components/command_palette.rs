use super::{Icon, TextEntry};
use crate::theme::ThemePreset;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, UniformListScrollHandle, Window, deferred, div, px, rgb, rgba,
};

/// Stable command identity; shortcuts are display hints, dispatched by the application.
#[derive(Clone, Debug)]
pub struct CommandItem {
    pub id: SharedString,
    pub label: SharedString,
    pub group: SharedString,
    pub keywords: SharedString,
    pub icon: Option<Icon>,
    pub shortcut: Option<SharedString>,
    pub disabled: bool,
}
impl CommandItem {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            group: "".into(),
            keywords: "".into(),
            icon: None,
            shortcut: None,
            disabled: false,
        }
    }
    pub fn group(mut self, value: impl Into<SharedString>) -> Self {
        self.group = value.into();
        self
    }
    pub fn keywords(mut self, value: impl Into<SharedString>) -> Self {
        self.keywords = value.into();
        self
    }
    pub fn icon(mut self, value: Icon) -> Self {
        self.icon = Some(value);
        self
    }
    pub fn shortcut(mut self, value: impl Into<SharedString>) -> Self {
        self.shortcut = Some(value.into());
        self
    }
    pub fn disabled(mut self, value: bool) -> Self {
        self.disabled = value;
        self
    }
}
#[derive(Clone, Debug)]
pub enum CommandPaletteEvent {
    Executed(SharedString),
    Dismissed,
}

/// Compact searchable command surface. Register `bind_text_entry_keys` once.
/// Use `open` and `overlay` in a relative, full-size host for modal presentation.
/// The host owns the global Cmd/Ctrl+K shortcut and application command execution.
pub struct CommandPalette {
    items: Vec<CommandItem>,
    search: Entity<TextEntry>,
    active: Option<usize>,
    theme: ThemePreset,
    scroll: UniformListScrollHandle,
    search_terms: Vec<(String, String)>,
    matches: std::cell::RefCell<Option<(String, Vec<usize>)>>,
    scroll_pending: bool,
    open: bool,
    origin: Option<FocusHandle>,
    _search_subscription: Subscription,
}
impl EventEmitter<CommandPaletteEvent> for CommandPalette {}
impl Focusable for CommandPalette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.focus_handle(cx)
    }
}
impl CommandPalette {
    pub fn new(items: impl IntoIterator<Item = CommandItem>, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextEntry::new(cx, "搜索命令、集合…", "").font_size(16.));
        let subscription = cx.observe(&search, |this, _, cx| {
            this.active = this
                .visible(cx)
                .into_iter()
                .find(|&i| !this.items[i].disabled);
            this.scroll_pending = true;
            cx.notify();
        });
        let items: Vec<_> = items.into_iter().collect();
        let active = items.iter().position(|item| !item.disabled);
        Self {
            search_terms: items
                .iter()
                .map(|i| (i.label.to_lowercase(), i.keywords.to_lowercase()))
                .collect(),
            matches: Default::default(),
            items,
            search,
            active,
            theme: ThemePreset::default(),
            scroll: UniformListScrollHandle::new(),
            scroll_pending: true,
            open: false,
            origin: None,
            _search_subscription: subscription,
        }
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        self.search
            .update(cx, |input, cx| input.set_theme(theme, cx));
        cx.notify();
    }
    pub fn set_items(
        &mut self,
        items: impl IntoIterator<Item = CommandItem>,
        cx: &mut Context<Self>,
    ) {
        self.items = items.into_iter().collect();
        self.search_terms = self
            .items
            .iter()
            .map(|i| (i.label.to_lowercase(), i.keywords.to_lowercase()))
            .collect();
        *self.matches.borrow_mut() = None;
        self.active = self
            .visible(cx)
            .into_iter()
            .find(|&i| !self.items[i].disabled);
        self.scroll_pending = true;
        cx.notify();
    }
    pub fn query<'a>(&'a self, cx: &'a App) -> &'a str {
        self.search.read(cx).value()
    }
    pub fn set_query(&mut self, query: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.search
            .update(cx, |search, cx| search.set_value(query, cx));
        self.active = self
            .visible(cx)
            .into_iter()
            .find(|&i| !self.items[i].disabled);
        self.scroll_pending = true;
        cx.notify();
    }
    pub fn is_open(&self) -> bool {
        self.open
    }
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            return;
        }
        self.origin = window.focused(cx);
        self.open = true;
        self.set_query("", cx);
        window.focus(&self.search.focus_handle(cx), cx);
        cx.notify();
    }
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        if let Some(origin) = self.origin.take() {
            window.focus(&origin, cx);
        }
        cx.emit(CommandPaletteEvent::Dismissed);
        cx.notify();
    }
    fn visible(&self, cx: &App) -> Vec<usize> {
        let query = self.query(cx).trim().to_lowercase();
        let mut cache = self.matches.borrow_mut();
        if let Some((key, indices)) = &*cache
            && key == &query
        {
            return indices.clone();
        }
        let indices: Vec<_> = self
            .search_terms
            .iter()
            .enumerate()
            .filter(|(_, (label, keywords))| label.contains(&query) || keywords.contains(&query))
            .map(|(index, _)| index)
            .collect();
        *cache = Some((query, indices.clone()));
        indices
    }
    fn execute(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(index).filter(|item| !item.disabled) else {
            return;
        };
        let id = item.id.clone();
        if self.open {
            self.open = false;
            if let Some(origin) = self.origin.take() {
                window.focus(&origin, cx);
            }
        }
        cx.emit(CommandPaletteEvent::Executed(id));
        cx.notify();
    }
    /// Render only while open, above host content. Click the scrim or press Esc to dismiss.
    pub fn overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        self.open.then(|| {
            deferred(
                div()
                    .absolute()
                    .inset_0()
                    .size_full()
                    .bg(rgba(0x0c121c48))
                    // Modal scrim: block hover and wheel chaining to the page
                    // behind while the palette is open.
                    .occlude()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_4()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.close(window, cx);
                        }),
                    )
                    .child(cx.entity()),
            )
            .with_priority(100)
            .into_any_element()
        })
    }
}
#[cfg(test)]
fn matching_items(items: &[CommandItem], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item.label.to_lowercase().contains(&query)
                || item.keywords.to_lowercase().contains(&query)
        })
        .map(|(i, _)| i)
        .collect()
}
fn next_item(enabled: &[usize], active: Option<usize>, down: bool) -> Option<usize> {
    if enabled.is_empty() {
        return None;
    }
    let position = active.and_then(|i| enabled.iter().position(|&v| v == i));
    Some(
        enabled[match position {
            Some(p) if down => (p + 1) % enabled.len(),
            Some(p) => (p + enabled.len() - 1) % enabled.len(),
            None if down => 0,
            None => enabled.len() - 1,
        }],
    )
}
#[derive(Clone)]
enum PaletteRow {
    Heading(SharedString),
    Command(usize),
}
impl CommandPalette {
    fn render_row(&mut self, row: &PaletteRow, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = self.theme;
        let p = theme.palette();
        let PaletteRow::Command(i) = row else {
            let PaletteRow::Heading(label) = row else {
                unreachable!()
            };
            return div()
                .h(px(36.))
                .px(px(10.))
                .flex()
                .items_center()
                .text_size(px(11.))
                .text_color(rgb(p.text_muted))
                .child(label.clone())
                .into_any_element();
        };
        let i = *i;
        let item = &self.items[i];
        let active = self.active == Some(i);
        let mut row = div()
            .id(("command", i))
            .debug_selector(move || format!("palette-command-{i}"))
            .h(px(36.))
            .flex_shrink_0()
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(7.))
            .text_size(px(13.))
            .text_color(rgb(if active {
                0xffffff
            } else if item.disabled {
                p.text_muted
            } else {
                p.text
            }))
            .when(active, |el| el.bg(rgb(p.accent)))
            .when(!item.disabled, |el| el.cursor_pointer())
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                if !this.items[i].disabled && this.active != Some(i) {
                    this.active = Some(i);
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.execute(i, window, cx)));
        if let Some(icon) = item.icon {
            row = row.child(
                div()
                    .flex_shrink_0()
                    .child(icon.render(16.).text_color(rgb(if active {
                        0xffffff
                    } else {
                        p.text_muted
                    }))),
            );
        }
        row = row.child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .child(item.label.clone()),
        );
        if let Some(shortcut) = &item.shortcut {
            row = row.child(
                div()
                    .flex_shrink_0()
                    .px(px(5.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(if active {
                        theme.color(0x69b0ff)
                    } else {
                        p.border
                    }))
                    .font_family("Menlo")
                    .text_size(px(11.))
                    .child(shortcut.clone()),
            );
        }
        row.into_any_element()
    }
}
impl Render for CommandPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.search
            .update(cx, |input, cx| input.set_theme(self.theme, cx));
        let theme = self.theme;
        let p = self.theme.palette();
        let visible = self.visible(cx);
        // Group by first appearance, preserving input order within each group.
        let mut groups: Vec<SharedString> = vec![];
        for &i in &visible {
            if !groups.contains(&self.items[i].group) {
                groups.push(self.items[i].group.clone());
            }
        }
        let ordered: Vec<usize> = groups
            .iter()
            .flat_map(|g| {
                visible
                    .iter()
                    .copied()
                    .filter(|&i| &self.items[i].group == g)
                    .collect::<Vec<_>>()
            })
            .collect();
        let mut rows = Vec::new();
        for group in groups {
            if !group.is_empty() {
                rows.push(PaletteRow::Heading(group.clone()));
            }
            rows.extend(
                ordered
                    .iter()
                    .filter(|&&i| self.items[i].group == group)
                    .map(|&i| PaletteRow::Command(i)),
            );
        }
        if self.scroll_pending {
            if let Some(index) = rows
                .iter()
                .position(|r| matches!(r,PaletteRow::Command(i) if Some(*i)==self.active))
            {
                self.scroll
                    .scroll_to_item(index, gpui::ScrollStrategy::Nearest);
            }
            self.scroll_pending = false;
        }
        let list = if rows.is_empty() {
            div()
                .p_5()
                .text_center()
                .text_size(px(13.))
                .text_color(rgb(p.text_muted))
                .child("没有匹配的命令或集合")
                .into_any_element()
        } else {
            let height = px((rows.len() as f32 * 36.).min(320.));
            let rows = std::rc::Rc::new(rows);
            div()
                .p(px(6.))
                .child(
                    gpui::uniform_list(
                        "command-results",
                        rows.len(),
                        cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                            range
                                .map(|index| this.render_row(&rows[index], cx))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .h(height)
                    .w_full()
                    .track_scroll(&self.scroll),
                )
                .into_any_element()
        };
        div()
            .id("glass-command-palette")
            .debug_selector(|| "glass-command-palette".into())
            .w(px(580.))
            .max_w_full()
            .min_w_0()
            .rounded(px(16.))
            .border_1()
            .border_color(if theme.is_dark() {
                rgba(theme.glass_line())
            } else {
                rgb(p.border)
            })
            .bg(if theme.is_dark() {
                rgba(theme.menu_material())
            } else {
                rgb(p.surface)
            })
            .when(theme.is_dark(), |palette| palette.backdrop_blur(px(48.)))
            .shadow_lg()
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    window.focus(&this.search.focus_handle(cx), cx);
                }),
            )
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if this.search.read(cx).is_composing() {
                    return;
                }
                let key = event.keystroke.key.as_str();
                match key {
                    "up" | "down" => {
                        let visible = this.visible(cx);
                        let mut groups: Vec<SharedString> = vec![];
                        for &i in &visible {
                            if !groups.contains(&this.items[i].group) {
                                groups.push(this.items[i].group.clone());
                            }
                        }
                        let enabled: Vec<_> = groups
                            .iter()
                            .flat_map(|g| {
                                visible
                                    .iter()
                                    .copied()
                                    .filter(|&i| {
                                        &this.items[i].group == g && !this.items[i].disabled
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .collect();
                        this.active = next_item(&enabled, this.active, key == "down");
                        this.scroll_pending = true;
                        cx.notify();
                    }
                    "enter" => {
                        if let Some(i) = this.active {
                            this.execute(i, window, cx);
                        }
                    }
                    "escape" => {
                        if this.open {
                            this.close(window, cx);
                        } else {
                            this.set_query("", cx);
                        }
                    }
                    "tab" if this.open => {
                        window.focus(&this.search.focus_handle(cx), cx);
                    }
                    _ => return,
                }
                cx.stop_propagation();
                window.prevent_default();
            }))
            .child(
                div()
                    .px(px(16.))
                    .h(px(50.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .text_color(rgb(p.text))
                    .child(Icon::Search.render(20.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(24.))
                            .child(self.search.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child("esc"),
                    ),
            )
            .child(list)
            .child(
                div()
                    .px(px(16.))
                    .py(px(8.))
                    .border_t_1()
                    .border_color(rgb(p.border))
                    // GPUI overflow masks are rectangular; round the footer itself
                    // to the panel's inner radius (16px minus the 1px border).
                    .rounded_b(px(15.))
                    .bg(rgb(p.panel))
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(if self.open {
                        "↑ ↓ 选择    ↵ 执行    esc 关闭"
                    } else {
                        "↑ ↓ 选择    ↵ 执行    esc 清空"
                    }),
            )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_matches_unicode_keywords_and_empty_query() {
        let items = [
            CommandItem::new("new", "新建集合").keywords("Create"),
            CommandItem::new("photo", "拍摄手记"),
        ];
        assert_eq!(matching_items(&items, " CREATE "), vec![0]);
        assert_eq!(matching_items(&items, "拍摄"), vec![1]);
        assert_eq!(matching_items(&items, ""), vec![0, 1]);
        assert!(matching_items(&items, "missing").is_empty());
    }
    #[test]
    fn navigation_wraps_and_handles_removed_selection() {
        let enabled = [1, 4, 8];
        assert_eq!(next_item(&enabled, Some(8), true), Some(1));
        assert_eq!(next_item(&enabled, Some(1), false), Some(8));
        assert_eq!(next_item(&enabled, Some(2), true), Some(1));
        assert_eq!(next_item(&[], None, true), None);
    }
}
