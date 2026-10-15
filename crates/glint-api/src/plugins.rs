//! API-workbench adapter. All HTTP/environment policy stays outside glint-plugin.
use gpui_glint::plugins::{HostRegistry, Limits, Plugin, PluginError, PluginRuntime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
use wait_timeout::ChildExt;

pub const HOOK: &str = "request.before-send";
const MAX_JOB: u64 = 12 * 1024 * 1024;
const MAX_OUTPUT: u64 = 64 * 1024;
#[derive(Serialize, Deserialize)]
pub struct Job {
    pub plugin: Plugin,
    #[serde(default)]
    pub action_id: String,
    pub request: Value,
    pub variables: BTreeMap<String, String>,
}

pub fn directory() -> Result<PathBuf, String> {
    let path = super::storage::Store::default_path()?;
    Ok(path.parent().ok_or("无法确定插件目录")?.join("plugins"))
}
pub fn registry_store() -> Result<glint_plugin_registry::PluginStore, String> {
    glint_plugin_registry::PluginStore::new(
        directory()?
            .parent()
            .ok_or("无法确定插件源目录")?
            .join("plugin-registry"),
    )
}
pub fn catalog() -> (Vec<Plugin>, Vec<String>) {
    let builtin = Plugin::from_source(
        include_str!("../plugins/wps-kso1/glint-plugin.json"),
        include_str!("../plugins/wps-kso1/main.js"),
    )
    .expect("valid bundled plugin");
    let mut plugins = vec![builtin];
    let mut errors = vec![];
    if let Ok(directory) = directory() {
        if let Ok(entries) = std::fs::read_dir(directory) {
            let mut entries: Vec<_> = entries
                .flatten()
                .filter(|e| {
                    e.file_type().is_ok_and(|t| t.is_dir())
                        && e.path().join("glint-plugin.json").is_file()
                })
                .collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries.into_iter().take(100) {
                match Plugin::load(&entry.path()).and_then(|p| {
                    validate(&p)?;
                    Ok(p)
                }) {
                    Ok(plugin) if !plugins.iter().any(|p| p.manifest.id == plugin.manifest.id) => {
                        plugins.push(plugin)
                    }
                    _ => errors.push("有插件清单无效、ID 重复或声明了不支持的权限".into()),
                }
            }
        }
    }
    if let Ok(store) = registry_store() {
        match store.installed() {
            Ok(records) => {
                for record in records {
                    match store.load(&record).and_then(|plugin| {
                        validate(&plugin).map_err(|e| e.to_string())?;
                        Ok(plugin)
                    }) {
                        Ok(plugin) => plugins.push(plugin),
                        Err(_) => errors.push("有远程插件损坏或与当前应用不兼容".into()),
                    }
                }
            }
            Err(_) => errors.push("无法读取已安装远程插件".into()),
        }
    }
    (plugins, errors)
}
fn validate(plugin: &Plugin) -> Result<(), PluginError> {
    plugin.manifest.validate()?;
    if !plugin.manifest.capabilities.contains("request.read")
        || (plugin.manifest.contributions.is_empty()
            && plugin.manifest.hooks != BTreeSet::from([HOOK.into()]))
        || plugin.manifest.contributions.iter().any(|c| c.kind != HOOK)
    {
        return Err(PluginError("Unsupported API plugin extension".into()));
    }
    for c in &plugin.manifest.capabilities {
        if c == "crypto" || c == "request.read" {
            continue;
        }
        if let Some(name) = c.strip_prefix("environment.read.") {
            if !name.is_empty() && name.len() <= 128 {
                continue;
            }
        }
        if let Some(name) = c.strip_prefix("request.header.") {
            if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_ok()
                && name == name.to_ascii_lowercase()
                && !matches!(
                    name,
                    "host" | "content-length" | "transfer-encoding" | "connection"
                )
            {
                continue;
            }
        }
        return Err(PluginError("Unsupported plugin permission".into()));
    }
    Ok(())
}
pub fn run_job(job: Job) -> Result<Value, String> {
    validate(&job.plugin).map_err(|e| e.to_string())?;
    let allowed: BTreeSet<String> = job
        .plugin
        .manifest
        .capabilities
        .iter()
        .filter_map(|c| c.strip_prefix("environment.read.").map(str::to_owned))
        .collect();
    let mut host = HostRegistry::default().with_crypto();
    // Per-name access is checked here, beyond the generic host-call capability gate.
    let variables = job.variables;
    host.register("environment.get", "request.read", move |args| {
        let key = args
            .get(0)
            .and_then(Value::as_str)
            .ok_or_else(|| PluginError("Invalid variable name".into()))?;
        if !allowed.contains(key) {
            return Err(PluginError("Variable access denied".into()));
        }
        variables
            .get(key)
            .cloned()
            .map(Value::String)
            .ok_or_else(|| PluginError("Missing environment variable".into()))
    })
    .map_err(|e| e.to_string())?;
    let grants = job.plugin.manifest.capabilities.clone();
    let mut runtime = PluginRuntime::activate(&job.plugin, host, grants, Limits::default())
        .map_err(|e| e.to_string())?;
    let hook = if job.action_id.is_empty() {
        HOOK
    } else {
        &job.plugin
            .manifest
            .contributions
            .iter()
            .find(|c| c.id == job.action_id && c.kind == HOOK)
            .ok_or("前置动作不可用")?
            .hook
    };
    runtime
        .invoke(hook, &job.request)
        .map_err(|e| e.to_string())
}
/// Early CLI path: no GPUI initialization, database access, or credential-bearing diagnostics.
pub fn worker() -> bool {
    let mut text = String::new();
    let result = std::io::stdin()
        .take(MAX_JOB + 1)
        .read_to_string(&mut text)
        .map_err(|_| "无法读取插件任务".to_owned())
        .and_then(|_| {
            if text.len() as u64 > MAX_JOB {
                Err("插件任务过大".into())
            } else {
                serde_json::from_str::<Job>(&text).map_err(|_| "插件任务无效".into())
            }
        })
        .and_then(run_job);
    let response = match result {
        Ok(data) => json!({"ok":data}),
        Err(_) => json!({"error":"插件执行失败，请检查变量、权限与脚本"}),
    };
    let text = serde_json::to_string(&response).expect("JSON data");
    if text.len() as u64 > MAX_OUTPUT {
        print!("{{\"error\":\"插件输出过大\"}}");
        return false;
    }
    print!("{text}");
    true
}
struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
/// Wall-clock watchdog covers parsing, regex and host calls, unlike VM loop limits.
fn isolated(job: &Job) -> Result<Value, String> {
    let executable = std::env::current_exe().map_err(|_| "无法定位插件执行器")?;
    isolated_at(job, &executable, Duration::from_secs(3))
}
fn isolated_at(
    job: &Job,
    executable: &std::path::Path,
    timeout: Duration,
) -> Result<Value, String> {
    let input = serde_json::to_vec(job).map_err(|_| "无法编码插件任务")?;
    if input.len() as u64 > MAX_JOB {
        return Err("插件任务过大".into());
    }
    let mut command = Command::new(executable);
    command
        .arg("--plugin-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = ChildGuard(command.spawn().map_err(|_| "无法启动插件执行器")?);
    let mut stdin = child.0.stdin.take().ok_or("插件输入通道不可用")?;
    let stdout = child.0.stdout.take().ok_or("插件输出通道不可用")?;
    let writer = std::thread::spawn(move || {
        let result = stdin.write_all(&input);
        drop(stdin);
        result
    });
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        stdout
            .take(MAX_OUTPUT + 1)
            .read_to_string(&mut text)
            .map(|_| text)
    });
    let status = child
        .0
        .wait_timeout(timeout)
        .map_err(|_| "无法等待插件执行器")?;
    if status.is_none() {
        let _ = child.0.kill();
        let _ = child.0.wait();
    }
    let wrote = writer.join().map_err(|_| "插件输入失败")?;
    let text = reader
        .join()
        .map_err(|_| "插件输出失败")?
        .map_err(|_| "插件输出无效")?;
    if status.is_none() {
        return Err("插件执行超时（3 秒），已终止，请求未发送".into());
    }
    wrote.map_err(|_| "插件输入失败")?;
    if !status.is_some_and(|s| s.success()) || text.len() as u64 > MAX_OUTPUT {
        return Err("插件执行失败".into());
    }
    let response: Value = serde_json::from_str(&text).map_err(|_| "插件输出无效")?;
    response
        .get("ok")
        .cloned()
        .ok_or_else(|| "插件执行失败，请检查所需环境变量、权限与脚本".into())
}
pub fn apply(
    mut request: super::http::PreparedRequest,
    ids: &[super::model::BeforeAction],
    catalog: &[Plugin],
    variables: &super::environment::EnvironmentValues,
) -> Result<super::http::PreparedRequest, String> {
    let mut seen = BTreeSet::new();
    let mut families = BTreeMap::new();
    for approval in ids.iter().filter(|a| a.enabled) {
        let id = &approval.id;
        if !seen.insert((id, &approval.action_id)) {
            return Err("请求重复启用了同一插件".into());
        }
        let plugin = catalog
            .iter()
            .find(|p| &p.identity() == id)
            .ok_or("启用的插件已被移除，停止发送")?;
        validate(plugin).map_err(|e| e.to_string())?;
        if families
            .insert(plugin.family_identity(), id)
            .is_some_and(|previous| previous != id)
        {
            return Err("同一插件不能同时启用多个版本".into());
        }
        if !approval.matches(plugin) {
            return Err("插件内容或权限已变化，请在插件页重新授权后发送".into());
        }
        let variables = plugin
            .manifest
            .capabilities
            .iter()
            .filter_map(|capability| {
                let key = capability.strip_prefix("environment.read.")?;
                variables.0.get(key).map(|v| (key.to_owned(), v.clone()))
            })
            .collect();
        let job = Job {
            plugin: plugin.clone(),
            action_id: approval.action_id.clone(),
            request: request.plugin_payload()?,
            variables,
        };
        let output = isolated(&job)?;
        request.apply_plugin_headers(&output, &plugin.manifest.capabilities)?;
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registered_action_dispatches_its_own_hook() {
        let mut plugin = catalog().0.remove(0);
        plugin.manifest.hooks.insert("request.second".into());
        plugin
            .manifest
            .contributions
            .push(gpui_glint::plugins::Contribution {
                id: "second".into(),
                kind: HOOK.into(),
                title: "Second".into(),
                hook: "request.second".into(),
            });
        plugin.source="glint.on('request.before-send',()=>({setHeaders:{first:'yes'}})); glint.on('request.second',()=>({setHeaders:{second:'yes'}}));".into();
        let mut job = Job {
            plugin,
            action_id: "second".into(),
            request: json!({}),
            variables: BTreeMap::new(),
        };
        assert_eq!(
            run_job(Job {
                plugin: job.plugin.clone(),
                action_id: job.action_id.clone(),
                request: json!({}),
                variables: BTreeMap::new()
            })
            .unwrap(),
            json!({"setHeaders":{"second":"yes"}})
        );
        job.action_id = "missing".into();
        assert!(run_job(job).is_err());
    }
    #[test]
    fn bundled_kso_matches_official_vectors_and_empty_body_rule() {
        for (method, uri, body, expected) in [
            (
                "GET",
                "/v7/test?key=value",
                "",
                "ce8df66877175e5198c8ea1362ffddf82e4941c6f25a4ca205a1ad09d0faaf03",
            ),
            (
                "POST",
                "/v7/test/body",
                "{\"key\": \"value\"}",
                "c46e6c988130818ecba2484d51ac685948fbbef6814602c7874d6bfc41dc17b3",
            ),
        ] {
            let mut plugin = catalog().0.remove(0);
            plugin.source = format!(
                "const Date = class {{ toUTCString() {{ return 'Mon, 02 Jan 2006 15:04:05 GMT'; }} }};\n{}",
                plugin.source
            );
            let job = Job {
                plugin,
                action_id: String::new(),
                request: json!({"method":method,"uri":uri,"body":body,"headers":{"content-type":"application/json"}}),
                variables: BTreeMap::from([
                    ("wps_app_id".into(), "AK123456".into()),
                    ("wps_app_key".into(), "sk098765".into()),
                ]),
            };
            let output = run_job(job).unwrap();
            assert_eq!(
                output["setHeaders"]["X-Kso-Authorization"],
                format!("KSO-1 AK123456:{expected}")
            );
        }
    }
    #[test]
    fn undeclared_variable_cannot_be_read() {
        let mut plugin = catalog().0.remove(0);
        plugin.source="glint.on('request.before-send', () => glint.host.call('environment.get', 'other_secret'));".into();
        assert!(
            run_job(Job {
                plugin,
                action_id: String::new(),
                request: json!({}),
                variables: BTreeMap::from([("other_secret".into(), "hidden".into())])
            })
            .is_err()
        );
    }
    #[test]
    fn invalid_header_patch_is_atomic_and_signature_uses_encoded_uri() {
        let draft = super::super::model::Draft {
            url: "https://example.com/v7/test?key=a%20b#ignored".into(),
            body: "{\"key\": \"value\"}".into(),
            method: "POST".into(),
            ..Default::default()
        };
        let mut request = super::super::http::PreparedRequest::new(
            &draft,
            &super::super::environment::EnvironmentValues(BTreeMap::new()),
        )
        .unwrap();
        assert_eq!(
            request.plugin_payload().unwrap()["uri"],
            "/v7/test?key=a%20b"
        );
        assert_eq!(request.plugin_payload().unwrap()["body"], draft.body);
        let preview = request.preview.clone();
        let grants = BTreeSet::from(["request.header.x-ok".into()]);
        assert!(
            request
                .apply_plugin_headers(
                    &json!({"setHeaders":{"X-OK":"good","X-Forbidden":"bad"}}),
                    &grants
                )
                .is_err()
        );
        assert_eq!(request.preview, preview);
        assert!(
            request.plugin_payload().unwrap()["headers"]
                .get("x-ok")
                .is_none()
        );
    }
}

#[cfg(test)]
mod process_tests {
    use super::*;
    #[test]
    #[ignore = "requires cargo build -p glint-api and GLINT_API_TEST_EXECUTABLE"]
    fn native_plugin_worker_and_watchdog() {
        let executable = PathBuf::from(
            std::env::var_os("GLINT_API_TEST_EXECUTABLE").expect("built Glint API path"),
        );
        let plugin = catalog().0.remove(0);
        let mut job = Job {
            plugin,
            action_id: String::new(),
            request: json!({"method":"GET","uri":"/v7/test","body":"","headers":{"content-type":"application/json"}}),
            variables: BTreeMap::from([
                ("wps_app_id".into(), "AK123456".into()),
                ("wps_app_key".into(), "secret-fixture".into()),
            ]),
        };
        let output = isolated_at(&job, &executable, Duration::from_secs(3)).unwrap();
        assert!(
            output["setHeaders"]["X-Kso-Authorization"]
                .as_str()
                .unwrap()
                .starts_with("KSO-1 AK123456:")
        );
        job.plugin.source =
            "glint.on('request.before-send', () => { throw new Error('secret-fixture'); });".into();
        let error = isolated_at(&job, &executable, Duration::from_secs(3)).unwrap_err();
        assert!(!error.contains("secret-fixture"));
        // Catastrophic native regex work is beyond VM loop guards.
        job.plugin.source = "/(a+)+b/.test('a'.repeat(32));".into();
        let started = std::time::Instant::now();
        let error = isolated_at(&job, &executable, Duration::from_millis(300)).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(error.contains("超时"));
    }
}
