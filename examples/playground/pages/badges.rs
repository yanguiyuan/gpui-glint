use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb};
use gpui_glint::components::{
    Avatar, AvatarGroup, AvatarShape, ContentBadge, ContentType, CountBadge, CountBadgeKind,
    OverlayBadge, ReadStatus, StatusBadge, TagBadge, TagBadgeKind,
};
use gpui_glint::theme::ThemePreset;

use super::common::{page, panel};

pub struct BadgesPage {
    pub theme: ThemePreset,
}

impl Render for BadgesPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let content_types = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(ContentBadge::new(ContentType::Video, 28).theme(self.theme))
            .child(ContentBadge::new(ContentType::ImageGroup, 14).theme(self.theme))
            .child(ContentBadge::new(ContentType::Text, 5).theme(self.theme))
            .child(ContentBadge::new(ContentType::Link, 10).theme(self.theme))
            .child(ContentBadge::new(ContentType::Document, 7).theme(self.theme));
        let tags_and_counts = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(TagBadge::new("夜景").theme(self.theme))
            .child(TagBadge::new("长曝光").theme(self.theme))
            .child(TagBadge::new("城市").theme(self.theme))
            .child(
                TagBadge::new("已同步")
                    .theme(self.theme)
                    .kind(TagBadgeKind::Synced),
            )
            .child(
                TagBadge::new("真实数据")
                    .theme(self.theme)
                    .kind(TagBadgeKind::RealData),
            )
            .child(CountBadge::new("12").theme(self.theme))
            .child(
                CountBadge::new("3")
                    .theme(self.theme)
                    .kind(CountBadgeKind::Muted),
            )
            .child(
                CountBadge::new("PRO")
                    .theme(self.theme)
                    .kind(CountBadgeKind::Pro),
            );
        let cover_overlay = div()
            .w_full()
            .h(px(72.))
            .px(px(20.))
            .flex()
            .items_center()
            .gap_3()
            .rounded(px(14.))
            .bg(rgb(theme.color(0x30303d)))
            .child(OverlayBadge::new("No.024").theme(self.theme))
            .child(OverlayBadge::rating("9.2").theme(self.theme))
            .child(OverlayBadge::media("视频", 28).theme(self.theme));
        let statuses = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_5()
            .child(StatusBadge::new(ReadStatus::Unread).theme(self.theme))
            .child(StatusBadge::new(ReadStatus::Read).theme(self.theme))
            .child(StatusBadge::new(ReadStatus::Partial).theme(self.theme))
            .child(StatusBadge::new(ReadStatus::Failed).theme(self.theme));
        let members = [
            "林晓雨",
            "Ada Lovelace",
            "陈晨",
            "Grace Hopper",
            "周舟",
            "Alan Turing",
            "吴桐",
            "李悦",
        ];
        let avatars = div()
            .flex()
            .items_center()
            .gap_4()
            .child(Avatar::new("林晓雨").theme(self.theme))
            .child(Avatar::new("Ada Lovelace").size(48.).theme(self.theme))
            .child(
                Avatar::new("研发团队")
                    .text("研发")
                    .shape(AvatarShape::Rectangle)
                    .dimensions(64., 40.)
                    .theme(self.theme),
            )
            .child(Avatar::new("").theme(self.theme));
        let groups = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_8()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .gap_2()
                    .child(div().text_size(px(12.)).child("圆形头像组"))
                    .child(AvatarGroup::new(members.map(Avatar::new)).theme(self.theme)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .gap_2()
                    .child(div().text_size(px(12.)).child("矩形头像组"))
                    .child(
                        AvatarGroup::new(members.map(Avatar::new))
                            .shape(AvatarShape::Rectangle)
                            .max_visible(4)
                            .theme(self.theme),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .gap_2()
                    .child(div().text_size(px(12.)).child("无重叠头像组"))
                    .child(
                        AvatarGroup::new(members.into_iter().take(3).map(Avatar::new))
                            .size(32.)
                            .overlap(0.)
                            .theme(self.theme),
                    ),
            );
        let examples = div()
            .flex()
            .flex_col()
            .gap_5()
            .child(content_types)
            .child(tags_and_counts)
            .child(cover_overlay)
            .child(statuses)
            .child(div().text_size(px(14.)).child("头像与头像组"))
            .child(avatars)
            .child(groups)
            .child(div().text_size(px(12.)).text_color(rgb(theme.color(0x858c89))).child(
                "类型计数保持中性；标签表达分类；红色计数只表示未读，失败状态使用红色文字与圆点。",
            ));
        page(
            "05",
            "徽标与标签",
            "徽标用于浮在封面上的类型和计数，标签承载内容流里的分类。按语义使用浅玻璃、低饱和底色和必要的状态色。",
            panel(examples, self.theme),
            self.theme,
        )
    }
}
