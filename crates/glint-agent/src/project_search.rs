//! Background project lookup. Filename snapshots are cached; text is searched from disk.
use super::*;
use gpui_glint::components::CommandItem;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

type Cache = Arc<Mutex<Option<(PathBuf, Instant, Arc<project_context::Index>)>>>;
pub(super) struct Job {
    pub _task: Task<()>,
    pub cancelled: Arc<AtomicBool>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}
#[derive(Default)]
pub(super) struct Search {
    pub job: Option<Job>,
    cache: Cache,
    pub targets: Vec<(PathBuf, String, Option<usize>)>,
    pub key: Option<(PathBuf, String, String)>,
    pub base: Vec<CommandItem>,
}

pub(super) fn query(
    root: &std::path::Path,
    query: &str,
    category: &str,
    cache: &Cache,
    cancel: &AtomicBool,
) -> Result<Vec<(String, String, Option<usize>)>, String> {
    if ssh_project::is_remote(root) {
        return Err("SSH 项目搜索暂不可用，请选择本地项目".into());
    }
    let mut results = Vec::new();
    if category != "文本" {
        let cached = cache
            .lock()
            .unwrap()
            .as_ref()
            .filter(|(path, at, _)| path == root && at.elapsed() < Duration::from_secs(30))
            .map(|(_, _, index)| index.clone());
        let index = match cached {
            Some(index) => index,
            None => {
                let index = Arc::new(project_context::Index::scan_cancellable(root, cancel)?);
                *cache.lock().unwrap() = Some((root.to_path_buf(), Instant::now(), index.clone()));
                index
            }
        };
        for entry in index
            .search(query)
            .into_iter()
            .filter(|entry| !entry.directory)
        {
            results.push((entry.path.clone(), entry.path.clone(), None));
        }
    }
    if category == "文件" || query.is_empty() || cancel.load(Ordering::Relaxed) {
        return Ok(results);
    }
    let matcher = grep_regex::RegexMatcherBuilder::new()
        .case_insensitive(true)
        .fixed_strings(true)
        .build(query)
        .map_err(|e| e.to_string())?;
    let matches = Mutex::new(Vec::new());
    let mut walker = ignore::WalkBuilder::new(root);
    walker
        .hidden(false)
        .follow_links(false)
        .require_git(false)
        .threads(2)
        .filter_entry(|entry| {
            entry.depth() == 0
                || !matches!(
                    entry.file_name().to_str(),
                    Some(".git" | "target" | "node_modules" | ".venv" | ".cache")
                )
        });
    walker.build_parallel().run(|| {
        let matches = &matches;
        let matcher = &matcher;
        Box::new(move |entry| {
            if cancel.load(Ordering::Relaxed) || matches.lock().unwrap().len() >= 100 {
                return ignore::WalkState::Quit;
            }
            let Ok(entry) = entry else {
                return ignore::WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|kind| kind.is_file())
                || entry.metadata().map_or(true, |m| m.len() > 2 * 1024 * 1024)
            {
                return ignore::WalkState::Continue;
            }
            let relative = entry
                .path()
                .strip_prefix(root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .into_owned();
            let mut searcher = grep_searcher::SearcherBuilder::new()
                .line_number(true)
                .binary_detection(grep_searcher::BinaryDetection::quit(b'\0'))
                .build();
            let _ = searcher.search_path(
                matcher,
                entry.path(),
                grep_searcher::sinks::UTF8(|line_number, line| {
                    if cancel.load(Ordering::Relaxed) {
                        return Ok(false);
                    }
                    let excerpt: String = line.trim().chars().take(180).collect();
                    let mut matches = matches.lock().unwrap();
                    if matches.len() >= 100 {
                        return Ok(false);
                    }
                    matches.push((
                        relative.clone(),
                        format!("{relative}:{line_number}  {excerpt}"),
                        Some(line_number as usize),
                    ));
                    Ok(matches.len() < 100)
                }),
            );
            ignore::WalkState::Continue
        })
    });
    let mut matches = matches.into_inner().unwrap();
    matches.sort_by(|a, b| a.0.cmp(&b.0).then(a.2.cmp(&b.2)));
    results.extend(matches);
    Ok(results)
}

impl AgentWorkspace {
    pub(super) fn schedule_project_search(
        &mut self,
        query: String,
        category: String,
        cx: &mut Context<Self>,
    ) {
        let root = if self.page == crate::Page::Projects {
            self.browser_repositories
                .selected_root()
                .or_else(|| self.browser_project.clone())
        } else {
            self.history.active().project.clone()
        };
        let Some(root) = root else {
            return;
        };
        let key = (root.clone(), query.clone(), category.clone());
        if self.commands.search.key.as_ref() == Some(&key) {
            return;
        }
        self.commands.search.key = Some(key.clone());
        self.commands.search.job = None;
        self.commands.search.targets.clear();
        let base = self.commands.search.base.clone();
        self.commands
            .palette
            .update(cx, |palette, cx| palette.set_items(base.clone(), cx));
        if query.trim().is_empty() || matches!(category.as_str(), "操作" | "聊天") {
            return;
        }
        let mut loading = base.clone();
        loading.push(
            CommandItem::new("project-search-loading", "正在搜索项目…")
                .category(if category == "文本" {
                    "文本"
                } else {
                    "文件"
                })
                .keywords(query.clone())
                .disabled(true),
        );
        self.commands
            .palette
            .update(cx, |palette, cx| palette.set_items(loading, cx));
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        let cache = self.commands.search.cache.clone();
        let task = cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(180))
                .await;
            let work_root = root.clone();
            let work_query = query.clone();
            let work = cx.background_executor().spawn(async move {
                if ssh_project::is_remote(&work_root) {
                    let job = provider::Job::start(remote_query(work_root, work_query, category))?;
                    job.result().await
                } else {
                    query_project(&work_root, &work_query, &category, &cache, &cancel)
                }
            });
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if this.commands.search.key.as_ref() != Some(&key) {
                    return;
                }
                let mut items = base;
                match result {
                    Ok(results) => {
                        for (path, label, line) in results {
                            let index = this.commands.search.targets.len();
                            this.commands
                                .search
                                .targets
                                .push((root.clone(), path, line));
                            let target_path = &this.commands.search.targets[index].1;
                            let title = line.map_or_else(
                                || target_path.clone(),
                                |line| format!("{target_path}:{line}"),
                            );
                            let description = if let Some(line) = line {
                                label
                                    .strip_prefix(&format!("{target_path}:{line}  "))
                                    .unwrap_or(&label)
                                    .to_owned()
                            } else {
                                "打开文件".to_owned()
                            };
                            items.push(
                                CommandItem::new(format!("project-result:{index}"), title)
                                    .description(description)
                                    .category(if line.is_some() { "文本" } else { "文件" })
                                    .group(if line.is_some() {
                                        "文本匹配 · 最多 100 条"
                                    } else {
                                        "项目文件"
                                    })
                                    .keywords(query.clone())
                                    .icon(if line.is_some() {
                                        Icon::Text
                                    } else {
                                        Icon::Code
                                    }),
                            );
                        }
                    }
                    Err(error) => items.push(
                        CommandItem::new("search-error", error)
                            .category("文本")
                            .keywords(query)
                            .disabled(true),
                    ),
                }
                this.commands
                    .palette
                    .update(cx, |palette, cx| palette.set_items(items, cx));
                cx.notify();
            });
        });
        self.commands.search.job = Some(Job {
            _task: task,
            cancelled,
        });
    }
}
fn query_project(
    root: &std::path::Path,
    text: &str,
    category: &str,
    cache: &Cache,
    cancel: &AtomicBool,
) -> Result<Vec<(String, String, Option<usize>)>, String> {
    query(root, text, category, cache, cancel)
}

async fn remote_query(
    root: PathBuf,
    query: String,
    category: String,
) -> Result<Vec<(String, String, Option<usize>)>, String> {
    use tokio::io::AsyncReadExt;
    tokio::time::timeout(Duration::from_secs(8), async move {
        let remote = ssh_project::Remote::from_path(&root)?;
        let mut results = Vec::new();
        if category != "文本" {
            let index = remote.action("index", serde_json::json!({}), false).await?;
            if let Some(entries) = index["entries"].as_array() {
                results.extend(
                    entries
                        .iter()
                        .filter(|entry| entry["directory"].as_bool() == Some(false))
                        .filter_map(|entry| entry["path"].as_str())
                        .filter(|path| path.to_lowercase().contains(&query.to_lowercase()))
                        .take(60)
                        .map(|path| (path.to_owned(), path.to_owned(), None)),
                );
            }
        }
        if category == "文件" {
            return Ok(results);
        }
        let args = [
            "--json",
            "--fixed-strings",
            "--ignore-case",
            "--hidden",
            "--max-filesize",
            "2M",
            "--glob",
            "!.git",
            "--glob",
            "!target",
            "--glob",
            "!node_modules",
            "--",
            &query,
            ".",
        ]
        .map(str::to_owned);
        let mut command = remote.managed_run("rg", &args);
        command
            .stdin(std::process::Stdio::piped())
            .kill_on_drop(true)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .take(512 * 1024)
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| e.to_string())?;
        let capped = bytes.len() >= 512 * 1024;
        if capped {
            let _ = child.kill().await;
        }
        let status = child.wait().await.map_err(|e| e.to_string())?;
        if !capped && !status.success() && status.code() != Some(1) {
            return Err("远程文本搜索失败，请确认远端已安装 rg 且项目可读".into());
        }
        for line in bytes.split(|byte| *byte == b'\n') {
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
                continue;
            };
            if value["type"] != "match" {
                continue;
            }
            let Some(path) = value["data"]["path"]["text"].as_str() else {
                continue;
            };
            let line = value["data"]["line_number"].as_u64().unwrap_or(1) as usize;
            let excerpt: String = value["data"]["lines"]["text"]
                .as_str()
                .unwrap_or("")
                .trim()
                .chars()
                .take(180)
                .collect();
            results.push((
                path.to_owned(),
                format!("{path}:{line}  {excerpt}"),
                Some(line),
            ));
            if results.iter().filter(|(_, _, line)| line.is_some()).count() >= 100 {
                break;
            }
        }
        Ok(results)
    })
    .await
    .map_err(|_| "远程项目搜索超时，请缩小搜索范围".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_search_obeys_ignores_limits_and_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(dir.path().join("main.rs"), "hello [world]\n").unwrap();
        std::fs::write(dir.path().join("ignored.txt"), "[world]\n").unwrap();
        std::fs::create_dir(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target/build.rs"), "[world]\n").unwrap();
        let cancel = AtomicBool::new(false);
        let cache = Cache::default();
        let files = query(dir.path(), "main", "文件", &cache, &cancel).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "main.rs");
        let text = query(dir.path(), "[WORLD]", "文本", &cache, &cancel).unwrap();
        assert_eq!(text.len(), 1);
        assert_eq!(text[0].2, Some(1));
        cancel.store(true, Ordering::Relaxed);
        assert!(
            query(dir.path(), "hello", "文本", &cache, &cancel)
                .unwrap()
                .is_empty()
        );
    }
}
