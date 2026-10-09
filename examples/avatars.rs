//! Run with `cargo run --example avatars`.
use gpui::{
    App, AppContext, Bounds, Context, IntoElement, ParentElement, Render, RenderImage, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_glint::{
    components::{Avatar, AvatarGroup, AvatarShape, GlintAssets},
    theme::ThemePreset,
};
use image::{Frame, RgbaImage};
use std::{path::PathBuf, sync::Arc};

// A small native image fixture: no network or external image files needed.
fn portrait() -> Arc<RenderImage> {
    let pixels = RgbaImage::from_fn(80, 80, |x, y| {
        let head = (x as i32 - 40).pow(2) + (y as i32 - 28).pow(2) < 13 * 13;
        let body = (x as i32 - 40).pow(2) + (y as i32 - 79).pow(2) < 30 * 30;
        image::Rgba(if head || body {
            [210, 132, 47, 255]
        } else {
            [246, 233, 215, 255]
        })
    });
    Arc::new(RenderImage::new(vec![Frame::new(pixels)]))
}
struct AvatarExample {
    portrait: Arc<RenderImage>,
}
impl Render for AvatarExample {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let p = ThemePreset::Sage.palette();
        let names = [
            "林晓雨",
            "Ada Lovelace",
            "陈晨",
            "Grace Hopper",
            "周舟",
            "Alan Turing",
            "吴桐",
            "李悦",
        ];
        div()
            .size_full()
            .p_8()
            .flex()
            .flex_col()
            .gap_6()
            .bg(rgb(p.canvas))
            .text_color(rgb(p.text))
            .child(div().text_size(px(24.)).child("头像与头像组"))
            .child("图片、姓名缩写、自定义文字，以及图片加载失败后的回退。")
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_5()
                    .child(Avatar::new("林晓雨").image(self.portrait.clone()).size(56.))
                    .child(Avatar::new("Ada Lovelace").size(48.))
                    .child(
                        Avatar::new("研发团队")
                            .text("研发")
                            .shape(AvatarShape::Rectangle)
                            .dimensions(72., 48.),
                    )
                    .child(
                        Avatar::new("陈晨")
                            .image(PathBuf::from("/nonexistent/glass-avatar-example.png")),
                    )
                    .child(Avatar::new("👩🏽‍💻团队").size(48.))
                    .child(Avatar::new("")),
            )
            .child("圆形头像组 · 重叠排列 · 显示五人，剩余 +3")
            .child(
                AvatarGroup::new(names.map(Avatar::new))
                    .size(48.)
                    .ring_color(p.canvas),
            )
            .child("矩形头像组 · 显示四人，剩余 +4")
            .child(
                AvatarGroup::new(names.map(Avatar::new))
                    .shape(AvatarShape::Rectangle)
                    .size(48.)
                    .max_visible(4)
                    .ring_color(p.canvas),
            )
            .child("无重叠排列")
            .child(
                AvatarGroup::new(names.into_iter().take(3).map(Avatar::new))
                    .overlap(0.)
                    .ring_color(p.canvas),
            )
    }
}
fn main() {
    gpui_platform::application()
        .with_assets(GlintAssets)
        .run(|cx: &mut App| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(920.), px(620.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| AvatarExample {
                        portrait: portrait(),
                    })
                },
            )
            .unwrap();
            cx.activate(true);
        });
}
