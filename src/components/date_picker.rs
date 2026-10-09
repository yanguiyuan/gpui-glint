use super::Icon;
use crate::theme::ThemePreset;
pub use chrono::NaiveDate as Date;
use chrono::{Datelike, Local, Months};
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    deferred, div, prelude::*, px, rgb,
};

#[derive(Clone, Debug)]
pub enum DatePickerEvent {
    Changed(Option<Date>),
}
/// Single-date selection in the local calendar. Changed fires only for user edits.
pub struct DatePicker {
    value: Option<Date>,
    active: Date,
    month: Date,
    min: Option<Date>,
    max: Option<Date>,
    placeholder: SharedString,
    disabled: bool,
    clearable: bool,
    open: bool,
    width: f32,
    theme: ThemePreset,
    focus: FocusHandle,
}
impl EventEmitter<DatePickerEvent> for DatePicker {}
impl Focusable for DatePicker {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
fn first(date: Date) -> Date {
    date.with_day(1).unwrap()
}
fn calendar(month: Date) -> Vec<Option<Date>> {
    let offset = month.weekday().num_days_from_monday() as i64;
    (0..42)
        .map(|i| month.checked_add_signed(chrono::Duration::days(i - offset)))
        .collect()
}
fn allowed(date: Date, min: Option<Date>, max: Option<Date>) -> bool {
    min.is_none_or(|m| date >= m) && max.is_none_or(|m| date <= m)
}
impl DatePicker {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let today = Local::now().date_naive();
        Self {
            value: None,
            active: today,
            month: first(today),
            min: None,
            max: None,
            placeholder: "请选择日期".into(),
            disabled: false,
            clearable: true,
            open: false,
            width: 220.,
            theme: ThemePreset::default(),
            focus: cx.focus_handle(),
        }
    }
    pub fn selected(mut self, date: Date) -> Self {
        self.value = Some(date);
        self.active = date;
        self.month = first(date);
        self.normalize();
        self
    }
    /// Inclusive bounds. An inverted range disables all dates.
    pub fn range(mut self, min: Option<Date>, max: Option<Date>) -> Self {
        self.min = min;
        self.max = max;
        self.normalize();
        self
    }
    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = text.into();
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
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(140.);
        self
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }
    pub fn value(&self) -> Option<Date> {
        self.value
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            cx.notify();
        }
    }
    pub fn set_selected(&mut self, value: Option<Date>, cx: &mut Context<Self>) {
        self.value = value;
        self.normalize();
        cx.notify();
    }
    pub fn set_range(&mut self, min: Option<Date>, max: Option<Date>, cx: &mut Context<Self>) {
        self.min = min;
        self.max = max;
        self.normalize();
        cx.notify();
    }
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        if disabled {
            self.open = false;
        }
        cx.notify();
    }
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            cx.notify();
        }
    }
    fn normalize(&mut self) {
        self.value = self.value.filter(|d| allowed(*d, self.min, self.max));
        self.active = self.value.unwrap_or(self.active);
        if let Some(min) = self.min {
            self.active = self.active.max(min);
        }
        if let Some(max) = self.max {
            self.active = self.active.min(max);
        }
        self.month = first(self.active);
    }
    fn choose(&mut self, value: Option<Date>, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled
            || value.is_some_and(|d| !allowed(d, self.min, self.max))
            || (value.is_none() && !self.clearable)
        {
            return;
        }
        self.open = false;
        window.focus(&self.focus, cx);
        if self.value != value {
            self.value = value;
            self.normalize();
            cx.emit(DatePickerEvent::Changed(value));
        }
        cx.notify();
    }
    fn move_month(&mut self, forward: bool, cx: &mut Context<Self>) {
        let next = if forward {
            self.month.checked_add_months(Months::new(1))
        } else {
            self.month.checked_sub_months(Months::new(1))
        };
        if let Some(month) = next {
            self.month = month;
            self.active = month.with_day(self.active.day()).unwrap_or_else(|| {
                month
                    .checked_add_months(Months::new(1))
                    .and_then(|d| d.pred_opt())
                    .unwrap_or(month)
            });
            cx.notify();
        }
    }
    fn keyboard(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.disabled {
            return false;
        }
        if !self.open {
            if key != "enter" && key != "space" && key != "down" {
                return false;
            }
            self.open = true;
            self.normalize();
            cx.notify();
            return true;
        }
        match key {
            "escape" => self.close(cx),
            "enter" | "space" => self.choose(Some(self.active), window, cx),
            "pageup" => self.move_month(false, cx),
            "pagedown" => self.move_month(true, cx),
            "left" | "right" | "up" | "down" => {
                let step = match key {
                    "left" => -1,
                    "right" => 1,
                    "up" => -7,
                    _ => 7,
                };
                if let Some(date) = self.active.checked_add_signed(chrono::Duration::days(step)) {
                    if allowed(date, self.min, self.max) {
                        self.active = date;
                        self.month = first(date);
                        cx.notify();
                    }
                }
            }
            _ => return false,
        }
        true
    }
}
impl Render for DatePicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let mut root = div()
            .id("glass-date-picker")
            .relative()
            .w(px(self.width))
            .track_focus(&self.focus)
            .tab_index(if self.disabled { -1 } else { 0 })
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if this.keyboard(&event.keystroke.key, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .id("date-trigger")
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(rgb(if self.open { p.accent } else { p.border }))
                    .bg(rgb(p.surface))
                    .text_size(px(13.))
                    .text_color(rgb(if self.disabled || self.value.is_none() {
                        p.text_muted
                    } else {
                        p.text
                    }))
                    .when(!self.disabled, |e| {
                        e.cursor_pointer().hover(move |s| s.bg(rgb(p.panel)))
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, window, cx| {
                        if !this.disabled {
                            this.open = !this.open;
                            if this.open {
                                this.normalize();
                            }
                            window.focus(&this.focus, cx);
                            cx.notify();
                        }
                    }))
                    .child(
                        self.value
                            .map(|d| SharedString::from(d.format("%Y-%m-%d").to_string()))
                            .unwrap_or(self.placeholder.clone()),
                    )
                    .child(Icon::Calendar.render(14.)),
            );
        if self.open {
            let mut menu = div()
                .p_3()
                .rounded(px(12.))
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.surface))
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .mb_2()
                        .child(
                            div()
                                .id("previous-month")
                                .p_2()
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| this.move_month(false, cx)))
                                .child(Icon::ArrowLeft.render(14.)),
                        )
                        .child(format!(
                            "{} 年 {} 月",
                            self.month.year(),
                            self.month.month()
                        ))
                        .child(
                            div()
                                .id("next-month")
                                .p_2()
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| this.move_month(true, cx)))
                                .child(Icon::ChevronRight.render(14.)),
                        ),
                );
            let mut weekdays = div().flex();
            for day in ["一", "二", "三", "四", "五", "六", "日"] {
                weekdays = weekdays.child(
                    div()
                        .w(px(36.))
                        .h(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child(day),
                );
            }
            menu = menu.child(weekdays);
            let dates = calendar(self.month);
            let today = Local::now().date_naive();
            for week in dates.chunks(7) {
                let mut row = div().flex();
                for date in week {
                    let Some(date) = *date else {
                        row = row.child(div().size(px(36.)));
                        continue;
                    };
                    let enabled = allowed(date, self.min, self.max);
                    let selected = self.value == Some(date);
                    row = row.child(
                        div()
                            .id(("calendar-date", date.num_days_from_ce() as u64))
                            .size(px(36.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(7.))
                            .text_size(px(12.))
                            .bg(rgb(if selected { p.accent } else { p.surface }))
                            .text_color(rgb(if selected {
                                0xffffff
                            } else if !enabled || date.month() != self.month.month() {
                                p.text_muted
                            } else {
                                p.text
                            }))
                            .when(date == self.active, |e| {
                                e.border_1().border_color(rgb(p.accent))
                            })
                            .when(enabled, |e| {
                                e.cursor_pointer().hover(move |s| {
                                    s.bg(rgb(p.accent_tint)).text_color(rgb(p.text))
                                })
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose(Some(date), window, cx)
                            }))
                            .child(format!(
                                "{}{}",
                                date.day(),
                                if date == today { "·" } else { "" }
                            )),
                    );
                }
                menu = menu.child(row);
            }
            menu =
                menu.child(
                    div()
                        .mt_2()
                        .flex()
                        .justify_between()
                        .text_size(px(12.))
                        .child(
                            div()
                                .id("date-today")
                                .p_2()
                                .text_color(rgb(if allowed(today, self.min, self.max) {
                                    p.accent
                                } else {
                                    p.text_muted
                                }))
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.choose(Some(today), window, cx)
                                }))
                                .child("今天"),
                        )
                        .when(self.clearable, |e| {
                            e.child(
                                div()
                                    .id("date-clear")
                                    .p_2()
                                    .cursor_pointer()
                                    .text_color(rgb(p.text_muted))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.choose(None, window, cx)
                                    }))
                                    .child("清空"),
                            )
                        }),
                );
            root = root.child(
                deferred(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .w(px(278.))
                        .pt(px(36.))
                        .text_color(rgb(p.text))
                        .text_size(px(13.))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close(cx)))
                        .child(menu),
                )
                .with_priority(10),
            );
        }
        root
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Days;
    #[test]
    fn leap_year_and_week_alignment() {
        let month = Date::from_ymd_opt(2024, 2, 1).unwrap();
        let days = calendar(month);
        assert_eq!(days[0], Date::from_ymd_opt(2024, 1, 29));
        assert_eq!(days[31], Date::from_ymd_opt(2024, 2, 29));
        assert_eq!(days.len(), 42);
        assert!(Date::from_ymd_opt(2023, 2, 29).is_none());
    }
    #[test]
    fn inclusive_and_inverted_ranges() {
        let min = Date::from_ymd_opt(2026, 10, 4).unwrap();
        let max = min.checked_add_days(Days::new(4)).unwrap();
        assert!(allowed(min, Some(min), Some(max)));
        assert!(allowed(max, Some(min), Some(max)));
        assert!(!allowed(min.pred_opt().unwrap(), Some(min), Some(max)));
        assert!(!allowed(min, Some(max), Some(min)));
    }
    #[test]
    fn year_boundary_and_calendar_extremes() {
        assert_eq!(
            calendar(Date::from_ymd_opt(2026, 1, 1).unwrap())[0],
            Date::from_ymd_opt(2025, 12, 29)
        );
        assert!(calendar(first(Date::MIN)).iter().any(Option::is_none));
        assert_eq!(calendar(first(Date::MAX)).len(), 42);
    }
}
