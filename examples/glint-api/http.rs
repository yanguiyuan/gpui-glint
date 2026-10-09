use super::model::{Draft, Pair};
use reqwest::{
    Client, Method, Url,
    header::{CONTENT_TYPE, HeaderName, HeaderValue},
};
use std::{sync::OnceLock, time::Duration};

use super::environment::EnvironmentValues;

fn resolve_pairs(pairs: &[Pair], environment: &EnvironmentValues) -> Result<Vec<Pair>, String> {
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
    url: Url,
    headers: Vec<(HeaderName, HeaderValue)>,
    body: String,
    pub preview: String,
}
impl PreparedRequest {
    pub fn new(draft: &Draft, environment: &EnvironmentValues) -> Result<Self, String> {
        let method = Method::from_bytes(draft.method.as_bytes()).map_err(|_| "HTTP 方法无效")?;
        let mut resolved = draft.clone();
        resolved.headers = resolve_pairs(&draft.headers, environment)?;
        resolved.cookies = resolve_pairs(&draft.cookies, environment)?;
        if draft.auth {
            resolved.token = environment.resolve(&draft.token)?;
            if resolved.token.trim().is_empty() {
                return Err("请填写 Bearer Token 或选择 No Auth".into());
            }
        }
        resolved.url = environment.resolve(&draft.url)?;
        resolved.body = if matches!(draft.method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE") {
            environment.resolve(&draft.body)?
        } else {
            String::new()
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
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
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
            if name == reqwest::header::AUTHORIZATION && draft.auth && draft.token.trim().is_empty()
            {
                return Err("请填写 Bearer Token 或选择 No Auth".into());
            }
            let parsed =
                HeaderValue::from_str(&value).map_err(|_| format!("请求头 {} 的值无效", name))?;
            preview.push_str(&format!("\n{}: {}", header.key.trim(), value));
            headers.push((name, parsed));
        }
        // Default JSON content type matches the editor, but explicit content types win.
        if !body.is_empty() && !headers.iter().any(|(name, _)| name == CONTENT_TYPE) {
            let content_type = if serde_json::from_str::<serde_json::Value>(&body).is_ok() {
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
    pub fn language(&self) -> &'static str {
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

static HTTP: OnceLock<Result<(tokio::runtime::Runtime, Client), String>> = OnceLock::new();
pub async fn execute(request: PreparedRequest) -> Result<HttpResponse, String> {
    let (runtime, client) = HTTP
        .get_or_init(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|_| "无法启动 HTTP 执行器")?;
            let client = Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "无法初始化 HTTP 客户端")?;
            Ok((runtime, client))
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let client = client.clone();
    let task = runtime.spawn(async move {
        let mut builder = client.request(request.method, request.url);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body);
        }
        let mut response = builder.send().await.map_err(network_error)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect();
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
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
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
    fn read_request(stream: &mut TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
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
}
