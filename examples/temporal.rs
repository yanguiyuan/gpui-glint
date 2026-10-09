//! Run with `cargo run --example temporal`.
use gpui::{
    App, AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Render, Subscription,
    Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_glint::components::{
    Date, DateRange, DateRangePicker, DateTimePicker, DateTimePickerEvent, DateTimeRange,
    DateTimeRangePicker, GlintAssets, RangePickerEvent, TemporalValue, Time, TimePicker,
    TimePickerEvent, TimeRange, TimeRangePicker, Toast, ToastHost, ToastKind,
};
struct TemporalExample {
    time: Entity<TimePicker>,
    disabled_time: Entity<TimePicker>,
    datetime: Entity<DateTimePicker>,
    dates: Entity<DateRangePicker>,
    times: Entity<TimeRangePicker>,
    datetimes: Entity<DateTimeRangePicker>,
    toasts: Entity<ToastHost>,
    _subscriptions: Vec<Subscription>,
}
impl TemporalExample {
    fn new(cx: &mut Context<Self>) -> Self {
        let time =
            cx.new(|cx| TimePicker::new(cx).selected(Time::from_hms_opt(9, 30, 15).unwrap()));
        let disabled_time = cx.new(|cx| TimePicker::new(cx).disabled(true));
        let day = Date::from_ymd_opt(2026, 10, 5).unwrap();
        let datetime = cx.new(|cx| {
            DateTimePicker::new(cx)
                .selected(day.and_hms_opt(10, 30, 15).unwrap(), cx)
                .range(
                    Some(day.and_hms_opt(8, 0, 0).unwrap()),
                    Some(day.and_hms_opt(20, 0, 0).unwrap()),
                    cx,
                )
        });
        let dates = cx.new(|cx| {
            DateRangePicker::new(cx)
                .selected(DateRange::new(day, day.succ_opt().unwrap()).unwrap(), cx)
        });
        let times = cx.new(|cx| {
            TimeRangePicker::new(cx).selected(
                TimeRange::new(
                    Time::from_hms_opt(9, 0, 0).unwrap(),
                    Time::from_hms_opt(18, 0, 0).unwrap(),
                )
                .unwrap(),
                cx,
            )
        });
        let datetimes = cx.new(|cx| {
            DateTimeRangePicker::new(cx)
                .range(
                    Some(day.and_hms_opt(8, 30, 0).unwrap()),
                    Some(day.succ_opt().unwrap().and_hms_opt(1, 0, 0).unwrap()),
                    cx,
                )
                .selected(
                    DateTimeRange::new(
                        day.and_hms_opt(23, 59, 58).unwrap(),
                        day.succ_opt().unwrap().and_hms_opt(0, 0, 2).unwrap(),
                    )
                    .unwrap(),
                    cx,
                )
        });
        let toasts = cx.new(|_| ToastHost::new());
        let subscriptions = vec![
            cx.subscribe(&datetime, |this, _, event: &DateTimePickerEvent, cx| {
                let DateTimePickerEvent::Changed(value) = event;
                this.notify(
                    value
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "已清空日期时间".into()),
                    cx,
                );
            }),
            cx.subscribe(&time, |this, _, event: &TimePickerEvent, cx| {
                let TimePickerEvent::Changed(value) = event;
                this.notify(
                    value
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "已清空时间".into()),
                    cx,
                );
            }),
            cx.subscribe(&dates, |this, _, event: &RangePickerEvent<Date>, cx| {
                let RangePickerEvent::Changed(value) = event;
                this.notify(
                    value
                        .map(|v| format!("{} → {}", v.start.display(), v.end.display()))
                        .unwrap_or_else(|| "已清空日期范围".into()),
                    cx,
                );
            }),
            cx.subscribe(
                &times,
                |this, _, event: &gpui_glint::components::TimeRangePickerEvent, cx| {
                    let RangePickerEvent::Changed(value) = event;
                    this.notify(
                        value
                            .map(|v| format!("{} → {}", v.start.display(), v.end.display()))
                            .unwrap_or_else(|| "已清空时间范围".into()),
                        cx,
                    );
                },
            ),
            cx.subscribe(
                &datetimes,
                |this, _, event: &gpui_glint::components::DateTimeRangePickerEvent, cx| {
                    let RangePickerEvent::Changed(value) = event;
                    this.notify(
                        value
                            .map(|v| format!("{} → {}", v.start.display(), v.end.display()))
                            .unwrap_or_else(|| "已清空日期时间范围".into()),
                        cx,
                    );
                },
            ),
        ];
        Self {
            time,
            disabled_time,
            datetime,
            dates,
            times,
            datetimes,
            toasts,
            _subscriptions: subscriptions,
        }
    }
    fn notify(&mut self, message: String, cx: &mut Context<Self>) {
        self.toasts.update(cx, |host, cx| {
            host.push(Toast::new(message).kind(ToastKind::Success), cx);
        });
    }
}
impl Render for TemporalExample {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .bg(rgb(0xeff2ed))
            .text_color(rgb(0x1d1d1f))
            .child(
                div()
                    .id("temporal-demo")
                    .size_full()
                    .overflow_y_scroll()
                    .p_8()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(div().text_size(px(24.)).child("时间与范围选择"))
                    .child("统一浮层选择日期、时分秒和范围；确认后通过事件通知宿主。")
                    .child(
                        div()
                            .flex()
                            .gap_5()
                            .child(self.time.clone())
                            .child(self.disabled_time.clone()),
                    )
                    .child("日期时间 · 单值 · 08:00:00 至 20:00:00")
                    .child(self.datetime.clone())
                    .child("日期范围 · 年月日")
                    .child(self.dates.clone())
                    .child("时间范围 · 一天内的时分秒")
                    .child(self.times.clone())
                    .child("日期时间范围 · 精确到秒 · 限制到次日 01:00:00")
                    .child(self.datetimes.clone()),
            )
            .child(self.toasts.clone())
    }
}
fn main() {
    gpui_platform::application()
        .with_assets(GlintAssets)
        .run(|cx: &mut App| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1040.), px(820.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |_, cx| cx.new(TemporalExample::new),
            )
            .unwrap();
            cx.activate(true);
        });
}
