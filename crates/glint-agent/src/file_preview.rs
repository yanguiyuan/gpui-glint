//! Existing path references in chat are resolved off the UI thread, then linked.
use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use gpui_glint::components::{FilePreview, FilePreviewEntry, FilePreviewEvent};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, ops::Range, path::Path};
const IMAGE_LIMIT: u64 = 20 * 1024 * 1024;
fn is_png(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
}
const LIMIT: u64 = 2 * 1024 * 1024;
const PREFIX: &str = "glint-preview:";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Target {
    pub root: Option<PathBuf>,
    pub path: String,
    pub line: Option<usize>,
}
impl Target {
    fn link(&self) -> String {
        format!(
            "{PREFIX}{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).unwrap())
        )
    }
    fn decode(link: &str) -> Option<Self> {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(link.strip_prefix(PREFIX)?).ok()?).ok()
    }
}
struct Candidate {
    range: Range<usize>,
    value: String,
    explicit: bool,
}
fn plausible_reference(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !value
            .chars()
            .all(|c| c.is_ascii_punctuation() || c.is_whitespace())
        && !matches!(value, "or" | "and" | "或" | "且")
}
fn candidates(source: &str) -> Vec<Candidate> {
    let mut result = Vec::new();
    let mut in_link = false;
    let mut in_block = false;
    let mut link_start = None;
    for (event, range) in Parser::new(source).into_offset_iter() {
        if result.len() >= 128 {
            break;
        }
        match event {
            Event::Start(Tag::CodeBlock(_)) => in_block = true,
            Event::End(TagEnd::CodeBlock) => in_block = false,
            Event::Start(Tag::Link { dest_url, .. }) => {
                in_link = true;
                link_start = Some((range.start, dest_url.to_string()));
            }
            Event::End(TagEnd::Link) => {
                if let Some((start, value)) = link_start.take() {
                    result.push(Candidate {
                        range: start..range.end,
                        value,
                        explicit: true,
                    });
                }
                in_link = false;
            }
            Event::Code(value) if !in_block && !in_link && plausible_reference(&value) => result
                .push(Candidate {
                    range,
                    value: value.to_string(),
                    explicit: false,
                }),
            Event::Text(value)
                if !in_block && !in_link && source.get(range.clone()) == Some(value.as_ref()) =>
            {
                let mut offset = 0;
                for token in value.split_inclusive(char::is_whitespace) {
                    let text = token.trim_matches(|c: char| {
                        c.is_whitespace() || "，。；、,;!?()[]{}\"'<>".contains(c)
                    });
                    if plausible_reference(text)
                        && (text.contains('/') || text.contains('\\') || text.contains('.'))
                    {
                        let start = range.start + offset + token.find(text).unwrap();
                        result.push(Candidate {
                            range: start..start + text.len(),
                            value: text.into(),
                            explicit: false,
                        });
                    }
                    offset += token.len();
                    if result.len() >= 128 {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    result
}
fn target(value: &str, root: &Option<PathBuf>) -> Option<Target> {
    let mut value = value
        .trim()
        .strip_prefix('@')
        .unwrap_or(value.trim())
        .to_owned();
    if value.is_empty()
        || value.len() > 2048
        || value.contains(['\n', '\r', '\0'])
        || matches!(value.as_str(), "." | "..")
    {
        return None;
    }
    let mut line = None;
    if let Some((path, anchor)) = value.rsplit_once("#L") {
        if let Ok(number) = anchor.split('-').next()?.parse::<usize>() {
            line = Some(number);
            value = path.to_owned();
        }
    }
    for _ in 0..2 {
        if let Some((path, number)) = value.rsplit_once(':') {
            if let Ok(number) = number.split('-').next().unwrap_or(number).parse::<usize>() {
                line = Some(number);
                value = path.to_owned();
            } else {
                break;
            }
        }
    }
    let remote = root.as_deref().is_some_and(ssh_project::is_remote);
    if value.starts_with("file://") {
        if remote {
            return None;
        }
        value = reqwest::Url::parse(&value)
            .ok()?
            .to_file_path()
            .ok()?
            .to_string_lossy()
            .into_owned();
    } else if value.starts_with("ssh://") {
        let active = ssh_project::Remote::from_path(root.as_deref()?).ok()?;
        let cited = ssh_project::Remote::parse(&value, "~").ok()?;
        if active.target != cited.target || active.port != cited.port {
            return None;
        }
        value = cited.directory;
    } else if value.contains("://") || value.starts_with(PREFIX) || value.contains(['=', '|', '`'])
    {
        return None;
    }
    if remote {
        return Some(Target {
            root: root.clone(),
            path: value,
            line,
        });
    }
    let path = if let Some(suffix) = value.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME")?).join(suffix)
    } else {
        let path = PathBuf::from(&value);
        if path.is_absolute() {
            path
        } else {
            root.as_ref()?.join(path)
        }
    };
    Some(Target {
        root: root.clone(),
        path: path.to_string_lossy().into_owned(),
        line,
    })
}
fn previewable(path: &Path) -> bool {
    use std::io::Read;
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if meta.is_file() && is_png(path) {
        return meta.len() <= IMAGE_LIMIT;
    }
    if !meta.is_file() || meta.len() > LIMIT {
        return false;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut bytes = Vec::new();
    if file.take(8192).read_to_end(&mut bytes).is_err() || bytes.contains(&0) {
        return false;
    }
    // A sample can end in the middle of a UTF-8 codepoint.
    match std::str::from_utf8(&bytes) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    }
}
const PATH_CACHE_LIMIT: usize = 2048;
const PATH_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(10);
pub struct PathCache {
    epoch: std::sync::atomic::AtomicU64,
    values: std::sync::Mutex<HashMap<(Option<PathBuf>, String), (std::time::Instant, bool)>>,
    // Concurrent messages share completed lookups, including missing files.
    gate: tokio::sync::Mutex<()>,
    ttl: std::time::Duration,
    limit: usize,
}
impl Default for PathCache {
    fn default() -> Self {
        Self::new(PATH_CACHE_TTL, PATH_CACHE_LIMIT)
    }
}
fn path_key(root: &Option<PathBuf>, path: &str) -> String {
    let mut path = PathBuf::from(path);
    // Relative keys resolve against the same root the lookups use, so a path
    // and its absolute form invalidate the same entry.
    if !path.is_absolute()
        && let Some(root) = root.as_deref()
    {
        path = match ssh_project::Remote::from_path(root) {
            Ok(remote) => Path::new(&remote.directory).join(path),
            Err(_) => root.join(path),
        };
    }
    let mut normalized = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            part => normalized.push(part.as_os_str()),
        }
    }
    normalized.to_string_lossy().into_owned()
}
impl PathCache {
    fn new(ttl: std::time::Duration, limit: usize) -> Self {
        Self {
            epoch: Default::default(),
            values: Default::default(),
            gate: Default::default(),
            ttl,
            limit: limit.max(1),
        }
    }
    pub fn invalidate(&self, root: &Option<PathBuf>, path: Option<&str>) {
        self.epoch
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = path.map(|path| path_key(root, path));
        self.values
            .lock()
            .unwrap()
            .retain(|(r, p), _| r != root || path.as_ref().is_some_and(|path| path != p));
    }
}
pub struct Decorated {
    pub markdown: String,
    pub paths: std::collections::HashSet<String>,
    pub replacements: Vec<(Range<usize>, String)>,
}

pub fn apply_decorations(source: &str, replacements: &[(Range<usize>, String)]) -> String {
    let mut result = source.to_owned();
    for (range, replacement) in replacements.iter().rev() {
        result.replace_range(range.clone(), replacement);
    }
    result
}

/// Keep verified links while new tokens arrive, but re-check Markdown context:
/// a growing path or a newly opened code fence must not inherit an old link.
pub fn retain_decorations(previous: &str, source: &str, replacements: &mut Vec<(Range<usize>, String)>) {
    let previous_candidates = candidates(previous);
    let candidates = candidates(source);
    replacements.retain(|(range, _)| {
        source.get(range.clone()) == previous.get(range.clone())
            && candidates.iter().any(|candidate| candidate.range == *range
                && previous_candidates.iter().any(|old| old.range == *range
                    && old.value == candidate.value && old.explicit == candidate.explicit))
    });
}
#[cfg(test)]
pub async fn decorate(source: String, root: Option<PathBuf>) -> Result<String, String> {
    decorate_cached(source, root, Default::default())
        .await
        .map(|r| r.markdown)
}
pub async fn decorate_cached(
    source: String,
    root: Option<PathBuf>,
    cache: std::sync::Arc<PathCache>,
) -> Result<Decorated, String> {
    let candidates = candidates(&source);
    // Resolve every candidate once: the normalized key is what the cache stores,
    // so equivalent spellings of one file share a single lookup.
    let targets: Vec<_> = candidates
        .iter()
        .filter_map(|candidate| {
            target(&candidate.value, &root).map(|target| {
                let key = path_key(&root, &target.path);
                (candidate, target, key)
            })
        })
        .collect();
    let dependencies = targets.iter().map(|(_, _, key)| key.clone()).collect();
    let _gate = cache.gate.lock().await;
    let epoch = cache.epoch.load(std::sync::atomic::Ordering::Relaxed);
    let mut available = HashMap::new();
    let mut missing = Vec::new();
    {
        let values = cache.values.lock().unwrap();
        for (_, _, key) in &targets {
            if available.contains_key(key) || missing.contains(key) {
                continue;
            }
            match values
                .get(&(root.clone(), key.clone()))
                .filter(|(checked, _)| checked.elapsed() < cache.ttl)
            {
                Some((_, value)) => {
                    available.insert(key.clone(), *value);
                }
                None => missing.push(key.clone()),
            }
        }
    }
    let mut fresh = HashMap::new();
    if let Some(root) = root.as_deref().filter(|p| ssh_project::is_remote(p)) {
        if !missing.is_empty() {
            let response = ssh_project::Remote::from_path(root)?
                .action("preview_stat", serde_json::json!({"paths":missing}), true)
                .await?;
            for (path, kind) in response.as_object().ok_or("远程预览检查响应无效")? {
                fresh.insert(
                    path.clone(),
                    matches!(kind.as_str(), Some("directory" | "text" | "image")),
                );
            }
        }
    } else {
        for key in missing {
            let exists = {
                let path = Path::new(&key);
                path.is_dir() || previewable(path)
            };
            fresh.insert(key, exists);
        }
    }
    if !fresh.is_empty() {
        let mut values = cache.values.lock().unwrap();
        // Validate the generation under the same lock as the insertion: an
        // invalidation between a pre-lock check and this lock must win.
        if cache.epoch.load(std::sync::atomic::Ordering::Relaxed) == epoch {
            for (key, value) in &fresh {
                if values.len() >= cache.limit
                    && let Some(oldest) = values
                        .iter()
                        .min_by_key(|(_, v)| v.0)
                        .map(|(k, _)| k.clone())
                {
                    values.remove(&oldest);
                }
                values.insert(
                    (root.clone(), key.clone()),
                    (std::time::Instant::now(), *value),
                );
            }
        }
    }
    available.extend(fresh);
    let mut replacements = Vec::new();
    for (candidate, target, key) in targets {
        if available.get(&key) != Some(&true) {
            continue;
        }
        let original = &source[candidate.range.clone()];
        let label = if candidate.explicit {
            // Keep the original link label, replacing only its destination.
            original
                .strip_prefix('[')
                .and_then(|s| {
                    s.rsplit_once("](")
                        .or_else(|| s.rsplit_once("]["))
                        .map(|(label, _)| label)
                        .or_else(|| s.strip_suffix(']'))
                })
                .map(str::to_owned)
                .unwrap_or_else(|| candidate.value.replace(['[', ']'], ""))
        } else {
            original.to_owned()
        };
        replacements.push((
            candidate.range.clone(),
            format!("[{label}]({})", target.link()),
        ));
    }
    let result = apply_decorations(&source, &replacements);
    Ok(Decorated {
        markdown: result,
        paths: dependencies,
        replacements,
    })
}
enum Loaded {
    File(String),
    Image(Vec<u8>),
    Directory(Vec<FilePreviewEntry>, bool),
}
type PreviewData = (String, Option<String>, Loaded);
type NavigationData = (String, Vec<FilePreviewEntry>, bool);

async fn load_with_navigation(target: Target, known_directory: Option<String>) -> Result<(PreviewData, Option<NavigationData>), String> {
    let root = target.root.clone();
    let result = load(target).await?;
    let navigation = if !matches!(result.2, Loaded::Directory(..)) {
        if let Some(parent) = result.1.clone().filter(|parent| Some(parent) != known_directory.as_ref()) {
            match load(Target { root, path: parent, line: None }).await {
                Ok((path, _, Loaded::Directory(entries, truncated))) => Some((path, entries, truncated)),
                _ => None,
            }
        } else { None }
    } else { None };
    Ok((result, navigation))
}

async fn load(target: Target) -> Result<(String, Option<String>, Loaded), String> {
    if let Some(root) = target.root.as_deref().filter(|p| ssh_project::is_remote(p)) {
        let remote = ssh_project::Remote::from_path(root)?;
        let value = remote
            .action("preview", serde_json::json!({"path":target.path}), true)
            .await?;
        let path = value["path"].as_str().ok_or("远程预览路径无效")?.to_owned();
        let parent = value["parent"].as_str().map(str::to_owned);
        if value["kind"] == "image" {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(value["content"].as_str().ok_or("远程图片响应无效")?)
                .map_err(|_| "远程图片数据无效")?;
            return Ok((path, parent, Loaded::Image(bytes)));
        }
        if value["kind"] == "text" {
            return Ok((
                path,
                parent,
                Loaded::File(
                    value["content"]
                        .as_str()
                        .ok_or("远程文本响应无效")?
                        .to_owned(),
                ),
            ));
        }
        let entries = value["entries"]
            .as_array()
            .ok_or("远程目录响应无效")?
            .iter()
            .filter_map(|v| {
                Some(FilePreviewEntry {
                    name: v["name"].as_str()?.into(),
                    path: v["path"].as_str()?.into(),
                    directory: v["directory"].as_bool()?,
                    previewable: v["previewable"].as_bool()?,
                })
            })
            .collect();
        return Ok((
            path,
            parent,
            Loaded::Directory(entries, value["truncated"].as_bool().unwrap_or(false)),
        ));
    }
    let path = Path::new(&target.path)
        .canonicalize()
        .map_err(|_| "文件或目录不存在，可能已经移动或删除")?;
    let parent = path.parent().map(|p| p.to_string_lossy().into_owned());
    if path.is_dir() {
        let mut entries = Vec::new();
        for item in std::fs::read_dir(&path)
            .map_err(|_| "无法读取目录")?
            .take(1001)
        {
            let item = item.map_err(|_| "无法读取目录条目")?;
            let path = item.path();
            entries.push(FilePreviewEntry {
                name: item.file_name().to_string_lossy().into_owned(),
                path: path.to_string_lossy().into_owned(),
                directory: path.is_dir(),
                previewable: previewable(&path),
            });
        }
        entries.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.name.cmp(&b.name)));
        let truncated = entries.len() > 1000;
        entries.truncate(1000);
        return Ok((
            path.to_string_lossy().into_owned(),
            parent,
            Loaded::Directory(entries, truncated),
        ));
    }
    let metadata = std::fs::metadata(&path).map_err(|_| "无法读取文件属性")?;
    if metadata.is_file() && is_png(&path) {
        if metadata.len() > IMAGE_LIMIT {
            return Err("仅支持预览 20MB 以内的 PNG 图片".into());
        }
        let bytes = std::fs::read(&path).map_err(|_| "无法读取图片")?;
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err("PNG 图片格式无效".into());
        }
        return Ok((
            path.to_string_lossy().into_owned(),
            parent,
            Loaded::Image(bytes),
        ));
    }
    if !metadata.is_file() || metadata.len() > LIMIT {
        return Err("仅支持预览 2MB 以内的普通文本文件".into());
    }
    let content = std::fs::read_to_string(&path).map_err(|_| "该文件不是可预览的 UTF-8 文本")?;
    if content.as_bytes().contains(&0) {
        return Err("二进制文件不支持文本预览".into());
    }
    Ok((
        path.to_string_lossy().into_owned(),
        parent,
        Loaded::File(content),
    ))
}

// Hover previews read only a small prefix after the requested source line.
// The extra byte lets the display layer detect clipping without a full-file read.
async fn load_excerpt(
    target: Target,
    reference: gpui_glint::components::ComposerReference,
) -> Result<String, String> {
    let start = reference.start_line.unwrap_or(1).max(1);
    let lines = reference
        .end_line
        .map(|end| end.saturating_sub(start) + 1)
        .unwrap_or(25)
        .clamp(1, 25);
    if let Some(root) = target.root.as_deref().filter(|p| ssh_project::is_remote(p)) {
        let remote = ssh_project::Remote::from_path(root)?;
        return remote
            .action(
                "preview_excerpt",
                serde_json::json!({"path":target.path,"start_line":start,"lines":lines}),
                true,
            )
            .await?
            .as_str()
            .map(str::to_owned)
            .ok_or("远程预览响应无效".into());
    }
    use std::io::{BufRead, Read};
    let path = Path::new(&target.path);
    if !previewable(path) {
        return Err("不支持预览此文件".into());
    }
    let mut reader = std::io::BufReader::new(std::fs::File::open(path).map_err(|e| e.to_string())?);
    for _ in 1..start {
        if reader.skip_until(b'\n').map_err(|e| e.to_string())? == 0 {
            return Ok(String::new());
        }
    }
    let mut bytes = Vec::new();
    reader
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.contains(&0) {
        return Err("二进制文件不支持预览".into());
    }
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(error) if error.error_len().is_none() && bytes.len() == 4097 => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap()
        }
        Err(_) => return Err("文件不是 UTF-8 文本".into()),
    };
    Ok(text.split_inclusive('\n').take(lines).collect())
}

pub struct State {
    pub open: bool,
    pub expanded: bool,
    pub view: Entity<FilePreview>,
    pub paths: std::sync::Arc<PathCache>,
    target: Option<Target>,
    task: Option<Task<()>>,
    revision: u64,
    directory_tasks: HashMap<u64, Task<()>>,
    pub(super) hover_task: Option<Task<()>>,
}
impl State {
    pub fn new(cx: &mut Context<AgentWorkspace>) -> Self {
        let view = cx.new(FilePreview::new);
        cx.subscribe(&view, |this, _, event: &FilePreviewEvent, cx| match event {
            FilePreviewEvent::ExpandedChanged(expanded) => {
                this.file_preview.expanded = *expanded;
                this.right_sidebar_resize = None;
                cx.notify();
            }
            FilePreviewEvent::CloseRequested => {
                this.file_preview.expanded = false;
                this.file_preview.open = false;
                this.file_preview.task = None;
                this.file_preview.directory_tasks.clear();
                this.file_preview.view.update(cx, |view, cx| view.cancel_directory_requests(cx));
                cx.notify();
            }
            FilePreviewEvent::QuoteRequested { path, selection } => {
                if this.file_preview.target.is_none() {
                    // A pasted attachment is virtual; selected text must travel inline too.
                    let mut reference = gpui_glint::components::ComposerReference::pasted_text(
                        path,
                        &selection.text,
                    );
                    reference.start_line = Some(selection.start_line);
                    reference.end_line = Some(selection.end_line);
                    this.quote_reference(reference, cx);
                    return;
                }
                let path = if let Some(root) = this
                    .file_preview
                    .target
                    .as_ref()
                    .and_then(|t| t.root.as_deref())
                    .filter(|p| ssh_project::is_remote(p))
                {
                    let mut remote = ssh_project::Remote::from_path(root).unwrap();
                    remote.directory = path.clone();
                    remote.uri().to_string_lossy().into_owned()
                } else {
                    path.clone()
                };
                this.quote_file_excerpt(&path, selection, cx);
            }
            FilePreviewEvent::DirectoryRequested { path, request } => {
                let Some(mut target) = this.file_preview.target.clone() else { return; };
                target.path = path.clone();
                target.line = None;
                let request = *request;
                let path = path.clone();
                let work = cx.background_executor().spawn(async move {
                    let result = if target.root.as_deref().is_some_and(ssh_project::is_remote) {
                        provider::Job::start(load(target))?.result().await
                    } else { load(target).await };
                    match result? {
                        (_, _, Loaded::Directory(entries, truncated)) => Ok((entries, truncated)),
                        _ => Err("路径不是目录".into()),
                    }
                });
                let task = cx.spawn(async move |weak, cx| {
                    let result = work.await;
                    let _ = weak.update(cx, |this, cx| {
                        if let Some(task) = this.file_preview.directory_tasks.remove(&request) { task.detach(); }
                        this.file_preview.view.update(cx, |view, cx| view.set_directory_children(path, request, result, cx));
                    });
                });
                this.file_preview.directory_tasks.insert(request, task);
            }
            FilePreviewEvent::OpenRequested(path) => {
                if let Some(mut target) = this.file_preview.target.clone() {
                    target.path = path.clone();
                    target.line = None;
                    this.open_file_preview(target, cx);
                }
            }
        })
        .detach();
        Self {
            open: false,
            expanded: false,
            view,
            paths: Default::default(),
            target: None,
            task: None,
            revision: 0,
            directory_tasks: Default::default(),
            hover_task: None,
        }
    }
}
impl AgentWorkspace {
    pub fn invalidate_preview_paths(&self, id: Uuid, record: &tools::ToolRecord) {
        let Some(session) = self.history.sessions.iter().find(|s| s.id == id) else {
            return;
        };
        let root = session
            .project
            .clone();
        let path = match record.call.name.as_str() {
            "write_file" | "replace_in_file" => record
                .call
                .arguments
                .get("path")
                .and_then(serde_json::Value::as_str)
                .and_then(|p| target(p, &root))
                .map(|t| t.path),
            "code_mode" => {
                for child in record.nested_tools.iter() {
                    self.invalidate_preview_paths(id, child);
                }
                return;
            }
            "command_poll" | "command_stop" => {
                if serde_json::from_str::<serde_json::Value>(&record.result)
                    .ok()
                    .is_some_and(|v| v["status"] == "running")
                {
                    return;
                }
                None
            }
            "run_command" => {
                let program = record.call.arguments["program"].as_str().unwrap_or("");
                let args = record.call.arguments["args"].as_array();
                let readonly = matches!(program, "pwd" | "ls" | "cat" | "head" | "tail" | "wc")
                    || program == "git"
                        && args
                            .and_then(|a| a.first())
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|a| matches!(a, "status" | "ls-files"));
                if readonly {
                    return;
                }
                None
            }
            _ => return,
        };
        self.file_preview.paths.invalidate(&root, path.as_deref());
        let key = path.as_deref().map(|p| path_key(&root, p));
        for view in self.markdown_views.borrow_mut().values_mut() {
            if view.root == root
                && (key.as_ref().is_none_or(|p| view.paths.contains(p))
                    || view.links_task.is_some())
            {
                view.links_source = None;
                view.links_revision += 1;
                view.links_task = None;
            }
        }
    }
    pub fn reload_preview_after_undo(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if !self.file_preview.open {
            return;
        }
        if let Some(current) = self.file_preview.target.clone() {
            let key = path_key(&current.root, &current.path);
            if paths.iter().any(|path| {
                target(&path.to_string_lossy(), &current.root)
                    .is_some_and(|target| path_key(&target.root, &target.path) == key)
            }) {
                self.open_file_preview(current, cx);
            }
        }
    }
    pub fn load_reference_preview(
        &mut self,
        reference: gpui_glint::components::ComposerReference,
        cx: &mut Context<Self>,
    ) {
        if reference.directory || reference.preview.is_some() || reference.excerpt.is_some() {
            return;
        }
        let Some(target) = target(&reference.path, &self.history.active().project) else {
            return;
        };
        let session = self.history.active;
        let source = reference.clone();
        self.file_preview.hover_task = Some(cx.spawn(async move |weak, cx| {
            let work = cx.background_executor().spawn(async move {
                if target.root.as_deref().is_some_and(ssh_project::is_remote) {
                    provider::Job::start(load_excerpt(target, source))?
                        .result()
                        .await
                } else {
                    load_excerpt(target, source).await
                }
            });
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if this.history.active != session {
                    return;
                }
                let text = result
                    .unwrap_or_else(|_| "无法读取此文件的预览，文件可能已移动或删除。".into());
                this.composer.update(cx, |composer, cx| {
                    composer.set_reference_preview(&reference, &text, cx)
                });
            });
        }));
    }
    fn quote_file_excerpt(
        &mut self,
        path: &str,
        selection: &gpui_glint::components::CodeSelection,
        cx: &mut Context<Self>,
    ) {
        self.quote_reference(
            gpui_glint::components::ComposerReference::selection(
                path,
                selection.start_line,
                selection.end_line,
                &selection.text,
            ),
            cx,
        );
    }
    fn quote_reference(
        &mut self,
        reference: gpui_glint::components::ComposerReference,
        cx: &mut Context<Self>,
    ) {
        self.composer
            .update(cx, |composer, cx| composer.add_reference(reference, cx));
        self.capture_draft(cx);
        self.save_draft(cx);
        let focus = self.composer.focus_handle(cx);
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, cx| window.focus(&focus, cx));
        }
        cx.notify();
    }
    pub fn preview_link(&mut self, link: &str, origin: &Option<PathBuf>, cx: &mut Context<Self>) {
        if let Some(location) = link.strip_prefix("glint-paste://") {
            let parts: Vec<_> = location.split('/').collect();
            if parts.len() != 3 {
                return;
            }
            let (Ok(session), Ok(message), Ok(attachment)) = (
                uuid::Uuid::parse_str(parts[0]),
                parts[1].parse::<usize>(),
                parts[2].parse::<usize>(),
            ) else {
                return;
            };
            if session != self.history.active || &self.history.active().project != origin {
                return;
            }
            let Some(reference) = self
                .history
                .active()
                .messages
                .get(message)
                .and_then(|m| m.display.as_ref())
                .and_then(|d| d.attachments.get(attachment))
                .cloned()
            else {
                return;
            };
            let Some(content) = reference.excerpt else {
                return;
            };
            self.terminal_ui.open = false;
            self.edits_ui.open = false;
            self.git_ui.open = false;
            self.file_preview.open = true;
            self.file_preview.expanded = false;
            self.file_preview.revision += 1;
            self.file_preview.task = None;
            self.file_preview.target = None;
            self.file_preview.view.update(cx, |view, cx| {
                view.set_theme(self.theme, cx);
                view.set_file(reference.path, None, "text".into(), content, cx);
            });
            cx.notify();
            return;
        }
        if let Some(target) = Target::decode(link) {
            if &target.root == origin {
                self.open_file_preview(target, cx);
            }
        } else if link.starts_with("https://") || link.starts_with("http://") {
            cx.open_url(link);
        }
    }
    pub(super) fn preview_changed_file(
        &mut self,
        path: &Path,
        root: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if let Some(target) = target(&path.to_string_lossy(), &root) {
            self.open_file_preview_mode(target, true, cx);
        }
    }
    fn open_file_preview(&mut self, target: Target, cx: &mut Context<Self>) {
        self.open_file_preview_mode(target, false, cx);
    }
    pub(super) fn open_project_directory(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.history.active().project.clone() else {
            return;
        };
        let path = if ssh_project::is_remote(&root) {
            match ssh_project::Remote::from_path(&root) {
                Ok(remote) => remote.directory,
                Err(_) => return,
            }
        } else {
            root.to_string_lossy().into_owned()
        };
        self.page = crate::Page::Chat;
        self.git_ui.open = false;
        self.file_preview.expanded = false;
        self.right_sidebar_width = self.right_sidebar_width.max(400.);
        self.open_file_preview(
            Target {
                root: Some(root),
                path,
                line: None,
            },
            cx,
        );
    }
    fn open_file_preview_mode(&mut self, target: Target, rendered: bool, cx: &mut Context<Self>) {
        if !self.terminal_ui.bottom { self.terminal_ui.open = false; }
        self.file_preview.open = true;
        self.edits_ui.open = false;
        self.file_preview.revision += 1;
        let revision = self.file_preview.revision;
        self.file_preview.target = Some(target.clone());
        self.file_preview.view.update(cx, |view, cx| {
            view.set_theme(self.theme, cx);
            view.set_loading(target.path.clone(), cx);
        });
        let known_directory = self.file_preview.view.read(cx).navigation_path().map(str::to_owned);
        self.file_preview.task = Some(cx.spawn(async move |weak, cx| {
            let line = target.line;
            let work = cx.background_executor().spawn(async move {
                if target.root.as_deref().is_some_and(ssh_project::is_remote) {
                    provider::Job::start(load_with_navigation(target, known_directory))?.result().await
                } else {
                    load_with_navigation(target, known_directory).await
                }
            });
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if this.file_preview.revision != revision {
                    return;
                }
                if let Some(task) = this.file_preview.task.take() {
                    task.detach();
                }
                let result = result.map(|(result, navigation)| {
                    if let Some((path, entries, truncated)) = navigation {
                        this.file_preview.view.update(cx, |view, cx| view.set_navigation(path, entries, truncated, cx));
                    }
                    result
                });
                this.file_preview.view.update(cx, |view, cx| match result {
                    Ok((path, parent, Loaded::Image(bytes))) => {
                        view.set_image(
                            path,
                            parent,
                            std::sync::Arc::new(gpui::Image::from_bytes(
                                gpui::ImageFormat::Png,
                                bytes,
                            )),
                            cx,
                        );
                    }
                    Ok((path, parent, Loaded::File(content))) => {
                        let language = Path::new(&path)
                            .extension()
                            .and_then(|s| s.to_str())
                            .unwrap_or("text")
                            .to_owned();
                        view.set_file(path, parent, language, content, cx);
                        view.set_rendered_preview(rendered, cx);
                        if let Some(line) = line {
                            view.reveal_line(line, cx);
                        }
                    }
                    Ok((path, parent, Loaded::Directory(mut entries, truncated))) => {
                        if truncated {
                            entries.push(FilePreviewEntry {
                                name: "仅显示前 1000 项".into(),
                                path: String::new(),
                                directory: false,
                                previewable: false,
                            });
                        }
                        view.set_directory(path, parent, entries, cx);
                    }
                    Err(error) => view.set_error(error, cx),
                });
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub fn file_preview_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        self.file_preview.view.update(cx, |view, cx| {
            view.set_theme(self.theme, cx);
            view.set_expanded(self.file_preview.expanded, cx);
        });
        div()
            .when(self.file_preview.expanded, |view| view.w_full())
            .when(!self.file_preview.expanded, |view| {
                view.w(px(self.right_sidebar_width))
            })
            .flex_shrink_0()
            .h_full()
            .min_h_0()
            .border_l_1()
            .border_color(rgb(self.theme.palette().border))
            .child(self.file_preview.view.clone())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn pasted_attachment_links_open_without_reading_a_project_file(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        host.update(window, |host, cx| {
            let id = host.history.active;
            let content = "原始附件\n".repeat(30);
            let mut reference =
                gpui_glint::components::ComposerReference::pasted_text("粘贴文本-1.txt", &content);
            reference.token = reference.label();
            host.history
                .active_mut()
                .push(crate::model::Role::User, content);
            host.history.active_mut().messages[0].display = Some(crate::model::MessageDisplay {
                text: reference.token.clone(),
                attachments: vec![reference],
            });
            host.preview_link(&format!("glint-paste://{id}/0/0"), &None, cx);
            assert!(host.file_preview.open);
            assert!(host.file_preview.target.is_none());
            assert!(host.file_preview.task.is_none());
            host.file_preview.open = false;
            for link in [
                format!("glint-paste://{}/0/0", uuid::Uuid::new_v4()),
                format!("glint-paste://{id}/0/999"),
                "glint-paste://invalid".into(),
            ] {
                host.preview_link(&link, &None, cx);
                assert!(!host.file_preview.open);
            }
        });
    }
    #[gpui::test]
    fn command_opens_project_root_in_preview_sidebar(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        window.update(|window, cx| {
            host.update(cx, |host, cx| {
                host.history.active_mut().project = Some(dir.path().into());
                host.file_preview.expanded = true;
                host.edits_ui.open = true;
                host.git_ui.open = true;
                host.commands.pending = Some("project-files".into());
                host.execute_pending_command(window, cx);
                let target = host.file_preview.target.as_ref().unwrap();
                assert_eq!(target.root.as_deref(), Some(dir.path()));
                assert_eq!(target.path, dir.path().to_string_lossy());
                assert!(host.file_preview.open && !host.file_preview.expanded);
                assert!(!host.edits_ui.open && !host.git_ui.open && !host.terminal_ui.open);
            })
        });
    }
    fn rendered(source: &str) -> String {
        Parser::new(source)
            .filter_map(|e| match e {
                Event::Text(s) | Event::Code(s) => Some(s.to_string()),
                _ => None,
            })
            .collect()
    }
    #[tokio::test]
    async fn png_preview_loads_bytes_and_is_available_in_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.PNG");
        let bytes = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1sAAAAASUVORK5CYII=").unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert!(previewable(&path));
        let target = Target {
            root: None,
            path: path.to_string_lossy().into_owned(),
            line: None,
        };
        let (_, _, loaded) = load(target.clone()).await.unwrap();
        assert!(matches!(loaded, Loaded::Image(data) if data == bytes));
        let (_, _, listing) = load(Target {
            path: dir.path().to_string_lossy().into_owned(),
            ..target.clone()
        })
        .await
        .unwrap();
        assert!(matches!(listing, Loaded::Directory(entries, _) if entries[0].previewable));
        std::fs::write(&path, b"invalid png").unwrap();
        assert!(load(target).await.is_err());
    }
    #[tokio::test]
    async fn hover_excerpt_bounds_bytes_preserves_utf8_and_respects_line_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.rs");
        std::fs::write(&path, format!("first\n{}\nlast\n", "汉".repeat(400_000))).unwrap();
        let target = Target {
            root: Some(dir.path().into()),
            path: path.to_string_lossy().into_owned(),
            line: None,
        };
        let mut reference = gpui_glint::components::ComposerReference::file(
            path.to_string_lossy().into_owned(),
            false,
        );
        reference.start_line = Some(2);
        reference.end_line = Some(2);
        let text = load_excerpt(target.clone(), reference.clone())
            .await
            .unwrap();
        assert!(text.len() <= 4097);
        assert!(text.starts_with("汉"));
        assert!(!text.contains("last"));
        reference.start_line = Some(3);
        reference.end_line = Some(3);
        assert_eq!(
            load_excerpt(target.clone(), reference.clone())
                .await
                .unwrap(),
            "last\n"
        );
        std::fs::write(&path, [0, 1, 255]).unwrap();
        assert!(load_excerpt(target, reference).await.is_err());
    }
    #[tokio::test]
    async fn standalone_path_separators_and_operators_are_not_file_references() {
        let source = "`document.rs` / `rich_text.rs` 或 `|`、`||`、`/`、`\\`、`&&`。";
        let candidates = candidates(source);
        assert!(
            !candidates.iter().any(|candidate| matches!(
                candidate.value.as_str(),
                "/" | "\\" | "|" | "||" | "&&"
            ))
        );
        let result = decorate(source.into(), Some(std::env::temp_dir()))
            .await
            .unwrap();
        assert!(!result.contains(PREFIX));
    }

    #[tokio::test]
    async fn existing_references_link_without_changing_visible_text_or_fenced_code() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# readme").unwrap();
        std::fs::write(dir.path().join("image.bin"), [0, 1, 2]).unwrap();
        let source = "查看 `src/main.rs:12:5`、`src/`、README.md，[入口](src/main.rs#L2)。`missing.rs` `image.bin`\n\n```sh\nsrc/main.rs\n```\n\n[引用][code]\n\n[code]: src/main.rs\n";
        let result = decorate(source.into(), Some(dir.path().into()))
            .await
            .unwrap();
        assert_eq!(rendered(source), rendered(&result));
        let targets: Vec<_> = Parser::new(&result)
            .filter_map(|e| match e {
                Event::Start(Tag::Link { dest_url, .. }) => Target::decode(&dest_url),
                _ => None,
            })
            .collect();
        assert_eq!(targets.len(), 5, "{result}");
        assert_eq!(targets[0].line, Some(12));
        assert_eq!(targets[3].line, Some(2));
        assert!(result.contains("```sh\nsrc/main.rs\n```"));
        assert!(result.contains("`missing.rs` `image.bin`"));
    }
    #[tokio::test]
    async fn local_preview_handles_text_directories_binary_and_removed_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("folder")).unwrap();
        std::fs::write(dir.path().join("a.txt"), "alpha\nβeta\n").unwrap();
        std::fs::write(dir.path().join("b.bin"), [0, 1, 2]).unwrap();
        let root = Some(dir.path().to_path_buf());
        let t = target("a.txt#L2", &root).unwrap();
        assert_eq!(Target::decode(&t.link()), Some(t.clone()));
        assert!(
            matches!(load(t.clone()).await.unwrap().2, Loaded::File(ref s) if s.contains("βeta"))
        );
        let entries = match load(target("folder/..", &root).unwrap()).await.unwrap().2 {
            Loaded::Directory(e, _) => e,
            _ => panic!(),
        };
        assert!(entries[0].directory);
        assert!(
            !entries
                .iter()
                .find(|e| e.name == "b.bin")
                .unwrap()
                .previewable
        );
        assert!(load(target("b.bin", &root).unwrap()).await.is_err());
        std::fs::remove_file(dir.path().join("a.txt")).unwrap();
        assert!(load(t).await.is_err());
        assert!(
            !decorate("`a.txt`".into(), root)
                .await
                .unwrap()
                .contains(PREFIX)
        );
    }
    #[test]
    fn references_do_not_automatically_connect_to_other_ssh_hosts() {
        let root = Some(PathBuf::from("ssh://user@example.com/home/user/project"));
        assert!(target("ssh://other@example.org/etc/passwd", &root).is_none());
        assert!(target("ssh://user@example.com/etc/hosts#L4", &root).is_some());
        assert!(target("https://example.com/path.rs", &root).is_none());
        assert_eq!(target("src/main.rs:42:5", &root).unwrap().line, Some(42));
    }
    #[gpui::test]
    fn ordinary_file_references_load_display_preview_without_sending_source(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let path = dir.path().join("source.rs");
        std::fs::write(&path, "source preview only\n").unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        host.update(window, |h, cx| {
            h.composer.update(cx, |c, cx| {
                c.add_reference(
                    gpui_glint::components::ComposerReference::file(
                        path.to_string_lossy().into_owned(),
                        false,
                    ),
                    cx,
                )
            });
            let reference = h.composer.read(cx).references(cx)[0].clone();
            h.load_reference_preview(reference, cx);
        });
        window.run_until_parked();
        host.read_with(window, |h, cx| {
            let c = h.composer.read(cx);
            let references = c.references(cx);
            assert_eq!(
                references[0].preview.as_deref(),
                Some("source preview only\n")
            );
            let mut submission = gpui_glint::components::ComposerSubmission::text(c.value(cx));
            submission.references = references;
            assert!(!submission.prompt_text().contains("source preview only"));
            assert!(submission.prompt_text().contains("source.rs"));
        });
    }

    #[gpui::test]
    fn selected_excerpt_inserts_file_path_line_range_and_preserves_existing_prompt(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        host.update(window, |h, cx| {
            h.composer
                .update(cx, |composer, cx| composer.set_value("解释这里的逻辑", cx));
            h.quote_file_excerpt(
                "/project/source.rs",
                &gpui_glint::components::CodeSelection {
                    start_line: 12,
                    end_line: 14,
                    text: "fn example() {\n    // ``` quote\n}".into(),
                },
                cx,
            );
        });
        host.read_with(window, |h, cx| {
            let text = h.composer.read(cx).value(cx).to_string();
            assert!(text.starts_with("解释这里的逻辑"));
            assert!(text.contains("@source.rs:12-14"));
            assert!(!text.contains("fn example()"));
            let references = h.composer.read(cx).references(cx);
            assert_eq!(references.len(), 1);
            let mut submission = gpui_glint::components::ComposerSubmission::text(text.clone());
            submission.references = references.clone();
            let prompt = submission.prompt_text();
            assert!(prompt.contains("文件引用：/project/source.rs，行范围 12-14"));
            assert!(prompt.contains("````\nfn example()"));
            assert_eq!(h.history.active().draft_references, references);
            let path = dir.path().join("restart.json");
            h.history.save(&path).unwrap();
            let restored = model::History::load(&path).unwrap();
            assert_eq!(restored.active().draft_references, references);
            assert_eq!(restored.active().draft, text);
            assert_eq!(h.history.active().draft, text);
        });
    }

    #[gpui::test]
    fn changed_file_title_keeps_diff_navigation_and_preview_opens_rendered_file(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let path = dir.path().join("README.md");
        std::fs::write(&path, "# Changed file\n\nPreview content").unwrap();
        let change: crate::edits::Change = serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::new_v4(), "path": path, "label": "README.md",
            "before": "", "after": "# Changed file\n\nPreview content", "added": 3, "removed": 0
        }))
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        host.update(window, |host, cx| {
            let card = host.edit_diff("preview-test", &change, cx);
            card.update(cx, |_, cx| {
                cx.emit(gpui_glint::components::DiffCardEvent::TitleClicked)
            });
        });
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert!(host.edits_ui.open);
            assert!(!host.file_preview.open);
        });
        host.update(window, |host, cx| {
            host.preview_changed_file(&path, None, cx)
        });
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert!(host.file_preview.open);
            assert!(!host.edits_ui.open);
            assert_eq!(
                host.file_preview.target.as_ref().unwrap().path,
                path.to_string_lossy()
            );
        });
        assert!(
            window
                .debug_bounds("glass-document-scroll-preview")
                .is_some()
        );
    }

    #[gpui::test]
    fn preview_tree_expands_nested_folders_and_keeps_the_selected_file_when_collapsed(cx: &mut gpui::TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        std::fs::write(root.join("providers.json"), r#"{"providers":[],"active":null,"active_model":""}"#).unwrap();
        std::fs::create_dir_all(root.join("src/nested")).unwrap();
        std::fs::write(root.join("src/nested/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("README.md"), "# Project").unwrap();
        let (host, window) = cx.add_window_view(|_, cx| AgentWorkspace::with_path(root.join("sessions.json"), cx));
        window.simulate_resize(size(px(1440.), px(900.)));
        host.update(window, |host, cx| {
            host.storage_readable = false;
            host.file_preview.expanded = true;
            host.open_file_preview(Target { root: Some(root.clone()), path: root.to_string_lossy().into_owned(), line: None }, cx);
        });
        window.run_until_parked();
        let folder = Box::leak(format!("preview-entry:{}", root.join("src").display()).into_boxed_str());
        let nested = Box::leak(format!("preview-entry:{}", root.join("src/nested").display()).into_boxed_str());
        let file = Box::leak(format!("preview-entry:{}", root.join("src/nested/main.rs").display()).into_boxed_str());
        assert!(window.debug_bounds(nested).is_none());
        let row = window.debug_bounds(folder).unwrap();
        window.simulate_click(row.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, _| assert_eq!(host.file_preview.target.as_ref().unwrap().path, root.to_string_lossy()));
        let child = window.debug_bounds(nested).expect("expanded child folder");
        assert!(child.left() >= row.left());
        window.simulate_click(child.center(), Default::default());
        window.run_until_parked();
        let file_row = window.debug_bounds(file).expect("grandchild file");
        window.simulate_click(file_row.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert!(host.file_preview.expanded);
            assert_eq!(host.file_preview.target.as_ref().unwrap().path, root.join("src/nested/main.rs").to_string_lossy());
        });
        assert!(window.debug_bounds("file-preview-navigation").is_some());
        let row = window.debug_bounds(folder).unwrap();
        window.simulate_click(row.center(), Default::default());
        window.run_until_parked();
        assert!(window.debug_bounds(file).is_none());
        host.read_with(window, |host, _| assert_eq!(host.file_preview.target.as_ref().unwrap().path, root.join("src/nested/main.rs").to_string_lossy()));
        let row = window.debug_bounds(folder).unwrap();
        window.simulate_click(row.center(), Default::default());
        window.run_until_parked();
        assert!(window.debug_bounds(file).is_some(), "nested expansion state must survive collapsing an ancestor");
        host.read_with(window, |host, _| assert!(host.file_preview.directory_tasks.is_empty()));
    }

    #[gpui::test]
    fn expanded_preview_keeps_directory_and_switches_files_without_collapsing(cx: &mut gpui::TestAppContext) {
        cx.executor().allow_parking();
        cx.update(crate::initialize);
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        std::fs::write(root.join("providers.json"), r#"{"providers":[],"active":null,"active_model":""}"#).unwrap();
        std::fs::write(root.join("a.rs"), "fn a() {}
").unwrap();
        std::fs::write(root.join("b.rs"), "fn b() {}
").unwrap();
        let (host, window) = cx.add_window_view(|_, cx| AgentWorkspace::with_path(root.join("sessions.json"), cx));
        window.simulate_resize(size(px(1440.), px(900.)));
        host.update(window, |host, cx| {
            host.storage_readable = false;
            host.file_preview.expanded = true;
            host.history.active_mut().project = Some(root.clone());
            host.open_file_preview(Target { root: Some(root.clone()), path: root.join("a.rs").to_string_lossy().into_owned(), line: None }, cx);
        });
        window.run_until_parked();
        let navigation = window.debug_bounds("file-preview-navigation").expect("expanded preview keeps its folder");
        assert!(navigation.size.width > px(100.));
        let selector = Box::leak(format!("preview-entry:{}", root.join("b.rs").display()).into_boxed_str());
        let next = window.debug_bounds(selector).expect("sibling file in navigation");
        assert!(next.right() <= navigation.right());
        window.simulate_click(next.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert!(host.file_preview.expanded);
            assert_eq!(host.file_preview.target.as_ref().unwrap().path, root.join("b.rs").to_string_lossy());
        });
        assert!(window.debug_bounds("file-preview-navigation").is_some());
        window.update(|window, cx| host.update(cx, |host, cx| window.focus(&host.focus, cx)));
        window.simulate_keystrokes("ctrl-`");
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert!(host.file_preview.open && host.file_preview.expanded);
            assert!(host.terminal_ui.open && host.terminal_ui.bottom);
            assert_eq!(host.history.terminal_default_position, crate::terminal_ui::Position::Right);
            assert_eq!(host.file_preview.target.as_ref().unwrap().path, root.join("b.rs").to_string_lossy());
        });
        let terminal = window.debug_bounds("project-terminal-sidebar").unwrap();
        let navigation = window.debug_bounds("file-preview-navigation").unwrap();
        assert!(navigation.bottom() <= terminal.top() + px(1.), "preview must fit above the bottom terminal");
        let first_selector = Box::leak(format!("preview-entry:{}", root.join("a.rs").display()).into_boxed_str());
        let first = window.debug_bounds(first_selector).unwrap();
        window.simulate_click(first.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert!(host.terminal_ui.open && host.file_preview.expanded);
            assert_eq!(host.file_preview.target.as_ref().unwrap().path, root.join("a.rs").to_string_lossy());
        });
        let hide = window.debug_bounds("project-terminal-hide").unwrap();
        window.simulate_click(hide.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, _| assert!(!host.terminal_ui.open && host.file_preview.open && host.file_preview.expanded));
        assert!(window.debug_bounds("file-preview-navigation").unwrap().size.height > navigation.size.height);
        let collapse = window.debug_bounds("file-preview-expand").unwrap();
        window.simulate_click(collapse.center(), Default::default());
        window.run_until_parked();
        assert!(window.debug_bounds("file-preview-navigation").is_none());
        assert!(window.debug_bounds("agent-right-sidebar-resize").is_some());
        assert_eq!(std::fs::read_to_string(root.join("b.rs")).unwrap(), "fn b() {}
");
    }

    #[gpui::test]
    fn sidebar_opens_browses_and_resizes_without_modifying_files(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("preview.txt"), "read only\n").unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        host.update(window, |h, cx| {
            h.open_file_preview(
                Target {
                    root: Some(dir.path().into()),
                    path: dir.path().to_string_lossy().into_owned(),
                    line: None,
                },
                cx,
            );
        });
        window.run_until_parked();
        let path = dir.path().canonicalize().unwrap().join("preview.txt");
        let selector = Box::leak(format!("preview-entry:{}", path.display()).into_boxed_str());
        let entry = window.debug_bounds(selector).unwrap();
        window.simulate_click(entry.center(), gpui::Modifiers::default());
        window.run_until_parked();
        host.read_with(window, |h, _| {
            assert_eq!(
                h.file_preview.target.as_ref().unwrap().path,
                path.to_string_lossy()
            )
        });
        let handle = window.debug_bounds("agent-right-sidebar-resize").unwrap();
        let start = host.read_with(window, |h, _| h.right_sidebar_width);
        let from = handle.center();
        let to = point(from.x - px(100.), from.y);
        window.simulate_mouse_down(from, gpui::MouseButton::Left, Default::default());
        window.simulate_mouse_move(to, gpui::MouseButton::Left, Default::default());
        window.simulate_mouse_up(to, gpui::MouseButton::Left, Default::default());
        window.run_until_parked();
        host.read_with(window, |h, _| {
            assert!(h.right_sidebar_width > start);
            assert!(h.right_sidebar_resize.is_none());
        });
        assert_eq!(std::fs::read_to_string(path).unwrap(), "read only\n");
    }

    #[tokio::test]
    async fn streamed_paths_keep_verified_links_and_drop_changed_markdown_context() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
        let source = "查看 `a.rs`，继续说明。";
        let decorated = decorate_cached(source.into(), Some(dir.path().into()), Default::default()).await.unwrap();
        assert_eq!(decorated.replacements.len(), 1);
        let mut replacements = decorated.replacements.clone();
        let appended = format!("{source}\n\n新的总结 **结果**");
        retain_decorations(source, &appended, &mut replacements);
        assert_eq!(apply_decorations(&appended, &replacements), format!("{}\n\n新的总结 **结果**", decorated.markdown));
        let changed = "查看 `a.rsx`，继续说明。";
        retain_decorations(source, changed, &mut replacements);
        assert!(replacements.is_empty());
        let code = "a.rs";
        let decorated = decorate_cached(code.into(), Some(dir.path().into()), Default::default()).await.unwrap();
        let mut replacements = decorated.replacements;
        // Same byte range, but the line now belongs to a setext heading.
        // It is still a path candidate and can keep its link.
        retain_decorations(code, "a.rs\n----", &mut replacements);
        assert_eq!(replacements.len(), 1);
        retain_decorations(code, "a.rsx", &mut replacements);
        assert!(replacements.is_empty());
    }

    #[tokio::test]
    async fn path_cache_shares_lookups_within_its_ttl_and_after_invalidation() {
        let dir = tempfile::tempdir().unwrap();
        let root = Some(dir.path().to_path_buf());
        std::fs::write(dir.path().join("a.rs"), "a\n").unwrap();
        let cache = std::sync::Arc::new(PathCache::default());
        let decorate = |cache: std::sync::Arc<PathCache>| {
            let root = root.clone();
            async move {
                decorate_cached("`a.rs` 与 `src/../a.rs`".into(), root, cache)
                    .await
                    .unwrap()
            }
        };
        let first = decorate(cache.clone()).await;
        assert!(first.markdown.contains(PREFIX));
        assert_eq!(first.paths.len(), 1, "equivalent spellings share one key");
        std::fs::remove_file(dir.path().join("a.rs")).unwrap();
        // Inside the TTL the cached answer links without touching the disk again.
        assert!(decorate(cache.clone()).await.markdown.contains(PREFIX));
        cache.invalidate(&root, Some("a.rs"));
        assert!(!decorate(cache.clone()).await.markdown.contains(PREFIX));
    }

    #[tokio::test]
    async fn expired_entries_and_the_limit_keep_the_cache_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let root = Some(dir.path().to_path_buf());
        std::fs::write(dir.path().join("a.rs"), "a\n").unwrap();
        let expired = std::sync::Arc::new(PathCache::new(std::time::Duration::ZERO, 8));
        assert!(
            decorate_cached("`a.rs`".into(), root.clone(), expired.clone())
                .await
                .unwrap()
                .markdown
                .contains(PREFIX)
        );
        std::fs::remove_file(dir.path().join("a.rs")).unwrap();
        assert!(
            !decorate_cached("`a.rs`".into(), root.clone(), expired)
                .await
                .unwrap()
                .markdown
                .contains(PREFIX)
        );
        let bounded = std::sync::Arc::new(PathCache::new(PATH_CACHE_TTL, 2));
        for name in ["a", "b", "c", "d"] {
            std::fs::write(dir.path().join(format!("{name}.md")), "x\n").unwrap();
            let source = format!("`{name}.md`");
            assert!(
                decorate_cached(source, root.clone(), bounded.clone())
                    .await
                    .unwrap()
                    .markdown
                    .contains(PREFIX)
            );
        }
        assert!(bounded.values.lock().unwrap().len() <= 2);
    }

    #[tokio::test]
    async fn invalidation_is_scoped_to_one_root_or_path() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        for dir in [first.path(), second.path()] {
            std::fs::write(dir.join("one.rs"), "1\n").unwrap();
            std::fs::write(dir.join("two.rs"), "2\n").unwrap();
        }
        let roots = [
            Some(first.path().to_path_buf()),
            Some(second.path().to_path_buf()),
        ];
        let cache = std::sync::Arc::new(PathCache::default());
        for root in &roots {
            let decorated =
                decorate_cached("`one.rs` `two.rs`".into(), root.clone(), cache.clone())
                    .await
                    .unwrap();
            assert_eq!(decorated.paths.len(), 2);
        }
        assert_eq!(cache.values.lock().unwrap().len(), 4);
        std::fs::remove_file(first.path().join("one.rs")).unwrap();
        cache.invalidate(&roots[0], Some("one.rs"));
        assert_eq!(cache.values.lock().unwrap().len(), 3);
        assert!(
            !decorate_cached("`one.rs`".into(), roots[0].clone(), cache.clone())
                .await
                .unwrap()
                .markdown
                .contains(PREFIX)
        );
        cache.invalidate(&roots[0], None);
        assert_eq!(cache.values.lock().unwrap().len(), 2);
        assert!(
            decorate_cached("`one.rs`".into(), roots[1].clone(), cache.clone())
                .await
                .unwrap()
                .markdown
                .contains(PREFIX)
        );
    }
}
