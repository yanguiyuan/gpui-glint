//! MCP stdio connects to the live GPUI workbench through a private loopback bridge.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
const MAX_LINE: u64 = 64 * 1024 * 1024;
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
#[derive(Clone)]
pub struct CallMetadata {
    pub client: String,
    pub rpc_id: String,
    pub started_at_ms: i64,
    pub started: Instant,
}
pub struct Call {
    pub metadata: CallMetadata,
    pub rejection: Option<String>,
    pub name: String,
    pub arguments: Value,
    pub deadline: Instant,
    pub reply: std::sync::mpsc::Sender<Result<Value, String>>,
}
#[derive(Serialize, Deserialize)]
struct ConnectionInfo {
    address: SocketAddr,
    token: String,
}
pub struct Server {
    stop: Arc<AtomicBool>,
    connections: Arc<Mutex<HashMap<SocketAddr, TcpStream>>>,
    path: PathBuf,
    token: String,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(connections) = self.connections.lock() {
            for stream in connections.values() {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
        if read_info(&self.path).is_ok_and(|info| info.token == self.token) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
pub fn discovery_path() -> Result<PathBuf, String> {
    Ok(super::storage::Store::default_path()?.with_extension("mcp.json"))
}
// OS locks are released on process exit, including a crash. Keep the file: unlinking
// it would let two processes lock different files with the same name on Unix.
fn lock_file(path: &Path) -> Result<std::fs::File, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())
}
pub fn workbench_guard() -> Result<Option<std::fs::File>, String> {
    instance_guard(&discovery_path()?.with_extension("instance.lock"))
}
fn instance_guard(path: &Path) -> Result<Option<std::fs::File>, String> {
    let file = lock_file(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}
fn read_info(path: &Path) -> Result<ConnectionInfo, String> {
    let bytes = std::fs::read(path).map_err(|_| "Glint API 尚未运行，请先打开工作台".to_owned())?;
    let info: ConnectionInfo =
        serde_json::from_slice(&bytes).map_err(|_| "MCP 连接信息无效".to_owned())?;
    if !info.address.ip().is_loopback() || info.token.len() < 32 {
        return Err("MCP 连接信息无效".into());
    }
    Ok(info)
}
fn write_info(path: &Path, info: &ConnectionInfo) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(".mcp-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut file, info).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        #[cfg(windows)]
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
fn line(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, String> {
    let mut bytes = Vec::new();
    let count = reader
        .take(MAX_LINE + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| e.to_string())?;
    if count == 0 {
        return Ok(None);
    }
    if count as u64 > MAX_LINE {
        return Err("MCP 消息超过 64 MiB".into());
    }
    Ok(Some(bytes))
}
fn write_json(writer: &mut impl Write, value: &Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_LINE {
        return Err("MCP 消息超过 64 MiB".into());
    }
    writer
        .write_all(&bytes)
        .and_then(|_| writer.flush())
        .map_err(|e| e.to_string())
}
fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
#[derive(Default)]
struct Protocol {
    initialized: bool,
    ready: bool,
    client: String,
}
impl Protocol {
    fn handle(
        &mut self,
        bytes: &[u8],
        execute: impl FnOnce(&str, Value) -> Result<Value, String>,
    ) -> Option<Value> {
        let request: Value = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(_) => return Some(rpc_error(Value::Null, -32700, "Invalid JSON")),
        };
        let id = request.get("id").cloned();
        if !request.is_object()
            || request["jsonrpc"] != "2.0"
            || !request["method"].is_string()
            || id
                .as_ref()
                .is_some_and(|id| !(id.is_string() || id.is_i64() || id.is_u64()))
        {
            return Some(rpc_error(
                id.unwrap_or(Value::Null),
                -32600,
                "Invalid JSON-RPC request",
            ));
        }
        let method = request["method"].as_str().unwrap();
        if id.is_none() {
            if method == "notifications/initialized" && self.initialized {
                self.ready = true;
            }
            return None;
        }
        let id = id.unwrap();
        let success = |result| json!({"jsonrpc":"2.0","id":id,"result":result});
        if method == "ping" {
            return Some(success(json!({})));
        }
        if method == "initialize" {
            if self.initialized {
                return Some(rpc_error(id, -32600, "Already initialized"));
            }
            if !request["params"]["protocolVersion"].is_string()
                || !request["params"]["capabilities"].is_object()
                || !request["params"]["clientInfo"].is_object()
            {
                return Some(rpc_error(id, -32602, "Invalid initialize parameters"));
            }
            self.initialized = true;
            let client = &request["params"]["clientInfo"];
            self.client = super::mcp_history::clip(
                &format!(
                    "{} {}",
                    client["name"].as_str().unwrap_or("未知客户端"),
                    client["version"].as_str().unwrap_or("")
                ),
                256,
            );
            let requested = request["params"]["protocolVersion"].as_str().unwrap();
            let version =
                if ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"].contains(&requested) {
                    requested
                } else {
                    "2025-11-25"
                };
            return Some(success(
                json!({"protocolVersion":version,"capabilities":{"tools":{}},
                "serverInfo":{"name":"glint-api","version":super::updater::VERSION},
                "instructions":"Controls the live Glint API native GUI and persists mutations. Use stable request_id/record_id, not array indices. HTTP/SSE/WS operations return immediately; poll status/response/log tools and cancel explicitly. Another running request must be cancelled before switching. Secrets are hidden in reads unless include_secrets=true. GUI and MCP share the same workspace. Tool errors do not imply rollback of previously completed operations."}),
            ));
        }
        if !self.ready {
            return Some(rpc_error(
                id,
                -32000,
                "Initialize and send notifications/initialized first",
            ));
        }
        match method {
            "tools/list" => Some(success(json!({"tools":super::mcp_tools::catalog()}))),
            "tools/call" => {
                let Some(name) = request["params"]["name"].as_str() else {
                    return Some(rpc_error(id, -32602, "Missing tool name"));
                };
                let catalog = super::mcp_tools::catalog();
                let Some(tool) = catalog.iter().find(|tool| tool["name"] == name) else {
                    return Some(rpc_error(id, -32602, "Unknown tool"));
                };
                let arguments = request["params"]
                    .get("arguments")
                    .cloned()
                    .unwrap_or(json!({}));
                if let Err(error) = super::mcp_tools::validate(&tool["inputSchema"], &arguments) {
                    return Some(rpc_error(id, -32602, &error));
                }
                let (result, error) = match execute(name, arguments) {
                    Ok(value) => (json!({"result":value}), false),
                    Err(error) => (json!({"error":error}), true),
                };
                Some(success(
                    json!({"content":[{"type":"text","text":result.to_string()}],"structuredContent":result,"isError":error}),
                ))
            }
            _ => Some(rpc_error(id, -32601, "Method not found")),
        }
    }
}
impl Server {
    pub fn bind(path: PathBuf) -> Result<(Self, async_channel::Receiver<Call>), String> {
        let listener =
            TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let info = ConnectionInfo {
            address: listener.local_addr().map_err(|e| e.to_string())?,
            token: token.clone(),
        };
        write_info(&path, &info)?;
        let stop = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(HashMap::<SocketAddr, TcpStream>::new()));
        let (sender, receiver) = async_channel::bounded::<Call>(32);
        let server = Self {
            stop: stop.clone(),
            connections: connections.clone(),
            path,
            token: token.clone(),
        };
        std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, address)) => {
                        if connections.lock().unwrap().len() >= 16 {
                            continue;
                        }
                        if let Ok(clone) = stream.try_clone() {
                            connections.lock().unwrap().insert(address, clone);
                        }
                        let (connections, sender, token) =
                            (connections.clone(), sender.clone(), token.clone());
                        std::thread::spawn(move || {
                            let result = serve_connection(stream, &token, &sender);
                            let _ = result;
                            connections.lock().unwrap().remove(&address);
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok((server, receiver))
    }
}
fn serve_connection(
    mut stream: TcpStream,
    token: &str,
    sender: &async_channel::Sender<Call>,
) -> Result<(), String> {
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    stream.set_nodelay(true).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(CALL_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let auth: Value = serde_json::from_slice(&line(&mut reader)?.ok_or("Authentication required")?)
        .map_err(|_| "Invalid authentication")?;
    if auth["token"].as_str() != Some(token) {
        return Err("MCP authentication failed".into());
    }
    write_json(&mut stream, &json!({"authenticated":true}))?;
    stream.set_read_timeout(None).map_err(|e| e.to_string())?;
    let mut protocol = Protocol::default();
    while let Some(bytes) = line(&mut reader)? {
        let incoming = serde_json::from_slice::<Value>(&bytes).ok();
        let metadata = CallMetadata {
            client: protocol.client.clone(),
            rpc_id: super::mcp_history::clip(
                &incoming
                    .as_ref()
                    .and_then(|v| v.get("id"))
                    .map(Value::to_string)
                    .unwrap_or_default(),
                256,
            ),
            started_at_ms: chrono::Utc::now().timestamp_millis(),
            started: Instant::now(),
        };
        let ready = protocol.ready;
        let response = protocol.handle(&bytes, |name, arguments| {
            dispatch_call(sender, &metadata, name, arguments, None)
        });
        // Schema/name rejections never reach the business dispatcher, but are still tool attempts.
        if ready && response.as_ref().is_some_and(|v| v.get("error").is_some()) {
            if let Some(request) = incoming
                .as_ref()
                .filter(|v| v["method"] == "tools/call" && v.get("id").is_some())
            {
                let error = response.as_ref().unwrap()["error"]["message"]
                    .as_str()
                    .unwrap_or("调用被拒绝")
                    .to_owned();
                let _ = dispatch_call(
                    sender,
                    &metadata,
                    request["params"]["name"]
                        .as_str()
                        .unwrap_or("<未提供工具名>"),
                    request["params"]
                        .get("arguments")
                        .cloned()
                        .unwrap_or(Value::Null),
                    Some(error),
                );
            }
        }
        write_json(&mut stream, &json!({"response":response}))?;
    }
    Ok(())
}
fn dispatch_call(
    sender: &async_channel::Sender<Call>,
    metadata: &CallMetadata,
    name: &str,
    arguments: Value,
    rejection: Option<String>,
) -> Result<Value, String> {
    let (reply, receiver) = std::sync::mpsc::channel();
    sender
        .try_send(Call {
            name: name.into(),
            arguments,
            deadline: metadata.started + CALL_TIMEOUT,
            reply,
            metadata: metadata.clone(),
            rejection,
        })
        .map_err(|_| "工作台忙或已关闭，请重试".to_owned())?;
    receiver
        .recv_timeout(CALL_TIMEOUT)
        .map_err(|_| "工作台未响应，操作已过期".to_owned())?
}
fn connect(path: &Path) -> Result<(TcpStream, BufReader<TcpStream>), String> {
    let info = read_info(path)?;
    let mut stream = TcpStream::connect_timeout(&info.address, Duration::from_secs(2))
        .map_err(|_| "无法连接正在运行的 Glint API".to_owned())?;
    stream
        .set_read_timeout(Some(CALL_TIMEOUT))
        .map_err(|e| e.to_string())?;
    stream.set_nodelay(true).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    write_json(&mut stream, &json!({"token":info.token}))?;
    let auth: Value = serde_json::from_slice(&line(&mut reader)?.ok_or("MCP bridge closed")?)
        .map_err(|_| "MCP bridge authentication failed")?;
    if auth["authenticated"] != true {
        return Err("MCP bridge authentication failed".into());
    }
    Ok((stream, reader))
}
pub fn proxy() -> Result<(), String> {
    let path = discovery_path()?;
    proxy_io(std::io::stdin().lock(), std::io::stdout().lock(), || {
        connect_or_start(&path)
    })
}
fn connect_or_start(path: &Path) -> Result<(TcpStream, BufReader<TcpStream>), String> {
    match connect(path) {
        Ok(connection) => return Ok(connection),
        Err(error) if std::env::var_os("GLINT_API_MCP_NO_AUTOSTART").is_some() => {
            return Err(error);
        }
        Err(_) => {}
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    let lock = lock_file(&path.with_extension("startup.lock"))?;
    loop {
        // Another proxy may already have started the workbench.
        if let Ok(connection) = connect(path) {
            return Ok(connection);
        }
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(format!("等待 Glint API 启动锁失败：{error}")),
        }
    }
    if let Ok(connection) = connect(path) {
        return Ok(connection);
    }
    let mut command =
        std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command
        .arg("--mcp-host")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB.
        // MCP clients can clean up a proxy's entire job when refreshing tools.
        command.creation_flags(0x00000200 | 0x00000008 | 0x01000000);
    }
    let child = command.spawn();
    #[cfg(windows)]
    let child = child.or_else(|error| {
        use std::os::windows::process::CommandExt;
        if error.raw_os_error() != Some(5) {
            return Err(error);
        }
        // Some client jobs forbid breakaway. Still allow startup, with a useful
        // diagnostic rather than silently failing to open the workbench.
        eprintln!("Glint API: 客户端禁止脱离进程作业；工作台可能随 MCP 客户端退出");
        command.creation_flags(0x00000200 | 0x00000008).spawn()
    });
    let mut child = child.map_err(|e| format!("无法启动 Glint API：{e}"))?;
    loop {
        if let Ok(connection) = connect(path) {
            return Ok(connection);
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            // An existing instance can win the instance lock before publishing
            // discovery. Keep waiting when the duplicate exits successfully.
            if !status.success() {
                return Err(format!("Glint API 启动失败：{status}"));
            }
        }
        if Instant::now() >= deadline {
            return Err("Glint API 未能启动 MCP，请手动打开应用后重试".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn exchange(
    (stream, reader): &mut (TcpStream, BufReader<TcpStream>),
    bytes: &[u8],
) -> Result<Value, String> {
    stream.write_all(bytes).map_err(|e| e.to_string())?;
    if !bytes.ends_with(b"\n") {
        stream.write_all(b"\n").map_err(|e| e.to_string())?;
    }
    stream.flush().map_err(|e| e.to_string())?;
    let envelope: Value = serde_json::from_slice(&line(reader)?.ok_or("Glint API 已关闭")?)
        .map_err(|_| "MCP bridge response invalid")?;
    Ok(envelope["response"].clone())
}
fn proxy_io(
    mut input: impl BufRead,
    mut output: impl Write,
    mut connector: impl FnMut() -> Result<(TcpStream, BufReader<TcpStream>), String>,
) -> Result<(), String> {
    let mut connection = None;
    let mut initialization: Option<Vec<u8>> = None;
    let mut initialized = false;
    while let Some(bytes) = line(&mut input)? {
        let request = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
        // This notification has no client response. Preserve the client's ready
        // state even if its bridge acknowledgement is lost.
        let ready_notification =
            request["method"] == "notifications/initialized" && initialization.is_some();
        let result = (|| {
            if connection.is_none() {
                let mut next = connector()?;
                if let Some(init) = &initialization {
                    let reply = exchange(&mut next, init)?;
                    if reply.get("error").is_some() {
                        return Err("MCP 重新初始化失败".into());
                    }
                    if initialized {
                        exchange(
                            &mut next,
                            br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                        )?;
                    }
                }
                connection = Some(next);
            }
            exchange(connection.as_mut().unwrap(), &bytes)
        })();
        match result {
            Ok(response) => {
                if request["method"] == "initialize" && response.get("result").is_some() {
                    initialization = Some(bytes);
                }
                if !response.is_null() {
                    write_json(&mut output, &response)?;
                }
            }
            Err(error) => {
                // The GUI can outlive or restart independently of the client.
                // Keep stdio alive, but never replay a possibly executed tool.
                connection = None;
                if let Some(id) = request.get("id") {
                    write_json(
                        &mut output,
                        &rpc_error(
                            id.clone(),
                            -32000,
                            &format!(
                                "MCP 工作台连接中断；请求可能已执行，请确认结果后重试：{error}"
                            ),
                        ),
                    )?;
                }
                eprintln!("Glint API MCP bridge: {error}");
            }
        }
        initialized |= ready_notification;
    }
    Ok(())
}
pub fn configuration() -> Result<Value, String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut config = json!({"command":executable,"args":["--mcp"]});
    if let Some(path) = std::env::var_os("GLINT_API_DB_PATH") {
        config["env"] = json!({"GLINT_API_DB_PATH":path});
    }
    Ok(json!({"mcpServers":{"glint-api":config}}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn initialize() -> Value {
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})
    }
    fn handle(protocol: &mut Protocol, value: Value) -> Option<Value> {
        protocol.handle(value.to_string().as_bytes(), |_, _| Ok(json!({"ok":true})))
    }
    #[test]
    fn mcp_protocol_lifecycle_discovery_and_errors() {
        let mut protocol = Protocol::default();
        assert_eq!(
            handle(
                &mut protocol,
                json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})
            )
            .unwrap()["error"]["code"],
            -32000
        );
        assert_eq!(
            handle(&mut protocol, initialize()).unwrap()["result"]["protocolVersion"],
            "2025-11-25"
        );
        assert!(
            handle(
                &mut protocol,
                json!({"jsonrpc":"2.0","method":"notifications/initialized"})
            )
            .is_none()
        );
        let list = handle(
            &mut protocol,
            json!({"jsonrpc":"2.0","id":"list","method":"tools/list"}),
        )
        .unwrap();
        assert_eq!(list["id"], "list");
        assert!(list["result"]["tools"].as_array().unwrap().len() >= 50);
        let call = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"glint_workspace_get","arguments":{}}});
        let reply = protocol
            .handle(call.to_string().as_bytes(), |_, _| {
                Err("test tool error".into())
            })
            .unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(
            reply["result"]["structuredContent"]["error"],
            "test tool error"
        );
        assert!(reply.get("error").is_none());
        assert_eq!(
            protocol.handle(b"broken", |_, _| unreachable!()).unwrap()["error"]["code"],
            -32700
        );
        assert_eq!(
            handle(&mut protocol, json!([])).unwrap()["error"]["code"],
            -32600
        );
        assert_eq!(
            handle(
                &mut protocol,
                json!({"jsonrpc":"2.0","id":4,"method":"unknown"})
            )
            .unwrap()["error"]["code"],
            -32601
        );
        assert_eq!(handle(&mut protocol,json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"glint_request_get","arguments":{}}})).unwrap()["error"]["code"],-32602);
    }
    #[test]
    fn mcp_stdio_proxy_reaches_bridge_and_emits_only_rpc_responses() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workspace.mcp.json");
        let (server, receiver) = Server::bind(path.clone()).unwrap();
        let info = read_info(&path).unwrap();
        assert!(info.address.ip().is_loopback());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let worker = std::thread::spawn(move || {
            let call = receiver.recv_blocking().unwrap();
            assert_eq!(call.name, "glint_workspace_get");
            call.reply.send(Ok(json!({"live":true}))).unwrap();
        });
        let input=[initialize(),json!({"jsonrpc":"2.0","method":"notifications/initialized"}),json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"glint_workspace_get","arguments":{}}})].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")+"\n";
        let mut output = vec![];
        proxy_io(std::io::Cursor::new(input.as_bytes()), &mut output, || {
            connect(&path)
        })
        .unwrap();
        let replies = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(replies.len(), 3);
        assert_eq!(
            replies[2]["result"]["structuredContent"]["result"]["live"],
            true
        );
        worker.join().unwrap();
        drop(server);
        assert!(!path.exists());
    }
    #[test]
    fn workbench_instance_lock_is_exclusive_and_released_on_exit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("instance.lock");
        let guard = instance_guard(&path).unwrap().unwrap();
        let other_path = path.clone();
        assert!(
            std::thread::spawn(move || instance_guard(&other_path).unwrap().is_none())
                .join()
                .unwrap()
        );
        drop(guard);
        assert!(instance_guard(&path).unwrap().is_some());
    }
    #[test]
    fn proxy_reconnects_and_reinitializes_without_replaying_failed_call() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let init: Value =
                    serde_json::from_slice(&line(&mut reader).unwrap().unwrap()).unwrap();
                assert_eq!(init["method"], "initialize");
                write_json(
                    &mut stream,
                    &json!({"response":{"jsonrpc":"2.0","id":1,"result":{}}}),
                )
                .unwrap();
                let ready: Value =
                    serde_json::from_slice(&line(&mut reader).unwrap().unwrap()).unwrap();
                assert_eq!(ready["method"], "notifications/initialized");
                write_json(&mut stream, &json!({"response":null})).unwrap();
                let call: Value =
                    serde_json::from_slice(&line(&mut reader).unwrap().unwrap()).unwrap();
                assert_eq!(call["id"], if attempt == 0 { 2 } else { 3 });
                if attempt == 0 {
                    // Simulate a tool having executed just before the bridge dies.
                    stream.shutdown(Shutdown::Both).unwrap();
                } else {
                    write_json(
                        &mut stream,
                        &json!({"response":{"jsonrpc":"2.0","id":3,"result":{"reconnected":true}}}),
                    )
                    .unwrap();
                }
            }
        });
        let input = [
            initialize(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call"}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call"}),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        let mut output = vec![];
        proxy_io(std::io::Cursor::new(input), &mut output, || {
            let stream = TcpStream::connect(address).map_err(|e| e.to_string())?;
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let reader = BufReader::new(stream.try_clone().unwrap());
            Ok((stream, reader))
        })
        .unwrap();
        let replies: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[1]["id"], 2);
        assert_eq!(replies[1]["error"]["code"], -32000);
        assert_eq!(replies[2]["result"]["reconnected"], true);
        worker.join().unwrap();
    }
    #[test]
    fn mcp_bridge_rejects_missing_auth_and_non_loopback_discovery() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mcp.json");
        let (server, _receiver) = Server::bind(path.clone()).unwrap();
        let info = read_info(&path).unwrap();
        let mut socket = TcpStream::connect(info.address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write_json(&mut socket, &json!({"token":"wrong-token"})).unwrap();
        assert_eq!(socket.read(&mut [0]).unwrap(), 0);
        write_info(
            &path,
            &ConnectionInfo {
                address: "192.0.2.1:1234".parse().unwrap(),
                token: info.token,
            },
        )
        .unwrap();
        assert!(read_info(&path).is_err());
        drop(server);
    }
}
