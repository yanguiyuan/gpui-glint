use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::{io, path::Path};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub enum Role {
    User,
    Agent,
    Notice,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub blocks: Vec<crate::generation::ContentBlock>,
    #[serde(default)]
    pub tools: Vec<crate::tools::ToolRecord>,
    #[serde(default)]
    pub stats: Option<crate::generation::GenerationStats>,
    pub role: Role,
    pub text: String,
    pub time: DateTime<Local>,
}

#[derive(Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub project: Option<std::path::PathBuf>,
    pub id: Uuid,
    pub title: String,
    pub updated: DateTime<Local>,
    pub messages: Vec<Message>,
    pub draft: String,
}

impl Session {
    pub fn new() -> Self {
        Self {
            project: None,
            id: Uuid::new_v4(),
            title: "新会话".into(),
            updated: Local::now(),
            messages: vec![],
            draft: String::new(),
        }
    }

    pub fn push(&mut self, role: Role, text: String) {
        if self.messages.is_empty() && role == Role::User {
            self.title = text.split_whitespace().collect::<Vec<_>>().join(" ");
            if self.title.chars().count() > 24 {
                self.title = self.title.chars().take(24).collect::<String>() + "…";
            }
        }
        self.updated = Local::now();
        self.messages.push(Message {
            blocks: vec![],
            tools: vec![],
            stats: None,
            role,
            text,
            time: self.updated,
        });
    }
}

#[derive(Serialize, Deserialize)]
pub struct History {
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
            projects: vec![],
            group_by_project: true,
            active: session.id,
            sessions: vec![session],
        }
    }
}

impl History {
    pub fn load(path: &Path) -> io::Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e),
        };
        let mut history: Self = serde_json::from_slice(&bytes)?;
        if history.sessions.is_empty() {
            return Ok(Self::default());
        }
        if !history.sessions.iter().any(|s| s.id == history.active) {
            history.active = history.sessions[0].id;
        }
        Ok(history)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Replace only after a complete write, so an interrupted write preserves history.
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(temporary, path)
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
        // Reuse an empty conversation rather than accumulate blank records.
        if let Some(session) = self
            .sessions
            .iter()
            .find(|s| s.messages.is_empty() && s.draft.is_empty() && s.project == project)
        {
            self.active = session.id;
        } else {
            let mut session = Session::new();
            session.project = project;
            self.active = session.id;
            self.sessions.insert(0, session);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        history.active_mut().messages.last_mut().unwrap().blocks = vec![
            crate::generation::ContentBlock::Thinking("plan".into()),
            crate::generation::ContentBlock::Text("**测试回复**".into()),
        ];
        history.new_session();
        history.active_mut().draft = "第一行\n第二行".into();
        history.save(&path).unwrap();
        let loaded = History::load(&path).unwrap();
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
    fn corrupt_history_is_reported_without_overwriting_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sessions.json");
        std::fs::write(&path, "broken").unwrap();
        assert!(History::load(&path).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "broken");
    }
}
