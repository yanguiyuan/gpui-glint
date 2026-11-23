use super::{Icon, Popover};
use crate::theme::ThemePreset;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, ColorExt, ElementId, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, Window, div, px, rgb, rgba,
};

/// A compact action row shared by application dropdown menus.
pub struct MenuItem {
    id: ElementId,
    label: SharedString,
    shortcut: Option<SharedString>,
    icon: Option<Icon>,
    disabled: bool,
    theme: ThemePreset,
}
impl MenuItem {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            shortcut: None,
            icon: None,
            disabled: false,
            theme: ThemePreset::default(),
        }
    }
    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn render(self) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let group_name = SharedString::from(format!("menu-item:{}", self.id));
        let hover_bg = if self.theme.is_dark() {
            0x2d7df6
        } else {
            0x3b82f6
        };
        div()
            .id(self.id)
            .group(group_name.clone())
            .h(px(28.))
            .flex_shrink_0()
            .w_full()
            .px(px(10.))
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .rounded(px(7.))
            .text_size(px(13.))
            .text_color(rgb(if self.disabled { p.text_muted } else { p.text }))
            .when(!self.disabled, |row| {
                row.cursor_pointer().hover(move |s| {
                    s.bg(rgb(hover_bg)).text_color(rgb(0xffffff))
                })
            })
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(self.icon, |row, icon| {
                        row.child(icon.render(14.).flex_shrink_0())
                    })
                    .child(div().min_w_0().truncate().child(self.label)),
            )
            .when_some(self.shortcut, |row, shortcut| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .when(!self.disabled, |hint| {
                            hint.group_hover(group_name, |s| s.text_color(rgba(0xffffffd8)))
                        })
                        .child(shortcut),
                )
            })
    }
}

/// Hairline divider between logical groups inside a context menu.
pub fn menu_separator(theme: ThemePreset) -> gpui::Div {
    let p = theme.palette();
    div()
        .my(px(4.))
        .mx(px(10.))
        .h(px(1.))
        .flex_shrink_0()
        .bg(rgb(p.border).opacity(0.65))
}

/// Translucent macOS-style floating context menu container.
pub fn context_menu_surface(
    id: impl Into<ElementId>,
    theme: ThemePreset,
    width: f32,
) -> gpui::Stateful<gpui::Div> {
    let (bg, border) = if theme.is_dark() {
        (0x26282deb, 0xffffff1c)
    } else {
        (0xf8f9fbeb, 0x00000018)
    };
    div()
        .id(id)
        .w(px(width))
        .p(px(5.))
        .rounded(px(12.))
        .border_1()
        .border_color(rgba(border))
        .bg(rgba(bg))
        .shadow_xl()
        .occlude()
}

/// Controlled action menu with shared compact spacing and popup behavior.
pub struct DropdownMenu {
    popup: Popover,
}
impl DropdownMenu {
    pub fn new(
        id: impl Into<ElementId>,
        trigger: impl IntoElement,
        content: impl IntoElement,
    ) -> Self {
        Self {
            popup: Popover::new(id, trigger, content).padding(4.).width(180.),
        }
    }
    pub fn open(mut self, open: bool) -> Self {
        self.popup = self.popup.open(open);
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.popup = self.popup.width(width);
        self
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.popup = self.popup.theme(theme);
        self
    }
    pub fn on_open_change(
        mut self,
        callback: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.popup = self.popup.on_open_change(callback);
        self
    }
    pub fn render(self) -> gpui::Stateful<gpui::Div> {
        self.popup.render()
    }
}
