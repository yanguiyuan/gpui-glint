use super::*;

impl Workbench {
    pub(super) fn tree_delete_popover(
        &self,
        target: model::TreeNode,
        trigger: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let (title, description) = match &target {
            model::TreeNode::Request(index) => (
                format!("删除请求「{}」？", self.requests[*index].name),
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
                                        if let Some(group) = this.collections.get_mut(*collection) {
                                            group.remove(path, &mut this.requests);
                                            this.collapsed_directories.retain(|(c, folder)| {
                                                *c != *collection
                                                    || (folder != path
                                                        && !folder.starts_with(&format!("{path}/")))
                                            });
                                            this.current_collection = *collection;
                                            this.current_directory =
                                                RequestGroups::parent(path).to_owned();
                                            this.group_edit = None;
                                            this.storage_dirty = true;
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
            let trigger = window.debug_bounds("delete-request:0").unwrap();
            window.simulate_click(trigger.center(), gpui::Modifiers::default());
            host.update(window, |host, _| {
                assert_eq!(host.requests.len(), 1);
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
                assert_eq!(host.requests.len(), 1);
            });
        }
        hover(window, "request-leaf:0");
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
            assert!(host.requests.is_empty());
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
    fn directory_delete_confirms_and_preserves_nested_requests(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.sqlite3");
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = Workbench::with_store(storage::Store::open(&path).unwrap(), cx);
            host.group_edit = Some(GroupEdit::CreateCollection);
            host.group_name.update(cx, |e, cx| e.set_value("API", cx));
            host.save_group(cx);
            host.collections[0].add_child("", "folder").unwrap();
            host.collections[0].add_child("folder", "child").unwrap();
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
                assert!(host.collections[0].contains("folder/child"));
                assert_eq!(host.requests[0].group, "folder/child");
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
            assert!(host.collections[0].names.is_empty());
            assert_eq!(host.requests.len(), 1);
            assert!(host.requests[0].group.is_empty());
            assert!(host.tree_delete.is_none());
            host.persist(cx);
        });
        let saved = storage::Store::open(&path).unwrap().load().unwrap();
        assert!(saved.collections[0].names.is_empty());
        assert_eq!(saved.requests.len(), 1);
        assert!(saved.requests[0].group.is_empty());
    }
}
