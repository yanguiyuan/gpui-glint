use crate::theme::ThemePreset;
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, StatefulInteractiveElement, Styled, Window, deferred, div,
    prelude::*, px, rgb, rgba,
};

#[derive(Clone, Debug)]
pub enum ThemeSelectorEvent {
    Changed(ThemePreset),
}

/// A compact select control that previews each available theme before choosing it.
/// Subscribe to `Changed` and commit the new value with `set_selected`.
pub struct ThemeSelector {
    selected: ThemePreset,
    themes: Vec<ThemePreset>,
    theme: ThemePreset,
    open: bool,
    disabled: bool,
    width: f32,
    active: usize,
    focus: FocusHandle,
}

impl EventEmitter<ThemeSelectorEvent> for ThemeSelector {}

impl Focusable for ThemeSelector {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ThemeSelector {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            selected: ThemePreset::default(),
            themes: ThemePreset::ALL.to_vec(),
            theme: ThemePreset::default(),
            open: false,
            disabled: false,
            width: 240.,
            active: ThemePreset::ALL
                .iter()
                .position(|theme| *theme == ThemePreset::default())
                .unwrap_or(0),
            focus: cx.focus_handle(),
        }
    }

    pub fn selected(mut self, theme: ThemePreset) -> Self {
        self.selected = theme;
        self.active = self
            .themes
            .iter()
            .position(|candidate| *candidate == theme)
            .unwrap_or(0);
        self
    }

    /// Configure available themes in display order, removing duplicates.
    /// The selected value is preserved even when absent from the list.
    pub fn themes(mut self, themes: impl IntoIterator<Item = ThemePreset>) -> Self {
        self.replace_themes(themes);
        self
    }

    pub fn available_themes(&self) -> &[ThemePreset] {
        &self.themes
    }

    /// Update options without emitting Changed or changing the selected theme.
    /// An empty list closes the popup and prevents it from opening.
    pub fn set_themes(
        &mut self,
        themes: impl IntoIterator<Item = ThemePreset>,
        cx: &mut Context<Self>,
    ) {
        self.replace_themes(themes);
        cx.notify();
    }

    fn replace_themes(&mut self, themes: impl IntoIterator<Item = ThemePreset>) {
        let mut options = Vec::new();
        for theme in themes {
            if !options.contains(&theme) {
                options.push(theme);
            }
        }
        self.themes = options;
        self.active = self
            .themes
            .iter()
            .position(|theme| *theme == self.selected)
            .unwrap_or(0);
        if self.themes.is_empty() {
            self.open = false;
        }
    }

    /// Controls the chrome of the selector; each option always previews its own palette.
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(160.);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn selected_theme(&self) -> ThemePreset {
        self.selected
    }

    pub fn set_selected(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.selected = theme;
        self.active = self
            .themes
            .iter()
            .position(|candidate| *candidate == theme)
            .unwrap_or(0);
        cx.notify();
    }

    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            cx.notify();
        }
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

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.themes.is_empty() {
            return;
        }
        self.active = self
            .themes
            .iter()
            .position(|theme| *theme == self.selected)
            .unwrap_or(0);
        self.open = true;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn choose(&mut self, theme: ThemePreset, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || !self.themes.contains(&theme) {
            return;
        }
        self.open = false;
        window.focus(&self.focus, cx);
        if self.selected != theme {
            cx.emit(ThemeSelectorEvent::Changed(theme));
        }
        cx.notify();
    }

    fn keyboard(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.disabled || self.themes.is_empty() {
            return false;
        }
        match key {
            "escape" if self.open => {
                self.close(cx);
                window.focus(&self.focus, cx);
            }
            "enter" | "space" if !self.open => self.open(window, cx),
            "enter" if self.open => self.choose(self.themes[self.active], window, cx),
            "up" | "down" | "home" | "end" => {
                if !self.open {
                    self.open(window, cx);
                }
                self.active = match key {
                    "home" => 0,
                    "end" => self.themes.len() - 1,
                    "up" => self.active.saturating_sub(1),
                    _ => (self.active + 1).min(self.themes.len() - 1),
                };
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

impl Render for ThemeSelector {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let selected = self.selected;
        let disabled = self.disabled || self.themes.is_empty();
        let mut root = div()
            .id("glass-theme-selector")
            .relative()
            .w(px(self.width))
            .flex_shrink_0()
            .track_focus(&self.focus)
            .tab_index(if disabled { -1 } else { 0 })
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if this.keyboard(&event.keystroke.key, window, cx) {
                    cx.stop_propagation();
                    window.prevent_default();
                }
            }));

        let mut trigger = div()
            .id("theme-selector-trigger")
            .h(px(30.))
            .w_full()
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(9.))
            .rounded(px(7.))
            .border_1()
            .border_color(rgb(if self.open { p.accent } else { p.border }))
            .bg(rgb(p.surface))
            .when(!disabled, |el| {
                el.cursor_pointer()
                    .hover(|style| style.bg(rgb(p.surface_subtle)))
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
            .child(swatch(selected, 30., 18.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_size(px(13.))
                    .text_color(rgb(p.text))
                    .child(selected.name()),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(p.text_muted))
                    .child("⌄"),
            );

        if disabled {
            trigger = trigger.opacity(0.55);
        }
        root = root.child(trigger);

        if self.open {
            let mut options = div()
                .id("theme-selector-options")
                .max_h(px(360.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(5.));
            for (index, theme) in self.themes.iter().copied().enumerate() {
                let palette = theme.palette();
                let is_selected = theme == selected;
                let is_active = self.active == index;
                options = options.child(
                    div()
                        .id(("theme-selector-option", index))
                        .w_full()
                        .p(px(7.))
                        .flex()
                        .items_center()
                        .gap(px(9.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(rgb(if is_selected {
                            palette.accent
                        } else {
                            palette.border
                        }))
                        .bg(rgb(if is_active {
                            palette.accent_tint
                        } else {
                            palette.surface
                        }))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(palette.accent_tint)))
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.choose(theme, window, cx)),
                        )
                        .child(swatch(theme, 54., 36.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette.text))
                                        .child(theme.name()),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(palette.text_muted))
                                        .child(theme.description()),
                                ),
                        )
                        .when(is_selected, |el| {
                            el.child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(palette.accent))
                                    .child("✓"),
                            )
                        }),
                );
            }
            let popup = div()
                .absolute()
                .top_0()
                .left_0()
                .w(px(self.width))
                .pt(px(36.))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close(cx)))
                .child({
                    let mut surface = div()
                        .p(px(7.))
                        .rounded(px(11.))
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
                        // Keep wheel events inside the popup from scrolling the
                        // page content behind it.
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _, _, cx| cx.stop_propagation()),
                        )
                        .child(options);
                    if self.theme.is_dark() {
                        surface = surface.backdrop_blur(px(48.));
                    }
                    surface
                });
            root = root.child(deferred(popup).with_priority(10));
        }
        root
    }
}

fn swatch(theme: ThemePreset, width: f32, height: f32) -> gpui::Div {
    let p = theme.palette();
    div()
        .w(px(width))
        .h(px(height))
        .flex_shrink_0()
        .relative()
        .overflow_hidden()
        .rounded(px(6.))
        .border_1()
        .border_color(rgba(0x1c202018))
        .bg(rgb(p.canvas))
        .child(
            div()
                .absolute()
                .left(px(width * 0.34))
                .right(px(3.))
                .top(px(4.))
                .bottom(px(0.))
                .rounded_t(px(4.))
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.surface)),
        )
        .child(
            div()
                .absolute()
                .left(px(4.))
                .top(px(5.))
                .w(px(width * 0.18))
                .h(px(height * 0.38))
                .rounded(px(3.))
                .bg(rgb(p.surface_subtle)),
        )
        .child(
            div()
                .absolute()
                .left(px(width * 0.42))
                .top(px(height * 0.52))
                .w(px(width * 0.32))
                .h(px(3.))
                .rounded(px(2.))
                .bg(rgb(p.accent)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext;
    use std::{cell::RefCell, rc::Rc};

    #[gpui::test]
    fn theme_options_default_to_all_and_preserve_selection(cx: &mut gpui::TestAppContext) {
        let selector = cx.new(ThemeSelector::new);
        selector.update(cx, |selector, cx| {
            assert_eq!(selector.available_themes(), &ThemePreset::ALL);
            selector.set_selected(ThemePreset::DarkLinen, cx);
            selector.set_themes(
                [
                    ThemePreset::Moonstone,
                    ThemePreset::Sage,
                    ThemePreset::Moonstone,
                ],
                cx,
            );
            assert_eq!(
                selector.available_themes(),
                &[ThemePreset::Moonstone, ThemePreset::Sage]
            );
            assert_eq!(selector.selected_theme(), ThemePreset::DarkLinen);
            assert_eq!(selector.active, 0);
            selector.set_selected(ThemePreset::Sage, cx);
            assert_eq!(selector.active, 1);
        });
        let custom = cx.new(|cx| {
            ThemeSelector::new(cx)
                .themes([ThemePreset::DarkObsidian, ThemePreset::Moonstone])
                .selected(ThemePreset::Moonstone)
        });
        custom.update(cx, |selector, _| assert_eq!(selector.active, 1));
    }

    #[gpui::test]
    fn keyboard_chooses_only_available_themes_and_handles_empty_updates(
        cx: &mut gpui::TestAppContext,
    ) {
        let (selector, window) = cx.add_window_view(|_, cx| {
            ThemeSelector::new(cx).themes([ThemePreset::Moonstone, ThemePreset::DarkObsidian])
        });
        let changes = Rc::new(RefCell::new(Vec::new()));
        let collected = changes.clone();
        let _subscription = window.update(|_, cx| {
            cx.subscribe(&selector, move |_, event: &ThemeSelectorEvent, _| {
                let ThemeSelectorEvent::Changed(theme) = event;
                collected.borrow_mut().push(*theme);
            })
        });
        window.update(|window, cx| {
            selector.update(cx, |selector, cx| {
                assert!(selector.keyboard("enter", window, cx));
                assert!(selector.open);
                selector.keyboard("end", window, cx);
                assert_eq!(selector.active, 1);
                selector.keyboard("down", window, cx);
                assert_eq!(selector.active, 1);
                selector.keyboard("enter", window, cx);
                assert!(!selector.open);
                assert_eq!(selector.selected_theme(), ThemePreset::Sage);
                selector.choose(ThemePreset::DarkLinen, window, cx);
            })
        });
        assert_eq!(*changes.borrow(), vec![ThemePreset::DarkObsidian]);
        window.update(|window, cx| {
            selector.update(cx, |selector, cx| {
                selector.keyboard("enter", window, cx);
                selector.keyboard("end", window, cx);
                selector.set_themes([ThemePreset::Moonstone], cx);
                assert_eq!(selector.active, 0);
                selector.keyboard("enter", window, cx);
                selector.keyboard("enter", window, cx);
                selector.set_themes([], cx);
                assert!(!selector.open);
                for key in ["enter", "space", "up", "down", "home", "end"] {
                    assert!(!selector.keyboard(key, window, cx));
                }
            })
        });
        assert_eq!(
            *changes.borrow(),
            vec![ThemePreset::DarkObsidian, ThemePreset::Moonstone]
        );
    }
}
