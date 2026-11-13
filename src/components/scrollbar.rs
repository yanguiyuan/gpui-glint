use std::{cell::Cell, rc::Rc};

use gpui::prelude::FluentBuilder;
use gpui::{Bounds, Div, ParentElement, Pixels, ScrollHandle, Styled, div, point, px, rgba, size};

use crate::theme::ThemePreset;

/// Retains the thumb grab position across renders.
#[derive(Clone, Default)]
pub struct ScrollbarState(Rc<Cell<Option<Pixels>>>);

/// A visible, draggable rail for a GPUI scroll handle.
pub struct Scrollbar {
    handle: ScrollHandle,
    state: ScrollbarState,
    vertical: bool,
    theme: ThemePreset,
    maximum: Option<Pixels>,
    extent: Option<Pixels>,
}

impl Scrollbar {
    pub fn vertical(handle: &ScrollHandle, state: &ScrollbarState) -> Self {
        Self { handle: handle.clone(), state: state.clone(), vertical: true, theme: ThemePreset::default(), maximum: None, extent: None }
    }

    pub fn horizontal(handle: &ScrollHandle, state: &ScrollbarState) -> Self {
        Self { vertical: false, ..Self::vertical(handle, state) }
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self { self.theme = theme; self }

    /// Custom ranges support synchronized, painted viewports without a layout
    /// wider than the visible pane (such as the two sides of a diff).
    pub fn range(mut self, maximum: Pixels, extent: Pixels) -> Self {
        self.maximum = Some(maximum); self.extent = Some(extent); self
    }

    pub fn render(self) -> Div {
        let vertical = self.vertical;
        let handle = self.handle;
        let drag = self.state.0;
        let dark = self.theme.is_dark();
        div()
            .when(vertical, |rail| rail.w(px(12.)).h_full())
            .when(!vertical, |rail| rail.h(px(12.)).w_full())
            .flex_shrink_0()
            .child(gpui::canvas(
                move |bounds, window, _| {
                    let maximum = self.maximum.unwrap_or_else(|| if vertical { handle.max_offset().y } else { handle.max_offset().x });
                    let length = if vertical { bounds.size.height } else { bounds.size.width };
                    if maximum <= px(0.) || length <= px(4.) { return None; }
                    let track = bounds.inset(px(2.));
                    let length = if vertical { track.size.height } else { track.size.width };
                    let extent = self.extent.unwrap_or(length).max(px(1.));
                    let thumb_length = (length * (extent / (extent + maximum))).max(px(24.)).min(length);
                    let travel = length - thumb_length;
                    let offset = if vertical { handle.offset().y } else { handle.offset().x };
                    let position = travel * (-offset / maximum).clamp(0., 1.);
                    let thumb = if vertical {
                        Bounds::new(point(bounds.left() + px(3.), track.top() + position), size(px(6.), thumb_length))
                    } else {
                        Bounds::new(point(track.left() + position, bounds.top() + px(3.)), size(thumb_length, px(6.)))
                    };
                    let hitbox = window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal);
                    Some((hitbox, thumb, track, maximum, travel, handle.clone()))
                },
                move |_, geometry, window, _| {
                    let Some((hitbox, thumb, track, maximum, travel, handle)) = geometry else { return; };
                    window.paint_quad(gpui::quad(thumb, px(3.), rgba(if dark { 0xffffff66 } else { 0x00000055 }), px(0.), rgba(0), Default::default()));
                    window.set_cursor_style(gpui::CursorStyle::Arrow, &hitbox);
                    let down_handle = handle.clone();
                    let down_drag = drag.clone();
                    window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, window, cx| {
                        if phase != gpui::DispatchPhase::Bubble || event.button != gpui::MouseButton::Left || !hitbox.is_hovered(window) { return; }
                        cx.stop_propagation(); window.prevent_default();
                        let grab = if thumb.contains(&event.position) {
                            if vertical { event.position.y - thumb.top() } else { event.position.x - thumb.left() }
                        } else if vertical { thumb.size.height / 2. } else { thumb.size.width / 2. };
                        down_drag.set(Some(grab));
                        set_position(&down_handle, if vertical { event.position.y - grab - track.top() } else { event.position.x - grab - track.left() }, travel, maximum, vertical);
                        window.refresh();
                    });
                    let move_handle = handle.clone();
                    let move_drag = drag.clone();
                    window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, cx| {
                        if phase != gpui::DispatchPhase::Capture { return; }
                        if let Some(grab) = move_drag.get() {
                            if !event.dragging() { move_drag.set(None); }
                            else {
                                set_position(&move_handle, if vertical { event.position.y - grab - track.top() } else { event.position.x - grab - track.left() }, travel, maximum, vertical);
                                cx.stop_propagation();
                            }
                            window.refresh();
                        }
                    });
                    window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, window, cx| {
                        if phase == gpui::DispatchPhase::Capture && event.button == gpui::MouseButton::Left && drag.take().is_some() {
                            cx.stop_propagation(); window.refresh();
                        }
                    });
                },
            ).size_full())
    }
}

fn set_position(handle: &ScrollHandle, position: Pixels, travel: Pixels, maximum: Pixels, vertical: bool) {
    let ratio = if travel > px(0.) { (position / travel).clamp(0., 1.) } else { 0. };
    let offset = handle.offset();
    handle.set_offset(if vertical { point(offset.x, -maximum * ratio) } else { point(-maximum * ratio, offset.y) });
}
