use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb};
use gpui_glint::components::{Button, ButtonKind, ButtonSize, Icon, IconButton};
use gpui_glint::theme::ThemePreset;

use super::common::{page, panel};

pub struct ButtonsPage {
    pub theme: ThemePreset,
}

impl Render for ButtonsPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let examples = div()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("继续 · 第 08 个")
                            .theme(self.theme)
                            .kind(ButtonKind::Primary),
                    )
                    .child(
                        Button::new("从头开始")
                            .theme(self.theme)
                            .kind(ButtonKind::Tinted),
                    )
                    .child(
                        Button::new("随机一个")
                            .theme(self.theme)
                            .kind(ButtonKind::Glass),
                    )
                    .child(
                        Button::new("查看全部 ›")
                            .theme(self.theme)
                            .kind(ButtonKind::Plain),
                    )
                    .child(Button::new("取消").theme(self.theme).kind(ButtonKind::Gray)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("开始播放")
                            .theme(self.theme)
                            .kind(ButtonKind::Primary)
                            .size(ButtonSize::Large),
                    )
                    .child(
                        Button::new("常规 30px")
                            .theme(self.theme)
                            .kind(ButtonKind::Primary),
                    )
                    .child(
                        Button::new("小号 25px")
                            .theme(self.theme)
                            .kind(ButtonKind::Primary)
                            .size(ButtonSize::Small),
                    )
                    .child(
                        Button::new("迷你 21px")
                            .theme(self.theme)
                            .kind(ButtonKind::Primary)
                            .size(ButtonSize::Mini),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("永久删除")
                            .theme(self.theme)
                            .kind(ButtonKind::Danger),
                    )
                    .child(
                        Button::new("移入回收站")
                            .theme(self.theme)
                            .kind(ButtonKind::DangerTinted),
                    )
                    .child(
                        Button::new("胶囊按钮")
                            .theme(self.theme)
                            .kind(ButtonKind::Glass)
                            .pill(),
                    )
                    .child(Button::new("已禁用").theme(self.theme).disabled()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(IconButton::new(Icon::Search, "搜索").theme(self.theme))
                    .child(IconButton::new(Icon::Settings, "设置").theme(self.theme))
                    .child(IconButton::new(Icon::Plus, "新建").theme(self.theme))
                    .child(IconButton::new(Icon::Pencil, "编辑").theme(self.theme))
                    .child(IconButton::new(Icon::Trash, "删除").theme(self.theme))
                    .child(IconButton::new(Icon::More, "更多").theme(self.theme)),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(theme.color(0x7e8582)))
                    .child("按钮高度：38 / 30 / 25 / 21px · 主操作用实心蓝，危险操作仅用红色"),
            );
        page(
            "01",
            "按钮",
            "主按钮承担明确动作；次级按钮降低视觉重量，危险色只用于不可逆或高风险操作。",
            panel(examples, self.theme),
            self.theme,
        )
    }
}
