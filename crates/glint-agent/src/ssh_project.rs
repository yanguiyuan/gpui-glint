//! SSH project identity and transport. Remote roots are persisted as SSH URIs.
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

const HELPER: &str = include_str!("ssh_project.py");
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub target: String,
    pub port: Option<u16>,
    pub directory: String,
}
pub fn is_remote(path: &Path) -> bool {
    path.to_string_lossy().starts_with("ssh://")
}
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

impl Remote {
    pub fn parse(input: &str, directory: &str) -> Result<Self, String> {
        let input = input.trim();
        let (target, port, directory) = if let Some(uri) = input.strip_prefix("ssh://") {
            let (authority, path) = uri.split_once('/').unwrap_or((uri, ""));
            let (target, port) = if let Some((target, port)) = authority.rsplit_once(':') {
                (
                    target,
                    Some(port.parse::<u16>().map_err(|_| "SSH 端口无效")?),
                )
            } else {
                (authority, None)
            };
            (
                target.to_owned(),
                port,
                if path.is_empty() {
                    if uri.contains('/') {
                        "/".into()
                    } else {
                        directory.to_owned()
                    }
                } else {
                    if path == "~" || path.starts_with("~/") {
                        path.to_owned()
                    } else {
                        format!("/{path}")
                    }
                },
            )
        } else {
            let parts: Vec<_> = input.split_whitespace().collect();
            let parts = if parts.first() == Some(&"ssh") {
                &parts[1..]
            } else {
                &parts[..]
            };
            match parts {
                [target] => (target.to_string(), None, directory.to_owned()),
                ["-p", port, target] => (
                    target.to_string(),
                    Some(port.parse::<u16>().map_err(|_| "SSH 端口无效")?),
                    directory.to_owned(),
                ),
                _ => return Err("请输入 ssh user@host 或 ssh -p 端口 user@host".into()),
            }
        };
        if target.is_empty()
            || target.starts_with('-')
            || target.starts_with('@')
            || target.ends_with('@')
            || target.matches('@').count() > 1
            || !target
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"@._-".contains(&b))
            || port == Some(0)
        {
            return Err("SSH 主机地址或端口无效".into());
        }
        let directory = if directory.trim().is_empty() {
            "~".into()
        } else {
            directory
        };
        if directory.contains(['\0', '\n', '\r']) {
            return Err("远程目录无效".into());
        }
        Ok(Self {
            target,
            port,
            directory,
        })
    }
    pub fn from_path(path: &Path) -> Result<Self, String> {
        Self::parse(&path.to_string_lossy(), "~")
    }
    pub fn uri(&self) -> PathBuf {
        PathBuf::from(format!(
            "ssh://{}{}{}/{}",
            self.target,
            if self.port.is_some() { ":" } else { "" },
            self.port.map(|p| p.to_string()).unwrap_or_default(),
            self.directory.trim_start_matches('/')
        ))
    }
    pub fn label(&self) -> String {
        let name = self
            .directory
            .rsplit('/')
            .find(|s| !s.is_empty())
            .unwrap_or("/");
        format!("{} · {name}", self.target)
    }
    pub fn command(&self, remote_command: &str) -> tokio::process::Command {
        let mut command = tokio::process::Command::new("ssh");
        command.args([
            "-T",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=2",
            "-o",
            "StrictHostKeyChecking=accept-new",
        ]);
        if let Some(port) = self.port {
            command.args(["-p", &port.to_string()]);
        }
        command
            .arg("--")
            .arg(&self.target)
            .arg(remote_command)
            .kill_on_drop(true);
        for (name, _) in std::env::vars() {
            if name.ends_with("_KEY")
                || name.ends_with("_TOKEN")
                || name.ends_with("_SECRET")
                || name == "deepseek_api_key"
            {
                command.env_remove(name);
            }
        }
        command
    }
    #[cfg(test)]
    pub fn run(&self, program: &str, args: &[String]) -> tokio::process::Command {
        let argv = std::iter::once(program)
            .chain(args.iter().map(String::as_str))
            .map(quote)
            .collect::<Vec<_>>()
            .join(" ");
        self.command(&format!("cd -- {} && exec {argv}", quote(&self.directory)))
    }
    pub fn managed_run(&self, program: &str, args: &[String]) -> tokio::process::Command {
        const WORKER: &str = r#"import subprocess,sys,signal,threading,os,json
argv=json.loads(sys.argv[1])
p=subprocess.Popen(argv,start_new_session=True,stdin=subprocess.DEVNULL)
def stop(*_):
 try: os.killpg(p.pid,signal.SIGKILL)
 except ProcessLookupError: pass
def eof():
 sys.stdin.buffer.read()
 stop()
for s in (signal.SIGTERM,signal.SIGHUP,signal.SIGINT):signal.signal(s,stop)
threading.Thread(target=eof,daemon=True).start()
try: code=p.wait()
finally: stop()
sys.exit(code if code>=0 else 128-code)
"#;
        let argv = serde_json::to_string(
            &std::iter::once(program.to_owned())
                .chain(args.iter().cloned())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        self.command(&format!(
            "cd -- {} && python3 -u -c {} {}",
            quote(&self.directory),
            quote(WORKER),
            quote(&argv)
        ))
    }
    pub async fn action(&self, op: &str, args: Value, full_access: bool) -> Result<Value, String> {
        let mut command = self.command(&format!("python3 -c {}", quote(HELPER)));
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|e| format!("无法启动系统 SSH：{e}"))?;
        let payload = serde_json::to_vec(
            &json!({"root":self.directory,"op":op,"args":args,"full_access":full_access}),
        )
        .map_err(|e| e.to_string())?;
        let result = tokio::time::timeout(std::time::Duration::from_secs(60), async {
            let mut stdin = child.stdin.take().unwrap();
            stdin.write_all(&payload).await.map_err(|e| e.to_string())?;
            drop(stdin);
            child.wait_with_output().await.map_err(|e| e.to_string())
        })
        .await
        .map_err(|_| "远程操作超过 60 秒，执行结果未确认；请先核对远端状态".to_owned())??;
        if !result.status.success() {
            return Err(format!(
                "SSH 连接或远程操作失败：{}。请确认 SSH 密钥/ssh-agent、主机配置及远端 python3 可用",
                String::from_utf8_lossy(&result.stderr)
                    .chars()
                    .take(2000)
                    .collect::<String>()
                    .trim()
            ));
        }
        let response: Value = serde_json::from_slice(&result.stdout)
            .map_err(|_| "远程响应无效，请检查登录脚本是否向标准输出打印内容".to_owned())?;
        if response["ok"] != true {
            return Err(response["error"].as_str().unwrap_or("远程操作失败").into());
        }
        Ok(response["value"].clone())
    }
    pub async fn connect(mut self) -> Result<Self, String> {
        let value = self.action("probe", json!({}), false).await?;
        self.directory = value["root"].as_str().ok_or("远程目录响应无效")?.into();
        Ok(self)
    }
}
pub fn label(path: &Path) -> String {
    if is_remote(path) {
        return Remote::from_path(path)
            .map(|r| r.label())
            .unwrap_or_else(|_| path.display().to_string());
    }
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[cfg(unix)]
    #[test]
    fn remote_undo_checks_the_batch_and_restores_or_removes_recorded_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let a = root.join("a.rs");
        let b = root.join("b.rs");
        std::fs::write(&a, "after").unwrap();
        std::fs::write(&b, "external").unwrap();
        let changes = json!({"changes":[{"path":a,"before":"before","after":"after"},{"path":b,"before":null,"after":"created"}]});
        assert!(helper(&root, "undo_changes", changes.clone(), true).is_err());
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "after");
        std::fs::write(&b, "created").unwrap();
        let result = helper(&root, "undo_changes", changes.clone(), true).unwrap();
        assert_eq!(result["restored"].as_array().unwrap().len(), 2);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "before");
        assert!(!b.exists());
        assert_eq!(
            helper(&root, "undo_changes", changes, true).unwrap()["restored"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    #[cfg(unix)]
    #[test]
    fn remote_hover_excerpt_is_bounded_and_reads_only_requested_lines() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("long.rs"),
            format!("first\n{}\nlast\n", "汉".repeat(400_000)),
        )
        .unwrap();
        let value = helper(
            dir.path(),
            "preview_excerpt",
            json!({"path":"long.rs","start_line":2,"lines":1}),
            true,
        )
        .unwrap();
        let text = value.as_str().unwrap();
        assert!(text.len() <= 4097);
        assert!(text.starts_with("汉"));
        assert!(!text.contains("last"));
        assert_eq!(
            helper(
                dir.path(),
                "preview_excerpt",
                json!({"path":"long.rs","start_line":3,"lines":1}),
                true
            )
            .unwrap(),
            "last\n"
        );
    }
    #[cfg(unix)]
    fn helper(root: &Path, op: &str, args: Value, full: bool) -> Result<Value, String> {
        let mut child = std::process::Command::new("python3")
            .args(["-c", HELPER])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let payload =
            json!({"root":root.to_string_lossy(), "op":op, "args":args, "full_access":full});
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let value: Value = serde_json::from_slice(&result.stdout).unwrap();
        if value["ok"] == true {
            Ok(value["value"].clone())
        } else {
            Err(value["error"].as_str().unwrap().into())
        }
    }

    #[cfg(unix)]
    #[test]
    fn remote_edit_plan_is_read_only_and_commit_detects_a_concurrent_change() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("file");
        std::fs::write(&path, "old").unwrap();
        let read = helper(
            &root,
            "read_file",
            json!({"path":"file","include_metadata":true}),
            false,
        )
        .unwrap();
        assert_eq!(read["expected_sha256"], crate::atomic_file::digest("old"));
        let plan=helper(&root,"prepare_edit",json!({"path":"file","kind":"write_file","content":"new","expected_sha256":read["expected_sha256"]}),false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old");
        let commit = json!({"path":"file","canonical_path":plan["path"],"expected_sha256":plan["sha256"],"expected_absent":false,"content":plan["after"]});
        std::fs::write(&path, "external").unwrap();
        assert!(helper(&root, "commit_edit", commit.clone(), false).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        std::fs::write(&path, "old").unwrap();
        helper(&root, "commit_edit", commit, false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn remote_preview_distinguishes_directories_text_and_binary() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("file.rs"), "你好\nsource\n").unwrap();
        std::fs::write(dir.path().join("binary.bin"), [0, 255, 0]).unwrap();
        let kinds = helper(
            dir.path(),
            "preview_stat",
            json!({"paths":["src","file.rs","binary.bin","gone"]}),
            true,
        )
        .unwrap();
        assert_eq!(kinds["src"], "directory");
        assert_eq!(kinds["file.rs"], "text");
        assert!(kinds["binary.bin"].is_null());
        assert!(kinds["gone"].is_null());
        let text = helper(dir.path(), "preview", json!({"path":"file.rs"}), true).unwrap();
        assert_eq!(text["content"], "你好\nsource\n");
        let listing = helper(dir.path(), "preview", json!({"path":"."}), true).unwrap();
        assert_eq!(listing["entries"][0]["name"], "src");
        assert!(
            listing["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["name"] == "binary.bin" && v["previewable"] == false)
        );
        assert!(helper(dir.path(), "preview", json!({"path":"binary.bin"}), true).is_err());
    }

    #[test]
    fn ssh_identity_parses_commands_ports_and_round_trips_without_local_paths() {
        let remote = Remote::parse("ssh user@example.com", "/home/user/my project").unwrap();
        assert_eq!(remote.target, "user@example.com");
        assert_eq!(Remote::from_path(&remote.uri()).unwrap(), remote);
        assert!(is_remote(&remote.uri()));
        let root = Remote::parse("ssh user@host", "/").unwrap();
        assert_eq!(Remote::from_path(&root.uri()).unwrap(), root);
        let port = Remote::parse("ssh -p 2222 my-host", "~").unwrap();
        assert_eq!(Remote::from_path(&port.uri()).unwrap(), port);
        assert_eq!(port.port, Some(2222));
        assert_eq!(
            Remote::parse("ssh://user@host:2200/home/user/demo", "~")
                .unwrap()
                .directory,
            "/home/user/demo"
        );
        for invalid in [
            "ssh -o ProxyCommand=bad host",
            "ssh host touch bad",
            "ssh -p 0 host",
            "ssh host;bad",
            "-bad",
            "user@@host",
        ] {
            assert!(Remote::parse(invalid, "~").is_err(), "{invalid}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn ssh_command_preserves_argv_and_remote_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        let remote = Remote::parse("ssh -p 2222 test@host", dir.path().to_str().unwrap()).unwrap();
        let literal = "a 'quoted' value $(touch unexpected) ; $HOME `whoami`";
        let command = remote.run(
            "python3",
            &[
                "-c".into(),
                "import os,sys; print(os.getcwd()); print(sys.argv[1])".into(),
                literal.into(),
            ],
        );
        let args: Vec<_> = command.as_std().get_args().collect();
        assert!(args.iter().any(|v| *v == "BatchMode=yes"));
        assert!(args.iter().any(|v| *v == "2222"));
        let result = std::process::Command::new("sh")
            .arg("-c")
            .arg(args.last().unwrap())
            .output()
            .unwrap();
        assert!(result.status.success());
        let text = String::from_utf8(result.stdout).unwrap();
        assert!(text.contains(literal));
        assert!(!dir.path().join("unexpected").exists());
    }

    #[test]
    #[cfg(unix)]
    fn remote_helper_reads_writes_replaces_indexes_and_enforces_project_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "outside").unwrap();
        let name = "中文 ' file.txt";
        let written = helper(
            dir.path(),
            "write_file",
            json!({"path":name,"content":"one\ntwo\n"}),
            false,
        )
        .unwrap();
        assert_eq!(written["before"], Value::Null);
        assert_eq!(
            helper(
                dir.path(),
                "read_file",
                json!({"path":name,"start_line":2,"lines":1}),
                false
            )
            .unwrap(),
            "2: two"
        );
        assert!(
            helper(
                dir.path(),
                "replace_in_file",
                json!({"path":name,"old":"missing","new":"bad"}),
                false
            )
            .is_err()
        );
        helper(
            dir.path(),
            "replace_in_file",
            json!({"path":name,"old":"two","new":"updated"}),
            false,
        )
        .unwrap();
        assert!(
            helper(
                dir.path(),
                "search_files",
                json!({"pattern":"updated","glob":"*.txt"}),
                false
            )
            .unwrap()
            .as_str()
            .unwrap()
            .contains("updated")
        );
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/hidden"), "hidden").unwrap();
        let index = helper(dir.path(), "index", json!({}), false).unwrap();
        assert_eq!(index["entries"].as_array().unwrap().len(), 1);
        assert!(
            helper(
                dir.path(),
                "read_file",
                json!({"path":outside.path().join("secret")}),
                false
            )
            .is_err()
        );
        assert!(
            helper(
                dir.path(),
                "write_file",
                json!({"path":"../escape","content":"bad"}),
                false
            )
            .is_err()
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).unwrap();
            assert!(
                helper(
                    dir.path(),
                    "read_file",
                    json!({"path":"linked/secret"}),
                    false
                )
                .is_err()
            );
            assert!(
                helper(
                    dir.path(),
                    "write_file",
                    json!({"path":"linked/secret","content":"bad"}),
                    false
                )
                .is_err()
            );
            assert_eq!(
                helper(
                    dir.path(),
                    "read_file",
                    json!({"path":"linked/secret"}),
                    true
                )
                .unwrap(),
                "1: outside"
            );
        }
        let mut remote = Remote::parse("ssh test@host", "/remote/project").unwrap();
        let edited = helper(
            dir.path(),
            "write_file",
            json!({"path":name,"content":"changed"}),
            false,
        )
        .unwrap();
        remote.directory = "/remote/project".into();
        let change = crate::edits::remote_change(remote, name, edited)
            .unwrap()
            .1
            .unwrap();
        assert!(is_remote(&change.path));
        assert!(change.added > 0 && change.removed > 0);
    }

    #[test]
    #[cfg(unix)]
    fn remote_git_changes_include_modified_deleted_and_untracked_files() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let result = std::process::Command::new("git")
                .current_dir(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        };
        git(&["init", "-q"]);
        std::fs::write(dir.path().join("edited"), "before\n").unwrap();
        std::fs::write(dir.path().join("deleted"), "deleted\n").unwrap();
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "initial",
        ]);
        std::fs::write(dir.path().join("edited"), "after\n").unwrap();
        std::fs::remove_file(dir.path().join("deleted")).unwrap();
        std::fs::write(dir.path().join("new"), "new\n").unwrap();
        let result = helper(dir.path(), "workspace_changes", json!({}), false).unwrap();
        let rows = result["changes"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        let edited = rows.iter().find(|r| r["label"] == "edited").unwrap();
        assert_eq!(edited["before"], "before\n");
        assert_eq!(edited["after"], "after\n");
    }
}
