//! Panel-local repository choices; the AgentSpace conversation keeps its home project.
use super::*;
use gpui_glint::components::{Select, SelectEvent, SelectOption};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Panel {
    Diff,
    Preview,
    Terminal,
}

#[derive(Default)]
pub(super) struct State {
    home: Option<PathBuf>,
    options: Vec<(String, PathBuf)>,
    choices: HashMap<(PathBuf, Panel), PathBuf>,
    selects: HashMap<Panel, Entity<Select>>,
    task: Option<Task<()>>,
    checked: Option<Instant>,
    error: Option<String>,
    filtered: RefCell<Option<(PathBuf, Rc<Vec<edits::Change>>, Rc<Vec<edits::Change>>)>>,
}

impl AgentWorkspace {
    pub(super) fn panel_repository_root(&self, panel: Panel) -> Option<PathBuf> {
        if self.page == crate::Page::Projects && panel == Panel::Terminal {
            return self
                .browser_repositories
                .selected_root()
                .or_else(|| self.browser_project.clone());
        }
        let home = self.history.active().project.as_ref()?;
        let state = &self.panel_repositories;
        if state.home.as_ref() == Some(home) {
            if let Some(path) = state.choices.get(&(home.clone(), panel)) {
                if state.options.iter().any(|(_, root)| root == path) {
                    return Some(path.clone());
                }
            }
        }
        Some(home.clone())
    }

    pub(super) fn diff_repository_root(&self) -> Option<PathBuf> {
        self.panel_repository_root(Panel::Diff)
    }

    pub(super) fn filter_repository_edits(
        &self,
        files: Rc<Vec<edits::Change>>,
    ) -> Rc<Vec<edits::Change>> {
        if self.panel_repositories.home != self.history.active().project
            || self.panel_repositories.options.len() < 2
        {
            return files;
        }
        let Some(root) = self.diff_repository_root() else {
            return files;
        };
        let mut cache = self.panel_repositories.filtered.borrow_mut();
        if let Some((old_root, old, filtered)) = cache.as_ref() {
            if old_root == &root && Rc::ptr_eq(old, &files) {
                return filtered.clone();
            }
        }
        let filtered: Rc<Vec<edits::Change>> = Rc::new(
            files
                .iter()
                .filter(|file| {
                    self.panel_repositories
                        .options
                        .iter()
                        .filter(|(_, candidate)| file.path.starts_with(candidate))
                        .max_by_key(|(_, candidate)| candidate.components().count())
                        .is_some_and(|(_, owner)| owner == &root)
                })
                .cloned()
                .collect(),
        );
        *cache = Some((root, files, filtered.clone()));
        filtered
    }

    pub(super) fn repository_select(
        &self,
        panel: Panel,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Select>> {
        if self.page == crate::Page::Projects {
            return None;
        }
        if self.panel_repositories.home != self.history.active().project
            || self.panel_repositories.options.len() < 2
        {
            return None;
        }
        let select = self.panel_repositories.selects.get(&panel)?.clone();
        if select.read(cx).theme_preset() != self.theme {
            select.update(cx, |select, cx| select.set_theme(self.theme, cx));
        }
        Some(select)
    }

    pub(super) fn adopt_panel_repository(
        &mut self,
        panel: Panel,
        path: &str,
        cx: &mut Context<Self>,
    ) -> Option<PathBuf> {
        if self.panel_repositories.home != self.history.active().project {
            return None;
        }
        let home = self.history.active().project.clone()?;
        let root = self
            .panel_repositories
            .options
            .iter()
            .filter(|(_, root)| std::path::Path::new(path).starts_with(root))
            .max_by_key(|(_, root)| root.components().count())
            .map(|(_, root)| root.clone())?;
        self.panel_repositories
            .choices
            .insert((home, panel), root.clone());
        if let Some(select) = self.panel_repositories.selects.get(&panel) {
            let value: gpui::SharedString = root.to_string_lossy().into_owned().into();
            if select.read(cx).value() != Some(&value) {
                select.update(cx, |select, cx| select.set_selected(Some(value), cx));
            }
        }
        Some(root)
    }

    pub(super) fn sync_panel_repositories(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == crate::Page::Projects {
            return;
        }
        let home = self.history.active().project.clone();
        let changed = self.panel_repositories.home != home;
        if changed {
            self.panel_repositories.task = None;
            self.panel_repositories.home = home.clone();
            self.panel_repositories.options.clear();
            self.panel_repositories.selects.clear();
            self.panel_repositories.checked = None;
        }
        if self.panel_repositories.task.is_some()
            || self
                .panel_repositories
                .checked
                .is_some_and(|at| at.elapsed() < Duration::from_secs(5))
        {
            return;
        }
        for panel in [Panel::Diff, Panel::Preview, Panel::Terminal] {
            if !self.panel_repositories.selects.contains_key(&panel) {
                let select = cx.new(|cx| {
                    Select::new([], cx)
                        .searchable(true)
                        .width(180.)
                        .theme(self.theme)
                });
                cx.subscribe_in(
                    &select,
                    window,
                    move |this, _, event: &SelectEvent, window, cx| {
                        let SelectEvent::Changed(value) = event;
                        if let Some(index) = this
                            .panel_repositories
                            .options
                            .iter()
                            .position(|(_, root)| root.to_string_lossy().as_ref() == value.as_ref())
                        {
                            this.select_panel_repository(panel, index, window, cx);
                        }
                    },
                )
                .detach();
                self.panel_repositories.selects.insert(panel, select);
            }
        }
        self.panel_repositories.checked = Some(Instant::now());
        let Some(home) = home else {
            return;
        };
        let work_home = home.clone();
        let work = cx.background_executor().spawn(async move {
            let mut options = vec![(
                format!("{} · 主目录", ssh_project::label(&work_home)),
                work_home.clone(),
            )];
            if let Some(space) = agentspace::Space::load(&work_home)? {
                options.extend(
                    space
                        .repositories()
                        .map(|(name, path)| (name.to_owned(), path.to_owned())),
                );
            }
            Ok::<_, String>(options)
        });
        self.panel_repositories.task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if this.history.active().project.as_ref() != Some(&home) {
                    return;
                }
                if let Some(task) = this.panel_repositories.task.take() {
                    task.detach();
                }
                let options = match result {
                    Ok(options) => options,
                    Err(error) => {
                        if this.panel_repositories.error.as_ref() != Some(&error) {
                            this.git_ui.toasts.update(cx, |toasts, cx| {
                                toasts.push(
                                    gpui_glint::components::Toast::new("关联仓库读取失败")
                                        .description(error.clone())
                                        .kind(gpui_glint::components::ToastKind::Error),
                                    cx,
                                )
                            });
                        }
                        this.panel_repositories.error = Some(error);
                        return;
                    }
                };
                this.panel_repositories.error = None;
                if this.panel_repositories.options == options {
                    return;
                }
                let old_diff = this.diff_repository_root();
                let old_preview = this.panel_repository_root(Panel::Preview);
                let old_terminal = this.panel_repository_root(Panel::Terminal);
                this.panel_repositories.options = options.clone();
                for panel in [Panel::Diff, Panel::Preview, Panel::Terminal] {
                    let selected = this
                        .panel_repository_root(panel)
                        .unwrap_or_else(|| home.clone());
                    let index = options
                        .iter()
                        .position(|(_, root)| root == &selected)
                        .unwrap_or(0);
                    let select_options: Vec<_> = options
                        .iter()
                        .map(|(label, root)| {
                            SelectOption::new(root.to_string_lossy().into_owned(), label.clone())
                        })
                        .collect();
                    if let Some(select) = this.panel_repositories.selects.get(&panel) {
                        select.update(cx, |select, cx| {
                            select.set_options(select_options, cx);
                            select.set_selected(
                                Some(options[index].1.to_string_lossy().into_owned().into()),
                                cx,
                            );
                        });
                    }
                }
                if old_diff != this.diff_repository_root()
                    && this.edits_ui.open
                    && this.edits_ui.workspace_mode
                {
                    this.refresh_workspace_edits(cx);
                }
                if old_preview != this.panel_repository_root(Panel::Preview)
                    && this.file_preview.open
                {
                    this.switch_preview_repository(cx);
                }
                if old_terminal != this.panel_repository_root(Panel::Terminal)
                    && this.terminal_ui.open
                {
                    if let Some(select) = this.panel_repositories.selects.get(&Panel::Terminal) {
                        if let Some(root) = this.panel_repository_root(Panel::Terminal) {
                            select.update(cx, |_, cx| {
                                cx.emit(SelectEvent::Changed(
                                    root.to_string_lossy().into_owned().into(),
                                ))
                            });
                        }
                    }
                }
                cx.notify();
            });
        }));
    }

    pub(super) fn select_panel_repository(
        &mut self,
        panel: Panel,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.panel_repositories.home != self.history.active().project {
            return;
        }
        if panel == Panel::Diff && self.repository_change_busy(cx) {
            self.git_ui.toasts.update(cx, |toasts, cx| {
                toasts.push(
                    gpui_glint::components::Toast::new("文件操作进行中，请完成后再切换仓库"),
                    cx,
                )
            });
            return;
        }
        let Some((_, root)) = self.panel_repositories.options.get(index).cloned() else {
            return;
        };
        let Some(home) = self.history.active().project.clone() else {
            return;
        };
        self.panel_repositories
            .choices
            .insert((home, panel), root.clone());
        if let Some(select) = self.panel_repositories.selects.get(&panel) {
            select.update(cx, |select, cx| {
                select.set_selected(Some(root.to_string_lossy().into_owned().into()), cx)
            });
        }
        match panel {
            Panel::Diff => {
                self.edits_ui.selected = None;
                if self.edits_ui.workspace_mode {
                    self.refresh_workspace_edits(cx);
                }
            }
            Panel::Preview => self.switch_preview_repository(cx),
            Panel::Terminal => self.open_project_terminal(window, cx),
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn agentspace_panel_repositories_switch_and_stage_without_changing_the_session(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        let home = crate::workspace_diff::tests::many_files(1);
        let linked = crate::workspace_diff::tests::many_files(1);
        let home_root = home.path().canonicalize().unwrap();
        let linked_root = linked.path().canonicalize().unwrap();
        std::fs::write(
            home.path().join(agentspace::CONFIG_FILE),
            format!(
                "[[repositories]]\nname = 'backend'\npath = '{}'\n",
                linked.path().to_string_lossy().replace('\\', "/")
            ),
        )
        .unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::write(
            data.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            let mut host = AgentWorkspace::with_path(data.path().join("sessions.json"), cx);
            host.history.active_mut().project = Some(home_root.clone());
            host.open_workspace_diff(cx);
            host
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.run_until_parked();
        let recorded = Rc::new(vec![
            edits::Change::preview(
                home_root.join("home.rs"),
                "home.rs".into(),
                None,
                "home".into(),
                None,
            ),
            edits::Change::preview(
                linked_root.join("linked.rs"),
                "linked.rs".into(),
                None,
                "linked".into(),
                None,
            ),
        ]);
        host.read_with(window, |host, _| {
            let visible = host.filter_repository_edits(recorded.clone());
            assert_eq!(visible.len(), 1);
            assert_eq!(visible[0].label, "home.rs");
        });
        let diff = host.update(window, |host, cx| {
            host.repository_select(Panel::Diff, cx).unwrap()
        });
        diff.update(window, |_, cx| {
            cx.emit(SelectEvent::Changed(
                linked_root.to_string_lossy().into_owned().into(),
            ))
        });
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert_eq!(host.history.active().project.as_ref(), Some(&home_root));
            assert_eq!(host.diff_repository_root().as_ref(), Some(&linked_root));
            let visible = host.filter_repository_edits(recorded.clone());
            assert_eq!(visible.len(), 1);
            assert_eq!(visible[0].label, "linked.rs");
            assert_eq!(host.edits_ui.workspace_root.as_ref(), Some(&linked_root));
            assert_eq!(
                host.panel_repository_root(Panel::Preview).as_ref(),
                Some(&home_root)
            );
            assert_eq!(
                host.panel_repository_root(Panel::Terminal).as_ref(),
                Some(&home_root)
            );
        });
        let path = linked_root.join("vendor/0000.rs");
        let selector: &'static str =
            Box::leak(format!("stage-file:{}:false", path.display()).into_boxed_str());
        let checkbox = window.debug_bounds(selector).unwrap();
        window.simulate_click(checkbox.center(), Default::default());
        window.run_until_parked();
        assert!(
            crate::workspace_diff::inventory(&linked_root)
                .unwrap()
                .sources[&path]
                .staged
        );
        assert!(
            !crate::workspace_diff::inventory(&home_root)
                .unwrap()
                .sources[&home_root.join("vendor/0000.rs")]
                .staged
        );
        let preview = host.update(window, |host, cx| {
            host.repository_select(Panel::Preview, cx).unwrap()
        });
        preview.update(window, |_, cx| {
            cx.emit(SelectEvent::Changed(
                linked_root.to_string_lossy().into_owned().into(),
            ))
        });
        window.run_until_parked();
        let directory_entry: &'static str = Box::leak(
            format!("preview-entry:{}", linked_root.join("vendor").display()).into_boxed_str(),
        );
        assert!(window.debug_bounds(directory_entry).is_some());
        host.read_with(window, |host, _| {
            assert_eq!(
                host.panel_repository_root(Panel::Preview).as_ref(),
                Some(&linked_root)
            );
            assert_eq!(host.history.active().project.as_ref(), Some(&home_root));
            assert_eq!(host.diff_repository_root().as_ref(), Some(&linked_root));
        });
    }
}
