mod app_icon;
mod storage;

use gpui::{
    App, AppContext, Bounds, Context, Entity, FontWeight, PathPromptOptions, Subscription,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, point, prelude::*, px, rgb, rgba,
    size,
};
use gpui_glint::{
    components::{
        AppLayout, Button, ButtonKind, ButtonSize, CodeCard, CodeCardEvent, DocumentEditor,
        DocumentEvent, DropdownGroup, DropdownItem, GlintAssets, GroupedDropdown, Icon, IconButton,
        SearchInput, SegmentOption, SegmentedControl, TextEntry, ThemeSelector, ThemeSelectorEvent,
        Toast, ToastHost, ToastKind, Tooltip, bind_document_editor_keys, bind_text_entry_keys,
    },
    theme::ThemePreset,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

gpui::actions!(
    markdown_workspace,
    [
        NewDocument,
        OpenDocument,
        SaveDocument,
        CloseDocument,
        QuitWorkspace
    ]
);

struct OpenDoc {
    title_rename: Option<gpui::Task<()>>,
    id: u64,
    path: PathBuf,
    markdown: String,
    saved: String,
    editor: Entity<DocumentEditor>,
    source: Entity<CodeCard>,
    source_mode: bool,
    _subscriptions: Vec<Subscription>,
}

struct MarkdownWorkspace {
    focus: gpui::FocusHandle,
    more_menu: Entity<GroupedDropdown>,
    root: PathBuf,
    recent: Vec<PathBuf>,
    sidebar_filter: usize,
    documents: Vec<OpenDoc>,
    active: Option<u64>,
    next_id: u64,
    theme: ThemePreset,
    sidebar_visible: bool,
    theme_selector: Entity<ThemeSelector>,
    search: Entity<TextEntry>,
    notice: String,
    toasts: Entity<ToastHost>,
    _search_subscription: Subscription,
    _theme_subscription: Subscription,
}

impl MarkdownWorkspace {
    fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| TextEntry::new(cx, "搜索文档内容或标题", ""));
        let subscription = cx.observe(&search, |this, search, cx| {
            let query = search.read(cx).value().trim().to_owned();
            for doc in &this.documents {
                doc.editor
                    .update(cx, |editor, cx| editor.set_search_query(query.clone(), cx));
                doc.source
                    .update(cx, |source, cx| source.set_search_query(query.clone(), cx));
            }
            cx.notify();
        });
        let owner = cx.weak_entity();
        let more_menu = cx.new(|_| {
            GroupedDropdown::new("更多")
                .trigger_icon(Icon::More)
                .align_right()
                .show_selection(false)
                .on_select(move |label, window, cx| {
                    let _ = owner.update(cx, |this, cx| this.menu_action(&label, window, cx));
                })
        });
        let toasts = cx.new(|_| ToastHost::new());
        let theme_selector = cx.new(|cx| ThemeSelector::new(cx).width(220.));
        let theme_subscription = cx.subscribe(
            &theme_selector,
            |this, _, event: &ThemeSelectorEvent, cx| {
                let ThemeSelectorEvent::Changed(theme) = event;
                this.set_theme(*theme, cx);
            },
        );
        let mut this = Self {
            toasts,
            focus: cx.focus_handle(),
            more_menu,
            recent: Vec::new(),
            sidebar_filter: 0,
            documents: Vec::new(),
            active: None,
            next_id: 1,
            theme: ThemePreset::Sage,
            sidebar_visible: true,
            theme_selector,
            root,
            search,
            notice: "改动自动保存到本机".into(),
            _search_subscription: subscription,
            _theme_subscription: theme_subscription,
        };
        let session = fs::read_to_string(this.root.join("session.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
        if let Some(recent) = session
            .as_ref()
            .and_then(|session| session["recent"].as_array())
        {
            this.recent = recent
                .iter()
                .filter_map(|value| value.as_str())
                .map(PathBuf::from)
                .filter(|path| path.is_file())
                .collect();
        }
        // First launch (or a session from before the recent list existed):
        // seed the history with the default workspace documents.
        if this.recent.is_empty()
            && let Ok(mut files) = storage::list(&this.root)
        {
            files.sort();
            this.recent = files;
        }
        let restore_session = session.is_some();
        if let Some(session) = session {
            this.sidebar_visible = session["sidebar_visible"].as_bool().unwrap_or(true);
            this.sidebar_filter = match session["sidebar_filter"].as_u64() {
                Some(1) => 1,
                Some(2) => 2,
                _ => 0,
            };
            if let Some(theme) = session["theme"].as_str().and_then(ThemePreset::from_id) {
                this.theme = theme;
            }
            if let Some(tabs) = session["tabs"].as_array() {
                for tab in tabs.iter().filter_map(|v| v.as_str()) {
                    if let Some(path) = this
                        .recent
                        .iter()
                        .find(|p| p.file_name().is_some_and(|n| n == tab))
                        .cloned()
                    {
                        this.open_path(path, cx);
                    }
                }
            }
            if let Some(active) = session["active"].as_str() {
                this.active = this
                    .documents
                    .iter()
                    .find(|d| d.path.file_name().is_some_and(|n| n == active))
                    .map(|d| d.id)
                    .or(this.active);
            }
        }
        if !restore_session
            && this.documents.is_empty()
            && let Some(path) = this.recent.first().cloned()
        {
            this.open_path(path, cx);
        }
        this.theme_selector.update(cx, |selector, cx| {
            selector.set_selected(this.theme, cx);
            selector.set_theme(this.theme, cx);
        });
        this.search
            .update(cx, |entry, cx| entry.set_theme(this.theme, cx));
        this.more_menu
            .update(cx, |menu, cx| menu.set_theme(this.theme, cx));
        this
    }

    /// Moves a path to the front of the recent history (deduplicated).
    fn touch_recent(&mut self, path: PathBuf) {
        self.recent.retain(|recent| recent != &path);
        self.recent.insert(0, path);
        self.recent.truncate(50);
    }

    fn persist_session(&mut self) {
        let session = serde_json::json!({
            "theme": self.theme.id(),
            "sidebar_visible": self.sidebar_visible,
            "recent": self.recent.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>(),
            "sidebar_filter": self.sidebar_filter,
            "tabs": self.documents.iter().map(|d| d.path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>(),
            "active": self.documents.iter().find(|d| Some(d.id) == self.active).map(|d| d.path.file_name().unwrap().to_string_lossy().into_owned()),
        });
        if let Err(error) =
            storage::atomic_write(&self.root.join("session.json"), &session.to_string())
        {
            self.notice = format!("保存工作区状态失败：{error}");
        }
    }

    fn open_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(doc) = self.documents.iter().find(|d| d.path == path) {
            self.active = Some(doc.id);
        } else {
            let markdown = match fs::read_to_string(&path) {
                Ok(source) => source,
                Err(error) => {
                    self.notice = format!("打开失败：{error}");
                    cx.notify();
                    return;
                }
            };
            let (saved, markdown) = match storage::read_recovery(&path) {
                Ok(Some((saved, source))) if source != markdown => {
                    self.notice = "已恢复未保存改动。可尝试保存，或导出到其他位置".into();
                    (saved, source)
                }
                Ok(_) => (markdown.clone(), markdown),
                Err(error) => {
                    self.notice = format!("读取恢复记录失败：{error}");
                    (markdown.clone(), markdown)
                }
            };
            let id = self.next_id;
            self.next_id += 1;
            let editor = cx.new(|cx| {
                DocumentEditor::new(cx, &markdown)
                    .theme(self.theme)
                    .rounded_corners(false)
                    .title(storage::title(&path), cx)
            });
            let source =
                cx.new(|cx| CodeCard::new(cx, "markdown", markdown.clone()).theme(self.theme));
            let query = self.search.read(cx).value().trim().to_owned();
            editor.update(cx, |editor, cx| editor.set_search_query(query.clone(), cx));
            source.update(cx, |source, cx| source.set_search_query(query, cx));
            let visual_subscription =
                cx.subscribe(&editor, move |this, _, event, cx| match event {
                    DocumentEvent::Changed(content) => this.changed(id, content.to_string(), cx),
                    DocumentEvent::TitleChanged(title) => {
                        let title = title.to_string();
                        let timer = cx
                            .background_executor()
                            .timer(std::time::Duration::from_millis(500));
                        let task = cx.spawn(async move |this, cx| {
                            timer.await;
                            let _ = this.update(cx, |this, cx| this.rename_title(id, &title, cx));
                        });
                        if let Some(doc) = this.documents.iter_mut().find(|doc| doc.id == id) {
                            doc.title_rename = Some(task);
                        }
                    }
                    DocumentEvent::Mermaid(event) => {
                        let toast = match event {
                            gpui_glint::components::MermaidEvent::SourceCopied(_) => {
                                Toast::new("已复制源码").kind(ToastKind::Success)
                            }
                            gpui_glint::components::MermaidEvent::PngExported(path) => {
                                Toast::new("已导出 PNG")
                                    .kind(ToastKind::Success)
                                    .description(path.display().to_string())
                            }
                            gpui_glint::components::MermaidEvent::ExportFailed(error) => {
                                Toast::new("导出失败")
                                    .kind(ToastKind::Error)
                                    .description(error.clone())
                            }
                        };
                        this.toasts.update(cx, |host, cx| {
                            host.push(toast, cx);
                        });
                    }
                    DocumentEvent::LinkActivated(url) => {
                        if url.starts_with("https://") || url.starts_with("http://") {
                            cx.open_url(url);
                        }
                    }
                });
            let source_subscription = cx.subscribe(&source, move |this, _, event, cx| {
                if let CodeCardEvent::Changed(content) = event {
                    this.changed(id, content.to_string(), cx);
                }
            });
            self.documents.push(OpenDoc {
                id,
                path: path.clone(),
                saved,
                markdown,
                editor,
                source,
                source_mode: false,
                _subscriptions: vec![visual_subscription, source_subscription],
                title_rename: None,
            });
            self.touch_recent(path);
            self.active = Some(id);
        }
        self.persist_session();
        cx.notify();
    }

    /// Opens a markdown file from anywhere on disk (the CLI path argument).
    /// Files outside the library stay outside it: they open in place and are
    /// not listed in the sidebar.
    fn open_external(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // Keep the user's spelling of the path for display; resolve only to
        // verify the file exists.
        let exists = path
            .canonicalize()
            .map(|resolved| resolved.is_file())
            .unwrap_or(false);
        if !exists {
            self.notice = format!("无法打开 {}：文件不存在", path.display());
            cx.notify();
            return;
        }
        self.open_path(path, cx);
    }

    fn changed(&mut self, id: u64, content: String, cx: &mut Context<Self>) {
        if let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) {
            doc.markdown = content;
        }
        self.save(id);
        cx.notify();
    }

    fn save(&mut self, id: u64) -> bool {
        let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) else {
            return true;
        };
        if doc.markdown == doc.saved {
            storage::clear_recovery(&doc.path);
            return true;
        }
        if let Err(error) = storage::write_recovery(&doc.path, &doc.saved, &doc.markdown) {
            self.notice = format!("恢复记录写入失败，改动仍在当前标签中：{error}");
            return false;
        }
        match storage::save(&doc.path, &doc.saved, &doc.markdown) {
            Ok(()) => {
                doc.saved = doc.markdown.clone();
                storage::clear_recovery(&doc.path);
                self.notice = "已保存到本机".into();
                true
            }
            Err(error) => {
                self.notice = format!("保存失败，改动仍在当前标签中：{error}");
                false
            }
        }
    }

    fn create_document(&mut self, cx: &mut Context<Self>) {
        match storage::create(&self.root, "未命名文档", "# 未命名文档\n\n") {
            Ok(path) => {
                self.open_path(path, cx);
            }
            Err(error) => {
                self.notice = format!("新建失败：{error}");
                cx.notify();
            }
        }
    }

    fn close_document(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.save(id) {
            cx.notify();
            return;
        }
        if let Some(index) = self.documents.iter().position(|d| d.id == id) {
            self.documents.remove(index);
            if self.active == Some(id) {
                self.active = self
                    .documents
                    .get(index.min(self.documents.len().saturating_sub(1)))
                    .map(|d| d.id);
            }
        }
        self.persist_session();
        cx.notify();
    }

    fn rename_title(&mut self, id: u64, name: &str, cx: &mut Context<Self>) {
        if !self.save(id) {
            cx.notify();
            return;
        }
        if let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) {
            match storage::rename(&doc.path, name) {
                Ok(path) => {
                    if let Some(recent) = self.recent.iter_mut().find(|recent| **recent == doc.path)
                    {
                        *recent = path.clone();
                    }
                    doc.path = path;
                    doc.editor.update(cx, |editor, cx| {
                        editor.set_title(storage::title(&doc.path), cx)
                    });
                    self.notice = "文档已重命名".into();
                }
                Err(error) => {
                    self.notice = format!("重命名失败：{error}");
                    doc.editor.update(cx, |editor, cx| {
                        editor.set_title(storage::title(&doc.path), cx)
                    });
                    self.toasts.update(cx, |host, cx| {
                        host.push(
                            Toast::new("重命名失败")
                                .kind(ToastKind::Error)
                                .description(error.to_string()),
                            cx,
                        );
                    });
                }
            }
        }
        self.persist_session();
        cx.notify();
    }

    fn import_documents(&mut self, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("导入 Markdown 文档（保留原文件）".into()),
        });
        cx.spawn(async move |this, cx| {
            let result = picker.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        for path in paths {
                            if !path.extension().is_some_and(|ext| {
                                ext.eq_ignore_ascii_case("md")
                                    || ext.eq_ignore_ascii_case("markdown")
                            }) {
                                this.notice = "请选择 .md 或 .markdown 格式的文档".into();
                                continue;
                            }
                            let result = fs::read_to_string(&path).and_then(|source| {
                                storage::create(&this.root, &storage::title(&path), &source)
                            });
                            match result {
                                Ok(copy) => {
                                    this.open_path(copy, cx);
                                    this.notice = "已导入副本，原文件保留".into();
                                }
                                Err(error) => {
                                    this.notice = format!("导入 {} 失败：{error}", path.display())
                                }
                            }
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => this.notice = format!("打开文件选择器失败：{error}"),
                    Err(error) => this.notice = format!("文件选择器异常：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn export_document(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active) else {
            return;
        };
        let name = doc.path.file_name().unwrap().to_string_lossy().into_owned();
        let content = doc.markdown.clone();
        let picker = cx.prompt_for_new_path(&self.root, Some(&name));
        cx.spawn(async move |this, cx| {
            let result = picker.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Ok(Some(path))) => {
                        if this.documents.iter().any(|d| d.path == path) {
                            this.notice = "请选择文档库以外的位置或其他文件名导出".into();
                        } else {
                            this.notice = match storage::atomic_write(&path, &content) {
                                Ok(()) => format!("已导出到 {}", path.display()),
                                Err(error) => format!("导出失败：{error}"),
                            };
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => this.notice = format!("导出失败：{error}"),
                    Err(error) => this.notice = format!("文件选择器异常：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_source_mode(&mut self, source_mode: bool, cx: &mut Context<Self>) {
        if let Some(doc) = self
            .documents
            .iter_mut()
            .find(|d| Some(d.id) == self.active)
        {
            if doc.source_mode == source_mode {
                return;
            }
            if doc.source_mode {
                if doc.editor.read(cx).markdown() != doc.markdown {
                    doc.editor
                        .update(cx, |editor, cx| editor.set_markdown(&doc.markdown, cx));
                }
            } else if doc.source.read(cx).content() != doc.markdown {
                doc.source.update(cx, |source, cx| {
                    source.sync_content(doc.markdown.clone(), cx)
                });
            }
            doc.source_mode = source_mode;
        }
        cx.notify();
    }

    fn menu_action(&mut self, action: &str, _window: &mut Window, cx: &mut Context<Self>) {
        match action {
            "新建文档" => self.create_document(cx),
            "打开 Markdown…" => self.import_documents(cx),
            "打开文档库" => cx.reveal_path(&self.root),
            "导出 Markdown…" => self.export_document(cx),
            "关闭文档" => {
                if let Some(id) = self.active {
                    self.close_document(id, cx);
                }
            }
            "保存" => {
                if let Some(id) = self.active {
                    self.save(id);
                }
            }
            "复制 Markdown" => {
                if let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(doc.markdown.clone()));
                    self.notice = "已复制 Markdown".into();
                }
            }
            _ => {
                if let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active) {
                    doc.editor.update(cx, |editor, cx| match action {
                        "撤销" => editor.undo(cx),
                        "重做" => editor.redo(cx),
                        "完成编辑" => editor.finish_editing(cx),
                        "预览" | "返回编辑" => editor.set_preview(!editor.is_preview(), cx),
                        _ => {}
                    });
                }
            }
        }
        cx.notify();
    }

    fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        self.theme_selector.update(cx, |selector, cx| {
            selector.set_selected(theme, cx);
            selector.set_theme(theme, cx);
        });
        self.search
            .update(cx, |entry, cx| entry.set_theme(theme, cx));
        self.more_menu
            .update(cx, |menu, cx| menu.set_theme(theme, cx));
        for doc in &self.documents {
            doc.editor
                .update(cx, |editor, cx| editor.set_theme(self.theme, cx));
            doc.source
                .update(cx, |source, cx| source.set_theme(theme, cx));
        }
        self.persist_session();
        cx.notify();
    }
}

impl Render for MarkdownWorkspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.toasts
            .update(cx, |host, cx| host.set_theme(self.theme, cx));
        let p = self.theme.palette();
        let active_doc = self.documents.iter().find(|d| Some(d.id) == self.active);
        let title = active_doc
            .map(|d| storage::title(&d.path))
            .unwrap_or_else(|| "文档工作区".into());
        window.set_window_title(&format!("{title} — Glint Markdown"));
        let source_mode = active_doc.is_some_and(|d| d.source_mode);
        let preview = active_doc.is_some_and(|d| d.editor.read(cx).is_preview());
        let has_document = active_doc.is_some();
        let mut groups = vec![
            DropdownGroup::new("工作区")
                .item(DropdownItem::new("新建文档").icon(Icon::Plus))
                .item(DropdownItem::new("打开 Markdown…").icon(Icon::Upload))
                .item(DropdownItem::new("打开文档库").icon(Icon::Folders)),
        ];
        if has_document {
            groups.push(
                DropdownGroup::new("当前文档")
                    .item(DropdownItem::new("保存").icon(Icon::CheckCircle))
                    .item(DropdownItem::new("复制 Markdown").icon(Icon::Copy))
                    .item(DropdownItem::new("导出 Markdown…").icon(Icon::Download))
                    .item(DropdownItem::new("关闭文档").icon(Icon::Close)),
            );
            if !source_mode {
                groups.push(
                    DropdownGroup::new("编辑")
                        .item(DropdownItem::new("撤销").icon(Icon::ArrowLeft))
                        .item(DropdownItem::new("重做").icon(Icon::ChevronRight))
                        .item(DropdownItem::new("完成编辑").icon(Icon::CheckCircle))
                        .item(
                            DropdownItem::new(if preview { "返回编辑" } else { "预览" })
                                .icon(Icon::Eye),
                        ),
                );
            }
        }
        self.more_menu
            .update(cx, |menu, cx| menu.set_groups(groups, cx));
        let workspace = cx.entity();
        let topbar = div()
            .size_full()
            .pl(px(if cfg!(target_os = "macos") { 84. } else { 16. }))
            .pr_4()
            .flex()
            .items_center()
            .gap_3()
            .text_size(px(12.))
            .text_color(rgb(p.text_muted))
            .child(div().size(px(8.)).rounded(px(3.)).bg(rgb(p.accent)))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(p.text))
                    .child("GLINT MARKDOWN"),
            )
            .child(
                Tooltip::new(
                    div()
                        .id("markdown-toggle-sidebar")
                        .size(px(28.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(7.))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(p.surface_subtle)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sidebar_visible = !this.sidebar_visible;
                            this.persist_session();
                            cx.notify();
                        }))
                        .child(Icon::Sidebar.render(18.).text_color(rgb(p.text_muted))),
                    if self.sidebar_visible {
                        "收起文档列表"
                    } else {
                        "展开文档列表"
                    },
                )
                .render(cx),
            )
            .child(
                if active_doc.is_some_and(|doc| !doc.path.starts_with(&self.root)) {
                    div()
                        .min_w_0()
                        .max_w(px(420.))
                        .truncate()
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child(
                            active_doc
                                .map(|doc| display_path(&doc.path))
                                .unwrap_or_default(),
                        )
                } else {
                    div().child("本机工作区")
                },
            )
            .child(gpui_glint::components::window_drag_region())
            .child(self.theme_selector.clone())
            .when(has_document, |bar| {
                bar.child(
                    SegmentedControl::new([SegmentOption::new("排版"), SegmentOption::new("源码")])
                        .id("markdown-editor-mode")
                        .theme(self.theme)
                        .selected(usize::from(source_mode))
                        .on_change(move |index, _, cx| {
                            workspace.update(cx, |this, cx| this.set_source_mode(index == 1, cx));
                        }),
                )
            })
            .child(self.more_menu.clone());

        let query = self.search.read(cx).value().trim().to_lowercase();
        let dark = self.theme.is_dark();
        let mut list = div()
            .id("markdown-library-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_1();
        let mut matches = 0usize;
        let visible_paths: Vec<&PathBuf> = self
            .recent
            .iter()
            .filter(|path| match self.sidebar_filter {
                1 => path.starts_with(&self.root),
                2 => !path.starts_with(&self.root),
                _ => true,
            })
            .collect();
        for path in visible_paths {
            let name = storage::title(path);
            let open_doc = self.documents.iter().find(|d| &d.path == path);
            let content_matches = !query.is_empty()
                && open_doc
                    .map(|d| d.markdown.to_lowercase().contains(&query))
                    .unwrap_or_else(|| {
                        fs::read_to_string(path).is_ok_and(|s| s.to_lowercase().contains(&query))
                    });
            if !query.is_empty() && !name.to_lowercase().contains(&query) && !content_matches {
                continue;
            }
            matches += 1;
            let selected = open_doc.is_some_and(|d| Some(d.id) == self.active);
            let path = path.clone();
            list = list.child(
                div()
                    .id(("library-document", matches))
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .cursor_pointer()
                    .bg(if dark {
                        rgba(if selected { 0xffffff26 } else { 0x00000000 })
                    } else {
                        rgb(if selected { p.surface } else { p.panel })
                    })
                    .text_color(rgb(if selected { p.text } else { p.text_muted }))
                    .hover(move |s| {
                        s.bg(if dark {
                            rgba(0xffffff14)
                        } else {
                            rgb(p.surface_subtle)
                        })
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.open_path(path.clone(), cx)))
                    .child(Icon::Text.render(14.))
                    .child(div().min_w_0().text_ellipsis().child(name)),
            );
        }
        if matches == 0 {
            list = list.child(
                div()
                    .p_3()
                    .text_size(px(12.))
                    .text_color(rgb(p.text_muted))
                    .child("没有匹配的文档"),
            );
        }
        let sidebar = div()
            .size_full()
            .p_3()
            .flex()
            .flex_col()
            .gap_3()
            .bg(if dark {
                rgba(self.theme.panel_material())
            } else {
                rgb(p.panel)
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("文档")
                    .child(
                        Tooltip::new(
                            IconButton::new(Icon::Plus, "新建文档")
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(|this, _, _, cx| this.create_document(cx))),
                            "新建文档 · ⌘/Ctrl N",
                        )
                        .render(cx),
                    ),
            )
            .child(
                SearchInput::new("搜索文档")
                    .theme(self.theme)
                    .width(218.)
                    .control(self.search.clone())
                    .render(),
            )
            .child({
                let visible = self
                    .recent
                    .iter()
                    .filter(|path| match self.sidebar_filter {
                        1 => path.starts_with(&self.root),
                        2 => !path.starts_with(&self.root),
                        _ => true,
                    })
                    .count();
                let workspace_entity = cx.entity();
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(p.text_muted))
                            .child(format!("最近 · {visible}")),
                    )
                    .child(
                        SegmentedControl::new([
                            SegmentOption::new("全部"),
                            SegmentOption::new("工作区"),
                            SegmentOption::new("外部"),
                        ])
                        .id("markdown-sidebar-filter")
                        .theme(self.theme)
                        .selected(self.sidebar_filter)
                        .on_change(move |index, _, cx| {
                            workspace_entity.update(cx, |this, cx| {
                                this.sidebar_filter = index;
                                this.persist_session();
                                cx.notify();
                            });
                        })
                        .render(),
                    )
            })
            .child(list)
            .child(
                Button::new("打开 Markdown…")
                    .kind(ButtonKind::Glass)
                    .size(ButtonSize::Small)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| this.import_documents(cx))),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("导入副本 · 自动保存 · 无需联网"),
            );

        let mut content = div().size_full().min_w_0();
        if let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active) {
            if doc.source_mode {
                content = content.child(
                    div()
                        .id("markdown-source-scroll")
                        .size_full()
                        .overflow_y_scroll()
                        .child(doc.source.clone()),
                );
            } else {
                let status_message = self.notice.clone();
                doc.editor.update(cx, |editor, cx| {
                    editor.set_status_message(Some(status_message), cx)
                });
                content = content.child(
                    div()
                        .size_full()
                        .border_1()
                        .border_color(rgb(p.border))
                        .overflow_hidden()
                        .child(doc.editor.clone()),
                );
            }
        } else {
            content = content.child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .text_color(rgb(p.text_muted))
                    .child(Icon::Text.render(32.))
                    .child(
                        div()
                            .text_size(px(24.))
                            .text_color(rgb(p.text))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("下一篇，从这里开始"),
                    )
                    .child("新建一篇文档，或从左侧打开已有 Markdown。")
                    .child(
                        Button::new("新建文档")
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.create_document(cx))),
                    ),
            );
        }
        div()
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .key_context("MarkdownWorkspace")
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.more_menu.update(cx, |menu, cx| {
                        menu.close();
                        cx.notify();
                    });
                }),
            )
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.more_menu.update(cx, |menu, cx| {
                        menu.close();
                        cx.notify();
                    });
                }
            }))
            .on_action(cx.listener(|this, _: &NewDocument, _, cx| this.create_document(cx)))
            .on_action(cx.listener(|this, _: &OpenDocument, _, cx| this.import_documents(cx)))
            .on_action(cx.listener(|this, _: &SaveDocument, _, cx| {
                if let Some(id) = this.active {
                    this.save(id);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CloseDocument, _, cx| {
                if let Some(id) = this.active {
                    this.close_document(id, cx);
                }
            }))
            .child(
                AppLayout::new(topbar, sidebar, content)
                    .sidebar_width(244.)
                    .sidebar_visible(self.sidebar_visible)
                    .theme(self.theme),
            )
            .child(self.toasts.clone())
    }
}

/// `~/...` abbreviation for paths under the home directory.
fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy().into_owned();
    match std::env::var_os("HOME") {
        Some(home) => text
            .strip_prefix(home.to_string_lossy().as_ref())
            .map(|rest| format!("~{rest}"))
            .unwrap_or(text),
        None => text,
    }
}

fn main() {
    let cli_path = std::env::args().nth(1);
    if cli_path
        .as_deref()
        .is_some_and(|arg| arg == "-h" || arg == "--help")
    {
        println!("用法：mdeditor [Markdown 文件路径]");
        println!("  不带参数时打开本机文档库；带路径时直接打开该文件预览。");
        return;
    }
    let root = storage::library_path().expect("无法确定文档库位置");
    let new_library = !root.exists();
    fs::create_dir_all(&root).expect("无法创建文档库");
    if new_library {
        storage::create(&root, "示例文档", include_str!("welcome.md")).expect("无法创建欢迎文档");
    }
    gpui_platform::application()
        .with_assets(GlintAssets)
        .run(move |cx: &mut App| {
            app_icon::install();
            bind_text_entry_keys(cx);
            bind_document_editor_keys(cx);
            cx.set_menus(vec![
                gpui::Menu {
                    disabled: false,
                    name: "Glint Markdown".into(),
                    items: vec![gpui::MenuItem::action("退出 Glint Markdown", QuitWorkspace)],
                },
                gpui::Menu {
                    disabled: false,
                    name: "文件".into(),
                    items: vec![
                        gpui::MenuItem::action("新建文档", NewDocument),
                        gpui::MenuItem::action("打开 Markdown…", OpenDocument),
                        gpui::MenuItem::action("保存", SaveDocument),
                        gpui::MenuItem::action("关闭文档", CloseDocument),
                    ],
                },
            ]);
            cx.bind_keys([
                gpui::KeyBinding::new("cmd-q", QuitWorkspace, None),
                gpui::KeyBinding::new("ctrl-q", QuitWorkspace, None),
            ]);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            for modifier in ["cmd", "ctrl"] {
                cx.bind_keys([
                    gpui::KeyBinding::new(
                        &format!("{modifier}-n"),
                        NewDocument,
                        Some("MarkdownWorkspace"),
                    ),
                    gpui::KeyBinding::new(
                        &format!("{modifier}-o"),
                        OpenDocument,
                        Some("MarkdownWorkspace"),
                    ),
                    gpui::KeyBinding::new(
                        &format!("{modifier}-s"),
                        SaveDocument,
                        Some("MarkdownWorkspace"),
                    ),
                    gpui::KeyBinding::new(
                        &format!("{modifier}-w"),
                        CloseDocument,
                        Some("MarkdownWorkspace"),
                    ),
                ]);
            }
            let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(16.), px(14.))),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                move |window, cx| {
                    let workspace = cx.new(|cx| MarkdownWorkspace::new(root, cx));
                    if let Some(path) = cli_path.clone() {
                        workspace
                            .update(cx, |workspace, cx| workspace.open_external(path.into(), cx));
                    }
                    let focus = workspace.read(cx).focus.clone();
                    window.focus(&focus, cx);
                    let quit_workspace = workspace.downgrade();
                    cx.on_action(move |_: &QuitWorkspace, cx| {
                        let saved = quit_workspace
                            .update(cx, |this, cx| {
                                let ids: Vec<_> = this.documents.iter().map(|doc| doc.id).collect();
                                let mut saved = true;
                                for id in ids {
                                    saved &= this.save(id);
                                }
                                this.persist_session();
                                cx.notify();
                                saved
                            })
                            .unwrap_or(true);
                        if saved {
                            cx.quit();
                        }
                    });
                    let weak = workspace.downgrade();
                    window.on_window_should_close(cx, move |_, cx| {
                        weak.update(cx, |this, cx| {
                            let ids: Vec<_> = this.documents.iter().map(|doc| doc.id).collect();
                            let mut saved = true;
                            for id in ids {
                                saved &= this.save(id);
                            }
                            this.persist_session();
                            cx.notify();
                            saved
                        })
                        .unwrap_or(true)
                    });
                    workspace
                },
            )
            .expect("无法打开编辑器窗口");
            cx.activate(true);
        });
}
