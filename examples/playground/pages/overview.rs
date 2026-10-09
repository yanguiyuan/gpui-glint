use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb};
use gpui_glint::theme::ThemePreset;

use super::common::{page, panel};

pub struct OverviewPage {
    pub theme: ThemePreset,
}

impl Render for OverviewPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let cards = div()
            .flex()
            .flex_wrap()
            .gap_4()
            .child(tile("01", "按钮", "动作与状态", self.theme))
            .child(tile("02", "图标", "常用符号与图形", self.theme))
            .child(tile("03", "布局容器", "窗口与内容分区", self.theme))
            .child(tile("04", "输入与选择", "表单、筛选与切换", self.theme))
            .child(tile("05", "徽标与标签", "分类、计数与状态", self.theme))
            .child(tile("06", "主题", "五套配色 · 浅色与深色", self.theme));
        page(
            "00",
            "组件预览",
            "GPUI 桌面组件 Playground。通过左侧目录切换组件页；每个页面保留独立的交互状态。",
            panel(cards, self.theme),
            self.theme,
        )
    }
}

fn tile(
    number: &'static str,
    title: &'static str,
    caption: &'static str,
    theme: ThemePreset,
) -> gpui::Div {
    div()
        .w(px(210.))
        .flex()
        .flex_col()
        .gap_2()
        .p_4()
        .rounded(px(14.))
        .border_1()
        .border_color(rgb(theme.palette().border))
        .bg(rgb(theme.palette().surface))
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(theme.palette().text_muted))
                .child(number),
        )
        .child(
            div()
                .text_size(px(15.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(rgb(theme.palette().text))
                .child(title),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(rgb(theme.palette().text_muted))
                .child(caption),
        )
}
