use crate::theme::ThemePreset;
use gpui::{
    Animation, AnimationExt, AnyElement, ElementId, HighlightStyle, IntoElement, ParentElement,
    SharedString, Styled, StyledText, div, rgb, rgb_to_hsla,
};
use std::{ops::Range, time::Duration};
use unicode_segmentation::UnicodeSegmentation;

/// A quiet, repeating highlight for pending status text.
///
/// Typography is inherited from the parent. Give each instance a stable, unique
/// ID so unrelated renders preserve its animation phase. GPUI automatically
/// renders a static frame when the system requests reduced motion.
///
/// ```ignore
/// div().text_size(px(12.)).child(
///     ShimmerText::new("reply-pending", "正在等待模型回复…").theme(theme)
/// )
/// ```
pub struct ShimmerText {
    id: ElementId,
    text: SharedString,
    theme: ThemePreset,
    duration: Duration,
    animated: bool,
}

impl ShimmerText {
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            theme: ThemePreset::default(),
            duration: Duration::from_millis(2200),
            animated: true,
        }
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    /// Duration of a complete sweep, including a short pause before repeating.
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = duration.max(Duration::from_millis(1));
        self
    }

    /// Disable animation for a settled state or an application preference.
    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    pub fn render(self) -> AnyElement {
        let p = self.theme.palette();
        let base = p.text_muted;
        let peak = p.text;
        let label = div().text_color(rgb(base));
        if !self.animated || self.text.is_empty() {
            return label.child(self.text).into_any_element();
        }
        let ranges = grapheme_ranges(&self.text);
        label
            .with_animation(
                self.id,
                Animation::new(self.duration).repeat(),
                move |label, phase| {
                    let count = ranges.len() as f32;
                    let highlights = ranges.iter().enumerate().map(|(index, range)| {
                        let position = (index as f32 + 0.5) / count;
                        (
                            range.clone(),
                            HighlightStyle {
                                color: Some(rgb_to_hsla(rgb(mix(
                                    base,
                                    peak,
                                    strength(position, phase),
                                )))),
                                ..Default::default()
                            },
                        )
                    });
                    // Style one continuous text run: spacing, wrapping, emoji
                    // and combining marks retain their normal text layout.
                    label.child(StyledText::new(self.text.clone()).with_highlights(highlights))
                },
            )
            .into_any_element()
    }
}

impl IntoElement for ShimmerText {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        self.render()
    }
}

fn grapheme_ranges(text: &str) -> Vec<Range<usize>> {
    text.grapheme_indices(true)
        .map(|(start, grapheme)| start..start + grapheme.len())
        .collect()
}

fn strength(position: f32, phase: f32) -> f32 {
    // The band starts/ends outside the text, so the repeating boundary has no
    // jump. The final fifth of the cycle rests at the base color.
    let center = -0.35 + 1.7 * (phase / 0.8).min(1.);
    let distance = ((position - center) / 0.3).abs().min(1.);
    (1. + (distance * std::f32::consts::PI).cos()) / 2.
}

fn mix(base: u32, peak: u32, amount: f32) -> u32 {
    [16, 8, 0].into_iter().fold(0, |color, shift| {
        let from = ((base >> shift) & 255) as f32;
        let to = ((peak >> shift) & 255) as f32;
        color | (((from + (to - from) * amount).round() as u32) << shift)
    })
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
            div().flex().child(
                div()
                    .id("shimmer-label")
                    .debug_selector(|| "shimmer-label".into())
                    .text_size(gpui::px(15.))
                    .child(
                        ShimmerText::new("pending", "等待 e\u{301} 👩‍💻…")
                            .theme(self.theme)
                            .animated(self.animated),
                    ),
            )
        }
    }

    #[gpui::test]
    fn static_animated_and_reduced_motion_labels_keep_the_same_layout(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, window) = cx.add_window_view(|_, _| Preview {
            animated: true,
            theme: ThemePreset::Moonstone,
        });
        let animated = window.debug_bounds("shimmer-label").unwrap();
        assert!(animated.size.width > gpui::px(0.));
        view.update(window, |view, cx| {
            view.animated = false;
            cx.notify();
        });
        assert_eq!(window.debug_bounds("shimmer-label").unwrap(), animated);
        window.update(|_, cx| cx.set_reduce_motion(true));
        view.update(window, |view, cx| {
            view.animated = true;
            view.theme = ThemePreset::DarkObsidian;
            cx.notify();
        });
        assert_eq!(window.debug_bounds("shimmer-label").unwrap(), animated);
    }

    #[test]
    fn shimmer_keeps_unicode_graphemes_and_byte_ranges_intact() {
        let text = "等待 e\u{301} 👩‍💻…";
        let ranges = grapheme_ranges(text);
        let parts: Vec<_> = ranges.iter().map(|range| &text[range.clone()]).collect();
        assert_eq!(parts, ["等", "待", " ", "e\u{301}", " ", "👩‍💻", "…"]);
        assert_eq!(parts.concat(), text);
        assert!(grapheme_ranges("").is_empty());
    }

    #[test]
    fn band_travels_left_to_right_and_repeats_without_a_flash() {
        assert!(strength(0.2, 0.26) > strength(0.8, 0.26));
        assert!(strength(0.8, 0.54) > strength(0.2, 0.54));
        for position in [0., 0.2, 0.5, 0.8, 1.] {
            assert_eq!(strength(position, 0.), 0.);
            assert_eq!(strength(position, 0.8), 0.);
            assert_eq!(strength(position, 1.), 0.);
        }
        assert_eq!(mix(0x777777, 0xeeeeee, 0.), 0x777777);
        assert_eq!(mix(0x777777, 0xeeeeee, 1.), 0xeeeeee);
    }
}
