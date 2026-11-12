use super::*;
use gpui_glint::components::{GroupListGroup, GroupListHeader, GroupListItem};

impl AgentWorkspace {
    pub(super) fn remove_project_dialog(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let path = self.project_grouping.removing.as_ref().unwrap();
        let p = self.theme.palette();
        let body = div().flex().flex_col().gap_4()
            .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child("移除项目？"))
            .child(div().p_3().rounded(px(8.)).bg(rgb(p.surface_subtle)).flex().flex_col().gap_1()
                .child(project_grouping::project_label(&self.history, path))
                .child(div().text_size(px(11.)).text_color(rgb(p.text_muted)).truncate()
                    .child(gpui_glint::paths::display_path(path))))
            .child("仅从项目列表移除，磁盘文件和历史会话会保留。再次打开此项目即可恢复。")
            .child(div().flex().justify_end().gap_2()
                .child(Button::new("取消").kind(ButtonKind::Plain).theme(self.theme).render()
                    .debug_selector(|| "cancel-remove-project".into())
                    .on_click(cx.listener(|this, _, _, cx| { this.project_grouping.removing = None; cx.notify(); })))
                .child(Button::new("移除项目").kind(ButtonKind::Danger).theme(self.theme).render()
                    .debug_selector(|| "confirm-remove-project".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(path) = this.project_grouping.removing.take() { this.remove_project(path, cx); }
                        cx.notify();
                    }))));
        let weak = cx.entity().downgrade();
        gpui_glint::components::Dialog::new("remove-project-dialog", body).width(480.).theme(self.theme)
            .on_close(move |_, _, cx| { let _ = weak.update(cx, |this, cx| {
                this.project_grouping.removing = None;
                cx.notify();
            }); }).render()
    }

    pub(super) fn remove_project(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.history.projects.contains(&path) { return; }
        self.capture_draft(cx);
        self.history.projects.retain(|project| project != &path);
        self.history.removed_projects.insert(path.clone());
        if self.history.active().project.as_ref() == Some(&path) {
            let next = self.history.sessions.iter().find(|session| session.project.as_ref().is_some_and(|project| self.history.projects.contains(project))).map(|session| session.id);
            if let Some(next) = next { self.switch(next, cx); }
            else {
                self.history.new_session_for(self.history.projects.first().cloned());
                self.restore_draft(cx);
                self.open_conversation_at_latest();
            }
            self.edits_ui.open = false;
            self.file_preview.open = false;
            self.git_ui.open = false;
            self.terminal_ui.open = false;
            self.project_actions.panel = false;
        }
        self.project_picker_open = false;
        self.project_grouping.editing = None;
        self.git_ui.toasts.update(cx, |toasts, cx| toasts.push(
            gpui_glint::components::Toast::new("已从列表移除项目")
                .description("文件与历史会话已保留，再次打开项目即可恢复。"), cx));
        self.save(cx);
        cx.notify();
    }

    pub fn choose_project(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.capture_draft(cx);
        self.history.removed_projects.remove(&path);
        if !self.history.projects.contains(&path) {
            self.history.projects.push(path.clone());
        }
        self.refresh_project_metadata(vec![path.clone()], cx);
        // Preserve older standalone conversations by migrating them into the
        // first project chosen after the project-only workflow is enabled.
        for session in &mut self.history.sessions {
            if session.project.is_none()
                && (!session.messages.is_empty()
                    || !session.draft.is_empty()
                    || !session.draft_images.is_empty())
            {
                session.project = Some(path.clone());
            }
        }
        let active = self.history.active;
        let running = self.session_busy(active);
        if self.history.active().messages.is_empty() && !running {
            self.history.active_mut().project = Some(path);
        } else if self.history.active().project.as_ref() != Some(&path) {
            self.history.new_session_for(Some(path));
        }
        self.restore_draft(cx);
        images::release_previews(self.image_views.borrow_mut().clear(), cx);
        self.project_picker_open = false;
        if matches!(self.page, crate::Page::Settings | crate::Page::Calendar) {
            self.page = crate::Page::Chat;
        }
        self.save(cx);
        cx.notify();
    }

    pub fn project_selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let label = self
            .history
            .active()
            .project
            .as_ref()
            .map(|path| project_grouping::repository_label(&self.history, path))
            .unwrap_or_else(|| "选择项目".into());
        let trigger = div()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(px(8.))
            .text_color(rgb(p.text))
            .hover(|style| style.bg(rgb(p.border)))
            .child(Icon::Folder.render(16.))
            .child(label)
            .child(Icon::ChevronDown.render(12.));
        let mut content = div().flex().flex_col().gap_1();
        for path in &self.history.projects {
            let path = path.clone();
            let label = project_grouping::project_label(&self.history, &path);
            content = content.child(
                div()
                    .id(SharedString::from(format!(
                        "choose-project:{}",
                        path.display()
                    )))
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(p.surface_subtle)))
                    .child(Icon::Folder.render(15.))
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.choose_project(path.clone(), cx)),
                    ),
            );
        }
        content = content.child(
            Button::new("设置当前项目分组…")
                .icon(Icon::Folders)
                .kind(ButtonKind::Plain)
                .theme(self.theme)
                .render()
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(path) = this.history.active().project.clone() {
                        this.edit_project_group(path, cx);
                    }
                })),
        );
        content = content.child(
            Button::new("打开项目目录…")
                .icon(Icon::Plus)
                .kind(ButtonKind::Plain)
                .theme(self.theme)
                .render()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_picker_open = false;
                    this.open_project(cx);
                    cx.notify();
                })),
        );
        content = content.child(
            Button::new("打开 SSH 项目…")
                .icon(Icon::Globe)
                .kind(ButtonKind::Plain)
                .theme(self.theme)
                .render()
                .on_click(cx.listener(|this, _, window, cx| this.open_ssh_project(window, cx))),
        );
        let weak = cx.entity().downgrade();
        Popover::new("composer-project-selector", trigger, content)
            .open(self.project_picker_open)
            .above_trigger()
            .trigger_height(28.)
            .width(280.)
            .padding(6.)
            .theme(self.theme)
            .on_open_change(move |open, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.project_picker_open = open;
                    if open {
                        this.git_ui.picker_open = false;
                    }
                    cx.notify();
                });
            })
    }

    pub fn pick_context(&mut self, directories: bool, cx: &mut Context<Self>) {
        if self
            .history
            .active()
            .project
            .as_deref()
            .is_some_and(ssh_project::is_remote)
        {
            self.refresh_project_context(cx);
            self.composer
                .update(cx, |composer, cx| composer.open_context_panel(cx));
            return;
        }
        let session = self.history.active;
        let picker = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: !directories,
            directories,
            multiple: true,
            prompt: Some(if directories {
                "添加目录".into()
            } else {
                "添加文件".into()
            }),
        });
        cx.spawn(async move |weak, cx| {
            let result = picker.await;
            let _ = weak.update(cx, |this, cx| {
                if this.history.active != session {
                    return;
                }
                match result {
                    Ok(Ok(Some(paths))) => {
                        this.composer.update(cx, |composer, cx| {
                            for path in paths {
                                composer.add_reference(
                                    gpui_glint::components::ComposerReference::file(
                                        path.to_string_lossy().into_owned(),
                                        directories,
                                    ),
                                    cx,
                                );
                            }
                        });
                        this.capture_draft(cx);
                        this.save(cx);
                    }
                    Ok(Ok(None)) => {}
                    _ => this.provider_error = Some("无法选择文件或目录".into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn open_project(&mut self, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("打开项目".into()),
        });
        cx.spawn(async move |weak, cx| {
            let result = picker.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            match path.canonicalize().and_then(|path| {
                                if path.is_dir() {
                                    Ok(path)
                                } else {
                                    Err(std::io::Error::other("not a directory"))
                                }
                            }) {
                                Ok(path) => {
                                    this.choose_project(path, cx);
                                }
                                Err(_) => this.provider_error = Some("无法打开项目目录".into()),
                            }
                        }
                    }
                    Ok(Ok(None)) => {}
                    _ => this.provider_error = Some("无法选择项目目录".into()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

pub fn session_groups(
    history: &History,
    query: &str,
    running: impl IntoIterator<Item = Uuid>,
) -> Vec<GroupListGroup> {
    session_groups_with_partners(history, query, running, &[])
}

pub fn session_groups_with_partners(
    history: &History,
    query: &str,
    running: impl IntoIterator<Item = Uuid>,
    partners: &[partners::Partner],
) -> Vec<GroupListGroup> {
    if history.group_by_partner {
        return partner_session_groups(history, query, running, partners);
    }
    let running: std::collections::HashSet<_> = running.into_iter().collect();
    let query = query.trim().to_lowercase();
    let mut sessions: Vec<_> = history
        .sessions
        .iter()
        .filter(|session| {
            query.is_empty()
                || session.title.to_lowercase().contains(&query)
                || session.project.as_ref().is_some_and(|path| {
                    project_grouping::project_label(history, path).to_lowercase().contains(&query)
                        || project_grouping::group_name(history, path).is_some_and(|name| name.to_lowercase().contains(&query))
                })
                || session
                    .messages
                    .iter()
                    .any(|message| message.text.to_lowercase().contains(&query))
        })
        .collect();
    sessions.sort_by_key(|session| std::cmp::Reverse(session.updated));
    let mut groups: Vec<GroupListGroup> = Vec::new();
    if history.group_by_project {
        for path in &history.projects {
            let id = format!("project:{}", path.display());
            if groups.iter().any(|group| group.header.id.as_ref() == id) {
                continue;
            }
            groups.push(GroupListGroup::new(
                GroupListHeader::new(id, project_grouping::project_label(history, path))
                    .icon(Icon::Folder)
                    .actions([Icon::Plus, Icon::More])
                    .menu([("设置分组".into(), Icon::Folders), ("移除项目".into(), Icon::Trash)]),
            ));
        }
    }
    for session in sessions {
        let (id, label, icon) = if history.group_by_project {
            match &session.project {
                Some(path) if !history.removed_projects.contains(path) => (
                    format!("project:{}", path.display()),
                    project_grouping::project_label(history, path),
                    Icon::Folder,
                ),
                _ => continue,
            }
        } else {
            let day = session.updated.format("%Y-%m-%d").to_string();
            let label = if session.updated.date_naive() == chrono::Local::now().date_naive() {
                "今天".into()
            } else {
                day.clone()
            };
            (format!("date:{day}"), label, Icon::Clock)
        };
        let index = groups
            .iter()
            .position(|group| group.header.id.as_ref() == id)
            .unwrap_or_else(|| {
                groups.push(GroupListGroup::new(
                    GroupListHeader::new(id, label).icon(icon),
                ));
                groups.len() - 1
            });
        let mut item = GroupListItem::new(session.id.to_string(), session.title.clone())
            .reserve_icon_space(true)
            .meta(session.updated.format("%H:%M").to_string())
            .action(Icon::Trash)
            .confirm_action("删除此会话？");
        if running.contains(&session.id) {
            item = item.icon(Icon::Spinner);
        }
        groups[index].items.push(item);
    }
    if history.group_by_project {
        nest_project_groups(history, groups)
    } else {
        groups
    }
}

fn nest_project_groups(history: &History, groups: Vec<GroupListGroup>) -> Vec<GroupListGroup> {
    let mut result: Vec<GroupListGroup> = Vec::new();
    for mut group in groups {
        let Some(path) = group.header.id.strip_prefix("project:").map(PathBuf::from) else {
            result.push(group); continue;
        };
        let Some(name) = project_grouping::group_name(history, &path) else {
            result.push(group); continue;
        };
        group.header.label = project_grouping::repository_label(history, &path).into();
        let id = format!("project-group:{name}");
        let index = result.iter().position(|parent| parent.header.id.as_ref() == id).unwrap_or_else(|| {
            result.push(GroupListGroup::new(GroupListHeader::new(id, name.to_owned()).icon(Icon::Folders)));
            result.len() - 1
        });
        result[index].children.push(group);
    }
    result
}

fn partner_session_groups(
    history: &History,
    query: &str,
    running: impl IntoIterator<Item = Uuid>,
    partners: &[partners::Partner],
) -> Vec<GroupListGroup> {
    let running: std::collections::HashSet<_> = running.into_iter().collect();
    let query = query.trim().to_lowercase();
    let mut groups: Vec<_> = partners
        .iter()
        .filter(|p| query.is_empty() || p.name.to_lowercase().contains(&query))
        .map(|p| {
            GroupListGroup::new(
                GroupListHeader::new(format!("partner:{}", p.id), p.name.clone())
                    .icon(Icon::User)
                    .actions([Icon::Plus, Icon::Settings]),
            )
        })
        .collect();
    let mut sessions: Vec<_> = history.sessions.iter().collect();
    sessions.sort_by_key(|s| std::cmp::Reverse(s.updated));
    for session in sessions {
        let partner = partners.iter().find(|p| Some(p.id) == session.partner_id);
        if !query.is_empty()
            && !session.title.to_lowercase().contains(&query)
            && !session
                .messages
                .iter()
                .any(|m| m.text.to_lowercase().contains(&query))
            && !partner.is_some_and(|p| p.name.to_lowercase().contains(&query))
        {
            continue;
        }
        let (id, label, actions) = match partner {
            Some(p) => (
                format!("partner:{}", p.id),
                p.name.clone(),
                vec![Icon::Plus, Icon::Settings],
            ),
            None => ("partner:none".into(), "普通 Agent".into(), vec![Icon::Plus]),
        };
        let index = groups
            .iter()
            .position(|g| g.header.id.as_ref() == id)
            .unwrap_or_else(|| {
                groups.push(GroupListGroup::new(
                    GroupListHeader::new(id, label)
                        .icon(Icon::User)
                        .actions(actions),
                ));
                groups.len() - 1
            });
        let mut item = GroupListItem::new(session.id.to_string(), session.title.clone())
            .reserve_icon_space(true)
            .meta(session.updated.format("%H:%M").to_string())
            .action(Icon::Trash)
            .confirm_action("删除此会话？");
        if running.contains(&session.id) {
            item = item.icon(Icon::Spinner);
        }
        groups[index].items.push(item);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn project_more_menu_removes_persistently_and_reopening_restores_history(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("keep.txt"), "keep").unwrap();
        let path = dir.path().join("sessions.json");
        std::fs::write(dir.path().join("providers.json"), r#"{"providers":[],"active":null,"active_model":""}"#).unwrap();
        let (host, window) = cx.add_window_view(|_, cx| AgentWorkspace::with_path(path.clone(), cx));
        window.simulate_resize(size(px(1200.), px(900.)));
        let old_session = host.update(window, |host, cx| {
            host.history.projects.push(project.clone());
            host.history.active_mut().project = Some(project.clone());
            host.history.active_mut().push(Role::User, "保留这条历史".into());
            cx.notify();
            host.history.active
        });
        let group = SharedString::from(format!("project:{}", project.display()));
        let list = host.read_with(window, |host, _| host.group_list.clone());
        list.update(window, |_, cx| cx.emit(GroupListEvent::HeaderMenuAction { group: group.clone(), index: 0 }));
        host.read_with(window, |host, _| assert_eq!(host.project_grouping.editing.as_ref(), Some(&project)));
        list.update(window, |_, cx| cx.emit(GroupListEvent::HeaderMenuAction { group: group.clone(), index: 1 }));
        host.read_with(window, |host, _| {
            assert!(host.history.projects.contains(&project));
            assert_eq!(host.project_grouping.removing.as_ref(), Some(&project));
        });
        let cancel = window.debug_bounds("cancel-remove-project").unwrap();
        window.simulate_click(cancel.center(), Default::default());
        host.read_with(window, |host, _| {
            assert!(host.project_grouping.removing.is_none());
            assert!(host.history.projects.contains(&project));
        });
        list.update(window, |_, cx| cx.emit(GroupListEvent::HeaderMenuAction { group, index: 1 }));
        let confirm = window.debug_bounds("confirm-remove-project").unwrap();
        window.simulate_click(confirm.center(), Default::default());
        host.update(window, |host, _| {
            assert!(!host.history.projects.contains(&project));
            assert_ne!(host.history.active, old_session);
            assert!(host.history.sessions.iter().any(|session| session.id == old_session && session.messages[0].text == "保留这条历史"));
            assert!(session_groups(&host.history, "", None).is_empty());
            host.history.save(&path).unwrap();
            let restored = History::load(&path).unwrap();
            assert!(!restored.projects.contains(&project));
            assert!(session_groups(&restored, "", None).is_empty());
        });
        assert_eq!(std::fs::read_to_string(project.join("keep.txt")).unwrap(), "keep");
        host.update(window, |host, cx| {
            host.choose_project(project.clone(), cx);
            assert!(!host.history.removed_projects.contains(&project));
            assert_eq!(session_groups(&host.history, "", None).len(), 1);
            assert!(host.history.sessions.iter().any(|session| session.id == old_session));
        });
    }

    #[test]
    fn worktree_groups_keep_repository_and_session_identity_across_repositories() {
        let mut history = History::default();
        let idl = PathBuf::from("/worktree/idl/feat/chat-governance-openapi");
        let kim = PathBuf::from("/worktree/kim/feat/chat-governance-openapi");
        for (path, name) in [(&idl, "idl"), (&kim, "kim")] {
            history.new_session_for(Some(path.clone()));
            history.project_metadata.insert(path.clone(), project_grouping::Metadata {
                repository_name: name.into(), branch: "feat/chat-governance-openapi".into(), linked_worktree: true,
            });
        }
        let active = history.active;
        let groups = session_groups(&history, "", Some(active));
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].header.label.as_ref(), "feat/chat-governance-openapi");
        assert_eq!(groups[0].children.len(), 2);
        assert_eq!(groups[0].children[0].header.label.as_ref(), "idl");
        assert_eq!(groups[0].children[1].header.label.as_ref(), "kim");
        assert_ne!(groups[0].children[0].header.id, groups[0].children[1].header.id);
        assert_eq!(groups[0].children[1].items[0].id.as_ref(), active.to_string());
        assert_eq!(groups[0].children[1].items[0].icon, Some(Icon::Spinner));
        assert_eq!(session_groups(&history, "idl", None)[0].children[0].items.len(), 1);
        assert_eq!(session_groups(&history, "chat-governance", None)[0].children.iter().map(|group| group.items.len()).sum::<usize>(), 2);
        history.project_group_overrides.insert(idl.clone(), "custom".into());
        assert_eq!(session_groups(&history, "", None).len(), 2);
        history.project_group_overrides.insert(kim, String::new());
        let ungrouped = session_groups(&history, "", None);
        assert!(ungrouped[1].header.label.contains("kim"));
        assert!(ungrouped[1].children.is_empty());
    }

    #[test]
    fn companion_groups_include_empty_partners_and_cross_project_sessions_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let repo = partners::Repository::new(dir.path().join("sessions.json"));
        let a = repo.create().unwrap();
        let b = repo.create().unwrap();
        repo.update(a, "知微", "◌", "").unwrap();
        repo.update(b, "林舟", "◌", "").unwrap();
        let ps = repo.list().unwrap();
        let mut h = History::default();
        h.preferred_partner = Some(a);
        h.active_mut().partner_id = Some(a);
        h.active_mut().project = Some("/a".into());
        h.active_mut().push(Role::User, "实现滚动".into());
        let first = h.active;
        h.new_session_for(Some("/b".into()));
        h.active_mut().push(Role::User, "验证边界".into());
        let second = h.active;
        h.group_by_partner = true;
        let groups = session_groups_with_partners(&h, "", Some(first), &ps);
        assert_eq!(groups.len(), 2);
        let group = groups
            .iter()
            .find(|g| g.header.id.as_ref() == format!("partner:{a}"))
            .unwrap();
        assert_eq!(group.items.len(), 2);
        assert!(
            group
                .items
                .iter()
                .any(|i| i.id.as_ref() == first.to_string())
        );
        assert!(
            group
                .items
                .iter()
                .any(|i| i.id.as_ref() == second.to_string())
        );
        assert!(
            groups
                .iter()
                .find(|g| g.header.id.as_ref() == format!("partner:{b}"))
                .unwrap()
                .items
                .is_empty()
        );
        let searched = session_groups_with_partners(&h, "知微", None, &ps);
        assert_eq!(searched.len(), 1);
        assert_eq!(searched[0].items.len(), 2);
        let by_title = session_groups_with_partners(&h, "验证边界", None, &ps);
        assert_eq!(by_title.len(), 1);
        assert_eq!(by_title[0].items.len(), 1);
    }

    #[test]
    fn deleting_every_session_preserves_empty_project_headers() {
        let mut history = History::default();
        history.new_session_for(Some("/project/a".into()));
        let a = history.active;
        history.active_mut().push(Role::User, "A".into());
        history.new_session_for(Some("/project/b".into()));
        let b = history.active;
        history.remove(a);
        history.remove(b);
        let groups = session_groups(&history, "", None);
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().all(|group| group.items.is_empty()));
        assert!(groups.iter().all(|group| group.header.actions.len() == 2));
        history.new_session_for(Some("/project/a".into()));
        assert_eq!(
            history.active().project.as_deref(),
            Some(std::path::Path::new("/project/a"))
        );
    }

    #[test]
    fn only_the_running_session_has_a_leading_icon() {
        let mut history = History::default();
        history.active_mut().project = Some("/project/a".into());
        let running = history.active;
        history.active_mut().push(Role::User, "Running".into());
        history.new_session();
        for state in [None, Some(running)] {
            let groups = session_groups(&history, "", state);
            for item in groups.iter().flat_map(|group| &group.items) {
                if state.is_some() && item.id.as_ref() == running.to_string() {
                    assert!(matches!(item.icon, Some(Icon::Spinner)));
                } else {
                    assert!(item.icon.is_none());
                }
            }
        }
    }

    #[test]
    fn project_sessions_stay_separate_and_support_both_grouping_modes() {
        let mut history = History::default();
        history.new_session_for(Some("/project/a".into()));
        history.active_mut().push(Role::User, "First".into());
        history.new_session_for(Some("/project/b".into()));
        assert_eq!(
            history.active().project.as_deref(),
            Some(std::path::Path::new("/project/b"))
        );
        assert_eq!(session_groups(&history, "", None).len(), 2);
        history.group_by_project = false;
        assert_eq!(session_groups(&history, "", None).len(), 1);
        assert_eq!(session_groups(&history, "First", None)[0].items.len(), 1);
    }
}
