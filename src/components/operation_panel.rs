use super::Icon;
use crate::theme::ThemePreset;
use gpui::{AnyElement, IntoElement, ParentElement, SharedString, Styled, div, px, rgb};

/// Neutral result surface for an operation inside a collapsible activity row.
/// The host owns scrolling, streamed content, and execution status.
pub struct OperationPanel {
    title: SharedString,
    theme: ThemePreset,
    body: Vec<AnyElement>,
    status: Option<(Icon, SharedString)>,
}

impl OperationPanel {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            theme: ThemePreset::default(),
            body: vec![],
            status: None,
        }
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn child(mut self, content: impl IntoElement) -> Self {
        self.body.push(content.into_any_element());
        self
    }

    pub fn status(mut self, icon: Icon, label: impl Into<SharedString>) -> Self {
        self.status = Some((icon, label.into()));
        self
    }

    pub fn render(self) -> AnyElement {
        let p = self.theme.palette();
        let mut panel = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.canvas))
            .p_3()
            .text_color(rgb(p.text_muted))
            .child(div().text_size(px(12.)).child(self.title))
            .child(div().w_full().min_w_0().children(self.body));
        if let Some((icon, label)) = self.status {
            panel = panel.child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .text_size(px(11.))
                    .child(icon.render(13.))
                    .child(label),
            );
        }
        panel.into_any_element()
    }
}

impl IntoElement for OperationPanel {
    type Element = AnyElement;
    fn into_element(self) -> Self::Element {
        self.render()
    }
}
