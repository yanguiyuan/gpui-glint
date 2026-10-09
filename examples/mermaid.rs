//! Run with `cargo run --example mermaid`.
use gpui::{
    App, AppContext, Bounds, Context, Entity, Render, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, size,
};
use gpui_glint::components::{DocumentEditor, GlintAssets, bind_document_editor_keys};

struct MermaidExample {
    document: Entity<DocumentEditor>,
}
impl Render for MermaidExample {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.document.clone())
    }
}
fn main() {
    gpui_platform::application()
        .with_assets(GlintAssets)
        .run(|cx: &mut App| {
            bind_document_editor_keys(cx);
            let bounds = Bounds::centered(None, size(px(1200.), px(900.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| MermaidExample {
                        document: cx.new(|cx| DocumentEditor::new(cx, include_str!("mermaid.md"))),
                    })
                },
            )
            .unwrap();
            cx.activate(true);
        });
}
