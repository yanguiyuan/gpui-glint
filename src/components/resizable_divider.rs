use crate::theme::ThemePreset;
use gpui::{CursorStyle, Div, InteractiveElement, MouseButton, ParentElement, Stateful, Styled, div, px, rgb};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DividerAxis {
    #[default]
    Horizontal,
    Vertical,
}

/// A 5px interactive resize handle that renders a crisp 1px hairline at rest
/// and highlights to a 2px accent line on hover or while actively dragging.
pub struct ResizableDivider {
    id: gpui::SharedString,
    debug_selector: Option<&'static str>,
    axis: DividerAxis,
    dragging: bool,
    theme: ThemePreset,
    thickness: f32,
}

impl ResizableDivider {
    pub fn horizontal(id: impl Into<gpui::SharedString>) -> Self {
        Self {
            id: id.into(),
            debug_selector: None,
            axis: DividerAxis::Horizontal,
            dragging: false,
            theme: ThemePreset::default(),
            thickness: 5.,
        }
    }

    pub fn vertical(id: impl Into<gpui::SharedString>) -> Self {
        Self {
            id: id.into(),
            debug_selector: None,
            axis: DividerAxis::Vertical,
            dragging: false,
            theme: ThemePreset::default(),
            thickness: 5.,
        }
    }

    pub fn debug_selector(mut self, selector: &'static str) -> Self {
        self.debug_selector = Some(selector);
        self
    }

    pub fn dragging(mut self, dragging: bool) -> Self {
        self.dragging = dragging;
        self
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn thickness(mut self, thickness: f32) -> Self {
        self.thickness = thickness.max(3.);
        self
    }

    pub fn render(self) -> Stateful<Div> {
        let p = self.theme.palette();
        let group_name = gpui::SharedString::from(format!("{}-group", self.id));
        let dragging = self.dragging;
        let line = match self.axis {
            DividerAxis::Horizontal => div()
                .h_full()
                .w(px(if dragging { 2. } else { 1. }))
                .bg(rgb(if dragging { p.accent } else { p.border }))
                .group_hover(group_name.clone(), move |style| {
                    style.w(px(2.)).bg(rgb(p.accent))
                }),
            DividerAxis::Vertical => div()
                .w_full()
                .h(px(if dragging { 2. } else { 1. }))
                .bg(rgb(if dragging { p.accent } else { p.border }))
                .group_hover(group_name.clone(), move |style| {
                    style.h(px(2.)).bg(rgb(p.accent))
                }),
        };
        let mut handle = div()
            .id(self.id)
            .group(group_name)
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center();
        if let Some(selector) = self.debug_selector {
            handle = handle.debug_selector(move || selector.into());
        }
        match self.axis {
            DividerAxis::Horizontal => handle
                .w(px(self.thickness))
                .h_full()
                .cursor(CursorStyle::ResizeLeftRight),
            DividerAxis::Vertical => handle
                .h(px(self.thickness))
                .w_full()
                .cursor(CursorStyle::ResizeUpDown),
        }
        .child(line)
    }

    pub fn on_drag_start(
        self,
        handler: impl Fn(&gpui::MouseDownEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Stateful<Div> {
        self.render().on_mouse_down(MouseButton::Left, handler)
    }
}
