//! A project-scoped gopls session for the read-only file preview.
use gpui::SharedString;
use notify::{EventKind, RecursiveMode, Watcher};
use reqwest::Url;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};

const TIMEOUT: Duration = Duration::from_secs(60);
type Definition = Result<(String, usize), String>;

struct Lookup {
    uri: Url,
    source: SharedString,
    line: usize,
    character: usize,
    reply: oneshot::Sender<Definition>,
}

pub(super) struct GoDefinitions {
    lookups: mpsc::Sender<Lookup>,
    worker: JoinHandle<()>,
    status: watch::Receiver<ProcessState>,
}

#[derive(Clone)]
pub(super) enum ProcessState {
    Starting,
    Running(u32),
    Failed(String),
    Finished,
}

impl GoDefinitions {
    pub fn start(root: PathBuf, first_file: PathBuf) -> Result<Self, String> {
        let (lookups, receiver) = mpsc::channel(4);
        let (status_sender, status) = watch::channel(ProcessState::Starting);
        let worker =
            crate::provider::runtime()?.spawn(run(root, first_file, receiver, status_sender));
        Ok(Self {
            lookups,
            worker,
            status,
        })
    }

    pub fn process_state(&self) -> ProcessState {
        self.status.borrow().clone()
    }

    pub fn lookup(
        &self,
        path: String,
        source: SharedString,
        offset: usize,
    ) -> Result<crate::provider::Job<(String, usize)>, String> {
        let lookups = self.lookups.clone();
        crate::provider::Job::start(async move {
            let path = Path::new(&path);
            if tokio::fs::read(path).await.as_deref().ok() != Some(source.as_bytes()) {
                return Err("文件内容已变化，请重新打开文件后再试".into());
            }
            let uri = file_uri(path).ok_or("无法读取 Go 文件路径")?;
            let before = source.get(..offset).ok_or("无法读取 Go 符号位置")?;
            let line = before.bytes().filter(|byte| *byte == b'\n').count();
            let character = before
                .rsplit('\n')
                .next()
                .unwrap_or("")
                .encode_utf16()
                .count();
            let (reply, result) = oneshot::channel();
            tokio::time::timeout(TIMEOUT, async {
                lookups
                    .send(Lookup {
                        uri,
                        source,
                        line,
                        character,
                        reply,
                    })
                    .await
                    .map_err(|_| "Go 定义查询已中断")?;
                result.await.map_err(|_| "Go 定义查询已中断")?
            })
            .await
            .map_err(|_| "查找 Go 定义超时")?
        })
    }
}

impl Drop for GoDefinitions {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

async fn run(
    root: PathBuf,
    first_file: PathBuf,
    mut lookups: mpsc::Receiver<Lookup>,
    status: watch::Sender<ProcessState>,
) {
    let initial = async {
        let mut server = Session::start(&root).await?;
        let file = root.join(first_file);
        if tokio::fs::metadata(&file)
            .await
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= 2 * 1024 * 1024)
            && let Ok(path) = tokio::fs::canonicalize(file).await
            && let Some(uri) = file_uri(&path)
            && let Ok(source) = tokio::fs::read_to_string(path).await
        {
            tokio::time::timeout(TIMEOUT, server.open_document(&uri, &source.into()))
                .await
                .map_err(|_| "打开 Go 文件超时")?
                .map_err(|error| format!("gopls 连接中断：{error}"))?;
        }
        Ok(server)
    }
    .await;
    report_start(&initial, &status);
    let mut session = Some(initial);
    loop {
        let lookup = if let Some(Ok(server)) = session.as_mut() {
            tokio::select! {
                lookup = lookups.recv() => lookup,
                message = server.messages.recv() => {
                    if let Err(error) = server.handle_message(message).await {
                        status.send_replace(ProcessState::Failed(format!("gopls 连接中断：{error}")));
                        session = None;
                    }
                    continue;
                }
            }
        } else {
            lookups.recv().await
        };
        let Some(lookup) = lookup else { return };
        if lookup.reply.is_closed() {
            continue;
        }
        // Start a new session when a filesystem event reports lost changes.
        let active = match session.take() {
            Some(Ok(server)) if !server.changes_lost() => Ok(server),
            Some(Err(error)) => Err(error),
            _ => {
                status.send_replace(ProcessState::Starting);
                let result = Session::start(&root).await;
                report_start(&result, &status);
                result
            }
        };
        let result = match active {
            Ok(mut server) => {
                match tokio::time::timeout(TIMEOUT, server.definition(&lookup)).await {
                    Ok(Ok(response)) => {
                        if server.watcher.is_some() {
                            session = Some(Ok(server));
                        } else {
                            status.send_replace(ProcessState::Finished);
                        }
                        parse_definition(response).await
                    }
                    Ok(Err(error)) => {
                        let error = format!("gopls 连接中断：{error}");
                        status.send_replace(ProcessState::Failed(error.clone()));
                        Err(error)
                    }
                    Err(_) => {
                        let error = "查找 Go 定义超时".to_owned();
                        status.send_replace(ProcessState::Failed(error.clone()));
                        Err(error)
                    }
                }
            }
            Err(error) => Err(error),
        };
        let _ = lookup.reply.send(result);
    }
}

fn report_start(result: &Result<Session, String>, status: &watch::Sender<ProcessState>) {
    status.send_replace(match result {
        Ok(server) => server
            .child
            .id()
            .map_or(ProcessState::Finished, ProcessState::Running),
        Err(error) => ProcessState::Failed(error.clone()),
    });
}

// A missing map means the watcher lost events and the session must be rebuilt.
type Changes = Arc<Mutex<Option<HashMap<PathBuf, u8>>>>;

struct Session {
    child: Child,
    reader: JoinHandle<()>,
    input: ChildStdin,
    messages: mpsc::Receiver<io::Result<Value>>,
    next_id: u64,
    document: Option<(Url, SharedString)>,
    watcher: Option<notify::RecommendedWatcher>,
    changes: Changes,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl Session {
    async fn start(root: &Path) -> Result<Self, String> {
        let root = tokio::fs::canonicalize(root)
            .await
            .map_err(|_| "无法打开 Go 项目目录")?;
        let root_uri = file_uri(&root).ok_or("无法读取 Go 项目路径")?;
        let mut child = crate::child_process::async_command("gopls")
            .arg("serve")
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    "请安装 gopls 并将它加入 PATH".into()
                } else {
                    format!("无法启动 gopls：{error}")
                }
            })?;
        let input = child.stdin.take().expect("piped gopls stdin");
        let output = child.stdout.take().expect("piped gopls stdout");
        let (sender, messages) = mpsc::channel(16);
        let reader = tokio::spawn(read_messages(output, sender));
        let changes = Changes::default();
        *changes.lock().unwrap() = Some(HashMap::new());
        let watcher = watch(&root, changes.clone()).ok();
        let mut session = Self {
            child,
            reader,
            input,
            messages,
            next_id: 0,
            document: None,
            watcher,
            changes,
        };
        let result = tokio::time::timeout(
            TIMEOUT,
            session.request(
                "initialize",
                json!({
                    "processId": std::process::id(),
                "rootUri": root_uri.as_str(),
                    "capabilities": {},
                    "initializationOptions": { "diagnosticsTrigger": "Save" },
                }),
            ),
        )
        .await
        .map_err(|_| "gopls 初始化超时")?
        .map_err(|error| format!("无法初始化 gopls：{error}"))?;
        if let Some(message) = result.pointer("/error/message").and_then(Value::as_str) {
            return Err(format!("无法初始化 gopls：{message}"));
        }
        session
            .notify("initialized", json!({}))
            .await
            .map_err(|error| error.to_string())?;
        Ok(session)
    }

    fn changes_lost(&self) -> bool {
        self.changes.lock().unwrap().is_none()
    }

    async fn definition(&mut self, lookup: &Lookup) -> io::Result<Value> {
        let changes = self.changes.lock().unwrap().as_mut().map(std::mem::take);
        let changes = changes.ok_or_else(|| io::Error::other("项目文件变化过多，请重试"))?;
        if !changes.is_empty() {
            let changes: Vec<_> = changes
                .into_iter()
                .filter_map(|(path, kind)| {
                    let uri = file_uri(&path)?;
                    let kind = if !path.exists() {
                        3
                    } else if kind == 3 {
                        1
                    } else {
                        kind
                    };
                    Some(json!({ "uri": uri.as_str(), "type": kind }))
                })
                .collect();
            self.notify(
                "workspace/didChangeWatchedFiles",
                json!({ "changes": changes }),
            )
            .await?;
        }
        self.open_document(&lookup.uri, &lookup.source).await?;
        self.request(
            "textDocument/definition",
            json!({
                "textDocument": { "uri": lookup.uri.as_str() },
                "position": { "line": lookup.line, "character": lookup.character }
            }),
        )
        .await
    }

    async fn open_document(&mut self, uri: &Url, source: &SharedString) -> io::Result<()> {
        if self
            .document
            .as_ref()
            .is_none_or(|(open_uri, open_source)| open_uri != uri || open_source != source)
        {
            if let Some((uri, _)) = self.document.take() {
                self.notify(
                    "textDocument/didClose",
                    json!({ "textDocument": { "uri": uri.as_str() } }),
                )
                .await?;
            }
            self.notify("textDocument/didOpen", json!({
                "textDocument": { "uri": uri.as_str(), "languageId": "go", "version": 1, "text": source.as_ref() }
            })).await?;
            self.document = Some((uri.clone(), source.clone()));
        }
        Ok(())
    }

    async fn send(&mut self, message: Value) -> io::Result<()> {
        let body = serde_json::to_vec(&message)?;
        self.input
            .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
            .await?;
        self.input.write_all(&body).await
    }

    async fn notify(&mut self, method: &str, params: Value) -> io::Result<()> {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    async fn request(&mut self, method: &str, params: Value) -> io::Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await?;
        loop {
            let message = self.messages.recv().await;
            if let Some(message) = self.handle_message(message).await?
                && message["id"].as_u64() == Some(id)
            {
                return Ok(message);
            }
        }
    }

    async fn handle_message(
        &mut self,
        message: Option<io::Result<Value>>,
    ) -> io::Result<Option<Value>> {
        let message = message.ok_or(io::ErrorKind::UnexpectedEof)??;
        if message.get("method").is_some() {
            if let Some(id) = message.get("id") {
                self.send(json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": { "code": -32601, "message": "Unsupported client request" }
                }))
                .await?;
            }
            return Ok(None);
        }
        Ok(Some(message))
    }
}

async fn read_messages(output: ChildStdout, sender: mpsc::Sender<io::Result<Value>>) {
    let mut output = BufReader::new(output);
    loop {
        let result = read_message(&mut output).await;
        let failed = result.is_err();
        if sender.send(result).await.is_err() || failed {
            return;
        }
    }
}

async fn read_message(output: &mut BufReader<ChildStdout>) -> io::Result<Value> {
    let mut header = String::new();
    let mut length = None;
    loop {
        header.clear();
        if output.read_line(&mut header).await? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        if header == "\r\n" {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let length = length
        .filter(|length| *length <= 16 * 1024 * 1024)
        .ok_or(io::ErrorKind::InvalidData)?;
    let mut body = vec![0; length];
    output.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body)?)
}

fn watch(root: &Path, changes: Changes) -> notify::Result<notify::RecommendedWatcher> {
    let root = root.to_owned();
    let watched_root = root.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let mut pending = changes.lock().unwrap();
        let Ok(event) = event else {
            *pending = None;
            return;
        };
        if event.need_rescan() {
            *pending = None;
            return;
        }
        let kind = match event.kind {
            EventKind::Create(_) | EventKind::Modify(notify::event::ModifyKind::Name(_)) => 1,
            EventKind::Modify(_) => 2,
            EventKind::Remove(_) => 3,
            _ => return,
        };
        let Some(files) = pending.as_mut() else {
            return;
        };
        for path in event.paths {
            if path
                .strip_prefix(&watched_root)
                .unwrap_or(&path)
                .components()
                .any(|part| part.as_os_str() == ".git")
            {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            let is_go = name.ends_with(".go")
                || matches!(
                    name,
                    "go.mod" | "go.sum" | "go.work" | "go.work.sum" | "modules.txt"
                );
            if is_go || (kind != 2 && path.extension().is_none()) {
                files
                    .entry(path)
                    .and_modify(|previous| {
                        if *previous != 1 || kind != 2 {
                            *previous = kind;
                        }
                    })
                    .or_insert(kind);
            }
        }
        if files.len() > 4096 {
            *pending = None;
        }
    })?;
    watcher.watch(&root, RecursiveMode::Recursive)?;
    Ok(watcher)
}

fn file_uri(path: &Path) -> Option<Url> {
    // Keep the canonical path for file I/O. gopls URIs use ordinary Windows drive/UNC paths.
    let path = gpui_glint::paths::display_path(path);
    Url::from_file_path(path).ok()
}

async fn parse_definition(message: Value) -> Definition {
    let error = message.pointer("/error/message").and_then(Value::as_str);
    if let Some(error) = error {
        return Err(
            if error.contains("no identifier") || error.contains("no object") {
                "未找到 Go 符号定义".into()
            } else {
                error.into()
            },
        );
    }
    let result = &message["result"];
    let location = result.get(0).unwrap_or(result);
    let uri = location["uri"].as_str().ok_or("未找到 Go 符号定义")?;
    let line = location
        .pointer("/range/start/line")
        .and_then(Value::as_u64)
        .ok_or("无法解析 gopls 返回的定义位置")?;
    let path = Url::parse(uri)
        .ok()
        .and_then(|uri| uri.to_file_path().ok())
        .ok_or("gopls 返回了无法预览的路径")?;
    let path = tokio::fs::canonicalize(path)
        .await
        .map_err(|_| "无法打开 Go 定义所在文件")?;
    Ok((path.to_string_lossy().into_owned(), line as usize + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Go and gopls on PATH"]
    async fn go_definitions_reuses_the_session_and_observes_file_edits() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let router = root.join("router.go");
        let handler = root.join("handler.go");
        let call = "package fixture\nfunc route(h *Handler) { _ = \"中文🙂\"; _ = h.handle }\n";
        let declaration = "package fixture\ntype Handler struct{}\nfunc (*Handler) handle() {}\n";
        std::fs::write(root.join("go.mod"), "module example.com/fixture\ngo 1.21\n").unwrap();
        std::fs::write(&router, call).unwrap();
        std::fs::write(&handler, declaration).unwrap();

        let client = GoDefinitions::start(root, router.clone()).unwrap();
        let lookup = || {
            client
                .lookup(
                    router.to_string_lossy().into_owned(),
                    call.into(),
                    call.find("handle").unwrap(),
                )
                .unwrap()
        };
        let expected_path = handler.to_string_lossy().into_owned();
        assert_eq!(lookup().result().await.unwrap(), (expected_path.clone(), 3));

        std::fs::write(
            &handler,
            declaration.replace("type Handler", "\n\ntype Handler"),
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if lookup().result().await.unwrap() == (expected_path.clone(), 5) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("gopls should receive the definition file change");
    }
}
