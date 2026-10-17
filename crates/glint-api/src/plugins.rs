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
pub const OAUTH_PRESET: &str = "oauth.preset";
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
    let oauth = Plugin::from_source(
        include_str!("../plugins/wps-oauth/glint-plugin.json"),
        include_str!("../plugins/wps-oauth/main.js"),
    )
    .expect("valid bundled OAuth plugin");
    let feishu = Plugin::from_source(
        include_str!("../plugins/feishu-oauth/glint-plugin.json"),
        include_str!("../plugins/feishu-oauth/main.js"),
    )
    .expect("valid bundled Feishu OAuth plugin");
    let mut plugins = vec![builtin, oauth, feishu];
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
    validate_manifest(&plugin.manifest)
}
pub fn validate_manifest(manifest: &gpui_glint::plugins::Manifest) -> Result<(), PluginError> {
    manifest.validate()?;
    let actions =
        manifest.contributions.iter().any(|c| c.kind == HOOK) || manifest.contributions.is_empty();
    if (actions && !manifest.capabilities.contains("request.read"))
        || (manifest.contributions.is_empty() && manifest.hooks != BTreeSet::from([HOOK.into()]))
        || (!actions && (!manifest.hooks.is_empty() || !manifest.capabilities.is_empty()))
    {
        return Err(PluginError("Unsupported API plugin extension".into()));
    }
    for contribution in &manifest.contributions {
        match contribution.kind.as_str() {
            HOOK if !contribution.hook.is_empty() && contribution.data.is_none() => {}
            OAUTH_PRESET if contribution.hook.is_empty() => {
                let preset: OAuthPreset = serde_json::from_value(
                    contribution
                        .data
                        .clone()
                        .ok_or_else(|| PluginError("Missing OAuth preset configuration".into()))?,
                )
                .map_err(|_| PluginError("Invalid OAuth preset configuration".into()))?;
                preset
                    .config()
                    .map_err(|_| PluginError("Invalid OAuth preset configuration".into()))?;
            }
            _ => return Err(PluginError("Unsupported API plugin contribution".into())),
        }
    }
    for c in &manifest.capabilities {
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
fn action_hook<'a>(plugin: &'a Plugin, action_id: &str) -> Result<&'a str, String> {
    if action_id.is_empty() && plugin.manifest.hooks.contains(HOOK) {
        return Ok(HOOK);
    }
    plugin
        .manifest
        .contributions
        .iter()
        .find(|c| c.id == action_id && c.kind == HOOK)
        .map(|c| c.hook.as_str())
        .ok_or("前置动作不可用".into())
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthPreset {
    pub authorization_url: String,
    pub token_url: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default = "pkce_default")]
    pub pkce: bool,
    #[serde(default)]
    pub basic_auth: bool,
    #[serde(default)]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub manual_callback: bool,
    #[serde(default)]
    pub rolling_refresh_expiry: bool,
}
fn pkce_default() -> bool {
    true
}
impl OAuthPreset {
    pub fn config(&self) -> Result<super::oauth::Config, String> {
        let mut config = super::oauth::Config {
            authorization_url: self.authorization_url.clone(),
            token_url: self.token_url.clone(),
            scope: self.scope.clone(),
            pkce: self.pkce,
            basic_auth: self.basic_auth,
            manual_callback: self.manual_callback,
            rolling_refresh_expiry: self.rolling_refresh_expiry,
            ..Default::default()
        };
        if let Some(redirect) = &self.redirect_uri {
            config.redirect_uri = redirect.clone();
        }
        config.client_id = "preset-validation".into();
        config.validate()?;
        config.client_id.clear();
        Ok(config)
    }
}
#[derive(Clone)]
pub struct PresetEntry {
    pub key: String,
    pub title: String,
    pub preset: OAuthPreset,
}
pub fn oauth_presets(catalog: &[Plugin]) -> Vec<PresetEntry> {
    let mut result = vec![];
    for plugin in catalog {
        if catalog.iter().any(|other| {
            other.family_identity() == plugin.family_identity()
                && semver::Version::parse(&other.manifest.version).ok()
                    > semver::Version::parse(&plugin.manifest.version).ok()
        }) {
            continue;
        }
        if validate(plugin).is_err() {
            continue;
        }
        for contribution in plugin
            .manifest
            .contributions
            .iter()
            .filter(|c| c.kind == OAUTH_PRESET)
        {
            if let Some(preset) = contribution
                .data
                .clone()
                .and_then(|data| serde_json::from_value(data).ok())
            {
                result.push(PresetEntry {
                    key: format!("{}::{}", plugin.identity(), contribution.id),
                    title: contribution.title.clone(),
                    preset,
                });
            }
        }
    }
    result
}
pub fn is_bundled(plugin: &Plugin) -> bool {
    plugin.origin.is_none()
        && ["com.wps.kso1", "com.wps.oauth", "com.feishu.oauth"]
            .contains(&plugin.manifest.id.as_str())
}
#[cfg(test)]
pub fn test_oauth_config() -> super::oauth::Config {
    let plugin = Plugin::from_source(
        include_str!("../plugins/wps-oauth/glint-plugin.json"),
        include_str!("../plugins/wps-oauth/main.js"),
    )
    .unwrap();
    oauth_presets(&[plugin])[0].preset.config().unwrap()
}
pub fn run_job(job: Job) -> Result<Value, String> {
    validate(&job.plugin).map_err(|e| e.to_string())?;
    let hook = action_hook(&job.plugin, &job.action_id)?;
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
/// Validate declared environment inputs without running plugin code or exposing values.
pub fn validate_variables(
    ids: &[super::model::BeforeAction],
    catalog: &[Plugin],
    variables: &super::environment::EnvironmentValues,
) -> Result<(), String> {
    for approval in ids.iter().filter(|action| action.enabled) {
        let plugin = catalog
            .iter()
            .find(|plugin| plugin.identity() == approval.id)
            .ok_or("启用的前置动作所属插件已被移除，请重新选择动作")?;
        action_hook(plugin, &approval.action_id)?;
        let title = plugin
            .manifest
            .contributions
            .iter()
            .find(|action| action.id == approval.action_id)
            .map(|action| action.title.as_str())
            .unwrap_or(&plugin.manifest.name);
        let mut missing = vec![];
        let mut empty = vec![];
        for name in plugin
            .manifest
            .capabilities
            .iter()
            .filter_map(|permission| permission.strip_prefix("environment.read."))
        {
            match variables.0.get(name) {
                None => missing.push(name),
                Some(value) if value.trim().is_empty() => empty.push(name),
                _ => {}
            }
        }
        let mut reasons = vec![];
        if !missing.is_empty() {
            reasons.push(format!("缺少环境变量：{}", missing.join("、")));
        }
        if !empty.is_empty() {
            reasons.push(format!("环境变量未填写：{}", empty.join("、")));
        }
        if !reasons.is_empty() {
            return Err(format!(
                "前置动作「{title}」{}。请在当前环境中配置后重试",
                reasons.join("；")
            ));
        }
    }
    Ok(())
}

pub fn apply(
    mut request: super::http::PreparedRequest,
    ids: &[super::model::BeforeAction],
    catalog: &[Plugin],
    variables: &super::environment::EnvironmentValues,
) -> Result<super::http::PreparedRequest, String> {
    validate_variables(ids, catalog, variables)?;
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
        let title = plugin
            .manifest
            .contributions
            .iter()
            .find(|action| action.id == approval.action_id)
            .map(|action| action.title.as_str())
            .unwrap_or(&plugin.manifest.name);
        let output = isolated(&job).map_err(|error| format!("前置动作「{title}」失败：{error}"))?;
        request
            .apply_plugin_headers(&output, &plugin.manifest.capabilities)
            .map_err(|error| format!("前置动作「{title}」无法应用请求头：{error}"))?;
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_and_empty_action_variables_are_named_without_exposing_values() {
        let plugin = catalog()
            .0
            .into_iter()
            .find(|plugin| plugin.manifest.id == "com.wps.kso1")
            .unwrap();
        let mut action = super::super::model::BeforeAction {
            approval: gpui_glint::plugins::Approval::for_plugin(&plugin),
            action_id: "sign-request".into(),
            enabled: true,
        };
        let catalog = vec![plugin];
        let mut variables = super::super::environment::EnvironmentValues(BTreeMap::new());
        let error = validate_variables(&[action.clone()], &catalog, &variables).unwrap_err();
        assert!(error.contains("WPS KSO-1 签名"));
        assert!(error.contains("wps_app_id") && error.contains("wps_app_key"));
        variables
            .0
            .insert("wps_app_id".into(), "private-app-id".into());
        variables.0.insert("wps_app_key".into(), "  ".into());
        let error = validate_variables(&[action.clone()], &catalog, &variables).unwrap_err();
        assert!(error.contains("未填写：wps_app_key"));
        assert!(!error.contains("private-app-id"));
        assert!(!error.contains("wps_app_id"));
        action.enabled = false;
        assert!(validate_variables(&[action.clone()], &catalog, &variables).is_ok());
        action.enabled = true;
        variables
            .0
            .insert("wps_app_key".into(), "private-key".into());
        assert!(validate_variables(&[action], &catalog, &variables).is_ok());
    }

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
                data: None,
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

#[cfg(test)]
mod oauth_preset_tests {
    use super::*;
    fn plugin() -> Plugin {
        Plugin::from_source(
            include_str!("../plugins/wps-oauth/glint-plugin.json"),
            "throw new Error('never-execute-preset')",
        )
        .unwrap()
    }
    #[test]
    fn oauth_preset_is_inert_validated_and_cannot_run_as_a_request_action() {
        let plugin = plugin();
        validate(&plugin).unwrap();
        let presets = oauth_presets(std::slice::from_ref(&plugin));
        assert_eq!(presets.len(), 1);
        let config = presets[0].preset.config().unwrap();
        assert_eq!(
            config.authorization_url,
            "https://openapi.wps.cn/oauth2/auth"
        );
        assert!(!config.pkce);
        assert_eq!(config.scope, "kso.user_base.read");
        assert!(config.client_id.is_empty());
        assert_ne!(config.id, presets[0].preset.config().unwrap().id);
        let error = run_job(Job {
            plugin: plugin.clone(),
            action_id: "user-authorization".into(),
            request: json!({}),
            variables: BTreeMap::new(),
        })
        .unwrap_err();
        assert_eq!(error, "前置动作不可用");
        let binding = super::super::model::BeforeAction {
            approval: gpui_glint::plugins::Approval::for_plugin(&plugin),
            action_id: "user-authorization".into(),
            enabled: true,
        };
        assert!(
            validate_variables(
                &[binding],
                &[plugin],
                &super::super::environment::EnvironmentValues(BTreeMap::new())
            )
            .is_err()
        );
    }
    #[test]
    fn oauth_preset_rejects_secrets_untrusted_urls_hooks_and_permissions() {
        for (field, value) in [
            ("client_secret", json!("secret")),
            ("client_id", json!("app")),
            ("access_token", json!("secret")),
            ("token_url", json!("http://evil.test/token")),
        ] {
            let mut plugin = plugin();
            plugin.manifest.contributions[0].data.as_mut().unwrap()[field] = value;
            assert!(validate(&plugin).is_err());
            assert!(oauth_presets(&[plugin]).is_empty());
        }
        let mut bad = plugin();
        bad.manifest.hooks.insert(HOOK.into());
        bad.manifest.contributions[0].hook = HOOK.into();
        assert!(validate(&bad).is_err());
        let mut bad = plugin();
        bad.manifest
            .capabilities
            .insert("environment.read.wps_app_key".into());
        assert!(validate(&bad).is_err());
    }
    #[test]
    fn oauth_presets_follow_latest_version_and_removal_does_not_change_copied_config() {
        let old = plugin();
        let mut new = old.clone();
        new.manifest.version = "2.0.0".into();
        new.manifest.contributions[0].title = "Updated preset".into();
        new.manifest.contributions[0].data.as_mut().unwrap()["scope"] = json!("new.scope");
        let copied = oauth_presets(std::slice::from_ref(&old))[0]
            .preset
            .config()
            .unwrap();
        let presets = oauth_presets(&[old, new]);
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].title, "Updated preset");
        assert_eq!(presets[0].preset.scope, "new.scope");
        assert_eq!(copied.scope, "kso.user_base.read");
        assert!(oauth_presets(&[]).is_empty());
        assert_eq!(copied.scope, "kso.user_base.read");
    }
}
