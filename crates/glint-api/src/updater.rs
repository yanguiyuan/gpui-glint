use reqwest::Client;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(any(target_os = "windows", test))]
use std::io::Read;
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const RELEASES: &str = "https://api.github.com/repos/yanguiyuan/gpui-glint/releases";
const DOWNLOADS: &str = "https://github.com/yanguiyuan/gpui-glint/releases/download/";
const MAX_INSTALLER_SIZE: u64 = 200 * 1024 * 1024;
const MAX_METADATA_SIZE: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    pub version: Version,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}
impl Update {
    pub fn release_url(&self) -> String {
        format!(
            "https://github.com/yanguiyuan/gpui-glint/releases/tag/glint-api-v{}",
            self.version
        )
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    url: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

fn select_update(releases: &[Release], current: &Version) -> Result<Option<Update>, String> {
    let newest = releases
        .iter()
        .filter_map(|release| {
            let version = Version::parse(release.tag_name.strip_prefix("glint-api-v")?).ok()?;
            (!release.draft
                && !release.prerelease
                && version.pre.is_empty()
                && version.build.is_empty()
                && version > *current)
                .then_some((release, version))
        })
        .max_by(|(_, a), (_, b)| a.cmp(b));
    let Some((release, version)) = newest else {
        return Ok(None);
    };
    let name = format!("Glint-API-{version}-windows-x64-setup.exe");
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .ok_or_else(|| format!("版本 {version} 尚未提供 Windows 安装包"))?;
    if asset.browser_download_url != format!("{DOWNLOADS}glint-api-v{version}/{name}") {
        return Err("更新安装包地址不属于官方发布仓库".into());
    }
    asset
        .url
        .strip_prefix("https://api.github.com/repos/yanguiyuan/gpui-glint/releases/assets/")
        .and_then(|id| id.parse::<u64>().ok())
        .filter(|id| *id > 0)
        .ok_or("更新安装包 API 地址无效")?;
    if asset.size == 0 || asset.size > MAX_INSTALLER_SIZE {
        return Err("更新安装包大小无效".into());
    }
    let sha256 = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or("更新安装包缺少有效的 GitHub SHA-256 校验值")?
        .to_ascii_lowercase();
    Ok(Some(Update {
        version,
        url: asset.url.clone(),
        size: asset.size,
        sha256,
    }))
}

fn client() -> Result<Client, String> {
    Client::builder()
        .https_only(true)
        .user_agent(format!("glint-api/{VERSION}"))
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(30 * 60))
        .build()
        .map_err(|error| format!("无法初始化更新客户端：{error}"))
}

fn authenticate(request: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder, String> {
    let token = std::env::var("GLINT_API_UPDATE_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    if let Some(token) = token {
        let mut header =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token.trim()))
                .map_err(|_| "GitHub 更新令牌格式无效")?;
        header.set_sensitive(true);
        Ok(request.header(reqwest::header::AUTHORIZATION, header))
    } else {
        Ok(request)
    }
}

static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .map_err(|error| format!("无法启动更新执行器：{error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub async fn check() -> Result<Option<Update>, String> {
    runtime()?
        .spawn(async {
            let client = client()?;
            let mut releases = Vec::new();
            // App and library releases share this repository. Do not use /latest,
            // and do not assume release creation order is semantic version order.
            for page in 1..=10 {
                let request = client
                    .get(RELEASES)
                    .query(&[("per_page", 100), ("page", page)])
                    .header("Accept", "application/vnd.github+json")
                    .header("X-GitHub-Api-Version", "2022-11-28")
                    .timeout(Duration::from_secs(20));
                let mut response = authenticate(request)?
                    .send()
                    .await
                    .map_err(|error| format!("无法连接 GitHub：{}", error.without_url()))?;
                if response.status() == reqwest::StatusCode::FORBIDDEN
                    || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
                {
                    return Err("GitHub 更新检查暂时限流，请稍后重试".into());
                }
                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    return Err("更新源无法访问；私有发布源需要设置 GLINT_API_UPDATE_TOKEN".into());
                }
                if !response.status().is_success() {
                    return Err(format!("GitHub 更新检查失败：HTTP {}", response.status()));
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|error| error.without_url().to_string())?
                {
                    if bytes.len() + chunk.len() > MAX_METADATA_SIZE {
                        return Err("GitHub 更新信息过大".into());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                let batch: Vec<Release> =
                    serde_json::from_slice(&bytes).map_err(|_| "GitHub 更新信息格式无效")?;
                let finished = batch.len() < 100;
                releases.extend(batch);
                if finished {
                    return select_update(&releases, &Version::parse(VERSION).unwrap());
                }
            }
            Err("发布记录过多，无法确认最新版本，请查看发布页面".into())
        })
        .await
        .map_err(|error| format!("更新检查中断：{error}"))?
}

pub fn check_blocking() -> Result<Option<Update>, String> {
    runtime()?.block_on(check())
}

/// Download an explicitly requested release for diagnostics/CI without installing it.
pub fn download_release(version: &str, directory: &Path) -> Result<PathBuf, String> {
    let version = Version::parse(version).map_err(|_| "发布版本无效")?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err("仅支持稳定版本".into());
    }
    runtime()?.block_on(async {
        let client = client()?;
        let mut response = authenticate(
            client
                .get(format!("{RELEASES}/tags/glint-api-v{version}"))
                .header("Accept", "application/vnd.github+json")
                .timeout(Duration::from_secs(20)),
        )?
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?
        .error_for_status()
        .map_err(|error| error.without_url().to_string())?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| error.without_url().to_string())?
        {
            if bytes.len() + chunk.len() > MAX_METADATA_SIZE {
                return Err("GitHub 更新信息过大".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let release: Release =
            serde_json::from_slice(&bytes).map_err(|_| "GitHub 发布信息格式无效")?;
        if release.tag_name != format!("glint-api-v{version}") {
            return Err("发布版本不符".into());
        }
        let update = select_update(&[release], &Version::parse("0.0.0").unwrap())?
            .ok_or("未找到可下载的稳定版本")?;
        fs::create_dir_all(directory).map_err(|error| error.to_string())?;
        let path = directory.join(format!("Glint-API-{version}-windows-x64-setup.exe"));
        let response = authenticate(
            client
                .get(&update.url)
                .header("Accept", "application/octet-stream"),
        )?
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
        let (sender, _) = async_channel::bounded(32);
        save_download(response, &path, &update, &sender).await?;
        Ok(path)
    })
}

pub enum DownloadEvent {
    Progress(u64, u64),
    Finished(Result<PathBuf, String>),
}
pub struct DownloadTask(tokio::task::JoinHandle<()>);
impl Drop for DownloadTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub fn start_download(
    update: Update,
    sender: async_channel::Sender<DownloadEvent>,
) -> Result<DownloadTask, String> {
    Ok(DownloadTask(runtime()?.spawn(async move {
        let result = async {
            let cache = cache_directory()?;
            cleanup_cache(&cache);
            let directory = cache.join(uuid::Uuid::new_v4().to_string());
            fs::create_dir_all(&directory).map_err(|error| format!("无法创建更新目录：{error}"))?;
            let path = directory.join(format!(
                "Glint-API-{}-windows-x64-setup.exe",
                update.version
            ));
            let response = authenticate(
                client()?
                    .get(&update.url)
                    .header("Accept", "application/octet-stream"),
            )?
            .send()
            .await
            .map_err(|error| format!("下载更新失败：{}", error.without_url()))?;
            save_download(response, &path, &update, &sender).await?;
            Ok(path)
        }
        .await;
        let _ = sender.send(DownloadEvent::Finished(result)).await;
    })))
}

struct PartialFile(PathBuf);
impl Drop for PartialFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
async fn save_download(
    mut response: reqwest::Response,
    path: &Path,
    update: &Update,
    sender: &async_channel::Sender<DownloadEvent>,
) -> Result<(), String> {
    if !response.status().is_success() {
        return Err(format!("下载更新失败：HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|size| size != update.size)
    {
        return Err("安装包下载大小与发布信息不符".into());
    }
    let partial = PartialFile(path.with_extension("exe.part"));
    let mut file =
        File::create(&partial.0).map_err(|error| format!("无法写入更新目录：{error}"))?;
    let mut hash = Sha256::new();
    let mut received = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("下载更新中断，请重试：{}", error.without_url()))?
    {
        received += chunk.len() as u64;
        if received > update.size || received > MAX_INSTALLER_SIZE {
            return Err("安装包下载大小超出发布信息".into());
        }
        file.write_all(&chunk)
            .map_err(|error| format!("保存更新失败：{error}"))?;
        hash.update(&chunk);
        let _ = sender.try_send(DownloadEvent::Progress(received, update.size));
    }
    if received != update.size || format!("{:x}", hash.finalize()) != update.sha256 {
        return Err("安装包完整性校验失败，已丢弃下载文件".into());
    }
    file.sync_all()
        .map_err(|error| format!("保存更新失败：{error}"))?;
    drop(file);
    fs::rename(&partial.0, path).map_err(|error| format!("保存更新失败：{error}"))?;
    Ok(())
}

fn cache_directory() -> Result<PathBuf, String> {
    let local = std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(|path| PathBuf::from(path).join("AppData/Local"))
        })
        .ok_or("无法确定 Windows 更新缓存目录")?;
    Ok(local.join("Glint API/updates"))
}

fn cleanup_cache(cache: &Path) {
    let Ok(entries) = fs::read_dir(cache) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir())
            && uuid::Uuid::parse_str(&entry.file_name().to_string_lossy()).is_ok()
            && entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(24 * 3600))
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

pub fn can_install() -> bool {
    cfg!(all(target_os = "windows", target_arch = "x86_64"))
        && !cfg!(debug_assertions)
        && std::env::current_exe().ok().is_some_and(|path| {
            path.file_name().is_some_and(|name| name == "glint-api.exe")
                && path
                    .parent()
                    .is_some_and(|parent| parent.join("unins000.exe").is_file())
        })
}

#[derive(Serialize, Deserialize)]
pub struct UpdateJob {
    pub installer: PathBuf,
    pub executable: PathBuf,
    pub parent_pid: u32,
    pub sha256: String,
    pub size: u64,
    pub version: String,
    pub report: PathBuf,
    pub relaunch: bool,
}
#[derive(Serialize, Deserialize)]
pub struct UpdateReport {
    pub success: bool,
    pub version: String,
    pub message: String,
}

pub fn launch_installer(update: &Update, installer: &Path) -> Result<(), String> {
    if !can_install() {
        return Err("自动安装仅支持通过 setup 安装的 Windows x64 发布版".into());
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let directory = installer.parent().ok_or("更新路径无效")?;
    let helper = directory.join("glint-api-update-helper.exe");
    fs::copy(&executable, &helper).map_err(|error| format!("无法准备更新助手：{error}"))?;
    let job = UpdateJob {
        installer: installer.to_owned(),
        executable,
        parent_pid: std::process::id(),
        sha256: update.sha256.clone(),
        size: update.size,
        version: update.version.to_string(),
        report: cache_directory()?.join("last-result.json"),
        relaunch: true,
    };
    let path = directory.join("job.json");
    fs::write(&path, serde_json::to_vec(&job).unwrap())
        .map_err(|error| format!("无法准备更新任务：{error}"))?;
    std::process::Command::new(helper)
        .arg("--apply-update")
        .arg(path)
        .spawn()
        .map_err(|error| format!("无法启动更新助手：{error}"))?;
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
fn verify_file(job: &UpdateJob) -> Result<(), String> {
    let mut file =
        File::open(&job.installer).map_err(|error| format!("无法读取安装包：{error}"))?;
    if file.metadata().map_err(|error| error.to_string())?.len() != job.size
        || job.size == 0
        || job.size > MAX_INSTALLER_SIZE
    {
        return Err("安装包大小校验失败".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    if format!("{:x}", hash.finalize()) != job.sha256 {
        return Err("安装包完整性校验失败".into());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn wait_for_parent(pid: u32) -> Result<(), String> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn WaitForSingleObject(handle: *mut c_void, timeout: u32) -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn GetLastError() -> u32;
    }
    if pid == 0 || pid == std::process::id() {
        return Err("更新进程信息无效".into());
    }
    unsafe {
        let handle = OpenProcess(0x00100000, 0, pid); // SYNCHRONIZE
        if handle.is_null() {
            return if GetLastError() == 87 {
                Ok(())
            } else {
                Err("无法等待应用退出".into())
            };
        }
        let result = WaitForSingleObject(handle, 120_000);
        CloseHandle(handle);
        if result == 0 {
            Ok(())
        } else {
            Err("应用未能退出，更新已停止".into())
        }
    }
}

pub fn run_helper(path: &Path) -> Result<(), String> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Err("自动安装仅支持 Windows".into())
    }
    #[cfg(target_os = "windows")]
    {
        if fs::metadata(path).map_err(|error| error.to_string())?.len() > 32 * 1024 {
            return Err("更新任务无效".into());
        }
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        let job: UpdateJob = serde_json::from_slice(&bytes).map_err(|_| "更新任务格式无效")?;
        let version = Version::parse(&job.version).map_err(|_| "更新版本无效")?;
        let parent = wait_for_parent(job.parent_pid);
        let parent_exited = parent.is_ok();
        let result = (|| {
            parent?;
            verify_file(&job)?;
            let directory = job.executable.parent().ok_or("安装路径无效")?;
            let exit = std::process::Command::new(&job.installer)
                .args([
                    "/VERYSILENT",
                    "/SUPPRESSMSGBOXES",
                    "/NORESTART",
                    "/SP-",
                    "/CLOSEAPPLICATIONS",
                ])
                .arg(format!("/DIR={}", directory.display()))
                .status()
                .map_err(|error| format!("无法启动安装包：{error}"))?;
            if !matches!(exit.code(), Some(0 | 3010)) {
                return Err(format!("安装失败（退出码 {:?}）", exit.code()));
            }
            Ok(())
        })();
        let report = UpdateReport {
            success: result.is_ok(),
            version: version.to_string(),
            message: result
                .as_ref()
                .err()
                .cloned()
                .unwrap_or_else(|| format!("已更新到 {version}")),
        };
        let reported = fs::write(&job.report, serde_json::to_vec(&report).unwrap())
            .map_err(|error| format!("无法保存更新结果：{error}"));
        if job.relaunch && parent_exited {
            std::process::Command::new(&job.executable)
                .spawn()
                .map_err(|error| format!("无法重启应用：{error}"))?;
        }
        if result.is_ok() {
            let _ = fs::remove_file(&job.installer);
        }
        result.and(reported)
    }
}

pub fn take_report() -> Option<UpdateReport> {
    let path = cache_directory().ok()?.join("last-result.json");
    if fs::metadata(&path).ok()?.len() > 32 * 1024 {
        return None;
    }
    let bytes = fs::read(&path).ok()?;
    let _ = fs::remove_file(path);
    if bytes.len() > 32 * 1024 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(version: &str) -> Release {
        let name = format!("Glint-API-{version}-windows-x64-setup.exe");
        Release {
            tag_name: format!("glint-api-v{version}"),
            draft: false,
            prerelease: false,
            assets: vec![Asset {
                name: name.clone(),
                url: "https://api.github.com/repos/yanguiyuan/gpui-glint/releases/assets/123"
                    .into(),
                browser_download_url: format!("{DOWNLOADS}glint-api-v{version}/{name}"),
                size: 3,
                digest: Some(format!("sha256:{}", "a".repeat(64))),
            }],
        }
    }
    #[test]
    fn selects_highest_stable_app_version_and_never_downgrades() {
        let mut library = release("99.0.0");
        library.tag_name = "v99.0.0".into();
        let mut draft = release("8.0.0");
        draft.draft = true;
        let mut pre = release("7.0.0");
        pre.prerelease = true;
        let releases = [
            release("0.1.2"),
            library,
            draft,
            pre,
            release("0.1.10"),
            release("10.0.0-beta.1"),
        ];
        assert_eq!(
            select_update(&releases, &Version::parse("0.1.1").unwrap())
                .unwrap()
                .unwrap()
                .version
                .to_string(),
            "0.1.10"
        );
        assert!(
            select_update(&releases, &Version::parse("0.1.10").unwrap())
                .unwrap()
                .is_none()
        );
        assert!(
            select_update(&releases, &Version::parse("1.0.0").unwrap())
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn incomplete_or_untrusted_newest_release_is_not_installed() {
        let current = Version::parse("0.1.0").unwrap();
        let mut bad = release("0.1.2");
        bad.assets.clear();
        assert!(
            select_update(&[release("0.1.1"), bad], &current)
                .unwrap_err()
                .contains("尚未提供")
        );
        let mut bad = release("0.1.2");
        bad.assets[0].browser_download_url = "https://example.com/setup.exe".into();
        assert!(select_update(&[bad], &current).is_err());
        for digest in [
            None,
            Some("sha256:wrong".into()),
            Some(format!("md5:{}", "a".repeat(64))),
        ] {
            let mut bad = release("0.1.2");
            bad.assets[0].digest = digest;
            assert!(select_update(&[bad], &current).is_err());
        }
        let mut bad = release("0.1.2");
        bad.assets[0].size = MAX_INSTALLER_SIZE + 1;
        assert!(select_update(&[bad], &current).is_err());
    }
    fn response(bytes: &'static [u8]) -> reqwest::Response {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 1024];
            let _ = stream.read(&mut request);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .unwrap();
            stream.write_all(bytes).unwrap();
        });
        runtime().unwrap().block_on(async {
            Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(format!("http://{address}"))
                .send()
                .await
                .unwrap()
        })
    }
    #[test]
    fn download_checks_size_hash_and_discards_partial_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("setup.exe");
        let mut update = Update {
            version: Version::parse("0.1.2").unwrap(),
            url: String::new(),
            size: 3,
            sha256: format!("{:x}", Sha256::digest(b"abc")),
        };
        let (sender, receiver) = async_channel::bounded(32);
        runtime()
            .unwrap()
            .block_on(save_download(response(b"abc"), &path, &update, &sender))
            .unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"abc");
        assert!(matches!(
            receiver.try_recv().unwrap(),
            DownloadEvent::Progress(3, 3)
        ));
        fs::remove_file(&path).unwrap();
        update.sha256 = "0".repeat(64);
        assert!(
            runtime()
                .unwrap()
                .block_on(save_download(response(b"abc"), &path, &update, &sender))
                .is_err()
        );
        assert!(!path.exists() && !path.with_extension("exe.part").exists());
        update.size = 4;
        assert!(
            runtime()
                .unwrap()
                .block_on(save_download(response(b"abc"), &path, &update, &sender))
                .is_err()
        );
        assert!(!path.exists());
    }
    #[test]
    fn helper_rechecks_installer_after_download_and_before_execution() {
        let directory = tempfile::tempdir().unwrap();
        let installer = directory.path().join("setup.exe");
        fs::write(&installer, b"abc").unwrap();
        let job = UpdateJob {
            installer: installer.clone(),
            executable: directory.path().join("glint-api.exe"),
            parent_pid: 1,
            sha256: format!("{:x}", Sha256::digest(b"abc")),
            size: 3,
            version: "0.1.2".into(),
            report: directory.path().join("report.json"),
            relaunch: false,
        };
        verify_file(&job).unwrap();
        fs::write(&installer, b"xyz").unwrap();
        assert!(verify_file(&job).is_err());
    }

    #[test]
    #[ignore = "Connects to the release repository; private sources require GLINT_API_UPDATE_TOKEN"]
    fn downloads_published_installer_using_the_asset_api() {
        let directory = tempfile::tempdir().unwrap();
        runtime().unwrap().block_on(async {
            let client = client().unwrap();
            let response = authenticate(
                client
                    .get(RELEASES)
                    .header("Accept", "application/vnd.github+json"),
            )
            .unwrap()
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
            let bytes = response.bytes().await.unwrap();
            assert!(bytes.len() < MAX_METADATA_SIZE);
            let releases: Vec<Release> = serde_json::from_slice(&bytes).unwrap();
            let update = select_update(&releases, &Version::parse("0.0.0").unwrap())
                .unwrap()
                .unwrap();
            let response = authenticate(
                client
                    .get(&update.url)
                    .header("Accept", "application/octet-stream"),
            )
            .unwrap()
            .send()
            .await
            .unwrap();
            let path = directory.path().join("setup.exe");
            let (sender, _) = async_channel::bounded(32);
            save_download(response, &path, &update, &sender)
                .await
                .unwrap();
            assert_eq!(fs::metadata(path).unwrap().len(), update.size);
        });
    }
}
