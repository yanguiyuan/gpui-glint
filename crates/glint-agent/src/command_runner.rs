//! Managed commands: bounded previews, private logs, scoped polling and cancellation.
use crate::{
    database, ssh_project,
    tools::{ToolContext, ToolEvent},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    sync::{Notify, watch},
};
const WINDOW: usize = 32 * 1024;
const LOG_LIMIT: u64 = 32 * 1024 * 1024;

/// Spawn failures are reported before the process exists, so the message can
/// explain PATH instead of only echoing the OS error.
fn spawn_error(program: &str, error: &std::io::Error) -> String {
    if error.kind() != std::io::ErrorKind::NotFound {
        return format!("无法启动命令：{error}");
    }
    let searched = crate::user_path::effective().len();
    format!(
        "无法启动命令：{error}。在 {searched} 个 PATH 目录中找不到 `{program}`；可改用绝对路径，或确认该工具已安装（Finder 启动的应用不继承终端 PATH，Glint 已尝试登录 shell 与常见安装目录）"
    )
}

fn environment_error(stderr: &str) -> bool {
    let text = stderr.to_ascii_lowercase();
    ["shim: could not create process", "untrusted mount point", "the path cannot be traversed"]
        .iter().any(|message| text.contains(message))
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct Scope {
    anchor: PathBuf,
    root: Option<PathBuf>,
    partner: Option<uuid::Uuid>,
    session: Option<uuid::Uuid>,
}
impl Scope {
    fn new(c: &ToolContext) -> Self {
        Self {
            anchor: c.memory.anchor.clone(),
            root: c.root.clone(),
            partner: c.memory.partner,
            session: c.session_id,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ResultData {
    pub run_id: uuid::Uuid,
    pub status: String,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub elapsed_ms: u64,
    pub stdout: String,
    pub stderr: String,
    pub output_truncated: [bool; 2],
    pub log_truncated: [bool; 2],
    pub bytes: [u64; 2],
    pub stdout_log: PathBuf,
    pub stderr_log: PathBuf,
    pub error: Option<String>,
    pub remote_state: Option<String>,
    #[serde(default)]
    pub failure_kind: Option<String>,
    #[serde(default)]
    pub environment_unavailable: bool,
    #[serde(default)]
    pub resolved_program: Option<PathBuf>,
    #[serde(default)]
    pub retry_count: u8,
}
#[derive(Clone, Serialize, Deserialize)]
struct Stored {
    scope: Scope,
    result: ResultData,
}
#[derive(Default)]
struct Buffer {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: u64,
}
impl Buffer {
    fn push(&mut self, bytes: &[u8]) {
        self.total += bytes.len() as u64;
        let n = (WINDOW - self.head.len()).min(bytes.len());
        self.head.extend_from_slice(&bytes[..n]);
        for byte in &bytes[n..] {
            if self.tail.len() == WINDOW {
                self.tail.pop_front();
            }
            self.tail.push_back(*byte);
        }
    }
    fn text(&self) -> String {
        let mut text = String::from_utf8_lossy(&self.head).into_owned();
        if self.total > 2 * WINDOW as u64 {
            text.push_str("\n…输出已截断，可通过 command_poll 分段读取日志…\n");
        }
        text.push_str(&String::from_utf8_lossy(
            &self.tail.iter().copied().collect::<Vec<_>>(),
        ));
        text
    }
}
struct State {
    result: ResultData,
    buffers: [Buffer; 2],
    dirty: bool,
}
struct Run {
    scope: Scope,
    call_id: Option<String>,
    state: Mutex<State>,
    cancel: watch::Sender<bool>,
    done: Notify,
    finished: AtomicBool,
    manual_cancel: AtomicBool,
    started: Instant,
}
fn registry() -> &'static Mutex<HashMap<uuid::Uuid, Arc<Run>>> {
    static RUNS: OnceLock<Mutex<HashMap<uuid::Uuid, Arc<Run>>>> = OnceLock::new();
    RUNS.get_or_init(Default::default)
}

/// UI cancellation is scoped to the conversation and the exact tool call.
pub fn cancel_call(anchor: &Path, session: uuid::Uuid, call_id: &str) -> bool {
    let runs = registry().lock().unwrap();
    let mut matching = runs.values().filter(|run| run.scope.anchor == anchor
        && run.scope.session == Some(session) && run.call_id.as_deref() == Some(call_id)
        && !run.finished.load(Ordering::Acquire));
    let Some(run) = matching.next() else { return false; };
    if matching.next().is_some() { return false; }
    run.manual_cancel.store(true, Ordering::Release);
    run.cancel.send(true).is_ok()
}

pub fn call_cancelling(anchor: &Path, session: uuid::Uuid, call_id: &str) -> bool {
    registry().lock().unwrap().values().any(|run| run.scope.anchor == anchor
        && run.scope.session == Some(session) && run.call_id.as_deref() == Some(call_id)
        && *run.cancel.borrow())
}
fn store(scope: &Scope, result: &ResultData) -> Result<(), String> {
    database::save_command_run(
        &scope.anchor,
        result.run_id,
        &serde_json::to_value(Stored {
            scope: scope.clone(),
            result: result.clone(),
        })
        .map_err(|e| e.to_string())?,
    )
}
fn prune(anchor: &Path, current: uuid::Uuid) {
    let active: std::collections::HashSet<_> = registry()
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, run)| !run.finished.load(Ordering::Acquire))
        .map(|(id, _)| *id)
        .chain(std::iter::once(current))
        .collect();
    let directory = anchor
        .parent()
        .unwrap_or(Path::new("."))
        .join("command-runs");
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut completed: Vec<_> = entries
        .flatten()
        .filter_map(|entry| {
            let id = uuid::Uuid::parse_str(&entry.file_name().to_string_lossy()).ok()?;
            if active.contains(&id) || !entry.file_type().ok()?.is_dir() {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, id, entry.path()))
        })
        .collect();
    completed.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let mut bytes = ["stdout.log", "stderr.log"]
        .iter()
        .filter_map(|name| {
            std::fs::metadata(
                anchor
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join("command-runs")
                    .join(current.to_string())
                    .join(name),
            )
            .ok()
        })
        .map(|m| m.len())
        .sum::<u64>();
    for (index, (_, id, path)) in completed.into_iter().enumerate() {
        let size = ["stdout.log", "stderr.log"]
            .iter()
            .filter_map(|name| std::fs::metadata(path.join(name)).ok())
            .map(|m| m.len())
            .sum::<u64>();
        bytes += size;
        if index >= 127 || bytes > 512 * 1024 * 1024 {
            if std::fs::remove_dir_all(path).is_ok() {
                let _ = database::remove_command_run(anchor, id);
                registry().lock().unwrap().remove(&id);
            }
        }
    }
}
fn snapshot(run: &Run) -> ResultData {
    let state = run.state.lock().unwrap_or_else(|e| e.into_inner());
    let mut result = state.result.clone();
    result.stdout = state.buffers[0].text();
    result.stderr = state.buffers[1].text();
    if result.status == "running" {
        result.elapsed_ms = run.started.elapsed().as_millis() as u64;
    }
    result
}
struct AbortTask(tokio::task::JoinHandle<()>);
impl Drop for AbortTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
fn publisher(
    run: Arc<Run>,
    events: Option<(String, async_channel::Sender<ToolEvent>)>,
) -> Option<AbortTask> {
    let (call_id, events) = events?;
    Some(AbortTask(tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_millis(80));
        loop {
            timer.tick().await;
            let output = {
                let mut state = run.state.lock().unwrap();
                if !state.dirty || events.len() > 16 {
                    continue;
                }
                state.dirty = false;
                format!("{}\n{}", state.buffers[0].text(), state.buffers[1].text())
            };
            let _ = events.try_send(ToolEvent::Output {
                call_id: call_id.clone(),
                output,
            });
        }
    })))
}
async fn wait(run: &Run) {
    loop {
        let notified = run.done.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if run.finished.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }
}
struct Waiter {
    cancel: watch::Sender<bool>,
    armed: bool,
}
impl Drop for Waiter {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.cancel.send(true);
        }
    }
}
struct Group {
    pid: u32,
}
impl Group {
    fn terminate(&self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.pid as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        {
            let _ = crate::child_process::command("taskkill")
                .args(["/PID", &self.pid.to_string(), "/T", "/F"])
                .output();
        }
    }
}
impl Drop for Group {
    fn drop(&mut self) {
        self.terminate();
    }
}
async fn capture(
    mut reader: impl AsyncRead + Unpin,
    stream: usize,
    run: Arc<Run>,
) -> Result<(), String> {
    let path = {
        let state = run.state.lock().unwrap();
        if stream == 0 {
            state.result.stdout_log.clone()
        } else {
            state.result.stderr_log.clone()
        }
    };
    let mut file = if run.scope.anchor.as_os_str().is_empty() {
        None
    } else {
        Some(
            tokio::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .map_err(|e| e.to_string())?,
        )
    };
    let mut bytes = [0; 8192];
    let mut logged = 0u64;
    loop {
        let count = reader.read(&mut bytes).await.map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        let keep = (LOG_LIMIT - logged).min(count as u64) as usize;
        if keep > 0 {
            if let Some(file) = file.as_mut() {
                file.write_all(&bytes[..keep])
                    .await
                    .map_err(|e| e.to_string())?;
            }
            logged += keep as u64;
        }
        let mut state = run.state.lock().unwrap();
        state.buffers[stream].push(&bytes[..count]);
        state.result.bytes[stream] = state.buffers[stream].total;
        state.result.output_truncated[stream] = state.buffers[stream].total > 2 * WINDOW as u64;
        state.result.log_truncated[stream] = state.buffers[stream].total > LOG_LIMIT;
        state.dirty = true;
    }
    if let Some(mut file) = file {
        file.flush().await.map_err(|e| e.to_string())?;
    }
    Ok(())
}
async fn worker(
    run: Arc<Run>,
    _permit: tokio::sync::OwnedSemaphorePermit,
    cwd: PathBuf,
    program: String,
    args: Vec<String>,
    seconds: u64,
    mut cancel: watch::Receiver<bool>,
    events: Option<(String, async_channel::Sender<ToolEvent>)>,
) {
    let started = run.started;
    let _publisher = publisher(run.clone(), events.clone());
    let remote = ssh_project::is_remote(&cwd);
    let mut spawn_attempted = false;
    let mut spawned = false;
    let work = async {
        if *cancel.borrow() {
            return Err("启动前已取消".to_string());
        }
        let target = if remote {
            PathBuf::from(&program)
        } else {
            crate::executable::resolve(&program, &cwd)
        };
        if !remote {
            run.state.lock().unwrap().result.resolved_program = Some(target.clone());
        }
        let mut command = if remote {
            ssh_project::Remote::from_path(&cwd)?.managed_run(&program, &args)
        } else {
            let mut c = crate::child_process::async_command(&target);
            c.args(&args).current_dir(&cwd);
            c
        };
        command
            .stdin(if remote {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            command.process_group(0);
        }
        for (name, _) in std::env::vars() {
            if name.ends_with("_KEY")
                || name.ends_with("_TOKEN")
                || name.ends_with("_SECRET")
                || name == "deepseek_api_key"
            {
                command.env_remove(name);
            }
        }
        // The native log view understands ANSI styling even though pipes are
        // used for capture. Ask common tools to retain their colored output.
        command.env_remove("NO_COLOR")
            .env("TERM", "xterm-256color")
            .env("FORCE_COLOR", "1")
            .env("CLICOLOR_FORCE", "1")
            .env("CARGO_TERM_COLOR", "always");
        if !remote {
            crate::vault_environment::with_variables(|name, value| {
                command.env(name, value);
            });
            // A Finder-launched application only has launchd's minimal PATH, but
            // children (cargo -> rustc, build scripts, hooks) need the toolchains
            // the user's shell configuration adds.
            let mut directories = Vec::new();
            if let Some(rg) = crate::executable::bundled_rg() {
                directories.push(rg.parent().unwrap().to_owned());
            }
            directories.extend(crate::user_path::effective().iter().cloned());
            if let Ok(path) = std::env::join_paths(directories) {
                command.env("PATH", path);
            }
        }
        spawn_attempted = true;
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(first) if !remote => {
                // A failed spawn has no command effects. Retry only if a fresh
                // resolution finds a different executable; never replay an exit.
                let retry = crate::executable::resolve(&program, &cwd);
                if retry == target || *cancel.borrow() {
                    return Err(spawn_error(&program, &first));
                }
                let mut fallback = crate::child_process::async_command(&retry);
                fallback.args(&args).current_dir(&cwd)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped()).kill_on_drop(true);
                #[cfg(unix)]
                fallback.process_group(0);
                // Reuse the exact sanitized environment of the first attempt.
                for (key, value) in command.as_std().get_envs() {
                    if let Some(value) = value { fallback.env(key, value); }
                    else { fallback.env_remove(key); }
                }
                let mut state = run.state.lock().unwrap();
                state.result.retry_count = 1;
                state.result.resolved_program = Some(retry);
                drop(state);
                fallback.spawn().map_err(|e| format!("无法启动命令：{e}（首次错误：{first}）"))?
            }
            Err(error) => return Err(format!("无法启动命令：{error}")),
        };
        spawned = true;
        let group = Group {
            pid: child.id().ok_or("命令未获得进程 ID")?,
        };
        let stdin = child.stdin.take();
        let mut out = tokio::spawn(capture(child.stdout.take().unwrap(), 0, run.clone()));
        let mut err = tokio::spawn(capture(child.stderr.take().unwrap(), 1, run.clone()));
        let (status, reason) = tokio::select! {
          status=child.wait()=>(Some(status.map_err(|e|e.to_string())?),None),
          _=tokio::time::sleep(Duration::from_secs(seconds))=>(None,Some("timed_out")),
          _=cancel.changed()=>(None,Some("cancelled")),
        };
        drop(stdin);
        if reason.is_some() {
            if remote {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            group.terminate();
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        let drained = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(&mut out, &mut err)
        })
        .await;
        let mut error = None;
        match drained {
            Ok((out, err)) => {
                for result in [out, err] {
                    match result {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => error = Some(e),
                        Err(e) => error = Some(e.to_string()),
                    }
                }
            }
            Err(_) => {
                group.terminate();
                out.abort();
                err.abort();
                let _ = out.await;
                let _ = err.await;
                error = Some("子进程持续占用输出流，已清理进程组".into());
            }
        }
        let mut state = run.state.lock().unwrap();
        state.result.exit_code = status.as_ref().and_then(|s| s.code());
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            state.result.signal = status.as_ref().and_then(|s| s.signal());
        }
        state.result.status = reason
            .unwrap_or(
                if status.as_ref().is_some_and(|s| s.success()) && error.is_none() {
                    "completed"
                } else {
                    "failed"
                },
            )
            .into();
        if remote && (reason.is_some() || state.result.exit_code == Some(255)) {
            state.result.remote_state =
                Some("SSH 连接已关闭；远程停止结果未确认，请核对远端状态".into());
        }
        state.result.error = error;
        if reason == Some("cancelled") && run.manual_cancel.load(Ordering::Acquire) {
            state.result.error = Some("用户手动终止了此命令；不要自动重试。".into());
        }
        state.result.failure_kind = match state.result.status.as_str() {
            "timed_out" => Some("timeout".into()),
            "cancelled" => Some("cancelled".into()),
            "failed" if state.result.exit_code.is_some_and(|code| code != 0) => Some("nonzero_exit".into()),
            "failed" => Some("io_error".into()),
            _ => None,
        };
        if state.result.status == "failed" {
            state.result.environment_unavailable = environment_error(&state.buffers[1].text());
        }
        Ok::<(), String>(())
    }
    .await;
    {
        let mut state = run.state.lock().unwrap();
        if let Err(error) = work {
            state.result.status = if *cancel.borrow() {
                "cancelled"
            } else {
                "failed"
            }
            .into();
            state.result.error = Some(if run.manual_cancel.load(Ordering::Acquire) {
                "用户手动终止了此命令；不要自动重试。".into()
            } else { error });
            state.result.failure_kind = Some(if *cancel.borrow() {
                "cancelled"
            } else if spawn_attempted && !spawned {
                "spawn_failed"
            } else {
                "io_error"
            }.into());
            state.result.environment_unavailable = spawn_attempted && !spawned;
        }
        state.result.stdout = state.buffers[0].text();
        state.result.stderr = state.buffers[1].text();
        state.result.elapsed_ms = started.elapsed().as_millis() as u64;
    }
    let result = snapshot(&run);
    let scope = run.scope.clone();
    let saved = result.clone();
    if !scope.anchor.as_os_str().is_empty() {
        let anchor = scope.anchor.clone();
        let persisted = tokio::task::spawn_blocking(move || {
            store(&scope, &saved)?;
            prune(&anchor, saved.run_id);
            Ok::<(), String>(())
        })
        .await;
        if !matches!(persisted, Ok(Ok(()))) {
            run.state.lock().unwrap().result.error =
                Some("命令已结束，但最终状态保存失败；重启后需核对日志".into());
        }
    }
    if let Some((call_id, events)) = events.filter(|(_, events)| events.len() <= 16) {
        let _ = events.try_send(ToolEvent::Output {
            call_id,
            output: format!("{}\n{}", result.stdout, result.stderr),
        });
    }
    run.finished.store(true, Ordering::Release);
    run.done.notify_waiters();
}
async fn start(
    scope: Scope,
    cwd: PathBuf,
    program: String,
    args: Vec<String>,
    seconds: u64,
    events: Option<(String, async_channel::Sender<ToolEvent>)>,
) -> Result<Arc<Run>, String> {
    static SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let permit = SLOTS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(32)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| "同时运行的命令已达到 32 个")?;
    let persistent = !scope.anchor.as_os_str().is_empty();
    let id = uuid::Uuid::new_v4();
    let directory = scope
        .anchor
        .parent()
        .unwrap_or(Path::new("."))
        .join("command-runs")
        .join(id.to_string());
    let dir = directory.clone();
    if persistent {
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    dir.parent().unwrap(),
                    std::fs::Permissions::from_mode(0o700),
                )
                .map_err(|e| e.to_string())?;
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                    .map_err(|e| e.to_string())?;
            }
            for name in ["stdout.log", "stderr.log"] {
                let _file = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(dir.join(name))
                    .map_err(|e| e.to_string())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    _file.set_permissions(std::fs::Permissions::from_mode(0o600))
                        .map_err(|e| e.to_string())?;
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())??;
    }
    let (cancel, receiver) = watch::channel(false);
    let run = Arc::new(Run {
        scope,
        call_id: events.as_ref().map(|(call_id, _)| call_id.clone()),
        state: Mutex::new(State {
            result: ResultData {
                run_id: id,
                status: "running".into(),
                exit_code: None,
                signal: None,
                elapsed_ms: 0,
                stdout: String::new(),
                stderr: String::new(),
                bytes: [0; 2],
                output_truncated: [false; 2],
                log_truncated: [false; 2],
                stdout_log: directory.join("stdout.log"),
                stderr_log: directory.join("stderr.log"),
                error: None,
                remote_state: None,
                failure_kind: None,
                environment_unavailable: false,
                resolved_program: None,
                retry_count: 0,
            },
            buffers: Default::default(),
            dirty: false,
        }),
        cancel,
        done: Notify::new(),
        finished: AtomicBool::new(false),
        manual_cancel: AtomicBool::new(false),
        started: Instant::now(),
    });
    let scope = run.scope.clone();
    let result = snapshot(&run);
    if persistent {
        let stored = tokio::task::spawn_blocking(move || store(&scope, &result))
            .await
            .map_err(|e| e.to_string())?;
        if let Err(error) = stored {
            let _ = std::fs::remove_dir_all(&directory);
            return Err(error);
        }
        let mut runs = registry().lock().unwrap();
        if runs.len() >= 128 {
            if let Some(id) = runs
                .iter()
                .find(|(_, r)| r.finished.load(Ordering::Acquire))
                .map(|(id, _)| *id)
            {
                runs.remove(&id);
            }
        }
        runs.insert(id, run.clone());
    }
    let owned = run.clone();
    tokio::spawn(async move {
        use futures_util::FutureExt;
        if std::panic::AssertUnwindSafe(worker(
            owned.clone(),
            permit,
            cwd,
            program,
            args,
            seconds,
            receiver,
            events,
        ))
        .catch_unwind()
        .await
        .is_err()
        {
            {
                let mut state = owned.state.lock().unwrap_or_else(|e| e.into_inner());
                state.result.status = "failed".into();
                state.result.error = Some("命令工作任务意外中断，请核对日志和实际执行结果".into());
                state.result.failure_kind = Some("io_error".into());
            }
            let scope = owned.scope.clone();
            let result = snapshot(&owned);
            if !scope.anchor.as_os_str().is_empty() {
                let _ = tokio::task::spawn_blocking(move || store(&scope, &result)).await;
            }
            owned.finished.store(true, Ordering::Release);
            owned.done.notify_waiters();
        }
    });
    Ok(run)
}
async fn foreground_scope(
    scope: Scope,
    cwd: PathBuf,
    program: String,
    args: Vec<String>,
    seconds: u64,
    background: bool,
    events: Option<(String, async_channel::Sender<ToolEvent>)>,
) -> Result<ResultData, String> {
    let run = start(scope, cwd, program, args, seconds, events).await?;
    let mut waiter = Waiter {
        cancel: run.cancel.clone(),
        armed: !background,
    };
    if !background {
        wait(&run).await;
        waiter.armed = false;
    }
    Ok(snapshot(&run))
}
pub async fn foreground(
    context: &ToolContext,
    cwd: PathBuf,
    program: String,
    args: Vec<String>,
    seconds: u64,
    background: bool,
    events: Option<(String, async_channel::Sender<ToolEvent>)>,
) -> Result<ResultData, String> {
    foreground_scope(
        Scope::new(context),
        cwd,
        program,
        args,
        seconds,
        background,
        events,
    )
    .await
}
pub async fn poll(
    context: &ToolContext,
    args: &serde_json::Value,
    stop: bool,
) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Poll {
        run_id: uuid::Uuid,
        wait_ms: Option<u64>,
        stream: Option<String>,
        offset: Option<u64>,
        limit: Option<u64>,
    }
    let parsed: Poll =
        serde_json::from_value(args.clone()).map_err(|e| format!("命令查询参数无效：{e}"))?;
    if parsed.wait_ms.is_some_and(|n| n > 5000)
        || parsed.limit.is_some_and(|n| !(1..=65536).contains(&n))
    {
        return Err("wait_ms 必须为 0..5000，limit 必须为 1..65536".into());
    }
    if parsed.stream.is_none() && (parsed.offset.is_some() || parsed.limit.is_some()) {
        return Err("读取分页日志时必须指定 stream".into());
    }
    if stop && (parsed.wait_ms.is_some() || parsed.stream.is_some()) {
        return Err("command_stop 只接受 run_id".into());
    }
    let id = parsed.run_id;
    let scope = Scope::new(context);
    let active = registry().lock().unwrap().get(&id).cloned();
    let result = if let Some(run) = active {
        if run.scope != scope {
            return Err("命令不属于当前会话、伙伴或项目".into());
        }
        if stop {
            let _ = run.cancel.send(true);
            wait(&run).await;
        } else if snapshot(&run).status == "running" {
            let wait = args["wait_ms"].as_u64().unwrap_or(0).min(5000);
            if wait > 0 {
                let _ = tokio::time::timeout(Duration::from_millis(wait), self::wait(&run)).await;
            }
        }
        snapshot(&run)
    } else {
        let anchor = scope.anchor.clone();
        let value = tokio::task::spawn_blocking(move || database::command_run(&anchor, id))
            .await
            .map_err(|e| e.to_string())??
            .ok_or("命令记录不存在")?;
        let saved: Stored = serde_json::from_value(value).map_err(|e| e.to_string())?;
        if saved.scope != scope {
            return Err("命令不属于当前会话、伙伴或项目".into());
        }
        let mut result = saved.result;
        if result.status == "running" {
            result.status = "interrupted".into();
            result.error = Some("此命令不在当前进程中运行，实际状态未确认，请核对日志".into());
        }
        result
    };
    let mut value = serde_json::to_value(&result).map_err(|e| e.to_string())?;
    if let Some(stream) = args["stream"].as_str() {
        let path = match stream {
            "stdout" => result.stdout_log,
            "stderr" => result.stderr_log,
            _ => return Err("stream 必须为 stdout 或 stderr".into()),
        };
        let offset = args["offset"].as_u64().unwrap_or(0);
        let limit = args["limit"].as_u64().unwrap_or(16384).clamp(1, 65536);
        value["page"]=tokio::task::spawn_blocking(move||->Result<serde_json::Value,String>{use std::io::{Read,Seek,SeekFrom};let mut file=std::fs::File::open(path).map_err(|e|e.to_string())?;let total=file.metadata().map_err(|e|e.to_string())?.len();if offset>total{return Err("日志 offset 超出范围".into());}file.seek(SeekFrom::Start(offset)).map_err(|e|e.to_string())?;let mut bytes=Vec::new();file.take(limit).read_to_end(&mut bytes).map_err(|e|e.to_string())?;let end=offset+bytes.len() as u64;Ok(serde_json::json!({"content":String::from_utf8_lossy(&bytes),"offset":offset,"next_offset":(end<total).then_some(end),"total_bytes":total}))}).await.map_err(|e|e.to_string())??;
    }
    Ok(value.to_string())
}
pub fn cancel_anchor(anchor: &Path) {
    for run in registry().lock().unwrap().values() {
        if run.scope.anchor == anchor && !run.finished.load(Ordering::Acquire) {
            let _ = run.cancel.send(true);
        }
    }
}
pub async fn shutdown(anchor: PathBuf) {
    let runs: Vec<_> = registry()
        .lock()
        .unwrap()
        .values()
        .filter(|r| r.scope.anchor == anchor)
        .cloned()
        .collect();
    for run in &runs {
        let _ = run.cancel.send(true);
    }
    for run in runs {
        wait(&run).await;
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    program: String,
    args: Vec<String>,
    cwd: Option<String>,
    timeout_seconds: Option<u64>,
    #[serde(default)]
    background: bool,
}
pub async fn run(
    context: &ToolContext,
    args: &serde_json::Value,
    call_id: &str,
) -> Result<String, String> {
    let args: Args =
        serde_json::from_value(args.clone()).map_err(|e| format!("命令参数无效：{e}"))?;
    let seconds = args
        .timeout_seconds
        .unwrap_or(if args.background { 3600 } else { 60 });
    if !(1..=86400).contains(&seconds) {
        return Err("timeout_seconds 必须为 1..86400".into());
    }
    let access = *context.access.lock().map_err(|_| "无法读取权限")?;
    let root = context
        .root
        .clone()
        .or_else(|| {
            (access == crate::tools::AccessMode::FullAccess)
                .then(|| std::env::current_dir().ok())
                .flatten()
        })
        .ok_or("请先打开项目")?;
    let cwd = if let Some(cwd) = args.cwd {
        if ssh_project::is_remote(&root) {
            let mut remote = ssh_project::Remote::from_path(&root)?;
            let resolved = remote
                .action(
                    "resolve_directory",
                    serde_json::json!({"path":cwd}),
                    access == crate::tools::AccessMode::FullAccess,
                )
                .await?;
            remote.directory = resolved.as_str().ok_or("远程工作目录无效")?.into();
            remote.uri()
        } else {
            let path = crate::tools::resolve_file(context, &cwd, false)?;
            if !path.is_dir() {
                return Err("cwd 必须为目录".into());
            }
            path.canonicalize().map_err(|e| e.to_string())?
        }
    } else {
        root
    };
    let result = foreground(
        context,
        cwd,
        args.program,
        args.args,
        seconds,
        args.background,
        (!args.background).then(|| (call_id.into(), context.events.clone())),
    )
    .await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}
/// Search/legacy tests need no session registry. Keep this path managed too.
pub async fn internal(
    root: &Path,
    program: &str,
    args: &[String],
    events: Option<(&str, &async_channel::Sender<ToolEvent>)>,
) -> Result<String, String> {
    let result = foreground_scope(
        Scope {
            anchor: PathBuf::new(),
            root: Some(root.into()),
            session: None,
            partner: None,
        },
        root.into(),
        program.into(),
        args.to_vec(),
        60,
        false,
        events.map(|(id, sender)| (id.into(), sender.clone())),
    )
    .await?;
    if (program == "rg" || program == "rg.exe") && result.exit_code == Some(1)
        && result.stderr.trim().is_empty() && result.error.is_none() && !result.environment_unavailable
        && result.failure_kind.as_deref() == Some("nonzero_exit") {
        return Ok(String::new());
    }
    if result.status != "completed" {
        return Err(format!(
            "exit: {:?}; failure_kind: {}\n{}\n{}\n{}",
            result.exit_code, result.failure_kind.as_deref().unwrap_or("unknown"),
            result.stdout, result.stderr, result.error.as_deref().unwrap_or("")
        ));
    }
    Ok(format!(
        "exit: {}\n{}\n{}",
        result.exit_code.unwrap_or(0),
        result.stdout,
        result.stderr
    ))
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[tokio::test]
    async fn manual_stop_cancels_only_the_selected_call_and_preserves_output() {
        let temp = tempfile::tempdir().unwrap();
        let scope = Scope { anchor: temp.path().join("sessions.json"), root: Some(temp.path().into()), partner: None, session: Some(uuid::Uuid::new_v4()) };
        let (events, _) = async_channel::unbounded();
        let first = start(scope.clone(), temp.path().into(), "powershell.exe".into(), vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), "[Console]::WriteLine('before stop'); [IO.File]::WriteAllText('ready.txt','ready'); Start-Sleep -Seconds 30".into()], 60, Some(("first-call".into(), events.clone()))).await.unwrap();
        let other = start(scope.clone(), temp.path().into(), "powershell.exe".into(), vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), "Start-Sleep -Seconds 30".into()], 60, Some(("other-call".into(), events))).await.unwrap();
        let deadline = Instant::now()+Duration::from_secs(15);
        while !temp.path().join("ready.txt").exists() {
            assert!(Instant::now()<deadline, "test command did not start");
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        assert!(!cancel_call(&scope.anchor, uuid::Uuid::new_v4(), "first-call"));
        assert!(cancel_call(&scope.anchor, scope.session.unwrap(), "first-call"));
        tokio::time::timeout(Duration::from_secs(10), wait(&first)).await.unwrap();
        let result = snapshot(&first);
        assert_eq!(result.status, "cancelled");
        assert_eq!(result.failure_kind.as_deref(), Some("cancelled"));
        assert!(result.error.as_deref().unwrap().contains("用户手动终止"));
        assert!(result.stdout.contains("before stop"));
        assert!(!other.finished.load(Ordering::Acquire), "unrelated command must stay alive");
        other.cancel.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(10), wait(&other)).await.unwrap();
    }

    async fn local(root: &Path, program: &str, args: &[&str], timeout: u64) -> ResultData {
        foreground_scope(
            Scope { anchor: PathBuf::new(), root: Some(root.into()), partner: None, session: None },
            root.into(), program.into(), args.iter().map(|arg| (*arg).into()).collect(),
            timeout, false, None,
        ).await.unwrap()
    }

    #[tokio::test]
    async fn startup_exit_timeout_and_environment_failures_are_distinct() {
        let temp = tempfile::tempdir().unwrap();
        let missing = local(temp.path(), "glint-nonexistent-executable-82793.exe", &[], 5).await;
        assert_eq!(missing.failure_kind.as_deref(), Some("spawn_failed"));
        assert_eq!(missing.exit_code, None);
        assert!(missing.environment_unavailable);
        let failed = local(temp.path(), "cmd.exe", &["/D", "/C", "echo failed 1>&2 & exit /b 7"], 5).await;
        assert_eq!(failed.failure_kind.as_deref(), Some("nonzero_exit"));
        assert_eq!(failed.exit_code, Some(7));
        assert!(!failed.environment_unavailable);
        assert_eq!(failed.retry_count, 0);
        let environment = local(temp.path(), "cmd.exe", &["/D", "/C", "echo Shim: Could not create process 1>&2 & exit /b 1"], 5).await;
        assert!(environment.environment_unavailable);
        assert_eq!(environment.retry_count, 0, "never replay a command based on stderr text");
        let timeout = local(temp.path(), "powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", "Start-Sleep -Seconds 30"], 1).await;
        assert_eq!(timeout.failure_kind.as_deref(), Some("timeout"));
        assert_eq!(timeout.status, "timed_out");
    }

    #[tokio::test]
    #[cfg(target_arch = "x86_64")]
    async fn bundled_search_handles_matches_and_empty_results() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("file.txt"), "hello").unwrap();
        assert!(internal(temp.path(), "rg", &["hello".into(), "file.txt".into()], None).await.unwrap().contains("hello"));
        assert_eq!(internal(temp.path(), "rg", &["absent".into(), "file.txt".into()], None).await.unwrap(), "");
        let shell = local(temp.path(), "cmd.exe", &["/D", "/C", "rg --version"], 5).await;
        assert_eq!(shell.status, "completed");
        assert!(shell.stdout.starts_with("ripgrep 15.1.0"));
    }
    #[tokio::test]
    async fn managed_command_runs_without_a_console_and_keeps_both_output_streams() {
        let root = tempfile::tempdir().unwrap();
        let script = r#"Add-Type -Name ConsoleProbe -Namespace GlintTest -MemberDefinition '[System.Runtime.InteropServices.DllImport("kernel32.dll")] public static extern System.IntPtr GetConsoleWindow();'; [Console]::Write([GlintTest.ConsoleProbe]::GetConsoleWindow().ToInt64()); [Console]::Error.Write('managed stderr'); exit 7"#;
        let result = foreground_scope(
            Scope { anchor: PathBuf::new(), root: Some(root.path().to_owned()), partner: None, session: Some(uuid::Uuid::new_v4()) },
            root.path().to_owned(), "powershell.exe".into(),
            vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), script.into()],
            30, false, None,
        ).await.unwrap();
        assert_eq!(result.status, "failed");
        assert_eq!(result.exit_code, Some(7));
        assert_eq!(result.stdout, "0");
        assert_eq!(result.stderr, "managed stderr");
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    fn context(root: &Path) -> ToolContext {
        let mut c = crate::edits::tests::context(root);
        c.session_id = Some(uuid::Uuid::new_v4());
        c
    }
    async fn shell(c: &ToolContext, script: &str, background: bool, timeout: u64) -> Value {
        serde_json::from_str(&run(c,&json!({"program":"sh","args":["-c",script],"background":background,"timeout_seconds":timeout}),"test").await.unwrap()).unwrap()
    }
    #[tokio::test]
    async fn omitted_background_waits_for_completion_and_final_output() {
        let temp = tempfile::tempdir().unwrap();
        let c = context(temp.path());
        let result = run(
            &c,
            &json!({"program":"sh","args":["-c","echo started; sleep 0.1; echo finished"],"timeout_seconds":5}),
            "foreground-default",
        ).await.unwrap();
        let result: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(result["status"], "completed");
        assert_eq!(result["exit_code"], 0);
        assert!(result["stdout"].as_str().unwrap().contains("finished"));
    }

    #[tokio::test]
    async fn nonzero_exit_is_failed_and_rg_no_matches_remains_successful_search() {
        let temp = tempfile::tempdir().unwrap();
        let c = context(temp.path());
        let v = shell(&c, "echo output; echo failure >&2; exit 7", false, 5).await;
        assert_eq!(v["status"], "failed");
        assert_eq!(v["exit_code"], 7);
        assert!(v["stderr"].as_str().unwrap().contains("failure"));
        let call = crate::tools::ToolCall {
            id: "failed".into(),
            name: "run_command".into(),
            arguments: json!({"program":"sh","args":["-c","exit 3"]}),
        };
        let record = crate::tools::execute(call, &c).await;
        assert!(!record.success);
        std::fs::write(temp.path().join("file"), "hello").unwrap();
        assert_eq!(
            internal(temp.path(), "rg", &["absent".into(), "file".into()], None)
                .await
                .unwrap(),
            ""
        );
    }
    #[tokio::test]
    async fn background_jobs_are_scoped_can_be_stopped_and_survive_registry_eviction() {
        let temp = tempfile::tempdir().unwrap();
        let c = context(temp.path());
        let now = Instant::now();
        let v = shell(&c, "echo started; sleep 30", true, 60).await;
        assert_eq!(v["status"], "running");
        assert!(now.elapsed() < Duration::from_secs(3));
        let args = json!({"run_id":v["run_id"]});
        let mut foreign = c.clone();
        foreign.session_id = Some(uuid::Uuid::new_v4());
        assert!(poll(&foreign, &args, true).await.is_err());
        foreign = c.clone();
        foreign.memory.partner = Some(uuid::Uuid::new_v4());
        assert!(poll(&foreign, &args, false).await.is_err());
        let result: Value = serde_json::from_str(&poll(&c, &args, true).await.unwrap()).unwrap();
        assert_eq!(result["status"], "cancelled");
        let id = uuid::Uuid::parse_str(v["run_id"].as_str().unwrap()).unwrap();
        registry().lock().unwrap().remove(&id);
        let saved: Value = serde_json::from_str(&poll(&c, &args, false).await.unwrap()).unwrap();
        assert_eq!(saved["status"], "cancelled");
        assert!(poll(&foreign, &args, false).await.is_err());
    }
    #[tokio::test]
    async fn timeout_cleans_process_group_and_shutdown_waits_for_background_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let c = context(temp.path());
        let v = shell(&c, "sleep 30 & echo $! > child.pid; wait", false, 1).await;
        assert_eq!(v["status"], "timed_out");
        let pid: i32 = std::fs::read_to_string(temp.path().join("child.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        for _ in 0..30 {
            if unsafe { libc::kill(pid, 0) } != 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_ne!(
            unsafe { libc::kill(pid, 0) },
            0,
            "descendant survived timeout"
        );
        let v = shell(&c, "sleep 30", true, 60).await;
        shutdown(c.memory.anchor.clone()).await;
        let saved: Value = serde_json::from_str(
            &poll(&c, &json!({"run_id":v["run_id"]}), false)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(saved["status"], "cancelled");
    }
    #[tokio::test]
    async fn output_keeps_head_tail_and_private_pageable_logs() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let c = context(temp.path());
        let v = shell(
            &c,
            "python3 -c 'import sys;sys.stdout.write(\"HEAD\"+\"x\"*200000+\"TAIL\")'",
            false,
            5,
        )
        .await;
        assert_eq!(v["status"], "completed");
        let text = v["stdout"].as_str().unwrap();
        assert!(text.starts_with("HEAD") && text.ends_with("TAIL"));
        assert!(text.len() < 70000);
        assert_eq!(v["output_truncated"][0], true);
        assert_eq!(v["log_truncated"][0], false);
        let path = Path::new(v["stdout_log"].as_str().unwrap());
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let page: Value = serde_json::from_str(
            &poll(
                &c,
                &json!({"run_id":v["run_id"],"stream":"stdout","offset":200004,"limit":4}),
                false,
            )
            .await
            .unwrap(),
        )
        .unwrap();
        assert_eq!(page["page"]["content"], "TAIL");
        assert_eq!(page["page"]["next_offset"], Value::Null);
        assert!(
            poll(&c, &json!({"run_id":v["run_id"],"offset":1}), false)
                .await
                .is_err()
        );
        assert!(
            poll(&c, &json!({"run_id":v["run_id"],"wait_ms":-1}), false)
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn dropping_foreground_request_cancels_its_worker() {
        let temp = tempfile::tempdir().unwrap();
        let c = context(temp.path());
        let task = tokio::spawn({
            let c = c.clone();
            async move { shell(&c, "echo ready > ready; sleep 30", false, 60).await }
        });
        for _ in 0..100 {
            if temp.path().join("ready").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(temp.path().join("ready").exists());
        task.abort();
        let _ = task.await;
        tokio::time::timeout(Duration::from_secs(5), shutdown(c.memory.anchor.clone()))
            .await
            .unwrap();
        let runs = registry().lock().unwrap();
        let owned: Vec<_> = runs
            .values()
            .filter(|r| r.scope.anchor == c.memory.anchor)
            .collect();
        assert_eq!(owned.len(), 1);
        assert_eq!(snapshot(owned[0]).status, "cancelled");
        assert!(owned[0].finished.load(Ordering::Acquire));
    }
    #[test]
    fn previews_are_bounded_even_when_streams_never_end_a_line() {
        let mut b = Buffer::default();
        b.push(b"HEAD");
        for _ in 0..1000 {
            b.push(&[b'x'; 8192]);
        }
        b.push(b"TAIL");
        let text = b.text();
        assert!(text.starts_with("HEAD") && text.ends_with("TAIL"));
        assert!(text.len() < 70000);
    }
}
