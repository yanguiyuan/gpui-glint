//! 外部供应商配置文件：`~/.glint-workbench/providers.json`。
//!
//! 文件属于 glint，外部工具（Magpie 的 Agent 插件之类）往里写供应商，glint 只读它，
//! 把里面的供应商合并进本地供应商目录：同 id 覆盖、文件里删掉的从目录里移除。
//! 用户的供应商不受影响，API key 也从文件搬进本地密钥库，不留在供应商元数据里。
//!
//! ```json
//! {
//!   "model": "选中模型（顶层兜底，给没有自己 model 的供应商）",
//!   "active": "magpie（可选，id 或 name）",
//!   "providers": [
//!     {
//!       "id": "magpie（可选，缺省用 name）",
//!       "name": "Magpie",
//!       "base_url": "http://127.0.0.1:3425/v1",
//!       "api_key": "magpie（可选，写进本地密钥库）",
//!       "protocol": "OpenAi（可选）",
//!       "model": "…（可选）",
//!       "models": ["…"],
//!       "models_url": "…（可选）",
//!       "model_context_windows": {"模型": 1000000},
//!       "context_window_overrides": {"模型": 131072},
//!       "response_timeout_seconds": 180,
//!       "enabled": true
//!     }
//!   ]
//! }
//! ```

use crate::{
    provider::{Config, Kind, Protocol},
    provider_catalog::ProviderCatalog,
};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const CONFIG_FILE: &str = "providers.json";
/// 外部来源标记，用来认出（并在下次启动时清掉）从这个文件合并进来的供应商。
const SOURCE: &str = "providers-file";
/// Magpie 网关的默认地址和共享 token，文件里没写时用它们兜底。
const DEFAULT_GATEWAY: &str = "http://127.0.0.1:3425/v1";
const DEFAULT_KEY: &str = "magpie";
const DEFAULT_NAME: &str = "Magpie";

/// 文件固定放在 glint 的工作目录下，和供应商数据库（数据目录里的 providers.json）
/// 分开，避免同名冲突。`GLINT_PROVIDERS_FILE` 可以指到别处。
pub fn config_path() -> PathBuf {
    if let Some(path) = std::env::var_os("GLINT_PROVIDERS_FILE").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    home()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".glint-workbench")
        .join(CONFIG_FILE)
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[derive(Default, Deserialize)]
struct FileConfig {
    #[serde(default)]
    model: String,
    #[serde(default)]
    active: String,
    #[serde(default)]
    providers: Vec<Entry>,
}

#[derive(Clone, Default, Deserialize)]
struct Entry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    protocol: Option<Protocol>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    models_url: Option<String>,
    #[serde(default)]
    model_context_windows: HashMap<String, u64>,
    #[serde(default)]
    context_window_overrides: HashMap<String, u64>,
    #[serde(default)]
    response_timeout_seconds: Option<u64>,
    #[serde(default)]
    enabled: Option<bool>,
}

/// Magpie 的配置目录，只读，用来推出网关地址。
fn magpie_config_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(xdg).join("magpie"));
    }
    Some(home()?.join(".config").join("magpie"))
}

/// `gateway.json` 里是 `{"addr":"127.0.0.1:3425", …}`。
fn magpie_gateway() -> Option<String> {
    let bytes = std::fs::read(magpie_config_dir()?.join("gateway.json")).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let addr = value.get("addr")?.as_str()?.trim();
    if addr.is_empty() {
        return None;
    }
    Some(if addr.starts_with("http") {
        addr.trim_end_matches('/').to_owned()
    } else {
        format!("http://{}/v1", addr.trim_end_matches('/'))
    })
}

fn default_gateway() -> String {
    magpie_gateway().unwrap_or_else(|| DEFAULT_GATEWAY.to_owned())
}

/// Magpie 只在配置文件已存在时才会写入，不会自己创建，所以启动时先放一个空的。
/// 已经存在的文件不碰；没装 Magpie 时不留下任何文件。
pub fn ensure_config() {
    let installed = magpie_config_dir().map(|dir| dir.exists()).unwrap_or(false);
    if !installed {
        return;
    }
    let path = config_path();
    if path.exists() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, b"{\"providers\": []}\n");
}

/// 由名称或 id 得出稳定的供应商 id，重启后仍是同一个。
fn stable_id(seed: &str) -> Uuid {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let mut hash = OFFSET;
    for byte in seed.trim().to_lowercase().bytes() {
        hash ^= u128::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    Uuid::from_u128(if hash == 0 { 1 } else { hash })
}

fn entry_id(entry: &Entry) -> Uuid {
    let seed = if entry.id.trim().is_empty() {
        entry.name.trim()
    } else {
        entry.id.trim()
    };
    stable_id(seed)
}

fn build(anchor: &Path, entry: &Entry, fallback_model: &str) -> Option<Config> {
    let base_url = entry.base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        return None;
    }
    let name = if entry.name.trim().is_empty() {
        DEFAULT_NAME
    } else {
        entry.name.trim()
    };
    let model = if entry.model.trim().is_empty() {
        fallback_model.trim()
    } else {
        entry.model.trim()
    };
    let mut config = Config::default();
    config.response_timeout_seconds = entry.response_timeout_seconds.unwrap_or(180);
    config.credential_store = Some(anchor.to_path_buf());
    config.models_url = entry
        .models_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned);
    config.protocol = entry.protocol.unwrap_or_default();
    config.discovery_id = Some(SOURCE.to_owned());
    config.id = entry_id(entry);
    config.models = entry
        .models
        .iter()
        .map(|model| model.trim().to_owned())
        .filter(|model| !model.is_empty())
        .collect();
    config.model_context_windows = entry.model_context_windows.clone();
    config.context_window_overrides = entry.context_window_overrides.clone();
    config.kind = Kind::Compatible;
    config.enabled = entry.enabled.unwrap_or(true);
    config.name = name.to_owned();
    config.base_url = base_url.to_owned();
    config.model = model.to_owned();
    config.credential_id = config.id;
    Some(config)
}

/// 文件里写明的供应商；没写 providers 时退化成一条默认的 Magpie 供应商
/// （Magpie 的某些路径只把选中的模型写进文件）。
fn entries(file: &FileConfig) -> Vec<Entry> {
    if file.providers.is_empty() {
        let model = file.model.trim();
        if model.is_empty() {
            return vec![];
        }
        let base_url = default_gateway();
        return vec![Entry {
            name: DEFAULT_NAME.to_owned(),
            base_url: base_url.clone(),
            api_key: DEFAULT_KEY.to_owned(),
            models_url: Some(format!("{base_url}/models")),
            model: model.to_owned(),
            ..Entry::default()
        }];
    }
    let mut entries = file.providers.clone();
    if !file.model.trim().is_empty() {
        if let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry.model.trim().is_empty())
        {
            entry.model = file.model.trim().to_owned();
        }
    }
    entries
}

fn remove(anchor: &Path, catalog: &mut ProviderCatalog, id: Uuid) {
    let Some(index) = catalog
        .providers
        .iter()
        .position(|provider| provider.id == id)
    else {
        return;
    };
    if catalog.active == Some(id) {
        let next = catalog
            .providers
            .iter()
            .find(|provider| provider.id != id && provider.enabled)
            .map(|provider| (provider.id, provider.model.clone()));
        match next {
            Some((id, model)) => {
                catalog.active = Some(id);
                catalog.active_model = model;
            }
            None => {
                catalog.active = None;
                catalog.active_model.clear();
            }
        }
    }
    catalog.providers.remove(index);
    let mut config = Config::default();
    config.credential_store = Some(anchor.to_path_buf());
    config.credential_id = id;
    let _ = config.remove_key();
}

#[derive(Default)]
pub struct State {
    signature: Option<Vec<u8>>,
    previous_active: Option<Uuid>,
    injected: HashSet<Uuid>,
    pub error: Option<String>,
}

impl State {
    /// 这个供应商是否来自配置文件。
    pub fn is_injected(&self, id: Uuid) -> bool {
        self.injected.contains(&id)
    }

    pub fn connected(&self) -> bool {
        !self.injected.is_empty()
    }
}

/// 把文件里的供应商合并进目录。返回 true 表示有变化。
pub fn apply(anchor: &Path, catalog: &mut ProviderCatalog, state: &mut State) -> bool {
    let bytes = std::fs::read(config_path()).unwrap_or_default();
    let parsed = if bytes.is_empty() {
        Ok(None)
    } else {
        serde_json::from_slice::<FileConfig>(&bytes)
            .map(Some)
            .map_err(|error| format!("供应商配置文件解析失败：{error}"))
    };
    // 兜底网关地址可能来自 Magpie 的 gateway.json：它变了也要重新合并。
    let mut signature = bytes.clone();
    signature.push(0);
    signature.extend_from_slice(default_gateway().as_bytes());
    if state.signature.as_deref() == Some(signature.as_slice()) {
        return false;
    }
    state.signature = Some(signature);
    state.error = None;

    let file = match parsed {
        Ok(file) => file.unwrap_or_default(),
        Err(error) => {
            state.error = Some(error);
            return true;
        }
    };

    let entries = entries(&file);
    let mut injected = HashSet::new();
    let mut merged = Vec::new();
    for entry in &entries {
        let Some(config) = build(anchor, entry, &file.model) else {
            continue;
        };
        let id = config.id;
        let model = config.model.clone();
        let key = entry.api_key.trim();
        if !key.is_empty() {
            if let Err(error) = config.save_key(key) {
                state.error = Some(error);
            }
        }
        catalog.upsert(config);
        injected.insert(id);
        merged.push((
            id,
            model,
            entry.id.trim().to_owned(),
            entry.name.trim().to_owned(),
        ));
    }

    // 文件里已经去掉的供应商，从目录里拿掉（包括上次运行留下的）。
    let stale: Vec<Uuid> = catalog
        .providers
        .iter()
        .filter(|provider| {
            provider.discovery_id.as_deref() == Some(SOURCE) && !injected.contains(&provider.id)
        })
        .map(|provider| provider.id)
        .collect();
    for id in stale {
        remove(anchor, catalog, id);
    }

    if merged.is_empty() {
        state.injected.clear();
        if let Some(previous) = state.previous_active.take() {
            if let Some(provider) = catalog
                .providers
                .iter()
                .find(|provider| provider.id == previous)
                .map(|provider| (provider.id, provider.model.clone()))
            {
                let _ = catalog.activate(provider.0, provider.1);
            }
        }
        if let Err(error) = catalog.save(anchor) {
            state.error = Some(error);
        }
        return true;
    }

    let previous = std::mem::take(&mut state.injected);
    if previous.is_empty() {
        // 第一次合并：记住用户原来的选择，文件清空后能换回去。
        state.previous_active = catalog.active.filter(|id| !injected.contains(id));
    }

    let wanted = file.active.trim();
    let explicit = (!wanted.is_empty())
        .then(|| {
            merged.iter().find(|(_, _, id, name)| {
                id.eq_ignore_ascii_case(wanted) || name.eq_ignore_ascii_case(wanted)
            })
        })
        .flatten();
    let target = explicit.or_else(|| merged.first());
    if let Some((id, model, _, _)) = target {
        if let Err(error) = catalog.activate(*id, model.clone()) {
            state.error = Some(error);
        }
    }

    state.injected = injected;
    if let Err(error) = catalog.save(anchor) {
        state.error = Some(error);
    }
    true
}
