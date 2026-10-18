use super::model::{BodyFormat, Draft, Pair};
use reqwest::{
    Client, Method, Url,
    header::{CONTENT_TYPE, HeaderName, HeaderValue},
};
use std::{sync::OnceLock, time::Duration};

use super::environment::{EnvironmentValues, ProxySettings};

fn resolve_pairs(
    pairs: &[Pair],
    environment: &super::template::TemplateResolver,
) -> Result<Vec<Pair>, String> {
    pairs
        .iter()
        .map(|pair| {
            if !pair.enabled || pair.key.trim().is_empty() {
                return Ok(pair.clone());
            }
            Ok(Pair {
                enabled: pair.enabled,
                key: environment.resolve(&pair.key)?,
                value: environment.resolve(&pair.value)?,
                note: pair.note.clone(),
            })
        })
        .collect()
}

pub struct PreparedRequest {
    method: Method,
    pub(super) url: Url,
    pub(super) headers: Vec<(HeaderName, HeaderValue)>,
    body: String,
    pub(super) skip_tls_verification: bool,
    pub(super) proxy: Option<ProxySettings>,
    pub preview: String,
}
impl PreparedRequest {
    pub fn with_oauth_token(mut self, token: &str) -> Result<Self, String> {
        let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| "OAuth Token 格式无效")?;
        value.set_sensitive(true);
        self.headers
            .retain(|(name, _)| name != reqwest::header::AUTHORIZATION);
        self.headers.push((reqwest::header::AUTHORIZATION, value));
        self.apply_plugin_headers(&serde_json::json!({"setHeaders": {}}), &Default::default())?;
        Ok(self)
    }

    /// Export the resolved request after plugin headers have been applied.
    pub fn to_curl(&self) -> Result<String, String> {
        let mut args = vec![
            "curl --globoff --compressed".to_owned(),
            format!("--request {}", super::curl::quote(self.method.as_str())?),
            format!("--url {}", super::curl::quote(self.url.as_str())?),
        ];
        if self.method == Method::HEAD {
            args.push("--head".into());
        }
        if self.skip_tls_verification {
            args.push("--insecure".into());
        }
        if let Some(proxy) = &self.proxy {
            if proxy.built_in {
                args.push("--noproxy '*'".into());
                for mapping in &proxy.mappings {
                    let ip = if mapping.ip.contains(':') {
                        format!("[{}]", mapping.ip.trim())
                    } else {
                        mapping.ip.trim().to_owned()
                    };
                    let domain = mapping
                        .domain
                        .trim()
                        .trim_end_matches('.')
                        .to_ascii_lowercase();
                    let mut ports = vec![80, 443, self.url.port_or_known_default().unwrap_or(80)];
                    ports.sort_unstable();
                    ports.dedup();
                    for port in ports {
                        args.push(format!(
                            "--resolve {}",
                            super::curl::quote(&format!("{domain}:{port}:{ip}"))?
                        ));
                    }
                }
            } else {
                args.push(format!("--proxy {}", super::curl::quote(proxy.url.trim())?));
                args.push("--noproxy ''".into());
            }
        } else {
            args.push("--noproxy '*'".into());
        }
        if !self
            .headers
            .iter()
            .any(|(name, _)| name == reqwest::header::USER_AGENT)
        {
            args.push("--header 'User-Agent:'".into());
        }
        for (name, value) in &self.headers {
            let value = if value.is_sensitive() {
                "[已隐藏]"
            } else {
                std::str::from_utf8(value.as_bytes())
                    .map_err(|_| "请求头不是 UTF-8 文本，无法导出 cURL")?
            };
            let header = if value.is_empty() {
                format!("{name};")
            } else {
                format!("{name}: {value}")
            };
            args.push(format!("--header {}", super::curl::quote(&header)?));
        }
        if !self.body.is_empty() {
            args.push(format!("--data-raw {}", super::curl::quote(&self.body)?));
        }
        Ok(args.join(" \\\n  "))
    }

    pub fn plugin_payload(&self) -> Result<serde_json::Value, String> {
        let mut headers = serde_json::Map::new();
        for (name, value) in &self.headers {
            if headers.contains_key(name.as_str()) {
                return Err("启用签名插件时，请合并重复的请求头".into());
            }
            headers.insert(
                name.to_string(),
                value.to_str().map_err(|_| "请求头不是文本")?.into(),
            );
        }
        let uri = match self.url.query() {
            Some(query) => format!("{}?{}", self.url.path(), query),
            None => self.url.path().to_owned(),
        };
        Ok(
            serde_json::json!({"method":self.method.as_str(), "uri":uri, "body":self.body, "headers":headers}),
        )
    }
    /// Validate the complete patch before changing the request; scripts cannot mutate body/URL.
    pub fn apply_plugin_headers(
        &mut self,
        output: &serde_json::Value,
        grants: &std::collections::BTreeSet<String>,
    ) -> Result<(), String> {
        let object = output
            .as_object()
            .filter(|o| o.len() == 1)
            .ok_or("插件应返回 setHeaders 对象")?;
        let patch = object
            .get("setHeaders")
            .and_then(serde_json::Value::as_object)
            .filter(|p| p.len() <= 32)
            .ok_or("插件请求头输出无效")?;
        let mut changes = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for (name, value) in patch {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| "插件请求头名称无效")?;
            if !grants.contains(&format!("request.header.{name}"))
                || !seen.insert(name.to_string())
                || matches!(
                    name.as_str(),
                    "host" | "content-length" | "transfer-encoding" | "connection"
                )
            {
                return Err("插件修改了未授权的请求头".into());
            }
            let value = value
                .as_str()
                .filter(|v| v.len() <= 8192)
                .ok_or("插件请求头的值无效")?;
            let mut parsed = HeaderValue::from_str(value).map_err(|_| "插件请求头的值无效")?;
            if self
                .headers
                .iter()
                .any(|(existing, value)| existing == &name && value.is_sensitive())
            {
                parsed.set_sensitive(true);
            }
            changes.push((name, parsed));
        }
        for (name, value) in changes {
            self.headers.retain(|(existing, _)| existing != &name);
            self.headers.push((name, value));
        }
        self.preview = format!("{} {}", self.method, self.url);
        for (name, value) in &self.headers {
            let shown = if matches!(
                name.as_str(),
                "authorization" | "x-kso-authorization" | "cookie"
            ) {
                "[已隐藏]"
            } else {
                value.to_str().unwrap_or("[二进制]")
            };
            self.preview.push_str(&format!("\n{name}: {shown}"));
        }
        self.preview.push_str("\n\n");
        self.preview.push_str(&self.body);
        Ok(())
    }
    pub fn with_proxy(mut self, proxy: &ProxySettings) -> Result<Self, String> {
        proxy.validate()?;
        if proxy.enabled {
            self.skip_tls_verification |= proxy.skip_tls_verification;
            self.proxy = Some(proxy.clone());
        }
        Ok(self)
    }
    pub fn new(draft: &Draft, environment: &EnvironmentValues) -> Result<Self, String> {
        let method = Method::from_bytes(if draft.method == "WS" {
            b"GET"
        } else {
            draft.method.as_bytes()
        })
        .map_err(|_| "HTTP 方法无效")?;
        let environment = super::template::TemplateResolver::for_request(environment, draft)?;
        let mut resolved = draft.clone();
        resolved.headers = resolve_pairs(&draft.headers, &environment)?;
        resolved.cookies = resolve_pairs(&draft.cookies, &environment)?;
        if draft.oauth.is_some() {
            resolved.auth = false;
            resolved.token.clear();
        }
        if draft.auth && draft.oauth.is_none() {
            resolved.token = environment.resolve(&draft.token)?;
            if resolved.token.trim().is_empty() {
                return Err("请填写 Bearer Token 或选择 No Auth".into());
            }
        }
        resolved.url = environment.resolve(&draft.url)?;
        let sends_body = matches!(draft.method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE");
        resolved.body = if !sends_body {
            String::new()
        } else if draft.body_format == BodyFormat::UrlEncoded {
            let mut serializer = form_urlencoded::Serializer::new(String::new());
            for pair in draft
                .form
                .iter()
                .filter(|pair| pair.enabled && !pair.key.trim().is_empty())
            {
                serializer.append_pair(
                    &environment.resolve(&pair.key)?,
                    &environment.resolve(&pair.value)?,
                );
            }
            serializer.finish()
        } else {
            environment.resolve(&draft.body)?
        };
        resolved.params = draft
            .params
            .iter()
            .map(|pair| {
                if pair.enabled && !pair.key.trim().is_empty() {
                    Ok(Pair {
                        enabled: true,
                        key: environment.resolve(&pair.key)?,
                        value: environment.resolve(&pair.value)?,
                        note: pair.note.clone(),
                    })
                } else {
                    Ok(pair.clone())
                }
            })
            .collect::<Result<_, String>>()?;
        let mut url = Url::parse(&resolved.resolved_url()?).map_err(|_| "请输入有效的 HTTP URL")?;
        if !(matches!(url.scheme(), "http" | "https")
            || draft.method == "WS" && matches!(url.scheme(), "ws" | "wss"))
            || url.host_str().is_none()
        {
            return Err("仅支持 HTTP / HTTPS URL".into());
        }
        url.set_fragment(None);
        let body = resolved.body.clone();
        let mut headers = Vec::new();
        let mut preview = format!("{} {}", method, url);
        for header in resolved.resolved_headers() {
            let name = HeaderName::from_bytes(header.key.trim().as_bytes())
                .map_err(|_| "请求头名称无效")?;
            let value = header.value;
            if name == reqwest::header::AUTHORIZATION
                && draft.auth
                && draft.oauth.is_none()
                && draft.token.trim().is_empty()
            {
                return Err("请填写 Bearer Token 或选择 No Auth".into());
            }
            let parsed =
                HeaderValue::from_str(&value).map_err(|_| format!("请求头 {} 的值无效", name))?;
            preview.push_str(&format!("\n{}: {}", header.key.trim(), value));
            headers.push((name, parsed));
        }
        // Default JSON content type matches the editor, but explicit content types win.
        if (!body.is_empty() || sends_body && draft.body_format == BodyFormat::UrlEncoded)
            && !headers.iter().any(|(name, _)| name == CONTENT_TYPE)
        {
            let content_type = if draft.body_format == BodyFormat::UrlEncoded {
                "application/x-www-form-urlencoded"
            } else if serde_json::from_str::<serde_json::Value>(&body).is_ok() {
                "application/json"
            } else {
                "text/plain; charset=utf-8"
            };
            headers.push((CONTENT_TYPE, HeaderValue::from_static(content_type)));
            preview.push_str(&format!("\nContent-Type: {content_type}"));
        }
        preview.push_str("\n\n");
        preview.push_str(&body);
        Ok(Self {
            method,
            url,
            headers,
            body,
            skip_tls_verification: draft.skip_tls_verification,
            proxy: None,
            preview,
        })
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub text: String,
    pub bytes: usize,
    pub json: Option<serde_json::Value>,
}
impl HttpResponse {
    pub fn is_event_stream(&self) -> bool {
        self.headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("content-type")
                && value
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .eq_ignore_ascii_case("text/event-stream")
        })
    }
    pub fn language(&self) -> &'static str {
        if self.is_event_stream() {
            return "text";
        }
        let mime = self
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| {
                value
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            })
            .unwrap_or_default();
        match mime.as_str() {
            "text/html" | "application/xhtml+xml" => "html",
            "text/css" => "css",
            "text/javascript" | "application/javascript" => "javascript",
            "text/xml" | "application/xml" | "image/svg+xml" => "xml",
            _ if self.json.is_some() => "json",
            _ if self
                .text
                .trim_start()
                .get(..15)
                .is_some_and(|text| text.eq_ignore_ascii_case("<!doctype html>"))
                || self
                    .text
                    .trim_start()
                    .get(..5)
                    .is_some_and(|text| text.eq_ignore_ascii_case("<html")) =>
            {
                "html"
            }
            _ => "text",
        }
    }
}
struct AbortOnDrop(tokio::task::AbortHandle);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

static HTTP: OnceLock<Result<(tokio::runtime::Runtime, [Client; 2]), String>> = OnceLock::new();
pub(super) fn client_builder(skip_tls_verification: bool) -> reqwest::ClientBuilder {
    streaming_client_builder(skip_tls_verification).timeout(Duration::from_secs(30))
}
fn streaming_client_builder(skip_tls_verification: bool) -> reqwest::ClientBuilder {
    Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .danger_accept_invalid_certs(skip_tls_verification)
}
#[cfg(test)]
pub async fn execute(request: PreparedRequest) -> Result<HttpResponse, String> {
    execute_stream(request, None).await
}
pub async fn execute_stream(
    request: PreparedRequest,
    progress: Option<async_channel::Sender<HttpResponse>>,
) -> Result<HttpResponse, String> {
    let (runtime, clients) = HTTP
        .get_or_init(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|_| "无法启动 HTTP 执行器")?;
            let client = |skip_tls_verification| {
                streaming_client_builder(skip_tls_verification)
                    .build()
                    .map_err(|_| "无法初始化 HTTP 客户端")
            };
            Ok((runtime, [client(false)?, client(true)?]))
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let client = clients[usize::from(request.skip_tls_verification)].clone();
    let task = runtime.spawn(async move {
        let client = if let Some(proxy) = &request.proxy {
            proxy.apply(streaming_client_builder(request.skip_tls_verification))?
                .build()
                .map_err(|_| "无法初始化代理客户端".to_owned())?
        } else {
            client
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let mut builder = client.request(request.method, request.url);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body);
        }
        let mut response = tokio::time::timeout_at(deadline, builder.send())
            .await
            .map_err(|_| "等待响应头超时（30 秒）".to_owned())?
            .map_err(network_error)?;
        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
        let mut snapshot = HttpResponse {
            status,
            headers: headers.clone(),
            text: String::new(),
            bytes: 0,
            json: None,
        };
        if snapshot.is_event_stream() {
            let mut buffer = super::sse::Buffer::new();
            if let Some(sender) = &progress {
                sender
                    .send(snapshot.clone())
                    .await
                    .map_err(|_| "SSE 已取消")?;
            }
            let mut ticker = tokio::time::interval(Duration::from_millis(50));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut dirty = false;
            loop {
                tokio::select! {
                    chunk = response.chunk() => {
                        match chunk {
                            Ok(Some(chunk)) => { buffer.push(&chunk, false); dirty = true; }
                            Ok(None) => break,
                            Err(error) => {
                                snapshot.text = buffer.text();
                                snapshot.bytes = buffer.bytes;
                                if let Some(sender) = &progress { let _ = sender.send(snapshot.clone()).await; }
                                return Err(network_error(error));
                            }
                        }
                    }
                    _ = ticker.tick(), if dirty && progress.is_some() => {
                        snapshot.text = buffer.text();
                        snapshot.bytes = buffer.bytes;
                        if let Some(sender) = &progress {
                            sender.send(snapshot.clone()).await.map_err(|_| "SSE 已取消")?;
                        }
                        dirty = false;
                    }
                }
            }
            buffer.push(&[], true);
            snapshot.text = buffer.text();
            snapshot.bytes = buffer.bytes;
            if let Some(sender) = &progress {
                let _ = sender.send(snapshot.clone()).await;
            }
            return Ok(snapshot);
        }
        let encoding = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| {
                v.split(';').skip(1).find_map(|part| {
                    let (name, value) = part.trim().split_once('=')?;
                    name.eq_ignore_ascii_case("charset")
                        .then(|| value.trim().trim_matches('"'))
                })
            })
            .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
            .unwrap_or(encoding_rs::UTF_8);
        let mut bytes = Vec::new();
        while let Some(chunk) = tokio::time::timeout_at(deadline, response.chunk())
            .await
            .map_err(|_| "HTTP 请求超时（30 秒）".to_owned())?
            .map_err(network_error)?
        {
            if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
                return Err("响应内容超过 16 MiB，请使用下载工具获取".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let (text, _, _) = encoding.decode(&bytes);
        let text = text.into_owned();
        let json = serde_json::from_str(&text).ok();
        Ok(HttpResponse {
            status,
            headers,
            text,
            bytes: bytes.len(),
            json,
        })
    });
    let _abort = AbortOnDrop(task.abort_handle());
    task.await.map_err(|_| "HTTP 请求已终止".to_string())?
}
fn network_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "请求超时（连接最多 10 秒，总耗时最多 30 秒）"
    } else if error.is_connect() {
        "连接失败，请检查地址、网络和 TLS 证书"
    } else if error.is_body() || error.is_decode() {
        "读取响应内容失败"
    } else {
        "HTTP 请求失败，请检查请求配置和网络"
    }
    .into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn response_language_uses_case_insensitive_mime_and_html_fallback() {
        let mut response = super::HttpResponse {
            status: 200,
            headers: vec![("Content-Type".into(), "Text/HTML; charset=utf-8".into())],
            text: "<p>你好</p>".into(),
            bytes: 0,
            json: None,
        };
        assert_eq!(response.language(), "html");
        response.headers.clear();
        response.text = " \n<!DOCTYPE html><html></html>".into();
        assert_eq!(response.language(), "html");
        response.text = "ordinary text".into();
        assert_eq!(response.language(), "text");
        response.json = Some(serde_json::json!({"value":1}));
        assert_eq!(response.language(), "json");
    }

    use super::*;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::mpsc,
        thread,
    };
    fn variables(base_url: &str) -> EnvironmentValues {
        EnvironmentValues(
            [
                ("base_url".into(), base_url.into()),
                ("workspace_id".into(), "test_workspace".into()),
                ("api_token".into(), "test_token".into()),
            ]
            .into(),
        )
    }
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }
    fn tls_server_config() -> std::sync::Arc<rustls::ServerConfig> {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
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
        std::sync::Arc::new(config)
    }
    #[test]
    fn tls_verification_is_enabled_by_default_and_skip_is_request_scoped() {
        let config = tls_server_config();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("https://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            for _ in 0..3 {
                let (socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let connection = rustls::ServerConnection::new(config.clone()).unwrap();
                let mut stream = rustls::StreamOwned::new(connection, socket);
                let mut request = [0; 2048];
                if stream.read(&mut request).is_ok() {
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK",
                    );
                    let _ = stream.flush();
                }
            }
        });
        let runtime = runtime();
        for skip in [false, true, false] {
            let draft = Draft {
                url: url.clone(),
                skip_tls_verification: skip,
                ..Draft::default()
            };
            let result = runtime.block_on(execute(
                PreparedRequest::new(&draft, &variables("")).unwrap(),
            ));
            if skip {
                let response = result.unwrap();
                assert_eq!(response.status, 200);
                assert_eq!(response.text, "OK");
            } else {
                assert!(
                    result.is_err(),
                    "untrusted TLS must fail when verification is enabled"
                );
            }
        }
        server.join().unwrap();
    }
    #[test]
    fn built_in_proxy_connects_mapped_ip_preserves_host_port_and_tls_sni() {
        for tls in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = thread::spawn(move || {
                let (socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut stream: Box<dyn ReadWrite> = if tls {
                    let mut stream = rustls::StreamOwned::new(
                        rustls::ServerConnection::new(tls_server_config()).unwrap(),
                        socket,
                    );
                    stream.conn.complete_io(&mut stream.sock).unwrap();
                    assert_eq!(stream.conn.server_name(), Some("mapped.invalid"));
                    Box::new(stream)
                } else {
                    Box::new(socket)
                };
                let request = read_mapped_request(&mut stream);
                assert!(request.starts_with("GET /mapped HTTP/1.1"));
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains(&format!("host: mapped.invalid:{port}"))
                );
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK",
                    )
                    .unwrap();
                stream.flush().unwrap();
            });
            let proxy = ProxySettings {
                enabled: true,
                built_in: true,
                skip_tls_verification: tls,
                mappings: vec![super::super::environment::HostMapping {
                    domain: "mapped.invalid".into(),
                    ip: "127.0.0.1".into(),
                }],
                ..Default::default()
            };
            let draft = Draft {
                url: format!(
                    "{}://mapped.invalid:{port}/mapped",
                    if tls { "https" } else { "http" }
                ),
                ..Default::default()
            };
            let request = PreparedRequest::new(&draft, &variables(""))
                .unwrap()
                .with_proxy(&proxy)
                .unwrap();
            let curl = request.to_curl().unwrap();
            assert!(
                curl.contains("--resolve")
                    && curl.contains(&format!("mapped.invalid:{port}:127.0.0.1"))
            );
            let result = runtime().block_on(execute(request)).unwrap();
            assert_eq!(result.status, 200);
            server.join().unwrap();
        }
    }
    trait ReadWrite: std::io::Read + std::io::Write {}
    impl<T: std::io::Read + std::io::Write> ReadWrite for T {}
    #[test]
    fn proxy_routes_http_and_https_and_applies_tls_setting() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = ProxySettings {
            enabled: true,
            url: format!("http://{}", listener.local_addr().unwrap()),
            skip_tls_verification: true,
            ..ProxySettings::default()
        };
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            assert!(
                read_request(&mut stream)
                    .starts_with("GET http://target.invalid/resource HTTP/1.1")
            );
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
                .unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            assert!(read_request(&mut stream).starts_with("CONNECT target.invalid:443 HTTP/1.1"));
            stream
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .unwrap();
            let mut tls = rustls::StreamOwned::new(
                rustls::ServerConnection::new(tls_server_config()).unwrap(),
                stream,
            );
            let mut request = [0; 2048];
            let n = tls.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..n]).starts_with("GET /resource HTTP/1.1"));
            tls.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
                .unwrap();
            tls.flush().unwrap();
        });
        let runtime = runtime();
        for scheme in ["http", "https"] {
            let draft = Draft {
                url: format!("{scheme}://target.invalid/resource"),
                ..Draft::default()
            };
            let prepared = PreparedRequest::new(&draft, &variables(""))
                .unwrap()
                .with_proxy(&proxy)
                .unwrap();
            assert_eq!(runtime.block_on(execute(prepared)).unwrap().status, 200);
        }
        server.join().unwrap();
        let disabled = ProxySettings {
            enabled: false,
            ..proxy
        };
        let draft = Draft {
            url: "https://target.invalid".into(),
            ..Draft::default()
        };
        let prepared = PreparedRequest::new(&draft, &variables(""))
            .unwrap()
            .with_proxy(&disabled)
            .unwrap();
        assert!(prepared.proxy.is_none() && !prepared.skip_tls_verification);
    }
    #[test]
    fn https_proxy_certificate_validation_can_be_disabled() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_url = format!("https://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            for _ in 0..3 {
                let (socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut tls = rustls::StreamOwned::new(
                    rustls::ServerConnection::new(tls_server_config()).unwrap(),
                    socket,
                );
                let mut request = [0; 2048];
                if let Ok(n) = tls.read(&mut request) {
                    assert!(
                        String::from_utf8_lossy(&request[..n])
                            .starts_with("GET http://target.invalid/ HTTP/1.1")
                    );
                    tls.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK",
                    )
                    .unwrap();
                    tls.flush().unwrap();
                }
            }
        });
        let runtime = runtime();
        for skip in [false, true, false] {
            let draft = Draft {
                url: "http://target.invalid".into(),
                ..Draft::default()
            };
            let proxy = ProxySettings {
                enabled: true,
                url: proxy_url.clone(),
                skip_tls_verification: skip,
                ..ProxySettings::default()
            };
            let prepared = PreparedRequest::new(&draft, &variables(""))
                .unwrap()
                .with_proxy(&proxy)
                .unwrap();
            assert_eq!(runtime.block_on(execute(prepared)).is_ok(), skip);
        }
        server.join().unwrap();
    }
    fn read_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        read_mapped_request(stream)
    }
    fn read_mapped_request(stream: &mut impl std::io::Read) -> String {
        let mut bytes = Vec::new();
        let mut chunk = [0; 2048];
        loop {
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
            if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&bytes[..end]);
                let length = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    return String::from_utf8(bytes).unwrap();
                }
            }
        }
    }
    #[test]
    fn sends_template_methods_and_cookie_extraction_with_consistent_snapshot() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            tx.send(read_request(&mut stream)).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
                .unwrap();
        });
        let draft = Draft {
            method: "POST".into(),
            url: format!("{url}/{{{{uuid()}}}}"),
            params: vec![Pair::new("n", "{{random_int(7,7)}}", "")],
            headers: vec![Pair::new("Cookie", "csrftoken=old", ""), Pair::new("X-CSRFToken", "{{cookie('csrftoken')}}", ""), Pair::new("X-Request-ID", "{{uuid()}}", "")],
            cookies: vec![Pair::new("csrftoken", "{{random_string(24)}}", "")],
            body: r#"{"id":"{{uuid()}}","csrf":"{{cookie('csrftoken')}}","xss":"{{xss()}}","time":"{{now()}}"}"#.into(),
            ..Draft::default()
        };
        let request = PreparedRequest::new(&draft, &variables("")).unwrap();
        let id = request.url.path().trim_start_matches('/').to_string();
        let body: serde_json::Value = serde_json::from_str(&request.body).unwrap();
        assert_eq!(body["id"], id);
        let csrf = body["csrf"].as_str().unwrap();
        assert_eq!(csrf.len(), 24);
        assert!(request.preview.contains(&format!("X-CSRFToken: {csrf}")));
        let preview = request.preview.clone();
        assert_eq!(runtime().block_on(execute(request)).unwrap().status, 200);
        let wire = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(wire.starts_with(&format!("POST /{id}?n=7 HTTP/1.1")));
        assert!(wire.contains(&format!("x-request-id: {id}\r\n")));
        assert!(wire.contains(&format!("x-csrftoken: {csrf}\r\n")));
        assert!(wire.contains(&format!("cookie: csrftoken={csrf}\r\n")));
        assert!(preview.ends_with(wire.split_once("\r\n\r\n").unwrap().1));
        server.join().unwrap();
    }
    #[test]
    fn sends_urlencoded_form_with_templates_duplicate_keys_and_exact_plugin_payload() {
        use std::{io::Write, net::TcpListener, sync::mpsc, thread};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            tx.send(read_request(&mut stream)).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
                .unwrap();
        });
        let mut env = variables(&base);
        env.0.insert("name".into(), "中 文".into());
        env.0.insert("field".into(), "tag".into());
        let mut disabled = Pair::new("skip", "{{missing}}", "");
        disabled.enabled = false;
        let mut draft = Draft {
            method: "POST".into(),
            url: format!("{base}/form"),
            body_format: BodyFormat::UrlEncoded,
            body: "{{unused_raw_missing}}".into(),
            form: vec![
                Pair::new("name", "{{name}}", ""),
                Pair::new("{{field}}", "a+b&=", ""),
                Pair::new("tag", "second", ""),
                Pair::new("empty", "", ""),
                disabled,
                Pair::new("", "{{missing}}", ""),
            ],
            ..Draft::default()
        };
        let expected = "name=%E4%B8%AD+%E6%96%87&tag=a%2Bb%26%3D&tag=second&empty=";
        let request = PreparedRequest::new(&draft, &env).unwrap();
        assert_eq!(request.plugin_payload().unwrap()["body"], expected);
        assert_eq!(
            request.plugin_payload().unwrap()["headers"]["content-type"],
            "application/x-www-form-urlencoded"
        );
        assert!(request.preview.ends_with(expected));
        runtime().block_on(execute(request)).unwrap();
        let wire = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(
            wire.to_lowercase()
                .contains("content-type: application/x-www-form-urlencoded\r\n")
        );
        assert!(wire.ends_with(expected));
        server.join().unwrap();
        draft.headers.push(Pair::new(
            "Content-Type",
            "application/x-www-form-urlencoded; charset=UTF-8",
            "",
        ));
        let request = PreparedRequest::new(&draft, &env).unwrap();
        assert_eq!(
            request.plugin_payload().unwrap()["headers"]["content-type"],
            "application/x-www-form-urlencoded; charset=UTF-8"
        );
        draft.form.push(Pair::new("missing", "{{missing}}", ""));
        assert!(
            PreparedRequest::new(&draft, &env)
                .err()
                .unwrap()
                .contains("missing")
        );
    }

    #[test]
    fn empty_form_keeps_mime_and_get_ignores_body_configuration() {
        let mut draft = Draft {
            method: "POST".into(),
            url: "https://example.test/".into(),
            body_format: BodyFormat::UrlEncoded,
            ..Draft::default()
        };
        let env = variables("https://example.test");
        let request = PreparedRequest::new(&draft, &env).unwrap();
        assert!(request.body.is_empty());
        assert_eq!(
            request.plugin_payload().unwrap()["headers"]["content-type"],
            "application/x-www-form-urlencoded"
        );
        draft.method = "GET".into();
        draft.form.push(Pair::new("key", "{{missing}}", ""));
        let request = PreparedRequest::new(&draft, &env).unwrap();
        assert!(request.body.is_empty());
        assert!(
            request.plugin_payload().unwrap()["headers"]
                .get("content-type")
                .is_none()
        );
    }

    #[test]
    fn sends_resolved_post_headers_cookies_and_body_and_keeps_http_error_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            tx.send(read_request(&mut stream)).unwrap();
            stream.write_all(b"HTTP/1.1 418 I'm a teapot\r\nContent-Type: application/json\r\nContent-Length: 15\r\nX-Test: actual\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nConnection: close\r\n\r\n{\"error\":true}\n").unwrap();
        });
        let mut draft = super::super::model::test_requests().remove(0).draft;
        draft.method = "POST".into();
        draft.url = "{{base_url}}/echo#fragment".into();
        draft.params = vec![Pair::new("query", "{{search}}", "")];
        draft
            .headers
            .push(Pair::new("Cookie", "session=old; keep=1", ""));
        draft.cookies = vec![Pair::new("{{cookie_name}}", "{{cookie_value}}", "")];
        draft
            .headers
            .push(Pair::new("X-Customer", "{{customer_id}}", ""));
        draft.body = "{\"workspace\":\"{{workspace_id}}\",\"customer\":\"{{customer_id}}\"}".into();
        let mut env = variables(&base);
        for (key, value) in [
            ("search", "中 文"),
            ("cookie_name", "session"),
            ("cookie_value", "new"),
            ("customer_id", "customer_1"),
        ] {
            env.0.insert(key.into(), value.into());
        }
        let request = PreparedRequest::new(&draft, &env).unwrap();
        assert!(request.preview.contains("Bearer test_token"));
        let response = runtime().block_on(execute(request)).unwrap();
        let wire = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(wire.starts_with("POST /echo?query=%E4%B8%AD%20%E6%96%87 HTTP/1.1"));
        let headers = wire.to_lowercase();
        assert!(headers.contains("authorization: bearer test_token\r\n"));
        assert!(headers.contains("cookie: session=new; keep=1\r\n"));
        assert!(headers.contains("x-workspace: test_workspace\r\n"));
        assert!(headers.contains("content-type: application/json\r\n"));
        assert!(headers.contains("x-customer: customer_1\r\n"));
        assert!(wire.ends_with("{\"workspace\":\"test_workspace\",\"customer\":\"customer_1\"}"));
        assert_eq!(response.status, 418);
        assert_eq!(response.bytes, 15);
        assert_eq!(response.text, "{\"error\":true}\n");
        assert_eq!(response.json.unwrap()["error"], true);
        assert_eq!(
            response
                .headers
                .iter()
                .filter(|(key, _)| key == "set-cookie")
                .count(),
            2
        );
        server.join().unwrap();
    }
    #[test]
    fn accepts_plain_text_empty_response_and_preserves_redirects() {
        for (status, body, extra) in [
            (202, "plain text", ""),
            (204, "", ""),
            (302, "redirect", "Location: http://127.0.0.1:1/never\r\n"),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let wire = read_request(&mut stream);
                assert!(wire.starts_with("GET / HTTP/1.1"));
                assert!(!wire.to_lowercase().contains("authorization:"));
                stream.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: text/plain\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
            });
            let mut draft = super::super::model::test_requests().remove(0).draft;
            draft.url = url.clone();
            draft.params.clear();
            draft.headers.clear();
            draft.auth = false;
            let response = runtime()
                .block_on(execute(
                    PreparedRequest::new(&draft, &variables(&url)).unwrap(),
                ))
                .unwrap();
            assert_eq!(response.status, status);
            assert_eq!(response.text, body);
            assert!(response.json.is_none());
            server.join().unwrap();
        }
    }
    #[test]
    fn rejects_unresolved_variables_and_header_injection_before_network() {
        let mut draft = super::super::model::test_requests().remove(0).draft;
        let mut env = variables("http://localhost:1");
        env.0.insert("api_token".into(), String::new());
        assert!(
            PreparedRequest::new(&draft, &env)
                .err()
                .unwrap()
                .contains("api_token")
        );
        env.0.insert("api_token".into(), "test".into());
        draft
            .headers
            .push(Pair::new("X-Unknown", "{{unknown}}", ""));
        assert!(
            PreparedRequest::new(&draft, &env)
                .err()
                .unwrap()
                .contains("unknown")
        );
        draft.headers.pop();
        draft
            .headers
            .push(Pair::new("X-Test", "value\r\nInjected: true", ""));
        assert!(PreparedRequest::new(&draft, &env).is_err());
        draft.headers.pop();
        draft.headers.push(Pair {
            enabled: false,
            key: "X-Disabled".into(),
            value: "{{unknown}}".into(),
            note: String::new(),
        });
        assert!(PreparedRequest::new(&draft, &env).is_ok());
        draft.url = "file:///tmp/test".into();
        assert!(PreparedRequest::new(&draft, &env).is_err());
    }
    #[test]
    fn dropping_request_future_closes_pending_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            tx.send(()).unwrap();
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).unwrap(), 0);
        });
        let mut draft = super::super::model::test_requests().remove(0).draft;
        draft.url = url.clone();
        draft.params.clear();
        draft.headers.clear();
        draft.auth = false;
        let request = PreparedRequest::new(&draft, &variables(&url)).unwrap();
        let runtime = runtime();
        runtime.block_on(async {
            let task = tokio::spawn(execute(request));
            tokio::task::yield_now().await;
            // Server signals only after the request is on the wire.
            let ready = tokio::task::spawn_blocking(move || {
                rx.recv_timeout(Duration::from_secs(3)).unwrap()
            });
            ready.await.unwrap();
            task.abort();
            assert!(task.await.err().unwrap().is_cancelled());
        });
        server.join().unwrap();
    }
    #[test]
    fn sse_streams_before_eof_and_survives_the_http_total_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/events", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            assert!(request.starts_with("POST /events HTTP/1.1"));
            assert!(request.contains("accept: text/event-stream"));
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: Text/Event-Stream; charset=utf-8\r\nConnection: close\r\n\r\n").unwrap();
            stream
                .write_all("id: 1\nevent: message\ndata: 你好\n\n".as_bytes())
                .unwrap();
            // A normal response has a 30-second total deadline; an SSE stream must outlive it.
            thread::sleep(Duration::from_secs(31));
            stream.write_all(b"data: [DONE]\n\n").unwrap();
        });
        let draft = Draft {
            method: "POST".into(),
            url,
            body: "{}".into(),
            headers: vec![Pair::new("Accept", "text/event-stream", "")],
            ..Draft::default()
        };
        let request = PreparedRequest::new(&draft, &variables("")).unwrap();
        runtime().block_on(async {
            let (sender, receiver) = async_channel::bounded(1);
            let task = tokio::spawn(execute_stream(request, Some(sender)));
            let mut first = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(first.is_event_stream());
            assert_eq!(first.status, 200);
            while !first.text.contains("你好") {
                first = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
                    .await
                    .unwrap()
                    .unwrap();
            }
            assert!(!task.is_finished(), "events must be visible before EOF");
            let mut last = first;
            while let Ok(response) = tokio::time::timeout(Duration::from_secs(35), receiver.recv())
                .await
                .unwrap()
            {
                last = response;
            }
            assert!(last.text.contains("data: [DONE]"));
            assert_eq!(task.await.unwrap().unwrap().text, last.text);
        });
        server.join().unwrap();
    }
    #[test]
    fn sse_cancellation_closes_an_open_event_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/events", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: first\n\n").unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            assert_eq!(stream.read(&mut [0]).unwrap(), 0);
        });
        let request = PreparedRequest::new(
            &Draft {
                url,
                ..Draft::default()
            },
            &variables(""),
        )
        .unwrap();
        runtime().block_on(async {
            let (sender, receiver) = async_channel::bounded(1);
            let task = tokio::spawn(execute_stream(request, Some(sender)));
            loop {
                let response = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
                    .await
                    .unwrap()
                    .unwrap();
                if response.text.contains("data: first") {
                    break;
                }
            }
            task.abort();
            assert!(task.await.err().unwrap().is_cancelled());
        });
        server.join().unwrap();
    }
    #[test]
    fn sse_disconnect_error_keeps_the_latest_partial_content() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/events", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
            let data = "data: partial\n\n";
            write!(stream, "{:X}\r\n{data}\r\n", data.len()).unwrap();
            // Disconnect without the terminating chunk, producing a transport error.
        });
        let request = PreparedRequest::new(
            &Draft {
                url,
                ..Draft::default()
            },
            &variables(""),
        )
        .unwrap();
        runtime().block_on(async {
            let (sender, receiver) = async_channel::bounded(1);
            let task = tokio::spawn(execute_stream(request, Some(sender)));
            let mut text = String::new();
            while let Ok(response) = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
                .await
                .unwrap()
            {
                text = response.text;
            }
            assert!(text.contains("data: partial"));
            assert!(task.await.unwrap().is_err());
        });
        server.join().unwrap();
    }
}
