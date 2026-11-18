use super::Icon;
use crate::theme::ThemePreset;
use gpui::{
    Animation, AnimationExt as _, AnyElement, IntoElement, ParentElement, SharedString, Styled,
    div, px, rgb,
};

/// Neutral result surface for an operation inside a collapsible activity row.
/// The host owns scrolling, streamed content, and execution status.
pub struct OperationPanel {
    title: SharedString,
    theme: ThemePreset,
    body: Vec<AnyElement>,
    status: Option<(Icon, SharedString)>,
    status_animation_id: SharedString,
}

impl OperationPanel {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            theme: ThemePreset::default(),
            body: vec![],
            status: None,
            status_animation_id: SharedString::from("operation-panel-status"),
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

    /// Stable id for the status spinner animation. Pass something unique per
    /// panel (such as the tool call id) so several running panels animate
    /// independently instead of sharing one animation state.
    pub fn status_animation_id(mut self, id: impl Into<SharedString>) -> Self {
        self.status_animation_id = id.into();
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
            // A static spinner glyph reads as a frozen icon, so the running
            // state rotates it just like the sidebar and composer spinners.
            let status_icon = if matches!(icon, Icon::Spinner) {
                icon.render(13.)
                    .with_animation(
                        self.status_animation_id,
                        Animation::new(std::time::Duration::from_millis(900)).repeat(),
                        |icon, delta| {
                            icon.with_transformation(gpui::Transformation::rotate(gpui::radians(
                                std::f32::consts::TAU * delta,
                            )))
                        },
                    )
                    .into_any_element()
            } else {
                icon.render(13.).into_any_element()
            };
            panel = panel.child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .text_size(px(11.))
                    .child(status_icon)
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
