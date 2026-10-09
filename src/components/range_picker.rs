use super::{Button, ButtonKind, Date, Icon, Time};
use crate::theme::ThemePreset;
pub use chrono::NaiveDateTime as DateTime;
use chrono::{Datelike, Local, Months, Timelike};
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement,
    Styled, Window, anchored, deferred, div, point, prelude::*, px, rgb,
};

mod sealed {
    pub trait Sealed {}
    impl Sealed for chrono::NaiveDate {}
    impl Sealed for chrono::NaiveTime {}
    impl Sealed for chrono::NaiveDateTime {}
}
/// Supported values are Date, Time, and DateTime. Datetimes are wall-clock values without a timezone.
pub trait TemporalValue: sealed::Sealed + Copy + Ord + std::fmt::Debug + 'static {
    const HAS_DATE: bool;
    const HAS_TIME: bool;
    fn parts(self) -> (Option<Date>, Option<Time>);
    fn assemble(date: Option<Date>, time: Option<Time>) -> Option<Self>;
    fn display(self) -> String;
    fn normalized(self) -> Self;
}
impl TemporalValue for Date {
    const HAS_DATE: bool = true;
    const HAS_TIME: bool = false;
    fn parts(self) -> (Option<Date>, Option<Time>) {
        (Some(self), None)
    }
    fn assemble(date: Option<Date>, _: Option<Time>) -> Option<Self> {
        date
    }
    fn display(self) -> String {
        self.format("%Y-%m-%d").to_string()
    }
    fn normalized(self) -> Self {
        self
    }
}
impl TemporalValue for Time {
    const HAS_DATE: bool = false;
    const HAS_TIME: bool = true;
    fn parts(self) -> (Option<Date>, Option<Time>) {
        (None, Some(self))
    }
    fn assemble(_: Option<Date>, time: Option<Time>) -> Option<Self> {
        time
    }
    fn display(self) -> String {
        self.format("%H:%M:%S").to_string()
    }
    fn normalized(self) -> Self {
        super::time_picker::seconds(self)
    }
}
impl TemporalValue for DateTime {
    const HAS_DATE: bool = true;
    const HAS_TIME: bool = true;
    fn parts(self) -> (Option<Date>, Option<Time>) {
        (Some(self.date()), Some(self.time()))
    }
    fn assemble(date: Option<Date>, time: Option<Time>) -> Option<Self> {
        Some(date?.and_time(time?))
    }
    fn display(self) -> String {
        self.format("%Y-%m-%d %H:%M:%S").to_string()
    }
    fn normalized(self) -> Self {
        self.with_nanosecond(0).unwrap()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueRange<T> {
    pub start: T,
    pub end: T,
}
impl<T: TemporalValue> ValueRange<T> {
    /// Equal endpoints are valid. Inverted ranges are rejected, rather than silently reordered.
    pub fn new(start: T, end: T) -> Option<Self> {
        let (start, end) = (start.normalized(), end.normalized());
        (start <= end).then_some(Self { start, end })
    }
}
pub type DateRange = ValueRange<Date>;
pub type TimeRange = ValueRange<Time>;
pub type DateTimeRange = ValueRange<DateTime>;
#[derive(Clone, Debug)]
pub enum RangePickerEvent<T> {
    Changed(Option<ValueRange<T>>),
}
pub type DateRangePicker = RangePicker<Date>;
pub type TimeRangePicker = RangePicker<Time>;
pub type DateTimeRangePicker = RangePicker<DateTime>;
pub type DateRangePickerEvent = RangePickerEvent<Date>;
pub type TimeRangePickerEvent = RangePickerEvent<Time>;
pub type DateTimeRangePickerEvent = RangePickerEvent<DateTime>;

/// A unified range field with one popup and one transaction. No nested picker popups.
pub struct RangePicker<T: TemporalValue> {
    value: Option<ValueRange<T>>,
    draft: [Option<T>; 2],
    month: [Date; 2],
    min: Option<T>,
    max: Option<T>,
    disabled: bool,
    clearable: bool,
    theme: ThemePreset,
    labels: [SharedString; 2],
    placeholder: SharedString,
    width: f32,
    open: bool,
    single: bool,
    active: usize,
    column: usize,
    focus: FocusHandle,
    scroll: [[ScrollHandle; 3]; 2],
    scroll_pending: bool,
}
impl<T: TemporalValue> EventEmitter<RangePickerEvent<T>> for RangePicker<T> {}
impl<T: TemporalValue> Focusable for RangePicker<T> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
fn valid<T: TemporalValue>(range: ValueRange<T>, min: Option<T>, max: Option<T>) -> bool {
    range.start <= range.end
        && min.is_none_or(|m| range.start >= m)
        && max.is_none_or(|m| range.end <= m)
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
fn time_parts(time: Time) -> [u32; 3] {
    [time.hour(), time.minute(), time.second()]
}
impl<T: TemporalValue> RangePicker<T> {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let month = first(Local::now().date_naive());
        Self {
            value: None,
            draft: [None, None],
            month: [
                month,
                month.checked_add_months(Months::new(1)).unwrap_or(month),
            ],
            min: None,
            max: None,
            disabled: false,
            clearable: true,
            theme: ThemePreset::default(),
            labels: ["开始".into(), "结束".into()],
            placeholder: "请选择范围".into(),
            width: if T::HAS_DATE && T::HAS_TIME {
                420.
            } else if T::HAS_DATE {
                300.
            } else {
                260.
            },
            open: false,
            single: false,
            active: 0,
            column: 0,
            focus: cx.focus_handle(),
            scroll: std::array::from_fn(|_| std::array::from_fn(|_| ScrollHandle::new())),
            scroll_pending: true,
        }
    }
    pub(crate) fn single(mut self) -> Self {
        self.single = true;
        self.width = 240.;
        self.placeholder = "请选择日期时间".into();
        self
    }
    pub(crate) fn configure_clearable(&mut self, value: bool) {
        self.clearable = value;
    }
    pub(crate) fn configure_placeholder(&mut self, value: SharedString) {
        self.placeholder = value;
    }
    pub(crate) fn configure_width(&mut self, value: f32) {
        self.width = value.max(160.);
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
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
    pub fn labels(mut self, start: impl Into<SharedString>, end: impl Into<SharedString>) -> Self {
        self.labels = [start.into(), end.into()];
        self
    }
    pub fn placeholder(mut self, text: impl Into<SharedString>) -> Self {
        self.placeholder = text.into();
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(160.);
        self
    }
    pub fn selected(mut self, value: ValueRange<T>, cx: &mut Context<Self>) -> Self {
        self.set_selected(Some(value), cx);
        self
    }
    pub fn range(mut self, min: Option<T>, max: Option<T>, cx: &mut Context<Self>) -> Self {
        self.set_range(min, max, cx);
        self
    }
    pub fn value(&self) -> Option<ValueRange<T>> {
        self.value
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            cx.notify();
        }
    }
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        if disabled {
            self.close(cx);
        }
        cx.notify();
    }
    pub fn set_selected(&mut self, value: Option<ValueRange<T>>, cx: &mut Context<Self>) {
        self.value = value
            .and_then(|v| ValueRange::new(v.start, v.end))
            .filter(|v| valid(*v, self.min, self.max));
        self.restore();
        cx.notify();
    }
    pub fn set_range(&mut self, min: Option<T>, max: Option<T>, cx: &mut Context<Self>) {
        self.min = min.map(TemporalValue::normalized);
        self.max = max.map(TemporalValue::normalized);
        if self.value.is_some_and(|v| !valid(v, self.min, self.max)) {
            self.value = None;
        }
        self.restore();
        cx.notify();
    }
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        self.restore();
        cx.notify();
    }
    pub fn reset_draft(&mut self, cx: &mut Context<Self>) {
        self.restore();
        cx.notify();
    }
    fn restore(&mut self) {
        self.draft = self
            .value
            .map(|v| [Some(v.start), Some(v.end)])
            .unwrap_or([None, None]);
        let date = self.draft[0]
            .and_then(|v| v.parts().0)
            .or_else(|| self.min.and_then(|v| v.parts().0))
            .unwrap_or_else(|| Local::now().date_naive());
        self.month[0] = first(date);
        self.month[1] = self.draft[1]
            .and_then(|v| v.parts().0)
            .map(first)
            .unwrap_or_else(|| {
                self.month[0]
                    .checked_add_months(Months::new(1))
                    .unwrap_or(self.month[0])
            });
        self.scroll_pending = true;
    }
    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        self.restore();
        self.open = true;
        self.active = 0;
        self.column = 0;
        window.focus(&self.focus, cx);
        cx.notify();
    }
    fn draft_range(&self) -> Option<ValueRange<T>> {
        Some(ValueRange {
            start: self.draft[0]?,
            end: if self.single {
                self.draft[0]?
            } else {
                self.draft[1]?
            },
        })
    }
    fn commit(&mut self, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let Some(v) = self.draft_range().filter(|v| valid(*v, self.min, self.max)) else {
            return;
        };
        self.open = false;
        if self.value != Some(v) {
            self.value = Some(v);
            cx.emit(RangePickerEvent::Changed(self.value));
        }
        self.restore();
        cx.notify();
    }
    fn clear(&mut self, cx: &mut Context<Self>) {
        if self.disabled || !self.clearable {
            return;
        }
        let changed = self.value.is_some();
        self.value = None;
        self.close(cx);
        if changed {
            cx.emit(RangePickerEvent::Changed(None));
        }
    }
    fn base(&self, index: usize) -> (Date, Time) {
        let now = Local::now();
        let (date, time) = self.draft[index]
            .map(TemporalValue::parts)
            .unwrap_or((None, None));
        let mut date = date.unwrap_or_else(|| now.date_naive());
        if let Some(min) = self.min.and_then(|v| v.parts().0) {
            date = date.max(min);
        }
        if let Some(max) = self.max.and_then(|v| v.parts().0) {
            date = date.min(max);
        }
        let mut time = time
            .unwrap_or_else(|| Time::from_hms_opt(now.hour(), now.minute(), now.second()).unwrap());
        if self.draft[index].is_none() {
            if let Some(mut value) = T::assemble(Some(date), Some(time)) {
                if let Some(min) = self.min {
                    value = value.max(min);
                }
                if let Some(max) = self.max {
                    value = value.min(max);
                }
                let parts = value.parts();
                date = parts.0.unwrap_or(date);
                time = parts.1.unwrap_or(time);
            }
        }
        (date, time)
    }

    fn within(&self, value: T) -> bool {
        self.min.is_none_or(|m| value >= m) && self.max.is_none_or(|m| value <= m)
    }
    fn date_enabled(&self, date: Date) -> bool {
        self.min.and_then(|v| v.parts().0).is_none_or(|m| date >= m)
            && self.max.and_then(|v| v.parts().0).is_none_or(|m| date <= m)
    }
    fn edit_date(&mut self, index: usize, date: Date, cx: &mut Context<Self>) {
        if !self.date_enabled(date) {
            return;
        }
        let (_, time) = self.base(index);
        if let Some(mut value) = T::assemble(Some(date), Some(time)) {
            // Clamp time only on an inclusive boundary day, while retaining the chosen date.
            if let Some(min) = self.min {
                if min.parts().0 == Some(date) && value < min {
                    value = min;
                }
            }
            if let Some(max) = self.max {
                if max.parts().0 == Some(date) && value > max {
                    value = max;
                }
            }
            self.draft[index] = Some(value);
            self.month[index] = first(date);
            self.active = index;
            self.scroll_pending = true;
            cx.notify();
        }
    }
    fn time_candidate(&self, index: usize, column: usize, value: u32) -> Option<T> {
        let (date, time) = self.base(index);
        let mut parts = time_parts(time);
        parts[column] = value;
        T::assemble(
            Some(date),
            Some(Time::from_hms_opt(parts[0], parts[1], parts[2])?),
        )
    }
    fn edit_time(&mut self, index: usize, column: usize, value: u32, cx: &mut Context<Self>) {
        if let Some(value) = self
            .time_candidate(index, column, value)
            .filter(|v| self.within(*v))
        {
            self.draft[index] = Some(value);
            self.active = index;
            self.column = column;
            self.scroll_pending = true;
            cx.notify();
        }
    }
    fn move_month(&mut self, index: usize, forward: bool, cx: &mut Context<Self>) {
        let next = if forward {
            self.month[index].checked_add_months(Months::new(1))
        } else {
            self.month[index].checked_sub_months(Months::new(1))
        };
        if let Some(date) = next {
            self.month[index] = date;
            cx.notify();
        }
    }
    fn keyboard(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.disabled {
            return false;
        }
        if !self.open {
            if matches!(key, "enter" | "space" | "down") {
                self.open(window, cx);
                return true;
            }
            return false;
        }
        match key {
            "escape" => self.close(cx),
            "enter" => self.commit(cx),
            "tab" if !self.single => {
                self.active = 1 - self.active;
                cx.notify();
            }
            "pageup" if T::HAS_DATE => self.move_month(self.active, false, cx),
            "pagedown" if T::HAS_DATE => self.move_month(self.active, true, cx),
            "left" | "right" if T::HAS_TIME => {
                self.column = if key == "left" {
                    self.column.saturating_sub(1)
                } else {
                    (self.column + 1).min(2)
                };
                cx.notify();
            }
            "up" | "down" if T::HAS_TIME => {
                let current = time_parts(self.base(self.active).1)[self.column];
                let count = if self.column == 0 { 24 } else { 60 };
                let next = if key == "up" {
                    (current + count - 1) % count
                } else {
                    (current + 1) % count
                };
                self.edit_time(self.active, self.column, next, cx);
            }
            "left" | "right" | "up" | "down" if T::HAS_DATE => {
                let step = match key {
                    "left" => -1,
                    "right" => 1,
                    "up" => -7,
                    _ => 7,
                };
                if let Some(date) = self
                    .base(self.active)
                    .0
                    .checked_add_signed(chrono::Duration::days(step))
                {
                    self.edit_date(self.active, date, cx);
                }
            }
            _ => return false,
        };
        true
    }
    fn calendar_panel(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let month = self.month[index];
        let mut panel = div().w(px(252.)).flex().flex_col().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .mb_2()
                .child(
                    div()
                        .id(("prev-month", index))
                        .p_2()
                        .cursor_pointer()
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.move_month(index, false, cx)),
                        )
                        .child(Icon::ArrowLeft.render(14.)),
                )
                .child(format!("{} 年 {} 月", month.year(), month.month()))
                .child(
                    div()
                        .id(("next-month", index))
                        .p_2()
                        .cursor_pointer()
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.move_month(index, true, cx)),
                        )
                        .child(Icon::ChevronRight.render(14.)),
                ),
        );
        let mut weekdays = div().flex();
        for day in ["一", "二", "三", "四", "五", "六", "日"] {
            weekdays = weekdays.child(
                div()
                    .w(px(36.))
                    .h(px(24.))
                    .flex()
                    .justify_center()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(day),
            );
        }
        panel = panel.child(weekdays);
        let range = self.draft_range();
        for week in calendar(month).chunks(7) {
            let mut row = div().flex();
            for date in week {
                let Some(date) = *date else {
                    row = row.child(div().size(px(36.)));
                    continue;
                };
                let enabled = self.date_enabled(date);
                let selected = self
                    .draft
                    .iter()
                    .take(if self.single { 1 } else { 2 })
                    .flatten()
                    .any(|v| v.parts().0 == Some(date));
                let interior = range.is_some_and(|v| {
                    v.start.parts().0.is_some_and(|s| date >= s)
                        && v.end.parts().0.is_some_and(|e| date <= e)
                });
                row = row.child(
                    div()
                        .id((
                            "range-day",
                            index as u64 * 1_000_000_000
                                + (date.num_days_from_ce() as i64 + 100_000_000) as u64,
                        ))
                        .size(px(36.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .text_size(px(12.))
                        .bg(rgb(if selected {
                            p.accent
                        } else if interior {
                            p.accent_tint
                        } else {
                            p.surface
                        }))
                        .text_color(rgb(if selected {
                            0xffffff
                        } else if !enabled || date.month() != month.month() {
                            p.text_muted
                        } else {
                            p.text
                        }))
                        .when(enabled, |el| {
                            el.cursor_pointer()
                                .hover(move |s| s.bg(rgb(p.accent_tint)).text_color(rgb(p.text)))
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit_date(index, date, cx);
                            window.focus(&this.focus, cx);
                        }))
                        .child(date.day().to_string()),
                );
            }
            panel = panel.child(row);
        }
        panel
    }
    fn time_panel(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let values = time_parts(self.base(index).1);
        let mut columns = div().flex().gap_2();
        for (column, label) in ["时", "分", "秒"].into_iter().enumerate() {
            if self.scroll_pending {
                self.scroll[index][column]
                    .set_offset(point(px(0.), px(-(values[column] as f32 * 28.))));
            }
            let mut list = div()
                .id(("range-time-column", index * 3 + column))
                .h(px(140.))
                .w(px(76.))
                .overflow_y_scroll()
                .track_scroll(&self.scroll[index][column])
                .flex()
                .flex_col();
            for value in 0..if column == 0 { 24 } else { 60 } {
                let enabled = self
                    .time_candidate(index, column, value)
                    .is_some_and(|v| self.within(v));
                list = list.child(
                    div()
                        .id(("range-time", index * 180 + column * 60 + value as usize))
                        .h(px(28.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .text_size(px(13.))
                        .bg(rgb(if value == values[column] {
                            p.accent_tint
                        } else {
                            p.surface
                        }))
                        .text_color(rgb(if !enabled {
                            p.text_muted
                        } else if value == values[column] {
                            p.accent
                        } else {
                            p.text
                        }))
                        .when(enabled, |el| {
                            el.cursor_pointer().hover(|s| s.bg(rgb(p.panel)))
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.edit_time(index, column, value, cx);
                            window.focus(&this.focus, cx);
                        }))
                        .child(format!("{value:02}")),
                );
            }
            columns = columns.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(if self.active == index && self.column == column {
                                p.accent
                            } else {
                                p.text_muted
                            }))
                            .child(label),
                    )
                    .child(list),
            );
        }
        columns
    }
}
impl<T: TemporalValue> Render for RangePicker<T> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let p = self.theme.palette();
        let display = self
            .value
            .map(|v| {
                if self.single {
                    v.start.display()
                } else {
                    format!("{} → {}", v.start.display(), v.end.display())
                }
            })
            .map(SharedString::from)
            .unwrap_or(self.placeholder.clone());
        let mut root = div()
            .id("glass-range-picker")
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
                    .id("range-trigger")
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
                    .when(!self.disabled, |el| {
                        el.cursor_pointer().hover(move |s| s.bg(rgb(p.panel)))
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.open {
                            this.close(cx);
                        } else {
                            this.open(window, cx);
                        }
                    }))
                    .child(display)
                    .child(
                        if T::HAS_DATE {
                            Icon::Calendar
                        } else {
                            Icon::Clock
                        }
                        .render(14.),
                    ),
            );
        if self.open {
            let count = if self.single { 1 } else { 2 };
            let mut panels = div().flex().gap_4();
            for index in 0..count {
                panels = panels.child(
                    div()
                        .w(px(252.))
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .id(("range-endpoint", index))
                                .rounded(px(6.))
                                .p_2()
                                .bg(rgb(if self.active == index {
                                    p.accent_tint
                                } else {
                                    p.panel
                                }))
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.active = index;
                                    window.focus(&this.focus, cx);
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(p.text_muted))
                                        .child(if self.single {
                                            SharedString::from("日期时间")
                                        } else {
                                            self.labels[index].clone()
                                        }),
                                )
                                .child(
                                    self.draft[index]
                                        .map(|v| v.display())
                                        .unwrap_or_else(|| "请选择".into()),
                                ),
                        )
                        .when(T::HAS_DATE, |el| el.child(self.calendar_panel(index, cx)))
                        .when(T::HAS_TIME, |el| el.child(self.time_panel(index, cx))),
                );
            }
            self.scroll_pending = false;
            let draft = self.draft_range();
            let acceptable = draft.is_some_and(|v| valid(v, self.min, self.max));
            let message = match draft {
                Some(v) if v.start > v.end => "开始不能晚于结束",
                Some(v) if !valid(v, self.min, self.max) => "所选范围超出可选区间",
                None => "请选择完整值",
                _ => "",
            };
            let menu = div()
                .p_3()
                .rounded(px(12.))
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.surface))
                .shadow_lg()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(panels)
                .child(
                    div()
                        .h(px(24.))
                        .mt_2()
                        .text_size(px(11.))
                        .text_color(rgb(if draft.is_some() {
                            theme.color(0xd9342b)
                        } else {
                            p.text_muted
                        }))
                        .child(message),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_2()
                        .when(self.clearable, |el| {
                            el.child(
                                Button::new("清空")
                                    .kind(ButtonKind::Plain)
                                    .render()
                                    .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
                            )
                        })
                        .child(
                            Button::new("取消")
                                .kind(ButtonKind::Glass)
                                .render()
                                .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
                        )
                        .child(
                            Button::new("确定")
                                .render()
                                .when(!acceptable, |el| el.opacity(0.4))
                                .on_click(cx.listener(|this, _, _, cx| this.commit(cx))),
                        ),
                );
            root = root.child(
                deferred(
                    anchored().snap_to_window_with_margin(px(8.)).child(
                        div()
                            .pt(px(36.))
                            .w(px(if self.single { 278. } else { 546. }))
                            .text_color(rgb(p.text))
                            .text_size(px(13.))
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close(cx)))
                            .child(menu),
                    ),
                )
                .with_priority(12),
            );
        }
        root
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates_and_time_ranges_reject_reversed_endpoints() {
        let d = Date::from_ymd_opt(2026, 10, 5).unwrap();
        assert!(DateRange::new(d, d).is_some());
        assert!(DateRange::new(d, d.pred_opt().unwrap()).is_none());
        let early = Time::from_hms_opt(0, 0, 0).unwrap();
        let late = Time::from_hms_opt(23, 59, 59).unwrap();
        assert!(TimeRange::new(early, late).is_some());
        assert!(TimeRange::new(late, early).is_none());
    }
    #[test]
    fn datetimes_allow_cross_midnight_and_boundaries() {
        let d = Date::from_ymd_opt(2024, 2, 29).unwrap();
        let start = d.and_hms_opt(23, 59, 59).unwrap();
        let end = d.succ_opt().unwrap().and_hms_opt(0, 0, 0).unwrap();
        let range = DateTimeRange::new(start, end).unwrap();
        assert!(valid(range, Some(start), Some(end)));
        assert!(!valid(range, Some(end), Some(start)));
        assert_eq!(end.date(), Date::from_ymd_opt(2024, 3, 1).unwrap());
    }
    #[test]
    fn datetime_values_normalize_fractional_seconds() {
        let d = Date::from_ymd_opt(2026, 10, 5).unwrap();
        let start = d.and_hms_milli_opt(12, 0, 0, 500).unwrap();
        let range = DateTimeRange::new(start, start).unwrap();
        assert_eq!(range.start.nanosecond(), 0);
    }
}
