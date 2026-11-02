//! Bounded source previews for structured prompt references.
use super::ComposerReference;
use crate::theme::ThemePreset;
use gpui::prelude::FluentBuilder;
use gpui::{InteractiveElement, IntoElement, ParentElement, Styled, div, px, rgb, rgba};
pub(super) fn reference_preview(
    reference: &ComposerReference,
    theme: ThemePreset,
    width: f32,
) -> impl IntoElement {
    let p = theme.palette();
    let text = reference
        .preview
        .as_deref()
        .or(reference.excerpt.as_deref())
        .unwrap_or(if reference.directory {
            "目录引用，Agent 可按需查看目录内容。"
        } else {
            "正在读取文件预览…"
        });
    // Reserve a fixed final line for the fade; never grow into the conversation.
    let columns = ((width - 26.) / 7.3).floor().max(1.) as usize;
    let rows: usize = text
        .lines()
        .map(|line| {
            line.chars()
                .map(|c| {
                    if c == '\t' {
                        4
                    } else if c.is_ascii() {
                        1
                    } else {
                        2
                    }
                })
                .sum::<usize>()
                .div_ceil(columns)
                .max(1)
        })
        .sum();
    let clipped = reference.preview_truncated || rows > 10;
    let mut body = div()
        .relative()
        .max_h(px(180.))
        .overflow_hidden()
        .font_family("Menlo")
        .text_size(px(12.))
        .line_height(px(18.))
        .child(text.to_owned());
    if clipped {
        for index in 0..8 {
            let t = (index + 1) as f32 / 8.;
            body = body.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px((7 - index) as f32 * 6.))
                    .h(px(6.))
                    .when(index == 7, |strip| {
                        strip.debug_selector(|| "reference-preview-fade".into())
                    })
                    .backdrop_blur(px(t * t * 6.))
                    .bg(rgba((p.surface << 8) | ((t * t * 255.) as u32))),
            );
        }
    }
    div()
        .debug_selector(|| "prompt-reference-preview".into())
        .w(px(width))
        .max_h(px(300.))
        .overflow_hidden()
        .min_w_0()
        .p(px(12.))
        .rounded(px(9.))
        .border_1()
        .border_color(rgb(p.border))
        .bg(rgb(p.surface))
        .text_color(rgb(p.text))
        .shadow_lg()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .child(reference.icon().render(14.).text_color(rgb(p.accent)))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(reference.label().trim_start_matches('@').to_owned()),
                ),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(11.))
                .text_color(rgb(p.text_muted))
                .child(crate::paths::display_path(&reference.path)),
        )
        .child(body)
        .when(clipped, |root| {
            root.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child("仅展示部分内容 · 完整内容可在文件中查看"),
            )
        })
}
