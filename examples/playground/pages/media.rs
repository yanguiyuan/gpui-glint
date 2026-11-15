use gpui::{
    Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div, prelude::*, px, rgb,
};
use gpui_glint::{
    components::{
        Icon, ImageItem, ImageStream, ImageViewer, NavigationItem, NavigationRail, ThumbnailItem,
        ThumbnailList, ThumbnailListEvent,
    },
    theme::ThemePreset,
};
use std::sync::Arc;

/// All images are local memory sources: the example needs no server or credentials.
pub struct MediaPage {
    theme: ThemePreset,
    items: Vec<ImageItem>,
    thumbnails: Entity<ThumbnailList>,
    stream: Entity<ImageStream>,
    selected: usize,
    continuous: bool,
}
impl MediaPage {
    pub fn new(theme: ThemePreset, cx: &mut Context<Self>) -> Self {
        let items: Vec<_> = (0..120)
            .map(|index| {
                let (width, height) = if index % 3 == 0 {
                    (240, 160)
                } else {
                    (160, 240)
                };
                let color = match index % 3 {
                    0 => [92, 126, 151, 255],
                    1 => [133, 150, 123, 255],
                    _ => [173, 142, 112, 255],
                };
                let image = Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
                    image::RgbaImage::from_pixel(width, height, image::Rgba(color)),
                )]));
                ImageItem::new(index.to_string(), image).dimensions(width, height)
            })
            .collect();
        let thumbnails = cx.new(ThumbnailList::new);
        thumbnails.update(cx, |list, cx| {
            list.set_theme(theme, cx);
            list.set_items(
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        ThumbnailItem::new(item.id.clone(), format!("第 {} 页", index + 1))
                            .subtitle("本地图片 · 无网络请求")
                            .source(item.source.clone())
                    })
                    .collect(),
                cx,
            );
            list.set_selected(Some("0".into()), cx);
        });
        let stream = cx.new(ImageStream::new);
        stream.update(cx, |stream, cx| {
            stream.set_theme(theme, cx);
            stream.set_items(items.clone(), 0, cx);
        });
        cx.subscribe(&thumbnails, |this, _, event: &ThumbnailListEvent, cx| {
            if let ThumbnailListEvent::Selected(id) = event {
                if let Ok(index) = id.parse::<usize>() {
                    this.selected = index;
                    this.thumbnails
                        .update(cx, |list, cx| list.set_selected(Some(id.clone()), cx));
                    this.stream
                        .update(cx, |stream, cx| stream.scroll_to(index, cx));
                    cx.notify();
                }
            }
        })
        .detach();
        Self {
            theme,
            items,
            thumbnails,
            stream,
            selected: 0,
            continuous: false,
        }
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        self.thumbnails
            .update(cx, |list, cx| list.set_theme(theme, cx));
        self.stream
            .update(cx, |stream, cx| stream.set_theme(theme, cx));
        cx.notify();
    }
}
impl Render for MediaPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.theme.palette();
        let rail = NavigationRail::new([NavigationItem::new(
            "media-images",
            "图片阅读",
            Icon::ImageGroup,
        )])
        .footer([NavigationItem::new(
            "media-mode",
            "切换单页 / 连续模式",
            Icon::Refresh,
        )])
        .selected("media-images")
        .theme(self.theme)
        .on_select(cx.listener(|this, id: &gpui::SharedString, _, cx| {
            if id.as_ref() == "media-mode" {
                this.continuous = !this.continuous;
                cx.notify();
            }
        }))
        .render(cx);
        let content = if self.continuous {
            self.stream.clone().into_any_element()
        } else {
            ImageViewer::from_item(self.items[self.selected].clone())
                .theme(self.theme)
                .padding(20.)
                .render()
        };
        div()
            .size_full()
            .min_h_0()
            .flex()
            .bg(rgb(palette.canvas))
            .child(div().w(px(46.)).flex_shrink_0().child(rail))
            .child(
                div()
                    .w(px(200.))
                    .min_h_0()
                    .flex_shrink_0()
                    .child(self.thumbnails.clone()),
            )
            .child(div().flex_1().min_w_0().min_h_0().child(content))
    }
}
