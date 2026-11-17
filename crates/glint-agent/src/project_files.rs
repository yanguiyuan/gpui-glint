//! Project-page layout owns a persistent explorer and independently loaded documents.
use super::*;
use crate::file_preview::{Loaded, Target};
use gpui_glint::components::{FilePreview, FilePreviewEvent};

struct Document {
    id: u64,
    path: String,
    view: Entity<FilePreview>,
    _task: Task<()>,
}

pub(super) struct ProjectFiles {
    root: Option<PathBuf>,
    tree: Entity<FilePreview>,
    documents: Vec<Document>,
    active: Option<usize>,
    task: Option<Task<()>>,
    directories: std::collections::HashMap<u64, Task<()>>,
    theme: ThemePreset,
    visible: bool,
    next_document_id: u64,
}

async fn read(target: Target) -> Result<(String, Option<String>, Loaded), String> {
    if target.root.as_deref().is_some_and(ssh_project::is_remote) {
        provider::Job::start(file_preview::load(target))?
            .result()
            .await
    } else {
        file_preview::load(target).await
    }
}

impl ProjectFiles {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let tree = cx.new(FilePreview::new);
        cx.subscribe(&tree, |this, _, event: &FilePreviewEvent, cx| match event {
            FilePreviewEvent::OpenRequested(path) => this.open(path.clone(), cx),
            FilePreviewEvent::DirectoryRequested { path, request } => {
                this.expand(path.clone(), *request, cx)
            }
            _ => {}
        })
        .detach();
        Self {
            root: None,
            tree,
            documents: vec![],
            active: None,
            task: None,
            directories: Default::default(),
            theme: ThemePreset::Moonstone,
            visible: true,
            next_document_id: 0,
        }
    }

    pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        if self.root.as_ref() == Some(&root) {
            return;
        }
        self.root = Some(root);
        self.documents.clear();
        self.active = None;
        self.refresh(cx);
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        self.directories.clear();
        let path = if ssh_project::is_remote(&root) {
            match ssh_project::Remote::from_path(&root) {
                Ok(remote) => remote.directory,
                Err(_) => return,
            }
        } else {
            root.to_string_lossy().into_owned()
        };
        self.tree.update(cx, |view, cx| {
            view.cancel_directory_requests(cx);
            view.set_theme(self.theme, cx);
            view.set_loading(path.clone(), cx);
        });
        let target = Target {
            root: Some(root.clone()),
            path,
            line: None,
        };
        let work = cx.background_executor().spawn(read(target));
        self.task = Some(cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |this, cx| {
                if this.root.as_ref() != Some(&root) {
                    return;
                }
                if let Some(task) = this.task.take() {
                    task.detach();
                }
                this.tree.update(cx, |view, cx| match result {
                    Ok((path, parent, Loaded::Directory(entries, _))) => {
                        view.set_directory(path, parent, entries, cx)
                    }
                    Err(error) => view.set_error(error, cx),
                    _ => view.set_error("项目目录不可用".into(), cx),
                });
                if let Some(index) = this.active {
                    this.tree.update(cx, |view, cx| {
                        view.set_selected_path(this.documents[index].path.clone(), cx)
                    });
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn expand(&mut self, path: String, request: u64, cx: &mut Context<Self>) {
        let target = Target {
            root: self.root.clone(),
            path: path.clone(),
            line: None,
        };
        let work = cx.background_executor().spawn(read(target));
        let task = cx.spawn(async move |weak, cx| {
            let result = work.await.and_then(|(_, _, data)| match data {
                Loaded::Directory(entries, truncated) => Ok((entries, truncated)),
                _ => Err("目录不可用".into()),
            });
            let _ = weak.update(cx, |this, cx| {
                if let Some(task) = this.directories.remove(&request) {
                    task.detach();
                }
                this.tree.update(cx, |view, cx| {
                    view.set_directory_children(path, request, result, cx)
                });
            });
        });
        self.directories.insert(request, task);
    }

    fn open(&mut self, path: String, cx: &mut Context<Self>) {
        self.open_at(path, None, cx);
    }

    pub(super) fn open_search_result(
        &mut self,
        path: String,
        line: Option<usize>,
        query: String,
        cx: &mut Context<Self>,
    ) {
        self.open_at(path, line, cx);
        if let Some(index) = self.active {
            self.documents[index]
                .view
                .update(cx, |view, cx| view.set_document_search(query, line, cx));
        }
    }

    pub(super) fn open_at(&mut self, path: String, line: Option<usize>, cx: &mut Context<Self>) {
        if let Some(index) = self
            .documents
            .iter()
            .position(|document| document.path == path)
        {
            self.activate(index, cx);
            if let Some(line) = line {
                self.documents[index]
                    .view
                    .update(cx, |view, cx| view.reveal_line(line, cx));
            }
            return;
        }
        let view = cx.new(FilePreview::new);
        cx.observe(&view, |_, _, cx| cx.notify()).detach();
        view.update(cx, |view, cx| {
            view.set_theme(self.theme, cx);
            view.set_loading(path.clone(), cx);
            view.set_standalone(true, cx);
            view.set_content_only(true, cx);
            view.set_expanded(true, cx);
        });
        let target = Target {
            root: self.root.clone(),
            path: path.clone(),
            line: None,
        };
        let work = cx.background_executor().spawn(read(target));
        let document_view = view.downgrade();
        let task = cx.spawn(async move |_, cx| {
            let result = work.await;
            let _ = document_view.update(cx, |view, cx| match result {
                Ok((path, parent, Loaded::File(content))) => {
                    let language = std::path::Path::new(&path)
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("text")
                        .to_owned();
                    view.set_file(path, parent, language, content, cx);
                    if let Some(line) = line {
                        view.reveal_line(line, cx);
                    }
                }
                Ok((path, parent, Loaded::Image(bytes))) => view.set_image(
                    path,
                    parent,
                    std::sync::Arc::new(gpui::Image::from_bytes(gpui::ImageFormat::Png, bytes)),
                    cx,
                ),
                Err(error) => view.set_error(error, cx),
                _ => view.set_error("请从目录树选择文件".into(), cx),
            });
        });
        self.next_document_id += 1;
        self.documents.push(Document {
            id: self.next_document_id,
            path,
            view,
            _task: task,
        });
        self.activate(self.documents.len() - 1, cx);
    }

    fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        self.active = Some(index);
        self.tree.update(cx, |view, cx| {
            view.set_selected_path(self.documents[index].path.clone(), cx)
        });
        for (i, document) in self.documents.iter().enumerate() {
            document.view.update(cx, |view, cx| {
                view.set_active(self.visible && i == index, cx)
            });
        }
        cx.notify();
    }

    fn close(&mut self, index: usize, cx: &mut Context<Self>) {
        self.documents[index]
            .view
            .update(cx, |view, cx| view.set_active(false, cx));
        self.documents.remove(index);
        self.active = self.active.and_then(|active| {
            if self.documents.is_empty() {
                None
            } else if active > index {
                Some(active - 1)
            } else {
                Some(active.min(self.documents.len() - 1))
            }
        });
        if let Some(index) = self.active {
            self.activate(index, cx);
        }
        cx.notify();
    }

    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        for (index, document) in self.documents.iter().enumerate() {
            document.view.update(cx, |view, cx| {
                view.set_active(visible && self.active == Some(index), cx)
            });
        }
    }

    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme == theme {
            return;
        }
        self.theme = theme;
        self.tree.update(cx, |view, cx| view.set_theme(theme, cx));
        for document in &self.documents {
            document
                .view
                .update(cx, |view, cx| view.set_theme(theme, cx));
        }
        cx.notify();
    }
}

impl Render for ProjectFiles {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.theme.palette();
        self.tree.update(cx, |view, cx| {
            view.set_standalone(true, cx);
            view.set_expanded(true, cx);
        });
        let weak_select = cx.entity().downgrade();
        let weak_close = cx.entity().downgrade();
        let tabs =
            gpui_glint::components::ClosableTabs::new(self.documents.iter().map(|document| {
                let label = std::path::Path::new(&document.path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                gpui_glint::components::ClosableTab::new(document.id, label).icon(Icon::Text)
            }))
            .selected(self.active.map(|index| self.documents[index].id))
            .theme(self.theme)
            .fill_height()
            .content_padding(0.)
            .on_select(move |id, _, cx| {
                let _ = weak_select.update(cx, |this, cx| {
                    if let Some(index) =
                        this.documents.iter().position(|document| document.id == id)
                    {
                        this.activate(index, cx);
                    }
                });
            })
            .on_close(move |id, _, cx| {
                let _ = weak_close.update(cx, |this, cx| {
                    if let Some(index) =
                        this.documents.iter().position(|document| document.id == id)
                    {
                        this.close(index, cx);
                    }
                });
            });
        let document_pane = if let Some(index) = self.active {
            let document = &self.documents[index];
            let controls = document
                .view
                .update(cx, |view, cx| view.document_controls(cx));
            tabs.trailing(controls)
                .content(document.id, document.view.clone())
                .render()
        } else {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(p.text_muted))
                .child("从左侧目录树打开文件")
        };
        div()
            .size_full()
            .flex()
            .min_h_0()
            .min_w_0()
            .child(
                div()
                    .w(px(260.))
                    .h_full()
                    .flex_shrink_0()
                    .border_r_1()
                    .border_color(rgb(p.border))
                    .child(self.tree.clone()),
            )
            .child(div().flex_1().min_w_0().h_full().child(document_pane))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn tabs_reuse_document_views_close_neighbors_and_keep_explorer(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let first = root.join("first.rs").to_string_lossy().into_owned();
        let second = root.join("second.rs").to_string_lossy().into_owned();
        std::fs::write(&first, "fn first() {}\n").unwrap();
        std::fs::write(&second, "fn second() {}\n").unwrap();
        let (files, window) = cx.add_window_view(|_, cx| ProjectFiles::new(cx));
        files.update(window, |files, cx| files.set_root(root.clone(), cx));
        window.run_until_parked();
        files.update(window, |files, cx| {
            let tree = files.tree.entity_id();
            files.open(first.clone(), cx);
            let first_view = files.documents[0].view.entity_id();
            files.open(second.clone(), cx);
            assert_eq!(files.active, Some(1));
            files.open(first.clone(), cx);
            assert_eq!(files.documents.len(), 2);
            assert_eq!(files.active, Some(0));
            assert_eq!(files.documents[0].view.entity_id(), first_view);
            files.close(0, cx);
            assert_eq!(files.active, Some(0));
            assert_eq!(files.documents[0].path, second);
            files.close(0, cx);
            assert!(files.active.is_none());
            assert!(files.documents.is_empty());
            assert_eq!(files.tree.entity_id(), tree);
            files.open(first.clone(), cx);
            files.set_root(root.join("another-project"), cx);
            assert!(files.documents.is_empty());
            assert!(files.active.is_none());
        });
    }
}
