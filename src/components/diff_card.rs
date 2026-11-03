use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    rc::Rc,
};

use gpui::prelude::FluentBuilder;
use gpui::{
    App, Bounds, Context, Element, ElementId, GlobalElementId, InspectorElementId,
    InteractiveElement, IntoElement, LayoutId, ParentElement, Pixels, Render, ScrollHandle,
    ShapedLine, SharedString, StatefulInteractiveElement, Style, Styled, TextAlign, TextRun,
    Window, div, px, rgb,
};

use super::Icon;
use super::code_card::CodeColors;
use super::scroll_chain::ChainScroll;
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
    pub whitespace_row: u32,
    pub whitespace_marker: u32,
    pub whitespace_text: u32,
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
        whitespace_row: 0xf3f3f6,
        whitespace_marker: 0xb4b4bd,
        whitespace_text: 0x85858d,
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
        whitespace_row: 0x25272e,
        whitespace_marker: 0x5a5a63,
        whitespace_text: 0x8b8b94,
    };
}

/// Line-level diff with bounded computation time and original line numbers.
pub fn diff_lines(old_text: &str, new_text: &str) -> Vec<DiffLine> {
    let diff = similar::TextDiff::configure()
        .timeout(std::time::Duration::from_millis(250))
        .diff_lines(old_text, new_text);
    diff.iter_all_changes()
        .map(|change| DiffLine {
            kind: match change.tag() {
                similar::ChangeTag::Equal => DiffLineKind::Context,
                similar::ChangeTag::Insert => DiffLineKind::Added,
                similar::ChangeTag::Delete => DiffLineKind::Deleted,
            },
            old_number: change.old_index().map(|index| index + 1),
            new_number: change.new_index().map(|index| index + 1),
            text: change
                .value()
                .trim_end_matches('\n')
                .trim_end_matches('\r')
                .to_owned()
                .into(),
        })
        .collect()
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
    // Generated/minified lines can contain thousands of tokens. Keep optional
    // word highlighting from allocating a quadratic table on the UI thread.
    if old_words.len().saturating_mul(new_words.len()) > 100_000 {
        return (vec![0..deleted.len()], vec![0..added.len()]);
    }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffCardEvent {
    TitleClicked,
}
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
    header_visible: bool,
    title_clickable: bool,
    side_by_side: bool,
    theme: ThemePreset,
    rows: Option<Rc<Vec<Row>>>,
    row_offsets: Vec<f32>,
    runs: HashMap<usize, Vec<TextRun>>,
    scroll: ScrollHandle,
    rendered_rows: Range<usize>,
    totals: (usize, usize),
    /// Line indices belonging to replaced pairs that differ only in whitespace.
    whitespace_pairs: HashSet<usize>,
    hide_whitespace: bool,
}

impl gpui::EventEmitter<DiffCardEvent> for DiffCard {}
impl DiffCard {
    /// Opt into title navigation while keeping the disclosure arrow independent.
    pub fn title_clickable(mut self, enabled: bool) -> Self {
        self.title_clickable = enabled;
        self
    }
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
            header_visible: true,
            title_clickable: false,
            side_by_side: false,
            theme: ThemePreset::default(),
            rows: None,
            row_offsets: Vec::new(),
            runs: HashMap::new(),
            scroll: ScrollHandle::new(),
            rendered_rows: 0..0,
            totals: (0, 0),
            whitespace_pairs: HashSet::new(),
            hide_whitespace: false,
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
            header_visible: true,
            title_clickable: false,
            side_by_side: false,
            theme: ThemePreset::default(),
            rows: None,
            row_offsets: Vec::new(),
            runs: HashMap::new(),
            scroll: ScrollHandle::new(),
            rendered_rows: 0..0,
            totals: (0, 0),
            whitespace_pairs: HashSet::new(),
            hide_whitespace: false,
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

    /// Hosts with their own file disclosure header can display just the diff.
    pub fn header_visible(mut self, visible: bool) -> Self {
        self.header_visible = visible;
        self
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            self.runs.clear();
            cx.notify();
        }
    }

    /// Align old/new lines in two columns, with blank cells for unmatched lines.
    pub fn side_by_side(mut self, enabled: bool) -> Self {
        self.side_by_side = enabled;
        self
    }

    pub fn set_side_by_side(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.side_by_side != enabled {
            self.side_by_side = enabled;
            self.rows = None;
            cx.notify();
        }
    }

    /// Number of replaced line pairs whose only difference is whitespace.
    pub fn whitespace_changed_pairs(&self) -> usize {
        self.whitespace_pairs.len() / 2
    }

    pub fn hide_whitespace_enabled(&self) -> bool {
        self.hide_whitespace
    }

    pub fn hide_whitespace(mut self, enabled: bool) -> Self {
        self.hide_whitespace = enabled;
        self
    }

    /// Treats formatting-only rows as foldable context; stats stay raw.
    pub fn set_hide_whitespace(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.hide_whitespace != enabled {
            self.hide_whitespace = enabled;
            self.rows = None;
            self.runs.clear();
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
        self.expanded_hunks.clear();
        self.expanded_hunks = (0..self.hunk_count()).collect();
        self.rows = None;
        cx.notify();
    }

    pub fn body_visible(&self) -> bool {
        self.body_visible
    }

    pub fn stats(&self) -> (usize, usize) {
        self.totals
    }

    fn hunk_count(&self) -> usize {
        self.hunks()
            .iter()
            .filter(|row| matches!(row, Row::Hunk { .. }))
            .count()
    }

    fn recompute_word_tints(&mut self) {
        self.rows = None;
        self.runs.clear();
        self.whitespace_pairs.clear();
        self.totals = self.lines.iter().fold((0, 0), |(adds, dels), line| {
            (
                adds + usize::from(line.kind == DiffLineKind::Added),
                dels + usize::from(line.kind == DiffLineKind::Deleted),
            )
        });
        self.word_tints = vec![None; self.lines.len()];
        // Anchor pairs by whitespace-normalized content before pairing the
        // remaining replacements. Insertions must not shift every later pair.
        let mut index = 0;
        while index < self.lines.len() {
            if self.lines[index].kind == DiffLineKind::Deleted {
                let deleted_start = index;
                let mut end = index;
                while end < self.lines.len() && self.lines[end].kind == DiffLineKind::Deleted {
                    end += 1;
                }
                let deleted_end = end;
                let added_start = end;
                let mut added_end = added_start;
                while added_end < self.lines.len()
                    && self.lines[added_end].kind == DiffLineKind::Added
                {
                    added_end += 1;
                }
                if added_end > added_start {
                    let deleted: Vec<_> = (deleted_start..deleted_end).collect();
                    let added: Vec<_> = (added_start..added_end).collect();
                    for (deleted, added) in align_replacements(&deleted, &added, &self.lines) {
                        let (Some(deleted), Some(added)) = (deleted, added) else {
                            continue;
                        };
                        let (deleted_words, added_words) =
                            changed_word_ranges(&self.lines[deleted].text, &self.lines[added].text);
                        self.word_tints[deleted] = Some((deleted_words, Vec::new()));
                        self.word_tints[added] = Some((Vec::new(), added_words));
                        if is_whitespace_only_change(
                            &self.lines[deleted].text,
                            &self.lines[added].text,
                        ) {
                            self.whitespace_pairs.insert(deleted);
                            self.whitespace_pairs.insert(added);
                        }
                    }
                }
                index = if added_end > added_start {
                    added_end
                } else {
                    deleted_end
                };
            } else {
                index += 1;
            }
        }
    }

    fn visible_rows(&mut self) -> (Rc<Vec<Row>>, Range<usize>) {
        let rows = if self.rows.is_none() {
            let rows = Rc::new(if self.side_by_side {
                split_rows(self.hunks(), &self.lines)
            } else {
                self.hunks()
            });
            self.row_offsets = Vec::with_capacity(rows.len() + 1);
            self.row_offsets.push(0.);
            for row in rows.iter() {
                let height = match row {
                    Row::Line(_) | Row::Whitespace(_, _) | Row::Pair(_, _) => LINE_HEIGHT,
                    Row::Hunk { .. } => 46.,
                };
                self.row_offsets
                    .push(self.row_offsets.last().copied().unwrap_or(0.) + height);
            }
            self.rows = Some(rows.clone());
            rows
        } else {
            self.rows.as_ref().unwrap().clone()
        };
        let top = (-f32::from(self.scroll.offset().y)).max(0.);
        let height = f32::from(self.scroll.bounds().size.height).max(24. * LINE_HEIGHT);
        let start = self
            .row_offsets
            .partition_point(|offset| *offset < top)
            .saturating_sub(9)
            .min(rows.len());
        let end = (self
            .row_offsets
            .partition_point(|offset| *offset <= top + height)
            + 8)
        .min(rows.len());
        self.rendered_rows = start..end;
        (rows, self.rendered_rows.clone())
    }

    /// Walks the rows, grouping long context runs into collapsible hunks.
    fn hunks(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut hunk_index = 0usize;
        let mut run: Vec<Row> = Vec::new();
        let flush_run = |rows: &mut Vec<Row>,
                         run: &mut Vec<Row>,
                         hunk_index: &mut usize,
                         expanded: &HashSet<usize>| {
            if run.len() > CONTEXT_VISIBLE * 2 + 1 {
                rows.extend_from_slice(&run[..CONTEXT_VISIBLE]);
                let hidden = run.len() - CONTEXT_VISIBLE * 2;
                let id = *hunk_index;
                *hunk_index += 1;
                if expanded.contains(&id) {
                    rows.extend_from_slice(&run[CONTEXT_VISIBLE..]);
                } else {
                    rows.push(Row::Hunk {
                        id,
                        count: hidden,
                        whitespace_count: run[CONTEXT_VISIBLE..run.len() - CONTEXT_VISIBLE]
                            .iter()
                            .filter(|row| matches!(row, Row::Whitespace(_, _)))
                            .count(),
                        first: match run[CONTEXT_VISIBLE] {
                            Row::Line(i) | Row::Whitespace(_, i) => i,
                            _ => unreachable!(),
                        },
                    });
                    rows.extend_from_slice(&run[run.len() - CONTEXT_VISIBLE..]);
                }
            } else {
                rows.extend_from_slice(run);
            }
            run.clear();
        };
        for row in unified_rows(&self.lines) {
            let context = match row {
                Row::Line(index) => self.lines[index].kind == DiffLineKind::Context,
                Row::Whitespace(_, _) => self.hide_whitespace,
                _ => false,
            };
            if context {
                run.push(row);
            } else {
                flush_run(&mut rows, &mut run, &mut hunk_index, &self.expanded_hunks);
                rows.push(row);
            }
        }
        flush_run(&mut rows, &mut run, &mut hunk_index, &self.expanded_hunks);
        rows
    }
}

#[derive(Clone, Copy, Debug)]
enum Row {
    Line(usize),
    /// One neutral row for a formatting-only replacement, with both origins.
    Whitespace(usize, usize),
    Pair(Option<usize>, Option<usize>),
    Hunk {
        id: usize,
        count: usize,
        whitespace_count: usize,
        first: usize,
    },
}

/// Align exact token sequences first, then pair only within the gaps between
/// anchors. Keep token boundaries: removing all spaces could merge identifiers.
fn align_replacements(
    deleted: &[usize],
    added: &[usize],
    lines: &[DiffLine],
) -> Vec<(Option<usize>, Option<usize>)> {
    if deleted.is_empty() {
        return added.iter().map(|&i| (None, Some(i))).collect();
    }
    if added.is_empty() {
        return deleted.iter().map(|&i| (Some(i), None)).collect();
    }
    let keys = |indices: &[usize]| -> Vec<String> {
        indices
            .iter()
            .map(|&i| {
                lines[i]
                    .text
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    };
    let old_keys = keys(deleted);
    let new_keys = keys(added);
    let old_refs: Vec<_> = old_keys.iter().map(String::as_str).collect();
    let new_refs: Vec<_> = new_keys.iter().map(String::as_str).collect();
    let diff = similar::TextDiff::configure()
        .timeout(std::time::Duration::from_millis(50))
        .diff_slices(&old_refs, &new_refs);
    let mut pairs = Vec::new();
    let (mut old, mut new) = (0, 0);
    let gap = |pairs: &mut Vec<_>, old: Range<usize>, new: Range<usize>| {
        for offset in 0..old.len().max(new.len()) {
            pairs.push((
                (offset < old.len()).then(|| deleted[old.start + offset]),
                (offset < new.len()).then(|| added[new.start + offset]),
            ));
        }
    };
    for change in diff.iter_all_changes() {
        if change.tag() != similar::ChangeTag::Equal {
            continue;
        }
        let i = change.old_index().unwrap();
        let j = change.new_index().unwrap();
        gap(&mut pairs, old..i, new..j);
        pairs.push((Some(deleted[i]), Some(added[j])));
        old = i + 1;
        new = j + 1;
    }
    gap(&mut pairs, old..deleted.len(), new..added.len());
    pairs
}

fn unified_rows(lines: &[DiffLine]) -> Vec<Row> {
    let pairs = split_rows((0..lines.len()).map(Row::Line).collect(), lines);
    let mut rows = Vec::new();
    let mut deleted = Vec::new();
    let mut added = Vec::new();
    let flush = |rows: &mut Vec<Row>, deleted: &mut Vec<Row>, added: &mut Vec<Row>| {
        rows.append(deleted);
        rows.append(added);
    };
    for row in pairs {
        let Row::Pair(old, new) = row else {
            unreachable!()
        };
        match (old, new) {
            (Some(old), Some(new)) if old == new => {
                flush(&mut rows, &mut deleted, &mut added);
                rows.push(Row::Line(new));
            }
            (Some(old), Some(new))
                if is_whitespace_only_change(&lines[old].text, &lines[new].text) =>
            {
                flush(&mut rows, &mut deleted, &mut added);
                rows.push(Row::Whitespace(old, new));
            }
            (old, new) => {
                if let Some(old) = old {
                    deleted.push(Row::Line(old));
                }
                if let Some(new) = new {
                    added.push(Row::Line(new));
                }
            }
        }
    }
    flush(&mut rows, &mut deleted, &mut added);
    rows
}

fn split_rows(rows: Vec<Row>, lines: &[DiffLine]) -> Vec<Row> {
    let mut output = Vec::new();
    let mut deleted = Vec::new();
    let mut added = Vec::new();
    let flush = |output: &mut Vec<Row>, deleted: &mut Vec<usize>, added: &mut Vec<usize>| {
        for (old, new) in align_replacements(deleted, added, lines) {
            output.push(Row::Pair(old, new));
        }
        deleted.clear();
        added.clear();
    };
    for row in rows {
        match row {
            Row::Line(i) if lines[i].kind == DiffLineKind::Deleted => deleted.push(i),
            Row::Line(i) if lines[i].kind == DiffLineKind::Added => added.push(i),
            Row::Line(i) => {
                flush(&mut output, &mut deleted, &mut added);
                output.push(Row::Pair(Some(i), Some(i)));
            }
            Row::Whitespace(old, new) => {
                flush(&mut output, &mut deleted, &mut added);
                output.push(Row::Pair(Some(old), Some(new)));
            }
            other => {
                flush(&mut output, &mut deleted, &mut added);
                output.push(other);
            }
        }
    }
    flush(&mut output, &mut deleted, &mut added);
    output
}

impl DiffCard {
    fn split_cell(
        &mut self,
        index: Option<usize>,
        old: bool,
        colors: CodeColors,
        palette: DiffColors,
    ) -> gpui::Div {
        let mut cell = div()
            .flex_1()
            .min_w_0()
            .h(px(LINE_HEIGHT))
            .flex()
            .overflow_hidden();
        let Some(index) = index else {
            return cell.bg(rgb(palette.expander_bg));
        };
        let line = &self.lines[index];
        let whitespace_only = self.whitespace_pairs.contains(&index);
        let hidden_whitespace = whitespace_only && self.hide_whitespace;
        let (background, word_bg, ranges) = if hidden_whitespace {
            (None, None, None)
        } else if whitespace_only {
            (Some(palette.whitespace_row), None, None)
        } else {
            match line.kind {
                DiffLineKind::Deleted => (
                    Some(palette.deleted_row),
                    Some(palette.deleted_word),
                    self.word_tints[index]
                        .as_ref()
                        .map(|(ranges, _)| ranges.as_slice()),
                ),
                DiffLineKind::Added => (
                    Some(palette.added_row),
                    Some(palette.added_word),
                    self.word_tints[index]
                        .as_ref()
                        .map(|(_, ranges)| ranges.as_slice()),
                ),
                DiffLineKind::Context => (None, None, None),
            }
        };
        let display_text: SharedString = if whitespace_only && !hidden_whitespace {
            visualize_whitespace(&line.text)
        } else {
            line.text.clone()
        };
        let runs = self
            .runs
            .entry(index)
            .or_insert_with(|| {
                if whitespace_only && !hidden_whitespace {
                    dim_runs(&display_text, palette.whitespace_text)
                } else {
                    line_runs(&line.text, &self.language, colors, ranges, word_bg)
                }
            })
            .clone();
        cell = cell.when_some(background, |cell, background| cell.bg(rgb(background)));
        cell.child(
            div()
                .w(px(50.))
                .flex_shrink_0()
                .pr(px(10.))
                .text_align(TextAlign::Right)
                .text_size(px(12.))
                .line_height(px(LINE_HEIGHT))
                .text_color(rgb(palette.number))
                .child(
                    (if old {
                        line.old_number
                    } else {
                        line.new_number
                    })
                    .map(|n| n.to_string())
                    .unwrap_or_default(),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .line_height(px(LINE_HEIGHT))
                .child(DiffLineElement {
                    runs,
                    text: display_text,
                }),
        )
    }
}

impl Render for DiffCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .debug_selector(|| "diff-card-header".into())
            .w_full()
            .h(px(46.))
            .flex_shrink_0()
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
                    .id("diff-card-title")
                    .debug_selector(|| "diff-card-title".into())
                    .flex_shrink_0()
                    .text_size(px(13.5))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(p.text))
                    .child(self.name.clone())
                    .when(self.title_clickable, |title| {
                        title
                            .cursor_pointer()
                            .hover(|title| title.text_color(rgb(p.accent)))
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.stop_propagation();
                                cx.emit(DiffCardEvent::TitleClicked);
                            }))
                    }),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .text_color(rgb(p.text_muted))
                    .child(crate::paths::display_path(self.path.as_ref())),
            )
            .child(div().flex_1())
            .when(self.whitespace_changed_pairs() > 0, |header| {
                header.child(
                    div()
                        .id("diff-card-whitespace-toggle")
                        .debug_selector(|| "diff-card-whitespace-toggle".into())
                        .flex_shrink_0()
                        .h(px(22.))
                        .px(px(9.))
                        .flex()
                        .items_center()
                        .rounded(px(11.))
                        .border_1()
                        .border_color(rgb(if self.hide_whitespace {
                            p.accent
                        } else {
                            p.border
                        }))
                        .text_size(px(11.))
                        .text_color(rgb(if self.hide_whitespace {
                            p.accent
                        } else {
                            p.text_muted
                        }))
                        .cursor_pointer()
                        .hover(|style| style.text_color(rgb(p.accent)))
                        .child(if self.hide_whitespace {
                            format!("显示空白 {}", self.whitespace_changed_pairs())
                        } else {
                            format!("隐藏空白 {}", self.whitespace_changed_pairs())
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.set_hide_whitespace(!this.hide_whitespace, cx);
                        })),
                )
            })
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
            .font_family(crate::code_font::family())
            .when(self.header_visible, |root| root.child(header));

        if self.body_visible {
            if self.side_by_side {
                root = root.child(
                    div()
                        .w_full()
                        .flex()
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child(div().flex_1().min_w_0().px_3().py_2().child("修改前"))
                        .child(div().flex_1().min_w_0().px_3().py_2().child("修改后")),
                );
            } else {
                let mut headings = div()
                    .w_full()
                    .flex()
                    .pl(px(6.))
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted));
                for label in ["旧行", "新行"] {
                    headings = headings.child(
                        div()
                            .w(px(44.))
                            .flex_shrink_0()
                            .pr(px(10.))
                            .text_align(TextAlign::Right)
                            .child(label),
                    );
                }
                root = root.child(headings);
            }
            let (rows, visible) = self.visible_rows();
            let paint_offset = self.scroll.offset();
            let card = cx.entity();
            let mut body = div()
                .on_children_prepainted(move |_, _, cx| {
                    card.update(cx, |this, cx| {
                        if this.scroll.offset() != paint_offset {
                            let id = cx.entity_id();
                            cx.defer(move |cx| cx.notify(id));
                        }
                    });
                })
                .id("glass-diff-card-body")
                .debug_selector(|| "glass-diff-card-body".into())
                .w_full()
                .max_h(px(if self.side_by_side {
                    (f32::from(window.viewport_size().height) - 220.).max(240.)
                } else {
                    24. * LINE_HEIGHT
                }))
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .chain_scroll_y(&self.scroll)
                .flex()
                .flex_col();
            let top = self.row_offsets.get(visible.start).copied().unwrap_or(0.);
            body = body.child(div().h(px(top)).flex_shrink_0());
            // Bound cached syntax runs as well as live UI elements.
            if self.runs.len() > 512 {
                self.runs.retain(|index, _| {
                    rows[visible.clone()].iter().any(|row| match row {
                        Row::Line(i) => i == index,
                        Row::Whitespace(_, i) => i == index,
                        Row::Pair(left, right) => {
                            left.as_ref() == Some(index) || right.as_ref() == Some(index)
                        }
                        Row::Hunk { .. } => false,
                    })
                });
            }
            for row in rows[visible.clone()].iter().cloned() {
                match row {
                    Row::Pair(left, right) => {
                        let left = self.split_cell(left, true, colors, diff_colors);
                        let right = self.split_cell(right, false, colors, diff_colors);
                        body = body.child(
                            div()
                                .w_full()
                                .h(px(LINE_HEIGHT))
                                .flex_shrink_0()
                                .flex()
                                .child(left.border_r_1().border_color(rgb(p.border)))
                                .child(right),
                        );
                    }
                    Row::Line(index) | Row::Whitespace(_, index) => {
                        let line = &self.lines[index];
                        let whitespace_only = self.whitespace_pairs.contains(&index);
                        let hidden_whitespace = whitespace_only && self.hide_whitespace;
                        let (tint, word_ranges) = if hidden_whitespace {
                            (None, None)
                        } else if whitespace_only {
                            (Some(rgb(diff_colors.whitespace_row)), None)
                        } else {
                            match line.kind {
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
                            }
                        };
                        let old_number = match row {
                            Row::Whitespace(old, _) => self.lines[old].old_number,
                            _ => line.old_number,
                        };
                        let word_bg = match line.kind {
                            DiffLineKind::Added => Some(diff_colors.added_word),
                            DiffLineKind::Deleted => Some(diff_colors.deleted_word),
                            DiffLineKind::Context => None,
                        };
                        let marker = match line.kind {
                            _ if hidden_whitespace => None,
                            DiffLineKind::Added => Some(if whitespace_only {
                                diff_colors.whitespace_marker
                            } else {
                                diff_colors.added_marker
                            }),
                            DiffLineKind::Deleted => Some(if whitespace_only {
                                diff_colors.whitespace_marker
                            } else {
                                diff_colors.deleted_marker
                            }),
                            DiffLineKind::Context => None,
                        };
                        let display_text: SharedString = if whitespace_only && !hidden_whitespace {
                            visualize_whitespace(&line.text)
                        } else {
                            line.text.clone()
                        };
                        let runs = self
                            .runs
                            .entry(index)
                            .or_insert_with(|| {
                                if whitespace_only && !hidden_whitespace {
                                    dim_runs(&display_text, diff_colors.whitespace_text)
                                } else {
                                    line_runs(
                                        &line.text,
                                        &self.language,
                                        colors,
                                        word_ranges.as_deref(),
                                        word_bg,
                                    )
                                }
                            })
                            .clone();
                        let mut row = div()
                            .w_full()
                            .h(px(LINE_HEIGHT))
                            .flex_shrink_0()
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
                        for number in [old_number, line.new_number] {
                            row = row.child(
                                div()
                                    .w(px(44.))
                                    .flex_shrink_0()
                                    .pr(px(10.))
                                    .text_align(TextAlign::Right)
                                    .text_size(px(12.))
                                    .line_height(px(LINE_HEIGHT))
                                    .text_color(rgb(diff_colors.number))
                                    .child(number.map(|n| n.to_string()).unwrap_or_default()),
                            );
                        }
                        row = row.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .line_height(px(LINE_HEIGHT))
                                .child(DiffLineElement {
                                    runs,
                                    text: display_text,
                                }),
                        );
                        body = body.child(row);
                    }
                    Row::Hunk {
                        id,
                        count,
                        first,
                        whitespace_count,
                    } => {
                        body = body.child(
                            div()
                                .id(SharedString::from(format!("glass-diff-hunk-{id}")))
                                .mx(px(8.))
                                .my(px(6.))
                                .h(px(34.))
                                .flex_shrink_0()
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
                                    this.rows = None;
                                    let _ = first;
                                    cx.notify();
                                }))
                                .child(div().flex_shrink_0().child(Icon::ChevronDown.render(13.)))
                                .child(if whitespace_count > 0 {
                                    format!("{count} lines ({whitespace_count} whitespace-only)")
                                } else {
                                    format!("{count} unmodified lines")
                                }),
                        );
                    }
                }
            }
            let bottom = self.row_offsets.last().copied().unwrap_or(0.)
                - self.row_offsets.get(visible.end).copied().unwrap_or(0.);
            body = body.child(div().h(px(bottom.max(0.))).flex_shrink_0());
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
    let mut syntax_index = 0;
    let mut word_index = 0;
    let words = word_ranges.unwrap_or_default();
    let mut runs: Vec<TextRun> = boundaries
        .windows(2)
        .map(|pair| {
            let start = pair[0];
            while syntax_index < syntax.len() && syntax[syntax_index].0.end <= start {
                syntax_index += 1;
            }
            while word_index < words.len() && words[word_index].end <= start {
                word_index += 1;
            }
            let color = syntax
                .get(syntax_index)
                .filter(|(range, _)| range.start <= start && start < range.end)
                .map(|(_, color)| *color)
                .unwrap_or(colors.text);
            let tinted = word_bg.is_some()
                && words
                    .get(word_index)
                    .is_some_and(|range| range.start <= start && start < range.end);
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

/// True when two replaced lines differ only in whitespace: the non-whitespace
/// token sequence is identical but the raw text is not. Used to de-emphasize
/// formatting churn such as realigned `=` columns.
fn is_whitespace_only_change(old: &str, new: &str) -> bool {
    old != new && old.split_whitespace().eq(new.split_whitespace())
}

/// Makes whitespace visible on de-emphasized rows so the exact spacing change
/// can be spotted at a glance: spaces render as `·`, tabs as `→`.
fn visualize_whitespace(text: &str) -> SharedString {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            ' ' => out.push('·'),
            '\t' => out.push('→'),
            other if other.is_whitespace() => out.push('·'),
            other => out.push(other),
        }
    }
    out.into()
}

/// One muted run covering the whole (already visualized) line; de-emphasized
/// rows deliberately skip syntax and word coloring.
fn dim_runs(text: &str, color: u32) -> Vec<TextRun> {
    vec![TextRun {
        letter_spacing: None,
        len: text.len(),
        font: window_font(),
        color: gpui::rgb_to_hsla(rgb(color)),
        background_color: None,
        underline: None,
        strikethrough: None,
    }]
}

// The selected code font is inherited from the card root at paint time.
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

    #[gpui::test]
    fn clickable_title_emits_navigation_without_folding_diff(cx: &mut gpui::TestAppContext) {
        use std::{cell::Cell, rc::Rc};
        let (card, window) = cx.add_window_view(|_, cx| {
            DiffCard::new(cx, "rust", "old", "new")
                .name("src/main.rs")
                .title_clickable(true)
        });
        let count = Rc::new(Cell::new(0));
        let hits = count.clone();
        window.update(|_, cx| {
            cx.subscribe(&card, move |_, _: &DiffCardEvent, _| {
                hits.set(hits.get() + 1)
            })
            .detach()
        });
        let title = window.debug_bounds("diff-card-title").unwrap();
        window.simulate_click(title.center(), gpui::Modifiers::default());
        window.run_until_parked();
        assert_eq!(count.get(), 1);
        card.read_with(window, |card, _| assert!(card.body_visible));
        let header = window.debug_bounds("diff-card-header").unwrap();
        window.simulate_click(
            gpui::point(header.right() - px(10.), header.center().y),
            gpui::Modifiers::default(),
        );
        window.run_until_parked();
        card.read_with(window, |card, _| assert!(!card.body_visible));
        assert_eq!(count.get(), 1);
    }

    #[gpui::test]
    fn long_diff_only_builds_visible_rows_reuses_highlights_and_scrolls_to_end(
        cx: &mut gpui::TestAppContext,
    ) {
        let text = (0..3000)
            .map(|index| format!("let item_{index} = {index};\n"))
            .collect::<String>();
        let (card, window) = cx.add_window_view(|_, cx| DiffCard::new(cx, "rust", "", text));
        window.run_until_parked();
        let rows = card.update(window, |card, cx| {
            assert!(card.rendered_rows.len() < 50);
            assert!(card.runs.len() < 50);
            assert!(card.scroll.max_offset().y > px(50_000.));
            let rows = card.rows.as_ref().unwrap().clone();
            cx.notify();
            rows
        });
        window.run_until_parked();
        let bounds = window.debug_bounds("glass-diff-card-body").unwrap();
        window.simulate_event(gpui::ScrollWheelEvent {
            position: bounds.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-2000.))),
            ..Default::default()
        });
        window.run_until_parked();
        card.update(window, |card, _| assert!(card.rendered_rows.start > 50));
        card.update(window, |card, cx| {
            assert!(Rc::ptr_eq(&rows, card.rows.as_ref().unwrap()));
            card.scroll.scroll_to_bottom();
            cx.notify();
        });
        window.run_until_parked();
        card.update(window, |card, cx| {
            assert_eq!(card.rendered_rows.end, 3000);
            assert!(card.rendered_rows.len() < 50);
            assert!(card.runs.contains_key(&2999));
            card.set_lines(Vec::new(), cx);
        });
        window.run_until_parked();
        card.update(window, |card, _| {
            assert_eq!(card.stats(), (0, 0));
            assert_eq!(card.row_offsets, vec![0.]);
            assert!(card.runs.is_empty());
        });
    }

    #[test]
    fn minified_word_diff_has_bounded_cost() {
        let old = "a + ".repeat(2000);
        let new = "b + ".repeat(2000);
        let (deleted, added) = changed_word_ranges(&old, &new);
        assert_eq!(deleted, vec![0..old.len()]);
        assert_eq!(added, vec![0..new.len()]);
    }

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
    fn side_by_side_aligns_replacements_and_pads_unequal_change_groups() {
        let lines = diff_lines("same\nold1\nold2\ntail\n", "same\nnew\ntail\nextra\n");
        let rows = split_rows((0..lines.len()).map(Row::Line).collect(), &lines);
        let pairs: Vec<_> = rows
            .into_iter()
            .map(|row| match row {
                Row::Pair(left, right) => (
                    left.map(|i| lines[i].text.to_string()),
                    right.map(|i| lines[i].text.to_string()),
                ),
                _ => panic!("expected paired lines"),
            })
            .collect();
        assert_eq!(
            pairs,
            vec![
                (Some("same".into()), Some("same".into())),
                (Some("old1".into()), Some("new".into())),
                (Some("old2".into()), None),
                (Some("tail".into()), Some("tail".into())),
                (None, Some("extra".into())),
            ]
        );
    }

    #[gpui::test]
    fn side_by_side_reuses_hunk_disclosures_and_virtualizes_rows(cx: &mut gpui::TestAppContext) {
        let old = (0..3000).map(|i| format!("line {i}\n")).collect::<String>();
        let new = old.replace("line 1500\n", "changed\n");
        let (card, window) =
            cx.add_window_view(|_, cx| DiffCard::new(cx, "text", old, new).side_by_side(true));
        window.run_until_parked();
        card.update(window, |card, cx| {
            assert!(
                card.rows
                    .as_ref()
                    .unwrap()
                    .iter()
                    .any(|row| matches!(row, Row::Hunk { .. }))
            );
            card.expand_all(cx);
        });
        window.run_until_parked();
        card.update(window, |card, cx| {
            assert_eq!(card.rows.as_ref().unwrap().len(), 3000);
            assert!(card.rendered_rows.len() < 100);
            card.set_side_by_side(false, cx);
        });
        window.run_until_parked();
        card.update(window, |card, _| {
            assert!(
                card.rows
                    .as_ref()
                    .unwrap()
                    .iter()
                    .all(|row| matches!(row, Row::Line(_)))
            );
        });
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

    #[test]
    fn whitespace_only_detection_ignores_spacing_but_not_content() {
        assert!(is_whitespace_only_change("let a  = 1;", "let a = 1;"));
        assert!(is_whitespace_only_change("\tx = y;", "        x = y;"));
        assert!(is_whitespace_only_change("a  b", "a b  "));
        // Identical text is not a change, and punctuation is content.
        assert!(!is_whitespace_only_change("let a = 1;", "let a = 1;"));
        assert!(!is_whitespace_only_change("a.b", "a,b"));
        assert!(!is_whitespace_only_change("let a = 1;", "let a = 2;"));
    }

    #[test]
    fn visualize_whitespace_marks_spaces_and_tabs() {
        assert_eq!(visualize_whitespace("a  b\tc"), "a··b→c");
    }

    #[gpui::test]
    fn alignment_changes_with_insertions_keep_pairs_and_source_numbers(
        cx: &mut gpui::TestAppContext,
    ) {
        // Reproduce the Go constant block: 47 realigned constants, with three
        // new definitions inserted at new lines 665..667 after 14 old entries.
        let prefix = (1..=650)
            .map(|i| format!("// context {i}\n"))
            .collect::<String>();
        let mut old = prefix.clone();
        let mut new = prefix;
        for i in 0..47 {
            old.push_str(&format!("\tMLangItem{i:<2} = \"item_{i}\"\n"));
            if i == 14 {
                for name in ["Layout", "VerticalSpacing", "HorizontalSpacing"] {
                    new.push_str(&format!("\tMLangContainer{name} = \"container_{name}\"\n"));
                }
            }
            new.push_str(&format!("\tMLangItem{i:<2}  = \"item_{i}\"\n"));
        }
        let (card, window) = cx.add_window_view(|_, cx| DiffCard::new(cx, "go", old, new));
        window.run_until_parked();
        card.update(window, |card, cx| {
            assert_eq!(card.whitespace_changed_pairs(), 47);
            assert_eq!(card.stats(), (50, 47));
            let rows = unified_rows(&card.lines);
            assert_eq!(rows.len(), 700);
            let added: Vec<_> = rows
                .iter()
                .filter_map(|row| match row {
                    Row::Line(i) if card.lines[*i].kind == DiffLineKind::Added => {
                        card.lines[*i].new_number
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(added, vec![665, 666, 667]);
            assert!(rows.iter().any(|row| match *row {
                Row::Whitespace(old, new) =>
                    card.lines[old].old_number == Some(665)
                        && card.lines[new].new_number == Some(668),
                _ => false,
            }));
            let split = split_rows(rows, &card.lines);
            assert_eq!(split.len(), 700);
            assert_eq!(
                split
                    .iter()
                    .filter(|row| matches!(row, Row::Pair(None, Some(_))))
                    .count(),
                3
            );
            card.set_hide_whitespace(true, cx);
        });
        window.run_until_parked();
        card.update(window, |card, cx| {
            assert!(card.hunks().iter().any(
                |row| matches!(row, Row::Hunk { whitespace_count, .. } if *whitespace_count > 0)
            ));
            card.expand_all(cx);
            assert_eq!(card.hunks().len(), 700);
        });
    }

    #[test]
    fn normalized_alignment_handles_deletions_and_duplicate_lines() {
        let lines = diff_lines(
            "\t重复 = 1;\n\tremoved = 0;\n\t重复 = 1;\n\tlast = 2;\n",
            "\t重复  = 1;\n\t重复  = 1;\n\tlast  = 2;\n",
        );
        let rows = unified_rows(&lines);
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows.iter()
                .filter(|row| matches!(row, Row::Whitespace(_, _)))
                .count(),
            3
        );
        assert!(
            rows.iter()
                .any(|row| matches!(row, Row::Line(i) if lines[*i].text.contains("removed")))
        );
        let split = split_rows(rows, &lines);
        assert_eq!(
            split
                .iter()
                .filter(|row| matches!(row, Row::Pair(Some(_), None)))
                .count(),
            1
        );
    }

    #[gpui::test]
    fn grouped_replacements_pair_positionally_for_whitespace_detection(
        cx: &mut gpui::TestAppContext,
    ) {
        // A whole-block replace emits D D A A; alignment pairs 0↔2 and 1↔3.
        let (card, window) = cx.add_window_view(|_, cx| {
            DiffCard::new(
                cx,
                "text",
                "alpha = 1;\nbeta = 2;\n",
                "alpha =  1;\ngamma = 3;\n",
            )
        });
        window.run_until_parked();
        card.read_with(window, |card, _| {
            assert_eq!(card.whitespace_changed_pairs(), 1);
            assert!(card.whitespace_pairs.contains(&0) && card.whitespace_pairs.contains(&2));
            assert!(!card.whitespace_pairs.contains(&1));
        });
    }

    #[gpui::test]
    fn whitespace_only_pairs_toggle_between_dimmed_and_context(cx: &mut gpui::TestAppContext) {
        let old = "keep\nalpha = 1;\nmid\nbeta = 2;\ntail\n";
        let new = "keep\nalpha =  1;\nmid\ngamma = 3;\ntail\n";
        let (card, window) = cx.add_window_view(|_, cx| DiffCard::new(cx, "text", old, new));
        window.run_until_parked();
        card.update(window, |card, cx| {
            // Line 1/2 differ only in alignment; 3/4 rename a constant.
            assert_eq!(card.whitespace_changed_pairs(), 1);
            assert!(card.whitespace_pairs.contains(&1) && card.whitespace_pairs.contains(&2));
            let rows = card.rows.as_ref().unwrap().clone();
            assert!(rows.iter().any(|row| matches!(row, Row::Whitespace(1, 2))));
            assert_eq!(card.stats(), (2, 2));
            card.set_hide_whitespace(true, cx);
        });
        window.run_until_parked();
        card.update(window, |card, cx| {
            let rows = card.rows.as_ref().unwrap().clone();
            assert!(
                rows.iter()
                    .all(|row| !matches!(row, Row::Line(1) | Row::Line(2)))
            );
            assert!(rows.iter().any(|row| matches!(row, Row::Line(3))));
            // Formatting is retained as context, so hiding it never silently
            // drops source lines or loses the old/new origins.
            assert!(rows.iter().any(|row| matches!(row, Row::Whitespace(1, 2))));
            // Raw stats keep counting the whitespace churn.
            assert_eq!(card.stats(), (2, 2));
            card.set_hide_whitespace(false, cx);
        });
        window.run_until_parked();
        let pill = window.debug_bounds("diff-card-whitespace-toggle").unwrap();
        window.simulate_click(pill.center(), gpui::Modifiers::default());
        window.run_until_parked();
        card.read_with(window, |card, _| {
            assert!(card.hide_whitespace_enabled());
            // The pill click must not fold the diff body.
            assert!(card.body_visible);
        });
    }
}
