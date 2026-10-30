//! Collection tree rendering, context menus, and drag-and-drop behavior.

use super::*;

impl Workbench {
    pub(super) fn drop_tree_node(
        &mut self,
        source: &model::TreeNode,
        target: &model::TreeNode,
        placement: model::Placement,
        cx: &mut Context<Self>,
    ) {
        use model::{Placement, TreeNode};
        self.directory_settings = None;
        self.capture_draft(cx);
        let old_names = self
            .workspace
            .collections
            .iter()
            .map(|c| c.collection.clone())
            .collect::<Vec<_>>();
        let selected_name = old_names.get(self.current_collection).cloned();
        let moved = if let (
            TreeNode::Directory {
                collection: from,
                path,
            },
            TreeNode::Directory {
                collection: to,
                path: target_path,
            },
        ) = (source, target)
        {
            if path.is_empty() && target_path.is_empty() && placement != Placement::Inside {
                let name = old_names[*from].clone();
                let destination = old_names[*to].clone();
                if from != to {
                    let collection = self.workspace.collections.remove(*from);
                    let position = self
                        .workspace
                        .collections
                        .iter()
                        .position(|c| c.collection == destination)
                        .unwrap()
                        + usize::from(placement == Placement::After);
                    self.workspace.collections.insert(position, collection);
                }
                Ok(TreeNode::Directory {
                    collection: self
                        .workspace
                        .collections
                        .iter()
                        .position(|c| c.collection == name)
                        .unwrap(),
                    path: String::new(),
                })
            } else {
                model::move_node(
                    &mut self.workspace.collections,
                    &mut self.workspace.requests,
                    source,
                    target,
                    placement,
                )
            }
        } else {
            model::move_node(
                &mut self.workspace.collections,
                &mut self.workspace.requests,
                source,
                target,
                placement,
            )
        };
        match moved {
            Ok(moved) => {
                let remap = |collection: usize, path: String| {
                    if let (
                        TreeNode::Directory {
                            collection: from,
                            path: old,
                        },
                        TreeNode::Directory {
                            collection: to,
                            path: next,
                        },
                    ) = (source, &moved)
                    {
                        if !old.is_empty()
                            && collection == *from
                            && (path == *old || path.starts_with(&format!("{old}/")))
                        {
                            return (*to, format!("{next}{}", &path[old.len()..]));
                        }
                    }
                    (
                        self.workspace
                            .collections
                            .iter()
                            .position(|c| Some(&c.collection) == old_names.get(collection))
                            .unwrap_or(0),
                        path,
                    )
                };
                self.collapsed_directories = std::mem::take(&mut self.collapsed_directories)
                    .into_iter()
                    .map(|(c, p)| remap(c, p))
                    .collect();
                let (c, path) = remap(self.current_collection, self.current_directory.clone());
                self.current_collection = c;
                self.current_directory = path;
                if let Some(name) = selected_name {
                    if matches!(source, TreeNode::Directory { path, .. } if path.is_empty()) {
                        self.current_collection = self
                            .workspace
                            .collections
                            .iter()
                            .position(|c| c.collection == name)
                            .unwrap_or(0);
                    }
                }
                let (collection, mut parent) = match moved {
                    TreeNode::Directory { collection, path } => {
                        (collection, RequestGroups::parent(&path).to_owned())
                    }
                    TreeNode::Request(index) => (
                        self.workspace
                            .collections
                            .iter()
                            .position(|c| c.collection == self.workspace.requests[index].collection)
                            .unwrap(),
                        self.workspace.requests[index].group.clone(),
                    ),
                };
                loop {
                    self.collapsed_directories
                        .remove(&(collection, parent.clone()));
                    if parent.is_empty() {
                        break;
                    }
                    parent = RequestGroups::parent(&parent).to_owned();
                }
                self.workspace.storage_dirty = true;
                self.tree_menu = None;
                self.group_edit = None;
                self.hovered_tree_row = None;
                self.notice = "已移动节点".into();
                self.sync_commands(cx);
                self.persist(cx);
            }
            Err(error) => self.notice = error,
        }
        self.tree_drop = None;
        cx.notify();
    }

    pub(super) fn node_key(node: &model::TreeNode) -> String {
        match node {
            model::TreeNode::Request(index) => format!("request-leaf:{index}"),
            model::TreeNode::Directory { collection, path } => {
                format!("request-directory:{collection}:{path}")
            }
        }
    }

    pub(super) fn node_from_key(&self, key: &str) -> Option<model::TreeNode> {
        if let Some(index) = key
            .strip_prefix("request-leaf:")
            .and_then(|s| s.parse::<usize>().ok())
        {
            return (index < self.workspace.requests.len())
                .then_some(model::TreeNode::Request(index));
        }
        let (collection, path) = key.strip_prefix("request-directory:")?.split_once(':')?;
        let collection = collection.parse::<usize>().ok()?;
        self.workspace
            .collections
            .get(collection)
            .filter(|c| c.contains(path))?;
        Some(model::TreeNode::Directory {
            collection,
            path: path.into(),
        })
    }

    pub(super) fn tree_item(
        &self,
        node: model::TreeNode,
        label: String,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> TreeListItem {
        let owner = cx.entity().downgrade();
        let drop_owner = owner.clone();
        let indicator = self
            .tree_drop
            .as_ref()
            .filter(|(target, _)| *target == node)
            .map(|(_, p)| *p)
            .filter(|_| cx.has_active_drag());
        TreeListItem::new(Self::node_key(&node), label)
            .depth(depth)
            .theme(self.theme)
            .draggable(cx.entity_id())
            .drop_indicator(indicator)
            .on_drag_over(move |event: &TreeDrop, _, cx| {
                let _ = owner.update(cx, |this, cx| {
                    if let Some(target) = this.node_from_key(&event.target) {
                        let next = Some((target, event.placement));
                        if next != this.tree_drop {
                            this.tree_drop = next;
                            cx.notify();
                        }
                    }
                });
            })
            .on_drop(move |event: &TreeDrop, _, cx| {
                let _ = drop_owner.update(cx, |this, cx| {
                    if let (Some(source), Some(target)) = (
                        this.node_from_key(&event.source),
                        this.node_from_key(&event.target),
                    ) {
                        this.drop_tree_node(&source, &target, event.placement, cx);
                    }
                });
            })
    }

    pub(super) fn tree_row(
        &self,
        entry: model::TreeEntry,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let (node, depth, label, expanded) = match entry {
            model::TreeEntry::Directory {
                collection,
                path,
                depth,
                expanded,
            } => {
                let label = if path.is_empty() {
                    self.workspace.collections[collection].collection.clone()
                } else {
                    path.rsplit('/').next().unwrap_or(&path).into()
                };
                (
                    model::TreeNode::Directory { collection, path },
                    depth,
                    label,
                    Some(expanded),
                )
            }
            model::TreeEntry::Request { index, depth } => (
                model::TreeNode::Request(index),
                depth,
                self.workspace.requests[index].name.clone(),
                None,
            ),
        };
        let key = Self::node_key(&node);
        let debug_key = key.clone();
        let hovered = self.hovered_tree_row.as_deref() == Some(key.as_str());
        let mut item = self.tree_item(node.clone(), label.clone(), depth, cx);
        match node {
            model::TreeNode::Directory { collection, path } => {
                item = item.branch(expanded.unwrap()).selected(
                    self.current_collection == collection && self.current_directory == path,
                );
                if path.is_empty() {
                    item = item.content(
                        div()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label),
                    );
                }
                if hovered
                    || self.tree_menu.as_ref() == Some(&(collection, path.clone()))
                    || self.tree_delete.as_ref()
                        == Some(&model::TreeNode::Directory {
                            collection,
                            path: path.clone(),
                        })
                {
                    item = item.trailing(self.directory_menu(collection, path.clone(), cx));
                }
                item.render()
                    .debug_selector(move || debug_key.clone())
                    .on_click(
                        cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                            if event.click_count() == 2 {
                                window.focus(&this.focus, cx);
                                this.open_directory_settings(collection, path.clone(), cx);
                                return;
                            }
                            this.tree_menu = None;
                            this.tree_delete = None;
                            this.current_collection = collection;
                            this.current_directory = path.clone();
                            let key = (collection, path.clone());
                            if !this.collapsed_directories.remove(&key) {
                                this.collapsed_directories.insert(key);
                            }
                            this.group_edit = None;
                            this.group_error = None;
                            cx.notify();
                        }),
                    )
            }
            model::TreeNode::Request(index) => {
                let request = &self.workspace.requests[index];
                item = item.selected(index == self.active).content(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .w(px(34.))
                                .flex_shrink_0()
                                .text_size(px(9.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(self
                                    .theme
                                    .color(method_color(&request.draft.method))))
                                .child(request.draft.method.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(request.name.clone()),
                        ),
                );
                if hovered
                    || self.request_menu == Some(index)
                    || self.tree_delete.as_ref() == Some(&model::TreeNode::Request(index))
                {
                    item = item.trailing(self.request_actions_menu(index, cx));
                }
                item.render()
                    .debug_selector(move || debug_key.clone())
                    .on_click(cx.listener(move |this, _, _, cx| this.choose(index, cx)))
            }
        }
        .on_hover(cx.listener(move |this, hover, _, cx| {
            if *hover {
                this.hovered_tree_row = Some(key.clone());
            } else if this.hovered_tree_row.as_deref() == Some(key.as_str()) {
                this.hovered_tree_row = None;
            }
            cx.notify();
        }))
        .into_any_element()
    }

    pub(super) fn directory_menu(
        &self,
        collection: usize,
        path: String,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let target = model::TreeNode::Directory {
            collection,
            path: path.clone(),
        };
        if self.tree_delete.as_ref() == Some(&target) {
            return div().child(
                self.tree_delete_popover(
                    target,
                    IconButton::new(Icon::More, "目录操作")
                        .theme(self.theme)
                        .render()
                        .size(px(22.)),
                    cx,
                ),
            );
        }
        let p = self.theme.palette();
        let key = (collection, path.clone());
        let open = self.tree_menu.as_ref() == Some(&key);
        let toggle_key = key.clone();
        let debug_key = format!("directory-menu:{collection}:{path}");
        let mut trigger = div().relative().flex_shrink_0().child(
            IconButton::new(Icon::More, "目录操作")
                .theme(self.theme)
                .render()
                .id(format!("directory-menu:{collection}:{path}"))
                .debug_selector(move || debug_key.clone())
                .size(px(22.))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.tree_menu = if this.tree_menu.as_ref() == Some(&toggle_key) {
                        None
                    } else {
                        Some(toggle_key.clone())
                    };
                    this.tree_delete = None;
                    this.show_collection_menu = false;
                    window.focus(&this.focus, cx);
                    cx.notify();
                })),
        );
        if open {
            let mut menu = div()
                .p_1()
                .rounded(px(8.))
                .border_1()
                .border_color(rgb(p.border))
                .bg(rgb(p.panel))
                .shadow_md()
                .text_size(px(12.));
            for (action, icon, label) in [
                (0, Icon::Plus, "新建请求"),
                (1, Icon::Folder, "新建目录"),
                (
                    2,
                    Icon::Pencil,
                    if path.is_empty() {
                        "重命名集合"
                    } else {
                        "重命名目录"
                    },
                ),
                (3, Icon::Trash, "删除目录"),
                (4, Icon::Settings, "目录设置"),
                (5, Icon::Code, "集合变量"),
            ] {
                if path.is_empty() && action == 3 || !path.is_empty() && action == 5 {
                    continue;
                }
                let path = path.clone();
                let debug_key = format!("directory-menu-action:{collection}:{path}:{action}");
                menu = menu.child(
                    div()
                        .id(format!(
                            "directory-menu-action:{collection}:{path}:{action}"
                        ))
                        .debug_selector(move || debug_key.clone())
                        .h(px(32.))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded(px(5.))
                        .cursor_pointer()
                        .when(action == 3, |item| {
                            item.text_color(rgb(self.theme.color(0xd9342b)))
                        })
                        .hover(|style| style.bg(rgb(p.surface_subtle)))
                        .child(icon.render(14.).flex_shrink_0())
                        .child(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.tree_menu = None;
                            if action == 2 && path.is_empty() {
                                this.edit_collection_name(collection, window, cx);
                                return;
                            }
                            this.current_collection = collection;
                            this.current_directory = path.clone();
                            match action {
                                0 => this.add_request_to(path.clone(), cx),
                                1 => {
                                    this.edit_directory(collection, path.clone(), None, window, cx)
                                }
                                2 => this.edit_directory(
                                    collection,
                                    path.clone(),
                                    Some(path.clone()),
                                    window,
                                    cx,
                                ),
                                5 => {
                                    let name =
                                        this.workspace.collections[collection].collection.clone();
                                    match this.set_variable_scope(Some(name), cx) {
                                        Ok(()) => {
                                            this.editing_environment = this.selected_environment(cx)
                                        }
                                        Err(error) => this.variable_error = Some(error),
                                    }
                                    this.page = crate::Page::Environments;
                                    this.sync_variable_table(cx);
                                    cx.notify();
                                }
                                4 => {
                                    window.focus(&this.focus, cx);
                                    this.open_directory_settings(collection, path.clone(), cx);
                                }
                                _ => {
                                    this.tree_delete = Some(model::TreeNode::Directory {
                                        collection,
                                        path: path.clone(),
                                    });
                                    cx.notify();
                                }
                            }
                        })),
                );
            }
            let popup = div()
                .absolute()
                .top_0()
                .right_0()
                .w(px(168.))
                .pt(px(26.))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.tree_menu = None;
                    cx.notify();
                }))
                .child(menu);
            trigger = trigger.child(gpui::deferred(popup).with_priority(20));
        }
        trigger
    }
}
