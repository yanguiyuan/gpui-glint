//! Native Messaging relay to an explicitly unlocked local page.
//! There is no database/key access in the browser host process.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::Duration,
};
use zeroize::{Zeroize, Zeroizing};

pub const HOST_NAME: &str = "com.glint.vault";
const MAX_REQUEST: usize = 8192;
const MAX_REPLY: usize = 65536;
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub op: String,
    pub origin: String,
    pub entry_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
}
pub struct Pending {
    pub request: Request,
    pub respond: SyncSender<Reply>,
}
pub struct Reply(pub Value);
impl Reply {
    pub fn error(error: &str) -> Self {
        Self(json!({"ok":false,"error":error}))
    }
}
impl Drop for Reply {
    fn drop(&mut self) {
        fn clear(value: &mut Value) {
            match value {
                Value::String(text) => text.zeroize(),
                Value::Array(values) => values.iter_mut().for_each(clear),
                Value::Object(values) => values.values_mut().for_each(clear),
                _ => {}
            }
        }
        clear(&mut self.0);
    }
}
#[derive(Serialize, Deserialize)]
struct Discovery {
    port: u16,
    token: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    token: String,
    request: Request,
}

pub fn origin(website: &str) -> Option<String> {
    let url = url::Url::parse(website).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(url.origin().ascii_serialization())
}
pub fn matches(website: &str, requested: &str) -> bool {
    origin(website).is_some_and(|actual| actual == requested)
}
fn token() -> Result<String, String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| "系统随机数不可用")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn discovery_path(database: &Path) -> PathBuf {
    database.with_file_name("browser-bridge.json")
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension(format!("{}.tmp", token()?));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .map_err(|_| "无法创建本机连接文件")?;
        file.write_all(bytes).map_err(|_| "无法写入本机连接文件")?;
        file.sync_all().map_err(|_| "无法刷新本机连接文件")?;
        std::fs::rename(&temporary, path).map_err(|_| "无法保存本机连接文件")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
/// Lifetime follows the unlocked page; the discovery record contains no vault key.
pub struct Bridge {
    pub pending: Receiver<Pending>,
    stop: Arc<AtomicBool>,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl Bridge {
    pub fn start(database: &Path) -> Result<Self, String> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| "无法启动浏览器连接")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "无法配置浏览器连接")?;
        let token = token()?;
        let (sender, pending) = mpsc::sync_channel(8);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let active = Arc::new(AtomicUsize::new(0));
        let discovery = Discovery {
            port: listener
                .local_addr()
                .map_err(|_| "无法读取本机端口")?
                .port(),
            token: token.clone(),
        };
        write_private(
            &discovery_path(database),
            &serde_json::to_vec(&discovery).map_err(|_| "连接信息无效")?,
        )?;
        std::thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, peer)) => {
                        if !peer.ip().is_loopback() || active.load(Ordering::Relaxed) >= 8 {
                            drop(stream);
                            continue;
                        }
                        active.fetch_add(1, Ordering::Relaxed);
                        let active = active.clone();
                        let sender = sender.clone();
                        let token = token.clone();
                        std::thread::spawn(move || {
                            let _ = serve(stream, &token, sender);
                            active.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(40))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self { pending, stop })
    }
}
fn serve(mut stream: TcpStream, token: &str, sender: SyncSender<Pending>) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    BufReader::new((&stream).take((MAX_REQUEST + 1) as u64))
        .read_until(b'\n', &mut bytes)
        .map_err(|_| "请求读取失败")?;
    if bytes.len() > MAX_REQUEST || !bytes.ends_with(b"\n") {
        return Err("请求过长或不完整".into());
    }
    let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|_| "请求格式无效")?;
    if envelope.token != token {
        return Err("连接认证失败".into());
    }
    let request = envelope.request;
    let reply = if origin(&request.origin).as_deref() != Some(request.origin.as_str())
        || !matches!(request.op.as_str(), "list" | "fill" | "copy")
        || request.entry_id.as_ref().is_some_and(|id| id.len() > 64)
    {
        Reply::error("网站或操作无效")
    } else {
        let (respond, response) = mpsc::sync_channel(1);
        if sender.try_send(Pending { request, respond }).is_err() {
            Reply::error("密码箱当前忙碌，请重试")
        } else {
            response
                .recv_timeout(Duration::from_secs(60))
                .unwrap_or_else(|_| Reply::error("本机确认超时，请重新点击填充"))
        }
    };
    let mut bytes = Zeroizing::new(serde_json::to_vec(&reply.0).map_err(|_| "回复格式无效")?);
    if bytes.len() > MAX_REPLY {
        return Err("回复过长".into());
    }
    bytes.push(b'\n');
    stream.write_all(&bytes).map_err(|_| "回复失败".into())
}
/// One bounded request to the most recently unlocked window.
fn relay(database: &Path, request: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
    if request.len() > MAX_REQUEST {
        return Err("请求过长".into());
    }
    let request: Request = serde_json::from_slice(request).map_err(|_| "扩展请求格式无效")?;
    let bytes = std::fs::read(discovery_path(database))
        .map_err(|_| "请先打开并解锁 Glint Vault（或 API / Agent 中的密码箱）")?;
    if bytes.len() > 4096 {
        return Err("本机连接信息无效".into());
    }
    let discovery: Discovery = serde_json::from_slice(&bytes).map_err(|_| "本机连接信息无效")?;
    if discovery.token.len() != 64 {
        return Err("本机连接信息无效".into());
    }
    let mut stream = TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], discovery.port)),
        Duration::from_secs(2),
    )
    .map_err(|_| "密码箱已锁定，请在本机应用重新解锁")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(65)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    let mut bytes = serde_json::to_vec(&Envelope {
        token: discovery.token,
        request,
    })
    .map_err(|_| "请求编码失败")?;
    bytes.push(b'\n');
    stream.write_all(&bytes).map_err(|_| "连接中断")?;
    let mut reply = Zeroizing::new(Vec::new());
    BufReader::new(stream.take((MAX_REPLY + 1) as u64))
        .read_until(b'\n', &mut reply)
        .map_err(|_| "连接中断或确认超时")?;
    if reply.len() > MAX_REPLY || !reply.ends_with(b"\n") {
        return Err("本机回复无效".into());
    }
    reply.pop();
    Ok(reply)
}
/// Native host protocol: four-byte native-endian length followed by UTF-8 JSON.
pub fn native_host(
    database: &Path,
    extension_origin: &str,
    mut input: impl Read,
    mut output: impl Write,
) -> Result<(), String> {
    let allowlist_path = database.with_file_name("browser-extensions.json");
    let allowed: Vec<String> = std::fs::read(allowlist_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    if !allowed.iter().any(|origin| origin == extension_origin) {
        return Err("扩展未配对，请先运行本机注册命令".into());
    }
    loop {
        let mut size = [0; 4];
        match input.read(&mut size[..1]) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => return Err("无法读取扩展请求".into()),
        }
        input
            .read_exact(&mut size[1..])
            .map_err(|_| "请求头不完整")?;
        let length = u32::from_ne_bytes(size) as usize;
        if length == 0 || length > MAX_REQUEST {
            return Err("扩展请求长度无效".into());
        }
        let mut bytes = vec![0; length];
        input.read_exact(&mut bytes).map_err(|_| "请求内容不完整")?;
        let reply = match relay(database, &bytes) {
            Ok(reply) => reply,
            Err(error) => Zeroizing::new(
                serde_json::to_vec(&json!({"ok":false,"error":error}))
                    .map_err(|_| "错误编码失败")?,
            ),
        };
        output
            .write_all(&(reply.len() as u32).to_ne_bytes())
            .and_then(|_| output.write_all(&reply))
            .and_then(|_| output.flush())
            .map_err(|_| "无法返回扩展结果")?;
    }
    Ok(())
}

pub fn install_host(database: &Path, browser: &str, extension_id: &str) -> Result<PathBuf, String> {
    if extension_id.len() != 32
        || !extension_id
            .bytes()
            .all(|byte| (b'a'..=b'p').contains(&byte))
    {
        return Err("扩展 ID 应为 32 位 a–p 字符".into());
    }
    if !matches!(browser, "chrome" | "edge" | "egolite") {
        return Err("browser 必须为 chrome、edge 或 egolite".into());
    }
    #[cfg(not(target_os = "macos"))]
    if browser == "egolite" {
        return Err("Ego Lite 本机注册目前支持 macOS；此系统请使用 Chrome 或 Edge".into());
    }
    let data = database.parent().ok_or("密码箱路径无效")?;
    let directory = data.join("native-host");
    std::fs::create_dir_all(&directory).map_err(|_| "无法创建本机主机目录")?;
    let executable = std::env::current_exe().map_err(|_| "无法定位主机程序")?;
    let installed = directory.join(if cfg!(windows) {
        "glint-vault-host.exe"
    } else {
        "glint-vault-host"
    });
    if executable != installed {
        std::fs::copy(executable, &installed).map_err(|_| "无法安装本机主机程序")?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法设置主机程序权限")?;
    }
    let origin = format!("chrome-extension://{extension_id}/");
    let allowfile = data.join("browser-extensions.json");
    let mut allowed: Vec<String> = std::fs::read(&allowfile)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    if !allowed.contains(&origin) {
        allowed.push(origin.clone());
    }
    write_private(
        &allowfile,
        &serde_json::to_vec(&allowed).map_err(|_| "扩展配置无效")?,
    )?;
    #[cfg(target_os = "macos")]
    let config = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME 不可用")?
        // Ego Lite 0.6 uses Chrome's Native Messaging lookup path on macOS.
        .join(if browser == "edge" {
            "Library/Application Support/Microsoft Edge/NativeMessagingHosts"
        } else {
            "Library/Application Support/Google/Chrome/NativeMessagingHosts"
        });
    #[cfg(target_os = "linux")]
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .ok_or("用户配置目录不可用")?
        .join(if browser == "chrome" {
            "google-chrome/NativeMessagingHosts"
        } else if browser == "edge" {
            "microsoft-edge/NativeMessagingHosts"
        } else {
            "chromium/NativeMessagingHosts"
        });
    #[cfg(target_os = "windows")]
    let config = directory.join(browser);
    std::fs::create_dir_all(&config).map_err(|_| "无法创建浏览器主机配置目录")?;
    let manifest = config.join(format!("{HOST_NAME}.json"));
    // Each browser has its own exact extension allowlist; no wildcard origins.
    write_private(&manifest,&serde_json::to_vec_pretty(&json!({"name":HOST_NAME,"description":"Glint Vault local bridge","path":installed,"type":"stdio","allowed_origins":[origin]})).map_err(|_|"主机配置无效")?)?;
    #[cfg(target_os = "windows")]
    {
        let key = format!(
            "HKCU\\Software\\{}\\NativeMessagingHosts\\{HOST_NAME}",
            if browser == "chrome" {
                "Google\\Chrome"
            } else if browser == "edge" {
                "Microsoft\\Edge"
            } else {
                "Chromium"
            }
        );
        let status = std::process::Command::new("reg")
            .args(["ADD", &key, "/ve", "/t", "REG_SZ", "/d"])
            .arg(&manifest)
            .arg("/f")
            .status()
            .map_err(|_| "无法注册本机主机")?;
        if !status.success() {
            return Err("本机主机注册失败".into());
        }
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matching_is_exact_origin_and_never_suffix_based() {
        assert!(matches("https://EXAMPLE.com/login", "https://example.com"));
        assert!(matches("https://example.com:443/a", "https://example.com"));
        for wrong in [
            "https://evil-example.com",
            "https://example.com.evil.test",
            "https://sub.example.com",
            "http://example.com",
            "https://example.com:8443",
        ] {
            assert!(!matches("https://example.com", wrong));
        }
        assert!(origin("javascript:alert(1)").is_none());
        assert!(origin("https://user:pass@example.com").is_none());
    }
    #[test]
    fn authenticated_bridge_and_native_framing_preserve_bounded_json() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("vault.sqlite3");
        let bridge = Bridge::start(&database).unwrap();
        let worker = std::thread::spawn(move || {
            let pending = bridge.pending.recv_timeout(Duration::from_secs(3)).unwrap();
            assert_eq!(pending.request.origin, "https://example.com");
            pending
                .respond
                .send(Reply(
                    json!({"ok":true,"entries":[{"id":"one","title":"Example","username":"user"}]}),
                ))
                .unwrap_or_else(|_| panic!("reply channel closed"));
            // Keep listener alive for the response; no credentials in this test.
            std::thread::sleep(Duration::from_millis(100));
        });
        let extension = "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/";
        std::fs::write(
            database.with_file_name("browser-extensions.json"),
            serde_json::to_vec(&vec![extension]).unwrap(),
        )
        .unwrap();
        let body = serde_json::to_vec(&Request {
            op: "list".into(),
            origin: "https://example.com".into(),
            entry_id: None,
            offset: None,
        })
        .unwrap();
        let mut framed = (body.len() as u32).to_ne_bytes().to_vec();
        framed.extend(&body);
        let mut output = Vec::new();
        native_host(&database, extension, framed.as_slice(), &mut output).unwrap();
        let length = u32::from_ne_bytes(output[..4].try_into().unwrap()) as usize;
        assert_eq!(length, output.len() - 4);
        let reply: Value = serde_json::from_slice(&output[4..]).unwrap();
        assert_eq!(reply["entries"][0]["id"], "one");
        assert!(native_host(&database, "chrome-extension://wrong/", &[][..], Vec::new()).is_err());
        assert!(
            native_host(
                &database,
                extension,
                &((MAX_REQUEST + 1) as u32).to_ne_bytes()[..],
                Vec::new()
            )
            .is_err()
        );
        worker.join().unwrap();
    }
    #[test]
    fn unauthenticated_requests_never_reach_the_page() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("vault.sqlite3");
        let bridge = Bridge::start(&database).unwrap();
        let discovery: Discovery =
            serde_json::from_slice(&std::fs::read(discovery_path(&database)).unwrap()).unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", discovery.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream.write_all(b"{\"token\":\"wrong\",\"request\":{\"op\":\"list\",\"origin\":\"https://example.com\",\"entry_id\":null}}\n").unwrap();
        let mut bytes = Vec::new();
        let _ = stream.read_to_end(&mut bytes);
        assert!(bytes.is_empty());
        assert!(bridge.pending.try_recv().is_err());
    }
}
