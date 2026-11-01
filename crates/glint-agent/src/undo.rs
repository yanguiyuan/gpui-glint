//! Conflict-checked reversal of this session's recorded net file changes.
use crate::{edits::Change, ssh_project};
use std::{io::Write, path::PathBuf};
#[derive(Default)]
pub struct Outcome {
    pub restored: Vec<PathBuf>,
    pub error: Option<String>,
}
fn current(change: &Change) -> Result<Option<String>, String> {
    let parent = change.path.parent().ok_or("无效的文件路径")?;
    if parent.canonicalize().map_err(|e| e.to_string())? != parent {
        return Err("父目录路径发生变化，未撤销".into());
    }
    match std::fs::symlink_metadata(&change.path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
            Err("文件类型已变化，未撤销".into())
        }
        Ok(meta) if meta.len() > 2 * 1024 * 1024 => Err("文件内容已变化，未撤销".into()),
        Ok(_) => std::fs::read_to_string(&change.path)
            .map(Some)
            .map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
fn check(change: &Change) -> Result<bool, String> {
    let current = current(change)?;
    if current.as_deref() == change.before.as_deref() {
        return Ok(false);
    }
    if current.as_deref() != Some(change.after.as_ref()) {
        return Err(format!("{} 的内容已被其他操作修改，未覆盖", change.label));
    }
    Ok(true)
}
fn restore(change: &Change) -> Result<(), String> {
    if !check(change)? {
        return Ok(());
    }
    let Some(before) = change.before.as_deref() else {
        return std::fs::remove_file(&change.path).map_err(|e| e.to_string());
    };
    let temp = change
        .path
        .with_file_name(format!(".glint-undo-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        let permissions = std::fs::metadata(&change.path)
            .map_err(|e| e.to_string())?
            .permissions();
        file.set_permissions(permissions)
            .map_err(|e| e.to_string())?;
        file.write_all(before.as_bytes())
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        // Recheck after staging the replacement; preserve later external edits.
        if check(change)? {
            std::fs::rename(&temp, &change.path).map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&temp);
    result
}
pub async fn apply(files: Vec<Change>, root: Option<PathBuf>) -> Outcome {
    if files.iter().any(|c| ssh_project::is_remote(&c.path)) {
        return remote(files, root).await.unwrap_or_else(|error| Outcome {
            error: Some(error),
            ..Default::default()
        });
    }
    // A known conflict anywhere prevents starting a batch reversal.
    for change in &files {
        if let Err(error) = check(change) {
            return Outcome {
                error: Some(error),
                ..Default::default()
            };
        }
    }
    let mut result = Outcome::default();
    for change in files {
        match restore(&change) {
            Ok(()) => result.restored.push(change.path),
            Err(error) => {
                result.error = Some(format!("{}：{error}", change.label));
                break;
            }
        }
    }
    result
}
async fn remote(files: Vec<Change>, root: Option<PathBuf>) -> Result<Outcome, String> {
    let root = ssh_project::Remote::from_path(root.as_deref().ok_or("远程项目已不可用")?)?;
    let mut paths = std::collections::HashMap::new();
    let mut changes = Vec::new();
    for file in &files {
        let target = ssh_project::Remote::from_path(&file.path)?;
        if target.target != root.target || target.port != root.port {
            return Err("远程变更不属于当前 SSH 项目".into());
        }
        paths.insert(target.directory.clone(), file.path.clone());
        changes.push(
            serde_json::json!({"path":target.directory,"before":file.before,"after":file.after}),
        );
    }
    let value = root
        .action("undo_changes", serde_json::json!({"changes":changes}), true)
        .await?;
    Ok(Outcome {
        restored: value["restored"]
            .as_array()
            .ok_or("远程撤销响应无效")?
            .iter()
            .filter_map(|p| paths.get(p.as_str()?).cloned())
            .collect(),
        error: value["error"].as_str().map(str::to_owned),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn change(path: PathBuf, before: Option<&str>, after: &str) -> Change {
        serde_json::from_value(serde_json::json!({"id":uuid::Uuid::new_v4(),"path":path,"label":path.file_name().unwrap().to_string_lossy(),"before":before,"after":after,"added":1,"removed":1})).unwrap()
    }
    #[test]
    fn recorded_chain_detects_intermediate_external_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("source.rs");
        let make = |change: Change| crate::tools::ToolRecord {
            call: crate::tools::ToolCall {
                id: uuid::Uuid::new_v4().to_string(),
                name: "write_file".into(),
                arguments: serde_json::json!({}),
            },
            result: "ok".into(),
            success: true,
            change: Some(change),
            subagent: None,
            elapsed_ms: 0,
            live_output: None,
            nested_tools: Default::default(),
        };
        let first = make(change(path.clone(), Some("original"), "first edit"));
        let next = make(change(path.clone(), Some("external edit"), "second edit"));
        let paths = std::collections::HashSet::from([path]);
        assert!(
            crate::edits::verify_reversal_chain(vec![&first, &next], &paths, &Default::default())
                .is_err()
        );
        let excluded = std::collections::BTreeSet::from([first.change.as_ref().unwrap().id]);
        assert!(
            crate::edits::verify_reversal_chain(vec![&first, &next], &paths, &excluded).is_ok()
        );
    }
    #[tokio::test]
    async fn restores_existing_files_deletes_created_files_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let a = root.join("existing.rs");
        let b = root.join("created.rs");
        std::fs::write(&a, "after").unwrap();
        std::fs::write(&b, "new").unwrap();
        let files = vec![
            change(a.clone(), Some("before"), "after"),
            change(b.clone(), None, "new"),
        ];
        let result = apply(files.clone(), Some(root.clone())).await;
        assert!(result.error.is_none());
        assert_eq!(result.restored.len(), 2);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "before");
        assert!(!b.exists());
        assert_eq!(apply(files, Some(root)).await.restored.len(), 2);
    }
    #[tokio::test]
    async fn a_conflict_prevents_starting_the_batch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let a = root.join("a.rs");
        let b = root.join("b.rs");
        std::fs::write(&a, "after").unwrap();
        std::fs::write(&b, "external edit").unwrap();
        let result = apply(
            vec![
                change(a.clone(), Some("before"), "after"),
                change(b.clone(), Some("before"), "after"),
            ],
            Some(root),
        )
        .await;
        assert!(result.error.is_some());
        assert!(result.restored.is_empty());
        assert_eq!(std::fs::read_to_string(a).unwrap(), "after");
        assert_eq!(std::fs::read_to_string(b).unwrap(), "external edit");
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn preserves_permissions_and_rejects_symlink_substitution() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("run.sh");
        std::fs::write(&path, "after").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o751)).unwrap();
        assert!(
            apply(
                vec![change(path.clone(), Some("before"), "after")],
                Some(root.clone())
            )
            .await
            .error
            .is_none()
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o751
        );
        let alias = root.join("alias");
        symlink(&path, &alias).unwrap();
        assert!(
            apply(vec![change(alias, Some("wrong"), "before")], Some(root))
                .await
                .error
                .is_some()
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "before");
    }
}
