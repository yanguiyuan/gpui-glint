use super::{DateTime, DateTimeRange, RangePicker, RangePickerEvent};
use crate::theme::ThemePreset;
use chrono::Timelike;
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Subscription, Window,
};

#[derive(Clone, Debug)]
pub enum DateTimePickerEvent {
    Changed(Option<DateTime>),
}
/// Single date and time in one calendar/time popup, at whole-second precision.
/// Wall-clock values have no timezone. Only user confirmation/clear emits Changed.
pub struct DateTimePicker {
    editor: Entity<RangePicker<DateTime>>,
    _subscription: Subscription,
}
impl EventEmitter<DateTimePickerEvent> for DateTimePicker {}
impl Focusable for DateTimePicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.read(cx).focus_handle(cx)
    }
}
impl DateTimePicker {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| RangePicker::new(cx).single());
        let subscription = cx.subscribe(&editor, |_, _, event: &RangePickerEvent<DateTime>, cx| {
            let RangePickerEvent::Changed(value) = event;
            cx.emit(DateTimePickerEvent::Changed(value.map(|v| v.start)));
            cx.notify();
        });
        Self {
            editor,
            _subscription: subscription,
        }
    }
    pub fn selected(mut self, value: DateTime, cx: &mut Context<Self>) -> Self {
        self.set_selected(Some(value), cx);
        self
    }
    pub fn range(
        mut self,
        min: Option<DateTime>,
        max: Option<DateTime>,
        cx: &mut Context<Self>,
    ) -> Self {
        self.set_range(min, max, cx);
        self
    }
    pub fn theme(mut self, theme: ThemePreset, cx: &mut Context<Self>) -> Self {
        self.set_theme(theme, cx);
        self
    }
    pub fn disabled(mut self, disabled: bool, cx: &mut Context<Self>) -> Self {
        self.set_disabled(disabled, cx);
        self
    }
    pub fn clearable(self, clearable: bool, cx: &mut Context<Self>) -> Self {
        self.editor
            .update(cx, |p, _| p.configure_clearable(clearable));
        self
    }
    pub fn placeholder(self, text: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        self.editor
            .update(cx, |p, _| p.configure_placeholder(text.into()));
        self
    }
    pub fn width(self, width: f32, cx: &mut Context<Self>) -> Self {
        self.editor.update(cx, |p, _| p.configure_width(width));
        self
    }
    pub fn value(&self, cx: &App) -> Option<DateTime> {
        self.editor.read(cx).value().map(|v| v.start)
    }
    pub fn set_selected(&mut self, value: Option<DateTime>, cx: &mut Context<Self>) {
        self.editor.update(cx, |p, cx| {
            p.set_selected(
                value.map(|v| DateTimeRange {
                    start: v.with_nanosecond(0).unwrap(),
                    end: v.with_nanosecond(0).unwrap(),
                }),
                cx,
            )
        });
        cx.notify();
    }
    pub fn set_range(
        &mut self,
        min: Option<DateTime>,
        max: Option<DateTime>,
        cx: &mut Context<Self>,
    ) {
        self.editor.update(cx, |p, cx| p.set_range(min, max, cx));
        cx.notify();
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.editor.update(cx, |p, cx| p.set_theme(theme, cx));
    }
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.editor.update(cx, |p, cx| p.set_disabled(disabled, cx));
    }
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.editor.update(cx, |p, cx| p.close(cx));
    }
}
impl Render for DateTimePicker {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.editor.clone()
    }
}
