use super::{Icon, PromptEditor};
use crate::theme::ThemePreset;
use gpui::prelude::FluentBuilder;
use gpui::{
    Animation, AnimationExt as _, App, AppContext, BoxShadow, Context, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, IntoElement, KeyBinding, MouseButton,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    actions, deferred, div, point, px, relative, rgb, rgba,
};
use std::collections::HashMap;

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
}

impl ComposerContextEntry {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            label: label.into(),
            description: None,
            color: None,
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

#[derive(Clone, Debug)]
pub enum ComposerEvent {
    /// The prompt text changed.
    Changed(SharedString),
    /// An accessory chip without options was clicked; the payload is the chip id.
    ChipClicked(SharedString),
    /// A select-style chip picked a new value.
    ChipSelected {
        chip: SharedString,
        value: SharedString,
    },
    /// The prompt was submitted via the send button or the Enter shortcut.
    Submitted(SharedString),
    /// The prompt was queued because a submission is already in flight.
    Queued(SharedString),
    /// A context panel entry was picked. The matching mention token
    /// (`@label`) has already been inserted into the prompt.
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
pub struct Composer {
    editor: Entity<PromptEditor>,
    theme: ThemePreset,
    chips: Vec<ComposerChip>,
    menu_chips: Vec<ComposerChip>,
    chip_values: HashMap<SharedString, SharedString>,
    open_chip: Option<SharedString>,
    queue: Vec<SharedString>,
    context_trigger: Option<SharedString>,
    context_title: SharedString,
    context_sections: Vec<ComposerContextSection>,
    context_hints: Vec<(SharedString, SharedString)>,
    context_open: bool,
    mention_colors: HashMap<SharedString, u32>,
    busy: bool,
    disabled: bool,
    clear_on_submit: bool,
    send_enabled_override: Option<bool>,
    focus: FocusHandle,
    _subscription: Subscription,
}

impl EventEmitter<ComposerEvent> for Composer {}
impl Focusable for Composer {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Composer {
    pub fn new(placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| PromptEditor::new(cx, placeholder).min_rows(2));
        let subscription = cx.observe(&editor, |this, _, cx| {
            let text: SharedString = this.editor.read(cx).value().to_owned().into();
            cx.emit(ComposerEvent::Changed(text));
            cx.notify();
        });
        Self {
            editor,
            theme: ThemePreset::default(),
            chips: Vec::new(),
            menu_chips: Vec::new(),
            chip_values: HashMap::new(),
            open_chip: None,
            queue: Vec::new(),
            context_trigger: None,
            context_title: "添加".into(),
            context_sections: Vec::new(),
            context_hints: Vec::new(),
            context_open: false,
            mention_colors: HashMap::new(),
            busy: false,
            disabled: false,
            clear_on_submit: true,
            send_enabled_override: None,
            focus: cx.focus_handle(),
            _subscription: subscription,
        }
    }

    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
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
        self.open_chip = None;
        cx.notify();
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

    /// Appends a colored `@label` mention token to the prompt and registers
    /// its paint color.
    pub fn add_mention(
        &mut self,
        label: impl Into<SharedString>,
        color: u32,
        cx: &mut Context<Self>,
    ) {
        let label = label.into();
        let mut next = self.editor.read(cx).value().to_owned();
        if !next.is_empty() && !next.ends_with(' ') && !next.ends_with('\n') {
            next.push(' ');
        }
        next.push('@');
        next.push_str(&label);
        next.push(' ');
        self.mention_colors
            .insert(SharedString::from(format!("@{label}")), color);
        let highlights: Vec<(SharedString, u32)> = self
            .mention_colors
            .iter()
            .map(|(token, color)| (token.clone(), *color))
            .collect();
        self.editor.update(cx, |editor, cx| {
            editor.set_value(next, cx);
            editor.set_highlights(highlights, cx);
        });
        cx.notify();
    }

    pub fn context_panel_open(&self) -> bool {
        self.context_open
    }

    pub fn open_context_panel(&mut self, cx: &mut Context<Self>) {
        if !self.context_sections.is_empty() {
            self.context_open = true;
            self.open_chip = None;
            cx.notify();
        }
    }

    pub fn close_context_panel(&mut self, cx: &mut Context<Self>) {
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

    /// Number of prompts waiting in the queue.
    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    fn sendable(&self, cx: &App) -> bool {
        if self.disabled {
            return false;
        }
        match self.send_enabled_override {
            Some(enabled) => enabled,
            None => !self.editor.read(cx).is_empty(),
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if !self.sendable(cx) || self.editor.read(cx).is_composing() {
            return;
        }
        let text: SharedString = self.editor.read(cx).value().to_owned().into();
        if self.clear_on_submit {
            self.editor
                .update(cx, |editor, cx| editor.set_value("", cx));
        }
        if self.busy {
            self.queue.push(text.clone());
            cx.emit(ComposerEvent::Queued(text));
        } else {
            cx.emit(ComposerEvent::Submitted(text));
        }
        cx.notify();
    }

    fn submit_next_queued(&mut self, cx: &mut Context<Self>) {
        if self.queue.is_empty() {
            return;
        }
        let next = self.queue.remove(0);
        cx.emit(ComposerEvent::Submitted(next));
    }

    fn queue_submit_now(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.queue.len() {
            return;
        }
        let text = self.queue.remove(index);
        cx.emit(ComposerEvent::Submitted(text));
        cx.notify();
    }

    fn queue_edit(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.queue.len() {
            return;
        }
        let text = self.queue.remove(index);
        self.editor
            .update(cx, |editor, cx| editor.set_value(text, cx));
        cx.notify();
    }

    fn queue_remove(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.queue.len() {
            self.queue.remove(index);
            cx.notify();
        }
    }
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        self.editor
            .update(cx, |editor, cx| editor.set_theme(theme, cx));
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
                                this.context_open = true;
                                this.open_chip = None;
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
            if let Some(icon) = chip.icon {
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

        let send_button = div()
            .id("glass-composer-send")
            .size(px(28.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .cursor_pointer()
            .bg(rgb(if sendable { p.accent } else { p.surface_subtle }))
            .text_color(rgb(if sendable { 0xffffff } else { p.text_muted }))
            .when(sendable, |button| {
                button.hover(move |style| style.opacity(0.88))
            })
            .on_click(cx.listener(|this, _, _, cx| this.submit(cx)))
            .child(Icon::ArrowUp.render(16.));

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
            bar = bar.child(
                div()
                    .flex_shrink_0()
                    .text_color(rgb(p.text_muted))
                    .child(Icon::Spinner.render(14.))
                    .with_animation(
                        "glass-composer-spinner",
                        Animation::new(std::time::Duration::from_millis(900)).repeat(),
                        |spinner, delta| spinner.opacity(0.3 + 0.7 * delta),
                    ),
            );
        }
        bar = bar
            .children(self.menu_chips.iter().map(|chip| chip_button(chip, cx)))
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
        if self.context_open {
            card = card.child(self.context_panel_element(cx));
        }
        let card = card.child(editor_scroll).child(bar);

        let mut root = div()
            .id("glass-composer")
            .key_context("GlassComposer")
            .track_focus(&self.focus)
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(6.))
            .on_action(cx.listener(|this, _: &Submit, _, cx| this.submit(cx)))
            .capture_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
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

        for (index, text) in self.queue.iter().enumerate() {
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
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.))
                        .text_color(rgb(p.text))
                        .child(text.clone()),
                )
                .child(
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
                .child(
                    icon_action(
                        SharedString::from(format!("glass-composer-queue-edit-{index}")),
                        Icon::Pencil,
                        p,
                        dark,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.queue_edit(index, cx);
                    })),
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
            root = root.child(row);
        }

        root.child(card)
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
        let menu_width = (label_width + 64.)
            .clamp(168., 360.)
            .min((f32::from(window.viewport_size().width) - 32.).max(120.));
        let rows = chip.options.iter().enumerate().map(|(index, option)| {
            let is_selected = selection.as_ref() == Some(&option.value);
            div()
                .id(SharedString::from(format!(
                    "glass-composer-option-{}-{index}",
                    chip.id
                )))
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

        for section in &self.context_sections {
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
            for entry in &section.entries {
                let entry_color = theme.color(entry.color.unwrap_or(p.accent));
                panel = panel.child(
                    div()
                        .id(SharedString::from(format!(
                            "glass-composer-context-{}",
                            entry.id
                        )))
                        .h(px(40.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .rounded(px(8.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover_bg))
                        .on_click({
                            let id = entry.id.clone();
                            let label = entry.label.clone();
                            let color = entry.color.unwrap_or(p.accent);
                            cx.listener(move |this, _, _, cx| {
                                this.close_context_panel(cx);
                                this.add_mention(label.clone(), color, cx);
                                cx.emit(ComposerEvent::ContextSelected {
                                    id: id.clone(),
                                    label: label.clone(),
                                });
                                cx.notify();
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
    fn refreshing_model_options_preserves_prompt_queue_and_valid_selection(
        cx: &mut gpui::TestAppContext,
    ) {
        let composer = cx.new(|cx| {
            Composer::new("Message", cx)
                .menu_chip(ComposerChip::new("model").option("a", "A").option("b", "B"))
        });
        composer.update(cx, |composer, cx| {
            composer.set_value("未发送的内容\n第二行", cx);
            composer.queue.push("排队中的内容".into());
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
}
