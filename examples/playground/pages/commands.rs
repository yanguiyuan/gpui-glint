use super::common::{page, panel};
use gpui::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px, rgb,
};
use gpui_glint::{
    components::{CommandItem, CommandPalette, CommandPaletteEvent, Icon},
    theme::ThemePreset,
};

pub struct CommandsPage {
    theme: ThemePreset,
    inline: Entity<CommandPalette>,
    modal: Entity<CommandPalette>,
    feedback: String,
    _subscriptions: Vec<Subscription>,
}
fn commands() -> Vec<CommandItem> {
    vec![
        CommandItem::new("new", "新建集合")
            .group("常用动作")
            .icon(Icon::Plus)
            .keywords("create collection")
            .shortcut("⌘ N"),
        CommandItem::new("import", "导入资源")
            .group("常用动作")
            .icon(Icon::Download)
            .keywords("import file")
            .shortcut("⌘ I"),
        CommandItem::new("photo", "拍摄手记")
            .group("最近集合")
            .icon(Icon::Folders)
            .keywords("photo notes"),
        CommandItem::new("film", "胶片扫描存档 2019—2024")
            .group("最近集合")
            .icon(Icon::Folders)
            .keywords("film archive"),
        CommandItem::new("settings", "打开设置")
            .group("设置")
            .icon(Icon::Settings)
            .keywords("preferences settings")
            .shortcut("⌘ ,"),
        CommandItem::new("export", "导出集合（暂无资源）")
            .group("设置")
            .icon(Icon::Upload)
            .disabled(true),
    ]
}
impl CommandsPage {
    pub fn new(theme: ThemePreset, cx: &mut Context<Self>) -> Self {
        let inline = cx.new(|cx| CommandPalette::new(commands(), cx).theme(theme));
        let modal = cx.new(|cx| CommandPalette::new(commands(), cx).theme(theme));
        let subscriptions = [&inline, &modal]
            .into_iter()
            .map(|entity| {
                cx.subscribe(
                    entity,
                    |this: &mut Self, _, event: &CommandPaletteEvent, cx| {
                        match event {
                            CommandPaletteEvent::Executed(id) => {
                                this.feedback = format!("演示：已执行命令 {id}")
                            }
                            CommandPaletteEvent::Dismissed => {
                                this.feedback = "已关闭命令面板".into()
                            }
                        };
                        cx.notify();
                    },
                )
            })
            .collect();
        Self {
            theme,
            inline,
            modal,
            feedback: "输入关键词筛选，↑ ↓ 选择，Enter 执行。".into(),
            _subscriptions: subscriptions,
        }
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        self.inline.update(cx, |p, cx| p.set_theme(theme, cx));
        self.modal.update(cx, |p, cx| p.set_theme(theme, cx));
        cx.notify();
    }
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal
            .update(cx, |palette, cx| palette.open(window, cx));
        cx.notify();
    }
    pub fn overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        self.modal.update(cx, |palette, cx| palette.overlay(cx))
    }
}
impl Render for CommandsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        page("16.5","命令面板","紧凑单行布局：图标、名称与快捷键同排；按分组搜索命令与集合。",
            div().flex().flex_col().gap_4()
            .child(panel(div().flex().justify_center().child(self.inline.clone()),self.theme))
            .child(div().id("open-command-demo").tab_index(0).cursor_pointer().px_3().py_2().rounded(px(7.)).border_1().border_color(rgb(p.border)).text_size(px(13.)).child("打开浮层　⌘ / Ctrl + K").on_click(cx.listener(|this,_,window,cx|this.open(window,cx))))
            .child(div().text_size(px(12.)).text_color(rgb(p.text_muted)).child(self.feedback.clone()))
            .child(div().text_size(px(12.)).text_color(rgb(p.text_muted)).child("36px 结果行 · 580px 面板 · 320px 列表限高 · Esc 关闭并恢复焦点。快捷键标签仅作提示，示例动作只输出执行反馈。")),self.theme)
    }
}
