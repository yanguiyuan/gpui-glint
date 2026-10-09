use crate::theme::ThemePreset;
use gpui::{
    AnyElement, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder, px, rgb, rgba,
};

/// A full-window shell with a topbar, outer navigation rail, and workspace.
///
/// The sidebar and content share a rounded glass panel, separated by a subtle
/// divider. The panel is raised from the topbar and navigation rail with a
/// light edge and soft shadow.
pub struct AppLayout {
    topbar: AnyElement,
    navigation_rail: Option<AnyElement>,
    sidebar: AnyElement,
    content: AnyElement,
    navigation_rail_width: f32,
    sidebar_width: f32,
    sidebar_visible: bool,
    theme: ThemePreset,
}

impl AppLayout {
    /// Creates the shell from its three content regions.
    pub fn new(
        topbar: impl IntoElement,
        sidebar: impl IntoElement,
        content: impl IntoElement,
    ) -> Self {
        Self {
            topbar: topbar.into_any_element(),
            navigation_rail: None,
            sidebar: sidebar.into_any_element(),
            content: content.into_any_element(),
            navigation_rail_width: 48.,
            sidebar_width: 248.,
            sidebar_visible: true,
            theme: ThemePreset::default(),
        }
    }

    /// Adds the narrow navigation rail outside the sidebar/content panel.
    pub fn navigation_rail(mut self, rail: impl IntoElement) -> Self {
        self.navigation_rail = Some(rail.into_any_element());
        self
    }

    /// Sets the width of the outer navigation rail in pixels. Defaults to 48px.
    pub fn navigation_rail_width(mut self, width: f32) -> Self {
        self.navigation_rail_width = width.max(0.);
        self
    }

    /// Sets the width of the sidebar in pixels. Defaults to 248px.
    pub fn sidebar_width(mut self, width: f32) -> Self {
        self.sidebar_width = width.max(0.);
        self
    }

    /// Shows or hides the sidebar, allowing content to fill the workspace when hidden.
    pub fn sidebar_visible(mut self, visible: bool) -> Self {
        self.sidebar_visible = visible;
        self
    }

    /// Applies a Glint UI surface palette to the outer shell.
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn render(self) -> gpui::Div {
        let palette = self.theme.palette();
        let topbar = div()
            .w_full()
            .h(px(42.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .child(self.topbar);

        let sidebar = div()
            .w(px(self.sidebar_width))
            .h_full()
            .flex_shrink_0()
            .overflow_hidden()
            .border_r_1()
            .border_color(if self.theme.is_dark() {
                rgba(0xffffff14)
            } else {
                rgb(palette.border)
            })
            .child(self.sidebar);

        let content = div()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .child(self.content);

        let workspace = div()
            .flex_1()
            .h_full()
            .flex()
            .overflow_hidden()
            .rounded_tl(px(16.))
            .border_1()
            .border_color(rgba(self.theme.glass_line()))
            .bg(if self.theme.is_dark() {
                rgba(0x00000000)
            } else {
                rgb(palette.workspace)
            })
            .shadow_md()
            .when(self.sidebar_visible, |workspace| workspace.child(sidebar))
            .child(content);

        let mut body = div()
            .flex_1()
            .h(px(0.))
            .min_h_0()
            .flex()
            .bg(if self.theme.is_dark() {
                rgba(0x00000000)
            } else {
                rgb(palette.canvas)
            });

        if let Some(rail) = self.navigation_rail {
            body = body.child(
                div()
                    .w(px(self.navigation_rail_width))
                    .h_full()
                    .flex_shrink_0()
                    .overflow_hidden()
                    .child(rail),
            );
        }
        body = body.child(workspace);

        div()
            .relative()
            .when(self.theme.is_dark(), |root| {
                root.child(super::wallpaper::wallpaper(self.theme))
            })
            .text_color(rgb(palette.text))
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(palette.canvas))
            .child(topbar)
            .child(body)
    }
}

impl IntoElement for AppLayout {
    type Element = gpui::Div;

    fn into_element(self) -> Self::Element {
        self.render()
    }
}
