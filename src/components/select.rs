use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, deferred, div, px, rgb, rgba,
};

use super::{Icon, TextEntry};
use crate::theme::ThemePreset;

/// A stable value and its display label. Values should be unique within a Select.
#[derive(Clone, Debug)]
pub struct SelectOption {
    pub value: SharedString,
    pub label: SharedString,
    pub disabled: bool,
    /// 可选的 0xRRGGBB 文字色，用于选项及选中后的触发器。
    pub color: Option<u32>,
}
impl SelectOption {
    pub fn new(value: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            disabled: false,
            color: None,
        }
    }
    pub fn color(mut self, color: u32) -> Self {
        self.color = Some(color);
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

#[derive(Clone, Debug)]
pub enum SelectEvent {
    Changed(SharedString),
}

/// Controlled single selection. Subscribe to Changed and commit with set_selected.
/// Search uses TextEntry; register bind_text_entry_keys once in the application.
pub struct Select {
    options: Vec<SelectOption>,
    selected: Option<SharedString>,
    placeholder: SharedString,
    searchable: bool,
    disabled: bool,
    open: bool,
    active: Option<usize>,
    scroll: ScrollHandle,
    scroll_pending: bool,
    width: f32,
    min_width: f32,
    embedded: bool,
    theme: ThemePreset,
    focus: FocusHandle,
    search: Entity<TextEntry>,
    _search_subscription: Subscription,
}
impl EventEmitter<SelectEvent> for Select {}
impl Focusable for Select {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Select {
    pub fn new(options: impl IntoIterator<Item = SelectOption>, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextEntry::new(cx, "搜索选项…", ""));
        let subscription = cx.observe(&search, |this, _, cx| {
            this.scroll_pending = true;
            if !this
                .visible(cx)
                .contains(&this.active.unwrap_or(usize::MAX))
            {
                this.active = this
                    .visible(cx)
                    .into_iter()
                    .find(|&i| !this.options[i].disabled);
            }
            cx.notify();
        });
        Self {
            options: options.into_iter().collect(),
            selected: None,
            placeholder: "请选择".into(),
            searchable: false,
            disabled: false,
            open: false,
            active: None,
            scroll: ScrollHandle::new(),
            scroll_pending: true,
            width: 220.,
            min_width: 100.,
            embedded: false,
            theme: ThemePreset::default(),
            focus: cx.focus_handle(),
            search,
            _search_subscription: subscription,
        }
    }
    pub(crate) fn embedded(mut self, width: f32) -> Self {
        self.embedded = true;
        self.width = width.max(0.);
        self.min_width = 60.;
        self
    }
    pub fn selected(mut self, value: impl Into<SharedString>) -> Self {
        self.selected = Some(value.into());
        self
    }
    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = text.into();
        self
    }
    pub fn searchable(mut self, searchable: bool) -> Self {
        self.searchable = searchable;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(0.);
        self
    }
    pub fn set_width(&mut self, width: f32, cx: &mut Context<Self>) {
        self.width = width.max(0.);
        cx.notify();
    }
    /// Minimum trigger width in pixels (100 by default). Can be lowered for table cells.
    /// Applied independently of builder order; the popup follows the effective width.
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width.max(0.);
        self
    }
    pub fn set_min_width(&mut self, width: f32, cx: &mut Context<Self>) {
        self.min_width = width.max(0.);
        cx.notify();
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
    pub fn theme_preset(&self) -> ThemePreset {
        self.theme
    }
    pub fn value(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }
    pub fn set_selected(&mut self, value: Option<SharedString>, cx: &mut Context<Self>) {
        self.selected = value;
        cx.notify();
    }
    /// Unknown selected values display the placeholder until a matching option is supplied.
    pub fn set_options(
        &mut self,
        options: impl IntoIterator<Item = SelectOption>,
        cx: &mut Context<Self>,
    ) {
        self.options = options.into_iter().collect();
        self.scroll_pending = true;
        self.active = self
            .visible(cx)
            .into_iter()
            .find(|&i| !self.options[i].disabled);
        cx.notify();
    }
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        if disabled {
            self.open = false;
        }
        cx.notify();
    }
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        cx.notify();
    }
    fn visible(&self, cx: &App) -> Vec<usize> {
        matching_options(
            &self.options,
            if self.searchable {
                self.search.read(cx).value()
            } else {
                ""
            },
        )
    }
    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        self.search
            .update(cx, |search, cx| search.set_value("", cx));
        self.active = self
            .options
            .iter()
            .position(|o| !o.disabled && Some(&o.value) == self.selected.as_ref())
            .or_else(|| self.options.iter().position(|o| !o.disabled));
        self.open = true;
        self.scroll_pending = true;
        if self.searchable {
            window.focus(&self.search.focus_handle(cx), cx);
        } else {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }
    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(option) = self.options.get(index).filter(|o| !o.disabled) {
            let value = option.value.clone();
            self.open = false;
            window.focus(&self.focus, cx);
            if Some(&value) != self.selected.as_ref() {
                cx.emit(SelectEvent::Changed(value));
            }
            cx.notify();
        }
    }
    fn keyboard(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.disabled {
            return false;
        }
        match key {
            "escape" if self.open => {
                self.close(cx);
                window.focus(&self.focus, cx);
            }
            "enter" | "space" if !self.open => self.open(window, cx),
            "enter" if self.open => {
                if let Some(i) = self.active {
                    self.choose(i, window, cx);
                }
            }
            "up" | "down" | "home" | "end" => {
                if !self.open {
                    self.open(window, cx);
                }
                let enabled: Vec<_> = self
                    .visible(cx)
                    .into_iter()
                    .filter(|&i| !self.options[i].disabled)
                    .collect();
                self.active = next_option(&enabled, self.active, key);
                self.scroll_pending = true;
                cx.notify();
            }
            "tab" if self.open => {
                self.close(cx);
                return false;
            }
            _ => return false,
        }
        true
    }
}
fn matching_options(options: &[SelectOption], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    options
        .iter()
        .enumerate()
        .filter(|(_, o)| {
            o.label.to_lowercase().contains(&query) || o.value.to_lowercase().contains(&query)
        })
        .map(|(i, _)| i)
        .collect()
}
fn next_option(enabled: &[usize], active: Option<usize>, key: &str) -> Option<usize> {
    if enabled.is_empty() {
        return None;
    }
    let position = active.and_then(|a| enabled.iter().position(|&i| i == a));
    let index = match key {
        "home" => 0,
        "end" => enabled.len() - 1,
        "up" => position.map_or(enabled.len() - 1, |p| p.saturating_sub(1)),
        _ => position.map_or(0, |p| (p + 1).min(enabled.len() - 1)),
    };
    Some(enabled[index])
}
impl Render for Select {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.search
            .update(cx, |input, cx| input.set_theme(self.theme, cx));
        let p = self.theme.palette();
        let option = self
            .options
            .iter()
            .find(|o| Some(&o.value) == self.selected.as_ref());
        let label = option
            .map(|o| o.label.clone())
            .unwrap_or(self.placeholder.clone());
        let mut trigger = div()
            .id("select-trigger")
            .h(px(30.))
            .w_full()
            .px(px(10.))
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .rounded(px(7.))
            .border_1()
            .border_color(rgb(if self.open { p.accent } else { p.border }))
            .bg(rgb(p.surface))
            .text_size(px(13.))
            .text_color(rgb(if self.disabled || option.is_none() {
                p.text_muted
            } else {
                option.and_then(|option| option.color).unwrap_or(p.text)
            }))
            .when(!self.disabled, |el| {
                el.cursor_pointer()
                    .hover(move |s| s.bg(rgb(p.surface_subtle)))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                if this.open {
                    this.close(cx);
                } else {
                    this.open(window, cx);
                }
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(label),
            )
            .child(div().flex_shrink_0().child(Icon::ChevronDown.render(12.)));
        if self.embedded {
            trigger = trigger
                .border_0()
                .rounded(px(0.))
                .bg(gpui::rgba(0x00000000));
        }
        let mut root = div()
            .id("glass-select")
            .relative()
            .w(px(self.width.max(self.min_width)))
            .flex_shrink_0()
            .track_focus(&self.focus)
            .tab_index(if self.disabled { -1 } else { 0 })
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                // Home/End remain text navigation while searching.
                if this.searchable
                    && this.open
                    && matches!(event.keystroke.key.as_str(), "home" | "end" | "space")
                {
                    return;
                }
                if this.keyboard(&event.keystroke.key, window, cx) {
                    cx.stop_propagation();
                    window.prevent_default();
                }
            }))
            .child(trigger);
        if self.open {
            let mut menu = div()
                .p(px(7.))
                .rounded(px(10.))
                .border_1()
                .border_color(if self.theme.is_dark() {
                    rgba(self.theme.glass_line())
                } else {
                    rgb(p.border)
                })
                .bg(if self.theme.is_dark() {
                    rgba(self.theme.menu_material())
                } else {
                    rgb(p.surface)
                })
                .shadow_lg()
                // Keep wheel events inside the popup from scrolling the page
                // content behind it.
                .occlude()
                .flex()
                .flex_col()
                .gap(px(5.))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|_, _, _, cx| cx.stop_propagation()),
                );
            if self.theme.is_dark() {
                menu = menu.backdrop_blur(px(48.));
            }
            if self.searchable {
                menu = menu.child(
                    div()
                        .h(px(28.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .rounded(px(5.))
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.surface_subtle))
                        .child(Icon::Search.render(13.))
                        .child(div().flex_1().min_w_0().child(self.search.clone())),
                );
            }
            let visible = self.visible(cx);
            if self.scroll_pending {
                if let Some(position) = visible.iter().position(|&i| Some(i) == self.active) {
                    self.scroll.scroll_to_item(position);
                }
                self.scroll_pending = false;
            }
            let mut list = div()
                .id("select-options")
                .max_h(px(188.))
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .flex()
                .flex_col()
                .gap(px(1.));
            if visible.is_empty() {
                list = list.child(
                    div()
                        .p_3()
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child("没有匹配的选项"),
                );
            }
            for i in visible {
                let o = &self.options[i];
                let selected = Some(&o.value) == self.selected.as_ref();
                list = list.child(
                    div()
                        .id(("select-option", i))
                        .h(px(30.))
                        .flex_shrink_0()
                        .px(px(9.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .rounded(px(5.))
                        .text_size(px(13.))
                        .text_color(rgb(if o.disabled {
                            p.text_muted
                        } else {
                            o.color.unwrap_or(if selected { p.accent } else { p.text })
                        }))
                        .when(self.active == Some(i) && !o.disabled, |el| {
                            el.bg(rgb(p.accent)).text_color(rgb(0xffffff))
                        })
                        .when(!o.disabled, |el| {
                            el.cursor_pointer()
                                .hover(move |s| s.bg(rgb(p.accent)).text_color(rgb(0xffffff)))
                        })
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.choose(i, window, cx)),
                        )
                        .child(o.label.clone())
                        .when(selected, |el| el.child("✓")),
                );
            }
            // Include the trigger in the outside-click bounds so it still toggles closed.
            let popup = div()
                .absolute()
                .top_0()
                .left_0()
                .w(px(if self.embedded {
                    self.width.max(self.min_width).max(150.)
                } else {
                    self.width.max(self.min_width)
                }))
                .pt(px(36.))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close(cx)))
                .child(menu.child(list));
            root = root.child(deferred(popup).with_priority(10));
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_matches_labels_and_values_case_insensitively() {
        let options = [
            SelectOption::new("staging", "预发布环境"),
            SelectOption::new("prod", "生产环境"),
        ];
        assert_eq!(matching_options(&options, " STAG "), vec![0]);
        assert_eq!(matching_options(&options, "生产"), vec![1]);
        assert!(matching_options(&options, "missing").is_empty());
    }
    #[test]
    fn navigation_handles_filtered_gaps_and_boundaries() {
        let enabled = [1, 4, 8];
        assert_eq!(next_option(&enabled, Some(1), "down"), Some(4));
        assert_eq!(next_option(&enabled, Some(8), "down"), Some(8));
        assert_eq!(next_option(&enabled, Some(1), "up"), Some(1));
        assert_eq!(next_option(&enabled, Some(2), "up"), Some(8));
        assert_eq!(next_option(&[], None, "down"), None);
        assert_eq!(next_option(&enabled, Some(4), "home"), Some(1));
        assert_eq!(next_option(&enabled, None, "end"), Some(8));
    }
}
