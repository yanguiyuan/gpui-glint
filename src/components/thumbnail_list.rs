//! A virtual, selectable list for media and document thumbnails.
use crate::theme::ThemePreset;
use gpui::{
    Context, EventEmitter, ImageSource, IntoElement, ObjectFit, ParentElement, Render,
    SharedString, Styled, StyledImage, UniformListScrollHandle, Window, div, img, prelude::*, px,
    rgb,
};

#[derive(Clone)]
pub struct ThumbnailItem {
    pub id: SharedString,
    pub title: SharedString,
    pub subtitle: Option<SharedString>,
    pub source: Option<ImageSource>,
    pub placeholder: SharedString,
    pub trailing: Option<SharedString>,
}
impl ThumbnailItem {
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            subtitle: None,
            source: None,
            placeholder: "".into(),
            trailing: None,
        }
    }
    pub fn source(mut self, source: impl Into<ImageSource>) -> Self {
        self.source = Some(source.into());
        self
    }
    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }
    pub fn trailing(mut self, trailing: impl Into<SharedString>) -> Self {
        self.trailing = Some(trailing.into());
        self
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThumbnailListEvent {
    Selected(SharedString),
    LoadMore,
}

pub struct ThumbnailList {
    items: Vec<ThumbnailItem>,
    selected: Option<SharedString>,
    theme: ThemePreset,
    scroll: UniformListScrollHandle,
    has_more: bool,
    requested: bool,
    row_height: f32,
}
impl EventEmitter<ThumbnailListEvent> for ThumbnailList {}
impl ThumbnailList {
    pub fn new(_: &mut Context<Self>) -> Self {
        Self {
            items: vec![],
            selected: None,
            theme: ThemePreset::default(),
            scroll: UniformListScrollHandle::default(),
            has_more: false,
            requested: false,
            row_height: 52.,
        }
    }
    pub fn set_items(&mut self, items: Vec<ThumbnailItem>, cx: &mut Context<Self>) {
        self.items = items;
        self.requested = false;
        cx.notify();
    }
    pub fn append_items(&mut self, items: Vec<ThumbnailItem>, cx: &mut Context<Self>) {
        self.items.extend(items);
        self.requested = false;
        cx.notify();
    }
    pub fn set_selected(&mut self, id: Option<SharedString>, cx: &mut Context<Self>) {
        self.selected = id;
        cx.notify();
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }
    pub fn set_scroll_handle(&mut self, handle: UniformListScrollHandle, cx: &mut Context<Self>) {
        self.scroll = handle;
        cx.notify();
    }
    pub fn set_has_more(&mut self, has_more: bool, cx: &mut Context<Self>) {
        if self.has_more != has_more {
            self.has_more = has_more;
            self.requested = false;
            cx.notify();
        }
    }
    pub fn scroll_to_id(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(index) = self.items.iter().position(|item| item.id.as_ref() == id) {
            self.scroll.scroll_to_item(index, gpui::ScrollStrategy::Top);
            cx.notify();
        }
    }
    fn row(&self, index: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let item = &self.items[index];
        let p = self.theme.palette();
        let selected = self.selected.as_ref() == Some(&item.id);
        let id = item.id.clone();
        let selector = format!("thumbnail-{}", item.id);
        let placeholder = item.placeholder.clone();
        let fallback = move || {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(8.))
                .text_color(rgb(p.text_muted))
                .child(placeholder.clone())
                .into_any_element()
        };
        let thumbnail = match &item.source {
            Some(source) => img(source.clone())
                .size_full()
                .rounded(px(4.))
                .object_fit(ObjectFit::Cover)
                .with_fallback(fallback)
                .into_any_element(),
            None => fallback(),
        };
        div()
            .id(item.id.clone())
            .debug_selector(move || selector.clone())
            .w_full()
            .min_w_0()
            .h(px(self.row_height))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .py(px(7.))
            .rounded(px(8.))
            .cursor_pointer()
            .when(selected, |row| row.bg(rgb(p.accent_tint)))
            .hover(|row| row.bg(rgb(p.surface)))
            .on_click(
                cx.listener(move |_, _, _, cx| cx.emit(ThumbnailListEvent::Selected(id.clone()))),
            )
            .child(
                div()
                    .w(px(28.))
                    .h(px(38.))
                    .flex_shrink_0()
                    .rounded(px(4.))
                    .bg(rgb(p.surface_subtle))
                    .child(thumbnail),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(if selected { p.accent } else { p.text }))
                            .child(item.title.clone()),
                    )
                    .children(item.subtitle.clone().map(|text| {
                        div()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .child(text)
                    })),
            )
            .children(item.trailing.clone().map(|text| {
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child(text)
            }))
            .into_any_element()
    }
}
impl Render for ThumbnailList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        gpui::uniform_list(
            "thumbnail-list",
            self.items.len(),
            cx.processor(|this: &mut Self, range: std::ops::Range<usize>, _, cx| {
                if this.has_more
                    && !this.requested
                    && range.end.saturating_add(6) >= this.items.len()
                {
                    this.requested = true;
                    let weak = cx.entity().downgrade();
                    cx.defer(move |cx| {
                        let _ = weak.update(cx, |_, cx| cx.emit(ThumbnailListEvent::LoadMore));
                    });
                }
                range.map(|index| this.row(index, cx)).collect::<Vec<_>>()
            }),
        )
        .size_full()
        .min_h_0()
        .track_scroll(&self.scroll)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn virtualizes_images_and_emits_selection_and_one_pagination_request(
        cx: &mut gpui::TestAppContext,
    ) {
        let seen = std::rc::Rc::new(std::cell::RefCell::new(std::collections::HashSet::new()));
        let source = std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::new(4, 4),
        )]));
        let (list, window) = cx.add_window_view(|_, cx| {
            let mut list = ThumbnailList::new(cx);
            list.set_items(
                (0..2240)
                    .map(|index| {
                        let seen = seen.clone();
                        let source = source.clone();
                        ThumbnailItem::new(format!("item-{index}"), format!("Item {index}")).source(
                            move |_: &mut Window, _: &mut gpui::App| {
                                seen.borrow_mut().insert(index);
                                Some(Ok(source.clone()))
                            },
                        )
                    })
                    .collect(),
                cx,
            );
            list.set_has_more(true, cx);
            list
        });
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let received = events.clone();
        let _observer = window.new(|cx: &mut Context<()>| {
            cx.subscribe(&list, move |_, _, event: &ThumbnailListEvent, _| {
                received.borrow_mut().push(event.clone())
            })
            .detach();
        });
        window.run_until_parked();
        assert!(
            seen.borrow().len() < 30,
            "offscreen thumbnails must not load"
        );
        let first = window.debug_bounds("thumbnail-item-0").unwrap();
        window.simulate_click(first.center(), gpui::Modifiers::default());
        assert_eq!(
            events.borrow().as_slice(),
            [ThumbnailListEvent::Selected("item-0".into())]
        );
        list.update(window, |list, cx| list.scroll_to_id("item-2239", cx));
        window.run_until_parked();
        assert!(window.debug_bounds("thumbnail-item-2239").is_some());
        assert!(window.debug_bounds("thumbnail-item-0").is_none());
        assert!(seen.borrow().len() < 60);
        list.update(window, |_, cx| cx.notify());
        window.run_until_parked();
        assert_eq!(
            events
                .borrow()
                .iter()
                .filter(|event| matches!(event, ThumbnailListEvent::LoadMore))
                .count(),
            1
        );
    }
}
