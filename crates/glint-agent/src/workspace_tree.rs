//! Directory rows for the Git inventory; file indices retain the original diff identity.
use crate::edits::Change;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    #[default]
    List,
    Tree,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Directory {
        path: String,
        label: String,
        depth: usize,
        count: usize,
        collapsed: bool,
    },
    File {
        index: usize,
        depth: usize,
    },
}

#[derive(Default)]
struct Directory {
    directories: BTreeMap<String, Directory>,
    files: BTreeMap<String, usize>,
    count: usize,
}

pub fn rows(files: &[Change], collapsed: &HashSet<String>) -> Vec<Row> {
    let mut root = Directory::default();
    for (index, file) in files.iter().enumerate() {
        let mut parts = file.label.split('/').peekable();
        let mut directory = &mut root;
        directory.count += 1;
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                directory.files.insert(part.into(), index);
            } else {
                directory = directory.directories.entry(part.into()).or_default();
                directory.count += 1;
            }
        }
    }
    fn flatten(
        directory: &Directory,
        prefix: &str,
        depth: usize,
        collapsed: &HashSet<String>,
        rows: &mut Vec<Row>,
    ) {
        for (label, child) in &directory.directories {
            let path = if prefix.is_empty() {
                label.clone()
            } else {
                format!("{prefix}/{label}")
            };
            let is_collapsed = collapsed.contains(&path);
            rows.push(Row::Directory {
                path: path.clone(),
                label: label.clone(),
                depth,
                count: child.count,
                collapsed: is_collapsed,
            });
            if !is_collapsed {
                flatten(child, &path, depth + 1, collapsed, rows);
            }
        }
        rows.extend(directory.files.values().map(|index| Row::File {
            index: *index,
            depth,
        }));
    }
    let mut result = Vec::new();
    flatten(&root, "", 0, collapsed, &mut result);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(label: &str) -> Change {
        Change::preview(label.into(), label.into(), None, "".into(), Some((0, 0)))
    }

    #[test]
    fn workspace_tree_groups_sorts_and_collapses_without_losing_file_identity() {
        let files = [
            file("z.rs"),
            file("src/z.rs"),
            file("src/nested/a.rs"),
            file("src/a.rs"),
            file("测试/a.rs"),
        ];
        let all = rows(&files, &HashSet::new());
        assert_eq!(
            all,
            vec![
                Row::Directory {
                    path: "src".into(),
                    label: "src".into(),
                    depth: 0,
                    count: 3,
                    collapsed: false
                },
                Row::Directory {
                    path: "src/nested".into(),
                    label: "nested".into(),
                    depth: 1,
                    count: 1,
                    collapsed: false
                },
                Row::File { index: 2, depth: 2 },
                Row::File { index: 3, depth: 1 },
                Row::File { index: 1, depth: 1 },
                Row::Directory {
                    path: "测试".into(),
                    label: "测试".into(),
                    depth: 0,
                    count: 1,
                    collapsed: false
                },
                Row::File { index: 4, depth: 1 },
                Row::File { index: 0, depth: 0 },
            ]
        );
        let collapsed = rows(&files, &HashSet::from(["src".into(), "src/nested".into()]));
        assert_eq!(collapsed.len(), 4);
        assert!(matches!(
            &collapsed[0],
            Row::Directory {
                count: 3,
                collapsed: true,
                ..
            }
        ));
        let reopened = rows(&files, &HashSet::from(["src/nested".into()]));
        assert!(!reopened.contains(&Row::File { index: 2, depth: 2 }));
        assert!(reopened.contains(&Row::File { index: 3, depth: 1 }));
    }

    #[test]
    fn workspace_tree_display_defaults_for_existing_settings_and_round_trips() {
        let mut value = serde_json::to_value(crate::model::History::default()).unwrap();
        value.as_object_mut().unwrap().remove("workspace_display");
        let mut history: crate::model::History = serde_json::from_value(value).unwrap();
        assert!(history.workspace_display == DisplayMode::List);
        history.workspace_display = DisplayMode::Tree;
        let loaded: crate::model::History =
            serde_json::from_value(serde_json::to_value(history).unwrap()).unwrap();
        assert!(loaded.workspace_display == DisplayMode::Tree);
    }
}
