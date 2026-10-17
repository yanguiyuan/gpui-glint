use super::Icon;
use crate::theme::ThemePreset;
use gpui::StatefulInteractiveElement;
use gpui::{
    Context, EventEmitter, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    Styled, Task, Window, deferred, div, px, rgb,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastKind {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}
/// 通知堆栈在宿主容器内的显示位置。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastPosition {
    #[default]
    BottomRight,
    TopRight,
    TopCenter,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub message: SharedString,
    pub description: Option<SharedString>,
    pub kind: ToastKind,
    /// None keeps the notification visible until explicitly dismissed.
    pub duration: Option<Duration>,
}
impl Toast {
    pub fn new(message: impl Into<SharedString>) -> Self {
        Self {
            message: message.into(),
            description: None,
            kind: ToastKind::Info,
            duration: Some(Duration::from_secs(4)),
        }
    }
    pub fn kind(mut self, kind: ToastKind) -> Self {
        self.kind = kind;
        self
    }
    pub fn description(mut self, text: impl Into<SharedString>) -> Self {
        self.description = Some(text.into());
        self
    }
    pub fn duration(mut self, duration: Option<Duration>) -> Self {
        self.duration = duration;
        self
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastDismissReason {
    Closed,
    Expired,
    Overflow,
}
#[derive(Clone, Debug)]
pub enum ToastEvent {
    Dismissed {
        id: ToastId,
        reason: ToastDismissReason,
    },
}
struct Entry {
    id: ToastId,
    toast: Toast,
    remaining: Option<Duration>,
    started: Option<Instant>,
    timer: Option<Task<()>>,
}
/// Mount once inside a relative, full-size application root. Notifications default to bottom right; configure with `position`.
/// Automatic dismissal pauses while hovered, keeping longer messages readable.
pub struct ToastHost {
    entries: Vec<Entry>,
    next_id: u64,
    theme: ThemePreset,
    max_visible: usize,
    position: ToastPosition,
}
impl Default for ToastHost {
    fn default() -> Self {
        Self::new()
    }
}
impl EventEmitter<ToastEvent> for ToastHost {}
impl ToastHost {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_id: 0,
            theme: ThemePreset::default(),
            max_visible: 3,
            position: ToastPosition::default(),
        }
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    /// 设置整个通知堆栈的位置，默认右下。
    pub fn position(mut self, position: ToastPosition) -> Self {
        self.position = position;
        self
    }
    /// 移动现有通知，不重置其自动关闭计时。
    pub fn set_position(&mut self, position: ToastPosition, cx: &mut Context<Self>) {
        if self.position != position {
            self.position = position;
            cx.notify();
        }
    }
    pub fn max_visible(mut self, count: usize) -> Self {
        self.max_visible = count.max(1);
        self
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            cx.notify();
        }
    }
    /// Currently visible notifications, in insertion order.
    pub fn toasts(&self) -> impl Iterator<Item = &Toast> {
        self.entries.iter().map(|entry| &entry.toast)
    }
    pub fn push(&mut self, toast: Toast, cx: &mut Context<Self>) -> ToastId {
        while self.entries.len() >= self.max_visible {
            self.dismiss(self.entries[0].id, ToastDismissReason::Overflow, cx);
        }
        self.next_id += 1;
        let id = ToastId(self.next_id);
        self.entries.push(Entry {
            id,
            remaining: toast.duration,
            toast,
            started: None,
            timer: None,
        });
        self.resume(id, cx);
        cx.notify();
        id
    }
    pub fn dismiss(&mut self, id: ToastId, reason: ToastDismissReason, cx: &mut Context<Self>) {
        if let Some(index) = self.entries.iter().position(|e| e.id == id) {
            self.entries.remove(index);
            cx.emit(ToastEvent::Dismissed { id, reason });
            cx.notify();
        }
    }
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<_> = self.entries.iter().map(|e| e.id).collect();
        for id in ids {
            self.dismiss(id, ToastDismissReason::Closed, cx);
        }
    }
    fn pause(&mut self, id: ToastId) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            if let (Some(left), Some(start)) = (e.remaining, e.started.take()) {
                e.remaining = Some(left.saturating_sub(start.elapsed()));
            }
            e.timer = None;
        }
    }
    fn resume(&mut self, id: ToastId, cx: &mut Context<Self>) {
        let Some(e) = self.entries.iter_mut().find(|e| e.id == id) else {
            return;
        };
        let Some(left) = e.remaining else { return };
        if e.timer.is_some() {
            return;
        }
        e.started = Some(Instant::now());
        let timer = cx.background_executor().timer(left);
        e.timer = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| {
                this.dismiss(id, ToastDismissReason::Expired, cx)
            });
        }));
    }
}
impl Render for ToastHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let p = self.theme.palette();
        let mut stack = div().w(px(340.)).max_w_full().flex().flex_col().gap_2();
        for e in &self.entries {
            let id = e.id;
            let (color, symbol) = match e.toast.kind {
                ToastKind::Info => (p.accent, "i"),
                ToastKind::Success => (theme.color(0x24864c), "✓"),
                ToastKind::Warning => (theme.color(0xb37716), "!"),
                ToastKind::Error => (theme.color(0xd9342b), "×"),
            };
            let mut text = div()
                .flex_1()
                .min_w_0()
                .text_size(px(13.))
                .text_color(rgb(p.text))
                .child(e.toast.message.clone());
            if let Some(description) = &e.toast.description {
                text = text.child(
                    div()
                        .mt_1()
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child(description.clone()),
                );
            }
            stack = stack.child(
                div()
                    .id(("glass-toast", id.0))
                    .p_3()
                    .flex()
                    .items_start()
                    .gap_3()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(p.border))
                    .bg(rgb(p.surface))
                    .shadow_lg()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_hover(cx.listener(move |this, hovered, _, cx| {
                        if *hovered {
                            this.pause(id);
                        } else {
                            this.resume(id, cx);
                        }
                    }))
                    .child(
                        div()
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(rgb(p.panel))
                            .text_color(rgb(color))
                            .child(symbol),
                    )
                    .child(text)
                    .child(
                        div()
                            .id(("close-toast", id.0))
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .text_color(rgb(p.text_muted))
                            .hover(move |s| s.bg(rgb(p.surface_subtle)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.dismiss(id, ToastDismissReason::Closed, cx);
                            }))
                            .child(Icon::Close.render(12.)),
                    ),
            );
        }
        // The wrapper centers without a fixed half-width offset, so narrow hosts
        // retain equal 20px margins. Only toast cards receive pointer events.
        let anchor = div().absolute().left(px(20.)).right(px(20.)).flex();
        let anchor = match self.position {
            ToastPosition::BottomRight => anchor.bottom(px(20.)).justify_end(),
            ToastPosition::TopRight => anchor.top(px(20.)).justify_end(),
            ToastPosition::TopCenter => anchor.top(px(20.)).justify_center(),
        };
        deferred(anchor.child(stack)).with_priority(30)
    }
}
