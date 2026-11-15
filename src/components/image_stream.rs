//! 连续图片阅读：只为视口相交的页加载、解码图片，离屏即释放纹理。
use std::{collections::HashSet, ops::Range};

use super::{ImageItem, Scrollbar, ScrollbarState};
use crate::theme::ThemePreset;
use gpui::StyledImage;
use gpui::{
    Context, Entity, EventEmitter, ImageSource, InteractiveElement, IntoElement, ObjectFit,
    ParentElement, Render, Resource, RetainAllImageCache, ScrollHandle, StatefulInteractiveElement,
    Styled, Window, div, img, point, px, rgb,
};

#[derive(Clone, Debug)]
pub enum ImageStreamEvent {
    PageChanged(usize),
}

#[derive(Default)]
struct Geometry {
    tops: Vec<f32>,
}
impl Geometry {
    fn new(width: f32, dimensions: &[Option<(f32, f32)>]) -> Self {
        let mut tops = Vec::with_capacity(dimensions.len() + 1);
        tops.push(0.);
        for dimension in dimensions {
            let ratio = dimension.map_or(1.4, |(w, h)| h / w.max(1.));
            tops.push(tops.last().copied().unwrap() + (width * ratio).max(1.));
        }
        Self { tops }
    }
    fn height(&self) -> f32 {
        self.tops.last().copied().unwrap_or(0.)
    }
    fn page(&self, top: f32) -> usize {
        self.tops
            .partition_point(|y| *y <= top)
            .saturating_sub(1)
            .min(self.tops.len().saturating_sub(2))
    }
    fn visible(&self, top: f32, height: f32) -> Range<usize> {
        if height <= 0. || self.tops.len() < 2 {
            return 0..0;
        }
        let start = self.page(top.max(0.));
        let end = self
            .tops
            .partition_point(|y| *y < top + height)
            .min(self.tops.len() - 1);
        start..end.max(start)
    }
    fn anchor(&self, top: f32) -> (usize, f32) {
        let index = self.page(top);
        let start = self.tops.get(index).copied().unwrap_or(0.);
        let end = self.tops.get(index + 1).copied().unwrap_or(start + 1.);
        (index, ((top - start) / (end - start).max(1.)).clamp(0., 1.))
    }
    fn anchor_top(&self, (index, fraction): (usize, f32)) -> f32 {
        let start = self.tops.get(index).copied().unwrap_or(0.);
        let end = self.tops.get(index + 1).copied().unwrap_or(start);
        start + fraction * (end - start)
    }
}

/// 视口之外还要留住的页数。
///
/// 虚拟列表只渲染视口内的页，但「只留视口」会让往回滚一页就要重新取图、往下滚一屏
/// 就出现空白。这里给视口加一圈缓冲：`before` 页已经看过的先不释放纹理，
/// `after` 页在滚到之前就取回来解好。两个数值都是设置项。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamWindow {
    /// 视口上方保留的页数。
    pub before: usize,
    /// 视口下方预加载的页数。
    pub after: usize,
}

impl Default for StreamWindow {
    fn default() -> Self {
        Self {
            before: 5,
            after: 5,
        }
    }
}

impl StreamWindow {
    /// 把视口范围向两侧扩成「保留窗口」。
    ///
    /// 空视口（还没量到尺寸、或者窗口被折叠）不预载任何页：否则一进阅读页
    /// 就会在没有可见位置的情况下先去取图。
    pub fn expand(self, visible: Range<usize>, total: usize) -> Range<usize> {
        if visible.is_empty() {
            return 0..0;
        }
        let start = visible.start.min(total);
        let start = start.saturating_sub(self.before);
        let end = visible.end.saturating_add(self.after).min(total);
        start..end.max(start)
    }
}

pub struct ImageStream {
    items: Vec<ImageItem>,
    dimensions: Vec<Option<(f32, f32)>>,
    geometry: Geometry,
    width: f32,
    height: f32,
    scroll: ScrollHandle,
    scrollbar: ScrollbarState,
    cache: Entity<RetainAllImageCache>,
    retained: HashSet<Resource>,
    failed: HashSet<usize>,
    theme: ThemePreset,
    page: usize,
    pending_page: Option<usize>,
    window: StreamWindow,
}
impl EventEmitter<ImageStreamEvent> for ImageStream {}
impl ImageStream {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            items: vec![],
            dimensions: vec![],
            geometry: Geometry::default(),
            width: 0.,
            height: 0.,
            scroll: ScrollHandle::new(),
            scrollbar: ScrollbarState::default(),
            cache: RetainAllImageCache::new(cx),
            retained: HashSet::new(),
            failed: HashSet::new(),
            theme: ThemePreset::default(),
            page: 0,
            pending_page: None,
            window: StreamWindow::default(),
        }
    }
    /// 更新保留窗口。设置页改数值时立刻生效，不必重开这一话。
    pub fn set_window(&mut self, window: StreamWindow, cx: &mut Context<Self>) {
        if self.window != window {
            self.window = window;
            cx.notify();
        }
    }
    /// Replace pages. Stable ids and optional dimensions are independent of URLs.
    pub fn set_items(&mut self, items: Vec<ImageItem>, page: usize, cx: &mut Context<Self>) {
        self.dimensions = items.iter().map(|item| item.dimensions).collect();
        self.items = items;
        self.geometry = Geometry::new(self.width, &self.dimensions);
        self.cache = RetainAllImageCache::new(cx);
        self.retained.clear();
        self.failed.clear();
        self.pending_page = Some(page.min(self.items.len().saturating_sub(1)));
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }
    /// Convenience for URL and embedded-asset strings. Use `set_items` for typed filesystem paths.
    pub fn set_sources(&mut self, sources: Vec<String>, page: usize, cx: &mut Context<Self>) {
        self.set_items(
            sources
                .into_iter()
                .enumerate()
                .map(|(index, source)| ImageItem::new(index.to_string(), source))
                .collect(),
            page,
            cx,
        );
    }
    pub fn scroll_to_id(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(index) = self.items.iter().position(|item| item.id.as_ref() == id) {
            self.scroll_to(index, cx);
        }
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }
    pub fn scroll_to(&mut self, page: usize, cx: &mut Context<Self>) {
        self.pending_page = Some(page.min(self.items.len().saturating_sub(1)));
        cx.notify();
    }
    fn reflow(&mut self, width: f32, height: f32) {
        let anchor = self.geometry.anchor(-f32::from(self.scroll.offset().y));
        self.width = width;
        self.height = height;
        self.geometry = Geometry::new(width, &self.dimensions);
        let top = self
            .geometry
            .anchor_top(anchor)
            .min((self.geometry.height() - height).max(0.));
        self.scroll.set_offset(point(px(0.), px(-top)));
    }
}
// GPUI's encoded-image decoder is private; use the public image conversion API.
#[derive(Clone)]
struct StreamImageDecoder;
impl gpui::Asset for StreamImageDecoder {
    type Source = std::sync::Arc<gpui::Image>;
    type Output = Result<std::sync::Arc<gpui::RenderImage>, gpui::ImageCacheError>;
    fn load(
        source: Self::Source,
        cx: &mut gpui::App,
    ) -> impl std::future::Future<Output = Self::Output> + Send + 'static {
        let renderer = cx.svg_renderer();
        async move { source.to_image_data(renderer).map_err(Into::into) }
    }
}

impl Render for ImageStream {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.theme.palette();
        if self.width > 0. && self.height > 0. {
            if let Some(page) = self.pending_page.take() {
                let top = self.geometry.tops.get(page).copied().unwrap_or(0.);
                self.scroll.set_offset(point(
                    px(0.),
                    px(-top.min((self.geometry.height() - self.height).max(0.))),
                ));
            }
        }
        let top = -f32::from(self.scroll.offset().y);
        let visible = if self.width > 0. {
            self.geometry.visible(top, self.height)
        } else {
            0..0
        };
        // 渲染仍然是「只画视口内的页」（虚拟列表的收益在这里），
        // 但纹理与预取按视口两侧的缓冲窗口来算。
        let wanted = self.window.expand(visible.clone(), self.items.len());
        let resources: HashSet<_> = wanted
            .clone()
            .filter_map(|index| match &self.items[index].source {
                ImageSource::Resource(resource) => Some(resource.clone()),
                _ => None,
            })
            .collect();
        for resource in self.retained.difference(&resources) {
            self.cache
                .update(cx, |cache, cx| cache.remove(resource, window, cx));
        }
        self.retained = resources;
        let mut content = div()
            .relative()
            .w_full()
            .h(px(self.geometry.height()))
            .flex_shrink_0();
        for index in visible {
            let source = self.items[index].source.clone();
            let mut row = div()
                .id(self.items[index].id.clone())
                .absolute()
                .top(px(self.geometry.tops[index]))
                .left_0()
                .w_full()
                .h(px(self.geometry.tops[index + 1] - self.geometry.tops[index]))
                .overflow_hidden()
                .debug_selector(move || format!("stream-page-{index}"));
            if self.failed.contains(&index) {
                row = row
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(12.))
                    .text_color(rgb(palette.text_muted))
                    .child(
                        div()
                            .id(("stream-retry", index))
                            .px_3()
                            .py_2()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.failed.remove(&index);
                                if let ImageSource::Resource(resource) = &source {
                                    this.cache
                                        .update(cx, |cache, cx| cache.remove(resource, window, cx));
                                } else {
                                    source.remove_asset(cx);
                                }
                                cx.notify();
                            }))
                            .child(format!("第 {} 页加载失败，点击重试", index + 1)),
                    );
            } else {
                row = row.child(
                    img(source)
                        .image_cache(&self.cache)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .with_loading(move || {
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(12.))
                                .text_color(rgb(palette.text_muted))
                                .child(format!("正在加载第 {} 页…", index + 1))
                                .into_any_element()
                        }),
                );
            }
            content = content.child(row);
        }
        let offset = self.scroll.offset();
        let entity = cx.entity();
        let viewport = div()
            .on_children_prepainted(move |_, window, cx| {
                entity.update(cx, |this, cx| {
                    let bounds = this.scroll.bounds();
                    let width = f32::from(bounds.size.width).round();
                    let height = f32::from(bounds.size.height);
                    let resized = width != this.width || height != this.height;
                    if resized {
                        this.reflow(width, height);
                    }
                    // 在布局阶段取缓存结果，GPUI 会把加载完成通知发给正确的阅读视图。
                    let mut changed = false;
                    if !resized && this.pending_page.is_none() && this.scroll.offset() == offset {
                        let visible = this
                            .geometry
                            .visible(-f32::from(this.scroll.offset().y), height);
                        // 同上：取图按缓冲窗口来。预取到的页顺带把真实尺寸量回来，
                        // 滚到它时不用再改排版（否则会看到一次跳动）。
                        let wanted = this.window.expand(visible, this.items.len());
                        for index in wanted {
                            let source = &this.items[index].source;
                            let image = match source {
                                ImageSource::Resource(resource) => this
                                    .cache
                                    .update(cx, |cache, cx| cache.load(resource, window, cx)),
                                ImageSource::Render(image) => Some(Ok(image.clone())),
                                ImageSource::Custom(load) => load(window, cx),
                                ImageSource::Image(image) => {
                                    window.use_asset::<StreamImageDecoder>(image, cx)
                                }
                            };
                            match image {
                                Some(Ok(image)) => {
                                    let size = image.size(0);
                                    let dimensions = (size.width.0 as f32, size.height.0 as f32);
                                    if dimensions.0 > 0.
                                        && dimensions.1 > 0.
                                        && this.dimensions[index] != Some(dimensions)
                                    {
                                        this.dimensions[index] = Some(dimensions);
                                        changed = true;
                                    }
                                }
                                Some(Err(_)) => {
                                    changed |= this.failed.insert(index);
                                }
                                None => {}
                            }
                        }
                        if changed {
                            this.reflow(width, height);
                        }
                    }
                    let page = this.geometry.page(-f32::from(this.scroll.offset().y));
                    if this.pending_page.is_none() && page != this.page {
                        this.page = page;
                        let weak = cx.entity().downgrade();
                        cx.defer(move |cx| {
                            let _ = weak
                                .update(cx, |_, cx| cx.emit(ImageStreamEvent::PageChanged(page)));
                        });
                    }
                    if resized || changed || this.scroll.offset() != offset {
                        let id = cx.entity_id();
                        cx.defer(move |cx| cx.notify(id));
                    }
                });
            })
            .id("image-stream-scroll")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(content);
        div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .overflow_hidden()
            .bg(rgb(palette.canvas))
            .debug_selector(|| "image-stream".into())
            .child(viewport)
            .child(
                Scrollbar::vertical(&self.scroll, &self.scrollbar)
                    .theme(self.theme)
                    .render(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct CountingImages {
        calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
        bytes: Vec<u8>,
    }
    impl gpui::http_client::HttpClient for CountingImages {
        fn get(
            &self,
            url: &str,
            _: bool,
        ) -> futures::future::BoxFuture<'static, anyhow::Result<gpui::http_client::HttpResponse>>
        {
            self.calls.lock().unwrap().push(url.to_owned());
            let body = self.bytes.clone();
            Box::pin(async move {
                Ok(gpui::http_client::HttpResponse {
                    status: reqwest::StatusCode::OK,
                    body,
                })
            })
        }
    }
    fn settle(window: &mut gpui::VisualTestContext, stream: &Entity<ImageStream>, page: usize) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            window.run_until_parked();
            window.update(|window, cx| {
                window.simulate_next_frame(cx);
            });
            window.run_until_parked();
            if stream.read_with(window, |stream, _| stream.dimensions[page].is_some()) {
                window.run_until_parked();
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "page={page} state={:?}",
                stream.read_with(window, |s, cx| (
                    s.width,
                    s.height,
                    s.page,
                    s.pending_page,
                    s.cache.read(cx).len(),
                    s.scroll.offset(),
                    s.geometry.height(),
                    s.dimensions.get(page).copied()
                ))
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    #[gpui::test]
    fn ten_thousand_pages_only_load_the_viewport_and_release_offscreen_textures(
        cx: &mut gpui::TestAppContext,
    ) {
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(100, 200))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (stream, window) = cx.add_window_view(|_, cx| {
            cx.set_http_client(std::sync::Arc::new(CountingImages {
                calls: calls.clone(),
                bytes: png.into_inner(),
            }));
            let mut stream = ImageStream::new(cx);
            // 此用例验证零预载窗口，独立于应用默认的前后缓冲配置。
            stream.set_window(
                StreamWindow {
                    before: 0,
                    after: 0,
                },
                cx,
            );
            stream.set_sources(
                (0..10_000)
                    .map(|index| format!("http://images.test/{index}.png"))
                    .collect(),
                0,
                cx,
            );
            stream
        });
        window.simulate_resize(gpui::size(px(600.), px(500.)));
        settle(window, &stream, 0);
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            ["http://images.test/0.png"]
        );
        assert!(window.debug_bounds("stream-page-0").is_some());
        assert!(window.debug_bounds("stream-page-1").is_none());
        stream.update(window, |stream, cx| stream.scroll_to(5000, cx));
        settle(window, &stream, 5000);
        assert!(window.debug_bounds("stream-page-0").is_none());
        assert!(window.debug_bounds("stream-page-5000").is_some());
        assert_eq!(calls.lock().unwrap().len(), 2, "不能加载跳过的页面");
        stream.read_with(window, |stream, cx| {
            assert_eq!(stream.cache.read(cx).len(), 1, "离屏纹理须释放");
            assert_eq!(stream.page, 5000);
        });
        window.simulate_resize(gpui::size(px(400.), px(500.)));
        window.run_until_parked();
        stream.read_with(window, |stream, _| {
            assert_eq!(stream.page, 5000, "调整窗口保留阅读位置")
        });
        assert_eq!(calls.lock().unwrap().len(), 2, "调整宽度不重新取图");
    }

    #[gpui::test]
    fn stable_ids_and_dimension_hints_support_non_resource_images(cx: &mut gpui::TestAppContext) {
        let image = std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::new(10, 20),
        )]));
        let (stream, window) = cx.add_window_view(|_, cx| {
            let mut stream = ImageStream::new(cx);
            stream.set_items(
                (0..3)
                    .map(|index| {
                        ImageItem::new(format!("page-{index}"), image.clone()).dimensions(10, 20)
                    })
                    .collect(),
                0,
                cx,
            );
            stream
        });
        window.simulate_resize(gpui::size(px(400.), px(300.)));
        window.run_until_parked();
        stream.update(window, |stream, cx| stream.scroll_to_id("page-2", cx));
        window.run_until_parked();
        assert!(window.debug_bounds("stream-page-2").is_some());
        stream.read_with(window, |stream, _| assert_eq!(stream.page, 2));
    }

    #[test]
    fn visible_range_excludes_touching_edges_and_pages_are_seamless() {
        let geometry = Geometry::new(100., &[Some((100., 200.)), Some((100., 50.)), None]);
        assert_eq!(geometry.tops, vec![0., 200., 250., 390.]);
        assert_eq!(geometry.visible(0., 200.), 0..1);
        assert_eq!(geometry.visible(200., 50.), 1..2);
        assert_eq!(geometry.visible(199., 52.), 0..3);
        assert_eq!(geometry.visible(0., 0.), 0..0);
        let anchor = geometry.anchor(225.);
        let resized = Geometry::new(200., &[Some((100., 200.)), Some((100., 50.)), None]);
        assert_eq!(resized.anchor_top(anchor), 450.);
    }
}
