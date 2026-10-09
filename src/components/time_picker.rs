use super::{Button, ButtonKind, Icon};
use crate::theme::ThemePreset;
pub use chrono::NaiveTime as Time;
use chrono::{Local, Timelike};
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, ScrollHandle, SharedString, Styled, Window, deferred, div,
    point, prelude::*, px, rgb,
};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub enum TimePickerEvent {
    Changed(Option<Time>),
}
pub(crate) fn seconds(time: Time) -> Time {
    Time::from_hms_opt(time.hour(), time.minute(), time.second()).unwrap()
}
pub(crate) fn allowed(time: Time, min: Option<Time>, max: Option<Time>) -> bool {
    min.is_none_or(|m| time >= m) && max.is_none_or(|m| time <= m)
}
fn parts(time: Time) -> [u32; 3] {
    [time.hour(), time.minute(), time.second()]
}
fn replace(time: Time, column: usize, value: u32) -> Option<Time> {
    let mut p = parts(time);
    *p.get_mut(column)? = value;
    Time::from_hms_opt(p[0], p[1], p[2])
}
/// Select a wall-clock time at whole-second precision, in 24-hour format.
/// Column edits are drafts; Confirm emits Changed. Escape/outside clicks discard the draft.
pub struct TimePicker {
    value: Option<Time>,
    draft: Time,
    min: Option<Time>,
    max: Option<Time>,
    open: bool,
    disabled: bool,
    clearable: bool,
    placeholder: SharedString,
    width: f32,
    theme: ThemePreset,
    focus: FocusHandle,
    active: usize,
    scroll: [ScrollHandle; 3],
    scroll_pending: bool,
    digits: Option<(String, Instant)>,
}
impl EventEmitter<TimePickerEvent> for TimePicker {}
impl Focusable for TimePicker {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl TimePicker {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            value: None,
            draft: seconds(Local::now().time()),
            min: None,
            max: None,
            open: false,
            disabled: false,
            clearable: true,
            placeholder: "请选择时间".into(),
            width: 160.,
            theme: ThemePreset::default(),
            focus: cx.focus_handle(),
            active: 0,
            scroll: std::array::from_fn(|_| ScrollHandle::new()),
            scroll_pending: true,
            digits: None,
        }
    }
    pub fn selected(mut self, time: Time) -> Self {
        self.value = Some(seconds(time));
        self.normalize();
        self
    }
    pub fn range(mut self, min: Option<Time>, max: Option<Time>) -> Self {
        self.min = min.map(seconds);
        self.max = max.map(seconds);
        self.normalize();
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn clearable(mut self, clearable: bool) -> Self {
        self.clearable = clearable;
        self
    }
    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = text.into();
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = if width.is_finite() {
            width.max(120.)
        } else {
            160.
        };
        self
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }
    pub fn value(&self) -> Option<Time> {
        self.value
    }
    pub fn set_selected(&mut self, value: Option<Time>, cx: &mut Context<Self>) {
        self.value = value.map(seconds);
        self.normalize();
        cx.notify();
    }
    pub fn set_range(&mut self, min: Option<Time>, max: Option<Time>, cx: &mut Context<Self>) {
        self.min = min.map(seconds);
        self.max = max.map(seconds);
        self.normalize();
        cx.notify();
    }
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        if disabled {
            self.close(cx);
        }
        cx.notify();
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            cx.notify();
        }
    }
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            self.digits = None;
            self.normalize();
            cx.notify();
        }
    }
    fn normalize(&mut self) {
        self.value = self.value.filter(|t| allowed(*t, self.min, self.max));
        self.draft = self.value.unwrap_or(self.draft);
        if let Some(min) = self.min {
            self.draft = self.draft.max(min);
        }
        if let Some(max) = self.max {
            self.draft = self.draft.min(max);
        }
        self.scroll_pending = true;
    }
    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        self.draft = self.value.unwrap_or_else(|| seconds(Local::now().time()));
        self.normalize();
        self.active = 0;
        self.digits = None;
        self.open = true;
        window.focus(&self.focus, cx);
        cx.notify();
    }
    fn choose(&mut self, value: Option<Time>, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled
            || value.is_some_and(|t| !allowed(t, self.min, self.max))
            || (value.is_none() && !self.clearable)
        {
            return;
        }
        self.open = false;
        self.digits = None;
        window.focus(&self.focus, cx);
        if self.value != value {
            self.value = value;
            cx.emit(TimePickerEvent::Changed(value));
        }
        cx.notify();
    }
    fn edit(&mut self, column: usize, value: u32, cx: &mut Context<Self>) {
        if let Some(time) =
            replace(self.draft, column, value).filter(|t| allowed(*t, self.min, self.max))
        {
            self.draft = time;
            self.active = column;
            self.scroll_pending = true;
            cx.notify();
        }
    }
    fn keyboard(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.disabled {
            return false;
        }
        if !self.open {
            if !matches!(key, "enter" | "space" | "down") {
                return false;
            }
            self.open(window, cx);
            return true;
        }
        match key {
            "escape" => self.close(cx),
            "enter" => self.choose(Some(self.draft), window, cx),
            "left" | "right" => {
                self.active = if key == "left" {
                    self.active.saturating_sub(1)
                } else {
                    (self.active + 1).min(2)
                };
                self.digits = None;
                cx.notify();
            }
            "up" | "down" | "home" | "end" => {
                self.digits = None;
                let max = if self.active == 0 { 23 } else { 59 };
                let current = parts(self.draft)[self.active];
                let value = match key {
                    "home" => 0,
                    "end" => max,
                    "up" => (current + max) % (max + 1),
                    _ => (current + 1) % (max + 1),
                };
                self.edit(self.active, value, cx);
            }
            _ if key.len() == 1 && key.as_bytes()[0].is_ascii_digit() => {
                let mut text = self
                    .digits
                    .take()
                    .filter(|(s, t)| s.len() < 2 && t.elapsed() < Duration::from_secs(1))
                    .map(|(s, _)| s)
                    .unwrap_or_default();
                text.push_str(key);
                let max = if self.active == 0 { 23 } else { 59 };
                if text.parse::<u32>().unwrap() > max {
                    text = key.to_owned();
                }
                self.edit(self.active, text.parse().unwrap(), cx);
                self.digits = Some((text, Instant::now()));
            }
            _ => return false,
        }
        true
    }
}
impl Render for TimePicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let mut root = div()
            .id("glass-time-picker")
            .relative()
            .w(px(self.width))
            .flex_shrink_0()
            .track_focus(&self.focus)
            .tab_index(if self.disabled { -1 } else { 0 })
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if this.keyboard(&event.keystroke.key, window, cx) {
                    cx.stop_propagation();
                    window.prevent_default();
                }
            }))
            .child(
                div()
                    .id("time-trigger")
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(rgb(if self.open { p.accent } else { p.border }))
                    .bg(rgb(p.surface))
                    .text_size(px(13.))
                    .text_color(rgb(if self.value.is_none() || self.disabled {
                        p.text_muted
                    } else {
                        p.text
                    }))
                    .when(!self.disabled, |el| {
                        el.cursor_pointer().hover(|s| s.bg(rgb(p.panel)))
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.open {
                            this.close(cx);
                        } else {
                            this.open(window, cx);
                        }
                    }))
                    .child(
                        self.value
                            .map(|t| SharedString::from(t.format("%H:%M:%S").to_string()))
                            .unwrap_or(self.placeholder.clone()),
                    )
                    .child(Icon::Clock.render(14.)),
            );
        if self.open {
            let values = parts(self.draft);
            let mut columns = div().flex().gap_2();
            for (column, label) in ["时", "分", "秒"].into_iter().enumerate() {
                if self.scroll_pending {
                    self.scroll[column]
                        .set_offset(point(px(0.), px(-(values[column] as f32 * 32.))));
                }
                let mut list = div()
                    .id(("time-column", column))
                    .h(px(180.))
                    .w(px(64.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll[column])
                    .flex()
                    .flex_col()
                    .gap_1();
                for value in 0..if column == 0 { 24 } else { 60 } {
                    let enabled = replace(self.draft, column, value)
                        .is_some_and(|t| allowed(t, self.min, self.max));
                    let selected = value == values[column];
                    list = list.child(
                        div()
                            .id(("time-option", (column * 60 + value as usize)))
                            .h(px(28.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .text_size(px(13.))
                            .bg(rgb(if selected { p.accent_tint } else { p.surface }))
                            .text_color(rgb(if !enabled {
                                p.text_muted
                            } else if selected {
                                p.accent
                            } else {
                                p.text
                            }))
                            .when(enabled, |el| {
                                el.cursor_pointer().hover(|s| s.bg(rgb(p.panel)))
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if enabled {
                                    this.digits = None;
                                    this.edit(column, value, cx);
                                    window.focus(&this.focus, cx);
                                }
                            }))
                            .child(format!("{value:02}")),
                    );
                }
                columns = columns.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(if self.active == column {
                                    p.accent
                                } else {
                                    p.text_muted
                                }))
                                .child(label),
                        )
                        .child(list),
                );
            }
            self.scroll_pending = false;
            let valid = allowed(self.draft, self.min, self.max);
            let menu = div()
                .p_3()
                .rounded(px(12.))
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.surface))
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(columns)
                .child(
                    div()
                        .mt_3()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            Button::new("现在")
                                .kind(ButtonKind::Plain)
                                .render()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.draft = seconds(Local::now().time());
                                    if let Some(min) = this.min {
                                        this.draft = this.draft.max(min);
                                    }
                                    if let Some(max) = this.max {
                                        this.draft = this.draft.min(max);
                                    }
                                    this.scroll_pending = true;
                                    cx.notify();
                                })),
                        )
                        .when(self.clearable, |el| {
                            el.child(
                                Button::new("清空")
                                    .kind(ButtonKind::Glass)
                                    .render()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.choose(None, window, cx)
                                    })),
                            )
                        })
                        .child(
                            Button::new("确定")
                                .render()
                                .when(!valid, |el| el.opacity(0.4))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.choose(Some(this.draft), window, cx)
                                })),
                        ),
                );
            root = root.child(
                deferred(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .w(px(248.))
                        .pt(px(36.))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close(cx)))
                        .child(menu),
                )
                .with_priority(11),
            );
        }
        root
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn second_precision_and_day_boundaries() {
        let t = Time::from_hms_milli_opt(23, 59, 59, 321).unwrap();
        assert_eq!(seconds(t).nanosecond(), 0);
        assert_eq!(replace(t, 2, 0), Time::from_hms_opt(23, 59, 0));
        assert!(replace(t, 0, 24).is_none());
        assert!(replace(t, 1, 60).is_none());
    }
    #[test]
    fn inclusive_bounds_and_inverted_range() {
        let min = Time::from_hms_opt(9, 30, 0).unwrap();
        let max = Time::from_hms_opt(18, 0, 0).unwrap();
        assert!(allowed(min, Some(min), Some(max)));
        assert!(allowed(max, Some(min), Some(max)));
        assert!(!allowed(
            Time::from_hms_opt(9, 29, 59).unwrap(),
            Some(min),
            Some(max)
        ));
        assert!(!allowed(min, Some(max), Some(min)));
    }
}
