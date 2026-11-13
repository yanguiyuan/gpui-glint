//! A batched Git inventory. File contents and Rust diffs are loaded only on disclosure.
use crate::edits::Change;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const EAGER_STATS_LIMIT: usize = 512;

#[cfg(test)]
thread_local! { static GIT_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStamp {
    modified: Option<std::time::SystemTime>,
    created: Option<std::time::SystemTime>,
    len: u64,
    kind: (bool, bool),
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    Some(FileStamp { modified: meta.modified().ok(), created: meta.created().ok(), len: meta.len(), kind: (meta.is_file(), meta.file_type().is_symlink()) })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryStamp(Vec<(PathBuf, Option<FileStamp>)>);

/// Index/HEAD detection is a few metadata reads, never a periodic git status.
/// A linked worktree has its own index/HEAD and a shared refs directory.
pub fn repository_stamp(root: &Path) -> Option<RepositoryStamp> {
    fn small_text(path: &Path) -> Option<String> {
        use std::io::Read;
        let mut text = String::new();
        std::fs::File::open(path).ok()?.take(8192).read_to_string(&mut text).ok()?;
        Some(text)
    }
    let dot_git = root.join(".git");
    let git_dir = if dot_git.is_dir() { dot_git.clone() } else {
        let text = small_text(&dot_git)?;
        let target = text.trim().strip_prefix("gitdir:")?.trim();
        root.join(target)
    };
    let common = small_text(&git_dir.join("commondir")).map(|path| git_dir.join(path.trim())).unwrap_or_else(|| git_dir.clone());
    let mut paths = vec![dot_git, git_dir.join("index"), git_dir.join("HEAD"), common.join("packed-refs"), git_dir.join("MERGE_HEAD"), git_dir.join("rebase-merge"), git_dir.join("rebase-apply")];
    if let Some(head) = small_text(&git_dir.join("HEAD")) {
        if let Some(reference) = head.trim().strip_prefix("ref: ") { paths.push(common.join(reference)); }
    }
    Some(RepositoryStamp(paths.into_iter().map(|path| { let stamp = file_stamp(&path); (path, stamp) }).collect()))
}

#[derive(Clone, Debug)]
pub struct Source {
    root: PathBuf,
    version: (Vec<u8>, Option<FileStamp>),
    pub path: PathBuf,
    pub label: String,
    old_path: String,
    head: Option<String>,
    pub stats: Option<(usize, usize)>,
    pub stats_ready: bool,
    pub status: FileStatus,
    untracked: bool,
    staged_status: FileStatus,
    unstaged_status: FileStatus,
    pub staged: bool,
    pub unstaged: bool,
    pub layer: Option<bool>,
    pub staged_stats: Option<(usize, usize)>,
    pub unstaged_stats: Option<(usize, usize)>,
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
    pub conflict_versions: Vec<(Vec<u8>, Option<FileStamp>)>,
    pub stamp: Option<RepositoryStamp>,
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
    let stamp = repository_stamp(&root);
    let status = checked(
        &root,
        &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
    )?;
    let head = git(&root, &["rev-parse", "--verify", "HEAD"])?;
    let head = if head.status.success() {
        Some(String::from_utf8_lossy(&head.stdout).trim().to_owned())
    } else {
        None
    };
    let mut inventory = parse_inventory(root, head, &status)?;
    inventory.stamp = stamp;
    Ok(inventory)
}

fn parse_inventory(root: PathBuf, head: Option<String>, status: &[u8]) -> Result<Inventory, String> {
    let mut result = Inventory {
        root: root.clone(),
        head: head.clone(),
        files: Vec::new(),
        sources: HashMap::new(),
        has_conflicts: false,
        conflict_versions: Vec::new(),
        stamp: None,
    };
    let mut fields = status.split(|b| *b == 0).filter(|f| !f.is_empty());
    while let Some(field) = fields.next() {
        let (status, label, old_path) = match field.first() {
            Some(b'#' | b'!') => continue,
            Some(b'?') => (b"??".as_slice(), std::str::from_utf8(field.get(2..).ok_or("Git 状态格式无效")?).map_err(|_| "文件名不是 UTF-8")?, None),
            Some(b'1' | b'2' | b'u') => {
                let count = match field[0] { b'1' => 9, b'2' => 10, _ => 11 };
                let parts: Vec<_> = field.splitn(count, |byte| *byte == b' ').collect();
                if parts.len() != count || parts[1].len() != 2 { return Err("Git 状态格式无效".into()); }
                let label = std::str::from_utf8(parts[count - 1]).map_err(|_| "文件名不是 UTF-8")?;
                let old = if field[0] == b'2' { Some(std::str::from_utf8(fields.next().ok_or("重命名记录不完整")?).map_err(|_| "文件名不是 UTF-8")?) } else { None };
                if field[0] == b'u' {
                    result.has_conflicts = true;
                    result.conflict_versions.push((field.to_vec(), file_stamp(&root.join(label))));
                    continue;
                }
                (parts[1], label, old)
            }
            _ => return Err("Git 状态格式无效".into()),
        };
        let normalized = [if status[0] == b'.' { b' ' } else { status[0] }, if status[1] == b'.' { b' ' } else { status[1] }];
        let status = normalized.as_slice();
        let old_path = old_path.unwrap_or(label);
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
                version: (field.to_vec(), file_stamp(&path)),
                path,
                label: label.into(),
                old_path: old_path.into(),
                head: head.clone(),
                stats,
                stats_ready: false,
                status: FileStatus::from_status(status),
                untracked: status == b"??",
                staged_status: FileStatus::from_status(&status[..1]),
                unstaged_status: if status == b"??" { FileStatus::Untracked } else { FileStatus::from_status(&status[1..2]) },
                staged: status[0] != b' ' && status[0] != b'?',
                unstaged: status[1] != b' ' || status == b"??",
                layer: None,
                staged_stats: None,
                unstaged_stats: None,
            },
        );
    }
    Ok(result)
}

#[cfg(test)]
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

#[cfg(test)]
pub fn layer_statistics(root: &Path) -> Result<(HashMap<String, (usize, usize)>, HashMap<String, (usize, usize)>), String> {
    scoped_statistics(root, &[])
}

fn path_batches(paths: &[String]) -> Vec<Vec<&str>> {
    let mut batches = vec![Vec::new()];
    let mut length = 0;
    for path in paths {
        // Windows command lines have a finite UTF-16 budget. Leave room for
        // quoting and Git arguments, including filenames containing quotes.
        let cost = path.encode_utf16().count() * 2 + 4;
        if length + cost > 12_000 && !batches.last().unwrap().is_empty() {
            batches.push(Vec::new()); length = 0;
        }
        batches.last_mut().unwrap().push(path.as_str()); length += cost;
    }
    batches
}

pub fn scoped_statistics(root: &Path, paths: &[String]) -> Result<(HashMap<String, (usize, usize)>, HashMap<String, (usize, usize)>), String> {
    fn read(root: &Path, paths: &[String], staged: bool) -> Result<HashMap<String, (usize, usize)>, String> {
        let mut counts = HashMap::new();
        for batch in path_batches(paths) {
            let mut args = vec!["--literal-pathspecs", "-c", "core.bigFileThreshold=2m", "diff", "--no-ext-diff", "--no-textconv", "--no-renames", "--numstat", "-z"];
            if staged { args.push("--cached"); }
            args.push("--"); args.extend(batch);
            let bytes = checked(root, &args)?;
            for record in bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
                let text = std::str::from_utf8(record).map_err(|_| "Git 文件名不是 UTF-8")?;
                let mut fields = text.splitn(3, '\t');
                let added = fields.next().and_then(|s| s.parse().ok());
                let removed = fields.next().and_then(|s| s.parse().ok());
                let path = fields.next().ok_or("Git 统计格式无效")?;
                if let (Some(a), Some(r)) = (added, removed) { counts.insert(path.to_owned(), (a, r)); }
            }
        }
        Ok(counts)
    }
    Ok((read(root, paths, true)?, read(root, paths, false)?))
}

pub struct StageUpdate {
    pub before_stamp: Option<RepositoryStamp>,
    pub affected: std::collections::HashSet<PathBuf>,
    pub inventory: Inventory,
}

fn scoped_inventory(sources: &[Source], head: Option<String>) -> Result<Inventory, String> {
    let first = sources.first().ok_or("未选择文件")?;
    let stamp = repository_stamp(&first.root);
    let mut status = Vec::new();
    // Keep each rename pair in one query so Git can recognize the rename.
    let mut batch = Vec::<String>::new();
    let mut length = 0;
    let read = |paths: &[String], bytes: &mut Vec<u8>| -> Result<(), String> {
        let mut args = vec!["--literal-pathspecs", "status", "--porcelain=v2", "-z", "--untracked-files=all", "--"];
        args.extend(paths.iter().map(String::as_str));
        bytes.extend(checked(&first.root, &args)?); Ok(())
    };
    if sources.iter().any(|source| source.root != first.root) { return Err("文件不在同一仓库".into()); }
    if sources.len() > EAGER_STATS_LIMIT {
        read(&[], &mut status)?;
    } else {
        for source in sources {
            let cost = (source.label.encode_utf16().count() + source.old_path.encode_utf16().count()) * 2 + 8;
            if length + cost > 12_000 && !batch.is_empty() { read(&batch, &mut status)?; batch.clear(); length = 0; }
            batch.push(source.label.clone());
            if source.old_path != source.label { batch.push(source.old_path.clone()); }
            length += cost;
        }
        if !batch.is_empty() { read(&batch, &mut status)?; }
    }
    let mut inventory = parse_inventory(first.root.clone(), head, &status)?;
    let allowed: std::collections::HashSet<_> = sources.iter().flat_map(Source::affected_paths).collect();
    inventory.sources.retain(|path, source| allowed.contains(path) || allowed.contains(&source.root.join(&source.old_path)));
    let mut seen = std::collections::HashSet::new();
    inventory.files.retain(|file| inventory.sources.contains_key(&file.path) && seen.insert(file.path.clone()));
    inventory.stamp = stamp;
    Ok(inventory)
}

/// Explicit file/group checkbox action. Paths are NUL-delimited on stdin so
/// large groups and names containing pathspec metacharacters remain literal.
#[cfg(test)]
pub fn set_staged(sources: &[Source], staged: bool) -> Result<(), String> {
    stage_and_refresh(sources, staged).map(|_| ())
}

pub fn stage_and_refresh(sources: &[Source], staged: bool) -> Result<StageUpdate, String> {
    use std::io::Write;
    use std::process::Stdio;
    let first = sources.first().ok_or("未选择文件")?;
    let head = git(&first.root, &["rev-parse", "--verify", "HEAD"])?;
    let head = head.status.success().then(|| String::from_utf8_lossy(&head.stdout).trim().to_owned());
    let fresh = scoped_inventory(sources, head.clone())?;
    let mut paths = std::collections::BTreeSet::new();
    for source in sources {
        if source.root != first.root || fresh.sources.get(&source.path).is_none_or(|current| current.old_path != source.old_path) {
            return Err("Git 文件状态已变化，请刷新后重试".into());
        }
        paths.insert(source.label.clone());
        if !staged && source.old_path != source.label { paths.insert(source.old_path.clone()); }
    }
    let mut command = crate::child_process::command("git");
    command.current_dir(&first.root).env("GIT_LITERAL_PATHSPECS", "1").env("GIT_TERMINAL_PROMPT", "0");
    if staged { command.args(["add"]); }
    else if fresh.head.is_some() { command.args(["reset", "--quiet", "HEAD"]); }
    else { command.args(["rm", "--cached", "--force", "--quiet"]); }
    command.args(["--pathspec-from-file=-", "--pathspec-file-nul"]);
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let mut input = child.stdin.take().ok_or("无法打开 Git 输入")?;
    let write_result = paths.iter().try_for_each(|path| input.write_all(path.as_bytes()).and_then(|_| input.write_all(&[0])));
    drop(input);
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()); }
    write_result.map_err(|e| e.to_string())?;
    let mut inventory = scoped_inventory(sources, head)?;
    let mut affected = std::collections::HashSet::new();
    let mut labels = std::collections::BTreeSet::new();
    for source in sources.iter().chain(fresh.sources.values()).chain(inventory.sources.values()) {
        affected.insert(source.path.clone());
        affected.insert(source.root.join(&source.old_path));
        labels.insert(source.label.clone()); labels.insert(source.old_path.clone());
    }
    // Counts are optional; a failed statistic must not hide a successful index update.
    let eager = sources.len() <= EAGER_STATS_LIMIT;
    let (staged_counts, unstaged_counts) = if eager {
        scoped_statistics(&first.root, &labels.into_iter().collect::<Vec<_>>()).unwrap_or_default()
    } else { Default::default() };
    for source in inventory.sources.values_mut() {
        source.stats_ready = eager;
        source.staged_stats = (source.old_path == source.label).then(|| staged_counts.get(&source.label).copied()).flatten();
        source.unstaged_stats = unstaged_counts.get(&source.label).copied();
    }
    Ok(StageUpdate { before_stamp: fresh.stamp, affected, inventory })
}

impl Source {
    pub fn same_version(&self, other: &Self) -> bool { self.version == other.version && self.old_path == other.old_path }

    pub fn overlaps(&self, pending: &std::collections::HashSet<PathBuf>) -> bool {
        !pending.is_empty() && (pending.contains(&self.path) || self.old_path != self.label && pending.contains(&self.root.join(&self.old_path)))
    }

    pub fn affected_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.path.clone()];
        if self.old_path != self.label { paths.push(self.root.join(&self.old_path)); }
        paths
    }
}

#[cfg(test)]
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

fn blob(source: &Source, spec: &str) -> Result<String, String> {
    let size = git(&source.root, &["cat-file", "-s", spec])?;
    if !size.status.success() { return Ok(String::new()); }
    let size: usize = String::from_utf8_lossy(&size.stdout).trim().parse().map_err(|_| "Git 文件大小无效")?;
    if size > 2 * 1024 * 1024 { return Err("Git 版本超过 2 MiB，未加载全文 Diff".into()); }
    let bytes = checked(&source.root, &["cat-file", "blob", spec])?;
    let text = String::from_utf8(bytes).map_err(|_| "Git 文件不是 UTF-8 文本")?;
    if text.contains('\0') { return Err("二进制文件不支持文本 Diff".into()); }
    Ok(text)
}

impl Source {
    pub fn for_layer(&self, staged: bool) -> Self {
        let mut source = self.clone();
        source.layer = Some(staged);
        source.status = if staged { self.staged_status } else { self.unstaged_status };
        source.stats = if staged { self.staged_stats } else { self.unstaged_stats };
        source
    }
}

fn worktree_text(source: &Source) -> Result<String, String> {
    let text = match std::fs::symlink_metadata(&source.path) {
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
    if text.contains('\0') { return Err("二进制文件不支持文本 Diff".into()); }
    Ok(text)
}

pub fn load(source: &Source) -> Result<Change, String> {
    if let Some(staged) = source.layer {
        let before = if staged {
            match &source.head { Some(head) => blob(source, &format!("{head}:{}", source.old_path))?, None => String::new() }
        } else if source.untracked { String::new() } else { blob(source, &format!(":0:{}", source.label))? };
        let after = if staged { blob(source, &format!(":0:{}", source.label))? } else {
            worktree_text(source)?
        };
        return Ok(Change::preview(source.path.clone(), source.label.clone(), Some(before.into()), after.into(), None));
    }
    let after = worktree_text(source)?;
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
        assert!(load(&data.sources[&data.files[0].path].for_layer(false)).unwrap_err().contains("二进制"));
    }

    #[test]
    fn versions_detect_index_only_edits_and_linked_worktree_index_changes() {
        let dir = many_files(1);
        let path = dir.path().join("vendor/0000.rs");
        git_ok(dir.path(), &["add", "."]);
        std::fs::write(&path, "working tree stays unchanged\n").unwrap();
        let before = inventory(dir.path()).unwrap();
        let source = before.sources.values().next().unwrap();
        let blob_file = dir.path().join("index-blob.txt");
        std::fs::write(&blob_file, "different index content\n").unwrap();
        let oid = checked(dir.path(), &["hash-object", "-w", "index-blob.txt"]).unwrap();
        let oid = String::from_utf8(oid).unwrap();
        git_ok(dir.path(), &["update-index", "--cacheinfo", "100644", oid.trim(), "vendor/0000.rs"]);
        let after = inventory(dir.path()).unwrap();
        let next = after.sources.values().find(|s| s.label == source.label).unwrap();
        assert!(!source.same_version(next), "same MM status and worktree metadata still detect index content changes");
        assert_eq!(load(&next.for_layer(true)).unwrap().after.as_ref(), "different index content\n");
        let parent = tempfile::tempdir().unwrap();
        let linked = parent.path().join("linked worktree");
        git_ok(dir.path(), &["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
        let stamp = repository_stamp(&linked).unwrap();
        std::fs::write(linked.join("vendor/0000.rs"), "linked change\n").unwrap();
        git_ok(&linked, &["add", "."]);
        assert_ne!(repository_stamp(&linked).unwrap(), stamp);
    }

    #[test]
    fn stage_updates_only_selected_paths_and_reports_clean_files() {
        let dir = many_files(3);
        let inventory = inventory(dir.path()).unwrap();
        let path = inventory.root.join("vendor/0001.rs");
        let source = inventory.sources[&path].clone();
        let update = stage_and_refresh(&[source], true).unwrap();
        assert_eq!(update.affected, [path.clone()].into_iter().collect());
        assert_eq!(update.inventory.files.len(), 1);
        assert!(update.inventory.files.iter().all(|file| file.before.is_none() && file.after.is_empty()));
        let source = update.inventory.sources[&path].clone();
        assert!(source.staged && !source.unstaged);
        assert_eq!(source.staged_stats, Some((1, 0)));
        std::fs::write(&path, "fn before() {}\n").unwrap();
        let update = stage_and_refresh(&[source], true).unwrap();
        assert_eq!(update.affected, [path.clone()].into_iter().collect());
        assert!(update.inventory.files.is_empty());
        assert!(update.inventory.sources.is_empty());
        assert!(checked(dir.path(), &["diff", "--cached", "--name-only"]).unwrap().is_empty());
        assert!(dir.path().join("vendor/0002.rs").exists());
    }

    #[test]
    fn partial_staging_previews_and_group_actions_preserve_worktree() {
        let dir = many_files(1);
        let path = dir.path().join("vendor/0000.rs");
        git_ok(dir.path(), &["add", "vendor/0000.rs"]);
        std::fs::write(&path, "fn before() {}\n// changed 0\n// unstaged\n").unwrap();
        let data = inventory(dir.path()).unwrap();
        let source = data.sources.values().next().unwrap();
        assert!(source.staged && source.unstaged);
        let staged = load(&source.for_layer(true)).unwrap();
        let unstaged = load(&source.for_layer(false)).unwrap();
        assert_eq!((staged.added, staged.removed), (1, 0));
        assert_eq!((unstaged.added, unstaged.removed), (1, 0));
        assert!(!staged.after.contains("unstaged"));
        assert!(unstaged.after.contains("unstaged"));
        assert_eq!(unstaged.before.as_deref(), Some(staged.after.as_ref()));
        let (staged_stats, unstaged_stats) = layer_statistics(dir.path()).unwrap();
        assert_eq!(staged_stats["vendor/0000.rs"], (1, 0));
        assert_eq!(unstaged_stats["vendor/0000.rs"], (1, 0));
        set_staged(&[source.clone()], true).unwrap();
        let data = inventory(dir.path()).unwrap();
        let source = data.sources.values().next().unwrap();
        assert!(source.staged && !source.unstaged);
        let before = std::fs::read(&path).unwrap();
        set_staged(&[source.clone()], false).unwrap();
        let data = inventory(dir.path()).unwrap();
        let source = data.sources.values().next().unwrap();
        assert!(!source.staged && source.unstaged);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn checkbox_handles_unborn_literal_paths_renames_and_deletions() {
        let dir = tempfile::tempdir().unwrap();
        git_ok(dir.path(), &["init"]);
        git_ok(dir.path(), &["config", "core.autocrlf", "false"]);
        for label in ["中文 [1].txt", "other.txt"] { std::fs::write(dir.path().join(label), "base\n").unwrap(); }
        let data = inventory(dir.path()).unwrap();
        let sources: Vec<_> = data.sources.into_values().collect();
        set_staged(&sources, true).unwrap();
        let data = inventory(dir.path()).unwrap();
        assert!(data.sources.values().all(|s| s.staged && !s.unstaged));
        let sources: Vec<_> = data.sources.into_values().collect();
        set_staged(&sources, false).unwrap();
        assert!(inventory(dir.path()).unwrap().sources.values().all(|s| !s.staged && s.unstaged));
        set_staged(&sources, true).unwrap();
        git_ok(dir.path(), &["commit", "-m", "baseline"]);
        git_ok(dir.path(), &["mv", "中文 [1].txt", "renamed.txt"]);
        std::fs::write(dir.path().join("renamed.txt"), "base\nnew\n").unwrap();
        let data = inventory(dir.path()).unwrap();
        let source = data.sources.values().next().unwrap();
        assert_eq!(source.for_layer(true).status, FileStatus::Renamed);
        assert_eq!(source.for_layer(false).status, FileStatus::Modified);
        assert_eq!(load(&source.for_layer(true)).unwrap().added, 0);
        assert_eq!(load(&source.for_layer(false)).unwrap().added, 1);
        set_staged(&[source.clone()], false).unwrap();
        let data = inventory(dir.path()).unwrap();
        assert!(data.sources.values().all(|s| !s.staged));
        let sources: Vec<_> = data.sources.into_values().collect();
        set_staged(&sources, true).unwrap();
        assert!(!dir.path().join("中文 [1].txt").exists());
        assert_eq!(std::fs::read_to_string(dir.path().join("renamed.txt")).unwrap(), "base\nnew\n");
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
