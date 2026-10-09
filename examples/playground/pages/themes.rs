use std::rc::Rc;

use gpui::{
    App, InteractiveElement, ParentElement, StatefulInteractiveElement, Styled, Window, div,
    linear_color_stop, linear_gradient, px, rgb, rgba,
};
use gpui_glint::theme::ThemePreset;

use super::common::page;

type ThemeHandler = Rc<dyn Fn(ThemePreset, &mut Window, &mut App)>;

pub struct ThemeShowcase {
    selected: ThemePreset,
    on_select: Option<ThemeHandler>,
}

impl ThemeShowcase {
    pub fn new(selected: ThemePreset) -> Self {
        Self {
            selected,
            on_select: None,
        }
    }

    pub fn on_select(
        mut self,
        handler: impl Fn(ThemePreset, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }

    pub fn render(self) -> gpui::Stateful<gpui::Div> {
        let cards = ThemePreset::ALL.into_iter().fold(
            div().w_full().flex().flex_wrap().gap(px(10.)),
            |cards, theme| cards.child(self.card(theme)),
        );
        let content = div().flex().flex_col().gap_4().child(cards);
        page(
            "06",
            "主题",
            "点击配色卡片预览整页，选择会自动记住。蓝色主操作与状态语义在各套主题中保持一致。",
            content,
            self.selected,
        )
    }

    fn card(&self, theme: ThemePreset) -> gpui::Stateful<gpui::Div> {
        let palette = theme.palette();
        let selected = self.selected == theme;
        let handler = self.on_select.clone();
        let mut card = div()
            .id(theme.id())
            .flex_1()
            .min_w(px(150.))
            .flex()
            .flex_col()
            .gap(px(7.))
            .p(px(10.))
            .rounded(px(12.))
            .border_1()
            .border_color(if selected {
                rgb(palette.accent)
            } else {
                rgb(palette.border)
            })
            .bg(rgb(palette.panel))
            .hover(move |style| style.bg(rgba(0xffffff75)))
            .cursor_pointer()
            .child(preview(theme))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child(theme.name()),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(px(16.5))
                    .text_color(rgb(palette.text_muted))
                    .child(theme.description()),
            );
        if let Some(handler) = handler {
            card = card.on_click(move |_, window, cx| handler(theme, window, cx));
        }
        card
    }
}

fn preview(theme: ThemePreset) -> gpui::Div {
    let palette = theme.palette();
    div()
        .w_full()
        .h(px(70.))
        .relative()
        .overflow_hidden()
        .rounded(px(9.))
        .border_1()
        .border_color(rgb(palette.border))
        .bg(linear_gradient(
            135.,
            linear_color_stop(rgb(palette.preview_start), 0.),
            linear_color_stop(rgb(palette.preview_end), 1.),
        ))
        .child(
            div()
                .absolute()
                .left(px(35.))
                .right(px(13.))
                .top(px(14.))
                .bottom(px(0.))
                .rounded_t(px(7.))
                .border_1()
                .border_color(rgb(palette.border))
                .bg(rgb(palette.surface)),
        )
}
