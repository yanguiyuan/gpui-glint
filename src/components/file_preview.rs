//! Reusable read-only file/directory preview. The host supplies data and performs I/O.
use super::{
    CodeCard, CodeCardEvent, CodeSelection, DocumentEditor, Icon, IconButton, Popover,
    SegmentOption, SegmentedControl, Skeleton, TextEntry,
};
use crate::paths::display_path;
use crate::theme::ThemePreset;
use gpui::{
    AppContext, Context, Entity, EventEmitter, Render, SharedString, Window, div, prelude::*, px,
    rgb,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BreadcrumbSegment {
    pub name: String,
    pub path: String,
    pub dir_path: String,
    pub is_file: bool,
}

pub fn split_breadcrumbs(full_path: &str, is_leaf_file: bool) -> Vec<BreadcrumbSegment> {
    let trimmed = full_path.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let (prefix, path_part) = if let Some(rest) = trimmed.strip_prefix("ssh://") {
        if let Some(slash_idx) = rest.find('/') {
            (format!("ssh://{}", &rest[..slash_idx]), &rest[slash_idx..])
        } else {
            (format!("ssh://{rest}"), "")
        }
    } else {
        (String::new(), trimmed)
    };

    if path_part.is_empty() {
        if !prefix.is_empty() {
            return vec![BreadcrumbSegment {
                name: prefix.clone(),
                path: prefix.clone(),
                dir_path: prefix,
                is_file: false,
            }];
        }
        return Vec::new();
    }

    let p = std::path::Path::new(path_part);
    let mut prefix_str = prefix;
    let mut has_root = false;
    let mut normal_names = Vec::new();

    for comp in p.components() {
        match comp {
            std::path::Component::Prefix(pfx) => {
                prefix_str = pfx.as_os_str().to_string_lossy().into_owned();
            }
            std::path::Component::RootDir => {
                has_root = true;
            }
            std::path::Component::Normal(name) => {
                normal_names.push(name.to_string_lossy().into_owned());
            }
            _ => {}
        }
    }

    if normal_names.is_empty() {
        let display = if has_root {
            if prefix_str.is_empty() {
                "/".to_owned()
            } else if prefix_str.ends_with('/') || prefix_str.ends_with('\\') {
                prefix_str
            } else {
                format!("{prefix_str}/")
            }
        } else {
            prefix_str
        };
        return vec![BreadcrumbSegment {
            name: display.clone(),
            path: display.clone(),
            dir_path: display,
            is_file: false,
        }];
    }

    let skip_first = has_root
        && normal_names.len() >= 2
        && (normal_names[0].eq_ignore_ascii_case("users")
            || normal_names[0].eq_ignore_ascii_case("home"));

    let mut segments = Vec::new();
    let mut current_path = if has_root {
        if prefix_str.is_empty() {
            "/".to_owned()
        } else if prefix_str.ends_with('/') || prefix_str.ends_with('\\') {
            prefix_str.clone()
        } else {
            format!("{prefix_str}/")
        }
    } else if !prefix_str.is_empty() {
        format!("{prefix_str}/")
    } else {
        String::new()
    };

    let total = normal_names.len();
    for (i, name) in normal_names.into_iter().enumerate() {
        let is_last = i + 1 == total;
        let is_file = is_last && is_leaf_file;

        if current_path.is_empty()
            || current_path == "/"
            || current_path.ends_with('/')
            || current_path.ends_with('\\')
        {
            current_path.push_str(&name);
        } else {
            current_path.push('/');
            current_path.push_str(&name);
        }

        if i == 0 && skip_first {
            continue;
        }

        let dir_path = if is_file {
            std::path::Path::new(&current_path)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| ".".to_owned())
        } else {
            current_path.clone()
        };

        segments.push(BreadcrumbSegment {
            name,
            path: current_path.clone(),
            dir_path,
            is_file,
        });
    }

    segments
}

pub const ELLIPSIS_BREADCRUMB_INDEX: usize = usize::MAX;

pub fn calculate_visible_breadcrumb_start(
    segments: &[BreadcrumbSegment],
    available_width: f32,
) -> usize {
    if segments.len() <= 1 {
        return 0;
    }

    let estimate_item_width = |seg: &BreadcrumbSegment| -> f32 {
        let mut text_w = 0.0;
        for ch in seg.name.chars() {
            if ch.is_ascii() {
                text_w += 7.0;
            } else {
                text_w += 13.0;
            }
        }
        let icon_w = if seg.is_file { 17.0 } else { 0.0 };
        // chevron (12) + padding (8) + gap (4) + icon + text
        24.0 + icon_w + text_w
    };

    let total_width: f32 = segments.iter().map(estimate_item_width).sum();
    if total_width <= available_width {
        return 0;
    }

    let ellipsis_width = 34.0;
    let space = (available_width - ellipsis_width).max(60.0);

    let last_idx = segments.len() - 1;
    let mut used = estimate_item_width(&segments[last_idx]);
    let mut start_idx = last_idx;

    while start_idx > 0 {
        let prev_idx = start_idx - 1;
        let w = estimate_item_width(&segments[prev_idx]);
        if used + w <= space {
            used += w;
            start_idx = prev_idx;
        } else {
            break;
        }
    }

    start_idx
}

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
    header_accessory: Option<gpui::AnyView>,
    quote_label: Option<SharedString>,
    #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
    html: Option<String>,
    #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
    browser: Option<Result<Entity<super::WebView>, String>>,
    expanded: bool,
    standalone: bool,
    content_only: bool,
    navigation: Option<(String, Vec<FilePreviewEntry>)>,
    navigation_scroll: gpui::UniformListScrollHandle,
    folders: std::collections::HashMap<String, FolderNode>,
    next_request: u64,
    open_breadcrumb: Option<usize>,
    breadcrumb_scroll: gpui::ScrollHandle,
    breadcrumb_width: Option<f32>,
    path: String,
    parent: Option<String>,
    page: Page,
    theme: ThemePreset,
    markdown: Option<Entity<DocumentEditor>>,
    markdown_preview: bool,
    scroll: gpui::ScrollHandle,
    search: Entity<TextEntry>,
    search_match: Option<(usize, usize)>,
    search_open: bool,
    pending_search: Option<(String, Option<usize>)>,
    pending_line: Option<usize>,
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
            header_accessory: None,
            quote_label: None,
            expanded: false,
            standalone: false,
            content_only: false,
            navigation: None,
            navigation_scroll: Default::default(),
            folders: Default::default(),
            next_request: 0,
            open_breadcrumb: None,
            breadcrumb_scroll: gpui::ScrollHandle::new(),
            breadcrumb_width: None,
            path: String::new(),
            parent: None,
            page: Page::Loading,
            theme: ThemePreset::default(),
            markdown: None,
            markdown_preview: false,
            scroll: Default::default(),
            search,
            search_match: None,
            search_open: false,
            pending_search: None,
            pending_line: None,
            _search_subscription: search_subscription,
            subscriptions: Vec::new(),
        }
    }

    /// Label used for the action that emits a selected source excerpt.
    pub fn quote_label(mut self, label: impl Into<SharedString>) -> Self {
        self.quote_label = Some(label.into());
        self
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
    /// Optional host control shown alongside the path in the fixed header.
    pub fn set_header_accessory(
        &mut self,
        accessory: Option<gpui::AnyView>,
        cx: &mut Context<Self>,
    ) {
        if self.header_accessory.as_ref().map(|view| view.entity_id())
            != accessory.as_ref().map(|view| view.entity_id())
        {
            self.header_accessory = accessory;
            cx.notify();
        }
    }

    pub fn open_breadcrumb(&self) -> Option<usize> {
        self.open_breadcrumb
    }

    pub fn set_open_breadcrumb(&mut self, idx: Option<usize>, cx: &mut Context<Self>) {
        if self.open_breadcrumb != idx {
            self.open_breadcrumb = idx;
            cx.notify();
        }
    }

    /// Synchronize an explorer's selected row without replacing its directory data.
    pub fn set_selected_path(&mut self, path: String, cx: &mut Context<Self>) {
        if self.path != path {
            self.path = path;
            self.open_breadcrumb = None;
            cx.notify();
        }
    }

    /// Render a document pane without the directory navigation owned by its host.
    pub fn set_content_only(&mut self, content_only: bool, cx: &mut Context<Self>) {
        if self.content_only != content_only {
            self.content_only = content_only;
            cx.notify();
        }
    }

    /// Full-page hosts own navigation and do not need sidebar expand/close controls.
    pub fn set_standalone(&mut self, standalone: bool, cx: &mut Context<Self>) {
        if self.standalone != standalone {
            self.standalone = standalone;
            cx.notify();
        }
    }

    pub fn set_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        if self.expanded != expanded {
            self.expanded = expanded;
            self.breadcrumb_width = None;
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
        self.pending_line = None;
        self.open_breadcrumb = None;
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
        self.open_breadcrumb = None;
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
        self.open_breadcrumb = None;
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
        self.open_breadcrumb = None;
        let language = crate::syntax::global()
            .language_for_path(&path)
            .unwrap_or_else(|| match Icon::for_file_name(&path) {
                Icon::Docker => "dockerfile".to_owned(),
                Icon::Makefile => "makefile".to_owned(),
                _ => language,
            });
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
            let editor = CodeCard::new(cx, language, text)
                .read_only(true)
                .quote_context(true)
                .track_selected_line(true)
                .show_header(false)
                .viewport()
                .theme(self.theme);
            match &self.quote_label {
                Some(label) => editor.quote_label(label.clone()),
                None => editor,
            }
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
        if let Some(line) = self.pending_line.take() {
            self.reveal_line(line, cx);
        }
        if let Some((query, line)) = self.pending_search.take() {
            self.set_document_search(query, line, cx);
        }
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

    /// Import a project text-search query, including when the file is still loading.
    pub fn set_document_search(
        &mut self,
        query: String,
        line: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        self.search_open = !query.is_empty();
        self.search
            .update(cx, |search, cx| search.set_value(query.clone(), cx));
        if let Page::File(editor) = &self.page {
            self.search_match = editor.update(cx, |editor, cx| {
                editor.set_search_query(query, cx);
                let count = editor.search_match_count();
                let active = editor.reveal_search_at_line(line.unwrap_or(1), cx);
                active.map(|index| (index, count))
            });
        } else {
            self.pending_search = Some((query, line));
        }
        cx.notify();
    }

    /// Controls that a document host can place beside its file tabs.
    pub fn document_controls(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .items_center()
            .gap_2()
            .when(
                self.expanded && !self.markdown_preview && matches!(self.page, Page::File(_)),
                |row| {
                    if self.content_only {
                        row.child(
                            IconButton::new(Icon::Search, "搜索文件内容")
                                .plain()
                                .theme(self.theme)
                                .render()
                                .debug_selector(|| "file-preview-search-trigger".into())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.search_open = !this.search_open;
                                    if this.search_open {
                                        use gpui::Focusable;
                                        window.focus(&this.search.focus_handle(cx), cx);
                                    } else {
                                        window.blur();
                                    }
                                    cx.notify();
                                })),
                        )
                    } else {
                        row.child(self.search_controls(cx))
                    }
                },
            )
            .when(self.has_rendered_preview(), |row| {
                let entity = cx.entity();
                row.child(
                    SegmentedControl::new([SegmentOption::new("源码"), SegmentOption::new("预览")])
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
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" && this.content_only {
                            this.search_open = false;
                            window.blur();
                            cx.stop_propagation();
                            cx.notify();
                        } else if event.keystroke.key == "enter" {
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
                        (Icon::ChevronUp, "上一个匹配", true),
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
        } else if matches!(self.page, Page::Loading) {
            self.pending_line = Some(line);
        }
    }

    /// Source snapshot and byte offset of a Go identifier under the pointer.
    pub fn go_source_at(
        &self,
        position: gpui::Point<gpui::Pixels>,
        cx: &gpui::App,
    ) -> Option<(String, SharedString, usize)> {
        let Page::File(editor) = &self.page else {
            return None;
        };
        if std::path::Path::new(&self.path).extension()?.to_str()? != "go" {
            return None;
        }
        let (source, offset) = editor.read(cx).identifier_source_at(position)?;
        Some((self.path.clone(), source, offset))
    }

    /// Decorate a source range without changing the selection or search results.
    pub fn set_source_underline(
        &mut self,
        range: Option<std::ops::Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        if let Page::File(editor) = &self.page {
            editor.update(cx, |editor, cx| editor.set_source_underline(range, cx));
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
        if node.expanded && node.entries.is_none() {
            if let Some(entries) = Self::read_local_directory_entries(&path) {
                node.entries = Some(entries);
            } else if node.loading.is_none() {
                self.next_request += 1;
                node.loading = Some(self.next_request);
                node.error = None;
                cx.emit(FilePreviewEvent::DirectoryRequested {
                    path,
                    request: self.next_request,
                });
            }
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
        if rows.is_empty() {
            return div()
                .id("file-preview-tree")
                .h_full()
                .child(div().p_4().child("空目录"));
        }
        let rows = std::rc::Rc::new(rows);
        let count = rows.len();
        div().id("file-preview-tree").h_full().min_h_0().child(
            gpui::uniform_list(
                "file-preview-tree-rows",
                count,
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    range
                        .map(|index| {
                            let (entry, depth) = rows[index].clone();
                            let path = entry.path.clone();
                            let expanded =
                                this.folders.get(&path).is_some_and(|node| node.expanded);

                            div()
                                .id(SharedString::from(format!("tree-entry:{path}")))
                                .debug_selector(move || format!("preview-entry:{path}"))
                                .h(px(26.))
                                .w_full()
                                .flex()
                                .items_center()
                                .gap_1()
                                .pl(px(8. + depth as f32 * 16.))
                                .pr_2()
                                .py_1()
                                .when(entry.path == this.path, |row| row.bg(rgb(p.accent_tint)))
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
                                            row.child(Icon::disclosure(12., expanded))
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
                                                cx.emit(FilePreviewEvent::OpenRequested(
                                                    path.clone(),
                                                ));
                                            }
                                        }))
                                })
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .h_full()
            .w_full()
            .track_scroll(&self.navigation_scroll),
        )
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
        self.open_breadcrumb = None;
        self.parent = parent;
        self.clear_html();
        self.page = Page::Directory(entries);
        self.markdown = None;
        self.markdown_preview = false;
        self.subscriptions.clear();
        self.scroll = Default::default();
        cx.notify();
    }

    fn read_local_directory_entries(dir_path: &str) -> Option<Vec<FilePreviewEntry>> {
        let p = std::path::Path::new(dir_path);
        if !p.is_dir() {
            return None;
        }
        let read_dir = std::fs::read_dir(p).ok()?;
        let mut entries = Vec::new();
        for item in read_dir.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            if name == ".DS_Store" || name == ".git" {
                continue;
            }
            let item_path = item.path();
            let is_dir = item_path.is_dir();
            entries.push(FilePreviewEntry {
                name,
                path: item_path.to_string_lossy().into_owned(),
                directory: is_dir,
                previewable: true,
            });
        }
        entries.sort_by(|a, b| match (a.directory, b.directory) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });
        Some(entries)
    }

    fn request_directory_if_needed(&mut self, dir_path: &str, cx: &mut Context<Self>) {
        if self.navigation.as_ref().is_some_and(|(root, _)| root == dir_path) {
            return;
        }
        let node = self.folders.entry(dir_path.to_owned()).or_default();
        if node.entries.is_some() {
            return;
        }
        if let Some(entries) = Self::read_local_directory_entries(dir_path) {
            node.entries = Some(entries);
            return;
        }
        if node.loading.is_none() {
            self.next_request += 1;
            node.loading = Some(self.next_request);
            node.error = None;
            cx.emit(FilePreviewEvent::DirectoryRequested {
                path: dir_path.to_owned(),
                request: self.next_request,
            });
        }
    }

    fn breadcrumb_dropdown(
        &self,
        idx: usize,
        seg: &BreadcrumbSegment,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let p = self.theme.palette();
        let folder_name = std::path::Path::new(&seg.dir_path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| {
                if seg.dir_path.is_empty() || seg.dir_path == "/" {
                    "根目录".to_owned()
                } else {
                    seg.name.clone()
                }
            });

        let dir_path_clone = seg.dir_path.clone();
        let header = div()
            .id("breadcrumb-dropdown-header")
            .debug_selector(|| "breadcrumb-dropdown-header".into())
            .flex()
            .items_center()
            .gap_2()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(rgb(p.surface_subtle))
            .cursor_pointer()
            .hover(|el| el.bg(rgb(p.workspace)))
            .child(Icon::Folder.render(13.).text_color(rgb(p.text_muted)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_ellipsis()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_size(px(12.))
                    .text_color(rgb(p.text))
                    .child(folder_name),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_breadcrumb = None;
                cx.emit(FilePreviewEvent::OpenRequested(dir_path_clone.clone()));
            }));

        let cached_entries = if let Some((root, entries)) = &self.navigation {
            if root == &seg.dir_path {
                Some(entries.clone())
            } else {
                None
            }
        } else {
            None
        };

        let node = self.folders.get(&seg.dir_path);
        let entries = cached_entries
            .or_else(|| node.and_then(|n| n.entries.clone()))
            .or_else(|| Self::read_local_directory_entries(&seg.dir_path));

        let is_loading = node.is_some_and(|n| n.loading.is_some()) && entries.is_none();
        let error_msg = node.and_then(|n| n.error.clone());

        let body: gpui::AnyElement = if is_loading {
            div()
                .flex()
                .items_center()
                .gap_2()
                .px(px(8.))
                .py(px(6.))
                .text_size(px(12.))
                .text_color(rgb(p.text_muted))
                .child(Icon::Spinner.render(12.).text_color(rgb(p.text_muted)))
                .child("正在加载…")
                .into_any_element()
        } else if let Some(err) = error_msg {
            div()
                .px(px(8.))
                .py(px(6.))
                .text_size(px(12.))
                .text_color(rgb(0xef4444))
                .child(err)
                .into_any_element()
        } else if let Some(entries) = entries {
            if entries.is_empty() {
                div()
                    .px(px(8.))
                    .py(px(6.))
                    .text_size(px(12.))
                    .text_color(rgb(p.text_muted))
                    .child("空目录")
                    .into_any_element()
            } else {
                let mut rows = Vec::new();
                self.tree_rows(&entries, 0, &mut rows);

                let current_path = self.path.clone();
                let mut list = div()
                    .id(SharedString::from(format!("breadcrumb-entries-list-{idx}")))
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .max_h(px(260.))
                    .overflow_y_scroll();

                for (entry, depth) in rows {
                    let entry_path = entry.path.clone();
                    let is_active = !entry.path.is_empty() && entry.path == current_path;
                    let is_placeholder = entry.path.is_empty();

                    let is_folder_expanded = if entry.directory {
                        self.folders.get(&entry.path).is_some_and(|n| n.expanded)
                    } else {
                        false
                    };

                    let icon = if entry.directory {
                        if is_folder_expanded {
                            Icon::FolderOpen.render(13.).text_color(rgb(p.text_muted)).into_any_element()
                        } else {
                            Icon::Folder.render(13.).text_color(rgb(p.text_muted)).into_any_element()
                        }
                    } else if is_placeholder {
                        div().w(px(13.)).into_any_element()
                    } else {
                        let file_icon = Icon::for_file_name(&entry.name);
                        let icon_color = Icon::color_for_file_name(&entry.name);
                        let svg = file_icon.render(13.);
                        if let Some(c) = icon_color {
                            svg.text_color(rgb(c))
                        } else {
                            svg.text_color(rgb(p.text))
                        }
                        .into_any_element()
                    };

                    let entry_click_path = entry.path.clone();
                    let is_dir = entry.directory;

                    let mut entry_row = div()
                        .id(SharedString::from(format!(
                            "breadcrumb-entry-{}-{}",
                            depth,
                            if entry.path.is_empty() {
                                &entry.name
                            } else {
                                &entry.path
                            }
                        )))
                        .debug_selector(move || format!("breadcrumb-entry:{}", entry_path))
                        .relative()
                        .flex()
                        .items_center()
                        .gap_2()
                        .pl(px(8. + depth as f32 * 16.))
                        .pr(px(8.))
                        .py(px(4.))
                        .rounded(px(4.))
                        .when(is_active || (is_dir && is_folder_expanded && depth == 0), |el| {
                            el.bg(rgb(p.surface_subtle))
                        })
                        .hover(|el| el.bg(rgb(p.surface_subtle)))
                        .when(!is_placeholder, |el| el.cursor_pointer());

                    for d in 1..=depth {
                        entry_row = entry_row.child(
                            div()
                                .absolute()
                                .left(px(8. + (d - 1) as f32 * 16. + 6.))
                                .top_0()
                                .bottom_0()
                                .w(px(1.))
                                .bg(rgb(p.border)),
                        );
                    }

                    entry_row = entry_row
                        .child(icon)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_ellipsis()
                                .text_size(px(12.))
                                .text_color(if is_active {
                                    rgb(p.accent)
                                } else if is_placeholder {
                                    rgb(p.text_muted)
                                } else {
                                    rgb(p.text)
                                })
                                .child(entry.name),
                        );

                    if is_dir {
                        entry_row = entry_row.on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_folder(entry_click_path.clone(), cx);
                        }));
                    } else if !is_placeholder {
                        entry_row = entry_row.on_click(cx.listener(move |this, _, _, cx| {
                            this.open_breadcrumb = None;
                            cx.emit(FilePreviewEvent::OpenRequested(entry_click_path.clone()));
                        }));
                    }

                    list = list.child(entry_row);
                }
                list.into_any_element()
            }
        } else {
            div()
                .flex()
                .items_center()
                .gap_2()
                .px(px(8.))
                .py(px(6.))
                .text_size(px(12.))
                .text_color(rgb(p.text_muted))
                .child(Icon::Spinner.render(12.).text_color(rgb(p.text_muted)))
                .child("正在加载…")
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap_1()
            .w_full()
            .child(header)
            .child(div().h(px(1.)).bg(rgb(p.border)).my(px(2.)))
            .child(body)
    }

    fn breadcrumb_ellipsis_dropdown(
        &self,
        hidden_segments: &[BreadcrumbSegment],
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let p = self.theme.palette();
        let weak = cx.entity().downgrade();

        let mut list = div()
            .id("breadcrumb-ellipsis-list")
            .flex()
            .flex_col()
            .gap(px(1.))
            .w_full()
            .max_h(px(280.))
            .overflow_y_scroll()
            .child(
                div()
                    .px_2()
                    .py(px(3.))
                    .text_size(px(11.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(rgb(p.text_muted))
                    .child("上层路径"),
            )
            .child(div().h(px(1.)).bg(rgb(p.border)).my(px(2.)));

        for seg in hidden_segments {
            let seg_clone = seg.clone();
            let weak_click = weak.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("breadcrumb-ellipsis-item-{}", seg.name)))
                    .debug_selector(move || format!("breadcrumb-ellipsis-item:{}", seg_clone.name))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(6.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .text_size(px(12.))
                    .text_color(rgb(p.text))
                    .hover(|el| el.bg(rgb(p.surface_subtle)))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(rgb(p.text_muted))
                            .child(Icon::Folder.render(13.)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(seg.name.clone()),
                    )
                    .on_click({
                        let seg_dir_path = seg.dir_path.clone();
                        move |_, _, cx| {
                            cx.stop_propagation();
                            let _ = weak_click.update(cx, |this, cx| {
                                this.open_breadcrumb = None;
                                cx.emit(FilePreviewEvent::OpenRequested(seg_dir_path.clone()));
                            });
                        }
                    }),
            );
        }

        list.into_any_element()
    }

    fn render_breadcrumbs(&self, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let is_leaf_file = match &self.page {
            Page::Directory(_) => false,
            Page::File(_) | Page::Image(_) => true,
            _ => !std::path::Path::new(&self.path).is_dir(),
        };

        let segments = split_breadcrumbs(&self.path, is_leaf_file);
        let p = self.theme.palette();

        let available_width = self
            .breadcrumb_width
            .unwrap_or(if self.expanded { 1200.0 } else { 260.0 });
        let start_idx = calculate_visible_breadcrumb_start(&segments, available_width);

        let weak = cx.entity().downgrade();
        let scroll_handle = self.breadcrumb_scroll.clone();

        let mut row = div()
            .on_children_prepainted(move |_, _, cx| {
                let width = f32::from(scroll_handle.bounds().size.width);
                if width > 0.0 {
                    let _ = weak.update(cx, |this, cx| {
                        if this.breadcrumb_width.map_or(true, |w| (w - width).abs() >= 8.0) {
                            this.breadcrumb_width = Some(width);
                            let id = cx.entity_id();
                            cx.defer(move |cx| cx.notify(id));
                        }
                    });
                }
            })
            .id("breadcrumb-segments-bar")
            .track_scroll(&self.breadcrumb_scroll)
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(2.))
            .overflow_x_hidden();

        if start_idx > 0 {
            row = row.child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .text_color(rgb(p.text_muted))
                    .child(Icon::ChevronRight.render(10.)),
            );

            let is_ellipsis_open = self.open_breadcrumb == Some(ELLIPSIS_BREADCRUMB_INDEX);
            let ellipsis_trigger = div()
                .id("breadcrumb-seg-ellipsis")
                .debug_selector(|| "breadcrumb-seg:ellipsis".into())
                .cursor_pointer()
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .px(px(5.))
                .py(px(2.))
                .rounded(px(4.))
                .text_size(px(12.))
                .text_color(if is_ellipsis_open {
                    rgb(p.accent)
                } else {
                    rgb(p.text_muted)
                })
                .hover(|el| el.bg(rgb(p.surface_subtle)).text_color(rgb(p.text)))
                .child("…");

            let weak_ell = cx.entity().downgrade();
            let ellipsis_content = if is_ellipsis_open {
                let hidden_segments = segments[0..start_idx].to_vec();
                self.breadcrumb_ellipsis_dropdown(&hidden_segments, cx)
            } else {
                div().into_any_element()
            };
            let ellipsis_popover = Popover::new(
                "breadcrumb-popover-ellipsis",
                ellipsis_trigger,
                ellipsis_content,
            )
            .open(is_ellipsis_open)
            .align_left()
            .width(220.)
            .trigger_height(24.)
            .gap(4.)
            .padding(6.)
            .theme(self.theme)
            .on_open_change(move |open, _window, cx| {
                let _ = weak_ell.update(cx, |this, cx| {
                    if open {
                        this.open_breadcrumb = Some(ELLIPSIS_BREADCRUMB_INDEX);
                    } else if this.open_breadcrumb == Some(ELLIPSIS_BREADCRUMB_INDEX) {
                        this.open_breadcrumb = None;
                    }
                    cx.notify();
                });
            });

            row = row.child(ellipsis_popover);
        }

        for idx in start_idx..segments.len() {
            let seg = &segments[idx];
            let is_last = idx == segments.len() - 1;
            row = row.child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .text_color(rgb(p.text_muted))
                    .child(Icon::ChevronRight.render(10.)),
            );

            let is_open = self.open_breadcrumb == Some(idx);
            let seg_clone = seg.clone();
            let seg_dir_path = seg.dir_path.clone();

            let trigger_btn = div()
                .id(SharedString::from(format!("breadcrumb-seg-{idx}")))
                .debug_selector(move || format!("breadcrumb-seg:{}", seg_clone.name))
                .cursor_pointer()
                .when(!is_last, |el| el.flex_shrink_0())
                .when(is_last, |el| el.flex_shrink_1().min_w_0())
                .flex()
                .items_center()
                .gap(px(4.))
                .px(px(4.))
                .py(px(2.))
                .rounded(px(4.))
                .text_size(px(12.))
                .text_color(if is_open {
                    rgb(p.accent)
                } else {
                    rgb(p.text)
                })
                .hover(|el| el.bg(rgb(p.surface_subtle)).text_color(rgb(p.text)))
                .when(seg.is_file, |el| {
                    let file_icon = Icon::for_file_name(&seg.name);
                    let icon_color = Icon::color_for_file_name(&seg.name);
                    let svg = file_icon.render(13.);
                    let svg = if let Some(color) = icon_color {
                        svg.text_color(rgb(color))
                    } else {
                        svg.text_color(rgb(p.text))
                    };
                    el.child(svg)
                })
                .child(
                    div()
                        .when(!is_last, |el| el.whitespace_nowrap())
                        .when(is_last, |el| el.min_w_0().truncate())
                        .child(seg.name.clone()),
                );

            let weak = cx.entity().downgrade();
            let dropdown_content = if is_open {
                self.breadcrumb_dropdown(idx, seg, cx).into_any_element()
            } else {
                div().into_any_element()
            };
            let popover = Popover::new(
                SharedString::from(format!("breadcrumb-popover-{idx}")),
                trigger_btn,
                dropdown_content,
            )
            .open(is_open)
            .align_left()
            .width(240.)
            .trigger_height(24.)
            .gap(4.)
            .padding(6.)
            .theme(self.theme)
            .on_open_change(move |open, _window, cx| {
                let _ = weak.update(cx, |this, cx| {
                    if open {
                        this.open_breadcrumb = Some(idx);
                        this.request_directory_if_needed(&seg_dir_path, cx);
                    } else if this.open_breadcrumb == Some(idx) {
                        this.open_breadcrumb = None;
                    }
                    cx.notify();
                });
            });

            row = row.child(popover);
        }

        row
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
            Page::Loading => body.child(
                div()
                    .p_4()
                    .debug_selector(|| "file-preview-loading-skeleton".into())
                    .child(
                        Skeleton::new("file-preview-loading")
                            .lines(12)
                            .height(12.)
                            .gap(14.)
                            .theme(self.theme),
                    ),
            ),
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
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(rgb(p.panel))
            .text_color(rgb(p.text))
            .when(!self.content_only, |root| {
                root.child(
                    div()
                        .debug_selector(|| "file-preview-header".into())
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_color(rgb(p.border))
                        .when(!self.standalone, |row| {
                            row.child(self.render_breadcrumbs(cx))
                        })
                        .when(self.standalone, |row| {
                            row.child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .text_size(px(12.))
                                    .child(
                                        if matches!(self.page, Page::Directory(_)) {
                                            "项目文件".to_owned()
                                        } else {
                                            std::path::Path::new(&self.path)
                                                .file_name()
                                                .map(|name| name.to_string_lossy().into_owned())
                                                .unwrap_or_default()
                                        },
                                    ),
                            )
                        })
                        .children(self.header_accessory.clone())
                        .child(self.document_controls(cx))
                        .when(!self.standalone, |row| {
                            row.child(
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
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        cx.emit(FilePreviewEvent::ExpandedChanged(!this.expanded));
                                    },
                                )),
                            )
                        })
                        .when(!self.standalone, |row| {
                            row.child(
                                IconButton::new(Icon::Close, "关闭预览")
                                    .plain()
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(FilePreviewEvent::CloseRequested)
                                    })),
                            )
                        }),
                )
            })
            .child({
                let mut content = div().flex_1().min_h_0().min_w_0().flex();
                if self.expanded && !self.content_only && !matches!(self.page, Page::Directory(_)) {
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
                                .child(div().px_4().py_2().text_size(px(12.)).truncate().child(
                                    if self.standalone {
                                        "项目文件".to_owned()
                                    } else {
                                        display_path(path)
                                    },
                                ))
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
            .when(
                self.content_only
                    && self.search_open
                    && !self.markdown_preview
                    && matches!(self.page, Page::File(_)),
                |root| {
                    root.child(
                        div()
                            .absolute()
                            .top(px(8.))
                            .right(px(12.))
                            .debug_selector(|| "file-preview-search-capsule".into())
                            .occlude()
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_3()
                            .py_1()
                            .rounded_full()
                            .border_1()
                            .border_color(rgb(p.border))
                            .bg(rgb(p.panel))
                            .shadow_md()
                            .child(self.search_controls(cx))
                            .child(
                                IconButton::new(Icon::Close, "关闭搜索")
                                    .plain()
                                    .theme(self.theme)
                                    .render()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.search_open = false;
                                        window.blur();
                                        cx.notify();
                                    })),
                            ),
                    )
                },
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Focusable;
    use std::{cell::RefCell, rc::Rc};
    #[gpui::test]
    fn large_directory_builds_only_visible_rows(cx: &mut gpui::TestAppContext) {
        let (view, window) = cx.add_window_view(|_, cx| FilePreview::new(cx));
        window.simulate_resize(gpui::size(px(600.), px(400.)));
        view.update(window, |view, cx| {
            view.set_expanded(true, cx);
            view.set_directory(
                "/project".into(),
                None,
                (0..1000)
                    .map(|index| FilePreviewEntry {
                        name: format!("file-{index}.rs"),
                        path: format!("/project/file-{index}.rs"),
                        directory: false,
                        previewable: true,
                    })
                    .collect(),
                cx,
            );
        });
        assert!(
            window
                .debug_bounds("preview-entry:/project/file-0.rs")
                .is_some()
        );
        assert!(
            window
                .debug_bounds("preview-entry:/project/file-999.rs")
                .is_none()
        );
    }

    #[gpui::test]
    fn project_query_survives_async_file_loading(cx: &mut gpui::TestAppContext) {
        let (view, window) = cx.add_window_view(|_, cx| FilePreview::new(cx));
        view.update(window, |view, cx| {
            view.set_content_only(true, cx);
            view.set_loading("main.rs".into(), cx);
            view.set_document_search("测试".into(), Some(2), cx);
            view.set_file(
                "main.rs".into(),
                None,
                "rust".into(),
                "// header\n// 测试\n".into(),
                cx,
            );
        });
        window.run_until_parked();
        view.read_with(window, |view, cx| {
            assert_eq!(view.search.read(cx).value(), "测试");
            assert!(view.search_open);
            let Page::File(editor) = &view.page else {
                panic!("file expected")
            };
            assert_eq!(editor.read(cx).search_query(), "测试");
            assert_eq!(view.search_match, Some((0, 1)));
        });
    }

    struct SearchHost(Entity<FilePreview>);
    impl Render for SearchHost {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let controls = self.0.update(cx, |view, cx| view.document_controls(cx));
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(controls)
                .child(div().flex_1().min_h_0().child(self.0.clone()))
        }
    }

    #[gpui::test]
    fn document_search_opens_focused_capsule_and_escape_closes_it(cx: &mut gpui::TestAppContext) {
        let (_host, window) = cx.add_window_view(|_, cx| {
            SearchHost(cx.new(|cx| {
                let mut view = FilePreview::new(cx);
                view.set_content_only(true, cx);
                view.set_expanded(true, cx);
                view.set_file(
                    "/project/main.rs".into(),
                    None,
                    "rust".into(),
                    "fn main() {}".into(),
                    cx,
                );
                view
            }))
        });
        assert!(window.debug_bounds("file-preview-search-capsule").is_none());
        let trigger = window.debug_bounds("file-preview-search-trigger").unwrap();
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        assert!(window.debug_bounds("file-preview-search-capsule").is_some());
        window.simulate_keystrokes("escape");
        assert!(window.debug_bounds("file-preview-search-capsule").is_none());
    }

    #[cfg(all(feature = "webview", any(target_os = "macos", target_os = "windows")))]
    #[gpui::test]
    fn hosted_document_omits_header_and_sidebar_keeps_it(cx: &mut gpui::TestAppContext) {
        let (view, window) = cx.add_window_view(|_, cx| FilePreview::new(cx));
        assert!(window.debug_bounds("file-preview-header").is_some());
        view.update(window, |view, cx| {
            view.set_standalone(true, cx);
            view.set_content_only(true, cx);
        });
        assert!(window.debug_bounds("file-preview-header").is_none());
        view.update(window, |view, cx| view.set_content_only(false, cx));
        assert!(window.debug_bounds("file-preview-header").is_some());
    }

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

    #[test]
    fn split_breadcrumbs_handles_various_paths() {
        let segs = split_breadcrumbs("/Users/ygy/Workspace/gpui-glint/src/components/icon.rs", true);
        assert_eq!(segs.len(), 6);
        assert_eq!(segs[0].name, "ygy");
        assert_eq!(segs[0].path, "/Users/ygy");
        assert_eq!(segs[0].dir_path, "/Users/ygy");
        assert!(!segs[0].is_file);

        assert_eq!(segs[1].name, "Workspace");
        assert_eq!(segs[2].name, "gpui-glint");
        assert_eq!(segs[3].name, "src");
        assert_eq!(segs[4].name, "components");
        assert_eq!(segs[4].dir_path, "/Users/ygy/Workspace/gpui-glint/src/components");
        assert!(!segs[4].is_file);

        assert_eq!(segs[5].name, "icon.rs");
        assert_eq!(segs[5].path, "/Users/ygy/Workspace/gpui-glint/src/components/icon.rs");
        assert_eq!(segs[5].dir_path, "/Users/ygy/Workspace/gpui-glint/src/components");
        assert!(segs[5].is_file);

        // Directory leaf
        let dir_segs = split_breadcrumbs("/Users/ygy/Workspace/gpui-glint/src/components", false);
        assert_eq!(dir_segs.len(), 5);
        assert_eq!(dir_segs.last().unwrap().name, "components");
        assert!(!dir_segs.last().unwrap().is_file);

        // Linux home
        let linux_segs = split_breadcrumbs("/home/ubuntu/project/main.py", true);
        assert_eq!(linux_segs[0].name, "ubuntu");
        assert_eq!(linux_segs.last().unwrap().name, "main.py");
        assert!(linux_segs.last().unwrap().is_file);

        // Relative path
        let rel_segs = split_breadcrumbs("crates/glint-agent/src/lib.rs", true);
        assert_eq!(rel_segs.len(), 4);
        assert_eq!(rel_segs[0].name, "crates");
        assert_eq!(rel_segs.last().unwrap().name, "lib.rs");

        // SSH URI
        let ssh_segs = split_breadcrumbs("ssh://user@host/home/user/app/index.js", true);
        assert_eq!(ssh_segs[0].name, "user");
        assert_eq!(ssh_segs.last().unwrap().name, "index.js");
        assert_eq!(ssh_segs.last().unwrap().dir_path, "ssh://user@host/home/user/app");
    }

    #[gpui::test]
    fn breadcrumbs_sidebar_navigation_and_popover_entry_click(cx: &mut gpui::TestAppContext) {
        let (view, window) = cx.add_window_view(|_, cx| {
            let mut preview = FilePreview::new(cx);
            preview.set_file(
                "/Users/ygy/Workspace/gpui-glint/src/components/icon.rs".into(),
                Some("/Users/ygy/Workspace/gpui-glint/src/components".into()),
                "rust".into(),
                "pub struct Icon;".into(),
                cx,
            );
            // Preload folder entries for components
            let entries = vec![
                FilePreviewEntry {
                    name: "code_card".into(),
                    path: "/Users/ygy/Workspace/gpui-glint/src/components/code_card".into(),
                    directory: true,
                    previewable: true,
                },
                FilePreviewEntry {
                    name: "code_font.rs".into(),
                    path: "/Users/ygy/Workspace/gpui-glint/src/components/code_font.rs".into(),
                    directory: false,
                    previewable: true,
                },
                FilePreviewEntry {
                    name: "icon.rs".into(),
                    path: "/Users/ygy/Workspace/gpui-glint/src/components/icon.rs".into(),
                    directory: false,
                    previewable: true,
                },
            ];
            preview.folders.insert(
                "/Users/ygy/Workspace/gpui-glint/src/components".into(),
                FolderNode {
                    expanded: false,
                    entries: Some(entries),
                    loading: None,
                    error: None,
                },
            );
            preview.folders.insert(
                "/Users/ygy/Workspace/gpui-glint/src/components/code_card".into(),
                FolderNode {
                    expanded: false,
                    entries: Some(vec![
                        FilePreviewEntry {
                            name: "source_syntax.rs".into(),
                            path: "/Users/ygy/Workspace/gpui-glint/src/components/code_card/source_syntax.rs".into(),
                            directory: false,
                            previewable: true,
                        },
                    ]),
                    loading: None,
                    error: None,
                },
            );
            preview
        });
        window.simulate_resize(gpui::size(px(600.), px(400.)));

        let opened = Rc::new(RefCell::new(Vec::new()));
        let received = opened.clone();
        let _observer = window.new(|cx: &mut Context<()>| {
            cx.subscribe(&view, move |_, _, event: &FilePreviewEvent, _| {
                if let FilePreviewEvent::OpenRequested(path) = event {
                    received.borrow_mut().push(path.clone());
                }
            })
            .detach();
        });

        window.run_until_parked();

        // Check breadcrumb segments are rendered
        assert!(window.debug_bounds("breadcrumb-seg:components").is_some());
        assert!(window.debug_bounds("breadcrumb-seg:icon.rs").is_some());

        // Open breadcrumb popover for "components"
        view.update(window, |preview, cx| {
            preview.set_open_breadcrumb(Some(4), cx); // segment 4 is "components"
        });
        window.run_until_parked();

        // Check header and entries inside popover dropdown are rendered
        assert!(window.debug_bounds("breadcrumb-dropdown-header").is_some());
        assert!(window.debug_bounds("breadcrumb-entry:/Users/ygy/Workspace/gpui-glint/src/components/code_card").is_some());
        // source_syntax.rs should not be visible yet because code_card is collapsed
        assert!(window.debug_bounds("breadcrumb-entry:/Users/ygy/Workspace/gpui-glint/src/components/code_card/source_syntax.rs").is_none());

        // Click code_card directory to expand it
        let dir_bounds = window.debug_bounds("breadcrumb-entry:/Users/ygy/Workspace/gpui-glint/src/components/code_card").unwrap();
        window.simulate_click(dir_bounds.center(), Default::default());
        window.run_until_parked();

        // Now source_syntax.rs should be visible inside the tree! Popover must stay open!
        assert_eq!(view.read_with(window, |p, _| p.open_breadcrumb()), Some(4));
        assert!(window.debug_bounds("breadcrumb-entry:/Users/ygy/Workspace/gpui-glint/src/components/code_card/source_syntax.rs").is_some());

        // Click nested entry "source_syntax.rs"
        let child_bounds = window.debug_bounds("breadcrumb-entry:/Users/ygy/Workspace/gpui-glint/src/components/code_card/source_syntax.rs").unwrap();
        window.simulate_click(child_bounds.center(), Default::default());
        window.run_until_parked();

        // Verify OpenRequested was emitted for the nested child and popover closed
        assert_eq!(*opened.borrow(), vec!["/Users/ygy/Workspace/gpui-glint/src/components/code_card/source_syntax.rs".to_owned()]);
        assert_eq!(view.read_with(window, |p, _| p.open_breadcrumb()), None);
    }

    #[test]
    fn calculate_visible_breadcrumb_start_prioritizes_leaf_and_hides_root() {
        let segs = split_breadcrumbs(
            "/Users/ygy/Workspace/gpui-glint/crates/glint-agent/src/edits_ui.rs",
            true,
        );
        assert_eq!(segs.len(), 7);
        assert_eq!(segs.last().unwrap().name, "edits_ui.rs");

        // When ample width (e.g. 1000px): all 7 items shown from index 0
        let start_wide = calculate_visible_breadcrumb_start(&segs, 1000.0);
        assert_eq!(start_wide, 0);

        // When narrow sidebar width (e.g. 240px, like in sidebar): root paths hidden, leaf edits_ui.rs visible
        let start_narrow = calculate_visible_breadcrumb_start(&segs, 240.0);
        assert!(start_narrow > 0);
        assert!(start_narrow < segs.len());
        // Leaf segment (index 6) is always included!
        assert!(segs[start_narrow..].iter().any(|s| s.name == "edits_ui.rs"));
        // Root segments (like "ygy", "Workspace") are hidden!
        assert!(!segs[start_narrow..].iter().any(|s| s.name == "ygy"));

        // When very tight (e.g. 80px): only leaf item shown
        let start_tiny = calculate_visible_breadcrumb_start(&segs, 80.0);
        assert_eq!(start_tiny, segs.len() - 1);
        assert_eq!(segs[start_tiny].name, "edits_ui.rs");
    }

    #[gpui::test]
    fn breadcrumbs_narrow_width_hides_root_paths_and_shows_leaf_with_ellipsis_popover(
        cx: &mut gpui::TestAppContext,
    ) {
        let (view, window) = cx.add_window_view(|_, cx| {
            let mut preview = FilePreview::new(cx);
            preview.set_file(
                "/Users/ygy/Workspace/gpui-glint/crates/glint-agent/src/edits_ui.rs".into(),
                Some("/Users/ygy/Workspace/gpui-glint/crates/glint-agent/src".into()),
                "rust".into(),
                "pub struct State;".into(),
                cx,
            );
            // Simulate narrow width of 240px
            preview.breadcrumb_width = Some(240.0);
            preview
        });
        window.simulate_resize(gpui::size(px(340.), px(400.)));

        let opened = Rc::new(RefCell::new(Vec::new()));
        let received = opened.clone();
        let _observer = window.new(|cx: &mut Context<()>| {
            cx.subscribe(&view, move |_, _, event: &FilePreviewEvent, _| {
                if let FilePreviewEvent::OpenRequested(path) = event {
                    received.borrow_mut().push(path.clone());
                }
            })
            .detach();
        });

        window.run_until_parked();

        // Leaf file edits_ui.rs MUST be visible
        assert!(window.debug_bounds("breadcrumb-seg:edits_ui.rs").is_some());
        // Ellipsis button MUST be visible
        assert!(window.debug_bounds("breadcrumb-seg:ellipsis").is_some());
        // Root item "ygy" MUST be hidden from the main bar
        assert!(window.debug_bounds("breadcrumb-seg:ygy").is_none());

        // Open ellipsis popover
        view.update(window, |preview, cx| {
            preview.set_open_breadcrumb(Some(ELLIPSIS_BREADCRUMB_INDEX), cx);
        });
        window.run_until_parked();

        // Hidden root item "ygy" should now be in the ellipsis dropdown
        assert!(window
            .debug_bounds("breadcrumb-ellipsis-item:ygy")
            .is_some());

        // Click "ygy" in the ellipsis dropdown
        let ygy_bounds = window
            .debug_bounds("breadcrumb-ellipsis-item:ygy")
            .unwrap();
        window.simulate_click(ygy_bounds.center(), Default::default());
        window.run_until_parked();

        // Verify OpenRequested was emitted for /Users/ygy and popover closed
        assert_eq!(*opened.borrow(), vec!["/Users/ygy".to_owned()]);
        assert_eq!(view.read_with(window, |p, _| p.open_breadcrumb()), None);
    }
}
