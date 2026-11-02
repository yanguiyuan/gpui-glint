//! Reusable read-only file/directory preview. The host supplies data and performs I/O.
use super::{
    CodeCard, CodeCardEvent, CodeSelection, DocumentEditor, Icon, IconButton, SegmentOption,
    SegmentedControl,
};
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
    ExpandedChanged(bool),
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
    Image(std::sync::Arc<gpui::Image>),
}
pub struct FilePreview {
    #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
    html: Option<String>,
    #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
    browser: Option<Result<Entity<super::WebView>, String>>,
    expanded: bool,
    path: String,
    parent: Option<String>,
    page: Page,
    theme: ThemePreset,
    markdown: Option<Entity<DocumentEditor>>,
    markdown_preview: bool,
    scroll: gpui::ScrollHandle,
    subscriptions: Vec<gpui::Subscription>,
}
impl EventEmitter<FilePreviewEvent> for FilePreview {}
impl FilePreview {
    pub fn new(_: &mut Context<Self>) -> Self {
        Self {
            #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
            html: None,
            #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
            browser: None,
            expanded: false,
            path: String::new(),
            parent: None,
            page: Page::Loading,
            theme: ThemePreset::default(),
            markdown: None,
            markdown_preview: false,
            scroll: Default::default(),
            subscriptions: Vec::new(),
        }
    }
    fn clear_html(&mut self) {
        #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
        {
            self.html = None;
            self.browser = None;
        }
    }
    /// Hide native child views when the host removes the preview from its layout.
    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
        if let Some(Ok(browser)) = &self.browser {
            browser.update(cx, |view, cx| {
                let _ = view.set_visible(active && self.markdown_preview, cx);
            });
        }
        let _ = (active, cx);
    }
    fn has_rendered_preview(&self) -> bool {
        #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
        if self.html.is_some() {
            return true;
        }
        self.markdown.is_some()
    }
    /// Synchronize the host layout with the preview's expand/collapse button.
    pub fn set_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        if self.expanded != expanded {
            self.expanded = expanded;
            cx.notify();
        }
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            if let Page::File(editor) = &self.page {
                editor.update(cx, |editor, cx| editor.set_theme(theme, cx));
            }
            if let Some(markdown) = &self.markdown {
                markdown.update(cx, |view, cx| view.set_theme(theme, cx));
            }
            cx.notify();
        }
    }
    pub fn set_loading(&mut self, path: String, cx: &mut Context<Self>) {
        self.path = path;
        self.clear_html();
        self.page = Page::Loading;
        self.markdown = None;
        self.markdown_preview = false;
        self.subscriptions.clear();
        self.parent = None;
        cx.notify();
    }
    pub fn set_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.clear_html();
        self.page = Page::Error(message);
        self.markdown = None;
        self.markdown_preview = false;
        cx.notify();
    }
    /// Display an image supplied by the host, without performing file I/O.
    pub fn set_image(
        &mut self,
        path: String,
        parent: Option<String>,
        image: std::sync::Arc<gpui::Image>,
        cx: &mut Context<Self>,
    ) {
        self.path = path;
        self.parent = parent;
        self.markdown = None;
        self.markdown_preview = false;
        self.subscriptions.clear();
        self.clear_html();
        self.page = Page::Image(image);
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
        self.clear_html();
        #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
        if matches!(Icon::for_language(&language), Icon::Html)
            || matches!(Icon::for_file_name(&path), Icon::Html)
        {
            self.html = Some(text.clone());
        }
        let is_markdown = matches!(language.to_ascii_lowercase().as_str(), "md" | "markdown")
            || std::path::Path::new(&path)
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
                });
        self.markdown = is_markdown.then(|| {
            cx.new(|cx| {
                DocumentEditor::new(cx, &text)
                    .scrollable_preview()
                    .theme(self.theme)
            })
        });
        self.markdown_preview = false;
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
    /// Select the rendered view when the current file supports it.
    pub fn set_rendered_preview(&mut self, preview: bool, cx: &mut Context<Self>) {
        self.markdown_preview = preview && self.has_rendered_preview();
        cx.notify();
    }
    pub fn reveal_line(&mut self, line: usize, cx: &mut Context<Self>) {
        if let Page::File(editor) = &self.page {
            self.markdown_preview = false;
            editor.update(cx, |editor, cx| editor.reveal_line(line, cx));
            cx.notify();
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
        self.clear_html();
        self.page = Page::Directory(entries);
        self.markdown = None;
        self.markdown_preview = false;
        self.subscriptions.clear();
        self.scroll = Default::default();
        cx.notify();
    }
}
impl Render for FilePreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
        {
            if self.markdown_preview
                && self.browser.is_none()
                && let Some(html) = &self.html
            {
                self.browser = Some(super::WebView::interactive_html_entity(html, window, cx));
            }
            self.set_active(true, cx);
        }
        let _ = window;
        let p = self.theme.palette();
        let mut body = div().flex_1().min_h_0().min_w_0().w_full();
        body = match &self.page {
            Page::Image(image) => body.flex().p_4().child(
                gpui::img(image.clone())
                    .size_full()
                    .object_fit(gpui::ObjectFit::Contain),
            ),
            Page::Loading => body.child(div().p_4().child("正在读取…")),
            Page::Error(message) => body.child(
                div()
                    .p_4()
                    .text_color(rgb(p.text_muted))
                    .child(message.clone()),
            ),
            Page::File(_) if self.markdown_preview && self.markdown.is_some() => {
                body.flex().child(self.markdown.as_ref().unwrap().clone())
            }
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
                                    Icon::for_file_name(&entry.name)
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
        #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
        if self.markdown_preview
            && let Some(browser) = &self.browser
        {
            body = match browser {
                Ok(browser) => div().flex_1().min_h_0().min_w_0().child(browser.clone()),
                Err(error) => div()
                    .flex_1()
                    .p_4()
                    .child(format!("HTML 预览失败：{error}")),
            };
        }
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
                    .when(self.has_rendered_preview(), |row| {
                        let entity = cx.entity();
                        row.child(
                            SegmentedControl::new([
                                SegmentOption::new("源码"),
                                SegmentOption::new("预览"),
                            ])
                            .selected(usize::from(self.markdown_preview))
                            .theme(self.theme)
                            .on_change(move |index, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.markdown_preview = index == 1;
                                    cx.notify();
                                });
                            })
                            .render(),
                        )
                    })
                    .child(
                        IconButton::new(
                            if self.expanded {
                                Icon::Collapse
                            } else {
                                Icon::Expand
                            },
                            if self.expanded {
                                "恢复侧栏预览"
                            } else {
                                "展开预览"
                            },
                        )
                        .plain()
                        .theme(self.theme)
                        .render()
                        .debug_selector(|| "file-preview-expand".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.emit(FilePreviewEvent::ExpandedChanged(!this.expanded));
                        })),
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
    #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
    #[gpui::test]
    fn html_preview_is_available_without_creating_native_view_until_selected(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let view = cx.new(FilePreview::new);
            view.update(cx, |view, cx| {
                view.set_file(
                    "/project/index.HTML".into(),
                    None,
                    "text".into(),
                    "<h1>Hello</h1>".into(),
                    cx,
                );
                assert!(view.has_rendered_preview());
                assert_eq!(view.html.as_deref(), Some("<h1>Hello</h1>"));
                assert!(view.browser.is_none());
                assert!(!view.markdown_preview);
                view.set_file(
                    "/project/main.rs".into(),
                    None,
                    "rust".into(),
                    "fn main() {}".into(),
                    cx,
                );
                assert!(!view.has_rendered_preview());
                assert!(view.html.is_none());
            });
        });
    }
    #[gpui::test]
    fn expand_button_requests_host_layout_changes(cx: &mut gpui::TestAppContext) {
        let (view, window) = cx.add_window_view(|_, cx| FilePreview::new(cx));
        window.simulate_resize(gpui::size(px(600.), px(400.)));
        let requests = Rc::new(RefCell::new(Vec::new()));
        let received = requests.clone();
        let _observer = window.new(|cx: &mut Context<()>| {
            cx.subscribe(&view, move |_, view, event: &FilePreviewEvent, cx| {
                if let FilePreviewEvent::ExpandedChanged(expanded) = event {
                    received.borrow_mut().push(*expanded);
                    view.update(cx, |view, cx| view.set_expanded(*expanded, cx));
                }
            })
            .detach();
        });
        for _ in 0..2 {
            window.run_until_parked();
            let button = window.debug_bounds("file-preview-expand").unwrap();
            window.simulate_click(button.center(), Default::default());
        }
        window.run_until_parked();
        assert_eq!(*requests.borrow(), vec![true, false]);
    }
    #[gpui::test]
    fn markdown_preview_is_read_only_and_resets_for_other_files(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let view = cx.new(FilePreview::new);
            view.update(cx, |view, cx| {
                view.set_file(
                    "/project/README.MD".into(),
                    None,
                    "text".into(),
                    "# Title\n\n**Hello**".into(),
                    cx,
                );
                let markdown = view.markdown.as_ref().unwrap();
                assert!(markdown.read(cx).is_preview());
                assert!(!view.markdown_preview);
                view.markdown_preview = true;
                view.reveal_line(2, cx);
                assert!(!view.markdown_preview);
                view.set_theme(ThemePreset::default(), cx);
                view.set_file(
                    "/project/main.rs".into(),
                    None,
                    "rust".into(),
                    "fn main() {}".into(),
                    cx,
                );
                assert!(view.markdown.is_none());
                assert!(!view.markdown_preview);
                view.set_file(
                    "/project/notes".into(),
                    None,
                    "markdown".into(),
                    "# Notes".into(),
                    cx,
                );
                assert!(view.markdown.is_some());
                view.set_loading("/project".into(), cx);
                assert!(view.markdown.is_none());
            });
        });
    }
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
