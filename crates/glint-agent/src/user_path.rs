//! Effective PATH for locally spawned commands.
//!
//! A Finder/Dock-launched application inherits launchd's minimal environment
//! (`PATH=/usr/bin:/bin:/usr/sbin:/sbin`), not the interactive shell's, so
//! toolchains installed by rustup, Homebrew, mise, bun or volta are invisible
//! and `run_command` fails at spawn with `No such file or directory`.
//!
//! The effective PATH is resolved once per process: the inherited PATH first
//! (so a terminal-launched build keeps its exact behaviour), then the login
//! shell's PATH, then well-known install directories that actually exist.
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};

/// A shell configuration that hangs must not block a tool call.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(4);

/// PATH used for every locally spawned command, as directories in lookup order.
pub fn effective() -> &'static [PathBuf] {
    static EFFECTIVE: OnceLock<Vec<PathBuf>> = OnceLock::new();
    EFFECTIVE.get_or_init(|| merge([inherited(), login_shell(), known_directories()]))
}

/// PATH for `Command::env`; falls back to the inherited value if a captured
/// directory cannot be joined (it contains the platform list separator).
pub fn env_value() -> OsString {
    std::env::join_paths(effective())
        .unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
}

/// Captures the login shell PATH on a background thread and caches it.
///
/// Called once at start-up so the first tool call does not pay for it.
pub fn warm_up() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::spawn(|| {
            effective();
        });
    });
}

fn inherited() -> Vec<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect()
}

/// Reads PATH from the user's login shell, which is where interactive
/// configurations put toolchain directories.
#[cfg(unix)]
fn login_shell() -> Vec<PathBuf> {
    let shell = std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|shell| shell.is_file())
        .unwrap_or_else(|| PathBuf::from("/bin/sh"));
    // -l runs login files, -i runs interactive ones such as .zshrc, and -c
    // prints only PATH. stderr is dropped: rc files may be chatty.
    let mut command = Command::new(&shell);
    command
        .args(["-ilc", r#"printf %s "$PATH""#])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let Ok(mut child) = command.spawn() else {
        return Vec::new();
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Vec::new();
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        let _ = sender.send(bytes);
    });
    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    let bytes = loop {
        match receiver.try_recv() {
            Ok(bytes) => break Some(bytes),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => break None,
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    match bytes {
        Some(bytes) => {
            let _ = child.wait();
            parse(&bytes)
        }
        None => {
            let _ = child.kill();
            let _ = child.wait();
            Vec::new()
        }
    }
}

#[cfg(windows)]
fn login_shell() -> Vec<PathBuf> {
    Vec::new()
}

/// Directories that exist regardless of what the shell configuration does.
fn known_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        directories.extend([
            home.join(".cargo/bin"),
            home.join(".rustup/bin"),
            home.join(".local/bin"),
            home.join(".local/share/mise/shims"),
            home.join(".bun/bin"),
            home.join(".volta/bin"),
            home.join(".moon/bin"),
            home.join(".deno/bin"),
            home.join("go/bin"),
            home.join(".orbstack/bin"),
        ]);
    }
    if cfg!(target_os = "macos") {
        directories.extend([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/opt/homebrew/sbin"),
            PathBuf::from("/usr/local/bin"),
        ]);
    } else if cfg!(unix) {
        directories.extend([PathBuf::from("/usr/local/bin"), PathBuf::from("/snap/bin")]);
    }
    directories.retain(|directory| directory.is_dir());
    directories
}

fn merge(groups: impl IntoIterator<Item = Vec<PathBuf>>) -> Vec<PathBuf> {
    let mut merged: Vec<PathBuf> = Vec::new();
    for group in groups {
        for directory in group {
            if directory.as_os_str().is_empty() {
                continue;
            }
            if merged.iter().any(|existing| same(existing, &directory)) {
                continue;
            }
            merged.push(directory);
        }
    }
    merged
}

fn same(left: &Path, right: &Path) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy().eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

/// The last output line carrying a PATH: rc files may print a banner first.
fn parse(bytes: &[u8]) -> Vec<PathBuf> {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .map(str::trim)
        .filter(|line| line.contains('/') || line.contains('\\'))
        .next_back()
        .map(|line| std::env::split_paths(&OsString::from(line)).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_order_and_drops_duplicates() {
        let merged = merge([
            vec![PathBuf::from("/bin"), PathBuf::from("/opt/tool")],
            vec![PathBuf::from("/opt/tool"), PathBuf::from("/usr/bin")],
            vec![PathBuf::new(), PathBuf::from("/bin")],
        ]);
        assert_eq!(
            merged,
            vec![
                PathBuf::from("/bin"),
                PathBuf::from("/opt/tool"),
                PathBuf::from("/usr/bin")
            ]
        );
    }

    #[test]
    fn parse_ignores_banner_text_and_reads_the_last_path_line() {
        let stdout = b"welcome to zsh\n/usr/bin:/bin:/opt/homebrew/bin\n";
        assert_eq!(
            parse(stdout),
            vec![
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
                PathBuf::from("/opt/homebrew/bin")
            ]
        );
    }

    #[test]
    fn parse_returns_nothing_for_empty_output() {
        assert!(parse(b"").is_empty());
    }

    /// Needs a real login shell; run with `--ignored` to inspect this machine.
    #[cfg(unix)]
    #[test]
    #[ignore]
    fn login_shell_path_is_captured() {
        let captured = login_shell();
        assert!(!captured.is_empty());
        assert!(captured.iter().all(|directory| directory.is_absolute()));
        eprintln!("captured: {captured:?}");
    }

    #[test]
    fn effective_contains_every_inherited_directory_first() {
        // Empty entries (which would mean "the working directory") and repeated
        // directories are dropped, so compare against the first occurrences.
        let mut expected: Vec<PathBuf> = Vec::new();
        for directory in inherited() {
            if directory.as_os_str().is_empty() || expected.contains(&directory) {
                continue;
            }
            expected.push(directory);
        }
        let effective = effective();
        assert!(effective.starts_with(&expected));
        assert!(effective.iter().any(|path| path.is_absolute()));
    }
}
