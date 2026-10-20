use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BrowserKind {
    Chromium,
    Firefox,
}
#[derive(Clone, Debug)]
pub struct Browser {
    pub name: String,
    pub path: PathBuf,
    pub kind: BrowserKind,
}

pub fn discover() -> Vec<Browser> {
    let mut browsers = Vec::new();
    #[cfg(target_os = "windows")]
    for (name, relative, kind) in [
        (
            "Google Chrome",
            "Google/Chrome/Application/chrome.exe",
            BrowserKind::Chromium,
        ),
        (
            "Microsoft Edge",
            "Microsoft/Edge/Application/msedge.exe",
            BrowserKind::Chromium,
        ),
        (
            "Mozilla Firefox",
            "Mozilla Firefox/firefox.exe",
            BrowserKind::Firefox,
        ),
        (
            "Brave",
            "BraveSoftware/Brave-Browser/Application/brave.exe",
            BrowserKind::Chromium,
        ),
    ] {
        for root in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"]
            .into_iter()
            .filter_map(std::env::var_os)
        {
            let path = PathBuf::from(root).join(relative);
            if path.is_file() {
                browsers.push(Browser {
                    name: name.into(),
                    path,
                    kind,
                });
                break;
            }
        }
    }
    #[cfg(target_os = "macos")]
    for (name, relative, kind) in [
        (
            "Google Chrome",
            "Google Chrome.app/Contents/MacOS/Google Chrome",
            BrowserKind::Chromium,
        ),
        (
            "Microsoft Edge",
            "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            BrowserKind::Chromium,
        ),
        (
            "Mozilla Firefox",
            "Firefox.app/Contents/MacOS/firefox",
            BrowserKind::Firefox,
        ),
        (
            "Brave",
            "Brave Browser.app/Contents/MacOS/Brave Browser",
            BrowserKind::Chromium,
        ),
    ] {
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
        for root in roots {
            let path = root.join(relative);
            if path.is_file() {
                browsers.push(Browser {
                    name: name.into(),
                    path,
                    kind,
                });
                break;
            }
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    for (name, commands, kind) in [
        (
            "Google Chrome",
            &[
                "google-chrome",
                "google-chrome-stable",
                "chromium",
                "chromium-browser",
            ][..],
            BrowserKind::Chromium,
        ),
        (
            "Microsoft Edge",
            &["microsoft-edge", "microsoft-edge-stable"][..],
            BrowserKind::Chromium,
        ),
        ("Mozilla Firefox", &["firefox"][..], BrowserKind::Firefox),
        (
            "Brave",
            &["brave-browser", "brave"][..],
            BrowserKind::Chromium,
        ),
    ] {
        let paths = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .unwrap_or_default();
        if let Some(path) = commands
            .iter()
            .flat_map(|command| paths.iter().map(move |root| root.join(command)))
            .find(|path| path.is_file())
        {
            browsers.push(Browser {
                name: name.into(),
                path,
                kind,
            });
        }
    }
    browsers
}

pub fn launch(browser: &Browser, directory: &Path, port: u16) -> Result<(), String> {
    // A unique profile is essential: an already running browser otherwise silently ignores proxy flags.
    let profile = directory
        .join("browser-sessions")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&profile).map_err(|e| e.to_string())?;
    let mut command = prepare(browser, &profile, port)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
        .spawn()
        .map_err(|e| format!("无法启动 {}：{e}", browser.name))?;
    Ok(())
}
fn prepare(browser: &Browser, profile: &Path, port: u16) -> Result<Command, String> {
    let mut command = Command::new(&browser.path);
    match browser.kind {
        BrowserKind::Chromium => {
            command
                .arg(format!("--user-data-dir={}", profile.display()))
                .arg(format!("--proxy-server=http://127.0.0.1:{port}"))
                .args([
                    "--proxy-bypass-list=<-loopback>",
                    "--disable-quic",
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
                    "about:blank",
                ]);
        }
        BrowserKind::Firefox => {
            std::fs::write(profile.join("user.js"), firefox_preferences(port))
                .map_err(|e| e.to_string())?;
            command
                .arg("-no-remote")
                .arg("-profile")
                .arg(&profile)
                .arg("about:blank");
        }
    }
    Ok(command)
}
fn firefox_preferences(port: u16) -> String {
    format!(
        r#"user_pref("network.proxy.type", 1);
user_pref("network.proxy.http", "127.0.0.1");
user_pref("network.proxy.http_port", {port});
user_pref("network.proxy.ssl", "127.0.0.1");
user_pref("network.proxy.ssl_port", {port});
user_pref("network.proxy.no_proxies_on", "");
user_pref("network.proxy.allow_hijacking_localhost", true);
user_pref("network.http.http3.enable", false);
user_pref("network.trr.mode", 5);
user_pref("media.peerconnection.enabled", false);
user_pref("security.enterprise_roots.enabled", true);
"#
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn chromium_launch_uses_a_separate_profile_and_proxies_localhost_without_skipping_certificates()
    {
        let directory = tempfile::tempdir().unwrap();
        let browser = super::Browser {
            name: "Chrome".into(),
            path: "chrome.exe".into(),
            kind: super::BrowserKind::Chromium,
        };
        let command = super::prepare(&browser, directory.path(), 8899).unwrap();
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.contains(&"--proxy-server=http://127.0.0.1:8899".into()));
        assert!(args.contains(&"--proxy-bypass-list=<-loopback>".into()));
        assert!(args.contains(&"--disable-quic".into()));
        assert!(args.iter().any(|arg| arg.starts_with("--user-data-dir=")));
        assert!(!args.iter().any(|arg| arg.contains("ignore-certificate")));
    }
    #[test]
    fn firefox_routes_http_https_and_localhost_and_trusts_os_roots() {
        let config = super::firefox_preferences(8899);
        for expected in [
            "http_port\", 8899",
            "ssl_port\", 8899",
            "no_proxies_on\", \"\"",
            "allow_hijacking_localhost\", true",
            "enterprise_roots.enabled\", true",
            "http3.enable\", false",
        ] {
            assert!(config.contains(expected), "{expected}");
        }
    }
}
