use crate::theme::ThemePreset;
use gpui::{IntoElement, ParentElement, Styled, div, prelude::*, px, rgb};
use std::rc::Rc;

/// Reusable pagination footer for tables and query results.
pub struct PaginationBar {
    page: usize,
    page_size: usize,
    total_items: Option<u64>,
    current_count: usize,
    theme: ThemePreset,
    page_sizes: &'static [usize],
    on_page_change: Option<Rc<dyn Fn(usize, &mut gpui::Window, &mut gpui::App)>>,
    on_page_size_change: Option<Rc<dyn Fn(usize, &mut gpui::Window, &mut gpui::App)>>,
}

impl PaginationBar {
    pub fn new(page: usize, page_size: usize, current_count: usize) -> Self {
        Self {
            page: page.max(1),
            page_size: page_size.max(1),
            total_items: None,
            current_count,
            theme: ThemePreset::default(),
            page_sizes: &[25, 50, 100, 200],
            on_page_change: None,
            on_page_size_change: None,
        }
    }

    pub fn total_items(mut self, total: Option<u64>) -> Self {
        self.total_items = total;
        self
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn on_page_change(
        mut self,
        handler: impl Fn(usize, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_page_change = Some(Rc::new(handler));
        self
    }

    pub fn on_page_size_change(
        mut self,
        handler: impl Fn(usize, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_page_size_change = Some(Rc::new(handler));
        self
    }

    pub fn total_pages(&self) -> Option<usize> {
        self.total_items.map(|total| {
            let pages = (total as usize).div_ceil(self.page_size);
            pages.max(1)
        })
    }

    pub fn render(self) -> impl IntoElement {
        let p = self.theme.palette();
        let total_pages = self.total_pages();
        let has_prev = self.page > 1;
        let has_next = match total_pages {
            Some(pages) => self.page < pages,
            None => self.current_count >= self.page_size,
        };
        let summary = match (self.total_items, total_pages) {
            (Some(total), Some(pages)) => {
                format!("共 {total} 条 · 第 {} / {pages} 页", self.page)
            }
            _ => format!("第 {} 页 · 本页 {} 条", self.page, self.current_count),
        };
        let on_page_prev = self.on_page_change.clone();
        let on_page_next = self.on_page_change;
        let page = self.page;

        let mut bar = div()
            .w_full()
            .h(px(36.))
            .flex_shrink_0()
            .px_3()
            .border_t_1()
            .border_color(rgb(p.border))
            .bg(rgb(p.surface))
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .text_size(px(11.))
            .text_color(rgb(p.text_muted))
            .child(div().child(summary));

        let mut controls = div().flex().items_center().gap_2();
        if let Some(on_size) = self.on_page_size_change {
            let mut sizes = div().flex().items_center().gap_1();
            for &size_opt in self.page_sizes {
                let active = size_opt == self.page_size;
                let cb = on_size.clone();
                sizes = sizes.child(
                    div()
                        .id(gpui::SharedString::from(format!("page-size-{size_opt}")))
                        .px_1p5()
                        .py(px(2.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .when(active, |d| {
                            d.bg(rgb(p.accent_tint))
                                .text_color(rgb(p.accent))
                                .font_weight(gpui::FontWeight::MEDIUM)
                        })
                        .when(!active, |d| d.hover(|s| s.bg(rgb(p.surface_subtle))))
                        .child(format!("{size_opt}/页"))
                        .on_click(move |_, window, cx| cb(size_opt, window, cx)),
                );
            }
            controls = controls.child(sizes);
        }

        controls = controls
            .child(
                div()
                    .id("pagination-prev")
                    .debug_selector(|| "pagination-prev".into())
                    .px_2()
                    .py(px(2.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(p.border))
                    .text_color(rgb(p.text))
                    .when(has_prev, |btn| {
                        btn.cursor_pointer()
                            .hover(|s| s.bg(rgb(p.surface_subtle)))
                            .on_click(move |_, window, cx| {
                                if let Some(cb) = &on_page_prev {
                                    cb(page.saturating_sub(1).max(1), window, cx);
                                }
                            })
                    })
                    .when(!has_prev, |btn| btn.opacity(0.4))
                    .child("‹ 上一页"),
            )
            .child(
                div()
                    .id("pagination-next")
                    .debug_selector(|| "pagination-next".into())
                    .px_2()
                    .py(px(2.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(p.border))
                    .text_color(rgb(p.text))
                    .when(has_next, |btn| {
                        btn.cursor_pointer()
                            .hover(|s| s.bg(rgb(p.surface_subtle)))
                            .on_click(move |_, window, cx| {
                                if let Some(cb) = &on_page_next {
                                    cb(page + 1, window, cx);
                                }
                            })
                    })
                    .when(!has_next, |btn| btn.opacity(0.4))
                    .child("下一页 ›"),
            );

        bar = bar.child(controls);
        bar
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_pages_calculates_ceil_with_minimum_one() {
        assert_eq!(PaginationBar::new(1, 50, 0).total_items(Some(0)).total_pages(), Some(1));
        assert_eq!(PaginationBar::new(1, 50, 50).total_items(Some(101)).total_pages(), Some(3));
        assert_eq!(PaginationBar::new(2, 50, 50).total_items(None).total_pages(), None);
    }
}
