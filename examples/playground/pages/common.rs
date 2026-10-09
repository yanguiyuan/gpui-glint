use gpui::{
    InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, div, px,
    rgb, rgba,
};
use gpui_glint::theme::ThemePreset;

pub fn heading(
    number: &'static str,
    title: &'static str,
    description: &'static str,
    theme: ThemePreset,
) -> gpui::Div {
    let palette = theme.palette();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .px(px(8.))
                        .py(px(2.))
                        .rounded(px(7.))
                        .bg(rgb(palette.surface_subtle))
                        .text_size(px(12.))
                        .text_color(rgb(theme.color(0x7e8582)))
                        .child(number),
                )
                .child(
                    div()
                        .text_size(px(22.))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(rgb(palette.text))
                        .child(title),
                ),
        )
        .child(
            div()
                .max_w(px(690.))
                .text_size(px(14.))
                .text_color(rgb(palette.text_muted))
                .child(description),
        )
}

pub fn panel(content: impl IntoElement, theme: ThemePreset) -> gpui::Div {
    let palette = theme.palette();
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_5()
        .p_6()
        .rounded(px(20.))
        .border_1()
        .border_color(rgba(theme.glass_line()))
        .backdrop_blur(px(if theme.is_dark() { 36. } else { 0. }))
        .bg(rgba(theme.panel_material()))
        .text_color(rgb(palette.text))
        .shadow_md()
        .child(content)
}

pub fn page(
    number: &'static str,
    title: &'static str,
    description: &'static str,
    body: impl IntoElement,
    theme: ThemePreset,
) -> gpui::Stateful<gpui::Div> {
    let palette = theme.palette();
    div()
        .id(SharedId::for_page(number))
        .size_full()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_4()
        .p_8()
        .bg(if theme.is_dark() {
            rgba(0x00000000)
        } else {
            rgb(palette.canvas)
        })
        .text_color(rgb(palette.text))
        .child(heading(number, title, description, theme))
        .child(body)
}

struct SharedId;

impl SharedId {
    fn for_page(number: &'static str) -> &'static str {
        match number {
            "00" => "page-overview",
            "01" => "page-buttons",
            "02" => "page-icons",
            "03" => "page-layout",
            "04" => "page-inputs",
            "05" => "page-badges",
            "12" => "page-tables",
            "16.4" => "page-code-cards",
            "17" => "page-tabs",
            _ => "page-component",
        }
    }
}

/// A sharp backdrop makes live filtering visible even on a soft wallpaper.
pub fn backdrop_preview(theme: ThemePreset, enabled: bool, toggle: impl IntoElement) -> gpui::Div {
    let palette = theme.palette();
    let glass = |title: &'static str, radius: f32| {
        div()
            .flex_1()
            .h_full()
            .rounded(px(16.))
            .backdrop_blur(px(radius))
            .bg(rgba(if theme.is_dark() {
                theme.panel_material()
            } else {
                0xffffff70
            }))
            .border_1()
            .border_color(rgba(if theme.is_dark() {
                theme.glass_line()
            } else {
                0xffffffb0
            }))
            .p_4()
            .text_size(px(13.))
            .text_color(rgb(palette.text))
            .child(title)
    };
    div().w_full().flex().flex_col().gap_3()
        .child(div().flex().items_center().justify_between()
            .child(div().flex().flex_col().gap_1()
                .child(div().text_size(px(14.)).text_color(rgb(palette.text)).child("玻璃材质 · 实时背景模糊对照"))
                .child(div().text_size(px(12.)).text_color(rgb(palette.text_muted)).child("左侧仅透明；右侧模糊背后的条纹与文字，面板自身文字保持清晰。")))
            .child(toggle))
        .child(div().relative().w_full().h(px(160.)).overflow_hidden().rounded(px(20.)).bg(rgb(palette.canvas))
            .child(div().absolute().top_0().left_0().size_full().flex().gap(px(10.))
                .children((0..100).map(|index| div().flex_shrink_0().w(px(24.)).h_full().bg(rgb(if index % 2 == 0 { 0x0a84ff } else { 0x34c759 })))))
            .child(div().absolute().top(px(88.)).left(px(12.)).whitespace_nowrap().text_size(px(26.)).text_color(rgb(0xffffff))
                .child("BACKGROUND 0123456789  ·  BACKGROUND 0123456789  ·  BACKGROUND 0123456789"))
            .child(div().absolute().top(px(12.)).left(px(12.)).right(px(12.)).bottom(px(12.)).flex().gap_5()
                .child(glass("透明面板 · 0px", 0.))
                .child(glass(if enabled { "实时模糊 · 36px" } else { "模糊已关闭 · 0px" }, if enabled { 36. } else { 0. }))))
}
