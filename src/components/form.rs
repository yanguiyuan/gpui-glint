//! Controlled form composition. The host owns values, validation and requests.
use super::{Button, ButtonKind, ButtonSize};
use crate::theme::ThemePreset;
use gpui::{
    AnyElement, App, IntoElement, KeyDownEvent, ParentElement, SharedString, Styled, Window, div,
    prelude::*, px, rgb, rgba,
};
use std::rc::Rc;

/// A submit intent from Enter or the primary action; the host decides what to do.
#[derive(Clone, Copy, Debug)]
pub struct FormSubmit;
type SubmitHandler = Rc<dyn Fn(&FormSubmit, &mut Window, &mut App)>;

/// A labeled control with a stable-height focus ring and optional field feedback.
/// Pass the control directly so percentage-height inputs keep their hit area.
pub struct FormField {
    id: SharedString,
    label: SharedString,
    control: AnyElement,
    trailing: Option<AnyElement>,
    description: Option<SharedString>,
    error: Option<SharedString>,
    theme: ThemePreset,
    focused: bool,
    required: bool,
    height: f32,
}
impl FormField {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        control: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            control: control.into_any_element(),
            trailing: None,
            description: None,
            error: None,
            theme: ThemePreset::default(),
            focused: false,
            required: false,
            height: 34.,
        }
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    /// Focus is controlled by the host, which can observe its input entities.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }
    /// Presentation only; the host remains responsible for validating the value.
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }
    pub fn description(mut self, text: impl Into<SharedString>) -> Self {
        self.description = Some(text.into());
        self
    }
    pub fn error(mut self, text: impl Into<SharedString>) -> Self {
        self.error = Some(text.into());
        self
    }
    pub fn trailing(mut self, control: impl IntoElement) -> Self {
        self.trailing = Some(control.into_any_element());
        self
    }
    pub fn control_height(mut self, height: f32) -> Self {
        if height.is_finite() {
            self.height = height.max(24.);
        }
        self
    }
    pub fn render(self) -> AnyElement {
        let p = self.theme.palette();
        let error_color = self.theme.color(0xd9342b);
        let invalid = self.error.is_some();
        let selector = self.id.clone();
        let label = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .text_size(px(11.))
            .text_color(rgb(p.text_muted))
            .child(self.label)
            .when(self.required, |label| {
                label.child(div().text_color(rgb(error_color)).child("*"))
            });
        let row = div()
            .id(self.id.clone())
            .debug_selector(move || selector.to_string())
            .h(px(self.height))
            .w_full()
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(8.))
            .border_1()
            .border_color(if invalid {
                rgb(error_color)
            } else if self.focused {
                rgb(p.accent)
            } else if self.theme.is_dark() {
                rgba(0xffffff1f)
            } else {
                rgb(p.border)
            })
            .bg(if self.theme.is_dark() {
                rgba(0x00000059)
            } else {
                rgb(p.surface)
            })
            .child(self.control)
            .children(self.trailing);
        let mut field = div().flex().flex_col().gap(px(6.)).child(label).child(
            div()
                .w_full()
                .p(px(2.))
                .rounded(px(10.))
                .bg(if self.focused {
                    rgb(if invalid {
                        self.theme.color(0xffe9e7)
                    } else {
                        p.accent_tint
                    })
                } else {
                    rgba(0)
                })
                .child(row),
        );
        if let Some(error) = self.error {
            let id = format!("{}-error", self.id);
            field = field.child(
                div()
                    .debug_selector(move || id.clone())
                    .text_size(px(11.))
                    .text_color(rgb(error_color))
                    .child(error),
            );
        } else if let Some(description) = self.description {
            let id = format!("{}-description", self.id);
            field = field.child(
                div()
                    .debug_selector(move || id.clone())
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(description),
            );
        }
        field.into_any_element()
    }
}

/// A field group with an optional heading and explanatory text.
pub struct FormSection {
    title: Option<SharedString>,
    description: Option<SharedString>,
    children: Vec<AnyElement>,
    theme: ThemePreset,
}
impl Default for FormSection {
    fn default() -> Self {
        Self::new()
    }
}
impl FormSection {
    pub fn new() -> Self {
        Self {
            title: None,
            description: None,
            children: vec![],
            theme: ThemePreset::default(),
        }
    }
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn render(self) -> AnyElement {
        let p = self.theme.palette();
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .children(self.title.map(|title| {
                div()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(rgb(p.text))
                    .child(title)
            }))
            .children(self.description.map(|text| {
                div()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(text)
            }))
            .children(self.children)
            .into_any_element()
    }
}
impl ParentElement for FormSection {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

/// A submit action and optional secondary control. Busy/disabled actions never
/// invoke the handler, even if a click is delivered programmatically.
pub struct FormActions {
    id: SharedString,
    label: SharedString,
    busy_label: SharedString,
    theme: ThemePreset,
    busy: bool,
    disabled: bool,
    full_width: bool,
    secondary: Option<AnyElement>,
    handler: Option<SubmitHandler>,
}
impl FormActions {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            id: "form-submit".into(),
            label: label.into(),
            busy_label: "正在提交…".into(),
            theme: ThemePreset::default(),
            busy: false,
            disabled: false,
            full_width: true,
            secondary: None,
            handler: None,
        }
    }
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = id.into();
        self
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }
    pub fn busy_label(mut self, label: impl Into<SharedString>) -> Self {
        self.busy_label = label.into();
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn full_width(mut self, full_width: bool) -> Self {
        self.full_width = full_width;
        self
    }
    pub fn secondary(mut self, secondary: impl IntoElement) -> Self {
        self.secondary = Some(secondary.into_any_element());
        self
    }
    pub fn on_submit(
        mut self,
        handler: impl Fn(&FormSubmit, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.handler = Some(Rc::new(handler));
        self
    }
    pub fn render(self) -> AnyElement {
        let disabled = self.busy || self.disabled;
        let mut button = Button::new(if self.busy {
            self.busy_label
        } else {
            self.label
        })
        .theme(self.theme)
        .kind(ButtonKind::Primary)
        .size(ButtonSize::Large);
        if disabled {
            button = button.disabled();
        }
        let selector = self.id.clone();
        let handler = self.handler;
        let primary = button
            .render()
            .id(self.id)
            .debug_selector(move || selector.to_string())
            .h(px(36.))
            .text_size(px(13.))
            .shadow_sm()
            .when(self.full_width, |button| button.flex_1().min_w_0())
            .on_click(move |_, window, cx| {
                if !disabled {
                    if let Some(handler) = &handler {
                        handler(&FormSubmit, window, cx);
                    }
                }
            });
        div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.))
            .children(self.secondary)
            .child(primary)
            .into_any_element()
    }
}

enum Actions {
    Standard(FormActions),
    Custom(AnyElement),
}
/// Form-level feedback and opt-in Enter submission. It never reads field values
/// or runs validation. The host handles IME composition and multiline controls.
/// Busy/disabled suppress submission; supplied child controls retain their own
/// enabled state and should be disabled by their owner when appropriate.
pub struct Form {
    id: SharedString,
    submit_guard: Option<Rc<dyn Fn(&Window, &App) -> bool>>,
    children: Vec<AnyElement>,
    actions: Option<Actions>,
    error: Option<SharedString>,
    hint: Option<SharedString>,
    error_id: SharedString,
    hint_id: SharedString,
    theme: ThemePreset,
    busy: bool,
    disabled: bool,
    submit_on_enter: bool,
    handler: Option<SubmitHandler>,
}
impl Default for Form {
    fn default() -> Self {
        Self::new()
    }
}
impl Form {
    pub fn new() -> Self {
        Self {
            id: "form".into(),
            submit_guard: None,
            children: vec![],
            actions: None,
            error: None,
            hint: None,
            error_id: "form-error".into(),
            hint_id: "form-hint".into(),
            theme: ThemePreset::default(),
            busy: false,
            disabled: false,
            submit_on_enter: false,
            handler: None,
        }
    }
    /// Give multiple forms in the same view distinct ids.
    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = id.into();
        self
    }
    /// Return false while an input is composing or owns Enter. Such events are
    /// left untouched for that control; the guard does not run validation.
    pub fn submit_guard(mut self, guard: impl Fn(&Window, &App) -> bool + 'static) -> Self {
        self.submit_guard = Some(Rc::new(guard));
        self
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn error(mut self, text: impl Into<SharedString>) -> Self {
        self.error = Some(text.into());
        self
    }
    pub fn hint(mut self, text: impl Into<SharedString>) -> Self {
        self.hint = Some(text.into());
        self
    }
    pub fn error_id(mut self, id: impl Into<SharedString>) -> Self {
        self.error_id = id.into();
        self
    }
    pub fn hint_id(mut self, id: impl Into<SharedString>) -> Self {
        self.hint_id = id.into();
        self
    }
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    /// Off by default so embedding a multiline editor does not change Enter behavior.
    pub fn submit_on_enter(mut self, enabled: bool) -> Self {
        self.submit_on_enter = enabled;
        self
    }
    pub fn on_submit(
        mut self,
        handler: impl Fn(&FormSubmit, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.handler = Some(Rc::new(handler));
        self
    }
    /// Standard actions inherit busy/disabled and the form's submit handler.
    pub fn actions(mut self, actions: FormActions) -> Self {
        self.actions = Some(Actions::Standard(actions));
        self
    }
    /// Custom action elements retain their own enabled state and handlers.
    pub fn actions_element(mut self, actions: impl IntoElement) -> Self {
        self.actions = Some(Actions::Custom(actions.into_any_element()));
        self
    }
    pub fn render(self) -> AnyElement {
        let p = self.theme.palette();
        let disabled = self.disabled || self.busy;
        let has_feedback =
            self.error.is_some() || (!self.busy && self.hint.is_some()) || self.actions.is_some();
        let submit_guard = self.submit_guard;
        let handler = self.handler;
        let actions = self.actions.map(|actions| match actions {
            Actions::Standard(mut actions) => {
                actions.busy |= self.busy;
                actions.disabled |= self.disabled;
                if actions.handler.is_none() {
                    actions.handler = handler.clone();
                }
                actions.render()
            }
            Actions::Custom(element) => element,
        });
        let mut feedback = div().flex().flex_col().gap(px(12.));
        if let Some(error) = self.error {
            let id = self.error_id;
            let selector = id.clone();
            feedback = feedback.child(
                div()
                    .id(id)
                    .debug_selector(move || selector.to_string())
                    .w_full()
                    .flex()
                    .items_start()
                    .gap(px(7.))
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(7.))
                    .bg(rgb(self.theme.color(0xffe9e7)))
                    .text_size(px(11.))
                    .text_color(rgb(self.theme.color(0xd9342b)))
                    .child(div().flex_shrink_0().w(px(13.)).child("!"))
                    .child(div().flex_1().min_w_0().child(error)),
            );
        } else if !self.busy {
            if let Some(hint) = self.hint {
                let id = self.hint_id;
                let selector = id.clone();
                feedback = feedback.child(
                    div()
                        .id(id)
                        .debug_selector(move || selector.to_string())
                        .text_size(px(11.))
                        .text_color(rgb(p.text_muted))
                        .child(hint),
                );
            }
        }
        div()
            .id(self.id)
            .w_full()
            .flex()
            .flex_col()
            .gap(px(18.))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let modifiers = &event.keystroke.modifiers;
                if self.submit_on_enter
                    && event.keystroke.key == "enter"
                    && !modifiers.control
                    && !modifiers.alt
                    && !modifiers.platform
                    && !modifiers.shift
                {
                    if submit_guard
                        .as_ref()
                        .is_some_and(|guard| !guard(window, cx))
                    {
                        return;
                    }
                    window.prevent_default();
                    cx.stop_propagation();
                    if !disabled {
                        if let Some(handler) = &handler {
                            handler(&FormSubmit, window, cx);
                        }
                    }
                }
            })
            .children(self.children)
            .when(has_feedback, |form| form.child(feedback.children(actions)))
            .into_any_element()
    }
}
impl ParentElement for Form {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::TextEntry;
    use gpui::{Context, Entity, Focusable, Render};

    struct Harness {
        input: Entity<TextEntry>,
        count: usize,
        busy: bool,
        disabled: bool,
        focused: bool,
        enter: bool,
        invalid: bool,
        allow_submit: bool,
    }
    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let mut field = FormField::new("name", "名称", self.input.clone())
                .required(true)
                .focused(self.focused)
                .description("填写名称");
            if self.invalid {
                field = field.error("名称不能为空");
            }
            let allow_submit = self.allow_submit;
            Form::new()
                .submit_guard(move |_, _| allow_submit)
                .busy(self.busy)
                .disabled(self.disabled)
                .submit_on_enter(self.enter)
                .error("提交失败")
                .hint("旧提示")
                .on_submit(cx.listener(|this, _: &FormSubmit, _, cx| {
                    this.count += 1;
                    cx.notify();
                }))
                .child(FormSection::new().child(field.render()).render())
                .actions(FormActions::new("保存"))
                .render()
        }
    }
    fn mount(cx: &mut gpui::TestAppContext) -> (Entity<Harness>, &mut gpui::VisualTestContext) {
        let (host, window) = cx.add_window_view(|_, cx| Harness {
            input: cx.new(|cx| TextEntry::new(cx, "", "")),
            count: 0,
            busy: false,
            disabled: false,
            focused: false,
            enter: true,
            invalid: false,
            allow_submit: true,
        });
        window.run_until_parked();
        (host, window)
    }
    #[gpui::test]
    fn standard_actions_inherit_handler_and_pending_state(cx: &mut gpui::TestAppContext) {
        let (host, window) = mount(cx);
        let save = window.debug_bounds("form-submit").unwrap();
        window.simulate_click(save.center(), gpui::Modifiers::default());
        host.read_with(window, |host, _| assert_eq!(host.count, 1));
        window.update(|window, cx| {
            let focus = host.read(cx).input.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        });
        window.simulate_keystrokes("enter");
        host.read_with(window, |host, _| assert_eq!(host.count, 2));
        host.update(window, |host, cx| {
            host.allow_submit = false;
            cx.notify();
        });
        window.run_until_parked();
        window.simulate_event(KeyDownEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
            is_held: false,
            prefer_character_input: false,
        });
        host.read_with(window, |host, _| {
            assert_eq!(host.count, 2, "an IME/control guard can keep Enter")
        });
        host.update(window, |host, cx| {
            host.allow_submit = true;
            cx.notify();
        });
        for busy in [true, false] {
            host.update(window, |host, cx| {
                host.busy = busy;
                host.disabled = !busy;
                cx.notify();
            });
            window.run_until_parked();
            let save = window.debug_bounds("form-submit").unwrap();
            window.simulate_click(save.center(), gpui::Modifiers::default());
            window.simulate_keystrokes("enter");
            host.read_with(window, |host, _| {
                assert_eq!(host.count, 2, "busy and disabled suppress click and Enter")
            });
        }
    }
    #[gpui::test]
    fn feedback_priority_and_focus_do_not_change_the_control_hit_area(
        cx: &mut gpui::TestAppContext,
    ) {
        let (host, window) = mount(cx);
        assert!(window.debug_bounds("form-error").is_some());
        assert!(window.debug_bounds("form-hint").is_none());
        assert!(window.debug_bounds("name-description").is_some());
        let before = window.debug_bounds("name").unwrap();
        host.update(window, |host, cx| {
            host.focused = true;
            host.invalid = true;
            host.enter = false;
            cx.notify();
        });
        window.run_until_parked();
        let after = window.debug_bounds("name").unwrap();
        assert_eq!(before.size, after.size);
        assert!(window.debug_bounds("name-error").is_some());
        assert!(window.debug_bounds("name-description").is_none());
        window.simulate_click(after.center(), gpui::Modifiers::default());
        window.simulate_input("文字");
        window.simulate_event(KeyDownEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
            is_held: false,
            prefer_character_input: false,
        });
        host.read_with(window, |host, cx| {
            assert_eq!(host.input.read(cx).value(), "文字");
            assert_eq!(host.count, 0, "Enter is opt-in");
        });
    }
}
