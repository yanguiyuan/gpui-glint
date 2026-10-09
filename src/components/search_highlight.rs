use gpui::{HighlightStyle, Hsla, rgba};
use std::ops::Range;

/// Match without case sensitivity while retaining byte ranges in the original UTF-8 text.
pub fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let query: String = query.chars().flat_map(char::to_lowercase).collect();
    let mut folded = String::new();
    let mut original = Vec::new();
    for (start, ch) in text.char_indices() {
        let end = start + ch.len_utf8();
        for lower in ch.to_lowercase() {
            folded.push(lower);
            original.extend(std::iter::repeat_n(start..end, lower.len_utf8()));
        }
    }
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for (start, _) in folded.match_indices(&query) {
        let range = original[start].start..original[start + query.len() - 1].end;
        if let Some(previous) = ranges.last_mut()
            && range.start < previous.end
        {
            previous.end = previous.end.max(range.end);
        } else {
            ranges.push(range);
        }
    }
    ranges
}

pub fn color(dark: bool) -> Hsla {
    gpui::rgb_to_hsla(rgba(if dark { 0xffcf6655 } else { 0xffcf6670 }))
}

/// Split existing style runs so search decoration preserves Markdown formatting.
pub fn apply_styles(
    text: &str,
    styles: Vec<(Range<usize>, HighlightStyle)>,
    query: &str,
    dark: bool,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let matches = match_ranges(text, query);
    let mut result = Vec::new();
    for (range, style) in styles {
        let mut boundaries = vec![range.start, range.end];
        for hit in &matches {
            if hit.start < range.end && hit.end > range.start {
                boundaries.extend([hit.start.max(range.start), hit.end.min(range.end)]);
            }
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        for pair in boundaries.windows(2) {
            let mut style = style;
            if matches
                .iter()
                .any(|hit| hit.start <= pair[0] && pair[0] < hit.end)
            {
                style.background_color = Some(color(dark));
            }
            result.push((pair[0]..pair[1], style));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_chinese_emoji_and_repeated_case_insensitive_terms() {
        let text = "📄 Rust 文档 rust，关键词关键词";
        let ranges = match_ranges(text, "RUST");
        assert_eq!(
            ranges
                .iter()
                .map(|range| &text[range.clone()])
                .collect::<Vec<_>>(),
            ["Rust", "rust"]
        );
        assert_eq!(match_ranges(text, "关键词").len(), 2);
        assert!(match_ranges(text, "").is_empty());
        assert!(match_ranges(text, "missing").is_empty());
    }

    #[test]
    fn lowercase_expansion_keeps_original_character_boundaries() {
        let text = "İ文档";
        assert_eq!(match_ranges(text, "i"), vec![0..2]);
        assert_eq!(match_ranges(text, "文档"), vec![2..8]);
    }

    #[test]
    fn highlight_crosses_style_boundaries_without_losing_formatting() {
        let bold = HighlightStyle {
            font_weight: Some(gpui::FontWeight::BOLD),
            ..Default::default()
        };
        let styles = apply_styles(
            "hello world",
            vec![(0..6, bold), (6..11, HighlightStyle::default())],
            "lo wo",
            false,
        );
        assert_eq!(
            styles
                .iter()
                .map(|(range, _)| range.clone())
                .collect::<Vec<_>>(),
            vec![0..3, 3..6, 6..8, 8..11]
        );
        assert_eq!(styles[1].1.font_weight, bold.font_weight);
        assert_eq!(styles[1].1.background_color, Some(color(false)));
        assert_eq!(styles[2].1.background_color, Some(color(false)));
        assert!(styles[0].1.background_color.is_none());
    }
}
