use crate::theme::ThemePreset;
use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Point,
    ScrollHandle, StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point,
    prelude::FluentBuilder, px, rgb,
};
use std::ops::Range;

/// A template expression offered after `{{`. Contains no resolved value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextCompletion {
    pub expression: String,
    pub description: String,
}
impl TextCompletion {
    pub fn new(expression: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            expression: expression.into(),
            description: description.into(),
        }
    }
}

#[derive(Default)]
pub(super) struct CompletionState {
    pub items: Vec<TextCompletion>,
    pub selected: usize,
    context: Option<(String, usize)>,
    dismissed: bool,
}
pub(super) struct CompletionMatch {
    pub range: Range<usize>,
    pub items: Vec<TextCompletion>,
}
pub(super) enum CompletionKey {
    Handled,
    Insert(Range<usize>, String),
}
impl CompletionState {
    pub fn set_items(&mut self, items: Vec<TextCompletion>) -> bool {
        if self.items == items {
            return false;
        }
        self.items = items;
        self.selected = 0;
        true
    }
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }
    pub fn active(
        &mut self,
        text: &str,
        selection: &Range<usize>,
        composing: bool,
    ) -> Option<CompletionMatch> {
        if self.items.is_empty() {
            return None;
        }
        let context = (text.to_owned(), selection.end);
        if self.context.as_ref() != Some(&context) {
            self.context = Some(context);
            self.selected = 0;
            self.dismissed = false;
        }
        if composing || self.dismissed || !selection.is_empty() {
            return None;
        }
        let cursor = selection.end;
        let before = text.get(..cursor)?;
        let start = before.rfind("{{")?;
        let query = &before[start + 2..];
        if query.contains(['{', '}', '\n', '\r']) {
            return None;
        }
        let query = query.trim().to_lowercase();
        let items: Vec<_> = self
            .items
            .iter()
            .filter(|item| item.expression.to_lowercase().contains(&query))
            .cloned()
            .collect();
        if items.is_empty() {
            return None;
        }
        self.selected = self.selected.min(items.len() - 1);
        let rest = &text[cursor..];
        let end = match rest.find("}}") {
            Some(end) if !rest[..end].contains(['{', '\n', '\r']) => cursor + end + 2,
            _ => {
                cursor
                    + rest
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .map(char::len_utf8)
                        .sum::<usize>()
            }
        };
        Some(CompletionMatch {
            range: start..end,
            items,
        })
    }
    pub fn key(
        &mut self,
        text: &str,
        selection: &Range<usize>,
        composing: bool,
        key: &str,
    ) -> Option<CompletionKey> {
        let active = self.active(text, selection, composing)?;
        match key {
            "up" => self.selected = (self.selected + active.items.len() - 1) % active.items.len(),
            "down" => self.selected = (self.selected + 1) % active.items.len(),
            "escape" => self.dismiss(),
            "tab" | "enter" => {
                return Some(CompletionKey::Insert(
                    active.range,
                    format!("{{{{{}}}}}", active.items[self.selected].expression),
                ));
            }
            _ => return None,
        }
        Some(CompletionKey::Handled)
    }
}

pub(super) fn completion_menu(
    active: CompletionMatch,
    selected: usize,
    position: Point<Pixels>,
    theme: ThemePreset,
    choose: impl Fn(usize, &mut Window, &mut App) + 'static,
    dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let choose = std::rc::Rc::new(choose);
    let p = theme.palette();
    let scroll = ScrollHandle::new();
    scroll.set_offset(point(px(0.), -px(selected.saturating_sub(4) as f32 * 40.)));
    let mut menu = div()
        .id("template-completion-menu")
        .debug_selector(|| "template-completion-menu".into())
        .w(px(320.))
        .p_1()
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(p.border))
        .bg(rgb(p.surface))
        .text_color(rgb(p.text))
        .shadow_lg()
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
        })
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .on_mouse_down_out(move |_, window, cx| dismiss(window, cx));
    let mut rows = div()
        .id("template-completion-scroll")
        .max_h(px(240.))
        .overflow_y_scroll()
        .track_scroll(&scroll);
    for (index, item) in active.items.into_iter().enumerate() {
        let choose = choose.clone();
        rows = rows.child(
            div()
                .id(("template-completion", index))
                .debug_selector(move || format!("template-completion-{index}").into())
                .h(px(40.))
                .px_2()
                .flex()
                .flex_col()
                .justify_center()
                .rounded(px(4.))
                .cursor_pointer()
                .when(index == selected, |el| el.bg(rgb(p.accent_tint)))
                .hover(|el| el.bg(rgb(p.accent_tint)))
                .child(
                    div()
                        .text_size(px(12.))
                        .child(format!("{{{{{}}}}}", item.expression)),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(p.text_muted))
                        .child(item.description),
                )
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                    choose(index, window, cx);
                }),
        );
    }
    menu = menu.child(rows).child(
        div()
            .px_2()
            .pt_1()
            .text_size(px(10.))
            .text_color(rgb(p.text_muted))
            .child("↑ ↓ 选择 · Tab / Enter 插入 · Esc 收起"),
    );
    deferred(
        anchored()
            .position(position)
            .snap_to_window_with_margin(px(8.))
            .child(menu),
    )
    .with_priority(50)
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn functions_remain_reachable_after_many_environment_variables() {
        let mut state = CompletionState::default();
        let mut items: Vec<_> = (0..20)
            .map(|i| TextCompletion::new(format!("variable_{i}"), "环境变量"))
            .collect();
        items.push(TextCompletion::new("uuid()", "内置函数 · UUID"));
        state.set_items(items);
        let active = state.active("{{", &(2..2), false).unwrap();
        assert_eq!(active.items.len(), 21);
        for _ in 0..20 {
            state.key("{{", &(2..2), false, "down");
        }
        let Some(CompletionKey::Insert(_, expression)) = state.key("{{", &(2..2), false, "enter")
        else {
            panic!("expected function insertion")
        };
        assert_eq!(expression, "{{uuid()}}");
        assert_eq!(
            state.active("{{uuid", &(6..6), false).unwrap().items[0].expression,
            "uuid()"
        );
    }

    #[test]
    fn ranges_preserve_unicode_suffix_and_existing_closing_braces() {
        let mut state = CompletionState::default();
        state.set_items(vec![TextCompletion::new("base_url", "环境变量")]);
        let text = "中文/{{ba}}/路径";
        let cursor = "中文/{{ba".len();
        let active = state.active(text, &(cursor..cursor), false).unwrap();
        assert_eq!(&text[active.range], "{{ba}}");
        assert!(state.active("{{base_url}}", &(12..12), false).is_none());
        assert!(state.active("{{ba", &(4..4), true).is_none());
        state.dismiss();
        assert!(state.active("{{ba", &(4..4), false).is_none());
        assert!(state.active("{{bas", &(5..5), false).is_some());
    }
}
