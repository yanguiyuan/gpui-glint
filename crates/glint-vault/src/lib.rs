//! Encrypted local storage and a native page shared by standalone and host apps.
pub mod browser;
pub mod store;
mod ui;
pub use ui::{VaultEvent, VaultPage};

/// Shared by Glint Vault, API and Agent; hosts may supply their own path instead.
pub fn default_path() -> std::path::PathBuf {
    use std::path::PathBuf;
    if let Some(path) = std::env::var_os("GLINT_VAULT_DATA_DIR") {
        return PathBuf::from(path).join("vault.sqlite3");
    }
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")));
    // Never silently put a password database in the current checkout.
    base.unwrap_or_else(std::env::temp_dir)
        .join("Glint Vault/vault.sqlite3")
}
