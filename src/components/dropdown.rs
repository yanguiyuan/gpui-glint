use crate::theme::ThemePreset;
use gpui::{
    App, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, deferred, div, px, rgb, rgba,
};
use std::rc::Rc;

use super::Icon;

/// A single selectable row in a grouped dropdown menu.
pub struct DropdownItem {
    label: SharedString,
    icon: Option<Icon>,
}

impl DropdownItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            icon: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// A titled group of dropdown items.
pub struct DropdownGroup {
    title: SharedString,
    items: Vec<DropdownItem>,
}

impl DropdownGroup {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            items: Vec::new(),
        }
    }

    pub fn item(mut self, item: DropdownItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn items(mut self, items: impl IntoIterator<Item = DropdownItem>) -> Self {
        self.items.extend(items);
        self
    }
}

/// An interactive dropdown that organizes menu items under group headings.
pub struct GroupedDropdown {
    theme: ThemePreset,
    label: SharedString,
    trigger_icon: Option<Icon>,
    groups: Vec<DropdownGroup>,
    selected: Option<SharedString>,
    show_selected_in_trigger: bool,
    is_open: bool,
    open_on_start: bool,
    align_right: bool,
    show_selection: bool,
    on_select: Option<Rc<dyn Fn(SharedString, &mut Window, &mut App)>>,
}

impl GroupedDropdown {
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

    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            theme: ThemePreset::default(),
            label: label.into(),
            trigger_icon: None,
            groups: Vec::new(),
            selected: None,
            show_selected_in_trigger: false,
            is_open: false,
            open_on_start: false,
            align_right: false,
            show_selection: true,
            on_select: None,
        }
    }

    pub fn trigger_icon(mut self, icon: Icon) -> Self {
        self.trigger_icon = Some(icon);
        self
    }

    pub fn group(mut self, group: DropdownGroup) -> Self {
        self.groups.push(group);
        self
    }

    /// Aligns the menu to the right edge of its trigger, useful in application topbars.
    pub fn align_right(mut self) -> Self {
        self.align_right = true;
        self
    }

    /// Action menus can omit the selection highlight used by value selectors.
    pub fn show_selection(mut self, show: bool) -> Self {
        self.show_selection = show;
        self
    }

    /// Updates contextual actions while retaining the menu's open state.
    pub fn set_groups(
        &mut self,
        groups: impl IntoIterator<Item = DropdownGroup>,
        cx: &mut Context<Self>,
    ) {
        self.groups = groups.into_iter().collect();
        cx.notify();
    }

    pub fn selected(mut self, label: impl Into<SharedString>) -> Self {
        self.selected = Some(label.into());
        self
    }

    /// Appends the selected item to the trigger label, useful for sorting and filtering controls.
    pub fn show_selected_in_trigger(mut self, show: bool) -> Self {
        self.show_selected_in_trigger = show;
        self
    }

    /// Opens the menu on its first render. Useful for component showcases.
    pub fn open_on_start(mut self) -> Self {
        self.open_on_start = true;
        self
    }

    pub fn on_select(
        mut self,
        callback: impl Fn(SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select = Some(Rc::new(callback));
        self
    }

    /// Closes the menu. Call this from an owner-level outside-click handler.
    pub fn close(&mut self) {
        self.is_open = false;
    }
}

impl Render for GroupedDropdown {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        if self.open_on_start {
            self.is_open = true;
            self.open_on_start = false;
        }

        let mut trigger = div()
            .id("glass-grouped-dropdown-trigger")
            .h(px(30.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(theme.color(0xd5dae2)))
            .bg(rgb(theme.color(0xfafcff)))
            .text_color(rgb(theme.color(0x48484a)))
            .text_size(px(13.))
            .cursor_pointer()
            .hover(move |style| style.bg(rgb(theme.color(0xf0f4f9))))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| {
                    cx.stop_propagation();
                }),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.is_open = !this.is_open;
                cx.notify();
            }));

        if let Some(icon) = self.trigger_icon {
            trigger = trigger.child(icon.render(16.));
        }
        let trigger_label = if self.show_selected_in_trigger {
            self.selected
                .as_ref()
                .map(|selected| SharedString::from(format!("{}{}", self.label, selected)))
                .unwrap_or_else(|| self.label.clone())
        } else {
            self.label.clone()
        };
        trigger = trigger
            .child(trigger_label)
            .child(Icon::ChevronDown.render(13.));

        let mut root = div().relative().flex().child(trigger);

        if self.is_open {
            let mut menu = div()
                .id("glass-grouped-dropdown-menu-surface")
                .absolute()
                .top(px(36.))
                .w(px(280.))
                .flex()
                .flex_col()
                .p_2()
                .rounded(px(15.))
                .border_1()
                .border_color(if theme.is_dark() {
                    rgba(theme.glass_line())
                } else {
                    rgb(theme.color(0xe4e6e9))
                })
                .bg(if theme.is_dark() {
                    rgba(theme.menu_material())
                } else {
                    rgb(theme.color(0xfbfbfd))
                })
                .shadow_lg()
                // Keep wheel events inside the menu from scrolling the page
                // content behind it.
                .occlude()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.is_open = false;
                    cx.notify();
                }));
            if theme.is_dark() {
                menu = menu.backdrop_blur(px(48.));
            }
            menu = if self.align_right {
                menu.right_0()
            } else {
                menu.left_0()
            };

            for (group_index, group) in self.groups.iter().enumerate() {
                if group_index > 0 {
                    menu = menu.child(div().mx_1().my_1().h(px(1.)).bg(if theme.is_dark() {
                        rgba(0xffffff14)
                    } else {
                        rgb(theme.color(0xe3e5e8))
                    }));
                }

                menu = menu.child(
                    div()
                        .px_2()
                        .pt_1()
                        .pb_1()
                        .text_size(px(11.5))
                        .text_color(rgb(theme.color(0x8e8e93)))
                        .child(group.title.clone()),
                );

                for (item_index, item) in group.items.iter().enumerate() {
                    let label = item.label.clone();
                    let selected =
                        self.show_selection && self.selected.as_ref() == Some(&item.label);
                    let mut row = div()
                        .id(SharedString::from(format!(
                            "glass-grouped-dropdown-{group_index}-{item_index}"
                        )))
                        .h(px(34.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded(px(6.))
                        .text_size(px(13.))
                        .text_color(rgb(if selected {
                            theme.color(0x0874e8)
                        } else {
                            theme.color(0x48484a)
                        }))
                        .cursor_pointer()
                        .hover(move |style| {
                            style.bg(if theme.is_dark() {
                                rgba(0xffffff14)
                            } else {
                                rgb(theme.color(0xf0f2f4))
                            })
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _, _, cx| {
                                cx.stop_propagation();
                            }),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.selected = Some(label.clone());
                            this.is_open = false;
                            if let Some(on_select) = &this.on_select {
                                on_select(label.clone(), window, cx);
                            }
                            cx.notify();
                        }));

                    if let Some(icon) = item.icon {
                        row = row.child(
                            div()
                                .w(px(18.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon.render(16.)),
                        );
                    }
                    row = row.child(item.label.clone());
                    menu = menu.child(row);
                }
            }

            root = root.child(deferred(menu).with_priority(10));
        }

        root
    }
}
