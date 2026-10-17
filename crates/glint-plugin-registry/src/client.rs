use crate::*;
use reqwest::blocking::{Client as HttpClient, Response};
use sha2::Digest;
use std::{io::Read, time::Duration};
#[derive(Clone)]
pub struct Client {
    http: HttpClient,
}
impl Client {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: HttpClient::builder()
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "Cannot initialize registry HTTP client")?,
        })
    }
    fn endpoint(source: &Source, path: &str) -> Result<String> {
        source.validate()?;
        Ok(format!("{}/v1/{path}", source.url.trim_end_matches('/')))
    }
    fn bytes(response: Response, limit: usize) -> Result<Vec<u8>> {
        if !response.status().is_success() {
            return Err(format!(
                "Registry request failed (HTTP {})",
                response.status().as_u16()
            ));
        }
        if response.content_length().is_some_and(|n| n > limit as u64) {
            return Err("Registry response too large".into());
        }
        let mut bytes = vec![];
        response
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read registry response")?;
        if bytes.len() > limit {
            return Err("Registry response too large".into());
        }
        Ok(bytes)
    }
    fn management<T: serde::de::DeserializeOwned, B: serde::Serialize>(
        &self,
        source: &Source,
        token: &str,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let mut auth = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| "Invalid management token")?;
        auth.set_sensitive(true);
        let response = self
            .http
            .post(Self::endpoint(source, path)?)
            .header(reqwest::header::AUTHORIZATION, auth)
            .json(body)
            .send()
            .map_err(|_| "Software management request failed")?;
        serde_json::from_slice(&Self::bytes(response, MAX_METADATA)?)
            .map_err(|_| "Invalid software response".into())
    }
    pub fn software_feed(
        &self,
        source: &Source,
        app: &str,
        minimum_sequence: u64,
    ) -> Result<software::Feed> {
        let response = self
            .http
            .get(software::feed_url(source, app)?)
            .send()
            .map_err(|_| "Cannot reach software source")?;
        let signed: SignedIndex = serde_json::from_slice(&Self::bytes(response, MAX_INDEX * 2)?)
            .map_err(|_| "Invalid software feed envelope")?;
        software::verify(&signed, &source.public_key, app, minimum_sequence)
    }
    pub fn software_upload(
        &self,
        source: &Source,
        token: &str,
        app: &str,
        path: &std::path::Path,
        target: &str,
    ) -> Result<software::Artifact> {
        source.validate()?;
        if !software::valid_app(app) || !software::valid_name(target) {
            return Err("Invalid software target or app".into());
        }
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid artifact filename")?;
        if !software::valid_name(name) {
            return Err("Invalid artifact filename".into());
        }
        let mut file = std::fs::File::open(path).map_err(|_| "Cannot open software artifact")?;
        let size = file
            .metadata()
            .map_err(|_| "Cannot read artifact size")?
            .len();
        if size == 0 || size > software::MAX_ARTIFACT {
            return Err("Invalid artifact size".into());
        }
        let mut hash = sha2::Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let n = file.read(&mut buffer).map_err(|_| "Cannot hash artifact")?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        let digest = format!("{:x}", hash.finalize());
        let mut auth = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| "Invalid management token")?;
        auth.set_sensitive(true);
        let response = self
            .http
            .post(format!(
                "{}/v1/apps/{app}/assets/{name}",
                source.url.trim_end_matches('/')
            ))
            .query(&[("target", target)])
            .header(reqwest::header::AUTHORIZATION, auth)
            .header(reqwest::header::CONTENT_LENGTH, size)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .timeout(Duration::from_secs(1800))
            .body(reqwest::blocking::Body::new(
                std::fs::File::open(path).map_err(|_| "Cannot read software artifact")?,
            ))
            .send()
            .map_err(|_| "Cannot upload software artifact")?;
        let artifact: software::Artifact = serde_json::from_slice(&Self::bytes(response, 8192)?)
            .map_err(|_| "Invalid artifact response")?;
        artifact.validate()?;
        if artifact.sha256 != digest
            || artifact.size != size
            || artifact.name != name
            || artifact.target != target
        {
            return Err("Uploaded software artifact verification failed".into());
        }
        Ok(artifact)
    }
    pub fn software_publish(
        &self,
        source: &Source,
        token: &str,
        app: &str,
        release: &software::Release,
    ) -> Result<software::Release> {
        software::feed_url(source, app)?;
        release.validate()?;
        self.management(source, token, &format!("apps/{app}/releases"), release)
    }
    pub fn software_yank(
        &self,
        source: &Source,
        token: &str,
        app: &str,
        version: &str,
        yanked: bool,
    ) -> Result<software::Release> {
        software::feed_url(source, app)?;
        if semver::Version::parse(version).is_err() || version.len() > 100 {
            return Err("Invalid software version".into());
        }
        self.management(
            source,
            token,
            &format!(
                "apps/{app}/releases/{version}/{}",
                if yanked { "yank" } else { "restore" }
            ),
            &serde_json::json!({}),
        )
    }
    pub fn inspect(&self, url: &str, allow_http: bool) -> Result<Info> {
        validate_url(url, allow_http)?;
        let response = self
            .http
            .get(format!("{}/v1/info", url.trim_end_matches('/')))
            .send()
            .map_err(|_| "Cannot reach registry")?;
        let info: Info = serde_json::from_slice(&Self::bytes(response, 8192)?)
            .map_err(|_| "Invalid registry identity")?;
        if info.protocol != 1 || info.source_id != digest(&decode_key(&info.public_key)?.to_bytes())
        {
            return Err("Invalid registry identity".into());
        }
        Ok(info)
    }
    pub fn fetch(&self, source: &Source) -> Result<SignedIndex> {
        let response = self
            .http
            .get(Self::endpoint(source, "index")?)
            .send()
            .map_err(|_| "Cannot fetch registry index")?;
        serde_json::from_slice(&Self::bytes(response, MAX_INDEX * 2)?)
            .map_err(|_| "Invalid signed index response".into())
    }
    pub fn download(&self, source: &Source, release: &Release) -> Result<Vec<u8>> {
        release.validate()?;
        if release.yanked {
            return Err("This release is withdrawn".into());
        }
        let response = self
            .http
            .get(Self::endpoint(
                source,
                &format!("packages/{}", release.sha256),
            )?)
            .send()
            .map_err(|_| "Cannot download plugin")?;
        let bytes = Self::bytes(response, MAX_PACKAGE)?;
        if bytes.len() as u64 != release.size || digest(&bytes) != release.sha256 {
            return Err("Plugin package verification failed".into());
        }
        let package = Package::decode(&bytes)?;
        if serde_json::to_value(&package.plugin.manifest).ok()
            != serde_json::to_value(&release.manifest).ok()
        {
            return Err("Package manifest differs from signed metadata".into());
        }
        Ok(bytes)
    }
    fn admin(
        &self,
        source: &Source,
        token: &str,
        path: &str,
        body: Option<&Publish>,
    ) -> Result<Release> {
        let mut authorization = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| "Invalid publish token")?;
        authorization.set_sensitive(true);
        let mut request = self
            .http
            .post(Self::endpoint(source, path)?)
            .header(reqwest::header::AUTHORIZATION, authorization);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .map_err(|_| "Registry management request failed")?;
        serde_json::from_slice(&Self::bytes(response, MAX_METADATA)?)
            .map_err(|_| "Invalid release response".into())
    }
    pub fn publish(&self, source: &Source, token: &str, publish: &Publish) -> Result<Release> {
        self.admin(source, token, "publish", Some(publish))
    }
    pub fn set_yanked(
        &self,
        source: &Source,
        token: &str,
        id: &str,
        version: &str,
        yanked: bool,
    ) -> Result<Release> {
        if !safe_plugin_id(id) || semver::Version::parse(version).is_err() {
            return Err("Invalid plugin ID/version".into());
        }
        self.admin(
            source,
            token,
            &format!(
                "plugins/{id}/{version}/{}",
                if yanked { "yank" } else { "restore" }
            ),
            None,
        )
    }
}
pub fn safe_plugin_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}
