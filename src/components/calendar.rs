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

/// A standalone month calendar with a selected day and per-date task titles and overflow counts.
pub struct Calendar {
    month: NaiveDate,
    selected: Option<NaiveDate>,
    markers: HashMap<NaiveDate, usize>,
    task_titles: HashMap<NaiveDate, Vec<String>>,
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
            task_titles: HashMap::new(),
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
    /// Ordered task titles shown in each day cell. Counts remain available via
    /// `set_markers` for callers that only have aggregate data.
    pub fn set_task_titles(
        &mut self,
        titles: HashMap<NaiveDate, Vec<String>>,
        cx: &mut Context<Self>,
    ) {
        if self.task_titles != titles {
            self.task_titles = titles;
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

/// Reserve a row for the overflow count when all titles cannot fit.
fn task_rows(height: f32, title_count: usize, total: usize) -> (usize, usize) {
    let slots = ((height - 16. - 20. - 4.).max(0.) / 14.).floor() as usize;
    let titles = if total <= slots {
        title_count.min(total).min(3)
    } else {
        title_count.min(slots.saturating_sub(1)).min(3)
    };
    let titles = if total > titles && titles == slots {
        titles.saturating_sub(1)
    } else {
        titles
    };
    (
        titles,
        if slots > 0 {
            total.saturating_sub(titles)
        } else {
            0
        },
    )
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
                let titles = self
                    .task_titles
                    .get(&date)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let count = self
                    .markers
                    .get(&date)
                    .copied()
                    .unwrap_or(0)
                    .max(titles.len());
                let (visible, remaining) = task_rows(self.day_height, titles.len(), count);
                row = row.child(
                    div()
                        .id(gpui::SharedString::from(format!("calendar-{date}")))
                        .flex_1()
                        .min_w_0()
                        .h(px(self.day_height))
                        .p_2()
                        .overflow_hidden()
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
                                .h(px(20.))
                                .line_height(px(20.))
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
                                    .mt_1()
                                    .w_full()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.accent))
                                    .children(titles.iter().take(visible).enumerate().map(
                                        |(index, title)| {
                                            let selector = format!("calendar-task-{date}-{index}");
                                            div()
                                                .h(px(14.))
                                                .line_height(px(14.))
                                                .w_full()
                                                .min_w_0()
                                                .text_ellipsis()
                                                .debug_selector(move || selector.clone())
                                                .child(title.clone())
                                        },
                                    ))
                                    .when(remaining > 0, |list| {
                                        list.child(
                                            div()
                                                .h(px(14.))
                                                .line_height(px(14.))
                                                .w_full()
                                                .text_ellipsis()
                                                .child(if visible == 0 {
                                                    format!("{remaining} 项任务")
                                                } else {
                                                    format!("另 {remaining} 项任务")
                                                }),
                                        )
                                    }),
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
    fn task_rows_reserve_space_for_overflow_and_keep_totals() {
        assert_eq!(task_rows(72., 1, 1), (1, 0));
        assert_eq!(task_rows(72., 2, 2), (2, 0));
        assert_eq!(task_rows(72., 5, 5), (1, 4));
        assert_eq!(task_rows(112., 5, 5), (3, 2));
        assert_eq!(task_rows(72., 0, 5), (0, 5));
        assert_eq!(task_rows(36., 5, 5), (0, 0));
        for height in [56., 72., 88., 112., 160.] {
            for total in 0..30 {
                let (visible, remaining) = task_rows(height, total, total);
                assert_eq!(visible + remaining, total);
                let rows = visible + usize::from(remaining > 0);
                assert!(rows as f32 * 14. <= height - 40.);
            }
        }
    }

    #[gpui::test]
    fn calendar_shows_titles_that_fit_and_keeps_date_selection(cx: &mut gpui::TestAppContext) {
        let date = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        let (calendar, window) = cx.add_window_view(|_, cx| Calendar::new(cx).selected(date));
        window.simulate_resize(gpui::size(px(840.), px(800.)));
        calendar.update(window, |calendar, cx| {
            calendar.set_task_titles(
                HashMap::from([(date, (0..5).map(|index| format!("Task {index}")).collect())]),
                cx,
            )
        });
        window.run_until_parked();
        assert!(window.debug_bounds("calendar-task-2026-10-31-0").is_some());
        assert!(window.debug_bounds("calendar-task-2026-10-31-1").is_none());
        calendar.update(window, |calendar, cx| {
            calendar.day_height = 112.;
            cx.notify();
        });
        window.run_until_parked();
        assert!(window.debug_bounds("calendar-task-2026-10-31-2").is_some());
        assert!(window.debug_bounds("calendar-task-2026-10-31-3").is_none());
        calendar.read_with(window, |calendar, _| {
            assert_eq!(calendar.value(), Some(date))
        });
    }

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
