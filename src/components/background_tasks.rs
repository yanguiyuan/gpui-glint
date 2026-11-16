use super::{Button, ButtonKind, ButtonSize, DropdownMenu, Icon};
use crate::theme::ThemePreset;
use gpui::{App, IntoElement, Window, div, prelude::*, px, rgb};
use std::rc::Rc;

#[derive(Clone)]
pub struct BackgroundTask {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub group: String,
    pub icon: Icon,
}

/// Shared topbar entry for live tasks. Owners supply status and navigation;
/// opening or selecting the menu never starts or stops work.
pub struct BackgroundTasksMenu {
    tasks: Vec<BackgroundTask>,
    theme: ThemePreset,
    open: bool,
    select: Rc<dyn Fn(&str, &mut Window, &mut App)>,
    toggle: Rc<dyn Fn(bool, &mut Window, &mut App)>,
}
impl BackgroundTasksMenu {
    pub fn new(tasks: Vec<BackgroundTask>) -> Self {
        Self {
            tasks,
            theme: ThemePreset::default(),
            open: false,
            select: Rc::new(|_, _, _| {}),
            toggle: Rc::new(|_, _, _| {}),
        }
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }
    pub fn on_select(mut self, callback: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.select = Rc::new(callback);
        self
    }
    pub fn on_open_change(
        mut self,
        callback: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.toggle = Rc::new(callback);
        self
    }
    pub fn render(self) -> impl IntoElement {
        let p = self.theme.palette();
        let count = self.tasks.len();
        let mut rows = div()
            .id("background-task-list")
            .flex()
            .flex_col()
            .max_h(px(360.))
            .overflow_y_scroll()
            .occlude();
        if self.tasks.is_empty() {
            rows = rows.child(
                div()
                    .p_3()
                    .text_color(rgb(p.text_muted))
                    .child("暂无运行中的任务"),
            );
        }
        let mut groups = Vec::<String>::new();
        for task in &self.tasks {
            if !groups.contains(&task.group) {
                groups.push(task.group.clone());
            }
        }
        for group in groups {
            rows = rows.child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child(group.clone()),
            );
            for task in self.tasks.iter().filter(|t| t.group == group) {
                let id = task.id.clone();
                let selector = id.clone();
                let select = self.select.clone();
                let toggle = self.toggle.clone();
                rows = rows.child(
                    div()
                        .id(format!("background-task-{id}"))
                        .debug_selector(move || format!("background-task-{selector}"))
                        .px_3()
                        .py_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        .rounded_md()
                        .hover(|s| s.bg(rgb(p.accent_tint)))
                        .child(task.icon.render(14.).text_color(rgb(p.accent)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(12.))
                                        .child(task.label.clone()),
                                )
                                .when(!task.detail.is_empty(), |r| {
                                    r.child(
                                        div()
                                            .truncate()
                                            .text_size(px(10.))
                                            .text_color(rgb(p.text_muted))
                                            .child(task.detail.clone()),
                                    )
                                }),
                        )
                        .child(div().size(px(5.)).rounded_full().bg(rgb(p.accent)))
                        .on_click(move |_, window, cx| {
                            toggle(false, window, cx);
                            select(&id, window, cx);
                        }),
                );
            }
        }
        let toggle = self.toggle;
        DropdownMenu::new(
            "background-tasks-menu",
            Button::new(format!("后台任务 · {count}"))
                .kind(ButtonKind::Plain)
                .size(ButtonSize::Mini)
                .theme(self.theme)
                .render()
                .debug_selector(|| "background-tasks-trigger".into()),
            rows,
        )
        .width(360.)
        .open(self.open)
        .theme(self.theme)
        .on_open_change(move |open, window, cx| toggle(open, window, cx))
        .render()
    }
}
