use super::Icon;
use crate::theme::ThemePreset;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Context, EntityId, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px, rgb,
};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeDropPlacement {
    Inside,
    Before,
    After,
}
#[derive(Clone, Debug)]
pub struct TreeDrop {
    pub source: SharedString,
    pub target: SharedString,
    pub placement: TreeDropPlacement,
}
type DropHandler = Rc<dyn Fn(&TreeDrop, &mut Window, &mut App)>;
#[derive(Clone)]
struct NodeDrag {
    scope: EntityId,
    id: SharedString,
    label: SharedString,
    theme: ThemePreset,
}
impl Render for NodeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(self.theme.palette().panel))
            .shadow_md()
            .child(self.label.clone())
    }
}

/// A controlled tree row. The owner supplies hierarchy, expansion, menus and moves.
/// IDs must be stable and unique within the drag scope. Different scopes cannot exchange nodes.
pub struct TreeListItem {
    id: SharedString,
    label: SharedString,
    depth: usize,
    branch: Option<bool>,
    selected: bool,
    theme: ThemePreset,
    content: Option<gpui::AnyElement>,
    trailing: Option<gpui::AnyElement>,
    scope: Option<EntityId>,
    indicator: Option<TreeDropPlacement>,
    on_drag_over: Option<DropHandler>,
    on_drop: Option<DropHandler>,
}
impl TreeListItem {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            depth: 0,
            branch: None,
            selected: false,
            theme: ThemePreset::default(),
            content: None,
            trailing: None,
            scope: None,
            indicator: None,
            on_drag_over: None,
            on_drop: None,
        }
    }
    pub fn depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }
    /// Render a folder with its current expansion state; no disclosure arrow is added.
    pub fn branch(mut self, expanded: bool) -> Self {
        self.branch = Some(expanded);
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    /// Replace the label area, for example with a request method and title.
    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }
    pub fn trailing(mut self, content: impl IntoElement) -> Self {
        self.trailing = Some(content.into_any_element());
        self
    }
    pub fn draggable(mut self, scope: EntityId) -> Self {
        self.scope = Some(scope);
        self
    }
    /// The owner retains drag feedback across renders.
    pub fn drop_indicator(mut self, placement: Option<TreeDropPlacement>) -> Self {
        self.indicator = placement;
        self
    }
    pub fn on_drag_over(
        mut self,
        handler: impl Fn(&TreeDrop, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_drag_over = Some(Rc::new(handler));
        self
    }
    pub fn on_drop(mut self, handler: impl Fn(&TreeDrop, &mut Window, &mut App) + 'static) -> Self {
        self.on_drop = Some(Rc::new(handler));
        self
    }
    pub fn render(self) -> gpui::Stateful<gpui::Div> {
        let p = self.theme.palette();
        let branch = self.branch.is_some();
        let mut row = div()
            .id(self.id.clone())
            .relative()
            .w_full()
            .min_w(px(180. + self.depth as f32 * 14.))
            .h(px(if branch { 32. } else { 30. }))
            .pl(px(8. + self.depth as f32 * 14.))
            .pr(px(4.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(6.))
            .cursor_pointer()
            .when(self.selected, |row| row.bg(rgb(p.accent_tint)))
            .hover(|style| style.bg(rgb(p.surface_subtle)));
        if let Some(expanded) = self.branch {
            row = row.child(
                if expanded {
                    Icon::FolderOpen
                } else {
                    Icon::Folder
                }
                .render(16.)
                .flex_shrink_0()
                .text_color(rgb(if self.selected {
                    p.accent
                } else {
                    p.text_muted
                })),
            );
        }
        row = row.child(div().flex_1().min_w_0().text_size(px(12.)).child(
            self.content.unwrap_or_else(|| {
                div()
                    .truncate()
                    .child(self.label.clone())
                    .into_any_element()
            }),
        ));
        if let Some(trailing) = self.trailing {
            row = row.child(trailing);
        }
        if let Some(scope) = self.scope {
            let target = self.id.clone();
            let drop_target = target.clone();
            let bounds = Rc::new(Cell::new(gpui::Bounds::<gpui::Pixels>::default()));
            let drop_bounds = bounds.clone();
            let hover_drag = Rc::new(std::cell::RefCell::new(None::<NodeDrag>));
            let style_drag = hover_drag.clone();
            let hover = self.on_drag_over;
            let drop = self.on_drop;
            row = row
                .on_drag(
                    NodeDrag {
                        scope,
                        id: self.id,
                        label: self.label,
                        theme: self.theme,
                    },
                    |drag, _, _, cx| cx.new(|_| drag.clone()),
                )
                .drag_over::<NodeDrag>(move |style, drag, _, _| {
                    if drag.scope == scope {
                        *style_drag.borrow_mut() = Some(drag.clone());
                        style.bg(rgb(p.accent_tint))
                    } else {
                        style
                    }
                })
                .child(
                    gpui::canvas(
                        move |row_bounds, window, cx| {
                            bounds.set(row_bounds);
                            if row_bounds.contains(&window.mouse_position()) {
                                if let Some(drag) = hover_drag.borrow().clone() {
                                    let event = TreeDrop {
                                        source: drag.id,
                                        target: target.clone(),
                                        placement: placement_at(
                                            branch,
                                            (window.mouse_position().y - row_bounds.top())
                                                / row_bounds.size.height,
                                        ),
                                    };
                                    let placement = event.placement;
                                    if let Some(handler) = hover.clone() {
                                        window.defer(cx, move |window, cx| {
                                            handler(&event, window, cx)
                                        });
                                    }
                                    return Some(placement);
                                }
                            }
                            None
                        },
                        move |bounds, placement, window, _| {
                            if let Some(placement) =
                                placement.filter(|p| *p != TreeDropPlacement::Inside)
                            {
                                let y = if placement == TreeDropPlacement::Before {
                                    bounds.top()
                                } else {
                                    bounds.bottom() - px(2.)
                                };
                                window.paint_quad(gpui::fill(
                                    gpui::Bounds::new(
                                        gpui::point(bounds.left(), y),
                                        gpui::size(bounds.size.width, px(2.)),
                                    ),
                                    rgb(p.accent),
                                ));
                            }
                        },
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
                .on_drop(move |drag: &NodeDrag, window, cx| {
                    if drag.scope == scope {
                        if let Some(handler) = &drop {
                            handler(
                                &TreeDrop {
                                    source: drag.id.clone(),
                                    target: drop_target.clone(),
                                    placement: placement_at(
                                        branch,
                                        (window.mouse_position().y - drop_bounds.get().top())
                                            / drop_bounds.get().size.height,
                                    ),
                                },
                                window,
                                cx,
                            );
                        }
                    }
                });
        }
        row.when(self.indicator == Some(TreeDropPlacement::Inside), |row| {
            row.bg(rgb(p.accent_tint))
        })
        .when(
            self.indicator
                .is_some_and(|placement| placement != TreeDropPlacement::Inside),
            |row| {
                row.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .h(px(2.))
                        .bg(rgb(p.accent))
                        .when(self.indicator == Some(TreeDropPlacement::Before), |line| {
                            line.top_0()
                        })
                        .when(self.indicator == Some(TreeDropPlacement::After), |line| {
                            line.bottom_0()
                        }),
                )
            },
        )
    }
}
fn placement_at(branch: bool, fraction: f32) -> TreeDropPlacement {
    if branch && fraction > 0.25 && fraction < 0.75 {
        TreeDropPlacement::Inside
    } else if fraction < 0.5 {
        TreeDropPlacement::Before
    } else {
        TreeDropPlacement::After
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    #[gpui::test]
    fn native_drag_reports_placement_and_isolates_scopes(cx: &mut gpui::TestAppContext) {
        struct Panel {
            other: gpui::Entity<u32>,
            drops: Rc<RefCell<Vec<TreeDrop>>>,
        }
        impl Render for Panel {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let drops = self.drops.clone();
                let other_drops = self.drops.clone();
                div()
                    .w(px(280.))
                    .flex()
                    .flex_col()
                    .child(
                        TreeListItem::new("source", "请求")
                            .depth(1)
                            .draggable(cx.entity_id())
                            .render()
                            .debug_selector(|| "tree-source".into()),
                    )
                    .child(
                        TreeListItem::new("folder", "目录")
                            .branch(false)
                            .draggable(cx.entity_id())
                            .on_drop(move |drop, _, _| drops.borrow_mut().push(drop.clone()))
                            .render()
                            .debug_selector(|| "tree-folder".into()),
                    )
                    .child(
                        TreeListItem::new("other", "另一棵树")
                            .branch(true)
                            .draggable(self.other.entity_id())
                            .on_drop(move |drop, _, _| other_drops.borrow_mut().push(drop.clone()))
                            .render()
                            .debug_selector(|| "tree-other".into()),
                    )
            }
        }
        let drops = Rc::new(RefCell::new(Vec::new()));
        let (_, window) = cx.add_window_view(|_, cx| Panel {
            other: cx.new(|_| 0),
            drops: drops.clone(),
        });
        let source = window.debug_bounds("tree-source").unwrap();
        let folder = window.debug_bounds("tree-folder").unwrap();
        for (target, expected) in [
            (folder.center(), Some(TreeDropPlacement::Inside)),
            (
                gpui::point(folder.center().x, folder.top() + px(2.)),
                Some(TreeDropPlacement::Before),
            ),
            (window.debug_bounds("tree-other").unwrap().center(), None),
        ] {
            window.simulate_event(gpui::MouseDownEvent {
                position: source.center(),
                button: gpui::MouseButton::Left,
                click_count: 1,
                ..Default::default()
            });
            for _ in 0..2 {
                window.simulate_event(gpui::MouseMoveEvent {
                    position: target,
                    pressed_button: Some(gpui::MouseButton::Left),
                    ..Default::default()
                });
            }
            let previous = drops.borrow().len();
            window.simulate_event(gpui::MouseUpEvent {
                position: target,
                button: gpui::MouseButton::Left,
                click_count: 1,
                ..Default::default()
            });
            if let Some(expected) = expected {
                assert_eq!(drops.borrow().len(), previous + 1);
                let events = drops.borrow();
                let event = events.last().unwrap();
                assert_eq!(event.source.as_ref(), "source");
                assert_eq!(event.target.as_ref(), "folder");
                assert_eq!(event.placement, expected);
            } else {
                assert_eq!(drops.borrow().len(), previous);
            }
        }
    }
}
