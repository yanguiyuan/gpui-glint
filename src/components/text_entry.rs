use super::text_completion::{CompletionKey, CompletionState, TextCompletion, completion_menu};
use super::text_reference::{TextReferenceResolver, reference_colors, references};
use crate::theme::ThemePreset;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

use gpui::{
    App, Bounds, Context, Element, ElementId, ElementInputHandler, Entity, EntityInputHandler,
    FocusHandle, Focusable, GlobalElementId, InspectorElementId, InteractiveElement, IntoElement,
    KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    Pixels, Render, ShapedLine, SharedString, StatefulInteractiveElement, Style, Styled,
    Subscription, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill, point, px,
    relative, rgb, size,
};

actions!(
    glass_text_entry,
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
    ]
);

/// Registers the standard editing shortcuts used by [`TextEntry`].
pub fn bind_text_entry_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, None),
        KeyBinding::new("delete", Delete, None),
        KeyBinding::new("left", Left, None),
        KeyBinding::new("right", Right, None),
        KeyBinding::new("shift-left", SelectLeft, None),
        KeyBinding::new("shift-right", SelectRight, None),
        KeyBinding::new("cmd-a", SelectAll, None),
        KeyBinding::new("home", Home, None),
        KeyBinding::new("end", End, None),
        KeyBinding::new("cmd-v", Paste, None),
        KeyBinding::new("cmd-x", Cut, None),
        KeyBinding::new("cmd-c", Copy, None),
        KeyBinding::new("ctrl-a", SelectAll, Some("GlassTextEntry")),
        KeyBinding::new("ctrl-v", Paste, Some("GlassTextEntry")),
        KeyBinding::new("ctrl-x", Cut, Some("GlassTextEntry")),
        KeyBinding::new("ctrl-c", Copy, Some("GlassTextEntry")),
    ]);
}

/// A single-line native text entry with IME composition, selection, clipboard, and caret support.
pub struct TextEntry {
    read_only: bool,
    completions: CompletionState,
    fit_content: bool,
    reference_resolver: Option<TextReferenceResolver>,
    focus_subscriptions: Vec<Subscription>,
    search_query: SharedString,
    theme: ThemePreset,
    font_size: f32,
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    scroll_x: Pixels,
    is_selecting: bool,
}

impl TextEntry {
    /// Offer template expressions after `{{`; empty by default.
    pub fn set_completions(&mut self, items: Vec<TextCompletion>, cx: &mut Context<Self>) {
        if self.completions.set_items(items) {
            cx.notify();
        }
    }
    fn choose_completion(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only || self.marked_range.is_some() {
            return;
        }
        if let Some(active) = self
            .completions
            .active(&self.content, &self.selected_range, false)
            && let Some(item) = active.items.get(index)
        {
            self.selected_range = active.range;
            self.replace_selection(&format!("{{{{{}}}}}", item.expression), cx);
            window.focus(&self.focus_handle, cx);
        }
    }
    fn completion_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = &event.keystroke.modifiers;
        if self.read_only
            || modifiers.platform
            || modifiers.control
            || modifiers.alt
            || modifiers.shift
        {
            return;
        }
        if let Some(action) = self.completions.key(
            &self.content,
            &self.selected_range,
            self.marked_range.is_some(),
            &event.keystroke.key,
        ) {
            if let CompletionKey::Insert(range, text) = action {
                self.selected_range = range;
                self.replace_selection(&text, cx);
            }
            cx.stop_propagation();
            window.prevent_default();
            cx.notify();
        }
    }

    /// Allow selection and copying while preventing keyboard and IME edits.
    pub fn read_only(mut self, value: bool) -> Self {
        self.read_only = value;
        self
    }
    /// Keep the full text width, for example inside a horizontally scrolling cell.
    pub fn fit_content(mut self) -> Self {
        self.fit_content = true;
        self
    }
    /// Opt into resolved reference badges while blurred and expression highlights while editing.
    /// The resolver receives the complete `{{name}}` expression. Errors use warning colors.
    pub fn set_reference_resolver(
        &mut self,
        resolver: Option<TextReferenceResolver>,
        cx: &mut Context<Self>,
    ) {
        self.reference_resolver = resolver;
        cx.notify();
    }
    pub(super) fn set_search_query(&mut self, query: SharedString, cx: &mut Context<Self>) {
        if self.search_query != query {
            self.search_query = query;
            cx.notify();
        }
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            cx.notify();
        }
    }

    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size.max(1.);
        self
    }
    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }
    /// Returns the current input, including edits made through the native input handler.
    pub fn value(&self) -> &str {
        &self.content
    }

    /// Whether the native input method is composing text.
    pub fn is_composing(&self) -> bool {
        self.marked_range.is_some()
    }

    /// Replaces the input and resets selection and IME composition.
    pub fn set_value(&mut self, value: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = value.into();
        let end = self.content.len();
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        self.scroll_x = px(0.);
        cx.notify();
    }

    pub fn new(
        cx: &mut Context<Self>,
        placeholder: impl Into<SharedString>,
        value: impl Into<SharedString>,
    ) -> Self {
        let content = value.into();
        let end = content.len();
        Self {
            read_only: false,
            completions: CompletionState::default(),
            fit_content: false,
            reference_resolver: None,
            focus_subscriptions: vec![],
            search_query: "".into(),
            theme: ThemePreset::default(),
            font_size: 13.,
            focus_handle: cx.focus_handle(),
            content,
            placeholder: placeholder.into(),
            selected_range: end..end,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            scroll_x: px(0.),
            is_selecting: false,
        }
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
        cx.notify();
    }

    fn replace_selection(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.read_only {
            return;
        }
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
        cx.notify();
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
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.read_only {
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
            self.replace_selection(&text.replace(['\n', '\r'], " "), cx);
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
        if self.read_only {
            return;
        }
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
        self.is_selecting = true;
        window.focus(&self.focus_handle, cx);
        let offset = self.index_for_mouse_position(event.position);
        if event.click_count >= 2 {
            self.is_selecting = false;
            self.selected_range = if event.click_count >= 3 {
                0..self.content.len()
            } else {
                word_range_at(&self.content, offset)
            };
            self.selection_reversed = false;
            cx.notify();
        } else if event.modifiers.shift {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn index_for_mouse_position(&self, position: gpui::Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (&self.last_bounds, &self.last_layout) else {
            return 0;
        };
        line.closest_index_for_x(position.x - bounds.left() + self.scroll_x)
    }
}

impl EntityInputHandler for TextEntry {
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
        if self.read_only {
            return;
        }
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
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
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.read_only {
            return;
        }
        let range = range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked_range.clone())
            .unwrap_or_else(|| self.selected_range.clone());
        self.content = format!(
            "{}{}{}",
            &self.content[..range.start],
            new_text,
            &self.content[range.end..]
        )
        .into();
        self.marked_range =
            (!new_text.is_empty()).then_some(range.start..range.start + new_text.len());
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range| self.range_from_utf16(range))
            .map(|new_range| new_range.start + range.start..new_range.end + range.start)
            .unwrap_or_else(|| {
                let cursor = range.start + new_text.len();
                cursor..cursor
            });
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(
                bounds.left() + line.x_for_index(range.start) - self.scroll_x,
                bounds.top(),
            ),
            point(
                bounds.left() + line.x_for_index(range.end) - self.scroll_x,
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
        let bounds = self.last_bounds?;
        let line = self.last_layout.as_ref()?;
        Some(
            self.offset_to_utf16(line.closest_index_for_x(point.x - bounds.left() + self.scroll_x)),
        )
    }
}

struct TextEntryElement {
    input: Entity<TextEntry>,
}

struct TextEntryPrepaint {
    scroll_x: Pixels,
    reference_highlights: Vec<gpui::PaintQuad>,
    search_highlights: Vec<gpui::PaintQuad>,
    line: Option<ShapedLine>,
    cursor: Option<gpui::PaintQuad>,
    selection: Option<gpui::PaintQuad>,
}

impl IntoElement for TextEntryElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextEntryElement {
    type RequestLayoutState = ();
    type PrepaintState = TextEntryPrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let theme = input.theme;
        let placeholder = input.content.is_empty();
        let display_text = if placeholder {
            input.placeholder.clone()
        } else {
            input.content.clone()
        };
        let style = window.text_style();
        let run = TextRun {
            letter_spacing: None,
            len: display_text.len(),
            font: style.font(),
            color: if placeholder {
                gpui::rgb_to_hsla(rgb(theme.color(0x92979a)))
            } else {
                style.color
            },
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        // Split at both selection and IME boundaries so selected glyphs use
        // white ink while composition underlines remain intact.
        let focused = input.focus_handle.is_focused(window);
        let refs = input
            .reference_resolver
            .as_ref()
            .map(|resolver| references(&input.content, resolver))
            .unwrap_or_default();
        let mut boundaries = vec![0, display_text.len()];
        for reference in &refs {
            boundaries.extend([reference.range.start, reference.range.end]);
        }
        if !placeholder {
            boundaries.extend([input.selected_range.start, input.selected_range.end]);
            if let Some(marked) = &input.marked_range {
                boundaries.extend([marked.start, marked.end]);
            }
        }
        let boundaries = text_run_boundaries(&display_text, boundaries);
        let mut runs: Vec<_> = boundaries
            .windows(2)
            .map(|range| {
                let start = range[0];
                let selected = !placeholder
                    && focused
                    && start >= input.selected_range.start
                    && start < input.selected_range.end;
                let color = if selected {
                    gpui::rgb_to_hsla(rgb(0xffffff))
                } else if let Some(reference) = refs
                    .iter()
                    .find(|reference| reference.range.contains(&start))
                {
                    gpui::rgb_to_hsla(rgb(reference_colors(theme, reference.value.is_err()).0))
                } else {
                    run.color
                };
                let composing = !placeholder
                    && input
                        .marked_range
                        .as_ref()
                        .is_some_and(|marked| start >= marked.start && start < marked.end);
                TextRun {
                    letter_spacing: None,
                    len: range[1] - start,
                    color,
                    underline: composing.then_some(UnderlineStyle {
                        color: Some(color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..run.clone()
                }
            })
            .collect();
        // Empty input still needs a zero-length font run for native font metrics.
        if runs.is_empty() {
            runs.push(run);
        }
        let line = window.text_system().shape_line(
            display_text,
            style.font_size.to_pixels(window.rem_size()),
            &runs,
            None,
        );
        let scroll_x = if focused && !input.fit_content {
            let caret = line.x_for_index(input.cursor_offset().min(line.len()));
            let available = (bounds.size.width - px(2.)).max(px(1.));
            let maximum = (line.width - available).max(px(0.));
            let mut offset = input.scroll_x.min(maximum);
            if caret < offset {
                offset = caret;
            }
            if caret > offset + available {
                offset = caret - available;
            }
            offset.clamp(px(0.), maximum)
        } else {
            px(0.)
        };
        let bounds = Bounds::new(point(bounds.left() - scroll_x, bounds.top()), bounds.size);
        let show_cursor = !input.read_only
            && input.focus_handle.is_focused(window)
            && input.selected_range.is_empty();
        let (selection, cursor) = if input.selected_range.is_empty() || placeholder {
            let cursor = show_cursor.then(|| {
                fill(
                    Bounds::new(
                        point(
                            bounds.left() + line.x_for_index(input.cursor_offset()),
                            bounds.top(),
                        ),
                        size(px(1.5), bounds.bottom() - bounds.top()),
                    ),
                    rgb(theme.color(0x087bfa)),
                )
            });
            (None, cursor)
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + line.x_for_index(input.selected_range.start),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + line.x_for_index(input.selected_range.end),
                            bounds.bottom(),
                        ),
                    ),
                    if focused {
                        rgb(theme.color(0x007aff))
                    } else {
                        rgb(theme.color(0xe1e5e9))
                    },
                )),
                None,
            )
        };
        TextEntryPrepaint {
            scroll_x,
            reference_highlights: refs
                .iter()
                .map(|reference| {
                    let mut background = fill(
                        Bounds::from_corners(
                            point(
                                bounds.left() + line.x_for_index(reference.range.start),
                                bounds.top() + px(1.),
                            ),
                            point(
                                bounds.left() + line.x_for_index(reference.range.end),
                                bounds.bottom() - px(1.),
                            ),
                        ),
                        rgb(reference_colors(theme, reference.value.is_err()).1),
                    );
                    background.corner_radii = px(4.).into();
                    background
                })
                .collect(),
            search_highlights: if placeholder {
                Vec::new()
            } else {
                super::search_highlight::match_ranges(&input.content, &input.search_query)
                    .into_iter()
                    .map(|hit| {
                        fill(
                            Bounds::from_corners(
                                point(bounds.left() + line.x_for_index(hit.start), bounds.top()),
                                point(bounds.left() + line.x_for_index(hit.end), bounds.bottom()),
                            ),
                            super::search_highlight::color(theme.is_dark()),
                        )
                    })
                    .collect()
            },
            line: Some(line),
            cursor,
            selection,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        for highlight in prepaint.search_highlights.drain(..) {
            window.paint_quad(highlight);
        }
        for highlight in prepaint.reference_highlights.drain(..) {
            window.paint_quad(highlight);
        }
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection);
        }
        let line = prepaint.line.take().unwrap();
        line.paint(
            bounds.origin - point(prepaint.scroll_x, px(0.)),
            window.line_height(),
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        )
        .unwrap();
        if focus_handle.is_focused(window) {
            if let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
        }
        self.input.update(cx, |input, cx| {
            let first_layout = input.last_bounds.is_none();
            input.scroll_x = prepaint.scroll_x;
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
            if first_layout
                && !input.completions.items.is_empty()
                && focus_handle.is_focused(window)
            {
                cx.notify();
            }
        });
    }
}

impl Render for TextEntry {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;

        if self.focus_subscriptions.is_empty() {
            self.focus_subscriptions
                .push(cx.on_focus(&self.focus_handle, window, |_, _, cx| cx.notify()));
            self.focus_subscriptions
                .push(cx.on_blur(&self.focus_handle, window, |_, _, cx| cx.notify()));
        }
        let refs = self
            .reference_resolver
            .as_ref()
            .map(|resolver| references(&self.content, resolver))
            .unwrap_or_default();
        let preview = !self.focus_handle.is_focused(window) && !refs.is_empty();
        let mut root = div()
            .id(("text-entry", cx.entity_id()))
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .flex()
            .items_center()
            .key_context("GlassTextEntry")
            .capture_key_down(cx.listener(Self::completion_key))
            .track_focus(&self.focus_handle)
            .cursor(gpui::CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, window, cx| {
                if event.button == MouseButton::Left
                    && this.reference_resolver.is_some()
                    && this.focus_handle.is_focused(window)
                {
                    window.blur();
                    cx.notify();
                }
            }))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .line_height(px(self.font_size * 1.5))
            .text_size(px(self.font_size))
            .text_color(rgb(theme.color(0x292d32)));
        if self.fit_content {
            let style = window.text_style();
            let line = window.text_system().shape_line(
                self.content.clone(),
                px(self.font_size),
                &[TextRun {
                    len: self.content.len(),
                    font: style.font(),
                    color: style.color,
                    ..Default::default()
                }],
                None,
            );
            root = root.min_w(line.width + px(1.)).flex_shrink_0();
        }
        if preview {
            let mut content = div()
                .id("reference-preview")
                .flex_1()
                .min_w_0()
                .overflow_x_scroll()
                .flex()
                .items_center()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        window.focus(&this.focus_handle, cx);
                        this.move_to(this.content.len(), cx);
                    }),
                );
            let mut offset = 0;
            for (index, reference) in refs.into_iter().enumerate() {
                if offset < reference.range.start {
                    content = content.child(
                        div()
                            .flex_shrink_0()
                            .whitespace_nowrap()
                            .child(self.content[offset..reference.range.start].to_owned()),
                    );
                }
                let warning = reference.value.is_err();
                let expression = self.content[reference.range.clone()].to_owned();
                let tooltip = match &reference.value {
                    Ok(_) => expression.clone(),
                    Err(error) => format!("{expression} · {error}"),
                };
                let label = reference
                    .value
                    .unwrap_or_else(|_| self.content[reference.range.clone()].to_owned());
                offset = reference.range.end;
                let badge = div()
                    .id(("reference", index))
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .px(px(4.))
                    .rounded(px(4.))
                    .bg(rgb(reference_colors(theme, warning).1))
                    .text_color(rgb(reference_colors(theme, warning).0))
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            window.focus(&this.focus_handle, cx);
                            this.selected_range = reference.range.clone();
                            this.selection_reversed = false;
                            this.is_selecting = false;
                            cx.notify();
                        }),
                    );
                content = content.child(
                    super::Tooltip::new(badge, tooltip)
                        .theme(theme)
                        .render(cx)
                        .id(("reference-tooltip", index))
                        .flex_shrink_0(),
                );
            }
            if offset < self.content.len() {
                content = content.child(
                    div()
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .child(self.content[offset..].to_owned()),
                );
            }
            root = root.child(content);
        } else {
            root = root.child(TextEntryElement { input: cx.entity() });
        }
        if self.focus_handle.is_focused(window)
            && !self.read_only
            && let Some(active) = self.completions.active(
                &self.content,
                &self.selected_range,
                self.marked_range.is_some(),
            )
            && let Some(bounds) = self.last_bounds
        {
            let x = self.last_layout.as_ref().map_or(px(0.), |line| {
                line.x_for_index(self.cursor_offset().min(line.len()))
            });
            let owner = cx.entity().downgrade();
            let dismiss_owner = owner.clone();
            root = root.child(completion_menu(
                active,
                self.completions.selected,
                point(
                    (bounds.left() + x - self.scroll_x).clamp(bounds.left(), bounds.right()),
                    bounds.bottom() + px(4.),
                ),
                theme,
                move |index, window, cx| {
                    let _ = owner.update(cx, |this, cx| this.choose_completion(index, window, cx));
                },
                move |_, cx| {
                    let _ = dismiss_owner.update(cx, |this, cx| {
                        this.completions.dismiss();
                        cx.notify();
                    });
                },
            ));
        }
        root
    }
}

impl Focusable for TextEntry {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Unicode word boundaries keep selection inside UTF-8 characters, including
/// punctuation and whitespace when those are double-clicked.
pub(super) fn word_range_at(text: &str, offset: usize) -> Range<usize> {
    let offset = offset.min(text.len());
    text.split_word_bound_indices()
        .find_map(|(start, segment)| {
            let end = start + segment.len();
            (offset < end || (offset == text.len() && end == offset)).then_some(start..end)
        })
        .unwrap_or(0..0)
}

/// Keep paint runs inside the UTF-8 string even when an IME reports stale offsets.
fn text_run_boundaries(text: &str, offsets: impl IntoIterator<Item = usize>) -> Vec<usize> {
    let mut boundaries = vec![0, text.len()];
    for offset in offsets {
        let mut offset = offset.min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        boundaries.push(offset);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use gpui::AppContext;
    #[gpui::test]
    fn table_search_highlights_editable_cells_and_clears_on_reset(cx: &mut gpui::TestAppContext) {
        use super::super::{DataTable, TableCell, TableColumn, TableRow};
        struct Table(Entity<DataTable>);
        impl Render for Table {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().child(self.0.clone())
            }
        }
        let entry = cx.new(|cx| TextEntry::new(cx, "", "中文 Host.TEST"));
        let input = entry.clone();
        let table = cx.new(|cx| {
            DataTable::new(
                cx,
                [TableColumn::new("域名", 220.)],
                [TableRow::new(
                    "row",
                    [
                        TableCell::custom(move || div().h(px(32.)).child(input.clone()))
                            .search_input(entry.clone(), cx),
                    ],
                )],
            )
            .document()
            .max_height(200.)
        });
        let (_, window) = cx.add_window_view(|_, _| Table(table.clone()));
        for query in ["HOST", "中文", ""] {
            table.update(window, |table, cx| table.set_search_query(query, cx));
            window.debug_bounds("data-table").unwrap();
            entry.read_with(window, |entry, _| {
                assert_eq!(entry.search_query.as_ref(), query);
                assert_eq!(entry.value(), "中文 Host.TEST");
            });
        }
    }
    #[gpui::test]
    fn long_text_scrolls_with_caret_and_preserves_unicode_copy(cx: &mut gpui::TestAppContext) {
        cx.update(bind_text_entry_keys);
        struct Input(Entity<TextEntry>);
        impl Render for Input {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(px(140.)).h(px(32.)).child(self.0.clone())
            }
        }
        let text = "前缀_".to_owned() + &"long_token_中🦀".repeat(60);
        let input = cx.new(|cx| TextEntry::new(cx, "", text.clone()));
        let (_, window) = cx.add_window_view(|_, _| Input(input.clone()));
        window.update(|window, cx| {
            let focus = input.read(cx).focus_handle.clone();
            window.focus(&focus, cx);
        });
        window.update(|window, _| window.refresh());
        input.update(window, |input, _| {
            let bounds = input.last_bounds.unwrap();
            let caret = input
                .last_layout
                .as_ref()
                .unwrap()
                .x_for_index(input.cursor_offset())
                - input.scroll_x;
            assert!(input.scroll_x > px(0.));
            assert!(caret >= px(0.) && caret <= bounds.size.width);
            let offset = input.index_for_mouse_position(bounds.center());
            assert!(offset > 0 && text.is_char_boundary(offset));
        });
        window.simulate_keystrokes("home");
        input.update(window, |input, _| {
            assert_eq!(input.selected_range, 0..0);
            assert_eq!(input.scroll_x, px(0.));
        });
        window.simulate_keystrokes("end");
        input.update(window, |input, _| assert!(input.scroll_x > px(0.)));
        window.simulate_keystrokes("cmd-a cmd-c");
        assert_eq!(window.read_from_clipboard().unwrap().text().unwrap(), text);
        window.update(|window, _| window.blur());
        input.update(window, |input, _| assert_eq!(input.scroll_x, px(0.)));
    }

    #[gpui::test]
    fn template_completion_keyboard_mouse_and_ime(cx: &mut gpui::TestAppContext) {
        struct Input(Entity<TextEntry>);
        impl Render for Input {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().p_4().w(px(400.)).h(px(32.)).child(self.0.clone())
            }
        }
        let input = cx.new(|cx| {
            let mut input = TextEntry::new(cx, "", "中文/{{");
            input.set_completions(
                vec![
                    TextCompletion::new("base_url", "环境变量"),
                    TextCompletion::new("token", "敏感变量"),
                ],
                cx,
            );
            input
        });
        let (_, window) = cx.add_window_view(|_, _| Input(input.clone()));
        window.update(|window, cx| {
            window.focus(&input.read(cx).focus_handle(cx), cx);
        });
        assert!(window.debug_bounds("template-completion-menu").is_some());
        window.simulate_keystrokes("down enter");
        input.update(window, |input, _| {
            assert_eq!(input.value(), "中文/{{token}}")
        });
        assert!(window.debug_bounds("template-completion-menu").is_none());
        window.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.set_value("前缀/{{ba}}/后缀", cx);
                input.move_to("前缀/{{ba".len(), cx);
                window.focus(&input.focus_handle, cx);
            });
        });
        let row = window.debug_bounds("template-completion-0").unwrap();
        window.simulate_click(row.center(), gpui::Modifiers::default());
        input.update(window, |input, _| {
            assert_eq!(input.value(), "前缀/{{base_url}}/后缀")
        });
        window.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.set_value("{{", cx);
                input.replace_and_mark_text_in_range(None, "中", None, window, cx);
            });
        });
        assert!(window.debug_bounds("template-completion-menu").is_none());
        input.update(window, |input, cx| {
            input.set_value("{{ba", cx);
        });
        assert!(window.debug_bounds("template-completion-menu").is_some());
        window.simulate_keystrokes("escape");
        assert!(window.debug_bounds("template-completion-menu").is_none());
        window.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.replace_text_in_range(None, "s", window, cx)
            });
        });
        window.simulate_keystrokes("tab");
        input.update(window, |input, _| assert_eq!(input.value(), "{{base_url}}"));
    }

    #[gpui::test]
    fn read_only_allows_copy_and_blocks_actions_and_ime(cx: &mut gpui::TestAppContext) {
        let (input, window) =
            cx.add_window_view(|_, cx| TextEntry::new(cx, "", "响应 🦀").read_only(true));
        window.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.select_all(&SelectAll, window, cx);
                input.copy(&Copy, window, cx);
                input.cut(&Cut, window, cx);
                input.backspace(&Backspace, window, cx);
                input.delete(&Delete, window, cx);
                input.paste(&Paste, window, cx);
                input.replace_text_in_range(None, "typing", window, cx);
                input.replace_and_mark_text_in_range(None, "输入", None, window, cx);
                assert_eq!(input.value(), "响应 🦀");
                assert_eq!(input.selected_range, 0.."响应 🦀".len());
                assert!(input.marked_range.is_none());
            });
        });
        assert_eq!(
            window.read_from_clipboard().unwrap().text().unwrap(),
            "响应 🦀"
        );
    }
    #[gpui::test]
    fn reference_badges_edit_raw_text_and_keep_inputs_independent(cx: &mut gpui::TestAppContext) {
        struct Inputs([Entity<TextEntry>; 2]);
        impl Render for Inputs {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(px(260.)).flex().flex_col().children(
                    self.0
                        .iter()
                        .map(|input| div().h(px(32.)).child(input.clone())),
                )
            }
        }
        let resolver: TextReferenceResolver = std::rc::Rc::new(|expression| {
            if expression == "{{aa}}" {
                Ok("中文解析值".into())
            } else {
                Err("未定义变量".into())
            }
        });
        let inputs = std::array::from_fn(|_| {
            cx.new(|cx| {
                let mut input = TextEntry::new(cx, "", "{{aa}}/{{missing}}");
                input.set_reference_resolver(Some(resolver.clone()), cx);
                input
            })
        });
        let (_, window) = cx.add_window_view(|_, _| Inputs(inputs.clone()));
        for index in [1, 0, 1] {
            window.simulate_click(
                point(px(10.), px(index as f32 * 32. + 16.)),
                gpui::Modifiers::default(),
            );
            inputs[index].update(window, |input, _| {
                assert_eq!(input.value(), "{{aa}}/{{missing}}");
                assert_eq!(input.selected_range, 0..6);
                assert!(
                    input.last_layout.is_some(),
                    "focused field renders its original expression"
                );
                assert_eq!(
                    input.last_layout.as_ref().unwrap().text.as_ref(),
                    input.value()
                );
            });
            window.update(|window, cx| {
                assert!(inputs[index].focus_handle(cx).is_focused(window));
                assert!(!inputs[1 - index].focus_handle(cx).is_focused(window));
            });
        }
        window.update(|window, cx| {
            inputs[1].update(cx, |input, cx| input.copy(&Copy, window, cx));
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref(),
                Some("{{aa}}")
            );
        });
        window.simulate_input("新值");
        inputs[1].update(window, |input, _| {
            assert_eq!(input.value(), "新值/{{missing}}")
        });
        inputs[0].update(window, |input, _| {
            assert_eq!(input.value(), "{{aa}}/{{missing}}")
        });
        window.simulate_click(point(px(300.), px(100.)), gpui::Modifiers::default());
        window.update(|window, cx| assert!(!inputs[1].focus_handle(cx).is_focused(window)));
        inputs[1].update(window, |input, _| {
            assert_eq!(input.value(), "新值/{{missing}}")
        });
    }

    #[test]
    fn double_click_selects_words_and_separators_at_utf8_offsets() {
        let text = "page_size application/json";
        assert_eq!(word_range_at(text, 4), 0..9);
        assert_eq!(word_range_at(text, 9), 9..10);
        assert_eq!(word_range_at(text, 15), 10..21);
        assert_eq!(word_range_at(text, 21), 21..22);
        assert_eq!(word_range_at(text, text.len()), 22..26);
        assert_eq!(word_range_at(text, usize::MAX), 22..26);
        assert_eq!(word_range_at("", 0), 0..0);
        let text = "中文 👨‍👩‍👧‍👦";
        for offset in 0..=text.len() {
            let range = word_range_at(text, offset);
            assert!(text.is_char_boundary(range.start));
            assert!(text.is_char_boundary(range.end));
        }
        assert_eq!(&text[word_range_at(text, 8)], "👨‍👩‍👧‍👦");
    }
    #[test]
    fn stale_ime_ranges_cannot_shape_beyond_text() {
        let text = "中文a";
        let boundaries = text_run_boundaries(text, [0, 1, 4, 7, 99]);
        assert_eq!(boundaries, vec![0, 3, 7]);
        assert_eq!(
            boundaries
                .windows(2)
                .map(|pair| pair[1] - pair[0])
                .sum::<usize>(),
            text.len()
        );
        assert!(
            boundaries
                .iter()
                .all(|&offset| text.is_char_boundary(offset))
        );
        assert_eq!(text_run_boundaries("", [0, 1, 99]), vec![0]);
    }
}
