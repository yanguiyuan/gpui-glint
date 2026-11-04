use gpui::{Context, Entity, Window, div, prelude::*, px, relative, rgb};
use gpui_glint::{
    components::{
        Button, ButtonKind, ButtonSize, Switch, plugin_card_details, plugin_details_back,
        plugin_row, plugin_summary,
    },
    theme::ThemePreset,
};
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    os::windows::{io::AsRawHandle, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    },
};

const MAX_ARCHIVE: u64 = 250 * 1024 * 1024;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const POLL: Duration = Duration::from_millis(250);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    adapter_api: u32,
    id: String,
    name: String,
    description: String,
    permission: String,
    version: String,
    url: String,
    sha256: String,
    archive_bytes: u64,
    archive_root: String,
    executable: String,
    status_path: String,
    token_header: String,
    busy_body: Value,
    busy_response: serde_json::Map<String, Value>,
}

struct Adapter {
    manifest: Manifest,
    bootstrap: Vec<u8>,
}

impl Adapter {
    fn load(bundled: (&[u8], &[u8])) -> Result<Self> {
        let (manifest, bootstrap) =
            if let Some(path) = std::env::var_os("GLINT_DESKTOP_ADAPTER_DIR") {
                let path = PathBuf::from(path);
                if !path.is_absolute() {
                    return Err("GLINT_DESKTOP_ADAPTER_DIR 必须为绝对路径".into());
                }
                (
                    fs::read(path.join("adapter.json"))?,
                    fs::read(path.join("bootstrap.cjs"))?,
                )
            } else {
                (bundled.0.to_vec(), bundled.1.to_vec())
            };
        let manifest: Manifest = serde_json::from_slice(&manifest)?;
        let safe_name = |value: &str| {
            !value.is_empty()
                && value.as_bytes()[0].is_ascii_alphanumeric()
                && !value.ends_with('.')
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        };
        if ![
            &manifest.id,
            &manifest.version,
            &manifest.archive_root,
            &manifest.executable,
        ]
        .into_iter()
        .all(|value| safe_name(value))
            || !manifest.executable.ends_with(".exe")
            || manifest.adapter_api != 1
            || manifest.permission != "managed-process"
            || manifest.archive_bytes == 0
            || manifest.archive_bytes > MAX_ARCHIVE
            || manifest.sha256.len() != 64
            || !manifest.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || reqwest::Url::parse(&manifest.url)?.scheme() != "https"
            || !manifest.status_path.starts_with('/')
            || reqwest::header::HeaderName::from_bytes(manifest.token_header.as_bytes()).is_err()
        {
            return Err("无效的桌面适配插件清单".into());
        }
        Ok(Self {
            manifest,
            bootstrap,
        })
    }
}

#[derive(Clone, Copy)]
enum Action {
    Install,
    Start,
    Stop,
    Uninstall,
}

#[derive(Default)]
struct Status {
    installed: bool,
    running: bool,
    message: String,
    pending: bool,
    show_message: bool,
    download_fraction: Option<f32>,
}

pub(super) struct Panel {
    agent: Entity<glint_agent::AgentWorkspace>,
    commands: mpsc::Sender<Action>,
    status: Status,
    title: String,
    version: String,
    details_open: bool,
    on_details: Box<dyn Fn(bool, &mut gpui::App)>,
    description: String,
    _updates: gpui::Task<()>,
    _agent: gpui::Subscription,
}

impl Panel {
    pub(super) fn new(
        agent: Entity<glint_agent::AgentWorkspace>,
        bundled: (&[u8], &[u8]),
        on_details: impl Fn(bool, &mut gpui::App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let adapter = Adapter::load(bundled).map_err(|error| error.to_string());
        let (title, version, description) = adapter
            .as_ref()
            .map(|adapter| {
                let manifest = &adapter.manifest;
                (
                    manifest.name.clone(),
                    format!("v{}", manifest.version),
                    manifest.description.clone(),
                )
            })
            .unwrap_or_else(|_| ("桌面适配插件".into(), String::new(), String::new()));
        let (commands, actions) = mpsc::channel();
        let (status, updates) = async_channel::unbounded();
        let busy = Arc::new(AtomicBool::new(agent.read(cx).active_session_busy()));
        let agent_busy = busy.clone();
        let subscription = cx.observe(&agent, move |_, agent, cx| {
            agent_busy.store(agent.read(cx).active_session_busy(), Ordering::Relaxed);
        });
        std::thread::spawn(move || {
            let result = match adapter {
                Ok(adapter) => {
                    supervise(actions, &status, busy, &adapter).map_err(|error| error.to_string())
                }
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                let _ = status.try_send(Status {
                    message: error,
                    pending: true,
                    ..Default::default()
                });
            }
        });
        let updates = cx.spawn(async move |this, cx| {
            while let Ok(status) = updates.recv().await {
                if this
                    .update(cx, |panel, cx| {
                        panel.status = status;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            agent,
            title,
            version,
            details_open: false,
            on_details: Box::new(on_details),
            description,
            commands,
            status: Status {
                pending: true,
                ..Default::default()
            },
            _updates: updates,
            _agent: subscription,
        }
    }

    fn button(
        &self,
        label: &'static str,
        action: Action,
        theme: ThemePreset,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut button = Button::new(label)
            .theme(theme)
            .kind(ButtonKind::Plain)
            .size(ButtonSize::Mini);
        if self.status.pending {
            button = button.disabled();
        }
        button
            .render()
            .on_click(cx.listener(move |this, _, _, cx| this.command(action, cx)))
    }

    fn command(&mut self, action: Action, cx: &mut Context<Self>) {
        if !self.status.pending {
            self.status.pending = self.commands.send(action).is_ok();
            self.status.show_message = true;
            self.status.message = if self.status.pending {
                match action {
                    Action::Install => "正在连接下载源…",
                    Action::Start => "正在启动桌面插件…",
                    Action::Stop => "正在停止桌面插件…",
                    Action::Uninstall => "正在卸载桌面插件…",
                }
            } else {
                "桌面插件管理进程已关闭"
            }
            .into();
            cx.notify();
        }
    }

    fn set_details(&mut self, open: bool, cx: &mut Context<Self>) {
        self.details_open = open;
        (self.on_details)(open, cx);
        cx.notify();
    }
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.agent.read(cx).host_theme();
        let badges = (!self.version.is_empty())
            .then(|| [self.version.clone().into(), "桌面适配".into()])
            .into_iter()
            .flatten();
        let summary = plugin_summary(self.title.clone(), badges, self.description.clone(), theme);
        let mut toggle = Switch::new("").checked(self.status.running).theme(theme);
        if !self.status.pending {
            let owner = cx.entity().downgrade();
            toggle = toggle.on_change(move |enabled, _, cx| {
                let _ = owner.update(cx, |panel, cx| {
                    panel.command(if enabled { Action::Start } else { Action::Stop }, cx);
                });
            });
        }
        let control = if self.status.installed {
            toggle.render().into_any_element()
        } else {
            div()
                .text_size(px(11.))
                .text_color(rgb(theme.palette().text_muted))
                .child("未安装")
                .into_any_element()
        };
        let body = if self.details_open {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div().flex().child(
                        plugin_details_back(theme)
                            .on_click(cx.listener(|this, _, _, cx| this.set_details(false, cx))),
                    ),
                )
                .child(plugin_row(summary, control))
                .child(
                    plugin_card_details(theme)
                        .child(self.description.clone())
                        .child("启用时会启动受管进程。"),
                )
                .when(!self.status.running, |body| {
                    body.child(div().flex().child(if self.status.installed {
                        self.button("卸载", Action::Uninstall, theme, cx)
                            .into_any_element()
                    } else {
                        self.button("安装", Action::Install, theme, cx)
                            .into_any_element()
                    }))
                })
        } else {
            div().flex().flex_col().gap_2().child(plugin_row(
                summary
                    .id("desktop-adapter-row")
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(theme.palette().panel)))
                    .on_click(cx.listener(|this, _, _, cx| this.set_details(true, cx))),
                control,
            ))
        };
        body.when_some(self.status.download_fraction, |card, fraction| {
            card.child(
                div()
                    .w_full()
                    .h(px(6.))
                    .rounded(px(3.))
                    .overflow_hidden()
                    .bg(rgb(theme.palette().border))
                    .child(
                        div()
                            .w(relative(fraction))
                            .h_full()
                            .bg(rgb(theme.palette().accent)),
                    ),
            )
        })
        .when(
            (self.status.show_message || self.status.pending || self.details_open)
                && !self.status.message.is_empty(),
            |card| card.child(div().text_size(px(12.)).child(self.status.message.clone())),
        )
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn supervise(
    actions: mpsc::Receiver<Action>,
    status: &async_channel::Sender<Status>,
    busy: Arc<AtomicBool>,
    adapter: &Adapter,
) -> Result<()> {
    let manifest = &adapter.manifest;
    let root = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("无法定位本机应用数据目录")?)
        .join("Glint Workbench")
        .join(&manifest.id);
    let install_dir = root.join(&manifest.version);
    let exe = install_dir.join(&manifest.executable);
    let profile = root.join("profile");
    let local = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()?;
    let mut process: Option<RunningAdapter> = None;
    let report = |process: &Option<RunningAdapter>, message: String, failed: bool| {
        let _ = status.try_send(Status {
            installed: exe.is_file(),
            running: process.is_some(),
            show_message: failed
                || process
                    .as_ref()
                    .is_some_and(|process| process.last_busy.is_none()),
            message,
            ..Default::default()
        });
    };
    report(&process, String::new(), false);
    loop {
        match actions.recv_timeout(POLL) {
            Ok(action) => {
                let result = match action {
                    Action::Install => install(
                        &root,
                        &install_dir,
                        adapter,
                        |message, download_fraction| {
                            let _ = status.try_send(Status {
                                pending: true,
                                message,
                                download_fraction,
                                ..Default::default()
                            });
                        },
                    )
                    .map(|()| "已安装"),
                    Action::Start if process.is_none() => {
                        RunningAdapter::start(&exe, &profile, adapter).map(|running| {
                            process = Some(running);
                            "正在连接桌面插件…"
                        })
                    }
                    Action::Start => Ok("桌面插件已启动"),
                    Action::Stop => {
                        process.take();
                        Ok("桌面插件已停用")
                    }
                    Action::Uninstall => {
                        process.take();
                        fs::remove_dir_all(&install_dir)
                            .map(|()| "已卸载，保留桌面插件配置")
                            .map_err(Into::into)
                    }
                };
                let failed = result.is_err();
                report(
                    &process,
                    result
                        .map(str::to_owned)
                        .unwrap_or_else(|error| error.to_string()),
                    failed,
                );
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
        if let Some(running) = process.as_mut() {
            let connected = running.last_busy.is_some();
            match running.sync(&local, manifest, busy.load(Ordering::Relaxed)) {
                Ok(()) if !connected && running.last_busy.is_some() => {
                    report(
                        &process,
                        "桌面插件已启动，正在跟随 Agent 会话状态".into(),
                        false,
                    );
                }
                Ok(()) => {}
                Err(error) => {
                    process.take();
                    report(&process, error.to_string(), true);
                }
            }
        }
    }
}

fn install(
    root: &Path,
    destination: &Path,
    adapter: &Adapter,
    progress: impl Fn(String, Option<f32>),
) -> Result<()> {
    let manifest = &adapter.manifest;
    if destination.join(&manifest.executable).is_file() {
        return Ok(());
    }
    fs::create_dir_all(root)?;
    let stage = tempfile::tempdir_in(root)?;
    let archive = stage.path().join("process.zip");
    let mut response = Client::builder()
        .timeout(Duration::from_secs(20 * 60))
        .build()?
        .get(&manifest.url)
        .send()?
        .error_for_status()?
        .take(MAX_ARCHIVE + 1);
    let mut output = File::create(&archive)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut downloaded = 0;
    let show_download = |downloaded: u64| {
        let fraction =
            downloaded.min(manifest.archive_bytes) as f32 / manifest.archive_bytes as f32;
        let mib = 1024. * 1024.;
        progress(
            format!(
                "正在下载桌面插件 · {:.1} / {:.1} MiB · {}%",
                downloaded as f64 / mib,
                manifest.archive_bytes as f64 / mib,
                downloaded.min(manifest.archive_bytes) * 100 / manifest.archive_bytes,
            ),
            Some(fraction),
        );
    };
    show_download(downloaded);
    let mut last_update = Instant::now();
    loop {
        let count = response.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        output.write_all(&buffer[..count])?;
        downloaded += count as u64;
        if last_update.elapsed() >= POLL {
            show_download(downloaded);
            last_update = Instant::now();
        }
    }
    progress("下载结束，正在校验安装包…".into(), None);
    drop(output);
    if downloaded != manifest.archive_bytes
        || format!("{:x}", hash.finalize()) != manifest.sha256.to_ascii_lowercase()
    {
        return Err("桌面插件下载包 SHA-256 校验失败".into());
    }
    progress("校验通过，正在解压并安装桌面插件…".into(), None);
    let result = Command::new(
        std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .ok_or("无法定位 Windows 解压工具")?
            .join("System32/tar.exe"),
    )
    .creation_flags(CREATE_NO_WINDOW)
    .arg("-xf")
    .arg(&archive)
    .arg("-C")
    .arg(stage.path())
    .output()?;
    if !result.status.success() {
        return Err(format!(
            "无法解压桌面插件：{}",
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }
    let package = stage.path().join(&manifest.archive_root);
    if !package.join(&manifest.executable).is_file() {
        return Err("桌面插件安装包缺少可执行文件".into());
    }
    let resources = package.join("resources");
    let bootstrap = resources.join("app");
    fs::rename(resources.join("app.asar"), resources.join("upstream.asar"))?;
    fs::create_dir(&bootstrap)?;
    fs::write(
        bootstrap.join("package.json"),
        r#"{"name":"glint-desktop-adapter","main":"main.cjs"}"#,
    )?;
    fs::write(bootstrap.join("main.cjs"), &adapter.bootstrap)?;
    fs::rename(package, destination)?;
    Ok(())
}

// Windows closes the job even if Workbench crashes, including Electron's subprocesses.
struct Job(HANDLE);

impl Job {
    fn new() -> io::Result<Self> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self(handle);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

struct RunningAdapter {
    job: Option<Job>,
    child: Child,
    runtime: tempfile::TempDir,
    deadline: Instant,
    endpoint: Option<(u16, String)>,
    last_busy: Option<bool>,
}

impl RunningAdapter {
    fn start(exe: &Path, profile: &Path, adapter: &Adapter) -> Result<Self> {
        let installation = exe.parent().ok_or("无效的安装路径")?;
        fs::write(
            installation.join("resources/app/main.cjs"),
            &adapter.bootstrap,
        )?;
        let runtime = tempfile::tempdir_in(
            exe.parent()
                .and_then(Path::parent)
                .ok_or("无效的安装路径")?,
        )?;
        let job = Job::new()?;
        let mut child = Command::new(exe)
            .env("GLINT_ADAPTER_PROFILE", profile)
            .env("GLINT_ADAPTER_RUNTIME", runtime.path())
            .env_remove("ELECTRON_RUN_AS_NODE")
            .env_remove("NODE_OPTIONS")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        if unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle().cast()) } == 0 {
            let error = io::Error::last_os_error();
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        Ok(Self {
            job: Some(job),
            child,
            runtime,
            deadline: Instant::now() + Duration::from_secs(20),
            endpoint: None,
            last_busy: None,
        })
    }

    fn sync(&mut self, client: &Client, manifest: &Manifest, busy: bool) -> Result<()> {
        if let Some(exit) = self.child.try_wait()? {
            return Err(format!("桌面插件已退出（{exit}）").into());
        }
        if self.endpoint.is_none() {
            let path = self.runtime.path().join("endpoint.json");
            match fs::read(&path) {
                Ok(bytes) => {
                    let endpoint: Value = serde_json::from_slice(&bytes)?;
                    let port = endpoint["port"]
                        .as_u64()
                        .and_then(|p| u16::try_from(p).ok())
                        .filter(|p| *p != 0);
                    let token = endpoint["token"].as_str().filter(|token| {
                        !token.is_empty()
                            && token.len() <= 128
                            && token
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                    });
                    let (Some(port), Some(token)) = (port, token) else {
                        return Err("桌面插件返回的连接信息无效".into());
                    };
                    if endpoint["pid"].as_u64() != Some(u64::from(self.child.id())) {
                        return Err("桌面插件连接信息与受管进程不匹配".into());
                    }
                    self.endpoint = Some((port, token.into()));
                    fs::remove_file(path)?;
                }
                Err(error)
                    if error.kind() == io::ErrorKind::NotFound
                        && Instant::now() < self.deadline =>
                {
                    return Ok(());
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    return Err("桌面插件启动超时".into());
                }
                Err(error) => return Err(error.into()),
            }
        }
        if self.last_busy != Some(busy) {
            let (port, token) = self.endpoint.as_ref().expect("endpoint initialized");
            let url = format!("http://127.0.0.1:{port}{}", manifest.status_path);
            let request = if busy {
                client.post(url).json(&manifest.busy_body)
            } else {
                client.delete(url)
            };
            let response = request
                .header(&manifest.token_header, token)
                .send()?
                .error_for_status()?;
            if busy && !manifest.busy_response.is_empty() {
                let body: Value = response.json()?;
                if manifest
                    .busy_response
                    .iter()
                    .any(|(key, value)| body.get(key) != Some(value))
                {
                    return Err("适配插件拒绝同步 Agent 工作状态".into());
                }
            }
            self.last_busy = Some(busy);
        }
        Ok(())
    }
}

impl Drop for RunningAdapter {
    fn drop(&mut self) {
        drop(self.job.take());
        let _ = self.child.wait();
    }
}
