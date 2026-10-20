use gpui::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div, px,
};
use gpui_glint::components::{DataTable, TableCell, TableColumn, TableRow};
use std::{cell::Cell, rc::Rc, time::Instant};

struct View(Entity<DataTable>);
impl Render for View {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().flex().flex_col().child(self.0.clone())
    }
}

// CPU layout/render benchmark on the test platform, not GPU frame-rate measurement.
#[gpui::test]
#[ignore = "manual bounded-table scrolling benchmark"]
fn bounded_table_scroll_cpu(cx: &mut gpui::TestAppContext) {
    for count in [442, 2_000, 10_000] {
        let calls = Rc::new(Cell::new(0usize));
        let table = cx.new(|cx| {
            DataTable::new(
                cx,
                [TableColumn::new("域名", 180.).flexible()],
                (0..count).map(|index| {
                    let calls = calls.clone();
                    TableRow::new(
                        format!("row-{index}"),
                        [TableCell::custom(move || {
                            calls.set(calls.get() + 1);
                            div().h(px(32.)).child(format!("host-{index}.test"))
                        })],
                    )
                }),
            )
            .document()
            .compact()
            .max_height(420.)
        });
        let start = Instant::now();
        let (_, window) = cx.add_window_view(|_, _| View(table.clone()));
        let initial_ms = start.elapsed().as_secs_f64() * 1000.;
        let initial_calls = calls.replace(0);
        let viewport = window.debug_bounds("table-body-scroll").unwrap();
        calls.set(0);
        let mut timings = Vec::new();
        for _ in 0..20 {
            let start = Instant::now();
            window.simulate_event(gpui::ScrollWheelEvent {
                position: viewport.center(),
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-40.))),
                ..Default::default()
            });
            timings.push(start.elapsed().as_secs_f64() * 1000.);
        }
        timings.sort_by(f64::total_cmp);
        eprintln!(
            "rows={count} initial_ms={initial_ms:.2} initial_cells={initial_calls} scroll_median_ms={:.2} scroll_p95_ms={:.2} scroll_cells={}",
            timings[10],
            timings[18],
            calls.get()
        );
    }
}
