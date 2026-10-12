use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb};
use gpui_glint::components::Icon;
use gpui_glint::theme::ThemePreset;

use super::common::{page, panel};

pub struct IconsPage {
    pub theme: ThemePreset,
}

impl Render for IconsPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let icons = [
            (Icon::Search, "搜索"),
            (Icon::Settings, "设置"),
            (Icon::Plus, "添加"),
            (Icon::Pencil, "编辑"),
            (Icon::Trash, "删除"),
            (Icon::Copy, "复制"),
            (Icon::Save, "保存"),
            (Icon::Upload, "上传"),
            (Icon::Download, "下载"),
            (Icon::Refresh, "刷新"),
            (Icon::Shuffle, "随机"),
            (Icon::Play, "播放"),
            (Icon::More, "更多"),
            (Icon::Database, "数据库"),
            (Icon::Globe, "网络"),
            (Icon::Workflow, "流程"),
            (Icon::Pulse, "状态"),
            (Icon::Key, "密钥"),
            (Icon::Folders, "文件夹"),
            (Icon::Folder, "收起目录"),
            (Icon::FolderOpen, "展开目录"),
            (Icon::Compass, "导航"),
            (Icon::Clock, "时间"),
            (Icon::User, "用户"),
            (Icon::Filter, "筛选"),
            (Icon::Video, "视频"),
            (Icon::ImageGroup, "图集"),
            (Icon::Text, "文本"),
            (Icon::Link, "链接"),
            (Icon::Tag, "标签"),
            (Icon::Star, "收藏"),
            (Icon::CheckCircle, "完成"),
            (Icon::Eye, "查看"),
            (Icon::Markdown, "Markdown"),
            (Icon::Rust, "Rust"),
            (Icon::Go, "Go"),
            (Icon::JavaScript, "JavaScript"),
            (Icon::TypeScript, "TypeScript"),
            (Icon::Python, "Python"),
            (Icon::Java, "Java"),
            (Icon::C, "C"),
            (Icon::Cpp, "C++"),
            (Icon::Html, "HTML"),
            (Icon::Css, "CSS"),
            (Icon::Json, "JSON"),
            (Icon::Shell, "Shell"),
            (Icon::Sql, "SQL"),
            (Icon::Yaml, "YAML"),
        ];
        let grid =
            icons
                .into_iter()
                .fold(div().flex().flex_wrap().gap_3(), |grid, (icon, label)| {
                    grid.child(
                        div()
                            .w(px(108.))
                            .h(px(82.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .rounded(px(12.))
                            .border_1()
                            .border_color(rgb(theme.color(0xe2e6e2)))
                            .bg(rgb(theme.palette().surface))
                            .child(icon.render(20.))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(theme.color(0x666e6b)))
                                    .child(label),
                            ),
                    )
                });
        page(
            "02",
            "图标",
            "统一使用轻线条与中性灰，确保图形作为信息提示而不是视觉主角。",
            panel(grid, self.theme),
            self.theme,
        )
    }
}
