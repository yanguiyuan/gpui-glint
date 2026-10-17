//! Portable, signed registry protocol. No GPUI, JavaScript engine or database driver.
mod client;
pub mod software;
mod store;
use base64::{Engine as _, engine::general_purpose::STANDARD};
pub use client::Client;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use glint_plugin::{Manifest, Plugin};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
pub use store::{Installed, PluginStore};
pub type Result<T> = std::result::Result<T, String>;
pub const MAX_PACKAGE: usize = 512 * 1024;
pub const MAX_METADATA: usize = 128 * 1024;
pub const MAX_INDEX: usize = 4 * 1024 * 1024;
pub const MAX_RELEASES: usize = 1000;
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
pub fn valid_alias(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostTarget {
    pub id: String,
    pub min_version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub format: u32,
    pub plugin: Plugin,
}
impl Package {
    pub fn encode(plugin: Plugin) -> Result<Vec<u8>> {
        let package = Self { format: 1, plugin };
        let bytes = serde_json::to_vec(&package).map_err(|_| "Cannot encode plugin")?;
        Self::decode(&bytes)?;
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_PACKAGE {
            return Err("Plugin package exceeds 512 KiB".into());
        }
        let package: Self = serde_json::from_slice(bytes).map_err(|_| "Invalid plugin package")?;
        if package.format != 1 || package.plugin.origin.is_some() {
            return Err("Unsupported package format".into());
        }
        let validated = Plugin::from_source(
            &serde_json::to_string(&package.plugin.manifest).map_err(|_| "Invalid manifest")?,
            &package.plugin.source,
        )
        .map_err(|e| e.to_string())?;
        if validated.manifest.id != package.plugin.manifest.id {
            return Err("Invalid plugin identity".into());
        }
        Ok(package)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub manifest: Manifest,
    pub hosts: Vec<HostTarget>,
    pub sha256: String,
    pub size: u64,
    pub changelog: String,
    pub yanked: bool,
}
impl Release {
    pub fn validate(&self) -> Result<()> {
        self.manifest.validate().map_err(|e| e.to_string())?;
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.size == 0
            || self.size > MAX_PACKAGE as u64
            || self.changelog.len() > 8192
            || self.hosts.is_empty()
            || self.hosts.len() > 16
        {
            return Err("Invalid release metadata".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for host in &self.hosts {
            if !valid_alias(&host.id)
                || !ids.insert(&host.id)
                || semver::Version::parse(&host.min_version).is_err()
            {
                return Err("Invalid host compatibility".into());
            }
        }
        Ok(())
    }
    pub fn supports(&self, host: &str, version: &str) -> bool {
        let Ok(version) = semver::Version::parse(version) else {
            return false;
        };
        !self.yanked
            && self.manifest.api_version == 1
            && self.hosts.iter().any(|h| {
                h.id == host
                    && semver::Version::parse(&h.min_version)
                        .is_ok_and(|minimum| version >= minimum)
            })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Index {
    pub format: u32,
    pub source_id: String,
    pub sequence: u64,
    pub issued_at: u64,
    pub expires_at: u64,
    pub releases: Vec<Release>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedIndex {
    pub payload: String,
    pub signature: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Info {
    pub protocol: u32,
    pub source_id: String,
    pub public_key: String,
}
pub fn info(key: &SigningKey) -> Info {
    let bytes = key.verifying_key().to_bytes();
    Info {
        protocol: 1,
        source_id: digest(&bytes),
        public_key: STANDARD.encode(bytes),
    }
}
pub fn sign(index: &Index, key: &SigningKey) -> Result<SignedIndex> {
    let bytes = serde_json::to_vec(index).map_err(|_| "Cannot encode index")?;
    if bytes.len() > MAX_INDEX {
        return Err("Registry index is full".into());
    }
    Ok(SignedIndex {
        payload: STANDARD.encode(&bytes),
        signature: STANDARD.encode(key.sign(&bytes).to_bytes()),
    })
}
pub fn verify(
    signed: &SignedIndex,
    public_key: &str,
    minimum_sequence: u64,
    allow_expired: bool,
) -> Result<Index> {
    if signed.payload.len() > MAX_INDEX * 4 / 3 + 4 {
        return Err("Registry index too large".into());
    }
    if signed.signature.len() != 88 {
        return Err("Invalid signature encoding".into());
    }
    let key = decode_key(public_key)?;
    let bytes = STANDARD
        .decode(&signed.payload)
        .map_err(|_| "Invalid index encoding")?;
    let signature = Signature::from_slice(
        &STANDARD
            .decode(&signed.signature)
            .map_err(|_| "Invalid signature encoding")?,
    )
    .map_err(|_| "Invalid signature")?;
    key.verify_strict(&bytes, &signature)
        .map_err(|_| "Registry signature verification failed")?;
    let index: Index = serde_json::from_slice(&bytes).map_err(|_| "Invalid registry index")?;
    let timestamp = now();
    if index.format != 1
        || index.source_id != digest(&key.to_bytes())
        || index.sequence < minimum_sequence
        || index.issued_at > timestamp.saturating_add(300)
        || index.expires_at <= index.issued_at
        || index.expires_at - index.issued_at > 86400
        || (!allow_expired && index.expires_at <= timestamp)
        || index.releases.len() > MAX_RELEASES
    {
        return Err("Registry index is incompatible, expired or rolled back".into());
    }
    let mut versions = std::collections::BTreeSet::new();
    for release in &index.releases {
        release.validate()?;
        if !versions.insert((&release.manifest.id, &release.manifest.version)) {
            return Err("Duplicate registry release".into());
        }
    }
    Ok(index)
}
pub fn decode_key(text: &str) -> Result<VerifyingKey> {
    if text.len() != 44 {
        return Err("Invalid Ed25519 public key encoding".into());
    }
    let bytes: [u8; 32] = STANDARD
        .decode(text)
        .map_err(|_| "Invalid public key")?
        .try_into()
        .map_err(|_| "Public key must contain 32 bytes")?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| "Invalid Ed25519 public key".into())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub name: String,
    pub url: String,
    pub public_key: String,
    #[serde(default)]
    pub allow_http: bool,
}
impl Source {
    pub fn validate(&self) -> Result<()> {
        if !valid_alias(&self.name) {
            return Err("Source name must contain lowercase letters, digits, - or _".into());
        }
        decode_key(&self.public_key)?;
        validate_url(&self.url, self.allow_http)
    }
    pub fn id(&self) -> Result<String> {
        Ok(digest(&decode_key(&self.public_key)?.to_bytes()))
    }
}
pub fn validate_url(text: &str, allow_http: bool) -> Result<()> {
    if text.len() > 2048 {
        return Err("Registry URL too long".into());
    }
    let url = reqwest::Url::parse(text).map_err(|_| "Invalid registry URL")?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.scheme() == "https" || url.scheme() == "http" && (local || allow_http))
    {
        return Err(
            "Use an HTTPS base URL (HTTP requires explicit opt-in outside loopback)".into(),
        );
    }
    Ok(())
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publish {
    pub package: String,
    pub hosts: Vec<HostTarget>,
    pub changelog: String,
}
impl Publish {
    pub fn new(bytes: &[u8], hosts: Vec<HostTarget>, changelog: String) -> Self {
        Self {
            package: STANDARD.encode(bytes),
            hosts,
            changelog,
        }
    }
    pub fn decode(&self) -> Result<Vec<u8>> {
        if self.package.len() > MAX_PACKAGE * 4 / 3 + 4 {
            return Err("Package too large".into());
        }
        STANDARD
            .decode(&self.package)
            .map_err(|_| "Invalid package encoding".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn empty(key: &SigningKey) -> Index {
        let issued_at = now();
        Index {
            format: 1,
            source_id: info(key).source_id,
            sequence: 4,
            issued_at,
            expires_at: issued_at + 3600,
            releases: vec![],
        }
    }
    #[test]
    fn signatures_identity_expiry_and_rollback_are_enforced() {
        let key = SigningKey::from_bytes(&[11; 32]);
        let index = empty(&key);
        let signed = sign(&index, &key).unwrap();
        assert_eq!(
            verify(&signed, &info(&key).public_key, 4, false)
                .unwrap()
                .sequence,
            4
        );
        assert!(
            verify(
                &signed,
                &info(&SigningKey::from_bytes(&[12; 32])).public_key,
                0,
                false
            )
            .is_err()
        );
        assert!(verify(&signed, &info(&key).public_key, 5, false).is_err());
        let mut tampered = signed.clone();
        tampered.payload.push('A');
        assert!(verify(&tampered, &info(&key).public_key, 0, false).is_err());
        let mut expired = index.clone();
        expired.issued_at = now() - 7200;
        expired.expires_at = now() - 3600;
        let signed = sign(&expired, &key).unwrap();
        assert!(verify(&signed, &info(&key).public_key, 0, false).is_err());
        assert!(verify(&signed, &info(&key).public_key, 0, true).is_ok());
        let mut wrong = index;
        wrong.source_id = "another source".into();
        assert!(
            verify(
                &sign(&wrong, &key).unwrap(),
                &info(&key).public_key,
                0,
                false
            )
            .is_err()
        );
    }
    #[test]
    fn source_configuration_refuses_credential_urls_and_unapproved_http() {
        assert!(validate_url("http://example.com", false).is_err());
        assert!(validate_url("http://127.0.0.1:8787", false).is_ok());
        assert!(validate_url("https://user:password@example.com", false).is_err());
        assert!(validate_url("https://example.com?token=secret", false).is_err());
        assert!(validate_url("https://example.com/registry", false).is_ok());
    }
    #[test]
    fn packages_preserve_declarative_contributions_with_no_hooks() {
        let manifest = serde_json::json!({"id":"test.presets","name":"Presets","version":"1.0.0","api_version":1,"entry":"main.js","hooks":[],"contributions":[{"id":"provider","kind":"test.preset","title":"Provider","data":{"setting":"value"}}]});
        let plugin =
            Plugin::from_source(&manifest.to_string(), "throw new Error('never execute');")
                .unwrap();
        let bytes = Package::encode(plugin).unwrap();
        let decoded = Package::decode(&bytes).unwrap();
        assert!(decoded.plugin.manifest.hooks.is_empty());
        assert_eq!(
            decoded.plugin.manifest.contributions[0]
                .data
                .as_ref()
                .unwrap()["setting"],
            "value"
        );
    }
    #[test]
    fn packages_validate_data_without_executing_scripts() {
        let manifest = r#"{"id":"test.plugin","name":"Test","version":"1.0.0","api_version":1,"entry":"main.js","capabilities":[],"hooks":["test.run"]}"#;
        let plugin =
            Plugin::from_source(manifest, "throw new Error('Never execute on server');").unwrap();
        let bytes = Package::encode(plugin).unwrap();
        assert_eq!(
            Package::decode(&bytes).unwrap().plugin.manifest.id,
            "test.plugin"
        );
        let mut data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        data["format"] = 2.into();
        assert!(Package::decode(&serde_json::to_vec(&data).unwrap()).is_err());
        assert!(Plugin::from_source(&manifest.replace("test.plugin", ".."), "").is_err());
    }
}
