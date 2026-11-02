//! Public-client Sign in with ChatGPT. Tokens never enter provider metadata.
use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use fs2::FileExt;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::File, path::PathBuf};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};

const ISSUER: &str = "https://auth.openai.com";
const TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
const RESOURCE: &str = "https://api.openai.com/v1";
const DIRECT_SCOPE: &str = "chatgpt.tokens.use.direct";

#[derive(Clone, Serialize, Deserialize)]
struct Credentials {
    client_id: String,
    subject: String,
    email: String,
    access_token: String,
    refresh_token: String,
    id_token: String,
    scopes: Vec<String>,
    expires_at: u64,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn database(config: &Config) -> Result<rusqlite::Connection, String> {
    let anchor = config
        .credential_store
        .as_deref()
        .ok_or("ChatGPT 凭据存储尚未初始化")?;
    let connection = crate::database::open(anchor)?;
    connection.execute_batch("CREATE TABLE IF NOT EXISTS chatgpt_auth(provider_id TEXT PRIMARY KEY, payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS chatgpt_host(id INTEGER PRIMARY KEY CHECK(id=1), host_id TEXT NOT NULL);")
        .map_err(|_| "无法初始化 ChatGPT 授权存储")?;
    Ok(connection)
}

fn load(config: &Config) -> Result<Option<Credentials>, String> {
    let raw: Option<String> = database(config)?
        .query_row(
            "SELECT payload FROM chatgpt_auth WHERE provider_id=?1",
            [config.id.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| "无法读取 ChatGPT 授权")?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(|_| "ChatGPT 授权记录损坏，请重新登录".into()))
        .transpose()
}

fn save(config: &Config, credentials: &Credentials) -> Result<(), String> {
    let payload = serde_json::to_string(credentials).map_err(|_| "无法编码 ChatGPT 授权")?;
    database(config)?.execute("INSERT INTO chatgpt_auth VALUES(?1,?2) ON CONFLICT(provider_id) DO UPDATE SET payload=excluded.payload", params![config.id.to_string(), payload])
        .map_err(|_| "无法保存 ChatGPT 授权")?;
    Ok(())
}

fn host_id(config: &Config) -> Result<String, String> {
    let db = database(config)?;
    db.execute(
        "INSERT OR IGNORE INTO chatgpt_host VALUES(1,?1)",
        [format!("urn:uuid:{}", uuid::Uuid::new_v4())],
    )
    .map_err(|_| "无法保存 ChatGPT 主机标识")?;
    db.query_row("SELECT host_id FROM chatgpt_host WHERE id=1", [], |row| {
        row.get(0)
    })
    .map_err(|_| "无法读取 ChatGPT 主机标识".into())
}

// A file lock serializes rotating refresh tokens across tasks AND app processes.
// Cancellation drops the handle. Never block a runtime worker waiting for it.
async fn lock(config: &Config) -> Result<File, String> {
    let anchor = config
        .credential_store
        .as_deref()
        .ok_or("ChatGPT 凭据存储尚未初始化")?;
    let path: PathBuf =
        crate::database::path(anchor).with_extension(format!("chatgpt-{}.lock", config.id));
    let _ = database(config)?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|_| "无法打开 ChatGPT 授权锁")?;
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(()),
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
                {
                    tokio::time::sleep(Duration::from_millis(50)).await
                }
                Err(_) => return Err("无法锁定 ChatGPT 授权".to_owned()),
            }
        }
    })
    .await
    .map_err(|_| "ChatGPT 授权正忙，请稍后重试")??;
    Ok(file)
}

pub struct Login {
    config: Config,
    listener: TcpListener,
    redirect: String,
    state: String,
    nonce: String,
    verifier: String,
    previous: Option<Credentials>,
}

fn random() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

pub async fn start_login(config: &Config) -> Result<(Login, String), String> {
    let _guard = lock(config).await?;
    let previous = load(config)?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| "无法启动 ChatGPT 本机登录回调")?;
    let redirect = format!(
        "http://127.0.0.1:{}/auth/callback",
        listener
            .local_addr()
            .map_err(|_| "无法获取回调端口")?
            .port()
    );
    let login = Login {
        config: config.clone(),
        listener,
        redirect,
        state: random(),
        nonce: random(),
        verifier: random(),
        previous,
    };
    let mut url = reqwest::Url::parse("https://auth.openai.com/api/accounts/authorize").unwrap();
    {
        let mut query = url.query_pairs_mut();
        query.extend_pairs([
            (
                "client_id",
                login
                    .previous
                    .as_ref()
                    .map(|c| c.client_id.as_str())
                    .unwrap_or("dynamic_agent_client"),
            ),
            ("ext_agent_host_id", host_id(config)?.as_str()),
            ("response_type", "code"),
            ("redirect_uri", &login.redirect),
            (
                "scope",
                "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct",
            ),
            ("resource", RESOURCE),
            ("state", &login.state),
            ("nonce", &login.nonce),
            ("code_challenge_method", "S256"),
            (
                "code_challenge",
                &URL_SAFE_NO_PAD.encode(Sha256::digest(login.verifier.as_bytes())),
            ),
        ]);
        if let Some(previous) = &login.previous {
            if !previous.id_token.is_empty() {
                query.append_pair("id_token_hint", &previous.id_token);
            }
            if !previous.email.is_empty() {
                query.append_pair("login_hint", &previous.email);
            }
        } else {
            query.append_pair("agent_name_hint", "Glint Agent");
        }
    }
    Ok((login, url.into()))
}

fn callback(login: &Login, target: &str) -> Result<(String, String), String> {
    if !target.starts_with("/auth/callback?") {
        return Err("无效的登录回调路径".into());
    }
    let url =
        reqwest::Url::parse(&format!("http://127.0.0.1{target}")).map_err(|_| "无效的登录回调")?;
    let mut fields = std::collections::HashMap::new();
    for (key, value) in url.query_pairs() {
        if fields
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Err("登录回调含重复参数".into());
        }
    }
    if fields.get("state") != Some(&login.state) {
        return Err("登录回调校验失败，请重试".into());
    }
    if fields.contains_key("error") {
        return Err("ChatGPT 授权未完成，请重新登录并允许使用订阅".into());
    }
    let client = fields
        .get("client_id")
        .cloned()
        .or_else(|| login.previous.as_ref().map(|c| c.client_id.clone()))
        .filter(|id| !id.is_empty() && id != "dynamic_agent_client")
        .ok_or("ChatGPT 未返回注册客户端标识")?;
    if login
        .previous
        .as_ref()
        .is_some_and(|c| c.client_id != client)
    {
        return Err("ChatGPT 返回了不同的账号注册，请新增供应商登录其他账号".into());
    }
    let code = fields
        .get("code")
        .filter(|code| !code.is_empty())
        .ok_or("登录回调缺少授权码")?
        .clone();
    Ok((client, code))
}

#[derive(Clone, Deserialize)]
struct Identity {
    sub: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    nonce: Option<String>,
}

async fn identity(
    client: &reqwest::Client,
    token: &str,
    client_id: &str,
    nonce: Option<&str>,
) -> Result<Identity, String> {
    use jsonwebtoken::jwk::JwkSet;
    let discovery = response_json(
        client
            .get("https://auth.openai.com/.well-known/openid-configuration")
            .send()
            .await
            .map_err(|_| "无法获取 OpenAI 登录配置")?,
    )
    .await?;
    if discovery["issuer"] != ISSUER {
        return Err("OpenAI 登录配置签发方无效".into());
    }
    let jwks_uri = discovery["jwks_uri"]
        .as_str()
        .ok_or("OpenAI 登录配置缺少公钥地址")?;
    let url = reqwest::Url::parse(jwks_uri).map_err(|_| "OpenAI 公钥地址无效")?;
    if url.scheme() != "https"
        || url.host_str() != Some("auth.openai.com")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("OpenAI 公钥地址无效".into());
    }
    let jwks: JwkSet = serde_json::from_value(
        response_json(
            client
                .get(url)
                .timeout(Duration::from_secs(30))
                .send()
                .await
                .map_err(|_| "无法获取 OpenAI 签名公钥")?,
        )
        .await?,
    )
    .map_err(|_| "OpenAI 签名公钥无效")?;
    validate_identity(token, &jwks, client_id, nonce)
}

fn validate_identity(
    token: &str,
    jwks: &jsonwebtoken::jwk::JwkSet,
    client_id: &str,
    nonce: Option<&str>,
) -> Result<Identity, String> {
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
    let header = decode_header(token).map_err(|_| "ChatGPT 身份令牌无效")?;
    if !matches!(header.alg, Algorithm::RS256 | Algorithm::ES256) {
        return Err("ChatGPT 身份签名算法无效".into());
    }
    let key = header
        .kid
        .as_deref()
        .and_then(|kid| jwks.find(kid))
        .ok_or("ChatGPT 身份签名公钥未知")?;
    let mut validation = Validation::new(header.alg);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[client_id]);
    validation.set_required_spec_claims(&["sub", "exp", "iat"]);
    validation.leeway = 5;
    validation.validate_nbf = true;
    let identity = decode::<Identity>(
        token,
        &DecodingKey::from_jwk(key).map_err(|_| "OpenAI 签名公钥无效")?,
        &validation,
    )
    .map_err(|_| "ChatGPT 身份签名或有效期校验失败")?
    .claims;
    if identity.sub.is_empty()
        || nonce.is_some_and(|nonce| identity.nonce.as_deref() != Some(nonce))
    {
        return Err("ChatGPT 身份校验失败".into());
    }
    Ok(identity)
}

fn updated(
    tokens: &Value,
    previous: Option<&Credentials>,
    client_id: String,
    identity: Identity,
) -> Result<Credentials, String> {
    let scopes: Vec<String> = tokens["scope"]
        .as_str()
        .map(|s| s.split_whitespace().map(str::to_owned).collect())
        .or_else(|| previous.map(|c| c.scopes.clone()))
        .unwrap_or_default();
    if !scopes.iter().any(|s| s == DIRECT_SCOPE) || !scopes.iter().any(|s| s == "resource.invoke") {
        return Err("尚未授权使用 ChatGPT 订阅，请重新登录并允许订阅访问".into());
    }
    if !tokens["token_type"]
        .as_str()
        .is_some_and(|s| s.eq_ignore_ascii_case("bearer"))
    {
        return Err("ChatGPT 返回了无效令牌类型".into());
    }
    if previous.is_some_and(|p| p.subject != identity.sub) {
        return Err("ChatGPT 登录账号与所选授权不一致，请新增供应商".into());
    }
    let access_token = tokens["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("ChatGPT 未返回访问令牌")?
        .to_owned();
    let expires = tokens["expires_in"]
        .as_u64()
        .filter(|s| *s > 0)
        .ok_or("ChatGPT 未返回令牌有效期")?;
    let refresh_token = tokens["refresh_token"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| previous.map(|c| c.refresh_token.clone()))
        .ok_or("ChatGPT 未返回续期令牌")?;
    let id_token = tokens["id_token"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| previous.map(|c| c.id_token.clone()))
        .ok_or("ChatGPT 未返回身份令牌")?;
    Ok(Credentials {
        client_id,
        subject: identity.sub,
        email: identity.email,
        access_token,
        refresh_token,
        id_token,
        scopes,
        expires_at: now().saturating_add(expires),
    })
}

pub async fn finish_login(login: Login) -> Result<String, String> {
    tokio::time::timeout(Duration::from_secs(600), finish_login_inner(login))
        .await
        .map_err(|_| "ChatGPT 登录已超时，请重试".to_owned())?
}

async fn finish_login_inner(login: Login) -> Result<String, String> {
    let (stream, _) = login
        .listener
        .accept()
        .await
        .map_err(|_| "读取登录回调失败")?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    // Bound and time-limit untrusted loopback HTTP input.
    use tokio::io::AsyncReadExt;
    let count = tokio::time::timeout(
        Duration::from_secs(5),
        (&mut reader).take(8192).read_line(&mut line),
    )
    .await
    .map_err(|_| "读取登录回调超时")?
    .map_err(|_| "读取登录回调失败")?;
    let result = if count >= 8192 || !line.ends_with('\n') {
        Err("登录回调超过大小限制".into())
    } else {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() == 3 && parts[0] == "GET" && parts[2] == "HTTP/1.1" {
            callback(&login, parts[1])
        } else {
            Err("无效的登录回调请求".into())
        }
    };
    let page = if result.is_ok() {
        "ChatGPT authorization received. Return to Glint to check sign-in."
    } else {
        "ChatGPT authorization failed. Return to Glint and try again."
    };
    let _ = reader.get_mut().write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{page}",page.len()).as_bytes()).await;
    let (client_id, code) = result?;
    let client = http_client().await?;
    let tokens = response_json(
        client
            .post(TOKEN)
            .timeout(Duration::from_secs(30))
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", client_id.as_str()),
                ("code", &code),
                ("code_verifier", &login.verifier),
                ("redirect_uri", &login.redirect),
                ("resource", RESOURCE),
            ])
            .send()
            .await
            .map_err(|_| "ChatGPT 授权码交换失败，请重新登录")?,
    )
    .await?;
    let token = tokens["id_token"]
        .as_str()
        .ok_or("ChatGPT 未返回身份令牌")?;
    let id = identity(&client, token, &client_id, Some(&login.nonce)).await?;
    let credentials = updated(&tokens, login.previous.as_ref(), client_id, id)?;
    let _guard = lock(&login.config).await?;
    let current = load(&login.config)?;
    let registration = |c: &Credentials| (c.client_id.clone(), c.subject.clone());
    if current.as_ref().map(registration) != login.previous.as_ref().map(registration)
        || login
            .previous
            .as_ref()
            .is_some_and(|p| !p.id_token.is_empty())
            && current.as_ref().is_some_and(|c| c.id_token.is_empty())
    {
        return Err("ChatGPT 授权状态已改变，请重新登录".into());
    }
    save(&login.config, &credentials)?;
    Ok(format!(
        "已登录 ChatGPT · {} · {}",
        credentials.email, credentials.client_id
    ))
}

pub(super) async fn access(config: &Config, stale: Option<&str>) -> Result<String, String> {
    let config = config.clone();
    let stale = stale.map(str::to_owned);
    // Once refresh starts, finish storing a rotating token even if its caller
    // cancels a generation. Dropping this JoinHandle detaches the short job.
    tokio::spawn(async move { access_inner(&config, stale.as_deref()).await })
        .await
        .map_err(|_| "ChatGPT 授权任务中断".to_owned())?
}

async fn access_inner(config: &Config, stale: Option<&str>) -> Result<String, String> {
    let _guard = lock(config).await?;
    let credentials = load(config)?.ok_or("尚未登录 ChatGPT，请通过浏览器授权订阅")?;
    if !credentials.access_token.is_empty()
        && credentials.expires_at > now() + 60
        && stale != Some(credentials.access_token.as_str())
    {
        return Ok(credentials.access_token);
    }
    if credentials.refresh_token.is_empty() {
        return Err("ChatGPT 登录已失效，请重新登录".into());
    }
    refresh(config, &credentials, &http_client().await?, TOKEN).await
}

async fn refresh(
    config: &Config,
    credentials: &Credentials,
    client: &reqwest::Client,
    endpoint: &str,
) -> Result<String, String> {
    let tokens = response_json(
        client
            .post(endpoint)
            .timeout(Duration::from_secs(30))
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", credentials.client_id.as_str()),
                ("refresh_token", credentials.refresh_token.as_str()),
                ("resource", RESOURCE),
            ])
            .send()
            .await
            .map_err(|_| "ChatGPT 令牌续期失败，请稍后重试")?,
    )
    .await?;
    let id = if let Some(token) = tokens["id_token"].as_str() {
        identity(&client, token, &credentials.client_id, None).await?
    } else {
        Identity {
            sub: credentials.subject.clone(),
            email: credentials.email.clone(),
            nonce: None,
        }
    };
    let refreshed = updated(
        &tokens,
        Some(credentials),
        credentials.client_id.clone(),
        id,
    )?;
    save(config, &refreshed)?;
    Ok(refreshed.access_token)
}

pub async fn account(config: &Config) -> Result<String, String> {
    access(config, None).await?;
    let credentials = load(config)?.ok_or("尚未登录 ChatGPT")?;
    Ok(format!(
        "已登录 ChatGPT · {} · {}（模型与额度以账号权限为准）",
        credentials.email, credentials.client_id
    ))
}

pub fn saved_account_label(config: &Config) -> Option<String> {
    if config.kind != crate::provider::Kind::ChatGpt {
        return None;
    }
    let credentials = load(config).ok()??;
    if credentials.access_token.is_empty() && credentials.refresh_token.is_empty() {
        return None;
    }
    Some(format!("{} · {}", credentials.email, credentials.client_id))
}

pub async fn sign_out(config: &Config) -> Result<String, String> {
    let config = config.clone();
    tokio::spawn(async move { sign_out_inner(&config).await })
        .await
        .map_err(|_| "ChatGPT 退出任务中断".to_owned())?
}

async fn sign_out_inner(config: &Config) -> Result<String, String> {
    let _guard = lock(config).await?;
    let Some(mut credentials) = load(config)? else {
        return Ok("尚未登录 ChatGPT".into());
    };
    let revoked = async {
        let client = http_client().await?;
        let discovery = response_json(
            client
                .get("https://auth.openai.com/.well-known/openid-configuration")
                .timeout(Duration::from_secs(15))
                .send()
                .await
                .map_err(|_| "无法读取注销配置")?,
        )
        .await?;
        let endpoint = discovery["revocation_endpoint"]
            .as_str()
            .ok_or("缺少注销地址")?;
        let url = reqwest::Url::parse(endpoint).map_err(|_| "无效注销地址")?;
        if url.scheme() != "https"
            || url.host_str() != Some("auth.openai.com")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("无效注销地址".to_owned());
        }
        for attempt in 0..3 {
            let response = client
                .post(url.clone())
                .timeout(Duration::from_secs(15))
                .form(&[
                    ("token", credentials.refresh_token.as_str()),
                    ("token_type_hint", "refresh_token"),
                    ("client_id", credentials.client_id.as_str()),
                ])
                .send()
                .await;
            match response {
                Ok(response) if response.status() == reqwest::StatusCode::OK => return Ok(()),
                Ok(response) if !response.status().is_server_error() => {
                    return Err("注销未确认".to_owned());
                }
                _ if attempt < 2 => tokio::time::sleep(Duration::from_secs(1 << attempt)).await,
                _ => return Err("注销未确认".to_owned()),
            }
        }
        Err("注销未确认".to_owned())
    }
    .await;
    credentials.access_token.clear();
    credentials.refresh_token.clear();
    credentials.id_token.clear();
    credentials.expires_at = 0;
    save(config, &credentials)?;
    Ok(if revoked.is_ok() {
        "已退出 ChatGPT；账号注册保留，可重新登录。"
    } else {
        "已清除本机登录；远端撤销未确认，可到 ChatGPT 设置中断开此应用。"
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(root: &std::path::Path) -> Config {
        Config {
            credential_store: Some(root.join("sessions.json")),
            kind: crate::provider::Kind::ChatGpt,
            ..Default::default()
        }
    }
    fn credentials() -> Credentials {
        Credentials {
            client_id: "oaiapp_test".into(),
            subject: "subject".into(),
            email: "user@example.com".into(),
            access_token: "secret-access".into(),
            refresh_token: "secret-refresh".into(),
            id_token: "secret-id".into(),
            scopes: vec![DIRECT_SCOPE.into(), "resource.invoke".into()],
            expires_at: now() + 3600,
        }
    }
    #[tokio::test]
    async fn oauth_uses_pkce_dynamic_registration_and_reuses_host_and_client() {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let (login, url) = start_login(&config).await.unwrap();
        let url = reqwest::Url::parse(&url).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["client_id"], "dynamic_agent_client");
        assert_eq!(query["agent_name_hint"], "Glint Agent");
        assert_eq!(
            query["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(login.verifier.as_bytes()))
        );
        assert_eq!(query["redirect_uri"], login.redirect);
        assert!(query["redirect_uri"].starts_with("http://127.0.0.1:"));
        let target = format!(
            "/auth/callback?state={}&code=code&client_id=oaiapp_new",
            login.state
        );
        assert_eq!(
            callback(&login, &target).unwrap(),
            ("oaiapp_new".into(), "code".into())
        );
        for bad in [
            "/auth/callback?state=wrong&code=code&client_id=oaiapp_new".to_string(),
            format!("/auth/callback?state={}&error=access_denied", login.state),
            format!("/auth/callback?state={}&code=code", login.state),
            format!(
                "/auth/callback?state={}&state={}&code=code&client_id=oaiapp_new",
                login.state, login.state
            ),
        ] {
            assert!(callback(&login, &bad).is_err());
        }
        drop(login);
        save(&config, &credentials()).unwrap();
        let (login, url2) = start_login(&config).await.unwrap();
        let url2 = reqwest::Url::parse(&url2).unwrap();
        let query2: std::collections::HashMap<_, _> = url2.query_pairs().into_owned().collect();
        assert_eq!(query2["client_id"], "oaiapp_test");
        assert_eq!(query2["ext_agent_host_id"], query["ext_agent_host_id"]);
        assert!(!query2.contains_key("agent_name_hint"));
        assert_ne!(query2["state"], query["state"]);
        assert_ne!(query2["nonce"], query["nonce"]);
        assert!(
            callback(
                &login,
                &format!(
                    "/auth/callback?state={}&code=code&client_id=oaiapp_other",
                    login.state
                )
            )
            .is_err()
        );
        assert_eq!(
            callback(
                &login,
                &format!("/auth/callback?state={}&code=code", login.state)
            )
            .unwrap()
            .0,
            "oaiapp_test"
        );
        // A browser reauthorization must leave the active account usable.
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), access(&config, None))
                .await
                .unwrap()
                .unwrap(),
            "secret-access"
        );
        let address = login.listener.local_addr().unwrap();
        drop(login);
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
    }
    #[tokio::test]
    async fn credentials_are_isolated_from_other_providers_and_public_metadata() {
        let root = tempfile::tempdir().unwrap();
        let first = config(root.path());
        let other = config(root.path());
        save(&first, &credentials()).unwrap();
        assert!(load(&other).unwrap().is_none());
        assert_eq!(access(&first, None).await.unwrap(), "secret-access");
        assert!(access(&other, None).await.is_err());
        assert!(!serde_json::to_string(&first).unwrap().contains("secret"));
        assert_eq!(host_id(&first).unwrap(), host_id(&other).unwrap());
    }
    #[tokio::test]
    async fn cancellation_releases_refresh_lock() {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let held = lock(&config).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(150), lock(&config))
                .await
                .is_err()
        );
        drop(held);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), lock(&config))
                .await
                .unwrap()
                .is_ok()
        );
    }
    #[test]
    fn rotation_preserves_registration_and_rejects_missing_grant_or_changed_identity() {
        let old = credentials();
        let id = || Identity {
            sub: "subject".into(),
            email: old.email.clone(),
            nonce: None,
        };
        let tokens = json!({"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600,"token_type":"Bearer"});
        let new = updated(&tokens, Some(&old), old.client_id.clone(), id()).unwrap();
        assert_eq!(new.refresh_token, "new-refresh");
        assert_eq!(new.id_token, old.id_token);
        assert_eq!(new.scopes, old.scopes);
        assert!(updated(&tokens, None, old.client_id.clone(), id()).is_err());
        assert!(
            updated(
                &tokens,
                Some(&old),
                old.client_id.clone(),
                Identity {
                    sub: "other".into(),
                    email: old.email.clone(),
                    nonce: None
                }
            )
            .is_err()
        );
        let mut denied = tokens.clone();
        denied["scope"] = json!("openid email");
        assert!(updated(&denied, Some(&old), old.client_id.clone(), id()).is_err());
    }

    #[tokio::test]
    async fn refresh_posts_saved_client_and_atomically_persists_rotated_tokens() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let old = credentials();
        save(&config, &old).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let header_end;
            loop {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    header_end = request.len();
                    break;
                }
            }
            let header = String::from_utf8_lossy(&request);
            let length: usize = header
                .lines()
                .find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse().unwrap())
                })
                .unwrap();
            request.resize(header_end + length, 0);
            socket.read_exact(&mut request[header_end..]).await.unwrap();
            let fields: std::collections::HashMap<_, _> = reqwest::Url::parse(&format!(
                "http://localhost/?{}",
                String::from_utf8_lossy(&request[header_end..])
            ))
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
            assert_eq!(fields["client_id"], "oaiapp_test");
            assert_eq!(fields["grant_type"], "refresh_token");
            assert_eq!(fields["refresh_token"], "secret-refresh");
            assert_eq!(fields["resource"], RESOURCE);
            assert!(!fields.contains_key("scope"));
            let body=json!({"access_token":"rotated-access","refresh_token":"rotated-refresh","expires_in":3600,"token_type":"Bearer"}).to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        assert_eq!(
            refresh(&config, &old, &client, &format!("http://{address}/token"))
                .await
                .unwrap(),
            "rotated-access"
        );
        server.await.unwrap();
        let saved = load(&config).unwrap().unwrap();
        assert_eq!(saved.refresh_token, "rotated-refresh");
        assert_eq!(saved.client_id, old.client_id);
        assert_eq!(saved.subject, old.subject);
        assert_eq!(saved.scopes, old.scopes);
        // A subsequent request reuses the replacement without contacting OAuth.
        assert_eq!(access(&config, None).await.unwrap(), "rotated-access");
    }
    #[test]
    fn signed_identity_rejects_wrong_signature_issuer_audience_expiry_nonce_and_key() {
        use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
        // A generated, test-only EC key. Never used outside this unit test.
        const KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgRQ2hxwk17Ilt3/6W
yBPlOKJBZGE8ENJtSu7cuAFSgG+hRANCAAR4vO7t7JKvLk7ypN6CQ0WTGqSv0vvM
fPMsHNWkMaW4M4+W+v8OTuDOZsl8WSwujFDQROjb0gxqPqBK6ZmekneZ
-----END PRIVATE KEY-----
"#;
        let jwks: jsonwebtoken::jwk::JwkSet=serde_json::from_value(json!({"keys":[{"kty":"EC","crv":"P-256","kid":"test-key","use":"sig","alg":"ES256","x":"eLzu7eySry5O8qTegkNFkxqkr9L7zHzzLBzVpDGluDM","y":"j5b6_w5O4M5myXxZLC6MUNBE6NvSDGo-oErpmZ6Sd5k"}]})).unwrap();
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("test-key".into());
        let signing = EncodingKey::from_ec_pem(KEY.as_bytes()).unwrap();
        let claims = json!({"iss":ISSUER,"sub":"subject","aud":["oaiapp_test"],"exp":now()+3600,"iat":now(),"nonce":"nonce","email":"user@example.com"});
        let token = encode(&header, &claims, &signing).unwrap();
        assert_eq!(
            validate_identity(&token, &jwks, "oaiapp_test", Some("nonce"))
                .unwrap()
                .sub,
            "subject"
        );
        assert!(validate_identity(&token, &jwks, "other", Some("nonce")).is_err());
        assert!(validate_identity(&token, &jwks, "oaiapp_test", Some("wrong")).is_err());
        for (field, value) in [
            ("iss", json!("https://other.example")),
            ("exp", json!(now() - 60)),
            ("sub", json!("")),
        ] {
            let mut bad = claims.clone();
            bad[field] = value;
            let token = encode(&header, &bad, &signing).unwrap();
            assert!(validate_identity(&token, &jwks, "oaiapp_test", Some("nonce")).is_err());
        }
        let mut segments: Vec<_> = token.split('.').map(str::to_owned).collect();
        let mut signature = URL_SAFE_NO_PAD.decode(&segments[2]).unwrap();
        signature[0] ^= 1;
        segments[2] = URL_SAFE_NO_PAD.encode(signature);
        assert!(
            validate_identity(&segments.join("."), &jwks, "oaiapp_test", Some("nonce")).is_err()
        );
        header.kid = Some("unknown-key".into());
        let token = encode(&header, &claims, &signing).unwrap();
        assert!(validate_identity(&token, &jwks, "oaiapp_test", Some("nonce")).is_err());
    }
}
