use super::Icon;
use crate::theme::ThemePreset;
use gpui::{
    Div, FontWeight, ImageSource, IntoElement, ObjectFit, ParentElement, SharedString, Styled,
    StyledImage, div, img, px, rgb,
};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AvatarShape {
    #[default]
    Circle,
    Rectangle,
}

/// Picture or text avatar. The name supplies initials when no explicit text is provided.
/// Image loading and failure both fall back to the same text; an empty name uses a user icon.
#[derive(Clone)]
pub struct Avatar {
    name: SharedString,
    text: Option<SharedString>,
    image: Option<ImageSource>,
    shape: AvatarShape,
    width: f32,
    height: f32,
    theme: ThemePreset,
    colors: Option<(u32, u32)>,
    text_size: Option<f32>,
    ring: Option<u32>,
}
impl Avatar {
    pub fn new(name: impl Into<SharedString>) -> Self {
        Self {
            name: name.into(),
            text: None,
            image: None,
            shape: AvatarShape::Circle,
            width: 40.,
            height: 40.,
            theme: ThemePreset::default(),
            colors: None,
            text_size: None,
            ring: None,
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Explicit avatar text, e.g. "研发", "AB", or an emoji.
    pub fn text(mut self, text: impl Into<SharedString>) -> Self {
        self.text = Some(text.into());
        self
    }
    /// Accepts GPUI ImageSource: URL, PathBuf, embedded asset, or a decoded image.
    pub fn image(mut self, source: impl Into<ImageSource>) -> Self {
        self.image = Some(source.into());
        self
    }
    pub fn shape(mut self, shape: AvatarShape) -> Self {
        self.shape = shape;
        self
    }
    pub fn size(mut self, size: f32) -> Self {
        let size = dimension(size);
        self.width = size;
        self.height = size;
        self
    }
    /// Rectangles can be wider than tall; circles use the smaller dimension as their diameter.
    pub fn dimensions(mut self, width: f32, height: f32) -> Self {
        self.width = dimension(width);
        self.height = dimension(height);
        self
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn colors(mut self, background: u32, foreground: u32) -> Self {
        self.colors = Some((background, foreground));
        self
    }
    pub fn text_size(mut self, size: f32) -> Self {
        self.text_size = Some(if size.is_finite() { size.max(6.) } else { 14. });
        self
    }
    pub fn ring(mut self, color: u32) -> Self {
        self.ring = Some(color);
        self
    }
    pub fn render(self) -> Div {
        let p = self.theme.palette();
        let (background, foreground) = self.colors.unwrap_or((p.accent_tint, p.accent));
        let (width, height) = match self.shape {
            AvatarShape::Circle => {
                let side = self.width.min(self.height);
                (side, side)
            }
            AvatarShape::Rectangle => (self.width, self.height),
        };
        let side = width.min(height);
        let radius = match self.shape {
            AvatarShape::Circle => side / 2.,
            AvatarShape::Rectangle => side * 0.2,
        };
        let label: SharedString = self.text.unwrap_or_else(|| initials(&self.name).into());
        let text_len = label.graphemes(true).count().max(1) as f32;
        let font_size = self
            .text_size
            .unwrap_or(side * 0.36)
            .min((width - 8.) / text_len)
            .max(6.);
        let inner_radius = (radius - if self.ring.is_some() { 2. } else { 0. }).max(0.);
        let fallback = move || {
            let mut el = div()
                .size_full()
                .rounded(px(inner_radius))
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .bg(rgb(background))
                .text_color(rgb(foreground))
                .text_size(px(font_size))
                .font_weight(FontWeight::MEDIUM);
            if label.is_empty() {
                el = el.child(Icon::User.render(side * 0.48));
            } else {
                el = el.child(label.clone());
            }
            el
        };
        let mut root = div()
            .w(px(width))
            .h(px(height))
            .flex_shrink_0()
            .rounded(px(radius))
            .overflow_hidden();
        if let Some(color) = self.ring {
            root = root.border_2().border_color(rgb(color));
        }
        if let Some(source) = self.image {
            let loading = fallback.clone();
            root.child(
                img(source)
                    .size_full()
                    .rounded(px(inner_radius))
                    .object_fit(ObjectFit::Cover)
                    .with_loading(move || loading().into_any_element())
                    .with_fallback(move || fallback().into_any_element()),
            )
        } else {
            root.child(fallback())
        }
    }
}
impl IntoElement for Avatar {
    type Element = Div;
    fn into_element(self) -> Div {
        self.render()
    }
}

fn dimension(value: f32) -> f32 {
    if value.is_finite() {
        value.max(16.)
    } else {
        40.
    }
}
fn initials(name: &str) -> String {
    let words: Vec<_> = name.split_whitespace().collect();
    let Some(first) = words.first() else {
        return String::new();
    };
    if words.len() > 1 {
        format!(
            "{}{}",
            first.graphemes(true).next().unwrap_or(""),
            words.last().unwrap().graphemes(true).next().unwrap_or("")
        )
        .to_uppercase()
    } else {
        first
            .graphemes(true)
            .take(2)
            .collect::<String>()
            .to_uppercase()
    }
}

/// Overlapping avatar stack. Later entries paint above earlier ones; rings keep each edge visible.
/// max_visible limits members, and a final +N avatar indicates additional members.
pub struct AvatarGroup {
    avatars: Vec<Avatar>,
    shape: AvatarShape,
    size: f32,
    overlap: Option<f32>,
    max_visible: usize,
    ring_color: Option<u32>,
    theme: ThemePreset,
}
impl AvatarGroup {
    pub fn new(avatars: impl IntoIterator<Item = Avatar>) -> Self {
        Self {
            avatars: avatars.into_iter().collect(),
            shape: AvatarShape::Circle,
            size: 40.,
            overlap: None,
            max_visible: 5,
            ring_color: None,
            theme: ThemePreset::default(),
        }
    }
    pub fn shape(mut self, shape: AvatarShape) -> Self {
        self.shape = shape;
        self
    }
    pub fn size(mut self, size: f32) -> Self {
        self.size = dimension(size);
        self
    }
    /// Horizontal overlap in pixels; 0 disables overlap. Defaults to 25% of the avatar size.
    pub fn overlap(mut self, pixels: f32) -> Self {
        self.overlap = Some(if pixels.is_finite() {
            pixels.max(0.)
        } else {
            0.
        });
        self
    }
    /// Match the surface behind the group to make overlapping edges distinct.
    pub fn ring_color(mut self, color: u32) -> Self {
        self.ring_color = Some(color);
        self
    }
    pub fn max_visible(mut self, count: usize) -> Self {
        self.max_visible = count;
        self
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn render(self) -> Div {
        let p = self.theme.palette();
        let ring_color = self.ring_color.unwrap_or(p.surface);
        let count = self.max_visible.min(self.avatars.len());
        let hidden = self.avatars.len() - count;
        // Keep at least half of every member visible, even for an excessive requested overlap.
        let overlap = self
            .overlap
            .unwrap_or(self.size * 0.25)
            .min(self.size * 0.5);
        let slots = count + usize::from(hidden > 0);
        // Negative margins implement overlap, but intrinsic flex measurement can
        // undercount the stack. Reserve its actual painted width for siblings.
        let width = if slots == 0 {
            0.
        } else {
            self.size + (slots - 1) as f32 * (self.size - overlap)
        };
        let mut row = div()
            .w(px(width))
            .h(px(self.size))
            .flex()
            .items_center()
            .flex_shrink_0();
        for (index, mut avatar) in self.avatars.into_iter().take(count).enumerate() {
            if avatar.text_size.is_none() {
                avatar.text_size = Some(self.size * 0.25);
            }
            row = row.child(
                div()
                    .flex_shrink_0()
                    .ml(px(if index == 0 { 0. } else { -overlap }))
                    .child(
                        avatar
                            .size(self.size)
                            .shape(self.shape)
                            .theme(self.theme)
                            .ring(ring_color),
                    ),
            );
        }
        if hidden > 0 {
            row = row.child(
                div()
                    .flex_shrink_0()
                    .ml(px(if count == 0 { 0. } else { -overlap }))
                    .child(
                        Avatar::new("")
                            .text(format!("+{hidden}"))
                            .size(self.size)
                            .shape(self.shape)
                            .theme(self.theme)
                            .colors(p.surface_subtle, p.text_muted)
                            .ring(ring_color),
                    ),
            );
        }
        row
    }
}
impl IntoElement for AvatarGroup {
    type Element = Div;
    fn into_element(self) -> Div {
        self.render()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initials_preserve_unicode_graphemes_and_names() {
        assert_eq!(initials("  Ada Lovelace "), "AL");
        assert_eq!(initials("林晓雨"), "林晓");
        assert_eq!(initials("alice"), "AL");
        assert_eq!(initials("👩🏽‍💻团队"), "👩🏽‍💻团");
        assert_eq!(initials("e\u{301}mile"), "E\u{301}M");
        assert_eq!(initials("   "), "");
    }
}
