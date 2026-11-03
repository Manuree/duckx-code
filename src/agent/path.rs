//! Keep an assistant path inside the workspace root.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// `raw` joined onto `root`, with `.` and `..` folded, then checked after
/// resolving symlinks. A path that climbs out is an error.
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
    resolve_under(&root, &clean)
}

/// Canonicalize `root` and `clean` (or the nearest existing ancestor of `clean`)
/// so a workspace symlink cannot point outside the folder.
fn resolve_under(root: &Path, clean: &Path) -> Result<PathBuf, String> {
    if !root.exists() {
        return Err("Workspace folder is missing".to_string());
    }
    let root_canon = std::fs::canonicalize(root).map_err(|err| {
        format!("Cannot resolve workspace {}: {err}", root.display())
    })?;

    let (base, suffix) = nearest_existing(clean);
    let base_canon = if base.as_os_str().is_empty() || !base.exists() {
        root_canon.clone()
    } else {
        std::fs::canonicalize(&base)
            .map_err(|err| format!("Cannot resolve {}: {err}", base.display()))?
    };

    let mut out = base_canon;
    for part in suffix {
        out.push(part);
    }

    if out != root_canon && !out.starts_with(&root_canon) {
        return Err("Path is outside the workspace".to_string());
    }
    Ok(out)
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
        let root = std::env::temp_dir().join(format!(
            "duckx-path-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn dotdot_is_rejected_and_a_child_is_kept() {
        let root = temp_root("lexical");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        assert!(within(&root, "../secret").is_err());
        assert!(within(&root, "src/../../secret").is_err());
        let child = within(&root, "src/main.rs").unwrap();
        assert_eq!(child, fs::canonicalize(root.join("src/main.rs")).unwrap());
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
            assert!(
                err.contains("outside"),
                "expected outside error, got {err}"
            );
        }
        #[cfg(not(unix))]
        {
            let _ = link;
        }
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn a_missing_child_stays_under_the_canonical_root() {
        let root = temp_root("missing");
        let path = within(&root, "new/file.rs").unwrap();
        let root_canon = fs::canonicalize(&root).unwrap();
        assert!(path.starts_with(&root_canon));
        assert_eq!(path, root_canon.join("new").join("file.rs"));
        let _ = fs::remove_dir_all(&root);
    }
}
