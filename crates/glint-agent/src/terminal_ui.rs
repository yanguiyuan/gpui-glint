//! Project-scoped native PTY terminals. Hiding a panel keeps its shell alive.
use super::*;
use alacritty_terminal::{
    event::{Event, EventListener, WindowSize},
    event_loop::{EventLoop, EventLoopSender, Msg},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point as TermPoint, Side},
    selection::{Selection, SelectionType},
    sync::FairMutex,
    term::{Term, TermMode, cell::Flags},
    tty,
    vte::ansi::{Color, Rgb},
};
use gpui::{
    ElementInputHandler, EntityInputHandler, FocusHandle, Focusable, TextRun, UTF16Selection,
};
use std::{
    borrow::Cow,
    collections::HashMap,
    ops::Range,
    path::Path,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

const CELL_W: f32 = 8.;
const CELL_H: f32 = 19.;
const FONT_SIZE: f32 = 13.;

fn fixed_pitch(widths: &[f32]) -> bool {
    widths.first().is_some_and(|width| width.is_finite() && *width > 0.
        && widths.iter().all(|other| other.is_finite() && (*other - *width).abs() < 0.02))
}

fn terminal_font(text: &gpui::TextSystem, configured: &str) -> (gpui::Font, f32) {
    // Generic CSS family names can resolve to a proportional UI fallback in
    // native font backends. Pick an installed, verified fixed-pitch family.
    let fallback: &[&str] = if cfg!(target_os = "macos") {
        &["Menlo", "SF Mono", "Monaco", "Courier New"]
    } else if cfg!(windows) {
        &["Consolas", "Cascadia Mono", "Lucida Console", "Courier New"]
    } else {
        &["DejaVu Sans Mono", "Liberation Mono", "Noto Sans Mono", "Monospace"]
    };
    let installed = text.all_font_names();
    for family in std::iter::once(configured).chain(fallback.iter().copied()) {
        if !installed.iter().any(|name| name.eq_ignore_ascii_case(family)) { continue; }
        let font = gpui::font(family.to_owned());
        let id = text.resolve_font(&font);
        let widths: Option<Vec<f32>> = ['0', 'i', 'W', 'm', '.', ' '].into_iter()
            .map(|ch| text.advance(id, px(FONT_SIZE), ch).ok().map(|advance| f32::from(advance.width)))
            .collect();
        if let Some(widths) = widths.filter(|widths| fixed_pitch(widths)) {
            return (font, widths[0]);
        }
    }
    let font = gpui::font(fallback[0]);
    let width = text.advance(text.resolve_font(&font), px(FONT_SIZE), '0')
        .map(|advance| f32::from(advance.width)).unwrap_or(CELL_W).max(1.);
    (font, width)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalColors {
    background: u32,
    foreground: u32,
    selection: u32,
    cursor: u32,
    muted: u32,
    ansi: [u32; 16],
}
impl TerminalColors {
    fn for_theme(theme: ThemePreset) -> Self {
        let p = theme.palette();
        let ansi = if theme.is_dark() {
            [
                0x1d2028, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xd0d6df,
                0x747d8c, 0xf08088, 0xb0df8e, 0xf5d88b, 0x80c5ff, 0xd898ee, 0x75d5df, 0xffffff,
            ]
        } else {
            [
                0x24292f, 0xb42318, 0x257235, 0x8a5d00, 0x0969da, 0x8250df, 0x087e8b, 0x6e7781,
                0x57606a, 0xc72c41, 0x287c3c, 0x916900, 0x125bca, 0x9a3da5, 0x007e8a, 0x1f2328,
            ]
        };
        Self {
            background: p.workspace,
            foreground: p.text,
            selection: p.accent_tint,
            cursor: p.accent,
            muted: p.text_muted,
            ansi,
        }
    }
}

gpui::actions!(project_terminal, [Paste, Copy, CopyOrInterrupt]);
struct StatusChanged;
impl gpui::EventEmitter<StatusChanged> for TerminalView {}
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        gpui::KeyBinding::new("ctrl-c", CopyOrInterrupt, Some("ProjectTerminal")),
        gpui::KeyBinding::new("ctrl-v", Paste, Some("ProjectTerminal")),
        gpui::KeyBinding::new("ctrl-shift-v", Paste, Some("ProjectTerminal")),
        gpui::KeyBinding::new("cmd-v", Paste, Some("ProjectTerminal")),
        gpui::KeyBinding::new("ctrl-shift-c", Copy, Some("ProjectTerminal")),
        gpui::KeyBinding::new("cmd-c", Copy, Some("ProjectTerminal")),
    ]);
}

#[derive(Default)]
pub struct State {
    pub open: bool,
    terminals: HashMap<PathBuf, Entity<TerminalView>>,
}

#[cfg(windows)]
fn git_bash_startup(bash: &Path) -> Result<(Vec<String>, PathBuf), String> {
    let base = bash
        .parent()
        .and_then(Path::parent)
        .ok_or("Git Bash 路径无效")?;
    let git = [
        base.join("cmd/git.exe"),
        base.join("bin/git.exe"),
        base.join("mingw64/bin/git.exe"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .ok_or("Git Bash 安装中找不到 Git 可执行文件")?;
    let git = crate::executable::resolve(&git.to_string_lossy(), base);
    let directory =
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("无法定位本机应用数据目录")?)
            .join("Glint/terminal");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    let path = directory.join("git-bash-init.bash");
    let before = crate::atomic_file::current(&path)?;
    let content = include_str!("../assets/git-bash-init.bash");
    if before.as_deref() != Some(content) {
        if let Err(error) = crate::atomic_file::replace(&path, before.as_deref(), content) {
            if crate::atomic_file::current(&path)?.as_deref() != Some(content) {
                return Err(error);
            }
        }
    }
    // Bash accepts DOS paths with forward slashes; strip extended-path prefixes.
    let rc = path
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('\\', "/");
    Ok((
        vec!["--rcfile".into(), rc, "-i".into()],
        git.parent().unwrap().into(),
    ))
}

fn shell_options(root: &Path) -> Result<(tty::Options, String), String> {
    let mut options = tty::Options::default();
    options.drain_on_exit = true;
    options.env.insert("TERM".into(), "xterm-256color".into());
    options.env.insert("COLORTERM".into(), "truecolor".into());
    #[cfg(windows)]
    {
        options.escape_args = true;
    }
    if let Ok(remote) = ssh_project::Remote::from_path(root) {
        let mut args = vec!["-tt".into()];
        if let Some(port) = remote.port {
            args.extend(["-p".into(), port.to_string()]);
        }
        args.extend([
            remote.target,
            format!(
                "cd -- {} && exec \"${{SHELL:-/bin/sh}}\" -l",
                ssh_project::quote(&remote.directory)
            ),
        ]);
        options.shell = Some(tty::Shell::new("ssh".into(), args));
        return Ok((options, "SSH".into()));
    }
    options.working_directory = Some(root.into());
    #[cfg(windows)]
    {
        let git = crate::executable::resolve("git", root);
        let mut candidates = Vec::new();
        if let Some(base) = git.parent().and_then(Path::parent) {
            candidates.extend([base.join("bin/bash.exe"), base.join("usr/bin/bash.exe")]);
            if let Some(parent) = base.parent() {
                candidates.push(parent.join("bin/bash.exe"));
            }
        }
        for name in ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(name) {
                candidates.extend([
                    PathBuf::from(&base).join("Git/bin/bash.exe"),
                    PathBuf::from(base).join("Programs/Git/bin/bash.exe"),
                ]);
            }
        }
        if let Some(bash) = candidates.into_iter().find(|path| path.is_file()) {
            let bash = crate::executable::resolve(&bash.to_string_lossy(), root);
            let (args, git_bin) = git_bash_startup(&bash)?;
            options.shell = Some(tty::Shell::new(bash.to_string_lossy().into_owned(), args));
            options.env.insert(
                "GLINT_NATIVE_GIT_BIN".into(),
                git_bin.to_string_lossy().into_owned(),
            );
            options.env.insert("CHERE_INVOKING".into(), "1".into());
            options.env.insert("MSYSTEM".into(), "MINGW64".into());
            return Ok((options, "Git Bash".into()));
        }
        options.shell = Some(tty::Shell::new(
            "powershell.exe".into(),
            vec!["-NoLogo".into()],
        ));
        return Ok((options, "PowerShell".into()));
    }
    #[cfg(not(windows))]
    {
        Ok((options, "Shell".into()))
    }
}

#[derive(Clone)]
struct Listener {
    wake: async_channel::Sender<()>,
    sender: Arc<OnceLock<EventLoopSender>>,
    exited: Arc<AtomicBool>,
    size: Arc<Mutex<WindowSize>>,
    colors: Arc<Mutex<TerminalColors>>,
}
impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let reply = match event {
            Event::PtyWrite(text) => Some(text),
            Event::ColorRequest(index, format) => {
                let color = ansi_color(index, *self.colors.lock().unwrap());
                Some(format(Rgb {
                    r: (color >> 16) as u8,
                    g: (color >> 8) as u8,
                    b: color as u8,
                }))
            }
            Event::TextAreaSizeRequest(format) => Some(format(*self.size.lock().unwrap())),
            Event::Exit | Event::ChildExit(_) => {
                self.exited.store(true, Ordering::Release);
                None
            }
            // Terminal escape sequences must not read/write the system clipboard.
            _ => None,
        };
        if let (Some(sender), Some(text)) = (self.sender.get(), reply) {
            let _ = sender.send(Msg::Input(text.into_bytes().into()));
        }
        let _ = self.wake.try_send(());
    }
}

struct Size {
    columns: usize,
    rows: usize,
}
impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.columns
    }
}
struct Session {
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    listener: Listener,
    label: String,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
    }
}
impl Session {
    fn start(
        root: &Path,
        theme: ThemePreset,
    ) -> Result<(Self, async_channel::Receiver<()>), String> {
        let (wake, receiver) = async_channel::bounded(1);
        let size = WindowSize {
            num_lines: 28,
            num_cols: 80,
            cell_width: CELL_W as u16,
            cell_height: CELL_H as u16,
        };
        let listener = Listener {
            wake,
            sender: Arc::new(OnceLock::new()),
            exited: Arc::new(AtomicBool::new(false)),
            size: Arc::new(Mutex::new(size)),
            colors: Arc::new(Mutex::new(TerminalColors::for_theme(theme))),
        };
        let (options, label) = shell_options(root)?;
        let pty =
            tty::new(&options, size, 0).map_err(|error| format!("无法启动项目终端：{error}"))?;
        let term = Arc::new(FairMutex::new(Term::new(
            Default::default(),
            &Size {
                columns: 80,
                rows: 28,
            },
            listener.clone(),
        )));
        let event_loop = EventLoop::new(term.clone(), listener.clone(), pty, true, false)
            .map_err(|error| error.to_string())?;
        let sender = event_loop.channel();
        let _ = listener.sender.set(sender.clone());
        let worker = event_loop.spawn();
        // Reap the loop and release its PTY even after the child exits naturally.
        std::thread::spawn(move || {
            let _ = worker.join();
        });
        Ok((
            Self {
                term,
                sender,
                listener,
                label,
            },
            receiver,
        ))
    }
    fn input(&self, bytes: Vec<u8>) {
        let mut term = self.term.lock();
        term.scroll_display(Scroll::Bottom);
        term.selection = None;
        drop(term);
        let _ = self.sender.send(Msg::Input(Cow::Owned(bytes)));
    }
    fn resize(&self, columns: usize, rows: usize, cell_width: f32) {
        let mut size = self.listener.size.lock().unwrap();
        let width = cell_width.round().max(1.) as u16;
        if usize::from(size.num_cols) == columns && usize::from(size.num_lines) == rows && size.cell_width == width {
            return;
        }
        size.cell_width = width;
        size.num_cols = columns as u16;
        size.num_lines = rows as u16;
        let resized = *size;
        // The parser may request the size while holding the terminal lock.
        // Release the size mutex before taking that lock on the UI thread.
        drop(size);
        self.term.lock().resize(Size { columns, rows });
        let _ = self.sender.send(Msg::Resize(resized));
    }
}

fn ansi_color(index: usize, colors: TerminalColors) -> u32 {
    match index {
        0..=15 => colors.ansi[index],
        16..=231 => {
            let i = index - 16;
            let v = |n| if n == 0 { 0 } else { 55 + n * 40 };
            (v(i / 36) << 16 | v(i / 6 % 6) << 8 | v(i % 6)) as u32
        }
        232..=255 => {
            let v = 8 + (index - 232) * 10;
            (v << 16 | v << 8 | v) as u32
        }
        257 => colors.background,
        258 => colors.cursor,
        259..=266 => colors.ansi[index - 259],
        268 => colors.muted,
        _ => colors.foreground,
    }
}
fn color(value: Color, colors: TerminalColors) -> u32 {
    match value {
        Color::Spec(rgb) => u32::from(rgb.r) << 16 | u32::from(rgb.g) << 8 | u32::from(rgb.b),
        Color::Indexed(index) => ansi_color(index as usize, colors),
        Color::Named(name) => ansi_color(name as usize, colors),
    }
}

struct TerminalView {
    font: gpui::Font,
    configured_font: SharedString,
    cell_width: f32,
    theme: ThemePreset,
    session: Option<Session>,
    focus: FocusHandle,
    _task: Option<Task<()>>,
    error: Option<String>,
    preedit: String,
    bounds: gpui::Bounds<gpui::Pixels>,
    dragging: bool,
    exit_notified: bool,
}
impl TerminalView {
    fn new(root: PathBuf, theme: ThemePreset, cx: &mut Context<Self>) -> Self {
        let configured_font = gpui_glint::code_font::family();
        let (font, cell_width) = terminal_font(cx.text_system(), &configured_font);
        let start = cx
            .background_executor()
            .spawn(async move { Session::start(&root, theme) });
        let task = cx.spawn(async move |weak, cx| {
            let receiver = match start.await {
                Ok((session, receiver)) => {
                    if weak
                        .update(cx, |this, cx| {
                            *session.listener.colors.lock().unwrap() =
                                TerminalColors::for_theme(this.theme);
                            this.session = Some(session);
                            cx.emit(StatusChanged);
                            cx.notify();
                        })
                        .is_err()
                    {
                        return;
                    }
                    receiver
                }
                Err(error) => {
                    let _ = weak.update(cx, |this, cx| {
                        this.error = Some(error);
                        cx.emit(StatusChanged);
                        cx.notify();
                    });
                    return;
                }
            };
            while receiver.recv().await.is_ok() {
                if weak
                    .update(cx, |this, cx| {
                        if !this.exit_notified
                            && this
                                .session
                                .as_ref()
                                .is_some_and(|s| s.listener.exited.load(Ordering::Acquire))
                        {
                            this.exit_notified = true;
                            cx.emit(StatusChanged);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            font,
            configured_font,
            cell_width,
            theme,
            session: None,
            focus: cx.focus_handle(),
            _task: Some(task),
            error: None,
            preedit: String::new(),
            bounds: Default::default(),
            dragging: false,
            exit_notified: false,
        }
    }
    fn set_theme(&mut self, theme: ThemePreset, cx: &mut Context<Self>) {
        if self.theme != theme {
            self.theme = theme;
            if let Some(session) = &self.session {
                *session.listener.colors.lock().unwrap() = TerminalColors::for_theme(theme);
            }
            cx.notify();
        }
    }
    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let (Some(session), Some(text)) = (
            &self.session,
            cx.read_from_clipboard().and_then(|item| item.text()),
        ) {
            let text = text.replace('\x1b', "").replace("\r\n", "\n");
            let bracketed = session
                .term
                .lock()
                .mode()
                .contains(TermMode::BRACKETED_PASTE);
            session.input(if bracketed {
                format!("\x1b[200~{text}\x1b[201~").into_bytes()
            } else {
                text.replace('\n', "\r").into_bytes()
            });
            cx.notify();
        }
    }
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self
            .session
            .as_ref()
            .and_then(|s| s.term.lock().selection_to_string())
        {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
    }
    fn copy_or_interrupt(&mut self, _: &CopyOrInterrupt, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(session) = &self.session {
            let selected = session
                .term
                .lock()
                .selection_to_string()
                .filter(|text| !text.is_empty());
            if let Some(text) = selected {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            } else {
                session.input(vec![3]);
            }
        }
    }
    fn key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.preedit.is_empty() {
            cx.propagate();
            return;
        }
        let key = &event.keystroke;
        if (key.modifiers.platform || key.modifiers.control) && key.key == "v" {
            self.paste(&Paste, window, cx);
            cx.stop_propagation();
            return;
        }
        if (key.modifiers.platform || key.modifiers.control && key.modifiers.shift)
            && key.key == "c"
        {
            self.copy(&Copy, window, cx);
            cx.stop_propagation();
            return;
        }
        if key.modifiers.control && !key.modifiers.shift && !key.modifiers.alt && key.key == "c" {
            self.copy_or_interrupt(&CopyOrInterrupt, window, cx);
            cx.stop_propagation();
            return;
        }
        let application = self
            .session
            .as_ref()
            .is_some_and(|session| session.term.lock().mode().contains(TermMode::APP_CURSOR));
        let bytes = key_bytes(
            &key.key,
            key.modifiers.control,
            key.modifiers.alt,
            key.modifiers.shift,
            application,
        );
        if let Some(bytes) = bytes {
            if let Some(session) = &self.session {
                session.input(bytes);
            }
            cx.stop_propagation();
            cx.notify();
        } else {
            cx.propagate();
        }
    }
    fn grid_point(&self, position: gpui::Point<gpui::Pixels>) -> TermPoint {
        let offset = self
            .session
            .as_ref()
            .map_or(0, |s| s.term.lock().grid().display_offset());
        let columns = (f32::from(self.bounds.size.width) / self.cell_width).max(2.) as usize;
        let rows = (f32::from(self.bounds.size.height) / CELL_H).max(1.) as usize;
        let col = ((f32::from(position.x - self.bounds.left()) / self.cell_width).max(0.) as usize)
            .min(columns - 1);
        let row =
            ((f32::from(position.y - self.bounds.top()) / CELL_H).max(0.) as usize).min(rows - 1);
        TermPoint::new(Line(row as i32 - offset as i32), Column(col))
    }
    fn paint(
        &mut self,
        bounds: gpui::Bounds<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let configured = gpui_glint::code_font::family();
        if configured != self.configured_font {
            let (font, cell_width) = terminal_font(window.text_system(), &configured);
            self.font = font;
            self.cell_width = cell_width;
            self.configured_font = configured;
        }
        self.bounds = bounds;
        let colors = TerminalColors::for_theme(self.theme);
        let Some(session) = &self.session else {
            return;
        };
        let cols = ((f32::from(bounds.size.width) / self.cell_width) as usize).clamp(2, 500);
        let rows = ((f32::from(bounds.size.height) / CELL_H) as usize).clamp(1, 300);
        session.resize(cols, rows, self.cell_width);
        let term = session.term.lock();
        let content = term.renderable_content();
        for cell in content.display_iter {
            let row = cell.point.line.0 + content.display_offset as i32;
            if row < 0 || row >= rows as i32 || cell.point.column.0 >= cols {
                continue;
            }
            let origin = point(
                bounds.left() + px(cell.point.column.0 as f32 * self.cell_width),
                bounds.top() + px(row as f32 * CELL_H),
            );
            let selected = content
                .selection
                .is_some_and(|selection| selection.contains(cell.point));
            let (mut fg, mut bg) = (color(cell.fg, colors), color(cell.bg, colors));
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if selected {
                bg = colors.selection;
                fg = colors.foreground;
            }
            if bg != colors.background {
                window.paint_quad(gpui::fill(
                    gpui::Bounds::new(origin, size(px(self.cell_width), px(CELL_H))),
                    rgb(bg),
                ));
            }
            if cell.flags.intersects(
                Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER | Flags::HIDDEN,
            ) || cell.c == ' ' && cell.zerowidth().is_none()
            {
                continue;
            }
            let mut text = cell.c.to_string();
            if let Some(chars) = cell.zerowidth() {
                text.extend(chars);
            }
            let mut font = self.font.clone();
            if cell.flags.contains(Flags::BOLD) {
                font.weight = FontWeight::BOLD;
            }
            let run = TextRun {
                len: text.len(),
                font,
                color: gpui::rgb_to_hsla(rgb(fg)),
                background_color: None,
                underline: None,
                strikethrough: None,
                letter_spacing: None,
            };
            let line = window
                .text_system()
                .shape_line(text.into(), px(FONT_SIZE), &[run], None);
            let _ = line.paint(origin, px(CELL_H), gpui::TextAlign::Left, None, window, cx);
        }
        let cursor = content.cursor.point;
        let row = cursor.line.0 + content.display_offset as i32;
        if row >= 0 && row < rows as i32 && content.mode.contains(TermMode::SHOW_CURSOR) {
            let origin = point(
                bounds.left() + px(cursor.column.0 as f32 * self.cell_width),
                bounds.top() + px(row as f32 * CELL_H),
            );
            if self.focus.is_focused(window) {
                window.paint_quad(gpui::fill(
                    gpui::Bounds::new(origin, size(px(2.), px(CELL_H))),
                    rgb(colors.cursor),
                ));
            }
            if !self.preedit.is_empty() {
                let run = TextRun {
                    len: self.preedit.len(),
                    font: self.font.clone(),
                    color: gpui::rgb_to_hsla(rgb(colors.cursor)),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                    letter_spacing: None,
                };
                let line = window.text_system().shape_line(
                    self.preedit.clone().into(),
                    px(FONT_SIZE),
                    &[run],
                    None,
                );
                let _ = line.paint(origin, px(CELL_H), gpui::TextAlign::Left, None, window, cx);
            }
        }
    }
}

fn key_bytes(
    key: &str,
    control: bool,
    alt: bool,
    shift: bool,
    application: bool,
) -> Option<Vec<u8>> {
    let sequence = match key {
        "enter" => "\r",
        "backspace" => {
            if alt {
                "\x1b\x7f"
            } else {
                "\x7f"
            }
        }
        "escape" => "\x1b",
        "tab" => {
            if shift {
                "\x1b[Z"
            } else {
                "\t"
            }
        }
        "up" => {
            if application {
                "\x1bOA"
            } else {
                "\x1b[A"
            }
        }
        "down" => {
            if application {
                "\x1bOB"
            } else {
                "\x1b[B"
            }
        }
        "right" => {
            if application {
                "\x1bOC"
            } else {
                "\x1b[C"
            }
        }
        "left" => {
            if application {
                "\x1bOD"
            } else {
                "\x1b[D"
            }
        }
        "home" => "\x1b[H",
        "end" => "\x1b[F",
        "delete" => "\x1b[3~",
        "pageup" => "\x1b[5~",
        "pagedown" => "\x1b[6~",
        _ if control && key.len() == 1 && key.as_bytes()[0].is_ascii_alphabetic() => {
            return Some(vec![key.as_bytes()[0].to_ascii_lowercase() - b'a' + 1]);
        }
        "space" if control => return Some(vec![0]),
        _ if alt && key.chars().count() == 1 => return Some(format!("\x1b{key}").into_bytes()),
        _ => return None,
    };
    Some(sequence.as_bytes().to_vec())
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = TerminalColors::for_theme(self.theme);
        let input = cx.entity();
        let paint = input.clone();
        let focus = self.focus.clone();
        div()
            .id("project-terminal-input")
            .debug_selector(|| "project-terminal-input".into())
            .size_full()
            .min_h_0()
            .p(px(8.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(colors.background))
            .key_context("ProjectTerminal")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::copy_or_interrupt))
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.dragging = true;
                    let point = this.grid_point(event.position);
                    if let Some(session) = &this.session {
                        session.term.lock().selection =
                            Some(Selection::new(SelectionType::Simple, point, Side::Left));
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if this.dragging {
                    let point = this.grid_point(event.position);
                    if let Some(session) = &this.session {
                        if let Some(selection) = &mut session.term.lock().selection {
                            selection.update(point, Side::Right);
                        }
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging = false),
            )
            .on_mouse_up_out(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging = false),
            )
            .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                if let Some(session) = &this.session {
                    let delta = event.delta.pixel_delta(px(CELL_H));
                    session.term.lock().scroll_display(Scroll::Delta(
                        (f32::from(delta.y) / CELL_H).round() as i32,
                    ));
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .when_some(self.error.clone(), |view, error| {
                view.child(div().p_3().text_color(rgb(colors.ansi[1])).child(error))
            })
            .when(self.session.is_none() && self.error.is_none(), |view| {
                view.child(
                    div()
                        .p_3()
                        .text_color(rgb(colors.foreground))
                        .child("正在启动终端…"),
                )
            })
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, input.clone()),
                            cx,
                        );
                        paint.update(cx, |this, cx| this.paint(bounds, window, cx));
                    },
                )
                .w_full()
                .flex_1()
                .min_h_0(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn terminal_font_requires_a_finite_fixed_pitch() {
        assert!(fixed_pitch(&[7.826, 7.826, 7.826, 7.826, 7.826, 7.826]));
        assert!(!fixed_pitch(&[8., 3., 12., 10., 4., 4.]));
        assert!(!fixed_pitch(&[0.; 6]));
        assert!(!fixed_pitch(&[8., f32::NAN]));
        assert!(!fixed_pitch(&[]));
    }

    #[test]
    fn terminal_palettes_follow_every_app_theme_and_keep_text_readable() {
        fn luminance(color: u32) -> f64 {
            let channel = |value: u32| {
                let c = value as f64 / 255.;
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(color >> 16 & 255)
                + 0.7152 * channel(color >> 8 & 255)
                + 0.0722 * channel(color & 255)
        }
        let contrast = |a, b| {
            let (a, b) = (luminance(a), luminance(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        for theme in ThemePreset::ALL {
            let colors = TerminalColors::for_theme(theme);
            assert_eq!(colors.background, theme.palette().workspace);
            assert_eq!(ansi_color(256, colors), theme.palette().text);
            assert_eq!(ansi_color(257, colors), colors.background);
            assert_eq!(ansi_color(258, colors), theme.palette().accent);
            assert!(
                contrast(colors.foreground, colors.background) >= 4.5,
                "{theme:?}"
            );
            assert!(
                contrast(colors.foreground, colors.selection) >= 4.5,
                "{theme:?}"
            );
            for index in [2, 3, 5, 10, 11, 13] {
                assert!(
                    contrast(colors.ansi[index], colors.background) >= 4.5,
                    "{theme:?} ANSI {index}"
                );
            }
            assert_eq!(
                color(
                    Color::Spec(Rgb {
                        r: 12,
                        g: 34,
                        b: 56
                    }),
                    colors
                ),
                0x0c2238
            );
        }
        let light = TerminalColors::for_theme(ThemePreset::Moonstone);
        let dark = TerminalColors::for_theme(ThemePreset::DarkObsidian);
        for index in 16..=255 {
            assert_eq!(ansi_color(index, light), ansi_color(index, dark));
        }
    }

    #[cfg(windows)]
    #[test]
    fn git_bash_profiles_cannot_redirect_go_children_through_scoop_shims() {
        let dir = tempfile::tempdir().unwrap();
        let git = crate::executable::resolve("git", dir.path());
        let bash = git
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .join("bin/bash.exe");
        if !bash.is_file() {
            return;
        }
        let go = crate::executable::resolve("go", dir.path());
        if !go.is_file() {
            return;
        }
        let source = dir.path().join("probe.go");
        std::fs::write(
            &source,
            r#"package main
import ("fmt"; "os"; "os/exec")
func main() {
 p, err := exec.LookPath("git"); if err != nil { panic(err) }
 fmt.Println("GLINT_NATIVE_GIT=" + p)
 fmt.Println("GLINT_PROFILE=" + os.Getenv("GLINT_PROFILE_SOURCED"))
 out, err := exec.Command("git", "--version").CombinedOutput()
 fmt.Print(string(out)); if err != nil { panic(err) }
}
"#,
        )
        .unwrap();
        let probe = dir.path().join("probe.exe");
        let built = crate::child_process::command(go)
            .current_dir(dir.path())
            .env("GO111MODULE", "off")
            .env("GOTOOLCHAIN", "local")
            .arg("build")
            .arg("-o")
            .arg(&probe)
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let home = dir.path().join("home");
        let shims = home.join("scoop/shims");
        std::fs::create_dir_all(&shims).unwrap();
        std::fs::write(shims.join("git.exe"), "broken shim must never execute").unwrap();
        std::fs::write(
            home.join(".bash_profile"),
            format!(
            "export GLINT_PROFILE_SOURCED=1\nexport PATH=\"$(/usr/bin/cygpath -u -- {}):$PATH\"\n",
                ssh_project::quote(&shims.to_string_lossy().replace('\\', "/"))
            ),
        )
        .unwrap();
        let (args, git_bin) = git_bash_startup(&bash).unwrap();
        let output = crate::child_process::command(&bash)
            .current_dir(dir.path())
            .args(args)
            .args([
                "-c",
                &format!(
                    "exec {}",
                    ssh_project::quote(&probe.to_string_lossy().replace('\\', "/"))
                ),
            ])
            .env("HOME", home.to_string_lossy().replace('\\', "/"))
            .env("CHERE_INVOKING", "1")
            .env("MSYSTEM", "MINGW64")
            .env("GLINT_NATIVE_GIT_BIN", &git_bin)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(stdout.contains("GLINT_PROFILE=1"), "{stdout}");
        assert!(stdout.contains("git version"), "{stdout}");
        let executable = stdout
            .lines()
            .find_map(|line| line.strip_prefix("GLINT_NATIVE_GIT="))
            .unwrap();
        assert_eq!(Path::new(executable).parent().unwrap(), git_bin);
        assert!(!executable.contains("shims"));
    }

    #[gpui::test]
    fn terminal_command_reuses_project_shell_and_accepts_keyboard_paste(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.executor().allow_parking();
        cx.update(crate::initialize);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("providers.json"),
            r#"{"providers":[],"active":null,"active_model":""}"#,
        )
        .unwrap();
        let (host, window) = cx.add_window_view(|_, cx| {
            AgentWorkspace::with_path(dir.path().join("sessions.json"), cx)
        });
        window.simulate_resize(size(px(1440.), px(900.)));
        window.update(|window, cx| {
            host.update(cx, |host, cx| {
                host.open_project_terminal(window, cx);
                assert!(!host.terminal_ui.open);
                host.history.active_mut().project = Some(dir.path().into());
                host.commands.pending = Some("project-terminal".into());
                host.execute_pending_command(window, cx);
            })
        });
        window.run_until_parked();
        let view = host.read_with(window, |host, _| {
            host.terminal_ui.terminals[dir.path()].clone()
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while !view.read_with(window, |view, _| view.session.is_some()) {
            assert!(
                std::time::Instant::now() < deadline,
                "terminal startup timed out"
            );
            std::thread::sleep(Duration::from_millis(20));
            window.run_until_parked();
        }
        view.read_with(window, |view, _| {
            assert!(view.cell_width.is_finite() && view.cell_width > 0.);
            assert_ne!(view.font.family.as_ref(), "monospace");
            let session = view.session.as_ref().unwrap();
            let expected = ((f32::from(view.bounds.size.width) / view.cell_width) as usize).clamp(2, 500);
            assert_eq!(session.term.lock().columns(), expected);
            assert_eq!(session.listener.size.lock().unwrap().cell_width, view.cell_width.round() as u16);
            let position = point(view.bounds.left() + px(view.cell_width * 4.5), view.bounds.top() + px(CELL_H * 2.5));
            assert_eq!(view.grid_point(position).column.0, 4, "selection must follow the measured glyph width");
        });
        window.update(|window, cx| {
            host.update(cx, |host, cx| {
                host.terminal_ui.open = false;
                host.open_project_terminal(window, cx);
                assert_eq!(
                    host.terminal_ui.terminals[dir.path()].entity_id(),
                    view.entity_id()
                );
                assert_eq!(host.terminal_ui.terminals.len(), 1);
            })
        });
        let label = view.read_with(window, |view, _| {
            view.session.as_ref().unwrap().label.clone()
        });
        let term = view.read_with(window, |view, _| {
            view.session.as_ref().unwrap().term.clone()
        });
        let original_theme = host.read_with(window, |host, _| host.theme);
        for theme in [ThemePreset::DarkObsidian, original_theme] {
            host.update(window, |host, cx| {
                host.theme = theme;
                cx.notify();
            });
            window.run_until_parked();
            view.read_with(window, |view, _| {
                let session = view.session.as_ref().unwrap();
                assert!(
                    Arc::ptr_eq(&term, &session.term),
                    "theme changes must preserve the running shell"
                );
                assert_eq!(
                    *session.listener.colors.lock().unwrap(),
                    TerminalColors::for_theme(theme)
                );
                assert_eq!(view.theme, theme);
            });
        }
        if label == "Git Bash" {
            window.update(|window, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                    "printf '\\n\\120\\101\\123\\124\\105\\137\\117\\113\\075\\061\\n'".into(),
                ));
                let focus = view.read(cx).focus.clone();
                window.focus(&focus, cx);
            });
            window.simulate_keystrokes("ctrl-v enter");
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            loop {
                window.run_until_parked();
                let text = view.read_with(window, |view, _| {
                    view.session
                        .as_ref()
                        .map(|s| {
                            s.term
                                .lock()
                                .grid()
                                .display_iter()
                                .map(|c| c.c)
                                .collect::<String>()
                        })
                        .unwrap_or_default()
                });
                if text.contains("PASTE_OK=1") {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "terminal keyboard/paste did not reach shell: {text}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            window.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.replace_and_mark_text_in_range(None, "中", None, window, cx);
                    assert_eq!(view.preedit, "中");
                    view.unmark_text(window, cx);
                    assert!(view.preedit.is_empty());
                })
            });
            window.simulate_input(
                "printf '\\n\\124\\131\\120\\105\\104\\137\\117\\113\\075\\061\\n'; : 中文",
            );
            window.simulate_keystrokes("enter");
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            loop {
                window.run_until_parked();
                let text: String = view.read_with(window, |view, _| {
                    view.session
                        .as_ref()
                        .unwrap()
                        .term
                        .lock()
                        .grid()
                        .display_iter()
                        .map(|cell| cell.c)
                        .collect()
                });
                if text.contains("TYPED_OK=1") {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "typed text did not reach shell: {text}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            view.update(window, |view, _| {
                let mut term = view.session.as_ref().unwrap().term.lock();
                let cells: Vec<_> = term
                    .grid()
                    .display_iter()
                    .map(|cell| (cell.point, cell.c))
                    .collect();
                let marker: Vec<_> = "TYPED_OK=1".chars().collect();
                let start = cells
                    .windows(marker.len())
                    .position(|cells| cells.iter().map(|(_, c)| *c).eq(marker.iter().copied()))
                    .unwrap();
                let mut selection =
                    Selection::new(SelectionType::Simple, cells[start].0, Side::Left);
                selection.update(cells[start + marker.len() - 1].0, Side::Right);
                term.selection = Some(selection);
            });
            window.simulate_keystrokes("ctrl-c");
            window.update(|_, cx| {
                assert_eq!(
                    cx.read_from_clipboard()
                        .and_then(|item| item.text())
                        .as_deref(),
                    Some("TYPED_OK=1")
                )
            });
            view.update(window, |view, _| {
                view.session.as_ref().unwrap().term.lock().selection = None
            });
            window.simulate_input("touch glint_ctrl_c_should_not_run");
            window.simulate_keystrokes("ctrl-c enter");
            window.simulate_input("printf '\\nINTERRUPT_%s=1\\n' OK");
            window.simulate_keystrokes("enter");
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            loop {
                window.run_until_parked();
                let text: String = view.read_with(window, |view, _| {
                    view.session
                        .as_ref()
                        .unwrap()
                        .term
                        .lock()
                        .grid()
                        .display_iter()
                        .map(|cell| cell.c)
                        .collect()
                });
                if text.contains("INTERRUPT_OK=1") {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "Ctrl+C did not return to shell: {text}"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(!dir.path().join("glint_ctrl_c_should_not_run").exists());
        }
    }

    #[test]
    fn terminal_keys_preserve_interrupts_and_application_cursor_mode() {
        assert_eq!(key_bytes("c", true, false, false, false).unwrap(), vec![3]);
        assert_eq!(
            key_bytes("up", false, false, false, true).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(
            key_bytes("up", false, false, false, false).unwrap(),
            b"\x1b[A"
        );
        assert_eq!(
            key_bytes("tab", false, false, true, false).unwrap(),
            b"\x1b[Z"
        );
        assert_eq!(key_bytes("中", false, false, false, false), None);
    }

    #[test]
    #[cfg(windows)]
    fn git_bash_starts_in_project_and_keeps_shell_state() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project spaces");
        std::fs::create_dir_all(root.join("child")).unwrap();
        let (options, label) = shell_options(&root).unwrap();
        assert_eq!(options.working_directory.as_ref(), Some(&root));
        if label != "Git Bash" {
            return;
        }
        assert_eq!(
            options.env.get("CHERE_INVOKING").map(String::as_str),
            Some("1")
        );
        let (session, _) = Session::start(&root, ThemePreset::Moonstone).unwrap();
        session.resize(160, 45, CELL_W);
        session.input(
            b"printf '\\nROOT_CHECK='; pwd -W\rcd child\rprintf '\\nSTATE_CHECK='; pwd -W\r"
                .to_vec(),
        );
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            let text: String = session
                .term
                .lock()
                .grid()
                .display_iter()
                .map(|cell| cell.c)
                .collect();
            let root_text = root.to_string_lossy().replace('\\', "/");
            if text.contains(&format!("STATE_CHECK={root_text}/child")) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "shell cwd/state not received: {text}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(session.term.lock().columns(), 160);
        session.input(b"exit\r".to_vec());
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !session.listener.exited.load(Ordering::Acquire) {
            assert!(std::time::Instant::now() < deadline, "shell did not exit");
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        *actual = Some(range.clone());
        Some(String::from_utf16_lossy(
            &self
                .preedit
                .encode_utf16()
                .skip(range.start)
                .take(range.len())
                .collect::<Vec<_>>(),
        ))
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self.preedit.encode_utf16().count();
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.preedit.is_empty()).then(|| 0..self.preedit.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.preedit.clear();
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit.clear();
        if let Some(session) = &self.session {
            session.input(text.as_bytes().to_vec());
        }
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit = text.into();
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: gpui::Bounds<gpui::Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<gpui::Bounds<gpui::Pixels>> {
        let caret = self
            .session
            .as_ref()
            .map(|s| s.term.lock().grid().cursor.point)
            .unwrap_or_default();
        Some(gpui::Bounds::new(
            point(
                bounds.left() + px(caret.column.0 as f32 * self.cell_width),
                bounds.top() + px(caret.line.0.max(0) as f32 * CELL_H),
            ),
            size(px(self.cell_width), px(CELL_H)),
        ))
    }
    fn character_index_for_point(
        &mut self,
        _: gpui::Point<gpui::Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}

impl AgentWorkspace {
    pub(super) fn sync_terminal_theme(&self, cx: &mut Context<Self>) {
        for terminal in self.terminal_ui.terminals.values() {
            terminal.update(cx, |terminal, cx| terminal.set_theme(self.theme, cx));
        }
    }
    pub(super) fn open_project_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.history.active().project.clone() else {
            return;
        };
        if !self.terminal_ui.terminals.contains_key(&root) {
            if self.terminal_ui.terminals.len() >= 16 {
                self.git_ui.toasts.update(cx, |toasts, cx| {
                    toasts.push(
                        gpui_glint::components::Toast::new("项目终端已达到 16 个")
                            .description("请先结束不再使用的项目终端。")
                            .kind(gpui_glint::components::ToastKind::Error),
                        cx,
                    );
                });
                cx.notify();
                return;
            }
            let terminal = cx.new(|cx| TerminalView::new(root.clone(), self.theme, cx));
            cx.subscribe(&terminal, |_, _, _: &StatusChanged, cx| cx.notify())
                .detach();
            self.terminal_ui.terminals.insert(root.clone(), terminal);
        }
        self.page = crate::Page::Chat;
        self.file_preview.open = false;
        self.edits_ui.open = false;
        self.git_ui.open = false;
        self.terminal_ui.open = true;
        self.right_sidebar_width = self.right_sidebar_width.max(500.);
        let terminal = &self.terminal_ui.terminals[&root];
        window.focus(&terminal.read(cx).focus_handle(cx), cx);
        cx.notify();
    }
    pub(super) fn project_terminal_sidebar(&self, cx: &mut Context<Self>) -> gpui::Div {
        let p = self.theme.palette();
        let terminal = self
            .history
            .active()
            .project
            .as_ref()
            .and_then(|root| self.terminal_ui.terminals.get(root))
            .cloned();
        let label = terminal
            .as_ref()
            .and_then(|view| {
                view.read(cx).session.as_ref().map(|s| {
                    format!(
                        "{}{}",
                        s.label,
                        if s.listener.exited.load(Ordering::Acquire) {
                            " · 已退出"
                        } else {
                            ""
                        }
                    )
                })
            })
            .unwrap_or_else(|| "项目终端".into());
        div()
            .debug_selector(|| "project-terminal-sidebar".into())
            .w(px(self.right_sidebar_width))
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(rgb(p.panel))
            .border_l_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .debug_selector(|| "project-terminal-header".into())
                    .h(px(46.))
                    .flex_shrink_0()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(Icon::Shell.render(16.).flex_shrink_0())
                            .child(div().min_w_0().max_w(px(132.)).truncate().child(label))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(11.))
                                    .text_color(rgb(p.text_muted))
                                    .child(
                                        self.history
                                            .active()
                                            .project
                                            .as_ref()
                                            .map(|path| ssh_project::label(path))
                                            .unwrap_or_default(),
                                    ),
                            ),
                    )
                    .child(
                        IconButton::new(Icon::Refresh, "重启项目终端")
                            .theme(self.theme)
                            .render()
                            .flex_shrink_0()
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(root) = this.history.active().project.clone() {
                                    this.terminal_ui.terminals.remove(&root);
                                    this.open_project_terminal(window, cx);
                                }
                            })),
                    )
                    .child(
                        IconButton::new(Icon::Trash, "结束项目终端")
                            .theme(self.theme)
                            .render()
                            .flex_shrink_0()
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(root) = this.history.active().project.clone() {
                                    this.terminal_ui.terminals.remove(&root);
                                }
                                this.terminal_ui.open = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        IconButton::new(Icon::Close, "隐藏终端侧栏")
                            .theme(self.theme)
                            .render()
                            .flex_shrink_0()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.terminal_ui.open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(div().flex_1().min_h_0().children(terminal))
    }
}
