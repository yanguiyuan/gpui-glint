use super::{Icon, Popover};
use crate::theme::ThemePreset;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, ElementId, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, Window,
    div, px, rgb,
};

/// A compact action row shared by application dropdown menus.
pub struct MenuItem {
    id: ElementId,
    label: SharedString,
    icon: Option<Icon>,
    disabled: bool,
    theme: ThemePreset,
}
impl MenuItem {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            disabled: false,
            theme: ThemePreset::default(),
        }
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
        div()
            .id(self.id)
            .h(px(32.))
            .flex_shrink_0()
            .w_full()
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded_sm()
            .text_size(px(12.))
            .text_color(rgb(if self.disabled { p.text_muted } else { p.text }))
            .when(!self.disabled, |row| {
                row.cursor_pointer()
                    .hover(move |s| s.bg(rgb(p.accent_tint)))
            })
            .when_some(self.icon, |row, icon| row.child(icon.render(14.)))
            .child(self.label)
    }
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
