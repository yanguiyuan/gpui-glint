//! The example owns persistence; reusable editor components only emit changes.
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub fn library_path() -> io::Result<PathBuf> {
    if let Some(path) =
        std::env::var_os("GLINT_MARKDOWN_HOME").or_else(|| std::env::var_os("GLASS_MARKDOWN_HOME"))
    {
        return Ok(path.into());
    }
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"));
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")));
    let library = base.map(|base| default_library_path(&base));
    library.ok_or_else(|| io::Error::other("无法确定文档库位置，请设置 GLINT_MARKDOWN_HOME"))
}

fn default_library_path(base: &Path) -> PathBuf {
    let current = base.join("gpui-glint/mdeditor");
    if current.exists() {
        return current;
    }
    // Reuse existing libraries in place so persisted absolute document paths,
    // open tabs and recovery records remain valid after the brand rename.
    for legacy in ["gpui-glass/mdeditor", "gpui-glass/markdown-editor"] {
        let path = base.join(legacy);
        if path.is_dir() {
            return path;
        }
    }
    current
}

pub fn title(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

pub fn filename(name: &str) -> io::Result<String> {
    if name.chars().any(char::is_control) {
        return Err(io::Error::other("名称不能包含控制字符"));
    }
    let name = name.trim().trim_end_matches(".md").trim();
    if name.is_empty()
        || name.starts_with('.')
        || name
            .chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
    {
        return Err(io::Error::other(
            "名称不能为空，且不能包含路径分隔符或特殊字符",
        ));
    }
    Ok(format!("{name}.md"))
}

pub fn list(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        // Don't follow symlinks into files outside the managed library.
        if entry.file_type()?.is_file() && path.extension().is_some_and(|e| e == "md") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

pub fn create(root: &Path, name: &str, source: &str) -> io::Result<PathBuf> {
    use io::Write;
    let base = title(Path::new(&filename(name)?));
    for suffix in 0..10000 {
        let label = if suffix == 0 {
            base.clone()
        } else {
            format!("{base} {suffix}")
        };
        let path = root.join(filename(&label)?);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(source.as_bytes())?;
                file.sync_all()?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other("同名文档过多，请换一个名称"))
}

pub fn atomic_write(path: &Path, source: &str) -> io::Result<()> {
    use io::Write;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = path.with_extension(format!("{}.{}.tmp", std::process::id(), unique));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(source.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Reject a changed or missing file instead of silently overwriting external edits.
pub fn save(path: &Path, previous: &str, source: &str) -> io::Result<()> {
    if fs::read_to_string(path)? != previous {
        return Err(io::Error::other(
            "磁盘文件已被其他程序修改。请导出当前文档后重新打开，避免覆盖外部修改",
        ));
    }
    atomic_write(path, source)
}

fn recovery_path(path: &Path) -> PathBuf {
    path.parent()
        .unwrap()
        .join(".recovery")
        .join(path.file_name().unwrap())
        .with_extension("json")
}

pub fn write_recovery(path: &Path, saved: &str, source: &str) -> io::Result<()> {
    let recovery = recovery_path(path);
    fs::create_dir_all(recovery.parent().unwrap())?;
    atomic_write(
        &recovery,
        &serde_json::json!({"saved": saved, "source": source}).to_string(),
    )
}

pub fn read_recovery(path: &Path) -> io::Result<Option<(String, String)>> {
    let contents = match fs::read_to_string(recovery_path(path)) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let value: serde_json::Value = serde_json::from_str(&contents)?;
    match (value["saved"].as_str(), value["source"].as_str()) {
        (Some(saved), Some(source)) => Ok(Some((saved.to_owned(), source.to_owned()))),
        _ => Err(io::Error::other("恢复文件格式不正确")),
    }
}

pub fn clear_recovery(path: &Path) {
    let _ = fs::remove_file(recovery_path(path));
}

pub fn rename(path: &Path, name: &str) -> io::Result<PathBuf> {
    let target = path.with_file_name(filename(name)?);
    if path == target {
        return Ok(target);
    }
    // Hard-link creation refuses to overwrite an existing name.
    fs::hard_link(path, &target)?;
    if let Err(error) = fs::remove_file(path) {
        let _ = fs::remove_file(&target);
        return Err(error);
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    static NEXT_LIBRARY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    struct Library(PathBuf);
    impl Library {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "glint-markdown-test-{}-{}-{}",
                std::process::id(),
                NEXT_LIBRARY.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Library {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn brand_rename_reuses_existing_libraries_without_moving_documents() {
        let base = Library::new();
        let current = base.0.join("gpui-glint/mdeditor");
        assert_eq!(default_library_path(&base.0), current);
        assert!(!current.exists());

        let oldest = base.0.join("gpui-glass/markdown-editor");
        fs::create_dir_all(&oldest).unwrap();
        let document = create(&oldest, "现有文档", "保留内容").unwrap();
        assert_eq!(default_library_path(&base.0), oldest);
        assert_eq!(fs::read_to_string(&document).unwrap(), "保留内容");

        let legacy = base.0.join("gpui-glass/mdeditor");
        fs::create_dir_all(&legacy).unwrap();
        assert_eq!(default_library_path(&base.0), legacy);

        fs::create_dir_all(&current).unwrap();
        assert_eq!(default_library_path(&base.0), current);
        assert!(document.exists());
    }

    #[test]
    fn imports_do_not_overwrite_and_saves_detect_external_changes() {
        let root = Library::new();
        let first = create(&root.0, "设计", "# A").unwrap();
        let second = create(&root.0, "设计", "# B").unwrap();
        assert_ne!(first, second);
        save(&first, "# A", "# Edited").unwrap();
        fs::write(&first, "external").unwrap();
        assert!(save(&first, "# Edited", "mine").is_err());
        assert_eq!(fs::read_to_string(first).unwrap(), "external");
    }
    #[test]
    fn rename_refuses_collisions_and_roundtrips_unicode() {
        let root = Library::new();
        let first = create(&root.0, "设计", "内容").unwrap();
        create(&root.0, "存在", "保留").unwrap();
        assert!(rename(&first, "存在").is_err());
        let moved = rename(&first, "新名称").unwrap();
        assert!(!first.exists());
        assert_eq!(fs::read_to_string(moved).unwrap(), "内容");
        assert_eq!(list(&root.0).unwrap().len(), 2);
        for invalid in ["", "../escape", "a/b", "a\\b", ".hidden", "a\n"] {
            assert!(filename(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn recovery_preserves_unsaved_changes_without_overwriting_disk() {
        let root = Library::new();
        let path = create(&root.0, "恢复", "original").unwrap();
        write_recovery(&path, "original", "unsaved").unwrap();
        fs::write(&path, "external").unwrap();
        let (saved, source) = read_recovery(&path).unwrap().unwrap();
        assert_eq!(source, "unsaved");
        assert!(save(&path, &saved, &source).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "external");
        clear_recovery(&path);
        assert!(read_recovery(&path).unwrap().is_none());
    }
}
