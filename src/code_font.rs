//! Application-wide font for code surfaces; ordinary UI typography is unchanged.
use gpui::SharedString;
use std::sync::{OnceLock, RwLock};

fn setting() -> &'static RwLock<String> {
    static FONT: OnceLock<RwLock<String>> = OnceLock::new();
    FONT.get_or_init(|| RwLock::new("Menlo".into()))
}

pub fn family() -> SharedString {
    setting()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .into()
}

/// Hosts should refresh their windows after changing this setting.
pub fn set_family(family: Option<&str>) {
    *setting().write().unwrap_or_else(|e| e.into_inner()) =
        family.filter(|s| !s.is_empty()).unwrap_or("Menlo").into();
}
