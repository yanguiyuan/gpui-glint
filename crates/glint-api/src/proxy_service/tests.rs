use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn test_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
fn settings() -> ProxySettings {
    ProxySettings {
        enabled: true,
        built_in: true,
        listener: super::super::environment::ProxyListenerSettings {
            port: 0,
            allow_lan: false,
        },
        ..Default::default()
    }
}
fn client(proxy: &RunningProxy) -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(proxy.endpoint()).unwrap())
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}
async fn wait_complete(store: &CaptureStore) -> CaptureRecord {
    for _ in 0..100 {
        if let Some(summary) = store.summaries().first() {
            let record = store.record(summary.id).unwrap();
            if matches!(record.state.as_str(), "完成" | "失败" | "已关闭") {
                return record;
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("capture did not complete");
}

#[test]
fn http_port_forwards_binary_and_headers_and_captures_full_exchange() {
    test_runtime().block_on(async {
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = origin.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (socket, _) = origin.accept().await.unwrap();
            http1::Builder::new()
                .serve_connection(
                    TokioIo::new(socket),
                    service_fn(|request: Request<Incoming>| async move {
                        assert_eq!(request.uri().to_string(), "/upload?q=%E4%B8%AD");
                        assert_eq!(request.headers()["host"], format!("mapped.test:{port}"));
                        assert_eq!(request.headers()["authorization"], "Bearer example");
                        assert_eq!(request.headers()["cookie"], "session=example");
                        assert!(!request.headers().contains_key("proxy-authorization"));
                        assert!(!request.headers().contains_key("x-hop"));
                        let bytes = request.into_body().collect().await.unwrap().to_bytes();
                        assert_eq!(bytes.as_ref(), b"\x00\x01binary\xff");
                        Ok::<_, Infallible>(
                            Response::builder()
                                .status(201)
                                .header("x-result", "ok")
                                .header("set-cookie", "a=1")
                                .header("set-cookie", "b=2")
                                .body(Full::new(bytes))
                                .unwrap(),
                        )
                    }),
                )
                .await
                .unwrap();
        });
        let mut config = settings();
        config
            .mappings
            .push(super::super::environment::HostMapping {
                domain: "mapped.test".into(),
                ip: "127.0.0.1".into(),
            });
        let store = Arc::new(CaptureStore::default());
        let proxy = RunningProxy::start(config, None, store.clone()).unwrap();
        assert!(proxy.address.ip().is_loopback());
        let response = client(&proxy)
            .post(format!("http://mapped.test:{port}/upload?q=%E4%B8%AD"))
            .header("authorization", "Bearer example")
            .header("cookie", "session=example")
            .header("proxy-authorization", "Basic ignored")
            .header("connection", "x-hop")
            .header("x-hop", "remove")
            .body(b"\x00\x01binary\xff".to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 201);
        assert_eq!(response.headers().get_all("set-cookie").iter().count(), 2);
        assert_eq!(
            response.bytes().await.unwrap().as_ref(),
            b"\x00\x01binary\xff"
        );
        let record = wait_complete(&store).await;
        assert_eq!(record.status, Some(201));
        assert_eq!(record.request.preview, b"\x00\x01binary\xff");
        assert_eq!(record.response.preview, b"\x00\x01binary\xff");
        drop(proxy);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    });
}

#[test]
fn https_requires_client_trust_and_decrypts_with_domain_mapping() {
    test_runtime().block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let ca = Arc::new(ca::Authority::generate(directory.path()).unwrap());
        assert_eq!(
            ca.fingerprint,
            ca::Authority::load(directory.path()).unwrap().fingerprint
        );
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = origin.local_addr().unwrap().port();
        let tls = tokio_rustls::TlsAcceptor::from(ca.tls_for("secure.test").unwrap());
        let server = tokio::spawn(async move {
            let (socket, _) = origin.accept().await.unwrap();
            let socket = tls.accept(socket).await.unwrap();
            http1::Builder::new()
                .serve_connection(
                    TokioIo::new(socket),
                    service_fn(|request: Request<Incoming>| async move {
                        assert_eq!(request.uri().to_string(), "/secret?value=1");
                        assert_eq!(request.headers()["x-client"], "browser");
                        Ok::<_, Infallible>(Response::new(Full::new(Bytes::from_static(
                            b"decrypted response",
                        ))))
                    }),
                )
                .await
                .unwrap();
        });
        let mut config = settings();
        config.skip_tls_verification = true;
        config
            .mappings
            .push(super::super::environment::HostMapping {
                domain: "secure.test".into(),
                ip: "127.0.0.1".into(),
            });
        let store = Arc::new(CaptureStore::default());
        let proxy = RunningProxy::start(config, Some(ca.clone()), store.clone()).unwrap();
        let url = format!("https://secure.test:{port}/secret?value=1");
        assert!(
            client(&proxy).get(&url).send().await.is_err(),
            "untrusted CA must be rejected"
        );
        let trusted = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(proxy.endpoint()).unwrap())
            .add_root_certificate(reqwest::Certificate::from_der(&ca.certificate).unwrap())
            .build()
            .unwrap();
        assert_eq!(
            trusted
                .get(&url)
                .header("x-client", "browser")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "decrypted response"
        );
        let record = wait_complete(&store).await;
        assert_eq!(record.method, "GET");
        assert_eq!(record.url, url);
        assert_eq!(record.response.preview, b"decrypted response");
        drop(proxy);
        drop(trusted);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    });
}

#[test]
fn upstream_proxy_redirects_and_self_loop_are_handled_without_following() {
    test_runtime().block_on(async {
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = upstream.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (socket, _) = upstream.accept().await.unwrap();
            http1::Builder::new()
                .serve_connection(
                    TokioIo::new(socket),
                    service_fn(|request: Request<Incoming>| async move {
                        assert_eq!(
                            request.uri().to_string(),
                            "http://does-not-resolve.invalid/path"
                        );
                        Ok::<_, Infallible>(
                            Response::builder()
                                .status(302)
                                .header("location", "http://elsewhere.invalid/")
                                .body(Full::new(Bytes::from_static(b"redirect from upstream")))
                                .unwrap(),
                        )
                    }),
                )
                .await
                .unwrap();
        });
        let mut config = settings();
        config.built_in = false;
        config.url = format!("http://127.0.0.1:{port}");
        let proxy = RunningProxy::start(config, None, Arc::default()).unwrap();
        let response = client(&proxy)
            .get("http://does-not-resolve.invalid/path")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 302);
        assert_eq!(response.text().await.unwrap(), "redirect from upstream");
        assert_eq!(
            client(&proxy)
                .get(proxy.endpoint())
                .send()
                .await
                .unwrap()
                .status(),
            508
        );
        drop(proxy);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    });
}

#[test]
fn streaming_response_arrives_before_eof_and_stopping_closes_open_connections() {
    test_runtime().block_on(async {
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = origin.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = origin.accept().await.unwrap();
            let mut request = vec![0u8; 4096]; socket.read(&mut request).await.unwrap();
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n9\r\ndata: 1\n\n\r\n").await.unwrap();
            let mut tail = [0u8; 1]; let _ = socket.read(&mut tail).await;
        });
        let store = Arc::new(CaptureStore::default());
        let proxy = RunningProxy::start(settings(), None, store.clone()).unwrap();
        let client = client(&proxy);
        let mut response = client.get(format!("http://127.0.0.1:{port}/sse")).send().await.unwrap();
        let chunk = tokio::time::timeout(Duration::from_secs(2), response.chunk()).await.unwrap().unwrap().unwrap();
        assert_eq!(chunk.as_ref(), b"data: 1\n\n");
        let id = store.summaries()[0].id;
        assert_eq!(store.record(id).unwrap().response.bytes, 9);
        let address = proxy.address;
        drop(proxy);
        let _ = tokio::time::timeout(Duration::from_secs(2), response.chunk()).await.unwrap();
        for _ in 0..100 {
            if tokio::net::TcpStream::connect(address).await.is_err() { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
        tokio::time::timeout(Duration::from_secs(2), server).await.unwrap().unwrap();
    });
}

#[test]
fn websocket_and_secure_websocket_upgrade_and_relay_messages() {
    async fn echo<S: AsyncRead + AsyncWrite + Unpin>(socket: S) {
        use futures_util::SinkExt;
        let mut websocket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let message = websocket.next().await.unwrap().unwrap();
        websocket.send(message).await.unwrap();
        websocket.close(None).await.unwrap();
    }
    for secure in [false, true] {
        test_runtime().block_on(async {
            use futures_util::SinkExt;
            let directory = tempfile::tempdir().unwrap();
            let ca = Arc::new(ca::Authority::generate(directory.path()).unwrap());
            let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = origin.local_addr().unwrap().port();
            let tls = tokio_rustls::TlsAcceptor::from(ca.tls_for("ws.test").unwrap());
            let server = tokio::spawn(async move {
                let (socket, _) = origin.accept().await.unwrap();
                if secure {
                    echo(tls.accept(socket).await.unwrap()).await;
                } else {
                    echo(socket).await;
                }
            });
            let mut config = settings();
            config.skip_tls_verification = true;
            config
                .mappings
                .push(super::super::environment::HostMapping {
                    domain: "ws.test".into(),
                    ip: "127.0.0.1".into(),
                });
            let captures = Arc::new(CaptureStore::default());
            let proxy = RunningProxy::start(config, Some(ca.clone()), captures.clone()).unwrap();
            let client = reqwest::Client::builder()
                .no_proxy()
                .proxy(reqwest::Proxy::all(proxy.endpoint()).unwrap())
                .add_root_certificate(reqwest::Certificate::from_der(&ca.certificate).unwrap())
                .build()
                .unwrap();
            let response = client
                .get(format!(
                    "{}://ws.test:{port}/socket",
                    if secure { "https" } else { "http" }
                ))
                .header("connection", "Upgrade")
                .header("upgrade", "websocket")
                .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                .header("sec-websocket-version", "13")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 101);
            let mut websocket = tokio_tungstenite::WebSocketStream::from_raw_socket(
                response.upgrade().await.unwrap(),
                tokio_tungstenite::tungstenite::protocol::Role::Client,
                None,
            )
            .await;
            websocket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    "代理回显".into(),
                ))
                .await
                .unwrap();
            let echoed = tokio::time::timeout(Duration::from_secs(2), websocket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(echoed.into_text().unwrap(), "代理回显");
            let _ = websocket.close(None).await;
            drop(websocket);
            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(wait_complete(&captures).await.status, Some(101));
        });
    }
}

#[test]
fn listening_address_and_port_conflicts_are_explicit() {
    let reserved = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut config = settings();
    config.listener.port = reserved.local_addr().unwrap().port();
    assert!(
        RunningProxy::start(config, None, Arc::default())
            .err()
            .unwrap()
            .contains("监听失败")
    );
    let mut config = settings();
    config.listener.allow_lan = true;
    let proxy = RunningProxy::start(config, None, Arc::default()).unwrap();
    assert!(proxy.address.ip().is_unspecified());
}

#[test]
fn captures_and_body_previews_are_bounded_and_clear_ignores_late_updates() {
    let store = CaptureStore::default();
    let peer = "127.0.0.1:1234".parse().unwrap();
    for _ in 0..RECORD_LIMIT + 5 {
        store.start("GET", "http://test/", peer, &Default::default());
    }
    let summaries = store.summaries();
    assert_eq!(summaries.len(), RECORD_LIMIT);
    assert_eq!(summaries.last().unwrap().id, 5);
    let id = summaries[0].id;
    store.update(id, |r| r.response.append(&vec![1; PREVIEW_LIMIT * 4]));
    let record = store.record(id).unwrap();
    assert_eq!(record.response.preview.len(), PREVIEW_LIMIT);
    assert_eq!(record.response.bytes, (PREVIEW_LIMIT * 4) as u64);
    assert!(record.response.display().contains("仅预览"));
    store.clear();
    store.update(id, |r| r.state = "late".into());
    assert!(store.summaries().is_empty());
}

#[test]
fn ca_import_checks_key_constraints_and_export_never_includes_private_key() {
    use rustls::pki_types::pem::PemObject;
    let directory = tempfile::tempdir().unwrap();
    let ca = ca::Authority::generate(directory.path()).unwrap();
    let identity = std::fs::read_to_string(directory.path().join("identity.pem")).unwrap();
    let export = directory.path().join("public.pem");
    ca.export(&export).unwrap();
    let public = std::fs::read_to_string(&export).unwrap();
    assert!(!public.contains("PRIVATE KEY"));
    assert_eq!(
        rustls::pki_types::CertificateDer::from_pem_slice(public.as_bytes()).unwrap(),
        ca.certificate
    );
    let other = rcgen::KeyPair::generate().unwrap();
    assert!(
        ca::Authority::parse(&public, &other.serialize_pem())
            .unwrap_err_string()
            .contains("不匹配")
    );
    let leaf = rcgen::CertificateParams::new(vec!["test".into()])
        .unwrap()
        .self_signed(&other)
        .unwrap();
    assert!(ca::Authority::parse(&leaf.pem(), &other.serialize_pem()).is_err());
    let target = directory.path().join("imported");
    // The file picker can return the key before the certificate.
    let reversed = format!(
        "{}\n{}",
        rustls::pki_types::PrivateKeyDer::from_pem_slice(identity.as_bytes())
            .map(|_| {
                let key_start = identity.find("-----BEGIN PRIVATE KEY-----").unwrap();
                &identity[key_start..]
            })
            .unwrap(),
        public
    );
    ca::Authority::import(&target, &reversed, &reversed).unwrap();
    assert_eq!(
        ca::Authority::load(&target).unwrap().fingerprint,
        ca.fingerprint
    );
}

trait ErrorString {
    fn unwrap_err_string(self) -> String;
}
impl<T> ErrorString for Result<T, String> {
    fn unwrap_err_string(self) -> String {
        self.err().expect("expected an error")
    }
}
