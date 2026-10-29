use super::Icon;
use crate::theme::ThemePreset;
use gpui::{App, FontWeight, MouseButton, SharedString, Window, div, prelude::*, px, rgb};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TodoListStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TodoListItem {
    pub content: SharedString,
    pub status: TodoListStatus,
}

/// Read-only agent progress. The host owns the list and its expansion state.
pub struct TodoList {
    items: Vec<TodoListItem>,
    expanded: bool,
    theme: ThemePreset,
    on_toggle: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
}

impl TodoList {
    pub fn new(items: Vec<TodoListItem>) -> Self {
        Self {
            items,
            expanded: true,
            theme: ThemePreset::default(),
            on_toggle: None,
        }
    }
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn on_toggle(mut self, callback: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(callback));
        self
    }

    pub fn render(self) -> gpui::Div {
        let p = self.theme.palette();
        let completed = self
            .items
            .iter()
            .filter(|item| item.status == TodoListStatus::Completed)
            .count();
        let title = if completed == self.items.len() {
            "任务已完成"
        } else {
            "任务清单"
        };
        let toggle = self.on_toggle;
        let header = div()
            .id("todo-list-toggle")
            .debug_selector(|| "todo-list-toggle".into())
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .when(toggle.is_some(), |header| {
                header
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(p.surface_subtle)))
            })
            .child(Icon::Workflow.render(14.).text_color(rgb(p.accent)))
            .child(div().flex_1().font_weight(FontWeight::MEDIUM).child(title))
            .child(
                div()
                    .text_color(rgb(p.text_muted))
                    .child(format!("{completed}/{}", self.items.len())),
            )
            .when(toggle.is_some(), |header| {
                header.child(
                    if self.expanded {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    }
                    .render(12.)
                    .text_color(rgb(p.text_muted)),
                )
            })
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                if let Some(toggle) = &toggle {
                    toggle(window, cx);
                }
            });
        let mut body = div()
            .w_full()
            .min_w_0()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .text_size(px(12.))
            .text_color(rgb(p.text))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(header);
        if self.expanded {
            let mut rows = div()
                .id("todo-list-rows")
                .debug_selector(|| "todo-list-rows".into())
                .max_h(px(220.))
                .overflow_y_scroll()
                .px_3()
                .pb_2()
                .flex()
                .flex_col()
                .gap_2();
            for item in self.items {
                let (symbol, color) = match item.status {
                    TodoListStatus::Pending => ("○", p.text_muted),
                    TodoListStatus::InProgress => ("◉", p.accent),
                    TodoListStatus::Completed => ("✓", p.text_muted),
                };
                rows = rows.child(
                    div()
                        .w_full()
                        .flex()
                        .items_start()
                        .gap_2()
                        .child(
                            div()
                                .w(px(14.))
                                .flex_shrink_0()
                                .text_color(rgb(color))
                                .child(symbol),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_color(rgb(color))
                                .child(item.content),
                        ),
                );
            }
            body = body.child(rows);
        }
        body
    }
}
