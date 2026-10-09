use crate::theme::ThemePreset;
use std::hash::{Hash, Hasher};

use gpui::{
    AnyElement, AppContext, Context, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px, rgb,
};

/// Wraps an element with a small tooltip shown while it is hovered.
pub struct Tooltip {
    theme: ThemePreset,
    trigger: AnyElement,
    label: SharedString,
}

impl Tooltip {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn new(trigger: impl IntoElement, label: impl Into<SharedString>) -> Self {
        Self {
            theme: ThemePreset::default(),
            trigger: trigger.into_any_element(),
            label: label.into(),
        }
    }

    pub fn render(self, cx: &mut Context<impl Render>) -> gpui::Stateful<gpui::Div> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.label.hash(&mut hasher);
        let tooltip = cx.new(|_| TooltipLabel(self.label, self.theme));
        div()
            .id(("glass-tooltip", hasher.finish()))
            .tooltip(move |_, _| tooltip.clone().into())
            .child(self.trigger)
    }
}

struct TooltipLabel(SharedString, ThemePreset);

impl Render for TooltipLabel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = self.1;
        div()
            .px_2()
            .py_1()
            .rounded(px(5.))
            .border_1()
            .border_color(rgb(if theme.is_dark() {
                theme.palette().border
            } else {
                0x41454e
            }))
            .bg(rgb(if theme.is_dark() {
                theme.palette().surface
            } else {
                0x282b31
            }))
            .text_color(rgb(0xffffff))
            .text_size(px(12.))
            .shadow_md()
            .child(self.0.clone())
    }
}
