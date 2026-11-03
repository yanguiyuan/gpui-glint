use crate::theme::ThemePreset;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use gpui::{
    App, Bounds, ClipboardEntry, ClipboardItem, Context, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, ImageFormat,
    ImageSource, InspectorElementId, InteractiveElement, IntoElement, KeyBinding, LayoutId,
    LineLayout, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    Render, SharedString, StatefulInteractiveElement, Style, Styled, TextAlign, TextRun,
    UTF16Selection, UnderlineStyle, Window, WrappedLine, actions, div, fill, point, px, relative,
    rgb, size,
};

actions!(
    glass_prompt_editor,
    [
        Backspace,
        Delete,
        Left,
        Right,
        Up,
        Down,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
        Newline
    ]
);

/// An image pasted into the editor, holding the clipboard's encoded bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PastedImage(Arc<gpui::Image>);

impl PastedImage {
    pub fn new(format: ImageFormat, bytes: Vec<u8>) -> Self {
        Self(Arc::new(gpui::Image::from_bytes(format, bytes)))
    }

    /// Encoded format of the clipboard bytes, e.g. TIFF for macOS screenshots.
    pub fn format(&self) -> ImageFormat {
        self.0.format
    }

    pub fn media_type(&self) -> &'static str {
        self.0.format.mime_type()
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0.bytes
    }

    /// Render source for `img()` previews, sharing the encoded bytes.
    pub fn source(&self) -> ImageSource {
        ImageSource::Image(self.0.clone())
    }
}

impl From<&gpui::Image> for PastedImage {
    fn from(image: &gpui::Image) -> Self {
        Self(Arc::new(image.clone()))
    }
}

/// Events emitted by [`PromptEditor`] for paste content it does not edit as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptEditorEvent {
    TextAttached(super::ComposerReference),
    /// One or more images were pasted; the editor text is left untouched.
    ImagePasted(Vec<PastedImage>),
    PasteFailed(SharedString),
    ReferenceHovered(SharedString),
}

impl EventEmitter<PromptEditorEvent> for PromptEditor {}

/// Extracts pasteable images from a clipboard item: embedded image entries
/// first, then image files among the external paths.
fn clipboard_images(item: &ClipboardItem) -> Result<Vec<PastedImage>, String> {
    let mut images = Vec::new();
    for entry in item.entries() {
        match entry {
            ClipboardEntry::Image(image) => {
                if image.bytes.len() > 20 * 1024 * 1024 {
                    return Err("图片原始数据超过 20MB".into());
                }
                images.push(PastedImage::from(image));
            }
            ClipboardEntry::ExternalPaths(paths) => {
                for path in paths.paths() {
                    if images.len() >= 16 {
                        return Err("一次最多粘贴 16 张图片".into());
                    }
                    if let Some(image) = image_file(path)? {
                        images.push(image);
                    }
                }
            }
            ClipboardEntry::String(_) => {}
        }
    }
    if images.len() > 16 {
        return Err("一次最多粘贴 16 张图片".into());
    }
    if images
        .iter()
        .map(|image| image.bytes().len())
        .sum::<usize>()
        > 64 * 1024 * 1024
    {
        return Err("图片原始数据总量超过 64MB".into());
    }
    Ok(images)
}

fn image_file(path: &Path) -> Result<Option<PastedImage>, String> {
    use std::io::Read;
    let Some(format) = image_format_for(path) else {
        return Ok(None);
    };
    let file = std::fs::File::open(path).map_err(|_| "无法读取粘贴的图片文件")?;
    if file.metadata().map_err(|_| "无法读取图片信息")?.len() > 20 * 1024 * 1024 {
        return Err("图片原始数据超过 20MB".into());
    }
    let mut bytes = Vec::new();
    file.take(20 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取图片")?;
    if bytes.len() > 20 * 1024 * 1024 {
        return Err("图片原始数据超过 20MB".into());
    }
    Ok(Some(PastedImage::new(format, bytes)))
}

fn image_format_for(path: &Path) -> Option<ImageFormat> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "webp" => ImageFormat::Webp,
        "gif" => ImageFormat::Gif,
        "svg" => ImageFormat::Svg,
        "bmp" => ImageFormat::Bmp,
        "tif" | "tiff" => ImageFormat::Tiff,
        "ico" => ImageFormat::Ico,
        "pbm" | "ppm" | "pgm" => ImageFormat::Pnm,
        _ => return None,
    })
}

/// Registers the editing shortcuts used by [`PromptEditor`]. Scoped to the
/// `GlassPromptEditor` key context so other inputs stay unaffected.
pub fn bind_prompt_editor_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some("GlassPromptEditor")),
        KeyBinding::new("delete", Delete, Some("GlassPromptEditor")),
        KeyBinding::new("left", Left, Some("GlassPromptEditor")),
        KeyBinding::new("right", Right, Some("GlassPromptEditor")),
        KeyBinding::new("up", Up, Some("GlassPromptEditor")),
        KeyBinding::new("down", Down, Some("GlassPromptEditor")),
        KeyBinding::new("shift-left", SelectLeft, Some("GlassPromptEditor")),
        KeyBinding::new("shift-right", SelectRight, Some("GlassPromptEditor")),
        KeyBinding::new("shift-up", SelectUp, Some("GlassPromptEditor")),
        KeyBinding::new("shift-down", SelectDown, Some("GlassPromptEditor")),
        KeyBinding::new("cmd-a", SelectAll, Some("GlassPromptEditor")),
        KeyBinding::new("home", Home, Some("GlassPromptEditor")),
        KeyBinding::new("end", End, Some("GlassPromptEditor")),
        KeyBinding::new("cmd-v", Paste, Some("GlassPromptEditor")),
        KeyBinding::new("cmd-x", Cut, Some("GlassPromptEditor")),
        KeyBinding::new("cmd-c", Copy, Some("GlassPromptEditor")),
        KeyBinding::new("ctrl-a", SelectAll, Some("GlassPromptEditor")),
        KeyBinding::new("ctrl-v", Paste, Some("GlassPromptEditor")),
        KeyBinding::new("ctrl-x", Cut, Some("GlassPromptEditor")),
        KeyBinding::new("ctrl-c", Copy, Some("GlassPromptEditor")),
    ]);
}

/// A multi-line, auto-wrapping native text editor with IME composition,
/// selection, clipboard, and caret support. Used by [`super::Composer`].
pub struct PromptEditor {
    theme: ThemePreset,
    font_size: f32,
    min_rows: usize,
    placeholder: SharedString,
    content: SharedString,
    selected_range: Range<usize>,
    marked_range: Option<Range<usize>>,
    selection_reversed: bool,
    is_selecting: bool,
    focus_handle: FocusHandle,
    last_bounds: Option<Bounds<Pixels>>,
    lines: Vec<EditorLine>,
    /// Text tokens painted in a custom color (e.g. `@mentions`), matched
    /// against whole @mentions and registered $skill tokens in the content.
    highlights: Vec<(SharedString, u32)>,
    reference_styles: Vec<(SharedString, super::Icon, u32)>,
    reference_previews: Vec<super::ComposerReference>,
    reference_hitboxes: Vec<(Bounds<Pixels>, SharedString)>,
    hovered_reference: Option<(SharedString, Bounds<Pixels>)>,
    reference_popover_open: bool,
    reference_popover_hovered: bool,
    reference_hover_task: Option<gpui::Task<()>>,
    reference_close_task: Option<gpui::Task<()>>,
    paste_task: Option<gpui::Task<()>>,
    attach_long_pastes: bool,
}

pub(crate) struct EditorLine {
    /// Byte offset of this line's first character inside the full content.
    pub start: usize,
    pub wrapped: WrappedLine,
}

impl PromptEditor {
    /// Only task composers opt into turning large clipboard text into references.
    pub fn attach_long_pastes(mut self, enabled: bool) -> Self {
        self.attach_long_pastes = enabled;
        self
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

    pub fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        let size = size.max(1.);
        if self.font_size != size {
            self.font_size = size;
            cx.notify();
        }
    }

    /// Paints occurrences of the given tokens (e.g. `"@Workflow"`) in the
    /// given color. Registered `$skill` tokens also use these colors; unlisted
    /// `@word` tokens fall back to the accent color. Unknown `$variables` do not.
    pub fn set_highlights(&mut self, highlights: Vec<(SharedString, u32)>, cx: &mut Context<Self>) {
        if self.highlights != highlights {
            self.highlights = highlights;
            cx.notify();
        }
    }

    pub fn set_reference_previews(
        &mut self,
        references: Vec<super::ComposerReference>,
        cx: &mut Context<Self>,
    ) {
        if self.reference_previews != references {
            self.reference_previews = references;
            cx.notify();
        }
    }
    fn close_reference_preview(&mut self, cx: &mut Context<Self>) {
        self.reference_hover_task = None;
        self.reference_close_task = None;
        self.hovered_reference = None;
        self.reference_popover_open = false;
        self.reference_popover_hovered = false;
        cx.notify();
    }
    fn delay_close_reference_preview(&mut self, cx: &mut Context<Self>) {
        self.reference_hover_task = None;
        if self.reference_close_task.is_some() || self.hovered_reference.is_none() {
            return;
        }
        let timer = cx
            .background_executor()
            .timer(std::time::Duration::from_millis(150));
        self.reference_close_task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(task) = this.reference_close_task.take() {
                    task.detach();
                }
                if !this.reference_popover_hovered {
                    this.close_reference_preview(cx);
                }
            });
        }));
    }
    pub fn set_reference_styles(
        &mut self,
        mut styles: Vec<(SharedString, super::Icon, u32)>,
        cx: &mut Context<Self>,
    ) {
        styles.retain(|(token, _, _)| token.len() > 1 && token.starts_with(['@', '$']));
        if self.reference_styles != styles {
            self.reference_styles = styles;
            cx.notify();
        }
    }
    /// Minimum visible rows before the editor stops shrinking.
    pub fn min_rows(mut self, rows: usize) -> Self {
        self.min_rows = rows.max(1);
        self
    }

    pub fn value(&self) -> &str {
        &self.content
    }

    /// Current UTF-8 selection range, for caret-aware host completions.
    pub fn selection_range(&self) -> Range<usize> {
        self.selected_range.clone()
    }

    /// Replace a valid UTF-8 range and leave the caret immediately after it.
    pub fn replace_range(
        &mut self,
        range: Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if range.start > range.end
            || range.end > self.content.len()
            || !self.content.is_char_boundary(range.start)
            || !self.content.is_char_boundary(range.end)
        {
            return false;
        }
        self.selected_range = range;
        self.replace_selection(text, cx);
        true
    }

    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Whether an IME composition is in progress; hosts should not submit mid-composition.
    pub fn is_composing(&self) -> bool {
        self.marked_range.is_some()
    }

    pub fn set_value(&mut self, value: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.paste_task = None;
        if self.hovered_reference.is_some() {
            self.close_reference_preview(cx);
        }
        let content = value.into();
        let end = content.len();
        self.content = content;
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    pub fn new(cx: &mut Context<Self>, placeholder: impl Into<SharedString>) -> Self {
        Self {
            theme: ThemePreset::default(),
            font_size: 13.,
            min_rows: 1,
            placeholder: placeholder.into(),
            content: "".into(),
            selected_range: 0..0,
            marked_range: None,
            selection_reversed: false,
            is_selecting: false,
            focus_handle: cx.focus_handle(),
            last_bounds: None,
            lines: Vec::new(),
            highlights: Vec::new(),
            reference_styles: Vec::new(),
            reference_previews: Vec::new(),
            reference_hitboxes: Vec::new(),
            hovered_reference: None,
            reference_popover_open: false,
            reference_popover_hovered: false,
            reference_hover_task: None,
            reference_close_task: None,
            paste_task: None,
            attach_long_pastes: false,
        }
    }

    pub(crate) fn line_height(&self) -> Pixels {
        px(self.font_size * 1.5)
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
        let mut utf16 = 0;
        for ch in self.content.chars() {
            if utf16 >= offset {
                break;
            }
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
        if self.hovered_reference.is_some() {
            self.close_reference_preview(cx);
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

    /// Shape every logical line with the current wrap width. Geometry only;
    /// paint adds selection and composition runs on top.
    fn shaped_lines(&self, window: &Window) -> Vec<EditorLine> {
        let width = self
            .last_bounds
            .map(|bounds| bounds.size.width)
            .unwrap_or(px(400.))
            .max(px(1.));
        let font = window.text_style().font();
        let font_size = px(self.font_size);
        let mut lines = Vec::new();
        let mut start = 0;
        for piece in self.content.split('\n') {
            let run = TextRun {
                letter_spacing: None,
                len: piece.len(),
                font: font.clone(),
                color: gpui::black().into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            if let Ok(wrapped) = window.text_system().shape_text(
                piece.to_string(),
                font_size,
                &[run],
                Some(width),
                None,
            ) {
                for wrapped in wrapped {
                    lines.push(EditorLine { start, wrapped });
                }
            }
            start += piece.len() + 1;
        }
        lines
    }

    fn total_rows(lines: &[EditorLine]) -> usize {
        lines
            .iter()
            .map(|line| line.wrapped.wrap_boundaries.len() + 1)
            .sum()
    }

    /// Byte index of the first character on visual row `row` of a line.
    fn row_start_index(line: &EditorLine, row: usize) -> usize {
        if row == 0 {
            0
        } else {
            let boundary = &line.wrapped.wrap_boundaries[row - 1];
            let run = &line.wrapped.unwrapped_layout.runs[boundary.run_ix];
            run.glyphs[boundary.glyph_ix].index
        }
    }

    /// Byte index just past the last character on visual row `row`.
    fn row_end_index(line: &EditorLine, row: usize) -> usize {
        if row < line.wrapped.wrap_boundaries.len() {
            Self::row_start_index(line, row + 1)
        } else {
            line.wrapped.len()
        }
    }

    /// Locate `offset` as (line, line byte offset, visual rows above the line).
    /// Returns `None` when no lines have been shaped yet (e.g. before the first
    /// paint, which the IME can reach while asking for the caret rect).
    fn locate<'a>(
        lines: &'a [EditorLine],
        offset: usize,
    ) -> Option<(&'a EditorLine, usize, usize)> {
        for (index, line) in lines.iter().enumerate() {
            let end = line.start + line.wrapped.len();
            if offset <= end || index == lines.len() - 1 {
                let local = offset.saturating_sub(line.start).min(line.wrapped.len());
                let rows_above = lines[..index]
                    .iter()
                    .map(|line| line.wrapped.wrap_boundaries.len() + 1)
                    .sum();
                return Some((line, local, rows_above));
            }
        }
        None
    }

    fn caret_point(&self, lines: &[EditorLine], line_height: Pixels) -> gpui::Point<Pixels> {
        let Some((line, local, rows_above)) = Self::locate(lines, self.cursor_offset()) else {
            return point(px(0.), px(0.));
        };
        line.wrapped
            .position_for_index(local, line_height)
            .map(|position| point(position.x, position.y + (rows_above as f32) * line_height))
            .unwrap_or(point(px(0.), (rows_above as f32) * line_height))
    }

    fn index_for_mouse_position(&self, position: gpui::Point<Pixels>) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        let Some(bounds) = self.last_bounds else {
            return 0;
        };
        let line_height = self.line_height();
        let x = position.x - bounds.left();
        let mut y = (position.y - bounds.top()).max(px(0.));
        let rows_total = Self::total_rows(&self.lines) as f32 * line_height;
        y = y.min(rows_total - px(0.001));
        let mut rows_above = 0usize;
        for (index, line) in self.lines.iter().enumerate() {
            let rows = line.wrapped.wrap_boundaries.len() + 1;
            let top = (rows_above as f32) * line_height;
            let is_last = index == self.lines.len() - 1;
            if y < top + (rows as f32) * line_height || is_last {
                let local_y = y - top;
                let local = line
                    .wrapped
                    .closest_index_for_position(point(x, local_y), line_height)
                    .unwrap_or_else(|closest| closest);
                return (line.start + local).min(self.content.len());
            }
            rows_above += rows;
        }
        self.content.len()
    }
}

impl PromptEditor {
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

    fn up(&mut self, _: &Up, window: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(true, window, cx);
    }

    fn down(&mut self, _: &Down, window: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(false, window, cx);
    }

    fn move_vertical(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        let lines = self.shaped_lines(window);
        if lines.is_empty() {
            return;
        }
        let line_height = self.line_height();
        let caret = self.caret_point(&lines, line_height);
        let target_y = if up {
            caret.y - line_height
        } else {
            caret.y + line_height
        };
        let rows_total = Self::total_rows(&lines) as f32 * line_height;
        if target_y < px(0.) {
            self.move_to(0, cx);
            return;
        }
        if target_y >= rows_total {
            self.move_to(self.content.len(), cx);
            return;
        }
        let mut rows_above = 0usize;
        for (index, line) in lines.iter().enumerate() {
            let rows = line.wrapped.wrap_boundaries.len() + 1;
            let top = (rows_above as f32) * line_height;
            let is_last = index == lines.len() - 1;
            if target_y < top + (rows as f32) * line_height || is_last {
                let local_y = target_y - top;
                let local = line
                    .wrapped
                    .closest_index_for_position(point(caret.x, local_y), line_height)
                    .unwrap_or_else(|closest| closest);
                self.move_to((line.start + local).min(self.content.len()), cx);
                return;
            }
            rows_above += rows;
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_up(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        self.select_vertical(true, window, cx);
    }

    fn select_down(&mut self, _: &SelectDown, window: &mut Window, cx: &mut Context<Self>) {
        self.select_vertical(false, window, cx);
    }

    fn select_vertical(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        let anchor = self.cursor_offset();
        self.move_vertical(up, window, cx);
        if self.selected_range.is_empty() {
            self.selected_range = anchor..self.cursor_offset();
            self.selection_reversed = self.cursor_offset() < anchor;
            if self.selection_reversed {
                self.selected_range = self.cursor_offset()..anchor;
            }
            cx.notify();
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    fn home(&mut self, _: &Home, window: &mut Window, cx: &mut Context<Self>) {
        let lines = self.shaped_lines(window);
        let start = Self::locate(&lines, self.cursor_offset())
            .map(|(line, _, _)| line.start)
            .unwrap_or(0);
        self.move_to(start, cx);
    }

    fn end(&mut self, _: &End, window: &mut Window, cx: &mut Context<Self>) {
        let lines = self.shaped_lines(window);
        let end = Self::locate(&lines, self.cursor_offset())
            .map(|(line, _, _)| line.start + line.wrapped.len())
            .unwrap_or(self.content.len());
        self.move_to(end, cx);
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_selection("", cx);
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_selection("", cx);
    }

    fn newline(&mut self, _: &Newline, _: &mut Window, cx: &mut Context<Self>) {
        self.replace_selection("\n", cx);
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let has_images = item.entries().iter().any(|entry| {
            matches!(
                entry,
                ClipboardEntry::Image(_) | ClipboardEntry::ExternalPaths(_)
            )
        });
        if !has_images {
            if let Some(text) = item.text() {
                self.paste_text(&text, cx);
            }
            return;
        }
        let fallback_text = item.text();
        let read = cx
            .background_executor()
            .spawn(async move { clipboard_images(&item) });
        self.paste_task = Some(cx.spawn(async move |weak, cx| {
            let result = read.await;
            let _ = weak.update(cx, |this, cx| match result {
                Ok(images) if !images.is_empty() => cx.emit(PromptEditorEvent::ImagePasted(images)),
                Err(error) => cx.emit(PromptEditorEvent::PasteFailed(error.into())),
                _ => {
                    if let Some(text) = fallback_text {
                        this.paste_text(&text, cx);
                    }
                }
            });
        }));
    }

    fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if !self.attach_long_pastes || (text.chars().count() <= 2000 && text.lines().count() <= 20)
        {
            self.replace_selection(&text, cx);
            return;
        }
        let mut number = 1;
        let reference = loop {
            let mut reference =
                super::ComposerReference::pasted_text(format!("粘贴文本-{number}.txt"), &text);
            reference.token = reference.label();
            if !self.content.contains(&reference.token)
                && !self
                    .reference_previews
                    .iter()
                    .any(|r| r.token == reference.token)
            {
                break reference;
            }
            number += 1;
        };
        let prefix = if self.content[..self.selected_range.start]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace())
        {
            " "
        } else {
            ""
        };
        self.replace_selection(&format!("{prefix}{} ", reference.token), cx);
        self.reference_styles.push((
            reference.token.clone().into(),
            reference.icon(),
            self.theme.palette().accent,
        ));
        self.reference_previews.push(reference.clone());
        cx.emit(PromptEditorEvent::TextAttached(reference));
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
        self.close_reference_preview(cx);
        self.is_selecting = true;
        window.focus(&self.focus_handle, cx);
        let offset = self.index_for_mouse_position(event.position);
        if event.modifiers.shift {
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
            return;
        }
        if self.reference_popover_hovered {
            return;
        }
        let hit = self
            .reference_hitboxes
            .iter()
            .find(|(bounds, token)| {
                bounds.contains(&event.position)
                    && self
                        .reference_previews
                        .iter()
                        .any(|r| r.token == token.as_ref())
            })
            .cloned();
        let Some((bounds, token)) = hit else {
            self.delay_close_reference_preview(cx);
            return;
        };
        self.reference_close_task = None;
        if self
            .hovered_reference
            .as_ref()
            .is_some_and(|(old, _)| old == &token)
        {
            return;
        }
        self.hovered_reference = Some((token.clone(), bounds));
        self.reference_popover_open = false;
        let timer = cx
            .background_executor()
            .timer(std::time::Duration::from_millis(300));
        self.reference_hover_task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .hovered_reference
                    .as_ref()
                    .is_some_and(|(old, _)| old == &token)
                {
                    this.reference_popover_open = true;
                    cx.emit(PromptEditorEvent::ReferenceHovered(token.clone()));
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }
}

impl EntityInputHandler for PromptEditor {
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
        _range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // The IME may ask for the caret rect before the first paint, so shape
        // fresh lines instead of trusting the cache.
        let line_height = self.line_height();
        let lines = self.shaped_lines(window);
        let caret = self.caret_point(&lines, line_height);
        Some(Bounds::from_corners(
            point(bounds.left() + caret.x, bounds.top() + caret.y),
            point(
                bounds.left() + caret.x + px(2.),
                bounds.top() + caret.y + line_height,
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

struct PromptEditorElement {
    input: Entity<PromptEditor>,
}

struct PromptEditorPrepaint {
    lines: Vec<EditorLine>,
    placeholder: Option<gpui::ShapedLine>,
    cursor: Option<gpui::PaintQuad>,
    selections: Vec<gpui::PaintQuad>,
    references: Vec<gpui::PaintQuad>,
    icons: Vec<(Bounds<Pixels>, super::Icon, u32)>,
    hitboxes: Vec<(Bounds<Pixels>, SharedString)>,
}

impl IntoElement for PromptEditorElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for PromptEditorElement {
    type RequestLayoutState = ();
    type PrepaintState = PromptEditorPrepaint;

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
        let input = self.input.read(cx);
        let cached_rows = PromptEditor::total_rows(&input.lines);
        let rows = cached_rows.max(input.min_rows);
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (rows as f32 * input.line_height()).into();
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
        let line_height = input.line_height();
        let font_size = px(input.font_size);
        let focused = input.focus_handle.is_focused(window);
        let style = window.text_style();
        let base_color = style.color;
        let placeholder_color = rgb(theme.color(0x92979a));
        let selection_color = if focused {
            rgb(theme.color(0x007aff))
        } else {
            rgb(theme.color(0xe1e5e9))
        };

        let mut lines = Vec::new();
        let mut selections = Vec::new();
        let mut reference_backgrounds = Vec::new();
        let mut reference_hitboxes = Vec::new();
        let mut reference_icons = Vec::new();
        let references: Vec<_> = input
            .reference_styles
            .iter()
            .flat_map(|(token, icon, color)| {
                input
                    .content
                    .match_indices(token.as_ref())
                    .filter(|(start, _)| {
                        (*start == 0
                            || input.content[..*start]
                                .chars()
                                .next_back()
                                .is_some_and(char::is_whitespace))
                            && input.content[*start + token.len()..]
                                .chars()
                                .next()
                                .is_none_or(|c| c.is_whitespace() || ",，。;；)]}".contains(c))
                    })
                    .map(move |(start, _)| (start..start + token.len(), *icon, *color))
            })
            .collect();
        let mut placeholder = None;
        let cursor = if input.content.is_empty() {
            let run = TextRun {
                letter_spacing: None,
                len: input.placeholder.len(),
                font: style.font(),
                color: gpui::rgb_to_hsla(placeholder_color),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line =
                window
                    .text_system()
                    .shape_line(input.placeholder.clone(), font_size, &[run], None);
            placeholder = Some(line);
            focused.then(|| {
                fill(
                    Bounds::new(
                        point(bounds.left(), bounds.top()),
                        size(px(1.5), line_height),
                    ),
                    rgb(theme.color(0x087bfa)),
                )
            })
        } else {
            let selected = input.selected_range.clone();
            let marked = input.marked_range.clone();
            let mut mentions =
                mention_ranges(&input.content, &input.highlights, theme.color(0x007aff));
            for (range, _, color) in &references {
                mentions.retain(|(old, _)| old.end <= range.start || old.start >= range.end);
                mentions.push((range.clone(), *color));
            }
            let mut rows_above = 0usize;
            let wrap_width = bounds.size.width.max(px(1.));
            let mut line_start = 0usize;
            for piece in input.content.split('\n') {
                let start = line_start;
                line_start += piece.len() + 1;
                let piece_len = piece.len();
                let mut boundaries = vec![0, piece_len];
                for (range, _, _) in &references {
                    if range.start >= start && range.start < start + piece_len {
                        boundaries.push(range.start - start);
                        boundaries.push(range.start - start + 1);
                    }
                }
                boundaries.extend(mentions.iter().flat_map(|(range, _)| {
                    [
                        range.start.saturating_sub(start).min(piece_len),
                        range.end.saturating_sub(start).min(piece_len),
                    ]
                }));
                if focused {
                    boundaries.extend([
                        selected.start.saturating_sub(start).min(piece_len),
                        selected.end.saturating_sub(start).min(piece_len),
                    ]);
                    if let Some(marked_range) = &marked {
                        boundaries.extend([
                            marked_range.start.saturating_sub(start).min(piece_len),
                            marked_range.end.saturating_sub(start).min(piece_len),
                        ]);
                    }
                }
                boundaries.sort_unstable();
                boundaries.dedup();
                let runs: Vec<TextRun> = boundaries
                    .windows(2)
                    .map(|window_range| {
                        let run_start = start + window_range[0];
                        let is_selected =
                            focused && run_start >= selected.start && run_start < selected.end;
                        let mention_color = mentions
                            .iter()
                            .find(|(range, _)| run_start >= range.start && run_start < range.end)
                            .map(|(_, color)| *color);
                        let color = if references
                            .iter()
                            .any(|(range, _, _)| run_start == range.start)
                        {
                            gpui::rgb_to_hsla(gpui::rgba(0x00000000))
                        } else if is_selected {
                            gpui::rgb_to_hsla(rgb(0xffffff))
                        } else if let Some(mention) = mention_color {
                            gpui::rgb_to_hsla(rgb(mention))
                        } else {
                            base_color
                        };
                        let composing = marked.as_ref().is_some_and(|marked_range| {
                            run_start >= marked_range.start && run_start < marked_range.end
                        });
                        TextRun {
                            letter_spacing: None,
                            len: window_range[1] - window_range[0],
                            font: style.font(),
                            color,
                            underline: composing.then_some(UnderlineStyle {
                                color: Some(color),
                                thickness: px(1.),
                                wavy: false,
                            }),
                            background_color: None,
                            strikethrough: None,
                        }
                    })
                    .collect();
                let runs = if runs.is_empty() {
                    vec![TextRun {
                        letter_spacing: None,
                        len: 0,
                        font: style.font(),
                        color: base_color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }]
                } else {
                    runs
                };
                if let Ok(wrapped_lines) = window.text_system().shape_text(
                    piece.to_string(),
                    font_size,
                    &runs,
                    Some(wrap_width),
                    None,
                ) {
                    for wrapped in wrapped_lines {
                        lines.push(EditorLine { start, wrapped });
                    }
                }
            }

            let mut row_offset = 0usize;
            for line in &lines {
                for (range, icon, color) in &references {
                    let from = range.start.max(line.start);
                    let to = range.end.min(line.start + line.wrapped.len());
                    if from >= to {
                        continue;
                    }
                    let first = line
                        .wrapped
                        .position_for_index(from - line.start, line_height)
                        .unwrap_or(point(px(0.), px(0.)));
                    let last = line
                        .wrapped
                        .position_for_index(to - line.start, line_height)
                        .unwrap_or(first);
                    let first_row = (first.y / line_height) as usize;
                    let last_row = (last.y / line_height) as usize;
                    for row in first_row..=last_row {
                        let left = if row == first_row { first.x } else { px(0.) };
                        let right = if row == last_row {
                            last.x
                        } else {
                            line.wrapped
                                .unwrapped_layout
                                .x_for_index(PromptEditor::row_end_index(line, row))
                                - line
                                    .wrapped
                                    .unwrapped_layout
                                    .x_for_index(PromptEditor::row_start_index(line, row))
                        };
                        if right > left {
                            reference_hitboxes.push((
                                Bounds::new(
                                    point(
                                        bounds.left() + left - px(1.),
                                        bounds.top()
                                            + (row_offset + row) as f32 * line_height
                                            + px(1.),
                                    ),
                                    size(right - left + px(2.), line_height - px(2.)),
                                ),
                                SharedString::from(input.content[range.clone()].to_owned()),
                            ));
                            reference_backgrounds.push(
                                fill(
                                    Bounds::new(
                                        point(
                                            bounds.left() + left - px(1.),
                                            bounds.top()
                                                + (row_offset + row) as f32 * line_height
                                                + px(1.),
                                        ),
                                        size(right - left + px(2.), line_height - px(2.)),
                                    ),
                                    gpui::rgba((*color << 8) | 0x18),
                                )
                                .corner_radii(px(4.)),
                            );
                        }
                    }
                    if from == range.start {
                        let icon_size = px(11.);
                        reference_icons.push((
                            Bounds::new(
                                point(
                                    bounds.left() + first.x,
                                    bounds.top()
                                        + row_offset as f32 * line_height
                                        + first.y
                                        + (line_height - icon_size) / 2.,
                                ),
                                size(icon_size, icon_size),
                            ),
                            *icon,
                            *color,
                        ));
                    }
                }
                row_offset += line.wrapped.wrap_boundaries.len() + 1;
            }

            if focused && !input.selected_range.is_empty() {
                for line in lines.iter() {
                    let rows = line.wrapped.wrap_boundaries.len() + 1;
                    let start_byte = line.start;
                    let end_byte = line.start + line.wrapped.len();
                    let selection_start = selected.start.clamp(start_byte, end_byte);
                    let selection_end = selected.end.clamp(start_byte, end_byte);
                    if selection_start < selection_end {
                        let local_start = selection_start - start_byte;
                        let local_end = selection_end - start_byte;
                        let layout: &LineLayout = &line.wrapped.unwrapped_layout;
                        let first = line
                            .wrapped
                            .position_for_index(local_start, line_height)
                            .unwrap_or(point(px(0.), px(0.)));
                        let last = line
                            .wrapped
                            .position_for_index(local_end, line_height)
                            .unwrap_or(point(px(0.), px(0.)));
                        let first_row = (first.y / line_height) as usize;
                        let last_row = (last.y / line_height) as usize;
                        for row in first_row..=last_row {
                            let row_left = if row == first_row { first.x } else { px(0.) };
                            let row_right = if row == last_row {
                                last.x
                            } else {
                                let row_start_byte = PromptEditor::row_start_index(line, row);
                                let row_end_byte = PromptEditor::row_end_index(line, row);
                                layout.x_for_index(row_end_byte)
                                    - layout.x_for_index(row_start_byte)
                            };
                            if row_right > row_left {
                                let y = bounds.top() + ((rows_above + row) as f32) * line_height;
                                selections.push(fill(
                                    Bounds::new(
                                        point(bounds.left() + row_left, y),
                                        size(row_right - row_left, line_height),
                                    ),
                                    selection_color,
                                ));
                            }
                        }
                    }
                    rows_above += rows;
                }
            }

            if focused && input.selected_range.is_empty() {
                let caret = input.caret_point(&lines, line_height);
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + caret.x, bounds.top() + caret.y),
                        size(px(1.5), line_height),
                    ),
                    rgb(theme.color(0x087bfa)),
                ))
            } else {
                None
            }
        };

        PromptEditorPrepaint {
            lines,
            placeholder,
            cursor,
            selections,
            references: reference_backgrounds,
            icons: reference_icons,
            hitboxes: reference_hitboxes,
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
        for background in prepaint.references.drain(..) {
            window.paint_quad(background);
        }
        for selection in prepaint.selections.drain(..) {
            window.paint_quad(selection);
        }
        let line_height = self.input.read(cx).line_height();
        if let Some(placeholder) = prepaint.placeholder.take() {
            placeholder
                .paint(
                    bounds.origin,
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .ok();
        }
        let mut rows_above = 0usize;
        for line in &prepaint.lines {
            let origin = point(
                bounds.left(),
                bounds.top() + (rows_above as f32) * line_height,
            );
            line.wrapped
                .paint(origin, line_height, TextAlign::Left, None, window, cx)
                .ok();
            rows_above += line.wrapped.wrap_boundaries.len() + 1;
        }
        for (bounds, icon, color) in prepaint.icons.drain(..) {
            let _ = window.paint_svg(
                bounds,
                format!("glint-ui/icons/{}.svg", icon.name()).into(),
                None,
                gpui::TransformationMatrix::unit(),
                gpui::rgb_to_hsla(rgb(color)),
                cx,
            );
        }
        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }
        let lines = std::mem::take(&mut prepaint.lines);
        let hitboxes = std::mem::take(&mut prepaint.hitboxes);
        self.input.update(cx, |input, _cx| {
            input.last_bounds = Some(bounds);
            input.lines = lines;
            input.reference_hitboxes = hitboxes;
        });
    }
}

impl Render for PromptEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui::prelude::FluentBuilder;
        let popup = self
            .hovered_reference
            .as_ref()
            .filter(|_| self.reference_popover_open)
            .and_then(|(token, bounds)| {
                self.reference_previews
                    .iter()
                    .find(|r| r.token == token.as_ref())
                    .map(|r| (r.clone(), *bounds))
            });
        let width = (f32::from(window.viewport_size().width) - 24.).clamp(120., 420.);
        let theme = self.theme;
        div()
            .id("prompt-editor")
            .flex_1()
            .min_w_0()
            .flex()
            .key_context("GlassPromptEditor")
            .track_focus(&self.focus_handle)
            .cursor(gpui::CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_hover(cx.listener(|this, hovered, _, cx| {
                if !hovered {
                    this.delay_close_reference_preview(cx);
                }
            }))
            .line_height(px(self.font_size * 1.5))
            .text_size(px(self.font_size))
            .text_color(rgb(theme.color(0x292d32)))
            .child(PromptEditorElement { input: cx.entity() })
            .when_some(popup, |root, (reference, bounds)| {
                root.child(
                    gpui::deferred(
                        gpui::anchored()
                            .position(point(bounds.left(), bounds.top() - px(8.)))
                            .anchor(gpui::Anchor::BottomLeft)
                            .snap_to_window_with_margin(px(12.))
                            .child(
                                div()
                                    .id("reference-hover-popover")
                                    .occlude()
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                                    .on_hover(cx.listener(|this, hovered, _, cx| {
                                        this.reference_popover_hovered = *hovered;
                                        if *hovered {
                                            this.reference_close_task = None;
                                        } else {
                                            this.delay_close_reference_preview(cx);
                                        }
                                    }))
                                    .child(super::reference_preview::reference_preview(
                                        &reference, self.theme, width,
                                    )),
                            ),
                    )
                    .with_priority(90),
                )
            })
    }
}

impl Focusable for PromptEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Paint @mentions and explicitly registered $skill tokens at word boundaries.
/// Unknown $variables remain ordinary text; files retain the default accent.
fn mention_ranges(
    content: &str,
    highlights: &[(SharedString, u32)],
    default: u32,
) -> Vec<(Range<usize>, u32)> {
    let color_for = |token: &str| -> Option<u32> {
        highlights
            .iter()
            .find(|(text, _)| text.as_ref() == token)
            .map(|(_, color)| *color)
            .or_else(|| token.starts_with('@').then_some(default))
    };
    let mut ranges = Vec::new();
    let mut current: Option<Range<usize>> = None;
    let mut previous_whitespace = true;
    for (index, ch) in content.char_indices() {
        let next = index + ch.len_utf8();
        if let Some(range) = current.as_mut() {
            if ch.is_whitespace() {
                let finished = range.clone();
                if finished.len() > 1
                    && let Some(color) = color_for(&content[finished.clone()])
                {
                    ranges.push((finished, color));
                }
                current = None;
            } else {
                range.end = next;
            }
        } else if matches!(ch, '@' | '$') && previous_whitespace {
            current = Some(index..next);
        }
        previous_whitespace = ch.is_whitespace();
    }
    if let Some(range) = current
        && range.len() > 1
    {
        let token = &content[range.clone()];
        if let Some(color) = color_for(token) {
            ranges.push((range, color));
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext;
    #[gpui::test]
    fn long_clipboard_text_is_opt_in_and_replaces_only_the_selection(
        cx: &mut gpui::TestAppContext,
    ) {
        let (editor, window) =
            cx.add_window_view(|_, cx| PromptEditor::new(cx, "Message").attach_long_pastes(true));
        let full = "中文`json`\n".repeat(30);
        window.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_value("before selected after", cx);
                editor.selected_range = 7..15;
                cx.write_to_clipboard(ClipboardItem::new_string(full.clone()));
                editor.paste(&Paste, window, cx);
                assert_eq!(editor.value(), "before @粘贴文本-1.txt  after");
                assert_eq!(
                    editor.reference_previews[0].excerpt.as_deref(),
                    Some(full.as_str())
                );
                assert!(editor.reference_previews[0].preview_truncated);
                editor.paste(&Paste, window, cx);
                assert!(editor.value().contains("@粘贴文本-2.txt"));
                assert_eq!(editor.reference_previews.len(), 2);
            });
        });
        let plain = cx.new(|cx| PromptEditor::new(cx, "plain"));
        plain.update(cx, |editor, cx| {
            editor.paste_text(&full, cx);
            assert_eq!(editor.value(), full);
            assert!(editor.reference_previews.is_empty());
        });
    }
    #[gpui::test]
    fn paste_attachment_threshold_counts_characters_and_lines(cx: &mut gpui::TestAppContext) {
        for (text, attached) in [
            ("中".repeat(2000), false),
            ("中".repeat(2001), true),
            ("行\n".repeat(20), false),
            ("行\n".repeat(21), true),
        ] {
            let editor = cx.new(|cx| PromptEditor::new(cx, "Message").attach_long_pastes(true));
            editor.update(cx, |editor, cx| {
                editor.paste_text(&text, cx);
                assert_eq!(!editor.reference_previews.is_empty(), attached);
                if !attached {
                    assert_eq!(editor.value(), text);
                }
            });
        }
    }

    #[gpui::test]
    fn reference_hover_is_bounded_fades_long_content_and_preserves_text(
        cx: &mut gpui::TestAppContext,
    ) {
        let (editor, window) = cx.add_window_view(|_, cx| PromptEditor::new(cx, "Message"));
        let events = std::rc::Rc::new(std::cell::Cell::new(0));
        let hits = events.clone();
        window.update(|_, cx| {
            cx.subscribe(&editor, move |_, event: &PromptEditorEvent, _| {
                if matches!(event, PromptEditorEvent::ReferenceHovered(_)) {
                    hits.set(hits.get() + 1);
                }
            })
            .detach()
        });
        window.simulate_resize(gpui::size(px(520.), px(400.)));
        editor.update(window, |editor, cx| {
            let mut reference = super::super::ComposerReference::selection(
                "/project/源.rs",
                2,
                80,
                &"第一行文档内容\n".repeat(79),
            );
            reference.token = "@源.rs:2-80".into();
            editor.set_value("解释 @源.rs:2-80 ", cx);
            editor.set_reference_styles(
                vec![(reference.token.clone().into(), reference.icon(), 0x007aff)],
                cx,
            );
            editor.set_reference_previews(vec![reference], cx);
        });
        window.run_until_parked();
        let position =
            editor.read_with(window, |editor, _| editor.reference_hitboxes[0].0.center());
        window.simulate_event(gpui::MouseMoveEvent {
            position,
            ..Default::default()
        });
        window.run_until_parked();
        assert_eq!(events.get(), 0, "hover reads must wait for the dwell delay");
        window
            .executor()
            .advance_clock(std::time::Duration::from_millis(350));
        window.run_until_parked();
        assert_eq!(events.get(), 1);
        let bounds = window
            .debug_bounds("prompt-reference-preview")
            .expect("hover preview");
        assert!(bounds.size.height <= px(300.));
        assert!(window.debug_bounds("reference-preview-fade").is_some());
        editor.update(window, |editor, cx| {
            assert_eq!(editor.value(), "解释 @源.rs:2-80 ");
            editor.set_value("继续提问", cx);
        });
        window.run_until_parked();
        assert!(window.debug_bounds("prompt-reference-preview").is_none());
    }

    #[gpui::test]
    fn restoring_text_cancels_an_inflight_image_paste(cx: &mut gpui::TestAppContext) {
        use std::{cell::Cell, rc::Rc};
        let (editor, window) = cx.add_window_view(|_, cx| PromptEditor::new(cx, "Message"));
        let events = Rc::new(Cell::new(0));
        let received = events.clone();
        let _subscription = window.update(|_, cx| {
            cx.subscribe(&editor, move |_, event: &PromptEditorEvent, _| {
                if matches!(event, PromptEditorEvent::ImagePasted(_)) {
                    received.set(received.get() + 1);
                }
            })
        });
        window.update(|window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_image(&gpui::Image::from_bytes(
                ImageFormat::Png,
                vec![1, 2, 3],
            )));
            editor.update(cx, |editor, cx| {
                editor.paste(&Paste, window, cx);
                editor.set_value("another conversation", cx);
            });
        });
        cx.run_until_parked();
        assert_eq!(events.get(), 0);
        editor.read_with(cx, |editor, _| {
            assert_eq!(editor.value(), "another conversation")
        });
    }

    #[gpui::test]
    fn clipboard_shortcuts_support_cmd_and_ctrl(cx: &mut gpui::TestAppContext) {
        cx.update(bind_prompt_editor_keys);
        for modifier in ["cmd", "ctrl"] {
            let (editor, window) = cx.add_window_view(|_, cx| PromptEditor::new(cx, "Message"));
            window.update(|window, cx| {
                editor.update(cx, |editor, cx| editor.set_value("original", cx));
                cx.write_to_clipboard(ClipboardItem::new_string("中文\r\nnext".into()));
                window.focus(&editor.read(cx).focus_handle(cx), cx);
            });
            window.simulate_keystrokes(&format!("{modifier}-a {modifier}-v"));
            editor.read_with(window, |editor, _| assert_eq!(editor.value(), "中文\nnext"));
            window.simulate_keystrokes(&format!("{modifier}-a {modifier}-c"));
            assert_eq!(
                window.read_from_clipboard().unwrap().text().unwrap(),
                "中文\nnext"
            );
            window.simulate_keystrokes(&format!("{modifier}-x"));
            editor.read_with(window, |editor, _| assert_eq!(editor.value(), ""));
            assert_eq!(
                window.read_from_clipboard().unwrap().text().unwrap(),
                "中文\nnext"
            );
        }
    }

    #[gpui::test]
    fn image_paste_shortcuts_emit_attachments_without_changing_text(cx: &mut gpui::TestAppContext) {
        use std::{cell::RefCell, rc::Rc};
        cx.update(bind_prompt_editor_keys);
        for modifier in ["cmd", "ctrl"] {
            let (editor, window) = cx.add_window_view(|_, cx| PromptEditor::new(cx, "Message"));
            let images = Rc::new(RefCell::new(Vec::new()));
            let received = images.clone();
            let _subscription = window.update(|window, cx| {
                editor.update(cx, |editor, cx| editor.set_value("keep this", cx));
                window.focus(&editor.read(cx).focus_handle(cx), cx);
                cx.write_to_clipboard(ClipboardItem::new_image(&gpui::Image::from_bytes(
                    ImageFormat::Png,
                    vec![1, 2, 3],
                )));
                cx.subscribe(&editor, move |_, event: &PromptEditorEvent, _| {
                    if let PromptEditorEvent::ImagePasted(images) = event {
                        received.borrow_mut().extend(images.clone());
                    }
                })
            });
            window.simulate_keystrokes(&format!("{modifier}-v"));
            window.run_until_parked();
            assert_eq!(
                &*images.borrow(),
                &[PastedImage::new(ImageFormat::Png, vec![1, 2, 3])]
            );
            editor.read_with(window, |editor, _| assert_eq!(editor.value(), "keep this"));
        }
    }

    #[test]
    fn line_splitting_keeps_byte_offsets() {
        let content = "第一行\nsecond\n";
        let mut lines = Vec::new();
        let mut start = 0;
        for piece in content.split('\n') {
            lines.push((start, piece.len()));
            start += piece.len() + 1;
        }
        assert_eq!(lines, vec![(0, 9), (10, 6), (17, 0)]);
        assert_eq!(start, content.len() + 1);
    }

    #[test]
    fn mentions_scan_tokens_and_colors() {
        let highlights = vec![("@Workflow".into(), 0x24864c)];
        let ranges = mention_ranges(
            "试试 @Workflow 和 @未注册 以及 email@a.com",
            &highlights,
            0x007aff,
        );
        let spans: Vec<(usize, usize, u32)> = ranges
            .into_iter()
            .map(|(range, color)| (range.start, range.end, color))
            .collect();
        // The @ inside email@a.com is not preceded by whitespace, so it is
        // not treated as a mention.
        assert_eq!(spans, vec![(7, 16, 0x24864c), (21, 31, 0x007aff)],);
    }

    #[test]
    fn registered_skill_tokens_are_highlighted_without_coloring_shell_variables() {
        let text = "$wps365-skills $HOME $unknown inline$wps365-skills @README";
        let ranges = mention_ranges(text, &[("$wps365-skills".into(), 0x007aff)], 0x112233);
        let spans: Vec<_> = ranges
            .iter()
            .map(|(range, color)| (&text[range.clone()], *color))
            .collect();
        assert_eq!(
            spans,
            vec![("$wps365-skills", 0x007aff), ("@README", 0x112233)]
        );
    }

    #[test]
    fn clipboard_images_come_from_entries_or_image_files() {
        let png = PastedImage::new(ImageFormat::Png, vec![1, 2, 3]);
        let mut item = ClipboardItem::new_string("纯文本".into());
        assert!(clipboard_images(&item).unwrap().is_empty());

        item = ClipboardItem::new_image(&gpui::Image::from_bytes(ImageFormat::Tiff, vec![9, 9]));
        let images = clipboard_images(&item).unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].format(), ImageFormat::Tiff);
        assert_eq!(images[0].media_type(), "image/tiff");
        assert_eq!(images[0].bytes(), &[9, 9]);

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shot.png");
        std::fs::write(&path, [4, 5, 6]).unwrap();
        item = ClipboardItem {
            entries: vec![ClipboardEntry::ExternalPaths(gpui::ExternalPaths(
                vec![path].into(),
            ))],
        };
        let images = clipboard_images(&item).unwrap();
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].format(), ImageFormat::Png);
        assert_eq!(images[0].bytes(), &[4, 5, 6]);
        assert_eq!(png, PastedImage::new(ImageFormat::Png, vec![1, 2, 3]));
    }

    #[test]
    fn image_formats_are_detected_from_file_extensions() {
        let directory = tempfile::tempdir().unwrap();
        let check = |name: &str, expected: Option<ImageFormat>| {
            let path = directory.path().join(name);
            assert_eq!(image_format_for(&path), expected, "{name}");
        };
        check("shot.png", Some(ImageFormat::Png));
        check("photo.JPG", Some(ImageFormat::Jpeg));
        check("capture.tiff", Some(ImageFormat::Tiff));
        check("notes.txt", None);
        check("archive", None);
    }
}
