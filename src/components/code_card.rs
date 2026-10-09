use super::rich_text::{Format, RichText};
use crate::theme::ThemePreset;
use gpui::AppContext;
use std::{collections::HashMap, ops::Range, rc::Rc};

use gpui::{
    App, Bounds, Context, Element, ElementId, ElementInputHandler, Entity, EntityInputHandler,
    EventEmitter, FocusHandle, Focusable, GlobalElementId, InspectorElementId, InteractiveElement,
    IntoElement, KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Render, ScrollHandle, ShapedLine, SharedString,
    StatefulInteractiveElement, Style, Styled, TextRun, UTF16Selection, UnderlineStyle, Window,
    actions, div, fill, point, px, rgb, rgba, size,
};

actions!(
    glass_code_card,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
        Up,
        Down,
        SelectUp,
        SelectDown,
        Newline,
        Indent,
        Undo,
        Redo,
        Continue,
        SoftNewline,
        RichBold,
        RichItalic,
    ]
);

/// Register once to enable code editing and clipboard shortcuts.
pub fn bind_code_card_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-b", RichBold, Some("GlassCodeCard")),
        KeyBinding::new("cmd-i", RichItalic, Some("GlassCodeCard")),
        KeyBinding::new("backspace", Backspace, Some("GlassCodeCard")),
        KeyBinding::new("delete", Delete, Some("GlassCodeCard")),
        KeyBinding::new("left", Left, Some("GlassCodeCard")),
        KeyBinding::new("right", Right, Some("GlassCodeCard")),
        KeyBinding::new("shift-left", SelectLeft, Some("GlassCodeCard")),
        KeyBinding::new("shift-right", SelectRight, Some("GlassCodeCard")),
        KeyBinding::new("cmd-a", SelectAll, Some("GlassCodeCard")),
        KeyBinding::new("home", Home, Some("GlassCodeCard")),
        KeyBinding::new("end", End, Some("GlassCodeCard")),
        KeyBinding::new("cmd-v", Paste, Some("GlassCodeCard")),
        KeyBinding::new("cmd-x", Cut, Some("GlassCodeCard")),
        KeyBinding::new("cmd-c", Copy, Some("GlassCodeCard")),
        KeyBinding::new("up", Up, Some("GlassCodeCard")),
        KeyBinding::new("down", Down, Some("GlassCodeCard")),
        KeyBinding::new("shift-up", SelectUp, Some("GlassCodeCard")),
        KeyBinding::new("shift-down", SelectDown, Some("GlassCodeCard")),
        KeyBinding::new("enter", Newline, Some("GlassCodeCard")),
        KeyBinding::new("shift-enter", SoftNewline, Some("GlassCodeCard")),
        KeyBinding::new("tab", Indent, Some("GlassCodeCard")),
        KeyBinding::new("cmd-z", Undo, Some("GlassCodeCard")),
        KeyBinding::new("cmd-shift-z", Redo, Some("GlassCodeCard")),
        KeyBinding::new("cmd-enter", Continue, Some("GlassCodeCard")),
        KeyBinding::new("ctrl-enter", Continue, Some("GlassCodeCard")),
    ]);
}

/// Notifications for integrating source edits into the owning document.
#[derive(Clone, Debug)]
pub enum CodeCardEvent {
    Changed(SharedString),
    Focused,
    SelectionStarted {
        editor: gpui::EntityId,
        offset: usize,
        extend: bool,
    },
    FormatApplied,
    ContinueAfter,
    Split {
        before: String,
        after: String,
    },
    MergePrevious,
    UndoRequested,
    RedoRequested,
    CommandKey(SharedString),
}

#[derive(Clone)]
struct Snapshot {
    content: SharedString,
    selection: Range<usize>,
}

#[derive(Clone, PartialEq)]
struct LineCacheKey {
    content: SharedString,
    revision: u64,
    language: SharedString,
    font: gpui::Font,
    color: gpui::Hsla,
    font_size: Pixels,
    wrap_width: Pixels,
    dark: bool,
    prose: bool,
    rich: bool,
    marked: Option<Range<usize>>,
}
struct ViewportCache {
    key: LineCacheKey,
    ranges: Vec<Range<usize>>,
    tokens: Vec<(Range<usize>, TokenKind)>,
    lines: HashMap<usize, EditorLine>,
    width: Pixels,
}
fn token_slice(
    tokens: &[(Range<usize>, TokenKind)],
    range: Range<usize>,
) -> &[(Range<usize>, TokenKind)] {
    let from = tokens.partition_point(|(token, _)| token.end <= range.start);
    let to = tokens.partition_point(|(token, _)| token.start < range.end);
    &tokens[from..to.max(from)]
}
struct LineCache {
    key: LineCacheKey,
    lines: Rc<Vec<(usize, EditorLine)>>,
    width: Pixels,
}

#[derive(Clone, Copy)]
enum ScrollAxis {
    Horizontal,
    Vertical,
}

/// An inline code editor with line numbers, restrained syntax colors and JSON validation.
/// Unknown fence languages render as plain text. Create with `cx.new(...)` and
/// subscribe to [`CodeCardEvent`] to integrate with a document.
pub struct CodeCard {
    viewport: bool,
    viewport_cache: Option<ViewportCache>,
    viewport_scroll: gpui::UniformListScrollHandle,
    scroll_axis: Option<ScrollAxis>,
    last_scroll: Option<std::time::Instant>,
    layout_revision: u64,
    line_cache: Option<LineCache>,
    search_cache: Option<(SharedString, SharedString, Rc<Vec<Range<usize>>>)>,
    search_query: SharedString,
    focus_handle: FocusHandle,
    content: SharedString,
    language: SharedString,
    read_only: bool,
    show_header: bool,
    prose: bool,
    rich: Option<RichText>,
    link_editor: Option<Entity<CodeCard>>,
    wrap_width: Pixels,
    document_managed: bool,
    command_mode: bool,
    block_editing: bool,
    dark: bool,
    history: Vec<Snapshot>,
    future: Vec<Snapshot>,
    scroll: ScrollHandle,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_lines: Vec<(usize, EditorLine, Bounds<Pixels>)>,
    is_selecting: bool,
    document_selection: Option<Range<usize>>,
    reveal_cursor: bool,
}

impl CodeCard {
    #[cfg(test)]
    pub(super) fn measured_widths(&self) -> (Pixels, Pixels, Pixels) {
        (
            self.scroll.bounds().size.width,
            self.wrap_width,
            self.line_cache.as_ref().map_or(px(0.), |cache| cache.width),
        )
    }
    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    /// Search decoration only; does not change content, selection, or undo history.
    pub fn set_search_query(&mut self, query: impl Into<SharedString>, cx: &mut Context<Self>) {
        let query = query.into();
        if self.search_query != query {
            self.search_query = query;
            cx.notify();
        }
    }

    pub fn new(
        cx: &mut Context<Self>,
        language: impl Into<SharedString>,
        value: impl Into<SharedString>,
    ) -> Self {
        let content = value.into();
        let end = content.len();
        Self {
            viewport: false,
            viewport_cache: None,
            viewport_scroll: gpui::UniformListScrollHandle::default(),
            scroll_axis: None,
            last_scroll: None,
            layout_revision: 0,
            line_cache: None,
            search_cache: None,
            search_query: "".into(),
            focus_handle: cx.focus_handle(),
            content,
            language: language.into(),
            read_only: false,
            show_header: true,
            prose: false,
            rich: None,
            link_editor: None,
            wrap_width: px(0.),
            document_managed: false,
            command_mode: false,
            block_editing: false,
            dark: false,
            history: Vec::new(),
            future: Vec::new(),
            scroll: ScrollHandle::new(),
            selected_range: end..end,
            selection_reversed: false,
            marked_range: None,
            last_lines: Vec::new(),
            is_selecting: false,
            document_selection: None,
            reveal_cursor: false,
        }
    }

    fn scroll_wheel(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = std::time::Instant::now();
        if matches!(event.touch_phase, gpui::TouchPhase::Started)
            || self
                .last_scroll
                .is_none_or(|last| now.duration_since(last).as_millis() > 250)
        {
            self.scroll_axis = None;
        }
        let delta = event.delta.pixel_delta(px(LINE_HEIGHT));
        if self.scroll_axis.is_none() && (delta.x != px(0.) || delta.y != px(0.)) {
            self.scroll_axis = Some(if delta.x.abs() > delta.y.abs() {
                ScrollAxis::Horizontal
            } else {
                ScrollAxis::Vertical
            });
        }
        match self.scroll_axis {
            Some(ScrollAxis::Horizontal) => {
                let offset = self.scroll.offset();
                let x = (offset.x + delta.x).clamp(-self.scroll.max_offset().x, px(0.));
                self.scroll.set_offset(point(x, px(0.)));
                // Consume even at the horizontal boundary, so the document
                // never starts scrolling vertically during the same gesture.
                cx.stop_propagation();
                if x != offset.x {
                    cx.notify();
                }
            }
            Some(ScrollAxis::Vertical) if delta.y == px(0.) => cx.stop_propagation(),
            _ => {} // The document handles only the vertical component.
        }
        self.last_scroll = Some(now);
        if matches!(event.touch_phase, gpui::TouchPhase::Ended) {
            self.scroll_axis = None;
        }
    }

    pub(super) fn rich_text(mut self, markdown: &str) -> Self {
        let rich = RichText::parse(markdown);
        self.content = rich.text.clone().into();
        self.selected_range = self.content.len()..self.content.len();
        self.rich = Some(rich);
        self.prose = true;
        self
    }
    fn apply_format(
        &mut self,
        format: Option<Format>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only || self.selected_range.is_empty() {
            return;
        }
        if let Some(rich) = &mut self.rich {
            cx.emit(CodeCardEvent::FormatApplied);
            rich.toggle(self.selected_range.clone(), format);
            self.link_editor = None;
            window.focus(&self.focus_handle, cx);
            self.changed(cx);
        }
    }
    fn rich_bold(&mut self, _: &RichBold, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_format(Some(Format::Bold), window, cx);
    }
    fn rich_italic(&mut self, _: &RichItalic, window: &mut Window, cx: &mut Context<Self>) {
        self.apply_format(Some(Format::Italic), window, cx);
    }
    /// Borderless, soft-wrapped multiline editor for document blocks and cells.
    pub fn prose(mut self, value: bool) -> Self {
        self.prose = value;
        self
    }
    pub fn block_editing(mut self, value: bool) -> Self {
        self.block_editing = value;
        self
    }
    pub fn set_read_only(&mut self, value: bool, cx: &mut Context<Self>) {
        if self.read_only != value {
            self.read_only = value;
            cx.notify();
        }
    }
    pub fn document_managed(mut self, value: bool) -> Self {
        self.document_managed = value;
        self
    }
    pub fn set_command_mode(&mut self, value: bool, cx: &mut Context<Self>) {
        self.command_mode = value;
        cx.notify();
    }
    /// Replace from document history without emitting a new edit event.
    pub fn sync_content(&mut self, value: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = value.into();
        self.selected_range = self.content.len()..self.content.len();
        self.marked_range = None;
        self.history.clear();
        self.future.clear();
        cx.notify();
    }
    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn read_only(mut self, value: bool) -> Self {
        self.read_only = value;
        self
    }

    /// Show the language, JSON status and formatting bar. Defaults to true.
    /// When hidden, the card starts directly with the numbered source content.
    pub fn show_header(mut self, value: bool) -> Self {
        self.show_header = value;
        self
    }

    /// Change header visibility without resetting content or editor state.
    pub fn set_show_header(&mut self, value: bool, cx: &mut Context<Self>) {
        if self.show_header != value {
            self.show_header = value;
            cx.notify();
        }
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.dark = theme.is_dark();
        self
    }

    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.set_dark(theme.is_dark(), cx);
        if let Some(editor) = &self.link_editor {
            editor.update(cx, |editor, cx| editor.set_theme(theme, cx));
        }
    }

    /// Overrides the code editor appearance independently of a surface preset.
    pub fn dark(mut self, value: bool) -> Self {
        self.dark = value;
        self
    }

    pub fn set_dark(&mut self, value: bool, cx: &mut Context<Self>) {
        if self.dark != value {
            self.dark = value;
            cx.notify();
        }
    }

    /// Fixed-height, internally scrolling, lazy read-only source viewer.
    /// Give the parent a definite height. Editable/prose cards retain their normal layout.
    pub fn viewport(mut self) -> Self {
        self.viewport = true;
        self
    }

    pub fn set_language(&mut self, language: impl Into<SharedString>, cx: &mut Context<Self>) {
        let language = language.into();
        if self.language == language {
            return;
        }
        self.language = language;
        cx.notify();
    }

    pub fn set_content(&mut self, value: impl Into<SharedString>, cx: &mut Context<Self>) {
        let value = value.into();
        if self.content == value {
            return;
        }
        if !self.read_only {
            self.remember();
        }
        self.content = value;
        let cursor = self.content.len();
        self.selected_range = cursor..cursor;
        self.marked_range = None;
        self.changed(cx);
    }

    pub(super) fn set_cursor_position(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.move_to(offset.min(self.content.len()), cx);
    }

    pub(super) fn set_document_selection(
        &mut self,
        range: Option<Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        if self.document_selection != range {
            self.document_selection = range;
            self.link_editor = None;
            cx.notify();
        }
    }

    pub(super) fn mouse_text_hit(&self, position: gpui::Point<Pixels>) -> Option<(f32, usize)> {
        let distance = self
            .last_lines
            .iter()
            .map(|(_, _, bounds)| {
                let dy = if position.y < bounds.top() {
                    bounds.top() - position.y
                } else if position.y > bounds.bottom() {
                    position.y - bounds.bottom()
                } else {
                    px(0.)
                };
                let dx = if position.x < bounds.left() {
                    bounds.left() - position.x
                } else if position.x > bounds.right() {
                    position.x - bounds.right()
                } else {
                    px(0.)
                };
                f32::from(dy) * 1000. + f32::from(dx)
            })
            .min_by(f32::total_cmp)?;
        Some((distance, self.index_for_mouse_position(position)))
    }

    pub fn json_valid(&self) -> Option<bool> {
        self.is_json()
            .then(|| serde_json::from_str::<serde_json::Value>(&self.content).is_ok())
    }

    pub fn format_json(&mut self, cx: &mut Context<Self>) -> bool {
        if self.read_only || !self.is_json() {
            return false;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&self.content) else {
            return false;
        };
        let formatted = serde_json::to_string_pretty(&value).expect("JSON value is serializable");
        if formatted != self.content.as_ref() {
            self.set_content(formatted, cx);
        }
        true
    }

    fn is_json(&self) -> bool {
        self.language.eq_ignore_ascii_case("json")
    }

    fn remember(&mut self) {
        self.history.push(Snapshot {
            content: self.content.clone(),
            selection: self.selected_range.clone(),
        });
        if self.history.len() > 200 {
            self.history.remove(0);
        }
        self.future.clear();
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.layout_revision = self.layout_revision.wrapping_add(1);
        self.reveal_cursor = true;
        let value = if let Some(rich) = &mut self.rich {
            rich.update_text(&self.content);
            rich.markdown().into()
        } else {
            self.content.clone()
        };
        cx.emit(CodeCardEvent::Changed(value));
        cx.notify();
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.document_managed {
            cx.emit(CodeCardEvent::UndoRequested);
            return;
        }
        if let Some(snapshot) = self.history.pop() {
            self.future.push(Snapshot {
                content: self.content.clone(),
                selection: self.selected_range.clone(),
            });
            self.restore(snapshot, cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.document_managed {
            cx.emit(CodeCardEvent::RedoRequested);
            return;
        }
        if let Some(snapshot) = self.future.pop() {
            self.history.push(Snapshot {
                content: self.content.clone(),
                selection: self.selected_range.clone(),
            });
            self.restore(snapshot, cx);
        }
    }

    fn restore(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.content = snapshot.content;
        self.selected_range = snapshot.selection;
        self.selection_reversed = false;
        self.marked_range = None;
        self.changed(cx);
    }

    fn newline(&mut self, _: &Newline, _window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.command_mode {
            cx.emit(CodeCardEvent::CommandKey("enter".into()));
            return;
        }
        if self.block_editing {
            cx.emit(CodeCardEvent::Split {
                before: self
                    .rich
                    .as_ref()
                    .map(|rich| rich.slice_markdown(0..self.selected_range.start))
                    .unwrap_or_else(|| self.content[..self.selected_range.start].to_owned()),
                after: self
                    .rich
                    .as_ref()
                    .map(|rich| rich.slice_markdown(self.selected_range.end..self.content.len()))
                    .unwrap_or_else(|| self.content[self.selected_range.end..].to_owned()),
            });
            return;
        }
        self.soft_newline(&SoftNewline, _window, cx);
    }
    fn soft_newline(&mut self, _: &SoftNewline, _: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let start = self.content[..cursor].rfind('\n').map_or(0, |i| i + 1);
        let indent: String = self.content[start..cursor]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        self.replace_selection(&format!("\n{indent}"), cx);
    }

    fn indent(&mut self, _: &Indent, _: &mut Window, cx: &mut Context<Self>) {
        self.replace_selection("    ", cx);
    }

    fn continue_after(&mut self, _: &Continue, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CodeCardEvent::ContinueAfter);
    }

    fn vertical(&mut self, down: bool, select: bool, cx: &mut Context<Self>) {
        if self.command_mode {
            cx.emit(CodeCardEvent::CommandKey(
                if down { "down" } else { "up" }.into(),
            ));
            return;
        }
        let cursor = self.cursor_offset();
        let start = self.content[..cursor].rfind('\n').map_or(0, |i| i + 1);
        let column = self.content[start..cursor].chars().count();
        let target = if down {
            self.content[cursor..].find('\n').map(|i| cursor + i + 1)
        } else if start > 0 {
            Some(self.content[..start - 1].rfind('\n').map_or(0, |i| i + 1))
        } else {
            None
        };
        if let Some(target) = target {
            let line = self.content[target..].split('\n').next().unwrap_or("");
            let offset = target
                + line
                    .char_indices()
                    .nth(column)
                    .map_or(line.len(), |(i, _)| i);
            if select {
                self.select_to(offset, cx);
            } else {
                self.move_to(offset, cx);
            }
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(false, false, cx);
    }
    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(true, false, cx);
    }
    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(false, true, cx);
    }
    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(true, true, cx);
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf16 >= offset {
                break;
            }
            utf16 += ch.len_utf16();
            utf8 += ch.len_utf8();
        }
        utf8
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf8 = 0;
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf8 >= offset {
                break;
            }
            utf8 += ch.len_utf8();
            utf16 += ch.len_utf16();
        }
        utf16
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range.start)..self.offset_from_utf16(range.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .char_indices()
            .rev()
            .find_map(|(index, _)| (index < offset).then_some(index))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .char_indices()
            .find_map(|(index, _)| (index > offset).then_some(index))
            .unwrap_or(self.content.len())
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.reveal_cursor = true;
        cx.notify();
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.reveal_cursor = true;
        cx.notify();
    }

    fn replace_selection(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        self.remember();
        let range = self.selected_range.clone();
        self.content = format!(
            "{}{}{}",
            &self.content[..range.start],
            text,
            &self.content[range.end..]
        )
        .into();
        let cursor = range.start + text.len();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.marked_range = None;
        self.changed(cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let offset = if self.selected_range.is_empty() {
            self.previous_boundary(self.cursor_offset())
        } else {
            self.selected_range.start
        };
        self.move_to(offset, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let offset = if self.selected_range.is_empty() {
            self.next_boundary(self.cursor_offset())
        } else {
            self.selected_range.end
        };
        self.move_to(offset, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let start = self.content[..self.cursor_offset()]
            .rfind('\n')
            .map_or(0, |i| i + 1);
        self.move_to(start, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let end = self.content[cursor..]
            .find('\n')
            .map_or(self.content.len(), |i| cursor + i);
        self.move_to(end, cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.block_editing && self.selected_range == (0..0) {
            cx.emit(CodeCardEvent::MergePrevious);
            return;
        }
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_selection("", cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_selection("", cx);
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_selection(&text.replace("\r\n", "\n"), cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            self.copy(&Copy, window, cx);
            self.replace_selection("", cx);
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.link_editor = None;
        cx.emit(CodeCardEvent::Focused);
        self.is_selecting = true;
        window.focus(&self.focus_handle, cx);
        let offset = self.index_for_mouse_position(event.position);
        if self.document_managed {
            cx.emit(CodeCardEvent::SelectionStarted {
                editor: cx.entity_id(),
                offset,
                extend: event.modifiers.shift,
            });
        }
        if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = false;
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn index_for_mouse_position(&self, position: gpui::Point<Pixels>) -> usize {
        let Some((start, line, bounds)) = self.last_lines.iter().min_by(|a, b| {
            let distance =
                |bounds: &Bounds<Pixels>| (f32::from(position.y - bounds.top()) - 10.5).abs();
            distance(&a.2).total_cmp(&distance(&b.2))
        }) else {
            return 0;
        };
        start + line.closest_index_for_x(position.x - bounds.left())
    }
}

impl EventEmitter<CodeCardEvent> for CodeCard {}

impl EntityInputHandler for CodeCard {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        if self.read_only {
            return;
        }
        if self.marked_range.is_none() {
            self.remember();
        }
        self.content = format!(
            "{}{}{}",
            &self.content[..range.start],
            new_text,
            &self.content[range.end..]
        )
        .into();
        let cursor = range.start + new_text.len();
        self.selected_range = cursor..cursor;
        self.marked_range = None;
        self.selection_reversed = false;
        self.changed(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        if self.read_only {
            return;
        }
        if self.marked_range.is_none() {
            self.remember();
        }
        self.content = format!(
            "{}{}{}",
            &self.content[..range.start],
            new_text,
            &self.content[range.end..]
        )
        .into();
        self.selection_reversed = false;
        self.marked_range =
            (!new_text.is_empty()).then_some(range.start..range.start + new_text.len());
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range| utf16_range(new_text, range))
            .map(|new_range| new_range.start + range.start..new_range.end + range.start)
            .unwrap_or_else(|| {
                let cursor = range.start + new_text.len();
                cursor..cursor
            });
        self.changed(cx);
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = self.range_from_utf16(&range_utf16);
        let (start, line, bounds) = self
            .last_lines
            .iter()
            .rev()
            .find(|(start, _, _)| *start <= range.start)?;
        Some(Bounds::from_corners(
            point(
                bounds.left() + line.x_for_index(range.start - start),
                bounds.top(),
            ),
            point(
                bounds.left() + line.x_for_index((range.end - start).min(line.len())),
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.offset_to_utf16(self.index_for_mouse_position(point)))
    }
}

const LINE_HEIGHT: f32 = 21.;

/// Semantic syntax colors, shared by the source and the showcase legend.
#[derive(Clone, Copy)]
pub struct CodeColors {
    pub surface: u32,
    pub head: u32,
    pub border: u32,
    pub text: u32,
    pub key: u32,
    pub string: u32,
    pub number: u32,
    pub type_name: u32,
    pub comment: u32,
    pub selection: u32,
}

impl CodeColors {
    pub const LIGHT: Self = Self {
        surface: 0xfafafc,
        head: 0xededef,
        border: 0xdedee3,
        text: 0x303036,
        key: 0x345d8c,
        string: 0xa24e32,
        number: 0x7a4ea3,
        type_name: 0x92683c,
        comment: 0x787882,
        selection: 0xd9e9fc,
    };
    pub const DARK: Self = Self {
        surface: 0x1b1d23,
        head: 0x292b31,
        border: 0x3c3e45,
        text: 0xe3e3e8,
        key: 0x88b8ef,
        string: 0xefac89,
        number: 0xc8a0ea,
        type_name: 0xdfba85,
        comment: 0x9898a3,
        selection: 0x193654,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenKind {
    Plain,
    Key,
    String,
    Number,
    Type,
    Comment,
}

impl TokenKind {
    fn color(self, colors: CodeColors) -> u32 {
        match self {
            Self::Plain => colors.text,
            Self::Key => colors.key,
            Self::String => colors.string,
            Self::Number => colors.number,
            Self::Type => colors.type_name,
            Self::Comment => colors.comment,
        }
    }
}

/// A small lexer for Rust/Go/JSON/HTML fences; other languages remain plain.
/// Ranges are byte offsets, so Unicode and original whitespace are preserved.
/// Syntax-colored ranges for one source snippet, as plain colors. Shared with
/// `DiffCard` so both cards highlight identically.
pub(crate) fn highlight_runs(
    source: &str,
    language: &str,
    colors: CodeColors,
) -> Vec<(Range<usize>, u32)> {
    tokens(source, language)
        .into_iter()
        .map(|(range, kind)| (range, kind.color(colors)))
        .collect()
}

fn tokens(source: &str, language: &str) -> Vec<(Range<usize>, TokenKind)> {
    if ["html", "htm", "xml", "svg"]
        .iter()
        .any(|name| language.eq_ignore_ascii_case(name))
    {
        return markup_tokens(source);
    }
    if ["css", "javascript", "js"]
        .iter()
        .any(|name| language.eq_ignore_ascii_case(name))
    {
        return web_tokens(source, language.eq_ignore_ascii_case("css"));
    }
    if language.eq_ignore_ascii_case("rust") || language.eq_ignore_ascii_case("rs") {
        return rust_tokens(source);
    }
    let json = language.eq_ignore_ascii_case("json");
    let go = language.eq_ignore_ascii_case("go") || language.eq_ignore_ascii_case("golang");
    if !json && !go {
        return vec![(0..source.len(), TokenKind::Plain)];
    }
    let mut result: Vec<(Range<usize>, TokenKind)> = Vec::new();
    let mut i = 0;
    while i < source.len() {
        let start = i;
        let c = source[i..].chars().next().unwrap();
        let mut kind = TokenKind::Plain;
        if go && source[i..].starts_with("//") {
            i += source[i..].find('\n').unwrap_or(source.len() - i);
            kind = TokenKind::Comment;
        } else if go && source[i..].starts_with("/*") {
            i += source[i + 2..]
                .find("*/")
                .map_or(source.len() - i, |end| end + 4);
            kind = TokenKind::Comment;
        } else if c == '"' || (go && (c == '\'' || c == '`')) {
            i += c.len_utf8();
            while i < source.len() {
                let next = source[i..].chars().next().unwrap();
                i += next.len_utf8();
                if next == c {
                    break;
                }
                if next == '\\' && c != '`' && i < source.len() {
                    i += source[i..].chars().next().unwrap().len_utf8();
                }
            }
            kind = if json && source[i..].trim_start().starts_with(':') {
                TokenKind::Key
            } else {
                TokenKind::String
            };
        } else if c.is_ascii_digit()
            || (json && c == '-' && source[i + 1..].starts_with(|c: char| c.is_ascii_digit()))
        {
            i += c.len_utf8();
            while i < source.len() {
                let next = source[i..].chars().next().unwrap();
                if !(next.is_ascii_alphanumeric()
                    || next == '.'
                    || next == '_'
                    || ((next == '+' || next == '-') && source[..i].ends_with(['e', 'E'])))
                {
                    break;
                }
                i += next.len_utf8();
            }
            kind = TokenKind::Number;
        } else if c.is_alphabetic() || c == '_' {
            i += c.len_utf8();
            while i < source.len() {
                let next = source[i..].chars().next().unwrap();
                if !next.is_alphanumeric() && next != '_' {
                    break;
                }
                i += next.len_utf8();
            }
            let word = &source[start..i];
            if matches!(word, "true" | "false" | "null")
                || (go
                    && matches!(
                        word,
                        "break"
                            | "case"
                            | "chan"
                            | "const"
                            | "continue"
                            | "default"
                            | "defer"
                            | "else"
                            | "fallthrough"
                            | "for"
                            | "func"
                            | "go"
                            | "goto"
                            | "if"
                            | "import"
                            | "interface"
                            | "map"
                            | "package"
                            | "range"
                            | "return"
                            | "select"
                            | "struct"
                            | "switch"
                            | "type"
                            | "var"
                            | "nil"
                    ))
            {
                kind = TokenKind::Number;
            } else if go && source[i..].trim_start().starts_with('(') {
                kind = TokenKind::Key;
            } else if go
                && (c.is_uppercase()
                    || source[i..].starts_with('.')
                    || matches!(
                        word,
                        "error"
                            | "string"
                            | "bool"
                            | "int"
                            | "int64"
                            | "uint"
                            | "byte"
                            | "rune"
                            | "float64"
                    ))
            {
                kind = TokenKind::Type;
            }
        } else {
            i += c.len_utf8();
        }
        if let Some((range, previous)) = result.last_mut()
            && *previous == kind
        {
            range.end = i;
            continue;
        }
        result.push((start..i, kind));
    }
    result
}

fn rust_literal_end(source: &str) -> Option<usize> {
    let prefix = usize::from(source.starts_with('b') || source.starts_with('c'));
    let tail = &source[prefix..];
    if let Some(raw) = tail.strip_prefix('r') {
        let hashes = raw.bytes().take_while(|b| *b == b'#').count();
        if raw.as_bytes().get(hashes) == Some(&b'"') {
            let start = prefix + 2 + hashes;
            let closing = format!("\"{}", "#".repeat(hashes));
            return Some(
                source[start..]
                    .find(&closing)
                    .map_or(source.len(), |end| start + end + closing.len()),
            );
        }
    }
    let quote = tail.chars().next()?;
    if quote == '\'' {
        let rest = &tail[1..];
        let ch = rest.chars().next()?;
        if ch != '\\' {
            // A lifetime/label starts with an apostrophe but has no closing quote.
            let end = 1 + ch.len_utf8();
            return (tail.as_bytes().get(end) == Some(&b'\'')).then_some(prefix + end + 1);
        }
    } else if quote != '"' {
        return None;
    }
    let mut i = prefix + 1;
    while i < source.len() {
        let ch = source[i..].chars().next().unwrap();
        i += ch.len_utf8();
        if ch == quote {
            return Some(i);
        }
        if ch == '\\' && i < source.len() {
            i += source[i..].chars().next().unwrap().len_utf8();
        } else if quote == '\'' && ch == '\n' {
            break;
        }
    }
    Some(i)
}

// Scan the complete document so multiline tags, comments, and raw text retain state.
fn push_web_token(
    result: &mut Vec<(Range<usize>, TokenKind)>,
    range: Range<usize>,
    kind: TokenKind,
) {
    if let Some((last, previous)) = result.last_mut() {
        if *previous == kind && last.end == range.start {
            last.end = range.end;
            return;
        }
    }
    result.push((range, kind));
}
fn markup_tokens(source: &str) -> Vec<(Range<usize>, TokenKind)> {
    let mut result = Vec::new();
    let mut i = 0;
    let lower = source.to_ascii_lowercase();
    while i < source.len() {
        let start = i;
        if source[i..].starts_with("<!--") {
            i = source[i + 4..]
                .find("-->")
                .map_or(source.len(), |end| i + 4 + end + 3);
            push_web_token(&mut result, start..i, TokenKind::Comment);
        } else if source.as_bytes()[i] == b'<' {
            i += 1;
            if i < source.len() && matches!(source.as_bytes()[i], b'/' | b'!' | b'?') {
                i += 1;
            }
            let name_start = i;
            while i < source.len()
                && (source.as_bytes()[i].is_ascii_alphanumeric()
                    || matches!(source.as_bytes()[i], b'-' | b':' | b'_'))
            {
                i += 1;
            }
            let tag = lower[name_start..i].to_owned();
            push_web_token(&mut result, start..i, TokenKind::Type);
            let mut closed = false;
            while i < source.len() {
                let start = i;
                let ch = source[i..].chars().next().unwrap();
                let kind = if ch == '>' {
                    i += 1;
                    closed = true;
                    TokenKind::Type
                } else if ch == '\'' || ch == '"' {
                    i += 1;
                    while i < source.len() {
                        let next = source[i..].chars().next().unwrap();
                        i += next.len_utf8();
                        if next == ch {
                            break;
                        }
                    }
                    TokenKind::String
                } else if ch.is_alphabetic() || matches!(ch, '_' | ':' | '-') {
                    i += ch.len_utf8();
                    while i < source.len() {
                        let next = source[i..].chars().next().unwrap();
                        if !(next.is_alphanumeric() || matches!(next, '_' | ':' | '-' | '.')) {
                            break;
                        }
                        i += next.len_utf8();
                    }
                    TokenKind::Key
                } else {
                    i += ch.len_utf8();
                    TokenKind::Plain
                };
                push_web_token(&mut result, start..i, kind);
                if closed {
                    break;
                }
            }
            if closed && !source[start..].starts_with("</") && (tag == "script" || tag == "style") {
                let closing = format!("</{tag}");
                // Raw-text closing tags need a name boundary, not merely a prefix.
                let end = lower[i..]
                    .match_indices(&closing)
                    .find_map(|(offset, _)| {
                        let end = i + offset + closing.len();
                        (end == source.len()
                            || matches!(
                                source.as_bytes()[end],
                                b'>' | b'/' | b' ' | b'\n' | b'\r' | b'\t'
                            ))
                        .then_some(i + offset)
                    })
                    .unwrap_or(source.len());
                result.extend(
                    web_tokens(&source[i..end], tag == "style")
                        .into_iter()
                        .map(|(range, kind)| ((i + range.start)..(i + range.end), kind)),
                );
                i = end;
            }
        } else if source.as_bytes()[i] == b'&'
            && source[i..]
                .find(';')
                .is_some_and(|end| end < 32 && !source[i..i + end].contains(char::is_whitespace))
        {
            i += source[i..].find(';').unwrap() + 1;
            push_web_token(&mut result, start..i, TokenKind::Number);
        } else {
            i += source[i..].chars().next().unwrap().len_utf8();
            push_web_token(&mut result, start..i, TokenKind::Plain);
        }
    }
    result
}
fn web_tokens(source: &str, css: bool) -> Vec<(Range<usize>, TokenKind)> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < source.len() {
        let start = i;
        let ch = source[i..].chars().next().unwrap();
        let kind = if source[i..].starts_with("/*") {
            i = source[i + 2..]
                .find("*/")
                .map_or(source.len(), |end| i + 2 + end + 2);
            TokenKind::Comment
        } else if !css && source[i..].starts_with("//") {
            i = source[i..].find('\n').map_or(source.len(), |end| i + end);
            TokenKind::Comment
        } else if matches!(ch, '\'' | '"' | '`') {
            i += ch.len_utf8();
            while i < source.len() {
                let next = source[i..].chars().next().unwrap();
                i += next.len_utf8();
                if next == '\\' && i < source.len() {
                    i += source[i..].chars().next().unwrap().len_utf8();
                } else if next == ch {
                    break;
                }
            }
            TokenKind::String
        } else if ch.is_ascii_digit() || (css && ch == '#') {
            i += ch.len_utf8();
            while i < source.len()
                && (source.as_bytes()[i].is_ascii_alphanumeric()
                    || matches!(source.as_bytes()[i], b'.' | b'%'))
            {
                i += 1;
            }
            TokenKind::Number
        } else if ch.is_alphabetic() || matches!(ch, '_' | '$' | '-') {
            i += ch.len_utf8();
            while i < source.len() {
                let next = source[i..].chars().next().unwrap();
                if !(next.is_alphanumeric() || matches!(next, '_' | '$' | '-')) {
                    break;
                }
                i += next.len_utf8();
            }
            let word = &source[start..i];
            if (!css
                && matches!(
                    word,
                    "const"
                        | "let"
                        | "var"
                        | "function"
                        | "return"
                        | "if"
                        | "else"
                        | "for"
                        | "while"
                        | "class"
                        | "new"
                        | "async"
                        | "await"
                        | "import"
                        | "export"
                        | "throw"
                        | "try"
                        | "catch"
                        | "true"
                        | "false"
                        | "null"
                        | "undefined"
                ))
                || (css && source[i..].trim_start().starts_with(':'))
            {
                TokenKind::Key
            } else {
                TokenKind::Plain
            }
        } else {
            i += ch.len_utf8();
            TokenKind::Plain
        };
        push_web_token(&mut result, start..i, kind);
    }
    result
}

fn rust_tokens(source: &str) -> Vec<(Range<usize>, TokenKind)> {
    let mut result: Vec<(Range<usize>, TokenKind)> = Vec::new();
    let mut i = 0;
    while i < source.len() {
        let start = i;
        let tail = &source[i..];
        let ch = tail.chars().next().unwrap();
        let mut kind = TokenKind::Plain;
        if tail.starts_with("//") {
            i += tail.find('\n').unwrap_or(tail.len());
            kind = TokenKind::Comment;
        } else if tail.starts_with("/*") {
            i += 2;
            let mut depth = 1;
            while i < source.len() && depth > 0 {
                if source[i..].starts_with("/*") {
                    depth += 1;
                    i += 2;
                } else if source[i..].starts_with("*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += source[i..].chars().next().unwrap().len_utf8();
                }
            }
            kind = TokenKind::Comment;
        } else if let Some(end) = rust_literal_end(tail) {
            i += end;
            kind = TokenKind::String;
        } else if ch.is_ascii_digit() {
            i += 1;
            while i < source.len() {
                let next = source[i..].chars().next().unwrap();
                if next.is_ascii_alphanumeric()
                    || next == '_'
                    || (next == '.' && source[i + 1..].starts_with(|c: char| c.is_ascii_digit()))
                    || ((next == '+' || next == '-') && source[..i].ends_with(['e', 'E']))
                {
                    i += next.len_utf8();
                } else {
                    break;
                }
            }
            kind = TokenKind::Number;
        } else if ch.is_alphabetic() || ch == '_' || ch == '\'' {
            i += ch.len_utf8();
            if tail.starts_with("r#") {
                i += 1;
            }
            while i < source.len() {
                let next = source[i..].chars().next().unwrap();
                if next.is_alphanumeric() || next == '_' {
                    i += next.len_utf8();
                } else {
                    break;
                }
            }
            let word = &source[start..i];
            kind = if matches!(
                word,
                "as" | "async"
                    | "await"
                    | "break"
                    | "const"
                    | "continue"
                    | "crate"
                    | "dyn"
                    | "else"
                    | "enum"
                    | "extern"
                    | "false"
                    | "fn"
                    | "for"
                    | "if"
                    | "impl"
                    | "in"
                    | "let"
                    | "loop"
                    | "match"
                    | "mod"
                    | "move"
                    | "mut"
                    | "pub"
                    | "ref"
                    | "return"
                    | "self"
                    | "Self"
                    | "static"
                    | "struct"
                    | "super"
                    | "trait"
                    | "true"
                    | "type"
                    | "unsafe"
                    | "use"
                    | "where"
                    | "while"
                    | "abstract"
                    | "become"
                    | "box"
                    | "do"
                    | "final"
                    | "macro"
                    | "override"
                    | "priv"
                    | "try"
                    | "typeof"
                    | "unsized"
                    | "virtual"
                    | "yield"
                    | "gen"
            ) {
                TokenKind::Number
            } else if ch == '\''
                || ch.is_uppercase()
                || matches!(
                    word,
                    "bool"
                        | "char"
                        | "str"
                        | "u8"
                        | "u16"
                        | "u32"
                        | "u64"
                        | "u128"
                        | "usize"
                        | "i8"
                        | "i16"
                        | "i32"
                        | "i64"
                        | "i128"
                        | "isize"
                        | "f32"
                        | "f64"
                )
            {
                TokenKind::Type
            } else if source[i..].trim_start().starts_with(['(', '!'])
                || source[..start]
                    .trim_end()
                    .rsplit(|c: char| !c.is_alphanumeric() && c != '_')
                    .next()
                    == Some("fn")
            {
                TokenKind::Key
            } else {
                TokenKind::Plain
            };
        } else {
            i += ch.len_utf8();
        }
        if let Some((range, previous)) = result.last_mut()
            && *previous == kind
        {
            range.end = i;
        } else {
            result.push((start..i, kind));
        }
    }
    result
}

fn utf16_range(text: &str, range: &Range<usize>) -> Range<usize> {
    let convert = |offset| {
        let mut count = 0;
        for (i, c) in text.char_indices() {
            if count >= offset {
                return i;
            }
            count += c.len_utf16();
        }
        text.len()
    };
    convert(range.start)..convert(range.end)
}

// Use shape_text for rich runs: shape_line merges adjacent fonts with identical decorations.
#[derive(Clone)]
enum EditorLine {
    Plain(ShapedLine),
    Rich(std::sync::Arc<gpui::WrappedLine>),
}
impl std::ops::Deref for EditorLine {
    type Target = gpui::LineLayout;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Plain(line) => line,
            Self::Rich(line) => &line.unwrapped_layout,
        }
    }
}
impl EditorLine {
    fn shape(
        window: &Window,
        rich: bool,
        text: SharedString,
        size: Pixels,
        runs: &[TextRun],
        _: Option<usize>,
    ) -> Self {
        if rich && !text.is_empty() {
            Self::Rich(std::sync::Arc::new(
                window
                    .text_system()
                    .shape_text(text, size, runs, None, None)
                    .unwrap()
                    .remove(0),
            ))
        } else {
            Self::Plain(window.text_system().shape_line(text, size, runs, None))
        }
    }
    fn len(&self) -> usize {
        match self {
            Self::Plain(l) => l.len(),
            Self::Rich(l) => l.len(),
        }
    }
    fn paint(
        &self,
        origin: gpui::Point<Pixels>,
        height: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) {
        match self {
            Self::Plain(l) => l.paint(origin, height, gpui::TextAlign::Left, None, window, cx),
            Self::Rich(l) => l.paint(origin, height, gpui::TextAlign::default(), None, window, cx),
        }
        .unwrap();
    }
    fn paint_background(
        &self,
        origin: gpui::Point<Pixels>,
        height: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) {
        match self {
            Self::Plain(l) => {
                l.paint_background(origin, height, gpui::TextAlign::Left, None, window, cx)
            }
            Self::Rich(l) => {
                l.paint_background(origin, height, gpui::TextAlign::default(), None, window, cx)
            }
        }
        .unwrap();
    }
}

struct CodeLine {
    search_matches: Vec<Range<usize>>,
    input: Entity<CodeCard>,
    start: usize,
    line: EditorLine,
    width: Pixels,
    line_height: Pixels,
}

struct LinePaint {
    inline_code_backgrounds: Vec<gpui::PaintQuad>,
    search_highlights: Vec<gpui::PaintQuad>,
    cursor: Option<gpui::PaintQuad>,
    selection: Option<gpui::PaintQuad>,
}

impl IntoElement for CodeLine {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for CodeLine {
    type RequestLayoutState = ();
    type PrepaintState = LinePaint;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = self.width.into();
        style.size.height = self.line_height.into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> LinePaint {
        let input = self.input.read(cx);
        let focused = input.focus_handle.is_focused(window);
        let end = self.start + self.line.len();
        let selected = input
            .document_selection
            .as_ref()
            .unwrap_or(&input.selected_range);
        let selection_start = selected.start.max(self.start);
        let selection_end = selected.end.min(end);
        let colors = if input.dark {
            CodeColors::DARK
        } else {
            CodeColors::LIGHT
        };
        let inline_code_backgrounds = input
            .rich
            .as_ref()
            .map(|rich| {
                rich.code_ranges(self.start..end)
                    .into_iter()
                    .map(|range| {
                        let left = bounds.left() + self.line.x_for_index(range.start - self.start)
                            - px(3.);
                        let right =
                            bounds.left() + self.line.x_for_index(range.end - self.start) + px(3.);
                        fill(
                            Bounds::from_corners(
                                point(left, bounds.top() + px(2.)),
                                point(right, bounds.bottom() - px(2.)),
                            ),
                            rgb(colors.head),
                        )
                        .corner_radii(px(8.))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let selection = ((focused || input.document_selection.is_some())
            && !selected.is_empty()
            && selection_start <= end
            && selection_end >= self.start)
            .then(|| {
                let right = if selected.end > end {
                    self.line.width + px(8.)
                } else {
                    self.line.x_for_index(selection_end - self.start)
                };
                fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + self.line.x_for_index(selection_start - self.start),
                            bounds.top(),
                        ),
                        point(bounds.left() + right, bounds.bottom()),
                    ),
                    rgb(colors.selection),
                )
            });
        let cursor = (focused
            && input.document_selection.is_none()
            && !input.read_only
            && input.selected_range.is_empty()
            && input.cursor_offset() >= self.start
            && input.cursor_offset() <= end)
            .then(|| {
                fill(
                    Bounds::new(
                        point(
                            bounds.left()
                                + self.line.x_for_index(input.cursor_offset() - self.start),
                            bounds.top(),
                        ),
                        size(px(1.5), self.line_height),
                    ),
                    rgb(0x007aff),
                )
            });
        let search_highlights = self
            .search_matches
            .iter()
            .map(|hit| {
                fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + self.line.x_for_index(hit.start - self.start),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + self.line.x_for_index(hit.end - self.start),
                            bounds.bottom(),
                        ),
                    ),
                    super::search_highlight::color(input.dark),
                )
            })
            .collect();
        LinePaint {
            inline_code_backgrounds,
            cursor,
            selection,
            search_highlights,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        paint: &mut LinePaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus_handle.clone();
        if self.start == 0 && !self.input.read(cx).read_only {
            window.handle_input(
                &focus,
                ElementInputHandler::new(bounds, self.input.clone()),
                cx,
            );
        }
        self.line
            .paint_background(bounds.origin, self.line_height, window, cx);
        for background in paint.inline_code_backgrounds.drain(..) {
            window.paint_quad(background);
        }
        for highlight in paint.search_highlights.drain(..) {
            window.paint_quad(highlight);
        }
        if let Some(selection) = paint.selection.take() {
            window.paint_quad(selection);
        }
        self.line.paint(bounds.origin, self.line_height, window, cx);
        if let Some(cursor) = paint.cursor.take() {
            window.paint_quad(cursor);
        }
        let viewport_width = self.input.read(cx).scroll.bounds().size.width;
        let input_entity = self.input.clone();
        let resized = self.input.update(cx, |input, _| {
            input
                .last_lines
                .push((self.start, self.line.clone(), bounds));
            let resized = self.start == 0
                && input.prose
                && viewport_width > px(0.)
                && input.wrap_width != viewport_width;
            if resized {
                input.wrap_width = viewport_width;
            }
            resized
        });
        if resized {
            cx.defer(move |cx| input_entity.update(cx, |_, cx| cx.notify()));
        }
    }
}

fn viewport_gutter_width(line_count: usize) -> Pixels {
    px((line_count.to_string().len() as f32 * 8. + 20.).max(42.))
}

impl CodeCard {
    fn viewport_wheel(
        &mut self,
        event: &gpui::ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(LINE_HEIGHT));
        if delta.x.abs() > delta.y.abs() {
            let (width, gutter_width) = self
                .viewport_cache
                .as_ref()
                .map_or((px(0.), px(42.)), |cache| {
                    (cache.width, viewport_gutter_width(cache.ranges.len()))
                });
            let bounds = self.viewport_scroll.0.borrow().base_handle.bounds();
            let max = (width + gutter_width + px(26.) - bounds.size.width).max(px(0.));
            let offset = self.scroll.offset();
            let next = (offset.x + delta.x).clamp(-max, px(0.));
            self.scroll.set_offset(point(next, px(0.)));
            cx.stop_propagation();
            if next != offset.x {
                cx.notify();
            }
        }
    }
    fn render_viewport(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        colors: CodeColors,
    ) -> gpui::Div {
        let mut font = window.text_style().font();
        font.family = "Menlo".into();
        let font_size = px(13.);
        let key = LineCacheKey {
            content: self.content.clone(),
            revision: self.layout_revision,
            language: self.language.clone(),
            font: font.clone(),
            color: window.text_style().color,
            font_size,
            wrap_width: px(0.),
            dark: self.dark,
            prose: false,
            rich: false,
            marked: None,
        };
        if self
            .viewport_cache
            .as_ref()
            .is_none_or(|cache| cache.key != key)
        {
            let reset_scroll = self
                .viewport_cache
                .as_ref()
                .is_none_or(|cache| cache.key.content != key.content);
            let mut offset = 0;
            let ranges = self
                .content
                .split('\n')
                .map(|line| {
                    let range = offset..offset + line.len();
                    offset += line.len() + 1;
                    range
                })
                .collect();
            self.viewport_cache = Some(ViewportCache {
                key,
                ranges,
                tokens: tokens(&self.content, &self.language),
                lines: HashMap::new(),
                width: px(0.),
            });
            if reset_scroll {
                self.viewport_scroll
                    .scroll_to_item(0, gpui::ScrollStrategy::Top);
                self.scroll.set_offset(point(px(0.), px(0.)));
            }
        }
        self.last_lines.clear();
        if self
            .search_cache
            .as_ref()
            .is_none_or(|(content, query, _)| {
                content != &self.content || query != &self.search_query
            })
        {
            self.search_cache = Some((
                self.content.clone(),
                self.search_query.clone(),
                Rc::new(super::search_highlight::match_ranges(
                    &self.content,
                    &self.search_query,
                )),
            ));
        }
        let search_matches = self.search_cache.as_ref().unwrap().2.clone();
        let count = self.viewport_cache.as_ref().unwrap().ranges.len();
        let gutter_width = viewport_gutter_width(count);
        let list = gpui::uniform_list(
            "code-card-viewport-lines",
            count,
            cx.processor(move |this, indexes: Range<usize>, window, cx| {
                let content = this.content.clone();
                let input = cx.entity();
                let offset = this.scroll.offset().x;
                let cache = this.viewport_cache.as_mut().unwrap();
                // Bound shaped glyph memory while retaining the document/token index.
                if cache.lines.len() > 512 {
                    cache.lines.clear();
                }
                indexes
                    .map(|index| {
                        let range = cache.ranges[index].clone();
                        let start = range.start;
                        let line = cache
                            .lines
                            .entry(index)
                            .or_insert_with(|| {
                                let runs = token_slice(&cache.tokens, range.clone())
                                    .iter()
                                    .map(|(token, kind)| TextRun {
                                        letter_spacing: None,
                                        len: token.end.min(range.end)
                                            - token.start.max(range.start),
                                        font: font.clone(),
                                        color: gpui::rgb_to_hsla(rgb(kind.color(colors))),
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                    })
                                    .collect::<Vec<_>>();
                                EditorLine::shape(
                                    window,
                                    false,
                                    content[range].to_owned().into(),
                                    font_size,
                                    &runs,
                                    None,
                                )
                            })
                            .clone();
                        cache.width = cache.width.max(line.width);
                        let end = start + line.len();
                        div()
                            .w_full()
                            .h(px(LINE_HEIGHT))
                            .flex()
                            .child(
                                div()
                                    .w(gutter_width)
                                    .h_full()
                                    .flex_shrink_0()
                                    .font_family("Menlo")
                                    .overflow_hidden()
                                    .pr(px(9.))
                                    .text_right()
                                    .border_r_1()
                                    .border_color(rgb(colors.border))
                                    .child((index + 1).to_string()),
                            )
                            .child(
                                div().flex_1().min_w_0().h_full().overflow_hidden().child(
                                    div()
                                        .ml(px(8.) + offset)
                                        .w(line.width + px(18.))
                                        .h_full()
                                        .child(CodeLine {
                                            input: input.clone(),
                                            start,
                                            width: line.width,
                                            line,
                                            line_height: px(LINE_HEIGHT),
                                            search_matches: search_matches
                                                .iter()
                                                .filter_map(|hit| {
                                                    let from = hit.start.max(start);
                                                    let to = hit.end.min(end);
                                                    (from < to).then_some(from..to)
                                                })
                                                .collect(),
                                        }),
                                ),
                            )
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .size_full()
        .track_scroll(&self.viewport_scroll)
        .on_scroll_wheel(cx.listener(Self::viewport_wheel));
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .key_context("GlassCodeCard")
            .track_focus(&self.focus_handle)
            .text_size(font_size)
            .line_height(px(LINE_HEIGHT))
            .text_color(rgb(colors.comment))
            .bg(rgb(colors.surface))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy))
            .children(self.show_header.then(|| {
                div()
                    .h(px(35.))
                    .flex_shrink_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .bg(rgb(colors.head))
                    .child(self.language.clone())
            }))
            .child(div().flex_1().min_h_0().child(list))
    }
}

impl Render for CodeCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = if self.dark {
            CodeColors::DARK
        } else {
            CodeColors::LIGHT
        };
        if self.viewport && self.read_only && !self.prose {
            return self.render_viewport(window, cx, colors);
        }
        let focused = self.focus_handle.is_focused(window);
        let rich_mode = self.rich.is_some();
        let font_size = if rich_mode {
            window.text_style().font_size.to_pixels(window.rem_size())
        } else {
            px(if self.prose { 14. } else { 13. })
        };
        let line_height = if rich_mode {
            // A heading can inherit the paragraph's fixed line height while
            // overriding its font size. Leave room for ascenders/descenders.
            window
                .text_style()
                .line_height_in_pixels(window.rem_size())
                .max(font_size * 1.4)
        } else {
            px(LINE_HEIGHT)
        };
        let anchor = self
            .last_lines
            .iter()
            .find(|(start, line, _)| {
                *start <= self.selected_range.start
                    && self.selected_range.start <= *start + line.len()
            })
            .map(|(start, line, bounds)| {
                point(
                    bounds.left() + line.x_for_index(self.selected_range.start - *start),
                    bounds.top() - px(36.),
                )
            });
        self.last_lines.clear();
        let mut font = window.text_style().font();
        if !self.prose {
            font.family = "Menlo".into();
        }
        let key = LineCacheKey {
            content: self.content.clone(),
            revision: self.layout_revision,
            language: self.language.clone(),
            font: font.clone(),
            color: window.text_style().color,
            font_size,
            wrap_width: self.wrap_width,
            dark: self.dark,
            prose: self.prose,
            rich: rich_mode,
            marked: self.marked_range.clone(),
        };
        if !self
            .line_cache
            .as_ref()
            .is_some_and(|cache| cache.key == key)
        {
            let all_tokens = tokens(&self.content, &self.language);
            let mut shaped = Vec::new();
            let mut width = px(0.);
            let mut start = 0;
            let prose_width = (self.wrap_width - px(if rich_mode { 0. } else { 30. })).max(px(1.));
            let prose_width = if self.wrap_width > px(0.) {
                prose_width
            } else {
                px(560.)
            };
            let mut segments = Vec::new();
            let mut wrap_start = 0;
            for logical in self.content.split('\n') {
                let mut text = logical;
                if self.prose {
                    while !text.is_empty() {
                        let run = TextRun {
                            letter_spacing: None,
                            len: text.len(),
                            font: font.clone(),
                            color: gpui::rgb_to_hsla(rgb(colors.text)),
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let measured = EditorLine::shape(
                            window,
                            rich_mode,
                            text.to_owned().into(),
                            font_size,
                            &self
                                .rich
                                .as_ref()
                                .map(|rich| {
                                    rich.runs(
                                        wrap_start..wrap_start + text.len(),
                                        &font,
                                        window.text_style().color,
                                    )
                                })
                                .unwrap_or_else(|| vec![run]),
                            None,
                        );
                        if measured.width <= prose_width {
                            break;
                        }
                        let mut cut = measured
                            .closest_index_for_x(prose_width)
                            .max(text.chars().next().unwrap().len_utf8());
                        while !text.is_char_boundary(cut) {
                            cut -= 1;
                        }
                        // closest_index can choose the glyph just past the right edge.
                        // Move back to a boundary that actually fits before splitting the line.
                        while cut > text.chars().next().unwrap().len_utf8()
                            && measured.x_for_index(cut) > prose_width
                        {
                            cut = text[..cut].char_indices().last().unwrap().0;
                        }
                        if let Some(space) = text[..cut].rfind(' ')
                            && space > cut / 2
                        {
                            cut = space + 1;
                        }
                        segments.push((text[..cut].to_owned(), false));
                        text = &text[cut..];
                        wrap_start += cut;
                    }
                }
                segments.push((text.to_owned(), true));
                wrap_start += text.len() + 1;
            }
            for (text, newline) in segments {
                let text = text.as_str();
                let end = start + text.len();
                let runs: Vec<TextRun> = token_slice(&all_tokens, start..end)
                    .iter()
                    .filter_map(|(range, kind)| {
                        let from = range.start.max(start);
                        let to = range.end.min(end);
                        (from < to).then(|| TextRun {
                            letter_spacing: None,
                            len: to - from,
                            font: font.clone(),
                            color: gpui::rgb_to_hsla(rgb(kind.color(colors))),
                            background_color: None,
                            underline: self
                                .marked_range
                                .as_ref()
                                .filter(|marked| marked.start < to && marked.end > from)
                                .map(|_| UnderlineStyle {
                                    color: Some(gpui::rgb_to_hsla(rgb(colors.text))),
                                    thickness: px(1.),
                                    wavy: false,
                                }),
                            strikethrough: None,
                        })
                    })
                    .collect();
                let runs = if let Some(rich) = &self.rich {
                    rich.runs(start..end, &font, window.text_style().color)
                } else {
                    runs
                };
                let line = EditorLine::shape(
                    window,
                    rich_mode,
                    text.to_owned().into(),
                    font_size,
                    &runs,
                    None,
                );
                width = width.max(line.width);
                shaped.push((start, line));
                start = end + usize::from(newline);
            }
            self.line_cache = Some(LineCache {
                key,
                lines: Rc::new(shaped),
                width,
            });
        }
        let cached = self.line_cache.as_ref().unwrap();
        let shaped = cached.lines.clone();
        let width = cached.width;
        let cursor_x = shaped
            .iter()
            .find(|(start, line)| {
                self.cursor_offset() >= *start && self.cursor_offset() <= *start + line.len()
            })
            .map_or(px(0.), |(start, line)| {
                line.x_for_index(self.cursor_offset() - *start)
            });
        if !self
            .search_cache
            .as_ref()
            .is_some_and(|(content, query, _)| {
                content == &self.content && query == &self.search_query
            })
        {
            self.search_cache = Some((
                self.content.clone(),
                self.search_query.clone(),
                Rc::new(super::search_highlight::match_ranges(
                    &self.content,
                    &self.search_query,
                )),
            ));
        }
        let search_matches = self.search_cache.as_ref().unwrap().2.clone();
        if self.reveal_cursor && focused && self.scroll.bounds().size.width > px(0.) {
            let viewport = self.scroll.bounds().size.width;
            let offset = self.scroll.offset().x;
            let visible = cursor_x + px(8.) + offset;
            let next = if visible < px(8.) {
                -cursor_x
            } else if visible > viewport - px(18.) {
                viewport - px(26.) - cursor_x
            } else {
                offset
            };
            self.scroll.set_offset(point(next.min(px(0.)), px(0.)));
            self.reveal_cursor = false;
        }
        let mut gutter = div()
            .w(px(42.))
            .flex_shrink_0()
            .py(px(if rich_mode {
                0.
            } else if self.prose {
                3.
            } else {
                13.
            }))
            .flex()
            .flex_col();
        let mut lines = div()
            .min_w_full()
            .w(width + px(if rich_mode { 0. } else { 26. }))
            .flex_shrink_0()
            .py(px(if rich_mode {
                0.
            } else if self.prose {
                3.
            } else {
                13.
            }))
            .flex()
            .flex_col();
        for (index, (start, line)) in shaped.iter().cloned().enumerate() {
            let current = focused
                && self.cursor_offset() >= start
                && self.cursor_offset() <= start + line.len();
            let background = if self.prose {
                if self.dark {
                    CodeColors::DARK.surface
                } else {
                    0xffffff
                }
            } else if current {
                colors.selection
            } else {
                colors.surface
            };
            gutter = gutter.child(
                div()
                    .h(px(LINE_HEIGHT))
                    .pr(px(9.))
                    .text_right()
                    .border_r_1()
                    .border_color(rgb(colors.border))
                    .bg(rgb(background))
                    .child((index + 1).to_string()),
            );
            let mut row = div()
                .min_w_full()
                .h(line_height)
                .pl(px(if rich_mode { 0. } else { 8. }))
                .pr(px(if rich_mode { 0. } else { 18. }));
            if !rich_mode {
                row = row.bg(rgb(background));
            }
            lines = lines.child(
                row.child(CodeLine {
                    search_matches: search_matches
                        .iter()
                        .filter_map(|hit| {
                            let from = hit.start.max(start);
                            let to = hit.end.min(start + line.len());
                            (from < to).then_some(from..to)
                        })
                        .collect(),
                    input: cx.entity(),
                    start,
                    line,
                    width,
                    line_height,
                }),
            );
        }
        let json_valid = self.json_valid();
        let can_format = json_valid == Some(true) && !self.read_only;
        let mut head = div()
            .h(px(35.))
            .flex_shrink_0()
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(rgb(colors.border))
            .bg(rgb(colors.head))
            .text_size(px(10.))
            .text_color(rgb(colors.comment))
            .child(
                super::Icon::for_language(&self.language)
                    .render(13.)
                    .text_color(rgb(colors.comment)),
            )
            .child(if self.language.is_empty() {
                SharedString::from("text")
            } else {
                self.language.clone()
            })
            .child(div().flex_1());
        if let Some(valid) = json_valid {
            head = head.child(
                div()
                    .text_color(rgb(if valid { 0x34c759 } else { 0xff3b30 }))
                    .child(if valid {
                        "✓ 有效 JSON"
                    } else {
                        "请检查 JSON 格式"
                    }),
            );
            let mut button = div()
                .id("format-json")
                .px(px(6.))
                .py(px(4.))
                .rounded(px(4.))
                .text_size(px(11.))
                .text_color(rgb(if can_format { 0x007aff } else { colors.comment }))
                .child("格式化");
            if can_format {
                button = button
                    .cursor_pointer()
                    .hover(|style| style.bg(rgba(0x007aff14)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.format_json(cx);
                    }));
            }
            head = head.child(button);
        }
        let mut source = div()
            .id("code-card-source")
            .key_context("GlassCodeCard")
            .track_focus(&self.focus_handle)
            .tab_index(0)
            .min_w_0()
            .w_full()
            .text_size(font_size)
            .line_height(line_height)
            .text_color(rgb(colors.comment))
            .cursor(gpui::CursorStyle::IBeam)
            .on_action(cx.listener(Self::rich_bold))
            .on_action(cx.listener(Self::rich_italic))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::soft_newline))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" && this.document_managed {
                    cx.emit(CodeCardEvent::CommandKey("escape".into()));
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(Self::indent))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::continue_after))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .min_h(if rich_mode {
                line_height
            } else {
                px(if self.prose { 27. } else { 89. })
            })
            .flex();
        if !self.prose {
            source = source.child(gutter);
        }
        source = source.child(
            div()
                .id("code-card-horizontal-scroll")
                .w(px(0.))
                .min_w_0()
                .flex_1()
                .overflow_x_hidden()
                .track_scroll(&self.scroll)
                .on_scroll_wheel(cx.listener(Self::scroll_wheel))
                .child(lines),
        );
        if self.prose {
            let mut container = div()
                .relative()
                .w_full()
                .max_w_full()
                .min_w_0()
                .child(source);
            if rich_mode
                && self.document_selection.is_none()
                && !self.read_only
                && !self.is_selecting
                && !self.selected_range.is_empty()
                && (focused || self.link_editor.is_some())
                && let Some(anchor) = anchor
            {
                let mut toolbar = div()
                    .id("rich-format-toolbar")
                    .flex()
                    .items_center()
                    .gap_1()
                    .p_1()
                    .rounded(px(7.))
                    .bg(rgb(colors.surface))
                    .border_1()
                    .border_color(rgb(colors.border))
                    .shadow_md()
                    .text_size(px(12.))
                    .text_color(rgb(colors.text))
                    .font_weight(gpui::FontWeight::NORMAL)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.stop_propagation()),
                    );
                for (label, format) in [
                    ("B", Some(Format::Bold)),
                    ("I", Some(Format::Italic)),
                    ("S", Some(Format::Strike)),
                    ("</>", Some(Format::Code)),
                    ("清除", None),
                ] {
                    toolbar = toolbar.child(
                        div()
                            .id(label)
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgba(0x007aff12)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.apply_format(format.clone(), window, cx);
                            }))
                            .child(label),
                    );
                }
                toolbar = toolbar.child(
                    div()
                        .id("rich-link")
                        .px_2()
                        .py_1()
                        .cursor_pointer()
                        .child("链接")
                        .on_click(cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            let field = cx.new(|cx| {
                                CodeCard::new(cx, "text", "https://")
                                    .prose(true)
                                    .dark(this.dark)
                            });
                            window.focus(&field.focus_handle(cx), cx);
                            this.link_editor = Some(field);
                            cx.notify();
                        })),
                );
                if let Some(field) = &self.link_editor {
                    toolbar = toolbar.child(div().w(px(180.)).child(field.clone())).child(
                        div()
                            .id("apply-rich-link")
                            .px_2()
                            .cursor_pointer()
                            .child("应用")
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                let url = this
                                    .link_editor
                                    .as_ref()
                                    .map(|field| field.read(cx).content().to_owned())
                                    .unwrap_or_default();
                                if !url.trim().is_empty() {
                                    this.apply_format(
                                        Some(Format::Link(url.trim().to_owned())),
                                        window,
                                        cx,
                                    );
                                }
                            })),
                    );
                }
                container = container.child(
                    gpui::deferred(
                        gpui::anchored()
                            .position(anchor)
                            .snap_to_window()
                            .child(toolbar),
                    )
                    .with_priority(40),
                );
            }
            return container;
        }
        div()
            .w_full()
            .max_w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .rounded(px(9.))
            .border_1()
            .border_color(rgb(colors.border))
            .overflow_hidden()
            .bg(rgb(colors.surface))
            .children(self.show_header.then_some(head))
            .child(source)
    }
}

impl Focusable for CodeCard {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn html_highlights_multiline_markup_raw_text_and_preserves_unicode() {
        let source = "<!-- 注释\n中文 -->\n<DIV data-name=\"多行\n属性 >\">雾 &amp; 夜</DIV><style>p { color: #fff; margin: 12px; }</style><script>const text = '<div>'; // 中文\n</script>";
        let ranges = tokens(source, "HTML");
        assert!(
            ranges
                .iter()
                .any(|(r, k)| *k == TokenKind::Comment && source[r.clone()].contains("注释\n中文"))
        );
        assert!(
            ranges
                .iter()
                .any(|(r, k)| *k == TokenKind::String && &source[r.clone()] == "\"多行\n属性 >\"")
        );
        assert!(
            ranges
                .iter()
                .any(|(r, k)| *k == TokenKind::Key && &source[r.clone()] == "data-name")
        );
        assert!(
            ranges
                .iter()
                .any(|(r, k)| *k == TokenKind::Key && &source[r.clone()] == "color")
        );
        assert!(
            ranges
                .iter()
                .any(|(r, k)| *k == TokenKind::Key && &source[r.clone()] == "const")
        );
        assert!(
            ranges
                .iter()
                .any(|(r, k)| *k == TokenKind::String && &source[r.clone()] == "'<div>'")
        );
        let mut end = 0;
        for (r, _) in ranges {
            assert_eq!(r.start, end);
            assert!(source.is_char_boundary(r.end));
            end = r.end;
        }
        assert_eq!(end, source.len());
        for source in [
            "<div title=\"未完成",
            "<!-- 未完成",
            "<style>p { color: red",
            "中文 &amp; <",
            "<script>let a = '</scripture>'; </script>",
        ] {
            let mut end = 0;
            for (r, _) in tokens(source, "html") {
                assert_eq!(r.start, end);
                assert!(r.end > r.start && source.is_char_boundary(r.end));
                end = r.end;
            }
            assert_eq!(end, source.len());
        }
    }

    #[gpui::test]
    fn large_readonly_viewport_shapes_visible_rows_and_scrolls(cx: &mut gpui::TestAppContext) {
        // Four/five digit line numbers need their own width rather than wrapping
        // inside the fixed gutter used by short editable snippets.
        assert_eq!(viewport_gutter_width(6749), px(52.));
        assert_eq!(viewport_gutter_width(20_001), px(60.));
        struct Viewer(Entity<CodeCard>);
        impl Render for Viewer {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(px(400.)).h(px(160.)).child(self.0.clone())
            }
        }
        let source = (0..20_000)
            .map(|i| format!("<div class=\"row\" data-index=\"{i}\">中文 &amp; content</div>\n"))
            .collect::<String>();
        let started = std::time::Instant::now();
        let mut editor = None;
        let (_, window) = cx.add_window_view(|_, cx| {
            let input = cx.new(|cx| {
                CodeCard::new(cx, "html", source)
                    .read_only(true)
                    .show_header(false)
                    .viewport()
            });
            editor = Some(input.clone());
            Viewer(input)
        });
        let editor = editor.unwrap();
        editor.update(window, |input, _| {
            let cache = input.viewport_cache.as_ref().unwrap();
            assert_eq!(cache.ranges.len(), 20_001);
            assert!(cache.lines.len() < 40, "shaped {} rows", cache.lines.len());
            assert!(input.line_cache.is_none());
            eprintln!(
                "1.2 MiB HTML first layout: {:?}, {} shaped rows",
                started.elapsed(),
                cache.lines.len()
            );
        });
        let bounds = editor.update(window, |input, _| {
            input.viewport_scroll.0.borrow().base_handle.bounds()
        });
        window.simulate_event(gpui::ScrollWheelEvent {
            position: bounds.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-300.))),
            ..Default::default()
        });
        window.run_until_parked();
        editor.update(window, |input, cx| {
            assert!(input.viewport_scroll.0.borrow().base_handle.offset().y < px(0.));
            assert!(input.last_lines.iter().any(|(start, _, _)| *start > 0));
            let revision = input.layout_revision;
            let content = input.content.clone();
            input.set_content(content, cx);
            input.set_language("html", cx);
            assert_eq!(input.layout_revision, revision);
            assert!(input.history.is_empty());
            assert!(input.viewport_cache.as_ref().unwrap().lines.len() < 80);
        });
        window.simulate_event(gpui::ScrollWheelEvent {
            position: bounds.center(),
            delta: gpui::ScrollDelta::Pixels(point(px(-150.), px(0.))),
            ..Default::default()
        });
        window.run_until_parked();
        editor.update(window, |input, _| assert!(input.scroll.offset().x < px(0.)));
    }

    #[gpui::test]
    fn heading_line_height_grows_with_the_font_size(cx: &mut gpui::TestAppContext) {
        struct Heading(Entity<CodeCard>);
        impl Render for Heading {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .w(px(320.))
                    .text_size(px(28.))
                    .line_height(px(24.))
                    .child(self.0.clone())
            }
        }
        let mut editor = None;
        let (_, window) = cx.add_window_view(|_, cx| {
            let input = cx.new(|cx| CodeCard::new(cx, "text", "").rich_text("gpui-glint 标题"));
            editor = Some(input.clone());
            Heading(input)
        });
        editor.unwrap().update(window, |editor, _| {
            assert!(!editor.last_lines.is_empty());
            for (_, _, bounds) in &editor.last_lines {
                assert!(
                    bounds.size.height >= px(39.),
                    "heading line bounds: {bounds:?}"
                );
            }
        });
    }

    #[gpui::test]
    fn prose_wraps_to_its_own_viewport_instead_of_the_window(cx: &mut gpui::TestAppContext) {
        struct NarrowEditor(Entity<CodeCard>);
        impl Render for NarrowEditor {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(px(320.)).child(self.0.clone())
            }
        }
        let mut editor = None;
        let (_, window) = cx.add_window_view(|_, cx| {
            let input = cx.new(|cx| {
                CodeCard::new(cx, "text", "").rich_text(&"中文正文和 **粗体关键词** ".repeat(30))
            });
            editor = Some(input.clone());
            NarrowEditor(input)
        });
        editor.unwrap().update(window, |editor, _| {
            let viewport = editor.scroll.bounds().size.width;
            assert!(viewport <= px(320.));
            assert_eq!(editor.wrap_width, viewport);
            assert!(editor.line_cache.as_ref().unwrap().width <= viewport + px(1.));
        });
    }

    #[gpui::test]
    fn long_code_scrolls_inside_a_narrow_card(cx: &mut gpui::TestAppContext) {
        struct NarrowEditor(Entity<CodeCard>, ScrollHandle);
        impl Render for NarrowEditor {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .id("document-scroll-test")
                    .w(px(320.))
                    .h(px(180.))
                    .overflow_y_scroll()
                    .track_scroll(&self.1)
                    .child(self.0.clone())
                    .child(div().h(px(1000.)))
            }
        }
        let mut editor = None;
        let parent_scroll = ScrollHandle::new();
        let (_, window) = cx.add_window_view(|_, cx| {
            let input = cx.new(|cx| CodeCard::new(cx, "rust", &"long_identifier ".repeat(30)));
            editor = Some(input.clone());
            NarrowEditor(input, parent_scroll.clone())
        });
        let editor = editor.unwrap();
        let position = editor.update(window, |editor, _| {
            let bounds = editor.scroll.bounds();
            assert!(bounds.size.width <= px(320.));
            assert!(editor.line_cache.as_ref().unwrap().width > bounds.size.width);
            bounds.center()
        });
        window.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(-200.), px(-50.))),
            touch_phase: gpui::TouchPhase::Started,
            ..Default::default()
        });
        editor.update(window, |editor, _| {
            assert!(editor.scroll.offset().x < px(0.));
        });
        assert_eq!(parent_scroll.offset().y, px(0.));
        // A later vertical-dominant event cannot switch the horizontal gesture.
        window.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(-1.), px(-100.))),
            ..Default::default()
        });
        assert_eq!(parent_scroll.offset().y, px(0.));
        let x = editor.update(window, |editor, _| editor.scroll.offset().x);
        window.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(-50.), px(-30.))),
            touch_phase: gpui::TouchPhase::Ended,
            ..Default::default()
        });
        assert_eq!(parent_scroll.offset().y, px(0.));
        let ended_x = editor.update(window, |editor, _| editor.scroll.offset().x);
        assert!(ended_x < x);
        window.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(-50.), px(-30.))),
            touch_phase: gpui::TouchPhase::Started,
            ..Default::default()
        });
        // Repeated horizontal gestures, including at the end, never leak vertically.
        window.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(-100000.), px(-20.))),
            ..Default::default()
        });
        assert_eq!(parent_scroll.offset().y, px(0.));
        let x = editor.update(window, |editor, _| editor.scroll.offset().x);
        window.simulate_event(gpui::ScrollWheelEvent {
            position,
            delta: gpui::ScrollDelta::Pixels(point(px(-10.), px(-40.))),
            touch_phase: gpui::TouchPhase::Started,
            ..Default::default()
        });
        assert!(parent_scroll.offset().y < px(0.));
        editor.update(window, |editor, _| assert_eq!(editor.scroll.offset().x, x));
    }

    #[gpui::test]
    fn line_cache_reuses_layout_but_refreshes_for_formatting_content_and_width(
        cx: &mut gpui::TestAppContext,
    ) {
        let (editor, window) = cx.add_window_view(|_, cx| {
            CodeCard::new(cx, "text", "")
                .rich_text("文字关键词和 **粗体**")
                .prose(true)
        });
        let cached = editor.update(window, |editor, cx| {
            let cached = editor.line_cache.as_ref().unwrap().lines.clone();
            editor.set_search_query("关键词", cx);
            cached
        });
        window.run_until_parked();
        editor.update(window, |editor, cx| {
            assert!(Rc::ptr_eq(
                &cached,
                &editor.line_cache.as_ref().unwrap().lines
            ));
            assert_eq!(editor.search_cache.as_ref().unwrap().2.len(), 1);
            editor
                .rich
                .as_mut()
                .unwrap()
                .toggle(0..6, Some(Format::Italic));
            editor.changed(cx);
        });
        window.run_until_parked();
        let formatted = editor.update(window, |editor, cx| {
            assert!(!Rc::ptr_eq(
                &cached,
                &editor.line_cache.as_ref().unwrap().lines
            ));
            let formatted = editor.line_cache.as_ref().unwrap().lines.clone();
            editor.set_content("改过的关键词", cx);
            formatted
        });
        window.run_until_parked();
        editor.update(window, |editor, _| {
            assert!(!Rc::ptr_eq(
                &formatted,
                &editor.line_cache.as_ref().unwrap().lines
            ));
        });
        window.simulate_resize(size(px(500.), px(400.)));
        window.run_until_parked();
        editor.update(window, |editor, _| {
            assert_eq!(
                editor.line_cache.as_ref().unwrap().key.wrap_width,
                editor.wrap_width
            );
        });
    }

    #[test]
    fn rust_highlights_literals_lifetimes_macros_and_nested_comments() {
        let source = "pub fn render<'a>(value: &'a str) -> Option<u32> {\n /* outer /* 中文 */ end */\n let text = br##\"raw \\\" 中文\ntext\"##; let ch = '\\u{4e2d}';\n println!(\"{}\", text); let n = 0xff_u32; let range = 1..=3;\n let r#type = '中'; // comment\n}";
        let ranges = tokens(source, "Rust");
        for (text, kind) in [
            ("pub", TokenKind::Number),
            ("fn", TokenKind::Number),
            ("render", TokenKind::Key),
            ("'a", TokenKind::Type),
            ("str", TokenKind::Type),
            ("Option", TokenKind::Type),
            ("/* outer /* 中文 */ end */", TokenKind::Comment),
            ("br##\"raw \\\" 中文\ntext\"##", TokenKind::String),
            ("'\\u{4e2d}'", TokenKind::String),
            ("'中'", TokenKind::String),
            ("println", TokenKind::Key),
            ("0xff_u32", TokenKind::Number),
            ("1", TokenKind::Number),
            ("3", TokenKind::Number),
            ("// comment", TokenKind::Comment),
        ] {
            assert!(
                ranges
                    .iter()
                    .any(|(range, actual)| &source[range.clone()] == text && *actual == kind),
                "missing {text:?}: {ranges:?}"
            );
        }
        assert!(ranges.iter().any(
            |(range, kind)| source[range.clone()].contains("..=") && *kind == TokenKind::Plain
        ));
        assert_eq!(ranges, tokens(source, "rs"));
        let mut end = 0;
        for (range, _) in ranges {
            assert_eq!(range.start, end);
            assert!(source.is_char_boundary(range.end));
            end = range.end;
        }
        assert_eq!(end, source.len());
    }

    #[test]
    fn rust_incomplete_source_stays_highlightable_while_typing() {
        for source in [
            "/* 未完成 /* 嵌套 */",
            "r###\"未完成",
            "b\"未完成\\",
            "'",
            "'static",
            "r#type",
            "'\\u{",
            "c\"文本\"",
            "1.5e-3f64",
        ] {
            let ranges = tokens(source, "rust");
            assert_eq!(ranges.first().unwrap().0.start, 0);
            assert_eq!(ranges.last().unwrap().0.end, source.len());
            for (range, _) in &ranges {
                assert!(source.get(range.clone()).is_some());
            }
        }
    }

    #[test]
    fn json_keys_escapes_numbers_and_unicode() {
        let source = r#"{"名字":"a\"b", "enabled":true,"limit":-2.5e+3,"empty":null}"#;
        let ranges = tokens(source, "json");
        assert_eq!(
            ranges
                .iter()
                .map(|(r, _)| &source[r.clone()])
                .collect::<String>(),
            source
        );
        assert!(
            ranges
                .iter()
                .any(|(r, k)| &source[r.clone()] == "\"名字\"" && *k == TokenKind::Key)
        );
        assert!(
            ranges
                .iter()
                .any(|(r, k)| &source[r.clone()] == "-2.5e+3" && *k == TokenKind::Number)
        );
        assert!(
            ranges
                .iter()
                .any(|(r, k)| &source[r.clone()] == r#""a\"b""# && *k == TokenKind::String)
        );
    }

    #[test]
    fn go_multiline_comments_and_raw_strings() {
        let source = "/* 中文\ncomment */\nfunc Run() {\n  text := `hello\nworld` // done\n}";
        let ranges = tokens(source, "go");
        assert!(
            ranges
                .iter()
                .any(|(r, k)| &source[r.clone()] == "/* 中文\ncomment */"
                    && *k == TokenKind::Comment)
        );
        assert!(
            ranges
                .iter()
                .any(|(r, k)| &source[r.clone()] == "`hello\nworld`" && *k == TokenKind::String)
        );
        assert!(
            ranges
                .iter()
                .any(|(r, k)| &source[r.clone()] == "Run" && *k == TokenKind::Key)
        );
    }

    #[test]
    fn unknown_language_is_plain_text() {
        assert_eq!(
            tokens("<div>中文</div>", "unknown"),
            vec![(0.."<div>中文</div>".len(), TokenKind::Plain)]
        );
    }

    #[test]
    fn ime_offsets_are_relative_to_composition() {
        assert_eq!(utf16_range("中😀文", &(1..3)), 3..7);
    }
}
