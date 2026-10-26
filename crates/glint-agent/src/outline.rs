//! Lightweight conversation navigation. Hover only invalidates this entity.
use super::*;
use gpui::{Animation, AnimationExt as _, EventEmitter, anchored, deferred, relative};
use std::time::{Duration, Instant};

const PITCH: f32 = 10.;
const TRANSITION: f32 = 0.18;
pub type Entry = (usize, String, String);
pub struct Jump(pub usize);

pub struct ConversationOutline {
    session: Uuid,
    entries: Vec<Entry>,
    active: Option<usize>,
    hover: Option<usize>,
    from: Vec<f32>,
    changed: Instant,
    version: u64,
    theme: ThemePreset,
}

impl EventEmitter<Jump> for ConversationOutline {}

impl ConversationOutline {
    pub fn new(theme: ThemePreset) -> Self {
        Self {
            session: Uuid::nil(),
            entries: Vec::new(),
            active: None,
            hover: None,
            from: Vec::new(),
            changed: Instant::now(),
            version: 0,
            theme,
        }
    }

    pub fn set(
        &mut self,
        session: Uuid,
        entries: Vec<Entry>,
        top: usize,
        theme: ThemePreset,
        cx: &mut Context<Self>,
    ) {
        let changed = self.session != session || self.entries != entries || self.theme != theme;
        if changed {
            self.session = session;
            self.entries = entries;
            self.theme = theme;
            self.hover = None;
            self.version += 1;
            self.from = self.widths();
        }
        self.set_top(top, cx);
        if changed {
            cx.notify();
        }
    }

    pub fn set_top(&mut self, top: usize, cx: &mut Context<Self>) {
        let active = self
            .entries
            .iter()
            .map(|(index, _, _)| *index)
            .filter(|index| *index <= top)
            .max();
        if self.active != active {
            self.transition(self.hover);
            self.active = active;
            cx.notify();
        }
    }

    fn widths(&self) -> Vec<f32> {
        self.entries
            .iter()
            .enumerate()
            .map(|(position, (index, _, _))| {
                wave_width(position, self.hover, self.active == Some(*index))
            })
            .collect()
    }

    fn transition(&mut self, hover: Option<usize>) {
        let progress = (self.changed.elapsed().as_secs_f32() / TRANSITION).clamp(0., 1.);
        let progress = 1. - (1. - progress).powi(3);
        let targets = self.widths();
        self.from = targets
            .iter()
            .enumerate()
            .map(|(index, target)| {
                let from = self.from.get(index).copied().unwrap_or(*target);
                from + (target - from) * progress
            })
            .collect();
        self.hover = hover;
        self.version += 1;
        self.changed = Instant::now();
    }
}

fn wave_width(position: usize, hover: Option<usize>, active: bool) -> f32 {
    let base: f32 = if active { 18. } else { 12. };
    let Some(hover) = hover else {
        return base;
    };
    let distance = position.abs_diff(hover) as f32;
    base.max(12. + 22. * (-0.24 * distance * distance).exp())
}

impl Render for ConversationOutline {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let targets = self.widths();
        let ticks = self
            .entries
            .iter()
            .enumerate()
            .map(|(position, (index, _, _))| {
                let index = *index;
                let from = self
                    .from
                    .get(position)
                    .copied()
                    .unwrap_or(targets[position]);
                let target = targets[position];
                div()
                    .id(("outline-tick", position))
                    .debug_selector(move || format!("outline-tick-{position}").into())
                    .w(px(40.))
                    .h(px(PITCH))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .child(
                        div()
                            .h(px(2.))
                            .rounded_full()
                            .bg(rgb(
                                if self.active == Some(index) || self.hover == Some(position) {
                                    p.text
                                } else {
                                    p.border
                                },
                            ))
                            .with_animation(
                                SharedString::from(format!(
                                    "outline-wave:{}:{position}",
                                    self.version
                                )),
                                Animation::new(Duration::from_millis(180))
                                    .with_easing(|delta| 1. - (1. - delta).powi(3)),
                                move |bar, delta| bar.w(px(from + (target - from) * delta)),
                            ),
                    )
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        let next = if *hovered {
                            Some(position)
                        } else if this.hover == Some(position) {
                            None
                        } else {
                            this.hover
                        };
                        if this.hover != next {
                            this.transition(next);
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(Jump(index));
                    }))
            });
        let mut root = div()
            .absolute()
            .left_0()
            .top_0()
            .bottom_0()
            .w(px(46.))
            .pl(px(6.))
            .flex()
            .flex_col()
            .justify_center()
            .items_start()
            .child(
                div()
                    .id("outline-ticks")
                    .debug_selector(|| "outline-ticks".into())
                    .flex()
                    .flex_col()
                    .children(ticks),
            );
        if let Some(position) = self.hover {
            if let Some((_, question, reply)) = self.entries.get(position) {
                let card = div()
                    .w(px(320.))
                    .p_3()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(p.border))
                    .bg(rgb(p.surface))
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_color(rgb(p.text))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(question.clone()),
                    )
                    .when(!reply.is_empty(), |card| {
                        card.child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(p.text_muted))
                                .child(reply.clone()),
                        )
                    });
                root = root.child(
                    div()
                        .absolute()
                        .left(px(46.))
                        .top(relative(0.5))
                        .mt(px((position as f32
                            - (self.entries.len() as f32 - 1.) / 2.)
                            * PITCH
                            - 12.))
                        .child(
                            deferred(anchored().snap_to_window_with_margin(px(8.)).child(card))
                                .with_priority(20),
                        ),
                );
            }
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wave_falls_off_symmetrically_and_preserves_active_tick() {
        assert!(wave_width(3, Some(3), false) > wave_width(2, Some(3), false));
        assert!(wave_width(2, Some(3), false) > wave_width(1, Some(3), false));
        assert_eq!(wave_width(2, Some(3), false), wave_width(4, Some(3), false));
        assert_eq!(wave_width(20, Some(3), true), 18.);
    }
    #[gpui::test]
    fn ticks_are_centered_compact_and_clicks_jump_to_message(cx: &mut gpui::TestAppContext) {
        let (outline, window) =
            cx.add_window_view(|_, _| ConversationOutline::new(ThemePreset::Moonstone));
        outline.update(window, |outline, cx| {
            outline.set(
                Uuid::new_v4(),
                vec![
                    (0, "A".into(), "reply".into()),
                    (2, "B".into(), "reply".into()),
                    (4, "C".into(), "reply".into()),
                ],
                2,
                ThemePreset::Moonstone,
                cx,
            )
        });
        let bounds = window.debug_bounds("outline-ticks").unwrap();
        let first = window.debug_bounds("outline-tick-0").unwrap();
        let second = window.debug_bounds("outline-tick-1").unwrap();
        assert_eq!(second.top() - first.top(), px(PITCH));
        assert_eq!(bounds.size.height, px(3. * PITCH));
        let window_height = window.update(|window, _| window.viewport_size().height);
        assert_eq!(bounds.center().y, window_height / 2.);
        let result = std::rc::Rc::new(std::cell::Cell::new(usize::MAX));
        let received = result.clone();
        let _subscription = window.update(|_, cx| {
            cx.subscribe(&outline, move |_, event: &Jump, _| received.set(event.0))
        });
        window.simulate_click(second.center(), gpui::Modifiers::default());
        assert_eq!(result.get(), 2);
    }
}
