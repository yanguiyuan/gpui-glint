use super::*;

impl Workbench {
    pub(super) fn request_actions_menu(
        &self,
        index: usize,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let trigger = IconButton::new(Icon::More, "请求操作")
            .theme(self.theme)
            .render()
            .size(px(20.))
            .debug_selector(move || format!("request-more:{index}").into());
        let target = model::TreeNode::Request(index);
        if self.tree_delete.as_ref() == Some(&target) {
            return self.tree_delete_popover(target, trigger, cx);
        }
        let menu = div()
            .child(
                MenuItem::new(format!("copy-request:{index}"), "复制请求")
                    .icon(Icon::Copy)
                    .theme(self.theme)
                    .render()
                    .debug_selector(move || format!("copy-request:{index}").into())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.request_menu = None;
                        this.duplicate_request(index, cx);
                    })),
            )
            .child(
                MenuItem::new(format!("delete-request:{index}"), "删除请求")
                    .icon(Icon::Trash)
                    .theme(self.theme)
                    .render()
                    .text_color(rgb(self.theme.color(0xd9342b)))
                    .debug_selector(move || format!("delete-request:{index}").into())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.request_menu = None;
                        this.tree_delete = Some(model::TreeNode::Request(index));
                        cx.notify();
                    })),
            );
        let owner = cx.entity().downgrade();
        DropdownMenu::new(format!("request-actions:{index}"), trigger, menu)
            .open(self.request_menu == Some(index))
            .width(150.)
            .theme(self.theme)
            .on_open_change(move |open, window, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.request_menu = open.then_some(index);
                    if open {
                        this.tree_menu = None;
                        this.tree_delete = None;
                        this.show_collection_menu = false;
                    }
                    window.focus(&this.focus, cx);
                    cx.notify();
                });
            })
            .render()
            .into_any_element()
    }

    fn duplicate_request(&mut self, index: usize, cx: &mut Context<Self>) {
        self.flush_request_editors(cx);
        self.capture_draft(cx);
        let Some(source) = self.workspace.requests.get(index).cloned() else {
            return;
        };
        let base = format!("{} 副本", source.name);
        let mut name = base.clone();
        let mut suffix = 2;
        while self
            .workspace
            .requests
            .iter()
            .any(|r| r.collection == source.collection && r.group == source.group && r.name == name)
        {
            name = format!("{base} {suffix}");
            suffix += 1;
        }
        let i = self.workspace.requests.len();
        self.workspace.requests.push(Request {
            id: model::new_id(),
            tree_order: 0,
            name: name.clone(),
            collection: source.collection,
            group: source.group,
            draft: source.draft.clone(),
            versions: vec![Record {
                id: model::new_id(),
                tag: String::new(),
                source: Default::default(),
                environment_name: None,
                draft: source.draft,
                label: "v1".into(),
                time: record_time(),
                detail: "复制请求".into(),
                elapsed_ms: None,
                exchange: None,
            }],
            history: vec![],
        });
        let _ = model::move_node(
            &mut self.workspace.collections,
            &mut self.workspace.requests,
            &model::TreeNode::Request(i),
            &model::TreeNode::Request(index),
            model::Placement::After,
        );
        self.workspace.responses.push(None);
        self.workspace.traces.push(vec![]);
        self.workspace.sent_requests.push(None);
        self.workspace.sent_curls.push(None);
        self.current_versions.push(0);
        self.workspace.storage_dirty = true;
        self.choose(i, cx);
        self.notice = format!("已复制请求 {name}");
        self.sync_groups(cx);
        cx.notify();
    }

    pub(super) fn tree_delete_popover(
        &self,
        target: model::TreeNode,
        trigger: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let (title, description) = match &target {
            model::TreeNode::Request(index) => (
                format!("删除请求「{}」？", self.workspace.requests[*index].name),
                "该请求及其版本、请求历史将一并删除。".to_owned(),
            ),
            model::TreeNode::Directory { path, .. } => (
                format!("删除目录「{}」？", path.rsplit('/').next().unwrap_or(path)),
                "目录及其子目录将被删除，其中的请求保留并移到上级目录。".to_owned(),
            ),
        };
        let confirm_target = target.clone();
        let content = div()
            .flex()
            .flex_col()
            .gap_3()
            .debug_selector(|| "tree-delete-confirmation".into())
            .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(self.theme.palette().text_muted))
                    .child(description),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("取消")
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "cancel-tree-delete".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tree_delete = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("确认删除")
                            .kind(ButtonKind::Danger)
                            .size(ButtonSize::Mini)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "confirm-tree-delete".into())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if this.tree_delete.as_ref() != Some(&confirm_target) {
                                    return;
                                }
                                this.tree_delete = None;
                                match &confirm_target {
                                    model::TreeNode::Request(index) => {
                                        this.delete_request(*index, cx)
                                    }
                                    model::TreeNode::Directory { collection, path } => {
                                        this.directory_settings = None;
                                        if let Some(group) =
                                            this.workspace.collections.get_mut(*collection)
                                        {
                                            group.remove(path, &mut this.workspace.requests);
                                            this.collapsed_directories.retain(|(c, folder)| {
                                                *c != *collection
                                                    || (folder != path
                                                        && !folder.starts_with(&format!("{path}/")))
                                            });
                                            this.current_collection = *collection;
                                            this.current_directory =
                                                RequestGroups::parent(path).to_owned();
                                            this.group_edit = None;
                                            this.workspace.storage_dirty = true;
                                            this.notice = "已删除目录，请求保留在上级目录中".into();
                                            this.sync_groups(cx);
                                        }
                                    }
                                }
                                cx.notify();
                            })),
                    ),
            );
        let owner = cx.entity().downgrade();
        let open = self.tree_delete.as_ref() == Some(&target);
        Popover::new(
            format!("tree-delete:{}", Self::node_key(&target)),
            trigger,
            content,
        )
        .theme(self.theme)
        .width(280.)
        .trigger_height(22.)
        .open(open)
        .on_open_change(move |open, window, cx| {
            let _ = owner.update(cx, |this, cx| {
                this.tree_delete = open.then(|| target.clone());
                this.tree_menu = None;
                window.focus(&this.focus, cx);
                cx.notify();
            });
        })
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hover(window: &mut gpui::VisualTestContext, selector: &'static str) {
        let bounds = window.debug_bounds(selector).unwrap();
        window.simulate_event(gpui::MouseMoveEvent {
            position: bounds.center(),
            ..Default::default()
        });
    }

    #[gpui::test]
    fn request_delete_requires_confirmation_and_supports_dismissal(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name.update(cx, |e, cx| e.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            host
        });
        for dismissal in ["cancel", "escape", "outside"] {
            hover(window, "request-leaf:0");
            let more = window.debug_bounds("request-more:0").unwrap();
            window.simulate_click(more.center(), gpui::Modifiers::default());
            let trigger = window.debug_bounds("delete-request:0").unwrap();
            window.simulate_click(trigger.center(), gpui::Modifiers::default());
            host.update(window, |host, _| {
                assert_eq!(host.workspace.requests.len(), 1);
                assert_eq!(host.tree_delete, Some(model::TreeNode::Request(0)));
            });
            assert!(window.debug_bounds("tree-delete-confirmation").is_some());
            match dismissal {
                "cancel" => {
                    let cancel = window.debug_bounds("cancel-tree-delete").unwrap();
                    window.simulate_click(cancel.center(), gpui::Modifiers::default());
                }
                "escape" => window.simulate_keystrokes("escape"),
                _ => window.simulate_click(point(px(650.), px(180.)), gpui::Modifiers::default()),
            }
            host.update(window, |host, _| {
                assert!(host.tree_delete.is_none());
                assert_eq!(host.workspace.requests.len(), 1);
            });
        }
        hover(window, "request-leaf:0");
        let more = window.debug_bounds("request-more:0").unwrap();
        window.simulate_click(more.center(), gpui::Modifiers::default());
        let trigger = window.debug_bounds("delete-request:0").unwrap();
        window.simulate_click(trigger.center(), gpui::Modifiers::default());
        let confirm = window.debug_bounds("confirm-tree-delete").unwrap();
        // Moving from the row to the popup must not unmount the confirmation.
        window.simulate_event(gpui::MouseMoveEvent {
            position: confirm.center(),
            ..Default::default()
        });
        window.simulate_click(confirm.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert!(host.workspace.requests.is_empty());
            assert!(host.tree_delete.is_none());
            host.persist(cx);
        });
        assert!(
            storage::Store::open(&path)
                .unwrap()
                .load()
                .unwrap()
                .requests
                .is_empty()
        );
    }

    #[gpui::test]
    fn request_menu_copies_current_draft_with_new_identity(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name.update(cx, |e, cx| e.set_value("API", cx));
            host.save_group(cx);
            host.add_request(cx);
            host
        });
        let original = host.update(window, |host, cx| {
            let mut draft = host.workspace.requests[0].draft.clone();
            draft.method = "POST".into();
            draft.url = "https://example.test/copied".into();
            draft.body = r#"{"message":"copy me"}"#.into();
            host.load(&draft, cx);
            host.capture_draft(cx);
            host.workspace.requests[0].clone()
        });
        hover(window, "request-leaf:0");
        let more = window.debug_bounds("request-more:0").unwrap();
        window.simulate_click(more.center(), gpui::Modifiers::default());
        assert!(window.debug_bounds("delete-request:0").is_some());
        window.simulate_keystrokes("escape");
        host.update(window, |host, _| assert!(host.request_menu.is_none()));
        hover(window, "request-leaf:0");
        let more = window.debug_bounds("request-more:0").unwrap();
        window.simulate_click(more.center(), gpui::Modifiers::default());
        let copy = window.debug_bounds("copy-request:0").unwrap();
        window.simulate_event(gpui::MouseMoveEvent {
            position: copy.center(),
            ..Default::default()
        });
        window.simulate_click(copy.center(), gpui::Modifiers::default());
        host.update(window, |host, cx| {
            assert_eq!(host.workspace.requests.len(), 2);
            assert_eq!(host.active, 1);
            assert!(host.request_menu.is_none());
            assert!(host.tree_delete.is_none());
            let copy = &host.workspace.requests[1];
            assert_ne!(copy.id, original.id);
            assert_eq!(copy.draft, original.draft);
            assert_eq!(copy.collection, original.collection);
            assert_eq!(copy.group, original.group);
            assert_eq!(copy.name, format!("{} 副本", original.name));
            assert!(copy.history.is_empty());
            assert_eq!(copy.versions.len(), 1);
            assert_ne!(copy.versions[0].id, original.versions[0].id);
            host.duplicate_request(0, cx);
            assert_eq!(
                host.workspace.requests[2].name,
                format!("{} 副本 2", original.name)
            );
            host.persist(cx);
        });
        let saved = storage::Store::open(&path).unwrap().load().unwrap();
        assert_eq!(saved.requests.len(), 3);
        assert!(saved.requests.iter().all(|r| r.draft == original.draft));
    }

    #[gpui::test]
    fn directory_delete_confirms_and_preserves_nested_requests(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name.update(cx, |e, cx| e.set_value("API", cx));
            host.save_group(cx);
            host.workspace.collections[0]
                .add_child("", "folder")
                .unwrap();
            host.workspace.collections[0]
                .add_child("folder", "child")
                .unwrap();
            host.add_request_to("folder/child".into(), cx);
            host
        });
        for confirm in [false, true] {
            hover(window, "request-directory:0:folder");
            let more = window.debug_bounds("directory-menu:0:folder").unwrap();
            window.simulate_click(more.center(), gpui::Modifiers::default());
            let delete = window
                .debug_bounds("directory-menu-action:0:folder:3")
                .unwrap();
            window.simulate_click(delete.center(), gpui::Modifiers::default());
            host.update(window, |host, _| {
                assert!(host.workspace.collections[0].contains("folder/child"));
                assert_eq!(host.workspace.requests[0].group, "folder/child");
                assert!(host.tree_menu.is_none());
                assert!(host.tree_delete.is_some());
            });
            let action = window
                .debug_bounds(if confirm {
                    "confirm-tree-delete"
                } else {
                    "cancel-tree-delete"
                })
                .unwrap();
            window.simulate_event(gpui::MouseMoveEvent {
                position: action.center(),
                ..Default::default()
            });
            window.simulate_click(action.center(), gpui::Modifiers::default());
        }
        host.update(window, |host, cx| {
            assert!(host.workspace.collections[0].names.is_empty());
            assert_eq!(host.workspace.requests.len(), 1);
            assert!(host.workspace.requests[0].group.is_empty());
            assert!(host.tree_delete.is_none());
            host.persist(cx);
        });
        let saved = storage::Store::open(&path).unwrap().load().unwrap();
        assert!(saved.collections[0].names.is_empty());
        assert_eq!(saved.requests.len(), 1);
        assert!(saved.requests[0].group.is_empty());
    }
}
