//! Scroll chaining for nested scrollable containers.
//!
//! gpui-ce dispatches wheel events to every scrollable ancestor, so a capped
//! diff card inside the workspace file list scrolled both itself and the list
//! in lockstep. The trait below restores the familiar contract: wheel events
//! scroll the innermost container first, and only once it is pinned at its top
//! or bottom edge does the event bubble so an outer container takes over.
//!
//! Registration order matters: a scrollable element registers its own built-in
//! wheel handler during paint, *before* any `on_scroll_wheel` listener attached
//! through this trait, and the bubble phase replays listeners in reverse. The
//! built-in handler therefore consumes (or clamps) the delta first; this
//! listener only decides whether the already-applied scroll should still reach
//! the ancestors.

use gpui::{App, InteractiveElement, ScrollHandle, ScrollWheelEvent, Window, px};

/// Margin (in pixels) that absorbs float drift when checking scroll edges.
const EDGE_EPSILON: f32 = 0.5;

/// Returns true when the vertical wheel delta was consumed by `handle` and
/// must not bubble to outer scrollable ancestors.
///
/// gpui stores scroll offsets in `[-max, 0]` (`0` = top, `-max` = bottom) and
/// wheel-down deltas arrive negative, matching the built-in scroll handler.
fn consumes_wheel(handle: &ScrollHandle, delta_y: f32) -> bool {
    if delta_y == 0. {
        return false;
    }
    let offset = handle.offset().y;
    let max = handle.max_offset().y;
    if delta_y < 0. {
        // Scrolling toward the bottom edge.
        offset > -max + px(EDGE_EPSILON)
    } else {
        // Scrolling toward the top edge.
        offset < px(-EDGE_EPSILON)
    }
}

pub trait ChainScroll: InteractiveElement + Sized {
    /// Chains vertical wheel scrolling of this element through `handle`.
    ///
    /// Place after `.id(..)` / `.overflow_y_scroll()` / `.track_scroll(..)` on
    /// the scrollable element itself. While the element can still scroll in the
    /// wheel direction the event stops here; at either edge it propagates so
    /// an outer scroll container takes over.
    fn chain_scroll_y(self, handle: &ScrollHandle) -> Self {
        let handle = handle.clone();
        self.on_scroll_wheel(
            move |event: &ScrollWheelEvent, window: &mut Window, cx: &mut App| {
                let delta_y = event.delta.pixel_delta(window.line_height()).y;
                if consumes_wheel(&handle, delta_y.into()) {
                    cx.stop_propagation();
                }
            },
        )
    }
}

impl<E: InteractiveElement> ChainScroll for E {}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Context, IntoElement, ParentElement, Render, ScrollDelta, StatefulInteractiveElement,
        Styled, TestAppContext, div, point, px,
    };

    struct Nested {
        inner: ScrollHandle,
        outer: ScrollHandle,
    }

    impl Render for Nested {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("chain-outer")
                .debug_selector(|| "chain-outer".into())
                .w(px(300.))
                .h(px(300.))
                .overflow_y_scroll()
                .track_scroll(&self.outer)
                .child(
                    div()
                        .id("chain-inner")
                        .debug_selector(|| "chain-inner".into())
                        .w_full()
                        .h(px(200.))
                        .overflow_y_scroll()
                        .track_scroll(&self.inner)
                        .chain_scroll_y(&self.inner)
                        .child(div().h(px(1000.))),
                )
                .child(div().h(px(1000.)))
        }
    }

    fn wheel(y: f32, position: gpui::Point<gpui::Pixels>) -> ScrollWheelEvent {
        ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(y))),
            touch_phase: gpui::TouchPhase::Started,
            ..Default::default()
        }
    }

    #[gpui::test]
    fn inner_scroller_wheels_first_and_chains_at_edges(cx: &mut TestAppContext) {
        let inner = ScrollHandle::new();
        let outer = ScrollHandle::new();
        let (_, window) = cx.add_window_view(|_, _| Nested {
            inner: inner.clone(),
            outer: outer.clone(),
        });
        window.run_until_parked();

        // Inside the range: the inner scroller consumes, the outer stays put.
        let position = window.debug_bounds("chain-inner").unwrap().center();
        window.simulate_event(wheel(-120., position));
        assert_eq!(inner.offset().y, px(-120.));
        assert_eq!(outer.offset().y, px(0.));

        // Pin the inner scroller at its bottom edge, then wheel down again:
        // the same event must now move the outer container instead.
        inner.set_offset(point(px(0.), px(-800.)));
        window.simulate_event(wheel(-120., position));
        assert_eq!(inner.offset().y, px(-800.));
        assert_eq!(outer.offset().y, px(-120.));

        // The outer scroll shifted the inner's on-screen bounds; aim at the
        // still-visible part of the inner for the remaining events.
        let bounds = window.debug_bounds("chain-inner").unwrap();
        let visible = point(bounds.center().x, bounds.top().max(px(0.)) + px(40.));

        // Wheeling up at the bottom edge keeps scrolling the inner first.
        window.simulate_event(wheel(120., visible));
        assert_eq!(inner.offset().y, px(-680.));
        assert_eq!(outer.offset().y, px(-120.));

        // Back at the top edge, wheel-up chains to the outer container.
        inner.set_offset(point(px(0.), px(0.)));
        window.simulate_event(wheel(120., visible));
        assert_eq!(inner.offset().y, px(0.));
        assert_eq!(outer.offset().y, px(0.));
    }
}
