//! WPS 365 protocol, matching the documented open-event SDK and glint-api plugins.
use super::*;
use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Message,
        handshake::{client::generate_key, derive_accept_key},
        protocol::{Role, WebSocketConfig},
    },
};

type HmacSha256 = Hmac<Sha256>;
fn signature(
    app: &str,
    key: &str,
    method: &str,
    path: &str,
    content_type: &str,
    date: &str,
    body: &str,
) -> String {
    let hash = if body.is_empty() {
        String::new()
    } else {
        format!("{:x}", Sha256::digest(body.as_bytes()))
    };
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(key.as_bytes()).expect("HMAC accepts any key");
    mac.update(format!("KSO-1{method}{path}{content_type}{date}{hash}").as_bytes());
    let hex = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    format!("KSO-1 {app}:{hex}")
}
fn decrypt(app: &str, key: &str, event: &Value) -> Result<Value, String> {
    let field = |name: &str| {
        event[name]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "WPS 事件字段不完整".to_owned())
    };
    let topic = field("topic")?;
    let nonce = field("nonce")?;
    let data = field("encrypted_data")?;
    let time = event["time"].as_i64().ok_or("WPS 事件时间无效")?;
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(key.as_bytes()).map_err(|_| "WPS 密钥无效")?;
    mac.update(format!("{app}:{topic}:{nonce}:{time}:{data}").as_bytes());
    let signature = URL_SAFE_NO_PAD
        .decode(field("signature")?)
        .map_err(|_| "WPS 事件签名格式无效")?;
    mac.verify_slice(&signature)
        .map_err(|_| "WPS 事件验签失败")?;
    let aes_key = format!("{:x}", md5::Md5::digest(key.as_bytes()));
    let iv = nonce.as_bytes().get(..16).ok_or("WPS 事件 nonce 无效")?;
    let mut ciphertext = STANDARD.decode(data).map_err(|_| "WPS 密文无效")?;
    let cipher = cbc::Decryptor::<aes::Aes256>::new_from_slices(aes_key.as_bytes(), iv)
        .map_err(|_| "WPS 密钥无效")?;
    let bytes = cipher
        .decrypt_padded_mut::<Pkcs7>(&mut ciphertext)
        .map_err(|_| "WPS 事件解密失败")?;
    serde_json::from_slice(bytes).map_err(|_| "WPS 事件内容不是 JSON".into())
}
fn incoming(app: Uuid, event: &Value, data: &Value) -> Result<Option<Incoming>, String> {
    if format!(
        "{}.{}",
        event["topic"].as_str().unwrap_or(""),
        event["operation"].as_str().unwrap_or("")
    ) != "kso.app_chat.message.create"
    {
        return Ok(None);
    }
    // Only human messages addressed to this app; never echo robot/service messages.
    if data["sender"]["type"] != "user" {
        return Ok(None);
    }
    let string = |v: &Value| {
        v.as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| "WPS 消息缺少会话或消息 ID".to_owned())
    };
    let text = if data["message"]["type"] == "text" {
        data["message"]["content"]["text"]["content"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    } else {
        // Keep unsupported attachments visible in history without inventing their contents.
        format!(
            "[WPS {} 消息：当前频道仅接入文本内容，请发送文字说明]",
            data["message"]["type"].as_str().unwrap_or("未知类型")
        )
    };
    if text.trim().trim_end_matches(['。', '!', '！', '.']) == "我是你的主人"
        && !matches!(data["chat"]["type"].as_str(), Some("single" | "p2p"))
    {
        return Ok(None);
    }
    if text.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(Incoming {
        app,
        company: string(&data["company_id"])?,
        chat: string(&data["chat"]["id"])?,
        message: string(&data["message"]["id"])?,
        sender: string(&data["sender"]["id"])?,
        text,
    }))
}
struct Token {
    value: String,
    until: Instant,
}
async fn token(client: &reqwest::Client, app: &Application, key: &str) -> Result<Token, String> {
    let response = client
        .post("https://openapi.wps.cn/oauth2/token")
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", app.app_id.as_str()),
            ("client_secret", key),
        ])
        .send()
        .await
        .map_err(|_| "WPS 获取凭证网络失败")?;
    let status = response.status();
    let value: Value = response.json().await.map_err(|_| "WPS 凭证响应无效")?;
    if !status.is_success() {
        return Err(format!("WPS 获取凭证失败（HTTP {}）", status.as_u16()));
    }
    let value_token = value["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("WPS 未返回应用凭证，请检查自建应用授权")?
        .to_owned();
    Ok(Token {
        value: value_token,
        until: Instant::now()
            + Duration::from_secs(
                value["expires_in"]
                    .as_u64()
                    .unwrap_or(7200)
                    .saturating_sub(120),
            ),
    })
}
fn message_body(chat: &str, text: &str) -> Value {
    json!({"type":"text","content":{"text":{"type":"markdown","content":text}},"receiver":{"type":"chat","receiver_id":chat}})
}
async fn send(
    client: &reqwest::Client,
    app: &Application,
    key: &str,
    access: &str,
    chat: &str,
    text: &str,
) -> Result<(), String> {
    let body = message_body(chat, text).to_string();
    let date = httpdate::fmt_http_date(std::time::SystemTime::now());
    let response = client
        .post("https://openapi.wps.cn/v7/messages/create")
        .bearer_auth(access)
        .header("Content-Type", "application/json")
        .header("X-Kso-Date", &date)
        .header(
            "X-Kso-Authorization",
            signature(
                &app.app_id,
                key,
                "POST",
                "/v7/messages/create",
                "application/json",
                &date,
                &body,
            ),
        )
        .body(body)
        .send()
        .await
        .map_err(|_| "WPS 发送失败，送达状态未知，请核对会话")?;
    let status = response.status();
    let result: Value = response
        .json()
        .await
        .map_err(|_| "WPS 发送响应无效，送达状态未知")?;
    if !status.is_success() || result["code"].as_i64() != Some(0) {
        return Err(format!(
            "WPS 发送失败（HTTP {}，code {}），请检查消息权限及机器人可见范围",
            status.as_u16(),
            result["code"]
        ));
    }
    Ok(())
}
fn reaction_type(stage: &str) -> &'static str {
    match stage {
        "received" => "emoji_receive",
        "done" => "emoji_done",
        _ => "emoji_error",
    }
}
fn reaction_url(chat: &str, message: &str, action: &str) -> reqwest::Url {
    let mut url = reqwest::Url::parse("https://openapi.wps.cn/").unwrap();
    url.path_segments_mut().unwrap().extend([
        "v7",
        "chats",
        chat,
        "messages",
        message,
        "reactions",
        action,
    ]);
    url
}
async fn react(
    client: &reqwest::Client,
    app: &Application,
    key: &str,
    access: &str,
    chat: &str,
    message: &str,
    stage: &str,
    action: &str,
) -> Result<(), String> {
    let url = reaction_url(chat, message, action);
    let body = json!({"reaction_type":reaction_type(stage)}).to_string();
    let date = httpdate::fmt_http_date(std::time::SystemTime::now());
    let signature = signature(
        &app.app_id,
        key,
        "POST",
        url.path(),
        "application/json",
        &date,
        &body,
    );
    let response = client
        .post(url)
        .bearer_auth(access)
        .header("Content-Type", "application/json")
        .header("X-Kso-Date", date)
        .header("X-Kso-Authorization", signature)
        .body(body)
        .send()
        .await
        .map_err(|_| "WPS 表情回复网络失败")?;
    let status = response.status();
    let value: Value = response.json().await.map_err(|_| "WPS 表情回复响应无效")?;
    if !status.is_success() || value["code"] != 0 {
        return Err(format!(
            "WPS 表情回复失败（HTTP {}，code {}）",
            status.as_u16(),
            value["code"]
        ));
    }
    Ok(())
}
/// Use the same HTTP upgrade transport as glint-api. Reqwest selects its TLS
/// provider explicitly and respects configured system HTTP(S) proxies.
async fn connect(
    client: &reqwest::Client,
    url: &str,
    app: &str,
    key: &str,
) -> Result<WebSocketStream<reqwest::Upgraded>, String> {
    let date = httpdate::fmt_http_date(std::time::SystemTime::now());
    let websocket_key = generate_key();
    let response = client
        .get(url)
        .header("X-Kso-Date", &date)
        .header(
            "X-Kso-Authorization",
            signature(app, key, "GET", "/v7/event/ws", "", &date, ""),
        )
        .header("X-Ack-Mode", "required")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", &websocket_key)
        .send()
        .await
        .map_err(|e| format!("WPS 连接失败：{}", e.without_url()))?;
    if response.status() != reqwest::StatusCode::SWITCHING_PROTOCOLS {
        return Err(format!(
            "WPS 握手失败：HTTP {}，请检查应用凭证和事件订阅配置",
            response.status().as_u16()
        ));
    }
    let headers = response.headers();
    let has_token = |header: &str, token: &str| {
        headers
            .get_all(header)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .any(|v| v.trim().eq_ignore_ascii_case(token))
    };
    if !has_token("connection", "upgrade")
        || !has_token("upgrade", "websocket")
        || headers
            .get("sec-websocket-accept")
            .and_then(|v| v.to_str().ok())
            != Some(derive_accept_key(websocket_key.as_bytes()).as_str())
        || headers.contains_key("sec-websocket-extensions")
        || headers.contains_key("sec-websocket-protocol")
    {
        return Err("WPS 返回了无效的 WebSocket 握手".into());
    }
    let stream = response.upgrade().await.map_err(|_| "WPS 升级长连接失败")?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    Ok(WebSocketStream::from_raw_socket(stream, Role::Client, Some(config)).await)
}
pub(super) async fn listen(
    app: &Application,
    key: &str,
    repo: &Repository,
    status: &async_channel::Sender<(Uuid, String)>,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "无法创建 WPS 客户端")?;
    let mut socket = tokio::time::timeout(
        Duration::from_secs(20),
        connect(
            &client,
            "https://openapi.wps.cn/v7/event/ws",
            &app.app_id,
            key,
        ),
    )
    .await
    .map_err(|_| "WPS 连接超时，请检查网络或代理")??;
    let _ = status.send((app.id, "已连接 · 监听消息".into())).await;
    let mut access: Option<Token> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut last = Instant::now();
    loop {
        tokio::select! {
            frame=socket.next()=>{
                let frame=frame.ok_or("WPS 连接关闭")?.map_err(|_|"WPS 连接中断")?;last=Instant::now();
                match frame {
                    Message::Ping(bytes)=>{socket.send(Message::Pong(bytes)).await.map_err(|_|"WPS 心跳失败")?;}
                    Message::Close(_)=>return Err("WPS 连接关闭".into()),
                    Message::Text(text)=>{
                        let event:Value=serde_json::from_str(&text).map_err(|_|"WPS 事件不是 JSON")?;
                        if event["type"]=="goaway" {return Err(if event["reason"]=="connection_replaced" {"连接被另一客户端替换，请断开其他监听器后重新启用"}else{"WPS 服务要求重连"}.into());}
                        let data=decrypt(&app.app_id,key,&event)?;
                        let result=incoming(app.id,&event,&data).and_then(|m|{if let Some(m)=m{repo.receive(app,&m)?;}Ok(())});
                        let ack=json!({"type":"ack","nonce":event["nonce"],"code":if result.is_ok(){200}else{500}});
                        socket.send(Message::Text(ack.to_string().into())).await.map_err(|_|"WPS ACK 失败")?;
                        result?;
                        // Refresh enrollment and inbox state even if the model is busy.
                        let _=status.send((app.id,"已连接 · 监听消息".into())).await;
                    }
                    _=>{}
                }
            }
            _=tick.tick()=>{
                if last.elapsed()>Duration::from_secs(100){return Err("WPS 心跳超时".into());}
                // Receipt reactions have priority over reply delivery. This is
                // independent of inference; failures must not block the reply.
                if let Some((row,chat,message,stage))=repo.reaction(app.id)? {
                    let result=async {
                        if access.as_ref().is_none_or(|t|t.until<=Instant::now()){access=Some(token(&client,app,key).await?);}
                        let access=&access.as_ref().unwrap().value;
                        react(&client,app,key,access,&chat,&message,&stage,"create").await?;
                        for old in repo.applied_reactions(row)?.into_iter().filter(|old|old!=&stage){
                            react(&client,app,key,access,&chat,&message,&old,"delete").await?;
                            repo.removed_reaction(row,&old)?;
                        }
                        Ok::<_,String>(())
                    }.await;
                    if let Err(error)=&result{access=None;let _=status.send((app.id,error.clone())).await;}
                    repo.reacted(row,&stage,result)?;
                    continue;
                }
                if let Some((id,chat,text))=repo.outgoing(app.id)? {
                    let result=async {
                        if access.as_ref().is_none_or(|t|t.until<=Instant::now()){access=Some(token(&client,app,key).await?);}
                        send(&client,app,key,&access.as_ref().unwrap().value,&chat,&text).await
                    }.await;
                    if let Err(error)=&result {access=None;let _=status.send((app.id,error.clone())).await;}
                    repo.sent(id,result)?;
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn upgrade_reports_http_rejection_instead_of_leaving_connecting() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v7/event/ws", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            socket.read(&mut bytes).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = connect(&client, &url, "fixture-app", "fixture-key").await;
        assert!(result.unwrap_err().contains("HTTP 401"));
        server.await.unwrap();
    }
    #[tokio::test]
    async fn upgrade_authentication_and_websocket_frames_work_through_http_client() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v7/event/ws", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let byte = socket.read_u8().await.unwrap();
                bytes.push(byte);
                if bytes.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let text = String::from_utf8(bytes).unwrap();
            let header = |name: &str| {
                text.lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case(name)
                            .then(|| value.trim().to_owned())
                    })
                    .unwrap()
            };
            assert_eq!(header("x-ack-mode"), "required");
            assert_eq!(
                header("x-kso-authorization"),
                signature(
                    "fixture-app",
                    "fixture-key",
                    "GET",
                    "/v7/event/ws",
                    "",
                    &header("x-kso-date"),
                    ""
                )
            );
            let accept = derive_accept_key(header("sec-websocket-key").as_bytes());
            socket.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").as_bytes()).await.unwrap();
            let mut ws = WebSocketStream::from_raw_socket(socket, Role::Server, None).await;
            ws.send(Message::Text("test-event".into())).await.unwrap();
            assert_eq!(
                ws.next().await.unwrap().unwrap().into_text().unwrap(),
                "test-ack"
            );
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut ws = connect(&client, &url, "fixture-app", "fixture-key")
            .await
            .unwrap();
        assert_eq!(
            ws.next().await.unwrap().unwrap().into_text().unwrap(),
            "test-event"
        );
        ws.send(Message::Text("test-ack".into())).await.unwrap();
        server.await.unwrap();
    }
    #[test]
    fn reactions_use_documented_codes_and_encode_message_path_segments() {
        assert_eq!(reaction_type("received"), "emoji_receive");
        assert_eq!(reaction_type("done"), "emoji_done");
        assert_eq!(reaction_type("failed"), "emoji_error");
        assert_eq!(
            reaction_url("chat/id", "msg?x", "create").as_str(),
            "https://openapi.wps.cn/v7/chats/chat%2Fid/messages/msg%3Fx/reactions/create"
        );
        assert_eq!(
            reaction_url("chat", "message", "delete").path(),
            "/v7/chats/chat/messages/message/reactions/delete"
        );
    }
    #[test]
    fn signing_matches_independent_hmac_vectors() {
        let date = "Mon, 02 Jan 2006 15:04:05 GMT";
        assert_eq!(
            signature(
                "test-app",
                "test-secret",
                "GET",
                "/v7/event/ws",
                "",
                date,
                ""
            ),
            "KSO-1 test-app:ca2b6a1be3d537c380e8e6ef827a7a8aaf8d025bd3b2a86259a153677723adbb"
        );
        assert_eq!(
            signature(
                "test-app",
                "test-secret",
                "POST",
                "/v7/messages/create",
                "application/json",
                date,
                r#"{"test":"body"}"#
            ),
            "KSO-1 test-app:ef06116e103f8bcca39537a719542a09a1bb1da8de6ed4b12bf656dba6db5cda"
        );
    }
    #[test]
    fn verifies_shared_glint_api_fixture_and_rejects_tampering() {
        let f: Value = serde_json::from_str(include_str!(
            "../../../glint-api/plugins/wps-event-decrypt/tests/fixture.json"
        ))
        .unwrap();
        assert_eq!(
            decrypt(
                f["app_id"].as_str().unwrap(),
                f["app_key"].as_str().unwrap(),
                &f["event"]
            )
            .unwrap(),
            f["data"]
        );
        let mut bad = f["event"].clone();
        bad["topic"] = "other".into();
        assert!(decrypt("fixture-app", "fixture-secret-only", &bad).is_err());
        for event in f["bad_cases"].as_array().unwrap() {
            assert!(decrypt("fixture-app", "fixture-secret-only", event).is_err());
        }
    }
    #[test]
    fn owner_claim_accepts_real_wps_p2p_events_and_preserves_group_filter() {
        let event = json!({"topic":"kso.app_chat.message","operation":"create"});
        let mut data = json!({"chat":{"id":"direct-chat","type":"p2p"},"company_id":"tenant","sender":{"id":"human-owner","type":"user"},"message":{"id":"claim-message","type":"text","content":{"text":{"content":"我是你的主人"}}}});
        let (dir, repo, mut app) = super::super::tests::fixture();
        app.enabled = false;
        repo.save(&app).unwrap();
        repo.clear_owner(app.id).unwrap();
        app.enabled = true;
        repo.save(&app).unwrap();
        let claim = incoming(app.id, &event, &data)
            .unwrap()
            .expect("WPS p2p is a direct conversation");
        repo.receive(&app, &claim).unwrap();
        assert_eq!(
            Repository::new(&dir.path().join("sessions.json"))
                .owner(app.id)
                .unwrap()
                .unwrap(),
            ("tenant".into(), "human-owner".into(), "direct-chat".into())
        );
        data["chat"]["type"] = "single".into();
        assert!(incoming(app.id, &event, &data).unwrap().is_some());
        for kind in ["group", "unknown", ""] {
            data["chat"]["type"] = kind.into();
            assert!(incoming(app.id, &event, &data).unwrap().is_none());
        }
        data["chat"]["type"] = "p2p".into();
        data["sender"]["type"] = "sp".into();
        assert!(incoming(app.id, &event, &data).unwrap().is_none());
    }
    #[test]
    fn routes_only_human_message_events_and_uses_documented_send_body() {
        let event = json!({"topic":"kso.app_chat.message","operation":"create"});
        let mut data = json!({"company_id":"tenant","chat":{"id":"chat"},"sender":{"id":"user","type":"user"},"message":{"id":"msg","type":"text","content":{"text":{"content":"你好"}}}});
        let app = Uuid::new_v4();
        let m = incoming(app, &event, &data).unwrap().unwrap();
        assert_eq!(m.text, "你好");
        assert_eq!(m.chat, "chat");
        data["sender"]["type"] = "sp".into();
        assert!(incoming(app, &event, &data).unwrap().is_none());
        assert!(
            incoming(
                app,
                &json!({"topic":"kso.app_chat","operation":"create"}),
                &data
            )
            .unwrap()
            .is_none()
        );
        assert_eq!(
            message_body("chat", "hello"),
            json!({"type":"text","content":{"text":{"type":"markdown","content":"hello"}},"receiver":{"receiver_id":"chat","type":"chat"}})
        );
    }
}
