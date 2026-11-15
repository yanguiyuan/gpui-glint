//! Read-only HTML prose rendered with GPUI, without a browser or remote resources.
use std::cell::RefCell;

use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, ScrollHandle,
    StatefulInteractiveElement, Styled, Window, div, point, px, rgb,
};
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, states::RawKind,
};

use super::{Scrollbar, ScrollbarState};
use crate::theme::ThemePreset;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Paragraph {
    text: String,
    heading: bool,
}

#[derive(Default)]
struct Prose {
    paragraphs: Vec<Paragraph>,
    current: Paragraph,
    ignored: Vec<String>,
    space: bool,
}
impl Prose {
    fn flush(&mut self) {
        self.current.text = self.current.text.trim().to_owned();
        if !self.current.text.is_empty() {
            self.paragraphs.push(std::mem::take(&mut self.current));
        }
        self.current = Paragraph::default();
        self.space = false;
    }
    fn text(&mut self, text: &str) {
        for ch in text.chars() {
            if ch.is_whitespace() {
                self.space = !self.current.text.is_empty();
            } else {
                if self.space && !self.current.text.ends_with('\n') {
                    self.current.text.push(' ');
                }
                self.space = false;
                self.current.text.push(ch);
            }
        }
    }
}
struct ProseSink(RefCell<Prose>);
impl TokenSink for ProseSink {
    type Handle = ();
    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
        let mut prose = self.0.borrow_mut();
        match token {
            Token::CharacterTokens(text) if prose.ignored.is_empty() => prose.text(&text),
            Token::TagToken(tag) => {
                let name = tag.name.as_ref();
                let start = tag.kind == TagKind::StartTag;
                if matches!(
                    name,
                    "head"
                        | "script"
                        | "style"
                        | "template"
                        | "iframe"
                        | "object"
                        | "svg"
                        | "noscript"
                ) {
                    if start {
                        prose.ignored.push(name.to_owned());
                        if name == "script" {
                            return TokenSinkResult::RawData(RawKind::ScriptData);
                        }
                        if matches!(name, "style" | "iframe" | "noscript") {
                            return TokenSinkResult::RawData(RawKind::Rawtext);
                        }
                    } else if let Some(index) =
                        prose.ignored.iter().rposition(|value| value == name)
                    {
                        prose.ignored.truncate(index);
                    }
                } else if prose.ignored.is_empty() {
                    match name {
                        "p" | "div" | "section" | "article" | "blockquote" | "li" | "pre"
                        | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                            prose.flush();
                            prose.current.heading =
                                start && matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
                        }
                        "br" if start => {
                            prose.current.text.push('\n');
                            prose.space = false;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}
fn parse(html: &str) -> Vec<Paragraph> {
    let input = BufferQueue::default();
    input.push_back(html.into());
    let tokenizer = Tokenizer::new(
        ProseSink(RefCell::new(Prose::default())),
        Default::default(),
    );
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    let mut prose = tokenizer.sink.0.into_inner();
    prose.flush();
    prose.paragraphs
}

/// A lightweight, theme-aware reader for HTML chapters, especially `<p>` prose.
/// HTML entities and `<br>` are decoded; headings are retained. Inline tags keep
/// their text. Scripts, styles, images and embedded resources are never executed
/// or fetched. Source CSS is ignored so every chapter follows the host theme.
pub struct HtmlReader {
    paragraphs: Vec<Paragraph>,
    theme: ThemePreset,
    font_size: f32,
    scroll: ScrollHandle,
    scrollbar: ScrollbarState,
}
impl HtmlReader {
    pub fn new(html: impl AsRef<str>, _cx: &mut Context<Self>) -> Self {
        Self {
            paragraphs: parse(html.as_ref()),
            theme: ThemePreset::default(),
            font_size: 17.,
            scroll: ScrollHandle::new(),
            scrollbar: ScrollbarState::default(),
        }
    }
    /// Replace a chapter and return to its beginning.
    pub fn set_html(&mut self, html: impl AsRef<str>, cx: &mut Context<Self>) {
        self.paragraphs = parse(html.as_ref());
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }
    /// Treat plain text as paragraphs without interpreting angle brackets as HTML.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.paragraphs = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| Paragraph {
                text: line.trim().to_owned(),
                heading: false,
            })
            .collect();
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }
    pub fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        if size.is_finite() {
            self.font_size = size.clamp(12., 32.);
            cx.notify();
        }
    }
    pub fn is_empty(&self) -> bool {
        self.paragraphs.is_empty()
    }
    pub fn paragraph_count(&self) -> usize {
        self.paragraphs.len()
    }
}
impl Render for HtmlReader {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let palette = self.theme.palette();
        let mut article = div()
            .w_full()
            .max_w(px(800.))
            .mx_auto()
            .px(px(32.))
            .py(px(28.))
            .flex()
            .flex_col()
            .gap(px(self.font_size))
            .text_size(px(self.font_size))
            .line_height(px(self.font_size * 1.9));
        for (index, paragraph) in self.paragraphs.iter().enumerate() {
            let text = if paragraph.heading {
                paragraph.text.clone()
            } else {
                format!("　　{}", paragraph.text)
            };
            let mut row = div()
                .w_full()
                .min_w_0()
                .debug_selector(move || format!("html-paragraph-{index}"));
            if paragraph.heading {
                row = row
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(self.font_size + 3.));
            }
            article = article.child(row.child(text));
        }
        if self.is_empty() {
            article = article.child(
                div()
                    .text_color(rgb(palette.text_muted))
                    .child("本章暂无正文"),
            );
        }
        div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .bg(rgb(palette.canvas))
            .text_color(rgb(palette.text))
            .debug_selector(|| "html-reader".into())
            .child(
                div()
                    .id("html-reader-scroll")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(article),
            )
            .child(
                Scrollbar::vertical(&self.scroll, &self.scrollbar)
                    .theme(self.theme)
                    .render(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paragraphs_entities_breaks_and_imperfect_html_become_readable_prose() {
        let blocks = parse(
            "<h2>章节 &amp; 标题</h2><p style='color:white'>第一段<b>加粗</b>&nbsp;文字<br>下一行</p><p>第二段<p>第三段 &#x4e2d;&#25991;",
        );
        assert_eq!(
            blocks.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(),
            vec![
                "章节 & 标题",
                "第一段加粗 文字\n下一行",
                "第二段",
                "第三段 中文"
            ]
        );
        assert!(blocks[0].heading);
        assert!(!blocks[1].heading);
    }
    #[test]
    fn metadata_scripts_and_embeds_do_not_leak_into_the_chapter() {
        let blocks = parse(
            "<head><title>元数据</title><style>p{color:red}</style></head><p>正文</p><script>if(a<b){evil()}</script><template><p>隐藏</p></template><iframe src='https://example.com'>外部</iframe><img src='https://example.com/a'><p>结尾 &lt;p&gt;</p>",
        );
        assert_eq!(
            blocks.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(),
            vec!["正文", "结尾 <p>"]
        );
    }
    #[gpui::test]
    fn changing_chapters_resets_scroll_and_theme_preserves_content(cx: &mut gpui::TestAppContext) {
        let html = "<p>正文</p>".repeat(80);
        let (reader, window) = cx.add_window_view(|_, cx| HtmlReader::new(&html, cx));
        window.run_until_parked();
        reader.update(window, |reader, cx| {
            reader.scroll.set_offset(point(px(0.), px(-200.)));
            reader.set_theme(ThemePreset::Moonstone, cx);
            assert_eq!(reader.paragraph_count(), 80);
            reader.set_html("<p>新一章</p>", cx);
            assert_eq!(reader.paragraph_count(), 1);
            assert_eq!(reader.scroll.offset().y, px(0.));
        });
    }
}
