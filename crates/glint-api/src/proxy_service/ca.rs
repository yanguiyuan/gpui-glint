use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, PublicKeyData,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};

/// The identity is kept separate from workspace exports. Only the public certificate is exported.
pub struct Authority {
    issuer: Issuer<'static, KeyPair>,
    pub certificate: CertificateDer<'static>,
    pub fingerprint: String,
    pub name: String,
    pub expires: String,
    expires_at: time::OffsetDateTime,
    leaves: Mutex<HashMap<String, Arc<rustls::ServerConfig>>>,
}
impl Authority {
    pub fn parse(certificate: &str, private_key: &str) -> Result<Self, String> {
        let der = CertificateDer::from_pem_slice(certificate.as_bytes())
            .map_err(|e| format!("证书 PEM 无效：{e}"))?;
        let key_der = PrivateKeyDer::from_pem_slice(private_key.as_bytes())
            .map_err(|e| format!("私钥 PEM 无效：{e}"))?;
        let key =
            KeyPair::try_from(&key_der).map_err(|e| format!("私钥无效（不支持加密私钥）：{e}"))?;
        let (_, parsed) = x509_parser::parse_x509_certificate(&der).map_err(|e| e.to_string())?;
        if !parsed
            .basic_constraints()
            .map_err(|e| e.to_string())?
            .is_some_and(|c| c.value.ca)
        {
            return Err("需要 CA 证书，不能使用网站服务器证书".into());
        }
        if !parsed.validity().is_valid() {
            return Err("CA 证书尚未生效或已过期".into());
        }
        if parsed
            .key_usage()
            .map_err(|e| e.to_string())?
            .is_some_and(|usage| !usage.value.key_cert_sign())
        {
            return Err("CA 证书不允许签发证书".into());
        }
        if parsed.public_key().raw != key.subject_public_key_info().as_slice() {
            return Err("CA 证书与私钥不匹配".into());
        }
        let name = parsed.subject().to_string();
        let expires = parsed.validity().not_after.to_string();
        let expires_at =
            time::OffsetDateTime::from_unix_timestamp(parsed.validity().not_after.timestamp())
                .map_err(|e| e.to_string())?;
        let fingerprint = Sha256::digest(&der)
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(":");
        let issuer = Issuer::from_ca_cert_der(&der, key).map_err(|e| e.to_string())?;
        Ok(Self {
            issuer,
            certificate: der,
            fingerprint,
            name,
            expires,
            expires_at,
            leaves: Mutex::new(HashMap::new()),
        })
    }
    pub fn load(directory: &Path) -> Result<Self, String> {
        let identity = std::fs::read_to_string(directory.join("identity.pem"))
            .map_err(|e| format!("请先生成或导入代理证书：{e}"))?;
        Self::parse(&identity, &identity)
    }
    pub fn generate(directory: &Path) -> Result<Self, String> {
        if directory.join("identity.pem").exists() {
            return Self::load(directory);
        }
        let mut params = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params
            .distinguished_name
            .push(DnType::CommonName, "Glint API Local Proxy CA");
        params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
        params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(3650);
        let key = KeyPair::generate().map_err(|e| e.to_string())?;
        let cert = params.self_signed(&key).map_err(|e| e.to_string())?;
        Self::import(directory, &cert.pem(), &key.serialize_pem())
    }
    pub fn import(directory: &Path, certificate: &str, private_key: &str) -> Result<Self, String> {
        let authority = Self::parse(certificate, private_key)?;
        std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
        let temporary = directory.join(format!("identity-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            use std::io::Write;
            let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
            file.write_all(format!("{}\n{}", certificate.trim(), private_key.trim()).as_bytes())
                .map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            drop(file);
            std::fs::rename(&temporary, directory.join("identity.pem")).map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result?;
        Ok(authority)
    }
    pub fn export(&self, path: &Path) -> Result<(), String> {
        // PEM for OpenSSL/curl, DER for the Windows certificate installer.
        let bytes = if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pem"))
        {
            use base64::Engine;
            let encoded = base64::engine::general_purpose::STANDARD.encode(&self.certificate);
            format!(
                "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
                encoded
                    .as_bytes()
                    .chunks(64)
                    .map(|line| std::str::from_utf8(line).unwrap())
                    .collect::<Vec<_>>()
                    .join("\n")
            )
            .into_bytes()
        } else {
            self.certificate.as_ref().to_vec()
        };
        std::fs::write(path, bytes).map_err(|e| e.to_string())
    }
    pub fn tls_for(&self, host: &str) -> Result<Arc<rustls::ServerConfig>, String> {
        let mut cache = self.leaves.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(config) = cache.get(host) {
            return Ok(config.clone());
        }
        let mut params =
            CertificateParams::new(vec![host.to_owned()]).map_err(|e| e.to_string())?;
        params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
        params.not_after =
            (time::OffsetDateTime::now_utc() + time::Duration::days(30)).min(self.expires_at);
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        let key = KeyPair::generate().map_err(|e| e.to_string())?;
        let certificate = params
            .signed_by(&key, &self.issuer)
            .map_err(|e| e.to_string())?;
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_no_client_auth()
            .with_single_cert(
                vec![certificate.der().clone(), self.certificate.clone()],
                PrivateKeyDer::Pkcs8(key.serialize_der().into()),
            )
            .map_err(|e| e.to_string())?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let config = Arc::new(config);
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(host.to_owned(), config.clone());
        Ok(config)
    }
}

pub fn install(directory: &Path) -> Result<String, String> {
    let certificate = directory.join("glint-proxy-ca.cer");
    Authority::load(directory)?.export(&certificate)?;
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = std::process::Command::new("certutil.exe");
        c.args(["-user", "-addstore", "Root"]).arg(&certificate);
        c
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = std::process::Command::new("security");
        c.args(["add-trusted-cert", "-r", "trustRoot", "-k"])
            .arg(
                std::path::PathBuf::from(std::env::var_os("HOME").ok_or("无法确定登录钥匙串")?)
                    .join("Library/Keychains/login.keychain-db"),
            )
            .arg(&certificate);
        c
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    return Err(format!(
        "请将 {} 导入系统或浏览器的受信任证书颁发机构",
        certificate.display()
    ));
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let output = command.output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "证书安装失败：{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok("代理 CA 已安装到当前用户的信任库，请重启通过代理启动的浏览器".into())
    }
}
