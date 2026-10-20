pub mod browser;
pub mod ca;

use super::environment::ProxySettings;
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::body::Body as _;
use hyper::{
    Request, Response, StatusCode,
    body::{Frame, Incoming},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use std::{
    collections::VecDeque,
    convert::Infallible,
    net::SocketAddr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{Semaphore, watch},
};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Body = UnsyncBoxBody<Bytes, Error>;
const PREVIEW_LIMIT: usize = 32 * 1024;
const RECORD_LIMIT: usize = 1_000;

#[derive(Clone, Default)]
pub struct CapturedBody {
    pub preview: Vec<u8>,
    pub bytes: u64,
}
impl CapturedBody {
    fn append(&mut self, bytes: &[u8]) {
        self.bytes += bytes.len() as u64;
        self.preview.extend_from_slice(
            &bytes[..bytes
                .len()
                .min(PREVIEW_LIMIT.saturating_sub(self.preview.len()))],
        );
    }
    pub fn display(&self) -> String {
        let mut text = if self.preview.contains(&0) {
            self.preview
                .chunks(16)
                .map(|line| {
                    line.iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            String::from_utf8_lossy(&self.preview).into_owned()
        };
        if self.bytes > self.preview.len() as u64 {
            text.push_str(&format!(
                "\n\n[仅预览前 {} 字节，共 {} 字节]",
                self.preview.len(),
                self.bytes
            ));
        }
        text
    }
}
#[derive(Clone)]
pub struct CaptureRecord {
    pub id: u64,
    pub time: String,
    pub method: String,
    pub url: String,
    pub peer: String,
    pub status: Option<u16>,
    pub state: String,
    pub error: Option<String>,
    pub elapsed_ms: u128,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub request: CapturedBody,
    pub response: CapturedBody,
}
#[derive(Clone)]
pub struct CaptureSummary {
    pub id: u64,
    pub time: String,
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub state: String,
    pub elapsed_ms: u128,
    pub bytes: u64,
}
#[derive(Default)]
pub struct CaptureStore {
    records: Mutex<VecDeque<CaptureRecord>>,
    next_id: AtomicU64,
    revision: AtomicU64,
}
impl CaptureStore {
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }
    pub fn summaries(&self) -> Vec<CaptureSummary> {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .rev()
            .map(|r| CaptureSummary {
                id: r.id,
                time: r.time.clone(),
                method: r.method.clone(),
                url: r.url.clone(),
                status: r.status,
                state: r.state.clone(),
                elapsed_ms: r.elapsed_ms,
                bytes: r.response.bytes,
            })
            .collect()
    }
    pub fn record(&self, id: u64) -> Option<CaptureRecord> {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }
    pub fn clear(&self) {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.revision.fetch_add(1, Ordering::Relaxed);
    }
    fn start(&self, method: &str, url: &str, peer: SocketAddr, headers: &hyper::HeaderMap) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        if records.len() >= RECORD_LIMIT {
            records.pop_front();
        }
        records.push_back(CaptureRecord {
            id,
            time: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
            method: method.into(),
            url: url.into(),
            peer: peer.to_string(),
            status: None,
            state: "等待响应".into(),
            error: None,
            elapsed_ms: 0,
            request_headers: header_pairs(headers),
            response_headers: vec![],
            request: Default::default(),
            response: Default::default(),
        });
        self.revision.fetch_add(1, Ordering::Relaxed);
        id
    }
    fn update(&self, id: u64, update: impl FnOnce(&mut CaptureRecord)) {
        let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(record) = records.iter_mut().find(|r| r.id == id) {
            update(record);
            self.revision.fetch_add(1, Ordering::Relaxed);
        }
    }
}
fn header_pairs(headers: &hyper::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .take(256)
        .map(|(name, value)| {
            (
                name.to_string(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect()
}
struct CaptureTransfer {
    store: Arc<CaptureStore>,
    id: u64,
    started: Instant,
    finished: bool,
}
impl CaptureTransfer {
    fn finish(&mut self, error: Option<String>, state: &str) {
        self.finished = true;
        self.store.update(self.id, |r| {
            r.state = state.into();
            r.error = error;
            r.elapsed_ms = self.started.elapsed().as_millis();
        });
    }
}
impl Drop for CaptureTransfer {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(Some("客户端断开或代理服务已停止".into()), "中断");
        }
    }
}

pub struct RunningProxy {
    pub address: SocketAddr,
    cancel: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for RunningProxy {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
        self.task.abort();
    }
}
fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}
#[derive(Clone)]
struct Service {
    client: reqwest::Client,
    ca: Option<Arc<ca::Authority>>,
    captures: Arc<CaptureStore>,
    cancel: watch::Receiver<bool>,
    address: SocketAddr,
    limits: Arc<Semaphore>,
}
impl RunningProxy {
    pub fn start(
        mut settings: ProxySettings,
        authority: Option<Arc<ca::Authority>>,
        captures: Arc<CaptureStore>,
    ) -> Result<Self, String> {
        settings.enabled = true;
        settings.validate()?;
        let bind = if settings.listener.allow_lan {
            "0.0.0.0"
        } else {
            "127.0.0.1"
        };
        let listener = std::net::TcpListener::bind((bind, settings.listener.port))
            .map_err(|e| format!("代理端口监听失败：{e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        if !settings.built_in
            && reqwest::Url::parse(&settings.url)
                .ok()
                .is_some_and(|url| is_self(&url, address))
        {
            return Err("上游代理不能指向当前代理监听端口".into());
        }
        let builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .danger_accept_invalid_certs(settings.skip_tls_verification)
            .gzip(false)
            .brotli(false)
            .deflate(false)
            .http1_only();
        let client = settings
            .apply(builder)?
            .build()
            .map_err(|e| e.to_string())?;
        let (cancel, receiver) = watch::channel(false);
        let state = Service {
            client,
            ca: authority,
            captures: captures.clone(),
            cancel: receiver,
            address,
            limits: Arc::new(Semaphore::new(512)),
        };
        let runtime = runtime()?;
        let task = runtime.spawn(async move {
            let Ok(listener) = tokio::net::TcpListener::from_std(listener) else { return; };
            let mut connections = tokio::task::JoinSet::new();
            let mut cancel = state.cancel.clone();
            loop {
                tokio::select! {
                    _ = cancel.changed() => break,
                    Some(_) = connections.join_next(), if !connections.is_empty() => {},
                    accepted = listener.accept() => match accepted {
                        Ok((socket, peer)) => {
                            let Ok(permit) = state.limits.clone().try_acquire_owned() else { continue; };
                            let service = state.clone();
                            connections.spawn(async move { let _permit = permit; serve(socket, service, peer, None).await; });
                        },
                        Err(_) => break,
                    }
                }
            }
        });
        Ok(Self {
            address,
            cancel,
            task,
        })
    }
    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.address.port())
    }
}
async fn serve<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    socket: S,
    service: Service,
    peer: SocketAddr,
    target: Option<reqwest::Url>,
) {
    let mut cancel = service.cancel.clone();
    let connection = http1::Builder::new()
        .max_buf_size(64 * 1024)
        .serve_connection(
            TokioIo::new(socket),
            service_fn(move |request| dispatch(request, service.clone(), peer, target.clone())),
        )
        .with_upgrades();
    tokio::select! { _ = connection => {}, _ = cancel.changed() => {} }
}
fn body(bytes: impl Into<Bytes>) -> Body {
    Full::new(bytes.into())
        .map_err(|never: Infallible| match never {})
        .boxed_unsync()
}
fn failure(status: StatusCode, text: impl Into<String>) -> Response<Body> {
    let mut response = Response::new(body(text.into()));
    *response.status_mut() = status;
    response
}
fn is_self(url: &reqwest::Url, address: SocketAddr) -> bool {
    url.port_or_known_default() == Some(address.port())
        && url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified() || ip == address.ip())
        })
}
fn clean_headers(headers: &hyper::HeaderMap, upgrade: bool) -> hyper::HeaderMap {
    let mut cleaned = headers.clone();
    if let Some(connection) = headers
        .get(hyper::header::CONNECTION)
        .and_then(|v| v.to_str().ok())
    {
        for name in connection.split(',').map(str::trim) {
            if !upgrade || !name.eq_ignore_ascii_case("upgrade") {
                cleaned.remove(name);
            }
        }
    }
    for name in [
        "proxy-connection",
        "proxy-authorization",
        "proxy-authenticate",
        "keep-alive",
        "te",
        "trailer",
        "transfer-encoding",
    ] {
        cleaned.remove(name);
    }
    if !upgrade {
        cleaned.remove("connection");
        cleaned.remove("upgrade");
    }
    cleaned
}
async fn dispatch(
    mut request: Request<Incoming>,
    service: Service,
    peer: SocketAddr,
    target: Option<reqwest::Url>,
) -> Result<Response<Body>, Infallible> {
    if request.method() == hyper::Method::CONNECT {
        if target.is_some() {
            return Ok(failure(StatusCode::BAD_REQUEST, "不能嵌套 CONNECT"));
        }
        return Ok(connect(request, service, peer));
    }
    let url = if let Some(base) = &target {
        if request.uri().scheme().is_some() || !request.uri().path().starts_with('/') {
            return Ok(failure(StatusCode::BAD_REQUEST, "HTTPS 隧道内需要相对路径"));
        }
        // Never let an origin-form path such as //other-host change the CONNECT authority.
        let mut url = base.clone();
        url.set_path(request.uri().path());
        url.set_query(request.uri().query());
        url
    } else {
        match reqwest::Url::parse(&request.uri().to_string()) {
            Ok(url)
                if url.scheme() == "http"
                    && url.host_str().is_some()
                    && url.username().is_empty()
                    && url.password().is_none() =>
            {
                url
            }
            _ => {
                return Ok(failure(
                    StatusCode::BAD_REQUEST,
                    "请使用 HTTP 代理请求或 HTTPS CONNECT",
                ));
            }
        }
    };
    if is_self(&url, service.address) {
        return Ok(failure(
            StatusCode::LOOP_DETECTED,
            "代理目标不能是自身监听端口",
        ));
    }
    let started = Instant::now();
    let id = service.captures.start(
        request.method().as_str(),
        url.as_str(),
        peer,
        request.headers(),
    );
    let mut transfer = CaptureTransfer {
        store: service.captures.clone(),
        id,
        started,
        finished: false,
    };
    let upgrade = request
        .headers()
        .get("upgrade")
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"));
    let incoming_upgrade = upgrade.then(|| hyper::upgrade::on(&mut request));
    let method = request.method().clone();
    let head = method == hyper::Method::HEAD;
    let headers = clean_headers(request.headers(), upgrade);
    let empty = request.body().is_end_stream();
    let capture = service.captures.clone();
    let stream = request.into_body().into_data_stream().map(move |chunk| {
        if let Ok(bytes) = &chunk {
            capture.update(id, |r| r.request.append(bytes));
        }
        chunk
    });
    let forwarded_body = if empty {
        reqwest::Body::from(Bytes::new())
    } else {
        reqwest::Body::wrap_stream(stream)
    };
    let result = service
        .client
        .request(method, url)
        .headers(headers)
        .body(forwarded_body)
        .send()
        .await;
    match result {
        Err(error) => {
            transfer.finish(Some(error.to_string()), "失败");
            Ok(failure(
                StatusCode::BAD_GATEWAY,
                format!("代理转发失败：{error}"),
            ))
        }
        Ok(response) => {
            let status = response.status();
            service.captures.update(id, |r| {
                r.status = Some(status.as_u16());
                r.response_headers = header_pairs(response.headers());
                r.state = "传输中".into();
                r.elapsed_ms = started.elapsed().as_millis();
            });
            let mut outgoing = Response::builder().status(status);
            *outgoing.headers_mut().unwrap() = clean_headers(
                response.headers(),
                upgrade && status == StatusCode::SWITCHING_PROTOCOLS,
            );
            if status == StatusCode::SWITCHING_PROTOCOLS {
                if let Some(incoming) = incoming_upgrade {
                    let Ok(permit) = service.limits.clone().try_acquire_owned() else {
                        transfer.finish(Some("连接数已达到上限".into()), "失败");
                        return Ok(failure(StatusCode::SERVICE_UNAVAILABLE, "连接数已达到上限"));
                    };
                    let mut cancel = service.cancel.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        let operation = async {
                            let mut incoming = TokioIo::new(incoming.await?);
                            let mut outgoing = response.upgrade().await?;
                            tokio::io::copy_bidirectional(&mut incoming, &mut outgoing).await?;
                            Ok::<_, Error>(())
                        };
                        let result = tokio::select! { result = operation => result, _ = cancel.changed() => Err("代理服务已停止".into()) };
                        let state = if result.is_ok() {
                            "已关闭"
                        } else {
                            "失败"
                        };
                        transfer.finish(result.err().map(|e| e.to_string()), state);
                    });
                }
                return Ok(outgoing.body(body(Bytes::new())).unwrap());
            }
            if head
                || status == StatusCode::NO_CONTENT
                || status == StatusCode::NOT_MODIFIED
                || response.content_length() == Some(0)
            {
                transfer.finish(None, "完成");
                return Ok(outgoing.body(body(Bytes::new())).unwrap());
            }
            let expected = response.content_length();
            let source = response.bytes_stream();
            let stream = futures_util::stream::unfold(
                (source, transfer, false),
                move |(mut source, mut transfer, done)| async move {
                    if done {
                        return None;
                    }
                    match source.next().await {
                        Some(Ok(bytes)) => {
                            let mut complete = false;
                            transfer.store.update(id, |r| {
                                r.response.append(&bytes);
                                r.elapsed_ms = started.elapsed().as_millis();
                                complete =
                                    expected.is_some_and(|length| r.response.bytes >= length);
                            });
                            if complete {
                                transfer.finish(None, "完成");
                            }
                            Some((
                                Ok::<_, Error>(Frame::data(bytes)),
                                (source, transfer, false),
                            ))
                        }
                        Some(Err(error)) => {
                            transfer.finish(Some(error.to_string()), "失败");
                            Some((Err(error.into()), (source, transfer, true)))
                        }
                        None => {
                            transfer.finish(None, "完成");
                            None
                        }
                    }
                },
            );
            Ok(outgoing
                .body(StreamBody::new(stream).boxed_unsync())
                .unwrap())
        }
    }
}
fn connect(mut request: Request<Incoming>, service: Service, peer: SocketAddr) -> Response<Body> {
    let Some(authority) = request.uri().authority().filter(|a| a.port_u16().is_some()) else {
        return failure(StatusCode::BAD_REQUEST, "CONNECT 需要 host:port");
    };
    let Ok(target) = reqwest::Url::parse(&format!("https://{authority}/")) else {
        return failure(StatusCode::BAD_REQUEST, "CONNECT 地址无效");
    };
    if is_self(&target, service.address) {
        return failure(StatusCode::LOOP_DETECTED, "不能连接当前代理自身");
    }
    let Some(ca) = &service.ca else {
        return failure(StatusCode::BAD_GATEWAY, "请先生成或导入 HTTPS 抓包 CA 证书");
    };
    let tls = match ca.tls_for(target.host_str().unwrap()) {
        Ok(config) => tokio_rustls::TlsAcceptor::from(config),
        Err(error) => return failure(StatusCode::BAD_GATEWAY, error),
    };
    let Ok(permit) = service.limits.clone().try_acquire_owned() else {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "连接数已达到上限");
    };
    let incoming = hyper::upgrade::on(&mut request);
    let mut cancel = service.cancel.clone();
    tokio::spawn(async move {
        let _permit = permit;
        let operation = async {
            let incoming = incoming.await.map_err(|e| e.to_string())?;
            let socket =
                tokio::time::timeout(Duration::from_secs(15), tls.accept(TokioIo::new(incoming)))
                    .await
                    .map_err(|_| "TLS 握手超时".to_owned())?
                    .map_err(|e| e.to_string())?;
            serve(socket, service.clone(), peer, Some(target.clone())).await;
            Ok::<_, String>(())
        };
        let result =
            tokio::select! { result = operation => result, _ = cancel.changed() => Ok(()) };
        if let Err(error) = result {
            let id =
                service
                    .captures
                    .start("CONNECT", target.as_str(), peer, &hyper::HeaderMap::new());
            service.captures.update(id, |r| {
                r.state = "失败".into();
                r.error = Some(error);
            });
        }
    });
    Response::new(body(Bytes::new()))
}
#[cfg(test)]
mod tests;
