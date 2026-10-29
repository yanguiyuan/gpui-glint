use super::Icon;
use crate::theme::ThemePreset;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Context, EventEmitter, InteractiveElement, IntoElement, ParentElement, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Window, div, px, rgb, rgba,
};

/// Builds the panel body on every frame, so hosts can stream updates into
/// thinking text, code previews, or terminal output.
pub type CollapsePanelContent = Box<dyn Fn(&mut Window, &mut App) -> AnyElement + 'static>;

#[derive(Clone, Debug)]
pub enum CollapsePanelEvent {
    /// The panel expanded or collapsed.
    Toggled { expanded: bool },
}

/// An AI-chat style collapsible panel: a trigger row (optional prefix icon,
/// summary, rotating chevron) over a body of custom content with a left
/// hairline. Typical bodies are thinking text, code previews, and terminal
/// output.
pub struct CollapsePanel {
    icon: Option<Icon>,
    summary: SharedString,
    streaming_summary: bool,
    summary_scroll: ScrollHandle,
    content: Option<CollapsePanelContent>,
    expanded: bool,
    compact: bool,
    theme: ThemePreset,
}

impl EventEmitter<CollapsePanelEvent> for CollapsePanel {}

impl CollapsePanel {
    pub fn new(summary: impl Into<SharedString>) -> Self {
        Self {
            icon: None,
            summary: summary.into(),
            streaming_summary: false,
            summary_scroll: ScrollHandle::new(),
            content: None,
            expanded: false,
            compact: false,
            theme: ThemePreset::default(),
        }
    }

    /// Keep the chevron next to the summary instead of at the far edge.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Sets the body builder; called on every frame while expanded.
    pub fn content(
        mut self,
        content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        self.content = Some(Box::new(content));
        self
    }

    /// Same as [`CollapsePanel::content`] taking the boxed type directly,
    /// useful when type inference needs a hand.
    pub fn content_box(mut self, content: CollapsePanelContent) -> Self {
        self.content = Some(content);
        self
    }

    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn set_summary(&mut self, summary: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.summary = summary.into();
        self.streaming_summary = false;
        cx.notify();
    }

    /// Follow the last visible line of a changing summary in a one-line viewport.
    pub fn set_streaming_summary(
        &mut self,
        summary: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.summary = summary.into();
        self.streaming_summary = true;
        self.summary_scroll.scroll_to_bottom();
        cx.notify();
    }

    pub fn set_content(
        &mut self,
        content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
        cx: &mut Context<Self>,
    ) {
        self.content = Some(Box::new(content));
        cx.notify();
    }

    pub fn set_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        if self.expanded != expanded {
            self.expanded = expanded;
            cx.emit(CollapsePanelEvent::Toggled { expanded });
            cx.notify();
        }
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        self.set_expanded(!self.expanded, cx);
    }

    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            cx.notify();
        }
    }
}

impl Render for CollapsePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let p = theme.palette();
        let dark = theme.is_dark();
        let hover_bg = if dark {
            rgba(0xffffff14)
        } else {
            rgb(p.surface_subtle)
        };
        let summary = div()
            .when(!self.compact, |summary| summary.flex_1())
            .min_w_0()
            .text_size(px(13.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(rgb(p.text_muted));
        let summary = if self.streaming_summary {
            summary
                .id("streaming-summary")
                .debug_selector(|| "streaming-summary".into())
                .h(px(20.))
                .overflow_y_scroll()
                .track_scroll(&self.summary_scroll)
                .child(
                    div()
                        .w_full()
                        .line_height(px(20.))
                        .child(self.summary.clone()),
                )
                .into_any_element()
        } else {
            summary
                .truncate()
                .child(self.summary.clone())
                .into_any_element()
        };

        let trigger =
            div()
                .id("glass-collapse-panel-trigger")
                .w_full()
                .h(px(30.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(hover_bg))
                .on_click(cx.listener(|this, _, _, cx| this.toggle(cx)))
                .when_some(self.icon, |row, icon| {
                    row.child(
                        div()
                            .flex_shrink_0()
                            .text_color(rgb(p.text_muted))
                            .child(icon.render(15.)),
                    )
                })
                .child(summary)
                .child(div().flex_shrink_0().text_color(rgb(p.text_muted)).child(
                    if self.expanded {
                        Icon::ChevronDown.render(12.)
                    } else {
                        Icon::ChevronRight.render(12.)
                    },
                ));

        let mut root = div()
            .id("glass-collapse-panel")
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(trigger);

        if self.expanded
            && let Some(content) = &self.content
        {
            root = root.child(
                div()
                    .ml(px(15.))
                    .pl(px(18.))
                    .py(px(2.))
                    .border_l_1()
                    .border_color(rgb(p.border))
                    .child(content(window, cx)),
            );
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext, Entity, TestAppContext};

    struct Preview(Entity<CollapsePanel>);
    impl Render for Preview {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(240.)).child(self.0.clone())
        }
    }

    #[gpui::test]
    fn streaming_summary_stays_one_line_and_follows_the_tail(cx: &mut TestAppContext) {
        let (preview, window) = cx.add_window_view(|_, cx| {
            Preview(cx.new(|_| CollapsePanel::new("思考").icon(Icon::Pulse)))
        });
        let panel = preview.read_with(window, |preview, _| preview.0.clone());
        panel.update(window, |panel, cx| {
            panel.set_streaming_summary(
                "正在分析内容并检查文件的相关依赖与实现细节。".repeat(20),
                cx,
            );
        });
        let bounds = window.debug_bounds("streaming-summary").unwrap();
        assert_eq!(bounds.size.height, px(20.));
        panel.read_with(window, |panel, _| {
            assert!(panel.summary_scroll.offset().y < px(0.));
            assert!(!panel.is_expanded());
        });
        panel.update(window, |panel, cx| {
            panel.set_expanded(true, cx);
            panel.set_summary("已思考 12.3 s", cx);
        });
        panel.read_with(window, |panel, _| {
            assert!(!panel.streaming_summary);
            assert!(panel.is_expanded());
        });
    }
}
