use super::{Button, ButtonKind, ButtonSize, DropdownMenu, Icon};
use crate::theme::ThemePreset;
use gpui::{App, IntoElement, Window, div, prelude::*, px, rgb};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundTaskState {
    Running,
    Completed,
    Failed,
    Cancelled,
    Finished,
}
impl BackgroundTaskState {
    pub fn is_running(self) -> bool {
        self == Self::Running
    }
    fn label(self) -> &'static str {
        match self {
            Self::Running => "运行中",
            Self::Completed => "已完成",
            Self::Failed => "失败",
            Self::Cancelled => "已停止",
            Self::Finished => "已结束",
        }
    }
}

#[derive(Clone)]
pub struct BackgroundTask {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub group: String,
    pub icon: Icon,
    pub state: BackgroundTaskState,
}

/// An in-app inbox: ended work stays until its result is opened. Viewing a
/// running task never acknowledges the completion that will happen later.
#[derive(Default)]
pub struct BackgroundTaskInbox {
    tasks: Vec<BackgroundTask>,
    dismissed: std::collections::HashSet<String>,
}
impl BackgroundTaskInbox {
    pub fn update(
        &mut self,
        current: Vec<BackgroundTask>,
        outcome: impl Fn(&BackgroundTask) -> BackgroundTaskState,
    ) -> Vec<BackgroundTask> {
        for task in &mut self.tasks {
            if !current.iter().any(|t| t.id == task.id) {
                let state = outcome(task);
                if state != BackgroundTaskState::Finished || task.state.is_running() {
                    task.state = state;
                }
            }
        }
        for task in current {
            if task.state.is_running() {
                self.dismissed.remove(&task.id);
            } else if self.dismissed.contains(&task.id) {
                continue;
            }
            if let Some(saved) = self.tasks.iter_mut().find(|t| t.id == task.id) {
                *saved = task;
            } else {
                self.tasks.push(task);
            }
        }
        self.tasks.clone()
    }
    pub fn acknowledge(&mut self, id: &str) {
        if self
            .tasks
            .iter()
            .any(|t| t.id == id && !t.state.is_running())
        {
            self.tasks.retain(|t| t.id != id);
            self.dismissed.insert(id.to_owned());
        }
    }
    /// Sources which stop emitting a completion after it is acknowledged do
    /// not need a tombstone; the same target may immediately run again.
    pub fn acknowledge_transient(&mut self, id: &str) {
        self.acknowledge(id);
        self.dismissed.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task(state: BackgroundTaskState) -> BackgroundTask {
        BackgroundTask {
            id: "job".into(),
            label: "编译".into(),
            detail: String::new(),
            group: "项目操作".into(),
            icon: Icon::Play,
            state,
        }
    }
    #[test]
    fn completions_survive_refresh_and_only_disappear_after_review() {
        let mut inbox = BackgroundTaskInbox::default();
        inbox.update(vec![task(BackgroundTaskState::Running)], |_| {
            BackgroundTaskState::Finished
        });
        inbox.acknowledge("job");
        assert_eq!(
            inbox.update(vec![], |_| BackgroundTaskState::Failed).len(),
            1
        );
        let ended = inbox.update(vec![], |_| BackgroundTaskState::Finished);
        assert_eq!(ended[0].state, BackgroundTaskState::Failed);
        inbox.acknowledge("job");
        assert!(
            inbox
                .update(vec![task(BackgroundTaskState::Failed)], |_| {
                    BackgroundTaskState::Finished
                })
                .is_empty()
        );
        assert_eq!(
            inbox
                .update(vec![task(BackgroundTaskState::Running)], |_| {
                    BackgroundTaskState::Finished
                })
                .len(),
            1
        );
        assert_eq!(
            inbox.update(vec![], |_| BackgroundTaskState::Completed)[0].state,
            BackgroundTaskState::Completed
        );
    }
    #[test]
    fn fast_completions_are_retained_without_a_running_frame() {
        let mut inbox = BackgroundTaskInbox::default();
        assert_eq!(
            inbox
                .update(vec![task(BackgroundTaskState::Completed)], |_| {
                    BackgroundTaskState::Finished
                })
                .len(),
            1
        );
        inbox.acknowledge_transient("job");
        assert_eq!(
            inbox
                .update(vec![task(BackgroundTaskState::Completed)], |_| {
                    BackgroundTaskState::Finished
                })
                .len(),
            1
        );
    }
}

/// Shared topbar entry for running and unreviewed tasks. Owners supply status and navigation;
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
                    .child("暂无后台任务"),
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
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(match task.state {
                                    BackgroundTaskState::Running => p.accent,
                                    BackgroundTaskState::Completed => self.theme.color(0x25845b),
                                    BackgroundTaskState::Failed => self.theme.color(0xd9342b),
                                    _ => p.text_muted,
                                }))
                                .child(task.state.label()),
                        )
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
