use super::Icon;
use crate::theme::ThemePreset;
use std::hash::{Hash, Hasher};

use gpui::{
    InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, div, px, rgb,
};

#[derive(Clone, Copy, Default)]
pub enum ButtonKind {
    #[default]
    Primary,
    Tinted,
    Glass,
    Plain,
    Gray,
    Danger,
    DangerTinted,
}

#[derive(Clone, Copy, Default)]
pub enum ButtonSize {
    Large,
    #[default]
    Regular,
    Small,
    Mini,
}

pub struct Button {
    theme: ThemePreset,
    label: SharedString,
    icon: Option<Icon>,
    kind: ButtonKind,
    size: ButtonSize,
    pill: bool,
    disabled: bool,
}

impl Button {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            theme: ThemePreset::default(),
            label: label.into(),
            icon: None,
            kind: ButtonKind::Primary,
            size: ButtonSize::Regular,
            pill: false,
            disabled: false,
        }
    }

    /// Place an SVG icon before the button label.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn kind(mut self, kind: ButtonKind) -> Self {
        self.kind = kind;
        self
    }
    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }
    pub fn pill(mut self) -> Self {
        self.pill = true;
        self
    }
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    pub fn render(self) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        let (height, font_size, padding) = match self.size {
            ButtonSize::Large => (38., 14.5, 20.),
            ButtonSize::Regular => (30., 13.5, 13.),
            ButtonSize::Small => (25., 12.5, 10.),
            ButtonSize::Mini => (21., 11.5, 8.),
        };
        let (background, foreground, border, hover, shadow) = match self.kind {
            ButtonKind::Primary => (
                theme.color(0x087bfa),
                0xffffff,
                theme.color(0x087bfa),
                theme.color(0x1688ff),
                true,
            ),
            ButtonKind::Tinted => (
                theme.color(0xe1efff),
                theme.color(0x0874e8),
                theme.color(0xe1efff),
                theme.color(0xd4e8ff),
                false,
            ),
            ButtonKind::Glass => (
                theme.color(0xfafcff),
                theme.color(0x262a32),
                theme.color(0xd5dae2),
                theme.color(0xf0f4f9),
                true,
            ),
            ButtonKind::Plain => (
                theme.color(0xf3f5f8),
                theme.color(0x0874e8),
                theme.color(0xf3f5f8),
                theme.color(0xe6f1ff),
                false,
            ),
            ButtonKind::Gray => (
                theme.color(0xe8ebf0),
                theme.color(0x262a32),
                theme.color(0xe8ebf0),
                theme.color(0xdde1e8),
                false,
            ),
            ButtonKind::Danger => (
                theme.color(0xf0443a),
                0xffffff,
                theme.color(0xf0443a),
                theme.color(0xff5a50),
                true,
            ),
            ButtonKind::DangerTinted => (
                theme.color(0xffe9e7),
                theme.color(0xd9342b),
                theme.color(0xffe9e7),
                theme.color(0xffdeda),
                false,
            ),
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.label.hash(&mut hasher);
        let id = ("glass-button", hasher.finish());
        let mut button = div()
            .id(id)
            .h(px(height))
            .px(px(padding))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(if self.pill { 999. } else { 7. }))
            .border_1()
            .border_color(rgb(border))
            .bg(rgb(background))
            .text_color(rgb(foreground))
            .text_size(px(font_size))
            .font_weight(gpui::FontWeight::MEDIUM)
            .gap(px(6.))
            .children(self.icon.map(|icon| icon.render(font_size)))
            .child(self.label);
        if self.disabled {
            button = button.opacity(0.45);
        } else {
            button = button
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(hover)))
                .active(move |style| style.opacity(0.82));
        }
        if shadow {
            button = button.shadow_md();
        }
        button
    }
}

impl IntoElement for Button {
    type Element = gpui::Stateful<gpui::Div>;

    fn into_element(self) -> Self::Element {
        self.render()
    }
}
