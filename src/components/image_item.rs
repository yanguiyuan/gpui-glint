use gpui::{ImageSource, SharedString};

/// A page shared by the single-image viewer and continuous image stream.
#[derive(Clone)]
pub struct ImageItem {
    pub id: SharedString,
    pub source: ImageSource,
    pub dimensions: Option<(f32, f32)>,
}
impl ImageItem {
    pub fn new(id: impl Into<SharedString>, source: impl Into<ImageSource>) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            dimensions: None,
        }
    }
    pub fn dimensions(mut self, width: u32, height: u32) -> Self {
        if width > 0 && height > 0 {
            self.dimensions = Some((width as f32, height as f32));
        }
        self
    }
}
