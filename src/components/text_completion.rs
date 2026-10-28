use crate::theme::ThemePreset;
use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Point,
    Styled, UniformListScrollHandle, Window, anchored, deferred, div, prelude::FluentBuilder, px,
    rgb,
};
use std::{
    ops::{Index, Range},
    rc::Rc,
};

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
    pub items: Rc<[TextCompletion]>,
    lowercase: Vec<String>,
    query: Option<String>,
    matches: Rc<[usize]>,
    pub selected: usize,
    scroll: UniformListScrollHandle,
    context: Option<(String, usize)>,
    dismissed: bool,
}
pub(super) struct CompletionMatch {
    pub range: Range<usize>,
    scroll: UniformListScrollHandle,
    pub items: CompletionItems,
}
/// Shared candidates and matching indices: no expression/description copies per frame.
#[derive(Clone)]
pub(super) struct CompletionItems {
    source: Rc<[TextCompletion]>,
    indices: Rc<[usize]>,
}
impl CompletionItems {
    pub fn len(&self) -> usize {
        self.indices.len()
    }
    pub fn get(&self, index: usize) -> Option<&TextCompletion> {
        self.indices.get(index).map(|i| &self.source[*i])
    }
}
impl Index<usize> for CompletionItems {
    type Output = TextCompletion;
    fn index(&self, index: usize) -> &Self::Output {
        &self.source[self.indices[index]]
    }
}
pub(super) enum CompletionKey {
    Handled,
    Insert(Range<usize>, String),
}
impl CompletionState {
    pub fn set_items(&mut self, items: Vec<TextCompletion>) -> bool {
        if self.items.as_ref() == items.as_slice() {
            return false;
        }
        self.lowercase = items
            .iter()
            .map(|item| item.expression.to_lowercase())
            .collect();
        self.items = items.into();
        self.query = None;
        self.matches = Rc::default();
        self.selected = 0;
        self.scroll
            .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
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
        if self
            .context
            .as_ref()
            .is_none_or(|(previous, cursor)| previous != text || *cursor != selection.end)
        {
            self.context = Some((text.to_owned(), selection.end));
            self.selected = 0;
            self.scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
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
        if self.query.as_deref() != Some(query.as_str()) {
            self.matches = self
                .lowercase
                .iter()
                .enumerate()
                .filter(|(_, expression)| expression.contains(&query))
                .map(|(index, _)| index)
                .collect::<Vec<_>>()
                .into();
            self.query = Some(query);
        }
        if self.matches.is_empty() {
            return None;
        }
        let items = CompletionItems {
            source: self.items.clone(),
            indices: self.matches.clone(),
        };
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
            scroll: self.scroll.clone(),
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
        if matches!(key, "up" | "down") {
            self.scroll
                .scroll_to_item(self.selected, gpui::ScrollStrategy::Nearest);
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
    let scroll = active.scroll;
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
    let height = px(active.items.len().min(6) as f32 * 40.);
    let items = active.items;
    let rows = gpui::uniform_list(
        "template-completion-scroll",
        items.len(),
        move |range, _, _| {
            range
                .map(|index| {
                    let item = &items[index];
                    let choose = choose.clone();
                    div()
                        .id(("template-completion", index))
                        .debug_selector(move || format!("template-completion-{index}").into())
                        .w_full()
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
                                .child(item.description.clone()),
                        )
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            choose(index, window, cx);
                        })
                })
                .collect::<Vec<_>>()
        },
    )
    .h(height)
    .w_full()
    .track_scroll(&scroll);
    menu = menu.child(rows).child(
        div()
            .px_2()
            .pt_1()
            .text_size(px(10.))
            .text_color(rgb(p.text_muted))
            .child("滚轮滑动 · ↑ ↓ 选择 · Tab / Enter 插入 · Esc 收起"),
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
    fn completion_query_cache_shares_data_and_invalidates_on_candidate_changes() {
        let mut state = CompletionState::default();
        state.set_items(
            (0..10_000)
                .map(|i| TextCompletion::new(format!("item_{i}"), "候选项"))
                .collect(),
        );
        let first = state.active("{{ITEM_99", &(9..9), false).unwrap();
        assert_eq!(first.items.len(), 111);
        for _ in 0..1_000 {
            let cached = state.active("{{ITEM_99", &(9..9), false).unwrap();
            assert!(Rc::ptr_eq(&first.items.source, &cached.items.source));
            assert!(Rc::ptr_eq(&first.items.indices, &cached.items.indices));
        }
        state.set_items(vec![TextCompletion::new("item_99999", "新增候选")]);
        let updated = state.active("{{ITEM_99", &(9..9), false).unwrap();
        assert_eq!(updated.items.len(), 1);
        assert_eq!(updated.items[0].expression, "item_99999");
        assert!(!Rc::ptr_eq(&first.items.source, &updated.items.source));
        assert!(state.active("{{missing", &(9..9), false).is_none());
        assert!(state.active("{{missing", &(9..9), false).is_none());
    }

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
