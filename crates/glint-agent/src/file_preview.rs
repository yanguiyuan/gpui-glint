//! Explicit chat file links resolve on activation. File content loads in the background.
use super::*;
use base64::Engine as _;
use gpui_glint::components::{FilePreview, FilePreviewEntry, FilePreviewEvent};
use std::{collections::HashMap, path::Path};
const IMAGE_LIMIT: u64 = 20 * 1024 * 1024;
fn is_png(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
}
const LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub root: Option<PathBuf>,
    pub path: String,
    pub line: Option<usize>,
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
    } else if value.contains("://") || value.contains(['=', '|', '`']) {
        return None;
    }
    if value.is_empty() || value.starts_with('#') {
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

fn markdown_target(link: &str, root: &Option<PathBuf>) -> Option<Target> {
    // Leave web schemes and fragment-only links to their own handlers.
    // A colon can also be part of a Windows drive or a path:line destination.
    if link.starts_with('#') || link.starts_with("//") {
        return None;
    }
    if let Some((prefix, rest)) = link.split_once(':')
        && prefix.starts_with(|c: char| c.is_ascii_alphabetic())
        && prefix
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"+-.".contains(&c))
        && !matches!(prefix, "file" | "ssh")
        && !rest.starts_with(|c: char| c.is_ascii_digit())
        && !(cfg!(windows) && prefix.len() == 1 && rest.starts_with(['/', '\\']))
    {
        return None;
    }
    target(link, root)
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
pub(super) enum Loaded {
    File(String),
    Image(Vec<u8>),
    Directory(Vec<FilePreviewEntry>, bool),
}
type PreviewData = (String, Option<String>, Loaded);
type NavigationData = (String, Vec<FilePreviewEntry>, bool);

async fn load_with_navigation(
    target: Target,
    known_directory: Option<String>,
) -> Result<(PreviewData, Option<NavigationData>), String> {
    let root = target.root.clone();
    let result = load(target).await?;
    let navigation = if !matches!(result.2, Loaded::Directory(..)) {
        if let Some(parent) = result
            .1
            .clone()
            .filter(|parent| Some(parent) != known_directory.as_ref())
        {
            match load(Target {
                root,
                path: parent,
                line: None,
            })
            .await
            {
                Ok((path, _, Loaded::Directory(entries, truncated))) => {
                    Some((path, entries, truncated))
                }
                _ => None,
            }
        } else {
            None
        }
    } else {
        None
    };
    Ok((result, navigation))
}

pub(super) async fn load(target: Target) -> Result<(String, Option<String>, Loaded), String> {
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
                this.file_preview
                    .view
                    .update(cx, |view, cx| view.cancel_directory_requests(cx));
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
                let Some(mut target) = this.file_preview.target.clone() else {
                    return;
                };
                target.path = path.clone();
                target.line = None;
                let request = *request;
                let path = path.clone();
                let work = cx.background_executor().spawn(async move {
                    let result = if target.root.as_deref().is_some_and(ssh_project::is_remote) {
                        provider::Job::start(load(target))?.result().await
                    } else {
                        load(target).await
                    };
                    match result? {
                        (_, _, Loaded::Directory(entries, truncated)) => Ok((entries, truncated)),
                        _ => Err("路径不是目录".into()),
                    }
                });
                let task = cx.spawn(async move |weak, cx| {
                    let result = work.await;
                    let _ = weak.update(cx, |this, cx| {
                        if let Some(task) = this.file_preview.directory_tasks.remove(&request) {
                            task.detach();
                        }
                        this.file_preview.view.update(cx, |view, cx| {
                            view.set_directory_children(path, request, result, cx)
                        });
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
            target: None,
            task: None,
            revision: 0,
            directory_tasks: Default::default(),
            hover_task: None,
        }
    }
}
impl AgentWorkspace {
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
        if let Some(location) = link
            .strip_prefix("glint-paste://")
            .or_else(|| link.strip_prefix("glint-reference://"))
        {
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
            if !reference.text_attachment {
                if let Some(mut target) = target(&reference.path, origin) {
                    target.line = reference.start_line;
                    self.open_file_preview(target, cx);
                }
                return;
            }
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
        if link.starts_with("https://") || link.starts_with("http://") {
            cx.open_url(link);
        } else if let Some(target) = markdown_target(link, origin) {
            self.open_file_preview(target, cx);
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
    pub(super) fn open_file_preview(&mut self, target: Target, cx: &mut Context<Self>) {
        self.open_file_preview_mode(target, false, cx);
    }
    pub(super) fn switch_preview_repository(&mut self, cx: &mut Context<Self>) {
        if self.page == crate::Page::Projects {
            return;
        }
        let expanded = self.file_preview.expanded;
        self.file_preview.directory_tasks.clear();
        self.open_project_directory(cx);
        self.file_preview.expanded = expanded;
    }

    pub(super) fn open_project_directory(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.panel_repository_root(crate::panel_repositories::Panel::Preview)
        else {
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
        if !self.terminal_ui.bottom {
            self.terminal_ui.open = false;
        }
        self.file_preview.open = true;
        self.edits_ui.open = false;
        self.file_preview.revision += 1;
        let revision = self.file_preview.revision;
        self.file_preview.target = Some(target.clone());
        self.file_preview.view.update(cx, |view, cx| {
            view.set_theme(self.theme, cx);
            view.set_loading(target.path.clone(), cx);
        });
        let known_directory = self
            .file_preview
            .view
            .read(cx)
            .navigation_path()
            .map(str::to_owned);
        self.file_preview.task = Some(cx.spawn(async move |weak, cx| {
            let line = target.line;
            let work = cx.background_executor().spawn(async move {
                if target.root.as_deref().is_some_and(ssh_project::is_remote) {
                    provider::Job::start(load_with_navigation(target, known_directory))?
                        .result()
                        .await
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
                        this.file_preview.view.update(cx, |view, cx| {
                            view.set_navigation(path, entries, truncated, cx)
                        });
                    }
                    result
                });
                if let Ok((path, _, _)) = &result {
                    if let Some(root) = this.adopt_panel_repository(
                        crate::panel_repositories::Panel::Preview,
                        path,
                        cx,
                    ) {
                        if let Some(target) = &mut this.file_preview.target {
                            target.root = Some(root);
                        }
                    }
                }
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
        let accessory = self
            .repository_select(crate::panel_repositories::Panel::Preview, cx)
            .map(gpui::AnyView::from);
        self.file_preview.view.update(cx, |view, cx| {
            view.set_standalone(false, cx);
            view.set_header_accessory(accessory, cx);
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
    fn streamed_markdown_keeps_explicit_file_links_and_opens_only_on_activation(
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
        host.update(window, |host, cx| {
            let root = Some(dir.path().to_path_buf());
            host.history.active_mut().project = root.clone();
            let text = "正文 main.rs、`src/main.rs` 和 [入口（第 42 行）](src/main.rs#L42)";
            let editor = host.markdown_text("answer".into(), text, cx);
            let updated = format!("{text}。");
            let same = host.markdown_text("answer".into(), &updated, cx);
            assert_eq!(editor.entity_id(), same.entity_id());
            assert_eq!(editor.read(cx).markdown(), updated);
            assert!(!host.file_preview.open);
            assert!(host.file_preview.task.is_none());

            editor.update(cx, |_, cx| {
                cx.emit(gpui_glint::components::DocumentEvent::LinkActivated(
                    "src/main.rs#L42".into(),
                ));
            });
        });
        host.update(window, |host, _| {
            assert!(host.file_preview.open);
            assert_eq!(
                host.file_preview.target,
                Some(Target {
                    root: Some(dir.path().into()),
                    path: dir
                        .path()
                        .join("src/main.rs")
                        .to_string_lossy()
                        .into_owned(),
                    line: Some(42),
                })
            );
        });
    }
    #[gpui::test]
    fn draft_reference_links_open_their_saved_text_or_file(cx: &mut gpui::TestAppContext) {
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

            let root = Some(dir.path().to_path_buf());
            host.history.active_mut().project = root.clone();
            let mut reference =
                gpui_glint::components::ComposerReference::selection("src/main.rs", 7, 9, "...");
            reference.token = reference.label();
            let submission = ComposerSubmission {
                text: format!("{} @main.rs:7-99", reference.token).into(),
                references: vec![reference],
                images: vec![],
            };
            let display = model::MessageDisplay::from_submission(&submission).unwrap();
            let link = format!("glint-reference://{id}/0/0");
            assert_eq!(
                display.markdown(id, 0),
                format!("[@main.rs:7-9]({link}) @main.rs:7-99")
            );
            host.history.active_mut().messages[0].display = Some(display);
            host.preview_link(&link, &root, cx);
            let target = host.file_preview.target.as_ref().unwrap();
            assert_eq!(Path::new(&target.path), dir.path().join("src/main.rs"));
            assert_eq!(target.line, Some(7));
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
    async fn local_preview_handles_text_directories_binary_and_removed_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("folder")).unwrap();
        std::fs::write(dir.path().join("a.txt"), "alpha\nβeta\n").unwrap();
        std::fs::write(dir.path().join("b.bin"), [0, 1, 2]).unwrap();
        let root = Some(dir.path().to_path_buf());
        let t = target("a.txt#L2", &root).unwrap();
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
    }
    #[test]
    fn references_do_not_automatically_connect_to_other_ssh_hosts() {
        let root = Some(PathBuf::from("ssh://user@example.com/home/user/project"));
        assert!(markdown_target("ssh://other@example.org/etc/passwd", &root).is_none());
        assert!(markdown_target("ssh://user@example.com/etc/hosts#L4", &root).is_some());
        for link in ["https://example.com/path.rs", "mailto:hello@host", "#L42"] {
            assert!(markdown_target(link, &root).is_none());
        }
        assert_eq!(
            markdown_target("src/main.rs:42:5", &root).unwrap().line,
            Some(42)
        );
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
    fn preview_tree_expands_nested_folders_and_keeps_the_selected_file_when_collapsed(
        cx: &mut gpui::TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        std::fs::write(
            root.join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        std::fs::create_dir_all(root.join("src/nested")).unwrap();
        std::fs::write(root.join("src/nested/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("README.md"), "# Project").unwrap();
        let (host, window) =
            cx.add_window_view(|_, cx| AgentWorkspace::with_path(root.join("sessions.json"), cx));
        window.simulate_resize(size(px(1440.), px(900.)));
        host.update(window, |host, cx| {
            host.storage_readable = false;
            host.file_preview.expanded = true;
            host.open_file_preview(
                Target {
                    root: Some(root.clone()),
                    path: root.to_string_lossy().into_owned(),
                    line: None,
                },
                cx,
            );
        });
        window.run_until_parked();
        let folder =
            Box::leak(format!("preview-entry:{}", root.join("src").display()).into_boxed_str());
        let nested = Box::leak(
            format!("preview-entry:{}", root.join("src/nested").display()).into_boxed_str(),
        );
        let file = Box::leak(
            format!(
                "preview-entry:{}",
                root.join("src/nested/main.rs").display()
            )
            .into_boxed_str(),
        );
        assert!(window.debug_bounds(nested).is_none());
        let row = window.debug_bounds(folder).unwrap();
        window.simulate_click(row.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert_eq!(
                host.file_preview.target.as_ref().unwrap().path,
                root.to_string_lossy()
            )
        });
        let child = window.debug_bounds(nested).expect("expanded child folder");
        assert!(child.left() >= row.left());
        window.simulate_click(child.center(), Default::default());
        window.run_until_parked();
        let file_row = window.debug_bounds(file).expect("grandchild file");
        window.simulate_click(file_row.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert!(host.file_preview.expanded);
            assert_eq!(
                host.file_preview.target.as_ref().unwrap().path,
                root.join("src/nested/main.rs").to_string_lossy()
            );
        });
        assert!(window.debug_bounds("file-preview-navigation").is_some());
        let row = window.debug_bounds(folder).unwrap();
        window.simulate_click(row.center(), Default::default());
        window.run_until_parked();
        assert!(window.debug_bounds(file).is_none());
        host.read_with(window, |host, _| {
            assert_eq!(
                host.file_preview.target.as_ref().unwrap().path,
                root.join("src/nested/main.rs").to_string_lossy()
            )
        });
        let row = window.debug_bounds(folder).unwrap();
        window.simulate_click(row.center(), Default::default());
        window.run_until_parked();
        assert!(
            window.debug_bounds(file).is_some(),
            "nested expansion state must survive collapsing an ancestor"
        );
        host.read_with(window, |host, _| {
            assert!(host.file_preview.directory_tasks.is_empty())
        });
    }

    #[gpui::test]
    fn expanded_preview_keeps_directory_and_switches_files_without_collapsing(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        cx.update(crate::initialize);
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        std::fs::write(
            root.join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("a.rs"),
            "fn a() {}
",
        )
        .unwrap();
        std::fs::write(
            root.join("b.rs"),
            "fn b() {}
",
        )
        .unwrap();
        let (host, window) =
            cx.add_window_view(|_, cx| AgentWorkspace::with_path(root.join("sessions.json"), cx));
        window.simulate_resize(size(px(1440.), px(900.)));
        host.update(window, |host, cx| {
            host.storage_readable = false;
            host.file_preview.expanded = true;
            host.history.active_mut().project = Some(root.clone());
            host.open_file_preview(
                Target {
                    root: Some(root.clone()),
                    path: root.join("a.rs").to_string_lossy().into_owned(),
                    line: None,
                },
                cx,
            );
        });
        window.run_until_parked();
        let navigation = window
            .debug_bounds("file-preview-navigation")
            .expect("expanded preview keeps its folder");
        assert!(navigation.size.width > px(100.));
        let selector =
            Box::leak(format!("preview-entry:{}", root.join("b.rs").display()).into_boxed_str());
        let next = window
            .debug_bounds(selector)
            .expect("sibling file in navigation");
        assert!(next.right() <= navigation.right());
        window.simulate_click(next.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert!(host.file_preview.expanded);
            assert_eq!(
                host.file_preview.target.as_ref().unwrap().path,
                root.join("b.rs").to_string_lossy()
            );
        });
        assert!(window.debug_bounds("file-preview-navigation").is_some());
        window.update(|window, cx| host.update(cx, |host, cx| window.focus(&host.focus, cx)));
        window.simulate_keystrokes("ctrl-`");
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert!(host.file_preview.open && host.file_preview.expanded);
            assert!(host.terminal_ui.open && host.terminal_ui.bottom);
            assert_eq!(
                host.history.terminal_default_position,
                crate::terminal_ui::Position::Right
            );
            assert_eq!(
                host.file_preview.target.as_ref().unwrap().path,
                root.join("b.rs").to_string_lossy()
            );
        });
        let terminal = window.debug_bounds("project-terminal-sidebar").unwrap();
        let navigation = window.debug_bounds("file-preview-navigation").unwrap();
        assert!(
            navigation.bottom() <= terminal.top() + px(1.),
            "preview must fit above the bottom terminal"
        );
        let first_selector =
            Box::leak(format!("preview-entry:{}", root.join("a.rs").display()).into_boxed_str());
        let first = window.debug_bounds(first_selector).unwrap();
        window.simulate_click(first.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert!(host.terminal_ui.open && host.file_preview.expanded);
            assert_eq!(
                host.file_preview.target.as_ref().unwrap().path,
                root.join("a.rs").to_string_lossy()
            );
        });
        let hide = window.debug_bounds("project-terminal-hide").unwrap();
        window.simulate_click(hide.center(), Default::default());
        window.run_until_parked();
        host.read_with(window, |host, _| {
            assert!(!host.terminal_ui.open && host.file_preview.open && host.file_preview.expanded)
        });
        assert!(
            window
                .debug_bounds("file-preview-navigation")
                .unwrap()
                .size
                .height
                > navigation.size.height
        );
        let collapse = window.debug_bounds("file-preview-expand").unwrap();
        window.simulate_click(collapse.center(), Default::default());
        window.run_until_parked();
        assert!(window.debug_bounds("file-preview-navigation").is_none());
        assert!(window.debug_bounds("agent-right-sidebar-resize").is_some());
        assert_eq!(
            std::fs::read_to_string(root.join("b.rs")).unwrap(),
            "fn b() {}
"
        );
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
}
