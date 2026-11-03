//! Git I/O shared by the composer branch picker and GitGraph. Never force checkout.
use gpui_glint::components::{
    DiffLine, GitCommit, GitCommitDetails, GitCommitFile, GitComparison, GitGraphData, diff_lines,
};
use std::{
    path::{Path, PathBuf},
    process::Output,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: String,
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
    pub locked: Option<String>,
    pub prunable: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BranchSelection {
    Switched,
    Worktree(PathBuf),
}

fn git_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    if crate::ssh_project::is_remote(root) {
        let mut remote = crate::ssh_project::Remote::from_path(root)?;
        remote.directory = path.into();
        Ok(remote.uri())
    } else {
        let path = PathBuf::from(path);
        let path = std::fs::canonicalize(&path).unwrap_or(path);
        #[cfg(windows)]
        {
            let text = path.to_string_lossy();
            if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
                return Ok(PathBuf::from(format!(r"\\{unc}")));
            }
            if let Some(dos) = text.strip_prefix(r"\\?\") {
                return Ok(PathBuf::from(dos));
            }
        }
        Ok(path)
    }
}

fn parse_worktrees(root: &Path, bytes: &[u8]) -> Result<Vec<Worktree>, String> {
    let mut worktrees = Vec::new();
    let mut current: Option<Worktree> = None;
    for field in bytes.split(|byte| *byte == 0) {
        let field = std::str::from_utf8(field).map_err(|_| "Git worktree 信息包含无效 UTF-8")?;
        if field.is_empty() {
            if let Some(tree) = current.take() {
                worktrees.push(tree);
            }
            continue;
        }
        let (key, value) = field.split_once(' ').unwrap_or((field, ""));
        if key == "worktree" {
            if let Some(tree) = current.take() {
                worktrees.push(tree);
            }
            current = Some(Worktree {
                path: git_path(root, value)?,
                ..Default::default()
            });
        } else if let Some(tree) = &mut current {
            match key {
                "HEAD" => tree.head = value.into(),
                "branch" => tree.branch = Some(value.into()),
                "detached" => tree.detached = true,
                "bare" => tree.bare = true,
                "locked" => tree.locked = Some(value.into()),
                "prunable" => tree.prunable = Some(value.into()),
                _ => {}
            }
        }
    }
    if let Some(tree) = current {
        worktrees.push(tree);
    }
    Ok(worktrees)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    pub current: String,
    pub branches: Vec<String>,
    pub worktree: PathBuf,
    pub common_dir: PathBuf,
    pub worktrees: Vec<Worktree>,
}

async fn run(root: &Path, args: &[&str]) -> Result<Output, String> {
    let mut command = if crate::ssh_project::is_remote(root) {
        let remote = crate::ssh_project::Remote::from_path(root)?;
        let argv = args
            .iter()
            .map(|s| crate::ssh_project::quote(s))
            .collect::<Vec<_>>()
            .join(" ");
        remote.command(&format!(
            "cd -- {} && GIT_TERMINAL_PROMPT=0 git {argv}",
            crate::ssh_project::quote(&remote.directory)
        ))
    } else {
        let mut command = crate::child_process::async_command("git");
        command.current_dir(root).args(args);
        command
    };
    command.env("GIT_TERMINAL_PROMPT", "0").kill_on_drop(true);
    tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
        .await
        .map_err(|_| "Git 操作超时".to_owned())?
        .map_err(|e| format!("无法运行 Git：{e}"))
}

fn text(output: Output) -> Result<String, String> {
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if error.is_empty() {
            format!("Git 操作失败：{}", output.status)
        } else {
            error
        })
    }
}

pub async fn repository(root: &Path) -> Result<Option<Repository>, String> {
    let probe = run(root, &["rev-parse", "--is-inside-work-tree"]).await?;
    if !probe.status.success() || String::from_utf8_lossy(&probe.stdout).trim() != "true" {
        return Ok(None);
    }
    let (symbolic, branches, top, common, output) = tokio::try_join!(
        run(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]),
        run(
            root,
            &[
                "for-each-ref",
                "--sort=refname",
                "--format=%(refname:strip=2)",
                "refs/heads/"
            ]
        ),
        run(root, &["rev-parse", "--show-toplevel"]),
        run(
            root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"]
        ),
        run(root, &["worktree", "list", "--porcelain", "-z"]),
    )?;
    let current = if symbolic.status.success() {
        text(symbolic)?
    } else {
        format!(
            "分离 HEAD · {}",
            text(run(root, &["rev-parse", "--short", "HEAD"]).await?)?
        )
    };
    let branches = text(branches)?.lines().map(str::to_owned).collect();
    let worktree = git_path(root, &text(top)?)?;
    let common_dir = git_path(root, &text(common)?)?;
    if !output.status.success() {
        return Err(text(output).unwrap_err());
    }
    let worktrees = parse_worktrees(root, &output.stdout)?;
    Ok(Some(Repository {
        current,
        branches,
        worktree,
        common_dir,
        worktrees,
    }))
}

/// Recheck occupancy at click time; navigating never checks out another copy.
pub async fn select_worktree(root: &Path, path: &Path) -> Result<BranchSelection, String> {
    let repo = repository(root).await?.ok_or("当前项目不是 Git 工作区")?;
    let tree = repo
        .worktrees
        .iter()
        .find(|tree| tree.path == path && !tree.bare)
        .ok_or("该 worktree 已不属于当前仓库，请刷新后重试")?;
    let target = repository(&tree.path)
        .await
        .map_err(|error| format!("无法打开 worktree {}：{error}", tree.path.display()))?
        .ok_or_else(|| format!("worktree 目录不可用：{}", tree.path.display()))?;
    if target.common_dir != repo.common_dir {
        return Err("worktree 已变更为其他仓库，请刷新后重试".into());
    }
    Ok(BranchSelection::Worktree(tree.path.clone()))
}

pub async fn select_branch(
    root: &Path,
    branch: &str,
    expected_worktree: Option<&Path>,
) -> Result<BranchSelection, String> {
    let repo = repository(root).await?.ok_or("当前项目不是 Git 工作区")?;
    if !repo.branches.iter().any(|name| name == branch) {
        return Err("该本地分支已不存在".into());
    }
    let reference = format!("refs/heads/{branch}");
    let occupied = repo
        .worktrees
        .iter()
        .find(|tree| tree.branch.as_deref() == Some(&reference) && tree.path != repo.worktree);
    if occupied.map(|tree| tree.path.as_path()) != expected_worktree {
        return Err("分支的 worktree 占用已变化，请刷新后重试".into());
    }
    if repo.current == branch {
        return Ok(BranchSelection::Switched);
    }
    if let Some(tree) = occupied {
        let target = repository(&tree.path)
            .await
            .map_err(|error| format!("无法打开 worktree {}：{error}", tree.path.display()))?
            .ok_or_else(|| format!("worktree 目录不可用：{}", tree.path.display()))?;
        if target.common_dir != repo.common_dir || target.current != branch {
            return Err("worktree 状态已变化，请刷新后重试".into());
        }
        return Ok(BranchSelection::Worktree(tree.path.clone()));
    }
    text(run(root, &["switch", "--no-guess", "--", branch]).await?)?;
    Ok(BranchSelection::Switched)
}

pub async fn switch(root: &Path, branch: &str) -> Result<(), String> {
    // Verify against local refs. No option injection, remote guessing, reset or stash.
    let repo = repository(root).await?.ok_or("当前项目不是 Git 工作区")?;
    if !repo.branches.iter().any(|b| b == branch) {
        return Err("该本地分支已不存在".into());
    }
    text(run(root, &["switch", "--no-guess", "--", branch]).await?)?;
    Ok(())
}

const FORMAT: &str = "--format=%H%x1f%P%x1f%an%x1f%ad%x1f%s%x1f%D";
fn commits(source: &str) -> Vec<GitCommit> {
    source
        .lines()
        .filter_map(|line| {
            let fields = line.splitn(6, '\x1f').collect::<Vec<_>>();
            if fields.len() != 6 {
                return None;
            }
            Some(GitCommit {
                id: fields[0].into(),
                parents: fields[1].split_whitespace().map(str::to_owned).collect(),
                author: fields[2].into(),
                date: fields[3].into(),
                summary: fields[4].into(),
                references: fields[5].into(),
            })
        })
        .collect()
}

async fn log(root: &Path, revision: &str) -> Result<Vec<GitCommit>, String> {
    let mut args = vec![
        "log",
        "--topo-order",
        "--max-count=100",
        "--date=short",
        FORMAT,
        revision,
    ];
    if revision == "--all" {
        args.push("HEAD");
    }
    args.push("--");
    Ok(commits(&text(run(root, &args).await?)?))
}

pub async fn graph(root: &Path, target: String) -> Result<GitGraphData, String> {
    let repo = repository(root)
        .await?
        .ok_or("当前项目尚未初始化 Git 仓库")?;
    let head = run(root, &["rev-parse", "--verify", "HEAD^{commit}"]).await?;
    let mut data = GitGraphData {
        current_branch: repo.current,
        branches: repo.branches,
        compare_branch: target.clone(),
        ..Default::default()
    };
    if !head.status.success() {
        return Ok(data);
    } // An initialized, unborn repository.
    let head = text(head)?;
    data.commits = log(root, "--all").await?;
    if !data.branches.contains(&target) {
        data.comparison_error = Some(format!("分支 {target} 不存在，请选择对比分支"));
        return Ok(data);
    }
    let target_id = text(
        run(
            root,
            &[
                "rev-parse",
                "--verify",
                &format!("refs/heads/{target}^{{commit}}"),
            ],
        )
        .await?,
    )?;
    let range = format!("{target_id}...{head}");
    let counts = text(run(root, &["rev-list", "--left-right", "--count", &range]).await?)?;
    let counts = counts.split_whitespace().collect::<Vec<_>>();
    data.comparison = Some(GitComparison {
        target_count: counts.first().and_then(|v| v.parse().ok()).unwrap_or(0),
        current_count: counts.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
        current_only: log(root, &format!("{target_id}..{head}")).await?,
        target_only: log(root, &format!("{head}..{target_id}")).await?,
    });
    Ok(data)
}

pub async fn details(root: &Path, id: &str) -> Result<GitCommitDetails, String> {
    if id.len() < 7 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("无效的提交标识".into());
    }
    let metadata = text(
        run(
            root,
            &[
                "show",
                "-s",
                "--format=%H%x00%P%x00%an%x00%aI%x00%cn%x00%cI%x00%B",
                id,
                "--",
            ],
        )
        .await?,
    )?;
    let fields = metadata.splitn(7, '\0').collect::<Vec<_>>();
    if fields.len() != 7 {
        return Err("提交信息格式无效".into());
    }
    let parents = fields[1]
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let output = if let Some(parent) = parents.first() {
        run(
            root,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--numstat",
                "-z",
                parent,
                fields[0],
                "--",
            ],
        )
        .await?
    } else {
        run(
            root,
            &[
                "diff-tree",
                "--root",
                "--no-commit-id",
                "-r",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--numstat",
                "-z",
                fields[0],
                "--",
            ],
        )
        .await?
    };
    if !output.status.success() {
        return Err(text(output).unwrap_err());
    }
    let stats = String::from_utf8(output.stdout).map_err(|_| "改动文件名不是有效的 UTF-8")?;
    let mut files = Vec::new();
    for record in stats.split('\0').filter(|s| !s.is_empty()) {
        let fields = record.splitn(3, '\t').collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err("文件改动信息格式无效".into());
        }
        files.push(GitCommitFile {
            path: fields[2].into(),
            additions: fields[0].parse().ok(),
            deletions: fields[1].parse().ok(),
        });
    }
    Ok(GitCommitDetails {
        id: fields[0].into(),
        parents,
        author: fields[2].into(),
        authored_at: fields[3].into(),
        committer: fields[4].into(),
        committed_at: fields[5].into(),
        message: fields[6].trim().into(),
        files,
    })
}

async fn blob(root: &Path, revision: Option<&str>, path: &str) -> Result<String, String> {
    let Some(revision) = revision else {
        return Ok(String::new());
    };
    let entry = text(
        run(
            root,
            &["--literal-pathspecs", "ls-tree", "-z", revision, "--", path],
        )
        .await?,
    )?;
    if entry.is_empty() {
        return Ok(String::new());
    }
    let fields = entry
        .split_once('\t')
        .ok_or("Git 文件对象格式无效")?
        .0
        .split_whitespace()
        .collect::<Vec<_>>();
    if fields.len() != 3 {
        return Err("Git 文件对象格式无效".into());
    }
    if fields[1] == "commit" {
        return Ok(format!("Subproject commit {}\n", fields[2]));
    }
    if fields[1] != "blob" {
        return Err("所选改动不是文件".into());
    }
    let size = text(run(root, &["cat-file", "-s", fields[2]]).await?)?
        .parse::<usize>()
        .map_err(|_| "文件大小无效")?;
    if size > 2 * 1024 * 1024 {
        return Err("文件超过 2 MiB，暂不支持在侧栏预览".into());
    }
    let output = run(root, &["cat-file", "blob", fields[2]]).await?;
    if !output.status.success() {
        return Err(text(output).unwrap_err());
    }
    if output.stdout.contains(&0) {
        return Err("二进制文件不支持文本 Diff 预览".into());
    }
    String::from_utf8(output.stdout).map_err(|_| "文件不是 UTF-8 文本，暂不支持 Diff 预览".into())
}

pub async fn file_diff(root: &Path, id: &str, path: &str) -> Result<Vec<DiffLine>, String> {
    let commit = details(root, id).await?;
    if !commit.files.iter().any(|file| file.path == path) {
        return Err("该文件不属于此提交的改动".into());
    }
    let old = blob(root, commit.parents.first().map(String::as_str), path).await?;
    let new = blob(root, Some(&commit.id), path).await?;
    Ok(diff_lines(&old, &new))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_porcelain_preserves_paths_and_state_flags() {
        let trees = parse_worktrees(Path::new("."), b"worktree /tmp/main\0HEAD abc\0branch refs/heads/main\0\0worktree /tmp/space \xe4\xb8\xad\xe6\x96\x87\nline\0HEAD def\0detached\0locked maintenance\0prunable missing directory\0\0worktree /tmp/bare\0bare\0\0").unwrap();
        assert_eq!(trees.len(), 3);
        assert_eq!(trees[0].branch.as_deref(), Some("refs/heads/main"));
        assert_eq!(trees[1].path, PathBuf::from("/tmp/space 中文\nline"));
        assert!(trees[1].detached);
        assert_eq!(trees[1].locked.as_deref(), Some("maintenance"));
        assert_eq!(trees[1].prunable.as_deref(), Some("missing directory"));
        assert!(trees[2].bare);
        let remote = parse_worktrees(
            Path::new("ssh://user@example.test:2222/repo"),
            b"worktree /other/worktree\0branch refs/heads/feature\0\0",
        )
        .unwrap();
        assert!(crate::ssh_project::is_remote(&remote[0].path));
        assert_eq!(
            crate::ssh_project::Remote::from_path(&remote[0].path)
                .unwrap()
                .directory,
            "/other/worktree"
        );
    }

    #[tokio::test]
    async fn occupied_branches_open_worktrees_and_stale_occupancy_never_switches() {
        let main = init();
        commit(main.path(), "base");
        let parent = tempfile::tempdir().unwrap();
        let linked = parent.path().join("linked 中文 spaces");
        git(
            main.path(),
            &["worktree", "add", "-b", "feature", linked.to_str().unwrap()],
        );
        git(
            main.path(),
            &[
                "worktree",
                "lock",
                "--reason",
                "keep",
                linked.to_str().unwrap(),
            ],
        );
        std::fs::write(main.path().join("draft.txt"), "main-only").unwrap();
        std::fs::write(linked.join("draft.txt"), "linked-only").unwrap();
        let repo = repository(main.path()).await.unwrap().unwrap();
        let tree = repo
            .worktrees
            .iter()
            .find(|tree| tree.branch.as_deref() == Some("refs/heads/feature"))
            .unwrap();
        assert!(tree.locked.is_some());
        assert_eq!(
            select_branch(main.path(), "feature", Some(&tree.path))
                .await
                .unwrap(),
            BranchSelection::Worktree(tree.path.clone())
        );
        assert_eq!(
            select_worktree(main.path(), &tree.path).await.unwrap(),
            BranchSelection::Worktree(tree.path.clone())
        );
        assert!(select_worktree(main.path(), parent.path()).await.is_err());
        assert_eq!(
            git(main.path(), &["symbolic-ref", "--short", "HEAD"]),
            "master"
        );
        assert_eq!(
            std::fs::read_to_string(linked.join("draft.txt")).unwrap(),
            "linked-only"
        );
        assert!(
            select_branch(main.path(), "feature", None)
                .await
                .unwrap_err()
                .contains("占用已变化")
        );
        assert_eq!(
            git(main.path(), &["symbolic-ref", "--short", "HEAD"]),
            "master"
        );
        git(main.path(), &["branch", "free"]);
        assert!(
            select_branch(main.path(), "free", Some(&tree.path))
                .await
                .is_err()
        );
        assert_eq!(
            git(main.path(), &["symbolic-ref", "--short", "HEAD"]),
            "master"
        );
        assert_eq!(
            select_branch(main.path(), "free", None).await.unwrap(),
            BranchSelection::Switched
        );
        assert_eq!(
            git(main.path(), &["symbolic-ref", "--short", "HEAD"]),
            "free"
        );
    }
    fn git(root: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .current_dir(root)
            .args([
                "-c",
                "user.name=GitGraph Test",
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
        text(output).unwrap()
    }
    fn init() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-b", "master"]);
        dir
    }
    fn commit(root: &Path, value: &str) {
        std::fs::write(root.join("file.txt"), value).unwrap();
        git(root, &["add", "file.txt"]);
        git(root, &["commit", "-m", value]);
    }
    #[tokio::test]
    async fn structured_files_and_historical_diff_handle_root_rename_delete_and_binary() {
        use gpui_glint::components::DiffLineKind;
        let dir = init();
        let root = dir.path();
        let original = "中文 [v1] file.txt";
        std::fs::write(root.join(original), "committed text\n").unwrap();
        git(root, &["add", "--", original]);
        git(root, &["commit", "-m", "Root commit\n\nDetails body"]);
        let id = git(root, &["rev-parse", "HEAD"]);
        let info = details(root, &id).await.unwrap();
        assert_eq!(info.id, id);
        assert!(info.parents.is_empty());
        assert_eq!(info.files[0].path, original);
        assert_eq!(info.files[0].additions, Some(1));
        assert!(info.message.contains("Details body"));
        let lines = file_diff(root, &id, original).await.unwrap();
        assert!(
            lines
                .iter()
                .any(|l| l.kind == DiffLineKind::Added && l.text.as_ref() == "committed text")
        );

        git(root, &["mv", "--", original, "renamed.txt"]);
        git(root, &["commit", "-m", "Rename"]);
        let renamed = git(root, &["rev-parse", "HEAD"]);
        let info = details(root, &renamed).await.unwrap();
        assert_eq!(info.files.len(), 2); // Explicit delete/add keeps exact paths without rename heuristics.
        std::fs::write(root.join("renamed.txt"), "uncommitted content").unwrap();
        let lines = file_diff(root, &renamed, "renamed.txt").await.unwrap();
        assert!(lines.iter().any(|l| l.text.as_ref() == "committed text"));
        assert!(!lines.iter().any(|l| l.text.contains("uncommitted")));
        assert!(
            file_diff(root, &renamed, original)
                .await
                .unwrap()
                .iter()
                .any(|l| l.kind == DiffLineKind::Deleted)
        );

        git(root, &["rm", "-f", "renamed.txt"]);
        git(root, &["commit", "-m", "Delete"]);
        let deleted = git(root, &["rev-parse", "HEAD"]);
        assert!(
            file_diff(root, &deleted, "renamed.txt")
                .await
                .unwrap()
                .iter()
                .any(|l| l.kind == DiffLineKind::Deleted)
        );

        std::fs::write(root.join("binary.dat"), [0, 1, 2, 3]).unwrap();
        git(root, &["add", "binary.dat"]);
        git(root, &["commit", "-m", "Binary"]);
        let binary = git(root, &["rev-parse", "HEAD"]);
        assert_eq!(
            details(root, &binary).await.unwrap().files[0].additions,
            None
        );
        assert!(
            file_diff(root, &binary, "binary.dat")
                .await
                .unwrap_err()
                .contains("二进制")
        );
        assert!(file_diff(root, &binary, "unrelated.txt").await.is_err());
    }
    #[tokio::test]
    async fn merge_details_and_diff_compare_against_first_parent() {
        let dir = init();
        let root = dir.path();
        commit(root, "base");
        git(root, &["checkout", "-b", "feature"]);
        std::fs::write(root.join("feature.txt"), "feature\n").unwrap();
        git(root, &["add", "feature.txt"]);
        git(root, &["commit", "-m", "Feature file"]);
        git(root, &["checkout", "master"]);
        commit(root, "master update");
        git(
            root,
            &["merge", "--no-ff", "feature", "-m", "Merge feature"],
        );
        let id = git(root, &["rev-parse", "HEAD"]);
        let details = details(root, &id).await.unwrap();
        assert_eq!(details.parents.len(), 2);
        assert_eq!(
            details
                .files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            vec!["feature.txt"]
        );
        assert!(
            !file_diff(root, &id, "feature.txt")
                .await
                .unwrap()
                .is_empty()
        );
    }
    #[tokio::test]
    async fn no_repository_unborn_detached_and_worktree_are_distinguished() {
        let plain = tempfile::tempdir().unwrap();
        assert!(repository(plain.path()).await.unwrap().is_none());
        let dir = init();
        let root = dir.path();
        assert_eq!(repository(root).await.unwrap().unwrap().current, "master");
        assert!(
            graph(root, "master".into())
                .await
                .unwrap()
                .commits
                .is_empty()
        );
        commit(root, "base");
        let worktree_parent = tempfile::tempdir().unwrap();
        let worktree = worktree_parent.path().join("linked");
        git(
            root,
            &[
                "worktree",
                "add",
                "-b",
                "linked",
                worktree.to_str().unwrap(),
                "master",
            ],
        );
        assert!(worktree.join(".git").is_file());
        assert_eq!(
            repository(&worktree).await.unwrap().unwrap().current,
            "linked"
        );
        git(root, &["checkout", "--detach"]);
        let detached = repository(root).await.unwrap().unwrap();
        assert!(detached.current.starts_with("分离 HEAD"));
        assert_eq!(
            select_worktree(&worktree, &detached.worktree)
                .await
                .unwrap(),
            BranchSelection::Worktree(detached.worktree)
        );
        assert!(
            !graph(root, "master".into())
                .await
                .unwrap()
                .commits
                .is_empty()
        );
    }
    #[tokio::test]
    async fn switch_refuses_conflicting_work_and_comparison_counts_both_sides() {
        let dir = init();
        let root = dir.path();
        commit(root, "base");
        git(root, &["branch", "feature"]);
        commit(root, "master-only");
        switch(root, "feature").await.unwrap();
        commit(root, "feature-only");
        let graph = graph(root, "master".into()).await.unwrap();
        let comparison = graph.comparison.unwrap();
        assert_eq!((comparison.current_count, comparison.target_count), (1, 1));
        assert_eq!(comparison.current_only[0].summary, "feature-only");
        assert_eq!(comparison.target_only[0].summary, "master-only");
        let detail = details(root, &comparison.current_only[0].id).await.unwrap();
        assert_eq!(detail.message, "feature-only");
        assert_eq!(detail.files[0].path, "file.txt");
        std::fs::write(root.join("file.txt"), "unsaved work").unwrap();
        let error = switch(root, "master").await.unwrap_err();
        assert!(error.contains("file.txt"));
        assert_eq!(repository(root).await.unwrap().unwrap().current, "feature");
        assert_eq!(
            std::fs::read_to_string(root.join("file.txt")).unwrap(),
            "unsaved work"
        );
        assert!(switch(root, "--discard-changes").await.is_err());
        assert!(
            super::graph(root, "missing".into())
                .await
                .unwrap()
                .comparison_error
                .is_some()
        );
    }
}
