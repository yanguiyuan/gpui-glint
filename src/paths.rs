//! Filesystem path labels. Formatting must never replace stored paths or I/O targets.
use std::path::Path;

/// Show familiar Windows drive/UNC syntax without changing device paths,
/// relative paths, POSIX paths, or SSH URIs. No filesystem access is performed.
pub fn display_path(path: impl AsRef<Path>) -> String {
    let path = path.as_ref().to_string_lossy();
    if let Some(share) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{share}");
    }
    if let Some(drive) = path.strip_prefix(r"\\?\") {
        let bytes = drive.as_bytes();
        if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1..3] == *b":\\" {
            return drive.to_owned();
        }
    }
    path.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_windows_filesystem_prefixes_are_converted_for_display() {
        for (path, expected) in [
            (r"\\?\D:\项目\a.md", r"D:\项目\a.md"),
            (r"\\?\D:\", r"D:\"),
            (r"\\?\UNC\server\share\a.md", r"\\server\share\a.md"),
            (r"\\server\share\a.md", r"\\server\share\a.md"),
            (r"\\?\Volume{1234}\a.md", r"\\?\Volume{1234}\a.md"),
            (r"\\.\pipe\service", r"\\.\pipe\service"),
            (
                "ssh://user@host/project/a.md",
                "ssh://user@host/project/a.md",
            ),
            ("/home/user/a.md", "/home/user/a.md"),
            ("docs/components.md", "docs/components.md"),
        ] {
            assert_eq!(display_path(path), expected);
        }
    }
}
