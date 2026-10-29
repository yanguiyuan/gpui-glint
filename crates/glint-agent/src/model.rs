use chrono::{DateTime, Local};
use gpui_glint::components::PastedImage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io, path::Path, sync::Arc};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub enum Role {
    User,
    Agent,
    Notice,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub thinking_ms: std::collections::BTreeMap<usize, u64>,
    #[serde(default)]
    pub blocks: Vec<crate::generation::ContentBlock>,
    #[serde(default)]
    pub tools: Vec<crate::tools::ToolRecord>,
    #[serde(default)]
    pub stats: Option<crate::generation::GenerationStats>,
    #[serde(default)]
    pub images: Vec<MessageImage>,
    #[serde(default)]
    pub memory_recall: Option<crate::memory::Recall>,
    pub role: Role,
    pub text: String,
    pub time: DateTime<Local>,
}

/// An image attached to a user message, persisted as base64 so the provider
/// keeps seeing it in later turns.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageImage {
    pub media_type: String,
    #[serde(default, skip_serializing_if = "str::is_empty")]
    pub data: Arc<str>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct DraftImageFile {
    media_type: String,
    file: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todos: Vec<crate::todo::Item>,
    #[serde(default)]
    pub project: Option<std::path::PathBuf>,
    pub id: Uuid,
    pub title: String,
    pub updated: DateTime<Local>,
    pub messages: Vec<Message>,
    pub draft: String,
    #[serde(skip)]
    pub draft_images: Vec<PastedImage>,
    #[serde(default)]
    pub context: Option<crate::context_usage::ContextSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    draft_image_files: Vec<DraftImageFile>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            todos: Vec::new(),
            project: None,
            id: Uuid::new_v4(),
            title: "新会话".into(),
            updated: Local::now(),
            messages: vec![],
            draft: String::new(),
            draft_images: Vec::new(),
            context: None,
            draft_image_files: Vec::new(),
        }
    }

    pub fn push(&mut self, role: Role, text: String) {
        self.push_message(role, text, vec![]);
    }

    pub fn push_message(&mut self, role: Role, text: String, images: Vec<MessageImage>) {
        if self.messages.is_empty() && role == Role::User {
            let subject = if text.trim().is_empty() && !images.is_empty() {
                "[图片]".to_string()
            } else {
                text.clone()
            };
            self.title = subject.split_whitespace().collect::<Vec<_>>().join(" ");
            if self.title.chars().count() > 24 {
                self.title = self.title.chars().take(24).collect::<String>() + "…";
            }
        }
        self.updated = Local::now();
        self.messages.push(Message {
            thinking_ms: Default::default(),
            blocks: vec![],
            tools: vec![],
            stats: None,
            memory_recall: None,
            images,
            role,
            text,
            time: self.updated,
        });
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct History {
    #[serde(default)]
    pub access_mode: crate::tools::AccessMode,
    #[serde(default)]
    pub projects: Vec<std::path::PathBuf>,
    #[serde(default)]
    pub group_by_project: bool,
    pub sessions: Vec<Session>,
    pub active: Uuid,
}

impl Default for History {
    fn default() -> Self {
        let session = Session::new();
        Self {
            access_mode: crate::tools::AccessMode::default(),
            projects: vec![],
            group_by_project: true,
            active: session.id,
            sessions: vec![session],
        }
    }
}

impl History {
    pub fn load(path: &Path) -> io::Result<Self> {
        let stored = crate::database::load_history(path).map_err(io::Error::other)?;
        let importing = stored.is_none();
        let mut history: Self = if let Some(value) = stored {
            serde_json::from_value(value)?
        } else {
            match std::fs::read(path) {
                Ok(bytes) => serde_json::from_slice(&bytes)?,
                Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
                Err(error) => return Err(error),
            }
        };
        if history.sessions.is_empty() {
            let session = Session::new();
            history.active = session.id;
            history.sessions.push(session);
        }
        if !history.sessions.iter().any(|s| s.id == history.active) {
            history.active = history.sessions[0].id;
        }
        for session in &history.sessions {
            if let Some(project) = &session.project {
                if !history.projects.contains(project) {
                    history.projects.push(project.clone());
                }
            }
        }
        if let Some(project) = history.projects.first().cloned() {
            for session in &mut history.sessions {
                if session.project.is_none()
                    && (!session.messages.is_empty()
                        || !session.draft.is_empty()
                        || !session.draft_image_files.is_empty())
                {
                    session.project = Some(project.clone());
                }
            }
        }
        let directory = path.with_extension("images");
        for session in &mut history.sessions {
            for image in session
                .messages
                .iter_mut()
                .flat_map(|message| &mut message.images)
            {
                if image.data.is_empty() {
                    if let Some(file) = &image.file {
                        let bytes = read_image_file(&directory, file)?;
                        image.data = String::from_utf8(bytes)
                            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
                            .into();
                    }
                }
            }
            for image in &session.draft_image_files {
                let format = gpui::ImageFormat::from_mime_type(&image.media_type)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "未知图片格式"))?;
                session.draft_images.push(PastedImage::new(
                    format,
                    read_image_file(&directory, &image.file)?,
                ));
            }
        }
        if importing {
            history.save(path)?;
        }
        Ok(history)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Snapshots share image bytes; all file work runs on the save worker.
        let directory = path.with_extension("images");
        let mut snapshot = self.clone();
        for session in &mut snapshot.sessions {
            for image in session
                .messages
                .iter_mut()
                .flat_map(|message| &mut message.images)
            {
                image.file = Some(write_image_file(&directory, image.data.as_bytes(), "b64")?);
                image.data = Arc::from("");
            }
            session.draft_image_files = session
                .draft_images
                .iter()
                .map(|image| {
                    Ok(DraftImageFile {
                        media_type: image.media_type().into(),
                        file: write_image_file(&directory, image.bytes(), "bin")?,
                    })
                })
                .collect::<io::Result<_>>()?;
        }
        crate::database::save_history(path, &snapshot).map_err(io::Error::other)
    }

    pub fn active(&self) -> &Session {
        self.sessions.iter().find(|s| s.id == self.active).unwrap()
    }

    pub fn active_mut(&mut self) -> &mut Session {
        self.sessions
            .iter_mut()
            .find(|s| s.id == self.active)
            .unwrap()
    }

    #[cfg(test)]
    pub fn new_session(&mut self) {
        self.new_session_for(None);
    }

    pub fn new_session_for(&mut self, project: Option<std::path::PathBuf>) {
        if let Some(project) = &project {
            if !self.projects.contains(project) {
                self.projects.push(project.clone());
            }
        }
        // Reuse an empty conversation rather than accumulate blank records.
        if let Some(session) = self.sessions.iter().find(|s| {
            s.messages.is_empty()
                && s.draft.is_empty()
                && s.draft_images.is_empty()
                && s.project == project
        }) {
            self.active = session.id;
        } else {
            let mut session = Session::new();
            session.project = project;
            self.active = session.id;
            self.sessions.insert(0, session);
        }
    }

    /// Deletes a session; when it was active the view falls back to the newest
    /// remaining one, and deleting the last session yields a fresh blank one.
    pub fn remove(&mut self, id: Uuid) -> bool {
        let Some(position) = self.sessions.iter().position(|session| session.id == id) else {
            return false;
        };
        self.sessions.remove(position);
        if self.active == id {
            if let Some(newest) = self.sessions.first() {
                self.active = newest.id;
            } else {
                let session = Session::new();
                self.active = session.id;
                self.sessions.push(session);
            }
        }
        true
    }
}

fn write_image_file(directory: &Path, bytes: &[u8], extension: &str) -> io::Result<String> {
    let name = format!("{:x}.{extension}", Sha256::digest(bytes));
    std::fs::create_dir_all(directory)?;
    let path = directory.join(&name);
    if !path.exists() {
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(temporary, path)?;
    }
    Ok(name)
}

fn read_image_file(directory: &Path, name: &str) -> io::Result<Vec<u8>> {
    let valid = name.rsplit_once('.').is_some_and(|(hash, extension)| {
        hash.len() == 64
            && hash.chars().all(|ch| ch.is_ascii_hexdigit())
            && matches!(extension, "bin" | "b64")
    });
    if !valid {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "图片文件路径无效",
        ));
    }
    std::fs::read(directory.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_a_history_with_no_sessions_keeps_its_projects() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sessions.json");
        let mut history = History::default();
        history.projects.push("/project/empty".into());
        history.sessions.clear();
        history.save(&path).unwrap();
        let loaded = History::load(&path).unwrap();
        assert_eq!(
            loaded.projects,
            vec![std::path::PathBuf::from("/project/empty")]
        );
        assert!(loaded.active().messages.is_empty());
    }

    #[test]
    fn image_files_and_drafts_survive_restart_without_inline_base64() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sessions.json");
        let mut history = History::default();
        let first = history.active;
        history.active_mut().draft = "draft A".into();
        history.active_mut().draft_images =
            vec![PastedImage::new(gpui::ImageFormat::Png, vec![1, 2, 3])];
        history.new_session_for(Some("/project/b".into()));
        assert_ne!(history.active, first);
        assert!(history.active().draft_images.is_empty());
        history.active_mut().push_message(
            Role::User,
            "B".into(),
            vec![MessageImage {
                media_type: "image/png".into(),
                data: "aGVsbG8=".into(),
                file: None,
            }],
        );
        history.save(&path).unwrap();
        let stored = crate::database::load_history(&path)
            .unwrap()
            .unwrap()
            .to_string();
        assert!(!stored.contains("aGVsbG8="));
        let loaded = History::load(&path).unwrap();
        assert_eq!(&*loaded.active().messages[0].images[0].data, "aGVsbG8=");
        let draft = loaded
            .sessions
            .iter()
            .find(|session| session.id == first)
            .unwrap();
        assert_eq!(draft.draft, "draft A");
        assert_eq!(draft.draft_images[0].bytes(), &[1, 2, 3]);
        let image_count = std::fs::read_dir(path.with_extension("images"))
            .unwrap()
            .count();
        loaded.save(&path).unwrap();
        assert_eq!(
            std::fs::read_dir(path.with_extension("images"))
                .unwrap()
                .count(),
            image_count
        );
    }

    #[test]
    fn legacy_inline_images_migrate_and_old_standalone_sessions_keep_messages() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sessions.json");
        let mut history = History::default();
        history.projects.push("/project/a".into());
        history.active_mut().push_message(
            Role::User,
            "old message".into(),
            vec![MessageImage {
                media_type: "image/png".into(),
                data: "aGVsbG8=".into(),
                file: None,
            }],
        );
        std::fs::write(&path, serde_json::to_vec(&history).unwrap()).unwrap();
        let loaded = History::load(&path).unwrap();
        assert_eq!(
            loaded.active().project.as_deref(),
            Some(Path::new("/project/a"))
        );
        assert_eq!(loaded.active().messages[0].text, "old message");
        loaded.save(&path).unwrap();
        assert!(
            !crate::database::load_history(&path)
                .unwrap()
                .unwrap()
                .to_string()
                .contains("aGVsbG8=")
        );
        assert!(std::fs::read_to_string(&path).unwrap().contains("aGVsbG8="));
    }

    #[test]
    fn history_round_trip_preserves_active_messages_and_multiline_draft() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/sessions.json");
        let mut history = History::load(&path).unwrap();
        let first = history.active;
        history
            .active_mut()
            .push(Role::User, "检查中文布局 🦀".into());
        history.active_mut().push(Role::Agent, "测试回复".into());
        history
            .active_mut()
            .messages
            .last_mut()
            .unwrap()
            .thinking_ms
            .insert(0, 12345);
        history.active_mut().messages.last_mut().unwrap().blocks = vec![
            crate::generation::ContentBlock::Thinking("plan".into()),
            crate::generation::ContentBlock::Text("**测试回复**".into()),
        ];
        history.new_session();
        history.active_mut().draft = "第一行\n第二行".into();
        history.access_mode = crate::tools::AccessMode::AutoApprove;
        history.save(&path).unwrap();
        let loaded = History::load(&path).unwrap();
        assert_eq!(
            loaded
                .sessions
                .iter()
                .find(|session| session.id == first)
                .unwrap()
                .messages[1]
                .thinking_ms
                .get(&0),
            Some(&12345)
        );
        let blocks = &loaded
            .sessions
            .iter()
            .find(|session| session.id == first)
            .unwrap()
            .messages[1]
            .blocks;
        assert!(
            matches!(&blocks[..], [crate::generation::ContentBlock::Thinking(plan), crate::generation::ContentBlock::Text(text)] if plan == "plan" && text == "**测试回复**")
        );
        assert_eq!(loaded.access_mode, crate::tools::AccessMode::AutoApprove);
        assert_eq!(loaded.active, history.active);
        assert_eq!(loaded.active().draft, "第一行\n第二行");
        assert_eq!(
            loaded
                .sessions
                .iter()
                .find(|s| s.id == first)
                .unwrap()
                .messages
                .len(),
            2
        );
    }

    #[test]
    fn empty_sessions_are_reused_and_titles_are_unicode_safe() {
        let mut history = History::default();
        history.new_session();
        assert_eq!(history.sessions.len(), 1);
        history.active_mut().push(Role::User, "界".repeat(30));
        assert_eq!(history.active().title.chars().count(), 25);
        history.new_session();
        assert_eq!(history.sessions.len(), 2);
    }

    #[test]
    fn removing_sessions_falls_back_to_newest_and_recreates_the_last_one() {
        let mut history = History::default();
        let first = history.active;
        history.active_mut().push(Role::User, "有内容".into());
        history.new_session();
        let second = history.active;
        assert_ne!(first, second);
        // Removing the active session falls back to the remaining one.
        assert!(history.remove(second));
        assert_eq!(history.active, first);
        assert!(!history.remove(second));
        // Deleting the last session leaves a fresh blank one.
        assert!(history.remove(first));
        assert_eq!(history.sessions.len(), 1);
        assert_eq!(history.sessions[0].messages.len(), 0);
        assert_eq!(history.active, history.sessions[0].id);
    }

    #[test]
    fn corrupt_history_is_reported_without_overwriting_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sessions.json");
        std::fs::write(&path, "broken").unwrap();
        assert!(History::load(&path).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "broken");
    }

    #[test]
    fn images_round_trip_and_title_falls_back_for_image_only_prompts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sessions.json");
        let mut history = History::load(&path).unwrap();
        history.active_mut().push_message(
            Role::User,
            String::new(),
            vec![MessageImage {
                media_type: "image/png".into(),
                data: "aGVsbG8=".into(),
                file: None,
            }],
        );
        assert_eq!(history.active().title, "[图片]");
        history.save(&path).unwrap();
        let loaded = History::load(&path).unwrap();
        let images = &loaded.active().messages[0].images;
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].media_type, "image/png");
        assert_eq!(&*images[0].data, "aGVsbG8=");
    }
}
