//! Lossless block boundaries and CommonMark/GFM parsing shared by the editor.
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag};

pub(super) fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_MATH
        | Options::ENABLE_DEFINITION_LIST
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentBlockKind {
    Paragraph,
    Heading(u8),
    Quote,
    List,
    Code,
    Table,
    Divider,
    Other,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentBlock {
    pub id: u64,
    pub markdown: String,
}
impl DocumentBlock {
    pub fn kind(&self) -> DocumentBlockKind {
        kind(&self.markdown)
    }
}
pub(super) fn kind(source: &str) -> DocumentBlockKind {
    match Parser::new_ext(source, options()).next() {
        Some(Event::Start(Tag::Heading { level, .. })) => DocumentBlockKind::Heading(level as u8),
        Some(Event::Start(Tag::BlockQuote(_))) => DocumentBlockKind::Quote,
        Some(Event::Start(Tag::List(_))) => DocumentBlockKind::List,
        Some(Event::Start(Tag::CodeBlock(_))) => DocumentBlockKind::Code,
        Some(Event::Start(Tag::Table(_))) => DocumentBlockKind::Table,
        Some(Event::Rule) => DocumentBlockKind::Divider,
        Some(Event::Start(Tag::Paragraph)) | None => DocumentBlockKind::Paragraph,
        _ => DocumentBlockKind::Other,
    }
}
/// Preserve original source inside every block, including references and HTML.
pub(super) fn split(source: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (event, range) in Parser::new_ext(source, options()).into_offset_iter() {
        match event {
            Event::Start(_) => {
                if depth == 0 {
                    start = range.start;
                }
                depth += 1;
            }
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    spans.push(start..range.end);
                }
            }
            _ if depth == 0 => spans.push(range),
            _ => {}
        }
    }
    let mut blocks = Vec::new();
    let mut cursor = 0;
    for span in spans {
        if span.start > cursor && !source[cursor..span.start].trim().is_empty() {
            blocks.push(source[cursor..span.start].trim().to_owned());
        }
        blocks.push(source[span.clone()].trim_end_matches('\n').to_owned());
        cursor = span.end;
    }
    if !source[cursor..].trim().is_empty() {
        blocks.push(source[cursor..].trim().to_owned());
    }
    if blocks.is_empty() {
        blocks.push(String::new());
    }
    blocks
}
/// Extract only link definitions, so global references resolve without repeating footnotes.
pub(super) fn references(source: &str) -> String {
    let parser = Parser::new_ext(source, options());
    let mut spans = parser
        .reference_definitions()
        .iter()
        .map(|(_, def)| def.span.clone())
        .collect::<Vec<_>>();
    spans.sort_by_key(|span| span.start);
    spans
        .into_iter()
        .map(|span| source[span].trim_end().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}
#[derive(Clone)]
pub(super) enum Node {
    Element(Tag<'static>, Vec<Node>),
    Text(String),
    Code(String),
    Html(String),
    Break,
    Rule,
    Task(bool),
    Math(String),
    Footnote(String),
}
pub(super) fn nodes(source: &str) -> Vec<Node> {
    let mut stack: Vec<(Tag<'static>, Vec<Node>)> = Vec::new();
    let mut root = Vec::new();
    for event in Parser::new_ext(source, options()) {
        let node = match event {
            Event::Start(tag) => {
                stack.push((tag.into_static(), Vec::new()));
                continue;
            }
            Event::End(_) => {
                let (tag, children) = stack.pop().unwrap();
                Node::Element(tag, children)
            }
            Event::Text(text) => Node::Text(text.to_string()),
            Event::Code(text) => Node::Code(text.to_string()),
            Event::Html(text) | Event::InlineHtml(text) => Node::Html(text.to_string()),
            Event::SoftBreak => Node::Text("\n".into()),
            Event::HardBreak => Node::Break,
            Event::Rule => Node::Rule,
            Event::TaskListMarker(checked) => Node::Task(checked),
            Event::InlineMath(text) | Event::DisplayMath(text) => Node::Math(text.to_string()),
            Event::FootnoteReference(text) => Node::Footnote(text.to_string()),
        };
        if let Some((_, children)) = stack.last_mut() {
            children.push(node);
        } else {
            root.push(node);
        }
    }
    root
}
pub(super) fn code(source: &str) -> (String, String) {
    let mut language = String::new();
    let mut content = String::new();
    for event in Parser::new_ext(source, options()) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                language = info.split_whitespace().next().unwrap_or("").to_owned()
            }
            Event::Text(text) => content.push_str(&text),
            _ => {}
        }
    }
    if content.ends_with('\n') {
        content.pop();
    }
    (language, content)
}
#[derive(Clone, Debug)]
pub(super) struct MarkdownTable {
    pub cells: Vec<Vec<String>>,
    pub alignment: Vec<Alignment>,
}
pub(super) fn table(source: &str) -> MarkdownTable {
    let mut cells = Vec::new();
    let mut row = Vec::new();
    let mut alignment = Vec::new();
    for (event, range) in Parser::new_ext(source, options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Table(a)) => alignment = a,
            Event::Start(Tag::TableCell) => row.push(source[range].trim().to_owned()),
            Event::End(pulldown_cmark::TagEnd::TableHead | pulldown_cmark::TagEnd::TableRow) => {
                cells.push(std::mem::take(&mut row))
            }
            _ => {}
        }
    }
    MarkdownTable { cells, alignment }
}
impl MarkdownTable {
    pub fn markdown(&self) -> String {
        let mut output = String::new();
        for (index, row) in self.cells.iter().enumerate() {
            output.push_str("| ");
            output.push_str(
                &row.iter()
                    .map(|cell| escape_cell(cell))
                    .collect::<Vec<_>>()
                    .join(" | "),
            );
            output.push_str(" |\n");
            if index == 0 {
                output.push_str("| ");
                output.push_str(
                    &self
                        .alignment
                        .iter()
                        .map(|a| match a {
                            Alignment::Left => ":---",
                            Alignment::Center => ":---:",
                            Alignment::Right => "---:",
                            Alignment::None => "---",
                        })
                        .collect::<Vec<_>>()
                        .join(" | "),
                );
                output.push_str(" |\n");
            }
        }
        output.trim_end().to_owned()
    }
}
pub(super) fn fence(language: &str, content: &str) -> String {
    let longest = content.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}{language}\n{content}\n{fence}")
}

fn escape_cell(cell: &str) -> String {
    let mut output = String::new();
    let mut slashes = 0;
    for c in cell.chars() {
        if c == '|' && slashes % 2 == 0 {
            output.push('\\');
        }
        if c == '\n' {
            output.push_str("<br>");
        } else {
            output.push(c);
        }
        if c == '\\' {
            slashes += 1;
        } else {
            slashes = 0;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boundaries_keep_nested_lists_and_quotes() {
        let source =
            "# 标题\n\n> **引用**\n>\n> - 嵌套\n\n- 第一\n  - 子项\n\n```go\nx := `a`\n```\n\n---";
        let blocks = split(source);
        assert_eq!(blocks.len(), 5);
        assert_eq!(kind(&blocks[1]), DocumentBlockKind::Quote);
        assert_eq!(kind(&blocks[2]), DocumentBlockKind::List);
    }
    #[test]
    fn table_roundtrip_keeps_alignment_and_inline_source() {
        let source = "| 名称 | 值 |\n| :--- | ---: |\n| **中文** | `42` |";
        let t = table(source);
        assert_eq!(t.cells[1][0], "**中文**");
        assert_eq!(table(&t.markdown()).cells, t.cells);
    }
    #[test]
    fn code_fences_do_not_break_on_embedded_fences() {
        let s = fence("md", "```\nx\n```");
        assert_eq!(code(&s).1, "```\nx\n```");
    }
    #[test]
    fn escaped_pipes_survive_repeated_cell_edits() {
        let source = "| A | B |\n| --- | --- |\n| a\\|b | 中文 |";
        let first = table(source);
        let second = table(&first.markdown());
        assert_eq!(first.cells, second.cells);
        assert_eq!(second.cells, table(&second.markdown()).cells);
    }
    #[test]
    fn gfm_extensions_parse_nested_inline_nodes() {
        let n = nodes("- [x] **粗体 *斜体*** 与 ~~删除~~、`代码`、[链接](https://example.com)");
        assert!(matches!(&n[0], Node::Element(Tag::List(_), _)));
        let n = nodes("![图片](cover.png)");
        assert!(
            matches!(&n[0],Node::Element(Tag::Paragraph,children) if matches!(&children[0],Node::Element(Tag::Image{..},_)))
        );
    }
    #[test]
    fn global_link_definitions_resolve_without_repeating_footnotes() {
        let definitions =
            references("[链接][a]\n\n[a]:https://example.com \"标题\"\n\n[^note]: 脚注");
        assert!(!definitions.contains("脚注"));
        let parsed = nodes(&format!("[链接][a]\n\n{definitions}"));
        assert!(
            matches!(&parsed[0], Node::Element(Tag::Paragraph, children) if matches!(&children[0], Node::Element(Tag::Link { dest_url, .. }, _) if dest_url.as_ref() == "https://example.com"))
        );
    }
    #[test]
    fn source_references_are_preserved() {
        assert!(
            split("[链接][a]\n\n[a]: https://example.com")
                .join("\n\n")
                .contains("[a]: https://example.com")
        );
    }
}
