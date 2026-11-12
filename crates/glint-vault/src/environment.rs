//! Live vault metadata and credentials explicitly enabled for child processes.
use crate::store::Entry;
use std::sync::{Arc, Mutex};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

#[derive(Clone, serde::Serialize, Zeroize, ZeroizeOnDrop)]
pub struct EntrySummary {
    pub id: String,
    pub title: String,
    pub username: String,
    pub website: String,
    pub env_identifier: String,
    pub env_name: Option<String>,
    pub updated_at: String,
}

#[derive(Default)]
struct State {
    unlocked: bool,
    entries: Vec<EntrySummary>,
    variables: Zeroizing<Vec<(String, String)>>,
}

#[derive(Clone, Default)]
pub struct Environment(Arc<Mutex<State>>);

impl Environment {
    pub(crate) fn replace(&self, entries: &[Entry]) {
        let entries_summary = entries
            .iter()
            .map(|entry| EntrySummary {
                id: entry.id.clone(),
                title: entry.title.clone(),
                username: entry.username.clone(),
                website: entry.website.clone(),
                env_identifier: entry.env_identifier.clone(),
                env_name: (!entry.env_identifier.is_empty())
                    .then(|| format!("GLINT_{}_PWD", entry.env_identifier.to_ascii_uppercase())),
                updated_at: entry.updated_at.clone(),
            })
            .collect();
        let variables = Zeroizing::new(
            entries
                .iter()
                .filter(|entry| !entry.env_identifier.is_empty())
                .map(|entry| {
                    (
                        format!("GLINT_{}_PWD", entry.env_identifier.to_ascii_uppercase()),
                        entry.password.clone(),
                    )
                })
                .collect(),
        );
        *self.0.lock().unwrap() = State {
            unlocked: true,
            entries: entries_summary,
            variables,
        };
    }

    pub(crate) fn clear(&self) {
        *self.0.lock().unwrap() = State::default();
    }

    /// Metadata only; a locked vault is distinct from an unlocked, empty vault.
    pub fn entries(&self) -> Option<Vec<EntrySummary>> {
        let state = self.0.lock().unwrap();
        state.unlocked.then(|| state.entries.clone())
    }

    /// The caller should only retain this snapshot until the child has started.
    pub fn snapshot(&self) -> Zeroizing<Vec<(String, String)>> {
        self.0.lock().unwrap().variables.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_named_entries_are_exported_and_live_handles_observe_clear() {
        let environment = Environment::default();
        let consumer = environment.clone();
        let mut entry = Entry::default();
        entry.password = "p@ss\"word\n$HOME;=你好".into();
        environment.replace(&[entry.clone()]);
        assert!(consumer.snapshot().is_empty());
        entry.env_identifier = "WORK_MAIL".into();
        environment.replace(&[entry.clone()]);
        assert_eq!(
            consumer.snapshot().as_slice(),
            &[("GLINT_WORK_MAIL_PWD".into(), entry.password.clone())]
        );
        environment.clear();
        assert!(consumer.snapshot().is_empty());
    }

    #[test]
    fn metadata_excludes_secrets_and_distinguishes_empty_from_locked() {
        let environment = Environment::default();
        assert!(environment.entries().is_none());
        environment.replace(&[]);
        assert!(environment.entries().unwrap().is_empty());
        let mut entry = Entry::default();
        entry.title = "Work".into();
        entry.password = "never-return-this-password".into();
        entry.notes = "never-return-these-notes".into();
        environment.replace(&[entry.clone()]);
        let entries = environment.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].env_name.is_none());
        let json = serde_json::to_string(&entries).unwrap();
        assert!(!json.contains(&entry.password));
        assert!(!json.contains(&entry.notes));
        entry.env_identifier = "WORK".into();
        environment.replace(&[entry]);
        assert_eq!(
            environment.entries().unwrap()[0].env_name.as_deref(),
            Some("GLINT_WORK_PWD")
        );
        environment.clear();
        assert!(environment.entries().is_none());
    }
}
