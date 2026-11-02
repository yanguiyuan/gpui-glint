//! Reusable read-only file/directory preview. The host supplies data and performs I/O.
use super::{CodeCard, CodeCardEvent, CodeSelection, Icon, IconButton};
use crate::paths::display_path;
use crate::theme::ThemePreset;
use gpui::{
    AppContext, Context, Entity, EventEmitter, Render, SharedString, Window, div, prelude::*, px,
    rgb,
};

#[derive(Clone, Debug)]
pub struct FilePreviewEntry {
    pub name: String,
    pub path: String,
    pub directory: bool,
    pub previewable: bool,
}
#[derive(Clone, Debug)]
pub enum FilePreviewEvent {
    OpenRequested(String),
    CloseRequested,
    QuoteRequested {
        path: String,
        selection: CodeSelection,
    },
}
enum Page {
    Loading,
    Error(String),
    Directory(Vec<FilePreviewEntry>),
    File(Entity<CodeCard>),
}
pub struct FilePreview {
    path: String,
    parent: Option<String>,
    page: Page,
    theme: ThemePreset,
    scroll: gpui::ScrollHandle,
    subscriptions: Vec<gpui::Subscription>,
}
impl EventEmitter<FilePreviewEvent> for FilePreview {}
impl FilePreview {
    pub fn new(_: &mut Context<Self>) -> Self {
        Self {
            path: String::new(),
            parent: None,
            page: Page::Loading,
            theme: ThemePreset::default(),
            scroll: Default::default(),
            subscriptions: Vec::new(),
        }
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            if let Page::File(editor) = &self.page {
                editor.update(cx, |editor, cx| editor.set_theme(theme, cx));
            }
            cx.notify();
        }
    }
    pub fn set_loading(&mut self, path: String, cx: &mut Context<Self>) {
        self.path = path;
        self.page = Page::Loading;
        self.subscriptions.clear();
        self.parent = None;
        cx.notify();
    }
    pub fn set_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.page = Page::Error(message);
        cx.notify();
    }
    pub fn set_file(
        &mut self,
        path: String,
        parent: Option<String>,
        language: String,
        text: String,
        cx: &mut Context<Self>,
    ) {
        self.path = path;
        self.parent = parent;
        self.subscriptions.clear();
        let editor = cx.new(|cx| {
            CodeCard::new(cx, language, text)
                .read_only(true)
                .quote_context(true)
                .show_header(false)
                .viewport()
                .theme(self.theme)
        });
        self.subscriptions
            .push(cx.subscribe(&editor, |this, _, event: &CodeCardEvent, cx| {
                if let CodeCardEvent::QuoteRequested = event {
                    this.quote(cx);
                }
            }));
        self.page = Page::File(editor);
        cx.notify();
    }
    fn quote(&self, cx: &mut Context<Self>) {
        if let Page::File(editor) = &self.page
            && let Some(selection) = editor.read(cx).selected_excerpt()
        {
            cx.emit(FilePreviewEvent::QuoteRequested {
                path: self.path.clone(),
                selection,
            });
        }
    }
    pub fn reveal_line(&mut self, line: usize, cx: &mut Context<Self>) {
        if let Page::File(editor) = &self.page {
            editor.update(cx, |editor, cx| editor.reveal_line(line, cx));
        }
    }
    pub fn set_directory(
        &mut self,
        path: String,
        parent: Option<String>,
        entries: Vec<FilePreviewEntry>,
        cx: &mut Context<Self>,
    ) {
        self.path = path;
        self.parent = parent;
        self.page = Page::Directory(entries);
        self.subscriptions.clear();
        self.scroll = Default::default();
        cx.notify();
    }
}
impl Render for FilePreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let mut body = div().flex_1().min_h_0().min_w_0().w_full();
        body = match &self.page {
            Page::Loading => body.child(div().p_4().child("正在读取…")),
            Page::Error(message) => body.child(
                div()
                    .p_4()
                    .text_color(rgb(p.text_muted))
                    .child(message.clone()),
            ),
            Page::File(editor) => body.flex().child(editor.clone()),
            Page::Directory(entries) => {
                let mut list = div()
                    .id("file-preview-directory")
                    .h_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll);
                if entries.is_empty() {
                    list = list.child(div().p_4().text_color(rgb(p.text_muted)).child("空目录"));
                }
                for entry in entries {
                    let path = entry.path.clone();
                    list = list.child(
                        div()
                            .id(SharedString::from(format!("preview-entry:{path}")))
                            .debug_selector(move || format!("preview-entry:{path}"))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_4()
                            .py_2()
                            .text_color(rgb(if entry.previewable || entry.directory {
                                p.text
                            } else {
                                p.text_muted
                            }))
                            .child(
                                if entry.directory {
                                    Icon::Folder
                                } else {
                                    Icon::Text
                                }
                                .render(15.),
                            )
                            .child(div().min_w_0().text_ellipsis().child(entry.name.clone()))
                            .when(entry.directory || entry.previewable, |row| {
                                let path = entry.path.clone();
                                row.cursor_pointer()
                                    .hover(|s| s.bg(rgb(p.surface_subtle)))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(FilePreviewEvent::OpenRequested(path.clone()))
                                    }))
                            }),
                    );
                }
                body.child(list)
            }
        };
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(rgb(p.panel))
            .text_color(rgb(p.text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .when_some(self.parent.clone(), |row, path| {
                        row.child(
                            IconButton::new(Icon::ArrowUp, "上级目录")
                                .plain()
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(FilePreviewEvent::OpenRequested(path.clone()))
                                })),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_ellipsis()
                            .text_size(px(12.))
                            .child(display_path(&self.path)),
                    )
                    .child(
                        IconButton::new(Icon::Close, "关闭预览")
                            .plain()
                            .theme(self.theme)
                            .render()
                            .on_click(
                                cx.listener(|_, _, _, cx| {
                                    cx.emit(FilePreviewEvent::CloseRequested)
                                }),
                            ),
                    ),
            )
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Focusable;
    use std::{cell::RefCell, rc::Rc};
    #[gpui::test]
    fn file_preview_quotes_exact_source_lines_and_remains_read_only(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::bind_code_card_keys);
        let content = "alpha\n中文 beta\nlast\n";
        let (view, window) = cx.add_window_view(|_, cx| {
            let mut view = FilePreview::new(cx);
            view.set_file(
                "/project/source.rs".into(),
                Some("/project".into()),
                "rust".into(),
                content.into(),
                cx,
            );
            view
        });
        window.simulate_resize(gpui::size(px(600.), px(400.)));
        let events = Rc::new(RefCell::new(Vec::new()));
        let received = events.clone();
        let _observer = window.new(|cx: &mut Context<()>| {
            cx.subscribe(&view, move |_, _, event: &FilePreviewEvent, _| {
                if let FilePreviewEvent::QuoteRequested { path, selection } = event {
                    received
                        .borrow_mut()
                        .push((path.clone(), selection.clone()));
                }
            })
            .detach();
        });
        window.run_until_parked();
        let editor = view.read_with(window, |view, _| match &view.page {
            Page::File(editor) => editor.clone(),
            _ => panic!(),
        });
        window.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
        window.simulate_keystrokes("cmd-a");
        window.run_until_parked();
        assert!(window.debug_bounds("quote-preview-selection").is_none());
        window.simulate_mouse_down(
            gpui::point(px(80.), px(55.)),
            gpui::MouseButton::Right,
            Default::default(),
        );
        window.run_until_parked();
        let quote = window.debug_bounds("code-card-quote").unwrap();
        window.simulate_click(quote.center(), Default::default());
        window.run_until_parked();
        assert_eq!(events.borrow().len(), 1);
        window.simulate_keystrokes("cmd-shift-l");
        window.run_until_parked();
        assert_eq!(events.borrow().len(), 2);
        let (path, selection) = events.borrow()[0].clone();
        assert_eq!(path, "/project/source.rs");
        assert_eq!((selection.start_line, selection.end_line), (1, 3));
        assert_eq!(selection.text, content);
        window.simulate_input("cannot edit");
        editor.read_with(window, |editor, _| assert_eq!(editor.content(), content));
    }
}
