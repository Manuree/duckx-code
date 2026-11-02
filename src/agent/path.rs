//! Keep an assistant path inside the workspace root.

use std::path::{Component, Path, PathBuf};

/// `raw` joined onto `root`, with `.` and `..` folded. A path that climbs out is an error.
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
    Ok(clean)
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

    #[test]
    fn dotdot_is_rejected_and_a_child_is_kept() {
        let root = Path::new(r"E:\work\duckx");
        assert!(within(root, r"..\secret").is_err());
        assert!(within(root, r"src\..\..\secret").is_err());
        assert_eq!(
            within(root, r"src\main.rs").unwrap(),
            PathBuf::from(r"E:\work\duckx\src\main.rs")
        );
        assert!(within(root, r"E:\other\a.rs").is_err());
    }
}
