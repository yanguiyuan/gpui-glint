use super::*;
use gpui_glint::components::{GroupListGroup, GroupListHeader, GroupListItem};

impl AgentWorkspace {
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
                                    this.history.active_mut().draft =
                                        this.composer.read(cx).value(cx).to_string();
                                    if !this.history.projects.contains(&path) {
                                        this.history.projects.push(path.clone());
                                    }
                                    this.history.new_session_for(Some(path));
                                    let draft = this.history.active().draft.clone();
                                    this.composer
                                        .update(cx, |composer, cx| composer.set_value(draft, cx));
                                    this.show_settings = false;
                                    this.show_calendar = false;
                                    this.save();
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

pub fn session_groups(history: &History, query: &str) -> Vec<GroupListGroup> {
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
                None => ("unassigned".into(), "独立会话".into(), Icon::Text),
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
        groups[index].items.push(
            GroupListItem::new(session.id.to_string(), session.title.clone())
                .icon(Icon::Text)
                .meta(session.updated.format("%H:%M").to_string()),
        );
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(session_groups(&history, "").len(), 3);
        history.group_by_project = false;
        assert_eq!(session_groups(&history, "").len(), 1);
        assert_eq!(session_groups(&history, "First")[0].items.len(), 1);
    }
}
