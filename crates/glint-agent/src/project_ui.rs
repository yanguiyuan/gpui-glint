use super::*;
use gpui_glint::components::{
    GroupListGroup, GroupListHeader, GroupListItem, MenuItem, ProjectOption, ProjectSelector,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkspacePicker {
    ChatProject,
    BrowserProject,
    ComposerProject,
    ChatSession,
}

impl AgentWorkspace {
    pub(super) fn remove_project_dialog(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let path = self.project_grouping.removing.as_ref().unwrap();
        let p = self.theme.palette();
        let body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("移除项目？"),
            )
            .child(
                div()
                    .p_3()
                    .rounded(px(8.))
                    .bg(rgb(p.surface_subtle))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(project_grouping::project_label(&self.history, path))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(p.text_muted))
                            .truncate()
                            .child(gpui_glint::paths::display_path(path)),
                    ),
            )
            .child("仅从项目列表移除，磁盘文件和历史会话会保留。再次打开此项目即可恢复。")
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("取消")
                            .kind(ButtonKind::Plain)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "cancel-remove-project".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.project_grouping.removing = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("移除项目")
                            .kind(ButtonKind::Danger)
                            .theme(self.theme)
                            .render()
                            .debug_selector(|| "confirm-remove-project".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(path) = this.project_grouping.removing.take() {
                                    this.remove_project(path, cx);
                                }
                                cx.notify();
                            })),
                    ),
            );
        let weak = cx.entity().downgrade();
        gpui_glint::components::Dialog::new("remove-project-dialog", body)
            .width(480.)
            .theme(self.theme)
            .on_close(move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.project_grouping.removing = None;
                    cx.notify();
                });
            })
            .render()
    }

    pub(super) fn remove_project(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.history.projects.contains(&path) {
            return;
        }
        self.capture_draft(cx);
        self.history.projects.retain(|project| project != &path);
        self.history.removed_projects.insert(path.clone());
        if self.history.agentspace.as_ref() == Some(&path) {
            self.history.agentspace = None;
        }
        if self.history.active().project.as_ref() == Some(&path) {
            let next = self
                .history
                .sessions
                .iter()
                .find(|session| {
                    session
                        .project
                        .as_ref()
                        .is_some_and(|project| self.history.projects.contains(project))
                })
                .map(|session| session.id);
            if let Some(next) = next {
                self.switch(next, cx);
            } else {
                self.history
                    .new_session_for(self.history.projects.first().cloned());
                self.restore_draft(cx);
                self.open_conversation_at_latest();
            }
            self.edits_ui.open = false;
            self.file_preview.open = false;
            self.git_ui.open = false;
            self.terminal_ui.open = false;
            self.project_actions.panel = false;
        }
        self.workspace_picker = None;
        self.project_grouping.editing = None;
        self.git_ui.toasts.update(cx, |toasts, cx| {
            toasts.push(
                gpui_glint::components::Toast::new("已从列表移除项目")
                    .description("文件与历史会话已保留，再次打开项目即可恢复。"),
                cx,
            )
        });
        self.save(cx);
        cx.notify();
    }

    pub fn choose_project(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.page == crate::Page::Projects {
            let restored = self.history.removed_projects.remove(&path);
            let added = !self.history.projects.contains(&path);
            if added {
                self.history.projects.push(path.clone());
            }
            if added || !self.history.project_metadata.contains_key(&path) {
                self.refresh_project_metadata(vec![path.clone()], cx);
            }
            self.browser_project = Some(path);
            self.workspace_picker = None;
            if added || restored {
                self.save(cx);
            }
            self.open_browser_project(cx);
            return;
        }
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
        self.workspace_picker = None;
        if matches!(self.page, crate::Page::Settings | crate::Page::Calendar) {
            self.page = crate::Page::Chat;
        }
        self.save(cx);
        cx.notify();
    }

    pub fn project_selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.project_selector_at(WorkspacePicker::ComposerProject, cx)
    }

    pub(super) fn project_selector_at(
        &self,
        picker: WorkspacePicker,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = self.theme.palette();
        let selected_project = if picker == WorkspacePicker::BrowserProject {
            self.browser_project.as_ref()
        } else {
            self.history.active().project.as_ref()
        };
        let label = selected_project
            .map(|path| project_grouping::project_label(&self.history, path))
            .unwrap_or_else(|| "选择项目".into());
        let icon = selected_project
            .map(|path| project_grouping::project_icon(&self.history, path))
            .unwrap_or(Icon::Folder);
        let (id, trigger_selector) = match picker {
            WorkspacePicker::ChatProject => ("chat-project-selector", "agent-topbar-project"),
            WorkspacePicker::BrowserProject => {
                ("browser-project-selector", "browser-topbar-project")
            }
            WorkspacePicker::ComposerProject => {
                ("composer-project-selector", "composer-project-trigger")
            }
            WorkspacePicker::ChatSession => unreachable!("session picker uses its own list"),
        };
        let projects = if self.workspace_picker == Some(picker) {
            self.history
                .projects
                .iter()
                .map(|path| ProjectOption {
                    path: path.clone(),
                    label: project_grouping::project_label(&self.history, path),
                    icon: project_grouping::project_icon(&self.history, path),
                    selected: selected_project == Some(path),
                    default_space: self.history.agentspace.as_ref() == Some(path),
                })
                .collect()
        } else {
            Vec::new()
        };
        let section = |label: &str| {
            div()
                .px_2()
                .pt_2()
                .pb_1()
                .text_size(px(10.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(p.text_muted))
                .child(label.to_owned())
        };
        let divider = || div().h(px(1.)).mx_2().my_1().bg(rgb(p.border));
        let mut content = div()
            .flex()
            .flex_col()
            .child(
                MenuItem::new("picker-open-project", "打开本地项目…")
                    .icon(Icon::FolderOpen)
                    .theme(self.theme)
                    .render()
                    .debug_selector(|| "picker-open-project".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.workspace_picker = None;
                        this.open_project(cx);
                        cx.notify();
                    })),
            )
            .child(
                MenuItem::new("picker-open-ssh", "连接 SSH 项目…")
                    .icon(Icon::Globe)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.workspace_picker = None;
                        this.open_ssh_project(window, cx);
                    })),
            );
        if let Some(path) = selected_project.cloned() {
            content = content.child(
                MenuItem::new("picker-project-group", "项目分组…")
                    .icon(Icon::Folders)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit_project_group(path.clone(), cx);
                    })),
            );
        }
        content = content
            .child(divider())
            .child(section("AgentSpace · 新会话默认目录"))
            .child(
                MenuItem::new("picker-choose-agentspace", "选择默认目录…")
                    .icon(Icon::Box)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.workspace_picker = None;
                        this.pick_agentspace(cx);
                    })),
            );
        if let Some(path) = selected_project
            .filter(|path| {
                !ssh_project::is_remote(path) && self.history.agentspace.as_ref() != Some(*path)
            })
            .cloned()
        {
            content = content.child(
                MenuItem::new("picker-set-agentspace", "将当前目录设为默认空间")
                    .icon(Icon::Box)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_agentspace(path.clone(), cx);
                    })),
            );
        }
        if self.history.agentspace.is_some() {
            content = content.child(
                MenuItem::new("picker-clear-agentspace", "取消默认空间")
                    .icon(Icon::Close)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.history.agentspace = None;
                        this.workspace_picker = None;
                        this.save(cx);
                        cx.notify();
                    })),
            );
        }
        let weak = cx.entity().downgrade();
        let popover = ProjectSelector::new(id, trigger_selector, label, icon, projects, self.theme)
            .open(self.workspace_picker == Some(picker))
            .popover(content, move |path, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    if picker == WorkspacePicker::ChatProject {
                        this.switch_chat_project(path.to_owned(), cx);
                    } else {
                        this.choose_project(path.to_owned(), cx);
                    }
                });
            });
        self.picker_popover(picker, popover, cx)
    }

    fn switch_chat_project(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.workspace_picker = None;
        if self.history.active().project.as_ref() == Some(&path) {
            cx.notify();
            return;
        }
        let session = self
            .history
            .sessions
            .iter()
            .filter(|session| session.project.as_ref() == Some(&path))
            .max_by_key(|session| session.updated)
            .map(|session| session.id);
        if let Some(session) = session {
            self.switch(session, cx);
        } else if self.history.active().project.is_none() {
            self.choose_project(path, cx);
        } else {
            let previous = self.history.active;
            self.history.new_session_for(Some(path));
            let session = std::mem::replace(&mut self.history.active, previous);
            self.switch(session, cx);
            self.save(cx);
        }
    }

    pub(super) fn session_selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        let active = self.history.active();
        let open = self.workspace_picker == Some(WorkspacePicker::ChatSession);
        let trigger = div()
            .debug_selector(|| "agent-topbar-session".into())
            .h(px(28.))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(6.))
            .hover(|style| style.bg(rgb(p.surface_subtle)))
            .child(div().max_w(px(200.)).truncate().child(active.title.clone()))
            .child(Icon::disclosure(12., open).text_color(rgb(p.text_muted)));
        let mut sessions = Vec::new();
        if open {
            sessions.extend(
                self.history
                    .sessions
                    .iter()
                    .filter(|session| session.project == active.project),
            );
            sessions.sort_by_key(|session| std::cmp::Reverse(session.updated));
        }
        let content = div()
            .id("session-picker-list")
            .max_h(px(280.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .children(sessions.into_iter().map(|session| {
                let id = session.id;
                let selector = format!("choose-session:{id}");
                MenuItem::new(SharedString::from(selector.clone()), session.title.clone())
                    .icon(if id == active.id {
                        Icon::CheckCircle
                    } else {
                        Icon::MessageCircle
                    })
                    .theme(self.theme)
                    .render()
                    .debug_selector(move || selector.clone())
                    .when(id == active.id, |row| row.bg(rgb(p.accent_tint)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.history.active != id {
                            this.switch(id, cx);
                        } else {
                            this.workspace_picker = None;
                            cx.notify();
                        }
                    }))
            }));
        let popover = Popover::new("chat-session-selector", trigger, content)
            .open(open)
            .trigger_height(28.)
            .width(316.)
            .padding(6.)
            .theme(self.theme);
        self.picker_popover(WorkspacePicker::ChatSession, popover, cx)
    }

    fn picker_popover(
        &self,
        picker: WorkspacePicker,
        popover: Popover,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let popover = popover.on_open_change(move |open, _, cx| {
            let _ = weak.update(cx, |this, cx| {
                if open {
                    this.workspace_picker = Some(picker);
                    this.git_ui.picker_open = false;
                } else if this.workspace_picker == Some(picker) {
                    this.workspace_picker = None;
                }
                cx.notify();
            });
        });
        if picker == WorkspacePicker::ComposerProject {
            popover.above_trigger()
        } else {
            popover
        }
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

    fn set_agentspace(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match agentspace::configure(&path) {
            Ok(path) => {
                self.history.agentspace = Some(path.clone());
                self.choose_project(path, cx);
                self.git_ui.toasts.update(cx, |toasts, cx| toasts.push(
                    gpui_glint::components::Toast::new("已设置默认 AgentSpace")
                        .description("加号将默认使用此目录。在 agentspace.toml 中配置关联仓库，下次发送消息时生效。"), cx));
            }
            Err(error) => self.provider_error = Some(error),
        }
        cx.notify();
    }

    fn pick_agentspace(&mut self, cx: &mut Context<Self>) {
        self.pick_project_directory(true, cx);
    }

    pub fn open_project(&mut self, cx: &mut Context<Self>) {
        self.pick_project_directory(false, cx);
    }

    fn pick_project_directory(&mut self, agentspace: bool, cx: &mut Context<Self>) {
        let picker = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(
                if agentspace {
                    "选择 AgentSpace 目录"
                } else {
                    "打开项目"
                }
                .into(),
            ),
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
                                    if agentspace {
                                        this.set_agentspace(path, cx);
                                    } else {
                                        this.choose_project(path, cx);
                                    }
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
                    project_grouping::project_label(history, path)
                        .to_lowercase()
                        .contains(&query)
                        || project_grouping::group_name(history, path)
                            .is_some_and(|name| name.to_lowercase().contains(&query))
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
                    .icon(project_grouping::project_icon(history, path))
                    .actions([Icon::Plus, Icon::More])
                    .menu([
                        ("设置分组".into(), Icon::Folders),
                        ("移除项目".into(), Icon::Trash),
                    ]),
            ));
        }
    }
    for session in sessions {
        let (id, label, icon) = if history.group_by_project {
            match &session.project {
                Some(path) if !history.removed_projects.contains(path) => (
                    format!("project:{}", path.display()),
                    project_grouping::project_label(history, path),
                    project_grouping::project_icon(history, path),
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
            result.push(group);
            continue;
        };
        let Some(name) = project_grouping::group_name(history, &path) else {
            result.push(group);
            continue;
        };
        group.header.label = project_grouping::repository_label(history, &path).into();
        let id = format!("project-group:{name}");
        let index = result
            .iter()
            .position(|parent| parent.header.id.as_ref() == id)
            .unwrap_or_else(|| {
                result.push(GroupListGroup::new(
                    GroupListHeader::new(id, name.to_owned()).icon(Icon::Folders),
                ));
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
    fn project_selectors_share_a_menu_and_chat_breadcrumb_switches_sessions(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        let (first_session, recent_session, older_session) = host.update(window, |host, cx| {
            host.history.new_session_for(Some(second.clone()));
            let older = host.history.active;
            host.history.active_mut().draft = "older draft".into();
            host.history.active_mut().updated -= chrono::Duration::minutes(1);
            host.history.new_session_for(Some(second.clone()));
            let recent = host.history.active;
            host.history.new_session_for(Some(first.clone()));
            host.restore_draft(cx);
            host.composer.update(cx, |composer, cx| {
                composer.set_draft("first draft", Vec::new(), cx);
            });
            cx.notify();
            (host.history.active, recent, older)
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        let click = |window: &mut gpui::VisualTestContext, selector: &'static str| {
            window.run_until_parked();
            let bounds = window
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector}"));
            window.simulate_click(bounds.center(), Default::default());
            window.run_until_parked();
        };
        let project = |path: &std::path::Path| -> &'static str {
            Box::leak(format!("choose-project:{}", path.display()).into_boxed_str())
        };
        let session =
            |id| -> &'static str { Box::leak(format!("choose-session:{id}").into_boxed_str()) };

        click(window, "composer-project-trigger");
        assert!(window.debug_bounds("picker-open-project").is_some());
        click(window, "agent-topbar-project");
        assert!(window.debug_bounds("picker-open-project").is_some());
        click(window, project(&second));
        host.read_with(window, |host, _| {
            assert_eq!(host.history.active, recent_session)
        });

        click(window, "agent-topbar-session");
        assert!(window.debug_bounds(session(first_session)).is_none());
        click(window, session(older_session));
        host.read_with(window, |host, cx| {
            assert_eq!(host.history.active, older_session);
            assert_eq!(host.composer.read(cx).value(cx), "older draft");
            let first = host
                .history
                .sessions
                .iter()
                .find(|s| s.id == first_session)
                .unwrap();
            assert_eq!(first.draft, "first draft");
        });

        host.update_in(window, |host, window, cx| {
            host.navigate(crate::Page::Projects, window, cx)
        });
        click(window, "browser-topbar-project");
        assert!(window.debug_bounds("picker-open-project").is_some());
        click(window, project(&first));
        host.read_with(window, |host, _| {
            assert_eq!(host.browser_project.as_ref(), Some(&first));
            assert_eq!(host.history.active, older_session);
        });
    }

    #[gpui::test]
    fn agentspace_default_switches_without_moving_existing_sessions(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let project = dir.path().join("project");
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(&project).unwrap();
        let path = dir.path().join("sessions.json");
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) =
            cx.add_window_view(|_, cx| AgentWorkspace::with_path(path.clone(), cx));
        let home = home.canonicalize().unwrap();
        host.update_in(window, |host, window, cx| {
            host.choose_project(project.clone(), cx);
            let old = host.history.active;
            host.history
                .active_mut()
                .push(Role::User, "existing".into());
            host.set_agentspace(home.clone(), cx);
            assert_eq!(host.history.agentspace.as_ref(), Some(&home));
            assert_eq!(
                host.history
                    .sessions
                    .iter()
                    .find(|s| s.id == old)
                    .unwrap()
                    .project
                    .as_ref(),
                Some(&project)
            );
            host.choose_project(project.clone(), cx);
            host.new_session(window, cx);
            assert_eq!(host.history.active().project.as_ref(), Some(&home));
            host.history.save(&path).unwrap();
            assert_eq!(
                History::load(&path).unwrap().agentspace.as_ref(),
                Some(&home)
            );
            host.remove_project(home.clone(), cx);
            assert!(host.history.agentspace.is_none());
            assert!(home.join(agentspace::CONFIG_FILE).exists());
        });
    }

    #[gpui::test]
    fn project_more_menu_removes_persistently_and_reopening_restores_history(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("keep.txt"), "keep").unwrap();
        let path = dir.path().join("sessions.json");
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) =
            cx.add_window_view(|_, cx| AgentWorkspace::with_path(path.clone(), cx));
        window.simulate_resize(size(px(1200.), px(900.)));
        let old_session = host.update(window, |host, cx| {
            host.history.projects.push(project.clone());
            host.history.active_mut().project = Some(project.clone());
            host.history
                .active_mut()
                .push(Role::User, "保留这条历史".into());
            cx.notify();
            host.history.active
        });
        let group = SharedString::from(format!("project:{}", project.display()));
        let list = host.read_with(window, |host, _| host.group_list.clone());
        list.update(window, |_, cx| {
            cx.emit(GroupListEvent::HeaderMenuAction {
                group: group.clone(),
                index: 0,
            })
        });
        host.read_with(window, |host, _| {
            assert_eq!(host.project_grouping.editing.as_ref(), Some(&project))
        });
        list.update(window, |_, cx| {
            cx.emit(GroupListEvent::HeaderMenuAction {
                group: group.clone(),
                index: 1,
            })
        });
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
        list.update(window, |_, cx| {
            cx.emit(GroupListEvent::HeaderMenuAction { group, index: 1 })
        });
        let confirm = window.debug_bounds("confirm-remove-project").unwrap();
        window.simulate_click(confirm.center(), Default::default());
        host.update(window, |host, _| {
            assert!(!host.history.projects.contains(&project));
            assert_ne!(host.history.active, old_session);
            assert!(
                host.history
                    .sessions
                    .iter()
                    .any(|session| session.id == old_session
                        && session.messages[0].text == "保留这条历史")
            );
            assert!(session_groups(&host.history, "", None).is_empty());
            host.history.save(&path).unwrap();
            let restored = History::load(&path).unwrap();
            assert!(!restored.projects.contains(&project));
            assert!(session_groups(&restored, "", None).is_empty());
        });
        assert_eq!(
            std::fs::read_to_string(project.join("keep.txt")).unwrap(),
            "keep"
        );
        host.update(window, |host, cx| {
            host.choose_project(project.clone(), cx);
            assert!(!host.history.removed_projects.contains(&project));
            assert_eq!(session_groups(&host.history, "", None).len(), 1);
            assert!(
                host.history
                    .sessions
                    .iter()
                    .any(|session| session.id == old_session)
            );
        });
    }

    #[test]
    fn worktree_groups_keep_repository_and_session_identity_across_repositories() {
        let mut history = History::default();
        let idl = PathBuf::from("/worktree/idl/feat/chat-governance-openapi");
        let kim = PathBuf::from("/worktree/kim/feat/chat-governance-openapi");
        for (path, name) in [(&idl, "idl"), (&kim, "kim")] {
            history.new_session_for(Some(path.clone()));
            history.project_metadata.insert(
                path.clone(),
                project_grouping::Metadata {
                    repository_name: name.into(),
                    branch: "feat/chat-governance-openapi".into(),
                    linked_worktree: true,
                },
            );
        }
        let active = history.active;
        let groups = session_groups(&history, "", Some(active));
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].header.label.as_ref(),
            "feat/chat-governance-openapi"
        );
        assert_eq!(groups[0].children.len(), 2);
        assert_eq!(groups[0].children[0].header.label.as_ref(), "idl");
        assert_eq!(groups[0].children[1].header.label.as_ref(), "kim");
        assert_ne!(
            groups[0].children[0].header.id,
            groups[0].children[1].header.id
        );
        assert_eq!(
            groups[0].children[1].items[0].id.as_ref(),
            active.to_string()
        );
        assert_eq!(groups[0].children[1].items[0].icon, Some(Icon::Spinner));
        assert_eq!(
            session_groups(&history, "idl", None)[0].children[0]
                .items
                .len(),
            1
        );
        assert_eq!(
            session_groups(&history, "chat-governance", None)[0]
                .children
                .iter()
                .map(|group| group.items.len())
                .sum::<usize>(),
            2
        );
        history
            .project_group_overrides
            .insert(idl.clone(), "custom".into());
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
