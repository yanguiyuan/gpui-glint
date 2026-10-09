use std::rc::Rc;

use gpui::{
    IntoElement, ParentElement, SharedString, Styled, Window, div, prelude::*, px, rgb, rgba,
};

use crate::{components::Icon, theme::ThemePreset};

type TabHandler = Rc<dyn Fn(usize, &mut Window, &mut gpui::App)>;

/// Visual style of a [`Tabs`] bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TabsKind {
    /// Accent underline under the active tab. The default.
    #[default]
    Underline,
    /// Filter-style pills: the active tab is a rounded capsule with a bold
    /// label, inactive tabs stay quiet text.
    Pill,
}

/// A label shown in a tab bar.
#[derive(Clone, Debug)]
pub struct TabOption {
    pub label: SharedString,
    pub disabled: bool,
    pub icon: Option<Icon>,
    pub count: Option<SharedString>,
}

impl TabOption {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            disabled: false,
            icon: None,
            count: None,
        }
    }

    /// Adds an optional icon before the tab label.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Optional count rendered after the label.
    pub fn count(mut self, count: impl Into<SharedString>) -> Self {
        self.count = Some(count.into());
        self
    }
}

/// A horizontal tab bar. The parent owns the selected panel and updates
/// `selected` after `on_change`.
pub struct Tabs {
    options: Vec<TabOption>,
    selected: usize,
    theme: ThemePreset,
    kind: TabsKind,
    on_change: Option<TabHandler>,
}

impl Tabs {
    pub fn new(options: impl IntoIterator<Item = TabOption>) -> Self {
        Self {
            options: options.into_iter().collect(),
            selected: 0,
            theme: ThemePreset::default(),
            kind: TabsKind::default(),
            on_change: None,
        }
    }

    pub fn selected(mut self, selected: usize) -> Self {
        self.selected = selected;
        self
    }

    /// Switches the visual style; defaults to [`TabsKind::Underline`].
    pub fn kind(mut self, kind: TabsKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn on_change(
        mut self,
        handler: impl Fn(usize, &mut Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }

    pub fn render(self) -> gpui::Div {
        match self.kind {
            TabsKind::Underline => self.render_underline(),
            TabsKind::Pill => self.render_pill(),
        }
    }

    fn render_underline(self) -> gpui::Div {
        let palette = self.theme.palette();
        let mut bar = div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(2.))
            .border_b_1()
            .border_color(rgb(palette.border))
            .text_size(px(13.));

        for (index, option) in self.options.into_iter().enumerate() {
            let active = self.selected == index;
            let disabled = option.disabled;
            let mut label = div().flex().items_center().gap(px(6.));
            if let Some(icon) = option.icon {
                label = label.child(icon.render(14.));
            }
            label = label.child(option.label);

            let mut tab = div()
                .id(("glass-tab", index))
                .h(px(36.))
                .px(px(12.))
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .when(active, |tab| {
                    tab.border_b_2().border_color(rgb(palette.accent))
                })
                .text_color(rgb(if disabled {
                    palette.text_muted
                } else if active {
                    palette.text
                } else {
                    palette.text_muted
                }))
                .when(!disabled, |el| {
                    el.cursor_pointer()
                        .hover(|style| style.bg(rgb(palette.surface_subtle)))
                })
                .child(label);

            if disabled {
                tab = tab.opacity(0.55);
            } else if !active && let Some(handler) = self.on_change.clone() {
                tab = tab.on_click(move |_, window, cx| handler(index, window, cx));
            }
            bar = bar.child(tab);
        }
        bar
    }

    fn render_pill(self) -> gpui::Div {
        let palette = self.theme.palette();
        let dark = self.theme.is_dark();
        let hover_bg = if dark {
            rgba(0xffffff0d)
        } else {
            rgba(0x0000000a)
        };
        let mut bar = div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(13.));

        for (index, option) in self.options.into_iter().enumerate() {
            let active = self.selected == index;
            let disabled = option.disabled;
            let mut label = div().flex().items_center().gap(px(6.));
            if let Some(icon) = option.icon {
                label = label.child(icon.render(14.));
            }
            label = label.child(
                div()
                    .font_weight(if active {
                        gpui::FontWeight::SEMIBOLD
                    } else {
                        gpui::FontWeight::MEDIUM
                    })
                    .child(option.label),
            );
            if let Some(count) = option.count {
                label = label.child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(palette.text_muted))
                        .child(count),
                );
            }

            let mut tab = div()
                .id(("glass-tab", index))
                .h(px(32.))
                .px(px(14.))
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .rounded(px(16.))
                .when(active, |tab| tab.bg(rgb(palette.surface_subtle)))
                .text_color(rgb(if disabled {
                    palette.text_muted
                } else {
                    palette.text
                }))
                .when(!disabled && !active, |el| {
                    el.cursor_pointer().hover(move |style| style.bg(hover_bg))
                })
                .when(!disabled && active, |el| el.cursor_pointer())
                .child(label);

            if disabled {
                tab = tab.opacity(0.55);
            } else if !active && let Some(handler) = self.on_change.clone() {
                tab = tab.on_click(move |_, window, cx| handler(index, window, cx));
            }
            bar = bar.child(tab);
        }
        bar
    }
}

impl IntoElement for Tabs {
    type Element = gpui::Div;

    fn into_element(self) -> Self::Element {
        self.render()
    }
}
