use super::{Avatar, Button, ButtonKind, ButtonSize, Icon, Popover};
use crate::theme::ThemePreset;
use gpui::prelude::FluentBuilder;
use gpui::{
    Animation, AnimationExt as _, Context, EventEmitter, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px, rgb,
    rgba,
};
use std::collections::HashSet;

/// A group header in a [`GroupList`]: an icon + title row whose trailing
/// actions appear only while hovered. `Folder`/`Folders` header icons follow
/// the group's expand state (closed while collapsed, open once expanded).
#[derive(Clone, Debug)]
pub struct GroupListHeader {
    pub id: SharedString,
    pub icon: Option<Icon>,
    pub avatar: Option<Avatar>,
    pub label: SharedString,
    pub actions: Vec<Icon>,
}

impl GroupListHeader {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            avatar: None,
            label: label.into(),
            actions: Vec::new(),
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Optional leading avatar, displayed in place of the header icon.
    pub fn avatar(mut self, avatar: Avatar) -> Self {
        self.avatar = Some(avatar);
        self
    }

    /// Trailing action icons revealed while the header is hovered.
    pub fn actions(mut self, actions: impl IntoIterator<Item = Icon>) -> Self {
        self.actions = actions.into_iter().collect();
        self
    }
}

/// One entry inside a [`GroupListGroup`]. While hovered, the leading icon may
/// swap to `hover_icon` and the trailing meta may swap to an `action` icon.
#[derive(Clone, Debug)]
pub struct GroupListItem {
    pub id: SharedString,
    pub icon: Option<Icon>,
    pub label: SharedString,
    pub meta: Option<SharedString>,
    pub prefix: Option<SharedString>,
    pub prefix_color: Option<u32>,
    pub hover_icon: Option<Icon>,
    pub reserve_icon_space: bool,
    pub action: Option<Icon>,
    pub action_confirmation: Option<SharedString>,
}

impl GroupListItem {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            label: label.into(),
            meta: None,
            prefix: None,
            prefix_color: None,
            hover_icon: None,
            reserve_icon_space: false,
            action: None,
            action_confirmation: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Keeps a 14px icon slot even when no leading icon is visible.
    pub fn reserve_icon_space(mut self, reserve: bool) -> Self {
        self.reserve_icon_space = reserve;
        self
    }

    /// A short leading label, such as an HTTP method.
    pub fn prefix(mut self, text: impl Into<SharedString>) -> Self {
        self.prefix = Some(text.into());
        self
    }
    pub fn prefix_color(mut self, color: u32) -> Self {
        self.prefix_color = Some(color);
        self
    }
    pub fn meta(mut self, meta: impl Into<SharedString>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    /// Leading icon shown in place of `icon` while the row is hovered,
    /// e.g. a pin shortcut.
    pub fn hover_icon(mut self, icon: Icon) -> Self {
        self.hover_icon = Some(icon);
        self
    }

    /// Requires confirmation in a popover before emitting ItemAction.
    pub fn confirm_action(mut self, title: impl Into<SharedString>) -> Self {
        self.action_confirmation = Some(title.into());
        self
    }

    /// Trailing action icon shown in place of `meta` while the row is hovered,
    /// e.g. a delete shortcut.
    pub fn action(mut self, icon: Icon) -> Self {
        self.action = Some(icon);
        self
    }
}

/// A titled group of [`GroupListItem`]s.
#[derive(Clone, Debug)]
pub struct GroupListGroup {
    pub header: GroupListHeader,
    pub items: Vec<GroupListItem>,
}

impl GroupListGroup {
    pub fn new(header: GroupListHeader) -> Self {
        Self {
            header,
            items: Vec::new(),
        }
    }

    pub fn item(mut self, item: GroupListItem) -> Self {
        self.items.push(item);
        self
    }
}

#[derive(Clone, Debug)]
pub enum GroupListEvent {
    /// A group header row was clicked.
    HeaderClicked { group: SharedString },
    /// One of a header's hover actions was clicked, by its position in
    /// [`GroupListHeader::actions`].
    HeaderAction { group: SharedString, index: usize },
    /// An item was clicked; the list updates its own selection first.
    ItemSelected {
        group: SharedString,
        item: SharedString,
    },
    /// An item's hover action was clicked, e.g. the delete shortcut.
    ItemAction {
        group: SharedString,
        item: SharedString,
    },
}

/// A sidebar source list: titled groups with icon + label + meta rows,
/// quiet hover states that reveal contextual actions, and a highlighted
/// selection pill. Item ids are expected to be unique across groups.
pub struct GroupList {
    groups: Vec<GroupListGroup>,
    selected: Option<SharedString>,
    hovered: Option<SharedString>,
    confirming: Option<SharedString>,
    collapsed: HashSet<SharedString>,
    theme: ThemePreset,
    width: Option<f32>,
}

impl EventEmitter<GroupListEvent> for GroupList {}

impl GroupList {
    pub fn new(groups: impl IntoIterator<Item = GroupListGroup>) -> Self {
        Self {
            groups: groups.into_iter().collect(),
            selected: None,
            hovered: None,
            confirming: None,
            collapsed: HashSet::new(),
            theme: ThemePreset::default(),
            width: None,
        }
    }

    pub fn selected(mut self, selected: Option<SharedString>) -> Self {
        self.selected = selected;
        self
    }

    /// Group ids that start collapsed; clicking a header toggles its group.
    pub fn collapsed(mut self, groups: impl IntoIterator<Item = impl Into<SharedString>>) -> Self {
        self.collapsed = groups.into_iter().map(Into::into).collect();
        self
    }

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

    /// Fixed width in pixels; by default the list fills its container.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width.max(0.));
        self
    }

    pub fn set_selected(&mut self, selected: Option<SharedString>, cx: &mut Context<Self>) {
        self.selected = selected;
        cx.notify();
    }

    /// The currently selected item id, if any.
    pub fn selected_item(&self) -> Option<&SharedString> {
        self.selected.as_ref()
    }

    /// Whether a group is currently collapsed.
    pub fn is_collapsed(&self, group: &SharedString) -> bool {
        self.collapsed.contains(group)
    }

    pub fn set_collapsed(
        &mut self,
        group: impl Into<SharedString>,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) {
        let group = group.into();
        if collapsed {
            self.collapsed.insert(group);
        } else {
            self.collapsed.remove(&group);
        }
        cx.notify();
    }

    /// Flips a group's collapsed state; the header click does this already.
    pub fn toggle_group(&mut self, group: impl Into<SharedString>, cx: &mut Context<Self>) {
        let group = group.into();
        if !self.collapsed.remove(&group) {
            self.collapsed.insert(group);
        }
        cx.notify();
    }

    pub fn set_groups(
        &mut self,
        groups: impl IntoIterator<Item = GroupListGroup>,
        cx: &mut Context<Self>,
    ) {
        self.groups = groups.into_iter().collect();
        if self.confirming.as_ref().is_some_and(|id| {
            !self
                .groups
                .iter()
                .any(|group| group.items.iter().any(|item| &item.id == id))
        }) {
            self.confirming = None;
        }
        cx.notify();
    }

    fn hovered_row(&mut self, hovered: Option<SharedString>, cx: &mut Context<Self>) {
        if self.hovered != hovered {
            self.hovered = hovered;
            cx.notify();
        }
    }
}

impl Render for GroupList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let p = theme.palette();
        let dark = theme.is_dark();
        let hover_bg = if dark {
            rgba(0xffffff14)
        } else {
            rgb(p.surface_subtle)
        };
        let selected_bg = if dark {
            rgba(0xffffff1f)
        } else {
            rgb(p.surface_subtle)
        };

        let mut root = div()
            .id("glass-group-list")
            .when_some(self.width, |list, width| list.w(px(width)))
            .flex()
            .flex_col()
            .gap(px(10.))
            .text_size(px(13.));

        for group in &self.groups {
            let group_id = group.header.id.clone();
            let header_key = SharedString::from(format!("h:{}", group_id));
            let header_hovered = self.hovered.as_ref() == Some(&header_key);
            let mut header = div()
                .id(header_key.clone())
                .w_full()
                .h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(7.))
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(p.text_muted))
                .when(header_hovered && !group.header.actions.is_empty(), |row| {
                    row.bg(hover_bg)
                })
                .on_hover(cx.listener({
                    let key = header_key.clone();
                    move |this, hovered: &bool, _, cx| {
                        this.hovered_row(hovered.then(|| key.clone()), cx);
                    }
                }))
                .on_click(cx.listener({
                    let group_id = group_id.clone();
                    move |this, _, _, cx| {
                        this.toggle_group(group_id.clone(), cx);
                        cx.emit(GroupListEvent::HeaderClicked {
                            group: group_id.clone(),
                        });
                        cx.notify();
                    }
                }));
            if let Some(avatar) = &group.header.avatar {
                header = header.child(avatar.clone().size(18.).theme(self.theme).render());
            } else if let Some(icon) = group.header.icon {
                // Folder headers follow the expand state like the directory
                // tree: closed folder while collapsed, open folder once shown.
                let icon = match icon {
                    Icon::Folder | Icon::Folders if self.collapsed.contains(&group_id) => {
                        Icon::Folder
                    }
                    Icon::Folder | Icon::Folders => Icon::FolderOpen,
                    other => other,
                };
                header = header.child(icon.render(15.));
            }
            header = header.child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .child(group.header.label.clone()),
            );
            if header_hovered {
                for (index, icon) in group.header.actions.iter().enumerate() {
                    header = header.child(
                        div()
                            .id(SharedString::from(format!(
                                "glass-group-list-header-action-{}-{index}",
                                group_id
                            )))
                            .size(px(22.))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .text_color(rgb(p.text_muted))
                            .cursor_pointer()
                            .hover(move |style| {
                                style
                                    .bg(if dark {
                                        rgba(0xffffff1f)
                                    } else {
                                        rgb(p.border)
                                    })
                                    .text_color(rgb(p.text))
                            })
                            .on_click(cx.listener({
                                let group_id = group_id.clone();
                                move |_, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.emit(GroupListEvent::HeaderAction {
                                        group: group_id.clone(),
                                        index,
                                    });
                                    cx.notify();
                                }
                            }))
                            .child(icon.render(14.)),
                    );
                }
            }
            let mut section = div().flex().flex_col().gap(px(2.)).child(header);

            if self.collapsed.contains(&group_id) {
                root = root.child(section);
                continue;
            }

            for item in &group.items {
                let item_id = item.id.clone();
                let item_key = SharedString::from(format!("i:{}", item_id));
                let item_hovered = self.hovered.as_ref() == Some(&item_key);
                let is_selected = self.selected.as_ref() == Some(&item_id);
                let mut row = div()
                    .id(item_key.clone())
                    .w_full()
                    .h(px(34.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(7.))
                    .text_size(px(13.))
                    .text_color(rgb(p.text))
                    .cursor_pointer()
                    .when(is_selected, |row| row.bg(selected_bg))
                    .when(item_hovered && !is_selected, |row| row.bg(hover_bg))
                    .on_hover(cx.listener({
                        let key = item_key.clone();
                        move |this, hovered: &bool, _, cx| {
                            this.hovered_row(hovered.then(|| key.clone()), cx);
                        }
                    }))
                    .on_click(cx.listener({
                        let group_id = group_id.clone();
                        let item_id = item_id.clone();
                        move |this, _, _, cx| {
                            this.selected = Some(item_id.clone());
                            cx.emit(GroupListEvent::ItemSelected {
                                group: group_id.clone(),
                                item: item_id.clone(),
                            });
                            cx.notify();
                        }
                    }));
                let leading_icon = if item_hovered {
                    item.hover_icon.or(item.icon)
                } else {
                    item.icon
                };
                if let Some(icon) = leading_icon {
                    let leading = div()
                        .size(px(14.))
                        .flex_shrink_0()
                        .text_color(rgb(p.text_muted));
                    row = if matches!(icon, Icon::Spinner) {
                        row.child(leading.child(icon.render(14.).with_animation(
                            SharedString::from(format!("group-list-running:{}", item.id)),
                            Animation::new(std::time::Duration::from_millis(900)).repeat(),
                            |icon, delta| {
                                icon.with_transformation(gpui::Transformation::rotate(
                                    gpui::radians(std::f32::consts::TAU * delta),
                                ))
                            },
                        )))
                    } else {
                        row.child(leading.child(icon.render(14.)))
                    };
                } else if item.reserve_icon_space {
                    row = row.child(div().size(px(14.)).flex_shrink_0());
                }
                if let Some(prefix) = &item.prefix {
                    row = row.child(
                        div()
                            .w(px(38.))
                            .flex_shrink_0()
                            .font_family("Menlo")
                            .text_size(px(10.))
                            .text_color(rgb(item.prefix_color.unwrap_or(p.text_muted)))
                            .child(prefix.clone()),
                    );
                }
                row = row.child(
                    div()
                        .id(SharedString::from(format!("group-list-label:{}", item.id)))
                        .debug_selector({
                            let id = item.id.clone();
                            move || format!("group-list-label:{id}").into()
                        })
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .child(item.label.clone()),
                );
                let trailing_action = (item_hovered || self.confirming.as_ref() == Some(&item_id))
                    .then_some(item.action)
                    .flatten();
                if let Some(action) = trailing_action {
                    let trigger = div()
                        .id(SharedString::from(format!(
                            "glass-group-list-item-action-{}",
                            item_id
                        )))
                        .debug_selector({
                            let id = item_id.clone();
                            move || format!("group-list-action:{id}").into()
                        })
                        .size(px(22.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .text_color(rgb(p.text_muted))
                        .cursor_pointer()
                        .hover(move |style| {
                            style
                                .bg(if dark {
                                    rgba(0xffffff1f)
                                } else {
                                    rgb(p.border)
                                })
                                .text_color(rgb(p.text))
                        })
                        .child(action.render(14.));
                    if let Some(title) = &item.action_confirmation {
                        let group = group_id.clone();
                        let id = item_id.clone();
                        let content = div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(title.clone()),
                            )
                            .child(
                                div()
                                    .text_color(rgb(p.text_muted))
                                    .child("删除后无法恢复。"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        Button::new("取消")
                                            .kind(ButtonKind::Gray)
                                            .size(ButtonSize::Small)
                                            .theme(theme)
                                            .render()
                                            .debug_selector(|| "group-list-cancel".into())
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.confirming = None;
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("删除")
                                            .kind(ButtonKind::Danger)
                                            .size(ButtonSize::Small)
                                            .theme(theme)
                                            .render()
                                            .debug_selector(|| "group-list-confirm".into())
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if this.confirming.as_ref() == Some(&id) {
                                                    this.confirming = None;
                                                    cx.emit(GroupListEvent::ItemAction {
                                                        group: group.clone(),
                                                        item: id.clone(),
                                                    });
                                                    cx.notify();
                                                }
                                            })),
                                    ),
                            );
                        let weak = cx.entity().downgrade();
                        let id = item_id.clone();
                        row = row.child(
                            Popover::new(
                                SharedString::from(format!("group-list-confirm:{item_id}")),
                                trigger,
                                content,
                            )
                            .open(self.confirming.as_ref() == Some(&item_id))
                            .width(240.)
                            .trigger_height(22.)
                            .theme(theme)
                            .on_open_change(move |open, _, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.confirming = open.then(|| id.clone());
                                    cx.notify();
                                });
                            }),
                        );
                    } else {
                        row = row.child(trigger.on_click(cx.listener({
                            let group = group_id.clone();
                            let item = item_id.clone();
                            move |_, _, _, cx| {
                                cx.stop_propagation();
                                cx.emit(GroupListEvent::ItemAction {
                                    group: group.clone(),
                                    item: item.clone(),
                                });
                            }
                        })));
                    }
                } else if let Some(meta) = &item.meta {
                    row = row.child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(12.))
                            .text_color(rgb(p.text_muted))
                            .child(meta.clone()),
                    );
                }
                section = section.child(row);
            }
            root = root.child(section);
        }

        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn action_popover_only_emits_after_confirmation(cx: &mut gpui::TestAppContext) {
        use std::{cell::Cell, rc::Rc};
        let (list, window) = cx.add_window_view(|_, _| {
            GroupList::new([
                GroupListGroup::new(GroupListHeader::new("project", "Project")).item(
                    GroupListItem::new("session", "Session")
                        .action(Icon::Trash)
                        .confirm_action("删除此会话？"),
                ),
            ])
            .width(260.)
        });
        let events = Rc::new(Cell::new(0));
        let received = events.clone();
        let _subscription = window.update(|_, cx| {
            cx.subscribe(&list, move |_, event, _| {
                if matches!(event, GroupListEvent::ItemAction { .. }) {
                    received.set(received.get() + 1);
                }
            })
        });
        list.update(window, |list, cx| {
            list.hovered = Some("i:session".into());
            cx.notify();
        });
        let action = window.debug_bounds("group-list-action:session").unwrap();
        window.simulate_click(action.center(), gpui::Modifiers::default());
        assert_eq!(events.get(), 0);
        assert!(list.read_with(window, |list, _| list.confirming.is_some()));
        let cancel = window.debug_bounds("group-list-cancel").unwrap();
        window.simulate_click(cancel.center(), gpui::Modifiers::default());
        assert_eq!(events.get(), 0);
        list.update(window, |list, cx| {
            list.hovered = Some("i:session".into());
            cx.notify();
        });
        let action = window.debug_bounds("group-list-action:session").unwrap();
        window.simulate_click(action.center(), gpui::Modifiers::default());
        let confirm = window.debug_bounds("group-list-confirm").unwrap();
        window.simulate_click(confirm.center(), gpui::Modifiers::default());
        assert_eq!(events.get(), 1);
    }

    #[gpui::test]
    fn reserved_icon_slot_keeps_labels_stable_when_running(cx: &mut gpui::TestAppContext) {
        let groups = |running| {
            let mut item = GroupListItem::new("session", "Session").reserve_icon_space(true);
            if running {
                item = item.icon(Icon::Spinner);
            }
            [GroupListGroup::new(GroupListHeader::new("project", "Project")).item(item)]
        };
        let (list, window) = cx.add_window_view(|_, _| GroupList::new(groups(false)));
        list.update(window, |_, cx| cx.notify());
        let idle = window.debug_bounds("group-list-label:session").unwrap();
        list.update(window, |list, cx| list.set_groups(groups(true), cx));
        let running = window.debug_bounds("group-list-label:session").unwrap();
        assert_eq!(idle, running);
        list.update(window, |list, cx| list.set_groups(groups(false), cx));
        assert_eq!(
            idle,
            window.debug_bounds("group-list-label:session").unwrap()
        );
    }
}
