//! Read-only command logs: retain ANSI styling without exposing control sequences.
use alacritty_terminal::vte::ansi::{Attr, Color, Handler, LineClearMode, NamedColor, Processor};
use gpui::{FontStyle, FontWeight, SharedString, StyledText, TextRun, UnderlineStyle, px, rgb};
use gpui_glint::theme::ThemePreset;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Style {
    foreground: Option<Color>,
    background: Option<Color>,
    bold: bool,
    italic: bool,
    underline: bool,
    reverse: bool,
}

#[derive(Default)]
struct Log {
    lines: Vec<Vec<(char, Style)>>,
    line: Vec<(char, Style)>,
    cursor: usize,
    style: Style,
}

impl Handler for Log {
    fn input(&mut self, c: char) {
        if self.cursor == self.line.len() {
            self.line.push((c, self.style));
        } else {
            self.line[self.cursor] = (c, self.style);
        }
        self.cursor += 1;
    }

    fn carriage_return(&mut self) {
        self.cursor = 0;
    }
    fn linefeed(&mut self) {
        self.lines.push(std::mem::take(&mut self.line));
        self.cursor = 0;
    }
    fn newline(&mut self) {
        self.linefeed();
    }
    fn backspace(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }
    fn put_tab(&mut self, count: u16) {
        for _ in 0..count {
            let spaces = 8 - self.cursor % 8;
            for _ in 0..spaces {
                self.input(' ');
            }
        }
    }
    fn clear_line(&mut self, mode: LineClearMode) {
        match mode {
            LineClearMode::Right => self.line.truncate(self.cursor),
            LineClearMode::Left => {
                for cell in self.line.iter_mut().take(self.cursor + 1) {
                    *cell = (' ', self.style);
                }
            }
            LineClearMode::All => self.line.fill((' ', self.style)),
        }
    }
    fn terminal_attribute(&mut self, attr: Attr) {
        match attr {
            Attr::Reset => self.style = Style::default(),
            Attr::Foreground(Color::Named(NamedColor::Foreground)) => self.style.foreground = None,
            Attr::Background(Color::Named(NamedColor::Background)) => self.style.background = None,
            Attr::Foreground(color) => self.style.foreground = Some(color),
            Attr::Background(color) => self.style.background = Some(color),
            Attr::Bold => self.style.bold = true,
            Attr::CancelBold | Attr::CancelBoldDim => self.style.bold = false,
            Attr::Italic => self.style.italic = true,
            Attr::CancelItalic => self.style.italic = false,
            Attr::Underline
            | Attr::DoubleUnderline
            | Attr::Undercurl
            | Attr::DottedUnderline
            | Attr::DashedUnderline => self.style.underline = true,
            Attr::CancelUnderline => self.style.underline = false,
            Attr::Reverse => self.style.reverse = true,
            Attr::CancelReverse => self.style.reverse = false,
            _ => {}
        }
    }
}

#[derive(Clone)]
pub(super) struct Output {
    text: SharedString,
    runs: Vec<TextRun>,
}

impl Output {
    pub fn parse(source: &str, theme: ThemePreset) -> Self {
        let mut log = Log::default();
        let mut parser: Processor = Processor::new();
        parser.advance(&mut log, source.as_bytes());
        log.lines.push(log.line);
        let mut text = String::new();
        let mut spans: Vec<(Style, usize)> = Vec::new();
        for (index, line) in log.lines.into_iter().enumerate() {
            if index > 0 {
                text.push('\n');
                if let Some((_, len)) = spans.last_mut() {
                    *len += 1;
                } else {
                    spans.push((Style::default(), 1));
                }
            }
            for (c, style) in line {
                text.push(c);
                if let Some((_, len)) = spans.last_mut().filter(|(last, _)| *last == style) {
                    *len += c.len_utf8();
                } else {
                    spans.push((style, c.len_utf8()));
                }
            }
        }
        let p = theme.palette();
        let runs = spans
            .into_iter()
            .map(|(style, len)| {
                let mut font = gpui::font("monospace");
                if style.bold {
                    font.weight = FontWeight::BOLD;
                }
                if style.italic {
                    font.style = FontStyle::Italic;
                }
                let mut foreground = style
                    .foreground
                    .map(|color| super::terminal_ui::output_color(color, theme))
                    .unwrap_or(p.text_muted);
                let mut background = style
                    .background
                    .map(|color| super::terminal_ui::output_color(color, theme));
                if style.reverse {
                    let old = foreground;
                    foreground = background.unwrap_or(p.workspace);
                    background = Some(old);
                }
                TextRun {
                    len,
                    font,
                    color: gpui::rgb_to_hsla(rgb(foreground)),
                    background_color: background.map(|color| gpui::rgb_to_hsla(rgb(color))),
                    underline: style.underline.then_some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(gpui::rgb_to_hsla(rgb(foreground))),
                        wavy: false,
                    }),
                    strikethrough: None,
                    letter_spacing: None,
                }
            })
            .collect();
        Self {
            text: text.into(),
            runs,
        }
    }

    pub fn render(&self) -> StyledText {
        StyledText::new(self.text.clone()).with_runs(self.runs.clone())
    }
}

/// Consume a wheel event once, handing off to the conversation at the edge.
pub(super) fn scroll_guard(
    scroll: &gpui::ScrollHandle,
    line_height: f32,
) -> impl gpui::IntoElement {
    use gpui::Styled;
    let scroll = scroll.clone();
    gpui::canvas(
        |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
        move |_, hitbox: gpui::Hitbox, window, _| {
            let view = window.current_view();
            window.on_mouse_event(move |event: &gpui::ScrollWheelEvent, phase, window, cx| {
                if phase != gpui::DispatchPhase::Capture || !hitbox.should_handle_scroll(window) {
                    return;
                }
                let delta = event.delta.pixel_delta(px(line_height));
                let offset = scroll.offset();
                let next = (offset.y + delta.y).clamp(-scroll.max_offset().y, px(0.));
                if next != offset.y {
                    scroll.set_offset(gpui::point(offset.x, next));
                    cx.stop_propagation();
                    cx.notify(view);
                }
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_colors_unicode_and_reset_preserve_byte_ranges() {
        for theme in [ThemePreset::Moonstone, ThemePreset::DarkObsidian] {
            let output = Output::parse(
                "plain \x1b[1;31m错误\x1b[0m \x1b[38;5;196mred\x1b[0m \x1b[38;2;12;34;56mrgb\x1b[0m",
                theme,
            );
            assert_eq!(output.text.as_ref(), "plain 错误 red rgb");
            assert_eq!(
                output.runs.iter().map(|run| run.len).sum::<usize>(),
                output.text.len()
            );
            assert_eq!(output.runs[1].font.weight, FontWeight::BOLD);
            assert_eq!(
                output.runs[1].color,
                gpui::rgb_to_hsla(rgb(super::super::terminal_ui::output_color(
                    Color::Indexed(1),
                    theme
                )))
            );
            assert_eq!(
                output.runs.last().unwrap().color,
                gpui::rgb_to_hsla(rgb(0x0c2238))
            );
        }
    }

    #[test]
    fn progress_and_terminal_controls_do_not_leak_into_the_log() {
        let output = Output::parse(
            "step 10%\r\x1b[2Kstep 100%\r\n\x1b]0;title\x07\x1b]8;;https://example.com\x07链接\x1b]8;;\x07\x1b[31",
            ThemePreset::Moonstone,
        );
        assert_eq!(output.text.as_ref(), "step 100%\n链接");
        assert!(!output.text.contains('\x1b'));
    }
}
