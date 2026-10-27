//! Shared tab styling, method colors, status labels, and record timestamps.

use super::*;

pub(super) fn tab_button(
    label: &str,
    selected: bool,
    theme: ThemePreset,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(label.to_owned())
        .h_full()
        .flex()
        .items_center()
        .text_size(px(11.))
        .cursor_pointer()
        .text_color(rgb(if selected {
            theme.palette().text
        } else {
            theme.palette().text_muted
        }))
        .when(selected, |s| s.border_b_2().border_color(rgb(0x007aff)))
        .hover(|s| s.text_color(rgb(0x007aff)))
        .child(label.to_owned())
}
pub(super) fn method_color(method: &str) -> u32 {
    match method {
        "POST" => 0xb37716,
        "DELETE" => 0xd9342b,
        "PATCH" | "PUT" => 0x876ab4,
        _ => 0x24864c,
    }
}
pub(super) fn status_label(status: u16) -> String {
    let reason = reqwest::StatusCode::from_u16(status)
        .ok()
        .and_then(|status| status.canonical_reason())
        .unwrap_or("");
    format!("{status} {reason}").trim_end().to_string()
}
pub(super) fn record_time() -> String {
    chrono::Utc::now()
        .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}
