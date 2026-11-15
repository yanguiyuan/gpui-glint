//! Compact application navigation with primary entries and pinned footer actions.
use super::{Icon, IconButton, Tooltip};
use crate::theme::ThemePreset;
use gpui::{
    AnyElement, App, Context, IntoElement, ParentElement, Render, SharedString, Styled, Window,
    div, prelude::*, px, rgb,
};
use std::rc::Rc;

#[derive(Clone)]
pub struct NavigationItem {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: Icon,
}
impl NavigationItem {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>, icon: Icon) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon,
        }
    }
}
/// A visual group. Empty groups create neither a gap nor a divider.
#[derive(Clone)]
pub struct NavigationGroup {
    pub id: SharedString,
    pub items: Vec<NavigationItem>,
}
impl NavigationGroup {
    pub fn new(
        id: impl Into<SharedString>,
        items: impl IntoIterator<Item = NavigationItem>,
    ) -> Self {
        Self {
            id: id.into(),
            items: items.into_iter().collect(),
        }
    }
}
type SelectHandler = Rc<dyn Fn(&SharedString, &mut Window, &mut App)>;
pub struct NavigationRail {
    groups: Vec<NavigationGroup>,
    footer: NavigationGroup,
    selected: Option<SharedString>,
    theme: ThemePreset,
    on_select: Option<SelectHandler>,
}
impl NavigationRail {
    pub fn new(items: impl IntoIterator<Item = NavigationItem>) -> Self {
        Self {
            groups: vec![NavigationGroup::new("navigation-main", items)],
            footer: NavigationGroup::new("navigation-footer", []),
            selected: None,
            theme: ThemePreset::default(),
            on_select: None,
        }
    }
    pub fn footer(mut self, items: impl IntoIterator<Item = NavigationItem>) -> Self {
        self.footer = NavigationGroup::new("navigation-footer", items);
        self
    }
    pub fn groups(mut self, groups: impl IntoIterator<Item = NavigationGroup>) -> Self {
        self.groups = groups.into_iter().collect();
        self
    }
    pub fn footer_group(mut self, group: NavigationGroup) -> Self {
        self.footer = group;
        self
    }
    pub fn selected(mut self, id: impl Into<SharedString>) -> Self {
        self.selected = Some(id.into());
        self
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn on_select(
        mut self,
        handler: impl Fn(&SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }
    pub fn render(self, cx: &mut Context<impl Render>) -> AnyElement {
        let mut rail = div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .py_3();
        let mut first = true;
        for group in self.groups.iter().filter(|group| !group.items.is_empty()) {
            if !first {
                rail = rail.child(self.separator(&group.id));
            }
            first = false;
            for item in &group.items {
                rail = rail.child(self.button(item, cx));
            }
        }
        rail = rail.child(div().flex_1().min_h_0());
        if !self.footer.items.is_empty() {
            rail = rail.child(self.separator(&self.footer.id));
            for item in &self.footer.items {
                rail = rail.child(self.button(item, cx));
            }
        }
        rail.into_any_element()
    }
    fn separator(&self, group: &SharedString) -> AnyElement {
        let id = format!("{group}-divider");
        div()
            .id(SharedString::from(id.clone()))
            .debug_selector(move || id.clone())
            .w(px(24.))
            .my_1()
            .h(px(1.))
            .flex_shrink_0()
            .bg(rgb(self.theme.palette().border))
            .into_any_element()
    }
    fn button(&self, item: &NavigationItem, cx: &mut Context<impl Render>) -> AnyElement {
        let id = item.id.clone();
        let selector = item.id.clone();
        let handler = self.on_select.clone();
        let palette = self.theme.palette();
        let trigger = IconButton::new(item.icon, item.label.clone())
            .theme(self.theme)
            .render()
            .id(item.id.clone())
            .debug_selector(move || selector.to_string())
            .when(self.selected.as_ref() == Some(&item.id), |button| {
                button
                    .bg(rgb(palette.accent_tint))
                    .text_color(rgb(palette.accent))
            })
            .on_click(move |_, window, cx| {
                if let Some(handler) = &handler {
                    handler(&id, window, cx);
                }
            });
        Tooltip::new(trigger, item.label.clone())
            .theme(self.theme)
            .render(cx)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Demo(std::rc::Rc<std::cell::RefCell<Vec<String>>>);
    impl Render for Demo {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let events = self.0.clone();
            NavigationRail::new([NavigationItem::new("primary", "Library", Icon::Folders)])
                .footer([NavigationItem::new("settings", "Settings", Icon::Settings)])
                .selected("primary")
                .on_select(move |id, _, _| events.borrow_mut().push(id.to_string()))
                .render(cx)
        }
    }
    #[gpui::test]
    fn footer_actions_keep_stable_ids_and_selection_is_owned_by_the_host(
        cx: &mut gpui::TestAppContext,
    ) {
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let (_, window) = cx.add_window_view(|_, _| Demo(events.clone()));
        window.run_until_parked();
        let primary = window.debug_bounds("primary").unwrap();
        let settings = window.debug_bounds("settings").unwrap();
        assert!(settings.top() > primary.bottom());
        window.simulate_click(settings.center(), gpui::Modifiers::default());
        assert_eq!(events.borrow().as_slice(), ["settings"]);
    }
}
