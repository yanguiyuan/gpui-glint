//! Controlled, single-image viewing surface. Navigation and transforms belong to the host.
use super::ImageItem;
use crate::theme::ThemePreset;
use gpui::{
    AnyElement, ImageSource, InteractiveElement, IntoElement, ObjectFit, ParentElement, Styled,
    StyledImage, div, img, px, rgb,
};

pub struct ImageViewer {
    source: ImageSource,
    theme: ThemePreset,
    padding: f32,
    toolbar: Option<AnyElement>,
}
impl ImageViewer {
    pub fn new(source: impl Into<ImageSource>) -> Self {
        Self {
            source: source.into(),
            theme: ThemePreset::default(),
            padding: 0.,
            toolbar: None,
        }
    }
    pub fn from_item(item: ImageItem) -> Self {
        Self::new(item.source)
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn padding(mut self, padding: f32) -> Self {
        if padding.is_finite() {
            self.padding = padding.max(0.);
        }
        self
    }
    /// Optional host-supplied navigation or transform controls.
    pub fn toolbar(mut self, toolbar: impl IntoElement) -> Self {
        self.toolbar = Some(toolbar.into_any_element());
        self
    }
    pub fn render(self) -> AnyElement {
        let palette = self.theme.palette();
        let image = div()
            .size_full()
            .relative()
            .overflow_hidden()
            .debug_selector(|| "image-viewer-image".into())
            .child(
                img(self.source)
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .debug_selector(|| "image-viewer-raster".into())
                    .with_loading(move || note("正在加载图片…", palette.text_muted))
                    .with_fallback(move || note("图片加载失败", palette.text_muted)),
            )
            .into_any_element();
        div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .children(self.toolbar)
            .child(Self::stage(image, self.padding))
            .into_any_element()
    }
    /// A bounded stage also usable for loading, empty and error states.
    pub fn stage(content: AnyElement, padding: f32) -> AnyElement {
        let padding = if padding.is_finite() {
            padding.max(0.)
        } else {
            0.
        };
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .w_full()
            .relative()
            .overflow_hidden()
            .debug_selector(|| "image-viewer-stage".into())
            .child(
                div()
                    .absolute()
                    .top(px(padding))
                    .bottom(px(padding))
                    .left(px(padding))
                    .right(px(padding))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(content),
            )
            .into_any_element()
    }
}
fn note(message: &'static str, color: u32) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(12.))
        .text_color(rgb(color))
        .child(message)
        .into_any_element()
}
