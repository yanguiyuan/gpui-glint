use gpui::{
    Context, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled, Window, div,
    px, rgb,
};
use gpui_glint::components::{AppLayout, Button};
use gpui_glint::theme::ThemePreset;

use super::common::{backdrop_preview, page, panel};

pub struct LayoutPage {
    pub theme: ThemePreset,
    blur_enabled: bool,
}

impl LayoutPage {
    pub fn new(theme: ThemePreset) -> Self {
        Self {
            theme,
            blur_enabled: true,
        }
    }
}

impl Render for LayoutPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let topbar = div()
            .w_full()
            .h_full()
            .flex()
            .items_center()
            .px_4()
            .bg(rgb(self.theme.palette().canvas))
            .text_size(px(12.))
            .text_color(rgb(theme.color(0x505651)))
            .child("应用顶栏 · 42px");
        let sidebar = div()
            .size_full()
            .p_4()
            .bg(rgb(theme.color(0xf7f8f6)))
            .text_size(px(12.))
            .text_color(rgb(theme.color(0x505651)))
            .child("侧边栏");
        let content = div()
            .size_full()
            .p_4()
            .bg(rgb(theme.color(0xf1f4f6)))
            .text_size(px(12.))
            .text_color(rgb(theme.color(0x505651)))
            .child("主内容区");
        let composition = div()
            .w_full()
            .h(px(320.))
            .overflow_hidden()
            .rounded(px(16.))
            .border_1()
            .border_color(rgb(theme.color(0xdfe4e0)))
            .child(
                AppLayout::new(topbar, sidebar, content)
                    .theme(self.theme)
                    .render(),
            );
        page(
            "03",
            "布局容器",
            "顶栏位于外层，导航栏与工作区并排；工作区内部再由侧边栏和主内容区组成一个带圆角、高光与柔和阴影的整体。",
            div()
                .flex()
                .flex_col()
                .gap_5()
                .child(backdrop_preview(
                    theme,
                    self.blur_enabled,
                    Button::new(if self.blur_enabled {
                        "关闭右侧模糊"
                    } else {
                        "开启右侧模糊"
                    })
                    .theme(theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.blur_enabled = !this.blur_enabled;
                        cx.notify();
                    })),
                ))
                .child(panel(composition, self.theme)),
            self.theme,
        )
    }
}
