use super::Icon;
use super::scroll_chain::ChainScroll;
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
/// summary, rotating chevron) over a body of custom content. By default the
/// body sits flush with the trigger icon, which suits the common case of one
/// result block; call [`CollapsePanel::guard_line`] to hang it off a left
/// hairline instead, which groups multi-row bodies such as a work log.
pub struct CollapsePanel {
    icon: Option<Icon>,
    summary: SharedString,
    summary_content: Option<CollapsePanelContent>,
    streaming_summary: bool,
    summary_scroll: ScrollHandle,
    content: Option<CollapsePanelContent>,
    expanded: bool,
    compact: bool,
    /// Draw the left hairline and indent the body under the trigger. Off by
    /// default, so a single result lines up with the trigger icon.
    guard_line: bool,
    expandable: bool,
    theme: ThemePreset,
}

impl EventEmitter<CollapsePanelEvent> for CollapsePanel {}

impl CollapsePanel {
    pub fn new(summary: impl Into<SharedString>) -> Self {
        Self {
            icon: None,
            summary: summary.into(),
            summary_content: None,
            streaming_summary: false,
            summary_scroll: ScrollHandle::new(),
            content: None,
            expanded: false,
            compact: false,
            guard_line: false,
            expandable: true,
            theme: ThemePreset::default(),
        }
    }

    pub fn expandable(mut self, expandable: bool) -> Self {
        self.expandable = expandable;
        if !expandable {
            self.expanded = false;
        }
        self
    }

    pub fn set_expandable(&mut self, expandable: bool, cx: &mut Context<Self>) {
        if self.expandable != expandable {
            self.expandable = expandable;
            if !expandable {
                self.expanded = false;
            }
            cx.notify();
        }
    }

    pub fn set_icon(&mut self, icon: Option<Icon>, cx: &mut Context<Self>) {
        if self.icon != icon {
            self.icon = icon;
            cx.notify();
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
        self.summary_content = None;
        self.summary = summary.into();
        self.streaming_summary = false;
        cx.notify();
    }

    /// Supply custom inline summary content, such as a clickable file name.
    /// Calling `set_summary` restores the default text summary.
    pub fn set_summary_content(
        &mut self,
        content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
        cx: &mut Context<Self>,
    ) {
        self.summary_content = Some(Box::new(content));
        cx.notify();
    }

    /// Follow the last visible line of a changing summary in a one-line viewport.
    pub fn set_streaming_summary(
        &mut self,
        summary: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.summary_content = None;
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

    pub fn is_expandable(&self) -> bool {
        self.expandable
    }

    pub fn set_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        if !self.expandable && expanded {
            return;
        }
        if self.expanded != expanded {
            self.expanded = expanded;
            cx.emit(CollapsePanelEvent::Toggled { expanded });
            cx.notify();
        }
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        if !self.expandable {
            return;
        }
        self.set_expanded(!self.expanded, cx);
    }

    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    /// Draw the left hairline through the body and indent it under the
    /// trigger. Off by default: one result block reads better flush left, lined
    /// up with the trigger icon. Turn it on for multi-row bodies such as a work
    /// log, where the line groups the rows.
    pub fn guard_line(mut self, show: bool) -> Self {
        self.guard_line = show;
        self
    }

    /// Setter form of [`CollapsePanel::guard_line`], for hosts that keep one
    /// entity and re-apply their layout choice as content changes.
    pub fn set_guard_line(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.guard_line != show {
            self.guard_line = show;
            cx.notify();
        }
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
        let summary = if let Some(content) = &self.summary_content {
            summary.child(content(window, cx)).into_any_element()
        } else if self.streaming_summary {
            summary
                .id("streaming-summary")
                .debug_selector(|| "streaming-summary".into())
                .h(px(20.))
                .overflow_y_scroll()
                .track_scroll(&self.summary_scroll)
                .chain_scroll_y(&self.summary_scroll)
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

        let trigger = div()
            .id("glass-collapse-panel-trigger")
            .debug_selector(|| "glass-collapse-panel-trigger".into())
            .w_full()
            .h(px(30.))
            .flex_shrink_0()
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(7.))
            .when(self.expandable, |row| {
                row.cursor_pointer()
                    .hover(move |style| style.bg(hover_bg))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle(cx)))
            })
            .when_some(self.icon, |row, icon| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .text_color(rgb(p.text_muted))
                        .child(icon.render(15.)),
                )
            })
            .child(summary)
            .when(self.expandable, |row| {
                row.child(
                    div()
                        .flex_shrink_0()
                        .text_color(rgb(p.text_muted))
                        .child(Icon::disclosure(12., self.expanded)),
                )
            });

        let mut root = div()
            .id("glass-collapse-panel")
            .debug_selector(|| "glass-collapse-panel".into())
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .child(trigger);

        if self.expandable
            && self.expanded
            && let Some(content) = &self.content
        {
            let body = div().py(px(2.)).flex_shrink_0();
            // The hairline doubles as the indent guide: drop both together so a
            // lined body stays aligned with the trigger icon above it.
            let body = if self.guard_line {
                body.ml(px(15.))
                    .pl(px(18.))
                    .border_l_1()
                    .border_color(rgb(p.border))
            } else {
                // No guide: line the body up with the trigger icon instead of
                // hanging it off a hairline.
                body.pl(px(10.))
            };
            root = root.child(body.child(content(window, cx)));
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
    fn non_expandable_panel_does_not_toggle_and_hides_body(cx: &mut TestAppContext) {
        let (preview, window) = cx.add_window_view(|_, cx| {
            Preview(
                cx.new(|_| {
                    CollapsePanel::new("Edited")
                        .expandable(false)
                        .content(|_, _| div().id("panel-body").child("Body content").into_any_element())
                }),
            )
        });
        let panel = preview.read_with(window, |preview, _| preview.0.clone());
        assert!(!panel.read_with(window, |panel, _| panel.is_expanded()));
        assert!(!panel.read_with(window, |panel, _| panel.is_expandable()));
        assert!(window.debug_bounds("panel-body").is_none());

        panel.update(window, |panel, cx| {
            panel.toggle(cx);
        });
        assert!(!panel.read_with(window, |panel, _| panel.is_expanded()));

        panel.update(window, |panel, cx| {
            panel.set_expanded(true, cx);
        });
        assert!(!panel.read_with(window, |panel, _| panel.is_expanded()));
        assert!(window.debug_bounds("panel-body").is_none());
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
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        window.run_until_parked();
        panel.read_with(window, |panel, _| assert!(panel.is_expanded()));
        window.simulate_click(bounds.center(), gpui::Modifiers::default());
        window.run_until_parked();
        panel.read_with(window, |panel, _| assert!(!panel.is_expanded()));
        panel.update(window, |panel, cx| {
            panel.set_expanded(true, cx);
            panel.set_summary("已思考 12.3 s", cx);
        });
        panel.read_with(window, |panel, _| {
            assert!(!panel.streaming_summary);
            assert!(panel.is_expanded());
        });
    }

    #[gpui::test]
    fn short_parent_never_compresses_an_expanded_panel(cx: &mut TestAppContext) {
        struct Viewport(Entity<CollapsePanel>);
        impl Render for Viewport {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .w(px(240.))
                    .h(px(80.))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(self.0.clone())
            }
        }
        let (_view, window) = cx.add_window_view(|_, cx| {
            Viewport(cx.new(|_| {
                CollapsePanel::new("思考")
                    .expanded(true)
                    .content(|_, _| div().h(px(200.)).into_any_element())
            }))
        });
        window.run_until_parked();
        let panel = window
            .debug_bounds("glass-collapse-panel")
            .expect("panel bounds");
        assert!(
            panel.size.height >= px(230.),
            "expanded panel was compressed to {panel:?}"
        );
    }
}
