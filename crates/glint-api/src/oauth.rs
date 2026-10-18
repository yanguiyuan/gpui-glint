//! OAuth for developer-owned applications. Credentials never enter workspace serialization.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub id: String,
    pub authorization_url: String,
    pub token_url: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub pkce: bool,
    pub basic_auth: bool,
    pub manual_callback: bool,
    /// Use the expiry returned with each rotated refresh token instead of a fixed deadline.
    #[serde(skip_serializing_if = "is_false")]
    pub rolling_refresh_expiry: bool,
}
fn is_false(value: &bool) -> bool {
    !*value
}
impl Default for Config {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            authorization_url: String::new(),
            token_url: String::new(),
            client_id: String::new(),
            redirect_uri: "http://127.0.0.1:17943/oauth/callback".into(),
            scope: String::new(),
            pkce: true,
            basic_auth: false,
            manual_callback: false,
            rolling_refresh_expiry: false,
        }
    }
}
impl Config {
    /// Resolve only the runtime copy; persisted drafts retain their template expressions.
    pub fn resolve(&self, resolver: &super::template::TemplateResolver) -> Result<Self, String> {
        let mut config = self.clone();
        for (name, field) in [
            ("授权地址", &mut config.authorization_url),
            ("Token 地址", &mut config.token_url),
            ("Client ID", &mut config.client_id),
            ("回调地址", &mut config.redirect_uri),
            ("Scope", &mut config.scope),
        ] {
            *field = resolver
                .resolve(field)
                .map_err(|error| format!("{name}：{error}"))?;
        }
        Ok(config)
    }
    pub fn validate(&self) -> Result<(), String> {
        endpoint(&self.authorization_url)?;
        endpoint(&self.token_url)?;
        if uuid::Uuid::parse_str(&self.id).is_err() || self.client_id.trim().is_empty() {
            return Err("请填写 Client ID".into());
        }
        let redirect = Url::parse(&self.redirect_uri).map_err(|_| "回调地址无效")?;
        if redirect.host_str().is_none()
            || redirect.fragment().is_some()
            || redirect.query().is_some()
            || !redirect.username().is_empty()
            || redirect.password().is_some()
            || !(redirect.scheme() == "https" || is_loopback(&redirect))
        {
            return Err("回调地址须为 HTTPS 或 HTTP 本机回环地址，不含查询参数、片段或凭据".into());
        }
        if !self.manual_callback
            && (!is_loopback(&redirect) || redirect.port_or_known_default() == Some(0))
        {
            return Err("自动接收仅支持 http://127.0.0.1:端口/路径；其他回调请启用手动模式".into());
        }
        let auth = Url::parse(&self.authorization_url).unwrap();
        if auth.query_pairs().any(|(key, _)| {
            matches!(
                key.as_ref(),
                "state"
                    | "client_id"
                    | "redirect_uri"
                    | "response_type"
                    | "scope"
                    | "code_challenge"
                    | "code_challenge_method"
            )
        }) {
            return Err("授权地址不要预填 OAuth 参数，请使用下方配置项".into());
        }
        Ok(())
    }
    /// Include all protocol settings and the environment to isolate credentials and rotations.
    pub fn key(&self, environment: &str) -> String {
        let bytes = serde_json::to_vec(&(self, environment)).expect("OAuth config serializes");
        format!("{:x}", Sha256::digest(bytes))
    }
}
fn is_loopback(url: &Url) -> bool {
    url.scheme() == "http" && url.host_str() == Some("127.0.0.1")
}
fn endpoint(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "OAuth 接口地址无效")?;
    if (url.scheme() != "https" && !is_loopback(&url))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("OAuth 接口须使用 HTTPS（本机 127.0.0.1 调试可使用 HTTP）".into());
    }
    Ok(url)
}
fn nonce() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub struct Session {
    pub url: String,
    pub config: Config,
    state: String,
    verifier: Option<String>,
    deadline: Instant,
    listener: Option<TcpListener>,
    pub cancelled: Arc<AtomicBool>,
    exchanged: AtomicBool,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
impl Session {
    pub fn start(config: Config) -> Result<Self, String> {
        config.validate()?;
        let redirect = Url::parse(&config.redirect_uri).unwrap();
        let listener = if config.manual_callback {
            None
        } else {
            let listener =
                TcpListener::bind(("127.0.0.1", redirect.port_or_known_default().unwrap()))
                    .map_err(|_| "无法监听 OAuth 回调端口，请更换端口或启用手动回调")?;
            listener
                .set_nonblocking(true)
                .map_err(|_| "无法启动回调监听")?;
            Some(listener)
        };
        let state = nonce();
        let verifier = config.pkce.then(nonce);
        let mut url = endpoint(&config.authorization_url)?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &config.client_id)
                .append_pair("redirect_uri", &config.redirect_uri)
                .append_pair("state", &state);
            if !config.scope.is_empty() {
                query.append_pair("scope", &config.scope);
            }
            if let Some(verifier) = &verifier {
                query
                    .append_pair(
                        "code_challenge",
                        &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
                    )
                    .append_pair("code_challenge_method", "S256");
            }
        }
        Ok(Self {
            url: url.into(),
            config,
            state,
            verifier,
            deadline: Instant::now() + Duration::from_secs(600),
            listener,
            cancelled: Arc::new(AtomicBool::new(false)),
            exchanged: AtomicBool::new(false),
        })
    }
    pub fn code(&self, callback: &str) -> Result<String, String> {
        if self.cancelled.load(Ordering::Acquire) || Instant::now() >= self.deadline {
            return Err("授权已取消或超时，请重新授权".into());
        }
        let expected = Url::parse(&self.config.redirect_uri).unwrap();
        let actual =
            Url::parse(callback.trim()).map_err(|_| "请粘贴包含 code 和 state 的完整回调地址")?;
        if actual.origin() != expected.origin()
            || actual.path() != expected.path()
            || actual.fragment().is_some()
            || !actual.username().is_empty()
            || actual.password().is_some()
        {
            return Err("回调地址与本次授权不匹配".into());
        }
        let pairs = actual.query_pairs().collect::<Vec<_>>();
        let single = |key: &str| -> Result<Option<String>, String> {
            let values = pairs.iter().filter(|(k, _)| k == key).collect::<Vec<_>>();
            if values.len() > 1 {
                return Err("回调包含重复的 OAuth 参数".into());
            }
            Ok(values.first().map(|(_, v)| v.to_string()))
        };
        if single("state")?.as_deref() != Some(&self.state) {
            return Err("授权 state 校验失败，请重新授权".into());
        }
        if single("error")?.is_some() {
            return Err("服务商拒绝或取消了本次授权".into());
        }
        single("code")?
            .filter(|c| !c.is_empty())
            .ok_or("回调缺少授权 code".into())
    }
    pub fn wait(&self) -> Result<String, String> {
        let listener = self.listener.as_ref().ok_or("本次授权使用手动回调")?;
        while !self.cancelled.load(Ordering::Acquire) && Instant::now() < self.deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if let Some(callback) = self.read_callback(&mut stream) {
                        match self.code(&callback) {
                            Ok(code) => {
                                respond(&mut stream, true);
                                return Ok(code);
                            }
                            Err(error) => {
                                respond(&mut stream, false);
                                if error == "服务商拒绝或取消了本次授权" {
                                    return Err(error);
                                }
                            }
                        }
                    } else {
                        respond(&mut stream, false);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Err(_) => return Err("OAuth 回调监听失败".into()),
            }
        }
        Err("授权已取消或超时，请重新授权".into())
    }
    fn read_callback(&self, stream: &mut TcpStream) -> Option<String> {
        // BSD/macOS can inherit O_NONBLOCK from the listener.
        stream.set_nonblocking(false).ok()?;
        let deadline = Instant::now() + Duration::from_secs(3);
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .ok()?;
        let mut bytes = vec![];
        let mut buffer = [0; 1024];
        while bytes.len() < 16 * 1024 {
            let remaining = deadline.checked_duration_since(Instant::now())?;
            stream.set_read_timeout(Some(remaining)).ok()?;
            let len = stream.read(&mut buffer).ok()?;
            if len == 0 {
                return None;
            }
            bytes.extend_from_slice(&buffer[..len]);
            if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                break;
            }
        }
        let text = std::str::from_utf8(&bytes).ok()?;
        let mut parts = text.lines().next()?.split_whitespace();
        if parts.next()? != "GET" {
            return None;
        }
        let target = parts.next()?;
        if !target.starts_with('/') || target.starts_with("//") {
            return None;
        }
        let redirect = Url::parse(&self.config.redirect_uri).ok()?;
        let host = text
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
            .map(|(_, value)| value.trim())
            .collect::<Vec<_>>();
        let expected_host = format!("127.0.0.1:{}", redirect.port_or_known_default()?);
        if host != [expected_host.as_str()] {
            return None;
        }
        Some(format!(
            "{}{}",
            redirect.origin().ascii_serialization(),
            target
        ))
    }
    pub fn exchange(
        &self,
        key: &str,
        code: &str,
        secret: &str,
        proxy: &super::environment::ProxySettings,
    ) -> Result<Access, String> {
        if self.cancelled.load(Ordering::Acquire) || Instant::now() >= self.deadline {
            return Err("授权已取消或超时".into());
        }
        self.exchange_using(&SystemVault, key, code, secret, proxy)
    }
    fn exchange_using(
        &self,
        vault: &dyn Vault,
        key: &str,
        code: &str,
        secret: &str,
        proxy: &super::environment::ProxySettings,
    ) -> Result<Access, String> {
        if self.cancelled.load(Ordering::Acquire) || self.exchanged.swap(true, Ordering::AcqRel) {
            return Err("本次授权已取消或授权码已使用，请重新授权".into());
        }
        let _guard = VAULT_LOCK.lock().map_err(|_| "凭据库繁忙")?;
        let mut credential = vault.read(key)?.unwrap_or_default();
        if !secret.is_empty() {
            credential.client_secret = secret.into();
        }
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", self.config.redirect_uri.as_str()),
        ];
        if let Some(verifier) = &self.verifier {
            form.push(("code_verifier", verifier));
        }
        credential.token = Some(token_request_via(
            &self.config,
            &credential.client_secret,
            form,
            None,
            proxy,
        )?);
        if self.cancelled.load(Ordering::Acquire) {
            return Err("授权已取消".into());
        }
        let access = credential.token.as_ref().unwrap().access();
        vault.write(key, &credential)?;
        Ok(access)
    }
}
fn respond(stream: &mut TcpStream, success: bool) {
    let (status, body) = if success {
        (
            "200 OK",
            "Authorization received. Return to Glint API to check the result.",
        )
    } else {
        (
            "400 Bad Request",
            "Invalid OAuth callback. Return to Glint API and try again.",
        )
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

#[derive(Clone)]
pub struct Access {
    pub token: String,
    pub expires_at: u64,
}
impl Access {
    pub fn valid(&self) -> bool {
        self.expires_at > now().saturating_add(30)
    }
}
#[derive(Default, Serialize, Deserialize)]
struct Credential {
    client_secret: String,
    token: Option<Token>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Token {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: u64,
    refresh_expires_at: Option<u64>,
}
impl Token {
    fn access(&self) -> Access {
        Access {
            token: self.access_token.clone(),
            expires_at: self.expires_at,
        }
    }
}
static VAULT_LOCK: Mutex<()> = Mutex::new(());
trait Vault {
    fn read(&self, key: &str) -> Result<Option<Credential>, String>;
    fn write(&self, key: &str, credential: &Credential) -> Result<(), String>;
}
struct SystemVault;
impl Vault for SystemVault {
    fn read(&self, key: &str) -> Result<Option<Credential>, String> {
        read_credential(key)
    }
    fn write(&self, key: &str, credential: &Credential) -> Result<(), String> {
        write_credential(key, credential)
    }
}
fn entry(key: &str) -> Result<keyring::Entry, String> {
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Err("此平台尚未提供 OAuth 系统凭据库".into());
    #[allow(unreachable_code)]
    keyring::Entry::new("cn.glint-api.oauth2", key).map_err(|_| "无法打开系统凭据库".into())
}
fn read_credential(key: &str) -> Result<Option<Credential>, String> {
    match entry(key)?.get_password() {
        Ok(value) => serde_json::from_str(&value)
            .map(Some)
            .map_err(|_| "OAuth 凭据损坏，请清除后重新授权".into()),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err("无法读取系统凭据库，请解锁或允许访问".into()),
    }
}
fn write_credential(key: &str, credential: &Credential) -> Result<(), String> {
    let data = serde_json::to_string(credential).map_err(|_| "无法保存 OAuth 凭据")?;
    entry(key)?
        .set_password(&data)
        .map_err(|_| "无法保存到系统凭据库，请重新授权".into())
}
pub fn forget(key: &str) -> Result<(), String> {
    let _guard = VAULT_LOCK.lock().map_err(|_| "凭据库繁忙")?;
    match entry(key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("无法清除系统凭据".into()),
    }
}
pub fn access(
    config: &Config,
    key: &str,
    force_refresh: bool,
    proxy: &super::environment::ProxySettings,
) -> Result<Access, String> {
    access_using_via(&SystemVault, config, key, force_refresh, proxy)
}
#[cfg(test)]
fn access_using(
    vault: &dyn Vault,
    config: &Config,
    key: &str,
    force_refresh: bool,
) -> Result<Access, String> {
    access_using_via(vault, config, key, force_refresh, &Default::default())
}
fn access_using_via(
    vault: &dyn Vault,
    config: &Config,
    key: &str,
    force_refresh: bool,
    proxy: &super::environment::ProxySettings,
) -> Result<Access, String> {
    config.validate()?;
    // Serialize refresh and replacement: refresh tokens may be single use.
    let _guard = VAULT_LOCK.lock().map_err(|_| "凭据库繁忙")?;
    let mut credential = vault.read(key)?.ok_or("尚未授权，请先点击授权")?;
    let old = credential.token.as_ref().ok_or("尚未授权，请先点击授权")?;
    if !force_refresh && old.access().valid() {
        return Ok(old.access());
    }
    let refresh = old
        .refresh_token
        .as_deref()
        .filter(|v| !v.is_empty())
        .ok_or("Token 已到期且没有刷新凭据，请重新授权")?;
    if old.refresh_expires_at.is_some_and(|expiry| expiry <= now()) {
        return Err("刷新凭据已到期，请重新授权".into());
    }
    let token = token_request_via(
        config,
        &credential.client_secret,
        vec![("grant_type", "refresh_token"), ("refresh_token", refresh)],
        Some(old),
        proxy,
    )?;
    let access = token.access();
    credential.token = Some(token);
    // One vault write replaces the whole pair; never persist either half separately.
    vault.write(key, &credential)?;
    Ok(access)
}
#[cfg(test)]
fn token_request<'a>(
    config: &'a Config,
    secret: &'a str,
    form: Vec<(&'a str, &'a str)>,
    old: Option<&Token>,
) -> Result<Token, String> {
    token_request_via(config, secret, form, old, &Default::default())
}
fn token_request_via<'a>(
    config: &'a Config,
    secret: &'a str,
    mut form: Vec<(&'a str, &'a str)>,
    old: Option<&Token>,
    proxy: &super::environment::ProxySettings,
) -> Result<Token, String> {
    proxy.validate()?;
    let mut builder = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10));
    builder = proxy.apply_blocking(builder)?;
    // OAuth credentials always require certificate validation, even if a business request opts out.
    let client = builder.build().map_err(|_| "无法创建 OAuth 客户端")?;
    let mut request = client.post(endpoint(&config.token_url)?);
    if config.basic_auth {
        let encode = |v: &str| form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>();
        request = request.basic_auth(encode(&config.client_id), Some(encode(secret)));
    } else {
        form.push(("client_id", &config.client_id));
        if !secret.is_empty() {
            form.push(("client_secret", secret));
        }
    }
    let response = request
        .form(&form)
        .send()
        .map_err(|_| "OAuth Token 请求失败或超时")?;
    let status = response.status();
    let mut body = vec![];
    response
        .take(128 * 1024 + 1)
        .read_to_end(&mut body)
        .map_err(|_| "无法读取 Token 响应")?;
    if !status.is_success() {
        return Err(format!(
            "OAuth Token 请求失败（HTTP {}），请检查应用配置或重新授权",
            status.as_u16()
        ));
    }
    if body.len() > 128 * 1024 {
        return Err("Token 响应超过大小限制".into());
    }
    let json: serde_json::Value =
        serde_json::from_slice(&body).map_err(|_| "Token 响应不是有效 JSON")?;
    parse_token(&json, old, config.rolling_refresh_expiry)
}
fn parse_token(
    json: &serde_json::Value,
    old: Option<&Token>,
    rolling_refresh_expiry: bool,
) -> Result<Token, String> {
    if json.get("error").is_some() {
        return Err("服务商拒绝了 Token 请求，请检查配置或重新授权".into());
    }
    let access_token = json["access_token"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or("响应缺少 access_token")?;
    reqwest::header::HeaderValue::from_str(&format!("Bearer {access_token}"))
        .map_err(|_| "access_token 格式无效")?;
    if !json["token_type"]
        .as_str()
        .is_some_and(|s| s.eq_ignore_ascii_case("bearer"))
    {
        return Err("仅支持 Bearer 类型的 OAuth Token".into());
    }
    let seconds = |name: &str| {
        json[name]
            .as_u64()
            .or_else(|| json[name].as_str().and_then(|v| v.parse().ok()))
    };
    let expires = seconds("expires_in")
        .filter(|n| *n > 30)
        .ok_or("Token 响应缺少有效的 expires_in")?;
    let refresh_expires_at = match (
        seconds("refresh_token_expires_in")
            .or_else(|| seconds("refresh_expires_in"))
            .map(|n| now().saturating_add(n)),
        old.and_then(|v| v.refresh_expires_at),
    ) {
        (Some(a), Some(b)) if !rolling_refresh_expiry => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    Ok(Token {
        access_token: access_token.into(),
        expires_at: now().saturating_add(expires),
        refresh_token: json["refresh_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .or_else(|| old.and_then(|v| v.refresh_token.clone())),
        refresh_expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{collections::HashMap, thread};

    #[derive(Default)]
    struct MemoryVault {
        values: Mutex<HashMap<String, String>>,
        fail_write: AtomicBool,
    }
    impl Vault for MemoryVault {
        fn read(&self, key: &str) -> Result<Option<Credential>, String> {
            Ok(self
                .values
                .lock()
                .unwrap()
                .get(key)
                .map(|v| serde_json::from_str(v).unwrap()))
        }
        fn write(&self, key: &str, credential: &Credential) -> Result<(), String> {
            if self.fail_write.load(Ordering::Acquire) {
                return Err("无法保存到系统凭据库，请重新授权".into());
            }
            self.values
                .lock()
                .unwrap()
                .insert(key.into(), serde_json::to_string(credential).unwrap());
            Ok(())
        }
    }
    fn manual() -> Config {
        Config {
            authorization_url: "https://auth.example.test/authorize".into(),
            token_url: "https://auth.example.test/token".into(),
            client_id: "app-123".into(),
            manual_callback: true,
            ..Config::default()
        }
    }
    fn callback(session: &Session, code: &str) -> String {
        let mut url = Url::parse(&session.config.redirect_uri).unwrap();
        url.query_pairs_mut()
            .append_pair("state", &session.state)
            .append_pair("code", code);
        url.into()
    }
    fn server(replies: Vec<(u16, serde_json::Value)>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let mut requests = vec![];
            for (status, body) in replies {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "missing token request");
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = vec![];
                let mut byte = [0];
                while !bytes.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    bytes.push(byte[0]);
                    assert!(bytes.len() < 65536);
                }
                let headers = String::from_utf8(bytes.clone()).unwrap();
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .map(|(_, v)| v.trim().parse().unwrap())
                    })
                    .unwrap();
                let mut body_bytes = vec![0; length];
                stream.read_exact(&mut body_bytes).unwrap();
                bytes.extend(body_bytes);
                requests.push(String::from_utf8(bytes).unwrap());
                let body = body.to_string();
                write!(stream, "HTTP/1.1 {status} Reply\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            requests
        });
        (format!("http://{address}/token"), worker)
    }
    fn reply(access: &str, refresh: &str) -> serde_json::Value {
        json!({"access_token":access,"refresh_token":refresh,"token_type":"bearer","expires_in":7200,"refresh_expires_in":31536000})
    }
    #[test]
    fn oauth_templates_resolve_runtime_configuration_and_exchange_without_changing_draft() {
        let (url, worker) = server(vec![(200, reply("access", "refresh"))]);
        let values = super::super::environment::EnvironmentValues(
            [
                ("host".into(), "https://auth.example.test".into()),
                ("app_id".into(), "app&one".into()),
                ("alias".into(), "{{app_id}}".into()),
                ("secret".into(), "secret&encoded".into()),
                ("token_url".into(), url),
                (
                    "callback".into(),
                    "http://127.0.0.1:17943/oauth/callback".into(),
                ),
                ("scopes".into(), "one two".into()),
            ]
            .into_iter()
            .collect(),
        );
        let config = Config {
            authorization_url: "{{host}}/authorize".into(),
            token_url: "{{token_url}}".into(),
            client_id: "{{alias}}".into(),
            redirect_uri: "{{callback}}".into(),
            scope: "{{scopes}}".into(),
            ..manual()
        };
        let resolver = super::super::template::TemplateResolver::new(&values);
        let resolved = config.resolve(&resolver).unwrap();
        let session = Session::start(resolved).unwrap();
        let auth = Url::parse(&session.url).unwrap();
        let query = auth.query_pairs().collect::<HashMap<_, _>>();
        assert_eq!(query["client_id"], "app&one");
        assert_eq!(query["scope"], "one two");
        assert_eq!(
            query["redirect_uri"],
            "http://127.0.0.1:17943/oauth/callback"
        );
        session
            .exchange_using(
                &MemoryVault::default(),
                "test",
                "code",
                &resolver.resolve("{{secret}}").unwrap(),
                &Default::default(),
            )
            .unwrap();
        let requests = worker.join().unwrap();
        let form = form_urlencoded::parse(requests[0].split("\r\n\r\n").nth(1).unwrap().as_bytes())
            .into_owned()
            .collect::<HashMap<_, _>>();
        assert_eq!(form["client_id"], "app&one");
        assert_eq!(form["client_secret"], "secret&encoded");
        assert_eq!(config.client_id, "{{alias}}");
        assert_eq!(config.token_url, "{{token_url}}");
        assert!(
            config
                .resolve(&super::super::template::TemplateResolver::new(
                    &super::super::environment::EnvironmentValues(Default::default())
                ))
                .is_err()
        );
    }

    #[test]
    fn oauth_legacy_config_keeps_credential_key() {
        let config = manual();
        let mut legacy = serde_json::to_value(&config).unwrap();
        assert!(legacy.get("rolling_refresh_expiry").is_none());
        let decoded: Config = serde_json::from_value(legacy.clone()).unwrap();
        // Exact field order of the pre-policy Config is part of its credential key.
        let original = format!(
            r#"[{{"id":"{}","authorization_url":"https://auth.example.test/authorize","token_url":"https://auth.example.test/token","client_id":"app-123","redirect_uri":"http://127.0.0.1:17943/oauth/callback","scope":"","pkce":true,"basic_auth":false,"manual_callback":true}},"env"]"#,
            config.id
        );
        assert_eq!(
            decoded.key("env"),
            format!("{:x}", Sha256::digest(original.as_bytes()))
        );
        legacy["rolling_refresh_expiry"] = json!(true);
        let rolling: Config = serde_json::from_value(legacy).unwrap();
        assert_ne!(rolling.key("env"), config.key("env"));
    }

    #[test]
    fn feishu_v3_form_exchange_and_refresh_use_rotated_expiry() {
        let response = |access: &str, refresh: &str| {
            json!({
                "code": 0, "access_token": access, "refresh_token": refresh,
                "token_type": "Bearer", "expires_in": 7200,
                "refresh_token_expires_in": 604800, "scope": "offline_access"
            })
        };
        let (url, worker) = server(vec![
            (200, response("feishu-access-one", "feishu-refresh-one")),
            (200, response("feishu-access-two", "feishu-refresh-two")),
        ]);
        let plugin = super::super::plugins::catalog()
            .0
            .into_iter()
            .find(|p| p.manifest.id == "com.feishu.oauth")
            .unwrap();
        let mut config = super::super::plugins::oauth_presets(&[plugin])[0]
            .preset
            .config()
            .unwrap();
        config.client_id = "cli_feishu".into();
        config.scope = "offline_access contact:user.base:readonly".into();
        config.token_url = url;
        config.manual_callback = true;
        let session = Session::start(config).unwrap();
        let auth = Url::parse(&session.url).unwrap();
        let query = auth.query_pairs().collect::<HashMap<_, _>>();
        assert_eq!(query["scope"], "offline_access contact:user.base:readonly");
        assert_eq!(query["state"], session.state);
        assert!(!query.contains_key("code_challenge"));
        let vault = MemoryVault::default();
        session
            .exchange_using(
                &vault,
                "feishu",
                "one-use-code",
                "app-secret",
                &Default::default(),
            )
            .unwrap();
        let mut credential = vault.read("feishu").unwrap().unwrap();
        let token = credential.token.as_mut().unwrap();
        assert!(token.refresh_expires_at.unwrap() >= now() + 604790);
        token.expires_at = now();
        token.refresh_expires_at = Some(now() + 1000);
        vault.write("feishu", &credential).unwrap();
        assert_eq!(
            access_using(&vault, &session.config, "feishu", false)
                .unwrap()
                .token,
            "feishu-access-two"
        );
        let saved = vault.read("feishu").unwrap().unwrap().token.unwrap();
        assert_eq!(saved.refresh_token.as_deref(), Some("feishu-refresh-two"));
        assert!(saved.refresh_expires_at.unwrap() >= now() + 604790);
        // If a service omits the new expiry, do not invent an extension.
        let missing = json!({"access_token":"next", "token_type":"Bearer", "expires_in":7200});
        assert_eq!(
            parse_token(&missing, Some(&saved), true)
                .unwrap()
                .refresh_expires_at,
            saved.refresh_expires_at
        );
        let requests = worker.join().unwrap();
        for (i, request) in requests.iter().enumerate() {
            let body = request.split("\r\n\r\n").nth(1).unwrap();
            let form = form_urlencoded::parse(body.as_bytes())
                .into_owned()
                .collect::<HashMap<_, _>>();
            assert_eq!(form["client_id"], "cli_feishu");
            assert_eq!(form["client_secret"], "app-secret");
            assert!(!form.contains_key("code_verifier"));
            if i == 0 {
                assert_eq!(form["grant_type"], "authorization_code");
                assert_eq!(form["code"], "one-use-code");
                assert_eq!(form["redirect_uri"], session.config.redirect_uri);
            } else {
                assert_eq!(form["grant_type"], "refresh_token");
                assert_eq!(form["refresh_token"], "feishu-refresh-one");
            }
            assert!(
                request
                    .to_lowercase()
                    .contains("content-type: application/x-www-form-urlencoded")
            );
        }
    }

    #[test]
    fn oauth_authorization_url_encodes_scope_redirect_state_and_pkce() {
        let mut config = manual();
        config.scope = "one,two three".into();
        config.client_id = "app&=?".into();
        let session = Session::start(config.clone()).unwrap();
        let url = Url::parse(&session.url).unwrap();
        let query = url.query_pairs().collect::<HashMap<_, _>>();
        assert_eq!(query["client_id"], config.client_id);
        assert_eq!(query["scope"], config.scope);
        assert_eq!(query["redirect_uri"], config.redirect_uri);
        assert_eq!(query["response_type"], "code");
        assert_eq!(query["state"], session.state);
        assert_eq!(query["code_challenge_method"], "S256");
        assert_eq!(
            query["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(
                session.verifier.as_ref().unwrap().as_bytes()
            ))
        );
        let wps = Session::start(Config {
            client_id: "wps-app".into(),
            manual_callback: true,
            ..super::super::plugins::test_oauth_config()
        })
        .unwrap();
        assert!(!wps.url.contains("code_challenge"));
    }
    #[test]
    fn oauth_callback_checks_origin_path_state_duplicates_denial_and_deadline() {
        let mut session = Session::start(manual()).unwrap();
        let valid = callback(&session, "code&with=symbols");
        assert_eq!(session.code(&valid).unwrap(), "code&with=symbols");
        for bad in [
            valid.replace("127.0.0.1", "evil.test"),
            valid.replace("/oauth/callback", "/other"),
            valid.replace(&session.state, "wrong"),
            format!("{valid}&state=extra"),
            format!("{valid}&code=extra"),
            format!("{valid}#ignored"),
        ] {
            assert!(session.code(&bad).is_err());
        }
        assert!(
            session
                .code(&format!("{valid}&error=access_denied"))
                .err()
                .unwrap()
                .contains("拒绝")
        );
        session.deadline = Instant::now();
        assert!(session.code(&valid).is_err());
    }
    #[test]
    fn oauth_loopback_ignores_unrelated_callbacks_and_closes_after_cancel() {
        let free = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = free.local_addr().unwrap();
        drop(free);
        let session = Arc::new(
            Session::start(Config {
                redirect_uri: format!("http://{address}/oauth/callback"),
                manual_callback: false,
                ..manual()
            })
            .unwrap(),
        );
        let valid = callback(&session, "test-code");
        let worker_session = session.clone();
        let worker = thread::spawn(move || worker_session.wait());
        for (url, expected) in [
            (valid.replace(&session.state, "incorrect"), "400"),
            (valid, "200"),
        ] {
            let target = Url::parse(&url).unwrap();
            let mut stream = TcpStream::connect(address).unwrap();
            write!(
                stream,
                "GET {}?{} HTTP/1.1\r\nHost: {address}\r\n\r\n",
                target.path(),
                target.query().unwrap()
            )
            .unwrap();
            let mut response = String::new();
            let result = stream.read_to_string(&mut response);
            assert!(
                result.is_ok()
                    || result
                        .is_err_and(|error| error.kind() == std::io::ErrorKind::ConnectionReset)
            );
            assert!(response.starts_with(&format!("HTTP/1.1 {expected}")));
            assert!(!response.contains("test-code"));
        }
        assert_eq!(worker.join().unwrap().unwrap(), "test-code");
        drop(session);
        let again = TcpListener::bind(address).unwrap();
        drop(again);
        let session = Arc::new(
            Session::start(Config {
                redirect_uri: format!("http://{address}/oauth/callback"),
                manual_callback: false,
                ..manual()
            })
            .unwrap(),
        );
        let cancel = session.cancelled.clone();
        let worker = thread::spawn(move || session.wait());
        cancel.store(true, Ordering::Release);
        assert!(worker.join().unwrap().is_err());
        assert!(TcpListener::bind(address).is_ok());
    }
    #[test]
    fn oauth_token_exchange_and_refresh_rotate_and_preserve_absolute_deadline() {
        let (url, worker) = server(vec![
            (200, reply("access-one", "refresh-one")),
            (200, reply("access-two", "refresh-two")),
        ]);
        let session = Session::start(Config {
            token_url: url,
            ..manual()
        })
        .unwrap();
        let vault = MemoryVault::default();
        assert_eq!(
            session
                .exchange_using(
                    &vault,
                    "test",
                    "code&encoded",
                    "app-secret",
                    &Default::default()
                )
                .unwrap()
                .token,
            "access-one"
        );
        assert!(
            session
                .exchange_using(&vault, "test", "again", "", &Default::default())
                .is_err()
        );
        let mut credential = vault.read("test").unwrap().unwrap();
        credential.token.as_mut().unwrap().expires_at = now();
        let refresh_deadline = now() + 1000;
        credential.token.as_mut().unwrap().refresh_expires_at = Some(refresh_deadline);
        vault.write("test", &credential).unwrap();
        let access = access_using(&vault, &session.config, "test", false).unwrap();
        assert_eq!(access.token, "access-two");
        // A valid access token is reused: no third network request.
        assert_eq!(
            access_using(&vault, &session.config, "test", false)
                .unwrap()
                .token,
            "access-two"
        );
        let saved = vault.read("test").unwrap().unwrap().token.unwrap();
        assert_eq!(saved.refresh_token.as_deref(), Some("refresh-two"));
        assert_eq!(saved.refresh_expires_at, Some(refresh_deadline));
        let requests = worker.join().unwrap();
        let first =
            form_urlencoded::parse(requests[0].split("\r\n\r\n").nth(1).unwrap().as_bytes())
                .into_owned()
                .collect::<HashMap<_, _>>();
        assert_eq!(first["grant_type"], "authorization_code");
        assert_eq!(first["code"], "code&encoded");
        assert_eq!(first["client_secret"], "app-secret");
        assert_eq!(
            first["code_verifier"],
            session.verifier.as_ref().unwrap().as_str()
        );
        assert!(
            requests[0]
                .to_lowercase()
                .contains("content-type: application/x-www-form-urlencoded")
        );
        assert!(requests[1].contains("refresh_token=refresh-one"));
        assert!(!requests[1].contains("code_verifier"));
    }
    #[test]
    fn oauth_concurrent_sends_share_one_refresh_and_reuse_rotated_pair() {
        let (url, worker) = server(vec![(200, reply("shared-access", "rotated-refresh"))]);
        let config = Config {
            token_url: url,
            ..manual()
        };
        let vault = Arc::new(MemoryVault::default());
        let mut token = parse_token(&reply("expired", "single-use-refresh"), None, false).unwrap();
        token.expires_at = now();
        vault
            .write(
                "shared",
                &Credential {
                    client_secret: "test-secret".into(),
                    token: Some(token),
                },
            )
            .unwrap();
        let gate = Arc::new(std::sync::Barrier::new(3));
        let workers = (0..2)
            .map(|_| {
                let config = config.clone();
                let vault = vault.clone();
                let gate = gate.clone();
                thread::spawn(move || {
                    gate.wait();
                    access_using(vault.as_ref(), &config, "shared", false)
                        .unwrap()
                        .token
                })
            })
            .collect::<Vec<_>>();
        gate.wait();
        for worker in workers {
            assert_eq!(worker.join().unwrap(), "shared-access");
        }
        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].contains("refresh_token=single-use-refresh"));
        assert_eq!(
            vault
                .read("shared")
                .unwrap()
                .unwrap()
                .token
                .unwrap()
                .refresh_token
                .as_deref(),
            Some("rotated-refresh")
        );
    }
    #[test]
    fn oauth_basic_client_auth_encodes_credentials_and_errors_do_not_echo_secrets() {
        let (url, worker) = server(vec![
            (200, reply("ok", "refresh")),
            (400, json!({"error":"private-app-secret"})),
        ]);
        let config = Config {
            token_url: url,
            basic_auth: true,
            client_id: "app:id".into(),
            ..manual()
        };
        assert!(
            token_request(
                &config,
                "s+ecret",
                vec![("grant_type", "refresh_token")],
                None,
            )
            .is_ok()
        );
        let error = token_request(
            &config,
            "private-app-secret",
            vec![("grant_type", "refresh_token")],
            None,
        )
        .err()
        .unwrap();
        assert!(!error.contains("private-app-secret"));
        let requests = worker.join().unwrap();
        let encoded = base64::engine::general_purpose::STANDARD.encode("app%3Aid:s%2Becret");
        assert!(requests[0].contains(&format!("Basic {encoded}")));
        assert!(!requests[0].contains("client_secret="));
    }
    #[test]
    fn oauth_invalid_tokens_expired_refresh_and_vault_errors_stop_sending() {
        assert!(
            parse_token(
                &json!({"access_token":"secret","expires_in":7200,"token_type":"mac"}),
                None,
                false,
            )
            .is_err()
        );
        assert!(parse_token(&json!({"access_token":"secret\r\nInjected: x","expires_in":7200,"token_type":"bearer"}), None, false).is_err());
        assert!(
            parse_token(
                &json!({"access_token":"secret","token_type":"bearer"}),
                None,
                false,
            )
            .is_err()
        );
        let mut token = parse_token(&reply("old", "expired"), None, false).unwrap();
        token.expires_at = now();
        token.refresh_expires_at = Some(now());
        let vault = MemoryVault::default();
        vault
            .write(
                "test",
                &Credential {
                    client_secret: "".into(),
                    token: Some(token),
                },
            )
            .unwrap();
        assert!(
            access_using(&vault, &manual(), "test", false)
                .err()
                .unwrap()
                .contains("到期")
        );
        let (url, worker) = server(vec![(200, reply("new", "new-refresh"))]);
        let session = Session::start(Config {
            token_url: url,
            ..manual()
        })
        .unwrap();
        vault.fail_write.store(true, Ordering::Release);
        assert!(
            session
                .exchange_using(&vault, "other", "code", "secret", &Default::default())
                .err()
                .unwrap()
                .contains("凭据库")
        );
        assert!(vault.read("other").unwrap().is_none());
        worker.join().unwrap();
    }
    #[test]
    fn oauth_config_rejects_unsafe_endpoints_and_isolates_environment_and_scope() {
        let config = manual();
        assert!(config.validate().is_ok());
        assert_ne!(config.key("workspace/dev"), config.key("workspace/prod"));
        let other = Config {
            scope: "more-permissions".into(),
            ..config.clone()
        };
        assert_ne!(config.key("dev"), other.key("dev"));
        for url in [
            "http://evil.test/token",
            "file:///tmp/token",
            "https://secret@auth.test/token",
            "http://127.0.0.2/token",
        ] {
            assert!(
                Config {
                    token_url: url.into(),
                    ..config.clone()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            Config {
                authorization_url: "https://auth.test/?state=fixed".into(),
                ..config.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            Config {
                redirect_uri: "https://my.callback.test/".into(),
                manual_callback: false,
                ..config
            }
            .validate()
            .is_err()
        );
    }
}
