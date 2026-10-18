use crate::theme::ThemePreset;
use gpui::{AnyElement, App, ElementId, FocusHandle, Window, div, prelude::*, px, rgb};
use std::rc::Rc;

pub struct DialogDismiss;

type CloseHandler = Rc<dyn Fn(&DialogDismiss, &mut Window, &mut App)>;

/// Controlled modal frame. Render only while open, inside a full-window relative root.
/// The host owns form state and initial/return focus; dismissal never submits the form.
pub struct Dialog {
    id: ElementId,
    content: AnyElement,
    width: f32,
    theme: ThemePreset,
    dismissible: bool,
    priority: usize,
    on_close: Option<CloseHandler>,
    focus_handles: Vec<FocusHandle>,
}
impl Dialog {
    pub fn new(id: impl Into<ElementId>, content: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            content: content.into_any_element(),
            width: 420.,
            theme: ThemePreset::default(),
            dismissible: true,
            priority: 110,
            on_close: None,
            focus_handles: vec![],
        }
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(120.);
        self
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    /// Disable Escape and scrim dismissal while a form submission is pending.
    pub fn dismissible(mut self, dismissible: bool) -> Self {
        self.dismissible = dismissible;
        self
    }
    pub fn priority(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }
    pub fn on_close(
        mut self,
        handler: impl Fn(&DialogDismiss, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Some(Rc::new(handler));
        self
    }
    /// Ordered form controls for modal Tab/Shift-Tab navigation.
    pub fn focus_handles(mut self, handles: impl IntoIterator<Item = FocusHandle>) -> Self {
        self.focus_handles = handles.into_iter().collect();
        self
    }
    pub fn render(self) -> AnyElement {
        let p = self.theme.palette();
        let close = self.on_close.clone();
        let keyboard = self.on_close;
        let dismissible = self.dismissible;
        let focus_handles = self.focus_handles;
        let panel = div()
            .id(self.id)
            .w(px(self.width))
            .max_w_full()
            .max_h_full()
            .overflow_y_scroll()
            .p_5()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .text_color(rgb(p.text))
            .shadow_lg()
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_key_down(move |event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "tab" && !focus_handles.is_empty() {
                    let current = focus_handles
                        .iter()
                        .position(|handle| handle.is_focused(window));
                    let next = match current {
                        Some(current) if event.keystroke.modifiers.shift => {
                            (current + focus_handles.len() - 1) % focus_handles.len()
                        }
                        Some(current) => (current + 1) % focus_handles.len(),
                        None if event.keystroke.modifiers.shift => focus_handles.len() - 1,
                        None => 0,
                    };
                    window.focus(&focus_handles[next], cx);
                    window.prevent_default();
                    cx.stop_propagation();
                }
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    window.prevent_default();
                    if dismissible {
                        if let Some(close) = &keyboard {
                            close(&DialogDismiss, window, cx);
                        }
                    }
                }
            })
            .child(self.content);
        gpui::deferred(
            div()
                .absolute()
                .inset_0()
                .size_full()
                .bg(gpui::rgba(0x0c121c48))
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .p_5()
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    if dismissible {
                        if let Some(close) = &close {
                            close(&DialogDismiss, window, cx);
                        }
                    }
                })
                .child(panel),
        )
        .with_priority(self.priority)
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, FocusHandle, Render, point};

    struct DialogView {
        open: bool,
        dismissible: bool,
        focus: [FocusHandle; 2],
        background_clicks: usize,
        background_scrolls: usize,
    }
    impl Render for DialogView {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("dialog-test-root")
                .relative()
                .size_full()
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, _| this.background_clicks += 1),
                )
                .on_scroll_wheel(cx.listener(|this, _, _, _| this.background_scrolls += 1))
                .when(self.open, |root| {
                    root.child(
                        Dialog::new(
                            "test-dialog",
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .id("first")
                                        .debug_selector(|| "dialog-first".into())
                                        .track_focus(&self.focus[0])
                                        .h(px(50.))
                                        .child("first"),
                                )
                                .child(
                                    div()
                                        .id("second")
                                        .track_focus(&self.focus[1])
                                        .h(px(50.))
                                        .child("second"),
                                ),
                        )
                        .focus_handles(self.focus.clone())
                        .dismissible(self.dismissible)
                        .on_close(cx.listener(|this, _, _, cx| {
                            this.open = false;
                            cx.notify();
                        }))
                        .render(),
                    )
                })
        }
    }
    fn view(cx: &mut Context<DialogView>) -> DialogView {
        DialogView {
            open: true,
            dismissible: true,
            focus: [cx.focus_handle(), cx.focus_handle()],
            background_clicks: 0,
            background_scrolls: 0,
        }
    }
    #[gpui::test]
    fn dialog_blocks_background_cycles_focus_and_dismisses(cx: &mut gpui::TestAppContext) {
        let (host, window) = cx.add_window_view(|_, cx| view(cx));
        window.update(|window, cx| {
            let focus = host.read(cx).focus[0].clone();
            window.focus(&focus, cx);
        });
        window.simulate_keystrokes("tab");
        assert!(window.update(|window, cx| host.read(cx).focus[1].is_focused(window)));
        window.simulate_keystrokes("tab");
        assert!(window.update(|window, cx| host.read(cx).focus[0].is_focused(window)));
        window.simulate_keystrokes("shift-tab");
        assert!(window.update(|window, cx| host.read(cx).focus[1].is_focused(window)));
        let content = window.debug_bounds("dialog-first").unwrap();
        window.simulate_click(content.center(), gpui::Modifiers::default());
        window.simulate_event(gpui::ScrollWheelEvent {
            position: content.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-50.))),
            ..Default::default()
        });
        host.update(window, |host, _| {
            assert!(host.open);
            assert_eq!(host.background_clicks, 0);
            assert_eq!(host.background_scrolls, 0);
        });
        window.simulate_keystrokes("escape");
        host.update(window, |host, cx| {
            assert!(!host.open);
            host.open = true;
            cx.notify();
        });
        window.simulate_click(point(px(2.), px(2.)), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert!(!host.open);
            assert_eq!(host.background_clicks, 0);
        });
    }
    #[gpui::test]
    fn pending_dialog_rejects_escape_and_scrim_dismissal(cx: &mut gpui::TestAppContext) {
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = view(cx);
            host.dismissible = false;
            host
        });
        window.update(|window, cx| {
            let focus = host.read(cx).focus[0].clone();
            window.focus(&focus, cx);
        });
        window.simulate_keystrokes("escape");
        window.simulate_click(point(px(2.), px(2.)), gpui::Modifiers::default());
        host.update(window, |host, _| {
            assert!(host.open);
            assert_eq!(host.background_clicks, 0);
        });
    }
}
