use crate::theme::ThemePreset;
use gpui::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, anchored, deferred, div, px, rgb,
};
use std::rc::Rc;

type OpenHandler = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// Controlled, non-modal popup attached below a trigger, aligned to its right edge.
/// Give each instance a stable, unique ID and rebuild it from the host's open state.
pub struct Popover {
    id: ElementId,
    trigger: AnyElement,
    content: AnyElement,
    open: bool,
    width: f32,
    trigger_height: f32,
    gap: f32,
    theme: ThemePreset,
    on_open_change: Option<OpenHandler>,
}
impl Popover {
    pub fn new(
        id: impl Into<ElementId>,
        trigger: impl IntoElement,
        content: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            trigger: trigger.into_any_element(),
            content: content.into_any_element(),
            open: false,
            width: 280.,
            trigger_height: 30.,
            gap: 6.,
            theme: ThemePreset::default(),
            on_open_change: None,
        }
    }
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(120.);
        self
    }
    /// Height of the trigger, used to leave room above the popup. Default: 30 pixels.
    pub fn trigger_height(mut self, height: f32) -> Self {
        self.trigger_height = height.max(0.);
        self
    }
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap.max(0.);
        self
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn on_open_change(
        mut self,
        handler: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Rc::new(handler));
        self
    }
    pub fn render(self) -> gpui::Stateful<gpui::Div> {
        let open = self.open;
        let toggle = self.on_open_change.clone();
        let keyboard = self.on_open_change.clone();
        let p = self.theme.palette();
        let mut root = div()
            .id(self.id)
            .relative()
            .flex_shrink_0()
            .focusable()
            .on_key_down(move |event: &gpui::KeyDownEvent, window, cx| {
                let next = match event.keystroke.key.as_str() {
                    "escape" if open => Some(false),
                    "enter" | "space" if !open => Some(true),
                    _ => None,
                };
                if let Some(next) = next {
                    cx.stop_propagation();
                    window.prevent_default();
                    if let Some(handler) = &keyboard {
                        handler(next, window, cx);
                    }
                }
            })
            .child(
                div()
                    .id("popover-trigger")
                    .cursor_pointer()
                    .child(self.trigger)
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        if let Some(handler) = &toggle {
                            handler(!open, window, cx);
                        }
                    }),
            );
        if open {
            let dismiss = self.on_open_change;
            // The outside-click bounds include the trigger and its gap, so clicking the
            // trigger again closes it without a mouse-down/click reopen race.
            let popup = div()
                .pt(px(self.trigger_height + self.gap))
                .w(px(self.width))
                .on_mouse_down_out(move |_, window, cx| {
                    if let Some(handler) = &dismiss {
                        handler(false, window, cx);
                    }
                })
                .child(
                    div()
                        .id("popover-content")
                        .p_4()
                        .rounded(px(9.))
                        .border_1()
                        .border_color(rgb(p.border))
                        .bg(rgb(p.surface))
                        .text_color(rgb(p.text))
                        .text_size(px(12.))
                        .shadow_lg()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                        .child(self.content),
                );
            root = root.child(
                deferred(
                    div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .w(px(self.width))
                        .child(anchored().snap_to_window_with_margin(px(8.)).child(popup)),
                )
                .with_priority(20),
            );
        }
        root
    }
}
impl IntoElement for Popover {
    type Element = gpui::Stateful<gpui::Div>;
    fn into_element(self) -> Self::Element {
        self.render()
    }
}
