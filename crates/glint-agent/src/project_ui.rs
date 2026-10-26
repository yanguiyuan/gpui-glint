use super::*;
use gpui_glint::components::{GroupListGroup, GroupListHeader, GroupListItem};

impl AgentWorkspace {
    pub fn choose_project(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.capture_draft(cx);
        if !self.history.projects.contains(&path) {
            self.history.projects.push(path.clone());
        }
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
        let running = self
            .pending
            .as_ref()
            .or(self.preparing.as_ref())
            .is_some_and(|(id, _)| *id == active);
        if self.history.active().messages.is_empty() && !running {
            self.history.active_mut().project = Some(path);
        } else if self.history.active().project.as_ref() != Some(&path) {
            self.history.new_session_for(Some(path));
        }
        self.restore_draft(cx);
        images::release_previews(self.image_views.borrow_mut().clear(), cx);
        self.project_picker_open = false;
        self.show_settings = false;
        self.show_calendar = false;
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
            .map(|path| {
                path.file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
                    .into_owned()
            })
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
            let label = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned();
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
                    cx.notify();
                });
            })
    }

    pub fn pick_context(&mut self, directories: bool, cx: &mut Context<Self>) {
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
                        let root = this.history.active().project.as_deref();
                        let labels: Vec<_> = paths
                            .iter()
                            .map(|path| {
                                let path = root
                                    .and_then(|root| path.strip_prefix(root).ok())
                                    .unwrap_or(path);
                                if path.as_os_str().is_empty() {
                                    ".".to_string()
                                } else {
                                    path.display().to_string()
                                }
                            })
                            .collect();
                        this.composer.update(cx, |composer, cx| {
                            for label in labels {
                                composer.add_mention(label, this.theme.palette().accent, cx);
                            }
                        });
                        this.history.active_mut().draft =
                            this.composer.read(cx).value(cx).to_string();
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
    running: Option<Uuid>,
) -> Vec<GroupListGroup> {
    let query = query.trim().to_lowercase();
    let mut sessions: Vec<_> = history
        .sessions
        .iter()
        .filter(|session| {
            query.is_empty()
                || session.title.to_lowercase().contains(&query)
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
                GroupListHeader::new(
                    id,
                    path.file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                )
                .icon(Icon::Folder)
                .actions([Icon::Plus]),
            ));
        }
    }
    for session in sessions {
        let (id, label, icon) = if history.group_by_project {
            match &session.project {
                Some(path) => (
                    format!("project:{}", path.display()),
                    path.file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                    Icon::Folder,
                ),
                None => continue,
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
        if running == Some(session.id) {
            item = item.icon(Icon::Spinner);
        }
        groups[index].items.push(item);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(groups.iter().all(|group| group.header.actions.len() == 1));
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
