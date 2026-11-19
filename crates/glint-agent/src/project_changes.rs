//! Cached Git state and per-path staging intents for the project Changes page.
use crate::workspace_diff::{self, Source, StageUpdate};
use gpui::Task;
use gpui_glint::components::{ChangeRow, ChangeStatus};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

pub(super) struct ChangeSnapshot {
    root: PathBuf,
    sources: HashMap<PathBuf, Source>,
    conflicts: Vec<String>,
    stamp: Option<workspace_diff::RepositoryStamp>,
}

impl ChangeSnapshot {
    pub fn read(root: &Path) -> Result<Self, String> {
        let mut inventory = workspace_diff::inventory(root)?;
        // The inventory never carries numstat counts, so read them for the whole list
        // once. Without them every row would look like a binary change.
        let labels: Vec<String> = inventory
            .sources
            .values()
            .flat_map(|source| {
                let mut paths = vec![source.label.clone()];
                if source.old_path != source.label {
                    paths.push(source.old_path.clone());
                }
                paths
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let counts = workspace_diff::scoped_statistics(&inventory.root, &labels);
        let known = counts.is_ok();
        let (staged_counts, unstaged_counts) = counts.unwrap_or_default();
        for source in inventory.sources.values_mut() {
            source.stats_ready = known;
            source.staged_stats = staged_counts.get(&source.label).copied();
            source.unstaged_stats = unstaged_counts.get(&source.label).copied();
        }
        Ok(Self {
            root: inventory.root,
            sources: inventory.sources,
            conflicts: inventory.conflict_labels,
            stamp: inventory.stamp,
        })
    }

    pub fn rows(&self) -> Vec<ChangeRow> {
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        let mut untracked = Vec::new();
        for source in self.sources.values() {
            let path = source.path.to_string_lossy().into_owned();
            if source.staged {
                staged.push((
                    source.label.clone(),
                    path.clone(),
                    source.layer_status(true),
                    source.layer_stats(true),
                    source.stats_ready && source.old_path == source.label,
                ));
            }
            if source.unstaged {
                let status = source.layer_status(false);
                let entry = (
                    source.label.clone(),
                    path,
                    status,
                    source.layer_stats(false),
                    source.stats_ready,
                );
                if status == workspace_diff::FileStatus::Untracked {
                    untracked.push(entry);
                } else {
                    unstaged.push(entry);
                }
            }
        }
        let groups = [
            ("已暂存", Some(true), staged),
            ("未暂存", Some(false), unstaged),
            ("未跟踪", None, untracked),
        ];
        let mut rows = Vec::new();
        let mut conflicts = self.conflicts.clone();
        if !conflicts.is_empty() {
            conflicts.sort();
            rows.push(ChangeRow::Group {
                title: "待解决冲突".into(),
                count: conflicts.len(),
                staged: None,
            });
            rows.extend(conflicts.into_iter().map(|label| ChangeRow::File {
                path: self.root.join(&label).to_string_lossy().into_owned(),
                label,
                depth: 0,
                status: Some(ChangeStatus::Conflict),
                staged: false,
                additions: None,
                deletions: None,
                binary: false,
            }));
        }
        for (title, layer, mut entries) in groups {
            if entries.is_empty() {
                continue;
            }
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            rows.push(ChangeRow::Group {
                title: title.to_owned(),
                count: entries.len(),
                staged: layer,
            });
            rows.extend(
                entries
                    .into_iter()
                    .map(|(label, path, status, stats, known)| {
                        ChangeRow::File {
                            path,
                            label,
                            depth: 0,
                            status: Some(status.into()),
                            staged: layer.unwrap_or(false),
                            additions: stats.map(|(added, _)| added),
                            deletions: stats.map(|(_, removed)| removed),
                            // Only trust "no numstat row" once the counts were actually read.
                            binary: known && layer.is_some() && stats.is_none(),
                        }
                    }),
            );
        }
        rows
    }

    fn apply(&mut self, update: StageUpdate) -> bool {
        let refresh = self.stamp != update.before_stamp
            || update
                .inventory
                .sources
                .values()
                .any(|source| !source.stats_ready);
        self.sources
            .retain(|_, source| !source.overlaps(&update.affected));
        self.sources.extend(update.inventory.sources);
        self.stamp = update.inventory.stamp;
        refresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_batches_same_direction_and_coalesces_clicks_before_and_during_sync() {
        let repo = workspace_diff::tests::many_files(3);
        let snapshot = ChangeSnapshot::read(repo.path()).unwrap();
        let paths = ["0000", "0001", "0002"].map(|name| {
            snapshot
                .root
                .join(format!("vendor/{name}.rs"))
                .to_string_lossy()
                .into_owned()
        });
        let mut changes = ProjectChanges {
            snapshot: Some(snapshot),
            ..Default::default()
        };
        changes.enqueue(StageSelection::Files(&paths), true);
        changes.enqueue(StageSelection::Files(&paths[..1]), false);
        assert!(!changes.intents().contains_key(&paths[0]));

        let (staged, files) = changes.take_batch().unwrap();
        assert!(staged);
        assert_eq!(files.len(), 2);
        changes.enqueue(StageSelection::Files(&paths[1..]), false);
        changes.enqueue(StageSelection::Files(&paths[2..]), true);
        assert_eq!(changes.intents()[&paths[1]], false);
        assert_eq!(changes.intents()[&paths[2]], true);

        changes.complete(workspace_diff::stage_and_refresh(&files, staged));
        let (staged, files) = changes.take_batch().unwrap();
        assert!(!staged);
        assert_eq!(files.len(), 1);
        changes.complete(workspace_diff::stage_and_refresh(&files, staged));
        assert!(!changes.busy());
        assert!(changes.error.is_none());
        let snapshot = workspace_diff::inventory(repo.path()).unwrap();
        assert!(!snapshot.sources[Path::new(&paths[0])].staged);
        assert!(!snapshot.sources[Path::new(&paths[1])].staged);
        assert!(snapshot.sources[Path::new(&paths[2])].staged);
    }
}

pub(super) enum StageSelection<'a> {
    Files(&'a [String]),
    Group(bool),
}

#[derive(Default)]
pub(super) struct ProjectChanges {
    pub snapshot: Option<ChangeSnapshot>,
    pending: BTreeMap<PathBuf, bool>,
    running: BTreeMap<PathBuf, bool>,
    pub debounce: Option<Task<()>>,
    pub task: Option<Task<()>>,
    pub refresh_pending: bool,
    pub error: Option<String>,
}

impl ProjectChanges {
    pub fn busy(&self) -> bool {
        !self.pending.is_empty() || !self.running.is_empty()
    }

    pub fn enqueue(&mut self, selection: StageSelection<'_>, staged: bool) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let paths: Vec<_> = match selection {
            StageSelection::Files(paths) => paths.iter().map(PathBuf::from).collect(),
            StageSelection::Group(layer) => snapshot
                .sources
                .values()
                .filter(|source| {
                    if layer {
                        source.staged
                    } else {
                        source.unstaged
                    }
                })
                .map(|source| source.path.clone())
                .collect(),
        };
        for path in paths {
            let Some(source) = snapshot.sources.get(&path) else {
                continue;
            };
            if let Some(in_flight) = self.running.get(&path) {
                if *in_flight == staged {
                    self.pending.remove(&path);
                } else {
                    self.pending.insert(path, staged);
                }
            } else if let Some(queued) = self.pending.get(&path) {
                if *queued != staged {
                    self.pending.remove(&path);
                }
            } else if if staged {
                source.unstaged
            } else {
                source.staged
            } {
                self.pending.insert(path, staged);
            }
        }
        if self.pending.is_empty() {
            self.debounce = None;
        }
    }

    pub fn intents(&self) -> HashMap<String, bool> {
        self.running
            .iter()
            .chain(self.pending.iter())
            .map(|(path, staged)| (path.to_string_lossy().into_owned(), *staged))
            .collect()
    }

    pub fn take_batch(&mut self) -> Option<(bool, Vec<Source>)> {
        let snapshot = self.snapshot.as_ref()?;
        self.pending
            .retain(|path, _| snapshot.sources.contains_key(path));
        let staged = *self.pending.first_key_value()?.1;
        let sources: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, target)| **target == staged)
            .map(|(path, _)| snapshot.sources[path].clone())
            .collect();
        for source in &sources {
            self.pending.remove(&source.path);
            self.running.insert(source.path.clone(), staged);
        }
        Some((staged, sources))
    }

    pub fn complete(&mut self, result: Result<StageUpdate, String>) {
        self.running.clear();
        match result {
            Ok(update) => {
                if let Some(snapshot) = self.snapshot.as_mut() {
                    self.refresh_pending |= snapshot.apply(update);
                }
            }
            Err(error) => {
                self.pending.clear();
                self.refresh_pending = true;
                self.error = Some(format!("暂存区同步失败：{error}"));
            }
        }
    }
}
