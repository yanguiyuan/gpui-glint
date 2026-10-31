//! User consent belongs to the enrolled owner of one channel application.
use super::*;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub const REDIRECT: &str = "http://127.0.0.1:17944/oauth/wps/callback";
const SCOPES: &str = "kso.chat.readwrite";
#[derive(Clone, Serialize, Deserialize)]
struct Grant {
    access: String,
    refresh: Option<String>,
    expires: i64,
    refresh_expires: Option<i64>,
    version: Uuid,
}
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法创建 WPS 授权客户端".into())
}
fn handles() -> &'static Mutex<HashMap<Uuid, Worker>> {
    static HANDLES: OnceLock<Mutex<HashMap<Uuid, Worker>>> = OnceLock::new();
    HANDLES.get_or_init(Default::default)
}
fn events() -> &'static (
    async_channel::Sender<(Uuid, String)>,
    async_channel::Receiver<(Uuid, String)>,
) {
    static EVENTS: OnceLock<(
        async_channel::Sender<(Uuid, String)>,
        async_channel::Receiver<(Uuid, String)>,
    )> = OnceLock::new();
    EVENTS.get_or_init(async_channel::unbounded)
}
pub fn updates() -> Vec<(Uuid, String)> {
    let mut result = vec![];
    while let Ok(event) = events().1.try_recv() {
        result.push(event);
    }
    result
}
fn owner(repo: &Repository, app: Uuid) -> Result<(String, String), String> {
    repo.owner(app)?
        .map(|(c, s, _)| (c, s))
        .ok_or_else(|| "请先在机器人单聊认领主人，再进行用户授权".into())
}
pub fn status(repo: &Repository, app: Uuid) -> Result<Value, String> {
    let (company, sender) = match owner(repo, app) {
        Ok(v) => v,
        Err(_) => return Ok(json!({"authorized":false,"status":"owner_required"})),
    };
    let db = repo.open()?;
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM user_grants WHERE app=?1 AND company=?2 AND sender=?3",
            params![app.to_string(), company, sender],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(payload) = payload {
        let grant: Grant = serde_json::from_str(&payload).map_err(|_| "用户授权记录损坏")?;
        return Ok(
            json!({"authorized":grant.expires>now()||grant.refresh.as_ref().is_some_and(|_|grant.refresh_expires.is_none_or(|expiry|expiry>now())),"status":if grant.expires>now(){"authorized"}else{"refresh_required"},"expires_at":grant.expires}),
        );
    }
    let pending:Option<(String,Option<String>)>=db.query_row("SELECT status,error FROM user_authorizations WHERE app=?1 AND company=?2 AND sender=?3 AND expires>?4 ORDER BY rowid DESC LIMIT 1",params![app.to_string(),company,sender,now()],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?;
    Ok(match pending {
        Some((status, error)) => json!({"authorized":false,"status":status,"error":error}),
        None => json!({"authorized":false,"status":"not_authorized"}),
    })
}
pub fn cancel(app: Uuid) -> Result<(), String> {
    if let Some(worker) = handles().lock().map_err(|_| "授权任务繁忙")?.remove(&app) {
        drop(worker);
    }
    Ok(())
}
pub fn revoke(repo: &Repository, app: Uuid) -> Result<(), String> {
    if let Some(worker) = handles().lock().map_err(|_| "授权任务繁忙")?.remove(&app) {
        drop(worker);
    }
    let mut db = repo.open()?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM user_grants WHERE app=?1", [app.to_string()])
        .map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM user_authorizations WHERE app=?1",
        [app.to_string()],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}
fn authorize_url(app: &Application, state: &str) -> String {
    let mut url = reqwest::Url::parse("https://openapi.wps.cn/oauth2/auth").unwrap();
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &app.app_id)
        .append_pair("redirect_uri", REDIRECT)
        .append_pair("scope", SCOPES)
        .append_pair("state", state);
    url.into()
}
pub fn begin(repo: Repository, app: Application) -> Result<String, String> {
    let (company, sender) = owner(&repo, app.id)?;
    repo.secret(app.id)?;
    let mut workers = handles().lock().map_err(|_| "授权任务繁忙")?;
    if workers
        .get(&app.id)
        .is_some_and(|worker| !worker.0.is_finished())
    {
        return Err("这个应用已有待完成的授权，请先完成或取消授权后重试".into());
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", 17944))
        .map_err(|_| "用户授权回调端口 17944 被占用，请先完成或取消其他授权")?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "无法启动授权回调")?;
    let state = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let db = repo.open()?;
    db.execute(
        "DELETE FROM user_authorizations WHERE app=?1",
        [app.id.to_string()],
    )
    .map_err(|e| e.to_string())?;
    db.execute(
        "INSERT INTO user_authorizations VALUES(?1,?2,?3,?4,?5,'pending',NULL)",
        params![state, app.id.to_string(), company, sender, now() + 600],
    )
    .map_err(|e| e.to_string())?;
    let url = authorize_url(&app, &state);
    let app_id = app.id;
    let task = crate::provider::runtime()?.spawn(async move {
        let result = tokio::time::timeout(
            Duration::from_secs(600),
            serve(listener, &repo, &app, &state),
        )
        .await
        .unwrap_or_else(|_| Err("用户授权已超时，请重新发起".into()));
        let message = match result {
            Ok(()) => "用户授权成功，可以管理会话标签。".to_owned(),
            Err(error) => {
                if let Ok(db) = repo.open() {
                    let _ = db.execute(
                        "UPDATE user_authorizations SET status='failed',error=?2 WHERE state=?1",
                        params![state, error],
                    );
                }
                error
            }
        };
        let _ = events().0.send((app_id, message)).await;
    });
    workers.insert(app_id, Worker(task));
    Ok(url)
}
fn callback_code(target: &str, expected: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(&format!("http://127.0.0.1:17944{target}"))
        .map_err(|_| "授权回调无效")?;
    if url.path() != "/oauth/wps/callback" {
        return Err("授权回调路径不符".into());
    }
    let pairs: Vec<_> = url.query_pairs().collect();
    let field = |name: &str| {
        let values: Vec<_> = pairs.iter().filter(|(k, _)| k == name).collect();
        if values.len() == 1 {
            Some(values[0].1.to_string())
        } else {
            None
        }
    };
    if field("state").as_deref() != Some(expected) {
        return Err("授权 state 校验失败".into());
    }
    if pairs.iter().any(|(key, _)| key == "error") {
        return Err("用户未同意授权".into());
    }
    field("code")
        .filter(|code| !code.is_empty() && code.len() <= 4096)
        .ok_or_else(|| "授权回调缺少有效 code".into())
}
async fn reply(socket: &mut tokio::net::TcpStream, ok: bool) {
    let text = if ok {
        "授权完成。请返回 Glint 或 WPS 对话继续操作。"
    } else {
        "授权未完成。请返回 Glint 查看原因，或重新发起授权。"
    };
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{text}",
        if ok { "200 OK" } else { "400 Bad Request" },
        text.len()
    );
    let _ = socket.write_all(response.as_bytes()).await;
}
async fn serve(
    listener: std::net::TcpListener,
    repo: &Repository,
    app: &Application,
    state: &str,
) -> Result<(), String> {
    let listener = tokio::net::TcpListener::from_std(listener).map_err(|_| "无法启动授权回调")?;
    loop {
        let (mut socket, _) = listener.accept().await.map_err(|_| "授权回调监听失败")?;
        let request = tokio::time::timeout(Duration::from_secs(5), async {
            let mut bytes = vec![];
            while bytes.len() < 8192 {
                let b = socket.read_u8().await.map_err(|_| "授权回调读取失败")?;
                bytes.push(b);
                if bytes.ends_with(b"\r\n\r\n") {
                    return String::from_utf8(bytes).map_err(|_| "授权回调编码无效");
                }
            }
            Err("授权回调超出限制")
        })
        .await;
        let Ok(Ok(request)) = request else {
            reply(&mut socket, false).await;
            continue;
        };
        let line = request.lines().next().unwrap_or("");
        let mut parts = line.split_whitespace();
        let method = parts.next();
        let target = parts.next().unwrap_or("");
        let host = request
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(key, _)| key.eq_ignore_ascii_case("host"))
            .map(|(_, value)| value.trim());
        if method != Some("GET") || host != Some("127.0.0.1:17944") {
            reply(&mut socket, false).await;
            continue;
        }
        let code = match callback_code(target, state) {
            Ok(code) => code,
            Err(error) => {
                reply(&mut socket, false).await;
                if error == "用户未同意授权" {
                    return Err(error);
                }
                continue;
            }
        };
        let result = complete(repo, app, state, &code).await;
        reply(&mut socket, result.is_ok()).await;
        return result;
    }
}
fn seconds(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|v| *v > 0)
}
fn parse_grant(value: &Value, old: Option<&Grant>) -> Result<Grant, String> {
    let access = value["access_token"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or("WPS 未返回用户 token")?
        .to_owned();
    let expires = now()
        .checked_add(seconds(&value["expires_in"]).ok_or("用户 token 有效期无效")?)
        .ok_or("用户 token 有效期无效")?;
    let reported_refresh_expiry =
        seconds(&value["refresh_expires_in"]).and_then(|v| now().checked_add(v));
    let refresh_expires = match (old.and_then(|g| g.refresh_expires), reported_refresh_expiry) {
        (Some(old), Some(new)) => Some(old.min(new)),
        (Some(old), None) => Some(old),
        (None, new) => new,
    };
    Ok(Grant {
        access,
        refresh: value["refresh_token"]
            .as_str()
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .or_else(|| old.and_then(|g| g.refresh.clone())),
        expires,
        refresh_expires,
        version: Uuid::new_v4(),
    })
}
async fn exchange(
    client: &reqwest::Client,
    app: &Application,
    secret: &str,
    form: &[(&str, &str)],
    old: Option<&Grant>,
) -> Result<Grant, String> {
    let mut fields = form.to_vec();
    fields.extend([
        ("client_id", app.app_id.as_str()),
        ("client_secret", secret),
    ]);
    let response = client
        .post("https://openapi.wps.cn/oauth2/token")
        .form(&fields)
        .send()
        .await
        .map_err(|_| "WPS 用户授权 token 请求失败")?;
    if !response.status().is_success() {
        return Err(format!(
            "WPS 用户授权 token 失败（HTTP {}），请重新授权",
            response.status().as_u16()
        ));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|_| "WPS 用户 token 响应无效")?;
    parse_grant(&value, old)
}
async fn complete(
    repo: &Repository,
    app: &Application,
    state: &str,
    code: &str,
) -> Result<(), String> {
    let (company, sender) = owner(repo, app.id)?;
    let claimed=repo.open()?.execute("UPDATE user_authorizations SET status='processing' WHERE state=?1 AND app=?2 AND company=?3 AND sender=?4 AND status='pending' AND expires>?5",params![state,app.id.to_string(),company,sender,now()]).map_err(|e|e.to_string())?;
    if claimed != 1 {
        return Err("授权请求已使用、失效或主人已变更".into());
    }
    let client = client()?;
    let secret = repo.secret(app.id)?;
    let grant = exchange(
        &client,
        app,
        &secret,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT),
        ],
        None,
    )
    .await?;
    // The consent is attached to the owner who initiated this flow. Chat-only
    // scope cannot independently identify the browser account; do not request
    // unrelated profile permissions or claim a verified account identity.
    let mut db = repo.open()?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let live:i64=tx.query_row("SELECT count(*) FROM user_authorizations a JOIN owners o ON o.app=a.app AND o.company=a.company AND o.sender=a.sender WHERE a.state=?1 AND a.status='processing' AND a.expires>?2",params![state,now()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if live != 1 {
        return Err("授权已取消或主人已变更".into());
    }
    tx.execute("INSERT INTO user_grants VALUES(?1,?2,?3,?4) ON CONFLICT(app) DO UPDATE SET company=excluded.company,sender=excluded.sender,payload=excluded.payload",params![app.id.to_string(),company,sender,serde_json::to_string(&grant).map_err(|_|"无法保存用户授权")?]).map_err(|e|e.to_string())?;
    tx.execute(
        "UPDATE user_authorizations SET status='authorized' WHERE state=?1",
        [state],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}
static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
pub async fn access(repo: &Repository, app: &Application) -> Result<String, String> {
    let _lock = REFRESH_LOCK.lock().await;
    let (company, sender) = owner(repo, app.id)?;
    let payload: String = repo
        .open()?
        .query_row(
            "SELECT payload FROM user_grants WHERE app=?1 AND company=?2 AND sender=?3",
            params![app.id.to_string(), company, sender],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or("请先调用 channel_user_authorize 完成频道主人用户授权")?;
    let old: Grant = serde_json::from_str(&payload).map_err(|_| "用户授权记录损坏，请重新授权")?;
    if old.expires > now() + 60 {
        return Ok(old.access);
    }
    let refresh = old
        .refresh
        .as_deref()
        .filter(|_| old.refresh_expires.is_none_or(|expiry| expiry > now()))
        .ok_or("用户授权已到期，请重新授权")?;
    let grant = exchange(
        &client()?,
        app,
        &repo.secret(app.id)?,
        &[("grant_type", "refresh_token"), ("refresh_token", refresh)],
        Some(&old),
    )
    .await?;
    let changed=repo.open()?.execute("UPDATE user_grants SET payload=?2 WHERE app=?1 AND payload=?3 AND company=?4 AND sender=?5 AND EXISTS(SELECT 1 FROM owners WHERE owners.app=user_grants.app AND owners.company=user_grants.company AND owners.sender=user_grants.sender)",params![app.id.to_string(),serde_json::to_string(&grant).map_err(|_|"无法保存用户授权")?,payload,company,sender]).map_err(|e|e.to_string())?;
    if changed != 1 {
        return Err("用户授权已撤销或主人已变更".into());
    }
    Ok(grant.access)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authorization_url_only_requests_chat_permission() {
        let (_dir, _repo, app) = super::super::tests::fixture();
        let url = reqwest::Url::parse(&authorize_url(&app, "csrf-state")).unwrap();
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(url.host_str(), Some("openapi.wps.cn"));
        assert_eq!(query["client_id"], app.app_id);
        assert_eq!(query["redirect_uri"], REDIRECT);
        assert_eq!(query["scope"], "kso.chat.readwrite");
        assert_eq!(query["state"], "csrf-state");
        assert!(!query.contains_key("client_secret"));
    }
    #[test]
    fn callback_validates_path_state_and_unambiguous_code() {
        assert_eq!(
            callback_code(
                "/oauth/wps/callback?state=expected&code=one-time-code",
                "expected"
            )
            .unwrap(),
            "one-time-code"
        );
        for target in [
            "/oauth/wps/callback?state=wrong&code=x",
            "/other?state=expected&code=x",
            "/oauth/wps/callback?state=expected&state=expected&code=x",
            "/oauth/wps/callback?state=expected&code=x&code=y",
            "/oauth/wps/callback?state=expected",
            "/oauth/wps/callback?state=expected&error=access_denied",
        ] {
            assert!(callback_code(target, "expected").is_err(), "{target}");
        }
    }
    #[test]
    fn rotated_token_keeps_refresh_deadline() {
        let old=parse_grant(&json!({"access_token":"old-token","refresh_token":"old-refresh","expires_in":7200,"refresh_expires_in":"3600"}),None).unwrap();
        let fresh=parse_grant(&json!({"access_token":"new-token","refresh_token":"new-refresh","expires_in":7200,"refresh_expires_in":31536000}),Some(&old)).unwrap();
        assert_eq!(fresh.refresh_expires, old.refresh_expires);
        assert_eq!(fresh.refresh.as_deref(), Some("new-refresh"));
        let no_refresh = parse_grant(
            &json!({"access_token":"new-token","expires_in":7200}),
            Some(&old),
        )
        .unwrap();
        assert_eq!(no_refresh.refresh, old.refresh);
        assert!(parse_grant(&json!({"access_token":"token","expires_in":0}), None).is_err());
    }
    #[tokio::test]
    async fn owner_scoped_sqlite_grants_remain_private_and_can_be_cleared() {
        let (_dir, repo, mut app) = super::super::tests::fixture();
        let grant=parse_grant(&json!({"access_token":"private-token","refresh_token":"private-refresh","expires_in":7200,"refresh_expires_in":86400}),None).unwrap();
        repo.open()
            .unwrap()
            .execute(
                "INSERT INTO user_grants VALUES(?1,'tenant-a','user-a',?2)",
                params![app.id.to_string(), serde_json::to_string(&grant).unwrap()],
            )
            .unwrap();
        assert_eq!(access(&repo, &app).await.unwrap(), "private-token");
        let public = status(&repo, app.id).unwrap();
        assert_eq!(public["authorized"], true);
        assert!(!public.to_string().contains("private"));
        repo.open()
            .unwrap()
            .execute(
                "UPDATE owners SET sender='someone-else' WHERE app=?1",
                [app.id.to_string()],
            )
            .unwrap();
        assert!(access(&repo, &app).await.is_err());
        assert_eq!(status(&repo, app.id).unwrap()["authorized"], false);
        revoke(&repo, app.id).unwrap();
        let count: i64 = repo
            .open()
            .unwrap()
            .query_row("SELECT count(*) FROM user_grants", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        app.enabled = false;
        repo.save(&app).unwrap();
        repo.clear_owner(app.id).unwrap();
        assert_eq!(status(&repo, app.id).unwrap()["status"], "owner_required");
    }
}
