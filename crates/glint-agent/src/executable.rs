//! Resolve local executables without launching package-manager wrappers.
use std::path::{Path, PathBuf};

/// Explicit paths retain their meaning; only bare rg uses our bundled copy.
pub fn resolve(program: &str, cwd: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let path = Path::new(program);
        if path.components().count() == 1
            && (program.eq_ignore_ascii_case("rg") || program.eq_ignore_ascii_case("rg.exe"))
        {
            if let Some(path) = bundled_rg() {
                return path;
            }
        }
        let candidate = find(program, cwd).unwrap_or_else(|| path.into());
        return resolve_candidate(&candidate).unwrap_or(candidate);
    }
    #[cfg(not(windows))]
    {
        let _ = cwd;
        program.into()
    }
}

#[cfg(windows)]
fn find(program: &str, cwd: &Path) -> Option<PathBuf> {
    let program = Path::new(program);
    let directories = if program.components().count() > 1 || program.is_absolute() {
        vec![cwd.to_owned()]
    } else {
        // Do not implicitly search the untrusted project directory before PATH.
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect()
    };
    for directory in directories {
        let candidate = directory.join(program);
        if candidate.is_file() || expand_reparse(&candidate).is_ok() {
            return Some(candidate);
        }
        if candidate.extension().is_none() {
            let exe = candidate.with_extension("exe");
            if exe.is_file() || expand_reparse(&exe).is_ok() {
                return Some(exe);
            }
        }
    }
    None
}

#[cfg(windows)]
fn resolve_candidate(path: &Path) -> std::io::Result<PathBuf> {
    let mut target = path.to_owned();
    for _ in 0..32 {
        let expanded = expand_reparse(&target)?;
        let Some(shim_target) = scoop_target(&expanded) else {
            return Ok(expanded);
        };
        target = shim_target;
    }
    Err(std::io::Error::other("executable shim resolution cycle"))
}

#[cfg(windows)]
fn scoop_target(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    if !parent
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("shims")
        || !parent
            .parent()?
            .file_name()?
            .to_string_lossy()
            .eq_ignore_ascii_case("scoop")
        || !path
            .extension()?
            .to_string_lossy()
            .eq_ignore_ascii_case("exe")
    {
        return None;
    }
    let metadata = std::fs::read_to_string(path.with_extension("shim")).ok()?;
    // A shim with injected arguments is not equivalent to executing its target.
    let mut target = None;
    for line in metadata.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let (key, value) = line.split_once('=')?;
        let value = value.trim();
        match key.trim() {
            "path" => target = Some(PathBuf::from(value.strip_prefix('"')?.strip_suffix('"')?)),
            "args" if value != "" && value != "\"\"" => return None,
            "args" => {}
            _ => return None,
        }
    }
    target.filter(|path| path.is_absolute())
}

/// read_link opens the reparse point itself (FSCTL_GET_REPARSE_POINT), so a
/// junction can be expanded even when traversing that junction is disallowed.
#[cfg(windows)]
fn expand_reparse(path: &Path) -> std::io::Result<PathBuf> {
    use std::os::windows::fs::MetadataExt;
    let mut path = std::path::absolute(path)?;
    for _ in 0..32 {
        let mut current = PathBuf::new();
        let components: Vec<_> = path.components().collect();
        let mut replaced = false;
        for (index, component) in components.iter().enumerate() {
            current.push(component);
            if !matches!(component, std::path::Component::Normal(_)) {
                continue;
            }
            if std::fs::symlink_metadata(&current)?.file_attributes() & 0x400 != 0 {
                let link = std::fs::read_link(&current)?;
                let mut next = if link.is_absolute() {
                    link
                } else {
                    current.parent().unwrap().join(link)
                };
                for rest in &components[index + 1..] {
                    next.push(rest);
                }
                path = next;
                replaced = true;
                break;
            }
        }
        if !replaced {
            return Ok(path);
        }
    }
    Err(std::io::Error::other("executable reparse resolution cycle"))
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub fn bundled_rg() -> Option<PathBuf> {
    use sha2::{Digest, Sha256};
    use std::sync::OnceLock;
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        const BYTES: &[u8] = include_bytes!("../assets/tools/ripgrep-15.1.0-windows-x64/rg.exe");
        let root =
            PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Glint/tools/ripgrep-15.1.0");
        std::fs::create_dir_all(&root).ok()?;
        let root = expand_reparse(&root).ok()?;
        for (name, bytes) in [
            (
                "COPYING",
                include_bytes!("../assets/tools/ripgrep-15.1.0-windows-x64/COPYING").as_slice(),
            ),
            (
                "LICENSE-MIT",
                include_bytes!("../assets/tools/ripgrep-15.1.0-windows-x64/LICENSE-MIT").as_slice(),
            ),
            (
                "UNLICENSE",
                include_bytes!("../assets/tools/ripgrep-15.1.0-windows-x64/UNLICENSE").as_slice(),
            ),
        ] {
            std::fs::write(root.join(name), bytes).ok()?;
        }
        let path = root.join("rg.exe");
        if std::fs::read(&path)
            .ok()
            .is_some_and(|bytes| Sha256::digest(&bytes) == Sha256::digest(BYTES))
        {
            return Some(path);
        }
        let temporary = root.join(format!("rg-{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&temporary, BYTES).ok()?;
        let installed = std::fs::rename(&temporary, &path);
        if installed.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        let bytes = std::fs::read(&path).ok()?;
        (Sha256::digest(bytes) == Sha256::digest(BYTES)).then_some(path)
    })
    .clone()
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
pub fn bundled_rg() -> Option<PathBuf> {
    None
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn resolves_scoop_metadata_and_directory_junction_without_running_the_shim() {
        let temp = tempfile::tempdir().unwrap();
        let shims = temp.path().join("scoop/shims");
        let real = temp.path().join("release 中文 with spaces");
        std::fs::create_dir_all(&shims).unwrap();
        std::fs::create_dir_all(&real).unwrap();
        let exe = real.join("probe.exe");
        std::fs::write(&exe, b"not a program; resolution must not execute it").unwrap();
        let junction = temp.path().join("current");
        let output = crate::child_process::command("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&real)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let shim = shims.join("probe.exe");
        std::fs::write(&shim, b"fake shim").unwrap();
        std::fs::write(
            shim.with_extension("shim"),
            format!("path = \"{}\"\n", junction.join("probe.exe").display()),
        )
        .unwrap();
        assert_eq!(
            resolve_candidate(&shim).unwrap(),
            expand_reparse(&exe).unwrap()
        );
        std::fs::write(
            shim.with_extension("shim"),
            format!("path = \"{}\"\nargs = --injected\n", exe.display()),
        )
        .unwrap();
        assert_eq!(
            resolve_candidate(&shim).unwrap(),
            expand_reparse(&shim).unwrap()
        );
        // Remove only the junction itself, leaving its target for TempDir cleanup.
        std::fs::remove_dir(&junction).unwrap();
    }

    #[test]
    fn bundled_rg_is_a_real_executable_and_explicit_paths_are_not_replaced() {
        let Some(rg) = bundled_rg() else { return };
        assert_eq!(resolve("rg", Path::new(".")), rg);
        let output = crate::child_process::command(&rg)
            .arg("--version")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).starts_with("ripgrep 15.1.0"));
        let explicit = std::env::current_exe().unwrap();
        assert_eq!(
            resolve(explicit.to_str().unwrap(), Path::new(".")),
            expand_reparse(&explicit).unwrap()
        );
    }
}
