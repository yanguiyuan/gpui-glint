use gpui::{ScrollDelta, ScrollWheelEvent, TestAppContext, point, px};
use gpui_glint::components::DocumentEditor;
use std::time::Instant;

// CPU rendering benchmark with GPUI's test platform; excludes real GPU and display timing.
#[gpui::test]
#[ignore = "manual scrolling benchmark"]
fn document_scroll_cpu(cx: &mut TestAppContext) {
    for count in [20, 100, 300] {
        let markdown = (0..count).map(|i| format!("## 小节 {i}\n\n这是一段用于检查滚动性能的正文，包含 **粗体关键词**、[链接](https://example.com)，以及更多文字。"))
            .collect::<Vec<_>>().join("\n\n");
        let (document, window) = cx.add_window_view(|_, cx| DocumentEditor::new(cx, &markdown));
        for query in ["", "关键词"] {
            document.update(window, |editor, cx| editor.set_search_query(query, cx));
            window.run_until_parked();
            let mut timings = Vec::new();
            for _ in 0..20 {
                let start = Instant::now();
                window.simulate_event(ScrollWheelEvent {
                    position: point(px(400.), px(300.)),
                    delta: ScrollDelta::Pixels(point(px(0.), px(-30.))),
                    ..Default::default()
                });
                timings.push(start.elapsed().as_secs_f64() * 1000.);
            }
            timings.sort_by(f64::total_cmp);
            eprintln!(
                "blocks={} query={query:?} median_ms={:.2} p95_ms={:.2}",
                count * 2,
                timings[10],
                timings[18]
            );
        }
    }
}
