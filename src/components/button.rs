use super::Icon;
use crate::theme::ThemePreset;
use std::hash::{Hash, Hasher};

use gpui::{
    InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, div, px, rgb,
};

#[derive(Clone, Copy, Default, PartialEq)]
pub enum ButtonKind {
    #[default]
    Primary,
    Tinted,
    Glass,
    Plain,
    Gray,
    Danger,
    DangerTinted,
    /// Borderless and transparent until hovered. For quiet triggers that sit
    /// beside a segmented control or a real primary action, so those keep the
    /// visual weight. Pair with [`Button::foreground`] to tint live state.
    Ghost,
}

/// How much depth a button casts. `Glass` defaults to `Subtle` because it
/// already carries a 1px border and a full `shadow_md` reads too heavy next to
/// flat cards.
#[derive(Clone, Copy, PartialEq)]
pub enum ButtonShadow {
    Flat,
    Subtle,
    Normal,
}

#[derive(Clone, Copy, Default)]
pub enum ButtonSize {
    Large,
    #[default]
    Regular,
    Small,
    Mini,
    /// Text-height, for triggers that share a row with 24px segmented controls.
    Compact,
}

pub struct Button {
    theme: ThemePreset,
    label: SharedString,
    icon: Option<Icon>,
    disclosure: Option<bool>,
    kind: ButtonKind,
    size: ButtonSize,
    pill: bool,
    disabled: bool,
    truncate_label: bool,
    shadow: Option<ButtonShadow>,
    foreground: Option<u32>,
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
            disclosure: None,
            kind: ButtonKind::Primary,
            size: ButtonSize::Regular,
            pill: false,
            disabled: false,
            truncate_label: false,
            shadow: None,
            foreground: None,
        }
    }

    /// Place an SVG icon before the button label.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    /// Add a trailing chevron that follows the panel’s expanded state.
    pub fn disclosure(mut self, expanded: bool) -> Self {
        self.disclosure = Some(expanded);
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
    /// Override the depth this button casts. Defaults to the per-kind choice
    /// (`Glass` is `Subtle`, `Primary`/`Danger` are `Normal`, the rest flat).
    pub fn shadow(mut self, shadow: ButtonShadow) -> Self {
        self.shadow = Some(shadow);
        self
    }
    /// Override the label and icon color, e.g. to tint a `Ghost` trigger while
    /// its work is running.
    pub fn foreground(mut self, color: u32) -> Self {
        self.foreground = Some(color);
        self
    }
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }

    pub fn truncate_label(mut self) -> Self {
        self.truncate_label = true;
        self
    }

    pub fn render(self) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        let (height, font_size, padding) = match self.size {
            ButtonSize::Large => (38., 14.5, 20.),
            ButtonSize::Regular => (30., 13.5, 13.),
            ButtonSize::Small => (25., 12.5, 10.),
            ButtonSize::Mini => (21., 11.5, 8.),
            ButtonSize::Compact => (24., 12.5, 6.),
        };
        // `None` background/border means the button paints nothing of its own,
        // which is what `Ghost` needs to stay quiet next to a segmented control.
        let (background, foreground, border, hover, shadow) = match self.kind {
            ButtonKind::Primary => (
                Some(theme.color(0x087bfa)),
                theme.color(0xffffff),
                Some(theme.color(0x087bfa)),
                Some(theme.color(0x1688ff)),
                ButtonShadow::Normal,
            ),
            ButtonKind::Tinted => (
                Some(theme.color(0xe1efff)),
                theme.color(0x0874e8),
                Some(theme.color(0xe1efff)),
                Some(theme.color(0xd4e8ff)),
                ButtonShadow::Flat,
            ),
            ButtonKind::Glass => (
                Some(theme.color(0xfafcff)),
                theme.color(0x262a32),
                Some(theme.color(0xd5dae2)),
                Some(theme.color(0xf0f4f9)),
                ButtonShadow::Subtle,
            ),
            ButtonKind::Plain => (
                Some(theme.color(0xf3f5f8)),
                theme.color(0x0874e8),
                Some(theme.color(0xf3f5f8)),
                Some(theme.color(0xe6f1ff)),
                ButtonShadow::Flat,
            ),
            ButtonKind::Gray => (
                Some(theme.color(0xe8ebf0)),
                theme.color(0x262a32),
                Some(theme.color(0xe8ebf0)),
                Some(theme.color(0xdde1e8)),
                ButtonShadow::Flat,
            ),
            ButtonKind::Danger => (
                Some(theme.color(0xf0443a)),
                theme.color(0xffffff),
                Some(theme.color(0xf0443a)),
                Some(theme.color(0xff5a50)),
                ButtonShadow::Normal,
            ),
            ButtonKind::DangerTinted => (
                Some(theme.color(0xffe9e7)),
                theme.color(0xd9342b),
                Some(theme.color(0xffe9e7)),
                Some(theme.color(0xffdeda)),
                ButtonShadow::Flat,
            ),
            ButtonKind::Ghost => (
                None,
                theme.color(0x6e6e73),
                None,
                Some(theme.color(0xf3f5f8)),
                ButtonShadow::Flat,
            ),
        };
        let foreground = self.foreground.unwrap_or(foreground);
        let shadow = self.shadow.unwrap_or(shadow);
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
            .text_color(rgb(foreground))
            .text_size(px(font_size))
            .font_weight(gpui::FontWeight::MEDIUM)
            .gap(px(6.))
            .children(self.icon.map(|icon| icon.render(font_size)));
        if let Some(background) = background {
            button = button.bg(rgb(background));
        }
        if let Some(border) = border {
            button = button.border_1().border_color(rgb(border));
        }
        button = if self.truncate_label {
            button
                .min_w_0()
                .overflow_hidden()
                .child(div().min_w_0().truncate().child(self.label))
        } else {
            button.child(self.label)
        };
        button = button.children(
            self.disclosure
                .map(|expanded| Icon::disclosure(font_size, expanded)),
        );
        if self.disabled {
            button = button.opacity(0.45);
        } else {
            button = button
                .cursor_pointer()
                .hover(move |style| match hover {
                    Some(hover) => style.bg(rgb(hover)),
                    None => style,
                })
                .active(move |style| style.opacity(0.82));
        }
        button = match shadow {
            ButtonShadow::Flat => button,
            ButtonShadow::Subtle => button.shadow_xs(),
            ButtonShadow::Normal => button.shadow_md(),
        };
        button
    }
}

impl IntoElement for Button {
    type Element = gpui::Stateful<gpui::Div>;

    fn into_element(self) -> Self::Element {
        self.render()
    }
}
