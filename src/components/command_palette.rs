use super::{Icon, TextEntry};
use crate::theme::{ThemePalette, ThemePreset};
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
    pub description: Option<SharedString>,
    pub group: SharedString,
    pub category: SharedString,
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
            description: None,
            group: "".into(),
            category: "".into(),
            keywords: "".into(),
            icon: None,
            shortcut: None,
            disabled: false,
        }
    }
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }
    pub fn group(mut self, value: impl Into<SharedString>) -> Self {
        self.group = value.into();
        self
    }
    pub fn category(mut self, category: impl Into<SharedString>) -> Self {
        self.category = category.into();
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
    SearchChanged {
        query: String,
        category: SharedString,
    },
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
    categories: Vec<SharedString>,
    category: SharedString,
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
        Self::with_placeholder(items, "搜索命令…", cx)
    }

    pub fn with_placeholder(
        items: impl IntoIterator<Item = CommandItem>,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| TextEntry::new(cx, placeholder, "").font_size(15.));
        let subscription = cx.observe(&search, |this, _, cx| {
            this.active = this
                .visible(cx)
                .into_iter()
                .find(|&i| !this.items[i].disabled);
            this.scroll_pending = true;
            cx.emit(CommandPaletteEvent::SearchChanged {
                query: this.query(cx).to_owned(),
                category: this.category.clone(),
            });
            cx.notify();
        });
        let items: Vec<_> = items.into_iter().collect();
        let active = items.iter().position(|item| !item.disabled);
        Self {
            search_terms: items
                .iter()
                .map(|i| {
                    (
                        i.label.to_lowercase(),
                        format!("{} {}", i.keywords, i.description.as_deref().unwrap_or(""))
                            .to_lowercase(),
                    )
                })
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
            categories: vec![],
            category: "全部".into(),
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
            .map(|i| {
                (
                    i.label.to_lowercase(),
                    format!("{} {}", i.keywords, i.description.as_deref().unwrap_or(""))
                        .to_lowercase(),
                )
            })
            .collect();
        *self.matches.borrow_mut() = None;
        self.active = self
            .visible(cx)
            .into_iter()
            .find(|&i| !self.items[i].disabled);
        self.scroll_pending = true;
        cx.notify();
    }
    pub fn set_categories(
        &mut self,
        categories: impl IntoIterator<Item = SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.categories = categories.into_iter().collect();
        self.category = "全部".into();
        *self.matches.borrow_mut() = None;
        cx.notify();
    }

    fn select_category(&mut self, category: SharedString, cx: &mut Context<Self>) {
        self.category = category;
        *self.matches.borrow_mut() = None;
        self.active = self
            .visible(cx)
            .into_iter()
            .find(|&i| !self.items[i].disabled);
        self.scroll_pending = true;
        cx.emit(CommandPaletteEvent::SearchChanged {
            query: self.query(cx).to_owned(),
            category: self.category.clone(),
        });
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
            .filter(|(index, (label, keywords))| {
                (self.categories.is_empty()
                    || self.category.as_ref() == "全部"
                    || self.items[*index].category == self.category)
                    && (label.contains(&query) || keywords.contains(&query))
            })
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

fn shortcut_keycap(label: impl Into<SharedString>, p: ThemePalette) -> gpui::Div {
    let label: SharedString = label.into();
    div()
        .h(px(18.))
        .flex()
        .items_center()
        .flex_shrink_0()
        .px(px(4.))
        .rounded(px(4.))
        .border_1()
        .border_color(rgb(p.border))
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(label)
}

impl CommandPalette {
    fn row_height(&self) -> f32 {
        if self.items.iter().any(|item| item.description.is_some()) {
            44.
        } else {
            32.
        }
    }
    fn render_row(
        &mut self,
        row: &PaletteRow,
        row_height: f32,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let p = self.theme.palette();
        let PaletteRow::Command(i) = row else {
            let PaletteRow::Heading(label) = row else {
                unreachable!()
            };
            return div()
                .h(px(row_height))
                .px(px(10.))
                .pb(px(4.))
                .flex()
                .items_end()
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
            .w_full()
            .h(px(row_height))
            .flex_shrink_0()
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(7.))
            .text_size(px(13.))
            .text_color(rgb(if item.disabled { p.text_muted } else { p.text }))
            .when(active, |el| el.bg(rgb(p.accent_tint)))
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
                    .child(icon.render(15.).text_color(rgb(if active {
                        p.accent
                    } else {
                        p.text_muted
                    }))),
            );
        }
        row = row.child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(div().truncate().child(item.label.clone()))
                .when_some(item.description.clone(), |column, description| {
                    column.child(
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(description),
                    )
                }),
        );
        if let Some(shortcut) = &item.shortcut {
            let label = if shortcut.contains('⌘') {
                shortcut
                    .replace(" + ", " ")
                    .split_whitespace()
                    .map(|key| match key {
                        "Ctrl" => "⌃",
                        "Alt" => "⌥",
                        "Shift" => "⇧",
                        "Enter" | "ENTER" => "↵",
                        key => key,
                    })
                    .collect::<String>()
                    .into()
            } else {
                shortcut.clone()
            };
            row = row.child(shortcut_keycap(label, p));
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
                .child("没有匹配的结果")
                .into_any_element()
        } else {
            let row_height = self.row_height();
            let height = px((rows.len() as f32 * row_height).min(320.));
            let rows = std::rc::Rc::new(rows);
            div()
                .p(px(6.))
                .child(
                    gpui::uniform_list(
                        "command-results",
                        rows.len(),
                        cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                            range
                                .map(|index| this.render_row(&rows[index], row_height, cx))
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
                    .h(px(44.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .when(self.categories.is_empty(), |search| {
                        search.border_b_1().border_color(rgb(p.border))
                    })
                    .text_color(rgb(p.text))
                    .child(Icon::Search.render(17.).text_color(rgb(p.text_muted)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(px(24.))
                            .child(self.search.clone()),
                    ),
            )
            .when(!self.categories.is_empty(), |root| {
                root.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .px(px(12.))
                        .pb(px(7.))
                        .border_b_1()
                        .border_color(rgb(p.border))
                        .text_size(px(12.))
                        .children(self.categories.iter().map(|category| {
                            let value = category.clone();
                            let selected = *category == self.category;
                            div()
                                .id(gpui::ElementId::Name(category.clone()))
                                .h(px(24.))
                                .flex()
                                .items_center()
                                .px(px(9.))
                                .rounded(px(5.))
                                .text_color(rgb(if selected { p.text } else { p.text_muted }))
                                .cursor_pointer()
                                .when(selected, |tab| tab.bg(rgb(p.accent_tint)))
                                .when(!selected, |tab| {
                                    tab.hover(|style| style.bg(rgb(p.surface_subtle)))
                                })
                                .child(category.clone())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select_category(value.clone(), cx)
                                }))
                        })),
                )
            })
            .child(list)
            .child(
                div()
                    .px(px(16.))
                    .py(px(6.))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(12.))
                    .border_t_1()
                    .border_color(rgb(p.border))
                    // GPUI overflow masks are rectangular; round the footer itself
                    // to the panel's inner radius (16px minus the 1px border).
                    .rounded_b(px(15.))
                    .bg(rgb(p.panel))
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .children(
                        [
                            ("↑↓", "选择"),
                            ("↵", "执行"),
                            ("Esc", if self.open { "关闭" } else { "清空" }),
                        ]
                        .into_iter()
                        .map(|(key, label)| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .child(shortcut_keycap(key, p))
                                .child(label)
                        }),
                    ),
            )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn category_filter_preserves_query_and_limits_navigation(cx: &mut gpui::TestAppContext) {
        let (palette, window) = cx.add_window_view(|_, cx| {
            CommandPalette::new(
                [
                    CommandItem::new("action", "main action").category("操作"),
                    CommandItem::new("file", "main.rs").category("文件"),
                ],
                cx,
            )
        });
        palette.update(window, |palette, cx| {
            palette.set_categories(["全部", "操作", "文件"].map(Into::into), cx);
            palette.set_query("main", cx);
            palette.select_category("文件".into(), cx);
            assert_eq!(palette.visible(cx), vec![1]);
            assert_eq!(palette.active, Some(1));
            assert_eq!(palette.query(cx), "main");
        });
    }
    #[gpui::test]
    fn command_rows_fill_the_list_and_hover_in_trailing_space(cx: &mut gpui::TestAppContext) {
        let (palette, window) = cx.add_window_view(|_, cx| {
            CommandPalette::new(
                [
                    CommandItem::new("new", "新建会话").group("会话"),
                    CommandItem::new("open", "打开本地项目").group("项目"),
                ],
                cx,
            )
        });
        window.simulate_resize(gpui::size(px(640.), px(500.)));
        window.run_until_parked();
        let panel = window.debug_bounds("glass-command-palette").unwrap();
        let row = window.debug_bounds("palette-command-1").unwrap();
        // One pixel of border and six pixels of list padding on each side.
        assert_eq!(row.origin.x, panel.origin.x + px(7.));
        assert_eq!(row.size.width, panel.size.width - px(14.));
        window.simulate_mouse_move(
            gpui::point(row.right() - px(10.), row.center().y),
            None,
            Default::default(),
        );
        window.run_until_parked();
        palette.read_with(window, |palette, _| assert_eq!(palette.active, Some(1)));
    }

    #[gpui::test]
    fn thousands_of_commands_render_only_visible_rows_and_search_reaches_the_end(
        cx: &mut gpui::TestAppContext,
    ) {
        let (palette, window) = cx.add_window_view(|_, cx| {
            CommandPalette::new(
                (0..5000).map(|i| CommandItem::new(format!("command-{i}"), format!("Command {i}"))),
                cx,
            )
        });
        window.simulate_resize(gpui::size(px(640.), px(500.)));
        window.run_until_parked();
        assert!(window.debug_bounds("palette-command-0").is_some());
        assert!(window.debug_bounds("palette-command-4999").is_none());
        palette.update(window, |p, cx| p.set_query("Command 4999", cx));
        window.run_until_parked();
        assert!(window.debug_bounds("palette-command-4999").is_some());
        palette.read_with(window, |p, cx| assert_eq!(p.visible(cx), vec![4999]));
    }
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
