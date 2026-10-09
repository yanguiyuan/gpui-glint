//! Run with `cargo run --example feedback`.
use gpui::{
    App, AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Render, Subscription,
    Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use gpui_glint::components::{
    Button, ButtonKind, Date, DatePicker, DatePickerEvent, GlintAssets, Toast, ToastHost,
    ToastKind, ToastPosition,
};
use std::time::Duration;
struct FeedbackExample {
    dates: Vec<Entity<DatePicker>>,
    toasts: Entity<ToastHost>,
    _subscriptions: Vec<Subscription>,
}
impl FeedbackExample {
    fn new(cx: &mut Context<Self>) -> Self {
        let toasts = cx.new(|_| ToastHost::new());
        let dates = vec![
            cx.new(DatePicker::new),
            cx.new(|cx| {
                DatePicker::new(cx)
                    .range(
                        Date::from_ymd_opt(2026, 10, 1),
                        Date::from_ymd_opt(2026, 10, 31),
                    )
                    .selected(Date::from_ymd_opt(2026, 10, 4).unwrap())
            }),
            cx.new(|cx| DatePicker::new(cx).disabled(true)),
        ];
        let subscriptions = dates
            .iter()
            .map(|date| {
                cx.subscribe(date, |this, _, event: &DatePickerEvent, cx| {
                    let DatePickerEvent::Changed(value) = event;
                    let message = value
                        .map(|d| format!("已选择 {d}"))
                        .unwrap_or_else(|| "已清空日期".into());
                    this.toasts.update(cx, |host, cx| {
                        host.push(Toast::new(message).kind(ToastKind::Success), cx);
                    });
                })
            })
            .collect();
        Self {
            dates,
            toasts,
            _subscriptions: subscriptions,
        }
    }
}
impl Render for FeedbackExample {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut buttons = div().flex().gap_3();
        for (label, kind) in [
            ("信息", ToastKind::Info),
            ("成功", ToastKind::Success),
            ("警告", ToastKind::Warning),
            ("错误", ToastKind::Error),
        ] {
            buttons = buttons.child(
                Button::new(label)
                    .kind(ButtonKind::Glass)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toasts.update(cx, |host, cx| {
                            host.push(
                                Toast::new(label)
                                    .kind(kind)
                                    .description("悬停暂停自动关闭，移开后继续计时"),
                                cx,
                            );
                        });
                    })),
            );
        }
        buttons = buttons
            .child(
                Button::new("常驻通知")
                    .kind(ButtonKind::Glass)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toasts.update(cx, |host, cx| {
                            host.push(
                                Toast::new("修改未保存")
                                    .kind(ToastKind::Warning)
                                    .duration(None),
                                cx,
                            );
                        });
                    })),
            )
            .child(
                Button::new("快速关闭")
                    .kind(ButtonKind::Glass)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toasts.update(cx, |host, cx| {
                            host.push(
                                Toast::new("一秒后自动关闭").duration(Some(Duration::from_secs(1))),
                                cx,
                            );
                        });
                    })),
            );
        let mut positions = div().flex().gap_3();
        for (label, position) in [
            ("右下方", ToastPosition::BottomRight),
            ("右上方", ToastPosition::TopRight),
            ("正上方", ToastPosition::TopCenter),
        ] {
            positions = positions.child(
                Button::new(label)
                    .kind(ButtonKind::Glass)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toasts.update(cx, |host, cx| {
                            host.set_position(position, cx);
                            host.push(Toast::new(format!("通知位置：{label}")), cx);
                        });
                    })),
            );
        }
        div().relative().size_full().p_8().bg(gpui::rgb(0xeff2ed)).flex().flex_col().gap_5()
            .child(div().text_size(px(24.)).child("日期选择与通知"))
            .child("选择日期后由外部订阅事件弹出 Toast。方向键选择，PageUp / PageDown 翻月，Enter 确认，Esc 关闭。")
            .child(div().flex().gap_5().child(self.dates[0].clone()).child(self.dates[1].clone()).child(self.dates[2].clone()))
            .child(positions).child(buttons).child(self.toasts.clone())
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
                        size(px(980.), px(640.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |_, cx| cx.new(FeedbackExample::new),
            )
            .unwrap();
            cx.activate(true);
        });
}
