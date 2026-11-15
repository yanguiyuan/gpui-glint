//! cargo run --example video_player --features video -- /path/to/video.mp4
#[cfg(any(target_os = "macos", target_os = "windows"))]
#[path = "playground/pages/video.rs"]
mod video;
fn main() {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
        use gpui_glint::{
            components::{GlintAssets, VideoSource},
            theme::ThemePreset,
        };
        gpui_platform::application()
            .with_assets(GlintAssets)
            .run(|cx: &mut App| {
                let source = std::env::args().nth(1);
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None,
                            size(px(1024.), px(720.)),
                            cx,
                        ))),
                        ..Default::default()
                    },
                    move |window, cx| {
                        cx.new(|cx| {
                            let mut page =
                                video::VideoPage::new(window, ThemePreset::default(), cx);
                            if let Some(source) = source {
                                page.load(
                                    if source.starts_with("http://")
                                        || source.starts_with("https://")
                                    {
                                        VideoSource::url(source)
                                    } else {
                                        VideoSource::file(source)
                                    },
                                    cx,
                                );
                            }
                            page
                        })
                    },
                )
                .unwrap();
                cx.activate(true);
            });
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    eprintln!("VideoPlayer currently requires macOS or Windows.");
}
