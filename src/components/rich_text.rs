//! Semantic inline text: editing coordinates never contain Markdown delimiters.
use gpui::{
    Font, FontStyle, FontWeight, Hsla, StrikethroughStyle, TextRun, UnderlineStyle, px, rgb,
};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Format {
    Bold,
    Italic,
    Strike,
    Code,
    Link(String),
}
#[derive(Clone, Debug)]
struct Mark {
    range: Range<usize>,
    format: Format,
}
#[derive(Clone, Debug, Default)]
pub(super) struct RichText {
    pub text: String,
    marks: Vec<Mark>,
}
impl RichText {
    pub fn parse(source: &str) -> Self {
        let mut result = Self::default();
        let mut stack = Vec::new();
        let mut html_stack: Vec<(usize, Format)> = Vec::new();
        for event in Parser::new_ext(source, super::document_markdown::options()) {
            match event {
                Event::Start(tag) => {
                    let format = match tag {
                        Tag::Strong => Some(Format::Bold),
                        Tag::Emphasis => Some(Format::Italic),
                        Tag::Strikethrough => Some(Format::Strike),
                        Tag::Link { dest_url, .. } => Some(Format::Link(dest_url.to_string())),
                        _ => None,
                    };
                    stack.push((result.text.len(), format));
                }
                Event::End(_) => {
                    if let Some((start, Some(format))) = stack.pop() {
                        result.marks.push(Mark {
                            range: start..result.text.len(),
                            format,
                        });
                    }
                }
                Event::Text(text) => result.text.push_str(&text),
                Event::Code(text) => {
                    let start = result.text.len();
                    result.text.push_str(&text);
                    result.marks.push(Mark {
                        range: start..result.text.len(),
                        format: Format::Code,
                    });
                }
                Event::SoftBreak | Event::HardBreak => result.text.push('\n'),
                Event::Html(text) | Event::InlineHtml(text) => {
                    if matches!(text.trim(), "<br>" | "<br/>" | "<br />") {
                        result.text.push('\n');
                    } else if let Some(format) = match text.trim() {
                        "<strong>" => Some(Format::Bold),
                        "<em>" => Some(Format::Italic),
                        "<del>" => Some(Format::Strike),
                        "<code>" => Some(Format::Code),
                        _ => text
                            .trim()
                            .strip_prefix("<a href=\"")
                            .and_then(|v| v.strip_suffix("\">"))
                            .map(|v| {
                                Format::Link(
                                    v.replace("&quot;", "\"")
                                        .replace("&lt;", "<")
                                        .replace("&amp;", "&"),
                                )
                            }),
                    } {
                        html_stack.push((result.text.len(), format));
                    } else if matches!(
                        text.trim(),
                        "</strong>" | "</em>" | "</del>" | "</code>" | "</a>"
                    ) {
                        if let Some((start, format)) = html_stack.pop() {
                            result.marks.push(Mark {
                                range: start..result.text.len(),
                                format,
                            });
                        }
                    } else {
                        result.text.push_str(&text);
                    }
                }
                Event::InlineMath(text) | Event::DisplayMath(text) => result.text.push_str(&text),
                Event::FootnoteReference(label) => result.text.push_str(&format!("[^{label}]")),
                _ => {}
            }
        }
        result
    }
    pub fn update_text(&mut self, next: &str) {
        let mut prefix = 0;
        for (a, b) in self.text.chars().zip(next.chars()) {
            if a != b {
                break;
            }
            prefix += a.len_utf8();
        }
        let mut suffix = 0;
        for (a, b) in self.text[prefix..]
            .chars()
            .rev()
            .zip(next[prefix..].chars().rev())
        {
            if a != b {
                break;
            }
            suffix += a.len_utf8();
        }
        let end = self.text.len() - suffix;
        let added = next.len() - prefix - suffix;
        let shift = added as isize - (end - prefix) as isize;
        for mark in &mut self.marks {
            mark.range.start = if mark.range.start <= prefix {
                mark.range.start
            } else if mark.range.start >= end {
                mark.range.start.saturating_add_signed(shift)
            } else {
                prefix
            };
            mark.range.end = if mark.range.end < prefix {
                mark.range.end
            } else if mark.range.end > end {
                mark.range.end.saturating_add_signed(shift)
            } else {
                prefix + added
            };
        }
        self.marks.retain(|m| !m.range.is_empty());
        self.text = next.to_owned();
    }
    pub fn toggle(&mut self, range: Range<usize>, format: Option<Format>) {
        if range.is_empty() {
            return;
        }
        let remove = format.as_ref().is_some_and(|f| {
            self.text[range.clone()].char_indices().all(|(i, _)| {
                self.marks
                    .iter()
                    .any(|m| m.format == *f && m.range.contains(&(range.start + i)))
            })
        });
        let mut marks = Vec::new();
        for mark in self.marks.drain(..) {
            let same = format.as_ref().is_none_or(|f| {
                mark.format == *f || matches!((f, &mark.format), (Format::Link(_), Format::Link(_)))
            });
            if !same || mark.range.end <= range.start || mark.range.start >= range.end {
                marks.push(mark);
                continue;
            }
            if mark.range.start < range.start {
                marks.push(Mark {
                    range: mark.range.start..range.start,
                    format: mark.format.clone(),
                });
            }
            if mark.range.end > range.end {
                marks.push(Mark {
                    range: range.end..mark.range.end,
                    format: mark.format,
                });
            }
        }
        if !remove && let Some(format) = format {
            marks.push(Mark { range, format });
        }
        self.marks = marks;
    }
    fn formats(&self, offset: usize) -> Vec<Format> {
        let mut formats = Vec::new();
        for m in &self.marks {
            if m.range.contains(&offset) && !formats.contains(&m.format) {
                formats.push(m.format.clone());
            }
        }
        formats.sort_by_key(|f| match f {
            Format::Link(_) => 0,
            Format::Bold => 1,
            Format::Italic => 2,
            Format::Strike => 3,
            Format::Code => 4,
        });
        formats
    }
    pub fn code_ranges(&self, range: Range<usize>) -> Vec<Range<usize>> {
        self.marks
            .iter()
            .filter(|mark| mark.format == Format::Code)
            .filter_map(|mark| {
                let start = mark.range.start.max(range.start);
                let end = mark.range.end.min(range.end);
                (start < end).then_some(start..end)
            })
            .collect()
    }
    pub fn slice_markdown(&self, range: Range<usize>) -> String {
        self.slice(range).markdown()
    }
    pub fn join_markdown(&self, end: usize, other: &Self, start: usize) -> String {
        let mut joined = self.slice(0..end);
        let suffix = other.slice(start..other.text.len());
        let offset = joined.text.len();
        joined.text.push_str(&suffix.text);
        joined
            .marks
            .extend(suffix.marks.into_iter().map(|mut mark| {
                mark.range = mark.range.start + offset..mark.range.end + offset;
                mark
            }));
        joined.markdown()
    }
    fn slice(&self, range: Range<usize>) -> Self {
        let mut sliced = Self {
            text: self.text[range.clone()].to_owned(),
            marks: Vec::new(),
        };
        for mark in &self.marks {
            let start = mark.range.start.max(range.start);
            let end = mark.range.end.min(range.end);
            if start < end {
                sliced.marks.push(Mark {
                    range: start - range.start..end - range.start,
                    format: mark.format.clone(),
                });
            }
        }
        sliced
    }
    pub fn markdown(&self) -> String {
        let fence = "`".repeat(
            self.text
                .split(|c| c != '`')
                .map(str::len)
                .max()
                .unwrap_or(0)
                + 1,
        );
        let open = |f: &Format| match f {
            Format::Bold => "**".to_owned(),
            Format::Italic => "*".to_owned(),
            Format::Strike => "~~".to_owned(),
            Format::Code => fence.clone(),
            Format::Link(_) => "[".to_owned(),
        };
        let close = |f: &Format| match f {
            Format::Link(url) => format!(
                "]({})",
                url.replace('\\', "\\\\")
                    .replace('(', "\\(")
                    .replace(')', "\\)")
            ),
            _ => open(f),
        };
        let mut out = String::new();
        let mut current = Vec::new();
        for (offset, ch) in self.text.char_indices() {
            let next = self.formats(offset);
            let common = current
                .iter()
                .zip(&next)
                .take_while(|(a, b)| a == b)
                .count();
            for format in current[common..].iter().rev() {
                out.push_str(&close(format));
            }
            for format in &next[common..] {
                out.push_str(&open(format));
            }
            if !next.contains(&Format::Code)
                && matches!(
                    ch,
                    '\\' | '*' | '_' | '~' | '`' | '[' | ']' | '<' | '>' | '#' | '!'
                )
            {
                out.push('\\');
            }
            if ch == '\n' {
                out.push_str("  \n");
            } else {
                out.push(ch);
            }
            current = next;
        }
        for format in current.iter().rev() {
            out.push_str(&close(format));
        }
        let parsed = Self::parse(&out);
        if parsed.text == self.text
            && self
                .text
                .char_indices()
                .all(|(i, _)| parsed.formats(i) == self.formats(i))
        {
            return out;
        }
        // Crossing or intraword marks cannot always use Markdown delimiters. Inline
        // HTML is part of CommonMark and preserves these ranges without ambiguity.
        let tag = |format: &Format, closing: bool| -> String {
            let name = match format {
                Format::Bold => "strong",
                Format::Italic => "em",
                Format::Strike => "del",
                Format::Code => "code",
                Format::Link(_) => "a",
            };
            if closing {
                format!("</{name}>")
            } else if let Format::Link(url) = format {
                format!(
                    "<a href=\"{}\">",
                    url.replace('&', "&amp;")
                        .replace('"', "&quot;")
                        .replace('<', "&lt;")
                )
            } else {
                format!("<{name}>")
            }
        };
        let mut out = String::new();
        let mut current = Vec::new();
        for (i, ch) in self.text.char_indices() {
            let next = self.formats(i);
            let common = current
                .iter()
                .zip(&next)
                .take_while(|(a, b)| a == b)
                .count();
            for f in current[common..].iter().rev() {
                out.push_str(&tag(f, true));
            }
            for f in &next[common..] {
                out.push_str(&tag(f, false));
            }
            if ch == '\n' {
                out.push_str("<br>");
            } else if ch.is_ascii_punctuation() {
                out.push_str(&format!("&#{};", ch as u32));
            } else {
                out.push(ch);
            }
            current = next;
        }
        for f in current.iter().rev() {
            out.push_str(&tag(f, true));
        }
        out
    }
    pub fn runs(&self, range: Range<usize>, base: &Font, color: Hsla) -> Vec<TextRun> {
        let mut cuts = vec![range.start, range.end];
        for mark in &self.marks {
            if mark.range.start > range.start && mark.range.start < range.end {
                cuts.push(mark.range.start);
            }
            if mark.range.end > range.start && mark.range.end < range.end {
                cuts.push(mark.range.end);
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        cuts.windows(2)
            .map(|cut| {
                let mut run = TextRun {
                    letter_spacing: None,
                    len: cut[1] - cut[0],
                    font: base.clone(),
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                for format in self.formats(cut[0]) {
                    match format {
                        Format::Bold => run.font.weight = FontWeight::BOLD,
                        Format::Italic => run.font.style = FontStyle::Italic,
                        Format::Strike => {
                            run.strikethrough = Some(StrikethroughStyle {
                                color: None,
                                thickness: px(1.),
                            })
                        }
                        Format::Code => {
                            run.font.family = "Menlo".into();
                        }
                        Format::Link(_) => {
                            run.color = gpui::rgb_to_hsla(rgb(0x007aff));
                            run.underline = Some(UnderlineStyle {
                                color: None,
                                thickness: px(1.),
                                wavy: false,
                            });
                        }
                    }
                }
                run
            })
            .collect()
    }
}

#[derive(Clone)]
pub(super) struct Leaf {
    pub range: Range<usize>,
    pub source: String,
}
/// Inline source spans inside structural Markdown blocks, excluding block markers.
pub(super) fn leaves(source: &str) -> Vec<Leaf> {
    let mut result = Vec::new();
    let mut pending: Option<Range<usize>> = None;
    let flush = |pending: &mut Option<Range<usize>>, result: &mut Vec<Leaf>| {
        if let Some(range) = pending.take() {
            result.push(Leaf {
                source: source[range.clone()]
                    .split('\n')
                    .enumerate()
                    .map(|(i, line)| {
                        if i == 0 {
                            return line;
                        }
                        let mut rest = line;
                        while let Some(next) = rest.trim_start().strip_prefix('>') {
                            rest = next.strip_prefix(' ').unwrap_or(next);
                        }
                        rest
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                range,
            });
        }
    };
    let mut inline_depth = 0;
    let mut excluded_depth = 0;
    for (event, range) in
        Parser::new_ext(source, super::document_markdown::options()).into_offset_iter()
    {
        match &event {
            Event::Start(Tag::CodeBlock(_) | Tag::Image { .. } | Tag::FootnoteDefinition(_)) => {
                flush(&mut pending, &mut result);
                excluded_depth += 1;
                continue;
            }
            Event::End(TagEnd::CodeBlock | TagEnd::Image | TagEnd::FootnoteDefinition) => {
                excluded_depth -= 1;
                continue;
            }
            _ if excluded_depth > 0 => continue,
            _ => {}
        }
        let inline = match &event {
            Event::Start(Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. }) => {
                inline_depth += 1;
                true
            }
            Event::End(
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link,
            ) => {
                inline_depth -= 1;
                true
            }
            Event::Text(_)
            | Event::Code(_)
            | Event::SoftBreak
            | Event::HardBreak
            | Event::InlineHtml(_)
            | Event::InlineMath(_)
            | Event::FootnoteReference(_) => true,
            _ => false,
        };
        if inline {
            if let Some(span) = &mut pending {
                span.start = span.start.min(range.start);
                span.end = span.end.max(range.end);
            } else {
                pending = Some(range);
            }
        } else if inline_depth == 0 {
            flush(&mut pending, &mut result);
        }
    }
    flush(&mut pending, &mut result);
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typing_preserves_inline_formats_and_unicode() {
        let mut t = RichText::parse("**中文** 与 [链接](https://example.com)");
        t.update_text("中英文 与 链接");
        let parsed = RichText::parse(&t.markdown());
        assert_eq!(parsed.text, t.text);
        assert!(parsed.formats(0).contains(&Format::Bold));
        assert!(
            parsed
                .formats("中英文 与 ".len())
                .contains(&Format::Link("https://example.com".into()))
        );
    }
    #[test]
    fn selection_format_and_clear_roundtrip() {
        let mut t = RichText::parse("中文 ordinary");
        t.toggle(0..6, Some(Format::Bold));
        t.toggle(0..6, Some(Format::Italic));
        assert_eq!(RichText::parse(&t.markdown()).text, t.text);
        t.toggle(0..6, None);
        assert!(t.formats(0).is_empty());
    }
    #[test]
    fn partial_and_overlapping_marks_survive_markdown_roundtrip() {
        for a in 0..5 {
            for b in a + 1..=5 {
                for c in 0..5 {
                    for d in c + 1..=5 {
                        let mut t = RichText::parse("abcde");
                        t.toggle(a..b, Some(Format::Bold));
                        t.toggle(c..d, Some(Format::Italic));
                        let source = t.markdown();
                        let parsed = RichText::parse(&source);
                        assert_eq!(parsed.text, t.text, "{source}");
                        for offset in 0..5 {
                            assert_eq!(
                                parsed.formats(offset),
                                t.formats(offset),
                                "{source} at {offset}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn whitespace_code_links_and_crossing_marks_roundtrip() {
        let mut t = RichText::parse("first  \n中文 `tick` & end");
        t.toggle(0..t.text.len(), Some(Format::Italic));
        t.toggle(2..9, Some(Format::Bold));
        t.toggle(
            0..5,
            Some(Format::Link("https://example.com/?a=1&b=2".into())),
        );
        let source = t.markdown();
        let parsed = RichText::parse(&source);
        assert_eq!(parsed.text, t.text, "{source}");
        for (i, _) in t.text.char_indices() {
            assert_eq!(parsed.formats(i), t.formats(i), "{source} at {i}");
        }
    }
    #[test]
    fn quoted_continuations_keep_visible_newlines() {
        let source = "> first  \n> next **bold**";
        let leaves = leaves(source);
        assert_eq!(leaves.len(), 1);
        assert_eq!(RichText::parse(&leaves[0].source).text, "first\nnext bold");
        assert_eq!(&source[leaves[0].range.clone()], "first  \n> next **bold**");
    }
    #[test]
    fn structural_markers_are_outside_editable_spans() {
        let source = "## 标题 **加粗**\n\n> 引用\n>\n> - 子项\n\n- [x] 任务";
        let l = leaves(source);
        assert_eq!(
            l.iter().map(|x| x.source.as_str()).collect::<Vec<_>>(),
            ["标题 **加粗**", "引用", "子项", "任务"]
        );
    }
}
