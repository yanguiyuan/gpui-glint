//! Signed software release feeds. Artifact URLs are derived from a configured source, never metadata.
use crate::{MAX_INDEX, Result, SignedIndex, Source, decode_key, digest, now};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Signer, SigningKey};
use serde::{Deserialize, Serialize};
pub const MAX_ARTIFACT: u64 = 200 * 1024 * 1024;
pub const MAX_VERSIONS: usize = 256;
pub fn valid_app(s: &str) -> bool {
    crate::valid_alias(s)
}
pub fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 160
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub target: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
}
impl Artifact {
    pub fn validate(&self) -> Result<()> {
        if !valid_name(&self.name)
            || !valid_name(&self.target)
            || self.target.len() > 64
            || self.size == 0
            || self.size > MAX_ARTIFACT
            || self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("Invalid software artifact".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Release {
    pub version: String,
    pub notes: String,
    #[serde(default)]
    pub published_at: u64,
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub yanked: bool,
}
impl Release {
    pub fn validate(&self) -> Result<()> {
        if self.version.len() > 100
            || semver::Version::parse(&self.version).is_err()
            || self.notes.len() > 8192
            || self.artifacts.is_empty()
            || self.artifacts.len() > 8
        {
            return Err("Invalid software release".into());
        }
        let mut targets = std::collections::BTreeSet::new();
        for a in &self.artifacts {
            a.validate()?;
            if !targets.insert(&a.target) {
                return Err("Duplicate software target".into());
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feed {
    pub format: u32,
    pub app_id: String,
    pub source_id: String,
    pub sequence: u64,
    pub issued_at: u64,
    pub expires_at: u64,
    pub releases: Vec<Release>,
}
pub fn sign(feed: &Feed, key: &SigningKey) -> Result<SignedIndex> {
    let bytes = serde_json::to_vec(feed).map_err(|_| "Cannot encode software feed")?;
    if bytes.len() > MAX_INDEX {
        return Err("Software feed is full".into());
    }
    Ok(SignedIndex {
        payload: STANDARD.encode(&bytes),
        signature: STANDARD.encode(key.sign(&bytes).to_bytes()),
    })
}
pub fn verify(signed: &SignedIndex, key: &str, app: &str, minimum_sequence: u64) -> Result<Feed> {
    if signed.payload.len() > MAX_INDEX * 4 / 3 + 4 || signed.signature.len() != 88 {
        return Err("Software feed exceeds limit".into());
    }
    let key = decode_key(key)?;
    let bytes = STANDARD
        .decode(&signed.payload)
        .map_err(|_| "Invalid software feed encoding")?;
    let signature = Signature::from_slice(
        &STANDARD
            .decode(&signed.signature)
            .map_err(|_| "Invalid software signature")?,
    )
    .map_err(|_| "Invalid software signature")?;
    key.verify_strict(&bytes, &signature)
        .map_err(|_| "Software signature verification failed")?;
    let feed: Feed = serde_json::from_slice(&bytes).map_err(|_| "Invalid software feed")?;
    let time = now();
    if feed.format != 1
        || !valid_app(app)
        || feed.app_id != app
        || feed.source_id != digest(&key.to_bytes())
        || feed.sequence < minimum_sequence
        || feed.issued_at > time.saturating_add(300)
        || feed.expires_at <= time
        || feed.expires_at <= feed.issued_at
        || feed.expires_at - feed.issued_at > 86400
        || feed.releases.len() > MAX_VERSIONS
    {
        return Err("Software feed is incompatible, expired or rolled back".into());
    }
    let mut versions = std::collections::BTreeSet::new();
    for r in &feed.releases {
        r.validate()?;
        if !versions.insert(&r.version) {
            return Err("Duplicate software release".into());
        }
    }
    Ok(feed)
}
pub fn feed_url(source: &Source, app: &str) -> Result<String> {
    source.validate()?;
    if !valid_app(app) {
        return Err("Invalid application ID".into());
    }
    Ok(format!(
        "{}/v1/apps/{app}/releases",
        source.url.trim_end_matches('/')
    ))
}
pub fn artifact_url(source: &Source, app: &str, artifact: &Artifact) -> Result<String> {
    feed_url(source, app)?;
    artifact.validate()?;
    Ok(format!(
        "{}/v1/apps/{app}/assets/{}/{}",
        source.url.trim_end_matches('/'),
        artifact.sha256,
        artifact.name
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signature_binds_app_origin_sequence_and_expiry() {
        let key = SigningKey::from_bytes(&[7; 32]);
        let info = crate::info(&key);
        let time = now();
        let mut feed = Feed {
            format: 1,
            app_id: "glint-api".into(),
            source_id: info.source_id,
            sequence: 3,
            issued_at: time,
            expires_at: time + 3600,
            releases: vec![],
        };
        let signed = sign(&feed, &key).unwrap();
        verify(&signed, &info.public_key, "glint-api", 3).unwrap();
        assert!(verify(&signed, &info.public_key, "another-app", 0).is_err());
        assert!(verify(&signed, &info.public_key, "glint-api", 4).is_err());
        assert!(
            verify(
                &signed,
                &crate::info(&SigningKey::from_bytes(&[8; 32])).public_key,
                "glint-api",
                0
            )
            .is_err()
        );
        feed.expires_at = time - 1;
        assert!(
            verify(
                &sign(&feed, &key).unwrap(),
                &info.public_key,
                "glint-api",
                0
            )
            .is_err()
        );
    }
}
