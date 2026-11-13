//! A batched Git inventory. File contents and Rust diffs are loaded only on disclosure.
use crate::edits::Change;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(test)]
thread_local! { static GIT_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[derive(Clone, Debug)]
pub struct Source {
    root: PathBuf,
    pub path: PathBuf,
    pub label: String,
    old_path: String,
    head: Option<String>,
    pub stats: Option<(usize, usize)>,
    pub status: FileStatus,
    untracked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus { Added, Modified, Deleted, Renamed, Untracked }

impl FileStatus {
    fn from_status(status: &[u8]) -> Self {
        if status == b"??" { Self::Untracked }
        else if status.contains(&b'D') { Self::Deleted }
        else if status.contains(&b'R') { Self::Renamed }
        else if status.contains(&b'A') || status.contains(&b'C') { Self::Added }
        else { Self::Modified }
    }
}

pub struct Inventory {
    pub root: PathBuf,
    pub head: Option<String>,
    pub files: Vec<Change>,
    pub sources: HashMap<PathBuf, Source>,
    pub has_conflicts: bool,
}

fn git(root: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    #[cfg(test)]
    GIT_CALLS.with(|count| count.set(count.get() + 1));
    crate::child_process::command("git")
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(args)
        .output()
        .map_err(|e| e.to_string())
}
fn checked(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = git(root, args)?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

pub fn inventory(root: &Path) -> Result<Inventory, String> {
    let top = checked(root, &["rev-parse", "--show-toplevel"])?;
    let root = PathBuf::from(
        std::str::from_utf8(&top)
            .map_err(|_| "Git 路径不是 UTF-8")?
            .trim_end_matches(['\r', '\n']),
    )
    .canonicalize()
    .map_err(|e| e.to_string())?;
    let status = checked(
        &root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let head = git(&root, &["rev-parse", "--verify", "HEAD"])?;
    let head = if head.status.success() {
        Some(String::from_utf8_lossy(&head.stdout).trim().to_owned())
    } else {
        None
    };
    let mut result = Inventory {
        root: root.clone(),
        head: head.clone(),
        files: Vec::new(),
        sources: HashMap::new(),
        has_conflicts: false,
    };
    let mut fields = status.split(|b| *b == 0).filter(|f| !f.is_empty());
    while let Some(field) = fields.next() {
        if field.len() < 4 {
            return Err("Git 状态格式无效".into());
        }
        let label = std::str::from_utf8(&field[3..]).map_err(|_| "文件名不是 UTF-8")?;
        let renamed = field[..2].iter().any(|b| *b == b'R' || *b == b'C');
        let old_path = if renamed {
            std::str::from_utf8(fields.next().ok_or("重命名记录不完整")?)
                .map_err(|_| "文件名不是 UTF-8")?
        } else {
            label
        };
        if field[..2].contains(&b'U') || field[..2] == *b"AA" || field[..2] == *b"DD" {
            result.has_conflicts = true;
            continue;
        }
        let stats = None;
        let path = root.join(label);
        // Empty placeholders deliberately never compute a diff or retain file content.
        result.files.push(Change::preview(
            path.clone(),
            label.into(),
            None,
            Arc::from(""),
            Some(stats.unwrap_or_default()),
        ));
        result.sources.insert(
            path.clone(),
            Source {
                root: root.clone(),
                path,
                label: label.into(),
                old_path: old_path.into(),
                head: head.clone(),
                stats,
                status: FileStatus::from_status(&field[..2]),
                untracked: field[..2] == *b"??",
            },
        );
    }
    Ok(result)
}

pub fn statistics(
    root: &Path,
    head: Option<&str>,
) -> Result<HashMap<String, (usize, usize)>, String> {
    let mut args = vec![
        "-c",
        "core.bigFileThreshold=2m",
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        "--numstat",
        "-z",
    ];
    if let Some(head) = head {
        args.push(head);
    } else {
        args.push("--cached");
    }
    args.push("--");
    let stats = checked(&root, &args)?;
    let mut counts = HashMap::new();
    for record in stats.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let text = std::str::from_utf8(record).map_err(|_| "Git 文件名不是 UTF-8")?;
        let mut fields = text.splitn(3, '\t');
        let added = fields.next().and_then(|s| s.parse::<usize>().ok());
        let removed = fields.next().and_then(|s| s.parse::<usize>().ok());
        let path = fields.next().ok_or("Git 统计格式无效")?;
        if let (Some(added), Some(removed)) = (added, removed) {
            counts.insert(path.to_owned(), (added, removed));
        }
    }
    Ok(counts)
}

pub fn apply_statistics(
    files: &mut [Change],
    sources: &mut HashMap<PathBuf, Source>,
    counts: &HashMap<String, (usize, usize)>,
) {
    for file in files {
        if let Some(source) = sources.get_mut(&file.path) {
            source.stats = if source.old_path == source.label {
                counts.get(&source.label).copied()
            } else {
                None
            };
            if let Some((added, removed)) = source.stats {
                file.added = added;
                file.removed = removed;
            }
        }
    }
}

pub fn load(source: &Source) -> Result<Change, String> {
    let after = match std::fs::symlink_metadata(&source.path) {
        Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
            return Err("链接和子模块不支持文本预览".into());
        }
        Ok(meta) if meta.len() > 2 * 1024 * 1024 => {
            return Err(format!(
                "大文件（{:.1} MiB），为保持流畅已跳过全文 Diff",
                meta.len() as f64 / (1024. * 1024.)
            ));
        }
        Ok(_) => {
            let path = source.path.canonicalize().map_err(|e| e.to_string())?;
            if !path.starts_with(&source.root) {
                return Err("文件位于仓库外，未读取".into());
            }
            crate::atomic_file::current(&path)?.ok_or("文件已删除，请刷新后重试")?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    let before = if source.untracked || source.head.is_none() {
        None
    } else {
        let spec = format!("{}:{}", source.head.as_deref().unwrap(), source.old_path);
        let size = git(&source.root, &["cat-file", "-s", &spec])?;
        if size.status.success() {
            let size: usize = String::from_utf8_lossy(&size.stdout)
                .trim()
                .parse()
                .map_err(|_| "Git 文件大小无效")?;
            if size > 2 * 1024 * 1024 {
                return Err("HEAD 版本超过 2 MiB，未加载全文 Diff".into());
            }
            let bytes = checked(&source.root, &["cat-file", "blob", &spec])?;
            Some(String::from_utf8(bytes).map_err(|_| "HEAD 文件不是 UTF-8 文本")?)
        } else {
            Some(String::new())
        }
    };
    if after.contains('\0') || before.as_deref().is_some_and(|s| s.contains('\0')) {
        return Err("二进制文件不支持文本 Diff".into());
    }
    Ok(Change::preview(
        source.path.clone(),
        source.label.clone(),
        before.map(Arc::from),
        Arc::from(after),
        None,
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn git_ok(root: &Path, args: &[&str]) {
        let out = crate::child_process::command("git")
            .current_dir(root)
            .args([
                "-c",
                "user.name=Workspace Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=disabled-hooks",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    pub(crate) fn many_files(count: usize) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_ok(dir.path(), &["init", "-b", "main"]);
        git_ok(dir.path(), &["config", "core.autocrlf", "false"]);
        std::fs::create_dir(dir.path().join("vendor")).unwrap();
        for i in 0..count {
            std::fs::write(
                dir.path().join(format!("vendor/{i:04}.rs")),
                "fn before() {}\n",
            )
            .unwrap();
        }
        git_ok(dir.path(), &["add", "."]);
        git_ok(dir.path(), &["commit", "-m", "baseline"]);
        for i in 0..count {
            std::fs::write(
                dir.path().join(format!("vendor/{i:04}.rs")),
                format!("fn before() {{}}\n// changed {i}\n"),
            )
            .unwrap();
        }
        dir
    }

    #[test]
    fn thousands_of_files_load_metadata_with_three_git_processes_and_no_content_snapshots() {
        let dir = many_files(3089);
        let calls = GIT_CALLS.with(|count| count.get());
        let started = std::time::Instant::now();
        let mut data = inventory(dir.path()).unwrap();
        let listing_time = started.elapsed();
        assert_eq!(GIT_CALLS.with(|count| count.get()) - calls, 3);
        assert_eq!(data.files.len(), 3089);
        assert!(
            data.files
                .iter()
                .all(|file| file.before.is_none() && file.after.is_empty())
        );
        assert!(data.sources.values().all(|source| source.stats.is_none()));
        let ids: Vec<_> = data.files.iter().map(|file| file.id).collect();
        let started = std::time::Instant::now();
        let counts = statistics(&data.root, data.head.as_deref()).unwrap();
        let statistics_time = started.elapsed();
        apply_statistics(&mut data.files, &mut data.sources, &counts);
        assert_eq!(GIT_CALLS.with(|count| count.get()) - calls, 4);
        assert!(
            data.files
                .iter()
                .all(|file| file.added == 1 && file.removed == 0 && file.after.is_empty())
        );
        assert_eq!(
            data.files.iter().map(|file| file.id).collect::<Vec<_>>(),
            ids
        );
        let file = load(&data.sources[&data.files[2500].path]).unwrap();
        assert_eq!((file.added, file.removed), (1, 0));
        assert!(file.after.contains("// changed 2500"));
        eprintln!(
            "3089 tracked changes: metadata={listing_time:?}, deferred statistics={statistics_time:?}, snapshots before disclosure=0 bytes"
        );
    }

    #[test]
    fn lazy_reads_preserve_staged_rename_delete_unborn_and_binary_semantics() {
        let dir = many_files(1);
        assert_eq!(inventory(dir.path()).unwrap().sources.values().next().unwrap().status, FileStatus::Modified);
        git_ok(dir.path(), &["mv", "vendor/0000.rs", "renamed 中文 [1].rs"]);
        let mut data = inventory(dir.path()).unwrap();
        let counts = statistics(&data.root, data.head.as_deref()).unwrap();
        apply_statistics(&mut data.files, &mut data.sources, &counts);
        let file = &data.files[0];
        assert_eq!(data.sources[&file.path].status, FileStatus::Renamed);
        assert_eq!(file.label, "renamed 中文 [1].rs");
        assert!(data.sources[&file.path].stats.is_none());
        assert_eq!(load(&data.sources[&file.path]).unwrap().added, 1);
        git_ok(dir.path(), &["add", "."]);
        git_ok(dir.path(), &["commit", "-m", "rename"]);
        std::fs::remove_file(dir.path().join("renamed 中文 [1].rs")).unwrap();
        let data = inventory(dir.path()).unwrap();
        assert_eq!(data.sources[&data.files[0].path].status, FileStatus::Deleted);
        assert_eq!(load(&data.sources[&data.files[0].path]).unwrap().removed, 2);

        let unborn = tempfile::tempdir().unwrap();
        git_ok(unborn.path(), &["init"]);
        std::fs::write(unborn.path().join("first.txt"), "first\n").unwrap();
        let data = inventory(unborn.path()).unwrap();
        assert_eq!(data.sources[&data.files[0].path].status, FileStatus::Untracked);
        assert_eq!(load(&data.sources[&data.files[0].path]).unwrap().added, 1);
        git_ok(unborn.path(), &["add", "first.txt"]);
        assert_eq!(inventory(unborn.path()).unwrap().sources.values().next().unwrap().status, FileStatus::Added);
        std::fs::write(unborn.path().join("first.txt"), [0, 1, 2]).unwrap();
        assert!(
            load(&data.sources[&data.files[0].path])
                .unwrap_err()
                .contains("二进制")
        );
    }

    #[test]
    fn large_untracked_files_are_listed_without_reading_or_diffing_their_contents() {
        let dir = tempfile::tempdir().unwrap();
        git_ok(dir.path(), &["init"]);
        std::fs::File::create(dir.path().join("large.txt"))
            .unwrap()
            .set_len(8 * 1024 * 1024)
            .unwrap();
        let data = inventory(dir.path()).unwrap();
        assert_eq!(data.files.len(), 1);
        assert!(data.files[0].after.is_empty());
        assert!(
            load(&data.sources[&data.files[0].path])
                .unwrap_err()
                .contains("大文件")
        );
    }
}
