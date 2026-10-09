use std::rc::Rc;

use gpui::{
    AnyElement, App, IntoElement, ParentElement, SharedString, Styled, Window, div, prelude::*, px,
    rgb, rgba,
};

use super::Icon;
use crate::theme::ThemePreset;

type TabHandler = Rc<dyn Fn(u64, &mut Window, &mut App)>;

/// A sidebar entry with a stable identity and optional leading icon.
pub struct SideTab {
    pub id: u64,
    pub label: SharedString,
    pub icon: Option<Icon>,
}

impl SideTab {
    pub fn new(id: u64, label: impl Into<SharedString>) -> Self {
        Self {
            id,
            label: label.into(),
            icon: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// A settings-style sidebar navigator: a section list on a muted track beside
/// a content panel. The active entry carries an accent bar on the card's left
/// edge; the parent owns selection.
pub struct SideTabs {
    tabs: Vec<SideTab>,
    selected: Option<u64>,
    theme: ThemePreset,
    on_select: Option<TabHandler>,
    contents: Vec<(u64, AnyElement)>,
}

impl SideTabs {
    pub fn new(tabs: impl IntoIterator<Item = SideTab>) -> Self {
        Self {
            tabs: tabs.into_iter().collect(),
            selected: None,
            theme: ThemePreset::default(),
            on_select: None,
            contents: Vec::new(),
        }
    }

    pub fn selected(mut self, selected: Option<u64>) -> Self {
        self.selected = selected;
        self
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    /// Registers the panel content shown when the tab with this id is active.
    pub fn content(mut self, id: u64, content: impl IntoElement) -> Self {
        self.contents.push((id, content.into_any_element()));
        self
    }

    pub fn on_select(mut self, handler: impl Fn(u64, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }

    pub fn render(mut self) -> gpui::Div {
        if self.tabs.is_empty() {
            return div();
        }
        let p = self.theme.palette();
        let dark = self.theme.is_dark();
        let mut active_content = None;
        if let Some(selected) = self.selected
            && let Some(index) = self.contents.iter().position(|(id, _)| *id == selected)
        {
            active_content = Some(self.contents.remove(index).1);
        }

        let mut sidebar = div()
            .id("glass-side-tabs")
            .w(px(160.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .rounded_l(px(12.))
            .bg(if dark {
                rgba(0xffffff03)
            } else {
                rgb(p.canvas)
            })
            .border_r_1()
            .border_color(if dark {
                rgba(0xffffff14)
            } else {
                rgb(p.border)
            })
            .py(px(8.))
            .text_size(px(13.));

        for tab in self.tabs {
            let active = self.selected == Some(tab.id);
            let select = self.on_select.clone();
            let mut label = div().min_w_0().flex().items_center().gap_2();
            if let Some(icon) = tab.icon {
                label = label.child(icon.render(14.));
            }
            label = label.child(div().min_w_0().text_ellipsis().child(tab.label));

            let mut item = div()
                .id(("glass-side-tab", tab.id))
                .h(px(40.))
                .flex()
                .when(active, |item| {
                    item.bg(if dark {
                        rgba(0x0a84ff14)
                    } else {
                        rgb(p.accent_tint)
                    })
                    .text_color(rgb(if dark { p.text } else { p.accent }))
                    .font_weight(gpui::FontWeight::MEDIUM)
                })
                .when(!active, |item| {
                    item.text_color(rgb(if dark { p.text_muted } else { p.text }))
                        .cursor_pointer()
                        .hover(move |style| {
                            style.bg(if dark {
                                rgba(0xffffff08)
                            } else {
                                rgb(p.surface)
                            })
                        })
                })
                .child(
                    div()
                        .w(px(3.))
                        .h_full()
                        .flex_shrink_0()
                        .when(active, |bar| bar.bg(rgb(p.accent))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .flex()
                        .items_center()
                        .px(px(16.))
                        .child(label),
                );

            if let Some(handler) = select {
                let id = tab.id;
                item = item.on_click(move |_, window, cx| handler(id, window, cx));
            }
            sidebar = sidebar.child(item);
        }

        let mut panel = div()
            .flex_1()
            .min_w_0()
            .min_h(px(160.))
            .rounded_r(px(12.))
            .bg(if dark {
                rgba(0xffffff08)
            } else {
                rgb(p.surface)
            })
            .p(px(20.))
            .text_size(px(13.))
            .text_color(rgb(p.text));
        if let Some(content) = active_content {
            panel = panel.child(content);
        }

        div()
            .w_full()
            .min_w_0()
            .flex()
            .when(dark, |card| {
                card.rounded(px(12.))
                    .border_1()
                    .border_color(rgba(0xffffff14))
                    .backdrop_blur(px(24.))
            })
            .child(sidebar)
            .child(panel)
    }
}

impl IntoElement for SideTabs {
    type Element = gpui::Div;

    fn into_element(self) -> Self::Element {
        self.render()
    }
}
