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

    #[gpui::test]
    fn project_group_editor_modes_only_persist_on_save(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("providers.json"), r#"{"providers":[],"active":null,"active_model":""}"#).unwrap();
        let project = PathBuf::from("/repo/feature");
        let (host, window) = cx.add_window_view(|_, cx| AgentWorkspace::with_path(dir.path().join("sessions.json"), cx));
        window.simulate_resize(size(px(1200.), px(900.)));
        host.update(window, |host, cx| {
            host.history.project_group_overrides.insert(project.clone(), "custom".into());
            host.edit_project_group(project.clone(), cx);
        });
        let auto = window.debug_bounds("project-group-auto").unwrap();
        window.simulate_click(auto.center(), Default::default());
        host.read_with(window, |host, _| {
            assert!(host.project_grouping.automatic);
            assert_eq!(host.history.project_group_overrides[&project], "custom");
        });
        let save = window.debug_bounds("save-project-group").unwrap();
        window.simulate_click(save.center(), Default::default());
        host.read_with(window, |host, _| assert!(!host.history.project_group_overrides.contains_key(&project)));
        host.update(window, |host, cx| host.edit_project_group(project.clone(), cx));
        let manual = window.debug_bounds("project-group-manual").unwrap();
        window.simulate_click(manual.center(), Default::default());
        host.update(window, |host, cx| { host.project_grouping.name.update(cx, |input, cx| input.set_value("", cx)); });
        let save = window.debug_bounds("save-project-group").unwrap();
        window.simulate_click(save.center(), Default::default());
        host.read_with(window, |host, _| assert_eq!(host.history.project_group_overrides[&project], ""));
    }

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
    pub removing: Option<PathBuf>,
    automatic: bool,
    name: Entity<TextEntry>,
    _observer: Subscription,
}

impl State {
    pub fn new(cx: &mut Context<AgentWorkspace>) -> Self {
        let name = cx.new(|cx| TextEntry::new(cx, "例如 chat-governance-openapi", ""));
        let observer = cx.observe(&name, |_, _, cx| cx.notify());
        Self {
            editing: None,
            removing: None,
            automatic: true,
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
        self.project_grouping.automatic = !self.history.project_group_overrides.contains_key(&path);
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
        let has_names = !names.is_empty();
        let mut choices = div().flex().flex_wrap().gap_2();
        for name in names {
            choices = choices.child(
                Button::new(name.clone())
                    .kind(if self.project_grouping.name.read(cx).value() == name { ButtonKind::Tinted } else { ButtonKind::Plain })
                    .size(ButtonSize::Small)
                    .theme(self.theme)
                    .render()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.project_grouping.automatic = false;
                        this.project_grouping
                            .name
                            .update(cx, |field, cx| field.set_value(name.clone(), cx));
                    })),
            );
        }
        let automatic = self.project_grouping.automatic;
        let body =
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child("设置项目分组"))
                .child(div().p_3().rounded(px(8.)).bg(rgb(self.theme.palette().surface_subtle)).flex().flex_col().gap_1()
                    .child(div().flex().items_center().gap_2().child(Icon::Folder.render(16.)).child(project_label(&self.history, path)))
                    .child(div().text_size(px(11.)).text_color(rgb(self.theme.palette().text_muted)).truncate()
                        .child(gpui_glint::paths::display_path(path))))
                .child(div().flex().gap_2().children([
                    ("自动跟随分支", true), ("手动设置", false),
                ].into_iter().map(|(label, mode)| Button::new(label)
                    .kind(if automatic == mode { ButtonKind::Tinted } else { ButtonKind::Plain }).theme(self.theme).render()
                    .debug_selector(move || if mode { "project-group-auto".into() } else { "project-group-manual".into() })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.project_grouping.automatic = mode;
                        window.focus(&if mode { this.focus.clone() } else { this.project_grouping.name.focus_handle(cx) }, cx);
                        cx.notify();
                    })))))
                .when(automatic, |body| body.child(div().text_size(px(12.)).text_color(rgb(self.theme.palette().text_muted))
                    .child("根据 worktree 分支自动分组；普通项目保持不分组。分支变化后会自动更新。")))
                .when(!automatic, |body| body.child(
                    InputField::new("组名")
                        .label("组名（留空不分组）")
                        .control(self.project_grouping.name.clone())
                        .theme(self.theme)
                        .render().w_full(),
                ).when(has_names, |body| body.child(div().text_size(px(11.)).text_color(rgb(self.theme.palette().text_muted)).child("已有分组 · 点击选择")).child(choices)))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .pt_3()
                        .border_t_1()
                        .border_color(rgb(self.theme.palette().border))
                        .gap_2()
                        .child(
                            Button::new("取消")
                                .kind(ButtonKind::Plain)
                                .theme(self.theme)
                                .render()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.project_grouping.editing = None;
                                    cx.notify();
                                })),
                        )
                        .child(Button::new("保存").theme(self.theme).render().debug_selector(|| "save-project-group".into()).on_click(
                            cx.listener(|this, _, _, cx| {
                                let name = this.project_grouping.name.read(cx).value().to_string();
                                this.set_project_group(if this.project_grouping.automatic { None } else { Some(name) }, cx);
                            }),
                        )),
                );
        let weak = cx.entity().downgrade();
        Dialog::new("project-group-dialog", body)
            .width(520.)
            .theme(self.theme)
            .focus_handles(if automatic { vec![] } else { vec![self.project_grouping.name.focus_handle(cx)] })
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
