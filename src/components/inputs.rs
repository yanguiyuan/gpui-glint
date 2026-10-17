use crate::theme::ThemePreset;
use std::rc::Rc;

use super::Icon;
use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px, rgb,
};

type ChangeHandler<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

/// A styled text field. Text editing can be connected by composing it with a GPUI input handler.
pub struct InputField {
    theme: ThemePreset,
    label: Option<SharedString>,
    value: Option<SharedString>,
    placeholder: SharedString,
    control: Option<AnyElement>,
    error: Option<SharedString>,
    width: f32,
    focused: bool,
}

impl InputField {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn new(placeholder: impl Into<SharedString>) -> Self {
        Self {
            theme: ThemePreset::default(),
            label: None,
            value: None,
            placeholder: placeholder.into(),
            control: None,
            error: None,
            width: 260.,
            focused: false,
        }
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn value(mut self, value: impl Into<SharedString>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Replaces the sample text with a custom GPUI input element.
    pub fn control(mut self, control: impl IntoElement) -> Self {
        self.control = Some(control.into_any_element());
        self
    }

    pub fn error(mut self, message: impl Into<SharedString>) -> Self {
        self.error = Some(message.into());
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(0.);
        self
    }

    pub fn render(self) -> gpui::Div {
        let theme = self.theme;
        let invalid = self.error.is_some();
        let focused = self.focused && !invalid;
        let mut field = div()
            .w(px(self.width))
            .flex()
            .flex_col()
            .gap_1()
            .text_size(px(13.));

        if let Some(label) = self.label {
            field = field.child(
                div()
                    .pl(px(2.))
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(rgb(theme.color(0x555b59)))
                    .child(label),
            );
        }

        let control = if let Some(control) = self.control {
            div()
                .h(px(32.))
                .w_full()
                .px(px(11.))
                .flex()
                .items_center()
                .rounded(px(7.))
                .border_1()
                .border_color(rgb(if invalid {
                    theme.color(0xff3b30)
                } else if focused {
                    theme.color(0x087bfa)
                } else {
                    theme.color(0xe1e5e2)
                }))
                .bg(rgb(theme.color(0xfcfdfc)))
                .shadow_sm()
                .child(control)
        } else {
            div()
                .h(px(32.))
                .w_full()
                .px(px(11.))
                .flex()
                .items_center()
                .rounded(px(7.))
                .border_1()
                .border_color(rgb(if invalid {
                    theme.color(0xff3b30)
                } else if focused {
                    theme.color(0x087bfa)
                } else {
                    theme.color(0xe1e5e2)
                }))
                .bg(rgb(theme.color(0xfcfdfc)))
                .shadow_sm()
                .text_color(rgb(if self.value.is_some() {
                    theme.color(0x292d32)
                } else {
                    theme.color(0x92979a)
                }))
                .child(self.value.unwrap_or(self.placeholder))
        };
        let control = if invalid || focused {
            div()
                .w_full()
                .p(px(3.5))
                .rounded(px(11.))
                .bg(rgb(if invalid {
                    theme.color(0xffe0dc)
                } else {
                    theme.color(0xdcecff)
                }))
                .child(control)
        } else {
            control
        };
        field = field.child(control);

        if let Some(error) = self.error {
            field = field.child(
                div()
                    .pl(px(2.))
                    .text_size(px(11.5))
                    .text_color(rgb(theme.color(0xff3b30)))
                    .child(error),
            );
        }

        field
    }
}

/// Search field with the compact key-hint treatment used by the Glass UI reference.
pub struct SearchInput {
    theme: ThemePreset,
    placeholder: SharedString,
    control: Option<AnyElement>,
    shortcut: Option<SharedString>,
    width: f32,
    focused: bool,
}

impl SearchInput {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn new(placeholder: impl Into<SharedString>) -> Self {
        Self {
            theme: ThemePreset::default(),
            placeholder: placeholder.into(),
            control: None,
            shortcut: None,
            width: 250.,
            focused: false,
        }
    }

    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// Replaces the placeholder with a custom GPUI input element.
    pub fn control(mut self, control: impl IntoElement) -> Self {
        self.control = Some(control.into_any_element());
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(0.);
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn render(self) -> gpui::Div {
        let theme = self.theme;
        let mut field = div()
            .w(px(self.width))
            .min_w_0()
            .h(px(30.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(7.))
            .border_1()
            .border_color(rgb(if self.focused {
                theme.color(0x087bfa)
            } else {
                theme.color(0xe1e5e2)
            }))
            .bg(rgb(theme.color(0xfcfdfc)))
            .shadow_sm()
            .text_size(px(13.))
            .text_color(rgb(theme.color(0x92979a)))
            .child(Icon::Search.render(13.).flex_shrink_0());

        field = field.child(if let Some(control) = self.control {
            div().flex_1().min_w_0().child(control)
        } else {
            div().flex_1().min_w_0().truncate().child(self.placeholder)
        });

        if let Some(shortcut) = self.shortcut {
            field = field.child(
                div()
                    .px(px(4.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(theme.color(0xe1e5e2)))
                    .text_size(px(10.))
                    .child(shortcut),
            );
        }
        field
    }
}

#[derive(Clone)]
pub struct SegmentOption {
    pub label: SharedString,
    pub count: Option<SharedString>,
}

impl SegmentOption {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            count: None,
        }
    }

    pub fn count(mut self, count: impl Into<SharedString>) -> Self {
        self.count = Some(count.into());
        self
    }
}

/// macOS-style segmented control with a recessed track and raised selected thumb.
pub struct SegmentedControl {
    id: ElementId,
    theme: ThemePreset,
    options: Vec<SegmentOption>,
    selected: usize,
    tinted: bool,
    on_change: Option<ChangeHandler<usize>>,
}

impl SegmentedControl {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    #[track_caller]
    pub fn new(options: impl IntoIterator<Item = SegmentOption>) -> Self {
        Self {
            id: ElementId::CodeLocation(*std::panic::Location::caller()),
            theme: ThemePreset::default(),
            options: options.into_iter().collect(),
            selected: 0,
            tinted: false,
            on_change: None,
        }
    }

    /// Stable identity for controls created in a loop or by the same helper.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }

    pub fn selected(mut self, selected: usize) -> Self {
        self.selected = selected;
        self
    }

    pub fn tinted(mut self) -> Self {
        self.tinted = true;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }

    pub fn render(self) -> gpui::Div {
        let theme = self.theme;
        let mut track = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(theme.color(0xdfe3df)))
            .bg(rgb(theme.color(0xe9ece9)));

        for (index, option) in self.options.into_iter().enumerate() {
            let selected = index == self.selected;
            let color = if selected && self.tinted {
                0xffffff
            } else if selected {
                theme.color(0x292d32)
            } else {
                theme.color(0x555b59)
            };
            let background = if selected && self.tinted {
                theme.color(0x087bfa)
            } else if selected {
                if theme.is_dark() {
                    0x5a5a5e
                } else {
                    theme.color(0xfcfdfc)
                }
            } else {
                theme.color(0xe9ece9)
            };

            let mut button = div()
                .id(ElementId::NamedChild(
                    std::sync::Arc::new(self.id.clone()),
                    format!("glass-segment-{index}").into(),
                ))
                .h(px(24.))
                .px(px(11.))
                .flex()
                .items_center()
                .gap_1()
                .rounded(px(6.))
                .bg(rgb(background))
                .text_size(px(12.5))
                .text_color(rgb(color))
                .child(option.label);

            if let Some(count) = option.count {
                button = button.child(
                    div()
                        .text_size(px(10.5))
                        .font_family("monospace")
                        .text_color(rgb(if selected && self.tinted {
                            theme.color(0xcfe5ff)
                        } else {
                            theme.color(0x92979a)
                        }))
                        .child(count),
                );
            }

            if selected {
                button = button.shadow_sm();
            }
            if let Some(on_change) = self.on_change.clone() {
                button = button
                    .cursor_pointer()
                    .on_click(move |_: &ClickEvent, window, cx| {
                        on_change(index, window, cx);
                    });
            }
            track = track.child(button);
        }
        track
    }
}

impl IntoElement for SegmentedControl {
    type Element = gpui::Div;

    fn into_element(self) -> Self::Element {
        self.render()
    }
}

/// The square check control on its own, for reuse outside a labeled row.
pub fn checkbox_control(checked: bool, indeterminate: bool) -> gpui::Div {
    checkbox_control_themed(checked, indeterminate, ThemePreset::default())
}

pub fn checkbox_control_themed(
    checked: bool,
    indeterminate: bool,
    theme: ThemePreset,
) -> gpui::Div {
    let active = checked || indeterminate;
    let mark = if indeterminate { "−" } else { "✓" };
    let mut control = div()
        .w(px(16.))
        .h(px(16.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .border_1()
        .border_color(rgb(if active {
            theme.color(0x087bfa)
        } else {
            theme.color(0xdfe3df)
        }))
        .bg(rgb(if checked {
            theme.color(0x087bfa)
        } else {
            theme.color(0xfcfdfc)
        }))
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(rgb(if checked {
            0xffffff
        } else {
            theme.color(0x555b59)
        }));
    if active {
        control = control.child(mark);
    }
    control
}

pub struct Checkbox {
    theme: ThemePreset,
    label: SharedString,
    checked: bool,
    indeterminate: bool,
    on_change: Option<ChangeHandler<bool>>,
}

impl Checkbox {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            theme: ThemePreset::default(),
            label: label.into(),
            checked: false,
            indeterminate: false,
            on_change: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn indeterminate(mut self, indeterminate: bool) -> Self {
        self.indeterminate = indeterminate;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }

    pub fn render(self) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        let mut row = div()
            .id(SharedString::from(format!("glass-checkbox-{}", self.label)))
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(13.5))
            .text_color(rgb(theme.color(0x555b59)))
            .child(checkbox_control_themed(
                self.checked,
                self.indeterminate,
                self.theme,
            ));
        if !self.label.is_empty() {
            row = row.child(self.label);
        }

        if let Some(on_change) = self.on_change {
            row = row
                .cursor_pointer()
                .on_click(move |_: &ClickEvent, window, cx| {
                    on_change(!self.checked, window, cx);
                });
        }
        row
    }
}

pub struct RadioOption {
    theme: ThemePreset,
    label: SharedString,
    selected: bool,
    on_select: Option<ChangeHandler<()>>,
}

impl RadioOption {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            theme: ThemePreset::default(),
            label: label.into(),
            selected: false,
            on_select: None,
        }
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn on_select(mut self, handler: impl Fn((), &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }

    pub fn render(self) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        let mut control = div()
            .w(px(16.))
            .h(px(16.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(999.))
            .border_1()
            .border_color(rgb(if self.selected {
                if theme.is_dark() {
                    0xffffff
                } else {
                    theme.color(0x087bfa)
                }
            } else {
                theme.color(0xdfe3df)
            }))
            .bg(rgb(if self.selected {
                if theme.is_dark() {
                    0xffffff
                } else {
                    theme.color(0x087bfa)
                }
            } else {
                theme.color(0xfcfdfc)
            }));
        if self.selected {
            control = control.child(
                div()
                    .w(px(5.))
                    .h(px(5.))
                    .rounded(px(999.))
                    .bg(rgb(0xffffff)),
            );
        }
        let mut row = div()
            .id(SharedString::from(format!("glass-radio-{}", self.label)))
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(13.5))
            .text_color(rgb(theme.color(0x555b59)))
            .child(control)
            .child(self.label);
        if let Some(on_select) = self.on_select {
            row = row
                .cursor_pointer()
                .on_click(move |_: &ClickEvent, window, cx| {
                    on_select((), window, cx);
                });
        }
        row
    }
}

pub struct Switch {
    theme: ThemePreset,
    label: SharedString,
    checked: bool,
    on_change: Option<ChangeHandler<bool>>,
}

impl Switch {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            theme: ThemePreset::default(),
            label: label.into(),
            checked: false,
            on_change: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }

    pub fn render(self) -> gpui::Stateful<gpui::Div> {
        let theme = self.theme;
        let mut track = div()
            .w(px(40.))
            .h(px(24.))
            .p(px(2.))
            .flex()
            .items_center()
            .justify_start()
            .rounded(px(999.))
            .bg(rgb(if self.checked {
                0x34c759
            } else {
                theme.color(0xe8ebe8)
            }));
        if self.checked {
            track = track.child(div().flex_1());
        }
        track = track.child(
            div()
                .w(px(20.))
                .h(px(20.))
                .rounded(px(999.))
                .bg(rgb(0xffffff))
                .shadow_md(),
        );
        let mut row = div()
            .id(SharedString::from(format!("glass-switch-{}", self.label)))
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(13.5))
            .text_color(rgb(theme.color(0x555b59)))
            .child(track)
            .child(self.label);
        if let Some(on_change) = self.on_change {
            row = row
                .cursor_pointer()
                .on_click(move |_: &ClickEvent, window, cx| {
                    on_change(!self.checked, window, cx);
                });
        }
        row
    }
}

pub struct Slider {
    theme: ThemePreset,
    value: f32,
    width: f32,
    on_change: Option<ChangeHandler<f32>>,
    drag_start: Option<(gpui::Pixels, f32)>,
}

impl Slider {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            cx.notify();
        }
    }

    pub fn new(value: f32) -> Self {
        Self {
            theme: ThemePreset::default(),
            value: value.clamp(0., 100.),
            width: 190.,
            on_change: None,
            drag_start: None,
        }
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(24.);
        self
    }

    pub fn on_change(mut self, handler: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }

    fn begin_drag(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.drag_start = Some((event.position.x, self.value));
        cx.notify();
    }

    fn drag(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !event.dragging() {
            return;
        }
        let Some((start_x, start_value)) = self.drag_start else {
            return;
        };

        let travel = (self.width - 18.).max(1.);
        let delta = (event.position.x - start_x) / px(travel) * 100.;
        let value = (start_value + delta).clamp(0., 100.);
        if (value - self.value).abs() > 0.01 {
            self.value = value;
            if let Some(on_change) = &self.on_change {
                on_change(value, window, cx);
            }
            cx.notify();
        }
    }

    fn end_drag(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.drag_start.take().is_some() {
            cx.notify();
        }
    }
}

impl Render for Slider {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let thumb_left = (self.width - 18.) * self.value / 100.;
        div()
            .id("glass-slider")
            .w(px(self.width))
            .h(px(20.))
            .relative()
            .flex()
            .items_center()
            .cursor(gpui::CursorStyle::ResizeLeftRight)
            .on_mouse_move(cx.listener(Self::drag))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::end_drag))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::end_drag))
            .child(
                div()
                    .w_full()
                    .h(px(4.))
                    .rounded(px(999.))
                    .bg(rgb(theme.color(0xe3e7e3))),
            )
            .child(
                div()
                    .id("glass-slider-thumb")
                    .absolute()
                    .left(px(thumb_left))
                    .top(px(1.))
                    .w(px(18.))
                    .h(px(18.))
                    .rounded(px(999.))
                    .bg(rgb(0xffffff))
                    .border_1()
                    .border_color(rgb(theme.color(0xe8ebe8)))
                    .shadow_md()
                    .cursor(gpui::CursorStyle::OpenHand)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::begin_drag)),
            )
    }
}

pub struct Stepper {
    theme: ThemePreset,
    value: i32,
    min: i32,
    max: i32,
    on_change: Option<ChangeHandler<i32>>,
}

impl Stepper {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn new(value: i32) -> Self {
        Self {
            theme: ThemePreset::default(),
            value,
            min: i32::MIN,
            max: i32::MAX,
            on_change: None,
        }
    }

    pub fn range(mut self, min: i32, max: i32) -> Self {
        self.min = min.min(max);
        self.max = max.max(min);
        self.value = self.value.clamp(self.min, self.max);
        self
    }

    pub fn on_change(mut self, handler: impl Fn(i32, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }

    pub fn render(self) -> gpui::Div {
        let theme = self.theme;
        let mut root = div()
            .h(px(28.))
            .flex()
            .items_center()
            .rounded(px(7.))
            .border_1()
            .border_color(rgb(theme.color(0xe1e5e2)))
            .bg(rgb(theme.color(0xfcfdfc)))
            .shadow_sm();

        if let Some(on_change) = self.on_change.clone() {
            let next = (self.value - 1).max(self.min);
            root = root.child(
                div()
                    .id("glass-stepper-decrement")
                    .w(px(26.))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(theme.color(0xf0f3f0))))
                    .on_click(move |_: &ClickEvent, window, cx| on_change(next, window, cx))
                    .child("−"),
            );
        } else {
            root = root.child(div().w(px(26.)).flex().justify_center().child("−"));
        }

        root = root.child(
            div()
                .h_full()
                .min_w(px(44.))
                .px(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .border_x_1()
                .border_color(rgb(theme.color(0xe8ebe8)))
                .text_size(px(12.5))
                .font_family("monospace")
                .child(self.value.to_string()),
        );

        if let Some(on_change) = self.on_change {
            let next = (self.value + 1).min(self.max);
            root.child(
                div()
                    .id("glass-stepper-increment")
                    .w(px(26.))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(theme.color(0xf0f3f0))))
                    .on_click(move |_: &ClickEvent, window, cx| on_change(next, window, cx))
                    .child("+"),
            )
        } else {
            root.child(div().w(px(26.)).flex().justify_center().child("+"))
        }
    }
}

/// Inline tag chips with optional remove handlers.
pub struct TokenInput {
    theme: ThemePreset,
    tags: Vec<SharedString>,
    placeholder: SharedString,
    on_remove: Option<ChangeHandler<usize>>,
    width: f32,
}

impl TokenInput {
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }

    pub fn new(
        tags: impl IntoIterator<Item = impl Into<SharedString>>,
        placeholder: impl Into<SharedString>,
    ) -> Self {
        Self {
            theme: ThemePreset::default(),
            tags: tags.into_iter().map(Into::into).collect(),
            placeholder: placeholder.into(),
            on_remove: None,
            width: 300.,
        }
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(0.);
        self
    }

    pub fn on_remove(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_remove = Some(Rc::new(handler));
        self
    }

    pub fn render(self) -> gpui::Div {
        let theme = self.theme;
        let mut root = div()
            .w(px(self.width))
            .min_h(px(32.))
            .px(px(6.))
            .py(px(4.))
            .flex()
            .items_center()
            .gap(px(5.))
            .rounded(px(7.))
            .border_1()
            .border_color(rgb(theme.color(0xe1e5e2)))
            .bg(rgb(theme.color(0xfcfdfc)))
            .shadow_sm();

        for (index, tag) in self.tags.into_iter().enumerate() {
            let mut chip = div()
                .id(SharedString::from(format!("glass-token-{index}")))
                .h(px(22.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap_1()
                .rounded(px(6.))
                .bg(rgb(theme.color(0xe3f0ff)))
                .text_size(px(12.5))
                .text_color(rgb(theme.color(0x0874e8)))
                .child(tag);
            if let Some(on_remove) = self.on_remove.clone() {
                chip = chip
                    .cursor_pointer()
                    .on_click(move |_: &ClickEvent, window, cx| {
                        on_remove(index, window, cx);
                    });
            }
            chip = chip.child(div().text_color(rgb(theme.color(0x6aa9f4))).child("×"));
            root = root.child(chip);
        }
        root.child(
            div()
                .flex_1()
                .text_size(px(13.))
                .text_color(rgb(theme.color(0x92979a)))
                .child(self.placeholder),
        )
    }
}
