use gpui::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, RenderImage,
    StatefulInteractiveElement, Styled, Window, div, px, rgb,
};
use gpui_glint::{
    components::{
        Button, ButtonKind, ButtonSize, ImageMasonry, ImageMasonryEvent, ImageMasonryItem,
    },
    theme::ThemePreset,
};
use std::sync::Arc;

pub struct MasonryPage {
    gallery: Entity<ImageMasonry>,
    theme: ThemePreset,
    selected: String,
    dense: bool,
    large: bool,
    samples: Vec<ImageMasonryItem>,
}
impl MasonryPage {
    pub fn new(theme: ThemePreset, cx: &mut Context<Self>) -> Self {
        let gallery = cx.new(ImageMasonry::new);
        let samples = sample_images();
        gallery.update(cx, |gallery, cx| {
            gallery.set_theme(theme, cx);
            gallery.set_layout(230., 5, 18., cx);
            gallery.set_items(samples.clone(), cx);
        });
        cx.subscribe(&gallery, |this, _, event: &ImageMasonryEvent, cx| {
            let ImageMasonryEvent::Selected { index, .. } = event;
            this.selected = format!("已选择 · {}", sample_title(*index));
            cx.notify();
        })
        .detach();
        Self {
            gallery,
            theme,
            selected: "选择一张图片，开始收集灵感".into(),
            dense: false,
            large: false,
            samples,
        }
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        self.gallery
            .update(cx, |gallery, cx| gallery.set_theme(theme, cx));
        cx.notify();
    }
    fn set_large(&mut self, large: bool, cx: &mut Context<Self>) {
        self.large = large;
        let count = if large { 10_000 } else { self.samples.len() };
        let items = (0..count)
            .map(|index| {
                let mut item = self.samples[index % self.samples.len()].clone();
                item.id = format!("sample-{index}").into();
                item.title = Some(sample_title(index).into());
                item
            })
            .collect();
        self.gallery
            .update(cx, |gallery, cx| gallery.set_items(items, cx));
        self.selected = "选择一张图片，开始收集灵感".into();
        cx.notify();
    }
}
impl Render for MasonryPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let mut density = div().flex().items_center().gap_1();
        for (label, dense) in [("舒展", false), ("紧凑", true)] {
            density = density.child(
                Button::new(label)
                    .size(ButtonSize::Small)
                    .kind(if dense == self.dense {
                        ButtonKind::Tinted
                    } else {
                        ButtonKind::Plain
                    })
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dense = dense;
                        this.gallery.update(cx, |gallery, cx| {
                            gallery.set_layout(
                                if dense { 150. } else { 230. },
                                if dense { 8 } else { 5 },
                                if dense { 12. } else { 18. },
                                cx,
                            )
                        });
                        cx.notify();
                    })),
            );
        }
        let mut counts = div().flex().items_center().gap_1();
        for (label, large) in [("精选 24 张", false), ("10,000 张", true)] {
            counts = counts.child(
                Button::new(label)
                    .size(ButtonSize::Small)
                    .kind(if large == self.large {
                        ButtonKind::Tinted
                    } else {
                        ButtonKind::Plain
                    })
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| this.set_large(large, cx))),
            );
        }
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .p_6()
            .gap_4()
            .bg(rgb(p.canvas))
            .text_color(rgb(p.text))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(24.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("色彩采样"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(p.text_muted))
                            .child("光影、地景与材质之间的配色灵感。"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .pb_3()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .child(counts)
                    .child(density),
            )
            .child(div().flex_1().min_h_0().child(self.gallery.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .pt_2()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(self.selected.clone()),
                    )
                    .child("离线色彩档案"),
            )
    }
}

fn sample_title(index: usize) -> String {
    let names = [
        "潮汐的留白",
        "晒暖的陶土",
        "远山入暮",
        "林间薄雾",
        "沙丘漫步",
        "静水微光",
    ];
    format!("{} · {:02}", names[index % names.len()], index + 1)
}

fn sample_images() -> Vec<ImageMasonryItem> {
    let colors = [
        (0x245f69_u32, 0xb5d5c5_u32),
        (0xb86440, 0xf0cea2),
        (0x494d81, 0xc4bfdc),
        (0x496952, 0xd3dcb6),
        (0x9d784d, 0xf0dfb7),
        (0x416b83, 0xc2d8dd),
    ];
    (0..24)
        .map(|index| {
            let width = 480;
            let height = [600, 400, 480, 680, 360][index % 5];
            let (dark, light) = colors[index % colors.len()];
            let pixels = image::RgbaImage::from_fn(width, height, |x, y| {
                let nx = x as f32 / width as f32;
                let ny = y as f32 / height as f32;
                let phase = index as f32 * 0.7;
                let horizon = 0.58 + (nx * 4. + phase).sin() * 0.085;
                let foreground = 0.83 + (nx * 5. + phase + 1.).sin() * 0.07;
                let distance = ((x as f32 - width as f32 * 0.67).powi(2)
                    + (y as f32 - height as f32 * 0.28).powi(2))
                .sqrt();
                let sun = ((width as f32 * 0.115 - distance) / 1.5 + 0.5).clamp(0., 1.);
                let sky = 0.42 + ny * 0.36;
                let terrain = if ny > foreground {
                    0.08 + nx * 0.18
                } else if ny > horizon {
                    0.22 + nx * 0.16
                } else {
                    sky + (0.96 - sky) * sun
                };
                let channel = |shift: u32| {
                    let a = ((dark >> shift) & 255u32) as f32;
                    let b = ((light >> shift) & 255u32) as f32;
                    (a + (b - a) * terrain) as u8
                };
                image::Rgba([channel(0), channel(8), channel(16), 255])
            });
            ImageMasonryItem::new(
                format!("sample-{index}"),
                Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])),
                width,
                height,
            )
            .title(sample_title(index))
        })
        .collect()
}
