//! Keep an assistant path inside the workspace root.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Strip a Windows `\\?\` (or `\\?\UNC\`) verbatim prefix for display and storage.
/// Comparison helpers should prefer this over storing `std::fs::canonicalize` results.
pub fn strip_verbatim(path: &Path) -> PathBuf {
    // Manual strip so Windows session paths still clean up when parsed on any OS.
    let text = path.to_string_lossy();
    let stripped = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else if let Some(rest) = text.strip_prefix("//?/UNC/") {
        PathBuf::from(format!("//{rest}"))
    } else if let Some(rest) = text.strip_prefix("//?/") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    };
    dunce::simplified(&stripped).to_path_buf()
}

/// Canonicalize for security checks without the Windows verbatim prefix.
pub fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    dunce::canonicalize(path)
}

/// `raw` joined onto `root`, with `.` and `..` folded, then checked after
/// resolving symlinks. A path that climbs out is an error.
///
/// The returned path is the lexical workspace path (no `\\?\` prefix). Canonical
/// forms are used only for the containment check.
pub fn within(root: &Path, raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("Path is empty".to_string());
    }
    let root = normalize(root);
    let joined = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        root.join(raw)
    };
    let clean = normalize(&joined);
    if clean != root && !clean.starts_with(&root) {
        return Err("Path is outside the workspace".to_string());
    }
    resolve_under(&root, &clean)?;
    Ok(strip_verbatim(&clean))
}

/// Canonicalize `root` and `clean` (or the nearest existing ancestor of `clean`)
/// so a workspace symlink cannot point outside the folder.
fn resolve_under(root: &Path, clean: &Path) -> Result<(), String> {
    if !root.exists() {
        return Err("Workspace folder is missing".to_string());
    }
    let root_canon = canonicalize(root)
        .map_err(|err| format!("Cannot resolve workspace {}: {err}", root.display()))?;

    let (base, suffix) = nearest_existing(clean);
    let base_canon = if base.as_os_str().is_empty() || !base.exists() {
        root_canon.clone()
    } else {
        canonicalize(&base).map_err(|err| format!("Cannot resolve {}: {err}", base.display()))?
    };

    let mut check = base_canon;
    for part in suffix {
        check.push(part);
    }

    if check != root_canon && !check.starts_with(&root_canon) {
        return Err("Path is outside the workspace".to_string());
    }
    Ok(())
}

/// Walk up from `path` until an existing directory or file is found.
/// Returns that ancestor and the missing components below it (root first).
fn nearest_existing(path: &Path) -> (PathBuf, Vec<OsString>) {
    let mut base = path.to_path_buf();
    let mut suffix = Vec::new();
    while !base.as_os_str().is_empty() && !base.exists() {
        match base.file_name() {
            Some(name) => {
                suffix.push(name.to_os_string());
                if !base.pop() {
                    break;
                }
            }
            None => break,
        }
    }
    suffix.reverse();
    (base, suffix)
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("duckx-path-{label}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn strip_verbatim_removes_windows_prefix() {
        let verbatim = PathBuf::from(r"\\?\D:\Work\file.rs");
        let stripped = strip_verbatim(&verbatim);
        let text = stripped.to_string_lossy();
        assert!(
            !text.contains(r"\\?\"),
            "verbatim prefix must be stripped: {text}"
        );
        // On non-Windows, dunce leaves ordinary paths alone.
        let unixish = PathBuf::from("/work/file.rs");
        assert_eq!(strip_verbatim(&unixish), unixish);
    }

    #[test]
    fn within_returns_lexical_path_not_canonical_verbatim() {
        let root = temp_root("lexical");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        assert!(within(&root, "../secret").is_err());
        assert!(within(&root, "src/../../secret").is_err());
        let child = within(&root, "src/main.rs").unwrap();
        let text = child.to_string_lossy();
        assert!(
            !text.contains(r"\\?\"),
            "within must not return a verbatim path: {text}"
        );
        assert!(
            child.ends_with(Path::new("src/main.rs")) || child.ends_with(Path::new("src\\main.rs"))
        );
        let outside = temp_root("other");
        assert!(within(&root, outside.join("a.rs").to_str().unwrap()).is_err());
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn a_symlink_out_of_the_workspace_is_rejected() {
        let root = temp_root("sym-root");
        let outside = temp_root("sym-out");
        fs::write(outside.join("secret.txt"), "nope\n").unwrap();
        let link = root.join("leak");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            let err = within(&root, "leak/secret.txt").unwrap_err();
            assert!(err.contains("outside"), "expected outside error, got {err}");
        }
        #[cfg(not(unix))]
        {
            let _ = link;
        }
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn a_missing_child_stays_under_the_root_without_verbatim() {
        let root = temp_root("missing");
        let path = within(&root, "new/file.rs").unwrap();
        let text = path.to_string_lossy();
        assert!(!text.contains(r"\\?\"), "{text}");
        assert!(
            path.ends_with(Path::new("new/file.rs")) || path.ends_with(Path::new("new\\file.rs"))
        );
        let _ = fs::remove_dir_all(&root);
    }
}
