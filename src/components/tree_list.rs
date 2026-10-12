use super::{TreeDrop, TreeListItem};
use crate::theme::ThemePreset;
use gpui::{
    App, EntityId, IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled,
    Window, div,
};
use std::{collections::HashSet, rc::Rc};

/// Business-neutral tree data; IDs must be unique across the entire tree.
#[derive(Clone)]
pub struct TreeListNode {
    pub id: SharedString,
    pub label: SharedString,
    pub children: Vec<Self>,
    branch: bool,
    search_text: SharedString,
}
impl TreeListNode {
    pub fn leaf(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        let label = label.into();
        Self {
            id: id.into(),
            search_text: label.clone(),
            label,
            children: vec![],
            branch: false,
        }
    }
    pub fn branch(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        children: impl IntoIterator<Item = Self>,
    ) -> Self {
        Self {
            children: children.into_iter().collect(),
            branch: true,
            ..Self::leaf(id, label)
        }
    }
    pub fn search_text(mut self, text: impl Into<SharedString>) -> Self {
        self.search_text = text.into();
        self
    }
}
#[derive(Clone, Debug)]
pub struct TreeListRow {
    pub id: SharedString,
    pub label: SharedString,
    pub depth: usize,
    pub branch: bool,
    pub expanded: bool,
}
type NodeHandler = Rc<dyn Fn(&SharedString, &mut Window, &mut App)>;
type DropHandler = Rc<dyn Fn(&TreeDrop, &mut Window, &mut App)>;

/// A controlled tree list. The owner retains selected/collapsed IDs and applies drops.
pub struct TreeList {
    nodes: Vec<TreeListNode>,
    collapsed: HashSet<SharedString>,
    selected: Option<SharedString>,
    query: SharedString,
    theme: ThemePreset,
    scope: Option<EntityId>,
    on_select: Option<NodeHandler>,
    on_toggle: Option<NodeHandler>,
    on_drop: Option<DropHandler>,
}
impl TreeList {
    pub fn new(nodes: impl IntoIterator<Item = TreeListNode>) -> Self {
        Self {
            nodes: nodes.into_iter().collect(),
            collapsed: HashSet::new(),
            selected: None,
            query: "".into(),
            theme: ThemePreset::default(),
            scope: None,
            on_select: None,
            on_toggle: None,
            on_drop: None,
        }
    }
    pub fn collapsed(mut self, ids: impl IntoIterator<Item = SharedString>) -> Self {
        self.collapsed = ids.into_iter().collect();
        self
    }
    pub fn selected(mut self, id: impl Into<SharedString>) -> Self {
        self.selected = Some(id.into());
        self
    }
    pub fn query(mut self, query: impl Into<SharedString>) -> Self {
        self.query = query.into();
        self
    }
    pub fn theme(mut self, theme: ThemePreset) -> Self {
        self.theme = theme;
        self
    }
    pub fn draggable(mut self, scope: EntityId) -> Self {
        self.scope = Some(scope);
        self
    }
    pub fn on_select(
        mut self,
        handler: impl Fn(&SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }
    pub fn on_toggle(
        mut self,
        handler: impl Fn(&SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle = Some(Rc::new(handler));
        self
    }
    pub fn on_drop(mut self, handler: impl Fn(&TreeDrop, &mut Window, &mut App) + 'static) -> Self {
        self.on_drop = Some(Rc::new(handler));
        self
    }
    /// Searching exposes matching descendants and their ancestors without changing collapse state.
    pub fn visible_rows(&self) -> Vec<TreeListRow> {
        fn matches(node: &TreeListNode, query: &str, found: &mut HashSet<SharedString>) -> bool {
            let mut hit = node.label.to_lowercase().contains(query)
                || node.search_text.to_lowercase().contains(query);
            for child in &node.children {
                hit |= matches(child, query, found);
            }
            if hit {
                found.insert(node.id.clone());
            }
            hit
        }
        fn walk(
            node: &TreeListNode,
            depth: usize,
            inherited: bool,
            query: &str,
            found: &HashSet<SharedString>,
            collapsed: &HashSet<SharedString>,
            rows: &mut Vec<TreeListRow>,
        ) {
            let searching = !query.is_empty();
            let hit = inherited
                || node.label.to_lowercase().contains(query)
                || node.search_text.to_lowercase().contains(query);
            if searching && !hit && !found.contains(&node.id) {
                return;
            }
            let expanded = node.branch && (searching || !collapsed.contains(&node.id));
            rows.push(TreeListRow {
                id: node.id.clone(),
                label: node.label.clone(),
                depth,
                branch: node.branch,
                expanded,
            });
            if expanded {
                for child in &node.children {
                    walk(child, depth + 1, hit, query, found, collapsed, rows);
                }
            }
        }
        let query = self.query.trim().to_lowercase();
        let mut found = HashSet::new();
        if !query.is_empty() {
            for node in &self.nodes {
                matches(node, &query, &mut found);
            }
        }
        let mut rows = vec![];
        for node in &self.nodes {
            walk(node, 0, false, &query, &found, &self.collapsed, &mut rows);
        }
        rows
    }
    /// Customize rows while reusing hierarchy, collapse and filtering logic.
    pub fn render_with(
        &self,
        mut renderer: impl FnMut(&TreeListRow) -> gpui::AnyElement,
    ) -> gpui::Div {
        div().w_full().flex().flex_col().children(
            self.visible_rows()
                .iter()
                .map(&mut renderer)
                .collect::<Vec<_>>(),
        )
    }
    pub fn render(self) -> gpui::Div {
        self.render_with(|row| {
            let mut item = TreeListItem::new(row.id.clone(), row.label.clone())
                .depth(row.depth)
                .selected(self.selected.as_ref() == Some(&row.id))
                .theme(self.theme);
            if row.branch {
                item = item.branch(row.expanded);
            }
            if let Some(scope) = self.scope {
                item = item.draggable(scope);
            }
            if let Some(handler) = self.on_drop.clone() {
                item = item.on_drop(move |event, window, cx| handler(event, window, cx));
            }
            let select = self.on_select.clone();
            let toggle = row.branch.then(|| self.on_toggle.clone()).flatten();
            let id = row.id.clone();
            item.render()
                .on_click(move |_, window, cx| {
                    if let Some(handler) = &select {
                        handler(&id, window, cx);
                    }
                    if let Some(handler) = &toggle {
                        handler(&id, window, cx);
                    }
                })
                .into_any_element()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filter_preserves_ancestors_and_restores_independent_collapse() {
        let nodes = vec![TreeListNode::branch(
            "root",
            "API",
            [
                TreeListNode::branch(
                    "folder",
                    "Users",
                    [TreeListNode::leaf("request", "List users").search_text("GET /v1/users")],
                ),
                TreeListNode::leaf("other", "Health"),
            ],
        )];
        let collapsed = [SharedString::from("folder")];
        assert_eq!(
            TreeList::new(nodes.clone())
                .collapsed(collapsed.clone())
                .visible_rows()
                .len(),
            3
        );
        let filtered = TreeList::new(nodes.clone())
            .collapsed(collapsed.clone())
            .query("/v1/users")
            .visible_rows();
        assert_eq!(
            filtered
                .iter()
                .map(|row| row.id.as_ref())
                .collect::<Vec<_>>(),
            ["root", "folder", "request"]
        );
        assert_eq!(filtered[2].depth, 2);
        assert_eq!(
            TreeList::new(nodes.clone())
                .collapsed(collapsed.clone())
                .query("API")
                .visible_rows()
                .len(),
            4
        );
        assert_eq!(
            TreeList::new(nodes)
                .collapsed(collapsed)
                .visible_rows()
                .len(),
            3
        );
    }
}
