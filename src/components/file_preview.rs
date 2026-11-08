//! Reusable read-only file/directory preview. The host supplies data and performs I/O.
use super::{
    CodeCard, CodeCardEvent, CodeSelection, DocumentEditor, Icon, IconButton, SegmentOption,
    SegmentedControl, TextEntry,
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
    DirectoryRequested {
        path: String,
        request: u64,
    },
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
#[derive(Default)]
struct FolderNode {
    expanded: bool,
    entries: Option<Vec<FilePreviewEntry>>,
    loading: Option<u64>,
    error: Option<String>,
}

pub struct FilePreview {
    #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
    html: Option<String>,
    #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
    browser: Option<Result<Entity<super::WebView>, String>>,
    expanded: bool,
    navigation: Option<(String, Vec<FilePreviewEntry>)>,
    navigation_scroll: gpui::ScrollHandle,
    folders: std::collections::HashMap<String, FolderNode>,
    next_request: u64,
    path: String,
    parent: Option<String>,
    page: Page,
    theme: ThemePreset,
    markdown: Option<Entity<DocumentEditor>>,
    markdown_preview: bool,
    scroll: gpui::ScrollHandle,
    search: Entity<TextEntry>,
    search_match: Option<(usize, usize)>,
    _search_subscription: gpui::Subscription,
    subscriptions: Vec<gpui::Subscription>,
}
impl EventEmitter<FilePreviewEvent> for FilePreview {}
impl FilePreview {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextEntry::new(cx, "搜索代码", "").font_size(12.));
        let search_subscription = cx.observe(&search, |this, search, cx| {
            let Page::File(editor) = &this.page else {
                return;
            };
            let query = search.read(cx).value().to_owned();
            if editor.read(cx).search_query() == query {
                return;
            }
            this.search_match = editor.update(cx, |editor, cx| {
                editor.set_search_query(query, cx);
                let count = editor.search_match_count();
                editor.reveal_search_match(0, cx).then_some((0, count))
            });
            cx.notify();
        });
        Self {
            #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
            html: None,
            #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
            browser: None,
            expanded: false,
            navigation: None,
            navigation_scroll: Default::default(),
            folders: Default::default(),
            next_request: 0,
            path: String::new(),
            parent: None,
            page: Page::Loading,
            theme: ThemePreset::default(),
            markdown: None,
            markdown_preview: false,
            scroll: Default::default(),
            search,
            search_match: None,
            _search_subscription: search_subscription,
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
            self.search
                .update(cx, |entry, cx| entry.set_theme(theme, cx));
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
        if self
            .navigation
            .as_ref()
            .is_some_and(|(root, _)| !std::path::Path::new(&path).starts_with(root))
        {
            self.navigation = None;
            self.folders.clear();
        }
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
        if parent.is_none() {
            self.navigation = None;
        }
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
        let language = match Icon::for_file_name(&path) {
            Icon::Docker => "dockerfile".to_owned(),
            Icon::Makefile => "makefile".to_owned(),
            _ => language,
        };
        if parent.is_none() {
            self.navigation = None;
        }
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
        self.search_match = None;
        self.search.update(cx, |entry, cx| entry.set_value("", cx));
        cx.notify();
    }
    fn step_search(&mut self, previous: bool, cx: &mut Context<Self>) {
        let (Page::File(editor), Some((current, count))) = (&self.page, self.search_match) else {
            return;
        };
        let next = if previous {
            (current + count - 1) % count
        } else {
            (current + 1) % count
        };
        editor.update(cx, |editor, cx| editor.reveal_search_match(next, cx));
        self.search_match = Some((next, count));
        cx.notify();
    }

    fn search_controls(&self, cx: &mut Context<Self>) -> gpui::Div {
        let status = self.search_match.map_or_else(
            || "无匹配".to_owned(),
            |(index, count)| format!("{} / {count}", index + 1),
        );
        div()
            .debug_selector(|| "file-preview-search".into())
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .id("file-preview-search-input")
                    .w(px(180.))
                    .h(px(28.))
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                        if event.keystroke.key == "enter" {
                            this.step_search(event.keystroke.modifiers.shift, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(self.search.clone()),
            )
            .when(!self.search.read(cx).value().is_empty(), |row| {
                row.child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(self.theme.palette().text_muted))
                        .child(status),
                )
            })
            .when(self.search_match.is_some(), |row| {
                row.children(
                    [
                        (Icon::ArrowUp, "上一个匹配", true),
                        (Icon::ChevronDown, "下一个匹配", false),
                    ]
                    .map(|(icon, label, previous)| {
                        IconButton::new(icon, label)
                            .plain()
                            .theme(self.theme)
                            .render()
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.step_search(previous, cx)),
                            )
                    }),
                )
            })
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
    fn directory_list(
        &self,
        entries: &[FilePreviewEntry],
        scroll: &gpui::ScrollHandle,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let mut list = div()
            .id("file-preview-directory")
            .h_full()
            .overflow_y_scroll()
            .track_scroll(scroll);
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
                    .when(entry.path == self.path, |row| row.bg(rgb(p.accent_tint)))
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
        list
    }

    pub fn navigation_path(&self) -> Option<&str> {
        let parent = std::path::Path::new(&self.path).parent()?.to_str()?;
        if self
            .navigation
            .as_ref()
            .is_some_and(|(root, _)| root == parent)
            || self
                .folders
                .get(parent)
                .is_some_and(|node| node.entries.is_some())
        {
            Some(parent)
        } else {
            None
        }
    }

    fn toggle_folder(&mut self, path: String, cx: &mut Context<Self>) {
        let node = self.folders.entry(path.clone()).or_default();
        node.expanded = !node.expanded;
        if node.expanded && node.entries.is_none() && node.loading.is_none() {
            self.next_request += 1;
            node.loading = Some(self.next_request);
            node.error = None;
            cx.emit(FilePreviewEvent::DirectoryRequested {
                path,
                request: self.next_request,
            });
        }
        cx.notify();
    }

    pub fn cancel_directory_requests(&mut self, cx: &mut Context<Self>) {
        for node in self.folders.values_mut() {
            if node.loading.take().is_some() {
                node.expanded = false;
            }
        }
        cx.notify();
    }

    pub fn set_directory_children(
        &mut self,
        path: String,
        request: u64,
        result: Result<(Vec<FilePreviewEntry>, bool), String>,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self
            .folders
            .get_mut(&path)
            .filter(|node| node.loading == Some(request))
        else {
            return;
        };
        node.loading = None;
        match result {
            Ok((mut entries, truncated)) => {
                Self::append_truncation(&mut entries, truncated);
                node.entries = Some(entries);
                node.error = None;
            }
            Err(error) => node.error = Some(error),
        }
        cx.notify();
    }

    fn append_truncation(entries: &mut Vec<FilePreviewEntry>, truncated: bool) {
        if truncated {
            entries.push(FilePreviewEntry {
                name: "仅显示前 1000 项".into(),
                path: String::new(),
                directory: false,
                previewable: false,
            });
        }
    }

    fn tree_rows(
        &self,
        entries: &[FilePreviewEntry],
        depth: usize,
        rows: &mut Vec<(FilePreviewEntry, usize)>,
    ) {
        if depth >= 64 {
            return;
        }
        for entry in entries {
            rows.push((entry.clone(), depth));
            if let Some(node) = self
                .folders
                .get(&entry.path)
                .filter(|node| entry.directory && node.expanded)
            {
                if let Some(entries) = &node.entries {
                    if entries.is_empty() {
                        rows.push((
                            FilePreviewEntry {
                                name: "空目录".into(),
                                path: String::new(),
                                directory: false,
                                previewable: false,
                            },
                            depth + 1,
                        ));
                    } else {
                        self.tree_rows(entries, depth + 1, rows);
                    }
                } else {
                    rows.push((
                        FilePreviewEntry {
                            name: node
                                .error
                                .as_ref()
                                .map(|error| format!("读取失败：{error}（收起后展开重试）"))
                                .unwrap_or_else(|| "正在读取…".into()),
                            path: String::new(),
                            directory: false,
                            previewable: false,
                        },
                        depth + 1,
                    ));
                }
            }
        }
    }

    fn navigation_tree(
        &self,
        entries: &[FilePreviewEntry],
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let mut rows = Vec::new();
        self.tree_rows(entries, 0, &mut rows);
        let mut list = div()
            .id("file-preview-tree")
            .h_full()
            .overflow_y_scroll()
            .track_scroll(&self.navigation_scroll);
        if rows.is_empty() {
            list = list.child(div().p_4().child("空目录"));
        }
        for (index, (entry, depth)) in rows.into_iter().enumerate() {
            let path = entry.path.clone();
            let expanded = self.folders.get(&path).is_some_and(|node| node.expanded);
            list = list.child(
                div()
                    .id(("tree-entry", index))
                    .debug_selector(move || format!("preview-entry:{path}"))
                    .flex()
                    .items_center()
                    .gap_1()
                    .pl(px(8. + depth as f32 * 16.))
                    .pr_2()
                    .py_1()
                    .when(entry.path == self.path, |row| row.bg(rgb(p.accent_tint)))
                    .text_color(rgb(if entry.directory || entry.previewable {
                        p.text
                    } else {
                        p.text_muted
                    }))
                    .child(
                        div()
                            .w(px(14.))
                            .flex_shrink_0()
                            .when(entry.directory, |row| {
                                row.child(
                                    if expanded {
                                        Icon::ChevronDown
                                    } else {
                                        Icon::ChevronRight
                                    }
                                    .render(12.),
                                )
                            }),
                    )
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
                            .hover(|style| style.bg(rgb(p.surface_subtle)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if entry.directory {
                                    this.toggle_folder(path.clone(), cx);
                                } else {
                                    cx.emit(FilePreviewEvent::OpenRequested(path.clone()));
                                }
                            }))
                    }),
            );
        }
        list
    }

    /// Retain the folder independently of the currently previewed file.
    pub fn set_navigation(
        &mut self,
        path: String,
        mut entries: Vec<FilePreviewEntry>,
        truncated: bool,
        cx: &mut Context<Self>,
    ) {
        Self::append_truncation(&mut entries, truncated);
        if let Some((root, _)) = &self.navigation {
            if root != &path && std::path::Path::new(&path).starts_with(root) {
                let node = self.folders.entry(path).or_default();
                node.entries = Some(entries);
                node.expanded = true;
                cx.notify();
                return;
            }
            if root != &path {
                self.folders.clear();
                self.navigation_scroll = Default::default();
            }
        } else {
            self.folders.clear();
            self.navigation_scroll = Default::default();
        }
        self.navigation = Some((path, entries));
        cx.notify();
    }

    pub fn set_directory(
        &mut self,
        path: String,
        parent: Option<String>,
        entries: Vec<FilePreviewEntry>,
        cx: &mut Context<Self>,
    ) {
        self.set_navigation(path.clone(), entries.clone(), false, cx);
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
                let list = if self.expanded {
                    self.navigation_tree(entries, cx)
                } else {
                    self.directory_list(entries, &self.scroll, cx)
                };
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
                    .when(
                        self.expanded
                            && !self.markdown_preview
                            && matches!(self.page, Page::File(_)),
                        |row| row.child(self.search_controls(cx)),
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
            .child({
                let mut content = div().flex_1().min_h_0().min_w_0().flex();
                if self.expanded && !matches!(self.page, Page::Directory(_)) {
                    if let Some((path, entries)) = &self.navigation {
                        content = content.child(
                            div()
                                .id("file-preview-navigation")
                                .debug_selector(|| "file-preview-navigation".into())
                                .w(px(260.))
                                .max_w(gpui::relative(0.4))
                                .h_full()
                                .flex_shrink_0()
                                .min_h_0()
                                .flex()
                                .flex_col()
                                .border_r_1()
                                .border_color(rgb(p.border))
                                .child(
                                    div()
                                        .px_4()
                                        .py_2()
                                        .text_size(px(12.))
                                        .truncate()
                                        .child(display_path(path)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_h_0()
                                        .child(self.navigation_tree(entries, cx)),
                                ),
                        );
                    }
                }
                content.child(
                    div()
                        .flex_1()
                        .w(px(0.))
                        .min_w_0()
                        .min_h_0()
                        .flex()
                        .child(body),
                )
            })
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
    fn tree_reuses_children_retries_errors_and_ignores_cancelled_requests(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            let view = cx.new(FilePreview::new);
            view.update(cx, |view, cx| {
                view.set_directory(
                    "/project".into(),
                    None,
                    vec![FilePreviewEntry {
                        name: "src".into(),
                        path: "/project/src".into(),
                        directory: true,
                        previewable: true,
                    }],
                    cx,
                );
                view.toggle_folder("/project/src".into(), cx);
                let first = view.next_request;
                view.set_directory_children(
                    "/project/src".into(),
                    first + 1,
                    Ok((vec![], false)),
                    cx,
                );
                assert!(view.folders["/project/src"].entries.is_none());
                view.set_directory_children(
                    "/project/src".into(),
                    first,
                    Err("读取失败".into()),
                    cx,
                );
                assert!(view.folders["/project/src"].error.is_some());
                view.toggle_folder("/project/src".into(), cx);
                view.toggle_folder("/project/src".into(), cx);
                let second = view.next_request;
                assert!(second > first);
                view.cancel_directory_requests(cx);
                view.set_directory_children("/project/src".into(), second, Ok((vec![], false)), cx);
                assert!(view.folders["/project/src"].entries.is_none());
                view.toggle_folder("/project/src".into(), cx);
                let third = view.next_request;
                view.set_directory_children("/project/src".into(), third, Ok((vec![], false)), cx);
                assert!(view.folders["/project/src"].entries.is_some());
                view.toggle_folder("/project/src".into(), cx);
                view.toggle_folder("/project/src".into(), cx);
                assert_eq!(
                    view.next_request, third,
                    "cached children must not request another read"
                );
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
    fn expanded_source_search_steps_through_matches_and_resets_for_a_new_file(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, window) = cx.add_window_view(|_, cx| {
            let mut preview = FilePreview::new(cx);
            preview.set_file(
                "main.go".into(),
                None,
                "go".into(),
                "needle\nNeedle".into(),
                cx,
            );
            preview
        });
        assert!(window.debug_bounds("file-preview-search").is_none());
        let entry = view.update(window, |view, cx| {
            view.set_expanded(true, cx);
            view.search.clone()
        });
        assert!(window.debug_bounds("file-preview-search").is_some());
        window.update(|window, cx| window.focus(&entry.focus_handle(cx), cx));
        entry.update(window, |entry, cx| entry.set_value("needle", cx));
        assert_eq!(
            view.read_with(window, |view, _| view.search_match),
            Some((0, 2))
        );
        window.simulate_keystrokes("enter");
        assert_eq!(
            view.read_with(window, |view, _| view.search_match),
            Some((1, 2))
        );
        window.simulate_keystrokes("shift-enter");
        assert_eq!(
            view.read_with(window, |view, _| view.search_match),
            Some((0, 2))
        );
        view.update(window, |view, cx| {
            view.set_file("other.go".into(), None, "go".into(), "needle".into(), cx);
            assert!(view.search.read(cx).value().is_empty());
            assert!(view.search_match.is_none());
        });
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
