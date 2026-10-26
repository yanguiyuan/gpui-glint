use super::Icon;
use crate::theme::ThemePreset;
use chrono::{Datelike, Duration, Local, Months, NaiveDate};
use gpui::{
    Context, EventEmitter, InteractiveElement, ParentElement, Render, StatefulInteractiveElement,
    Styled, Window, div, prelude::*, px, rgb,
};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub enum CalendarEvent {
    DateSelected(NaiveDate),
    MonthChanged(NaiveDate),
}

/// A standalone month calendar with a selected day and per-date event counts.
pub struct Calendar {
    month: NaiveDate,
    selected: Option<NaiveDate>,
    markers: HashMap<NaiveDate, usize>,
    day_height: f32,
    theme: ThemePreset,
}

impl EventEmitter<CalendarEvent> for Calendar {}

impl Calendar {
    pub fn new(_: &mut Context<Self>) -> Self {
        let today = Local::now().date_naive();
        Self {
            month: today.with_day(1).unwrap(),
            selected: Some(today),
            markers: HashMap::new(),
            day_height: 72.,
            theme: ThemePreset::default(),
        }
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn day_height(mut self, height: f32) -> Self {
        self.day_height = height.max(36.);
        self
    }
    pub fn selected(mut self, date: NaiveDate) -> Self {
        self.selected = Some(date);
        self.month = date.with_day(1).unwrap();
        self
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            cx.notify();
        }
    }
    pub fn set_markers(&mut self, markers: HashMap<NaiveDate, usize>, cx: &mut Context<Self>) {
        if self.markers != markers {
            self.markers = markers;
            cx.notify();
        }
    }
    pub fn set_selected(&mut self, date: NaiveDate, cx: &mut Context<Self>) {
        self.selected = Some(date);
        self.month = date.with_day(1).unwrap();
        cx.notify();
    }
    pub fn value(&self) -> Option<NaiveDate> {
        self.selected
    }
    fn move_month(&mut self, forward: bool, cx: &mut Context<Self>) {
        let next = if forward {
            self.month.checked_add_months(Months::new(1))
        } else {
            self.month.checked_sub_months(Months::new(1))
        };
        if let Some(next) = next {
            self.month = next;
            cx.emit(CalendarEvent::MonthChanged(next));
            cx.notify();
        }
    }
}

pub(crate) fn calendar_days(month: NaiveDate) -> Vec<NaiveDate> {
    let first = month.with_day(1).unwrap();
    let offset = first.weekday().num_days_from_monday() as i64;
    (0..42)
        .filter_map(|index| first.checked_add_signed(Duration::days(index - offset)))
        .collect()
}

impl Render for Calendar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let p = self.theme.palette();
        let today = Local::now().date_naive();
        let mut calendar = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(p.border))
            .overflow_hidden()
            .child(
                div()
                    .h(px(48.))
                    .px_4()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!(
                                "{} 年 {} 月",
                                self.month.year(),
                                self.month.month()
                            )),
                    )
                    .child(
                        div()
                            .id("calendar-today")
                            .px_2()
                            .py_1()
                            .cursor_pointer()
                            .child("今天")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_selected(today, cx);
                                cx.emit(CalendarEvent::DateSelected(today));
                            })),
                    )
                    .child(
                        div()
                            .id("calendar-previous")
                            .p_2()
                            .cursor_pointer()
                            .child(Icon::ArrowLeft.render(15.))
                            .on_click(cx.listener(|this, _, _, cx| this.move_month(false, cx))),
                    )
                    .child(
                        div()
                            .id("calendar-next")
                            .p_2()
                            .cursor_pointer()
                            .child(Icon::ChevronRight.render(15.))
                            .on_click(cx.listener(|this, _, _, cx| this.move_month(true, cx))),
                    ),
            );
        let mut weekdays = div().flex().border_b_1().border_color(rgb(p.border));
        for label in ["一", "二", "三", "四", "五", "六", "日"] {
            weekdays = weekdays.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .py_2()
                    .text_center()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(label),
            );
        }
        calendar = calendar.child(weekdays);
        for (week_index, week) in calendar_days(self.month).chunks(7).enumerate() {
            let mut row = div().flex().when(week_index > 0, |row| {
                row.border_t_1().border_color(rgb(p.border))
            });
            for (column, date) in week.iter().copied().enumerate() {
                let selected = self.selected == Some(date);
                let count = self.markers.get(&date).copied().unwrap_or(0);
                row = row.child(
                    div()
                        .id(gpui::SharedString::from(format!("calendar-{date}")))
                        .flex_1()
                        .min_w_0()
                        .h(px(self.day_height))
                        .p_2()
                        .when(column > 0, |cell| {
                            cell.border_l_1().border_color(rgb(p.border))
                        })
                        .when(selected, |cell| cell.bg(rgb(p.accent_tint)))
                        .cursor_pointer()
                        .hover(|cell| cell.bg(rgb(p.surface_subtle)))
                        .text_color(rgb(if selected || date == today {
                            p.accent
                        } else if date.month() != self.month.month() {
                            p.text_muted
                        } else {
                            p.text
                        }))
                        .child(
                            div()
                                .font_weight(if selected || date == today {
                                    gpui::FontWeight::SEMIBOLD
                                } else {
                                    gpui::FontWeight::NORMAL
                                })
                                .child(date.day().to_string()),
                        )
                        .when(count > 0, |cell| {
                            cell.child(
                                div()
                                    .mt_2()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.accent))
                                    .child(format!("{count} 项任务")),
                            )
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = Some(date);
                            cx.emit(CalendarEvent::DateSelected(date));
                            cx.notify();
                        })),
                );
            }
            calendar = calendar.child(row);
        }
        calendar
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn month_grid_covers_leap_day_and_year_boundaries_from_monday() {
        for month in [
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(),
        ] {
            let days = calendar_days(month);
            assert_eq!(days.len(), 42);
            assert_eq!(days[0].weekday().num_days_from_monday(), 0);
            assert!(
                days.windows(2)
                    .all(|pair| pair[1] - pair[0] == Duration::days(1))
            );
        }
        assert!(
            calendar_days(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
                .contains(&NaiveDate::from_ymd_opt(2024, 2, 29).unwrap())
        );
    }
}
