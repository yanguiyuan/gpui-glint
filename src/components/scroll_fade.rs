use gpui::{ParentElement, ScrollHandle, Styled, div, linear_color_stop, linear_gradient, px, rgb, rgba};

/// Non-interactive top/bottom linear gradient fade overlay for scrollable containers.
/// Attach inside a `.relative()` (and optionally `.overflow_hidden()`) parent.
pub struct ScrollFade {
    background: u32,
    height: f32,
    show_top: bool,
    show_bottom: bool,
}

impl ScrollFade {
    pub fn new(background: u32) -> Self {
        Self {
            background,
            height: 20.,
            show_top: true,
            show_bottom: true,
        }
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = height.max(4.);
        self
    }

    pub fn top(mut self, show: bool) -> Self {
        self.show_top = show;
        self
    }

    pub fn bottom(mut self, show: bool) -> Self {
        self.show_bottom = show;
        self
    }

    /// Show top/bottom fades only when content is actually clipped at that edge.
    pub fn from_scroll(mut self, scroll: &ScrollHandle) -> Self {
        let max_y = scroll.max_offset().y;
        if max_y <= px(1.) {
            self.show_top = false;
            self.show_bottom = false;
        } else {
            let offset_y = scroll.offset().y;
            self.show_top = offset_y < px(-1.);
            self.show_bottom = offset_y > -max_y + px(1.);
        }
        self
    }

    /// Show top/bottom fades for a virtualized `gpui::ListState`.
    pub fn from_list(mut self, list: &gpui::ListState) -> Self {
        let max_y = list.max_offset_for_scrollbar().y;
        if max_y <= px(1.) {
            self.show_top = false;
            self.show_bottom = false;
        } else {
            let offset_y = list.scroll_px_offset_for_scrollbar().y;
            self.show_top = offset_y < px(-1.);
            self.show_bottom = offset_y > -max_y + px(1.);
        }
        self
    }

    pub fn render(self) -> gpui::Div {
        let solid = rgb(self.background);
        let transparent = rgba(self.background << 8);
        let mut layer = div().absolute().inset_0();
        if self.show_top {
            layer = layer.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(self.height))
                    .bg(linear_gradient(
                        180.0,
                        linear_color_stop(solid, 0.0),
                        linear_color_stop(transparent, 1.0),
                    )),
            );
        }
        if self.show_bottom {
            layer = layer.child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(px(self.height))
                    .bg(linear_gradient(
                        0.0,
                        linear_color_stop(solid, 0.0),
                        linear_color_stop(transparent, 1.0),
                    )),
            );
        }
        layer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_fade_respects_manual_edges() {
        let fade = ScrollFade::new(0xffffff).height(24.).top(true).bottom(false);
        assert!(fade.show_top);
        assert!(!fade.show_bottom);
        assert_eq!(fade.height, 24.);
    }
}
