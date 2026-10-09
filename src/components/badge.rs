use crate::theme::ThemePreset;
use gpui::{IntoElement, ParentElement, SharedString, Styled, div, px, rgb};

use super::Icon;

#[derive(Clone, Copy)]
pub enum ContentType {
    Video,
    ImageGroup,
    Text,
    Link,
    Document,
}

impl ContentType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Video => "视频",
            Self::ImageGroup => "图集",
            Self::Text => "文本",
            Self::Link => "链接",
            Self::Document => "文档",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::Video => Icon::Video,
            Self::ImageGroup => Icon::ImageGroup,
            Self::Text => Icon::Text,
            Self::Link => Icon::External,
            Self::Document => Icon::Script,
        }
    }
}

/// A light glass pill that identifies a resource type and its item count.
pub struct ContentBadge {
    theme: ThemePreset,
    content_type: ContentType,
    count: u32,
}

impl ContentBadge {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn new(content_type: ContentType, count: u32) -> Self {
        Self {
            theme: ThemePreset::default(),
            content_type,
            count,
        }
    }

    pub fn render(self) -> gpui::Div {
        let theme = self.theme;
        div()
            .h(px(30.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(999.))
            .bg(rgb(theme.palette().surface))
            .text_color(rgb(theme.color(0x505451)))
            .text_size(px(13.))
            .child(self.content_type.icon().render(14.))
            .child(self.content_type.label())
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(theme.color(0x929895)))
                    .child(self.count.to_string()),
            )
    }
}

impl IntoElement for ContentBadge {
    type Element = gpui::Div;

    fn into_element(self) -> Self::Element {
        self.render()
    }
}

#[derive(Clone, Copy, Default)]
pub enum TagBadgeKind {
    #[default]
    Tag,
    Synced,
    RealData,
}

/// A content tag or a softly tinted state label.
pub struct TagBadge {
    theme: ThemePreset,
    label: SharedString,
    kind: TagBadgeKind,
}

impl TagBadge {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            theme: ThemePreset::default(),
            label: label.into(),
            kind: TagBadgeKind::Tag,
        }
    }

    pub fn kind(mut self, kind: TagBadgeKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn render(self) -> gpui::Div {
        let theme = self.theme;
        let (background, foreground, border, prefix) = match self.kind {
            TagBadgeKind::Tag => (
                theme.color(0xf0f2ee),
                theme.color(0x4e5451),
                theme.color(0xdfe4df),
                "# ",
            ),
            TagBadgeKind::Synced => (
                theme.color(0xdcecff),
                theme.color(0x087bfa),
                theme.color(0xdcecff),
                "",
            ),
            TagBadgeKind::RealData => (
                theme.color(0xe0f2e2),
                theme.color(0x16833a),
                theme.color(0xe0f2e2),
                "",
            ),
        };
        div()
            .h(px(30.))
            .px(px(12.))
            .flex()
            .items_center()
            .rounded(px(999.))
            .border_1()
            .border_color(rgb(border))
            .bg(rgb(background))
            .text_color(rgb(foreground))
            .text_size(px(13.))
            .child(format!("{prefix}{}", self.label))
    }
}

impl IntoElement for TagBadge {
    type Element = gpui::Div;

    fn into_element(self) -> Self::Element {
        self.render()
    }
}

#[derive(Clone, Copy, Default)]
pub enum CountBadgeKind {
    #[default]
    Unread,
    Muted,
    Pro,
}

/// Compact numeric or tier marker. Keep the red treatment for unread counts.
pub struct CountBadge {
    theme: ThemePreset,
    label: SharedString,
    kind: CountBadgeKind,
}

impl CountBadge {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            theme: ThemePreset::default(),
            label: label.into(),
            kind: CountBadgeKind::Unread,
        }
    }

    pub fn kind(mut self, kind: CountBadgeKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn render(self) -> gpui::Div {
        let theme = self.theme;
        let (background, foreground, horizontal_padding, shadow) = match self.kind {
            CountBadgeKind::Unread => (theme.color(0xff3d36), 0xffffff, 10., true),
            CountBadgeKind::Muted => (theme.color(0xe8ebe7), theme.color(0x555b58), 9., false),
            CountBadgeKind::Pro => (theme.color(0xd83bb4), 0xffffff, 12., true),
        };
        let badge = div()
            .h(px(25.))
            .px(px(horizontal_padding))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(999.))
            .bg(rgb(background))
            .text_color(rgb(foreground))
            .text_size(px(12.5))
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(self.label);
        if shadow { badge.shadow_sm() } else { badge }
    }
}

impl IntoElement for CountBadge {
    type Element = gpui::Div;

    fn into_element(self) -> Self::Element {
        self.render()
    }
}

#[derive(Clone, Copy, Default)]
pub enum OverlayBadgeKind {
    #[default]
    Index,
    Rating,
    Media,
}

/// A small label designed to sit over thumbnails and other colorful surfaces.
pub struct OverlayBadge {
    theme: ThemePreset,
    label: SharedString,
    kind: OverlayBadgeKind,
    count: Option<u32>,
}

impl OverlayBadge {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            theme: ThemePreset::default(),
            label: label.into(),
            kind: OverlayBadgeKind::Index,
            count: None,
        }
    }

    pub fn rating(label: impl Into<SharedString>) -> Self {
        Self {
            theme: ThemePreset::default(),
            label: label.into(),
            kind: OverlayBadgeKind::Rating,
            count: None,
        }
    }

    pub fn media(label: impl Into<SharedString>, count: u32) -> Self {
        Self {
            theme: ThemePreset::default(),
            label: label.into(),
            kind: OverlayBadgeKind::Media,
            count: Some(count),
        }
    }

    pub fn render(self) -> gpui::Div {
        let theme = self.theme;
        let (background, foreground, border) = match self.kind {
            OverlayBadgeKind::Index => (0x292a35, 0xffffff, 0x494a55),
            OverlayBadgeKind::Rating => (
                theme.color(0xf4f4f5),
                theme.color(0x33343c),
                theme.color(0xe2e3e7),
            ),
            OverlayBadgeKind::Media => (
                theme.color(0xe0e0e8),
                theme.color(0x4d4e57),
                theme.color(0xf0f0f4),
            ),
        };
        let mut badge = div()
            .h(px(26.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(5.))
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(border))
            .bg(rgb(background))
            .text_color(rgb(foreground))
            .text_size(px(12.5));
        match self.kind {
            OverlayBadgeKind::Index => {
                badge = badge.child(self.label);
            }
            OverlayBadgeKind::Rating => {
                badge = badge
                    .rounded(px(9.))
                    .child(div().text_color(rgb(theme.color(0xff9500))).child("★"))
                    .child(self.label);
            }
            OverlayBadgeKind::Media => {
                badge = badge
                    .rounded(px(999.))
                    .child(Icon::Play.render(11.))
                    .child(self.label);
                if let Some(count) = self.count {
                    badge = badge.child(count.to_string());
                }
            }
        }
        badge
    }
}

impl IntoElement for OverlayBadge {
    type Element = gpui::Div;

    fn into_element(self) -> Self::Element {
        self.render()
    }
}

#[derive(Clone, Copy)]
pub enum ReadStatus {
    Unread,
    Read,
    Partial,
    Failed,
}

/// A color dot and label for read/conversion state. Red is reserved for errors.
pub struct StatusBadge {
    theme: ThemePreset,
    status: ReadStatus,
}

impl StatusBadge {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn new(status: ReadStatus) -> Self {
        Self {
            theme: ThemePreset::default(),
            status,
        }
    }

    pub fn render(self) -> gpui::Div {
        let theme = self.theme;
        let (label, color) = match self.status {
            ReadStatus::Unread => ("未看", theme.color(0xaeb2b6)),
            ReadStatus::Read => ("已看", theme.color(0x087bfa)),
            ReadStatus::Partial => ("看到一半", theme.color(0xff9500)),
            ReadStatus::Failed => ("转码失败", theme.color(0xff3d36)),
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(14.))
            .text_color(rgb(color))
            .child(div().size(px(9.)).rounded(px(999.)).bg(rgb(color)))
            .child(label)
    }
}

impl IntoElement for StatusBadge {
    type Element = gpui::Div;

    fn into_element(self) -> Self::Element {
        self.render()
    }
}
