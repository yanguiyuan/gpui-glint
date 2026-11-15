//! Offline masonry gallery: cargo run --example image_masonry
#[path = "playground/pages/masonry.rs"]
mod masonry;
use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_glint::{components::GlintAssets, theme::ThemePreset};
fn main() {
    gpui_platform::application()
        .with_assets(GlintAssets)
        .run(|cx: &mut App| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1000.), px(760.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |_, cx| cx.new(|cx| masonry::MasonryPage::new(ThemePreset::default(), cx)),
            )
            .unwrap();
            cx.activate(true);
        });
}
