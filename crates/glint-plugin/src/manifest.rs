use crate::{PluginError, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

/// Declarative entry exposed by a plugin. The host decides which kinds it supports.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contribution {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub hook: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: u32,
    pub entry: String,
    #[serde(default)]
    pub capabilities: BTreeSet<String>,
    pub hooks: BTreeSet<String>,
    #[serde(default)]
    pub contributions: Vec<Contribution>,
}
impl Manifest {
    pub fn validate(&self) -> Result<()> {
        let valid_id = |s: &str| {
            !s.is_empty()
                && !s.split('.').any(str::is_empty)
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        };
        if !valid_id(&self.id) || self.name.trim().is_empty() || self.name.len() > 256 {
            return Err(PluginError("Invalid plugin identity".into()));
        }
        if self.api_version != 1 {
            return Err(PluginError("Unsupported plugin API version".into()));
        }
        semver::Version::parse(&self.version)
            .map_err(|_| PluginError("Invalid plugin version".into()))?;
        if self.entry.is_empty()
            || self.entry.contains('\\')
            || !Path::new(&self.entry)
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
        {
            return Err(PluginError(
                "Plugin entry must stay inside its directory".into(),
            ));
        }
        if self.hooks.is_empty()
            || self.hooks.iter().any(|h| !valid_id(h))
            || self
                .capabilities
                .iter()
                .any(|c| c.is_empty() || c.len() > 256)
        {
            return Err(PluginError("Invalid hooks or capabilities".into()));
        }
        if self.contributions.len() > 64 {
            return Err(PluginError("Too many contributions".into()));
        }
        let mut ids = BTreeSet::new();
        for c in &self.contributions {
            if !valid_id(&c.id)
                || !valid_id(&c.kind)
                || c.title.trim().is_empty()
                || c.title.len() > 256
                || !self.hooks.contains(&c.hook)
                || !ids.insert(&c.id)
            {
                return Err(PluginError("Invalid plugin contribution".into()));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plugin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    pub manifest: Manifest,
    pub source: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    pub source_id: String,
    pub package_sha256: String,
}
impl Plugin {
    pub fn family_identity(&self) -> String {
        match &self.origin {
            Some(origin) => format!("{}::{}", origin.source_id, self.manifest.id),
            None => self.manifest.id.clone(),
        }
    }
    pub fn identity(&self) -> String {
        match &self.origin {
            Some(origin) => format!(
                "{}::{}::{}",
                origin.source_id, self.manifest.id, self.manifest.version
            ),
            None => self.manifest.id.clone(),
        }
    }

    pub fn from_source(manifest: &str, source: &str) -> Result<Self> {
        if manifest.len() > 64 * 1024 || source.len() > 256 * 1024 {
            return Err(PluginError("Plugin exceeds source limit".into()));
        }
        let manifest: Manifest = serde_json::from_str(manifest)
            .map_err(|_| PluginError("Invalid plugin manifest".into()))?;
        manifest.validate()?;
        Ok(Self {
            origin: None,
            manifest,
            source: source.into(),
        })
    }
    /// Loads inert manifest/source data; no script executes during discovery.
    pub fn load(directory: &Path) -> Result<Self> {
        let read = |path: &Path, limit: u64| -> Result<String> {
            use std::io::Read;
            let file = std::fs::File::open(path)
                .map_err(|_| PluginError("Cannot read plugin file".into()))?;
            let mut data = String::new();
            file.take(limit + 1)
                .read_to_string(&mut data)
                .map_err(|_| PluginError("Plugin file must be UTF-8".into()))?;
            if data.len() as u64 > limit {
                return Err(PluginError("Plugin file too large".into()));
            }
            Ok(data)
        };
        let root = directory
            .canonicalize()
            .map_err(|_| PluginError("Plugin directory is missing".into()))?;
        let manifest_path = root
            .join("glint-plugin.json")
            .canonicalize()
            .map_err(|_| PluginError("Plugin manifest is missing".into()))?;
        if !manifest_path.starts_with(&root) {
            return Err(PluginError("Manifest escapes plugin directory".into()));
        }
        let text = read(&manifest_path, 64 * 1024)?;
        let manifest: Manifest = serde_json::from_str(&text)
            .map_err(|_| PluginError("Invalid plugin manifest".into()))?;
        manifest.validate()?;
        let entry = root
            .join(&manifest.entry)
            .canonicalize()
            .map_err(|_| PluginError("Plugin entry is missing".into()))?;
        if !entry.starts_with(&root) {
            return Err(PluginError("Entry escapes plugin directory".into()));
        }
        Self::from_source(&text, &read(&entry, 256 * 1024)?)
    }
}

/// Persisted host approval: changing code or requested permissions requires renewed consent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    pub id: String,
    pub capabilities: BTreeSet<String>,
    pub fingerprint: String,
}
impl Approval {
    pub fn for_plugin(plugin: &Plugin) -> Self {
        use sha2::{Digest, Sha256};
        let bytes = serde_json::to_vec(plugin).expect("plugin data is serializable");
        Self {
            id: plugin.identity(),
            capabilities: plugin.manifest.capabilities.clone(),
            fingerprint: format!("{:x}", Sha256::digest(&bytes)),
        }
    }
    pub fn matches(&self, plugin: &Plugin) -> bool {
        self == &Self::for_plugin(plugin)
    }
}
