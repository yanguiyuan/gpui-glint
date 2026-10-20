//! WebSocket sessions use the HTTP client's proxy/TLS policy for the upgrade.
use super::http::{PreparedRequest, client_builder};
use async_channel::{Receiver, Sender};
use futures_util::{SinkExt, StreamExt};
use std::{sync::OnceLock, time::Duration};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        self, Message,
        protocol::{Role, WebSocketConfig},
    },
};

pub enum Command {
    Text(String),
    Close,
}
pub enum Event {
    Connected,
    Sent(String),
    Text(String),
    Binary(Vec<u8>),
    Closed(String),
    Error(String),
}
pub struct Session {
    pub commands: Sender<Command>,
    pub events: Receiver<Event>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn start(request: PreparedRequest) -> Result<Session, String> {
    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    let runtime = RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let (commands, receiver) = async_channel::bounded(16);
    let (sender, events) = async_channel::bounded(128);
    let task = runtime.spawn(async move {
        if let Err(error) = run(request, receiver, &sender).await {
            let _ = sender.send(Event::Error(error)).await;
        }
    });
    Ok(Session {
        commands,
        events,
        task,
    })
}

fn has_token(headers: &reqwest::header::HeaderMap, name: &str, token: &str) -> bool {
    headers
        .get_all(name)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|v| v.trim().eq_ignore_ascii_case(token))
}

async fn run(
    mut request: PreparedRequest,
    commands: Receiver<Command>,
    events: &Sender<Event>,
) -> Result<(), String> {
    let scheme = match request.url.scheme() {
        "ws" => "http",
        "wss" => "https",
        _ => return Err("WebSocket 地址必须使用 ws:// 或 wss://".into()),
    };
    request
        .url
        .set_scheme(scheme)
        .map_err(|_| "WebSocket 地址无效")?;
    let mut builder = client_builder(request.skip_tls_verification);
    if let Some(proxy) = request.proxy {
        builder = proxy.apply(builder)?;
    }
    let client = builder.build().map_err(|e| e.to_string())?;
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in request.headers {
        if matches!(
            name.as_str(),
            "host"
                | "connection"
                | "upgrade"
                | "content-length"
                | "transfer-encoding"
                | "sec-websocket-key"
                | "sec-websocket-version"
                | "sec-websocket-extensions"
        ) {
            return Err(format!("WebSocket 自动管理握手头：{name}"));
        }
        headers.append(name, value);
    }
    let protocols: Vec<String> = headers
        .get_all("sec-websocket-protocol")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|v| v.trim().to_owned())
        .collect();
    let key = tungstenite::handshake::client::generate_key();
    let response = client
        .get(request.url)
        .headers(headers)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", &key)
        .send()
        .await
        .map_err(|e| format!("WebSocket 握手失败：{e}"))?;
    let headers = response.headers();
    if response.status() != reqwest::StatusCode::SWITCHING_PROTOCOLS {
        return Err(format!("WebSocket 握手失败：HTTP {}", response.status()));
    }
    if !has_token(headers, "connection", "upgrade")
        || !has_token(headers, "upgrade", "websocket")
        || headers
            .get("sec-websocket-accept")
            .and_then(|v| v.to_str().ok())
            != Some(tungstenite::handshake::derive_accept_key(key.as_bytes()).as_str())
    {
        return Err("服务器返回了无效的 WebSocket 握手".into());
    }
    if headers.contains_key("sec-websocket-extensions") {
        return Err("服务器返回了未协商的 WebSocket 扩展".into());
    }
    if let Some(protocol) = headers.get("sec-websocket-protocol") {
        if !protocol
            .to_str()
            .ok()
            .is_some_and(|v| protocols.iter().any(|p| p == v))
        {
            return Err("服务器返回了未请求的 WebSocket 子协议".into());
        }
    }
    let stream = response.upgrade().await.map_err(|e| e.to_string())?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    let mut socket = WebSocketStream::from_raw_socket(stream, Role::Client, Some(config)).await;
    events
        .send(Event::Connected)
        .await
        .map_err(|e| e.to_string())?;
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Ok(Command::Text(text)) => {
                    if text.len() > 1024 * 1024 { return Err("发送消息不能超过 1 MiB".into()); }
                    tokio::time::timeout(Duration::from_secs(10), socket.send(Message::Text(text.clone().into())))
                        .await.map_err(|_| "发送消息超时")?.map_err(|e| e.to_string())?;
                    events.send(Event::Sent(text)).await.map_err(|e| e.to_string())?;
                }
                Ok(Command::Close) | Err(_) => {
                    let _ = tokio::time::timeout(Duration::from_secs(2), socket.close(None)).await;
                    let _ = events.send(Event::Closed("已断开连接".into())).await;
                    return Ok(());
                }
            },
            message = socket.next() => match message {
                Some(Ok(Message::Text(text))) => { events.send(Event::Text(text.to_string())).await.map_err(|e| e.to_string())?; }
                Some(Ok(Message::Binary(bytes))) => { events.send(Event::Binary(bytes.to_vec())).await.map_err(|e| e.to_string())?; }
                Some(Ok(Message::Ping(_))) => {
                    // tungstenite queues the matching pong when reading the ping.
                    socket.flush().await.map_err(|e| e.to_string())?;
                }
                Some(Ok(Message::Close(frame))) => {
                    let _ = tokio::time::timeout(Duration::from_secs(2), socket.flush()).await;
                    let detail = frame.map(|f| format!("{} {}", f.code, f.reason)).unwrap_or_default();
                    let _ = events.send(Event::Closed(format!("服务器已关闭连接 {detail}"))).await;
                    return Ok(());
                }
                Some(Err(error)) => return Err(error.to_string()),
                None => { let _ = events.send(Event::Closed("连接已结束".into())).await; return Ok(()); }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        environment::{EnvironmentValues, ProxySettings},
        model::{Draft, Pair},
    };

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }
    async fn next(session: &Session) -> Event {
        tokio::time::timeout(Duration::from_secs(5), session.events.recv())
            .await
            .unwrap()
            .unwrap()
    }
    fn prepare(url: String) -> PreparedRequest {
        PreparedRequest::new(
            &Draft {
                method: "WS".into(),
                url,
                ..Draft::default()
            },
            &EnvironmentValues(Default::default()),
        )
        .unwrap()
    }
    #[test]
    fn echo_text_binary_ping_and_server_close() {
        runtime().block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_hdr_async(
                    stream,
                    |request: &tungstenite::handshake::server::Request, response| {
                        assert_eq!(
                            request.uri().path_and_query().unwrap().as_str(),
                            "/echo?q=hello%20world"
                        );
                        assert_eq!(request.headers()["authorization"], "Bearer secret");
                        assert_eq!(request.headers()["cookie"], "session=abc");
                        Ok(response)
                    },
                )
                .await
                .unwrap();
                socket.send(Message::Ping(vec![1, 2].into())).await.unwrap();
                let mut pong = false;
                let mut text = None;
                while !pong || text.is_none() {
                    match socket.next().await.unwrap().unwrap() {
                        Message::Pong(_) => pong = true,
                        message @ Message::Text(_) => text = Some(message),
                        _ => panic!("unexpected client message"),
                    }
                }
                let message = text.unwrap();
                socket.send(message).await.unwrap();
                socket
                    .send(Message::Binary(vec![0, 255, 1].into()))
                    .await
                    .unwrap();
                socket.close(None).await.unwrap();
                let _ = socket.next().await;
            });
            let draft = Draft {
                method: "WS".into(),
                url: format!("ws://{address}/echo"),
                params: vec![Pair::new("q", "hello world", "")],
                auth: true,
                token: "secret".into(),
                cookies: vec![Pair::new("session", "abc", "")],
                ..Draft::default()
            };
            let session = start(
                PreparedRequest::new(&draft, &EnvironmentValues(Default::default())).unwrap(),
            )
            .unwrap();
            assert!(matches!(next(&session).await, Event::Connected));
            session
                .commands
                .send(Command::Text("你好 🌏".into()))
                .await
                .unwrap();
            assert!(matches!(next(&session).await, Event::Sent(text) if text == "你好 🌏"));
            assert!(matches!(next(&session).await, Event::Text(text) if text == "你好 🌏"));
            assert!(matches!(next(&session).await, Event::Binary(bytes) if bytes == [0, 255, 1]));
            assert!(matches!(next(&session).await, Event::Closed(_)));
            server.await.unwrap();
        });
    }
    #[test]
    #[ignore = "uses real 31-second heartbeat intervals to check an idle WSS connection"]
    fn idle_wss_replies_to_periodic_pings_past_ninety_seconds() {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
        let interval = std::env::var("GLINT_TEST_WS_HEARTBEAT_MS")
            .map(|value| Duration::from_millis(value.parse().unwrap()))
            .unwrap_or(Duration::from_secs(31));
        let config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(
                include_bytes!("tests/fixtures/self-signed-cert.der").to_vec(),
            )],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                include_bytes!("tests/fixtures/test-only-key.der").to_vec(),
            )),
        )
        .unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
            let tls = rustls::StreamOwned::new(
                rustls::ServerConnection::new(std::sync::Arc::new(config)).unwrap(),
                tcp,
            );
            let mut socket = tungstenite::accept(tls).unwrap();
            for sequence in 1..=3 {
                // No application messages or user actions between server heartbeats.
                std::thread::sleep(interval);
                let payload = vec![sequence, 0, 255];
                socket.send(Message::Ping(payload.clone().into())).unwrap();
                assert_eq!(socket.read().unwrap(), Message::Pong(payload.into()));
                eprintln!(
                    "received matching Pong after {} ms",
                    u128::from(sequence) * interval.as_millis()
                );
            }
            socket
                .send(Message::Text("still connected".into()))
                .unwrap();
            socket.close(None).unwrap();
            let _ = socket.read();
        });
        let mut request = prepare(format!("wss://localhost:{}/", address.port()));
        request.skip_tls_verification = true;
        // Optional transport comparison, using only the local test server.
        if let Ok(url) = std::env::var("GLINT_TEST_WS_PROXY_URL") {
            request.proxy = Some(ProxySettings {
                enabled: true,
                url,
                ..Default::default()
            });
        }
        let session = start(request).unwrap();
        runtime().block_on(async {
            assert!(matches!(next(&session).await, Event::Connected));
            let event = tokio::time::timeout(Duration::from_secs(105), session.events.recv())
                .await
                .unwrap()
                .unwrap();
            match event {
                Event::Text(text) => assert_eq!(text, "still connected"),
                Event::Error(error) => panic!("idle WSS failed: {error}"),
                Event::Closed(reason) => panic!("idle WSS closed early: {reason}"),
                _ => panic!("unexpected idle WSS event"),
            }
            assert!(matches!(next(&session).await, Event::Closed(_)));
        });
        server.join().unwrap();
    }

    #[test]
    fn built_in_proxy_maps_websocket_host_and_keeps_upgrade_hostname() {
        runtime().block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_hdr_async(
                    stream,
                    move |request: &tungstenite::handshake::server::Request, response| {
                        assert_eq!(
                            request.headers()["host"],
                            format!("mapped.invalid:{}", address.port())
                        );
                        Ok(response)
                    },
                )
                .await
                .unwrap();
                assert!(matches!(
                    socket.next().await.unwrap().unwrap(),
                    Message::Close(_)
                ));
            });
            let proxy = ProxySettings {
                enabled: true,
                built_in: true,
                mappings: vec![super::super::environment::HostMapping {
                    domain: "mapped.invalid".into(),
                    ip: "127.0.0.1".into(),
                }],
                ..Default::default()
            };
            let request = prepare(format!("ws://mapped.invalid:{}/echo", address.port()))
                .with_proxy(&proxy)
                .unwrap();
            let session = start(request).unwrap();
            assert!(matches!(next(&session).await, Event::Connected));
            session.commands.send(Command::Close).await.unwrap();
            assert!(matches!(next(&session).await, Event::Closed(_)));
            server.await.unwrap();
        });
    }
    #[test]
    fn explicit_http_proxy_and_client_close() {
        runtime().block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                assert!(matches!(
                    socket.next().await.unwrap().unwrap(),
                    Message::Close(_)
                ));
            });
            let mut request = prepare("ws://unresolvable.invalid/echo".into());
            request.proxy = Some(ProxySettings {
                enabled: true,
                url: format!("http://{address}"),
                ..ProxySettings::default()
            });
            let session = start(request).unwrap();
            assert!(matches!(next(&session).await, Event::Connected));
            session.commands.send(Command::Close).await.unwrap();
            assert!(matches!(next(&session).await, Event::Closed(_)));
            server.await.unwrap();
        });
    }
    #[test]
    fn invalid_accept_and_http_rejection_are_errors() {
        use std::io::{Read, Write};
        for response in [
            "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: invalid\r\n\r\n",
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n",
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut buffer = [0; 8192];
                socket.read(&mut buffer).unwrap();
                socket.write_all(response.as_bytes()).unwrap();
            });
            let session = start(prepare(format!("ws://{address}/"))).unwrap();
            runtime().block_on(async {
                assert!(matches!(next(&session).await, Event::Error(_)));
            });
            server.join().unwrap();
        }
    }
    #[test]
    fn dropping_session_aborts_the_live_connection() {
        runtime().block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let _ = socket.next().await;
            });
            let session = start(prepare(format!("ws://{address}/"))).unwrap();
            assert!(matches!(next(&session).await, Event::Connected));
            drop(session);
            tokio::time::timeout(Duration::from_secs(5), server)
                .await
                .unwrap()
                .unwrap();
        });
    }
    #[test]
    fn wss_verifies_certificates_and_supports_request_scoped_skip() {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
        for skip in [false, true] {
            let config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(
                    include_bytes!("tests/fixtures/self-signed-cert.der").to_vec(),
                )],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                    include_bytes!("tests/fixtures/test-only-key.der").to_vec(),
                )),
            )
            .unwrap();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (tcp, _) = listener.accept().unwrap();
                tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
                let tls = rustls::StreamOwned::new(
                    rustls::ServerConnection::new(std::sync::Arc::new(config)).unwrap(),
                    tcp,
                );
                let socket = tungstenite::accept(tls);
                if skip {
                    let mut socket = socket.unwrap();
                    socket.send(Message::Text("secure echo".into())).unwrap();
                    let _ = socket.read();
                } else {
                    assert!(socket.is_err());
                }
            });
            let mut request = prepare(format!("wss://localhost:{}/", address.port()));
            request.skip_tls_verification = skip;
            let session = start(request).unwrap();
            runtime().block_on(async {
                if skip {
                    assert!(matches!(next(&session).await, Event::Connected));
                    assert!(
                        matches!(next(&session).await, Event::Text(text) if text == "secure echo")
                    );
                    session.commands.send(Command::Close).await.unwrap();
                    assert!(matches!(next(&session).await, Event::Closed(_)));
                } else {
                    assert!(matches!(next(&session).await, Event::Error(_)));
                }
            });
            server.join().unwrap();
        }
    }
}
