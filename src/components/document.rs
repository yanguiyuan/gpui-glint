use super::document_markdown::{self as md, Node};
pub use super::document_markdown::{DocumentBlock, DocumentBlockKind};
use super::inputs::checkbox_control_themed;
use super::{
    CodeCard, CodeCardEvent, DataTable, Icon, MermaidDiagram, MermaidEvent, TableCell, TableColumn,
    TableRow,
};
use crate::theme::ThemePreset;
use gpui::{
    AppContext, ClipboardItem, Context, Entity, EventEmitter, Focusable, FontStyle, FontWeight,
    HighlightStyle, InteractiveText, ScrollHandle, SharedString, StrikethroughStyle, StyledText,
    Subscription, UnderlineStyle, Window, deferred, div, img, prelude::*, px, rgb, rgba,
};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Parser, Tag};
gpui::actions!(glass_document, [DocumentUndo, DocumentRedo]);
/// Register once, in addition to (or instead of) bind_code_card_keys.
pub fn bind_document_editor_keys(cx: &mut gpui::App) {
    super::bind_code_card_keys(cx);
    super::bind_text_entry_keys(cx);
    cx.bind_keys([
        gpui::KeyBinding::new("cmd-z", DocumentUndo, Some("GlassDocumentEditor")),
        gpui::KeyBinding::new("cmd-shift-z", DocumentRedo, Some("GlassDocumentEditor")),
        gpui::KeyBinding::new("ctrl-z", DocumentUndo, Some("GlassDocumentEditor")),
        gpui::KeyBinding::new("ctrl-shift-z", DocumentRedo, Some("GlassDocumentEditor")),
    ]);
}
use super::rich_text;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    ops::Range,
    rc::Rc,
};

#[derive(Clone, Debug)]
pub enum DocumentEvent {
    Changed(SharedString),
    /// User edits to the document title, which is separate from Markdown and blocks.
    TitleChanged(SharedString),
    LinkActivated(SharedString),
    Mermaid(MermaidEvent),
}
#[derive(Clone, Copy)]
enum Command {
    Paragraph,
    H1,
    H2,
    H3,
    Quote,
    Bullet,
    Ordered,
    Task,
    Code,
    Mermaid,
    Table,
    Divider,
}
impl Command {
    const ALL: [Self; 12] = [
        Self::Paragraph,
        Self::H1,
        Self::H2,
        Self::H3,
        Self::Quote,
        Self::Bullet,
        Self::Ordered,
        Self::Task,
        Self::Code,
        Self::Mermaid,
        Self::Table,
        Self::Divider,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::Paragraph => "正文",
            Self::H1 => "一级标题",
            Self::H2 => "二级标题",
            Self::H3 => "三级标题",
            Self::Quote => "引用",
            Self::Bullet => "无序列表",
            Self::Ordered => "有序列表",
            Self::Task => "待办事项",
            Self::Code => "代码块",
            Self::Mermaid => "Mermaid 图表",
            Self::Table => "表格",
            Self::Divider => "分隔线",
        }
    }
    fn keyword(self) -> &'static str {
        match self {
            Self::Paragraph => "text paragraph",
            Self::H1 | Self::H2 | Self::H3 => "heading title",
            Self::Quote => "quote",
            Self::Bullet | Self::Ordered => "list",
            Self::Task => "task todo",
            Self::Code => "code",
            Self::Mermaid => "mermaid diagram flowchart 图表",
            Self::Table => "table",
            Self::Divider => "divider hr",
        }
    }
    fn sample(self) -> &'static str {
        match self {
            Self::Paragraph => "",
            Self::H1 => "# 标题",
            Self::H2 => "## 标题",
            Self::H3 => "### 标题",
            Self::Quote => "> 引用内容",
            Self::Bullet => "- 列表项",
            Self::Ordered => "1. 列表项",
            Self::Task => "- [ ] 待办事项",
            Self::Code => "```go\n// 在这里编写代码\n```",
            Self::Mermaid => "```mermaid\nflowchart LR\n    A[开始] --> B[结束]\n```",
            Self::Table => "| 名称 | 说明 |\n| --- | --- |\n| 项目 | 内容 |",
            Self::Divider => "---",
        }
    }
    fn icon(self) -> Icon {
        match self {
            Self::Code | Self::Mermaid => Icon::Code,
            Self::Table => Icon::Database,
            Self::Bullet | Self::Ordered | Self::Task => Icon::List,
            _ => Icon::Text,
        }
    }
}
struct BlockView {
    mode: (bool, bool),
    source: Entity<CodeCard>,
    table: Option<Entity<DataTable>>,
    mermaid: Option<Entity<MermaidDiagram>>,
    inline: Vec<Entity<CodeCard>>,
    cell_editors: Vec<Entity<CodeCard>>,
    _subscriptions: Vec<Subscription>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextPart {
    Inline(usize),
    Source,
    Code,
    Cell(usize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TextPoint {
    block: u64,
    part: TextPart,
    offset: usize,
}
#[derive(Clone, Copy)]
struct DocumentSelection {
    anchor: TextPoint,
    head: TextPoint,
    dragging: bool,
}
struct TextEditor {
    block: u64,
    part: TextPart,
    editor: Entity<CodeCard>,
}
impl TextEditor {
    fn matches(&self, point: TextPoint) -> bool {
        self.block == point.block && self.part == point.part
    }
}
#[derive(Clone)]
struct BlockDrag {
    theme: ThemePreset,
    owner: gpui::EntityId,
    id: u64,
    label: SharedString,
}
impl Render for BlockDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        div()
            .px_4()
            .py_2()
            .rounded(px(8.))
            .bg(rgb(theme.palette().surface))
            .shadow_lg()
            .text_size(px(13.))
            .child(self.label.clone())
    }
}
struct RenderBlock {
    id: u64,
    markdown: String,
    kind: DocumentBlockKind,
    nodes: Vec<Node>,
    label: SharedString,
    code: Option<(String, String)>,
}
impl RenderBlock {
    fn kind(&self) -> DocumentBlockKind {
        self.kind.clone()
    }
}

/// A native Markdown block editor. Source is owned here; persistence is handled
/// by subscribers to DocumentEvent::Changed. Register bind_document_editor_keys once.
pub struct DocumentEditor {
    embedded_preview: bool,
    scrollable_preview: bool,
    render_blocks: HashMap<u64, Rc<RenderBlock>>,
    render_references: String,
    character_count: usize,
    layout_measure_passes: u8,
    layout_width: gpui::Pixels,
    rendered_blocks: HashSet<u64>,
    render_order: Vec<u64>,
    reveal_block: Option<u64>,
    search_query: SharedString,
    title: Option<SharedString>,
    title_input: Entity<super::TextEntry>,
    _title_subscription: Subscription,
    focus: gpui::FocusHandle,
    focus_root: bool,
    blocks: Vec<DocumentBlock>,
    next_id: u64,
    views: HashMap<u64, BlockView>,
    embedded_mermaids: RefCell<HashMap<String, Entity<MermaidDiagram>>>,
    history: Vec<Vec<DocumentBlock>>,
    future: Vec<Vec<DocumentBlock>>,
    edit_group: Option<u64>,
    active: Option<u64>,
    hovered: Option<u64>,
    menu: Option<u64>,
    slash: Option<u64>,
    slash_index: usize,
    pending_focus: Option<u64>,
    pending_leaf: Option<usize>,
    source_mode: Option<u64>,
    preview: bool,
    rounded_corners: bool,
    status_bar_visible: bool,
    status_message: Option<SharedString>,
    toc_collapsed: bool,
    theme: ThemePreset,
    applied_theme: ThemePreset,
    scroll: ScrollHandle,
    rendering_block: Option<u64>,
    leaf_cursor: Cell<usize>,
    task_cursor: Cell<usize>,
    selection: Option<DocumentSelection>,
}
impl DocumentEditor {
    pub fn new(cx: &mut Context<Self>, markdown: impl AsRef<str>) -> Self {
        let title_input = cx.new(|cx| super::TextEntry::new(cx, "未命名文档", "").font_size(28.));
        let title_subscription = cx.observe(&title_input, |this, input, cx| {
            let value: SharedString = input.read(cx).value().to_owned().into();
            if this.title.as_ref().is_some_and(|title| title != &value) {
                this.title = Some(value.clone());
                cx.emit(DocumentEvent::TitleChanged(value));
                cx.notify();
            }
        });
        let mut this = Self {
            embedded_preview: false,
            scrollable_preview: false,
            render_blocks: HashMap::new(),
            render_references: String::new(),
            character_count: 0,
            layout_measure_passes: 2,
            layout_width: px(0.),
            rendered_blocks: HashSet::new(),
            render_order: Vec::new(),
            reveal_block: None,
            search_query: "".into(),
            title: None,
            title_input,
            _title_subscription: title_subscription,
            focus: cx.focus_handle(),
            focus_root: false,
            blocks: Vec::new(),
            next_id: 1,
            views: HashMap::new(),
            embedded_mermaids: RefCell::new(HashMap::new()),
            history: Vec::new(),
            future: Vec::new(),
            edit_group: None,
            active: None,
            hovered: None,
            menu: None,
            slash: None,
            slash_index: 0,
            pending_focus: None,
            pending_leaf: None,
            source_mode: None,
            preview: false,
            rounded_corners: true,
            status_bar_visible: true,
            status_message: None,
            toc_collapsed: false,
            theme: ThemePreset::default(),
            applied_theme: ThemePreset::default(),
            scroll: ScrollHandle::new(),
            rendering_block: None,
            leaf_cursor: Cell::new(0),
            task_cursor: Cell::new(0),
            selection: None,
        };
        this.load(markdown.as_ref());
        this.ensure_views(cx);
        this
    }
    /// Read-only, content-sized Markdown for chat messages and other host scroll views.
    /// Hides document chrome and leaves scrolling to the parent.
    pub fn embedded_preview(mut self) -> Self {
        self.embedded_preview = true;
        self.preview = true;
        self.status_bar_visible = false;
        self.toc_collapsed = true;
        self.rounded_corners = false;
        self
    }

    /// Read-only bounded Markdown with its own virtualized scroll viewport.
    pub fn scrollable_preview(mut self) -> Self {
        self.preview = true;
        self.scrollable_preview = true;
        self.status_bar_visible = false;
        self.toc_collapsed = true;
        self.rounded_corners = false;
        self
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    /// Current keyword used to decorate document text, without modifying Markdown.
    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    /// Highlight all case-insensitive matches in the title and document text.
    /// An empty query clears decoration. Does not emit Changed or alter undo history.
    pub fn set_search_query(&mut self, query: impl Into<SharedString>, cx: &mut Context<Self>) {
        let query = query.into();
        if self.search_query == query {
            return;
        }
        self.search_query = query.clone();
        self.title_input
            .update(cx, |input, cx| input.set_search_query(query.clone(), cx));
        for view in self.views.values() {
            for editor in std::iter::once(&view.source)
                .chain(&view.inline)
                .chain(&view.cell_editors)
            {
                editor.update(cx, |editor, cx| editor.set_search_query(query.clone(), cx));
            }
        }
        cx.notify();
    }
    /// Controls whether the editor clips its outer corners to a rounded shape.
    pub fn rounded_corners(mut self, rounded: bool) -> Self {
        self.rounded_corners = rounded;
        self
    }
    /// Controls the block-count and keyboard-hint bar below the document.
    pub fn status_bar_visible(mut self, visible: bool) -> Self {
        self.status_bar_visible = visible;
        self
    }
    /// Shows or hides the block-count and keyboard-hint bar below the document.
    pub fn set_status_bar_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.status_bar_visible != visible {
            self.status_bar_visible = visible;
            cx.notify();
        }
    }
    /// Sets an optional application message in the document status bar.
    pub fn set_status_message(
        &mut self,
        message: Option<impl Into<SharedString>>,
        cx: &mut Context<Self>,
    ) {
        let message = message.map(Into::into);
        if self.status_message != message {
            self.status_message = message;
            cx.notify();
        }
    }
    /// Adds a title above the body, outside the Markdown block model.
    pub fn title(mut self, title: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        self.set_title(title, cx);
        self
    }

    pub fn document_title(&self) -> Option<&SharedString> {
        self.title.as_ref()
    }

    /// Programmatic updates do not emit TitleChanged.
    pub fn set_title(&mut self, title: impl Into<SharedString>, cx: &mut Context<Self>) {
        let title = title.into();
        self.title = Some(title.clone());
        if self.title_input.read(cx).value() != title.as_ref() {
            self.title_input
                .update(cx, |input, cx| input.set_value(title, cx));
        }
        cx.notify();
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme == theme && self.applied_theme == theme {
            return;
        }
        self.theme = theme;
        self.title_input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        for view in self.views.values() {
            view.source
                .update(cx, |editor, cx| editor.set_theme(theme, cx));
            for editor in view.inline.iter().chain(&view.cell_editors) {
                editor.update(cx, |editor, cx| editor.set_theme(theme, cx));
            }
            if let Some(table) = &view.table {
                table.update(cx, |t, cx| t.set_theme(theme, cx));
            }
            if let Some(diagram) = &view.mermaid {
                diagram.update(cx, |diagram, cx| diagram.set_theme(theme, cx));
            }
        }
        for diagram in self.embedded_mermaids.borrow().values() {
            diagram.update(cx, |diagram, cx| diagram.set_theme(theme, cx));
        }
        self.applied_theme = theme;
        cx.notify();
    }
    /// Whether the document is currently in read-only preview mode.
    pub fn is_preview(&self) -> bool {
        self.preview
    }

    /// Switch editing mode without adding application controls to the editor.
    pub fn set_preview(&mut self, preview: bool, cx: &mut Context<Self>) {
        if self.preview != preview {
            self.finish_editing(cx);
            self.preview = preview;
            cx.notify();
        }
    }

    pub fn blocks(&self) -> &[DocumentBlock] {
        &self.blocks
    }
    pub fn markdown(&self) -> String {
        self.blocks
            .iter()
            .map(|b| b.markdown.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
    /// Replace a loaded document and clear its local history.
    pub fn set_markdown(&mut self, markdown: impl AsRef<str>, cx: &mut Context<Self>) {
        self.clear_selection(cx);
        self.load(markdown.as_ref());
        self.views.clear();
        self.history.clear();
        self.future.clear();
        self.active = None;
        self.source_mode = None;
        self.menu = None;
        self.slash = None;
        self.edit_group = None;
        self.title_input
            .update(cx, |input, cx| input.set_theme(self.theme, cx));
        self.ensure_views(cx);
        cx.notify();
    }
    fn load(&mut self, source: &str) {
        self.blocks = md::split(source)
            .into_iter()
            .map(|markdown| {
                let id = self.next_id;
                self.next_id += 1;
                DocumentBlock { id, markdown }
            })
            .collect();
    }
    fn remember(&mut self) {
        self.history.push(self.blocks.clone());
        if self.history.len() > 200 {
            self.history.remove(0);
        }
        self.future.clear();
        self.edit_group = None;
    }
    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(DocumentEvent::Changed(self.markdown().into()));
        cx.notify();
    }
    pub fn undo(&mut self, cx: &mut Context<Self>) {
        if let Some(previous) = self.history.pop() {
            self.future
                .push(std::mem::replace(&mut self.blocks, previous));
            self.restore(cx);
        }
    }
    pub fn redo(&mut self, cx: &mut Context<Self>) {
        if let Some(next) = self.future.pop() {
            self.history.push(std::mem::replace(&mut self.blocks, next));
            self.restore(cx);
        }
    }
    fn restore(&mut self, cx: &mut Context<Self>) {
        self.clear_selection(cx);
        self.views.clear();
        self.active = None;
        self.source_mode = None;
        self.menu = None;
        self.slash = None;
        self.edit_group = None;
        self.pending_focus = None;
        self.pending_leaf = None;
        self.focus_root = true;
        self.ensure_views(cx);
        self.changed(cx);
    }
    fn index(&self, id: u64) -> Option<usize> {
        self.blocks.iter().position(|b| b.id == id)
    }
    /// Commit the active block and dismiss editing controls.
    pub fn finish_editing(&mut self, cx: &mut Context<Self>) {
        self.clear_selection(cx);
        self.close_commands(cx);
        let mut blocks = Vec::new();
        let mut normalized = false;
        for block in self.blocks.clone() {
            let pieces = md::split(&block.markdown);
            normalized |= pieces.len() > 1;
            for (index, markdown) in pieces.into_iter().enumerate() {
                let id = if index == 0 {
                    block.id
                } else {
                    let id = self.next_id;
                    self.next_id += 1;
                    id
                };
                blocks.push(DocumentBlock { id, markdown });
            }
        }
        if normalized {
            self.remember();
            self.blocks = blocks;
            self.changed(cx);
        }
        self.active = None;
        self.menu = None;
        self.source_mode = None;
        self.edit_group = None;
        self.views.clear();
        self.focus_root = true;
        cx.notify();
    }
    fn close_commands(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.slash.take()
            && let Some(view) = self.views.get(&id)
        {
            view.source
                .update(cx, |editor, cx| editor.set_command_mode(false, cx));
            for editor in &view.inline {
                editor.update(cx, |editor, cx| editor.set_command_mode(false, cx));
            }
        }
    }
    fn activate(&mut self, id: u64, cx: &mut Context<Self>) {
        self.close_commands(cx);
        self.active = Some(id);
        self.pending_focus = Some(id);
        self.pending_leaf = None;
        self.menu = None;
        self.edit_group = None;
        cx.notify();
    }
    fn source_changed(&mut self, id: u64, value: &str, cx: &mut Context<Self>) {
        self.clear_selection(cx);
        let Some(i) = self.index(id) else { return };
        let old = &self.blocks[i].markdown;
        let next =
            if self.blocks[i].kind() == DocumentBlockKind::Code && self.source_mode != Some(id) {
                let (lang, _) = md::code(old);
                md::fence(&lang, value)
            } else {
                value.to_owned()
            };
        if *old == next {
            return;
        }
        if self.edit_group != Some(id) {
            self.remember();
            self.edit_group = Some(id);
        }
        self.blocks[i].markdown = next;
        let slash = value.starts_with('/')
            && !value.contains('\n')
            && self.blocks[i].kind() != DocumentBlockKind::Code;
        self.slash = slash.then_some(id);
        self.slash_index = 0;
        if let Some(view) = self.views.get(&id) {
            view.source
                .update(cx, |e, cx| e.set_command_mode(slash, cx));
        }
        self.changed(cx);
    }
    fn source_event(&mut self, id: u64, event: &CodeCardEvent, cx: &mut Context<Self>) {
        match event {
            CodeCardEvent::SelectionStarted {
                editor,
                offset,
                extend,
            } => {
                self.start_selection(*editor, *offset, *extend, cx);
            }
            CodeCardEvent::Focused => {
                if self.active != Some(id) {
                    self.close_commands(cx);
                    self.edit_group = None;
                }
                self.active = Some(id);
                cx.notify();
            }
            CodeCardEvent::FormatApplied => self.edit_group = None,
            CodeCardEvent::Changed(value) => self.source_changed(id, value, cx),
            CodeCardEvent::ContinueAfter => self.insert_after(id, "", cx),
            CodeCardEvent::UndoRequested => self.undo(cx),
            CodeCardEvent::RedoRequested => self.redo(cx),
            CodeCardEvent::Split { before, after } => {
                if let Some(i) = self.index(id) {
                    if self.blocks[i].kind() == DocumentBlockKind::List
                        && self.source_mode != Some(id)
                        && let Some((markdown, leaf)) = continue_list_item(&before, &after)
                    {
                        self.remember();
                        self.blocks[i].markdown = markdown;
                        self.views.remove(&id);
                        self.ensure_views(cx);
                        self.active = Some(id);
                        self.pending_focus = Some(id);
                        self.pending_leaf = Some(leaf);
                        if let Some(editor) =
                            self.views.get(&id).and_then(|view| view.inline.get(leaf))
                        {
                            editor.update(cx, |editor, cx| editor.set_cursor_position(0, cx));
                        }
                        self.changed(cx);
                    } else {
                        self.remember();
                        self.blocks[i].markdown = before.clone();
                        self.views.remove(&id);
                        self.insert_at(i + 1, after.clone(), cx);
                    }
                }
            }
            CodeCardEvent::MergePrevious => self.merge_previous(id, cx),
            CodeCardEvent::CommandKey(key) => {
                if key.as_ref() == "escape" {
                    self.close_commands(cx);
                    self.menu = None;
                    self.active = None;
                    self.focus_root = true;
                    cx.notify();
                    return;
                }
                let commands = self.commands();
                if commands.is_empty() {
                    return;
                }
                match key.as_ref() {
                    "up" => self.slash_index = self.slash_index.saturating_sub(1),
                    "down" => self.slash_index = (self.slash_index + 1).min(commands.len() - 1),
                    "enter" => {
                        if let Some(id) = self.slash {
                            self.convert(
                                id,
                                commands[self.slash_index.min(commands.len() - 1)],
                                cx,
                            );
                        }
                    }
                    _ => {}
                }
                cx.notify();
            }
        }
    }
    fn insert_at(&mut self, index: usize, markdown: String, cx: &mut Context<Self>) {
        let id = self.next_id;
        self.next_id += 1;
        self.blocks
            .insert(index.min(self.blocks.len()), DocumentBlock { id, markdown });
        self.slash = None;
        self.active = Some(id);
        self.pending_focus = Some(id);
        self.pending_leaf = None;
        self.ensure_views(cx);
        self.changed(cx);
    }
    fn insert_after(&mut self, id: u64, markdown: &str, cx: &mut Context<Self>) {
        if let Some(i) = self.index(id) {
            self.remember();
            self.insert_at(i + 1, markdown.to_owned(), cx);
        }
    }
    fn merge_previous(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(i) = self.index(id) {
            if i == 0 {
                return;
            }
            if matches!(
                self.blocks[i - 1].kind(),
                DocumentBlockKind::Code | DocumentBlockKind::Table
            ) {
                return;
            }
            self.remember();
            let b = self.blocks.remove(i);
            let previous = self.blocks[i - 1].id;
            self.blocks[i - 1].markdown.push_str(&b.markdown);
            self.views.remove(&id);
            self.views.remove(&previous);
            self.ensure_views(cx);
            self.activate(previous, cx);
            self.changed(cx);
        }
    }
    fn convert(&mut self, id: u64, command: Command, cx: &mut Context<Self>) {
        if let Some(i) = self.index(id) {
            self.remember();
            let source = self.blocks[i].markdown.trim();
            let plain = if source.starts_with('/') {
                ""
            } else {
                source
                    .trim_start_matches('#')
                    .trim_start_matches('>')
                    .trim()
            };
            self.blocks[i].markdown = match command {
                Command::Paragraph => plain.to_owned(),
                Command::H1 => format!("# {}", if plain.is_empty() { "标题" } else { plain }),
                Command::H2 => format!("## {}", if plain.is_empty() { "标题" } else { plain }),
                Command::H3 => format!("### {}", if plain.is_empty() { "标题" } else { plain }),
                Command::Quote => format!(
                    "> {}",
                    if plain.is_empty() {
                        "引用内容"
                    } else {
                        plain
                    }
                ),
                _ => command.sample().to_owned(),
            };
            self.views.remove(&id);
            self.source_mode = None;
            self.slash = None;
            self.menu = None;
            self.ensure_views(cx);
            self.activate(id, cx);
            self.changed(cx);
        }
    }
    pub fn move_block(&mut self, id: u64, before: Option<u64>, cx: &mut Context<Self>) {
        let Some(from) = self.index(id) else { return };
        let target = before
            .and_then(|id| self.index(id))
            .unwrap_or(self.blocks.len());
        if target == from || target == from + 1 {
            return;
        }
        self.remember();
        let block = self.blocks.remove(from);
        self.blocks
            .insert(if from < target { target - 1 } else { target }, block);
        self.menu = None;
        self.focus_root = true;
        self.changed(cx);
    }
    fn block_action(&mut self, id: u64, action: &str, cx: &mut Context<Self>) {
        let Some(i) = self.index(id) else { return };
        match action {
            "edit" => self.activate(id, cx),
            "source" => {
                self.source_mode = Some(id);
                self.views.remove(&id);
                self.ensure_views(cx);
                self.activate(id, cx);
            }
            "copy" => {
                cx.write_to_clipboard(ClipboardItem::new_string(self.blocks[i].markdown.clone()))
            }
            "duplicate" => {
                let source = self.blocks[i].markdown.clone();
                self.insert_after(id, &source, cx);
            }
            "delete" => {
                self.remember();
                self.blocks.remove(i);
                self.views.remove(&id);
                if self.blocks.is_empty() {
                    self.insert_at(0, String::new(), cx);
                }
                self.active = None;
                self.changed(cx);
            }
            "up" => {
                if i > 0 {
                    self.move_block(id, Some(self.blocks[i - 1].id), cx);
                }
            }
            "down" => {
                if i + 1 < self.blocks.len() {
                    self.move_block(id, self.blocks.get(i + 2).map(|b| b.id), cx);
                }
            }
            _ => {}
        }
        self.menu = None;
        cx.notify();
    }
    fn commands(&self) -> Vec<Command> {
        let query = self
            .slash
            .and_then(|id| self.index(id))
            .map(|i| {
                self.blocks[i]
                    .markdown
                    .trim_start_matches('/')
                    .to_lowercase()
            })
            .unwrap_or_default();
        Command::ALL
            .into_iter()
            .filter(|c| c.label().contains(&query) || c.keyword().contains(&query))
            .collect()
    }
    /// Parsing depends on document text and reference definitions, never scroll position.
    fn refresh_render_blocks(&mut self) {
        let ids: HashSet<_> = self.blocks.iter().map(|block| block.id).collect();
        self.views.retain(|id, _| ids.contains(id));
        let unchanged = self
            .blocks
            .iter()
            .map(|block| block.id)
            .eq(self.render_order.iter().copied())
            && self.blocks.iter().all(|block| {
                self.render_blocks
                    .get(&block.id)
                    .is_some_and(|cached| cached.markdown == block.markdown)
            });
        if unchanged {
            return;
        }
        self.layout_measure_passes = 2;
        self.render_order = self.blocks.iter().map(|block| block.id).collect();
        let markdown = self.markdown();
        let references = md::references(&markdown);
        let references_changed = references != self.render_references;
        self.character_count = markdown.chars().filter(|ch| !ch.is_whitespace()).count();
        self.render_blocks.retain(|id, _| ids.contains(id));
        for block in &self.blocks {
            if !references_changed
                && self
                    .render_blocks
                    .get(&block.id)
                    .is_some_and(|cached| cached.markdown == block.markdown)
            {
                continue;
            }
            let kind = block.kind();
            let nodes = md::nodes(&format!("{}\n\n{}", block.markdown, references));
            let label = plain(&nodes).into();
            let code = (kind == DocumentBlockKind::Code).then(|| md::code(&block.markdown));
            self.render_blocks.insert(
                block.id,
                Rc::new(RenderBlock {
                    id: block.id,
                    markdown: block.markdown.clone(),
                    kind,
                    nodes,
                    label,
                    code,
                }),
            );
        }
        self.render_references = references;
        if !self.embedded_mermaids.borrow().is_empty() {
            let mut sources = HashSet::new();
            for block in self.render_blocks.values() {
                collect_mermaid_sources(&block.nodes, &mut sources);
            }
            self.embedded_mermaids
                .borrow_mut()
                .retain(|source, _| sources.contains(source));
        }
    }

    fn ensure_views(&mut self, cx: &mut Context<Self>) {
        self.refresh_render_blocks();
        let blocks: Vec<_> = self
            .blocks
            .iter()
            .map(|block| self.render_blocks[&block.id].clone())
            .collect();
        for block in blocks {
            let id = block.id;
            let code = block.kind() == DocumentBlockKind::Code && self.source_mode != Some(id);
            let table_mode =
                block.kind() == DocumentBlockKind::Table && self.source_mode != Some(id);
            if self
                .views
                .get(&id)
                .is_some_and(|view| view.mode == (code, table_mode))
            {
                continue;
            }
            self.views.remove(&id);
            let (language, source) = if code {
                md::code(&block.markdown)
            } else {
                ("text".to_owned(), block.markdown.clone())
            };
            let editor = cx.new(|cx| {
                CodeCard::new(cx, language, source)
                    .theme(self.theme)
                    .prose(!code)
                    .document_managed(true)
                    .block_editing(!code && !table_mode && self.source_mode != Some(id))
            });
            let sub = cx.subscribe(&editor, move |this, _, event, cx| {
                this.source_event(id, event, cx)
            });
            let mut view = BlockView {
                mode: (code, table_mode),
                source: editor,
                table: None,
                mermaid: None,
                inline: Vec::new(),
                cell_editors: Vec::new(),
                _subscriptions: vec![sub],
            };
            if !code && !table_mode && self.source_mode != Some(id) {
                let mut leaves = block_leaves(&block.markdown);
                if block.markdown.is_empty() {
                    leaves.push(rich_text::Leaf {
                        range: 0..0,
                        source: String::new(),
                    });
                }
                let references = &self.render_references;
                let split = (leaves.len() == 1
                    && matches!(
                        block.kind(),
                        DocumentBlockKind::Paragraph | DocumentBlockKind::Heading(_)
                    ))
                    || block.kind() == DocumentBlockKind::List;
                for (index, leaf) in leaves.into_iter().enumerate() {
                    let text = format!("{}\n\n{}", leaf.source, references);
                    let editor = cx.new(|cx| {
                        CodeCard::new(cx, "text", "")
                            .theme(self.theme)
                            .rich_text(&text)
                            .document_managed(true)
                            .block_editing(split)
                    });
                    view._subscriptions
                        .push(cx.subscribe(&editor, move |this, _, event, cx| {
                            this.inline_event(id, index, event, cx)
                        }));
                    view.inline.push(editor);
                }
            }
            if block.kind() == DocumentBlockKind::Table && self.source_mode != Some(id) {
                let data = md::table(&block.markdown);
                let mut columns = Vec::new();
                let mut rows = Vec::new();
                for (col, name) in data.cells.first().into_iter().flatten().enumerate() {
                    let header = cx.new(|cx| {
                        CodeCard::new(cx, "text", "")
                            .theme(self.theme)
                            .rich_text(name)
                            .document_managed(true)
                    });
                    let subscription =
                        cx.subscribe(&header, move |this, _, event, cx| match event {
                            CodeCardEvent::Changed(value) => {
                                this.cell_changed(id, 0, col, value, cx)
                            }
                            CodeCardEvent::Focused | CodeCardEvent::FormatApplied => {
                                this.source_event(id, event, cx)
                            }
                            CodeCardEvent::UndoRequested => this.undo(cx),
                            CodeCardEvent::RedoRequested => this.redo(cx),
                            CodeCardEvent::SelectionStarted { .. } => {
                                this.source_event(id, event, cx)
                            }
                            _ => {}
                        });
                    view._subscriptions.push(subscription);
                    view.cell_editors.push(header.clone());
                    let mut column = TableColumn::new(name.clone(), 185.)
                        .flexible()
                        .control(header);
                    if data.alignment.get(col) == Some(&Alignment::Right) {
                        column = column.right();
                    }
                    columns.push(column);
                }
                for (row, cells) in data.cells.iter().skip(1).enumerate() {
                    let mut values = Vec::new();
                    for (col, value) in cells.iter().enumerate() {
                        let cell = cx.new(|cx| {
                            CodeCard::new(cx, "text", "")
                                .theme(self.theme)
                                .rich_text(value)
                                .document_managed(true)
                        });
                        let sub = cx.subscribe(&cell, move |this, _, event, cx| match event {
                            CodeCardEvent::Changed(value) => {
                                this.cell_changed(id, row + 1, col, value, cx)
                            }
                            CodeCardEvent::Focused | CodeCardEvent::FormatApplied => {
                                this.source_event(id, event, cx)
                            }
                            CodeCardEvent::UndoRequested => this.undo(cx),
                            CodeCardEvent::RedoRequested => this.redo(cx),
                            CodeCardEvent::SelectionStarted { .. } => {
                                this.source_event(id, event, cx)
                            }
                            _ => {}
                        });
                        view._subscriptions.push(sub);
                        view.cell_editors.push(cell.clone());
                        values.push(TableCell::editor(cell));
                    }
                    rows.push(TableRow::new(format!("{id}-row-{row}"), values));
                }
                view.table = Some(cx.new(|cx| {
                    DataTable::new(cx, columns, rows)
                        .document()
                        .theme(self.theme)
                }));
            }
            for editor in std::iter::once(&view.source)
                .chain(&view.inline)
                .chain(&view.cell_editors)
            {
                editor.update(cx, |editor, cx| {
                    editor.set_search_query(self.search_query.clone(), cx)
                });
            }
            self.views.insert(id, view);
        }
    }
    fn inline_event(
        &mut self,
        id: u64,
        leaf_index: usize,
        event: &CodeCardEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            CodeCardEvent::Changed(value) => {
                self.clear_selection(cx);
                let Some(i) = self.index(id) else { return };
                let leaves = block_leaves(&self.blocks[i].markdown);
                let range = leaves
                    .get(leaf_index)
                    .map(|leaf| leaf.range.clone())
                    .unwrap_or(0..0);
                if self.edit_group != Some(id) {
                    self.remember();
                    self.edit_group = Some(id);
                }
                let prefix = &self.blocks[i].markdown[..range.start];
                let line_prefix = prefix.rsplit('\n').next().unwrap_or("");
                let quote_prefix = line_prefix
                    .chars()
                    .take_while(|c| *c == '>' || c.is_whitespace())
                    .collect::<String>();
                let value = if quote_prefix.contains('>') {
                    value.replace('\n', &format!("\n{quote_prefix}"))
                } else {
                    value.to_string()
                };
                self.blocks[i].markdown.replace_range(range, &value);
                self.slash_index = 0;
                self.slash = (self.blocks[i].markdown.starts_with('/')
                    && !self.blocks[i].markdown.contains('\n'))
                .then_some(id);
                if let Some(view) = self.views.get(&id)
                    && let Some(editor) = view.inline.get(leaf_index)
                {
                    editor.update(cx, |editor, cx| {
                        editor.set_command_mode(self.slash == Some(id), cx)
                    });
                }
                self.changed(cx);
            }
            CodeCardEvent::Split { before, after } => {
                let Some(i) = self.index(id) else { return };
                let (prefix, suffix) = match block_leaves(&self.blocks[i].markdown).get(leaf_index)
                {
                    Some(leaf) => (
                        self.blocks[i].markdown[..leaf.range.start].to_owned(),
                        self.blocks[i].markdown[leaf.range.end..].to_owned(),
                    ),
                    None => (String::new(), String::new()),
                };
                self.source_event(
                    id,
                    &CodeCardEvent::Split {
                        before: format!("{prefix}{before}"),
                        after: format!("{after}{suffix}"),
                    },
                    cx,
                );
            }
            CodeCardEvent::MergePrevious => {
                let list = self.index(id).is_some_and(|i| {
                    self.blocks[i].kind() == DocumentBlockKind::List && self.source_mode != Some(id)
                });
                if !list || !self.join_list_item(id, leaf_index, cx) {
                    self.source_event(id, event, cx);
                }
            }
            _ => self.source_event(id, event, cx),
        }
    }
    fn cell_changed(
        &mut self,
        id: u64,
        row: usize,
        col: usize,
        value: &str,
        cx: &mut Context<Self>,
    ) {
        self.clear_selection(cx);
        let Some(i) = self.index(id) else { return };
        let mut table = md::table(&self.blocks[i].markdown);
        let Some(cell) = table.cells.get_mut(row).and_then(|r| r.get_mut(col)) else {
            return;
        };
        if cell == value {
            return;
        }
        *cell = value.to_owned();
        if self.edit_group != Some(id) {
            self.remember();
            self.edit_group = Some(id);
        }
        self.blocks[i].markdown = table.markdown();
        self.changed(cx);
    }
    fn table_action(&mut self, id: u64, action: &str, cx: &mut Context<Self>) {
        let Some(i) = self.index(id) else { return };
        let mut table = md::table(&self.blocks[i].markdown);
        self.remember();
        match action {
            "row" => table.cells.push(vec![String::new(); table.alignment.len()]),
            "column" => {
                table.alignment.push(Alignment::None);
                for row in &mut table.cells {
                    row.push(String::new());
                }
            }
            "remove-row" => {
                if table.cells.len() > 1 {
                    table.cells.pop();
                }
            }
            "remove-column" => {
                if table.alignment.len() > 1 {
                    table.alignment.pop();
                    for row in &mut table.cells {
                        row.pop();
                    }
                }
            }
            _ => {}
        }
        self.blocks[i].markdown = table.markdown();
        self.views.remove(&id);
        self.ensure_views(cx);
        self.changed(cx);
    }
}
impl EventEmitter<DocumentEvent> for DocumentEditor {}

fn inline(
    nodes: &[Node],
    text: &mut String,
    highlights: &mut Vec<(Range<usize>, HighlightStyle)>,
    links: &mut Vec<(Range<usize>, String)>,
    style: HighlightStyle,
    theme: ThemePreset,
) {
    for node in nodes {
        let start = text.len();
        let mut current = style;
        match node {
            Node::Text(value) => text.push_str(value),
            Node::Html(value) => {
                if matches!(value.trim(), "<br>" | "<br/>" | "<br />") {
                    text.push('\n');
                } else {
                    text.push_str(value);
                }
            }
            Node::Break => text.push('\n'),
            Node::Rule => {}
            Node::Task(done) => text.push_str(if *done { "☑ " } else { "☐ " }),
            Node::Code(value) | Node::Math(value) => {
                text.push_str(value);
                current.background_color = Some(gpui::rgb_to_hsla(rgba(if theme.is_dark() {
                    0xffffff12
                } else {
                    0x78788022
                })));
                current.color = Some(gpui::rgb_to_hsla(rgb(theme.color(0x343438))));
            }
            Node::Footnote(label) => text.push_str(&format!("[{label}]")),
            Node::Element(tag, children) => {
                match tag {
                    Tag::Strong => current.font_weight = Some(FontWeight::BOLD),
                    Tag::Emphasis => current.font_style = Some(FontStyle::Italic),
                    Tag::Strikethrough => {
                        current.strikethrough = Some(StrikethroughStyle {
                            color: None,
                            thickness: px(1.),
                        })
                    }
                    Tag::Link { .. } => {
                        current.color = Some(gpui::rgb_to_hsla(rgb(theme.color(0x007aff))));
                        current.underline = Some(UnderlineStyle {
                            color: None,
                            thickness: px(1.),
                            wavy: false,
                        });
                    }
                    _ => {}
                }
                inline(children, text, highlights, links, current, theme);
                if let Tag::Link { dest_url, .. } = tag {
                    links.push((start..text.len(), dest_url.to_string()));
                }
                continue;
            }
        }
        if text.len() > start {
            highlights.push((start..text.len(), current));
        }
    }
}
impl DocumentEditor {
    fn text_editors(&self) -> Vec<TextEditor> {
        let mut editors = Vec::new();
        for block in &self.blocks {
            let Some(view) = self.views.get(&block.id) else {
                continue;
            };
            if view.mode.1 {
                editors.extend(view.cell_editors.iter().enumerate().map(|(index, editor)| {
                    TextEditor {
                        block: block.id,
                        part: TextPart::Cell(index),
                        editor: editor.clone(),
                    }
                }));
            } else if self.source_mode == Some(block.id) || view.mode.0 {
                editors.push(TextEditor {
                    block: block.id,
                    part: if view.mode.0 {
                        TextPart::Code
                    } else {
                        TextPart::Source
                    },
                    editor: view.source.clone(),
                });
            } else {
                editors.extend(
                    view.inline
                        .iter()
                        .enumerate()
                        .map(|(index, editor)| TextEditor {
                            block: block.id,
                            part: TextPart::Inline(index),
                            editor: editor.clone(),
                        }),
                );
            }
        }
        editors
    }
    fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selection = None;
        for target in self.text_editors() {
            target
                .editor
                .update(cx, |editor, cx| editor.set_document_selection(None, cx));
        }
        cx.notify();
    }
    fn start_selection(
        &mut self,
        editor_id: gpui::EntityId,
        offset: usize,
        extend: bool,
        cx: &mut Context<Self>,
    ) {
        if self.preview {
            return;
        }
        let Some(target) = self
            .text_editors()
            .into_iter()
            .find(|target| target.editor.entity_id() == editor_id)
        else {
            return;
        };
        let point = TextPoint {
            block: target.block,
            part: target.part,
            offset,
        };
        let anchor = if extend {
            self.selection.map(|s| s.anchor).unwrap_or(point)
        } else {
            point
        };
        self.clear_selection(cx);
        self.selection = Some(DocumentSelection {
            anchor,
            head: point,
            dragging: true,
        });
        self.paint_selection(cx);
    }
    fn ordered_selection(&self) -> Option<(TextPoint, TextPoint)> {
        let selection = self.selection?;
        if selection.anchor.block == selection.head.block
            && selection.anchor.part == selection.head.part
        {
            return None;
        }
        let editors = self.text_editors();
        let anchor = editors.iter().position(|e| e.matches(selection.anchor))?;
        let head = editors.iter().position(|e| e.matches(selection.head))?;
        Some(if anchor < head {
            (selection.anchor, selection.head)
        } else {
            (selection.head, selection.anchor)
        })
    }
    fn paint_selection(&self, cx: &mut Context<Self>) {
        let editors = self.text_editors();
        let ordered = self.ordered_selection();
        let bounds = ordered.and_then(|(start, end)| {
            Some((
                editors.iter().position(|e| e.matches(start))?,
                editors.iter().position(|e| e.matches(end))?,
                start,
                end,
            ))
        });
        for (index, target) in editors.iter().enumerate() {
            let range = bounds.and_then(|(first, last, start, end)| {
                (first..=last).contains(&index).then(|| {
                    let length = target.editor.read(cx).content().len();
                    (if index == first {
                        start.offset.min(length)
                    } else {
                        0
                    })..(if index == last {
                        end.offset.min(length)
                    } else {
                        length
                    })
                })
            });
            target
                .editor
                .update(cx, |editor, cx| editor.set_document_selection(range, cx));
        }
    }
    fn selection_move(
        &mut self,
        event: &gpui::MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preview || !self.selection.is_some_and(|s| s.dragging) {
            return;
        }
        let hit =
            self.text_editors()
                .into_iter()
                .filter(|target| self.rendered_blocks.contains(&target.block))
                .filter_map(|target| {
                    target.editor.read(cx).mouse_text_hit(event.position).map(
                        |(distance, offset)| {
                            (
                                distance,
                                TextPoint {
                                    block: target.block,
                                    part: target.part,
                                    offset,
                                },
                            )
                        },
                    )
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((_, point)) = hit else { return };
        if let Some(selection) = &mut self.selection {
            selection.head = point;
        }
        self.paint_selection(cx);
        if self.ordered_selection().is_some() {
            window.focus(&self.focus, cx);
            self.close_commands(cx);
        } else if let Some(target) = self.text_editors().iter().find(|t| t.matches(point)) {
            window.focus(&target.editor.focus_handle(cx), cx);
        }
        cx.notify();
    }
    fn selection_up(
        &mut self,
        _: &gpui::MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(selection) = &mut self.selection {
            selection.dragging = false;
        }
        if self.ordered_selection().is_some() {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }
    fn delete_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.preview {
            return;
        }
        let Some((start, end)) = self.ordered_selection() else {
            return;
        };
        let Some((blocks, caret)) = delete_text_selection(&self.blocks, start, end) else {
            return;
        };
        self.remember();
        self.clear_selection(cx);
        self.blocks = blocks;
        self.views.clear();
        self.menu = None;
        self.slash = None;
        self.ensure_views(cx);
        if let Some(point) = caret {
            self.active = Some(point.block);
            if let Some(target) = self
                .text_editors()
                .into_iter()
                .find(|target| target.matches(point))
            {
                target.editor.update(cx, |editor, cx| {
                    editor.set_cursor_position(point.offset, cx)
                });
                self.pending_focus = None;
                self.pending_leaf = None;
                self.focus_root = false;
                window.focus(&target.editor.focus_handle(cx), cx);
            }
        } else {
            self.active = None;
            self.focus_root = true;
        }
        self.changed(cx);
    }
    fn selection_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ordered_selection().is_none() {
            return;
        }
        if matches!(event.keystroke.key.as_str(), "backspace" | "delete") {
            self.delete_selection(window, cx);
            cx.stop_propagation();
        } else if event.keystroke.modifiers.platform
            && matches!(event.keystroke.key.as_str(), "c" | "x")
        {
            let (start, end) = self.ordered_selection().unwrap();
            let editors = self.text_editors();
            let first = editors.iter().position(|e| e.matches(start)).unwrap();
            let last = editors.iter().position(|e| e.matches(end)).unwrap();
            let text = editors[first..=last]
                .iter()
                .enumerate()
                .map(|(index, target)| {
                    let content = target.editor.read(cx).content();
                    let a = if index == 0 { start.offset } else { 0 };
                    let b = if index == last - first {
                        end.offset
                    } else {
                        content.len()
                    };
                    content.get(a..b).unwrap_or("").to_owned()
                })
                .collect::<Vec<_>>()
                .join("\n");
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            if event.keystroke.key == "x" {
                self.delete_selection(window, cx);
            }
            cx.stop_propagation();
        } else if event.keystroke.key == "escape" {
            let head = self.selection.unwrap().head;
            self.clear_selection(cx);
            if let Some(target) = self
                .text_editors()
                .into_iter()
                .find(|target| target.matches(head))
            {
                target
                    .editor
                    .update(cx, |editor, cx| editor.set_cursor_position(head.offset, cx));
                window.focus(&target.editor.focus_handle(cx), cx);
            }
            cx.stop_propagation();
        }
    }
    /// Empty items become paragraph blocks; other items join the previous leaf.
    fn join_list_item(&mut self, id: u64, leaf_index: usize, cx: &mut Context<Self>) -> bool {
        let Some(i) = self.index(id) else {
            return false;
        };
        let markdown = self.blocks[i].markdown.clone();
        let leaves = block_leaves(&markdown);
        let Some(leaf) = leaves.get(leaf_index) else {
            return false;
        };
        let Some((line_start, _)) = list_item_line(&markdown, leaf.range.start) else {
            return false;
        };
        if let Some((before, after)) = empty_item_paragraph(&markdown, leaf_index) {
            self.remember();
            let mut replacement = Vec::new();
            if !before.is_empty() {
                replacement.push(DocumentBlock {
                    id: self.next_id,
                    markdown: before,
                });
                self.next_id += 1;
            }
            replacement.push(DocumentBlock {
                id,
                markdown: String::new(),
            });
            if !after.is_empty() {
                replacement.push(DocumentBlock {
                    id: self.next_id,
                    markdown: after,
                });
                self.next_id += 1;
            }
            self.blocks.splice(i..=i, replacement);
            self.views.remove(&id);
            self.ensure_views(cx);
            self.activate(id, cx);
            self.changed(cx);
            return true;
        }
        if line_start == 0 || leaf_index == 0 {
            return false;
        }
        let previous = &leaves[leaf_index - 1];
        // Only join adjacent simple items. Preserve multiline/nested structure.
        if markdown[previous.range.end..line_start].trim() != "" {
            return true;
        }
        let focus = leaf_index - 1;
        let cursor = rich_text::RichText::parse(&previous.source).text.len();
        self.remember();
        self.blocks[i]
            .markdown
            .replace_range(previous.range.end..leaf.range.start, "");
        self.views.remove(&id);
        self.ensure_views(cx);
        self.active = Some(id);
        self.pending_focus = Some(id);
        self.pending_leaf = Some(focus);
        if let Some(editor) = self.views.get(&id).and_then(|view| view.inline.get(focus)) {
            editor.update(cx, |editor, cx| editor.set_cursor_position(cursor, cx));
        }
        self.changed(cx);
        true
    }
    /// Flip the Nth `- [ ]` / `- [x]` marker of a block between checked states.
    fn toggle_task(&mut self, id: u64, index: usize, cx: &mut Context<Self>) {
        let Some(i) = self.index(id) else { return };
        let Some(mark) = find_task_mark(&self.blocks[i].markdown, index) else {
            return;
        };
        self.remember();
        let next = &mut self.blocks[i].markdown;
        next.replace_range(
            mark..mark + 1,
            if &next[mark..mark + 1] == " " {
                "x"
            } else {
                " "
            },
        );
        self.changed(cx);
    }
    fn task_control(
        &self,
        id: Option<u64>,
        done: bool,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let control = checkbox_control_themed(done, false, self.theme).mt(px(4.));
        if let Some(id) = id {
            control
                .id(SharedString::from(format!("document-task-{id}-{index}")))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_task(id, index, cx)))
                .into_any_element()
        } else {
            control.into_any_element()
        }
    }
    fn rich(&self, nodes: &[Node], cx: &mut Context<Self>) -> gpui::AnyElement {
        self.rich_leaf(nodes, false, cx)
    }
    fn highlighted_text(&self, text: impl Into<SharedString>) -> StyledText {
        let text = text.into();
        let highlights = super::search_highlight::apply_styles(
            &text,
            vec![(0..text.len(), HighlightStyle::default())],
            &self.search_query,
            self.theme.is_dark(),
        );
        StyledText::new(text).with_highlights(highlights)
    }
    fn rich_leaf(
        &self,
        nodes: &[Node],
        empty_item: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        // Task markers are drawn by the enclosing list's marker column.
        let task = matches!(nodes.first(), Some(Node::Task(_)));
        let nodes = if task { &nodes[1..] } else { nodes };
        if let Some(id) = self.rendering_block
            && self.source_mode != Some(id)
            && let Some(view) = self.views.get(&id)
        {
            let index = self.leaf_cursor.get();
            if let Some(editor) = view.inline.get(index) {
                if nodes.is_empty() && !task && !empty_item {
                    return div().into_any_element();
                }
                self.leaf_cursor.set(index + 1);
                editor.update(cx, |editor, cx| editor.set_read_only(self.preview, cx));
                return editor.clone().into_any_element();
            }
        }
        if nodes.is_empty() {
            return div().into_any_element();
        }
        let mut text = String::new();
        let mut highlights = Vec::new();
        let mut links = Vec::new();
        inline(
            nodes,
            &mut text,
            &mut highlights,
            &mut links,
            HighlightStyle::default(),
            self.theme,
        );
        let highlights = super::search_highlight::apply_styles(
            &text,
            highlights,
            &self.search_query,
            self.theme.is_dark(),
        );
        let styled = StyledText::new(text).with_highlights(highlights);
        let ranges = links.iter().map(|(range, _)| range.clone()).collect();
        let entity = cx.entity();
        InteractiveText::new("document-inline", styled)
            .on_click(ranges, move |index, _, cx| {
                if let Some((_, url)) = links.get(index) {
                    entity.update(cx, |_, cx| {
                        cx.emit(DocumentEvent::LinkActivated(url.clone().into()))
                    });
                    cx.stop_propagation();
                }
            })
            .into_any_element()
    }
    fn render_nodes(&self, nodes: &[Node], cx: &mut Context<Self>) -> gpui::Div {
        self.render_nodes_in_list_item(nodes, false, cx)
    }

    fn render_nodes_in_list_item(
        &self,
        nodes: &[Node],
        ordered_item: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        // Debug builds have large GPUI element temporaries. Check before entering
        // the large recursive frame, even for ordinary multi-level skill lists.
        stacker::maybe_grow(512 * 1024, 2 * 1024 * 1024, || {
            self.render_nodes_inner(nodes, ordered_item, cx)
        })
    }

    #[inline(never)]
    fn render_nodes_inner(
        &self,
        nodes: &[Node],
        ordered_item: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let theme = self.theme;
        let mut output = div().w_full().min_w_0().flex().flex_col().gap(px(10.));
        let mut index = 0;
        while let Some(node) = nodes.get(index) {
            if is_inline(node) {
                let start = index;
                while nodes.get(index).is_some_and(is_inline) {
                    index += 1;
                }
                output = output.child(div().w_full().child(self.rich(&nodes[start..index], cx)));
                continue;
            }
            index += 1;
            let element = match node {
                Node::Rule => div().h(px(1.)).my_3().bg(rgba(0x78788025)),
                Node::Element(tag, children) => match tag {
                    Tag::Heading { level, .. } => div()
                        .pt(px(8.))
                        .line_height(px(match *level as u8 {
                            1 => 40.,
                            2 => 32.,
                            3 => 26.,
                            _ => 24.,
                        }))
                        .text_size(px(match *level as u8 {
                            1 => 28.,
                            2 => 22.,
                            3 => 18.,
                            _ => 15.,
                        }))
                        .font_weight(FontWeight::BOLD)
                        .child(self.rich(children, cx)),
                    Tag::BlockQuote(_) => div()
                        .pl_4()
                        .py_3()
                        .pr_3()
                        .border_l_2()
                        .border_color(if theme.is_dark() {
                            rgba(0xffffff25)
                        } else {
                            rgb(theme.color(0xd2d6d4))
                        })
                        .rounded_r(px(8.))
                        .bg(rgba(if theme.is_dark() {
                            0xffffff06
                        } else {
                            0x7878800f
                        }))
                        .child(self.render_nodes(children, cx)),
                    Tag::List(start) => {
                        let mut list = div().flex().flex_col().gap_1();
                        for (index, item) in children.iter().enumerate() {
                            let task = item_task(item);
                            let task_index = task.is_some().then(|| {
                                let i = self.task_cursor.get();
                                self.task_cursor.set(i + 1);
                                i
                            });
                            let content = if let Node::Element(Tag::Item, children) = item {
                                if children.is_empty() {
                                    div().w_full().child(self.rich_leaf(children, true, cx))
                                } else {
                                    self.render_nodes_in_list_item(children, start.is_some(), cx)
                                }
                            } else {
                                self.render_nodes(std::slice::from_ref(item), cx)
                            };
                            let marker = match (task, task_index) {
                                (Some(done), Some(task_index)) => {
                                    self.task_control(self.rendering_block, done, task_index, cx)
                                }
                                _ => match start {
                                    Some(s) => div()
                                        .child(format!("{}.", s + index as u64))
                                        .into_any_element(),
                                    None => div()
                                        .size(px(5.))
                                        .rounded_full()
                                        .bg(rgb(theme.palette().text))
                                        .into_any_element(),
                                },
                            };
                            list = list.child(
                                div()
                                    .flex()
                                    .items_start()
                                    .gap(px(6.))
                                    .child(
                                        div()
                                            .debug_selector(|| {
                                                if start.is_some() {
                                                    "list-number-marker".into()
                                                } else {
                                                    "list-bullet-marker".into()
                                                }
                                            })
                                            .w(px(20.))
                                            .h(px(24.))
                                            .flex_shrink_0()
                                            .flex()
                                            .items_center()
                                            .justify_start()
                                            .child(marker),
                                    )
                                    .child(
                                        div()
                                            .debug_selector(|| {
                                                if start.is_some() {
                                                    "list-number-content".into()
                                                } else {
                                                    "list-bullet-content".into()
                                                }
                                            })
                                            .flex_1()
                                            .w(px(0.))
                                            .min_w_0()
                                            .child(content),
                                    ),
                            );
                        }
                        // Unordered continuations beneath a numbered item share
                        // its marker column; Markdown nesting remains unchanged.
                        if ordered_item && start.is_none() {
                            list.ml(px(-26.))
                        } else {
                            list
                        }
                    }
                    Tag::Paragraph
                        if children
                            .iter()
                            .any(|node| matches!(node, Node::Element(Tag::Image { .. }, _))) =>
                    {
                        self.render_nodes(children, cx)
                    }
                    Tag::Paragraph
                    | Tag::Item
                    | Tag::DefinitionListTitle
                    | Tag::DefinitionListDefinition => {
                        div().w_full().child(self.rich(children, cx))
                    }
                    Tag::Image {
                        dest_url, title, ..
                    } => div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            img(dest_url.to_string())
                                .max_w_full()
                                .max_h(px(360.))
                                .object_fit(gpui::ObjectFit::Contain),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(theme.color(0x8e8e93)))
                                .child(self.highlighted_text(if title.is_empty() {
                                    plain(children)
                                } else {
                                    title.to_string()
                                })),
                        ),
                    Tag::Table(alignment) => {
                        let mut table = div()
                            .w_full()
                            .min_w_0()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(rgb(theme.color(0xe5e5ea)))
                            .overflow_hidden();
                        for row in children {
                            if let Node::Element(row_tag, cells) = row {
                                let header = matches!(row_tag, Tag::TableHead);
                                let mut line = div().flex().bg(rgb(if header {
                                    theme.color(0xf2f3f2)
                                } else {
                                    theme.palette().surface
                                }));
                                for (col, cell) in cells.iter().enumerate() {
                                    let mut item = div()
                                        .flex_1()
                                        .w(px(0.))
                                        .min_w_0()
                                        .p_2()
                                        .border_r_1()
                                        .border_b_1()
                                        .border_color(rgb(theme.color(0xe8eae8)))
                                        .text_size(px(12.));
                                    if header {
                                        item = item.font_weight(FontWeight::SEMIBOLD);
                                    }
                                    if alignment.get(col) == Some(&Alignment::Right) {
                                        item = item.text_right();
                                    }
                                    let nodes = if let Node::Element(Tag::TableCell, nodes) = cell {
                                        nodes.as_slice()
                                    } else {
                                        std::slice::from_ref(cell)
                                    };
                                    line = line.child(item.child(self.rich(nodes, cx)));
                                }
                                table = table.child(line);
                            }
                        }
                        table
                    }
                    Tag::CodeBlock(CodeBlockKind::Fenced(info))
                        if info
                            .split_whitespace()
                            .next()
                            .is_some_and(|s| s.eq_ignore_ascii_case("mermaid")) =>
                    {
                        let source = plain(children);
                        let mut diagrams = self.embedded_mermaids.borrow_mut();
                        let diagram = diagrams
                            .entry(source.clone())
                            .or_insert_with(|| {
                                let diagram =
                                    cx.new(|_| MermaidDiagram::new(source).theme(self.theme));
                                cx.subscribe(&diagram, |_, _, event: &MermaidEvent, cx| {
                                    cx.emit(DocumentEvent::Mermaid(event.clone()));
                                })
                                .detach();
                                diagram
                            })
                            .clone();
                        diagram.update(cx, |diagram, cx| diagram.set_theme(self.theme, cx));
                        div().child(diagram)
                    }
                    Tag::CodeBlock(_) => div()
                        .p_3()
                        .rounded(px(8.))
                        .bg(rgb(theme.color(0xf5f5f7)))
                        .font_family("Menlo")
                        .child(self.highlighted_text(plain(children))),
                    Tag::FootnoteDefinition(label) => div()
                        .text_size(px(12.))
                        .child(self.highlighted_text(format!("[{label}] {}", plain(children)))),
                    _ => self.render_nodes(children, cx),
                },
                _ => div().child(self.rich(std::slice::from_ref(node), cx)),
            };
            output = output.child(element);
        }
        output
    }
    fn popup(&self, id: u64, slash: bool, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        let mut popup = div()
            .id(("document-popup", id as usize))
            .debug_selector(|| "document-block-popup".into())
            .w(px(230.))
            .max_h(px(460.))
            .overflow_y_scroll()
            .p(px(6.))
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(theme.color(0xe0e3e0)))
            .bg(rgb(theme.palette().surface))
            .shadow_lg()
            .flex()
            .flex_col()
            .gap(px(2.))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.stop_propagation()),
            )
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.menu = None;
                this.close_commands(cx);
                cx.notify();
            }));
        if slash {
            popup = popup.child(
                div()
                    .px_2()
                    .py_1()
                    .text_size(px(11.))
                    .text_color(rgb(theme.color(0x8e8e93)))
                    .child("插入块 · ↑↓ 选择 · Enter 确认"),
            );
            let commands = self.commands();
            if commands.is_empty() {
                popup = popup.child(div().p_2().child("没有匹配的块"));
            }
            for (index, command) in commands.into_iter().enumerate() {
                popup = popup.child(
                    div()
                        .id(("slash-command", index))
                        .px_2()
                        .py_1()
                        .rounded(px(5.))
                        .bg(if index == self.slash_index {
                            rgba(0x007aff16)
                        } else {
                            rgba(0xffffff00)
                        })
                        .flex()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        .hover(move |s| s.bg(rgba(0x007aff16)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.convert(id, command, cx);
                        }))
                        .child(command.icon().render(14.))
                        .child(command.label()),
                );
            }
        } else {
            for (action, label, icon) in [
                ("edit", "编辑块", Icon::Pencil),
                ("source", "编辑 Markdown 源码", Icon::Code),
                ("copy", "复制块", Icon::Copy),
                ("duplicate", "创建副本", Icon::Copy),
                ("up", "上移", Icon::ArrowUp),
                ("down", "下移", Icon::ChevronDown),
                ("delete", "删除块", Icon::Trash),
            ] {
                popup = popup.child(
                    div()
                        .id(action)
                        .debug_selector(|| format!("document-block-action-{action}"))
                        .px_2()
                        .py_1()
                        .rounded(px(5.))
                        .text_color(rgb(if action == "delete" {
                            theme.color(0xff3b30)
                        } else {
                            theme.color(0x303036)
                        }))
                        .flex()
                        .items_center()
                        .gap_2()
                        .cursor_pointer()
                        .hover(move |s| s.bg(rgba(0x78788012)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.block_action(id, action, cx);
                        }))
                        .child(icon.render(14.))
                        .child(label),
                );
            }
            if self
                .index(id)
                .is_some_and(|i| self.blocks[i].kind() == DocumentBlockKind::Table)
            {
                for (action, label) in [
                    ("row", "新增行"),
                    ("column", "新增列"),
                    ("remove-row", "删除末行"),
                    ("remove-column", "删除末列"),
                ] {
                    popup = popup.child(
                        div()
                            .id(action)
                            .px_2()
                            .py_1()
                            .cursor_pointer()
                            .hover(move |s| s.bg(rgba(0x78788012)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.table_action(id, action, cx);
                                this.menu = None;
                            }))
                            .child(label),
                    );
                }
            }
            popup = popup.child(div().h(px(1.)).my_1().bg(rgb(theme.color(0xe5e5ea))));
            for command in Command::ALL {
                popup = popup.child(
                    div()
                        .id(command.label())
                        .px_2()
                        .py_1()
                        .rounded(px(5.))
                        .text_size(px(12.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(rgba(0x78788012)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.convert(id, command, cx);
                        }))
                        .child(format!("转换为{}", command.label())),
                );
            }
        }
        popup
    }
}
/// Fragments at a visible text offset. `body` omits the suffix block marker so
/// two text blocks can join while retaining the first block's structure.
struct TextFragment {
    markdown: String,
    body: String,
    boundary: String,
    text: Option<rich_text::RichText>,
    quote_prefix: String,
}
fn text_fragment(
    block: &DocumentBlock,
    point: TextPoint,
    before: bool,
    references: &str,
) -> Option<TextFragment> {
    let fragment = match point.part {
        TextPart::Inline(index) => {
            let leaves = block_leaves(&block.markdown);
            let empty = rich_text::Leaf {
                range: 0..0,
                source: String::new(),
            };
            let leaf = leaves
                .get(index)
                .or_else(|| (index == 0 && block.markdown.is_empty()).then_some(&empty))?;
            let text = rich_text::RichText::parse(&format!("{}\n\n{references}", leaf.source));
            if !text.text.is_char_boundary(point.offset) {
                return None;
            }
            let part = text.slice_markdown(if before {
                0..point.offset
            } else {
                point.offset..text.text.len()
            });
            let line_start = block.markdown[..leaf.range.start]
                .rfind('\n')
                .map_or(0, |i| i + 1);
            let line_prefix = &block.markdown[line_start..leaf.range.start];
            let quote_prefix = line_prefix
                .chars()
                .take_while(|c| *c == '>' || c.is_whitespace())
                .collect::<String>();
            let part = if quote_prefix.contains('>') {
                part.replace('\n', &format!("\n{quote_prefix}"))
            } else {
                part
            };
            if before {
                let prefix = if point.offset == 0 {
                    if index == 0 {
                        ""
                    } else if let Some((start, _)) =
                        list_item_line(&block.markdown, leaf.range.start)
                    {
                        block.markdown[..start].trim_end_matches('\n')
                    } else {
                        &block.markdown[..leaf.range.start]
                    }
                } else {
                    &block.markdown[..leaf.range.start]
                };
                let markdown = format!("{prefix}{part}");
                TextFragment {
                    body: markdown.clone(),
                    markdown,
                    boundary: part,
                    text: Some(text),
                    quote_prefix,
                }
            } else {
                let tail = &block.markdown[leaf.range.end..];
                let body = format!("{part}{tail}");
                let markdown = if point.offset == text.text.len() {
                    tail.trim_start_matches('\n').to_owned()
                } else {
                    let line_start = block.markdown[..leaf.range.start]
                        .rfind('\n')
                        .map_or(0, |i| i + 1);
                    format!("{}{body}", &block.markdown[line_start..leaf.range.start])
                };
                TextFragment {
                    markdown,
                    body,
                    boundary: part,
                    text: Some(text),
                    quote_prefix,
                }
            }
        }
        TextPart::Source | TextPart::Code => {
            let code = point.part == TextPart::Code;
            let (language, content) = if code {
                md::code(&block.markdown)
            } else {
                (String::new(), block.markdown.clone())
            };
            if !content.is_char_boundary(point.offset) {
                return None;
            }
            let body = if before {
                &content[..point.offset]
            } else {
                &content[point.offset..]
            }
            .to_owned();
            let markdown = if code && !body.is_empty() {
                md::fence(&language, &body)
            } else {
                body.clone()
            };
            TextFragment {
                markdown,
                body,
                boundary: String::new(),
                text: None,
                quote_prefix: String::new(),
            }
        }
        TextPart::Cell(index) => {
            let mut table = md::table(&block.markdown);
            let count = table.cells.iter().map(Vec::len).sum::<usize>();
            if index >= count {
                return None;
            }
            for (current, cell) in table.cells.iter_mut().flatten().enumerate() {
                if current == index {
                    let text = rich_text::RichText::parse(&format!("{cell}\n\n{references}"));
                    if !text.text.is_char_boundary(point.offset) {
                        return None;
                    }
                    *cell = text.slice_markdown(if before {
                        0..point.offset
                    } else {
                        point.offset..text.text.len()
                    });
                } else if (before && current > index) || (!before && current < index) {
                    cell.clear();
                }
            }
            let markdown = if table.cells.iter().flatten().all(String::is_empty) {
                String::new()
            } else {
                table.markdown()
            };
            TextFragment {
                body: markdown.clone(),
                markdown,
                boundary: String::new(),
                text: None,
                quote_prefix: String::new(),
            }
        }
    };
    Some(fragment)
}
fn delete_text_selection(
    blocks: &[DocumentBlock],
    start: TextPoint,
    end: TextPoint,
) -> Option<(Vec<DocumentBlock>, Option<TextPoint>)> {
    let first = blocks.iter().position(|block| block.id == start.block)?;
    let last = blocks.iter().position(|block| block.id == end.block)?;
    if first > last {
        return None;
    }
    let references = md::references(
        &blocks
            .iter()
            .map(|b| b.markdown.as_str())
            .collect::<Vec<_>>()
            .join("\n\n"),
    );
    let prefix = text_fragment(&blocks[first], start, true, &references)?;
    let suffix = text_fragment(&blocks[last], end, false, &references)?;
    let mut replacement = Vec::new();
    let mut caret = None;
    if first == last
        && let (TextPart::Cell(a), TextPart::Cell(b)) = (start.part, end.part)
    {
        if a > b {
            return None;
        }
        let mut table = md::table(&blocks[first].markdown);
        for (index, cell) in table.cells.iter_mut().flatten().enumerate() {
            if !(a..=b).contains(&index) {
                continue;
            }
            let text = rich_text::RichText::parse(&format!("{cell}\n\n{references}"));
            *cell = if index == a {
                text.slice_markdown(0..start.offset)
            } else if index == b {
                text.slice_markdown(end.offset..text.text.len())
            } else {
                String::new()
            };
        }
        replacement.push(DocumentBlock {
            id: start.block,
            markdown: table.markdown(),
        });
        caret = Some(start);
    } else if matches!(
        (start.part, end.part),
        (TextPart::Inline(_), TextPart::Inline(_))
    ) && (first == last
        || matches!(
            (blocks[first].kind(), blocks[last].kind()),
            (
                DocumentBlockKind::Paragraph | DocumentBlockKind::Heading(_),
                DocumentBlockKind::Paragraph | DocumentBlockKind::Heading(_)
            ) | (DocumentBlockKind::List, DocumentBlockKind::List)
        ))
    {
        let (markdown, point) = if prefix.markdown.is_empty() {
            (
                suffix.markdown,
                TextPoint {
                    block: start.block,
                    part: TextPart::Inline(0),
                    offset: 0,
                },
            )
        } else {
            let mut joined = prefix.text.as_ref()?.join_markdown(
                start.offset,
                suffix.text.as_ref()?,
                end.offset,
            );
            if prefix.quote_prefix.contains('>') {
                joined = joined.replace('\n', &format!("\n{}", prefix.quote_prefix));
            }
            let before = prefix.markdown.strip_suffix(&prefix.boundary)?;
            let after = suffix.body.strip_prefix(&suffix.boundary)?;
            (format!("{before}{joined}{after}"), start)
        };
        if !markdown.is_empty() {
            replacement.push(DocumentBlock {
                id: start.block,
                markdown,
            });
            caret = Some(point);
        }
    } else {
        if !prefix.markdown.is_empty() {
            replacement.push(DocumentBlock {
                id: start.block,
                markdown: prefix.markdown,
            });
            caret = Some(start);
        }
        if !suffix.markdown.is_empty() {
            replacement.push(DocumentBlock {
                id: end.block,
                markdown: suffix.markdown,
            });
            if caret.is_none() {
                caret = Some(TextPoint {
                    offset: 0,
                    part: match end.part {
                        TextPart::Inline(_) => TextPart::Inline(0),
                        part => part,
                    },
                    ..end
                });
            }
        }
    }
    if replacement.is_empty() {
        replacement.push(DocumentBlock {
            id: start.block,
            markdown: String::new(),
        });
        caret = Some(TextPoint {
            block: start.block,
            part: TextPart::Inline(0),
            offset: 0,
        });
    }
    let mut result = blocks.to_vec();
    result.splice(first..=last, replacement);
    if let Some(point) = &mut caret
        && let TextPart::Inline(index) = point.part
        && let Some(block) = result.iter().find(|b| b.id == point.block)
    {
        let leaves = block_leaves(&block.markdown);
        if index >= leaves.len() && !leaves.is_empty() {
            point.part = TextPart::Inline(leaves.len() - 1);
            point.offset = rich_text::RichText::parse(&leaves.last()?.source)
                .text
                .len();
        }
    }
    Some((result, caret))
}
/// Whether a list item holds a task marker, and if so whether it is checked.
fn item_task(item: &Node) -> Option<bool> {
    let Node::Element(Tag::Item, children) = item else {
        return None;
    };
    for node in children {
        match node {
            Node::Task(done) => return Some(*done),
            Node::Element(Tag::Paragraph, paragraph) => {
                if let Some(Node::Task(done)) =
                    paragraph.iter().find(|n| matches!(n, Node::Task(_)))
                {
                    return Some(*done);
                }
            }
            _ => {}
        }
    }
    None
}
/// Byte offset within `line` where a list item's text starts, or None when the
/// line is not a list item.
fn item_marker_end(line: &str) -> Option<usize> {
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];
    let item = rest.strip_prefix(['-', '*', '+']).and_then(|r| {
        if r.is_empty() {
            Some(r)
        } else {
            r.strip_prefix(' ')
        }
    });
    let item = match item {
        Some(item) => item,
        None => {
            let (digits, tail) = rest.split_once('.')?;
            let tail = if tail.is_empty() {
                tail
            } else {
                tail.strip_prefix(' ')?
            };
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            tail
        }
    };
    let task = item
        .strip_prefix("[ ]")
        .or_else(|| item.strip_prefix("[x]"))
        .or_else(|| item.strip_prefix("[X]"))
        .filter(|after| after.is_empty() || after.starts_with(' '));
    let text = match task {
        Some(after) => after.strip_prefix(' ').unwrap_or(after),
        None => item,
    };
    Some(line.len() - text.len())
}
/// The line of the list item whose text starts at `text_start`.
fn list_item_line(markdown: &str, text_start: usize) -> Option<(usize, usize)> {
    let line_start = markdown[..text_start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = markdown[line_start..]
        .find('\n')
        .map_or(markdown.len(), |i| line_start + i);
    let marker_end = item_marker_end(&markdown[line_start..line_end])?;
    (marker_end == text_start - line_start).then_some((line_start, line_end))
}
/// leaves() plus zero-width spans inside bare list items, so empty items stay
/// editable and keep a stable leaf index.
fn block_leaves(markdown: &str) -> Vec<rich_text::Leaf> {
    let mut leaves = rich_text::leaves(markdown);
    for (event, range) in Parser::new_ext(markdown, md::options()).into_offset_iter() {
        if !matches!(event, Event::Start(Tag::Item)) {
            continue;
        }
        let offset = range.start;
        let line = markdown[offset..].split('\n').next().unwrap_or("");
        if let Some(marker_end) = item_marker_end(line)
            && line[marker_end..].trim().is_empty()
        {
            let at = offset + marker_end;
            if !leaves
                .iter()
                .any(|l| l.range.start == at || l.range.contains(&at))
            {
                let position = leaves.iter().take_while(|l| l.range.start < at).count();
                leaves.insert(
                    position,
                    rich_text::Leaf {
                        range: at..at,
                        source: String::new(),
                    },
                );
            }
        }
    }
    leaves
}
/// Keep the surrounding list fragments and turn the empty item into its own block.
fn empty_item_paragraph(markdown: &str, leaf_index: usize) -> Option<(String, String)> {
    let leaves = block_leaves(markdown);
    let leaf = leaves.get(leaf_index)?;
    let (start, end) = list_item_line(markdown, leaf.range.start)?;
    if !leaf.source.is_empty() || !markdown[leaf.range.start..end].trim().is_empty() {
        return None;
    }
    Some((
        markdown[..start].trim_end_matches('\n').to_owned(),
        markdown[end..].trim_start_matches('\n').to_owned(),
    ))
}
/// Enter inside a list item: extend the list with a new item (unchecked for
/// tasks) that carries the text after the caret. Returns the new markdown plus
/// the new item's leaf index.
fn continue_list_item(before: &str, after: &str) -> Option<(String, usize)> {
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = &before[line_start..];
    let indent = &line[..line.len() - line.trim_start().len()];
    let rest = &line[line.len() - line.trim_start().len()..];
    let mut number = None;
    let item = match rest
        .strip_prefix(['-', '*', '+'])
        .and_then(|r| r.strip_prefix(' '))
    {
        Some(item) => item,
        None => {
            let (digits, tail) = rest.split_once('.')?;
            let tail = tail.strip_prefix(' ')?;
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            number = Some(digits.parse::<u64>().ok()?.checked_add(1)?);
            tail
        }
    };
    let task = item.starts_with("[ ]") || item.starts_with("[x]") || item.starts_with("[X]");
    let marker = match number {
        Some(n) => format!("{n}. "),
        None => format!("{} ", &rest[..1]),
    };
    let marker = if task {
        format!("{marker}[ ] ")
    } else {
        marker
    };
    let leaf = block_leaves(before).len();
    Some((format!("{before}\n{indent}{marker}{after}"), leaf))
}
/// Byte offset of the character inside the brackets of the Nth task marker.
fn find_task_mark(source: &str, target: usize) -> Option<usize> {
    Parser::new_ext(source, md::options())
        .into_offset_iter()
        .filter_map(|(event, range)| {
            matches!(event, Event::TaskListMarker(_)).then_some(range.start + 1)
        })
        .nth(target)
}

fn is_inline(node: &Node) -> bool {
    match node {
        Node::Rule => false,
        Node::Element(tag, _) => matches!(
            tag,
            Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. }
        ),
        _ => true,
    }
}
fn collect_mermaid_sources(nodes: &[Node], sources: &mut HashSet<String>) {
    for node in nodes {
        if let Node::Element(tag, children) = node {
            if let Tag::CodeBlock(CodeBlockKind::Fenced(info)) = tag
                && info
                    .split_whitespace()
                    .next()
                    .is_some_and(|s| s.eq_ignore_ascii_case("mermaid"))
            {
                sources.insert(plain(children));
            } else {
                collect_mermaid_sources(children, sources);
            }
        }
    }
}
fn plain(nodes: &[Node]) -> String {
    let mut text = String::new();
    inline(
        nodes,
        &mut text,
        &mut Vec::new(),
        &mut Vec::new(),
        HighlightStyle::default(),
        ThemePreset::default(),
    );
    text
}
impl Render for DocumentEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        stacker::maybe_grow(512 * 1024, 2 * 1024 * 1024, || {
            self.render_document(window, cx)
        })
    }
}

impl DocumentEditor {
    #[inline(never)]
    fn render_document(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        // new() builds the initial child entities before the builder can set a
        // theme. Synchronize those existing views once, before their first paint.
        if self.applied_theme != theme {
            self.set_theme(theme, cx);
        }
        self.ensure_views(cx);
        if self.focus_root {
            window.focus(&self.focus, cx);
            self.focus_root = false;
        }
        if let Some(id) = self.pending_focus.take()
            && let Some(view) = self.views.get(&id)
        {
            if view.mode.1 {
                window.focus(&self.focus, cx);
            } else {
                let leaf = self.pending_leaf.take().unwrap_or(0);
                let editor = view
                    .inline
                    .get(leaf)
                    .or_else(|| view.inline.first())
                    .unwrap_or(&view.source);
                window.focus(&editor.focus_handle(cx), cx);
            }
        }
        let mut toc = div()
            .w(px(185.))
            .flex_shrink_0()
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .text_size(px(11.))
            .text_color(rgb(theme.color(0x6e7470)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .mb_2()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("目录"))
                    .child(
                        div()
                            .id("document-collapse-toc")
                            .size(px(24.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .text_size(px(16.))
                            .cursor_pointer()
                            .hover(move |s| s.bg(rgba(0x78788012)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toc_collapsed = true;
                                cx.notify();
                            }))
                            .child("«"),
                    ),
            );
        let paint_offset = self.scroll.offset();
        let document = cx.entity();
        let mut paper = div()
            .on_children_prepainted(move |_, _, cx| {
                document.update(cx, |this, cx| {
                    let width = this.scroll.bounds().size.width;
                    if this.layout_width != width {
                        this.layout_width = width;
                        this.layout_measure_passes = 2;
                    }
                    if this.layout_measure_passes > 0 {
                        this.layout_measure_passes -= 1;
                        let id = cx.entity_id();
                        cx.defer(move |cx| cx.notify(id));
                    } else if this.scroll.offset() != paint_offset {
                        let id = cx.entity_id();
                        cx.defer(move |cx| cx.notify(id));
                    }
                });
            })
            .id("document-paper")
            .debug_selector(|| "document-paper".into())
            .min_w_0()
            .when(!self.embedded_preview, |paper| {
                paper
                    .flex_1()
                    .w(px(0.))
                    .h_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .pt(px(if self.scrollable_preview { 8. } else { 32. }))
                    .pb(px(if self.scrollable_preview { 8. } else { 32. }))
            })
            .when(self.embedded_preview, |paper| {
                paper.w_full().flex_shrink_0()
            })
            .text_size(px(14.))
            .line_height(px(24.))
            .text_color(rgb(theme.color(0x303036)))
            .flex()
            .flex_col()
            .gap(px(12.))
            .backdrop_blur(px(if theme.is_dark() && !self.embedded_preview {
                36.
            } else {
                0.
            }))
            .bg(if self.embedded_preview || self.scrollable_preview {
                rgba(0x00000000)
            } else if theme.is_dark() {
                rgba(theme.panel_material())
            } else {
                rgb(theme.palette().surface)
            });
        if let Some(title) = &self.title {
            let mut heading = div()
                .w_full()
                .max_w(px(740.))
                .mx_auto()
                .mb_4()
                .h(px(48.))
                .flex_shrink_0()
                .font_weight(FontWeight::BOLD)
                .text_size(px(28.))
                .text_color(rgb(self.theme.palette().text));
            if self.preview {
                let title: SharedString = if title.is_empty() {
                    "未命名文档".into()
                } else {
                    title.clone()
                };
                heading = heading.child(self.highlighted_text(title));
            } else {
                heading = heading.child(self.title_input.clone());
            }
            paper = paper.child(heading);
        }
        let blocks: Vec<_> = self
            .blocks
            .iter()
            .map(|block| self.render_blocks[&block.id].clone())
            .collect();
        let mut toc_items = Vec::new();
        self.rendered_blocks.clear();
        let viewport = self.scroll.bounds();
        let offset = self.scroll.offset().y;
        let title_offset = usize::from(self.title.is_some());
        let can_clip = !self.embedded_preview
            && self.layout_measure_passes == 0
            && viewport.size.height > px(0.);
        for (index, block) in blocks.iter().enumerate() {
            let id = block.id;
            let kind = block.kind();
            let active = self.active == Some(id) && !self.preview;
            if let DocumentBlockKind::Heading(level) = kind
                && !self.toc_collapsed
            {
                toc_items.push((id, level, index, block.label.clone()));
            }
            let bounds = self.scroll.bounds_for_item(index + title_offset);
            let nearby = bounds.is_none_or(|bounds| {
                bounds.bottom() + offset >= viewport.top() - px(500.)
                    && bounds.top() + offset <= viewport.bottom() + px(500.)
            });
            if can_clip
                && !nearby
                && self.active != Some(id)
                && self.menu != Some(id)
                && self.slash != Some(id)
                && self.reveal_block != Some(id)
            {
                // Preserve exact measured height and child order for scroll and TOC anchors.
                paper = paper.child(
                    div()
                        .id(("document-block", id as usize))
                        .w_full()
                        .h(bounds.unwrap().size.height)
                        .flex_shrink_0(),
                );
                continue;
            }
            self.rendered_blocks.insert(id);
            let mut content = div().flex_1().min_w_0().max_w_full().w(px(0.));
            if kind == DocumentBlockKind::Code && self.source_mode != Some(id) {
                self.views[&id]
                    .source
                    .update(cx, |editor, cx| editor.set_read_only(self.preview, cx));
                let (language, source) = block.code.as_ref().expect("cached code block").clone();
                if language.eq_ignore_ascii_case("mermaid") {
                    let source_editor = self.views[&id].source.clone();
                    let diagram = self
                        .views
                        .get_mut(&id)
                        .unwrap()
                        .mermaid
                        .get_or_insert_with(|| {
                            let diagram = cx.new(|_| {
                                MermaidDiagram::new(source.clone())
                                    .theme(self.theme)
                                    .source_editor(source_editor)
                            });
                            cx.subscribe(&diagram, |_, _, event: &MermaidEvent, cx| {
                                cx.emit(DocumentEvent::Mermaid(event.clone()));
                            })
                            .detach();
                            diagram
                        })
                        .clone();
                    diagram.update(cx, |diagram, cx| {
                        diagram.set_source(source, cx);
                        diagram.set_theme(self.theme, cx);
                    });
                    content = content.child(diagram);
                } else {
                    self.views.get_mut(&id).unwrap().mermaid = None;
                    content = content.child(self.views[&id].source.clone());
                }
            } else if kind == DocumentBlockKind::Table && self.source_mode != Some(id) {
                if let Some(table) = &self.views[&id].table {
                    content = content.child(table.clone());
                }
                for editor in &self.views[&id].cell_editors {
                    editor.update(cx, |editor, cx| editor.set_read_only(self.preview, cx));
                }
            } else if self.source_mode == Some(id) && active {
                content = content.child(self.views[&id].source.clone());
            } else {
                self.rendering_block = Some(id);
                self.leaf_cursor.set(0);
                self.task_cursor.set(0);
                if block.markdown.is_empty() {
                    content = content.child(self.views[&id].inline[0].clone());
                } else {
                    content = content.child(self.render_nodes(&block.nodes, cx));
                }
                self.rendering_block = None;
            }
            let heading_level = match kind {
                DocumentBlockKind::Heading(level) => Some(level),
                _ => None,
            };
            let handle_label: SharedString = heading_level
                .map(|level| format!("H{level}").into())
                .unwrap_or_else(|| "⠿".into());
            let mut handle = div()
                .id(("block-handle", id as usize))
                .w(px(26.))
                .h(px(26.))
                .flex_shrink_0()
                .rounded(px(5.))
                .text_color(rgb(theme.color(0x8e8e93)))
                .text_center()
                .opacity(
                    if !self.preview
                        && (self.hovered == Some(id)
                            || (heading_level.is_none() && (active || self.menu == Some(id))))
                    {
                        1.
                    } else {
                        0.
                    },
                )
                .cursor_move()
                .hover(move |s| s.bg(rgba(0x78788018)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if this.preview {
                        return;
                    }
                    this.menu = if this.menu == Some(id) {
                        None
                    } else {
                        Some(id)
                    };
                    this.slash = None;
                    cx.notify();
                }))
                .child(handle_label);
            if heading_level.is_some() {
                handle = handle
                    .mt(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_family("Menlo")
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme.palette().text_muted));
            }
            if !self.preview {
                handle = handle.on_drag(
                    BlockDrag {
                        theme: self.theme,
                        owner: cx.entity_id(),
                        id,
                        label: block.label.chars().take(35).collect::<String>().into(),
                    },
                    |drag, _, _, cx| cx.new(|_| drag.clone()),
                );
            }
            let fully_selected = self.ordered_selection().is_some_and(|(start, end)| {
                let first = self.index(start.block).unwrap_or(index);
                let last = self.index(end.block).unwrap_or(index);
                first < index && index < last
            });
            let mut row = div()
                .id(("document-block", id as usize))
                .relative()
                .bg(if fully_selected {
                    rgba(0x007aff18)
                } else {
                    rgba(0xffffff00)
                })
                .w_full()
                .max_w(px(820.))
                .flex_shrink_0()
                .mx_auto()
                .when(!self.embedded_preview, |row| row.px_5())
                .flex()
                .items_start()
                .gap(px(8.))
                .border_t_2()
                .border_color(rgba(0xffffff00))
                .drag_over::<BlockDrag>(move |style, _, _, _| {
                    style.border_color(rgb(theme.color(0x007aff)))
                })
                .on_drop(cx.listener(move |this, drag: &BlockDrag, _, cx| {
                    if drag.owner == cx.entity_id() {
                        this.move_block(drag.id, Some(id), cx);
                    }
                }))
                .when(!self.preview, |row| {
                    row.on_hover(cx.listener(move |this, hovered, _, cx| {
                        if *hovered {
                            this.hovered = Some(id);
                        } else if this.hovered == Some(id) {
                            this.hovered = None;
                        }
                        cx.notify();
                    }))
                })
                .when(!self.embedded_preview, |row| row.child(handle))
                .child(content);
            if self.menu == Some(id) {
                row = row.child(
                    deferred(
                        div()
                            .absolute()
                            .top(px(30.))
                            .left_0()
                            .child(gpui::anchored().child(self.popup(id, false, cx))),
                    )
                    .with_priority(30),
                );
            }
            if self.slash == Some(id) {
                row = row.child(
                    deferred(
                        div()
                            .absolute()
                            .top(px(30.))
                            .left_0()
                            .child(gpui::anchored().child(self.popup(id, true, cx))),
                    )
                    .with_priority(30),
                );
            }
            paper = paper.child(row);
        }
        if !self.toc_collapsed {
            toc = toc.child(
                gpui::uniform_list(
                    "document-toc-items",
                    toc_items.len(),
                    cx.processor(move |_, range: Range<usize>, _, cx| {
                        range
                            .map(|item| {
                                let (id, level, index, label) = &toc_items[item];
                                let (id, level, index) = (*id, *level, *index);
                                div()
                                    .id(("toc", id as usize))
                                    .h(px(24.))
                                    .pl(px((level.saturating_sub(1) as f32) * 10.))
                                    .rounded(px(4.))
                                    .text_ellipsis()
                                    .cursor_pointer()
                                    .hover(move |s| s.bg(rgba(0x78788010)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.reveal_block = Some(id);
                                        this.scroll.scroll_to_top_of_item(
                                            index + usize::from(this.title.is_some()),
                                        );
                                        cx.notify();
                                    }))
                                    .child(label.clone())
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h_0(),
            );
        }
        self.reveal_block = None;
        let last = self.blocks.last().map(|b| b.id).unwrap_or(0);
        paper = paper.when(!self.preview, |paper| {
            paper.child(
                div()
                    .id("append-document-block")
                    .max_w(px(740.))
                    .w_full()
                    .mx_auto()
                    .py_3()
                    .text_color(rgb(theme.color(0xaeaeb2)))
                    .text_size(px(12.))
                    .cursor_pointer()
                    .on_drop(cx.listener(|this, drag: &BlockDrag, _, cx| {
                        if !this.preview && drag.owner == cx.entity_id() {
                            this.move_block(drag.id, None, cx);
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.preview {
                            this.insert_after(last, "", cx);
                        }
                    }))
                    .child("＋ 添加块"),
            )
        });
        if self.scrollable_preview {
            return div()
                .id("glass-document-scroll-preview")
                .debug_selector(|| "glass-document-scroll-preview".into())
                .size_full()
                .min_w_0()
                .min_h_0()
                .flex()
                .child(paper);
        }
        if self.embedded_preview {
            return div()
                .id("glass-document-preview")
                .debug_selector(|| "glass-document-preview".into())
                .w_full()
                .min_w_0()
                .child(paper);
        }
        div()
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .when(self.rounded_corners, |root| root.rounded(px(12.)))
            .flex()
            .flex_col()
            .bg(if theme.is_dark() {
                rgba(0x00000000)
            } else {
                rgb(theme.palette().canvas)
            })
            .id("glass-document-editor")
            .key_context("GlassDocumentEditor")
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::selection_key))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    if !event.modifiers.shift {
                        this.clear_selection(cx);
                    }
                }),
            )
            .on_mouse_move(cx.listener(Self::selection_move))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(Self::selection_up))
            .on_mouse_up_out(gpui::MouseButton::Left, cx.listener(Self::selection_up))
            .on_action(cx.listener(|this, _: &DocumentUndo, _, cx| this.undo(cx)))
            .on_action(cx.listener(|this, _: &DocumentRedo, _, cx| this.redo(cx)))
            .child(
                div()
                    .flex_1()
                    .h(px(0.))
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .relative()
                    .when(!self.toc_collapsed, |row| row.child(toc))
                    .child(paper)
                    .when(self.toc_collapsed, |row| {
                        row.child(
                            div()
                                .id("document-expand-toc")
                                .absolute()
                                .top(px(8.))
                                .left(px(8.))
                                .px_2()
                                .py_1()
                                .rounded(px(5.))
                                .bg(rgb(self.theme.palette().canvas))
                                .text_size(px(11.))
                                .text_color(rgb(theme.color(0x6e7470)))
                                .cursor_pointer()
                                .hover(move |s| s.bg(rgba(0x78788012)))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toc_collapsed = false;
                                    cx.notify();
                                }))
                                .child("» 目录"),
                        )
                    }),
            )
            .when(self.status_bar_visible, |editor| {
                editor.child(
                    div()
                        .h(px(28.))
                        .flex_shrink_0()
                        .px_4()
                        .flex()
                        .items_center()
                        .text_size(px(10.))
                        .text_color(rgb(theme.color(0x8e8e93)))
                        .child(format!(
                            "{} 个块 · Enter 拆块 · Shift Enter 换行 · ⌘ Enter 下一块",
                            self.blocks.len()
                        ))
                        .child(div().flex_1())
                        .when_some(self.status_message.clone(), |bar, message| {
                            bar.child(
                                div()
                                    .min_w_0()
                                    .max_w(px(300.))
                                    .text_ellipsis()
                                    .child(message),
                            )
                        })
                        .child(format!("{} 字符 · UTF-8 · Markdown", self.character_count)),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    #[gpui::test]
    fn nested_skill_lists_render_without_exhausting_the_ui_stack(cx: &mut gpui::TestAppContext) {
        use super::*;
        let nested = (0..6)
            .map(|depth| {
                format!(
                    "{}- Level {depth} with **text** and `code`",
                    "    ".repeat(depth)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let markdown = format!(
            "# Development guide\n\n{nested}\n\n| Kind | Value |\n| --- | --- |\n| List | Nested |\n\n```rust\nfn example() {{}}\n```"
        );
        let (document, window) =
            cx.add_window_view(|_, cx| DocumentEditor::new(cx, &markdown).scrollable_preview());
        window.run_until_parked();
        document.read_with(window, |editor, _| {
            assert!(editor.markdown().contains("Level 5"));
            assert!(editor.rendered_blocks.contains(&editor.blocks[1].id));
        });
        window.simulate_resize(gpui::size(px(700.), px(500.)));
        window.run_until_parked();
    }

    #[gpui::test]
    fn bounded_read_only_preview_culls_long_thinking_and_keeps_all_content(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::*;
        let text = (0..150)
            .map(|index| format!("Paragraph {index} with **reasoning** and details."))
            .collect::<Vec<_>>()
            .join("\n\n");
        let (document, window) =
            cx.add_window_view(|_, cx| DocumentEditor::new(cx, &text).scrollable_preview());
        window.run_until_parked();
        document.update(window, |editor, cx| {
            assert!(editor.preview);
            assert!(editor.rendered_blocks.len() < editor.blocks.len() / 2);
            assert!(editor.scroll.max_offset().y > px(0.));
            editor.scroll.scroll_to_bottom();
            cx.notify();
        });
        window.run_until_parked();
        document.update(window, |editor, _| {
            assert!(editor.rendered_blocks.len() < editor.blocks.len() / 2);
            assert!(
                editor
                    .rendered_blocks
                    .contains(&editor.blocks.last().unwrap().id)
            );
            assert_eq!(editor.blocks.len(), 150);
        });
    }
    #[gpui::test]
    fn read_only_preview_does_not_track_editing_hover(cx: &mut gpui::TestAppContext) {
        use super::*;
        let (document, window) = cx.add_window_view(|_, cx| {
            DocumentEditor::new(cx, "# Skill\n\nRead **instructions**.").scrollable_preview()
        });
        window.run_until_parked();
        let heading = document.read_with(window, |editor, _| {
            editor.scroll.bounds_for_item(0).unwrap()
        });
        window.simulate_event(gpui::MouseMoveEvent {
            position: heading.center(),
            ..Default::default()
        });
        window.run_until_parked();
        document.read_with(window, |editor, _| assert!(editor.hovered.is_none()));
    }

    #[gpui::test]
    fn embedded_markdown_uses_content_height_and_hides_document_chrome(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::*;
        let (document, window) = cx.add_window_view(|_, cx| {
            DocumentEditor::new(cx, "# Heading\n\n**bold** and `code`\n\n- first\n- second")
                .embedded_preview()
        });
        window.run_until_parked();
        assert!(window.debug_bounds("glass-document-preview").is_some());
        assert!(window.debug_bounds("document-expand-toc").is_none());
        assert!(window.debug_bounds("append-document-block").is_none());
        let paper = window.debug_bounds("document-paper").unwrap();
        assert!(paper.size.height > px(50.) && paper.size.height < px(400.));
        document.update(window, |editor, _| {
            assert!(editor.is_preview());
            assert_eq!(editor.rendered_blocks.len(), editor.blocks.len());
            assert_eq!(editor.scroll.max_offset().y, px(0.));
        });
    }

    #[gpui::test]
    fn block_popup_scroll_does_not_scroll_the_document(cx: &mut gpui::TestAppContext) {
        use super::*;
        let markdown = (0..80)
            .map(|i| format!("正文 {i}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let (document, window) = cx.add_window_view(|_, cx| DocumentEditor::new(cx, &markdown));
        document.update(window, |editor, cx| {
            editor.menu = Some(editor.blocks[0].id);
            cx.notify();
        });
        window.run_until_parked();
        let popup = window.debug_bounds("document-block-popup").unwrap();
        let before = window.debug_bounds("document-block-action-edit").unwrap();
        let parent_offset = document.update(window, |editor, _| {
            assert!(editor.scroll.max_offset().y > px(0.));
            editor.scroll.offset()
        });
        for delta in [-80., -10000., -10000., 10000., 10000.] {
            window.simulate_event(gpui::ScrollWheelEvent {
                position: popup.center(),
                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(delta))),
                ..Default::default()
            });
            window.run_until_parked();
            document.update(window, |editor, _| {
                assert_eq!(editor.scroll.offset(), parent_offset)
            });
            if delta == -80. {
                let after = window.debug_bounds("document-block-action-edit").unwrap();
                assert!(
                    after.top() < before.top(),
                    "menu must scroll: {before:?} -> {after:?}"
                );
            }
        }
    }

    #[gpui::test]
    fn unordered_continuations_align_with_numbered_items(cx: &mut gpui::TestAppContext) {
        use super::*;
        let markdown = "1. 先检查事件来源\n2. 再确认目标接口\n   - 验证鉴权\n   - 检查响应";
        let (document, window) = cx.add_window_view(|_, cx| DocumentEditor::new(cx, markdown));
        for preview in [false, true] {
            document.update(window, |editor, cx| editor.set_preview(preview, cx));
            window.run_until_parked();
            let number = window.debug_bounds("list-number-marker").unwrap();
            let bullet = window.debug_bounds("list-bullet-marker").unwrap();
            let number_text = window.debug_bounds("list-number-content").unwrap();
            let bullet_text = window.debug_bounds("list-bullet-content").unwrap();
            assert!(
                (number.left() - bullet.left()).abs() < px(1.),
                "number={number:?} bullet={bullet:?}"
            );
            assert!(
                (number_text.left() - bullet_text.left()).abs() < px(1.),
                "number={number_text:?} bullet={bullet_text:?}"
            );
            document.update(window, |editor, _| assert_eq!(editor.markdown(), markdown));
        }
    }

    #[gpui::test]
    fn mermaid_rendering_reserves_space_before_following_headings(cx: &mut gpui::TestAppContext) {
        use super::*;
        let source = "flowchart LR\n A[编写文档] --> B{包含图表}\n B --> C[完成]";
        let markdown = format!("```mermaid\n{source}\n```\n\n# 示例标题\n\n## 示例小节");
        let (document, window) = cx.add_window_view(|_, cx| DocumentEditor::new(cx, &markdown));
        window
            .executor()
            .advance_clock(std::time::Duration::from_millis(200));
        window.run_until_parked();
        for (width, preview) in [(1100., false), (500., false), (500., true), (1100., true)] {
            document.update(window, |editor, cx| editor.set_preview(preview, cx));
            window.simulate_resize(gpui::size(px(width), px(700.)));
            window.run_until_parked();
            window
                .executor()
                .advance_clock(std::time::Duration::from_millis(200));
            window.run_until_parked();
            let frame = window.debug_bounds("mermaid-frame").unwrap();
            let image = window
                .debug_bounds("mermaid-image")
                .unwrap_or_else(|| panic!("image missing: width={width} preview={preview}"));
            document.update(window, |editor, cx| {
                let diagram = editor.views[&editor.blocks[0].id].mermaid.as_ref().unwrap();
                assert!(
                    diagram.read(cx).svg().is_some(),
                    "diagram must finish rendering"
                );
                let diagram_bounds = editor.scroll.bounds_for_item(0).unwrap();
                let heading_bounds = editor.scroll.bounds_for_item(1).unwrap();
                let rendered = super::super::mermaid::render_mermaid(source, editor.theme).unwrap();
                assert!(
                    diagram_bounds.size.height >= px(rendered.height.min(480.) + 66.),
                    "diagram={diagram_bounds:?} svg height={}",
                    rendered.height
                );
                assert!(
                    heading_bounds.top() >= diagram_bounds.bottom() + px(10.),
                    "diagram={diagram_bounds:?} heading={heading_bounds:?}"
                );
                let heading_top = heading_bounds.top() + editor.scroll.offset().y;
                assert!(
                    heading_top >= frame.bottom(),
                    "frame={frame:?} heading={heading_bounds:?}"
                );
                assert!(
                    heading_top >= image.bottom(),
                    "image={image:?} heading={heading_bounds:?}"
                );
            });
        }
    }

    #[gpui::test]
    fn document_text_and_code_stay_inside_a_narrow_paper(cx: &mut gpui::TestAppContext) {
        use super::*;
        struct NarrowDocument(Entity<DocumentEditor>);
        impl Render for NarrowDocument {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(px(600.)).h(px(500.)).child(self.0.clone())
            }
        }
        let mut document = None;
        let (_, window) = cx.add_window_view(|_, cx| {
            let input = cx.new(|cx| DocumentEditor::new(cx,
                format!("| 第一列 | 第二列 | 第三列 | 第四列 | 第五列 |\n| --- | --- | --- | --- | --- |\n| 超长表格内容 | 内容 | 内容 | 内容 | 内容 |\n\n{}\n\n```rust\n{}\n```", "正文和 **关键词** ".repeat(100), "long_code_".repeat(100))));
            document = Some(input.clone());
            NarrowDocument(input)
        });
        document.unwrap().update(window, |editor, cx| {
            let paper = editor.scroll.bounds();
            assert!(paper.size.width <= px(600.));
            for (index, block) in editor.blocks.iter().enumerate() {
                let row = editor.scroll.bounds_for_item(index).unwrap();
                assert!(
                    row.right() <= paper.right() + px(1.),
                    "row={row:?} paper={paper:?}"
                );
                let view = &editor.views[&block.id];
                for leaf in &view.inline {
                    let (viewport, wrap, text) = leaf.read(cx).measured_widths();
                    assert!(
                        viewport <= row.size.width,
                        "viewport={viewport:?} row={row:?}"
                    );
                    assert_eq!(wrap, viewport);
                    assert!(
                        text <= viewport + px(1.),
                        "text={text:?} viewport={viewport:?}"
                    );
                }
                if block.kind() == DocumentBlockKind::Code {
                    assert!(view.source.read(cx).measured_widths().0 <= row.size.width);
                }
            }
        });
    }

    #[gpui::test]
    fn scroll_culling_keeps_heights_toc_targets_and_resize_measurements(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::*;
        let markdown = (0..100)
            .map(|i| format!("## 标题 {i}\n\n正文 **关键词** {i}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let (document, window) =
            cx.add_window_view(|_, cx| DocumentEditor::new(cx, &markdown).title("文档标题", cx));
        let (cached, height) = document.update(window, |editor, _| {
            assert_eq!(editor.layout_measure_passes, 0);
            assert!(
                editor.rendered_blocks.len() < editor.blocks.len() / 2,
                "rendered={} total={} viewport={:?} last={:?}",
                editor.rendered_blocks.len(),
                editor.blocks.len(),
                editor.scroll.bounds(),
                editor.scroll.bounds_for_item(editor.blocks.len())
            );
            (
                editor.render_blocks[&editor.blocks[0].id].clone(),
                editor.scroll.max_offset().y,
            )
        });
        document.update(window, |editor, cx| {
            let index = editor.blocks.len() / 2;
            let id = editor.blocks[index].id;
            editor.reveal_block = Some(id);
            editor.scroll.scroll_to_top_of_item(index + 1);
            cx.notify();
        });
        window.run_until_parked();
        document.update(window, |editor, _| {
            assert!(Rc::ptr_eq(
                &cached,
                &editor.render_blocks[&editor.blocks[0].id]
            ));
            assert!((editor.scroll.max_offset().y - height).abs() < px(1.));
            let index = editor.blocks.len() / 2;
            assert!(editor.rendered_blocks.contains(&editor.blocks[index].id));
            let bounds = editor.scroll.bounds_for_item(index + 1).unwrap();
            assert!(
                (bounds.top() + editor.scroll.offset().y - editor.scroll.bounds().top()).abs()
                    < px(1.)
            );
        });
        window.simulate_resize(gpui::size(px(700.), px(500.)));
        window.run_until_parked();
        document.update(window, |editor, _| {
            assert_eq!(editor.layout_measure_passes, 0);
            assert_eq!(editor.layout_width, editor.scroll.bounds().size.width);
        });
        document.update(window, |editor, cx| {
            let first = editor.blocks[0].id;
            let last = editor.blocks.last().unwrap().id;
            editor.move_block(last, Some(first), cx);
        });
        window.run_until_parked();
        document.update(window, |editor, _| {
            assert_eq!(
                editor.render_order,
                editor.blocks.iter().map(|b| b.id).collect::<Vec<_>>()
            );
        });
    }

    #[gpui::test]
    fn search_decoration_survives_mode_theme_and_document_updates(cx: &mut gpui::TestAppContext) {
        use super::*;
        let markdown = "# 关键词\n\n**关键词**和[关键词](https://example.com)\n\n```rust\n// 关键词\n```\n\n| 关键词 | 列 |\n| --- | --- |\n| 关键词 | 内容 |";
        let (document, cx) =
            cx.add_window_view(|_, cx| DocumentEditor::new(cx, markdown).title("关键词标题", cx));
        document.update(cx, |editor, cx| {
            let original = editor.markdown();
            editor.set_search_query("关键词", cx);
            assert_eq!(editor.markdown(), original);
            assert!(editor.history.is_empty());
            assert_eq!(editor.search_query(), "关键词");
            assert!(
                editor
                    .views
                    .values()
                    .any(|view| !view.cell_editors.is_empty())
            );
            for view in editor.views.values() {
                for child in std::iter::once(&view.source)
                    .chain(&view.inline)
                    .chain(&view.cell_editors)
                {
                    assert_eq!(child.read(cx).search_query(), "关键词");
                }
            }
            editor.set_preview(true, cx);
            editor.set_theme(ThemePreset::FogWhite, cx);
            editor.set_markdown("重新加载关键词", cx);
            assert_eq!(editor.search_query(), "关键词");
            for view in editor.views.values() {
                for child in std::iter::once(&view.source).chain(&view.inline) {
                    assert_eq!(child.read(cx).search_query(), "关键词");
                }
            }
        });
        cx.run_until_parked();
        document.update(cx, |editor, cx| {
            editor.set_search_query("", cx);
            assert_eq!(editor.search_query(), "");
            assert_eq!(editor.markdown(), "重新加载关键词");
            for view in editor.views.values() {
                for child in std::iter::once(&view.source).chain(&view.inline) {
                    assert_eq!(child.read(cx).search_query(), "");
                }
            }
            editor.set_preview(false, cx);
        });
        cx.run_until_parked();
    }

    use super::{
        block_leaves, continue_list_item, empty_item_paragraph, find_task_mark, list_item_line,
    };

    #[test]
    fn mermaid_fences_preserve_source_and_find_nested_diagrams() {
        let source = "flowchart LR\n A[开始] --> B[结束]";
        let fence = super::md::fence("mermaid", source);
        assert_eq!(
            super::md::code(&fence),
            ("mermaid".into(), source.to_owned())
        );
        let nested = "> ```Mermaid\n> flowchart LR\n> A --> B\n> ```";
        let mut sources = std::collections::HashSet::new();
        super::collect_mermaid_sources(&super::md::nodes(nested), &mut sources);
        assert!(sources.contains("flowchart LR\nA --> B\n"));
        assert!(super::Command::Mermaid.sample().contains("```mermaid"));
    }

    #[test]
    fn task_marks_locate_the_character_inside_brackets() {
        let source = "- [x] a\n- [ ] b\n\n1. [ ] c\n* [X] d\n> - [ ] e\n";
        let marks: Vec<char> = (0..5)
            .map(|i| {
                let at = find_task_mark(source, i).expect("task mark");
                source[at..].chars().next().unwrap()
            })
            .collect();
        assert_eq!(marks, vec!['x', ' ', ' ', 'X', ' ']);
        assert_eq!(find_task_mark(source, 5), None);
    }

    #[test]
    fn enter_extends_lists_with_a_new_item() {
        let (markdown, leaf) = continue_list_item("- [x] 先检查", "").unwrap();
        assert_eq!(markdown, "- [x] 先检查\n- [ ] ");
        assert_eq!(leaf, 1);
        let (markdown, _) = continue_list_item("2. 再确认", "目标接口").unwrap();
        assert_eq!(markdown, "2. 再确认\n3. 目标接口");
        let (markdown, _) = continue_list_item("- a", "").unwrap();
        assert_eq!(markdown, "- a\n- ");
        let (markdown, _) = continue_list_item("10. a", "").unwrap();
        assert_eq!(markdown, "10. a\n11. ");
        let (markdown, _) = continue_list_item("- [ ] ", "").unwrap();
        assert_eq!(markdown, "- [ ] \n- [ ] ");
        assert!(continue_list_item("正文段落", "").is_none());
    }

    #[test]
    fn bare_items_get_editable_spans_and_lines() {
        let markdown = "- [x] a\n- [ ] \n- b\n";
        let leaves = block_leaves(markdown);
        assert_eq!(leaves.len(), 3);
        assert!(leaves[1].range.is_empty());
        let (start, end) = list_item_line(markdown, leaves[1].range.start).unwrap();
        assert_eq!(&markdown[start..end], "- [ ] ");
        assert!(list_item_line(markdown, 0).is_none());
    }
    #[test]
    fn empty_items_become_paragraphs_without_losing_neighbors() {
        for marker in ["- ", "* ", "+ ", "1. ", "- [ ] ", "- [x] ", "-"] {
            assert_eq!(
                empty_item_paragraph(marker, 0),
                Some((String::new(), String::new()))
            );
            let source = format!("{marker}\n- 后面");
            assert_eq!(
                empty_item_paragraph(&source, 0),
                Some((String::new(), "- 后面".into()))
            );
            let source = format!("- 前面\n{marker}");
            assert_eq!(
                empty_item_paragraph(&source, 1),
                Some(("- 前面".into(), String::new()))
            );
        }
        assert_eq!(
            empty_item_paragraph("- 前面\n- \n- 后面", 1),
            Some(("- 前面".into(), "- 后面".into()))
        );
        assert_eq!(empty_item_paragraph("- 有文字", 0), None);
    }

    #[test]
    fn bare_bullets_remain_editable_before_and_after_typing() {
        let mut source = "- 第一项\n- \n- ".to_owned();
        assert_eq!(block_leaves(&source).len(), 3);
        let range = block_leaves(&source)[1].range.clone();
        source.replace_range(range, "中文输入");
        let leaves = block_leaves(&source);
        assert_eq!(leaves.len(), 3);
        assert_eq!(leaves[1].source, "中文输入");
        assert!(leaves[2].source.is_empty());
        source.replace_range(leaves[1].range.clone(), "");
        assert_eq!(block_leaves(&source).len(), 3);
    }

    #[test]
    fn parser_offsets_exclude_code_and_literal_task_markers() {
        let source =
            "- 外层\n  ```text\n  - [ ] 假任务\n  - \n  ```\n- [ ] 真任务\n- [x]不是任务\n- ";
        let at = find_task_mark(source, 0).unwrap();
        assert_eq!(&source[at - 1..at + 3], "[ ] ");
        assert_eq!(find_task_mark(source, 1), None);
        let leaves = block_leaves(source);
        assert_eq!(leaves.len(), 4);
        assert_eq!(leaves[2].source, "[x]不是任务");
        assert!(leaves[3].source.is_empty());
    }

    #[test]
    fn splitting_an_item_preserves_following_items() {
        let (source, leaf) = continue_list_item("- 前面\n- 中", "文\n- 后面").unwrap();
        assert_eq!(source, "- 前面\n- 中\n- 文\n- 后面");
        assert_eq!(leaf, 2);
        assert_eq!(block_leaves(&source)[leaf].source, "文");
        assert!(continue_list_item("18446744073709551615. a", "").is_none());
    }
    fn blocks(sources: &[&str]) -> Vec<super::DocumentBlock> {
        sources
            .iter()
            .enumerate()
            .map(|(index, source)| super::DocumentBlock {
                id: index as u64 + 1,
                markdown: source.to_string(),
            })
            .collect()
    }
    fn point(block: u64, leaf: usize, offset: usize) -> super::TextPoint {
        super::TextPoint {
            block,
            part: super::TextPart::Inline(leaf),
            offset,
        }
    }
    #[test]
    fn cross_block_delete_keeps_unicode_and_inline_formats() {
        let source = blocks(&["# **中文**标题", "中间段落", "后面*保留*"]);
        let (result, caret) =
            super::delete_text_selection(&source, point(1, 0, 6), point(3, 0, 6)).unwrap();
        assert_eq!(result.len(), 1);
        let leaf = super::block_leaves(&result[0].markdown).remove(0);
        let text = super::rich_text::RichText::parse(&leaf.source);
        assert_eq!(text.text, "中文保留");
        assert_eq!(text.slice_markdown(0..6), "**中文**");
        assert_eq!(text.slice_markdown(6..12), "*保留*");
        assert_eq!(caret, Some(point(1, 0, 6)));
        assert_eq!(source.len(), 3); // Undo stores the untouched original snapshot.
        assert!(super::delete_text_selection(&source, point(1, 0, 1), point(3, 0, 6)).is_none());
    }
    #[test]
    fn full_document_delete_leaves_one_editable_paragraph() {
        let source = blocks(&["第一段", "---", "最后段"]);
        let (result, caret) =
            super::delete_text_selection(&source, point(1, 0, 0), point(3, 0, 9)).unwrap();
        assert_eq!(result, blocks(&[""]));
        assert_eq!(caret, Some(point(1, 0, 0)));
    }
    #[test]
    fn cross_item_delete_keeps_unselected_list_items() {
        let source = blocks(&["- [x] abcd\n- [ ] 中间\n- [ ] efgh\n- 后面"]);
        let (result, caret) =
            super::delete_text_selection(&source, point(1, 0, 2), point(1, 2, 2)).unwrap();
        assert_eq!(result[0].markdown, "- [x] abgh\n- 后面");
        assert_eq!(caret, Some(point(1, 0, 2)));
    }
    #[test]
    fn empty_endpoints_and_removed_leaf_restore_a_valid_caret() {
        let source = blocks(&["", "middle", "rest"]);
        let (result, caret) =
            super::delete_text_selection(&source, point(1, 0, 0), point(3, 0, 2)).unwrap();
        assert_eq!(result, blocks(&["st"]));
        assert_eq!(caret, Some(point(1, 0, 0)));
        let source = blocks(&["- ab\n- \n- cd"]);
        let (result, caret) =
            super::delete_text_selection(&source, point(1, 1, 0), point(1, 2, 2)).unwrap();
        assert_eq!(result[0].markdown, "- ab");
        assert_eq!(caret, Some(point(1, 0, 2)));
    }
    #[test]
    fn deleting_across_code_keeps_the_fence_and_language() {
        let source = blocks(&["before", "```rust\nlet a = 1;\nlet b = 2;\n```", "after"]);
        let end = super::TextPoint {
            block: 2,
            part: super::TextPart::Code,
            offset: 11,
        };
        let (result, _) = super::delete_text_selection(&source, point(1, 0, 3), end).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].markdown, "bef");
        assert_eq!(
            super::md::code(&result[1].markdown),
            ("rust".into(), "let b = 2;".into())
        );
        assert_eq!(result[2].markdown, "after");
    }
    #[test]
    fn table_endpoints_keep_unselected_cells_and_alignment() {
        let source = blocks(&["| ab | cd |\n| :--- | ---: |\n| ef | gh |", "tail"]);
        let start = super::TextPoint {
            block: 1,
            part: super::TextPart::Cell(1),
            offset: 1,
        };
        let (result, _) = super::delete_text_selection(&source, start, point(2, 0, 2)).unwrap();
        let table = super::md::table(&result[0].markdown);
        assert_eq!(table.cells, vec![vec!["ab", "c"], vec!["", ""]]);
        assert_eq!(
            table.alignment,
            vec![
                pulldown_cmark::Alignment::Left,
                pulldown_cmark::Alignment::Right
            ]
        );
        assert_eq!(result[1].markdown, "il");
        let end = super::TextPoint {
            block: 1,
            part: super::TextPart::Cell(2),
            offset: 1,
        };
        let (result, _) = super::delete_text_selection(&source, start, end).unwrap();
        assert_eq!(
            super::md::table(&result[0].markdown).cells,
            vec![vec!["ab", "c"], vec!["f", "gh"]]
        );
    }
    #[test]
    fn cross_quote_leaf_delete_keeps_quoted_continuations() {
        let source = blocks(&["> first\n> second\n>\n> third\n> fourth"]);
        let (result, _) =
            super::delete_text_selection(&source, point(1, 0, 8), point(1, 1, 2)).unwrap();
        assert_eq!(result[0].markdown, "> first  \n> seird  \n> fourth");
        assert_eq!(result[0].kind(), super::DocumentBlockKind::Quote);
    }
    #[test]
    fn source_mode_fragments_use_raw_markdown_offsets() {
        let source = blocks(&["before", "```rust\ncode\n```"]);
        let end = super::TextPoint {
            block: 2,
            part: super::TextPart::Source,
            offset: 4,
        };
        let (result, _) = super::delete_text_selection(&source, point(1, 0, 2), end).unwrap();
        assert_eq!(result[1].markdown, "ust\ncode\n```");
    }
    #[test]
    fn joined_format_boundaries_keep_the_visible_text() {
        for (a, b) in [
            ("**left**", "**right**"),
            ("*left*", "*right*"),
            ("`left`", "`right`"),
            ("~~left~~", "~~right~~"),
        ] {
            let source = blocks(&[a, b]);
            let (result, _) =
                super::delete_text_selection(&source, point(1, 0, 2), point(2, 0, 2)).unwrap();
            let leaf = super::block_leaves(&result[0].markdown).remove(0);
            assert_eq!(
                super::rich_text::RichText::parse(&leaf.source).text,
                "leght",
                "{a} + {b}"
            );
        }
    }
    #[test]
    fn global_reference_links_use_visible_offsets_when_deleting() {
        let source = blocks(&[
            "keep [链接][target]",
            "[继续][target] tail",
            "[target]: https://example.com",
        ]);
        let (result, _) =
            super::delete_text_selection(&source, point(1, 0, 8), point(2, 0, 3)).unwrap();
        let references = super::md::references(
            &result
                .iter()
                .map(|b| b.markdown.as_str())
                .collect::<Vec<_>>()
                .join("\n\n"),
        );
        let leaf = super::block_leaves(&result[0].markdown).remove(0);
        let text = super::rich_text::RichText::parse(&format!("{}\n\n{references}", leaf.source));
        assert_eq!(text.text, "keep 链续 tail");
    }
}
