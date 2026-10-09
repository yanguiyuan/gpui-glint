use gpui::{Context, Entity, Subscription, Window, div, prelude::*, px, rgb, rgba};
use gpui_glint::{
    components::{Button, ButtonKind, ButtonSize, DocumentEditor, DocumentEvent},
    theme::ThemePreset,
};

pub struct DocumentsPage {
    theme: ThemePreset,
    editor: Entity<DocumentEditor>,
    _subscription: Subscription,
    markdown: String,
    notice: String,
}
impl DocumentsPage {
    pub fn new(theme: ThemePreset, cx: &mut Context<Self>) -> Self {
        let markdown = document_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_else(|| EXAMPLE.to_owned());
        let editor = cx.new(|cx| DocumentEditor::new(cx, &markdown).theme(theme));
        let subscription = cx.subscribe(&editor, |this, _, event, cx| {
            match event {
                DocumentEvent::Changed(source) => {
                    this.markdown = source.to_string();
                    this.notice = match save_document(&this.markdown) {
                        Ok(()) => "已保存到本机 · 可复制 Markdown 导出".into(),
                        Err(error) => format!("文档已更新，保存失败：{error}"),
                    };
                }
                DocumentEvent::Mermaid(_) => {}
                DocumentEvent::TitleChanged(_) => {}
                DocumentEvent::LinkActivated(url) => this.notice = format!("链接：{url}"),
            }
            cx.notify();
        });
        Self {
            theme,
            editor,
            _subscription: subscription,
            markdown,
            notice: "正文直接输入；选中文字设置格式。左侧手柄管理、拖动块；空块输入 / 插入内容。"
                .into(),
        }
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        self.editor
            .update(cx, |editor, cx| editor.set_theme(theme, cx));
        cx.notify();
    }
}
impl Render for DocumentsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let preview = self.editor.read(cx).is_preview();
        let mut toolbar = div()
            .h(px(42.))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .gap_2()
            .child(div().text_size(px(12.)).child("文档示例"))
            .child(div().flex_1());
        for (action, label) in [
            ("undo", "撤销"),
            ("redo", "重做"),
            ("finish", "完成编辑"),
            ("preview", if preview { "返回编辑" } else { "预览" }),
            ("copy", "复制 Markdown"),
        ] {
            toolbar = toolbar.child(
                Button::new(label)
                    .theme(self.theme)
                    .kind(ButtonKind::Plain)
                    .size(ButtonSize::Small)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if action == "copy" {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                this.editor.read(cx).markdown(),
                            ));
                        } else {
                            this.editor.update(cx, |editor, cx| match action {
                                "undo" => editor.undo(cx),
                                "redo" => editor.redo(cx),
                                "finish" => editor.finish_editing(cx),
                                _ => editor.set_preview(!editor.is_preview(), cx),
                            });
                        }
                        cx.notify();
                    })),
            );
        }
        div()
            .bg(if self.theme.is_dark() {
                rgba(0x00000000)
            } else {
                rgb(self.theme.palette().canvas)
            })
            .text_color(rgb(self.theme.palette().text))
            .size_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(
                div()
                    .flex_1()
                    .h(px(0.))
                    .min_h_0()
                    .min_w_0()
                    .child(self.editor.clone()),
            )
            .child(
                div()
                    .h(px(28.))
                    .flex_shrink_0()
                    .px_4()
                    .text_size(px(11.))
                    .text_color(rgb(self.theme.palette().text_muted))
                    .child(self.notice.clone()),
            )
    }
}
const EXAMPLE: &str = include_str!("../../mdeditor/welcome.md");

fn document_path() -> Option<std::path::PathBuf> {
    super::super::theme_storage_path().map(|path| path.with_file_name("document.md"))
}
fn save_document(markdown: &str) -> std::io::Result<()> {
    let path = document_path().ok_or_else(|| std::io::Error::other("无法确定文档目录"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("md.tmp");
    std::fs::write(&temporary, markdown)?;
    std::fs::rename(temporary, path)
}
