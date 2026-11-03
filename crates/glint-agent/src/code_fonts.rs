//! Owned font assets and durable selection. Conversion happens only on import.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub family: String,
    pub source_name: String,
    pub format: String,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Library {
    pub fonts: Vec<Entry>,
    pub selected: Option<String>,
}

pub fn directory(anchor: &Path) -> PathBuf {
    anchor.parent().unwrap_or(Path::new(".")).join("code-fonts")
}

fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn load(anchor: &Path) -> Result<Library, String> {
    let path = directory(anchor).join("settings.json");
    match std::fs::metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Library::default()),
        Err(e) => return Err(e.to_string()),
        Ok(meta) if meta.len() > 256 * 1024 => return Err("字体设置文件过大".into()),
        _ => {}
    }
    let data: Library = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("无法读取字体设置：{e}"))?;
    if data.fonts.len() > 32
        || data
            .fonts
            .iter()
            .any(|e| !valid_id(&e.id) || e.family.len() > 256)
        || data
            .selected
            .as_ref()
            .is_some_and(|id| !data.fonts.iter().any(|e| &e.id == id))
    {
        return Err("字体设置无效".into());
    }
    Ok(data)
}

pub fn save(anchor: &Path, library: &Library) -> Result<(), String> {
    let dir = directory(anchor);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join("settings.json");
    let before = crate::atomic_file::current(&path)?;
    let data = serde_json::to_string_pretty(library).map_err(|e| e.to_string())?;
    crate::atomic_file::replace(&path, before.as_deref(), &data)
}

fn family(bytes: &[u8]) -> Result<String, String> {
    let face = ttf_parser::Face::parse(bytes, 0).map_err(|_| "不是有效的 TTF / OTF 字体")?;
    for id in [
        ttf_parser::name_id::TYPOGRAPHIC_FAMILY,
        ttf_parser::name_id::FAMILY,
    ] {
        if let Some(name) = face
            .names()
            .into_iter()
            .filter(|name| name.name_id == id)
            .filter_map(|name| name.to_string())
            .find(|name| !name.trim().is_empty() && name.len() <= 256)
        {
            return Ok(name);
        }
    }
    Err("字体缺少可识别的字体族名称".into())
}

fn bounded_read(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("无法读取字体：{e}"))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("请选择字体文件".into());
    }
    let mut bytes = Vec::new();
    file.take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > LIMIT {
        return Err("字体文件超过 16 MiB".into());
    }
    Ok(bytes)
}

pub fn import(
    anchor: &Path,
    source: &Path,
    mut library: Library,
) -> Result<(Library, Entry, Vec<u8>), String> {
    let original = bounded_read(source)?;
    let id = format!("{:x}", Sha256::digest(&original));
    if let Some(entry) = library.fonts.iter().find(|e| e.id == id).cloned() {
        if let Ok(bytes) = read_font(anchor, &entry) {
            return Ok((library, entry, bytes));
        }
        library.fonts.retain(|entry| entry.id != id);
    }
    if library.fonts.len() >= 32 {
        return Err("最多保留 32 个历史字体".into());
    }
    let (format, bytes) = if original.starts_with(b"wOF2") {
        let declared = original
            .get(16..20)
            .and_then(|s| s.try_into().ok())
            .map(u32::from_be_bytes)
            .ok_or("WOFF2 文件头不完整")?;
        if declared as usize > LIMIT {
            return Err("WOFF2 解压后的字体超过 16 MiB".into());
        }
        let bytes = std::panic::catch_unwind(|| {
            woff2_patched::decode::convert_woff2_to_ttf(&mut std::io::Cursor::new(&original))
        })
        .map_err(|_| "WOFF2 字体格式错误")?
        .map_err(|e| format!("无法转换 WOFF2：{e}"))?;
        ("woff2", bytes)
    } else if original.starts_with(b"OTTO") {
        ("otf", original.clone())
    } else {
        ("ttf", original.clone())
    };
    if bytes.len() > LIMIT {
        return Err("解码后的字体超过 16 MiB".into());
    }
    let entry = Entry {
        id,
        family: family(&bytes)?,
        source_name: source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        format: format.into(),
    };
    let dir = directory(anchor);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Store both the original and decoded face. Selection never depends on the source path.
    std::fs::write(
        dir.join(format!("{}.{}", entry.id, entry.format)),
        &original,
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{}.font", entry.id)), &bytes).map_err(|e| e.to_string())?;
    library.fonts.push(entry.clone());
    Ok((library, entry, bytes))
}

pub fn read_font(anchor: &Path, entry: &Entry) -> Result<Vec<u8>, String> {
    if !valid_id(&entry.id) {
        return Err("字体标识无效".into());
    }
    let bytes = bounded_read(&directory(anchor).join(format!("{}.font", entry.id)))?;
    if family(&bytes)? != entry.family {
        return Err("托管字体文件已变化，请重新导入".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_font_is_not_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.woff2");
        std::fs::write(&path, "not a font").unwrap();
        let anchor = dir.path().join("sessions.json");
        assert!(import(&anchor, &path, Library::default()).is_err());
        assert!(!directory(&anchor).exists());
    }

    #[test]
    fn imported_font_survives_source_removal_and_deduplicates() {
        // Optional real-font probe supplied by the local validation command.
        let Some(source) = std::env::var_os("GLINT_TEST_FONT") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let source_copy = dir.path().join("source.woff2");
        std::fs::copy(PathBuf::from(source), &source_copy).unwrap();
        let anchor = dir.path().join("app/sessions.json");
        let (library, entry, bytes) = import(&anchor, &source_copy, Library::default()).unwrap();
        assert!(!bytes.is_empty());
        let (mut library, same, _) = import(&anchor, &source_copy, library).unwrap();
        assert_eq!(entry, same);
        assert_eq!(library.fonts.len(), 1);
        library.selected = Some(entry.id.clone());
        save(&anchor, &library).unwrap();
        std::fs::remove_file(source_copy).unwrap();
        let restored = load(&anchor).unwrap();
        assert_eq!(restored.selected, Some(entry.id.clone()));
        assert_eq!(read_font(&anchor, &entry).unwrap(), bytes);
    }
}
