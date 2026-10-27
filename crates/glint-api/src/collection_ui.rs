//! Collection sidebar, group editing, and collection/directory dialogs.

use super::*;

impl Workbench {
    pub(super) fn sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        if self.show_proxies {
            return self.proxy_sidebar(cx);
        }
        if self.show_mcp {
            return self.mcp_history_sidebar(cx);
        }
        if self.show_plugins {
            return self.plugin_sidebar(cx);
        }
        let p = self.theme.palette();
        let mut tree = div()
            .id("request-tree")
            .flex_1()
            .min_h_0()
            .overflow_scroll()
            .px_2()
            .pb_3();
        if self.show_environments {
            return self.environment_sidebar(cx);
        }
        let list = model::tree_list(
            &self.collections,
            &self.requests,
            &self.collapsed_directories,
            self.filter.read(cx).value(),
        );
        if list.visible_rows().is_empty() {
            tree = tree.child(
                div()
                    .p_3()
                    .text_size(px(11.))
                    .text_color(rgb(p.text_muted))
                    .child(if self.collections.is_empty() {
                        "先创建一个集合，再添加请求或目录"
                    } else {
                        "没有匹配的集合、目录或请求"
                    }),
            );
        }
        tree = tree.child(list.render_with(|row| self.tree_row(model::tree_entry(row), cx)));
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(p.panel))
            .child(
                div()
                    .flex()
                    .items_center()
                    .px_3()
                    .h(px(44.))
                    .gap_1()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(
                        if self.show_environments {
                            "环境变量"
                        } else {
                            "请求"
                        },
                    ))
                    .child(div().flex_1())
                    .child(
                        IconButton::new(Icon::Plus, "新建请求")
                            .theme(self.theme)
                            .render()
                            .on_click(cx.listener(|this, _, _, cx| this.add_request(cx))),
                    )
                    .when(!self.show_environments, |header| {
                        header.child(self.collection_menu(cx))
                    }),
            )
            .child(
                div().px_3().pb_2().child(
                    SearchInput::new("筛选集合、目录或请求…")
                        .control(self.filter.clone())
                        .theme(self.theme)
                        .render()
                        .w_full()
                        .min_w_0()
                        .debug_selector(|| "sidebar-search-input".into()),
                ),
            )
            .child(tree)
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .p_3()
                    .text_size(px(10.))
                    .text_color(rgb(p.text_muted))
                    .child("SQLite 存储 · 保存时生成版本快照"),
            )
    }

    pub(super) fn collection_menu(&self, cx: &mut Context<Self>) -> gpui::Div {
        let owner = cx.entity().downgrade();
        let menu = div()
            .flex()
            .flex_col()
            .occlude()
            .child(
                MenuItem::new("create-request-collection", "创建集合")
                    .icon(Icon::Folders)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, window, cx| this.edit_collection(window, cx))),
            )
            .child(
                MenuItem::new("import-curl-menu", "从 cURL 导入")
                    .icon(Icon::Download)
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "open-curl-import".into())
                    .on_click(cx.listener(|this, _, _, cx| this.open_curl_import(cx))),
            );
        div().child(
            DropdownMenu::new(
                "collection-actions-menu",
                IconButton::new(Icon::More, "更多集合操作")
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "collection-more".into()),
                menu,
            )
            .open(self.show_collection_menu)
            .width(160.)
            .theme(self.theme)
            .on_open_change(move |open, window, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.show_collection_menu = open;
                    window.focus(&this.focus, cx);
                    cx.notify();
                });
            })
            .render(),
        )
    }

    pub(super) fn edit_collection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_proxies = false;
        self.show_mcp = false;
        self.show_collection_menu = false;
        self.tree_menu = None;
        self.show_environments = false;
        self.show_plugins = false;
        self.group_name
            .update(cx, |entry, cx| entry.set_value("", cx));
        self.group_edit = Some(GroupEdit::CreateCollection);
        self.group_error = None;
        window.focus(&self.group_name.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn edit_collection_name(
        &mut self,
        collection: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.collections.get(collection) else {
            return;
        };
        let old = target.collection.clone();
        if self.current_collection != collection {
            self.current_directory.clear();
        }
        self.current_collection = collection;
        self.show_proxies = false;
        self.show_mcp = false;
        self.show_environments = false;
        self.show_plugins = false;
        self.tree_menu = None;
        self.show_collection_menu = false;
        self.group_name
            .update(cx, |entry, cx| entry.set_value(old.clone(), cx));
        self.group_edit = Some(GroupEdit::RenameCollection(old));
        self.group_error = None;
        window.focus(&self.group_name.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn sync_groups(&mut self, cx: &mut Context<Self>) {
        // Tree rows are derived directly from collections and requests at render time.
        if !self
            .collections
            .get(self.current_collection)
            .is_some_and(|c| c.contains(&self.current_directory))
        {
            self.current_directory.clear();
        }
        cx.notify();
    }

    pub(super) fn edit_directory(
        &mut self,
        collection: usize,
        parent: String,
        name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.current_collection = collection;
        self.current_directory = parent.clone();
        self.group_name.update(cx, |entry, cx| {
            entry.set_value(
                name.as_deref()
                    .map(|path| path.rsplit('/').next().unwrap_or(path))
                    .unwrap_or(""),
                cx,
            )
        });
        self.group_edit = Some(match name {
            Some(path) => GroupEdit::Rename(path),
            None => GroupEdit::Create(parent),
        });
        self.group_error = None;
        window.focus(&self.group_name.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    pub(super) fn save_group(&mut self, cx: &mut Context<Self>) {
        self.directory_settings = None;
        let name = self.group_name.read(cx).value().to_owned();
        let result = match &self.group_edit {
            Some(GroupEdit::CreateEnvironment) => {
                let name = name.trim();
                if name.is_empty() {
                    Err("请输入环境名称".into())
                } else if self.environment_names.iter().any(|n| n == name) {
                    Err("环境名称已存在".into())
                } else {
                    self.environment_names.push(name.to_owned());
                    self.environment_inputs
                        .push(EnvironmentVariables::default());
                    self.variable_rows.push(vec![]);
                    self.add_proxy_input(cx);
                    self.editing_environment = self.environment_names.len() - 1;
                    self.environment.update(cx, |select, cx| {
                        select.set_options(
                            self.environment_names
                                .iter()
                                .enumerate()
                                .map(|(i, n)| SelectOption::new(i.to_string(), n.clone())),
                            cx,
                        )
                    });
                    self.sync_variable_table(cx);
                    Ok(())
                }
            }
            Some(GroupEdit::CreateCollection) => {
                RequestGroups::create_collection(&name, &self.collections).map(|collection| {
                    self.collections.push(collection);
                    self.current_collection = self.collections.len() - 1;
                    self.current_directory.clear();
                    self.notice = format!("已创建集合 {}", name.trim());
                })
            }
            Some(GroupEdit::RenameCollection(old)) => self
                .collections
                .iter()
                .position(|collection| collection.collection == *old)
                .ok_or("集合不存在或已重命名，请重新选择".to_owned())
                .and_then(|index| {
                    RequestGroups::rename_collection(
                        index,
                        &name,
                        &mut self.collections,
                        &mut self.requests,
                    )
                })
                .map(|_| {
                    if self.editing_variable_collection.as_deref() == Some(old) {
                        self.editing_variable_collection = Some(name.trim().into());
                    }
                    self.notice = format!("已重命名集合为 {}", name.trim());
                }),
            Some(GroupEdit::Create(parent)) => self.collections[self.current_collection]
                .add_child(parent, &name)
                .map(|path| {
                    let node = model::TreeNode::Directory {
                        collection: self.current_collection,
                        path: path.clone(),
                    };
                    let target = model::TreeNode::Directory {
                        collection: self.current_collection,
                        path: parent.clone(),
                    };
                    let _ = model::move_node(
                        &mut self.collections,
                        &mut self.requests,
                        &node,
                        &target,
                        model::Placement::Inside,
                    );
                    self.current_directory = path.clone();
                    self.collapsed_directories
                        .remove(&(self.current_collection, path));
                    let mut path = parent.as_str();
                    loop {
                        self.collapsed_directories
                            .remove(&(self.current_collection, path.to_owned()));
                        if path.is_empty() {
                            break;
                        }
                        path = RequestGroups::parent(path);
                    }
                }),
            Some(GroupEdit::Rename(old)) => {
                let parent = RequestGroups::parent(old);
                let new = if parent.is_empty() {
                    name.trim().to_owned()
                } else {
                    format!("{parent}/{}", name.trim())
                };
                self.collections[self.current_collection]
                    .rename(old, &name, &mut self.requests)
                    .map(|_| {
                        let prefix = format!("{old}/");
                        self.collapsed_directories = self
                            .collapsed_directories
                            .iter()
                            .map(|(collection, path)| {
                                if *collection == self.current_collection
                                    && (path == old || path.starts_with(&prefix))
                                {
                                    (*collection, format!("{new}{}", &path[old.len()..]))
                                } else {
                                    (*collection, path.clone())
                                }
                            })
                            .collect();
                        self.current_directory = new;
                    })
            }
            None => return,
        };
        match result {
            Ok(()) => {
                self.storage_dirty = true;
                self.sync_groups(cx);
                self.sync_commands(cx);
                self.group_edit = None;
                self.group_error = None;
                self.filter.update(cx, |entry, cx| entry.set_value("", cx));
            }
            Err(error) => self.group_error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn group_editor(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        div()
            .debug_selector(|| "group-name-editor".into())
            .flex()
            .flex_col()
            .gap_2()
            .text_size(px(11.))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "enter" => this.save_group(cx),
                    "escape" => {
                        this.group_edit = None;
                        this.group_error = None;
                        cx.notify();
                    }
                    _ => return,
                }
                cx.stop_propagation();
                if this.group_edit.is_none() {
                    window.focus(&this.focus, cx);
                }
            }))
            .child(match self.group_edit {
                Some(GroupEdit::CreateCollection) => "创建集合",
                Some(GroupEdit::RenameCollection(_)) => "重命名集合",
                Some(GroupEdit::CreateEnvironment) => "创建环境",
                Some(GroupEdit::Create(_)) => "新建目录",
                _ => "重命名目录",
            })
            .child(
                div()
                    .h(px(30.))
                    .px_2()
                    .border_1()
                    .border_color(rgb(p.border))
                    .rounded_md()
                    .child(self.group_name.clone()),
            )
            .when_some(self.group_error.clone(), |editor, error| {
                editor.child(
                    div()
                        .text_color(rgb(self.theme.color(0xd9342b)))
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("取消")
                            .kind(ButtonKind::Gray)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "group-name-cancel".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.group_edit = None;
                                this.group_error = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("确定")
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "group-name-save".into())
                            .on_click(cx.listener(|this, _, _, cx| this.save_group(cx))),
                    ),
            )
    }

    pub(super) fn group_dialog(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        self.group_edit.as_ref()?;
        Some(
            Dialog::new("group-dialog", self.group_editor(cx))
                .focus_handles([self.group_name.read(cx).focus_handle(cx)])
                .theme(self.theme)
                .width(360.)
                .on_close(cx.listener(|this, _, window, cx| {
                    this.group_edit = None;
                    this.group_error = None;
                    window.focus(&this.focus, cx);
                    cx.notify();
                }))
                .render(),
        )
    }
}
