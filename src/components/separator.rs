use crate::theme::ThemePreset;
use gpui::{Styled, div, px, rgb};

/// A noninteractive line separating adjacent regions in a flex layout.
pub struct Separator {
    vertical: bool,
    theme: ThemePreset,
}
impl Separator {
    pub fn horizontal() -> Self {
        Self {
            vertical: false,
            theme: ThemePreset::default(),
        }
    }
    pub fn vertical() -> Self {
        Self {
            vertical: true,
            ..Self::horizontal()
        }
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn render(self) -> gpui::Div {
        let line = div().flex_shrink_0().bg(rgb(self.theme.palette().border));
        if self.vertical {
            line.w(px(1.)).h_full()
        } else {
            line.h(px(1.)).w_full()
        }
    }
}
