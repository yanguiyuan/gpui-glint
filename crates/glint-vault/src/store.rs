use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use argon2::{Algorithm, Argon2, Params, Version};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const FORMAT: i64 = 1;
const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Default, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub website: String,
    pub username: String,
    pub password: String,
    pub notes: String,
    pub updated_at: String,
}
impl Entry {
    fn validate(&self) -> Result<(), String> {
        if self.title.trim().is_empty() {
            return Err("请输入条目名称".into());
        }
        if self.title.len() > 1024
            || self.website.len() > 4096
            || self.username.len() > 4096
            || self.password.len() > 8192
            || self.notes.len() > 65536
        {
            return Err("条目内容过长".into());
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct VaultStore {
    path: PathBuf,
}
impl VaultStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    fn connection(&self) -> Result<Connection, String> {
        if let Some(parent) = self.path.parent() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = std::fs::DirBuilder::new();
                builder
                    .recursive(true)
                    .mode(0o700)
                    .create(parent)
                    .map_err(|_| "无法创建密码箱目录")?;
            }
            #[cfg(not(unix))]
            std::fs::create_dir_all(parent).map_err(|_| "无法创建密码箱目录")?;
        }
        if std::fs::symlink_metadata(&self.path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("密码箱文件不能是符号链接".into());
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&self.path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err("无法创建密码箱文件".into()),
        }
        let connection = Connection::open(&self.path).map_err(|_| "无法打开密码箱文件")?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|_| "密码箱文件已损坏")?;
        if version > FORMAT {
            return Err("密码箱版本较新，请升级应用".into());
        }
        if version == 0 {
            connection.execute_batch("BEGIN IMMEDIATE; CREATE TABLE IF NOT EXISTS vault(singleton INTEGER PRIMARY KEY CHECK(singleton=1),version INTEGER NOT NULL,revision INTEGER NOT NULL,salt BLOB NOT NULL,nonce BLOB NOT NULL,ciphertext BLOB NOT NULL); PRAGMA user_version=1; COMMIT;")
                .map_err(|_|"无法初始化密码箱")?;
        }
        connection.execute_batch("CREATE TABLE IF NOT EXISTS vault_settings(key TEXT PRIMARY KEY, value INTEGER NOT NULL CHECK(value IN (0,1)))")
            .map_err(|_| "无法读取密码箱设置")?;
        Ok(connection)
    }
    pub fn browser_auto_approve(&self) -> Result<bool, String> {
        self.browser_setting("browser_auto_approve")
    }
    pub fn browser_match_website(&self) -> Result<bool, String> {
        self.browser_setting("browser_match_website")
    }
    fn browser_setting(&self, key: &str) -> Result<bool, String> {
        if !self.path.exists() {
            return Ok(false);
        }
        self.connection()?
            .query_row(
                "SELECT value FROM vault_settings WHERE key=?1",
                [key],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map(|value| value.unwrap_or(false))
            .map_err(|_| "无法读取浏览器确认设置".into())
    }
    pub fn set_browser_auto_approve(&self, enabled: bool) -> Result<(), String> {
        self.set_browser_setting("browser_auto_approve", enabled)
    }
    pub fn set_browser_match_website(&self, enabled: bool) -> Result<(), String> {
        self.set_browser_setting("browser_match_website", enabled)
    }
    fn set_browser_setting(&self, key: &str, enabled: bool) -> Result<(), String> {
        self.connection()?.execute(
            "INSERT INTO vault_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, enabled],
        ).map(|_| ()).map_err(|_| "无法保存浏览器确认设置".into())
    }
    pub fn initialized(&self) -> Result<bool, String> {
        if !self.path.exists() {
            return Ok(false);
        }
        self.connection()?
            .query_row("SELECT EXISTS(SELECT 1 FROM vault)", [], |row| row.get(0))
            .map_err(|_| "无法读取密码箱".to_owned())
    }
    pub fn create(&self, password: &str) -> Result<Vault, String> {
        if password.chars().count() < 12 || password.len() > 1024 {
            return Err("主密码至少 12 个字符，最多 1024 字节".into());
        }
        let salt = random::<16>()?;
        let key = derive(password, &salt)?;
        let mut vault = Vault {
            store: self.clone(),
            key,
            salt,
            revision: 1,
            entries: vec![],
        };
        let (nonce, ciphertext) = vault.encrypt(&[], 1)?;
        let connection = self.connection()?;
        connection
            .execute(
                "INSERT INTO vault VALUES(1,?1,1,?2,?3,?4)",
                params![FORMAT, salt.as_slice(), nonce.as_slice(), ciphertext],
            )
            .map_err(|_| "密码箱已存在或无法写入；请解锁已有密码箱")?;
        vault.entries = vec![];
        Ok(vault)
    }
    pub fn unlock(&self, password: &str) -> Result<Vault, String> {
        if password.len() > 1024 {
            return Err("主密码过长".into());
        }
        let connection = self.connection()?;
        let row: Option<(i64, i64, Vec<u8>, Vec<u8>, Vec<u8>)> = connection
            .query_row(
                "SELECT version,revision,substr(salt,1,17),substr(nonce,1,13),substr(ciphertext,1,8388625) FROM vault WHERE singleton=1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| "无法读取密码箱")?;
        let (version, revision, salt, nonce, ciphertext) = row.ok_or("尚未创建密码箱")?;
        if version != FORMAT
            || revision < 1
            || salt.len() != 16
            || nonce.len() != 12
            || ciphertext.len() > MAX_BYTES + 16
        {
            return Err("密码箱格式无效或版本不兼容".into());
        }
        let salt: [u8; 16] = salt.try_into().map_err(|_| "盐长度无效")?;
        let key = derive(password, &salt)?;
        let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| "密钥格式无效")?;
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &ciphertext,
                        aad: &aad(&salt, revision),
                    },
                )
                .map_err(|_| "主密码错误，或密码箱内容已损坏")?,
        );
        let entries: Vec<Entry> =
            serde_json::from_slice(&plaintext).map_err(|_| "密码箱内容格式无效")?;
        if entries.len() > 10000 {
            return Err("密码箱条目过多".into());
        }
        for entry in &entries {
            entry.validate()?;
        }
        Ok(Vault {
            store: self.clone(),
            key,
            salt,
            revision,
            entries,
        })
    }
}
pub struct Vault {
    store: VaultStore,
    key: Zeroizing<[u8; 32]>,
    salt: [u8; 16],
    revision: i64,
    entries: Vec<Entry>,
}
impl Vault {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    fn encrypt(&self, entries: &[Entry], revision: i64) -> Result<([u8; 12], Vec<u8>), String> {
        let plaintext =
            Zeroizing::new(serde_json::to_vec(entries).map_err(|_| "无法序列化密码箱")?);
        if plaintext.len() > MAX_BYTES {
            return Err("密码箱超过 8MB，请减少条目内容".into());
        }
        let nonce = random::<12>()?;
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref()).map_err(|_| "密钥格式无效")?;
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &aad(&self.salt, revision),
                },
            )
            .map_err(|_| "密码箱加密失败")?;
        Ok((nonce, ciphertext))
    }
    fn persist(&mut self, entries: Vec<Entry>) -> Result<(), String> {
        let revision = self
            .revision
            .checked_add(1)
            .ok_or("密码箱版本计数超出范围")?;
        let (nonce, ciphertext) = self.encrypt(&entries, revision)?;
        let connection = self.store.connection()?;
        let updated=connection.execute("UPDATE vault SET revision=?1,nonce=?2,ciphertext=?3 WHERE singleton=1 AND revision=?4 AND salt=?5",params![revision,nonce.as_slice(),ciphertext,self.revision,self.salt.as_slice()]).map_err(|_|"无法保存密码箱，请检查磁盘空间和权限")?;
        if updated != 1 {
            return Err(
                "密码箱已在其他窗口修改。当前修改尚未保存，请先保留需要的内容，再锁定并重新解锁。"
                    .into(),
            );
        }
        self.entries = entries;
        self.revision = revision;
        Ok(())
    }
    /// Reload another window's committed revision without deriving a new key.
    pub fn refresh(&mut self) -> Result<bool, String> {
        let mut database = self.store.connection()?;
        let connection = database.transaction().map_err(|_| "无法读取密码箱")?;
        let (version, revision, salt): (i64, i64, Vec<u8>) = connection
            .query_row(
                "SELECT version,revision,substr(salt,1,17) FROM vault WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| "密码箱已不可用，请重新解锁")?;
        if version != FORMAT || salt.as_slice() != self.salt {
            return Err("密码箱密钥或格式已改变，请重新解锁".into());
        }
        if revision == self.revision {
            return Ok(false);
        }
        let (nonce, ciphertext): (Vec<u8>, Vec<u8>) = connection
            .query_row(
                "SELECT substr(nonce,1,13),substr(ciphertext,1,8388625) FROM vault WHERE singleton=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| "无法读取密码箱")?;
        if revision < 1 || nonce.len() != 12 || ciphertext.len() > MAX_BYTES + 16 {
            return Err("密码箱格式无效".into());
        }
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref()).map_err(|_| "密钥格式无效")?;
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &ciphertext,
                        aad: &aad(&self.salt, revision),
                    },
                )
                .map_err(|_| "密码箱内容已改变或损坏，请重新解锁")?,
        );
        let entries: Vec<Entry> =
            serde_json::from_slice(&plaintext).map_err(|_| "密码箱内容无效")?;
        if entries.len() > 10000 {
            return Err("密码箱条目过多".into());
        }
        for entry in &entries {
            entry.validate()?;
        }
        self.entries = entries;
        self.revision = revision;
        Ok(true)
    }

    pub fn upsert(&mut self, mut entry: Entry) -> Result<String, String> {
        entry.title = entry.title.trim().to_owned();
        entry.validate()?;
        entry.updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "系统时间无效")?
            .as_secs()
            .to_string();
        let mut entries = self.entries.clone();
        if entry.id.is_empty() {
            if entries.len() >= 10000 {
                return Err("密码箱条目过多".into());
            }
            entry.id = uuid::Uuid::new_v4().to_string();
            entries.push(entry.clone());
        } else {
            let existing = entries
                .iter_mut()
                .find(|value| value.id == entry.id)
                .ok_or("条目已不存在")?;
            *existing = entry.clone();
        }
        self.persist(entries)?;
        Ok(entry.id.clone())
    }
    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        if !self.entries.iter().any(|entry| entry.id == id) {
            return Err("条目已不存在".into());
        }
        let entries = self
            .entries
            .iter()
            .filter(|entry| entry.id != id)
            .cloned()
            .collect();
        self.persist(entries)
    }
}
fn aad(salt: &[u8; 16], revision: i64) -> Vec<u8> {
    let mut aad = b"glint-vault:1".to_vec();
    aad.extend_from_slice(salt);
    aad.extend_from_slice(&revision.to_le_bytes());
    aad
}
fn derive(password: &str, salt: &[u8; 16]) -> Result<Zeroizing<[u8; 32]>, String> {
    let params = Params::new(64 * 1024, 3, 1, Some(32)).map_err(|_| "密钥参数无效")?;
    let mut key = Zeroizing::new([0; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|_| "无法派生密码箱密钥")?;
    Ok(key)
}
fn random<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| "系统随机数不可用")?;
    Ok(bytes)
}
/// Uniform OS-random generation; each result contains all four character classes.
pub fn generate_password(length: usize) -> Result<Zeroizing<String>, String> {
    if !(16..=128).contains(&length) {
        return Err("生成密码长度应为 16–128".into());
    }
    const CLASSES: [&[u8]; 4] = [
        b"ABCDEFGHJKLMNPQRSTUVWXYZ",
        b"abcdefghijkmnopqrstuvwxyz",
        b"23456789",
        b"!@#$%^&*()-_=+",
    ];
    let alphabet: Vec<u8> = CLASSES.concat();
    loop {
        let mut password = Zeroizing::new(String::with_capacity(length));
        while password.len() < length {
            for byte in random::<64>()? {
                let bound = 256 - (256 % alphabet.len());
                if (byte as usize) < bound {
                    password.push(alphabet[byte as usize % alphabet.len()] as char);
                }
                if password.len() == length {
                    break;
                }
            }
        }
        if CLASSES
            .iter()
            .all(|class| password.bytes().any(|byte| class.contains(&byte)))
        {
            return Ok(password);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MASTER: &str = "a unique master password 2026";
    fn entry(title: &str, password: &str) -> Entry {
        Entry {
            title: title.into(),
            username: "private-user@example.com".into(),
            password: password.into(),
            website: "https://example.com/login".into(),
            notes: "private notes".into(),
            id: String::new(),
            updated_at: String::new(),
        }
    }
    #[test]
    fn encrypted_round_trip_wrong_password_tampering_and_private_file() {
        let directory = tempfile::tempdir().unwrap();
        let store = VaultStore::new(directory.path().join("vault.sqlite3"));
        assert!(!store.initialized().unwrap());
        assert!(store.create("short").is_err());
        let mut vault = store.create(MASTER).unwrap();
        let id = vault
            .upsert(entry("Private Login", "private-password-42"))
            .unwrap();
        drop(vault);
        let bytes = std::fs::read(store.path()).unwrap();
        for secret in [
            "Private Login",
            "private-user@example.com",
            "private-password-42",
            "private notes",
            MASTER,
        ] {
            assert!(
                !bytes
                    .windows(secret.len())
                    .any(|bytes| bytes == secret.as_bytes())
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(store.path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert!(store.unlock("incorrect password").is_err());
        assert!(store.create(MASTER).is_err());
        let mut restored = store.unlock(MASTER).unwrap();
        assert_eq!(restored.entries()[0].id, id);
        assert_eq!(restored.entries()[0].password, "private-password-42");
        restored.delete(&id).unwrap();
        assert!(restored.entries().is_empty());
        let connection = store.connection().unwrap();
        connection
            .execute("UPDATE vault SET revision=revision+1", [])
            .unwrap();
        assert!(restored.refresh().is_err());
        assert!(store.unlock(MASTER).is_err());
    }
    #[test]
    fn two_windows_share_sqlite_refresh_changes_and_never_overwrite_stale_edits() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.sqlite3");
        let store_a = VaultStore::new(path.clone());
        let store_b = VaultStore::new(path);
        let mut a = store_a.create(MASTER).unwrap();
        let mut b = store_b.unlock(MASTER).unwrap();
        let id = a.upsert(entry("First", "first-secret")).unwrap();
        assert!(b.upsert(entry("Stale edit", "must-not-overwrite")).is_err());
        assert!(b.entries().is_empty());
        assert!(b.refresh().unwrap());
        assert_eq!(b.entries()[0].id, id);
        assert!(!b.refresh().unwrap());
        let mut modified = b.entries()[0].clone();
        modified.password = "changed-in-second-window".into();
        b.upsert(modified).unwrap();
        assert!(a.refresh().unwrap());
        assert_eq!(a.entries()[0].password, "changed-in-second-window");
        a.delete(&id).unwrap();
        assert!(b.refresh().unwrap());
        assert!(b.entries().is_empty());
    }
    #[test]
    fn generator_uses_requested_length_and_all_character_classes() {
        for length in [16, 24, 64, 128] {
            let password = generate_password(length).unwrap();
            assert_eq!(password.len(), length);
            assert!(password.bytes().any(|byte| byte.is_ascii_uppercase()));
            assert!(password.bytes().any(|byte| byte.is_ascii_lowercase()));
            assert!(password.bytes().any(|byte| byte.is_ascii_digit()));
            assert!(password.bytes().any(|byte| !byte.is_ascii_alphanumeric()));
        }
        assert!(generate_password(8).is_err());
        assert!(generate_password(129).is_err());
    }
}
