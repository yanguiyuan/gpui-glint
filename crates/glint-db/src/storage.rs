use crate::model::ConnectionConfig;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SavedWorkspace {
    #[serde(default)]
    pub connections: Vec<ConnectionConfig>,
    #[serde(default)]
    pub active_connection: Option<Uuid>,
    #[serde(default)]
    pub sidebar_width: Option<f32>,
}

pub fn default_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".glint").join("db_connections.json");
    }
    PathBuf::from(".glint").join("db_connections.json")
}

pub fn load(path: &Path) -> SavedWorkspace {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return SavedWorkspace::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn save(path: &Path, state: &SavedWorkspace) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
    }
    let json =
        serde_json::to_string_pretty(state).map_err(|e| format!("序列化连接配置失败：{e}"))?;
    std::fs::write(path, json).map_err(|e| format!("保存连接配置失败：{e}"))?;
    Ok(())
}
