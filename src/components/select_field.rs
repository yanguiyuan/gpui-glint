use super::{Select, SelectEvent, SelectOption, TextEntry, TextReferenceResolver};
use crate::theme::ThemePreset;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Window, div, px, rgb,
};
use gpui::{BoxShadow, InteractiveElement, point};

/// 两个字段的当前值。选项值与显示标签分离，文本保留原文。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectFieldValue {
    pub selected: Option<SharedString>,
    pub text: SharedString,
}
#[derive(Clone, Debug)]
pub enum SelectFieldEvent {
    Changed(SelectFieldValue),
}

/// Select + TextEntry 共用 32px 外框。注册 bind_text_entry_keys 一次。
/// 用户修改会提交内部状态并发出 Changed；程序设置不会发出变更事件。
pub struct SelectField {
    select: Entity<Select>,
    input: Entity<TextEntry>,
    value: SelectFieldValue,
    theme: ThemePreset,
    disabled: bool,
    input_disabled: bool,
    error: Option<SharedString>,
    focus: FocusHandle,
    focus_subscriptions: Vec<Subscription>,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<SelectFieldEvent> for SelectField {}
impl Focusable for SelectField {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.select.focus_handle(cx)
    }
}
impl SelectField {
    pub fn new(
        options: impl IntoIterator<Item = SelectOption>,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let options: Vec<_> = options.into_iter().collect();
        let selected = options
            .iter()
            .find(|o| !o.disabled)
            .map(|o| o.value.clone());
        let select = cx.new(|cx| {
            let control = Select::new(options, cx).embedded(92.);
            if let Some(value) = selected.clone() {
                control.selected(value)
            } else {
                control
            }
        });
        let input = cx.new(|cx| TextEntry::new(cx, placeholder, "").font_size(12.));
        let select_subscription = cx.subscribe(
            &select,
            |this: &mut Self, select, event: &SelectEvent, cx| {
                let SelectEvent::Changed(value) = event;
                select.update(cx, |select, cx| {
                    select.set_selected(Some(value.clone()), cx)
                });
                this.value.selected = Some(value.clone());
                cx.emit(SelectFieldEvent::Changed(this.value.clone()));
                cx.notify();
            },
        );
        let input_subscription = cx.observe(&input, |this: &mut Self, input, cx| {
            let text: SharedString = input.read(cx).value().to_owned().into();
            if this.value.text != text {
                this.value.text = text;
                cx.emit(SelectFieldEvent::Changed(this.value.clone()));
                cx.notify();
            }
        });
        Self {
            select,
            input,
            value: SelectFieldValue {
                selected,
                text: "".into(),
            },
            theme: ThemePreset::default(),
            disabled: false,
            input_disabled: false,
            error: None,
            focus: cx.focus_handle(),
            focus_subscriptions: vec![],
            _subscriptions: vec![select_subscription, input_subscription],
        }
    }
    pub fn value(&self) -> &SelectFieldValue {
        &self.value
    }
    pub fn set_selected(&mut self, value: Option<SharedString>, cx: &mut Context<Self>) {
        self.value.selected = value.clone();
        self.select
            .update(cx, |select, cx| select.set_selected(value, cx));
        cx.notify();
    }
    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        let text = text.into();
        self.value.text = text.clone();
        self.input.update(cx, |input, cx| input.set_value(text, cx));
        cx.notify();
    }
    /// Display resolved reference badges in the text portion, preserving the raw field value.
    pub fn set_reference_resolver(
        &mut self,
        resolver: Option<TextReferenceResolver>,
        cx: &mut Context<Self>,
    ) {
        self.input
            .update(cx, |input, cx| input.set_reference_resolver(resolver, cx));
    }
    pub fn set_options(
        &mut self,
        options: impl IntoIterator<Item = SelectOption>,
        cx: &mut Context<Self>,
    ) {
        self.select
            .update(cx, |select, cx| select.set_options(options, cx));
        cx.notify();
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        self.theme = theme;
        self.input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        self.select
            .update(cx, |select, cx| select.set_theme(theme, cx));
        cx.notify();
    }
    /// Disable only the text input, keeping the dropdown interactive.
    pub fn input_disabled(mut self, disabled: bool) -> Self {
        self.input_disabled = disabled;
        self
    }
    pub fn set_input_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.input_disabled != disabled {
            self.input_disabled = disabled;
            cx.notify();
        }
    }
    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        self.select
            .update(cx, |select, cx| select.set_disabled(disabled, cx));
        cx.notify();
    }
    pub fn set_error(&mut self, error: Option<SharedString>, cx: &mut Context<Self>) {
        self.error = error;
        cx.notify();
    }
}
impl Render for SelectField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        self.input
            .update(cx, |input, cx| input.set_theme(self.theme, cx));
        if self.focus_subscriptions.is_empty() {
            let focus = self.focus.clone();
            self.focus_subscriptions
                .push(cx.on_focus_in(&focus, window, |_, _, cx| cx.notify()));
            self.focus_subscriptions
                .push(cx.on_focus_out(&focus, window, |_, _, _, cx| cx.notify()));
        }
        let input_disabled = self.disabled || self.input_disabled;
        if self.input_disabled && !self.disabled && self.input.focus_handle(cx).is_focused(window) {
            window.focus(&self.select.focus_handle(cx), cx);
        }
        let p = self.theme.palette();
        let focused = self.focus.contains_focused(window, cx) && !self.disabled;
        let border = if self.error.is_some() {
            theme.color(0xff3b30)
        } else if focused {
            p.accent
        } else {
            p.border
        };
        let field = div()
            .h(px(32.))
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .rounded(px(7.))
            .border_1()
            .border_color(rgb(border))
            .bg(rgb(if self.disabled { p.panel } else { p.surface }))
            .when(focused, |el| {
                el.shadow(vec![BoxShadow {
                    inset: false,
                    color: gpui::rgb_to_hsla(rgb(if self.error.is_some() {
                        theme.color(0xffc6c1)
                    } else {
                        p.accent_tint
                    })),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(3.),
                }])
            })
            .child(
                div()
                    .h(px(30.))
                    .flex_shrink_0()
                    .border_r_1()
                    .border_color(rgb(p.border))
                    .child(self.select.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(24.))
                    .px(px(11.))
                    .when(!input_disabled, |el| el.child(self.input.clone()))
                    .when(input_disabled, |el| {
                        el.text_size(px(12.))
                            .text_color(rgb(p.text_muted))
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(self.value.text.clone())
                    }),
            );
        div()
            .id("glass-select-field")
            .track_focus(&self.focus)
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(field)
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(theme.color(0xff3b30)))
                        .child(error),
                )
            })
    }
}
