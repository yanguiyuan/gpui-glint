use crate::theme::ThemePreset;
use gpui::{
    Animation, AnimationExt, AnyElement, ElementId, IntoElement, ParentElement, Styled, div,
    prelude::FluentBuilder, px, relative, rgb,
};
use std::time::Duration;

/// A noninteractive, softly pulsing placeholder for content that is still loading.
///
/// The default is one full-width bar. Compose blocks with `width`, `height` and
/// `rounded`, or use `lines` for a text/code placeholder with varied line lengths.
/// Keep the ID stable and unique within its view to preserve animation phase.
/// GPUI automatically uses a static frame when reduced motion is enabled.
///
/// ```ignore
/// Skeleton::new("pending-code").lines(8).height(12.).gap(14.).theme(theme)
/// ```
pub struct Skeleton {
    id: ElementId,
    theme: ThemePreset,
    width: Option<f32>,
    height: f32,
    radius: f32,
    lines: usize,
    gap: f32,
    duration: Duration,
    animated: bool,
}

impl Skeleton {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            theme: ThemePreset::default(),
            width: None,
            height: 14.,
            radius: 4.,
            lines: 1,
            gap: 10.,
            duration: Duration::from_millis(1800),
            animated: true,
        }
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    /// Fixed width in pixels; the default fills the available width.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width.max(0.));
        self
    }

    /// Height of each placeholder bar in pixels.
    pub fn height(mut self, height: f32) -> Self {
        self.height = height.max(0.);
        self
    }

    pub fn rounded(mut self, radius: f32) -> Self {
        self.radius = radius.max(0.);
        self
    }

    /// Render at least one line, varying widths to suggest text or code.
    pub fn lines(mut self, lines: usize) -> Self {
        self.lines = lines.max(1);
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.);
        self
    }

    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration.max(Duration::from_millis(1));
        self
    }

    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    pub fn render(self) -> AnyElement {
        let widths = [1., 0.82, 0.94, 0.64, 0.88, 0.72];
        let blocks = div()
            .w_full()
            .when_some(self.width, |blocks, width| blocks.w(px(width)))
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(self.gap))
            .children((0..self.lines).map(|index| {
                div()
                    .w(relative(widths[index % widths.len()]))
                    .h(px(self.height))
                    .flex_shrink_0()
                    .rounded(px(self.radius))
                    .bg(rgb(self.theme.palette().border))
            }));
        if !self.animated {
            return blocks.into_any_element();
        }
        blocks
            .with_animation(
                self.id,
                Animation::new(self.duration).repeat(),
                |blocks, phase| {
                    let pulse = (1. - (phase * std::f32::consts::TAU).cos()) / 2.;
                    blocks.opacity(0.5 + 0.5 * pulse)
                },
            )
            .into_any_element()
    }
}

impl IntoElement for Skeleton {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        self.render()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::InteractiveElement;

    struct Preview {
        animated: bool,
        theme: ThemePreset,
    }

    impl gpui::Render for Preview {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div().flex().items_start().child(
                div()
                    .id("skeleton-bounds")
                    .debug_selector(|| "skeleton-bounds".into())
                    .w(px(240.))
                    .child(
                        Skeleton::new("pending")
                            .lines(4)
                            .height(12.)
                            .gap(8.)
                            .animated(self.animated)
                            .theme(self.theme),
                    ),
            )
        }
    }

    #[gpui::test]
    fn animation_and_reduced_motion_preserve_placeholder_geometry(cx: &mut gpui::TestAppContext) {
        let (view, window) = cx.add_window_view(|_, _| Preview {
            animated: true,
            theme: ThemePreset::Moonstone,
        });
        let bounds = window.debug_bounds("skeleton-bounds").unwrap();
        assert_eq!(bounds.size.width, px(240.));
        assert_eq!(bounds.size.height, px(72.));
        view.update(window, |view, cx| {
            view.animated = false;
            cx.notify();
        });
        assert_eq!(window.debug_bounds("skeleton-bounds").unwrap(), bounds);
        window.update(|_, cx| cx.set_reduce_motion(true));
        view.update(window, |view, cx| {
            view.animated = true;
            view.theme = ThemePreset::DarkObsidian;
            cx.notify();
        });
        assert_eq!(window.debug_bounds("skeleton-bounds").unwrap(), bounds);
    }
}
