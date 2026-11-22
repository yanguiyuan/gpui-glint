//! Full-window image lightbox viewer with zoom, pan, gallery navigation, and copy/save actions.

use super::Icon;
use gpui::prelude::FluentBuilder;
use gpui::{
    Context, CursorStyle, EventEmitter, FocusHandle, FontWeight, Image, ImageFormat,
    InteractiveElement, IntoElement, MouseButton, ObjectFit, ParentElement, Pixels, Point, Render,
    StatefulInteractiveElement, Styled, StyledImage, Window, deferred, div, img, point, px, rgb, rgba,
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum ImageLightboxEvent {
    CloseRequested,
    CopyRequested(Arc<Image>),
    SaveRequested(Arc<Image>, usize),
}

pub struct ImageLightbox {
    images: Vec<Arc<Image>>,
    index: usize,
    dimensions: Option<(u32, u32)>,
    zoom: f32,
    pan: Point<f32>,
    dragging: Option<(Point<Pixels>, Point<f32>)>,
    origin: Option<FocusHandle>,
    focus: FocusHandle,
}

impl EventEmitter<ImageLightboxEvent> for ImageLightbox {}

impl ImageLightbox {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            images: Vec::new(),
            index: 0,
            dimensions: None,
            zoom: 1.0,
            pan: point(0., 0.),
            dragging: None,
            origin: None,
            focus: cx.focus_handle(),
        }
    }

    pub fn is_open(&self) -> bool {
        !self.images.is_empty()
    }

    pub fn current(&self) -> Option<&Arc<Image>> {
        self.images.get(self.index)
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn open(
        &mut self,
        images: Vec<Arc<Image>>,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if images.is_empty() {
            return;
        }
        let index = index.min(images.len() - 1);
        self.dimensions = probe_image_dimensions(&images[index]);
        self.images = images;
        self.index = index;
        self.zoom = 1.0;
        self.pan = point(0., 0.);
        self.dragging = None;
        self.origin = window.focused(cx);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub fn close(&mut self, window: Option<&mut Window>, cx: &mut Context<Self>) {
        if self.images.is_empty() {
            return;
        }
        self.images.clear();
        self.index = 0;
        self.dimensions = None;
        self.zoom = 1.0;
        self.pan = point(0., 0.);
        self.dragging = None;
        if let (Some(origin), Some(window)) = (self.origin.take(), window) {
            window.focus(&origin, cx);
        }
        cx.emit(ImageLightboxEvent::CloseRequested);
        cx.notify();
    }

    pub fn set_index(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.images.len() {
            self.index = index;
            self.dimensions = probe_image_dimensions(&self.images[index]);
            self.zoom = 1.0;
            self.pan = point(0., 0.);
            self.dragging = None;
            cx.notify();
        }
    }

    pub fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        self.zoom = zoom.clamp(0.25, 4.0);
        if (self.zoom - 1.0).abs() < 0.01 {
            self.zoom = 1.0;
            self.pan = point(0., 0.);
        }
        cx.notify();
    }
}

fn probe_image_dimensions(image: &Image) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(&image.bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

pub fn image_format_label(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "PNG",
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::Webp => "WEBP",
        ImageFormat::Gif => "GIF",
        ImageFormat::Svg => "SVG",
        ImageFormat::Bmp => "BMP",
        ImageFormat::Tiff => "TIFF",
        ImageFormat::Ico => "ICO",
        ImageFormat::Pnm => "PNM",
    }
}

pub fn image_format_ext(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
        ImageFormat::Gif => "gif",
        ImageFormat::Svg => "svg",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Tiff => "tiff",
        ImageFormat::Ico => "ico",
        ImageFormat::Pnm => "pnm",
    }
}

fn format_image_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

impl Render for ImageLightbox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(current) = self.images.get(self.index).cloned() else {
            return div().into_any_element();
        };

        let viewport = window.viewport_size();
        let maximum_width = (f32::from(viewport.width) - 96.).max(120.);
        let maximum_height = (f32::from(viewport.height) - 140.).max(90.);
        let total = self.images.len();
        let zoom_pct = (self.zoom * 100.0).round() as i32;
        let is_fit = (self.zoom - 1.0).abs() < 0.01;
        let format_label = image_format_label(current.format);
        let size_label = format_image_bytes(current.bytes.len());
        let dim_label = self.dimensions.map(|(w, h)| format!("{w} × {h}"));

        let image_element = if is_fit {
            img(current.clone())
                .max_w(px(maximum_width))
                .max_h(px(maximum_height))
                .object_fit(ObjectFit::Contain)
                .rounded(px(8.))
        } else if let Some((w, h)) = self.dimensions {
            let base_scale = (maximum_width / w as f32)
                .min(maximum_height / h as f32)
                .min(1.0);
            let target_w = w as f32 * base_scale * self.zoom;
            let target_h = h as f32 * base_scale * self.zoom;
            img(current.clone())
                .w(px(target_w))
                .h(px(target_h))
                .object_fit(ObjectFit::Contain)
                .rounded(px(8.))
        } else {
            img(current.clone())
                .max_w(px(maximum_width * self.zoom))
                .max_h(px(maximum_height * self.zoom))
                .object_fit(ObjectFit::Contain)
                .rounded(px(8.))
        };

        let glass_pill = |el: gpui::Stateful<gpui::Div>| {
            el.bg(rgba(0x161b24e8))
                .border_1()
                .border_color(rgba(0xffffff24))
                .text_color(rgb(0xf2f2f7))
                .shadow_lg()
        };

        let copy_source = current.clone();
        let save_source = current.clone();
        let save_index = self.index;

        deferred(
            div()
                .id("agent-image-lightbox")
                .debug_selector(|| "agent-image-lightbox".into())
                .absolute()
                .inset_0()
                .size_full()
                .bg(rgba(0x0b0f17e6))
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .track_focus(&self.focus)
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.close(Some(window), cx);
                        }
                        "left" if this.images.len() > 1 => {
                            let next = if this.index == 0 {
                                this.images.len() - 1
                            } else {
                                this.index - 1
                            };
                            this.set_index(next, cx);
                            cx.stop_propagation();
                        }
                        "right" if this.images.len() > 1 => {
                            let next = (this.index + 1) % this.images.len();
                            this.set_index(next, cx);
                            cx.stop_propagation();
                        }
                        "=" | "+" => {
                            this.set_zoom(this.zoom * 1.25, cx);
                            cx.stop_propagation();
                        }
                        "-" => {
                            this.set_zoom(this.zoom / 1.25, cx);
                            cx.stop_propagation();
                        }
                        "0" => {
                            this.set_zoom(1.0, cx);
                            cx.stop_propagation();
                        }
                        _ => {}
                    }
                }))
                .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                    cx.stop_propagation();
                    let dy = f32::from(event.delta.pixel_delta(px(20.)).y);
                    if dy.abs() > 0.1 {
                        let factor = (1.0 + dy * 0.004).clamp(0.8, 1.25);
                        this.set_zoom(this.zoom * factor, cx);
                    }
                }))
                .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                    if let Some((start_pos, start_pan)) = this.dragging {
                        if event.dragging() {
                            this.pan = point(
                                start_pan.x + f32::from(event.position.x - start_pos.x),
                                start_pan.y + f32::from(event.position.y - start_pos.y),
                            );
                            cx.notify();
                        } else {
                            this.dragging = None;
                        }
                        cx.stop_propagation();
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| {
                        this.dragging = None;
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| {
                        this.dragging = None;
                    }),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.close(Some(window), cx);
                    }),
                )
                .child(
                    div()
                        .id("agent-image-viewer")
                        .debug_selector(|| "agent-image-viewer".into())
                        .rounded(px(8.))
                        .border_1()
                        .border_color(rgba(0xffffff1a))
                        .shadow_lg()
                        .ml(px(self.pan.x))
                        .mt(px(self.pan.y))
                        .when(!is_fit, |d| d.cursor(CursorStyle::OpenHand))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                if (this.zoom - 1.0).abs() >= 0.01 {
                                    this.dragging = Some((event.position, this.pan));
                                    cx.notify();
                                }
                            }),
                        )
                        .child(image_element),
                )
                .child(
                    glass_pill(
                        div()
                            .id("agent-image-close")
                            .debug_selector(|| "agent-image-close".into())
                            .absolute()
                            .top(px(16.))
                            .right(px(16.))
                            .size(px(34.))
                            .rounded(px(10.))
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(rgba(0xffffff28))),
                    )
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.close(Some(window), cx);
                    }))
                    .child(Icon::Close.render(15.)),
                )
                .when(total > 1, |root| {
                    root.child(
                        glass_pill(
                            div()
                                .id("agent-image-prev")
                                .debug_selector(|| "agent-image-prev".into())
                                .absolute()
                                .left(px(16.))
                                .size(px(38.))
                                .rounded_full()
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(|s| s.bg(rgba(0xffffff28))),
                        )
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            let next = if this.index == 0 {
                                this.images.len() - 1
                            } else {
                                this.index - 1
                            };
                            this.set_index(next, cx);
                        }))
                        .child(div().text_size(px(22.)).pb(px(2.)).child("‹")),
                    )
                    .child(
                        glass_pill(
                            div()
                                .id("agent-image-next")
                                .debug_selector(|| "agent-image-next".into())
                                .absolute()
                                .right(px(16.))
                                .size(px(38.))
                                .rounded_full()
                                .cursor_pointer()
                                .flex()
                                .items_center()
                                .justify_center()
                                .hover(|s| s.bg(rgba(0xffffff28))),
                        )
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            let next = (this.index + 1) % this.images.len();
                            this.set_index(next, cx);
                        }))
                        .child(div().text_size(px(22.)).pb(px(2.)).child("›")),
                    )
                })
                .child(
                    glass_pill(
                        div()
                            .id("agent-image-toolbar")
                            .debug_selector(|| "agent-image-toolbar".into())
                            .absolute()
                            .bottom(px(20.))
                            .h(px(38.))
                            .px_3()
                            .rounded(px(12.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(12.)),
                    )
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .px(px(6.))
                            .py(px(2.))
                            .rounded(px(5.))
                            .bg(rgba(0xffffff16))
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format_label),
                    )
                    .when_some(dim_label, |bar, dims| {
                        bar.child(div().text_color(rgb(0xc8cbd2)).child(dims))
                    })
                    .child(div().text_color(rgb(0x9ea3ae)).child(size_label))
                    .when(total > 1, |bar| {
                        bar.child(
                            div()
                                .text_color(rgb(0xc8cbd2))
                                .child(format!("{} / {total}", self.index + 1)),
                        )
                    })
                    .child(div().w(px(1.)).h(px(14.)).bg(rgba(0xffffff24)))
                    .child(
                        div()
                            .id("agent-image-zoom-out")
                            .debug_selector(|| "agent-image-zoom-out".into())
                            .size(px(24.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(rgba(0xffffff1f)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.set_zoom(this.zoom / 1.25, cx);
                            }))
                            .child("−"),
                    )
                    .child(
                        div()
                            .id("agent-image-zoom-reset")
                            .debug_selector(|| "agent-image-zoom-reset".into())
                            .px(px(6.))
                            .h(px(24.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(rgba(0xffffff1f)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.set_zoom(1.0, cx);
                            }))
                            .child(if is_fit {
                                "适应".to_owned()
                            } else {
                                format!("{zoom_pct}%")
                            }),
                    )
                    .child(
                        div()
                            .id("agent-image-zoom-in")
                            .debug_selector(|| "agent-image-zoom-in".into())
                            .size(px(24.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(rgba(0xffffff1f)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.set_zoom(this.zoom * 1.25, cx);
                            }))
                            .child("+"),
                    )
                    .child(div().w(px(1.)).h(px(14.)).bg(rgba(0xffffff24)))
                    .child(
                        div()
                            .id("agent-image-copy")
                            .debug_selector(|| "agent-image-copy".into())
                            .px(px(8.))
                            .h(px(24.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .gap_1()
                            .hover(|s| s.bg(rgba(0xffffff1f)))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.stop_propagation();
                                cx.write_to_clipboard(gpui::ClipboardItem::new_image(&copy_source));
                                cx.emit(ImageLightboxEvent::CopyRequested(copy_source.clone()));
                            }))
                            .child(Icon::Copy.render(12.))
                            .child("复制"),
                    )
                    .child(
                        div()
                            .id("agent-image-save")
                            .debug_selector(|| "agent-image-save".into())
                            .px(px(8.))
                            .h(px(24.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .gap_1()
                            .hover(|s| s.bg(rgba(0xffffff1f)))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.stop_propagation();
                                cx.emit(ImageLightboxEvent::SaveRequested(
                                    save_source.clone(),
                                    save_index,
                                ));
                            }))
                            .child(Icon::Folder.render(12.))
                            .child("保存"),
                    ),
                ),
        )
        .with_priority(120)
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, size};

    #[gpui::test]
    fn lightbox_supports_zoom_gallery_navigation_and_close(cx: &mut TestAppContext) {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(300, 200, image::Rgba([40, 90, 160, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let img1 = Arc::new(Image::from_bytes(ImageFormat::Png, png.clone()));
        let img2 = Arc::new(Image::from_bytes(ImageFormat::Png, png));

        let (lightbox, window) = cx.add_window_view(|_, cx| ImageLightbox::new(cx));
        window.simulate_resize(size(px(1000.), px(760.)));
        window.run_until_parked();
        assert!(window.debug_bounds("agent-image-lightbox").is_none());

        lightbox.update_in(window, |lb, window, cx| {
            lb.open(vec![img1, img2], 0, window, cx);
        });
        window.run_until_parked();
        assert!(window.debug_bounds("agent-image-lightbox").is_some());
        assert!(window.debug_bounds("agent-image-toolbar").is_some());
        assert!(window.debug_bounds("agent-image-prev").is_some());
        assert!(window.debug_bounds("agent-image-next").is_some());

        // Click next arrow
        let next_btn = window.debug_bounds("agent-image-next").unwrap();
        window.simulate_click(next_btn.center(), Default::default());
        window.run_until_parked();
        assert_eq!(lightbox.read_with(window, |lb, _| lb.index()), 1);

        // Click zoom in
        let zoom_in = window.debug_bounds("agent-image-zoom-in").unwrap();
        window.simulate_click(zoom_in.center(), Default::default());
        window.run_until_parked();
        assert!(lightbox.read_with(window, |lb, _| lb.zoom() > 1.1));

        // Click zoom reset
        let zoom_reset = window.debug_bounds("agent-image-zoom-reset").unwrap();
        window.simulate_click(zoom_reset.center(), Default::default());
        window.run_until_parked();
        assert_eq!(lightbox.read_with(window, |lb, _| lb.zoom()), 1.0);
    }
}
