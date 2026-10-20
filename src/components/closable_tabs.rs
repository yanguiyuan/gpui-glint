use std::rc::Rc;

use gpui::{
    AnyElement, App, IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Point,
    SharedString, Styled, Window, div, prelude::*, px, rgb, rgba,
};

use super::{Icon, IconButton};
use crate::theme::ThemePreset;

type TabHandler = Rc<dyn Fn(u64, &mut Window, &mut App)>;
type AddHandler = Rc<dyn Fn(&mut Window, &mut App)>;
/// 右键点击标签时回调：(标签 id, 鼠标位置)，菜单由调用方渲染。
type ContextHandler = Rc<dyn Fn(u64, Point<Pixels>, &mut Window, &mut App)>;

/// 空间不足时标签可压缩到的最小宽度，保证关闭按钮仍然可见。
const MIN_TAB_WIDTH: f32 = 64.;
/// 标签在空间充足时的最大宽度。
const MAX_TAB_WIDTH: f32 = 240.;

/// A tab with a stable identity, label, and optional leading icon.
pub struct ClosableTab {
    pub id: u64,
    pub label: SharedString,
    pub icon: Option<Icon>,
    pub modified: bool,
}

impl ClosableTab {
    pub fn new(id: u64, label: impl Into<SharedString>) -> Self {
        Self {
            id,
            label: label.into(),
            icon: None,
            modified: false,
        }
    }

    /// Marks the tab as having unsaved changes.
    pub fn modified(mut self, modified: bool) -> Self {
        self.modified = modified;
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// A document-style card with closable folder tabs. The active tab shares the
/// card surface and merges seamlessly into the panel below; the parent owns
/// selection and removal.
pub struct ClosableTabs {
    tabs: Vec<ClosableTab>,
    selected: Option<u64>,
    theme: ThemePreset,
    on_select: Option<TabHandler>,
    on_close: Option<TabHandler>,
    on_context: Option<ContextHandler>,
    on_add: Option<AddHandler>,
    contents: Vec<(u64, AnyElement)>,
    fill_height: bool,
    content_padding: f32,
}

impl ClosableTabs {
    pub fn new(tabs: impl IntoIterator<Item = ClosableTab>) -> Self {
        Self {
            tabs: tabs.into_iter().collect(),
            selected: None,
            theme: ThemePreset::default(),
            on_select: None,
            on_close: None,
            on_context: None,
            on_add: None,
            contents: Vec::new(),
            fill_height: false,
            content_padding: 20.,
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

    /// Fills the parent's height and lets the selected content use the remaining space.
    pub fn fill_height(mut self) -> Self {
        self.fill_height = true;
        self
    }

    /// Sets the content panel's padding in pixels. Defaults to 20px.
    pub fn content_padding(mut self, padding: f32) -> Self {
        self.content_padding = padding.max(0.);
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

    pub fn on_close(mut self, handler: impl Fn(u64, &mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(handler));
        self
    }

    /// 右键点击标签时回调，调用方负责在给定位置渲染上下文菜单。
    pub fn on_context(
        mut self,
        handler: impl Fn(u64, Point<Pixels>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_context = Some(Rc::new(handler));
        self
    }

    /// Shows a trailing plus button. The caller owns creating or opening the new tab.
    /// The button remains available when all tabs are closed.
    pub fn on_add(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_add = Some(Rc::new(handler));
        self
    }

    pub fn render(mut self) -> gpui::Div {
        if self.tabs.is_empty() && self.on_add.is_none() {
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

        let mut strip = div()
            .id("glass-closable-tabs")
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .rounded_t(px(12.))
            .bg(if dark {
                rgba(0xffffff03)
            } else {
                rgb(p.canvas)
            })
            .pt(px(6.))
            .px(px(6.))
            // 标签变多时优先压缩自身宽度，只有压缩到最小宽度仍然放不下时才横向滚动。
            .max_w_full()
            .overflow_x_scroll()
            .flex()
            .items_end()
            .gap_1()
            .text_size(px(13.));

        for tab in self.tabs {
            let active = self.selected == Some(tab.id);
            let select = self.on_select.clone();
            let close = self.on_close.clone();
            let mut label = div().min_w_0().flex().items_center().gap_2();
            if let Some(icon) = tab.icon {
                label = label.child(icon.render(14.).flex_shrink_0());
            }
            label = label.child(div().min_w_0().truncate().child(tab.label));
            if tab.modified {
                label = label.child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(p.accent))
                        .child("●"),
                );
            }

            let mut item = div()
                .id(("glass-closable-tab", tab.id))
                .h(px(40.))
                // 不再固定宽度：标签多时按比例压缩，标签少时最多 240px。
                .min_w(px(MIN_TAB_WIDTH))
                .max_w(px(MAX_TAB_WIDTH))
                .px(px(12.))
                .flex_shrink_1()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .overflow_hidden()
                .rounded_t(px(10.))
                .when(active, |item| {
                    item.bg(if dark {
                        rgba(0xffffff0b)
                    } else {
                        rgb(p.surface)
                    })
                    .font_weight(gpui::FontWeight::MEDIUM)
                })
                .when(dark, |item| {
                    item.border_t_2().border_color(if active {
                        rgb(p.accent)
                    } else {
                        rgba(0x00000000)
                    })
                })
                .text_color(rgb(if active || !dark {
                    p.text
                } else {
                    p.text_muted
                }))
                .cursor_pointer()
                .when(!active, |item| {
                    item.hover(move |style| {
                        style.bg(if dark {
                            rgba(0xffffff08)
                        } else {
                            rgb(p.surface)
                        })
                    })
                })
                .child(label);

            if let Some(handler) = select {
                let id = tab.id;
                item = item.on_click(move |_, window, cx| handler(id, window, cx));
            }

            if let Some(handler) = self.on_context.clone() {
                let id = tab.id;
                item = item.on_mouse_down(
                    MouseButton::Right,
                    move |event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        handler(id, event.position, window, cx);
                    },
                );
            }

            let id = tab.id;
            item = item.child(
                div()
                    .id(("close-glass-closable-tab", tab.id))
                    .size(px(20.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .text_color(rgb(p.text_muted))
                    .hover(move |style| {
                        style.bg(if dark {
                            rgba(0xffffff12)
                        } else {
                            rgb(p.surface_subtle)
                        })
                    })
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        if let Some(handler) = close.clone() {
                            handler(id, window, cx);
                        }
                    })
                    .child(Icon::Close.render(11.)),
            );
            strip = strip.child(item);
        }

        if let Some(handler) = self.on_add {
            strip = strip.child(
                div()
                    .flex_shrink_0()
                    .px_1()
                    .h(px(40.))
                    .flex()
                    .items_center()
                    .child(
                        IconButton::new(Icon::Plus, "新建标签页")
                            .theme(self.theme)
                            .render()
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                handler(window, cx);
                            }),
                    ),
            );
        }

        let mut card = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .when(self.fill_height, |card| {
                card.h_full().flex_1().min_h_0().overflow_hidden()
            })
            .when(dark, |card| {
                card.rounded(px(12.))
                    .border_1()
                    .border_color(rgba(0xffffff14))
                    .backdrop_blur(px(24.))
            });
        card = card.child(strip);
        let mut panel = div()
            .w_full()
            .min_h(px(96.))
            .rounded_b(px(12.))
            .bg(if dark {
                rgba(0xffffff0b)
            } else {
                rgb(p.surface)
            })
            .p(px(self.content_padding))
            .when(self.fill_height, |panel| {
                panel
                    .flex_1()
                    .h(px(0.))
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
            })
            .text_size(px(13.))
            .text_color(rgb(p.text));
        if let Some(content) = active_content {
            panel = panel.child(content);
        }
        card.child(panel)
    }
}

impl IntoElement for ClosableTabs {
    type Element = gpui::Div;

    fn into_element(self) -> Self::Element {
        self.render()
    }
}
