use crate::theme::ThemePreset;
use gpui::{Div, ObjectFit, ParentElement, RenderImage, Styled, StyledImage, div, img};
use image::{Frame, RgbaImage};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};

static CACHE: LazyLock<Mutex<HashMap<&'static str, Arc<RenderImage>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Soft wallpaper behind the translucent dark materials. The artwork is rendered
/// once per preset; controls remain sharp and opaque over the blurred color fields.
pub(crate) fn wallpaper(theme: ThemePreset) -> Div {
    let mut cache = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    let image = cache.entry(theme.id()).or_insert_with(|| render(theme));
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .overflow_hidden()
        .child(img(image.clone()).size_full().object_fit(ObjectFit::Fill))
}

fn render(theme: ThemePreset) -> Arc<RenderImage> {
    const WIDTH: u32 = 1800;
    const HEIGHT: u32 = 1200;
    let channels = |color: u32| {
        [
            ((color >> 16) & 255) as f32,
            ((color >> 8) & 255) as f32,
            (color & 255) as f32,
        ]
    };
    let base = channels(theme.palette().canvas);
    let colors = theme.wallpaper_colors().map(channels);
    // HTML's wallpaper extends 25% beyond each edge. Keep the glow centers
    // near/outside those edges rather than drawing four closed ellipses inside.
    let fields = [
        (-0.01, 0.05, 0.22, 0.24),
        (0.98, -0.04, 0.20, 0.25),
        (0.86, 0.98, 0.25, 0.28),
        (0.05, 1.04, 0.21, 0.22),
    ];
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let mut color = base;
            for (index, &(cx, cy, sx, sy)) in fields.iter().enumerate().rev() {
                let dx = (x as f32 / WIDTH as f32 - cx) / sx;
                let dy = (y as f32 / HEIGHT as f32 - cy) / sy;
                let alpha = (-0.5 * (dx * dx + dy * dy)).exp();
                for channel in 0..3 {
                    color[channel] += (colors[index][channel] - color[channel]) * alpha;
                }
            }
            let luminance = color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
            let mut hash = y * WIDTH + x;
            hash ^= hash >> 16;
            hash = hash.wrapping_mul(0x7feb352d);
            hash ^= hash >> 15;
            hash = hash.wrapping_mul(0x846ca68b);
            hash ^= hash >> 16;
            let dither = hash as f64 / (u32::MAX as f64 + 1.);
            let quantize = |channel: f32| {
                // Dither BEFORE 8-bit quantization. Adding noise after an SVG
                // has already quantized its gradients leaves circular contours.
                let value = luminance + (channel - luminance) * 1.15;
                (value as f64 + dither).floor().clamp(0., 255.) as u8
            };
            // GPUI RenderImage uses BGRA.
            pixels.extend([
                quantize(color[2]),
                quantize(color[1]),
                quantize(color[0]),
                255,
            ]);
        }
    }
    let image = RgbaImage::from_raw(WIDTH, HEIGHT, pixels).expect("wallpaper BGRA buffer");
    Arc::new(RenderImage::new(vec![Frame::new(image)]))
}
