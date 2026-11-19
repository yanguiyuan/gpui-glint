use std::{path::PathBuf, rc::Rc};

use gpui::{App, ElementId, SharedString, Window, div, prelude::*, px, rgb};

use super::{Icon, Popover};
use crate::theme::ThemePreset;

pub struct ProjectOption {
    pub path: PathBuf,
    pub label: String,
    pub icon: Icon,
    pub selected: bool,
    pub default_space: bool,
}

/// Project navigation shared by topbars and the composer. The host supplies
/// the project catalog, selection behavior, and actions below the list.
pub struct ProjectSelector {
    id: ElementId,
    trigger_selector: &'static str,
    label: String,
    icon: Icon,
    open: bool,
    projects: Vec<ProjectOption>,
    theme: ThemePreset,
}

impl ProjectSelector {
    pub fn new(
        id: impl Into<ElementId>,
        trigger_selector: &'static str,
        label: String,
        icon: Icon,
        projects: Vec<ProjectOption>,
        theme: ThemePreset,
    ) -> Self {
        Self {
            id: id.into(),
            trigger_selector,
            label,
            icon,
            open: false,
            projects,
            theme,
        }
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn popover(
        self,
        actions: impl IntoElement,
        on_select: impl Fn(&std::path::Path, &mut Window, &mut App) + 'static,
    ) -> Popover {
        let p = self.theme.palette();
        let selector = self.trigger_selector;
        let trigger = div()
            .debug_selector(move || selector.into())
            .h(px(28.))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .rounded(px(6.))
            .hover(|style| style.bg(rgb(p.surface_subtle)))
            .child(self.icon.render(15.).text_color(rgb(p.text_muted)))
            .child(div().max_w(px(168.)).truncate().child(self.label))
            .child(Icon::disclosure(12., self.open).text_color(rgb(p.text_muted)));
        let on_select = Rc::new(on_select);
        let list = div()
            .id("project-picker-list")
            .max_h(px(220.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.))
            .when(self.projects.is_empty(), |list| {
                list.child(
                    div()
                        .px_2()
                        .py_2()
                        .text_color(rgb(p.text_muted))
                        .child("暂无项目"),
                )
            })
            .children(self.projects.into_iter().map(|project| {
                let select = on_select.clone();
                let selector = format!("choose-project:{}", project.path.display());
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector.clone())
                    .h(px(40.))
                    .flex_shrink_0()
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .when(project.selected, |row| row.bg(rgb(p.accent_tint)))
                    .hover(|style| style.bg(rgb(p.accent_tint)))
                    .child(
                        project
                            .icon
                            .render(15.)
                            .flex_shrink_0()
                            .text_color(rgb(p.text_muted)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().truncate().child(project.label))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.text_muted))
                                    .truncate()
                                    .child(crate::paths::display_path(&project.path)),
                            ),
                    )
                    .when(project.default_space, |row| {
                        row.child(
                            div()
                                .text_size(px(9.))
                                .text_color(rgb(p.text_muted))
                                .child("默认空间"),
                        )
                    })
                    .when(project.selected, |row| {
                        row.child(
                            Icon::CheckCircle
                                .render(14.)
                                .flex_shrink_0()
                                .text_color(rgb(p.accent)),
                        )
                    })
                    .on_click(move |_, window, cx| select(&project.path, window, cx))
            }));
        let content = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .px_2()
                    .pt_2()
                    .pb_1()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("切换项目"),
            )
            .child(list)
            .child(div().h(px(1.)).mx_2().my_1().bg(rgb(p.border)))
            .child(actions);
        Popover::new(self.id, trigger, content)
            .open(self.open)
            .trigger_height(28.)
            .width(316.)
            .padding(6.)
            .theme(self.theme)
    }
}
