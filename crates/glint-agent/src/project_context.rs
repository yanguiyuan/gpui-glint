//! Cached project filenames. Directory traversal never runs on typing/hover.
use super::*;
use std::path::{Component, Path};

const MAX_PATHS: usize = 50_000;
const MAX_RESULTS: usize = 60;
#[derive(Clone)]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub directory: bool,
    key: String,
    name_key: String,
    depth: usize,
}
pub struct Index {
    root: PathBuf,
    pub entries: Vec<Entry>,
    pub truncated: bool,
    pub skipped: usize,
}
impl Index {
    pub fn scan(root: &Path) -> Result<Self, String> {
        Self::scan_with_limit(root, MAX_PATHS)
    }
    fn scan_with_limit(root: &Path, limit: usize) -> Result<Self, String> {
        let root = root.canonicalize().map_err(|_| "项目目录已不可用")?;
        if !root.is_dir() {
            return Err("当前项目不是目录".into());
        }
        let mut builder = ignore::WalkBuilder::new(&root);
        builder
            .hidden(false)
            .follow_links(false)
            .require_git(false)
            .filter_entry(|entry| {
                entry.depth() == 0
                    || !(entry.file_type().is_some_and(|kind| kind.is_dir())
                        && matches!(
                            entry.file_name().to_str(),
                            Some(".git" | "target" | "node_modules" | ".venv" | ".cache")
                        ))
            });
        let mut index = Self {
            root: root.clone(),
            entries: vec![],
            truncated: false,
            skipped: 0,
        };
        for (scanned, item) in builder.build().enumerate() {
            if scanned >= 100_000 || index.entries.len() >= limit {
                index.truncated = true;
                break;
            }
            let entry = match item {
                Ok(entry) => entry,
                Err(_) => {
                    index.skipped += 1;
                    continue;
                }
            };
            if entry.depth() == 0
                || !entry
                    .file_type()
                    .is_some_and(|kind| kind.is_file() || kind.is_dir())
            {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&root)
                .map_err(|_| "索引路径超出项目")?;
            let Some(path) = relative.to_str() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = if cfg!(windows) {
                path.replace('\\', "/")
            } else {
                path.to_owned()
            };
            index.entries.push(Entry {
                path: path.clone(),
                key: path.to_lowercase(),
                name_key: name.to_lowercase(),
                name,
                directory: entry.file_type().unwrap().is_dir(),
                depth: relative.components().count(),
            });
        }
        index.entries.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(index)
    }
    pub fn search(&self, query: &str) -> Vec<&Entry> {
        let query = query.to_lowercase().replace('\\', "/");
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| query.is_empty() || entry.key.contains(&query))
            .collect();
        let rank = |entry: &Entry| {
            if query.is_empty() || entry.name_key == query {
                0
            } else if entry.name_key.starts_with(&query) {
                1
            } else if entry.name_key.contains(&query) {
                2
            } else {
                3
            }
        };
        let mut compare = |a: &&Entry, b: &&Entry| {
            rank(a)
                .cmp(&rank(b))
                .then(a.depth.cmp(&b.depth))
                .then(a.key.cmp(&b.key))
                .then(a.path.cmp(&b.path))
        };
        // Partition first so broad queries do not sort tens of thousands of paths
        // on the UI thread. Only the visible prefix needs ordering.
        if entries.len() > MAX_RESULTS {
            entries.select_nth_unstable_by(MAX_RESULTS, &mut compare);
            entries.truncate(MAX_RESULTS);
        }
        entries.sort_by(compare);
        entries
    }
    fn contains(&self, path: &str) -> bool {
        if !self.entries.iter().any(|entry| entry.path == path) {
            return false;
        }
        let relative = Path::new(path);
        if relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            return false;
        }
        self.root
            .join(relative)
            .canonicalize()
            .is_ok_and(|path| path.starts_with(&self.root))
    }
}
#[derive(Default)]
pub struct State {
    project: Option<PathBuf>,
    index: Option<Index>,
    task: Option<Task<()>>,
    query: String,
    error: Option<String>,
}
impl State {
    pub fn section(&self) -> Option<ComposerContextSection> {
        let mut entries: Vec<_> = self
            .index
            .as_ref()
            .into_iter()
            .flat_map(|index| index.search(&self.query))
            .map(|entry| {
                ComposerContextEntry::new(
                    format!("project-path:{}", entry.path),
                    entry.name.clone(),
                )
                .icon(if entry.directory {
                    Icon::Folder
                } else {
                    Icon::Text
                })
                .description(entry.path.clone())
                .insert_mention(false)
            })
            .collect();
        if self.task.is_some() && self.index.is_none() {
            entries.push(
                ComposerContextEntry::new("project-index-status", "正在扫描项目文件…")
                    .description(format!("正在查找：{}", self.query))
                    .insert_mention(false),
            );
        }
        if let Some(error) = &self.error {
            entries.push(
                ComposerContextEntry::new("project-index-status", "项目文件索引不可用")
                    .description(format!("{} · {error}", self.query))
                    .insert_mention(false),
            );
        }
        let title = if self.index.as_ref().is_some_and(|index| index.truncated) {
            "项目文件 · 索引已达上限"
        } else if self.index.as_ref().is_some_and(|index| index.skipped > 0) {
            "项目文件 · 部分目录不可读"
        } else {
            "项目文件"
        };
        (!entries.is_empty()).then(|| ComposerContextSection::new(title, entries))
    }
}
impl AgentWorkspace {
    pub fn reset_project_context(&mut self, cx: &mut Context<Self>) {
        self.project_context = State {
            project: self.history.active().project.clone(),
            ..Default::default()
        };
        self.sync_skill_context(cx);
        self.refresh_project_context(cx);
    }
    pub fn refresh_project_context(&mut self, cx: &mut Context<Self>) {
        let project = self.history.active().project.clone();
        if self.project_context.project != project {
            self.project_context = State {
                project: project.clone(),
                ..Default::default()
            };
        }
        if self.project_context.task.is_some() {
            return;
        }
        let Some(root) = project.clone() else {
            return;
        };
        self.project_context.error = None;
        let work = cx
            .background_executor()
            .spawn(async move { Index::scan(&root) });
        self.project_context.task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if this.history.active().project != project {
                    return;
                }
                if let Some(task) = this.project_context.task.take() {
                    task.detach();
                }
                match result {
                    Ok(index) => {
                        this.project_context.index = Some(index);
                        this.project_context.error = None;
                    }
                    Err(error) => {
                        this.project_context.index = None;
                        this.project_context.error = Some(error);
                    }
                }
                this.sync_skill_context(cx);
            });
        }));
        self.sync_skill_context(cx);
    }
    pub fn filter_project_context(&mut self, query: &str, cx: &mut Context<Self>) {
        if self.project_context.query != query {
            self.project_context.query = query.to_owned();
            self.sync_skill_context(cx);
        }
    }
    pub fn select_project_context(&mut self, path: &str, cx: &mut Context<Self>) {
        if self.project_context.project != self.history.active().project
            || !self
                .project_context
                .index
                .as_ref()
                .is_some_and(|index| index.contains(path))
        {
            self.provider_error = Some("项目文件已不可用，请重新打开上下文面板刷新".into());
            cx.notify();
            return;
        }
        let color = self.theme.palette().accent;
        self.composer.update(cx, |composer, cx| {
            composer.add_mention(path.to_owned(), color, cx)
        });
        self.capture_draft(cx);
        self.save(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexes_only_project_paths_respects_ignore_rules_and_filters_names_and_paths() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        for folder in ["src", "target", "node_modules", ".git", "ignored"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        std::fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
        for file in [
            "README.md",
            "src/ReadmeGuide.md",
            "src/main.rs",
            "target/cache",
            "node_modules/pkg",
            ".git/config",
            "ignored/private",
        ] {
            std::fs::write(root.join(file), "contents are not needed").unwrap();
        }
        std::fs::write(directory.path().join("outside.txt"), "outside").unwrap();
        let index = Index::scan(&root).unwrap();
        let results = index.search("README");
        assert_eq!(results[0].path, "README.md");
        assert_eq!(results[1].path, "src/ReadmeGuide.md");
        assert_eq!(index.search("src/main")[0].path, "src/main.rs");
        for excluded in [
            "target/",
            "node_modules/",
            ".git/",
            "ignored/",
            "outside.txt",
        ] {
            assert!(
                !index
                    .entries
                    .iter()
                    .any(|entry| entry.path.contains(excluded))
            );
        }
        assert!(index.search("not-present").is_empty());
        assert!(index.contains("README.md"));
        assert!(!index.contains("../outside.txt"));
        std::fs::remove_file(root.join("README.md")).unwrap();
        assert!(!index.contains("README.md"));
        // Filtering uses the index even after the file changes; selecting checks existence.
        assert_eq!(index.search("README")[0].path, "README.md");
    }
    #[cfg(unix)]
    #[test]
    fn symlink_targets_are_never_indexed_and_replaced_paths_cannot_escape() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let outside = directory.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("private"), "secret").unwrap();
        symlink(&outside, root.join("linked")).unwrap();
        std::fs::write(root.join("safe.txt"), "safe").unwrap();
        let index = Index::scan(&root).unwrap();
        assert!(
            !index
                .entries
                .iter()
                .any(|entry| entry.path.starts_with("linked"))
        );
        std::fs::remove_file(root.join("safe.txt")).unwrap();
        symlink(outside.join("private"), root.join("safe.txt")).unwrap();
        assert!(!index.contains("safe.txt"));
    }
    #[test]
    fn index_and_visible_result_counts_are_bounded() {
        let directory = tempfile::tempdir().unwrap();
        for i in 0..100 {
            std::fs::write(directory.path().join(format!("file-{i:03}.txt")), "x").unwrap();
        }
        let bounded = Index::scan_with_limit(directory.path(), 10).unwrap();
        assert_eq!(bounded.entries.len(), 10);
        assert!(bounded.truncated);
        let index = Index::scan(directory.path()).unwrap();
        assert_eq!(index.search("file").len(), MAX_RESULTS);
    }
    #[gpui::test]
    fn project_filename_candidates_replace_at_query_with_relative_path(
        cx: &mut gpui::TestAppContext,
    ) {
        struct Preview {
            composer: Entity<Composer>,
            _subscription: Subscription,
        }
        impl Render for Preview {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().w(px(650.)).pt(px(280.)).child(self.composer.clone())
            }
        }
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("docs")).unwrap();
        std::fs::write(directory.path().join("docs/README.md"), "content").unwrap();
        let state = State {
            project: Some(directory.path().to_path_buf()),
            index: Some(Index::scan(directory.path()).unwrap()),
            query: "readme".into(),
            ..Default::default()
        };
        let (preview, window) = cx.add_window_view(|_, cx| {
            let composer = cx.new(|cx| {
                Composer::new("Message", cx)
                    .context_on_at(true)
                    .context_panel("attach", "Context", [state.section().unwrap()])
            });
            let subscription = cx.subscribe(
                &composer,
                |this: &mut Preview, _, event: &ComposerEvent, cx| {
                    if let ComposerEvent::ContextSelected { id, .. } = event
                        && let Some(path) = id.strip_prefix("project-path:")
                    {
                        this.composer.update(cx, |composer, cx| {
                            composer.add_mention(path.to_owned(), 0x007aff, cx)
                        });
                    }
                },
            );
            Preview {
                composer,
                _subscription: subscription,
            }
        });
        let composer = preview.read_with(window, |this, _| this.composer.clone());
        composer.update(window, |composer, cx| {
            composer.set_value("检查 @README", cx)
        });
        let row = window
            .debug_bounds("composer-context:project-path:docs/README.md")
            .unwrap();
        window.simulate_click(row.center(), gpui::Modifiers::default());
        composer.read_with(window, |composer, cx| {
            assert_eq!(composer.value(cx).as_ref(), "检查 @docs/README.md ");
            assert!(!composer.context_panel_open());
        });
    }
}
