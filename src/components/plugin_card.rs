use super::{Button, ButtonKind, ButtonSize, Icon, TagBadge, TagBadgeKind};
use crate::theme::ThemePreset;
use gpui::{Div, FontWeight, IntoElement, SharedString, div, prelude::*, px, rgb};

/// Plugin summary. The navigation target and trailing controls are siblings.
pub fn plugin_row(summary: impl IntoElement, actions: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(summary)
        .child(actions)
}

pub fn plugin_summary(
    title: impl Into<SharedString>,
    badges: impl IntoIterator<Item = SharedString>,
    description: impl Into<SharedString>,
    theme: ThemePreset,
) -> Div {
    let palette = theme.palette();
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap_3()
        .px_2()
        .py_2()
        .rounded(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(38.))
                .flex_shrink_0()
                .rounded(px(9.))
                .border_1()
                .border_color(rgb(palette.border))
                .text_color(rgb(palette.accent))
                .child(Icon::Puzzle),
        )
        .child(
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(div().font_weight(FontWeight::MEDIUM).child(title.into()))
                        .children(badges.into_iter().map(|label| {
                            TagBadge::new(label)
                                .kind(TagBadgeKind::Metadata)
                                .theme(theme)
                                .render()
                        })),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(palette.text_muted))
                        .truncate()
                        .child(description.into()),
                ),
        )
}

pub fn plugin_group(title: &'static str) -> Div {
    div().flex().flex_col().gap_1().child(
        div()
            .px_2()
            .pb_1()
            .font_weight(FontWeight::MEDIUM)
            .child(title),
    )
}

pub fn plugin_card_details(theme: ThemePreset) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(theme.palette().border))
        .text_size(px(12.))
        .text_color(rgb(theme.palette().text_muted))
}

/// One read-only key/value line inside a plugin detail card.
pub fn plugin_meta_item(
    label: &'static str,
    value: impl Into<SharedString>,
    theme: ThemePreset,
) -> Div {
    let palette = theme.palette();
    div()
        .flex()
        .items_start()
        .gap_3()
        .child(
            div()
                .w(px(56.))
                .flex_shrink_0()
                .text_color(rgb(palette.text_muted))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(rgb(palette.text))
                .child(value.into()),
        )
}

/// Editable settings block, separated from the read-only metadata above it.
pub fn plugin_settings_section(theme: ThemePreset) -> Div {
    let palette = theme.palette();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .mt_1()
        .pt_3()
        .border_t_1()
        .border_color(rgb(palette.border))
}

/// Caption above one settings group.
pub fn plugin_section_title(
    title: impl Into<SharedString>,
    hint: impl Into<SharedString>,
    theme: ThemePreset,
) -> Div {
    let palette = theme.palette();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.text))
                .child(title.into()),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(palette.text_muted))
                .child(hint.into()),
        )
}

/// Bordered card holding one editable setting together with its own explanation.
pub fn plugin_setting_card(theme: ThemePreset) -> Div {
    let palette = theme.palette();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(palette.border))
        .bg(rgb(palette.surface))
}

pub fn plugin_details_back(theme: ThemePreset) -> gpui::Stateful<Div> {
    Button::new("返回插件列表")
        .icon(Icon::ArrowLeft)
        .kind(ButtonKind::Plain)
        .size(ButtonSize::Small)
        .theme(theme)
        .render()
}
