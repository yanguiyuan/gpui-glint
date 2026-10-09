use std::{collections::HashSet, ops::Range};

use gpui::prelude::FluentBuilder;
use gpui::{
    App, Bounds, Context, Element, ElementId, GlobalElementId, InspectorElementId,
    InteractiveElement, IntoElement, LayoutId, ParentElement, Pixels, Render, ShapedLine,
    SharedString, StatefulInteractiveElement, Style, Styled, TextAlign, TextRun, Window, div, px,
    rgb,
};

use super::Icon;
use super::code_card::CodeColors;
use crate::theme::ThemePreset;

/// One rendered line of a diff: context, added, or deleted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Added,
    Deleted,
}

/// A single line of a unified diff with its origin line numbers.
#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_number: Option<usize>,
    pub new_number: Option<usize>,
    pub text: SharedString,
}

impl DiffLine {
    fn context(number: usize, text: impl Into<SharedString>) -> Self {
        Self {
            kind: DiffLineKind::Context,
            old_number: None,
            new_number: Some(number),
            text: text.into(),
        }
    }

    fn added(number: usize, text: impl Into<SharedString>) -> Self {
        Self {
            kind: DiffLineKind::Added,
            old_number: None,
            new_number: Some(number),
            text: text.into(),
        }
    }

    fn deleted(number: usize, text: impl Into<SharedString>) -> Self {
        Self {
            kind: DiffLineKind::Deleted,
            old_number: Some(number),
            new_number: None,
            text: text.into(),
        }
    }
}

/// Colors for diff tints. Independent of the syntax palette; pick by appearance.
#[derive(Clone, Copy, Debug)]
pub struct DiffColors {
    pub deleted_row: u32,
    pub added_row: u32,
    pub deleted_word: u32,
    pub added_word: u32,
    pub deleted_marker: u32,
    pub added_marker: u32,
    pub number: u32,
    pub expander_bg: u32,
    pub expander_border: u32,
}

impl DiffColors {
    pub const LIGHT: Self = Self {
        deleted_row: 0xfdeeee,
        added_row: 0xe9f6ec,
        deleted_word: 0xf6c8c8,
        added_word: 0xbfe8c4,
        deleted_marker: 0xe05252,
        added_marker: 0x34a853,
        number: 0x9a9aa0,
        expander_bg: 0xf0f0f2,
        expander_border: 0xdedee3,
    };

    pub const DARK: Self = Self {
        deleted_row: 0x35232a,
        added_row: 0x1e3026,
        deleted_word: 0x57282c,
        added_word: 0x2a4f35,
        deleted_marker: 0xff6961,
        added_marker: 0x4bc26b,
        number: 0x77777e,
        expander_bg: 0x26282e,
        expander_border: 0x3c3e45,
    };
}

/// Line-level diff via LCS. Very large inputs fall back to a whole replace so
/// memory stays bounded.
pub(crate) fn diff_lines(old_text: &str, new_text: &str) -> Vec<DiffLine> {
    let mut old: Vec<&str> = old_text.split('\n').collect();
    let mut new: Vec<&str> = new_text.split('\n').collect();
    if old.last() == Some(&"") {
        old.pop();
    }
    if new.last() == Some(&"") {
        new.pop();
    }
    if old.len() * new.len() > 400 * 400 {
        let mut lines: Vec<DiffLine> = old
            .iter()
            .enumerate()
            .map(|(i, text)| DiffLine::deleted(i + 1, (*text).to_string()))
            .collect();
        lines.extend(
            new.iter()
                .enumerate()
                .map(|(i, text)| DiffLine::added(i + 1, (*text).to_string())),
        );
        return lines;
    }

    // lcs[i][j] = LCS length of old[i..] and new[j..]
    let mut lcs = vec![vec![0u32; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lcs[i][j] = if old[i] == new[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut lines = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            lines.push(DiffLine::context(j + 1, new[j].to_string()));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            lines.push(DiffLine::deleted(i + 1, old[i].to_string()));
            i += 1;
        } else {
            lines.push(DiffLine::added(j + 1, new[j].to_string()));
            j += 1;
        }
    }
    while i < old.len() {
        lines.push(DiffLine::deleted(i + 1, old[i].to_string()));
        i += 1;
    }
    while j < new.len() {
        lines.push(DiffLine::added(j + 1, new[j].to_string()));
        j += 1;
    }
    lines
}

/// Byte ranges of changed words between a deleted and its replacement line,
/// for the word-level tint used on paired rows.
pub(crate) fn changed_word_ranges(
    deleted: &str,
    added: &str,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    fn words(text: &str) -> Vec<(Range<usize>, &str)> {
        let mut result = Vec::new();
        let mut start = None;
        let mut wordlike = false;
        for (index, ch) in text.char_indices() {
            let current = ch.is_alphanumeric() || ch == '_';
            if current != wordlike
                && let Some(begin) = start.take()
            {
                result.push((begin..index, &text[begin..index]));
            }
            if current != wordlike || start.is_none() {
                start = Some(index);
            }
            wordlike = current;
        }
        if let Some(begin) = start {
            result.push((begin..text.len(), &text[begin..]));
        }
        result
    }
    let old_words = words(deleted);
    let new_words = words(added);
    // LCS over word tokens.
    let mut lcs = vec![vec![0u32; new_words.len() + 1]; old_words.len() + 1];
    for i in (0..old_words.len()).rev() {
        for j in (0..new_words.len()).rev() {
            lcs[i][j] = if old_words[i].1 == new_words[j].1 {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut deleted_changed: Vec<Range<usize>> = Vec::new();
    let mut added_changed: Vec<Range<usize>> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < old_words.len() && j < new_words.len() {
        if old_words[i].1 == new_words[j].1 {
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            deleted_changed.push(old_words[i].0.clone());
            i += 1;
        } else {
            added_changed.push(new_words[j].0.clone());
            j += 1;
        }
    }
    while i < old_words.len() {
        deleted_changed.push(old_words[i].0.clone());
        i += 1;
    }
    while j < new_words.len() {
        added_changed.push(new_words[j].0.clone());
        j += 1;
    }
    (deleted_changed, added_changed)
}

const LINE_HEIGHT: f32 = 21.;
const CONTEXT_VISIBLE: usize = 3;

/// A read-only unified diff card: file header with add/delete stats, numbered
/// rows tinted per kind with word-level highlights, and collapsible
/// "N unmodified lines" hunks. Built for reviewing AI-produced code changes.
pub struct DiffCard {
    language: SharedString,
    name: SharedString,
    path: SharedString,
    lines: Vec<DiffLine>,
    word_tints: Vec<Option<(Vec<Range<usize>>, Vec<Range<usize>>)>>,
    expanded_hunks: HashSet<usize>,
    body_visible: bool,
    theme: ThemePreset,
}

impl DiffCard {
    pub fn new(
        cx: &mut Context<Self>,
        language: impl Into<SharedString>,
        old_text: impl Into<SharedString>,
        new_text: impl Into<SharedString>,
    ) -> Self {
        let mut card = Self {
            language: language.into(),
            name: "diff".into(),
            path: "".into(),
            lines: Vec::new(),
            word_tints: Vec::new(),
            expanded_hunks: HashSet::new(),
            body_visible: true,
            theme: ThemePreset::default(),
        };
        card.set_diff(old_text, new_text, cx);
        card
    }

    /// Builds from precomputed rows; hosts that already hold a diff skip the
    /// internal LCS.
    pub fn from_lines(
        cx: &mut Context<Self>,
        language: impl Into<SharedString>,
        lines: Vec<DiffLine>,
    ) -> Self {
        let mut card = Self {
            language: language.into(),
            name: "diff".into(),
            path: "".into(),
            lines,
            word_tints: Vec::new(),
            expanded_hunks: HashSet::new(),
            body_visible: true,
            theme: ThemePreset::default(),
        };
        card.recompute_word_tints();
        cx.notify();
        card
    }

    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = name.into();
        self
    }

    pub fn path(mut self, path: impl Into<SharedString>) -> Self {
        self.path = path.into();
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

    pub fn set_name(&mut self, name: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.name = name.into();
        cx.notify();
    }

    pub fn set_path(&mut self, path: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.path = path.into();
        cx.notify();
    }

    /// Recomputes the diff from raw texts; expansion state resets.
    pub fn set_diff(
        &mut self,
        old_text: impl Into<SharedString>,
        new_text: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let old_text = old_text.into();
        let new_text = new_text.into();
        self.lines = diff_lines(&old_text, &new_text);
        self.recompute_word_tints();
        self.expanded_hunks.clear();
        self.body_visible = true;
        cx.notify();
    }

    pub fn set_lines(&mut self, lines: Vec<DiffLine>, cx: &mut Context<Self>) {
        self.lines = lines;
        self.recompute_word_tints();
        self.expanded_hunks.clear();
        cx.notify();
    }

    pub fn toggle_body(&mut self, cx: &mut Context<Self>) {
        self.body_visible = !self.body_visible;
        cx.notify();
    }

    pub fn expand_all(&mut self, cx: &mut Context<Self>) {
        self.expanded_hunks = (0..self.hunk_count()).collect();
        cx.notify();
    }

    pub fn body_visible(&self) -> bool {
        self.body_visible
    }

    pub fn stats(&self) -> (usize, usize) {
        let adds = self
            .lines
            .iter()
            .filter(|line| line.kind == DiffLineKind::Added)
            .count();
        let dels = self
            .lines
            .iter()
            .filter(|line| line.kind == DiffLineKind::Deleted)
            .count();
        (adds, dels)
    }

    fn hunk_count(&self) -> usize {
        self.hunks()
            .iter()
            .filter(|row| matches!(row, Row::Hunk { .. }))
            .count()
    }

    fn recompute_word_tints(&mut self) {
        self.word_tints = vec![None; self.lines.len()];
        let mut index = 0;
        while index < self.lines.len() {
            if self.lines[index].kind == DiffLineKind::Deleted
                && let Some(next) = self.lines.get(index + 1)
                && next.kind == DiffLineKind::Added
            {
                let (deleted_words, added_words) =
                    changed_word_ranges(&self.lines[index].text, &next.text);
                self.word_tints[index] = Some((deleted_words, Vec::new()));
                self.word_tints[index + 1] = Some((Vec::new(), added_words));
                index += 2;
            } else {
                index += 1;
            }
        }
    }

    /// Walks the rows, grouping long context runs into collapsible hunks.
    fn hunks(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut hunk_index = 0usize;
        let mut run: Vec<usize> = Vec::new();
        let flush_run = |rows: &mut Vec<Row>,
                         run: &mut Vec<usize>,
                         hunk_index: &mut usize,
                         expanded: &HashSet<usize>| {
            if run.len() > CONTEXT_VISIBLE * 2 + 1 {
                for &line in &run[..CONTEXT_VISIBLE] {
                    rows.push(Row::Line(line));
                }
                let hidden = run.len() - CONTEXT_VISIBLE * 2;
                let id = *hunk_index;
                *hunk_index += 1;
                if expanded.contains(&id) {
                    for &line in &run[CONTEXT_VISIBLE..] {
                        rows.push(Row::Line(line));
                    }
                } else {
                    rows.push(Row::Hunk {
                        id,
                        count: hidden,
                        first: run[CONTEXT_VISIBLE],
                    });
                    for &line in &run[run.len() - CONTEXT_VISIBLE..] {
                        rows.push(Row::Line(line));
                    }
                }
            } else {
                for &line in run.iter() {
                    rows.push(Row::Line(line));
                }
            }
            run.clear();
        };
        for (index, line) in self.lines.iter().enumerate() {
            if line.kind == DiffLineKind::Context {
                run.push(index);
            } else {
                flush_run(&mut rows, &mut run, &mut hunk_index, &self.expanded_hunks);
                rows.push(Row::Line(index));
            }
        }
        flush_run(&mut rows, &mut run, &mut hunk_index, &self.expanded_hunks);
        rows
    }
}

enum Row {
    Line(usize),
    Hunk {
        id: usize,
        count: usize,
        first: usize,
    },
}

impl Render for DiffCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let p = theme.palette();
        let dark = theme.is_dark();
        let colors = if dark {
            CodeColors::DARK
        } else {
            CodeColors::LIGHT
        };
        let diff_colors = if dark {
            DiffColors::DARK
        } else {
            DiffColors::LIGHT
        };
        let (adds, dels) = self.stats();

        let header = div()
            .id("glass-diff-card-header")
            .w_full()
            .h(px(46.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(9.))
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| this.toggle_body(cx)))
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(rgb(p.text_muted))
                    .child(Icon::for_language(&self.language).render(15.)),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(13.5))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(p.text))
                    .child(self.name.clone()),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .text_color(rgb(p.text_muted))
                    .child(self.path.clone()),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(13.))
                    .text_color(rgb(diff_colors.added_marker))
                    .child(format!("+{adds}")),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(13.))
                    .text_color(rgb(diff_colors.deleted_marker))
                    .child(format!("-{dels}")),
            )
            .child(div().flex_shrink_0().text_color(rgb(p.text_muted)).child(
                if self.body_visible {
                    Icon::ChevronDown.render(13.)
                } else {
                    Icon::ChevronDown.render(13.)
                },
            ));

        let mut root = div()
            .id("glass-diff-card")
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .rounded(px(9.))
            .border_1()
            .border_color(rgb(colors.border))
            .bg(rgb(colors.surface))
            .overflow_hidden()
            .font_family("Menlo")
            .child(header);

        if self.body_visible {
            let mut body = div()
                .id("glass-diff-card-body")
                .w_full()
                .max_h(px(24. * LINE_HEIGHT))
                .overflow_y_scroll()
                .flex()
                .flex_col();
            let rows = self.hunks();
            for row in rows {
                match row {
                    Row::Line(index) => {
                        let line = &self.lines[index];
                        let (tint, word_ranges) = match line.kind {
                            DiffLineKind::Context => (None, None),
                            DiffLineKind::Added => (
                                Some(rgb(diff_colors.added_row)),
                                self.word_tints[index]
                                    .as_ref()
                                    .map(|(_, added)| added.clone()),
                            ),
                            DiffLineKind::Deleted => (
                                Some(rgb(diff_colors.deleted_row)),
                                self.word_tints[index]
                                    .as_ref()
                                    .map(|(deleted, _)| deleted.clone()),
                            ),
                        };
                        let number = line.new_number.or(line.old_number);
                        let word_bg = match line.kind {
                            DiffLineKind::Added => Some(diff_colors.added_word),
                            DiffLineKind::Deleted => Some(diff_colors.deleted_word),
                            DiffLineKind::Context => None,
                        };
                        let marker = match line.kind {
                            DiffLineKind::Added => Some(diff_colors.added_marker),
                            DiffLineKind::Deleted => Some(diff_colors.deleted_marker),
                            DiffLineKind::Context => None,
                        };
                        let runs = line_runs(
                            &line.text,
                            &self.language,
                            colors,
                            word_ranges.as_deref(),
                            word_bg,
                        );
                        let mut row = div()
                            .w_full()
                            .flex()
                            .when_some(tint, |row: gpui::Div, tint| row.bg(tint));
                        if let Some(color) = marker {
                            let deleted = line.kind == DiffLineKind::Deleted;
                            let mut bar = div()
                                .w(px(6.))
                                .flex_shrink_0()
                                .border_l_3()
                                .border_color(rgb(color));
                            if deleted {
                                bar = bar.border_dashed();
                            }
                            row = row.child(bar);
                        } else {
                            row = row.child(div().w(px(6.)).flex_shrink_0());
                        }
                        row = row
                            .child(
                                div()
                                    .w(px(44.))
                                    .flex_shrink_0()
                                    .pr(px(10.))
                                    .text_align(TextAlign::Right)
                                    .text_size(px(12.))
                                    .line_height(px(LINE_HEIGHT))
                                    .text_color(rgb(diff_colors.number))
                                    .child(number.map(|n| n.to_string()).unwrap_or_default()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .line_height(px(LINE_HEIGHT))
                                    .child(DiffLineElement {
                                        runs,
                                        text: line.text.clone(),
                                    }),
                            );
                        body = body.child(row);
                    }
                    Row::Hunk { id, count, first } => {
                        body = body.child(
                            div()
                                .id(SharedString::from(format!("glass-diff-hunk-{id}")))
                                .mx(px(8.))
                                .my(px(6.))
                                .h(px(34.))
                                .px(px(10.))
                                .flex()
                                .items_center()
                                .gap(px(9.))
                                .rounded(px(8.))
                                .border_1()
                                .border_color(rgb(diff_colors.expander_border))
                                .bg(rgb(diff_colors.expander_bg))
                                .text_size(px(12.))
                                .text_color(rgb(p.text_muted))
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.expanded_hunks.insert(id);
                                    let _ = first;
                                    cx.notify();
                                }))
                                .child(div().flex_shrink_0().child(Icon::ChevronDown.render(13.)))
                                .child(format!("{count} unmodified lines")),
                        );
                    }
                }
            }
            root = root.child(body);
        }
        root
    }
}

/// Builds paint runs for one diff row: syntax colors underneath, word-level
/// background tints on top. Runs are contiguous and cover the whole text.
fn line_runs(
    text: &str,
    language: &str,
    colors: CodeColors,
    word_ranges: Option<&[Range<usize>]>,
    word_bg: Option<u32>,
) -> Vec<TextRun> {
    let syntax = super::code_card::highlight_runs(text, language, colors);
    let mut boundaries: Vec<usize> = syntax
        .iter()
        .flat_map(|(range, _)| [range.start, range.end])
        .collect();
    if let Some(words) = word_ranges {
        boundaries.extend(words.iter().flat_map(|range| [range.start, range.end]));
    }
    boundaries.push(0);
    boundaries.push(text.len());
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut runs: Vec<TextRun> = boundaries
        .windows(2)
        .map(|pair| {
            let start = pair[0];
            let color = syntax
                .iter()
                .find(|(range, _)| range.start <= start && start < range.end)
                .map(|(_, color)| *color)
                .unwrap_or(colors.text);
            let tinted = word_bg.is_some()
                && word_ranges.is_some_and(|words| {
                    words
                        .iter()
                        .any(|range| range.start <= start && start < range.end)
                });
            TextRun {
                letter_spacing: None,
                len: pair[1] - start,
                font: window_font(),
                color: gpui::rgb_to_hsla(rgb(color)),
                background_color: tinted.then(|| gpui::rgb_to_hsla(rgb(word_bg.unwrap()))),
                underline: None,
                strikethrough: None,
            }
        })
        .collect();
    if runs.is_empty() {
        runs.push(TextRun {
            letter_spacing: None,
            len: 0,
            font: window_font(),
            color: gpui::rgb_to_hsla(rgb(colors.text)),
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    runs
}

// The font is filled in at paint time from the window's text style; Menlo is
// set on the card root. A placeholder font resolves through the refinement.
fn window_font() -> gpui::Font {
    gpui::font(".SystemUIFont")
}

/// Paints one highlighted code line, including per-run background tints.
struct DiffLineElement {
    text: SharedString,
    runs: Vec<TextRun>,
}

struct DiffLinePrepaint {
    line: Option<ShapedLine>,
}

impl IntoElement for DiffLineElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for DiffLineElement {
    type RequestLayoutState = ();
    type PrepaintState = DiffLinePrepaint;

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
        style.size.height = px(LINE_HEIGHT).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let style = window.text_style();
        let mut runs = self.runs.clone();
        for run in &mut runs {
            run.font = style.font();
        }
        let line = window.text_system().shape_line(
            self.text.clone(),
            style.font_size.to_pixels(window.rem_size()),
            &runs,
            None,
        );
        DiffLinePrepaint { line: Some(line) }
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
        if let Some(line) = prepaint.line.take() {
            line.paint_background(
                bounds.origin,
                px(LINE_HEIGHT),
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            )
            .ok();
            line.paint(
                bounds.origin,
                px(LINE_HEIGHT),
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            )
            .ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_produces_context_and_pairs() {
        let old = "mod a;\npub use b::{One, Two};\nmod c;\n";
        let new = "mod a;\npub use b::{One, Two};\nmod d;\n\nmod e;\n";
        let lines = diff_lines(old, new);
        let kinds: Vec<DiffLineKind> = lines.iter().map(|line| line.kind).collect();
        assert_eq!(
            kinds,
            vec![
                DiffLineKind::Context,
                DiffLineKind::Context,
                DiffLineKind::Deleted,
                DiffLineKind::Added,
                DiffLineKind::Added,
                DiffLineKind::Added,
            ]
        );
        // Context rows carry the new numbering; deleted keep the old one.
        assert_eq!(lines[0].new_number, Some(1));
        assert_eq!(lines[2].old_number, Some(3));
        assert_eq!(lines[3].new_number, Some(3));
        assert_eq!(lines[3].old_number, None);
    }

    #[test]
    fn word_ranges_mark_only_changed_tokens() {
        let (deleted, added) = changed_word_ranges("mod compound_input;", "mod select_field;");
        assert_eq!(deleted, vec![4..18]); // compound_input
        assert_eq!(added, vec![4..16]); // select_field
    }

    #[test]
    fn trailing_newline_yields_no_phantom_line() {
        let lines = diff_lines("a\n", "a\n");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].kind, DiffLineKind::Context);
    }
}
