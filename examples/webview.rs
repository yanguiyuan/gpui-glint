#[cfg(any(target_os = "macos", target_os = "windows"))]
mod demo {
    use gpui::prelude::FluentBuilder;
    use gpui::{
        App, AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Render,
        StatefulInteractiveElement, Styled, Window, WindowBounds, WindowOptions, div, px, size,
    };
    use gpui_glint::components::{Button, CodeCard, GlintAssets, WebView, bind_code_card_keys};
    const HTML: &str = r#"<!doctype html>
<html lang="zh-CN">
<head><meta charset="utf-8"><style>
body { margin: 0; padding: 40px; font: 16px system-ui; color: #233047; background: #f3f7fb; }
article { max-width: 640px; padding: 28px; background: white; border: 1px solid #dce3eb; border-radius: 16px; }
h1 { margin-top: 0; } p { line-height: 1.8; } strong { color: #0874e8; }
</style></head>
<body><article><h1>Glint WebView</h1><p>这段 HTML 由 <strong>系统 WebView</strong> 渲染。</p><p>左侧是 CodeCard 的 HTML 源码高亮；调整窗口大小可以验证原生视图的布局同步。</p></article></body>
</html>"#;
    struct Demo {
        source: Entity<CodeCard>,
        browser: Entity<WebView>,
        visible: bool,
    }
    impl Render for Demo {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .bg(gpui::rgb(0xffffff))
                .child(
                    div()
                        .h(px(56.))
                        .px_4()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child("HTML / WebView")
                        .child(
                            Button::new(if self.visible {
                                "隐藏预览"
                            } else {
                                "显示预览"
                            })
                            .render()
                            .on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.visible = !this.visible;
                                    this.browser
                                        .update(cx, |view, cx| view.set_visible(this.visible, cx))
                                        .expect("无法切换 WebView");
                                    cx.notify();
                                },
                            )),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .child(
                            div()
                                .w(px(420.))
                                .h_full()
                                .overflow_hidden()
                                .child(self.source.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .h_full()
                                .when(self.visible, |el| el.child(self.browser.clone())),
                        ),
                )
        }
    }
    pub fn run() {
        gpui_platform::application()
            .with_assets(GlintAssets)
            .run(|cx: &mut App| {
                bind_code_card_keys(cx);
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None,
                            size(px(1120.), px(720.)),
                            cx,
                        ))),
                        ..Default::default()
                    },
                    |window, cx| {
                        cx.new(|cx| Demo {
                            source: cx.new(|cx| CodeCard::new(cx, "html", HTML).read_only(true)),
                            browser: cx.new(|cx| {
                                WebView::html(HTML, window, cx).expect("无法创建原生 WebView")
                            }),
                            visible: true,
                        })
                    },
                )
                .expect("无法打开 WebView 示例窗口");
                cx.activate(true);
            });
    }
}
fn main() {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    demo::run();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    eprintln!("Native WebView integration currently supports macOS and Windows only.");
}
