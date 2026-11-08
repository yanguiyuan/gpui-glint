use super::{Avatar, Icon, PastedImage, PromptEditor, PromptEditorEvent};
use crate::theme::ThemePreset;
use gpui::prelude::FluentBuilder;
use gpui::{
    Animation, AnimationExt as _, App, AppContext, BoxShadow, Context, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, IntoElement, KeyBinding, MouseButton, ObjectFit,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, StyledImage,
    Subscription, Window, actions, deferred, div, img, point, px, relative, rgb, rgba,
};
use std::{collections::HashMap, ops::Range};

/// A standalone @ token around the caret, excluding email addresses.
fn context_query_range(text: &str, caret: usize) -> Option<Range<usize>> {
    if !text.is_char_boundary(caret) {
        return None;
    }
    let prefix = &text[..caret];
    let start = prefix
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map_or(0, |(index, ch)| index + ch.len_utf8());
    if !prefix[start..].starts_with('@') {
        return None;
    }
    let end = text[caret..]
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map_or(text.len(), |(index, _)| caret + index);
    Some(start..end)
}

actions!(glass_composer, [Submit]);

/// Registers the composer shortcuts: Enter / ⌘-Enter submit, Shift-Enter
/// inserts a newline. Scoped to the `GlassComposer` key context.
pub fn bind_composer_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Submit, Some("GlassComposer")),
        KeyBinding::new("cmd-enter", Submit, Some("GlassComposer")),
        KeyBinding::new("ctrl-enter", Submit, Some("GlassComposer")),
        KeyBinding::new(
            "shift-enter",
            super::prompt_editor::Newline,
            Some("GlassComposer"),
        ),
    ]);
}

/// A labeled accessory on the composer's bottom bar. Left chips typically
/// trigger attachments or modes; menu chips sit next to the send button.
///
/// Chips created with [`ComposerChip::option`] behave as selects: clicking
/// opens an option menu above the chip, the selected label replaces the
/// chip's own label, and the choice is reported through
/// [`ComposerEvent::ChipSelected`].
#[derive(Clone, Debug)]
pub struct ComposerChip {
    pub id: SharedString,
    pub icon: Option<Icon>,
    pub label: Option<SharedString>,
    /// Icon and label color override, e.g. a warning tint for permission chips.
    pub tint: Option<u32>,
    pub chevron: bool,
    pub options: Vec<ComposerOption>,
    pub option_avatars: HashMap<SharedString, Avatar>,
    pub default_selected: Option<SharedString>,
}

impl ComposerChip {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            label: None,
            tint: None,
            chevron: false,
            options: Vec::new(),
            option_avatars: HashMap::new(),
            default_selected: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn tint(mut self, tint: u32) -> Self {
        self.tint = Some(tint);
        self
    }

    pub fn chevron(mut self) -> Self {
        self.chevron = true;
        self
    }

    /// Adds a selectable option. One or more options turn the chip into a select.
    pub fn option(
        mut self,
        value: impl Into<SharedString>,
        label: impl Into<SharedString>,
    ) -> Self {
        self.options.push(ComposerOption {
            value: value.into(),
            label: label.into(),
        });
        self
    }

    /// Avatar for a select option, used in both the chip and its menu row.
    pub fn option_avatar(mut self, value: impl Into<SharedString>, avatar: Avatar) -> Self {
        self.option_avatars.insert(value.into(), avatar);
        self
    }

    /// Pre-selects an option by value; defaults to the first option.
    pub fn default_selected(mut self, value: impl Into<SharedString>) -> Self {
        self.default_selected = Some(value.into());
        self
    }
}

/// One selectable value inside a [`ComposerChip`] select.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerOption {
    pub value: SharedString,
    pub label: SharedString,
}

/// One row of the context panel: an optional icon, a label, and an optional
/// grayed description.
#[derive(Clone, Debug)]
pub struct ComposerContextEntry {
    pub id: SharedString,
    pub icon: Option<Icon>,
    pub label: SharedString,
    pub description: Option<SharedString>,
    /// Icon and inserted-mention color; falls back to the accent color.
    pub color: Option<u32>,
    /// Whether selecting this entry inserts a mention, or only emits an event.
    pub insert_mention: bool,
}

impl ComposerContextEntry {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            label: label.into(),
            description: None,
            color: None,
            insert_mention: true,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Use false for entries that open a picker before inserting a mention.
    pub fn insert_mention(mut self, enabled: bool) -> Self {
        self.insert_mention = enabled;
        self
    }

    pub fn color(mut self, color: u32) -> Self {
        self.color = Some(color);
        self
    }
}

/// A titled (or untitled) group of [`ComposerContextEntry`]s in the panel.
#[derive(Clone, Debug)]
pub struct ComposerContextSection {
    pub title: Option<SharedString>,
    pub entries: Vec<ComposerContextEntry>,
}

impl ComposerContextSection {
    pub fn new(
        title: impl Into<SharedString>,
        entries: impl IntoIterator<Item = ComposerContextEntry>,
    ) -> Self {
        Self {
            title: Some(title.into()),
            entries: entries.into_iter().collect(),
        }
    }

    pub fn untitled(entries: impl IntoIterator<Item = ComposerContextEntry>) -> Self {
        Self {
            title: None,
            entries: entries.into_iter().collect(),
        }
    }
}

/// Measured context usage supplied by the host. A zero capacity hides the
/// indicator; the component never estimates token counts from prompt text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComposerContextUsage {
    pub used_tokens: u64,
    pub capacity_tokens: u64,
}

impl ComposerContextUsage {
    pub fn new(used_tokens: u64, capacity_tokens: u64) -> Self {
        Self {
            used_tokens,
            capacity_tokens,
        }
    }

    pub fn remaining_tokens(self) -> u64 {
        self.capacity_tokens.saturating_sub(self.used_tokens)
    }

    pub fn remaining_percent(self) -> u8 {
        if self.capacity_tokens == 0 {
            return 0;
        }
        (self.remaining_tokens() as f64 / self.capacity_tokens as f64 * 100.).round() as u8
    }

    fn used_ratio(self) -> f32 {
        if self.capacity_tokens == 0 {
            return 0.;
        }
        (self.used_tokens as f64 / self.capacity_tokens as f64).clamp(0., 1.) as f32
    }

    fn label(self) -> String {
        format!(
            "上下文剩余 {}% · 已使用 {} / {} tokens",
            self.remaining_percent(),
            self.used_tokens,
            self.capacity_tokens
        )
    }
}

/// A visible indicator even when a server has not supplied both numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposerContextState {
    Initial {
        capacity_tokens: Option<u64>,
    },
    Known(ComposerContextUsage),
    Unknown {
        used_tokens: Option<u64>,
        capacity_tokens: Option<u64>,
    },
}
impl ComposerContextState {
    fn ratio(self) -> f32 {
        match self {
            Self::Known(usage) => usage.used_ratio(),
            Self::Unknown { .. } | Self::Initial { .. } => 0.,
        }
    }
    fn label(self) -> String {
        match self {
            Self::Initial {
                capacity_tokens: Some(capacity),
            } => format!("新会话 · 上下文剩余 100% · 窗口 {capacity} tokens（尚未发送请求）"),
            Self::Initial {
                capacity_tokens: None,
            } => "新会话 · 上下文窗口未知，可在模型设置中填写".into(),
            Self::Known(usage) => format!("{} · 服务端统计，不含未发送草稿", usage.label()),
            Self::Unknown {
                used_tokens,
                capacity_tokens,
            } => match (used_tokens, capacity_tokens) {
                (Some(used), _) => {
                    format!("已使用 {used} tokens · 上下文窗口未知，可在模型设置中填写")
                }
                (_, Some(capacity)) => format!("上下文窗口 {capacity} tokens · 等待服务端返回用量"),
                _ => "上下文窗口与用量未知，可在模型设置中填写窗口上限".into(),
            },
        }
    }
}

/// A prompt ready to send: the text plus any images pasted via ⌘/Ctrl-V.
#[derive(Clone, Debug, PartialEq)]
pub struct ComposerSubmission {
    pub references: Vec<super::ComposerReference>,
    pub text: SharedString,
    pub images: Vec<PastedImage>,
}

impl ComposerSubmission {
    pub fn prompt_text(&self) -> String {
        let references: Vec<_> = self
            .references
            .iter()
            .filter(|reference| reference.is_present(&self.text))
            .map(|r| r.prompt())
            .collect();
        if references.is_empty() {
            self.text.to_string()
        } else {
            format!("{}\n\n{}", self.text, references.join("\n\n"))
        }
    }
    pub fn text(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            images: Vec::new(),
            references: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum ComposerEvent {
    QueueSteerRequested {
        id: u64,
        submission: ComposerSubmission,
    },
    QueueAdded {
        id: u64,
        submission: ComposerSubmission,
    },
    QueueSubmitted {
        id: u64,
        submission: ComposerSubmission,
    },
    QueueRemoved(u64),
    /// The prompt text changed.
    Changed(SharedString),
    AttachmentsChanged,
    ReferenceHovered(super::ComposerReference),
    AttachmentError(SharedString),
    StopRequested,
    /// An accessory chip without options was clicked; the payload is the chip id.
    ChipClicked(SharedString),
    /// A select-style chip picked a new value.
    ChipSelected {
        chip: SharedString,
        value: SharedString,
    },
    /// The prompt was submitted via the send button or the Enter shortcut.
    Submitted(ComposerSubmission),
    /// The prompt was queued because a submission is already in flight.
    Queued(ComposerSubmission),
    /// The context panel opened; hosts can refresh its entries asynchronously.
    ContextOpened,
    /// Current @query changed while the panel is open (also emitted on opening).
    ContextQueryChanged(SharedString),
    /// A context panel entry was picked. With automatic mention insertion enabled,
    /// the matching token has already been inserted by the time this is emitted.
    ContextSelected {
        id: SharedString,
        label: SharedString,
    },
}

/// An AI prompt composer: auto-growing multi-line prompt area over a toolbar
/// of accessory chips, with a busy spinner and send button on the right.
///
/// Submitting while [`Composer::busy`] queues the prompt: queued items render
/// as rows above the card with 立即 / edit / dismiss actions, and the next
/// queued prompt is submitted automatically once the host clears `busy`.
struct QueueEntry {
    scope: Option<String>,
    id: u64,
    submission: ComposerSubmission,
    label: Option<SharedString>,
}
impl std::ops::Deref for QueueEntry {
    type Target = ComposerSubmission;
    fn deref(&self) -> &Self::Target {
        &self.submission
    }
}
impl From<ComposerSubmission> for QueueEntry {
    fn from(submission: ComposerSubmission) -> Self {
        Self {
            id: 0,
            scope: None,
            submission,
            label: None,
        }
    }
}

pub struct Composer {
    editor: Entity<PromptEditor>,
    theme: ThemePreset,
    chips: Vec<ComposerChip>,
    menu_chips: Vec<ComposerChip>,
    hidden_menu_chips: std::collections::HashSet<SharedString>,
    chip_values: HashMap<SharedString, SharedString>,
    open_chip: Option<SharedString>,
    context_usage: Option<ComposerContextState>,
    queue: Vec<QueueEntry>,
    queue_editor: Option<(u64, Entity<PromptEditor>)>,
    queue_editor_references: Vec<super::ComposerReference>,
    queue_editor_subscription: Option<Subscription>,
    managed_queue: bool,
    queue_scope: Option<String>,
    next_queue_id: u64,
    /// Images pasted with ⌘/Ctrl-V, submitted with the next send.
    attachments: Vec<PastedImage>,
    context_trigger: Option<SharedString>,
    context_title: SharedString,
    context_sections: Vec<ComposerContextSection>,
    context_hints: Vec<(SharedString, SharedString)>,
    context_open: bool,
    context_from_at: bool,
    context_selected: usize,
    context_scroll: gpui::ScrollHandle,
    todos: Vec<super::TodoListItem>,
    todos_expanded: bool,
    observed_context_query: SharedString,
    context_on_at: bool,
    observed_text: SharedString,
    mention_colors: HashMap<SharedString, u32>,
    references: Vec<super::ComposerReference>,
    busy: bool,
    stop_on_busy: bool,
    disabled: bool,
    clear_on_submit: bool,
    send_enabled_override: Option<bool>,
    focus: FocusHandle,
    _subscriptions: [Subscription; 2],
}

impl EventEmitter<ComposerEvent> for Composer {}
impl Focusable for Composer {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Composer {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            PromptEditor::new(cx, placeholder)
                .min_rows(2)
                .attach_long_pastes(true)
        });
        let subscription = cx.observe(&editor, |this, _, cx| {
            let text: SharedString = this.editor.read(cx).value().to_owned().into();
            if text != this.observed_text {
                if this.context_on_at
                    && !this.context_sections.is_empty()
                    && this.context_query(cx).is_some()
                {
                    this.open_context_panel(cx);
                    this.context_from_at = true;
                }
                this.observed_text = text.clone();
                cx.emit(ComposerEvent::Changed(text));
            }
            if this.context_from_at && this.context_query(cx).is_none() {
                this.close_context_panel(cx);
            }
            if this.context_open {
                this.emit_context_query(false, cx);
            }
            cx.notify();
        });
        let events = cx.subscribe(&editor, |this, _, event: &PromptEditorEvent, cx| {
            match event {
                PromptEditorEvent::TextAttached(reference) => {
                    this.references.push(reference.clone());
                    this.sync_reference_styles(cx);
                    cx.emit(ComposerEvent::AttachmentsChanged);
                }
                PromptEditorEvent::ReferenceHovered(token) => {
                    if let Some(reference) =
                        this.references.iter().find(|r| r.token == token.as_ref())
                    {
                        cx.emit(ComposerEvent::ReferenceHovered(reference.clone()));
                    }
                }
                PromptEditorEvent::ImagePasted(images) => {
                    if this.attachments.len() + images.len() > 16 {
                        cx.emit(ComposerEvent::AttachmentError(
                            "一次最多添加 16 张图片".into(),
                        ));
                        return;
                    }
                    let bytes: usize = this
                        .attachments
                        .iter()
                        .chain(images)
                        .map(|image| image.bytes().len())
                        .sum();
                    if bytes > 64 * 1024 * 1024 {
                        cx.emit(ComposerEvent::AttachmentError(
                            "附件原始数据总量超过 64MB".into(),
                        ));
                        return;
                    }
                    this.attachments.extend(images.iter().cloned());
                    cx.emit(ComposerEvent::AttachmentsChanged);
                }
                PromptEditorEvent::PasteFailed(error) => {
                    cx.emit(ComposerEvent::AttachmentError(error.clone()))
                }
            }
            cx.notify();
        });
        Self {
            editor,
            theme: ThemePreset::default(),
            chips: Vec::new(),
            menu_chips: Vec::new(),
            hidden_menu_chips: Default::default(),
            chip_values: HashMap::new(),
            open_chip: None,
            context_usage: None,
            queue: Vec::new(),
            queue_editor: None,
            queue_editor_references: Vec::new(),
            queue_editor_subscription: None,
            managed_queue: false,
            queue_scope: None,
            next_queue_id: 0,
            attachments: Vec::new(),
            context_trigger: None,
            context_title: "添加".into(),
            context_sections: Vec::new(),
            context_hints: Vec::new(),
            context_open: false,
            context_from_at: false,
            context_selected: 0,
            context_scroll: Default::default(),
            todos: Vec::new(),
            todos_expanded: true,
            observed_context_query: "".into(),
            context_on_at: false,
            observed_text: "".into(),
            mention_colors: HashMap::new(),
            references: Vec::new(),
            busy: false,
            stop_on_busy: false,
            disabled: false,
            clear_on_submit: true,
            send_enabled_override: None,
            focus: cx.focus_handle(),
            _subscriptions: [subscription, events],
        }
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme == theme {
            return;
        }
        self.theme = theme;
        self.sync_reference_styles(cx);
        self.editor
            .update(cx, |editor, cx| editor.set_theme(theme, cx));
        cx.notify();
    }

    /// Accessory chips rendered on the left of the bottom bar, in order.
    pub fn chip(mut self, chip: ComposerChip) -> Self {
        self.chips.push(chip);
        self
    }

    /// Accessory chips rendered on the right of the bottom bar, before the send button.
    pub fn menu_chip(mut self, chip: ComposerChip) -> Self {
        self.menu_chips.push(chip);
        self
    }

    /// Adds or replaces a toolbar menu without resetting the prompt or queue.
    /// An explicit valid default selects that value; otherwise a still-valid
    /// current selection is retained, falling back to the first option.
    pub fn set_menu_chip(&mut self, chip: ComposerChip, cx: &mut Context<Self>) {
        let valid = |value: &SharedString| chip.options.iter().any(|option| &option.value == value);
        let selected = chip
            .default_selected
            .clone()
            .filter(valid)
            .or_else(|| self.chip_values.get(&chip.id).cloned().filter(valid))
            .or_else(|| chip.options.first().map(|option| option.value.clone()));
        if let Some(value) = selected {
            self.chip_values.insert(chip.id.clone(), value);
        } else {
            self.chip_values.remove(&chip.id);
        }
        if let Some(index) = self
            .menu_chips
            .iter()
            .position(|current| current.id == chip.id)
        {
            self.menu_chips[index] = chip;
        } else {
            self.menu_chips.push(chip);
        }
        // Refreshing another toolbar menu must not dismiss the active menu.
        // Keep this menu open too while its options are still available.
        if self.open_chip.as_ref().is_some_and(|id| {
            self.chips
                .iter()
                .chain(self.menu_chips.iter())
                .find(|chip| &chip.id == id)
                .is_none_or(|chip| chip.options.is_empty())
        }) {
            self.open_chip = None;
        }
        cx.notify();
    }

    /// Changes toolbar visibility without resetting the prompt, selection or queue.
    pub fn set_menu_chip_visible(
        &mut self,
        id: impl Into<SharedString>,
        visible: bool,
        cx: &mut Context<Self>,
    ) {
        let id = id.into();
        let changed = if visible {
            self.hidden_menu_chips.remove(&id)
        } else {
            self.hidden_menu_chips.insert(id.clone())
        };
        if changed {
            if self.open_chip.as_ref() == Some(&id) {
                self.open_chip = None;
            }
            cx.notify();
        }
    }

    /// Makes the chip with this id open the context panel (the `+` /
    /// at-mention panel) instead of emitting `ChipClicked`.
    pub fn context_panel(
        mut self,
        trigger: impl Into<SharedString>,
        title: impl Into<SharedString>,
        sections: impl IntoIterator<Item = ComposerContextSection>,
    ) -> Self {
        self.context_trigger = Some(trigger.into());
        self.context_title = title.into();
        self.context_sections = sections.into_iter().collect();
        self
    }

    /// Bottom hints of the context panel, rendered as key chips with labels.
    pub fn context_hints(
        mut self,
        hints: impl IntoIterator<Item = (impl Into<SharedString>, impl Into<SharedString>)>,
    ) -> Self {
        self.context_hints = hints
            .into_iter()
            .map(|(key, label)| (key.into(), label.into()))
            .collect();
        self
    }

    /// Shows a context usage ring before the model/accessory menus.
    pub fn context_usage(mut self, usage: ComposerContextUsage) -> Self {
        self.context_usage =
            (usage.capacity_tokens > 0).then_some(ComposerContextState::Known(usage));
        self
    }

    /// Updates measured token usage; None or zero capacity hides the ring.
    pub fn set_context_usage(
        &mut self,
        usage: Option<ComposerContextUsage>,
        cx: &mut Context<Self>,
    ) {
        self.set_context_state(
            usage
                .filter(|usage| usage.capacity_tokens > 0)
                .map(ComposerContextState::Known),
            cx,
        );
    }

    pub fn set_context_state(
        &mut self,
        state: Option<ComposerContextState>,
        cx: &mut Context<Self>,
    ) {
        if self.context_usage != state {
            self.context_usage = state;
            cx.notify();
        }
    }

    /// Opens the configured context panel when typing a standalone @.
    pub fn context_on_at(mut self, enabled: bool) -> Self {
        self.context_on_at = enabled;
        self
    }

    /// Appends a colored `@label` mention token to the prompt and registers
    /// its paint color.
    pub fn add_mention(
        &mut self,
        label: impl Into<SharedString>,
        color: u32,
        cx: &mut Context<Self>,
    ) {
        self.add_context_token(format!("@{}", label.into()), color, cx);
    }

    pub fn references(&self, cx: &App) -> Vec<super::ComposerReference> {
        let text = self.editor.read(cx).value();
        self.references
            .iter()
            .filter(|r| r.is_present(&text))
            .cloned()
            .collect()
    }
    pub fn set_references(
        &mut self,
        references: Vec<super::ComposerReference>,
        cx: &mut Context<Self>,
    ) {
        for reference in &self.references {
            self.mention_colors.remove(reference.token.as_str());
        }
        self.references = references;
        self.sync_reference_styles(cx);
    }
    fn sync_reference_styles(&mut self, cx: &mut Context<Self>) {
        let color = self.theme.palette().accent;
        let mut styles: Vec<_> = self
            .references
            .iter()
            .map(|r| (SharedString::from(r.token.clone()), r.icon(), color))
            .collect();
        for (token, color) in &self.mention_colors {
            if !styles.iter().any(|(t, _, _)| t == token) {
                styles.push((
                    token.clone(),
                    if token.starts_with('$') {
                        Icon::Workflow
                    } else {
                        Icon::User
                    },
                    *color,
                ));
            }
        }
        self.editor.update(cx, |editor, cx| {
            editor.set_reference_styles(styles, cx);
            editor.set_reference_previews(self.references.clone(), cx);
        });
    }
    pub fn set_reference_preview(
        &mut self,
        source: &super::ComposerReference,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(reference) = self.references.iter_mut().find(|r| {
            r.token == source.token
                && r.path == source.path
                && r.start_line == source.start_line
                && r.end_line == source.end_line
        }) {
            reference.set_preview(text);
            self.sync_reference_styles(cx);
        }
    }
    pub fn add_reference(
        &mut self,
        mut reference: super::ComposerReference,
        cx: &mut Context<Self>,
    ) {
        if let Some(existing) = self.references.iter_mut().find(|r| {
            r.path == reference.path
                && r.start_line == reference.start_line
                && r.end_line == reference.end_line
        }) {
            reference.token = existing.token.clone();
            *existing = reference.clone();
        } else {
            let base = reference.label();
            let mut token = base.clone();
            let mut number = 2;
            while self.references.iter().any(|r| r.token == token)
                || self.mention_colors.contains_key(token.as_str())
            {
                token = format!("{base}·{number}");
                number += 1;
            }
            reference.token = token;
            self.references.push(reference.clone());
        }
        if self.context_query(cx).is_some() || !reference.is_present(self.editor.read(cx).value()) {
            self.add_context_token(reference.token.clone(), self.theme.palette().accent, cx);
        }
        self.sync_reference_styles(cx);
        cx.emit(ComposerEvent::AttachmentsChanged);
        cx.notify();
    }

    fn context_query(&self, cx: &App) -> Option<(Range<usize>, String)> {
        let editor = self.editor.read(cx);
        let selection = editor.selection_range();
        if !self.context_on_at || !selection.is_empty() {
            return None;
        }
        let range = context_query_range(editor.value(), selection.end)?;
        let query = editor.value()[range.start + 1..selection.end].to_lowercase();
        Some((range, query))
    }
    /// Query after @ at the caret, or an empty string when no mention is active.
    pub fn context_filter(&self, cx: &App) -> String {
        self.context_query(cx)
            .map(|(_, query)| query)
            .unwrap_or_default()
    }
    fn emit_context_query(&mut self, force: bool, cx: &mut Context<Self>) {
        let query: SharedString = self.context_filter(cx).into();
        if force || query != self.observed_context_query {
            self.context_selected = 0;
            self.context_scroll.scroll_to_top_of_item(0);
            self.observed_context_query = query.clone();
            cx.emit(ComposerEvent::ContextQueryChanged(query));
        }
    }

    /// Consume only the @ query at the caret for an action-only context entry,
    /// such as changing the recipient. Other prompt text and attachments stay intact.
    pub fn clear_context_query(&mut self, cx: &mut Context<Self>) {
        if let Some((range, _)) = self.context_query(cx) {
            self.editor
                .update(cx, |editor, cx| editor.replace_range(range, "", cx));
        }
        self.close_context_panel(cx);
    }

    /// Register known context tokens for highlighting restored drafts as well as
    /// newly selected tokens. No prompt text or attachments are changed.
    pub fn register_context_tokens(
        &mut self,
        tokens: impl IntoIterator<Item = (SharedString, u32)>,
        cx: &mut Context<Self>,
    ) {
        self.mention_colors.extend(tokens);
        let highlights = self
            .mention_colors
            .iter()
            .map(|(token, color)| (token.clone(), *color))
            .collect();
        self.editor
            .update(cx, |editor, cx| editor.set_highlights(highlights, cx));
        self.sync_reference_styles(cx);
    }

    /// Append a literal context token such as `$review`, replacing a trailing
    /// standalone @ trigger. Existing text and pasted images are preserved.
    pub fn add_context_token(
        &mut self,
        token: impl Into<SharedString>,
        color: u32,
        cx: &mut Context<Self>,
    ) {
        let token = token.into();
        let query = self.context_query(cx).map(|(range, _)| range);
        if let Some(mut range) = query {
            if self.editor.read(cx).value()[range.end..].starts_with(' ') {
                range.end += 1;
            }
            let replacement = format!("{token} ");
            self.editor.update(cx, |editor, cx| {
                editor.replace_range(range, &replacement, cx);
            });
        } else {
            let mut next = self.editor.read(cx).value().to_owned();
            if !next.is_empty() && !next.ends_with(' ') && !next.ends_with('\n') {
                next.push(' ');
            }
            next.push_str(&token);
            next.push(' ');
            self.editor
                .update(cx, |editor, cx| editor.set_value(next, cx));
        }
        self.register_context_tokens([(token, color)], cx);
        cx.notify();
    }

    /// Replace dynamic context entries without closing an already open panel.
    pub fn set_context_sections(
        &mut self,
        sections: impl IntoIterator<Item = ComposerContextSection>,
        cx: &mut Context<Self>,
    ) {
        self.context_sections = sections.into_iter().collect();
        if self.context_sections.is_empty() {
            self.context_open = false;
        }
        cx.notify();
    }

    pub fn context_panel_open(&self) -> bool {
        self.context_open
    }

    pub fn set_todos(&mut self, todos: Vec<super::TodoListItem>, cx: &mut Context<Self>) {
        if self.todos != todos {
            self.todos_expanded = todos
                .iter()
                .any(|item| item.status != super::TodoListStatus::Completed);
            self.todos = todos;
            cx.notify();
        }
    }

    pub fn open_context_panel(&mut self, cx: &mut Context<Self>) {
        if !self.context_sections.is_empty() {
            let opening = !self.context_open;
            self.context_open = true;
            self.open_chip = None;
            if opening {
                self.context_from_at = false;
                self.context_selected = 0;
                cx.emit(ComposerEvent::ContextOpened);
                self.emit_context_query(true, cx);
            }
            cx.notify();
        }
    }

    pub fn close_context_panel(&mut self, cx: &mut Context<Self>) {
        self.context_from_at = false;
        if self.context_open {
            self.context_open = false;
            cx.notify();
        }
    }

    /// Shows a spinner on the right while a request is in flight. Clearing the
    /// flag submits the oldest queued prompt, if any.
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    /// Replaces send with a stop button while busy. Defaults to false so
    /// existing queue-based composers retain their submission behavior.
    pub fn stop_on_busy(mut self, enabled: bool) -> Self {
        self.stop_on_busy = enabled;
        self
    }

    /// Enables FIFO dispatch through QueueSubmitted, allowing hosts to bind each
    /// queued prompt to its original session/model. Keeps stop available separately.
    pub fn managed_queue(mut self, enabled: bool) -> Self {
        self.managed_queue = enabled;
        self
    }

    pub fn set_queue_label(
        &mut self,
        id: u64,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        if let Some(entry) = self.queue.iter_mut().find(|entry| entry.id == id) {
            entry.label = Some(label.into());
            cx.notify();
        }
    }

    pub fn remove_queued(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(index) = self.queue.iter().position(|entry| entry.id == id) {
            self.queue_remove(index, cx);
        }
    }

    pub fn set_busy(&mut self, busy: bool, cx: &mut Context<Self>) {
        self.busy = busy;
        if !busy {
            self.submit_next_queued(cx);
        }
        cx.notify();
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        cx.notify();
    }

    /// Whether submitting clears the prompt. Defaults to true.
    pub fn clear_on_submit(mut self, clear: bool) -> Self {
        self.clear_on_submit = clear;
        self
    }

    /// Force the send button state; by default it tracks whether the prompt
    /// has any text.
    pub fn send_enabled(mut self, enabled: bool) -> Self {
        self.send_enabled_override = Some(enabled);
        self
    }

    pub fn value(&self, cx: &App) -> SharedString {
        self.editor.read(cx).value().to_owned().into()
    }

    pub fn set_value(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.set_value(text, cx));
        cx.notify();
    }

    /// Restores one conversation's text and images together. Cancels pending
    /// file pastes from the previous conversation via PromptEditor::set_value.
    pub fn set_draft(
        &mut self,
        text: impl Into<SharedString>,
        images: Vec<PastedImage>,
        cx: &mut Context<Self>,
    ) {
        let text = text.into();
        self.set_references(Vec::new(), cx);
        self.observed_text = text.clone();
        for image in std::mem::replace(&mut self.attachments, images) {
            image.source().remove_asset(cx);
        }
        self.context_open = false;
        self.editor
            .update(cx, |editor, cx| editor.set_value(text, cx));
        cx.notify();
    }

    /// Number of prompts waiting in the queue.
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// Images pasted via ⌘/Ctrl-V that will be submitted with the next send.
    pub fn attachments(&self) -> &[PastedImage] {
        &self.attachments
    }

    /// Drops all pasted images without notifying submission flows.
    pub fn clear_attachments(&mut self, cx: &mut Context<Self>) {
        if !self.attachments.is_empty() {
            for image in self.attachments.drain(..) {
                image.source().remove_asset(cx);
            }
            cx.emit(ComposerEvent::AttachmentsChanged);
            cx.notify();
        }
    }

    fn sendable(&self, cx: &App) -> bool {
        if self.disabled {
            return false;
        }
        match self.send_enabled_override {
            Some(enabled) => enabled,
            None => !self.editor.read(cx).is_empty() || !self.attachments.is_empty(),
        }
    }

    fn submission(&mut self, text: SharedString, cx: &mut Context<Self>) -> ComposerSubmission {
        let images = if self.clear_on_submit || self.busy {
            std::mem::take(&mut self.attachments)
        } else {
            self.attachments.clone()
        };
        if self.clear_on_submit || self.busy {
            for image in &images {
                image.source().remove_asset(cx);
            }
        }
        let references = self
            .references
            .iter()
            .filter(|r| r.is_present(&text))
            .cloned()
            .collect();
        if self.clear_on_submit || self.busy {
            self.set_references(Vec::new(), cx);
        }
        ComposerSubmission {
            text,
            images,
            references,
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if !self.sendable(cx) || self.editor.read(cx).is_composing() {
            return;
        }
        let text: SharedString = self.editor.read(cx).value().to_owned().into();
        if self.clear_on_submit || self.busy {
            self.editor
                .update(cx, |editor, cx| editor.set_value("", cx));
        }
        let submission = self.submission(text.clone(), cx);
        if self.busy {
            self.next_queue_id += 1;
            let id = self.next_queue_id;
            self.queue.push(QueueEntry {
                id,
                scope: self.queue_scope.clone(),
                submission: submission.clone(),
                label: None,
            });
            if self.managed_queue {
                cx.emit(ComposerEvent::QueueAdded {
                    id,
                    submission: submission.clone(),
                });
            }
            cx.emit(ComposerEvent::Queued(submission));
        } else {
            cx.emit(ComposerEvent::Submitted(submission));
        }
        cx.notify();
    }

    /// Group queued prompts by the host conversation and show only that group.
    pub fn set_queue_scope(&mut self, scope: String, cx: &mut Context<Self>) {
        if self.queue_scope.as_ref() == Some(&scope) {
            return;
        }
        self.queue_scope = Some(scope);
        self.queue_editor = None;
        self.queue_editor_references.clear();
        self.queue_editor_subscription = None;
        cx.notify();
    }

    /// Dispatch the next queued prompt for a conversation that has become idle.
    pub fn submit_scoped_queue(&mut self, scope: &str, cx: &mut Context<Self>) {
        self.submit_next_in_scope(Some(scope), cx);
    }

    pub fn discard_scoped_queue(&mut self, scope: &str, cx: &mut Context<Self>) {
        self.queue.retain(|entry| entry.scope.as_deref() != Some(scope));
        if self.queue_scope.as_deref() == Some(scope) {
            self.queue_editor = None;
            self.queue_editor_references.clear();
            self.queue_editor_subscription = None;
        }
        cx.notify();
    }

    fn submit_next_queued(&mut self, cx: &mut Context<Self>) {
        let scope = self.queue_scope.clone();
        self.submit_next_in_scope(scope.as_deref(), cx);
    }

    fn submit_next_in_scope(&mut self, scope: Option<&str>, cx: &mut Context<Self>) {
        let Some(index) = self.queue.iter().position(|entry| entry.scope.as_deref() == scope) else {
            return;
        };
        if self.queue_editor.as_ref().is_some_and(|(id, _)| *id == self.queue[index].id) {
            return;
        }
        let next = self.queue.remove(index);
        cx.notify();
        if self.managed_queue {
            cx.emit(ComposerEvent::QueueSubmitted {
                id: next.id,
                submission: next.submission,
            });
        } else {
            cx.emit(ComposerEvent::Submitted(next.submission));
        }
    }

    fn queue_submit_now(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.managed_queue {
            return;
        }
        if index >= self.queue.len() {
            return;
        }
        let submission = self.queue.remove(index);
        cx.emit(ComposerEvent::Submitted(submission.submission));
        cx.notify();
    }

    fn queue_edit(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.queue.len() {
            return;
        }
        if self.managed_queue {
            let entry = &self.queue[index];
            let text = entry.text.clone();
            self.queue_editor_references = entry.submission.references.clone();
            let editor = cx.new(|cx| {
                let mut editor = PromptEditor::new(cx, "编辑排队消息")
                    .min_rows(1)
                    .attach_long_pastes(true)
                    .theme(self.theme);
                editor.set_value(text, cx);
                editor.set_reference_styles(
                    self.queue_editor_references
                        .iter()
                        .map(|r| {
                            (
                                r.token.clone().into(),
                                r.icon(),
                                self.theme.palette().accent,
                            )
                        })
                        .collect(),
                    cx,
                );
                editor.set_reference_previews(self.queue_editor_references.clone(), cx);
                editor
            });
            self.queue_editor_subscription = Some(cx.subscribe(
                &editor,
                |this, _, event: &PromptEditorEvent, cx| {
                    if let PromptEditorEvent::TextAttached(reference) = event {
                        this.queue_editor_references.push(reference.clone());
                        cx.notify();
                    }
                },
            ));
            self.queue_editor = Some((entry.id, editor));
            cx.notify();
            return;
        }
        let submission = self.queue.remove(index).submission;
        self.set_references(submission.references, cx);
        for image in std::mem::replace(&mut self.attachments, submission.images) {
            image.source().remove_asset(cx);
        }
        cx.emit(ComposerEvent::AttachmentsChanged);
        self.editor
            .update(cx, |editor, cx| editor.set_value(submission.text, cx));
        cx.notify();
    }

    fn save_queue_edit(&mut self, cx: &mut Context<Self>) {
        let Some((id, editor)) = &self.queue_editor else {
            return;
        };
        if editor.read(cx).is_composing() {
            return;
        }
        let text: SharedString = editor.read(cx).value().to_owned().into();
        let Some(entry) = self.queue.iter_mut().find(|entry| entry.id == *id) else {
            self.queue_editor = None;
            return;
        };
        if text.trim().is_empty() && entry.images.is_empty() {
            return;
        }
        entry.submission.references = self
            .queue_editor_references
            .iter()
            .filter(|reference| reference.is_present(&text))
            .cloned()
            .collect();
        entry.submission.text = text;
        self.queue_editor = None;
        self.queue_editor_references.clear();
        self.queue_editor_subscription = None;
        if !self.busy {
            self.submit_next_queued(cx);
        }
        cx.notify();
    }

    fn cancel_queue_edit(&mut self, cx: &mut Context<Self>) {
        self.queue_editor = None;
        self.queue_editor_references.clear();
        self.queue_editor_subscription = None;
        if !self.busy {
            self.submit_next_queued(cx);
        }
        cx.notify();
    }

    fn queue_remove(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.queue.len() {
            let removed = self.queue.remove(index);
            let editing = self
                .queue_editor
                .as_ref()
                .is_some_and(|(id, _)| *id == removed.id);
            if editing {
                self.queue_editor = None;
                self.queue_editor_references.clear();
                self.queue_editor_subscription = None;
            }
            if self.managed_queue {
                cx.emit(ComposerEvent::QueueRemoved(removed.id));
            }
            if editing && !self.busy {
                self.submit_next_queued(cx);
            }
            cx.notify();
        }
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        self.editor
            .update(cx, |editor, cx| editor.set_theme(theme, cx));
        if let Some((_, editor)) = &self.queue_editor {
            editor.update(cx, |editor, cx| editor.set_theme(theme, cx));
        }
        let p = theme.palette();
        let dark = theme.is_dark();
        let sendable = self.sendable(cx);

        let chip_button = |chip: &ComposerChip, cx: &mut Context<Self>| {
            let selection = self
                .chip_values
                .get(&chip.id)
                .cloned()
                .or_else(|| chip.default_selected.clone())
                .or_else(|| chip.options.first().map(|option| option.value.clone()));
            let selected_label = selection.as_ref().and_then(|value| {
                chip.options
                    .iter()
                    .find(|option| &option.value == value)
                    .map(|option| option.label.clone())
            });
            let is_menu_open = self.open_chip.as_ref() == Some(&chip.id);
            let tint = chip.tint.map(|t| theme.color(t));
            let mut chip_element = div()
                .id(SharedString::from(format!(
                    "glass-composer-chip-{}",
                    chip.id
                )))
                .debug_selector({
                    let id = chip.id.clone();
                    move || format!("composer-chip:{id}")
                })
                .relative()
                .h(px(26.))
                .px(px(8.))
                .min_w_0()
                .max_w(px(320.))
                .flex_shrink_1()
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(px(7.))
                .text_size(px(12.))
                .text_color(rgb(tint.unwrap_or(p.text_muted)))
                .cursor_pointer()
                .hover(move |style| {
                    style.bg(if dark {
                        rgba(0xffffff14)
                    } else {
                        rgb(p.surface_subtle)
                    })
                })
                .on_click({
                    let id = chip.id.clone();
                    cx.listener(move |this, _, _, cx| {
                        let opens_context = this.context_trigger.as_ref() == Some(&id)
                            && !this.context_sections.is_empty();
                        let has_options = this
                            .chips
                            .iter()
                            .chain(this.menu_chips.iter())
                            .find(|c| c.id == id)
                            .is_some_and(|c| !c.options.is_empty());
                        if opens_context {
                            if this.context_open {
                                this.context_open = false;
                            } else {
                                this.open_context_panel(cx);
                            }
                        } else if has_options {
                            if this.open_chip.as_ref() == Some(&id) {
                                this.open_chip = None;
                            } else {
                                this.open_chip = Some(id.clone());
                                this.context_open = false;
                            }
                        } else {
                            cx.emit(ComposerEvent::ChipClicked(id.clone()));
                        }
                        cx.notify();
                    })
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    // Keep panel / menu outside-click handlers from seeing the
                    // trigger click, which would immediately close again.
                    cx.stop_propagation();
                });
            if let Some(avatar) = selection
                .as_ref()
                .and_then(|value| chip.option_avatars.get(value))
            {
                chip_element = chip_element.child(avatar.clone().size(18.).theme(theme).render());
            } else if let Some(icon) = chip.icon {
                chip_element = chip_element.child(icon.render(14.).flex_shrink_0());
            }
            let chip_label = selected_label.or_else(|| chip.label.clone());
            if let Some(label) = chip_label {
                chip_element = chip_element.child(div().min_w_0().text_ellipsis().child(label));
            }
            if chip.chevron {
                chip_element = chip_element.child(Icon::ChevronDown.render(11.).flex_shrink_0());
            }
            if is_menu_open {
                chip_element = chip_element.child(self.chip_menu(chip, selection, window, cx));
            }
            chip_element
        };

        let stopping = self.busy && self.stop_on_busy && (!self.managed_queue || !sendable);
        let send_button = div()
            .id("glass-composer-send")
            .size(px(28.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(if stopping { 14. } else { 8. }))
            .cursor_pointer()
            .bg(rgb(if stopping {
                p.text
            } else if sendable {
                p.accent
            } else {
                p.surface_subtle
            }))
            .text_color(rgb(if sendable { 0xffffff } else { p.text_muted }))
            .when(sendable, |button| {
                button.hover(move |style| style.opacity(0.88))
            })
            .debug_selector(|| "composer-send-or-stop".into())
            .on_click(cx.listener(|this, _, _, cx| {
                if this.busy && this.stop_on_busy && (!this.managed_queue || !this.sendable(cx)) {
                    cx.emit(ComposerEvent::StopRequested);
                } else {
                    this.submit(cx);
                }
            }))
            .child(if stopping {
                div()
                    .size(px(9.))
                    .rounded(px(1.))
                    .bg(rgb(p.surface))
                    .into_any_element()
            } else {
                Icon::ArrowUp.render(16.).into_any_element()
            });

        let mut bar = div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(1.5))
            .px(px(10.))
            .pb(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(1.5))
                    .children(self.chips.iter().map(|chip| chip_button(chip, cx))),
            )
            .child(div().flex_1());
        if self.busy {
            bar = bar.child(div().flex_shrink_0().text_color(rgb(p.text_muted)).child(
                Icon::Spinner.render(14.).with_animation(
                    "glass-composer-spinner",
                    Animation::new(std::time::Duration::from_millis(900)).repeat(),
                    |spinner, delta| {
                        spinner.with_transformation(gpui::Transformation::rotate(gpui::radians(
                            std::f32::consts::TAU * delta,
                        )))
                    },
                ),
            ));
        }
        if let Some(usage) = self.context_usage {
            let ring = div()
                .id("composer-context-usage")
                .debug_selector(|| "composer-context-usage".into())
                .size(px(28.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    gpui::canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _| {
                            let center = bounds.center();
                            let radius = f32::from(bounds.size.width) / 2. - 1.5;
                            for (fraction, color) in [(1., p.border), (usage.ratio(), p.text_muted)]
                            {
                                if fraction <= 0. {
                                    continue;
                                }
                                let steps = (64. * fraction).ceil().max(1.) as usize;
                                let mut path = gpui::PathBuilder::stroke(px(2.5));
                                for step in 0..=steps {
                                    let angle = -std::f32::consts::FRAC_PI_2
                                        + std::f32::consts::TAU * fraction * step as f32
                                            / steps as f32;
                                    let point = point(
                                        center.x + px(radius * angle.cos()),
                                        center.y + px(radius * angle.sin()),
                                    );
                                    if step == 0 {
                                        path.move_to(point);
                                    } else {
                                        path.line_to(point);
                                    }
                                }
                                if let Ok(path) = path.build() {
                                    window.paint_path(path, rgb(color));
                                }
                            }
                        },
                    )
                    .size(px(16.)),
                );
            bar = bar.child(
                super::Tooltip::new(ring, usage.label())
                    .theme(theme)
                    .render(cx),
            );
        }
        if self.managed_queue && self.busy && self.stop_on_busy && sendable {
            bar = bar.child(
                div()
                    .id("composer-stop-current")
                    .debug_selector(|| "composer-stop-current".into())
                    .size(px(28.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(14.))
                    .bg(rgb(p.text))
                    .cursor_pointer()
                    .child(div().size(px(9.)).rounded(px(1.)).bg(rgb(p.surface)))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ComposerEvent::StopRequested))),
            );
        }
        bar = bar
            .children(
                self.menu_chips
                    .iter()
                    .filter(|chip| !self.hidden_menu_chips.contains(&chip.id))
                    .map(|chip| chip_button(chip, cx)),
            )
            .child(send_button);

        let editor_scroll = div()
            .id("glass-composer-editor-scroll")
            .flex_1()
            .min_h_0()
            .max_h(px(8. * f32::from(self.editor.read(cx).line_height())))
            .overflow_y_scroll()
            .px(px(13.))
            .pt(px(11.))
            .pb(px(4.))
            .child(self.editor.clone());

        let mut card = div()
            .relative()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .rounded(px(14.))
            .border_1()
            .border_color(if dark {
                rgba(theme.glass_line())
            } else {
                rgb(p.border)
            })
            .bg(if dark {
                rgba(theme.menu_material())
            } else {
                rgb(p.surface)
            })
            .when(dark, |card| card.backdrop_blur(px(48.)))
            .shadow(vec![BoxShadow {
                inset: false,
                color: gpui::rgb_to_hsla(rgba(0x0000002e)),
                offset: point(px(0.), px(6.)),
                blur_radius: px(20.),
                spread_radius: px(-6.),
            }]);
        let mut todo_panel = None;
        if self.context_open {
            card = card.child(self.context_panel_element(cx));
        } else if !self.todos.is_empty() {
            let weak = cx.entity().downgrade();
            let panel = super::TodoList::new(self.todos.clone())
                .expanded(self.todos_expanded)
                .theme(theme)
                .on_toggle(move |_, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.todos_expanded = !this.todos_expanded;
                        cx.notify();
                    });
                })
                .render();
            todo_panel = Some(
                deferred(
                    div()
                        .id("glass-composer-todo-panel")
                        .debug_selector(|| "glass-composer-todo-panel".into())
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom(relative(1.))
                        .mb(px(8.))
                        .shadow_lg()
                        .occlude()
                        .child(panel),
                )
                .into_any_element(),
            );
        }
        let card = card
            .when(!self.attachments.is_empty(), |card| {
                card.child(self.attachments_element(cx))
            })
            .child(editor_scroll)
            .child(bar);

        let root = div()
            .id("glass-composer")
            .relative()
            .key_context("GlassComposer")
            .track_focus(&self.focus)
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(6.))
            .on_action(cx.listener(|this, _: &Submit, window, cx| {
                if this.context_open {
                    if !this.editor.read(cx).is_composing() { this.accept_context_selection(window, cx); }
                    cx.stop_propagation();
                    return;
                }
                if this
                    .queue_editor
                    .as_ref()
                    .is_some_and(|(_, editor)| editor.focus_handle(cx).is_focused(window))
                {
                    this.save_queue_edit(cx);
                } else {
                    this.submit(cx);
                }
            }))
            .capture_action(cx.listener(|this, _: &super::prompt_editor::Up, _, cx| {
                if this.context_open && !this.editor.read(cx).is_composing() { this.move_context_selection(true, cx); }
                else { cx.propagate(); }
            }))
            .capture_action(cx.listener(|this, _: &super::prompt_editor::Down, _, cx| {
                if this.context_open && !this.editor.read(cx).is_composing() { this.move_context_selection(false, cx); }
                else { cx.propagate(); }
            }))
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    if this
                        .queue_editor
                        .as_ref()
                        .is_some_and(|(_, editor)| editor.focus_handle(cx).is_focused(window))
                    {
                        this.cancel_queue_edit(cx);
                        window.focus(&this.editor.focus_handle(cx), cx);
                        cx.stop_propagation();
                        return;
                    }
                    let mut closed = this.context_open;
                    this.context_open = false;
                    if this.open_chip.take().is_some() {
                        closed = true;
                    }
                    if closed {
                        cx.notify();
                    }
                }
            }));

        let mut queue_rows = div()
            .id("composer-queue-list")
            .max_h(px(180.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(6.));
        let mut has_queue = false;
        for (index, submission) in self.queue.iter().enumerate() {
            if submission.scope != self.queue_scope {
                continue;
            }
            has_queue = true;
            let row = div()
                .w_full()
                .flex()
                .items_center()
                .gap(px(9.))
                .px(px(12.))
                .py(px(11.))
                .rounded(px(14.))
                .bg(if dark { rgba(0xffffff0a) } else { rgb(p.panel) })
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(rgb(p.text_muted))
                        .child(Icon::Grip.render(14.)),
                )
                .child(
                    if let Some((_, editor)) = self
                        .queue_editor
                        .as_ref()
                        .filter(|(id, _)| *id == submission.id)
                    {
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .id(("queue-editor", submission.id))
                                    .max_h(px(140.))
                                    .overflow_y_scroll()
                                    .child(editor.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        super::Button::new("保存到队列")
                                            .size(super::ButtonSize::Mini)
                                            .theme(theme)
                                            .render()
                                            .debug_selector(|| "composer-queue-edit-save".into())
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.save_queue_edit(cx)
                                            })),
                                    )
                                    .child(
                                        super::Button::new("取消编辑")
                                            .size(super::ButtonSize::Mini)
                                            .kind(super::ButtonKind::Plain)
                                            .theme(theme)
                                            .render()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.cancel_queue_edit(cx)
                                            })),
                                    ),
                            )
                    } else {
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(rgb(p.text))
                            .child(if let Some(label) = &submission.label {
                                format!("排队 {} · {label} · {}", index + 1, submission.text)
                            } else {
                                submission.text.to_string()
                            })
                    },
                )
                .when(!self.managed_queue, |row| {
                    row.child(
                        div()
                            .id(SharedString::from(format!(
                                "glass-composer-queue-now-{index}"
                            )))
                            .h(px(26.))
                            .px(px(8.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .rounded(px(7.))
                            .text_size(px(12.))
                            .text_color(rgb(p.text_muted))
                            .cursor_pointer()
                            .hover(move |style| {
                                style.bg(if dark {
                                    rgba(0xffffff14)
                                } else {
                                    rgb(p.surface_subtle)
                                })
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.queue_submit_now(index, cx);
                            }))
                            .child(Icon::ArrowUp.render(12.))
                            .child("立即"),
                    )
                })
                .when(self.queue_editor.is_none(), |row| {
                    row.child(
                        icon_action(
                            SharedString::from(format!("glass-composer-queue-edit-{index}")),
                            Icon::Pencil,
                            p,
                            dark,
                        )
                        .debug_selector(move || format!("composer-queue-edit-{}", submission.id))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.queue_edit(index, cx);
                                if let Some((_, editor)) = &this.queue_editor {
                                    window.focus(&editor.focus_handle(cx), cx);
                                }
                            },
                        )),
                    )
                })
                .when(
                    self.managed_queue
                        && self.busy
                        && self
                            .queue_editor
                            .as_ref()
                            .is_none_or(|(id, _)| *id != submission.id),
                    |row| {
                        let id = submission.id;
                        let submission = submission.submission.clone();
                        row.child(
                            div()
                                .id(SharedString::from(format!("composer-queue-steer-{id}")))
                                .debug_selector(move || format!("composer-queue-steer-{id}"))
                                .flex()
                                .items_center()
                                .gap_1()
                                .px_2()
                                .py_1()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .text_color(rgb(p.accent))
                                .hover(|style| style.bg(rgb(p.surface_subtle)))
                                .child(Icon::Workflow.render(14.))
                                .child("引导")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(ComposerEvent::QueueSteerRequested {
                                        id,
                                        submission: submission.clone(),
                                    })
                                })),
                        )
                    },
                )
                .child(
                    icon_action(
                        SharedString::from(format!("glass-composer-queue-remove-{index}")),
                        Icon::Trash,
                        p,
                        dark,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.queue_remove(index, cx);
                    })),
                );
            queue_rows = queue_rows.child(row);
        }

        root.when(has_queue, |root| root.child(queue_rows))
            .child(card)
            .children(todo_panel)
    }
}

impl Composer {
    /// Builds the option menu for a select-style chip, anchored above it.
    fn chip_menu(
        &self,
        chip: &ComposerChip,
        selection: Option<SharedString>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = self.theme;
        let p = theme.palette();
        let dark = theme.is_dark();
        let style = window.text_style();
        let label_width = chip
            .options
            .iter()
            .map(|option| {
                let run = gpui::TextRun {
                    len: option.label.len(),
                    font: style.font(),
                    color: style.color,
                    letter_spacing: None,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                f32::from(
                    window
                        .text_system()
                        .shape_line(option.label.clone(), px(13.), &[run], None)
                        .width,
                )
            })
            .fold(0_f32, f32::max);
        let avatar_width = if chip.option_avatars.is_empty() {
            0.
        } else {
            28.
        };
        let menu_width = (label_width + 64. + avatar_width)
            .clamp(168., 360.)
            .min((f32::from(window.viewport_size().width) - 32.).max(120.));
        let rows = chip.options.iter().enumerate().map(|(index, option)| {
            let is_selected = selection.as_ref() == Some(&option.value);
            div()
                .id(SharedString::from(format!(
                    "glass-composer-option-{}-{index}",
                    chip.id
                )))
                .debug_selector({
                    let id = chip.id.clone();
                    let value = option.value.clone();
                    move || format!("composer-option:{id}:{value}")
                })
                .h(px(30.))
                .flex_shrink_0()
                .w_full()
                .min_w_0()
                .px(px(9.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(6.))
                .text_size(px(13.))
                .text_color(rgb(if is_selected { p.accent } else { p.text }))
                .cursor_pointer()
                .hover(move |style| style.bg(rgb(p.accent)).text_color(rgb(0xffffff)))
                .on_click({
                    let chip_id = chip.id.clone();
                    let value = option.value.clone();
                    cx.listener(move |this, _, _, cx| {
                        this.chip_values.insert(chip_id.clone(), value.clone());
                        this.open_chip = None;
                        cx.emit(ComposerEvent::ChipSelected {
                            chip: chip_id.clone(),
                            value: value.clone(),
                        });
                        cx.notify();
                    })
                })
                .when(!chip.option_avatars.is_empty(), |row| {
                    let leading = if let Some(avatar) = chip.option_avatars.get(&option.value) {
                        avatar.clone().size(18.).theme(theme).render()
                    } else {
                        div()
                            .size(px(18.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .when_some(chip.icon, |slot, icon| slot.child(icon.render(14.)))
                    };
                    row.child(leading)
                })
                .child(
                    div()
                        .id(("composer-option-label", index))
                        .debug_selector(move || format!("composer-option-label-{index}").into())
                        .flex_1()
                        .min_w_0()
                        .text_ellipsis()
                        .child(option.label.clone()),
                )
                .child(
                    div()
                        .id(("composer-option-check", index))
                        .debug_selector(move || format!("composer-option-check-{index}").into())
                        .w(px(14.))
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .child(if is_selected { "✓" } else { "" }),
                )
        });
        deferred(
            div()
                .absolute()
                .bottom(px(32.))
                .right_0()
                .id("glass-composer-chip-menu")
                .debug_selector(|| "composer-chip-menu".into())
                .w(px(menu_width))
                .max_h(px(280.))
                .overflow_y_scroll()
                .p(px(6.))
                .flex()
                .flex_col()
                .gap(px(1.))
                .rounded(px(10.))
                .border_1()
                .border_color(if dark {
                    rgba(theme.glass_line())
                } else {
                    rgb(p.border)
                })
                .bg(if dark {
                    rgba(theme.menu_material())
                } else {
                    rgb(p.surface)
                })
                .shadow_lg()
                .occlude()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if this.open_chip.take().is_some() {
                        cx.notify();
                    }
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .children(rows),
        )
        .with_priority(20)
        .into_any_element()
    }

    /// Thumbnail strip of pasted images above the input; each thumb can be
    /// removed before submitting.
    fn attachments_element(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = self.theme;
        let p = theme.palette();
        let dark = theme.is_dark();
        let thumbs = self.attachments.iter().enumerate().map(|(index, image)| {
            div()
                .id(SharedString::from(format!(
                    "glass-composer-attachment-{index}"
                )))
                .relative()
                .size(px(48.))
                .flex_shrink_0()
                .rounded(px(8.))
                .border_1()
                .border_color(if dark {
                    rgba(theme.glass_line())
                } else {
                    rgb(p.border)
                })
                .overflow_hidden()
                .child(
                    img(image.source())
                        .size(px(48.))
                        .object_fit(ObjectFit::Cover),
                )
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "glass-composer-attachment-remove-{index}"
                        )))
                        .absolute()
                        .top(px(2.))
                        .right(px(2.))
                        .size(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .bg(rgba(0x000000aa))
                        .text_color(rgb(0xffffff))
                        .cursor_pointer()
                        .hover(move |style| style.bg(rgb(p.accent)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if index < this.attachments.len() {
                                this.attachments.remove(index).source().remove_asset(cx);
                                cx.emit(ComposerEvent::AttachmentsChanged);
                                cx.notify();
                            }
                        }))
                        .child(Icon::Close.render(10.)),
                )
        });
        div()
            .id("glass-composer-attachments")
            .w_full()
            .min_w_0()
            .px(px(13.))
            .pt(px(10.))
            .flex()
            .gap(px(8.))
            .overflow_x_scroll()
            .children(thumbs)
            .into_any_element()
    }

    fn matching_context_entries(&self, cx: &App) -> Vec<(ComposerContextEntry, usize)> {
        let query = self.context_filter(cx).replace('\\', "/");
        let mut rows = Vec::new();
        let mut child = 1; // Panel title.
        for section in &self.context_sections {
            let entries: Vec<_> = section.entries.iter().filter(|entry| Self::context_entry_matches(entry, &query)).collect();
            if entries.is_empty() { continue; }
            child += usize::from(section.title.is_some());
            for entry in entries { rows.push((entry.clone(), child)); child += 1; }
        }
        rows
    }

    fn context_entry_matches(entry: &ComposerContextEntry, query: &str) -> bool {
        query.is_empty() || entry.label.to_lowercase().replace('\\', "/").contains(query)
            || entry.description.as_ref().is_some_and(|text| text.to_lowercase().replace('\\', "/").contains(query))
    }

    fn move_context_selection(&mut self, up: bool, cx: &mut Context<Self>) {
        let entries = self.matching_context_entries(cx);
        if entries.is_empty() { return; }
        let current = self.context_selected.min(entries.len() - 1);
        self.context_selected = if up { (current + entries.len() - 1) % entries.len() } else { (current + 1) % entries.len() };
        self.context_scroll.scroll_to_item(entries[self.context_selected].1);
        cx.notify();
    }

    fn select_context_entry(&mut self, entry: ComposerContextEntry, window: &mut Window, cx: &mut Context<Self>) {
        self.close_context_panel(cx);
        window.focus(&self.editor.focus_handle(cx), cx);
        if entry.insert_mention { self.add_mention(entry.label.clone(), entry.color.unwrap_or(self.theme.palette().accent), cx); }
        cx.emit(ComposerEvent::ContextSelected { id: entry.id, label: entry.label });
        cx.notify();
    }

    fn accept_context_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.matching_context_entries(cx);
        if !entries.is_empty() {
            let entry = entries[self.context_selected.min(entries.len() - 1)].0.clone();
            self.select_context_entry(entry, window, cx);
        }
    }

    /// Builds the context panel (添加 / at-mention panel) anchored above the card.
    fn context_panel_element(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = self.theme;
        let p = theme.palette();
        let dark = theme.is_dark();
        let surface_bg = if dark {
            rgba(theme.menu_material())
        } else {
            rgb(p.surface)
        };
        let surface_border = if dark {
            rgba(theme.glass_line())
        } else {
            rgb(p.border)
        };
        let hover_bg = if dark {
            rgba(0xffffff14)
        } else {
            rgb(p.surface_subtle)
        };

        let mut panel = div()
            .id("glass-composer-context-panel-scroll")
            .track_scroll(&self.context_scroll)
            .absolute()
            .left_0()
            .right_0()
            .bottom(relative(1.))
            .mb(px(8.))
            .max_h(px(420.))
            .overflow_y_scroll()
            .p(px(10.))
            .flex()
            .flex_col()
            .rounded(px(14.))
            .border_1()
            .border_color(surface_border)
            .bg(surface_bg)
            .shadow_lg()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.close_context_panel(cx);
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px(px(10.))
                    .pb(px(8.))
                    .text_size(px(15.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(p.text))
                    .child(self.context_title.clone()),
            );

        let query = self
            .context_query(cx)
            .map(|(_, query)| query)
            .unwrap_or_default();
        let match_query = query.replace('\\', "/");
        let mut matches = 0;
        let selected_index = self.context_selected.min(self.matching_context_entries(cx).len().saturating_sub(1));
        for section in &self.context_sections {
            let entries: Vec<_> = section
                .entries
                .iter()
                .filter(|entry| Self::context_entry_matches(entry, &match_query))
                .collect();
            if entries.is_empty() {
                continue;
            }
            let start_index = matches;
            matches += entries.len();
            if let Some(title) = &section.title {
                panel = panel.child(
                    div()
                        .px(px(10.))
                        .pt(px(6.))
                        .pb(px(4.))
                        .text_size(px(12.))
                        .text_color(rgb(p.text_muted))
                        .child(title.clone()),
                );
            }
            for (index, entry) in entries.into_iter().enumerate() {
                let selected = start_index + index == selected_index;
                let entry_color = theme.color(entry.color.unwrap_or(p.accent));
                panel = panel.child(
                    div()
                        .id(SharedString::from(format!(
                            "glass-composer-context-{}",
                            entry.id
                        )))
                        .debug_selector({
                            let id = entry.id.clone();
                            move || format!("composer-context:{id}")
                        })
                        .h(px(40.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .rounded(px(8.))
                        .cursor_pointer()
                        .when(selected, |row| row.bg(hover_bg))
                        .hover(move |style| style.bg(hover_bg))
                        .on_click({
                            let entry = entry.clone();
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.select_context_entry(entry.clone(), window, cx);
                            })
                        })
                        .when_some(entry.icon, |row, icon| {
                            row.child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(rgb(entry_color))
                                    .child(icon.render(16.)),
                            )
                        })
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(14.))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(rgb(p.text))
                                .child(entry.label.clone()),
                        )
                        .when_some(entry.description.clone(), |row, description| {
                            row.child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(rgb(p.text_muted))
                                    .child(description),
                            )
                        }),
                );
            }
        }

        if matches == 0 {
            panel = panel.child(
                div()
                    .p_3()
                    .text_color(rgb(p.text_muted))
                    .debug_selector(|| "composer-context-empty".into())
                    .child(format!("没有匹配“{query}”的上下文")),
            );
        }
        if !self.context_hints.is_empty() {
            panel = panel.child(
                div()
                    .mt(px(8.))
                    .pt(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .children(self.context_hints.iter().map(|(key, label)| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .child(
                                div()
                                    .h(px(20.))
                                    .px(px(6.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(5.))
                                    .border_1()
                                    .border_color(surface_border)
                                    .text_size(px(11.))
                                    .text_color(rgb(p.text_muted))
                                    .child(key.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(p.text_muted))
                                    .child(label.clone()),
                            )
                    })),
            );
        }

        deferred(panel).with_priority(20).into_any_element()
    }
}

fn icon_action(
    id: SharedString,
    icon: Icon,
    p: crate::theme::ThemePalette,
    dark: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .size(px(26.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .text_color(rgb(p.text_muted))
        .cursor_pointer()
        .hover(move |style| {
            style
                .bg(if dark {
                    rgba(0xffffff14)
                } else {
                    rgb(p.surface_subtle)
                })
                .text_color(rgb(p.text))
        })
        .child(icon.render(14.))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn pasted_text_survives_draft_restore_and_managed_queue_edit(cx: &mut gpui::TestAppContext) {
        cx.update(super::super::bind_prompt_editor_keys);
        let (composer, window) =
            cx.add_window_view(|_, cx| Composer::new("Message", cx).managed_queue(true));
        let content = "完整原文 including ``` fences\n".repeat(25);
        window.update(|window, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(content.clone()));
            window.focus(&composer.focus_handle(cx), cx);
        });
        window.simulate_keystrokes("ctrl-v");
        window.run_until_parked();
        composer.update(window, |composer, cx| {
            assert_eq!(composer.references.len(), 1);
            let draft = composer.value(cx).to_owned();
            let refs = composer.references.clone();
            assert!(!draft.contains("完整原文"));
            composer.set_draft(draft, Vec::new(), cx);
            composer.set_references(refs, cx);
            composer.set_busy(true, cx);
            composer.submit(cx);
            assert!(
                composer.queue[0]
                    .submission
                    .prompt_text()
                    .contains(&content)
            );
            composer.set_value("unsent draft", cx);
            composer.queue_edit(0, cx);
        });
        window.update(|window, cx| {
            let editor = composer.read(cx).queue_editor.as_ref().unwrap().1.clone();
            window.focus(&editor.focus_handle(cx), cx);
            cx.write_to_clipboard(gpui::ClipboardItem::new_string("第二份原文\n".repeat(25)));
        });
        window.simulate_keystrokes("ctrl-v");
        window.run_until_parked();
        composer.update(window, |composer, cx| {
            composer.save_queue_edit(cx);
            assert_eq!(composer.queue[0].submission.references.len(), 2);
            let prompt = composer.queue[0].submission.prompt_text();
            assert!(prompt.contains(&content));
            assert!(prompt.contains(&"第二份原文\n".repeat(25)));
            assert_eq!(composer.value(cx), "unsent draft");
            composer.queue_edit(0, cx);
            composer
                .queue_editor
                .as_ref()
                .unwrap()
                .1
                .update(cx, |editor, cx| editor.set_value("remove references", cx));
            composer.save_queue_edit(cx);
            assert!(composer.queue[0].submission.references.is_empty());
        });
    }
    #[gpui::test]
    fn managed_queue_edit_preserves_draft_attachments_and_dispatch_identity(
        cx: &mut gpui::TestAppContext,
    ) {
        use std::{cell::RefCell, rc::Rc};
        let (composer, window) =
            cx.add_window_view(|_, cx| Composer::new("Message", cx).managed_queue(true));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let events = sent.clone();
        let _subscription = window.update(|_, cx| {
            cx.subscribe(&composer, move |_, event, _| {
                if let ComposerEvent::QueueSubmitted { id, submission } = event {
                    events.borrow_mut().push((*id, submission.clone()));
                }
            })
        });
        let image = PastedImage::new(gpui::ImageFormat::Png, vec![1, 2, 3]);
        let mut reference = super::super::ComposerReference::file("/project/main.rs", false);
        reference.token = "@main.rs".into();
        composer.update(window, |composer, cx| {
            composer.set_busy(true, cx);
            composer.set_draft(
                format!("original {}", reference.token),
                vec![image.clone()],
                cx,
            );
            composer.set_references(vec![reference.clone()], cx);
            composer.submit(cx);
            composer.set_value("keep my unsent draft", cx);
        });
        window.run_until_parked();
        let edit = window.debug_bounds("composer-queue-edit-1").unwrap();
        window.simulate_click(edit.center(), Default::default());
        composer.update(window, |composer, cx| {
            assert_eq!(composer.queue_len(), 1);
            assert_eq!(composer.value(cx), "keep my unsent draft");
            let editor = composer.queue_editor.as_ref().unwrap().1.clone();
            editor.update(cx, |editor, cx| {
                editor.set_value(format!("edited {}", reference.token), cx)
            });
            composer.set_busy(false, cx);
            assert_eq!(composer.queue_len(), 1, "editing must hold the FIFO head");
        });
        let save = window.debug_bounds("composer-queue-edit-save").unwrap();
        window.simulate_click(save.center(), Default::default());
        window.run_until_parked();
        assert_eq!(sent.borrow().len(), 1);
        let sent = sent.borrow();
        assert_eq!(sent[0].0, 1);
        assert_eq!(
            sent[0].1.text.as_ref(),
            format!("edited {}", reference.token)
        );
        assert_eq!(sent[0].1.images, vec![image]);
        assert_eq!(sent[0].1.references[0].path, reference.path);
        composer.read_with(window, |composer, cx| {
            assert_eq!(composer.value(cx), "keep my unsent draft");
            assert!(composer.queue_editor.is_none());
        });
    }

    #[gpui::test]
    fn cancelling_managed_queue_edit_keeps_original_text_and_order(cx: &mut gpui::TestAppContext) {
        let (composer, window) =
            cx.add_window_view(|_, cx| Composer::new("Message", cx).managed_queue(true));
        composer.update(window, |composer, cx| {
            composer.set_busy(true, cx);
            for text in ["first", "second"] {
                composer.set_value(text, cx);
                composer.submit(cx);
            }
            composer.queue_edit(0, cx);
            composer
                .queue_editor
                .as_ref()
                .unwrap()
                .1
                .update(cx, |editor, cx| editor.set_value("unsaved", cx));
            composer.cancel_queue_edit(cx);
            assert_eq!(composer.queue[0].text.as_ref(), "first");
            assert_eq!(composer.queue[1].text.as_ref(), "second");
            assert_eq!(composer.queue[0].id, 1);
            assert!(composer.queue_editor.is_none());
        });
    }

    #[gpui::test]
    fn structured_references_stay_compact_restore_and_submit_with_exact_context(
        cx: &mut gpui::TestAppContext,
    ) {
        let composer = cx.new(|cx| Composer::new("Message", cx));
        composer.update(cx, |c, cx| {
            c.set_value("解释", cx);
            c.add_reference(
                super::super::ComposerReference::selection(
                    "/project/a/源.rs",
                    2,
                    3,
                    "short selection",
                ),
                cx,
            );
            c.add_reference(
                super::super::ComposerReference::selection(
                    "/project/b/源.rs",
                    2,
                    3,
                    "different selection",
                ),
                cx,
            );
            c.add_reference(
                super::super::ComposerReference::selection(
                    "/project/long.rs",
                    1,
                    200,
                    &"long text\n".repeat(200),
                ),
                cx,
            );
            let text = c.value(cx);
            let references = c.references(cx);
            assert!(text.len() < 160);
            assert!(!text.contains("short selection"));
            assert_eq!(references.len(), 3);
            assert_ne!(references[0].token, references[1].token);
            c.set_draft(text.clone(), Vec::new(), cx);
            c.set_references(references.clone(), cx);
            assert_eq!(c.references(cx), references);
            let submission = c.submission(text, cx);
            let prompt = submission.prompt_text();
            assert!(prompt.contains("/project/a/源.rs，行范围 2-3"));
            assert!(prompt.contains("short selection"));
            assert!(!prompt.contains("long text"));
            assert!(c.references(cx).is_empty());
            c.context_on_at = true;
            c.set_draft("查看 @main.rs", Vec::new(), cx);
            c.add_reference(
                super::super::ComposerReference::file("/project/main.rs", false),
                cx,
            );
            assert_eq!(c.value(cx).as_ref(), "查看 @main.rs ");
            assert!(!c.context_panel_open());
            let mut removed = submission.clone();
            removed.text = "删除全部引用".into();
            assert_eq!(removed.prompt_text(), "删除全部引用");
        });
    }

    #[gpui::test]
    fn managed_queue_sends_fifo_keeps_drafts_and_requires_explicit_steering_acceptance(
        cx: &mut gpui::TestAppContext,
    ) {
        use std::{cell::RefCell, rc::Rc};
        struct Preview(Entity<Composer>);
        impl Render for Preview {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .child(self.0.clone())
            }
        }
        let (preview, window) = cx.add_window_view(|_, cx| {
            Preview(cx.new(|cx| {
                Composer::new("Message", cx)
                    .clear_on_submit(false)
                    .stop_on_busy(true)
                    .managed_queue(true)
            }))
        });
        let composer = preview.read_with(window, |view, _| view.0.clone());
        let dispatched = Rc::new(RefCell::new(Vec::new()));
        let steering = Rc::new(RefCell::new(Vec::new()));
        let dispatch_events = dispatched.clone();
        let steer_events = steering.clone();
        let _subscription = window.update(|_, cx| {
            cx.subscribe(&composer, move |_, event, _| {
                if let ComposerEvent::QueueSubmitted { id, submission } = event {
                    dispatch_events
                        .borrow_mut()
                        .push((*id, submission.text.to_string()));
                }
                if let ComposerEvent::QueueSteerRequested { id, .. } = event {
                    steer_events.borrow_mut().push(*id);
                }
            })
        });
        composer.update(window, |composer, cx| {
            composer.set_busy(true, cx);
            composer.set_todos(
                vec![super::super::TodoListItem {
                    content: "Active task".into(),
                    status: super::super::TodoListStatus::InProgress,
                }],
                cx,
            );
            composer.set_value("first", cx);
        });
        assert!(window.debug_bounds("composer-stop-current").is_some());
        let send = window.debug_bounds("composer-send-or-stop").unwrap();
        window.simulate_click(send.center(), gpui::Modifiers::default());
        composer.update(window, |composer, cx| {
            assert_eq!(composer.queue_len(), 1);
            assert!(composer.value(cx).is_empty());
            composer.set_value("second", cx);
            composer.submit(cx);
        });
        let (first, second) = composer.read_with(window, |composer, _| {
            (composer.queue[0].id, composer.queue[1].id)
        });
        assert_eq!(second, 2);
        let button = window.debug_bounds("composer-queue-steer-2").unwrap();
        let todo = window.debug_bounds("glass-composer-todo-panel").unwrap();
        assert!(
            todo.bottom() < button.top(),
            "todo must not cover queued prompt controls"
        );
        window.simulate_click(button.center(), gpui::Modifiers::default());
        assert_eq!(*steering.borrow(), vec![second]);
        composer.read_with(window, |composer, _| assert_eq!(composer.queue_len(), 2));
        composer.update(window, |composer, cx| {
            composer.set_value("keep this unsent draft", cx);
            composer.set_busy(false, cx);
        });
        assert_eq!(*dispatched.borrow(), vec![(first, "first".to_string())]);
        composer.update(window, |composer, cx| {
            composer.set_busy(true, cx);
            composer.set_busy(false, cx);
        });
        assert_eq!(
            *dispatched.borrow(),
            vec![(first, "first".to_string()), (second, "second".to_string())]
        );
        composer.read_with(window, |composer, cx| {
            assert_eq!(composer.queue_len(), 0);
            assert_eq!(composer.value(cx).as_ref(), "keep this unsent draft");
        });
    }

    #[gpui::test]
    fn managed_queue_offers_steering_for_image_submissions(cx: &mut gpui::TestAppContext) {
        use std::{cell::RefCell, rc::Rc};
        let (composer, window) =
            cx.add_window_view(|_, cx| Composer::new("Message", cx).managed_queue(true));
        let steered = Rc::new(RefCell::new(Vec::new()));
        let events = steered.clone();
        let _subscription = window.update(|_, cx| {
            cx.subscribe(&composer, move |_, event, _| {
                if let ComposerEvent::QueueSteerRequested { id, submission } = event {
                    events.borrow_mut().push((
                        *id,
                        submission.text.to_string(),
                        submission.images.len(),
                    ));
                }
            })
        });
        let image = PastedImage::new(gpui::ImageFormat::Png, vec![1, 2, 3]);
        composer.update(window, |composer, cx| {
            composer.set_busy(true, cx);
            composer.set_draft("看这张图", vec![image], cx);
        });
        window.run_until_parked();
        composer.update(window, |composer, cx| composer.submit(cx));
        let id = composer.read_with(window, |composer, _| composer.queue[0].id);
        // 带图消息同样出现引导按钮，点击携带原文与图片数量。
        let selector: &'static str =
            Box::leak(format!("composer-queue-steer-{id}").into_boxed_str());
        let button = window
            .debug_bounds(selector)
            .expect("image submissions must expose the steer control");
        window.simulate_click(button.center(), gpui::Modifiers::default());
        assert_eq!(
            *steered.borrow(),
            vec![(id, "看这张图".to_string(), 1usize)]
        );
        composer.read_with(window, |composer, _| assert_eq!(composer.queue_len(), 1));
    }

    #[gpui::test]
    fn todo_panel_sits_above_editor_yields_to_at_panel_and_keeps_draft(
        cx: &mut gpui::TestAppContext,
    ) {
        struct Preview(Entity<Composer>);
        impl Render for Preview {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .p_4()
                    .child(self.0.clone())
            }
        }
        let (preview, window) = cx.add_window_view(|_, cx| {
            Preview(cx.new(|cx| {
                Composer::new("Message", cx).context_panel(
                    "attach",
                    "Context",
                    [ComposerContextSection::untitled([
                        ComposerContextEntry::new("file", "File"),
                    ])],
                )
            }))
        });
        let composer = preview.read_with(window, |preview, _| preview.0.clone());
        composer.update(window, |composer, cx| {
            composer.set_value("keep my draft", cx);
            composer.set_todos(
                vec![super::super::TodoListItem {
                    content: "检查实现".into(),
                    status: super::super::TodoListStatus::InProgress,
                }],
                cx,
            );
        });
        let panel = window.debug_bounds("glass-composer-todo-panel").unwrap();
        let toggle = window.debug_bounds("todo-list-toggle").unwrap();
        assert!(
            panel.bottom() < window.update(|window, _| window.viewport_size()).height - px(40.)
        );
        assert!(window.debug_bounds("todo-list-rows").is_some());
        window.simulate_click(toggle.center(), gpui::Modifiers::default());
        assert!(window.debug_bounds("todo-list-rows").is_none());
        composer.update(window, |composer, cx| composer.open_context_panel(cx));
        assert!(window.debug_bounds("glass-composer-todo-panel").is_none());
        composer.update(window, |composer, cx| composer.close_context_panel(cx));
        assert!(window.debug_bounds("glass-composer-todo-panel").is_some());
        composer.update(window, |composer, cx| {
            composer.set_todos(
                vec![super::super::TodoListItem {
                    content: "检查实现".into(),
                    status: super::super::TodoListStatus::Completed,
                }],
                cx,
            )
        });
        assert!(window.debug_bounds("todo-list-rows").is_none());
        composer.read_with(window, |composer, cx| {
            assert_eq!(composer.value(cx).as_ref(), "keep my draft")
        });
        composer.update(window, |composer, cx| composer.set_todos(Vec::new(), cx));
        assert!(window.debug_bounds("glass-composer-todo-panel").is_none());
    }

    #[test]
    fn context_queries_are_caret_scoped_and_do_not_match_email_addresses() {
        assert_eq!(
            context_query_range("检查 @wps 后续", "检查 @wp".len()),
            Some("检查 ".len().."检查 @wps".len())
        );
        assert_eq!(
            context_query_range("@文件", "@文件".len()),
            Some(0.."@文件".len())
        );
        assert_eq!(context_query_range("email@wps", 9), None);
        assert_eq!(context_query_range("@wps ", 5), None);
        assert_eq!(context_query_range("检查", 1), None);
    }

    #[gpui::test]
    fn context_filter_matches_labels_and_descriptions_and_replaces_middle_query(
        cx: &mut gpui::TestAppContext,
    ) {
        struct Preview(Entity<Composer>);
        impl Render for Preview {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(px(650.)).pt(px(300.)).child(self.0.clone())
            }
        }
        let (preview, window) = cx.add_window_view(|_, cx| {
            Preview(cx.new(|cx| {
                Composer::new("Message", cx)
                    .context_on_at(true)
                    .context_panel(
                        "attach",
                        "Context",
                        [ComposerContextSection::new(
                            "Skills",
                            [
                                ComposerContextEntry::new("wps", "wps365-skills")
                                    .description("Manage documents")
                                    .insert_mention(false),
                                ComposerContextEntry::new("review", "code-review")
                                    .description("Review code")
                                    .insert_mention(false),
                            ],
                        )],
                    )
            }))
        });
        let composer = preview.read_with(window, |preview, _| preview.0.clone());
        composer.update(window, |composer, cx| composer.set_value("@WPS", cx));
        assert!(window.debug_bounds("composer-context:wps").is_some());
        assert!(window.debug_bounds("composer-context:review").is_none());
        composer.update(window, |composer, cx| composer.set_value("@documents", cx));
        assert!(window.debug_bounds("composer-context:wps").is_some());
        composer.update(window, |composer, cx| composer.set_value("@unmatched", cx));
        assert!(window.debug_bounds("composer-context-empty").is_some());
        composer.update(window, |composer, cx| {
            composer.set_value("检查 @wp 后续", cx);
            composer.editor.update(cx, |editor, cx| {
                let caret = "检查 @wp".len();
                editor.replace_range(caret..caret, "", cx);
            });
            composer.add_context_token("$wps365-skills", 0x007aff, cx);
            assert_eq!(composer.value(cx).as_ref(), "检查 $wps365-skills 后续");
            assert_eq!(
                composer.editor.read(cx).selection_range().end,
                "检查 $wps365-skills ".len()
            );
        });
    }

    #[gpui::test]
    fn busy_stop_button_works_with_disabled_editor_and_preserves_draft(
        cx: &mut gpui::TestAppContext,
    ) {
        use std::{cell::Cell, rc::Rc};
        let (composer, window) =
            cx.add_window_view(|_, cx| Composer::new("Message", cx).stop_on_busy(true));
        let stops = Rc::new(Cell::new(0));
        let submits = Rc::new(Cell::new(0));
        let stop_count = stops.clone();
        let submit_count = submits.clone();
        let _subscription = window.update(|_, cx| {
            cx.subscribe(&composer, move |_, event, _| match event {
                ComposerEvent::StopRequested => stop_count.set(stop_count.get() + 1),
                ComposerEvent::Submitted(_) => submit_count.set(submit_count.get() + 1),
                _ => {}
            })
        });
        composer.update(window, |this, cx| {
            this.set_value("keep draft", cx);
            this.set_disabled(true, cx);
            this.set_busy(true, cx);
        });
        let button = window.debug_bounds("composer-send-or-stop").unwrap();
        window.simulate_click(button.center(), gpui::Modifiers::default());
        assert_eq!(stops.get(), 1);
        assert_eq!(submits.get(), 0);
        composer.read_with(window, |this, cx| {
            assert_eq!(this.value(cx).as_ref(), "keep draft")
        });
        composer.update(window, |this, cx| {
            this.set_busy(false, cx);
            this.set_disabled(false, cx);
        });
        let button = window.debug_bounds("composer-send-or-stop").unwrap();
        window.simulate_click(button.center(), gpui::Modifiers::default());
        assert_eq!(stops.get(), 1);
        assert_eq!(submits.get(), 1);
    }

    #[gpui::test]
    fn literal_context_tokens_replace_at_trigger_preserve_draft_and_panel_updates_stay_open(
        cx: &mut gpui::TestAppContext,
    ) {
        let composer = cx.new(|cx| {
            Composer::new("Message", cx)
                .context_panel(
                    "attach",
                    "Context",
                    [ComposerContextSection::untitled([
                        ComposerContextEntry::new("skill:review", "review").insert_mention(false),
                    ])],
                )
                .context_on_at(true)
        });
        composer.update(cx, |composer, cx| {
            composer.set_draft(
                "检查代码 @",
                vec![PastedImage::new(gpui::ImageFormat::Png, vec![1, 2, 3])],
                cx,
            );
            composer.open_context_panel(cx);
            composer.set_context_sections(
                [ComposerContextSection::new(
                    "Skills",
                    [ComposerContextEntry::new("skill:review", "review").insert_mention(false)],
                )],
                cx,
            );
            assert!(composer.context_panel_open());
            composer.add_context_token("$review", 0x007aff, cx);
            assert_eq!(composer.value(cx).as_ref(), "检查代码 $review ");
            assert_eq!(composer.attachments().len(), 1);
            composer.set_value("mail@example.com@", cx);
            composer.add_context_token("$review", 0x007aff, cx);
            assert_eq!(composer.value(cx).as_ref(), "mail@example.com@ $review ");
        });
    }

    #[gpui::test]
    fn action_context_entries_consume_only_the_at_query(cx: &mut gpui::TestAppContext) {
        let composer = cx.new(|cx| Composer::new("Message", cx).context_on_at(true));
        composer.update(cx, |c, cx| {
            c.set_draft(
                "检查代码 @知微",
                vec![PastedImage::new(gpui::ImageFormat::Png, vec![1, 2, 3])],
                cx,
            );
            c.clear_context_query(cx);
            assert_eq!(c.value(cx).as_ref(), "检查代码 ");
            assert_eq!(c.attachments().len(), 1);
            c.set_value("mail@example.com", cx);
            c.clear_context_query(cx);
            assert_eq!(c.value(cx).as_ref(), "mail@example.com");
            c.set_value("没有提及的草稿", cx);
            c.clear_context_query(cx);
            assert_eq!(c.value(cx).as_ref(), "没有提及的草稿");
        });
    }

    #[test]
    fn context_usage_clamps_remaining_and_handles_unknown_capacity() {
        let usage = ComposerContextUsage::new(32000, 128000);
        assert_eq!(usage.remaining_tokens(), 96000);
        assert_eq!(usage.remaining_percent(), 75);
        assert_eq!(usage.used_ratio(), 0.25);
        let full = ComposerContextUsage::new(200000, 128000);
        assert_eq!(full.remaining_tokens(), 0);
        assert_eq!(full.used_ratio(), 1.);
        assert_eq!(ComposerContextUsage::new(0, 0).remaining_percent(), 0);
        assert_eq!(
            ComposerContextUsage::new(u64::MAX, u64::MAX).remaining_tokens(),
            0
        );
    }

    #[gpui::test]
    fn context_usage_is_optional_and_updates_without_changing_the_prompt(
        cx: &mut gpui::TestAppContext,
    ) {
        let (composer, window) = cx.add_window_view(|_, cx| Composer::new("Message", cx));
        composer.update(window, |composer, cx| composer.set_value("keep draft", cx));
        assert!(window.debug_bounds("composer-context-usage").is_none());
        composer.update(window, |composer, cx| {
            composer.set_context_state(
                Some(ComposerContextState::Unknown {
                    used_tokens: None,
                    capacity_tokens: Some(128000),
                }),
                cx,
            )
        });
        assert!(window.debug_bounds("composer-context-usage").is_some());
        composer.update(window, |composer, cx| {
            composer.set_context_usage(Some(ComposerContextUsage::new(32000, 128000)), cx)
        });
        assert!(window.debug_bounds("composer-context-usage").is_some());
        composer.read_with(window, |composer, cx| {
            assert_eq!(composer.value(cx).as_ref(), "keep draft")
        });
        composer.update(window, |composer, cx| composer.set_context_usage(None, cx));
        assert!(window.debug_bounds("composer-context-usage").is_none());
    }

    #[gpui::test]
    fn restoring_drafts_isolates_images_and_nontext_notifications_do_not_emit_changed(
        cx: &mut gpui::TestAppContext,
    ) {
        use std::{cell::Cell, rc::Rc};
        let composer = cx.new(|cx| Composer::new("Message", cx));
        let changes = Rc::new(Cell::new(0));
        let received = changes.clone();
        let _observer = cx.new(|cx: &mut Context<()>| {
            cx.subscribe(&composer, move |_, _, event: &ComposerEvent, _| {
                if matches!(event, ComposerEvent::Changed(_)) {
                    received.set(received.get() + 1);
                }
            })
            .detach();
        });
        let image = PastedImage::new(gpui::ImageFormat::Png, vec![1, 2, 3]);
        composer.update(cx, |composer, cx| {
            composer.set_draft("A", vec![image.clone()], cx)
        });
        cx.run_until_parked();
        let editor = composer.read_with(cx, |composer, _| composer.editor.clone());
        editor.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert_eq!(changes.get(), 0);
        composer.update(cx, |composer, cx| composer.set_draft("B", Vec::new(), cx));
        cx.run_until_parked();
        composer.read_with(cx, |composer, cx| {
            assert!(composer.attachments().is_empty());
            assert_eq!(composer.value(cx).as_ref(), "B");
        });
        composer.update(cx, |composer, cx| composer.set_draft("A", vec![image], cx));
        cx.run_until_parked();
        composer.read_with(cx, |composer, _| {
            assert_eq!(composer.attachments()[0].bytes(), &[1, 2, 3])
        });
        editor.update(cx, |editor, cx| editor.set_value("edited", cx));
        cx.run_until_parked();
        assert_eq!(changes.get(), 1);
        editor.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert_eq!(changes.get(), 1);
    }

    #[gpui::test]
    fn at_trigger_opens_context_and_selected_path_replaces_the_trigger(
        cx: &mut gpui::TestAppContext,
    ) {
        let composer = cx.new(|cx| {
            Composer::new("Message", cx)
                .context_panel(
                    "attach",
                    "添加",
                    [ComposerContextSection::untitled([
                        ComposerContextEntry::new("file", "文件").insert_mention(false),
                    ])],
                )
                .context_on_at(true)
        });
        composer.update(cx, |composer, cx| composer.set_value("查看 @", cx));
        cx.run_until_parked();
        composer.update(cx, |composer, cx| {
            assert!(composer.context_panel_open());
            composer.close_context_panel(cx);
            composer.add_mention("src/main.rs", 0x007aff, cx);
            assert_eq!(composer.value(cx).as_ref(), "查看 @src/main.rs ");
        });
        cx.run_until_parked();
        composer.update(cx, |composer, cx| {
            composer.set_value("user@example.com", cx)
        });
        cx.run_until_parked();
        composer.read_with(cx, |composer, _| assert!(!composer.context_panel_open()));
    }

    #[gpui::test]
    fn refreshing_model_options_preserves_prompt_queue_and_valid_selection(
        cx: &mut gpui::TestAppContext,
    ) {
        let composer = cx.new(|cx| {
            Composer::new("Message", cx)
                .menu_chip(ComposerChip::new("model").option("a", "A").option("b", "B"))
        });
        composer.update(cx, |composer, cx| {
            composer.set_value("未发送的内容\n第二行", cx);
            composer
                .queue
                .push(ComposerSubmission::text("排队中的内容").into());
            composer.chip_values.insert("model".into(), "b".into());
            composer.set_menu_chip(
                ComposerChip::new("model").option("b", "B").option("c", "C"),
                cx,
            );
            assert_eq!(composer.value(cx).as_ref(), "未发送的内容\n第二行");
            assert_eq!(composer.queue_len(), 1);
            assert_eq!(composer.chip_values["model"].as_ref(), "b");
            assert_eq!(composer.menu_chips.len(), 1);
            composer.set_menu_chip(ComposerChip::new("model").option("c", "C"), cx);
            assert_eq!(composer.chip_values["model"].as_ref(), "c");
        });
    }

    #[gpui::test]
    fn toolbar_refresh_keeps_model_menu_open_until_options_are_removed(
        cx: &mut gpui::TestAppContext,
    ) {
        let composer = cx.new(|cx| {
            Composer::new("Message", cx).menu_chip(ComposerChip::new("model").option("a", "A"))
        });
        composer.update(cx, |composer, cx| {
            composer.open_chip = Some("model".into());
            composer.set_menu_chip(ComposerChip::new("partner").option("none", "None"), cx);
            assert_eq!(composer.open_chip.as_deref(), Some("model"));
            composer.set_menu_chip(ComposerChip::new("model").option("b", "B"), cx);
            assert_eq!(composer.open_chip.as_deref(), Some("model"));
            assert_eq!(composer.chip_values["model"].as_ref(), "b");
            composer.set_menu_chip(ComposerChip::new("model"), cx);
            assert!(composer.open_chip.is_none());
        });
    }

    #[gpui::test]
    fn at_panel_closes_with_trigger_and_supports_keyboard_selection_without_submitting(cx: &mut gpui::TestAppContext) {
        cx.update(bind_composer_keys);
        cx.update(super::super::bind_prompt_editor_keys);
        struct Preview(Entity<Composer>);
        impl Render for Preview {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { div().size_full().flex().flex_col().justify_end().child(self.0.clone()) }
        }
        let (preview, window) = cx.add_window_view(|_, cx| Preview(cx.new(|cx| {
            Composer::new("Message", cx).context_on_at(true).context_panel("attach", "Context", [ComposerContextSection::untitled([
                ComposerContextEntry::new("first", "First"), ComposerContextEntry::new("second", "Second")
            ])])
        })));
        let composer = preview.read_with(window, |p, _| p.0.clone());
        let submitted = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = submitted.clone();
        let _subscription = window.update(|_, cx| cx.subscribe(&composer, move |_, event, _| {
            if matches!(event, ComposerEvent::Submitted(_)) { count.set(count.get() + 1); }
        }));
        composer.update(window, |c, cx| c.set_value("@", cx));
        window.run_until_parked();
        window.update(|window, cx| window.focus(&composer.read(cx).editor.focus_handle(cx), cx));
        window.simulate_keystrokes("down enter");
        window.run_until_parked();
        composer.read_with(window, |c, cx| {
            assert!(!c.context_open);
            assert!(c.value(cx).contains("@Second"), "{}", c.value(cx));
        });
        assert_eq!(submitted.get(), 0);
        composer.update(window, |c, cx| c.set_value("@", cx));
        window.run_until_parked();
        window.simulate_keystrokes("backspace");
        window.run_until_parked();
        composer.read_with(window, |c, _| assert!(!c.context_open));
        composer.update(window, |c, cx| c.set_value("@no-match", cx));
        window.run_until_parked();
        window.simulate_keystrokes("enter");
        assert_eq!(submitted.get(), 0, "empty suggestions must not send a message");
    }

    #[gpui::test]
    fn access_menu_survives_toolbar_refresh_while_hovering_between_options(cx: &mut gpui::TestAppContext) {
        struct Preview(Entity<Composer>);
        impl Render for Preview {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().flex().flex_col().justify_end().child(self.0.clone())
            }
        }
        let (preview, window) = cx.add_window_view(|_, cx| Preview(cx.new(|cx| {
            Composer::new("Message", cx)
                .chip(ComposerChip::new("access").option("ask", "请求批准").option("auto", "自动批准").option("full", "完全访问权限"))
                .menu_chip(ComposerChip::new("model").option("a", "A"))
        })));
        let composer = preview.read_with(window, |preview, _| preview.0.clone());
        composer.update(window, |composer, cx| { composer.open_chip = Some("access".into()); cx.notify(); });
        window.run_until_parked();
        for value in ["ask", "auto", "full", "auto", "ask"] {
            let selector = Box::leak(format!("composer-option:access:{value}").into_boxed_str());
            let row = window.debug_bounds(selector).unwrap();
            window.simulate_mouse_move(row.center(), None, Default::default());
            composer.update(window, |composer, cx| {
                composer.set_menu_chip(ComposerChip::new("model").option("a", "A"), cx);
                composer.set_menu_chip(ComposerChip::new("partner").option("none", "None"), cx);
                assert_eq!(composer.open_chip.as_deref(), Some("access"));
            });
            window.run_until_parked();
            assert!(window.debug_bounds("composer-chip-menu").is_some());
        }

    }

    #[gpui::test]
    fn hovering_between_model_rows_keeps_menu_open(cx: &mut gpui::TestAppContext) {
        let (composer, window) = cx.add_window_view(|_, cx| {
            Composer::new("Message", cx).menu_chip(
                ComposerChip::new("model")
                    .option("a", "A")
                    .option("b", "B")
                    .option("c", "C"),
            )
        });
        composer.update(window, |composer, cx| {
            composer.open_chip = Some("model".into());
            cx.notify();
        });
        for selector in [
            "composer-option:model:a",
            "composer-option:model:b",
            "composer-option:model:c",
            "composer-option:model:b",
            "composer-option:model:a",
        ] {
            let row = window.debug_bounds(selector).unwrap();
            window.simulate_mouse_move(row.center(), None, Default::default());
            window.run_until_parked();
            composer.read_with(window, |composer, _| {
                assert_eq!(composer.open_chip.as_deref(), Some("model"));
            });
            assert!(window.debug_bounds("composer-chip-menu").is_some());
        }
    }

    #[gpui::test]
    fn long_model_labels_stay_inside_the_menu_and_leave_room_for_the_check(
        cx: &mut gpui::TestAppContext,
    ) {
        let (composer, window) = cx.add_window_view(|_, cx| {
            Composer::new("Message", cx).menu_chip(ComposerChip::new("model").option(
                "long",
                "DeepSeek · a-very-long-model-name-that-must-not-overflow-the-menu",
            ))
        });
        composer.update(window, |composer, cx| {
            composer.open_chip = Some("model".into());
            cx.notify();
        });
        let menu = window.debug_bounds("composer-chip-menu").unwrap();
        let label = window.debug_bounds("composer-option-label-0").unwrap();
        let check = window.debug_bounds("composer-option-check-0").unwrap();
        assert!(menu.size.width <= px(360.));
        assert!(label.left() >= menu.left());
        assert!(label.right() < check.left());
        assert!(check.right() < menu.right());
    }

    #[gpui::test]
    fn pasted_images_attach_submit_with_the_prompt_and_clear(cx: &mut gpui::TestAppContext) {
        let composer = cx.new(|cx| Composer::new("Message", cx));
        let editor = composer.read_with(cx, |composer, _| composer.editor.clone());
        let image = PastedImage::new(gpui::ImageFormat::Png, vec![1, 2, 3]);

        // The editor emits ImagePasted when the clipboard holds an image.
        editor.update(cx, |_, cx| {
            cx.emit(PromptEditorEvent::ImagePasted(vec![image]));
        });
        cx.run_until_parked();
        composer.read_with(cx, |composer, cx| {
            assert_eq!(composer.attachments().len(), 1);
            assert!(composer.sendable(cx));
        });

        composer.update(cx, |composer, cx| {
            composer.busy = true;
            composer.submit(cx);
        });
        composer.read_with(cx, |composer, _| {
            assert!(composer.attachments().is_empty());
            assert_eq!(composer.queue_len(), 1);
            let queued = &composer.queue[0];
            assert_eq!(queued.text.as_ref(), "");
            assert_eq!(queued.images.len(), 1);
            assert_eq!(queued.images[0].bytes(), &[1, 2, 3]);
        });
    }
}
