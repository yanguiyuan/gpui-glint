//! Cull rows inside a long message against the conversation's existing viewport.
//! Kept entities retain expansion/selection state; spacers retain exact geometry.
use gpui::{
    AnyView, Bounds, Context, InteractiveElement, IntoElement, ParentElement, Pixels, Render,
    ScrollHandle, Styled, Window, div, px,
};

const OVERSCAN: f32 = 400.;

pub struct HistoryRows {
    rows: Vec<AnyView>,
    scroll: ScrollHandle,
    measured: Vec<Bounds<Pixels>>,
    viewport_width: Pixels,
    scroll_width: Pixels,
    pub(crate) mounted: usize,
}

impl HistoryRows {
    pub fn new(rows: Vec<AnyView>, scroll: ScrollHandle) -> Self {
        Self {
            rows,
            scroll,
            measured: Vec::new(),
            viewport_width: px(0.),
            scroll_width: px(0.),
            mounted: 0,
        }
    }

    pub fn set_scroll(&mut self, scroll: ScrollHandle) {
        if scroll.bounds().size.height == px(0.) {
            self.measured.clear();
        }
        self.scroll = scroll;
    }

    pub fn set_rows(&mut self, rows: Vec<AnyView>, cx: &mut Context<Self>) {
        if self.rows.len() != rows.len()
            || self.rows.iter().zip(&rows).any(|(a, b)| a.entity_id() != b.entity_id())
        {
            self.measured.clear();
        }
        self.rows = rows;
        cx.notify();
    }

    fn visible(bounds: Bounds<Pixels>, scroll: &ScrollHandle) -> bool {
        let viewport = scroll.bounds();
        let offset = scroll.offset().y;
        bounds.bottom() + offset >= viewport.top() - px(OVERSCAN)
            && bounds.top() + offset <= viewport.bottom() + px(OVERSCAN)
    }
}

impl Render for HistoryRows {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = window.viewport_size().width;
        let scroll_width = self.scroll.bounds().size.width;
        let can_clip = self.viewport_width == width
            && self.scroll_width == scroll_width
            && self.measured.len() == self.rows.len()
            && self.scroll.bounds().size.height > px(0.);
        let mut mounted = Vec::with_capacity(self.rows.len());
        let mut body = div().w_full().min_w_0().flex().flex_col().gap_2();
        self.mounted = 0;
        for (index, row) in self.rows.iter().enumerate() {
            let visible = !can_clip || Self::visible(self.measured[index], &self.scroll);
            mounted.push(visible);
            if visible {
                self.mounted += 1;
                body = body.child(
                    div()
                        .id(("history-row", index))
                        .debug_selector(move || format!("history-row-{index}").into())
                        .w_full()
                        .min_w_0()
                        .flex_shrink_0()
                        .child(row.clone()),
                );
            } else {
                body = body.child(
                    div()
                        .id(("history-row", index))
                        .debug_selector(move || format!("history-spacer-{index}").into())
                        .w_full()
                        .h(self.measured[index].size.height)
                        .flex_shrink_0(),
                );
            }
        }
        let weak = cx.entity().downgrade();
        body.on_children_prepainted(move |mut bounds, _, cx| {
            let _ = weak.update(cx, |this, cx| {
                // Layout bounds include the ancestor scroll translation. Normalize
                // them so the next frame can use the latest offset, including jumps.
                let offset = this.scroll.offset().y;
                for row in &mut bounds {
                    row.origin.y -= offset;
                }
                let needs_refresh = !can_clip
                    || bounds
                        .iter()
                        .zip(&mounted)
                        .any(|(bounds, mounted)| Self::visible(*bounds, &this.scroll) != *mounted);
                this.measured = bounds;
                this.viewport_width = width;
                this.scroll_width = scroll_width;
                if needs_refresh && this.scroll.bounds().size.height > px(0.) {
                    let id = cx.entity_id();
                    cx.defer(move |cx| cx.notify(id));
                }
            });
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        AppContext, Entity, InteractiveElement, StatefulInteractiveElement, TestAppContext, point,
        size,
    };
    use std::{cell::Cell, rc::Rc};

    struct Row {
        height: f32,
        renders: Rc<Cell<usize>>,
    }
    impl Render for Row {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.renders.set(self.renders.get() + 1);
            div().h(px(self.height)).w_full().child("record")
        }
    }
    struct Preview {
        scroll: ScrollHandle,
        rows: Entity<HistoryRows>,
    }
    impl Render for Preview {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("history-rows-test")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child(self.rows.clone())
        }
    }

    #[gpui::test]
    fn long_process_mounts_only_nearby_rows_and_preserves_height_and_expansion(
        cx: &mut TestAppContext,
    ) {
        let renders = Rc::new(Cell::new(0));
        let mut first = None;
        let (preview, window) = cx.add_window_view(|_, cx| {
            let scroll = ScrollHandle::new();
            let rows = (0..500)
                .map(|index| {
                    let row = cx.new(|_| Row {
                        height: 30. + (index % 3) as f32 * 10.,
                        renders: renders.clone(),
                    });
                    if index == 0 {
                        first = Some(row.clone());
                    }
                    row.into()
                })
                .collect();
            Preview {
                rows: cx.new(|_| HistoryRows::new(rows, scroll.clone())),
                scroll,
            }
        });
        window.simulate_resize(size(px(800.), px(600.)));
        window.run_until_parked();
        let scroll = preview.read_with(window, |p, _| p.scroll.clone());
        let rows = preview.read_with(window, |p, _| p.rows.clone());
        let height = scroll.max_offset().y;
        assert!(height > px(20000.));
        rows.read_with(window, |r, _| {
            assert!(r.mounted < 40, "mounted {} of 500", r.mounted)
        });
        renders.set(0);
        preview.update(window, |_, cx| cx.notify());
        window.run_until_parked();
        assert!(
            renders.get() < 40,
            "offscreen rows must not render: {}",
            renders.get()
        );
        assert_eq!(scroll.max_offset().y, height);
        preview.update(window, |_, cx| {
            scroll.scroll_to_bottom();
            cx.notify();
        });
        window.run_until_parked();
        assert_eq!(scroll.max_offset().y, height);
        rows.read_with(window, |r, _| {
            assert!(r.mounted < 40);
            assert!(!HistoryRows::visible(r.measured[0], &scroll));
            assert!(HistoryRows::visible(*r.measured.last().unwrap(), &scroll));
        });
        preview.update(window, |_, cx| {
            scroll.set_offset(point(px(0.), px(0.)));
            cx.notify();
        });
        window.run_until_parked();
        rows.read_with(window, |r, _| {
            assert!(HistoryRows::visible(r.measured[0], &scroll))
        });
        first.unwrap().update(window, |row, cx| {
            row.height += 200.;
            cx.notify();
        });
        window.run_until_parked();
        assert_eq!(scroll.max_offset().y, height + px(200.));
        window.simulate_resize(size(px(600.), px(400.)));
        window.run_until_parked();
        rows.read_with(window, |r, _| assert!(r.mounted < 40));
        assert_eq!(scroll.max_offset().y, height + px(400.));
    }
}
