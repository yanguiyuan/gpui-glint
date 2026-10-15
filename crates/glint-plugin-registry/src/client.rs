use crate::*;
use reqwest::blocking::{Client as HttpClient, Response};
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
