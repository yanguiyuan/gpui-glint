//! Optional system-backed video surface. Controls live inside the native view so
//! they share its stacking and focus behavior. No bundled codec or ffmpeg process.
use super::{WebView, wry};
use crate::theme::ThemePreset;
use gpui::{
    AppContext, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, Styled, Task,
    Window, div,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use wry::{
    http::{Request, Response},
    raw_window_handle::HasWindowHandle,
};

const MAX_CHUNK: u64 = 2 * 1024 * 1024;
const HTML: &str = include_str!("video_player/player.html");

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoSource {
    File(PathBuf),
    Url(String),
}
impl VideoSource {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File(path.into())
    }
    pub fn url(url: impl Into<String>) -> Self {
        Self::Url(url.into())
    }
}

#[derive(Clone, Debug)]
pub struct VideoState {
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub muted: bool,
    pub playback_rate: f64,
    pub ended: bool,
    pub error: Option<String>,
}
impl Default for VideoState {
    fn default() -> Self {
        Self {
            playing: false,
            position: 0.,
            duration: 0.,
            volume: 1.,
            muted: false,
            playback_rate: 1.,
            ended: false,
            error: None,
        }
    }
}
#[derive(Clone, Debug)]
pub enum VideoPlayerEvent {
    Loaded {
        duration: f64,
        width: u32,
        height: u32,
    },
    StateChanged(VideoState),
    Progress {
        position: f64,
        duration: f64,
    },
    Ended,
    Error(String),
    FullscreenChanged,
}

#[derive(Default)]
struct LocalSource {
    generation: u64,
    path: Option<PathBuf>,
}
#[derive(Deserialize)]
struct Message {
    kind: String,
    generation: u64,
    #[serde(default)]
    position: f64,
    #[serde(default)]
    duration: f64,
    #[serde(default)]
    playing: bool,
    #[serde(default)]
    ended: bool,
    #[serde(default = "one")]
    volume: f64,
    #[serde(default)]
    muted: bool,
    #[serde(default = "one")]
    rate: f64,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    #[serde(default)]
    message: String,
}
fn one() -> f64 {
    1.
}
/// Send the same semantic palette used by GPUI controls to the native surface.
fn theme_config(theme: ThemePreset) -> Value {
    let palette = theme.palette();
    let hex = |color: u32| format!("#{color:06x}");
    json!({
        "scheme": if theme.is_dark() { "dark" } else { "light" },
        "canvas": hex(palette.canvas),
        "panel": hex(palette.panel),
        "surface": hex(palette.surface),
        "subtle": hex(palette.surface_subtle),
        "border": hex(palette.border),
        "text": hex(palette.text),
        "muted": hex(palette.text_muted),
        "accent": hex(palette.accent),
        "tint": hex(palette.accent_tint),
    })
}

fn nonnegative(value: f64) -> f64 {
    if value.is_finite() { value.max(0.) } else { 0. }
}

/// Requires the `video` feature and a real macOS / Windows window. The host must
/// call `set_visible(false, cx)` before removing this entity's layout node.
pub struct VideoPlayer {
    browser: Entity<WebView>,
    local: Arc<Mutex<LocalSource>>,
    source: Option<String>,
    generation: u64,
    title: String,
    ready: bool,
    visible: bool,
    looped: bool,
    theme: ThemePreset,
    state: VideoState,
    pending_seek: Option<f64>,
    _messages: Task<()>,
}
impl EventEmitter<VideoPlayerEvent> for VideoPlayer {}
impl VideoPlayer {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Result<Self, String> {
        let (native, local, receiver) = build_native(window)?;
        Ok(Self::attach(
            native,
            local,
            receiver,
            Window::window_handle(window),
            cx,
        ))
    }
    /// Create an entity while preserving native initialization errors for the host UI.
    pub fn new_entity(window: &mut Window, cx: &mut gpui::App) -> Result<Entity<Self>, String> {
        let (native, local, receiver) = build_native(window)?;
        let owning_window = Window::window_handle(window);
        Ok(cx.new(|cx| Self::attach(native, local, receiver, owning_window, cx)))
    }
    fn attach(
        native: wry::WebView,
        local: Arc<Mutex<LocalSource>>,
        receiver: async_channel::Receiver<Message>,
        owning_window: gpui::AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let browser = cx.new(|cx| WebView::from_native(native, cx));
        let messages = cx.spawn(async move |weak, cx| {
            while let Ok(message) = receiver.recv().await {
                if message.kind == "fullscreen" {
                    let current = weak
                        .read_with(cx, |this, _| this.generation == message.generation)
                        .unwrap_or(false);
                    if current {
                        let _ = owning_window.update(cx, |_, window, _| window.toggle_fullscreen());
                    }
                }
                if weak
                    .update(cx, |this, cx| this.receive(message, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            browser,
            local,
            source: None,
            generation: 0,
            title: String::new(),
            ready: false,
            visible: true,
            looped: false,
            theme: ThemePreset::default(),
            state: VideoState::default(),
            pending_seek: None,
            _messages: messages,
        }
    }
    pub fn state(&self) -> &VideoState {
        &self.state
    }
    /// Replaces the source and resets position. New sources start paused.
    pub fn set_source(
        &mut self,
        source: VideoSource,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let generation = self.generation.wrapping_add(1);
        let (url, path) = match source {
            VideoSource::File(path) => {
                let path = path
                    .canonicalize()
                    .map_err(|e| format!("无法打开视频：{e}"))?;
                if !path.is_file() {
                    return Err("视频路径必须是普通文件".into());
                }
                #[cfg(target_os = "windows")]
                let url = format!("http://glintvideo.localhost/{generation}/video");
                #[cfg(not(target_os = "windows"))]
                let url = format!("glintvideo://localhost/{generation}/video");
                (url, Some(path))
            }
            VideoSource::Url(url) => {
                validate_url(&url)?;
                (url, None)
            }
        };
        *self.local.lock().map_err(|_| "无法更新视频源")? = LocalSource { generation, path };
        self.generation = generation;
        self.source = Some(url);
        self.state.playing = false;
        self.pending_seek = None;
        self.state.position = 0.;
        self.state.duration = 0.;
        self.state.ended = false;
        self.state.error = None;
        self.apply_source(cx)?;
        cx.notify();
        Ok(())
    }
    pub fn clear(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        self.generation = self.generation.wrapping_add(1);
        self.source = None;
        *self.local.lock().map_err(|_| "无法更新视频源")? = LocalSource {
            generation: self.generation,
            path: None,
        };
        self.state.playing = false;
        self.pending_seek = None;
        self.state.position = 0.;
        self.state.duration = 0.;
        self.state.ended = false;
        self.state.error = None;
        self.apply_source(cx)?;
        cx.notify();
        Ok(())
    }
    pub fn set_title(
        &mut self,
        title: impl Into<String>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.title = title.into();
        self.command("title", json!(self.title), cx)
    }
    pub fn play(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        if self.source.is_none() {
            return Err("请先设置视频源".into());
        }
        if !self.visible {
            return Err("播放器已隐藏".into());
        }
        if !self.ready {
            self.state.playing = true;
            return Ok(());
        }
        self.command("play", Value::Null, cx)
    }
    pub fn pause(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        self.state.playing = false;
        self.command("pause", Value::Null, cx)
    }
    pub fn seek(&mut self, seconds: f64, cx: &mut Context<Self>) -> Result<(), String> {
        if !seconds.is_finite() || seconds < 0. {
            return Err("播放位置必须是非负秒数".into());
        }
        if !self.ready {
            self.pending_seek = Some(seconds);
            return Ok(());
        }
        self.command("seek", json!(seconds), cx)
    }
    pub fn set_volume(&mut self, volume: f64, cx: &mut Context<Self>) -> Result<(), String> {
        if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
            return Err("音量范围为 0..1".into());
        }
        self.state.volume = volume;
        self.command("volume", json!(volume), cx)
    }
    pub fn set_muted(&mut self, muted: bool, cx: &mut Context<Self>) -> Result<(), String> {
        self.state.muted = muted;
        self.command("mute", json!(muted), cx)
    }
    pub fn set_playback_rate(&mut self, rate: f64, cx: &mut Context<Self>) -> Result<(), String> {
        if !rate.is_finite() || !(0.25..=4.).contains(&rate) {
            return Err("播放速度范围为 0.25..4".into());
        }
        self.state.playback_rate = rate;
        self.command("rate", json!(rate), cx)
    }
    pub fn set_loop(&mut self, looped: bool, cx: &mut Context<Self>) -> Result<(), String> {
        self.looped = looped;
        self.command("loop", json!(looped), cx)
    }
    pub fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) -> Result<(), String> {
        self.theme = theme;
        self.command("theme", theme_config(theme), cx)
    }
    /// Hiding pauses playback. Showing again does not automatically resume.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) -> Result<(), String> {
        if self.visible == visible {
            return Ok(());
        }
        self.visible = visible;
        if !visible {
            self.state.playing = false;
        }
        self.command("visible", json!(visible), cx)?;
        self.browser
            .update(cx, |view, cx| view.set_visible(visible, cx))?;
        cx.notify();
        Ok(())
    }
    fn command(&self, action: &str, value: Value, cx: &mut Context<Self>) -> Result<(), String> {
        if !self.ready {
            return Ok(());
        }
        let command = json!({"action":action,"value":value});
        self.browser
            .read(cx)
            .raw()
            .evaluate_script(&format!("window.glintVideo.command({command});"))
            .map_err(|e| e.to_string())
    }
    fn apply_source(&self, cx: &mut Context<Self>) -> Result<(), String> {
        if !self.ready {
            return Ok(());
        }
        let config = json!({"generation":self.generation,"source":self.source,"position":self.pending_seek,"title":self.title,"volume":self.state.volume,"muted":self.state.muted,"rate":self.state.playback_rate,"looped":self.looped,"playing":self.state.playing,"visible":self.visible,"theme":theme_config(self.theme)});
        self.browser
            .read(cx)
            .raw()
            .evaluate_script(&format!("window.glintVideo.load({config});"))
            .map_err(|e| e.to_string())
    }
    fn receive(&mut self, message: Message, cx: &mut Context<Self>) {
        if message.kind == "ready" {
            self.ready = true;
            if let Err(error) = self.apply_source(cx) {
                self.state.error = Some(error.clone());
                cx.emit(VideoPlayerEvent::Error(error));
            }
            return;
        }
        if message.generation != self.generation {
            return;
        }
        self.state.position = nonnegative(message.position);
        self.state.duration = nonnegative(message.duration);
        self.state.playing = message.playing;
        self.state.ended = message.ended;
        self.state.volume = nonnegative(message.volume).min(1.);
        self.state.muted = message.muted;
        self.state.playback_rate = if message.rate.is_finite() {
            message.rate.clamp(0.25, 4.)
        } else {
            1.
        };
        match message.kind.as_str() {
            "loaded" => {
                self.pending_seek = None;
                self.state.error = None;
                cx.emit(VideoPlayerEvent::Loaded {
                    duration: self.state.duration,
                    width: message.width,
                    height: message.height,
                });
            }
            "state" => {
                if self.state.playing {
                    self.state.ended = false;
                    self.state.error = None;
                }
                cx.emit(VideoPlayerEvent::StateChanged(self.state.clone()));
            }
            "progress" => cx.emit(VideoPlayerEvent::Progress {
                position: self.state.position,
                duration: self.state.duration,
            }),
            "ended" => {
                self.state.playing = false;
                self.state.ended = true;
                cx.emit(VideoPlayerEvent::Ended);
            }
            "error" => {
                self.state.playing = false;
                self.state.error = Some(message.message.clone());
                cx.emit(VideoPlayerEvent::Error(message.message));
            }
            "fullscreen" => cx.emit(VideoPlayerEvent::FullscreenChanged),
            _ => return,
        }
        cx.notify();
    }
}
impl Render for VideoPlayer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(self.browser.clone())
    }
}

fn build_native(
    window: &Window,
) -> Result<
    (
        wry::WebView,
        Arc<Mutex<LocalSource>>,
        async_channel::Receiver<Message>,
    ),
    String,
> {
    let local = Arc::new(Mutex::new(LocalSource::default()));
    let (sender, receiver) = async_channel::bounded::<Message>(128);
    // One file reader per player; the queue and every response are bounded.
    let (requests, jobs) =
        std::sync::mpsc::sync_channel::<(Request<Vec<u8>>, wry::RequestAsyncResponder)>(8);
    let files = local.clone();
    std::thread::Builder::new()
        .name("glint-video-reader".into())
        .spawn(move || {
            for (request, responder) in jobs {
                responder.respond(serve_local(&request, &files));
            }
        })
        .map_err(|e| e.to_string())?;
    let builder = wry::WebViewBuilder::new()
        .with_html(HTML)
        .with_incognito(true)
        .with_background_color((16, 19, 24, 255))
        .with_autoplay(true)
        .with_navigation_handler(|url| url == "about:blank" || url.starts_with("about:blank#"))
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_download_started_handler(|_, _| false)
        .with_asynchronous_custom_protocol("glintvideo".into(), move |_, request, responder| {
            if let Err(error) = requests.try_send((request, responder)) {
                let (_, responder) = match error {
                    std::sync::mpsc::TrySendError::Full(job)
                    | std::sync::mpsc::TrySendError::Disconnected(job) => job,
                };
                responder.respond(response(503, b"Video reader busy".to_vec()));
            }
        })
        .with_ipc_handler(move |request| {
            if request.body().len() <= 16 * 1024 {
                if let Ok(message) = serde_json::from_str(request.body()) {
                    let _ = sender.try_send(message);
                }
            }
        });
    let raw = HasWindowHandle::window_handle(&*window).map_err(|e| e.to_string())?;
    let native = builder
        .with_visible(false)
        .build_as_child(&raw)
        .map_err(|e| e.to_string())?;
    Ok((native, local, receiver))
}

fn validate_url(url: &str) -> Result<(), String> {
    let uri: wry::http::Uri = url.parse().map_err(|_| "视频地址无效")?;
    if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.host().is_none_or(str::is_empty) {
        return Err("网络视频仅支持 HTTP / HTTPS 地址".into());
    }
    Ok(())
}
fn response(status: u16, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Access-Control-Allow-Origin", "*")
        .body(body)
        .unwrap()
}
fn byte_range(header: Option<&str>, len: u64) -> Result<(u64, u64), ()> {
    if len == 0 {
        return Err(());
    }
    let (start, end) = match header {
        None => (0, len - 1),
        Some(header) => {
            let range = header.strip_prefix("bytes=").ok_or(())?;
            let (start, end) = range.split_once('-').ok_or(())?;
            if start.is_empty() {
                let suffix: u64 = end.parse().map_err(|_| ())?;
                if suffix == 0 {
                    return Err(());
                }
                (len.saturating_sub(suffix), len - 1)
            } else {
                let start: u64 = start.parse().map_err(|_| ())?;
                let end = if end.is_empty() {
                    len - 1
                } else {
                    end.parse::<u64>().map_err(|_| ())?.min(len - 1)
                };
                (start, end)
            }
        }
    };
    if start >= len || end < start {
        return Err(());
    }
    Ok((start, end.min(start.saturating_add(MAX_CHUNK - 1))))
}
fn serve_local(request: &Request<Vec<u8>>, local: &Mutex<LocalSource>) -> Response<Vec<u8>> {
    if !matches!(request.method().as_str(), "GET" | "HEAD") {
        return response(405, vec![]);
    }
    let (generation, path) = match local.lock() {
        Ok(source) => (source.generation, source.path.clone()),
        Err(_) => return response(500, vec![]),
    };
    if request.uri().path() != format!("/{generation}/video") {
        return response(404, vec![]);
    }
    let Some(path) = path else {
        return response(404, vec![]);
    };
    match read_file_response(request, &path) {
        Ok(response) => response,
        Err(_) => response(404, vec![]),
    }
}
fn read_file_response(
    request: &Request<Vec<u8>>,
    path: &Path,
) -> std::io::Result<Response<Vec<u8>>> {
    let mut file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Ok(response(404, vec![]));
    }
    let len = metadata.len();
    let mime = match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "ogv" | "ogg" => "video/ogg",
        _ => "application/octet-stream",
    };
    let range = request
        .headers()
        .get("range")
        .and_then(|header| header.to_str().ok());
    let mut builder = Response::builder()
        .header("Content-Type", mime)
        .header("Accept-Ranges", "bytes")
        .header("Access-Control-Allow-Origin", "*");
    if request.method() == "HEAD" {
        return Ok(builder.header("Content-Length", len).body(vec![]).unwrap());
    }
    let Ok((start, end)) = byte_range(range, len) else {
        return Ok(builder
            .status(416)
            .header("Content-Range", format!("bytes */{len}"))
            .body(vec![])
            .unwrap());
    };
    if range.is_some() || len > MAX_CHUNK {
        builder = builder
            .status(206)
            .header("Content-Range", format!("bytes {start}-{end}/{len}"));
    }
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0; (end - start + 1) as usize];
    file.read_exact(&mut bytes)?;
    Ok(builder
        .header("Content-Length", bytes.len())
        .body(bytes)
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_controls_receive_every_theme_palette() {
        for theme in ThemePreset::ALL {
            let config = theme_config(theme);
            assert_eq!(
                config["scheme"],
                if theme.is_dark() { "dark" } else { "light" }
            );
            assert_eq!(config["panel"], format!("#{:06x}", theme.palette().panel));
            assert_eq!(config["text"], format!("#{:06x}", theme.palette().text));
            for key in [
                "canvas", "panel", "surface", "subtle", "border", "text", "muted", "accent", "tint",
            ] {
                assert_eq!(config[key].as_str().unwrap().len(), 7);
                assert!(HTML.contains(&format!("--{key}")), "CSS must use {key}");
            }
        }
        assert!(HTML.contains("case 'theme':applyTheme(config.value)"));
    }

    #[test]
    fn ranges_are_bounded_and_reject_invalid_requests() {
        assert_eq!(byte_range(Some("bytes=0-1"), 100), Ok((0, 1)));
        assert_eq!(byte_range(Some("bytes=20-"), 100), Ok((20, 99)));
        assert_eq!(byte_range(Some("bytes=-8"), 100), Ok((92, 99)));
        assert_eq!(byte_range(Some("bytes=90-999"), 100), Ok((90, 99)));
        assert_eq!(
            byte_range(Some("bytes=0-"), MAX_CHUNK * 20),
            Ok((0, MAX_CHUNK - 1))
        );
        for range in [
            "bytes=100-",
            "bytes=20-10",
            "bytes=-0",
            "bytes=1-2,4-5",
            "other=0-1",
        ] {
            assert!(byte_range(Some(range), 100).is_err());
        }
        assert!(byte_range(None, 0).is_err());
    }
    #[test]
    fn local_protocol_exposes_only_current_file_and_honors_seek_and_head() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("带空格 video.mp4");
        std::fs::write(&path, b"0123456789").unwrap();
        let local = Mutex::new(LocalSource {
            generation: 7,
            path: Some(path.clone()),
        });
        let get = |uri: &str, range: Option<&str>| {
            let mut request = Request::builder().uri(uri);
            if let Some(range) = range {
                request = request.header("Range", range);
            }
            serve_local(&request.body(vec![]).unwrap(), &local)
        };
        let page = get("glintvideo://localhost/7/video", Some("bytes=4-7"));
        assert_eq!(page.status(), 206);
        assert_eq!(page.headers()["content-range"], "bytes 4-7/10");
        assert_eq!(page.body(), b"4567");
        assert_eq!(get("glintvideo://localhost/6/video", None).status(), 404);
        assert_eq!(get("glintvideo://localhost/etc/passwd", None).status(), 404);
        let invalid = get("glintvideo://localhost/7/video", Some("bytes=99-"));
        assert_eq!(invalid.status(), 416);
        assert_eq!(invalid.headers()["content-range"], "bytes */10");
        let head = serve_local(
            &Request::builder()
                .method("HEAD")
                .uri("glintvideo://localhost/7/video")
                .body(vec![])
                .unwrap(),
            &local,
        );
        assert_eq!(head.headers()["content-length"], "10");
        assert!(head.body().is_empty());
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(MAX_CHUNK * 4).unwrap();
        let bounded = get("glintvideo://localhost/7/video", Some("bytes=0-"));
        assert_eq!(bounded.body().len(), MAX_CHUNK as usize);
        local.lock().unwrap().generation = 8;
        assert_eq!(get("glintvideo://localhost/7/video", None).status(), 404);
    }
    #[test]
    fn remote_sources_only_accept_network_urls_and_html_is_static() {
        assert!(validate_url("https://example.com/movie.mp4?token=123").is_ok());
        for url in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:video/mp4,abc",
            "https:///",
            "movie.mp4",
        ] {
            assert!(validate_url(url).is_err());
        }
        assert!(!HTML.contains("https://")); // The player has no remote script/font dependencies.
    }
}
