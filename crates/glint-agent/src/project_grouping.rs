use super::*;
use gpui_glint::components::{ButtonSize, Dialog, InputField};

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Metadata {
    pub repository_name: String,
    pub branch: String,
    pub linked_worktree: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(root: &std::path::Path, args: &[&str]) {
        let output = crate::child_process::command("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn arbitrary_worktree_directory_uses_main_repository_name() {
        let directory = tempfile::tempdir().unwrap();
        let main = directory.path().join("idl");
        let linked = directory.path().join("arbitrary-directory");
        std::fs::create_dir(&main).unwrap();
        git(&main, &["init", "-b", "main"]);
        git(
            &main,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "initial",
            ],
        );
        git(
            &main,
            &[
                "worktree",
                "add",
                "-b",
                "feat/chat-governance-openapi",
                linked.to_str().unwrap(),
            ],
        );
        let meta = metadata(&git_project::repository(&linked).await.unwrap().unwrap());
        assert_eq!(meta.repository_name, "idl");
        assert_eq!(meta.branch, "feat/chat-governance-openapi");
        assert!(meta.linked_worktree);
        assert!(!metadata(&git_project::repository(&main).await.unwrap().unwrap()).linked_worktree);
    }

    #[test]
    fn project_group_metadata_and_overrides_survive_reload_and_legacy_history() {
        let mut history = History::default();
        let path = PathBuf::from("/worktree/idl/feature");
        history.projects.push(path.clone());
        history.project_metadata.insert(
            path.clone(),
            Metadata {
                repository_name: "idl".into(),
                branch: "feat/chat-governance-openapi".into(),
                linked_worktree: true,
            },
        );
        history
            .project_group_overrides
            .insert(path.clone(), "chat-governance-openapi".into());
        let directory = tempfile::tempdir().unwrap();
        let anchor = directory.path().join("sessions.json");
        history.save(&anchor).unwrap();
        let restored = History::load(&anchor).unwrap();
        assert_eq!(
            group_name(&restored, &path),
            Some("chat-governance-openapi")
        );
        assert_eq!(repository_label(&restored, &path), "idl");
        let mut legacy = serde_json::to_value(history).unwrap();
        legacy.as_object_mut().unwrap().remove("project_metadata");
        legacy
            .as_object_mut()
            .unwrap()
            .remove("project_group_overrides");
        let legacy: History = serde_json::from_value(legacy).unwrap();
        assert!(legacy.project_metadata.is_empty());
        assert!(legacy.project_group_overrides.is_empty());
    }

    #[gpui::test]
    fn group_editor_can_assign_clear_and_restore_automatic_group(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (workspace, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(directory.path().join("sessions.json"), cx)
        });
        workspace.update(window, |this, cx| {
            let path = PathBuf::from("/worktree/kim/feature");
            this.history.projects.push(path.clone());
            this.history.project_metadata.insert(
                path.clone(),
                Metadata {
                    repository_name: "kim".into(),
                    branch: "feat/chat-governance-openapi".into(),
                    linked_worktree: true,
                },
            );
            this.edit_project_group(path.clone(), cx);
            assert_eq!(
                this.project_grouping.name.read(cx).value(),
                "feat/chat-governance-openapi"
            );
            this.set_project_group(Some(" chat-governance-openapi ".into()), cx);
            assert_eq!(
                group_name(&this.history, &path),
                Some("chat-governance-openapi")
            );
            this.edit_project_group(path.clone(), cx);
            this.set_project_group(Some(String::new()), cx);
            assert_eq!(group_name(&this.history, &path), None);
            this.edit_project_group(path.clone(), cx);
            this.set_project_group(None, cx);
            assert_eq!(
                group_name(&this.history, &path),
                Some("feat/chat-governance-openapi")
            );
        });
    }
}

pub fn metadata(repo: &git_project::Repository) -> Metadata {
    let main = repo
        .worktrees
        .first()
        .map(|tree| tree.path.as_path())
        .unwrap_or(&repo.worktree);
    let normalize = |path: &std::path::Path| {
        let value = path.to_string_lossy().replace('\\', "/");
        let value = value.strip_prefix("//?/").unwrap_or(&value).to_owned();
        if cfg!(windows) {
            value.to_lowercase()
        } else {
            value
        }
    };
    Metadata {
        repository_name: ssh_project::label(main),
        branch: repo
            .current
            .strip_prefix("refs/heads/")
            .unwrap_or(&repo.current)
            .to_owned(),
        linked_worktree: normalize(main) != normalize(&repo.worktree),
    }
}

pub fn group_name<'a>(history: &'a History, path: &std::path::Path) -> Option<&'a str> {
    if let Some(name) = history.project_group_overrides.get(path) {
        return (!name.is_empty()).then_some(name.as_str());
    }
    history
        .project_metadata
        .get(path)
        .filter(|meta| {
            meta.linked_worktree
                && !meta.branch.is_empty()
                && meta.branch != "HEAD"
                && !meta.branch.starts_with("分离 HEAD")
        })
        .map(|meta| meta.branch.as_str())
}

pub fn repository_label(history: &History, path: &std::path::Path) -> String {
    history
        .project_metadata
        .get(path)
        .map(|meta| meta.repository_name.clone())
        .unwrap_or_else(|| ssh_project::label(path))
}

pub fn project_label(history: &History, path: &std::path::Path) -> String {
    let label = repository_label(history, path);
    match history.project_metadata.get(path) {
        Some(meta) if meta.linked_worktree => format!("{label} · {}", meta.branch),
        _ => label,
    }
}

pub struct State {
    pub editing: Option<PathBuf>,
    name: Entity<TextEntry>,
    _observer: Subscription,
}

impl State {
    pub fn new(cx: &mut Context<AgentWorkspace>) -> Self {
        let name = cx.new(|cx| TextEntry::new(cx, "例如 chat-governance-openapi", ""));
        let observer = cx.observe(&name, |_, _, cx| cx.notify());
        Self {
            editing: None,
            name,
            _observer: observer,
        }
    }
}

impl AgentWorkspace {
    /// Git I/O runs outside rendering; failed/offline probes retain saved labels.
    pub(crate) fn refresh_project_metadata(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        let Ok(job) = provider::Job::start(async move {
            use futures_util::StreamExt;
            let mut probes = futures_util::stream::iter(paths.into_iter().map(|path| async move {
                let repo = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    git_project::repository(&path),
                )
                .await
                .ok()?
                .ok()??;
                Some((path, metadata(&repo)))
            }))
            .buffer_unordered(4);
            let mut result = Vec::new();
            while let Some(probe) = probes.next().await {
                if let Some(entry) = probe {
                    result.push(entry);
                }
            }
            Ok(result)
        }) else {
            return;
        };
        cx.spawn(async move |weak, cx| {
            // Keep external Tokio wakeups off GPUI's deterministic UI scheduler.
            while !job.is_finished() {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
            }
            if let Ok(entries) = job.result().await {
                let _ = weak.update(cx, |this, cx| {
                    let mut changed = false;
                    for (path, metadata) in entries {
                        if this.history.projects.contains(&path)
                            && this.history.project_metadata.get(&path) != Some(&metadata)
                        {
                            this.history.project_metadata.insert(path, metadata);
                            changed = true;
                        }
                    }
                    if changed {
                        this.save(cx);
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    pub(crate) fn edit_project_group(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let name = group_name(&self.history, &path).unwrap_or("").to_owned();
        self.project_grouping.name.update(cx, |field, cx| {
            field.set_theme(self.theme, cx);
            field.set_value(name, cx);
        });
        self.project_grouping.editing = Some(path);
        self.project_picker_open = false;
        cx.notify();
    }

    fn set_project_group(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        if let Some(path) = self.project_grouping.editing.take() {
            match name {
                Some(name) => {
                    self.history
                        .project_group_overrides
                        .insert(path, name.trim().to_owned());
                }
                None => {
                    self.history.project_group_overrides.remove(&path);
                }
            }
            self.save(cx);
            cx.notify();
        }
    }

    pub(crate) fn project_group_dialog(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(path) = &self.project_grouping.editing else {
            return div().into_any_element();
        };
        let mut names: Vec<_> = self
            .history
            .projects
            .iter()
            .filter_map(|path| group_name(&self.history, path).map(str::to_owned))
            .collect();
        names.sort();
        names.dedup();
        let mut choices = div().flex().flex_wrap().gap_2();
        for name in names {
            choices = choices.child(
                Button::new(name.clone())
                    .kind(ButtonKind::Gray)
                    .size(ButtonSize::Small)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.project_grouping
                            .name
                            .update(cx, |field, cx| field.set_value(name.clone(), cx));
                    })),
            );
        }
        let body =
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(div().text_size(px(18.)).child("项目分组"))
                .child(project_label(&self.history, path))
                .child(
                    InputField::new("组名")
                        .label("组名（留空不分组）")
                        .control(self.project_grouping.name.clone())
                        .theme(self.theme)
                        .render(),
                )
                .child(choices)
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            Button::new("跟随 worktree 分支")
                                .kind(ButtonKind::Gray)
                                .theme(self.theme)
                                .render()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.set_project_group(None, cx)),
                                ),
                        )
                        .child(
                            Button::new("取消")
                                .kind(ButtonKind::Gray)
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.project_grouping.editing = None;
                                    cx.notify();
                                })),
                        )
                        .child(Button::new("保存").theme(self.theme).render().on_click(
                            cx.listener(|this, _, _, cx| {
                                let name = this.project_grouping.name.read(cx).value().to_string();
                                this.set_project_group(Some(name), cx);
                            }),
                        )),
                );
        let weak = cx.entity().downgrade();
        Dialog::new("project-group-dialog", body)
            .width(520.)
            .theme(self.theme)
            .focus_handles(vec![self.project_grouping.name.focus_handle(cx)])
            .on_close(move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.project_grouping.editing = None;
                    cx.notify();
                });
            })
            .render()
            .into_any_element()
    }
}
