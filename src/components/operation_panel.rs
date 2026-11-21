use super::Icon;
use crate::theme::ThemePreset;
use gpui::{
    Animation, AnimationExt as _, AnyElement, IntoElement, ParentElement, SharedString, Styled,
    div, px, rgb,
};

/// Semantic color for the bottom-right execution status, so running, done,
/// skipped, and failed reads are distinguishable at a glance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusTone {
    /// Muted, for idle or skipped work.
    #[default]
    Neutral,
    /// Accent, for work in progress.
    Accent,
    /// Green, for a completed operation.
    Success,
    /// Amber, for interrupted or incomplete work.
    Warning,
    /// Red, for a failed operation.
    Danger,
}

impl StatusTone {
    const fn token(self) -> u32 {
        match self {
            Self::Neutral => 0,
            Self::Accent => 0x007aff,
            Self::Success => 0x24864c,
            Self::Warning => 0xb37716,
            Self::Danger => 0xd9342b,
        }
    }

    /// Resolve this tone against a theme preset.
    pub fn color(self, theme: ThemePreset) -> u32 {
        match self {
            Self::Neutral => theme.palette().text_muted,
            tone => theme.color(tone.token()),
        }
    }
}

/// Neutral result surface for an operation inside a collapsible activity row.
/// The host owns scrolling, streamed content, and execution status.
pub struct OperationPanel {
    title: SharedString,
    theme: ThemePreset,
    body: Vec<AnyElement>,
    status: Option<(Icon, SharedString)>,
    status_tone: StatusTone,
    status_animation_id: SharedString,
}

impl OperationPanel {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            theme: ThemePreset::default(),
            body: vec![],
            status: None,
            status_tone: StatusTone::default(),
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

    /// Color the status icon and label by outcome. Defaults to muted neutral.
    pub fn status_tone(mut self, tone: StatusTone) -> Self {
        self.status_tone = tone;
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
                    .text_color(rgb(self.status_tone.color(self.theme)))
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
