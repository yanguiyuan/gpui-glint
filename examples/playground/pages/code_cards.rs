use super::common::{page, panel};
use gpui::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div, px, rgb,
};
use gpui_glint::{
    components::{CodeCard, CodeColors, CollapsePanel, CollapsePanelContent, DiffCard, Icon},
    theme::ThemePreset,
};

pub struct CodeCardsPage {
    pub theme: ThemePreset,
    go: Entity<CodeCard>,
    json: Entity<CodeCard>,
    invalid_json: Entity<CodeCard>,
    headerless: Entity<CodeCard>,
    thinking: Entity<CollapsePanel>,
    terminal: Entity<CollapsePanel>,
    preview: Entity<CollapsePanel>,
    preview_card: Entity<CodeCard>,
    diff: Entity<DiffCard>,
}

impl CodeCardsPage {
    pub fn new(theme: ThemePreset, cx: &mut Context<Self>) -> Self {
        let thinking_text = "Fix test expectations — the mention scanner only fires when @ starts a word, so email@a.com is not a token. Update the offsets to byte ranges: @Workflow spans (7,16) in the fixture.";
        let thinking = cx.new(|_cx| {
            CollapsePanel::new("思考 · 持续了 11 秒")
                .icon(Icon::Pulse)
                .expanded(true)
                .theme(theme)
                .content(move |_, _cx| {
                    let theme = theme;
                    let p = theme.palette();
                    div()
                        .text_size(px(13.5))
                        .line_height(px(24.))
                        .text_color(rgb(p.text_muted))
                        .child(thinking_text)
                        .into_any_element()
                })
        });
        let terminal = cx.new(|_cx| {
            CollapsePanel::new("终端 · cargo test --lib")
                .icon(Icon::Script)
                .theme(theme)
                .content(move |_, _cx| {
                    let theme = theme;
                    let p = theme.palette();
                    div()
                        .font_family("Menlo")
                        .text_size(px(12.))
                        .line_height(px(20.))
                        .text_color(rgb(p.text_muted))
                        .child("running 64 tests\ntest theme::tests::theme_ids_round_trip ... ok\n...\ntest result: ok. 64 passed; 0 failed")
                        .into_any_element()
                })
        });

        const MOD_OLD: &str = "use crate::theme::ThemePreset;\nuse gpui::{\n    App, Context, InteractiveElement, IntoElement, ParentElement, Render,\n    SharedString, Styled, Window, div, px, rgb,\n};\n\npub use badge::{Badge, BadgeKind};\npub use button::{Button, ButtonKind};\npub use code_card::{CodeCard, CodeCardEvent};\npub use layout::AppLayout;\npub use mermaid::{MermaidDiagram, MermaidEvent, MermaidSvg};\npub use select::{Select, SelectEvent, SelectOption};\npub use tabs::{TabOption, Tabs};\npub use text_entry::{TextEntry, bind_text_entry_keys};\npub use theme_selector::{ThemeSelector, ThemeSelectorEvent};\npub use tooltip::Tooltip;\n\nmod compound_input;\npub use compound_input::{CompoundInput, CompoundInputEvent};\n\nmod command_palette;\npub use command_palette::{CommandItem, CommandPalette, CommandPaletteEvent};";
        const MOD_NEW: &str = "use crate::theme::ThemePreset;\nuse gpui::{\n    App, Context, InteractiveElement, IntoElement, ParentElement, Render,\n    SharedString, Styled, Window, div, px, rgb,\n};\n\npub use badge::{Badge, BadgeKind};\npub use button::{Button, ButtonKind};\npub use code_card::{CodeCard, CodeCardEvent};\npub use layout::AppLayout;\npub use mermaid::{MermaidDiagram, MermaidEvent, MermaidSvg};\npub use select::{Select, SelectEvent, SelectOption};\npub use tabs::{TabOption, Tabs, TabsKind};\npub use text_entry::{TextEntry, bind_text_entry_keys};\npub use theme_selector::{ThemeSelector, ThemeSelectorEvent};\npub use tooltip::Tooltip;\n\nmod select_field;\npub use select_field::{SelectField, SelectFieldEvent};\n\nmod composer;\npub use composer::{\n    Composer, ComposerChip, ComposerContextEntry, ComposerEvent,\n    bind_composer_keys,\n};\n\nmod command_palette;\npub use command_palette::{CommandItem, CommandPalette, CommandPaletteEvent};";
        let diff = cx.new(|cx| {
            DiffCard::new(cx, "rust", MOD_OLD, MOD_NEW)
                .name("mod.rs")
                .path("src/components/")
                .theme(theme)
        });
        let preview_card = cx.new(|cx| {
            CodeCard::new(
                cx,
                "go",
                "func main() {\n    fmt.Println(\"hello glint\")\n}",
            )
            .theme(theme)
        });
        let preview_editor = preview_card.clone();
        let preview_content: CollapsePanelContent =
            Box::new(move |_, _cx| div().child(preview_editor.clone()).into_any_element());
        let preview: Entity<CollapsePanel> = cx.new(|_| {
            CollapsePanel::new("生成代码预览")
                .theme(theme)
                .content_box(preview_content)
        });
        Self {
            theme,
            go: cx.new(|cx| CodeCard::new(cx, "go", "func repeat(value string, count int) []string {\n    // Example A, B, C\n    items := make([]string, 0, count)\n    for i := 0; i < count; i++ {\n        items = append(items, value)\n    }\n    return items\n}")),
            json: cx.new(|cx| CodeCard::new(cx, "json", r#"{"name":"会话标签","enabled":true,"limit":20}"#)),
            invalid_json: cx.new(|cx| CodeCard::new(cx, "json", "{\n  \"name\": \"会话标签\",\n  \"enabled\":\n}")),
            headerless: cx.new(|cx| {
                CodeCard::new(cx, "http", "POST https://api.example.com/v1/collections\nAccept: application/json\n\n{\n  \"name\": \"拍摄手记\"\n}")
                    .read_only(true)
                    .show_header(false)
            }),
            thinking,
            terminal,
            preview,
            preview_card,
            diff,
        }
    }
}

impl Render for CodeCardsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for card in [
            &self.go,
            &self.json,
            &self.invalid_json,
            &self.headerless,
            &self.preview_card,
        ] {
            card.update(cx, |card, cx| card.set_theme(self.theme, cx));
        }
        for panel_entity in [&self.thinking, &self.terminal, &self.preview] {
            panel_entity.update(cx, |panel, cx| panel.set_theme(self.theme, cx));
        }
        self.diff
            .update(cx, |card, cx| card.set_theme(self.theme, cx));
        let colors = if self.theme.is_dark() {
            CodeColors::DARK
        } else {
            CodeColors::LIGHT
        };
        let legend = div()
            .flex()
            .flex_wrap()
            .gap_4()
            .font_family("Menlo")
            .text_size(px(12.))
            .child(div().text_color(rgb(colors.number)).child("关键字 / 数值"))
            .child(div().text_color(rgb(colors.key)).child("函数 / JSON 键"))
            .child(div().text_color(rgb(colors.string)).child("字符串"))
            .child(
                div()
                    .text_color(rgb(colors.type_name))
                    .child("类型 / 命名空间"),
            )
            .child(div().text_color(rgb(colors.comment)).child("注释"));
        let states = div()
            .flex()
            .flex_wrap()
            .gap(px(18.))
            .child(
                div()
                    .flex_1()
                    .min_w(px(300.))
                    .child(example("JSON · 有效状态与格式化", self.json.clone())),
            )
            .child(div().flex_1().min_w(px(300.)).child(example(
                "JSON · 错误提示保持在语言栏",
                self.invalid_json.clone(),
            )));
        let examples = div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_5()
            .child(example(
                "普通代码 · Go　点击源码查看焦点行；长行可横向滚动",
                self.go.clone(),
            ))
            .child(legend)
            .child(example(
                "隐藏语言栏 · 直接显示代码内容",
                self.headerless.clone(),
            ))
            .child(states);
        let notes = div()
            .flex()
            .flex_wrap()
            .gap_4()
            .text_size(px(12.))
            .text_color(rgb(self.theme.palette().text_muted))
            .child("容器：9px 圆角 · 1px 弱边界 · 无独立阴影")
            .child("语言栏：35px · 图标 13px · 标签 10px")
            .child("源码：mono 13px / 21px · 上下 13px · 最少 3 行");
        let editing = div().text_size(px(13.)).text_color(rgb(self.theme.palette().text_muted))
            .child("直接在卡内编辑；JSON 实时校验，有效时可格式化。⌘ A / C / V 选择与复制粘贴，⌘ Z / ⇧⌘ Z 撤销与重做。未知语言保留纯文本，长代码随页面滚动。");
        let collapse_examples = div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.thinking.clone())
            .child(self.terminal.clone())
            .child(self.preview.clone())
            .child(self.diff.clone());
        page(
            "16.4",
            "文档代码卡",
            "代码卡嵌入正文，使用轻边界 + 中性语言栏 + 等宽源码形成清晰层级。编辑与阅读使用同一张卡；高亮只区分语法，卡内不再叠加悬浮卡片或阴影。",
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_4()
                .child(panel(examples, self.theme))
                .child(panel(collapse_examples, self.theme))
                .child(notes)
                .child(editing),
            self.theme,
        )
    }
}

fn example(caption: &'static str, card: Entity<CodeCard>) -> gpui::Div {
    div()
        .w_full()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_size(px(12.)).child(caption))
        .child(card)
}
