//! Same-directory atomic replacement, serialized per path inside this process.
use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};
pub fn digest(text: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
fn lock(path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if locks.len() > 1024 {
        locks.retain(|_, lock| lock.strong_count() > 0);
    }
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path.to_owned(), Arc::downgrade(&lock));
    lock
}
pub fn current(path: &Path) -> Result<Option<String>, String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
            Err("文件类型发生变化，未写入".into())
        }
        Ok(meta) if meta.len() > 2 * 1024 * 1024 => Err("文件超过 2MB，未写入".into()),
        Ok(_) => std::fs::read_to_string(path)
            .map(Some)
            .map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
pub fn replace(path: &Path, expected: Option<&str>, content: &str) -> Result<(), String> {
    let guard = lock(path);
    let _guard = guard.lock().unwrap_or_else(|e| e.into_inner());
    let parent = path.parent().ok_or("文件路径无效")?;
    let check = || -> Result<(), String> {
        if parent.canonicalize().map_err(|e| e.to_string())? != parent {
            return Err("父目录发生变化，未写入".into());
        }
        if current(path)?.as_deref() != expected {
            return Err("文件已被其他操作修改，请重新读取后重试".into());
        }
        Ok(())
    };
    check()?;
    let permissions = std::fs::metadata(path).ok().map(|m| m.permissions());
    if permissions.as_ref().is_some_and(|p| p.readonly()) {
        return Err("文件为只读，未写入".into());
    }
    let temp = parent.join(format!(".glint-write-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)
                .map_err(|e| e.to_string())?;
        }
        file.write_all(content.as_bytes())
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        check()?;
        std::fs::rename(&temp, path).map_err(|e| e.to_string())?;
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(temp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_expected_version_allows_one_commit_and_preserves_mode() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let path = root.join("file");
        std::fs::write(&path, "old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o750)).unwrap();
        }
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let jobs: Vec<_> = ["first", "second"]
            .into_iter()
            .map(|text| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    replace(&path, Some("old"), text)
                })
            })
            .collect();
        barrier.wait();
        let successes = jobs
            .into_iter()
            .filter_map(|h| h.join().unwrap().ok())
            .count();
        assert_eq!(successes, 1);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o750
            );
        }
    }
}
