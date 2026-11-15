//! Responsive shortest-column image gallery. Hosts supply image dimensions so loading
//! never shifts the layout; images are fetched by GPUI's normal image pipeline.
use super::{Icon, Scrollbar, ScrollbarState};
use crate::theme::ThemePreset;
use gpui::{
    Context, EventEmitter, ImageSource, InteractiveElement, IntoElement, ObjectFit, ParentElement,
    Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, StyledImage, Window,
    div, img, px, rgb,
};
use std::collections::HashSet;

#[derive(Clone)]
pub struct ImageMasonryItem {
    pub id: SharedString,
    pub source: ImageSource,
    /// Original image dimensions, used before the image finishes loading.
    pub width: u32,
    pub height: u32,
    pub title: Option<SharedString>,
}

impl ImageMasonryItem {
    pub fn new(
        id: impl Into<SharedString>,
        source: impl Into<ImageSource>,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            width,
            height,
            title: None,
        }
    }
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageMasonryEvent {
    Selected { id: SharedString, index: usize },
}

/// Owns a vertical scroll viewport. Place the entity in a container with a bounded
/// height. IDs must be unique; keep this entity alive to retain scroll position.
pub struct ImageMasonry {
    items: Vec<ImageMasonryItem>,
    theme: ThemePreset,
    min_column_width: f32,
    max_columns: usize,
    gap: f32,
    viewport_width: f32,
    selected: Option<SharedString>,
    loading: bool,
    scroll: ScrollHandle,
    viewport_height: f32,
    overscan: f32,
    geometry: Option<Geometry>,
    pending_anchor: Option<(usize, f32)>,
    pending_scroll: Option<usize>,
    scrollbar: ScrollbarState,
    image_cache: gpui::Entity<gpui::RetainAllImageCache>,
    retained_resources: HashSet<gpui::Resource>,
}

impl ImageMasonry {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            items: vec![],
            theme: ThemePreset::default(),
            min_column_width: 200.,
            max_columns: 6,
            gap: 12.,
            viewport_width: 0.,
            selected: None,
            loading: false,
            scroll: ScrollHandle::new(),
            viewport_height: 0.,
            overscan: 400.,
            geometry: None,
            pending_anchor: None,
            pending_scroll: None,
            scrollbar: ScrollbarState::default(),
            image_cache: gpui::RetainAllImageCache::new(cx),
            retained_resources: HashSet::new(),
        }
    }
    pub fn set_items(&mut self, items: Vec<ImageMasonryItem>, cx: &mut Context<Self>) {
        self.items = items;
        self.geometry = None;
        self.pending_anchor = None;
        self.pending_scroll = None;
        if self
            .selected
            .as_ref()
            .is_some_and(|id| !self.items.iter().any(|item| &item.id == id))
        {
            self.selected = None;
        }
        self.scroll.set_offset(gpui::point(px(0.), px(0.)));
        cx.notify();
    }
    pub fn append_items(
        &mut self,
        items: impl IntoIterator<Item = ImageMasonryItem>,
        cx: &mut Context<Self>,
    ) {
        let start = self.items.len();
        self.items.extend(items);
        if let Some(geometry) = &mut self.geometry {
            geometry.append(&self.items[start..], start, self.gap);
        }
        cx.notify();
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }
    /// Minimum desired column width, maximum column count, and both horizontal and vertical gap.
    pub fn set_layout(
        &mut self,
        min_column_width: f32,
        max_columns: usize,
        gap: f32,
        cx: &mut Context<Self>,
    ) {
        let minimum = finite_or(min_column_width, 200.).max(1.);
        let maximum = max_columns.clamp(1, 64);
        let gap = finite_or(gap, 12.).max(0.);
        if self.min_column_width == minimum && self.max_columns == maximum && self.gap == gap {
            return;
        }
        self.pending_anchor = self.geometry.as_ref().and_then(|geometry| {
            let top = -f32::from(self.scroll.offset().y);
            geometry
                .visible(top, self.viewport_height, 0.)
                .first()
                .map(|&index| (index, top - geometry.places[index].y))
        });
        self.min_column_width = minimum;
        self.max_columns = maximum;
        self.gap = gap;
        self.geometry = None;
        cx.notify();
    }
    /// Extra pixels rendered above and below the viewport. Default: 400.
    pub fn set_overscan(&mut self, pixels: f32, cx: &mut Context<Self>) {
        self.overscan = finite_or(pixels, 400.).max(0.);
        cx.notify();
    }
    /// Scroll directly to an item without constructing the intervening cards.
    pub fn scroll_to(&mut self, id: &str, cx: &mut Context<Self>) {
        self.ensure_geometry();
        if let Some(index) = self.items.iter().position(|item| item.id.as_ref() == id) {
            if self.viewport_width <= 0. || self.viewport_height <= 0. {
                self.pending_scroll = Some(index);
                cx.notify();
                return;
            }
            self.pending_scroll = None;
            let top = self.geometry.as_ref().unwrap().places[index].y;
            let maximum = (self.geometry.as_ref().unwrap().height() - self.viewport_height).max(0.);
            self.scroll
                .set_offset(gpui::point(px(0.), px(-top.min(maximum))));
            cx.notify();
        }
    }
    fn ensure_geometry(&mut self) {
        if self.geometry.is_none() {
            self.geometry = Some(Geometry::new(
                &self.items,
                self.viewport_width,
                self.min_column_width,
                self.max_columns,
                self.gap,
            ));
            if let Some((index, delta)) = self.pending_anchor.take() {
                let geometry = self.geometry.as_ref().unwrap();
                if let Some(place) = geometry.places.get(index) {
                    let maximum = (geometry.height() - self.viewport_height).max(0.);
                    self.scroll.set_offset(gpui::point(
                        px(0.),
                        px(-(place.y + delta).clamp(0., maximum)),
                    ));
                }
            }
        }
        if self.viewport_width > 0. && self.viewport_height > 0. {
            if let Some(index) = self.pending_scroll.take() {
                let geometry = self.geometry.as_ref().unwrap();
                if let Some(place) = geometry.places.get(index) {
                    let maximum = (geometry.height() - self.viewport_height).max(0.);
                    self.scroll
                        .set_offset(gpui::point(px(0.), px(-place.y.min(maximum))));
                }
            }
        }
    }

    pub fn set_selected(&mut self, id: Option<SharedString>, cx: &mut Context<Self>) {
        self.selected = id.filter(|id| self.items.iter().any(|item| &item.id == id));
        cx.notify();
    }
    /// Displays a pending state when empty, or a footer while the host appends another page.
    pub fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        self.loading = loading;
        cx.notify();
    }
    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(item) = self.items.get(index) {
            let id = item.id.clone();
            self.selected = Some(id.clone());
            cx.emit(ImageMasonryEvent::Selected { id, index });
            cx.notify();
        }
    }
}
impl EventEmitter<ImageMasonryEvent> for ImageMasonry {}

#[derive(Clone, Copy, Debug)]
struct Placement {
    x: f32,
    y: f32,
    width: f32,
    image_height: f32,
    height: f32,
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

struct Geometry {
    places: Vec<Placement>,
    columns: Vec<Vec<usize>>,
    bottoms: Vec<f32>,
    column_width: f32,
    gap: f32,
}
impl Geometry {
    fn new(items: &[ImageMasonryItem], width: f32, minimum: f32, maximum: usize, gap: f32) -> Self {
        let width = finite_or(width, 0.).max(0.);
        let gap = finite_or(gap, 12.).max(0.);
        let minimum = finite_or(minimum, 200.).max(1.);
        let columns =
            (((width + gap) / (minimum + gap)).floor() as usize).clamp(1, maximum.clamp(1, 64));
        let column_width = ((width - gap * (columns - 1) as f32) / columns as f32).max(0.);
        let mut geometry = Self {
            places: Vec::with_capacity(items.len()),
            columns: vec![vec![]; columns],
            bottoms: vec![0.; columns],
            column_width,
            gap,
        };
        geometry.append(items, 0, gap);
        geometry
    }
    fn append(&mut self, items: &[ImageMasonryItem], start: usize, gap: f32) {
        for (offset, item) in items.iter().enumerate() {
            let column = (0..self.columns.len())
                .min_by(|&a, &b| self.bottoms[a].total_cmp(&self.bottoms[b]))
                .unwrap();
            let ratio = if item.width == 0 || item.height == 0 {
                1.
            } else {
                item.height as f32 / item.width as f32
            };
            let image_height = self.column_width * ratio;
            let height = image_height + if item.title.is_some() { 34. } else { 0. };
            self.places.push(Placement {
                x: column as f32 * (self.column_width + gap),
                y: self.bottoms[column],
                width: self.column_width,
                image_height,
                height,
            });
            self.columns[column].push(start + offset);
            self.bottoms[column] += height + gap;
        }
    }
    fn height(&self) -> f32 {
        (self.bottoms.iter().copied().fold(0_f32, f32::max) - self.gap).max(0.)
    }
    fn visible(&self, offset: f32, height: f32, overscan: f32) -> Vec<usize> {
        let low = (offset - overscan).max(0.);
        let high = offset + height + overscan;
        let mut visible = Vec::new();
        for column in &self.columns {
            let start = column.partition_point(|&i| self.places[i].y + self.places[i].height < low);
            visible.extend(
                column[start..]
                    .iter()
                    .copied()
                    .take_while(|&i| self.places[i].y <= high),
            );
        }
        visible.sort_unstable();
        visible
    }
}
#[cfg(test)]
fn layout(
    items: &[ImageMasonryItem],
    width: f32,
    minimum: f32,
    maximum: usize,
    gap: f32,
) -> (Vec<Placement>, f32) {
    let geometry = Geometry::new(items, width, minimum, maximum, gap);
    let height = geometry.height();
    (geometry.places, height)
}

fn placeholder(theme: ThemePreset, failed: bool) -> gpui::Div {
    let p = theme.palette();
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .bg(rgb(p.surface_subtle))
        .text_color(rgb(p.text_muted))
        .text_size(px(11.))
        .child(Icon::ImageGroup.render(20.))
        .child(if failed {
            "图片加载失败"
        } else {
            "加载中…"
        })
}

impl Render for ImageMasonry {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let theme = self.theme;
        self.ensure_geometry();
        let paint_offset = self.scroll.offset();
        let geometry = self.geometry.as_ref().unwrap();
        let height = geometry.height();
        let visible = geometry.visible(
            -f32::from(paint_offset.y),
            self.viewport_height,
            self.overscan,
        );
        let resources: HashSet<_> = visible
            .iter()
            .filter_map(|&index| match &self.items[index].source {
                ImageSource::Resource(resource) => Some(resource.clone()),
                _ => None,
            })
            .collect();
        for resource in self.retained_resources.difference(&resources) {
            self.image_cache
                .update(cx, |cache, cx| cache.remove(resource, window, cx));
        }
        self.retained_resources = resources;
        let mut content = div().relative().w_full().h(px(height)).flex_shrink_0();
        if self.viewport_width > 0. {
            for index in visible {
                let item = &self.items[index];
                let place = geometry.places[index];
                let selected = self.selected.as_ref() == Some(&item.id);
                let mut card = div()
                    .id(item.id.clone())
                    .absolute()
                    .left(px(place.x))
                    .top(px(place.y))
                    .w(px(place.width))
                    .h(px(place.height))
                    .rounded(px(10.))
                    .overflow_hidden()
                    .bg(rgb(p.surface))
                    .cursor_pointer()
                    .focusable()
                    .focus_visible(|style| {
                        style.shadow(vec![gpui::BoxShadow {
                            inset: false,
                            color: gpui::rgb_to_hsla(rgb(p.accent)),
                            offset: gpui::point(px(0.), px(0.)),
                            blur_radius: px(0.),
                            spread_radius: px(2.),
                        }])
                    })
                    .debug_selector(move || format!("masonry-item-{index}").into())
                    .hover(|s| s.bg(rgb(p.surface_subtle)))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(index, cx)))
                    .on_key_down(cx.listener(
                        move |this, event: &gpui::KeyDownEvent, window, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                window.prevent_default();
                                cx.stop_propagation();
                                this.select(index, cx);
                            }
                        },
                    ))
                    .child(
                        div()
                            .w_full()
                            .h(px(place.image_height))
                            .overflow_hidden()
                            .child(
                                img(item.source.clone())
                                    .image_cache(&self.image_cache)
                                    .rounded_t(px(10.))
                                    .rounded_b(px(if item.title.is_some() { 0. } else { 10. }))
                                    .size_full()
                                    .object_fit(ObjectFit::Contain)
                                    .with_loading(move || {
                                        placeholder(theme, false).into_any_element()
                                    })
                                    .with_fallback(move || {
                                        placeholder(theme, true).into_any_element()
                                    }),
                            ),
                    );
                if let Some(title) = &item.title {
                    card = card.child(
                        div()
                            .h(px(34.))
                            .px_3()
                            .flex()
                            .items_center()
                            .text_size(px(12.))
                            .text_color(rgb(p.text))
                            .child(div().flex_1().min_w_0().truncate().child(title.clone())),
                    );
                }
                // Overlay selection avoids changing image dimensions or card positions.
                if selected {
                    card = card.child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded(px(10.))
                            .border_1()
                            .border_color(rgb(p.accent)),
                    );
                }
                if selected {
                    card = card.child(
                        div()
                            .absolute()
                            .top(px(8.))
                            .right(px(8.))
                            .size(px(22.))
                            .rounded_full()
                            .bg(rgb(p.accent))
                            .text_color(rgb(0xffffff))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(Icon::CheckCircle.render(14.)),
                    );
                }
                content = content.child(card);
            }
        }
        let entity = cx.entity();
        let mut body = div()
            .on_children_prepainted(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    let bounds = this.scroll.bounds();
                    let width = (f32::from(bounds.size.width) - 12.).max(0.).round();
                    let height = f32::from(bounds.size.height);
                    let width_changed = (this.viewport_width - width).abs() >= 1.;
                    if width_changed
                        || this.viewport_height != height
                        || this.scroll.offset() != paint_offset
                    {
                        this.viewport_width = width;
                        this.viewport_height = height;
                        if width_changed {
                            let top = -f32::from(paint_offset.y);
                            this.pending_anchor = this.geometry.as_ref().and_then(|geometry| {
                                geometry
                                    .visible(top, height, 0.)
                                    .first()
                                    .map(|&index| (index, top - geometry.places[index].y))
                            });
                            this.geometry = None;
                        }
                        let id = cx.entity_id();
                        cx.defer(move |cx| cx.notify(id));
                    }
                });
            })
            .id("image-masonry-scroll")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pr(px(12.))
            .flex()
            .flex_col();
        if self.items.is_empty() {
            body = body.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(rgb(p.text_muted))
                    .child(Icon::ImageGroup.render(28.))
                    .child(if self.loading {
                        "正在加载图片…"
                    } else {
                        "暂无图片"
                    }),
            );
        } else {
            body = body.child(content);
            if self.loading {
                body = body.child(
                    div()
                        .py_4()
                        .w_full()
                        .text_center()
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child("正在加载更多…"),
                );
            }
        }
        div()
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(body)
            .child(
                div().absolute().top_0().bottom_0().right_0().child(
                    Scrollbar::vertical(&self.scroll, &self.scrollbar)
                        .theme(self.theme)
                        .render(),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext;
    fn item(id: &str, width: u32, height: u32) -> ImageMasonryItem {
        ImageMasonryItem::new(
            id.to_owned(),
            std::path::PathBuf::from("fixture.png"),
            width,
            height,
        )
    }
    #[test]
    fn shortest_columns_preserve_ratios_and_leave_no_overlaps() {
        let items = vec![
            item("a", 100, 200),
            item("b", 100, 100),
            item("c", 100, 100),
            item("d", 100, 100).title("Caption"),
        ];
        let (places, height) = layout(&items, 420., 200., 6, 20.);
        assert_eq!(places[0].image_height, 400.);
        assert_eq!((places[1].x, places[1].y), (220., 0.));
        assert_eq!((places[2].x, places[2].y), (220., 220.));
        assert_eq!((places[3].x, places[3].y), (0., 420.));
        assert_eq!(places[3].height, 234.);
        assert_eq!(height, 654.);
        for (index, a) in places.iter().enumerate() {
            assert!(a.x >= 0. && a.x + a.width <= 420.);
            for b in &places[index + 1..] {
                assert!(
                    a.x + a.width <= b.x
                        || b.x + b.width <= a.x
                        || a.y + a.height <= b.y
                        || b.y + b.height <= a.y
                );
            }
        }
    }
    #[test]
    fn resizing_reflows_and_append_retains_existing_positions() {
        let mut items = vec![item("a", 100, 100), item("b", 100, 150), item("c", 0, 0)];
        let (narrow, _) = layout(&items, 160., 200., 6, 12.);
        assert!(narrow.iter().all(|p| p.x == 0. && p.width == 160.));
        assert_eq!(narrow[2].image_height, 160.);
        let (wide, _) = layout(&items, 624., 200., 6, 12.);
        assert_eq!(wide[2].x, 424.);
        items.push(item("d", 200, 120));
        let (appended, _) = layout(&items, 624., 200., 6, 12.);
        for (a, b) in wide.iter().zip(appended.iter()) {
            assert_eq!((a.x, a.y), (b.x, b.y));
        }
        assert_eq!(layout(&[], 0., 200., 6, 12.).1, 0.);
        assert!(
            layout(&items, f32::NAN, f32::INFINITY, 0, f32::NAN)
                .1
                .is_finite()
        );
    }
    #[test]
    fn hundred_thousand_items_use_column_search_and_incremental_append() {
        let mut items: Vec<_> = (0..100_000)
            .map(|i| {
                item(
                    &format!("image-{i}"),
                    100,
                    if i == 0 { 5000 } else { 80 + (i % 6) * 30 },
                )
            })
            .collect();
        let mut geometry = Geometry::new(&items, 1000., 200., 6, 12.);
        for offset in [0., 5000., 500_000., geometry.height() - 600.] {
            let visible = geometry.visible(offset, 600., 300.);
            assert!(visible.len() < 80, "only viewport cards should be built");
            let low = (offset - 300.).max(0.);
            let high = offset + 900.;
            let expected: Vec<_> = geometry
                .places
                .iter()
                .enumerate()
                .filter(|(_, p)| p.y + p.height >= low && p.y <= high)
                .map(|(i, _)| i)
                .collect();
            assert_eq!(visible, expected);
        }
        let old_height = geometry.height();
        let first = geometry.places[0];
        items.push(item("appended", 100, 120));
        geometry.append(&items[100_000..], 100_000, 12.);
        assert_eq!(geometry.places.len(), 100_001);
        assert_eq!(
            (geometry.places[0].x, geometry.places[0].y),
            (first.x, first.y)
        );
        assert!(geometry.height() >= old_height);
    }

    #[gpui::test]
    fn virtualized_images_load_only_near_viewport_and_large_scrolls_update(
        cx: &mut gpui::TestAppContext,
    ) {
        let loaded = std::rc::Rc::new(std::cell::RefCell::new(HashSet::new()));
        let seen = loaded.clone();
        let (gallery, window) = cx.add_window_view(|_, cx| {
            let mut gallery = ImageMasonry::new(cx);
            let pixels = std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
                image::RgbaImage::from_pixel(8, 8, image::Rgba([80, 120, 160, 255])),
            )]));
            gallery.set_items(
                (0..10_000)
                    .map(|index| {
                        let seen = seen.clone();
                        let pixels = pixels.clone();
                        ImageMasonryItem::new(
                            format!("image-{index}"),
                            move |_: &mut Window, _: &mut gpui::App| {
                                seen.borrow_mut().insert(index);
                                Some(Ok(pixels.clone()))
                            },
                            8,
                            8,
                        )
                    })
                    .collect(),
                cx,
            );
            gallery
        });
        window.simulate_resize(gpui::size(px(640.), px(600.)));
        window.run_until_parked();
        assert!(loaded.borrow().len() < 40);
        assert!(!loaded.borrow().contains(&9999));
        assert!(window.debug_bounds("masonry-item-9999").is_none());
        gallery.update(window, |gallery, cx| gallery.scroll_to("image-9999", cx));
        window.run_until_parked();
        assert!(window.debug_bounds("masonry-item-9999").is_some());
        assert!(window.debug_bounds("masonry-item-0").is_none());
        assert!(loaded.borrow().contains(&9999));
        assert!(loaded.borrow().len() < 80);
        let anchor = gallery.read_with(window, |gallery, _| {
            let geometry = gallery.geometry.as_ref().unwrap();
            geometry.visible(
                -f32::from(gallery.scroll.offset().y),
                gallery.viewport_height,
                0.,
            )[0]
        });
        window.simulate_resize(gpui::size(px(320.), px(600.)));
        window.run_until_parked();
        assert!(
            window
                .debug_bounds(Box::leak(format!("masonry-item-{anchor}").into_boxed_str()))
                .is_some()
        );
        gallery.update(window, |gallery, cx| gallery.scroll_to("image-0", cx));
        window.run_until_parked();
        window.simulate_event(gpui::ScrollWheelEvent {
            position: gpui::point(px(100.), px(200.)),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-5000.))),
            ..Default::default()
        });
        window.run_until_parked();
        assert!(window.debug_bounds("masonry-item-0").is_none());
        gallery.read_with(window, |gallery, _| {
            assert!(gallery.scroll.offset().y < px(-1000.))
        });
    }

    #[gpui::test]
    fn scroll_before_first_layout_targets_the_requested_item(cx: &mut gpui::TestAppContext) {
        let (gallery, window) = cx.add_window_view(|_, cx| {
            let mut gallery = ImageMasonry::new(cx);
            let source = std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
                image::RgbaImage::from_pixel(8, 8, image::Rgba([80, 120, 160, 255])),
            )]));
            gallery.set_layout(200., 6, 0., cx);
            gallery.set_items(
                (0..1000)
                    .map(|index| {
                        ImageMasonryItem::new(format!("image-{index}"), source.clone(), 8, 8)
                    })
                    .collect(),
                cx,
            );
            gallery.scroll_to("image-999", cx);
            gallery
        });
        window.run_until_parked();
        let state = gallery.read_with(window, |gallery, _| {
            format!(
                "viewport={}x{}, offset={:?}, max={:?}, pending={:?}, last={:?}",
                gallery.viewport_width,
                gallery.viewport_height,
                gallery.scroll.offset(),
                gallery.scroll.max_offset(),
                gallery.pending_scroll,
                gallery.geometry.as_ref().unwrap().places.last()
            )
        });
        assert!(window.debug_bounds("masonry-item-999").is_some(), "{state}");
        assert!(window.debug_bounds("masonry-item-0").is_none());
        gallery.read_with(window, |gallery, _| {
            assert!(gallery.pending_scroll.is_none())
        });
    }

    #[gpui::test]
    fn actual_viewport_resize_reflows_and_click_emits_stable_id(cx: &mut gpui::TestAppContext) {
        let (gallery, window) = cx.add_window_view(|_, cx| {
            let mut gallery = ImageMasonry::new(cx);
            let source = std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
                image::RgbaImage::from_pixel(8, 8, image::Rgba([80, 120, 160, 255])),
            )]));
            gallery.set_items(
                (0..4)
                    .map(|i| {
                        ImageMasonryItem::new(format!("image-{i}"), source.clone(), 8, 8)
                            .title(format!("图 {i}"))
                    })
                    .collect(),
                cx,
            );
            gallery
        });
        let received = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let events = received.clone();
        let _observer = window.new(|cx: &mut Context<()>| {
            cx.subscribe(&gallery, move |_, _, event: &ImageMasonryEvent, _| {
                events.borrow_mut().push(event.clone())
            })
            .detach();
        });
        window.simulate_resize(gpui::size(px(640.), px(600.)));
        window.run_until_parked();
        let first = window.debug_bounds("masonry-item-0").unwrap();
        let second = window.debug_bounds("masonry-item-1").unwrap();
        assert!(second.left() > first.left());
        window.simulate_click(second.center(), Default::default());
        window.run_until_parked();
        assert_eq!(
            *received.borrow(),
            vec![ImageMasonryEvent::Selected {
                id: "image-1".into(),
                index: 1
            }]
        );
        window.simulate_resize(gpui::size(px(180.), px(600.)));
        window.run_until_parked();
        let first = window.debug_bounds("masonry-item-0").unwrap();
        let second = window.debug_bounds("masonry-item-1").unwrap();
        assert_eq!(first.left(), second.left());
        assert!(second.top() >= first.bottom() + px(12.));
        assert!(second.right() <= px(180.));
    }
}
