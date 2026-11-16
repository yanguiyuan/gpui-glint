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
pub const EVENT_DECODE: &str = "websocket.message.decode";
pub const OAUTH_PRESET: &str = "oauth.preset";
pub const AGENT_TOOL: &str = "agent.tool";
/// Response header right-click actions: a declarative match for the menu, one isolated hook per click.
pub const HEADER_ACTION: &str = "response.header.action";
pub const HEADER_READ: &str = "response.read";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentTool {
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

impl AgentTool {
    pub(crate) fn parse(
        contribution: &gpui_glint::plugins::Contribution,
    ) -> Result<Self, PluginError> {
        let invalid = || PluginError("Invalid Agent tool configuration".into());
        let tool: Self = serde_json::from_value(contribution.data.clone().ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        if contribution.hook.is_empty()
            || tool.description.trim().is_empty()
            || tool.description.len() > 1024
            || tool.input_schema.get("type").and_then(Value::as_str) != Some("object")
            || tool
                .input_schema
                .get("properties")
                .is_some_and(|value| !value.is_object())
        {
            return Err(invalid());
        }
        Ok(tool)
    }

    pub(crate) fn name(id: &str, fingerprint: &str) -> String {
        use sha2::{Digest, Sha256};
        let hash = format!(
            "{:x}",
            Sha256::new()
                .chain_update(fingerprint)
                .chain_update(id)
                .finalize()
        );
        let label: String = id
            .chars()
            .take(24)
            .map(|ch| if ch == '.' { '_' } else { ch })
            .collect();
        format!("plugin_{label}_{}", &hash[..24])
    }
}
/// Placeholders a declarative copy / open-URL template may reference.
pub const HEADER_NAME_PLACEHOLDER: &str = "{{header.name}}";
pub const HEADER_VALUE_PLACEHOLDER: &str = "{{header.value}}";
pub const HEADER_STATUS_PLACEHOLDER: &str = "{{response.status}}";
/// The bare placeholder keys accepted inside `{{ }}`.
const HEADER_PLACEHOLDER_KEYS: [&str; 3] = ["header.name", "header.value", "response.status"];
/// Default template for both declarative actions: the header value itself.
pub const DEFAULT_HEADER_TEMPLATE: &str = HEADER_VALUE_PLACEHOLDER;
/// Bounds a template before any substitution, so storage stays small.
pub const MAX_HEADER_TEMPLATE: usize = 2048;

/// The only actions the host performs itself, letting a plugin skip JavaScript entirely.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BuiltinAction {
    Copy,
    OpenUrl,
}

/// Declarative match and action for one response header. The host evaluates it as-is, so opening
/// the context menu never executes plugin code.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HeaderMatch {
    /// One lowercase header name, a list of names, or `*` for every response header.
    pub header: HeaderNames,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<HeaderValueMatch>,
    /// Host-implemented action; a contribution with an action must not declare a hook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<BuiltinAction>,
    /// Copy template; defaults to the header value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Open-URL template; the plugin detail page may override it for this installation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// The response headers a contribution applies to: one exact name, a list of them, or `*`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderNames(Vec<String>);

/// Accepts either `"location"` or `["location", "content-location"]`.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawHeaderNames {
    One(String),
    Many(Vec<String>),
}

impl<'de> Deserialize<'de> for HeaderNames {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(match RawHeaderNames::deserialize(deserializer)? {
            RawHeaderNames::One(name) => vec![name],
            RawHeaderNames::Many(names) => names,
        }))
    }
}

/// Bounds how many headers one contribution may list.
const MAX_HEADER_NAMES: usize = 32;

impl HeaderNames {
    /// Editable form of the same list, so the detail page round-trips through its field.
    pub fn setting_text(&self) -> String {
        if self.0.iter().any(|name| name == "*") {
            "*".to_owned()
        } else {
            self.0.join(", ")
        }
    }

    /// Parsed from the detail-page field: `*`, or names separated by `,` / spaces / `·`.
    pub fn parse_setting(text: &str) -> Result<Self, String> {
        let mut names: Vec<String> = Vec::new();
        for name in text
            .split([',', '，', ';', ' ', '\t', '\n', '·'])
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            let name = name.to_ascii_lowercase();
            if !names.contains(&name) {
                names.push(name);
            }
        }
        if names.is_empty() {
            return Err("请填写 * 或至少一个响应头名".into());
        }
        if names.len() > MAX_HEADER_NAMES {
            return Err(format!("最多匹配 {MAX_HEADER_NAMES} 个响应头"));
        }
        let parsed = Self(names);
        if !parsed.is_valid() {
            return Err("响应头名称只能包含字母、数字和连字符，例如 location".into());
        }
        Ok(parsed)
    }

    /// Exact lowercase names only, without duplicates; `*` alone covers every response header.
    fn is_valid(&self) -> bool {
        !self.0.is_empty()
            && self.0.len() <= MAX_HEADER_NAMES
            && self.0.iter().all(|name| {
                name == "*"
                    || (name == &name.to_ascii_lowercase()
                        && reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_ok())
            })
            && self.0.iter().collect::<BTreeSet<_>>().len() == self.0.len()
    }

    fn matches(&self, name: &str) -> bool {
        self.0
            .iter()
            .any(|candidate| candidate == "*" || candidate.eq_ignore_ascii_case(name))
    }
}

/// Explicit substring predicates instead of regular expressions: linear time, no new dependency.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HeaderValueMatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<String>,
}

impl HeaderMatch {
    pub(crate) fn parse(
        contribution: &gpui_glint::plugins::Contribution,
    ) -> Result<Self, PluginError> {
        let invalid = || PluginError("Invalid response header action".into());
        let matched: Self = serde_json::from_value(contribution.data.clone().ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        // Either the host performs the action, or the click must run a declared hook.
        let hooked = !contribution.hook.is_empty();
        if !matched.is_valid() || hooked == matched.action.is_some() {
            return Err(invalid());
        }
        Ok(matched)
    }

    fn is_valid(&self) -> bool {
        self.header.is_valid()
            && self.value.as_ref().is_none_or(HeaderValueMatch::is_valid)
            && match self.action {
                None => self.text.is_none() && self.url.is_none(),
                Some(BuiltinAction::Copy) => {
                    self.url.is_none() && self.text.as_deref().is_none_or(valid_template)
                }
                Some(BuiltinAction::OpenUrl) => {
                    self.text.is_none() && self.url.as_deref().is_none_or(valid_url_template)
                }
            }
    }

    /// `headers` is the effective list: the installation override when the user set one.
    pub(crate) fn matches_with(&self, headers: &HeaderNames, name: &str, value: &str) -> bool {
        headers.matches(name)
            && self
                .value
                .as_ref()
                .is_none_or(|matcher| matcher.matches(value))
    }

    /// The template the host uses before any installation-level override.
    fn template(&self) -> &str {
        match self.action {
            Some(BuiltinAction::Copy) => self.text.as_deref(),
            Some(BuiltinAction::OpenUrl) => self.url.as_deref(),
            None => None,
        }
        .unwrap_or(DEFAULT_HEADER_TEMPLATE)
    }
}

/// Accepted template shape: bounded, and only the documented placeholders.
fn valid_template(template: &str) -> bool {
    if template.is_empty() || template.len() > MAX_HEADER_TEMPLATE {
        return false;
    }
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let Some(end) = rest[start + 2..].find("}}") else {
            return false;
        };
        if !HEADER_PLACEHOLDER_KEYS.contains(&&rest[start + 2..start + 2 + end]) {
            return false;
        }
        rest = &rest[start + 2 + end + 2..];
    }
    true
}

/// An open-URL template must already look like an absolute address, or be the header value.
fn valid_url_template(template: &str) -> bool {
    if !valid_template(template) {
        return false;
    }
    let trimmed = template.trim();
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || trimmed == DEFAULT_HEADER_TEMPLATE
}

/// Reported by the plugin detail page while a template is typed; empty restores the default.
pub fn validate_header_url_template(template: &str) -> Result<(), String> {
    let trimmed = template.trim();
    if trimmed.len() > MAX_HEADER_TEMPLATE {
        return Err(format!("URL 模板不能超过 {MAX_HEADER_TEMPLATE} 个字符"));
    }
    if trimmed.is_empty() || valid_url_template(trimmed) {
        return Ok(());
    }
    Err(format!(
        "URL 模板需以 http:// 或 https:// 开头，且只能使用 {}、{}、{}",
        HEADER_NAME_PLACEHOLDER, HEADER_VALUE_PLACEHOLDER, HEADER_STATUS_PLACEHOLDER
    ))
}

/// Copy templates have no address requirement, only the documented placeholders.
pub fn validate_header_copy_template(template: &str) -> Result<(), String> {
    let trimmed = template.trim();
    if trimmed.len() > MAX_HEADER_TEMPLATE {
        return Err(format!("复制模板不能超过 {MAX_HEADER_TEMPLATE} 个字符"));
    }
    if trimmed.is_empty() || valid_template(trimmed) {
        return Ok(());
    }
    Err(format!(
        "复制模板只能使用 {}、{}、{}",
        HEADER_NAME_PLACEHOLDER, HEADER_VALUE_PLACEHOLDER, HEADER_STATUS_PLACEHOLDER
    ))
}

/// Reported by the plugin detail page while the header list is typed; empty restores the default.
pub fn validate_header_names_setting(text: &str) -> Result<(), String> {
    HeaderNames::parse_setting(text).map(|_| ())
}

/// Substitute the supported placeholders; unknown or unterminated ones fail instead of leaking text.
fn expand_header_template(
    template: &str,
    name: &str,
    value: &str,
    status: Option<u16>,
) -> Result<String, String> {
    let status = status.map(|code| code.to_string()).unwrap_or_default();
    let mut expanded = String::with_capacity(template.len() + value.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        expanded.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let end = tail.find("}}").ok_or("模板中的占位符未闭合")?;
        expanded.push_str(match &tail[..end] {
            "header.name" => name,
            "header.value" => value,
            "response.status" => status.as_str(),
            _ => return Err("模板包含不支持的占位符".into()),
        });
        rest = &tail[end + 2..];
    }
    expanded.push_str(rest);
    if expanded.len() > MAX_HEADER_COPY {
        return Err("模板展开结果过大".into());
    }
    Ok(expanded)
}

/// Only absolute http(s) addresses may reach the browser; shared by hooks and templates.
fn header_url(text: &str) -> Result<String, String> {
    let text = text.trim();
    let parsed =
        reqwest::Url::parse(text).map_err(|_| "只允许在浏览器打开 http/https 地址".to_owned())?;
    if text.len() > MAX_HEADER_URL
        || !matches!(parsed.scheme(), "http" | "https")
        || parsed.host().is_none()
    {
        return Err("只允许在浏览器打开 http/https 地址".into());
    }
    Ok(text.to_owned())
}

/// Installation-level overrides, keyed by `identity::action` (template) and
/// `identity::action::headers` (header match).
pub type HeaderActionSettings = BTreeMap<String, String>;

/// Template override for one action.
pub fn header_setting_key(identity: &str, action_id: &str) -> String {
    format!("{identity}::{action_id}")
}

/// Header match override for one action: which response headers show the row.
pub fn header_names_setting_key(identity: &str, action_id: &str) -> String {
    format!("{identity}::{action_id}::headers")
}

/// One declarative action the plugin detail page can configure: the headers it appears on, and
/// the template it copies or opens.
pub struct HeaderActionConfig {
    pub action_id: String,
    pub title: String,
    pub action: BuiltinAction,
    pub default_template: String,
    pub template_key: String,
    /// Editable default header list, e.g. `*` or `location, content-location`.
    pub default_headers: String,
    pub headers_key: String,
    /// Optional value predicate summary, for the detail page.
    pub value_filter: Option<String>,
}

impl HeaderActionConfig {
    /// Label above the template editor.
    pub fn template_label(&self) -> &'static str {
        match self.action {
            BuiltinAction::Copy => "复制内容模板",
            BuiltinAction::OpenUrl => "打开 URL 模板",
        }
    }
}

/// Declarative actions this plugin exposes, with their effective defaults.
pub fn header_action_configs(plugin: &Plugin) -> Vec<HeaderActionConfig> {
    plugin
        .manifest
        .contributions
        .iter()
        .filter(|c| c.kind == HEADER_ACTION)
        .filter_map(|c| {
            let matched: HeaderMatch = serde_json::from_value(c.data.clone()?).ok()?;
            let action = matched.action?;
            Some(HeaderActionConfig {
                action_id: c.id.clone(),
                title: c.title.clone(),
                action,
                default_template: matched.template().to_owned(),
                template_key: header_setting_key(&plugin.identity(), &c.id),
                default_headers: matched.header.setting_text(),
                headers_key: header_names_setting_key(&plugin.identity(), &c.id),
                value_filter: matched.value.as_ref().and_then(HeaderValueMatch::describe),
            })
        })
        .collect()
}

/// The installation header list wins over the manifest one; empty falls back to the plugin.
fn effective_headers(
    settings: &HeaderActionSettings,
    identity: &str,
    action_id: &str,
    matched: &HeaderMatch,
) -> Result<HeaderNames, String> {
    match settings.get(&header_names_setting_key(identity, action_id)) {
        Some(text) if !text.trim().is_empty() => HeaderNames::parse_setting(text),
        _ => Ok(matched.header.clone()),
    }
}

/// The installation template wins over the manifest one; empty falls back to the plugin.
fn effective_template<'a>(
    settings: &'a HeaderActionSettings,
    identity: &str,
    action_id: &str,
    matched: &'a HeaderMatch,
) -> &'a str {
    settings
        .get(&header_setting_key(identity, action_id))
        .map(|template| template.trim())
        .filter(|template| !template.is_empty())
        .unwrap_or_else(|| matched.template())
}

impl HeaderValueMatch {
    fn is_valid(&self) -> bool {
        let predicates = [&self.prefix, &self.suffix, &self.contains, &self.equals];
        predicates.iter().any(|predicate| predicate.is_some())
            && predicates.iter().all(|predicate| {
                predicate
                    .as_ref()
                    .is_none_or(|text| !text.is_empty() && text.len() <= 256)
            })
    }

    /// One-line predicate summary, e.g. `前缀 https://`, for the detail page.
    fn describe(&self) -> Option<String> {
        let mut parts = Vec::new();
        for (label, text) in [
            ("前缀", &self.prefix),
            ("后缀", &self.suffix),
            ("包含", &self.contains),
            ("等于", &self.equals),
        ] {
            if let Some(text) = text {
                parts.push(format!("{label} {text}"));
            }
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    fn matches(&self, value: &str) -> bool {
        self.prefix
            .as_ref()
            .is_none_or(|text| value.starts_with(text.as_str()))
            && self
                .suffix
                .as_ref()
                .is_none_or(|text| value.ends_with(text.as_str()))
            && self
                .contains
                .as_ref()
                .is_none_or(|text| value.contains(text.as_str()))
            && self.equals.as_ref().is_none_or(|text| value == text)
    }
}

/// One host-rendered menu entry; the plugin and action are resolved again when it is clicked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderEntry {
    pub identity: String,
    pub action_id: String,
    pub title: String,
}

/// The only top-level results a header action may request from the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeaderAction {
    Copy(String),
    OpenUrl(String),
}

const MAX_HEADER_COPY: usize = 64 * 1024;
const MAX_HEADER_URL: usize = 2048;

const MAX_JOB: u64 = 12 * 1024 * 1024;
const MAX_OUTPUT: u64 = 64 * 1024;
fn output_limit(job: &Job) -> u64 {
    let kind = |kind: &str| {
        job.plugin
            .manifest
            .contributions
            .iter()
            .any(|c| c.id == job.action_id && c.kind == kind)
    };
    if kind(EVENT_DECODE) {
        2 * 1024 * 1024
    } else if kind(HEADER_ACTION) {
        1024 * 1024
    } else {
        MAX_OUTPUT
    }
}
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
    let decoder = Plugin::from_source(
        include_str!("../plugins/wps-event-decrypt/glint-plugin.json"),
        include_str!("../plugins/wps-event-decrypt/main.js"),
    )
    .expect("valid bundled event decoder");
    let wps2 = Plugin::from_source(
        include_str!("../plugins/wps2/glint-plugin.json"),
        include_str!("../plugins/wps2/main.js"),
    )
    .expect("valid bundled WPS-2 plugin");
    // Declarative response header actions: copy and open the header value without any script.
    let headers = Plugin::from_source(
        include_str!("../plugins/response-headers/glint-plugin.json"),
        include_str!("../plugins/response-headers/main.js"),
    )
    .expect("valid bundled response header actions");
    let mut plugins = vec![builtin, oauth, feishu, decoder, wps2, headers];
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
    let events = manifest
        .contributions
        .iter()
        .any(|c| c.kind == EVENT_DECODE);
    let agent_tools = manifest.contributions.iter().any(|c| c.kind == AGENT_TOOL);
    let header_actions = manifest
        .contributions
        .iter()
        .any(|c| c.kind == HEADER_ACTION);
    if (events && !manifest.capabilities.contains("event.read"))
        || (actions && !manifest.capabilities.contains("request.read"))
        || (header_actions && !manifest.capabilities.contains(HEADER_READ))
        || (manifest.contributions.is_empty() && manifest.hooks != BTreeSet::from([HOOK.into()]))
        || (!actions
            && !events
            && !header_actions
            && ((!agent_tools && !manifest.hooks.is_empty()) || !manifest.capabilities.is_empty()))
    {
        return Err(PluginError("Unsupported API plugin extension".into()));
    }
    for contribution in &manifest.contributions {
        match contribution.kind.as_str() {
            HOOK | EVENT_DECODE if !contribution.hook.is_empty() && contribution.data.is_none() => {
            }
            AGENT_TOOL => {
                AgentTool::parse(contribution)?;
            }
            HEADER_ACTION => {
                HeaderMatch::parse(contribution)?;
            }
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
        if c == "crypto" || c == "request.read" || c == "event.read" || c == HEADER_READ {
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

#[derive(Clone, Deserialize, PartialEq)]
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
#[derive(Clone, PartialEq)]
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
        && [
            "com.wps.kso1",
            "com.wps.wps2",
            "com.wps.oauth",
            "com.feishu.oauth",
            "com.wps.event-decrypt",
            "com.glint.response-headers",
        ]
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
    if let Some(tool) = job
        .plugin
        .manifest
        .contributions
        .iter()
        .find(|c| c.id == job.action_id && c.kind == AGENT_TOOL)
    {
        if !job.request.is_object() {
            return Err("Agent 插件工具参数必须为 JSON 对象".into());
        }
        let mut runtime = PluginRuntime::activate(
            &job.plugin,
            HostRegistry::default(),
            BTreeSet::new(),
            Limits::default(),
        )
        .map_err(|e| format!("无法初始化 Agent 插件：{e}"))?;
        return runtime
            .invoke(&tool.hook, &job.request)
            .map_err(|e| format!("Agent 插件工具执行失败：{e}"));
    }
    let event_hook = job
        .plugin
        .manifest
        .contributions
        .iter()
        .find(|c| c.id == job.action_id && c.kind == EVENT_DECODE);
    let header_hook = job
        .plugin
        .manifest
        .contributions
        .iter()
        .find(|c| c.id == job.action_id && c.kind == HEADER_ACTION);
    let (hook, read_capability) = if let Some(action) = header_hook {
        (action.hook.as_str(), HEADER_READ)
    } else if let Some(action) = event_hook {
        (action.hook.as_str(), "event.read")
    } else {
        (action_hook(&job.plugin, &job.action_id)?, "request.read")
    };
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
    host.register("environment.get", read_capability, move |args| {
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
        .map_err(|e| format!("初始化插件运行时或加载脚本失败：{e}"))?;
    runtime
        .invoke(hook, &job.request)
        .map_err(|e| format!("执行插件钩子 {hook} 失败：{e}"))
}
/// Early CLI path: no GPUI initialization, database access, or credential-bearing diagnostics.
pub fn worker() -> bool {
    // Windows gives the executable's main thread only 1 MiB. Boa initialization
    // exceeds that in debug builds; VM recursion limits cannot guard native stack use.
    match std::thread::Builder::new()
        .name("plugin-worker".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(worker_inner)
    {
        Ok(thread) => thread.join().unwrap_or_else(|_| {
            print!("{}", json!({"error":"插件执行线程异常终止"}));
            false
        }),
        Err(error) => {
            print!(
                "{}",
                json!({"error":format!("无法启动插件执行线程：{error}")})
            );
            false
        }
    }
}
fn worker_inner() -> bool {
    let mut text = String::new();
    let mut limit = MAX_OUTPUT;
    let result = super::updater::ensure_not_installing().and_then(|_| {
        std::io::stdin()
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
            .and_then(|job| {
                limit = output_limit(&job);
                run_job(job)
            })
    });
    let response = match result {
        Ok(data) => json!({"ok":data}),
        // run_job only returns credential-safe diagnostics from PluginError;
        // JavaScript exceptions and request/environment values never cross here.
        Err(error) => json!({"error":error}),
    };
    let text = serde_json::to_string(&response).expect("JSON data");
    if text.len() as u64 > limit {
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
pub(super) fn isolated(job: &Job) -> Result<Value, String> {
    let executable = std::env::current_exe().map_err(|_| "无法定位插件执行器")?;
    isolated_at(job, &executable, Duration::from_secs(3))
}
fn isolated_at(
    job: &Job,
    executable: &std::path::Path,
    timeout: Duration,
) -> Result<Value, String> {
    let limit = output_limit(job);
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
    let mut child = ChildGuard(
        command
            .spawn()
            .map_err(|error| format!("无法启动插件执行器：{error}"))?,
    );
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
            .take(limit + 1)
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
        return Err(format!(
            "插件执行超时（{} 毫秒），执行器已终止",
            timeout.as_millis()
        ));
    }
    if text.len() as u64 > limit {
        return Err("插件输出过大".into());
    }
    let status = status.expect("worker exited");
    if !status.success() {
        return parse_worker_response(status, &text);
    }
    wrote.map_err(|_| "插件输入失败")?;
    parse_worker_response(status, &text)
}
fn parse_worker_response(status: std::process::ExitStatus, text: &str) -> Result<Value, String> {
    if !status.success() {
        let reason = match status.code() {
            Some(code) if code as u32 == 0xC00000FD => {
                "插件执行器栈溢出（0xC00000FD），签名或解密未完成，请更新应用后重试".into()
            }
            Some(code) => format!(
                "插件执行器异常退出（退出码 {code} / 0x{:08X}）",
                code as u32
            ),
            None => format!("插件执行器被系统终止（{status}）"),
        };
        // A controlled worker failure can still return a useful JSON diagnostic.
        if let Ok(response) = serde_json::from_str::<Value>(text)
            && let Some(error) = response.get("error").and_then(Value::as_str)
        {
            return Err(format!("{reason}：{error}"));
        }
        return Err(reason);
    }
    let response: Value = serde_json::from_str(text).map_err(|_| {
        if text.is_empty() {
            "插件执行器未返回结果"
        } else {
            "插件执行器返回的结果不是有效 JSON"
        }
    })?;
    if let Some(error) = response.get("error").and_then(Value::as_str) {
        return Err(error.to_owned());
    }
    response
        .get("ok")
        .cloned()
        .ok_or_else(|| "插件执行器返回的结果缺少 ok / error 字段".into())
}
/// Validate declared environment inputs without running plugin code or exposing values.
fn resolve_variable<'a>(
    binding: &'a super::model::BeforeAction,
    name: &'a str,
    variables: &'a super::environment::EnvironmentValues,
) -> (&'a str, Option<&'a String>) {
    if let Some(custom) = binding
        .environment_bindings
        .get(name)
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
    {
        if let Some(value) = variables.0.get(custom) {
            return (custom, Some(value));
        }
    }
    (name, variables.0.get(name))
}

fn environment_inputs(
    binding: &super::model::BeforeAction,
    plugin: &Plugin,
    variables: &super::environment::EnvironmentValues,
) -> BTreeMap<String, String> {
    plugin
        .manifest
        .capabilities
        .iter()
        .filter_map(|capability| {
            let name = capability.strip_prefix("environment.read.")?;
            resolve_variable(binding, name, variables)
                .1
                .map(|value| (name.to_owned(), value.clone()))
        })
        .collect()
}

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
            let (resolved_name, value) = resolve_variable(approval, name, variables);
            match value {
                None => missing.push(resolved_name),
                Some(value) if value.trim().is_empty() => empty.push(resolved_name),
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
        let variables = environment_inputs(approval, plugin, variables);
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

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedEvent {
    pub data: String,
    pub title: Option<String>,
}
impl From<String> for DecodedEvent {
    fn from(data: String) -> Self {
        Self { data, title: None }
    }
}
#[cfg(test)]
pub fn decode_event(
    binding: &super::model::BeforeAction,
    catalog: &[Plugin],
    variables: &super::environment::EnvironmentValues,
    text: &str,
) -> Result<String, String> {
    decode_event_presentation(binding, catalog, variables, text).map(|event| event.data)
}
/// An event decoder runs in the same isolated worker, with its own pinned approval.
pub fn decode_event_presentation(
    binding: &super::model::BeforeAction,
    catalog: &[Plugin],
    variables: &super::environment::EnvironmentValues,
    text: &str,
) -> Result<DecodedEvent, String> {
    let plugin = catalog
        .iter()
        .find(|p| p.identity() == binding.id)
        .ok_or("事件解密插件已被移除，请重新选择")?;
    if !binding.enabled || !binding.matches(plugin) {
        return Err("事件解密插件或权限已变化，请重新选择插件".into());
    }
    if !plugin
        .manifest
        .contributions
        .iter()
        .any(|c| c.id == binding.action_id && c.kind == EVENT_DECODE)
    {
        return Err("插件不支持事件解密".into());
    }
    let mut inputs = BTreeMap::new();
    for key in plugin
        .manifest
        .capabilities
        .iter()
        .filter_map(|c| c.strip_prefix("environment.read."))
    {
        let (resolved_name, value) = resolve_variable(binding, key, variables);
        let value = value
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| format!("请在当前环境配置 {resolved_name}"))?;
        inputs.insert(key.to_owned(), value.clone());
    }
    let request = serde_json::from_str(text).map_err(|_| "接收消息不是有效 JSON，无法解密")?;
    let output = isolated(&Job {
        plugin: plugin.clone(),
        action_id: binding.action_id.clone(),
        request,
        variables: inputs,
    })?;
    event_presentation(output)
}
#[cfg(test)]
fn event_output(output: Value) -> Result<String, String> {
    event_presentation(output).map(|event| event.data)
}
fn event_presentation(output: Value) -> Result<DecodedEvent, String> {
    if let Some(error) = output.get("error").and_then(Value::as_str) {
        return Err(error.chars().take(300).collect());
    }
    let data = output.get("data").ok_or("事件解密插件没有返回 data")?;
    let title = output
        .get("title")
        .and_then(Value::as_str)
        .map(|title| {
            title
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(160)
                .collect::<String>()
        })
        .filter(|title| !title.is_empty());
    Ok(DecodedEvent {
        data: serde_json::to_string(data).map_err(|_| "无法读取事件解密结果")?,
        title,
    })
}

/// Menu entries for one response header. Matching and templating are declarative, so opening the
/// menu never starts a worker; a click may run a declarative action or an isolated hook.
pub fn header_menu(
    catalog: &[Plugin],
    settings: &HeaderActionSettings,
    name: &str,
    value: &str,
    status: Option<u16>,
) -> Vec<HeaderEntry> {
    let mut entries = vec![];
    for plugin in catalog {
        // Several installed versions may be listed; only the newest one contributes.
        if catalog.iter().any(|other| {
            other.family_identity() == plugin.family_identity()
                && semver::Version::parse(&other.manifest.version).ok()
                    > semver::Version::parse(&plugin.manifest.version).ok()
        }) || validate(plugin).is_err()
        {
            continue;
        }
        let identity = plugin.identity();
        for contribution in plugin
            .manifest
            .contributions
            .iter()
            .filter(|c| c.kind == HEADER_ACTION)
        {
            let Some(matched) = contribution
                .data
                .clone()
                .and_then(|data| serde_json::from_value::<HeaderMatch>(data).ok())
            else {
                continue;
            };
            // The detail page may narrow or widen the manifest header list; an unusable setting
            // hides the row instead of acting on a header the user did not choose.
            let Ok(headers) = effective_headers(settings, &identity, &contribution.id, &matched)
            else {
                continue;
            };
            if !matched.matches_with(&headers, name, value) {
                continue;
            }
            // A declarative row only appears when its effective template can produce the result.
            match matched.action {
                // An unusable copy template would only fail on click, so hide it up front.
                Some(BuiltinAction::Copy) => {
                    let template =
                        effective_template(settings, &identity, &contribution.id, &matched);
                    if expand_header_template(template, name, value, status).is_err() {
                        continue;
                    }
                }
                Some(BuiltinAction::OpenUrl) => {
                    let template =
                        effective_template(settings, &identity, &contribution.id, &matched);
                    if expand_header_template(template, name, value, status)
                        .and_then(|url| header_url(&url))
                        .is_err()
                    {
                        continue;
                    }
                }
                None => {}
            }
            entries.push(HeaderEntry {
                identity: identity.clone(),
                action_id: contribution.id.clone(),
                title: contribution.title.clone(),
            });
        }
    }
    entries
}

/// Runs a declarative action or the click hook in the supervised worker.
pub fn run_header_action(
    catalog: &[Plugin],
    settings: &HeaderActionSettings,
    entry: &HeaderEntry,
    name: &str,
    value: &str,
    status: Option<u16>,
    variables: &super::environment::EnvironmentValues,
) -> Result<HeaderAction, String> {
    let plugin = catalog
        .iter()
        .find(|plugin| plugin.identity() == entry.identity)
        .ok_or("响应头动作所属插件已不可用")?;
    validate(plugin).map_err(|error| error.to_string())?;
    let contribution = plugin
        .manifest
        .contributions
        .iter()
        .find(|c| c.id == entry.action_id && c.kind == HEADER_ACTION)
        .ok_or("响应头动作已不可用")?;
    let matched: HeaderMatch = contribution
        .data
        .clone()
        .and_then(|data| serde_json::from_value(data).ok())
        .ok_or("响应头动作配置无效")?;
    // Re-check the effective matcher so a menu left open cannot act on a header the user excluded.
    let headers = effective_headers(settings, &entry.identity, &entry.action_id, &matched)
        .map_err(|error| format!("匹配响应头配置不可用：{error}"))?;
    if !matched.matches_with(&headers, name, value) {
        return Err("响应头动作与当前响应头不匹配".into());
    }
    // Declarative actions never start a worker and need no environment inputs.
    if let Some(action) = matched.action {
        let template = effective_template(settings, &entry.identity, &entry.action_id, &matched);
        return match action {
            BuiltinAction::Copy => {
                let text = expand_header_template(template, name, value, status)?;
                if text.is_empty() {
                    return Err("响应头内容为空，没有可复制的内容".into());
                }
                if text.len() > MAX_HEADER_COPY {
                    return Err("复制内容过大".into());
                }
                Ok(HeaderAction::Copy(text))
            }
            BuiltinAction::OpenUrl => {
                let url = expand_header_template(template, name, value, status)?;
                header_url(&url).map(HeaderAction::OpenUrl)
            }
        };
    }
    let mut inputs = BTreeMap::new();
    for key in plugin
        .manifest
        .capabilities
        .iter()
        .filter_map(|capability| capability.strip_prefix("environment.read."))
    {
        let input = variables
            .0
            .get(key)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("请在当前环境配置 {key}"))?;
        inputs.insert(key.to_owned(), input.clone());
    }
    let output = isolated(&Job {
        plugin: plugin.clone(),
        action_id: contribution.id.clone(),
        request: json!({
            "header": { "name": name, "value": value },
            "response": { "status": status },
        }),
        variables: inputs,
    })?;
    header_action_output(output)
}

fn header_action_output(output: Value) -> Result<HeaderAction, String> {
    if let Some(error) = output.get("error").and_then(Value::as_str) {
        return Err(error.chars().take(300).collect());
    }
    match output.get("action").and_then(Value::as_str) {
        Some("copy") => {
            let text = output
                .get("text")
                .and_then(Value::as_str)
                .ok_or("插件没有返回要复制的文本")?;
            if text.is_empty() || text.len() > MAX_HEADER_COPY {
                return Err("插件返回的复制内容为空或过大".into());
            }
            Ok(HeaderAction::Copy(text.to_owned()))
        }
        Some("open-url") => {
            let url = output
                .get("url")
                .and_then(Value::as_str)
                .ok_or("插件没有返回要打开的地址")?;
            header_url(url).map(HeaderAction::OpenUrl)
        }
        _ => Err("插件返回了不支持的动作".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_wps2_matches_independent_vectors_for_body_and_path() {
        for (body, uri, content_type, md5, signature) in [
            (
                "",
                "/v7/test?key=a%20b&key=a%2Fb&empty=",
                "",
                "d0c59de440cbad6c366be76ad77b52cd",
                "46bea7e0e61a81b7bb781fee7e22d7efa83b2bc6",
            ),
            (
                " \r\n\t",
                "/v7/test?next=/openapi/v7/a",
                "application/json",
                "889cfb550fb79278f593cccd4be00156",
                "cb98003c1b03fcb1f7ff1d951b1ce77c07e29237",
            ),
            (
                "{\"text\":\"中文\"}\n",
                "/v7/test",
                "application/json; charset=utf-8",
                "d00fc8f8c9e44360b7f7dbdfd330a9fc",
                "c66f62d1f1e285704d2336bd0748acd1ea999cc3",
            ),
        ] {
            let mut plugin = catalog()
                .0
                .into_iter()
                .find(|p| p.manifest.id == "com.wps.wps2")
                .unwrap();
            assert!(is_bundled(&plugin));
            // Freeze only the clock; digests were computed independently with Python hashlib.
            plugin.source = format!(
                "const Date = class {{ toUTCString() {{ return 'Mon, 02 Jan 2006 15:04:05 GMT'; }} }};\n{}",
                plugin.source
            );
            let output = run_job(Job {
                plugin: plugin.clone(),
                action_id: "sign-request".into(),
                request: json!({"method":"POST","uri":uri,"body":body,"headers":{"content-type":content_type}}),
                variables: BTreeMap::from([
                    ("wps_app_id".into(), "AK123456".into()),
                    ("wps_app_key".into(), "sk098765".into()),
                ]),
            })
            .unwrap();
            let headers = &output["setHeaders"];
            assert_eq!(headers["Content-MD5"], md5);
            assert_eq!(
                headers["Authorization"],
                format!("WPS-2:AK123456:{signature}")
            );
            assert_eq!(headers["Date"], "Mon, 02 Jan 2006 15:04:05 GMT");
            assert_eq!(
                headers["Content-Type"],
                if content_type.is_empty() {
                    "application/json"
                } else {
                    content_type
                }
            );
            assert_eq!(headers.as_object().unwrap().len(), 4);
            assert!(
                run_job(Job {
                    plugin,
                    action_id: "sign-request".into(),
                    request: json!({}),
                    variables: BTreeMap::new(),
                })
                .is_err()
            );
        }
    }
    #[test]
    fn custom_environment_names_override_fallback_and_keep_worker_permissions_scoped() {
        let plugin = catalog().0.remove(0);
        let binding = super::super::model::BeforeAction {
            approval: gpui_glint::plugins::Approval::for_plugin(&plugin),
            action_id: "sign-request".into(),
            environment_bindings: BTreeMap::from([
                ("wps_app_id".into(), "wps_app_id1".into()),
                ("wps_app_key".into(), "wps_app_key1".into()),
                ("ungranted_secret".into(), "other_secret".into()),
            ]),
            ..Default::default()
        };
        let mut variables = super::super::environment::EnvironmentValues::new(BTreeMap::from([
            ("wps_app_id".into(), "DEFAULT".into()),
            ("wps_app_key".into(), "default-key".into()),
            ("wps_app_id1".into(), "CUSTOM".into()),
            ("wps_app_key1".into(), "custom-key".into()),
            ("other_secret".into(), "must-not-leak".into()),
        ]));
        validate_variables(&[binding.clone()], &[plugin.clone()], &variables).unwrap();
        let inputs = environment_inputs(&binding, &plugin, &variables);
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs["wps_app_id"], "CUSTOM");
        assert_eq!(inputs["wps_app_key"], "custom-key");
        let output = run_job(Job {
            plugin: plugin.clone(),
            action_id: "sign-request".into(),
            request: json!({"method":"GET","uri":"/v7/test","body":"","headers":{}}),
            variables: inputs,
        })
        .unwrap();
        assert!(
            output["setHeaders"]["X-Kso-Authorization"]
                .as_str()
                .unwrap()
                .starts_with("KSO-1 CUSTOM:")
        );
        variables.0.remove("wps_app_id1");
        variables.0.remove("wps_app_key1");
        assert_eq!(
            environment_inputs(&binding, &plugin, &variables)["wps_app_id"],
            "DEFAULT"
        );
        validate_variables(&[binding.clone()], &[plugin.clone()], &variables).unwrap();
        variables.0.insert("wps_app_key1".into(), "  ".into());
        let error = validate_variables(&[binding], &[plugin], &variables).unwrap_err();
        assert!(error.contains("未填写：wps_app_key1"));
        assert!(!error.contains("default-key"));
    }
    #[test]
    fn missing_and_empty_action_variables_are_named_without_exposing_values() {
        let plugin = catalog()
            .0
            .into_iter()
            .find(|plugin| plugin.manifest.id == "com.wps.kso1")
            .unwrap();
        let mut action = super::super::model::BeforeAction {
            environment_bindings: Default::default(),
            approval: gpui_glint::plugins::Approval::for_plugin(&plugin),
            action_id: "sign-request".into(),
            enabled: true,
        };
        let catalog = vec![plugin];
        let mut variables = super::super::environment::EnvironmentValues::new(BTreeMap::new());
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
    fn kso_private_openapi_strips_gateway_prefix_and_preserves_wire_request() {
        // Independent HMAC-SHA256 vectors, with UTF-8 body bytes and a fixed date.
        for (url, expected) in [
            (
                "https://example.test/v7/messages/create",
                "ddac2226ab4bd05a4eca84a39a20474dcceb7f83e93dc0d1dfc1607a16a629ca",
            ),
            (
                "https://example.test/openapi/v7/messages/create",
                "ddac2226ab4bd05a4eca84a39a20474dcceb7f83e93dc0d1dfc1607a16a629ca",
            ),
            (
                "https://example.test/path3/path4/openapi/v7/messages/create",
                "ddac2226ab4bd05a4eca84a39a20474dcceb7f83e93dc0d1dfc1607a16a629ca",
            ),
            (
                "http://example.test/custom/deployment/openapi/v7/messages/create",
                "ddac2226ab4bd05a4eca84a39a20474dcceb7f83e93dc0d1dfc1607a16a629ca",
            ),
            (
                "https://example.test/path3/path4/openapi/v7/messages/create?key=a%20b&key=a%2Fb&empty=",
                "95baba3fa1d81fa7c47222b2ebb0fd421bbcaeff13f27452fc6f4155f78bc148",
            ),
            (
                "https://example.test/path3/path4/openapix/v7/messages/create",
                "e75b98da28497ebf825e16989338c5c386ed154de025959ca198837b6843f1cf",
            ),
            (
                "https://example.test/path3/path4/openapi/custom/messages/create",
                "851de1bdc88270668c4518ad5bcbd9985df4205a954d808ef00a84c62387c9cd",
            ),
            (
                "https://example.test/v7/messages/create?next=/openapi/v7/test",
                "ce5873ca1ca1ba1cfac75b9518a8d161dfbbdc6608d29e083e3b1c5ba3baef8a",
            ),
        ] {
            let body = r#"{"type":"text","content":{"text":"测试消息"}}"#;
            let draft = super::super::model::Draft {
                method: "POST".into(),
                url: url.into(),
                body: body.into(),
                ..Default::default()
            };
            let mut request = super::super::http::PreparedRequest::new(
                &draft,
                &super::super::environment::EnvironmentValues::new(BTreeMap::new()),
            )
            .unwrap();
            let original_payload = request.plugin_payload().unwrap();
            let mut plugin = catalog().0.remove(0);
            plugin.source = format!(
                "const Date = class {{ toUTCString() {{ return 'Mon, 02 Jan 2006 15:04:05 GMT'; }} }};\n{}",
                plugin.source
            );
            let output = run_job(Job {
                plugin: plugin.clone(),
                action_id: String::new(),
                request: original_payload.clone(),
                variables: BTreeMap::from([
                    ("wps_app_id".into(), "AK123456".into()),
                    ("wps_app_key".into(), "sk098765".into()),
                ]),
            })
            .unwrap();
            assert_eq!(
                output["setHeaders"]["X-Kso-Authorization"],
                format!("KSO-1 AK123456:{expected}"),
                "{url}"
            );
            request
                .apply_plugin_headers(&output, &plugin.manifest.capabilities)
                .unwrap();
            assert_eq!(request.url.as_str(), url);
            let signed_payload = request.plugin_payload().unwrap();
            assert_eq!(signed_payload["uri"], original_payload["uri"]);
            assert_eq!(signed_payload["body"], body);
        }
    }
    #[test]
    fn event_websocket_signs_fixed_path_without_changing_connection_url() {
        for (method, url, expected) in [
            (
                "WS",
                "wss://example.test/path3/path4/openws/v7/event/ws",
                "508022d88aeef0c7bcdabf43eb51fcdd4aecda53fb608779a3acd4848e8ca81c",
            ),
            (
                "WS",
                "ws://example.test/openws/v7/event/ws?key=a%20b",
                "508022d88aeef0c7bcdabf43eb51fcdd4aecda53fb608779a3acd4848e8ca81c",
            ),
            (
                "WS",
                "wss://example.test/v7/event/ws",
                "508022d88aeef0c7bcdabf43eb51fcdd4aecda53fb608779a3acd4848e8ca81c",
            ),
            (
                "GET",
                "https://example.test/path3/path4/openws/v7/event/ws?key=a%20b",
                "87404bd00fdd2f320d0eb8bf4a1acf9b6198a93edb6366523f3e37e1138040d4",
            ),
            (
                "WS",
                "wss://example.test/path3/path4/other/ws?key=a%20b",
                "d8af566ea1e498439748c3f8b930d3d32bb818aa06297946484624a5e33a4a98",
            ),
        ] {
            let draft = super::super::model::Draft {
                method: method.into(),
                url: url.into(),
                ..Default::default()
            };
            let mut request = super::super::http::PreparedRequest::new(
                &draft,
                &super::super::environment::EnvironmentValues::new(BTreeMap::new()),
            )
            .unwrap();
            let mut plugin = catalog().0.remove(0);
            plugin.source = format!(
                "const Date = class {{ toUTCString() {{ return 'Mon, 02 Jan 2006 15:04:05 GMT'; }} }};\n{}",
                plugin.source
            );
            let output = run_job(Job {
                plugin: plugin.clone(),
                action_id: String::new(),
                request: request.plugin_payload().unwrap(),
                variables: BTreeMap::from([
                    ("wps_app_id".into(), "AK123456".into()),
                    ("wps_app_key".into(), "sk098765".into()),
                ]),
            })
            .unwrap();
            assert_eq!(
                output["setHeaders"]["X-Kso-Authorization"],
                format!("KSO-1 AK123456:{expected}"),
                "{url}"
            );
            request
                .apply_plugin_headers(&output, &plugin.manifest.capabilities)
                .unwrap();
            assert_eq!(request.url.as_str(), url);
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
            &super::super::environment::EnvironmentValues::new(BTreeMap::new()),
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
    #[cfg(windows)]
    fn exit_status(code: u32) -> std::process::ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code)
    }
    #[cfg(unix)]
    fn exit_status(code: u32) -> std::process::ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw((code as i32) << 8)
    }
    #[test]
    fn worker_protocol_preserves_safe_diagnostics_and_reports_missing_results() {
        assert_eq!(
            parse_worker_response(exit_status(0), r#"{"ok":{"value":1}}"#).unwrap(),
            json!({"value":1})
        );
        let detail = "执行插件钩子 request.before-send 失败：Plugin hook failed";
        let text = json!({"error":detail}).to_string();
        assert_eq!(
            parse_worker_response(exit_status(0), &text).unwrap_err(),
            detail
        );
        let error = parse_worker_response(exit_status(1), &text).unwrap_err();
        assert!(error.contains("0x00000001"));
        assert!(error.contains(detail));
        assert!(
            parse_worker_response(exit_status(0), "")
                .unwrap_err()
                .contains("未返回结果")
        );
        assert!(
            parse_worker_response(exit_status(0), "invalid")
                .unwrap_err()
                .contains("JSON")
        );
        assert!(
            parse_worker_response(exit_status(0), "{}")
                .unwrap_err()
                .contains("字段")
        );
        assert!(
            parse_worker_response(exit_status(2), "")
                .unwrap_err()
                .contains("0x00000002")
        );
    }
    #[test]
    #[cfg(windows)]
    fn worker_stack_overflow_has_an_actionable_diagnostic() {
        let error = parse_worker_response(exit_status(0xC00000FD), "").unwrap_err();
        assert!(error.contains("栈溢出"));
        assert!(error.contains("0xC00000FD"));
    }
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
        assert!(
            error.contains("执行插件钩子 request.before-send"),
            "{error}"
        );
        job.plugin.source = "const invalid = ;".into();
        let error = isolated_at(&job, &executable, Duration::from_secs(3)).unwrap_err();
        assert!(error.contains("初始化插件运行时或加载脚本"), "{error}");
        job.plugin.source =
            "glint.on('request.before-send', () => ({text:'x'.repeat(100000)}));".into();
        let error = isolated_at(&job, &executable, Duration::from_secs(3)).unwrap_err();
        assert!(error.contains("插件输出过大"), "{error}");
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
            environment_bindings: Default::default(),
            approval: gpui_glint::plugins::Approval::for_plugin(&plugin),
            action_id: "user-authorization".into(),
            enabled: true,
        };
        assert!(
            validate_variables(
                &[binding],
                &[plugin],
                &super::super::environment::EnvironmentValues::new(BTreeMap::new())
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

#[cfg(test)]
mod event_decode_tests {
    use super::*;
    pub(super) fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../plugins/wps-event-decrypt/tests/fixture.json"
        ))
        .unwrap()
    }
    fn job(event: Value) -> Job {
        let fixture = fixture();
        Job {
            plugin: catalog()
                .0
                .into_iter()
                .find(|p| p.manifest.id == "com.wps.event-decrypt")
                .unwrap(),
            action_id: "decrypt-event".into(),
            request: event,
            variables: BTreeMap::from([
                (
                    "wps_app_id".into(),
                    fixture["app_id"].as_str().unwrap().into(),
                ),
                (
                    "wps_app_key".into(),
                    fixture["app_key"].as_str().unwrap().into(),
                ),
            ]),
        }
    }
    #[test]
    fn wps_event_decrypts_independent_openssl_fixture_after_verification() {
        let fixture = fixture();
        let output = run_job(job(fixture["event"].clone())).unwrap();
        assert_eq!(output["data"]["data"], fixture["data"]);
        assert_eq!(output["data"]["signature_verified"], true);
        assert!(
            output["title"]
                .as_str()
                .unwrap()
                .contains("kso.app_chat.message.create")
        );
        assert!(event_output(output).unwrap().contains("事件解密测试"));
    }
    #[test]
    fn decoder_title_is_optional_bounded_and_separate_from_body() {
        let result =
            event_presentation(json!({"data": {"id": 7}, "title": "  hello\nworld \t"})).unwrap();
        assert_eq!(result.title.as_deref(), Some("hello world"));
        assert_eq!(result.data, r#"{"id":7}"#);
        for title in [Value::Null, json!(7), json!("  ")] {
            assert!(
                event_presentation(json!({"data": {}, "title": title}))
                    .unwrap()
                    .title
                    .is_none()
            );
        }
        assert!(
            event_presentation(json!({"data": {}}))
                .unwrap()
                .title
                .is_none()
        );
        assert_eq!(
            event_presentation(json!({"data": {}, "title": "文".repeat(300)}))
                .unwrap()
                .title
                .unwrap()
                .chars()
                .count(),
            160
        );
        assert!(event_presentation(json!({"error":"bad", "title":"ignored", "data":{}})).is_err());
    }
    #[test]
    fn wps_event_rejects_tampered_envelope_and_bad_key_without_plaintext() {
        let fixture = fixture();
        for field in ["signature", "nonce", "topic", "encrypted_data"] {
            let mut event = fixture["event"].clone();
            event[field] = "tampered".into();
            let output = run_job(job(event)).unwrap();
            assert!(output.get("data").is_none());
            assert!(event_output(output).is_err());
        }
        let mut wrong = job(fixture["event"].clone());
        wrong
            .variables
            .insert("wps_app_key".into(), "wrong-secret".into());
        let error = event_output(run_job(wrong).unwrap()).unwrap_err();
        assert!(error.contains("签名"));
        assert!(!error.contains("wrong-secret"));
    }
    #[test]
    fn signed_malformed_ciphertext_and_nonce_are_rejected_without_panics() {
        for event in fixture()["bad_cases"].as_array().unwrap() {
            let output = run_job(job(event.clone())).unwrap();
            assert!(output.get("data").is_none());
            assert!(event_output(output).unwrap_err().contains("解密失败"));
        }
    }
    #[test]
    fn decoder_requires_approval_and_named_variables_without_affecting_request_hooks() {
        let plugin = job(fixture()["event"].clone()).plugin;
        let mut binding = super::super::model::BeforeAction {
            environment_bindings: Default::default(),
            approval: gpui_glint::plugins::Approval::for_plugin(&plugin),
            action_id: "decrypt-event".into(),
            enabled: true,
        };
        let variables = super::super::environment::EnvironmentValues::new(BTreeMap::new());
        let error = decode_event(&binding, &[plugin.clone()], &variables, "{}").unwrap_err();
        assert!(error.contains("wps_app_"));
        binding.approval.fingerprint = "changed".into();
        assert!(
            decode_event(&binding, &[plugin.clone()], &variables, "{}")
                .unwrap_err()
                .contains("权限")
        );
        assert!(action_hook(&plugin, "decrypt-event").is_err());
    }

    fn header_action_plugin(contributions: Value, capabilities: &[&str], source: &str) -> Plugin {
        let manifest = json!({
            "id": "com.example.header-actions",
            "name": "响应头动作",
            "version": "1.0.0",
            "api_version": 1,
            "entry": "main.js",
            "capabilities": capabilities,
            "hooks": ["response.header.action"],
            "contributions": contributions
        });
        Plugin::from_source(&manifest.to_string(), source).expect("valid header action plugin")
    }

    const HEADER_HOOK: &str = "glint.on('response.header.action', (input) => \
        ({ action: 'copy', text: input.header.name + ': ' + input.header.value }));";

    #[test]
    fn response_header_menu_matches_declared_name_and_value_predicates() {
        let plugin = header_action_plugin(
            json!([
                {
                    "id": "open-callback",
                    "kind": "response.header.action",
                    "title": "在浏览器打开回跳地址",
                    "hook": "response.header.action",
                    "data": { "header": "location", "value": { "prefix": "https://" } }
                },
                {
                    "id": "copy-www-authenticate",
                    "kind": "response.header.action",
                    "title": "复制认证失败原因",
                    "hook": "response.header.action",
                    "data": { "header": "www-authenticate", "value": { "contains": "invalid_token" } }
                }
            ]),
            &["response.read"],
            HEADER_HOOK,
        );
        let no_settings = HeaderActionSettings::new();
        let entries = header_menu(
            &[plugin.clone()],
            &no_settings,
            "Location",
            "https://example.test/cb?code=1",
            Some(302),
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action_id, "open-callback");
        assert_eq!(entries[0].title, "在浏览器打开回跳地址");
        // Header names are compared case-insensitively; values must still satisfy the predicates.
        assert!(
            header_menu(
                &[plugin.clone()],
                &no_settings,
                "LOCATION",
                "/relative/cb",
                None
            )
            .is_empty()
        );
        assert_eq!(
            header_menu(
                &[plugin.clone()],
                &no_settings,
                "www-authenticate",
                "Bearer error=\"invalid_token\"",
                None,
            )
            .len(),
            1
        );
        assert!(
            header_menu(
                &[plugin],
                &no_settings,
                "x-trace",
                "https://example.test",
                None
            )
            .is_empty()
        );
    }

    #[test]
    fn header_actions_can_target_a_list_of_specific_response_headers() {
        // One contribution may cover several exact headers, so the menu stays focused on them.
        let plugin = header_action_plugin(
            json!([
                {
                    "id": "copy-redirect",
                    "kind": "response.header.action",
                    "title": "复制回跳地址",
                    "hook": "response.header.action",
                    "data": { "header": ["location", "content-location"] }
                },
                {
                    "id": "copy-trace",
                    "kind": "response.header.action",
                    "title": "复制追踪号",
                    "hook": "response.header.action",
                    "data": { "header": ["x-request-id"] }
                }
            ]),
            &["response.read"],
            HEADER_HOOK,
        );
        let settings = HeaderActionSettings::new();
        let ids = |name: &str| -> Vec<String> {
            header_menu(
                std::slice::from_ref(&plugin),
                &settings,
                name,
                "https://example.test",
                None,
            )
            .into_iter()
            .map(|entry| entry.action_id)
            .collect()
        };
        assert_eq!(ids("Location"), ["copy-redirect".to_owned()]);
        assert_eq!(ids("Content-Location"), ["copy-redirect".to_owned()]);
        assert_eq!(ids("x-request-id"), ["copy-trace".to_owned()]);
        assert!(ids("x-trace").is_empty());
        // A single name and a wildcard keep working exactly as before.
        for header in [
            json!("location"),
            json!(["location"]),
            json!("*"),
            json!(["*"]),
        ] {
            let single = header_action_plugin(
                json!([{
                    "id": "copy-any",
                    "kind": "response.header.action",
                    "title": "复制",
                    "hook": "response.header.action",
                    "data": { "header": header }
                }]),
                &["response.read"],
                HEADER_HOOK,
            );
            assert_eq!(
                header_menu(&[single], &settings, "location", "v", None).len(),
                1,
                "{header}"
            );
        }
        // Anything ambiguous is rejected up front instead of silently matching nothing.
        let mut too_many = vec![];
        for index in 0..MAX_HEADER_NAMES + 1 {
            too_many.push(format!("x-h{index}"));
        }
        for header in [
            json!([]),
            json!(["Location"]),
            json!(["location", "location"]),
            json!(["location", "not a header"]),
            json!([""]),
            json!(7),
            json!(too_many),
        ] {
            let invalid = header_action_plugin(
                json!([{
                    "id": "copy-any",
                    "kind": "response.header.action",
                    "title": "复制",
                    "hook": "response.header.action",
                    "data": { "header": header }
                }]),
                &["response.read"],
                HEADER_HOOK,
            );
            assert!(
                validate_manifest(&invalid.manifest).is_err(),
                "expected {header} to be rejected"
            );
        }
    }

    #[test]
    fn header_action_requires_the_response_capability_and_a_declared_hook() {
        let contribution = json!([{
            "id": "copy-location",
            "kind": "response.header.action",
            "title": "复制 Location",
            "hook": "response.header.action",
            "data": { "header": "location" }
        }]);
        let missing = header_action_plugin(contribution.clone(), &[], HEADER_HOOK);
        assert!(validate_manifest(&missing.manifest).is_err());
        let granted = header_action_plugin(contribution, &["response.read"], HEADER_HOOK);
        assert!(validate_manifest(&granted.manifest).is_ok());
        // A menu entry is inert data, so the click hook can never be empty.
        let unhooked = header_action_plugin(
            json!([{
                "id": "no-hook",
                "kind": "response.header.action",
                "title": "没有钩子",
                "data": { "header": "location" }
            }]),
            &["response.read"],
            HEADER_HOOK,
        );
        assert!(validate_manifest(&unhooked.manifest).is_err());
        for data in [
            json!({ "header": "Location" }),
            json!({ "header": "location", "value": {} }),
            json!({ "header": "location", "value": { "prefix": "" } }),
            json!({ "header": "location", "pattern": "^https" }),
        ] {
            let invalid = header_action_plugin(
                json!([{
                    "id": "copy-location",
                    "kind": "response.header.action",
                    "title": "复制 Location",
                    "hook": "response.header.action",
                    "data": data
                }]),
                &["response.read"],
                HEADER_HOOK,
            );
            assert!(validate_manifest(&invalid.manifest).is_err());
        }
    }

    #[test]
    fn header_action_output_accepts_only_copy_and_http_urls() {
        assert_eq!(
            header_action_output(json!({"action": "copy", "text": "value"})).unwrap(),
            HeaderAction::Copy("value".into())
        );
        assert_eq!(
            header_action_output(json!({"action": "open-url", "url": " https://a.test/x "}))
                .unwrap(),
            HeaderAction::OpenUrl("https://a.test/x".into())
        );
        assert_eq!(
            header_action_output(json!({"action": "open-url", "url": "http://a.test/x"})).unwrap(),
            HeaderAction::OpenUrl("http://a.test/x".into())
        );
        for output in [
            json!({"action": "open-url", "url": "file:///etc/passwd"}),
            json!({"action": "open-url", "url": "javascript:alert(1)"}),
            json!({"action": "open-url", "url": "https://"}),
            json!({"action": "open-url", "url": "not a url"}),
            json!({"action": "open-url"}),
            json!({"action": "copy"}),
            json!({"action": "copy", "text": ""}),
            json!({"action": "copy", "text": "x".repeat(MAX_HEADER_COPY + 1)}),
            json!({"action": "run", "text": "rm -rf /"}),
            json!({"text": "value"}),
        ] {
            assert!(header_action_output(output.clone()).is_err(), "{output}");
        }
        assert_eq!(
            header_action_output(json!({"error": "缺少参数"})),
            Err("缺少参数".into())
        );
    }

    #[test]
    fn response_header_hook_receives_the_header_and_returns_a_validated_action() {
        let plugin = header_action_plugin(
            json!([{
                "id": "open-callback",
                "kind": "response.header.action",
                "title": "在浏览器打开回跳地址",
                "hook": "response.header.action",
                "data": { "header": "location" }
            }]),
            &["response.read"],
            "glint.on('response.header.action', (input) => input.response.status === 302 \
                ? { action: 'open-url', url: input.header.value } \
                : { error: '状态码 ' + input.response.status });",
        );
        let output = run_job(Job {
            plugin: plugin.clone(),
            action_id: "open-callback".into(),
            request: json!({
                "header": {"name": "location", "value": "https://example.test/cb"},
                "response": {"status": 302}
            }),
            variables: BTreeMap::new(),
        })
        .unwrap();
        assert_eq!(
            header_action_output(output).unwrap(),
            HeaderAction::OpenUrl("https://example.test/cb".into())
        );
        let output = run_job(Job {
            plugin,
            action_id: "open-callback".into(),
            request: json!({
                "header": {"name": "location", "value": "https://example.test/cb"},
                "response": {"status": 200}
            }),
            variables: BTreeMap::new(),
        })
        .unwrap();
        assert_eq!(header_action_output(output), Err("状态码 200".into()));
    }

    fn bundled_header_plugin() -> Plugin {
        Plugin::from_source(
            include_str!("../plugins/response-headers/glint-plugin.json"),
            include_str!("../plugins/response-headers/main.js"),
        )
        .expect("valid bundled response header actions")
    }

    fn no_variables() -> crate::environment::EnvironmentValues {
        crate::environment::EnvironmentValues::new(BTreeMap::new())
    }

    #[test]
    fn bundled_header_actions_copy_and_open_urls_without_any_hook() {
        let plugin = bundled_header_plugin();
        validate(&plugin).unwrap();
        assert!(is_bundled(&plugin));
        assert!(plugin.manifest.hooks.is_empty());
        let identity = plugin.identity();
        assert_eq!(
            header_action_configs(&plugin)
                .iter()
                .map(|action| (
                    action.action_id.as_str(),
                    action.default_template.as_str(),
                    action.default_headers.as_str(),
                ))
                .collect::<Vec<_>>(),
            [
                ("copy-value", DEFAULT_HEADER_TEMPLATE, "*"),
                ("open-url", DEFAULT_HEADER_TEMPLATE, "*"),
            ]
        );
        let run = |action_id: &str, name: &str, value: &str, status: Option<u16>, settings| {
            run_header_action(
                std::slice::from_ref(&plugin),
                settings,
                &HeaderEntry {
                    identity: identity.clone(),
                    action_id: action_id.to_owned(),
                    title: String::new(),
                },
                name,
                value,
                status,
                &no_variables(),
            )
        };
        let settings = HeaderActionSettings::new();
        // Copy is template-expanded data only; it never starts the isolated worker.
        assert_eq!(
            run(
                "copy-value",
                "Location",
                "/callback?code=1",
                Some(302),
                &settings
            )
            .unwrap(),
            HeaderAction::Copy("/callback?code=1".into())
        );
        assert_eq!(
            run(
                "open-url",
                "Location",
                "https://example.test/cb",
                Some(302),
                &settings
            )
            .unwrap(),
            HeaderAction::OpenUrl("https://example.test/cb".into())
        );
        // A relative header value is not an address, so the row is neither offered nor executed.
        assert!(run("open-url", "Location", "/callback", Some(302), &settings).is_err());
        assert!(
            header_menu(
                &[plugin.clone()],
                &settings,
                "Location",
                "/callback",
                Some(302)
            )
            .iter()
            .all(|entry| entry.action_id == "copy-value")
        );
        // An installation template can turn any header into an address, and may use the status.
        let key = header_setting_key(&identity, "open-url");
        let mut configured = HeaderActionSettings::new();
        configured.insert(
            key.clone(),
            "https://example.test/redirect?next={{header.value}}&t=1".into(),
        );
        assert_eq!(
            run("open-url", "Location", "/cb", Some(302), &configured).unwrap(),
            HeaderAction::OpenUrl("https://example.test/redirect?next=/cb&t=1".into())
        );
        assert_eq!(
            header_menu(
                std::slice::from_ref(&plugin),
                &configured,
                "Location",
                "/cb",
                Some(302),
            )
            .iter()
            .map(|entry| entry.action_id.as_str())
            .collect::<Vec<_>>(),
            ["copy-value", "open-url"]
        );
        let mut with_status = HeaderActionSettings::new();
        with_status.insert(
            key,
            "https://example.test/s?code={{response.status}}&h={{header.name}}".into(),
        );
        assert_eq!(
            run("open-url", "X-Trace", "trace-1", Some(404), &with_status).unwrap(),
            HeaderAction::OpenUrl("https://example.test/s?code=404&h=X-Trace".into())
        );
        // The status is part of the menu decision too, so the two views cannot drift apart.
        assert!(
            header_menu(
                std::slice::from_ref(&plugin),
                &with_status,
                "X-Trace",
                "trace-1",
                Some(404),
            )
            .iter()
            .any(|entry| entry.action_id == "open-url")
        );
    }

    #[test]
    fn header_names_setting_narrows_the_menu_for_the_current_install() {
        let plugin = bundled_header_plugin();
        let identity = plugin.identity();
        let config = header_action_configs(&plugin)
            .into_iter()
            .find(|config| config.action_id == "open-url")
            .unwrap();
        // The wildcard is the plugin default; the detail page edits it per installation.
        assert_eq!(config.default_headers, "*");
        assert_eq!(
            config.headers_key,
            header_names_setting_key(&identity, "open-url")
        );
        assert_eq!(config.template_label(), "打开 URL 模板");

        fn ids(plugin: &Plugin, settings: &HeaderActionSettings, name: &str) -> Vec<String> {
            header_menu(
                std::slice::from_ref(plugin),
                settings,
                name,
                "https://example.test/cb",
                Some(302),
            )
            .into_iter()
            .map(|entry| entry.action_id)
            .collect()
        }
        let mut settings = HeaderActionSettings::new();
        settings.insert(
            config.headers_key.clone(),
            "location, content-location".into(),
        );
        assert_eq!(
            ids(&plugin, &settings, "Location"),
            ["copy-value".to_owned(), "open-url".to_owned()]
        );
        assert_eq!(
            ids(&plugin, &settings, "Content-Location"),
            ["copy-value".to_owned(), "open-url".to_owned()]
        );
        // The copy action keeps its own wildcard, so only the narrowed row disappears.
        assert_eq!(
            ids(&plugin, &settings, "content-type"),
            ["copy-value".to_owned()]
        );

        // A click re-checks the same effective list, so a menu left open cannot act elsewhere.
        let entry = HeaderEntry {
            identity: identity.clone(),
            action_id: "open-url".into(),
            title: String::new(),
        };
        let run = |name: &str, settings: &HeaderActionSettings| {
            run_header_action(
                std::slice::from_ref(&plugin),
                settings,
                &entry,
                name,
                "https://example.test/cb",
                Some(302),
                &no_variables(),
            )
        };
        assert!(run("location", &settings).is_ok());
        assert!(run("x-other", &settings).unwrap_err().contains("不匹配"));

        // An unusable list hides the row and names the reason instead of matching nothing.
        let mut broken = HeaderActionSettings::new();
        broken.insert(config.headers_key.clone(), "location/".into());
        assert!(
            ids(&plugin, &broken, "location")
                .iter()
                .all(|id| id != "open-url")
        );
        assert!(run("location", &broken).unwrap_err().contains("匹配响应头"));

        // The copy template is an installation setting too, with no address requirement.
        let mut copy = HeaderActionSettings::new();
        copy.insert(
            header_setting_key(&identity, "copy-value"),
            "{{header.name}}: {{header.value}}".into(),
        );
        let copy_entry = HeaderEntry {
            identity: identity.clone(),
            action_id: "copy-value".into(),
            title: String::new(),
        };
        assert_eq!(
            run_header_action(
                std::slice::from_ref(&plugin),
                &copy,
                &copy_entry,
                "Location",
                "/cb",
                Some(302),
                &no_variables(),
            )
            .unwrap(),
            HeaderAction::Copy("Location: /cb".into())
        );

        // The same rule applies to copy: an unusable template hides the row instead of failing later.
        let mut broken_copy = HeaderActionSettings::new();
        broken_copy.insert(
            header_setting_key(&identity, "copy-value"),
            "{{env.token}}".into(),
        );
        assert!(
            ids(&plugin, &broken_copy, "location")
                .iter()
                .all(|id| id != "copy-value")
        );

        // Typed lists are normalised, so the stored value always round-trips through the field.
        for (text, expected) in [
            ("*", "*"),
            (" Location ", "location"),
            ("location, Location", "location"),
            ("location · content-location", "location, content-location"),
            ("location，content-location", "location, content-location"),
        ] {
            assert_eq!(
                HeaderNames::parse_setting(text).unwrap().setting_text(),
                expected,
                "{text}"
            );
        }
        for text in ["", "   ", "location/", "x@y", "内容"] {
            assert!(validate_header_names_setting(text).is_err(), "{text}");
        }
        let too_many = (0..MAX_HEADER_NAMES + 1)
            .map(|index| format!("x-h{index}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(validate_header_names_setting(&too_many).is_err());
        assert!(validate_header_names_setting("location").is_ok());
        assert!(validate_header_copy_template("").is_ok());
        assert!(validate_header_copy_template("{{header.name}}: {{header.value}}").is_ok());
        assert!(validate_header_copy_template("{{env.token}}").is_err());
    }

    #[test]
    fn declarative_header_actions_reject_unusable_templates() {
        let declarative = |data: Value| {
            header_action_plugin(
                json!([{
                    "id": "action",
                    "kind": "response.header.action",
                    "title": "动作",
                    "data": data
                }]),
                &["response.read"],
                "",
            )
        };
        for data in [
            json!({ "header": "location", "action": "copy", "text": "{{env.token}}" }),
            json!({ "header": "location", "action": "copy", "text": "{{header.value" }),
            json!({ "header": "location", "action": "open-url", "url": "ftp://example.test/x" }),
            json!({ "header": "location", "action": "open-url", "text": "{{header.value}}" }),
            json!({ "header": "location", "action": "copy", "url": "https://example.test" }),
            json!({ "header": "location", "action": "run" }),
            json!({ "header": "location", "text": "{{header.value}}" }),
        ] {
            assert!(
                validate_manifest(&declarative(data.clone()).manifest).is_err(),
                "{data}"
            );
        }
        // An action and a hook are mutually exclusive; the empty script proves no worker runs.
        let both = header_action_plugin(
            json!([{
                "id": "action",
                "kind": "response.header.action",
                "title": "动作",
                "hook": "response.header.action",
                "data": { "header": "location", "action": "copy" }
            }]),
            &["response.read"],
            HEADER_HOOK,
        );
        assert!(validate_manifest(&both.manifest).is_err());
        // Empty text and empty header values are refused instead of copying nothing.
        let empty = declarative(json!({
            "header": "location", "action": "copy", "text": "{{header.value}}"
        }));
        assert!(
            run_header_action(
                &[empty],
                &HeaderActionSettings::new(),
                &HeaderEntry {
                    identity: "com.example.header-actions".into(),
                    action_id: "action".into(),
                    title: String::new(),
                },
                "location",
                "",
                None,
                &no_variables(),
            )
            .unwrap_err()
            .contains("为空")
        );
        assert!(expand_header_template("{{header.value}}", "a", "b", None).is_ok());
        assert!(expand_header_template("{{unknown}}", "a", "b", None).is_err());
        assert!(expand_header_template("{{header.value", "a", "b", None).is_err());
        assert!(validate_header_url_template("").is_ok());
        assert!(validate_header_url_template(DEFAULT_HEADER_TEMPLATE).is_ok());
        assert!(
            validate_header_url_template("https://example.test?q={{header.value}}&t=1").is_ok()
        );
        assert!(validate_header_url_template("example.test?q={{header.value}}").is_err());
        assert!(validate_header_url_template("https://example.test/{{x}}").is_err());
    }
}

#[cfg(test)]
mod event_process_tests {
    use super::*;
    #[test]
    #[ignore = "requires cargo build and GLINT_API_TEST_EXECUTABLE"]
    fn native_event_decoder_worker_verifies_and_decrypts_full_payload() {
        let executable = PathBuf::from(std::env::var_os("GLINT_API_TEST_EXECUTABLE").unwrap());
        let fixture: Value = serde_json::from_str(include_str!(
            "../plugins/wps-event-decrypt/tests/fixture.json"
        ))
        .unwrap();
        let mut job = Job {
            plugin: catalog()
                .0
                .into_iter()
                .find(|p| p.manifest.id == "com.wps.event-decrypt")
                .unwrap(),
            action_id: "decrypt-event".into(),
            request: fixture["event"].clone(),
            variables: BTreeMap::from([
                (
                    "wps_app_id".into(),
                    fixture["app_id"].as_str().unwrap().into(),
                ),
                (
                    "wps_app_key".into(),
                    fixture["app_key"].as_str().unwrap().into(),
                ),
            ]),
        };
        let output = isolated_at(&job, &executable, Duration::from_secs(3)).unwrap();
        assert_eq!(output["data"]["data"], fixture["data"]);
        job.plugin.source =
            "glint.on('websocket.message.decode',()=>({data:{text:'x'.repeat(100000)}}));".into();
        let output = isolated_at(&job, &executable, Duration::from_secs(3)).unwrap();
        assert_eq!(output["data"]["text"].as_str().unwrap().len(), 100000);
    }
}
