//! File reads and atomic saves. Callers run these off the UI thread.
//! Text is decoded in [`super::encoding`]; this module moves the bytes.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Reads `path` with BOM detection. A file that is not UTF-8 or UTF-16 is an error.
pub fn read_text(path: &Path) -> Result<String, String> {
    super::encoding::read_auto(path)
        .map(|decoded| decoded.text)
        .map_err(|err| err.message(path))
}

/// Writes `text` as UTF-8 by creating a temporary file in the same directory, flushing it,
/// then renaming it over `path`. The previous file is left in place if the write fails.
pub fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    write_bytes(path, text.as_bytes())
}

/// Writes `bytes` by creating a temporary file in the same directory, flushing it,
/// then renaming it over `path`. The previous file is left in place if the write fails.
pub fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = parent_dir(path);
    fs::create_dir_all(parent).map_err(|err| save_error(path, &err))?;

    let tmp = temp_path(parent, path);
    let guard = DeleteOnDrop(tmp.clone());
    {
        let mut file = File::create(&tmp).map_err(|err| save_error(path, &err))?;
        file.write_all(bytes)
            .map_err(|err| save_error(path, &err))?;
        file.sync_all().map_err(|err| save_error(path, &err))?;
    }
    fs::rename(&tmp, path).map_err(|err| save_error(path, &err))?;
    std::mem::forget(guard);

    // The file contents are already durable. Syncing the directory publishes the rename.
    #[cfg(unix)]
    {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

fn save_error(path: &Path, err: &io::Error) -> String {
    format!("Cannot save {}: {err}", path.display())
}

fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

fn temp_path(dir: &Path, dest: &Path) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let stem = dest.file_name().unwrap_or(std::ffi::OsStr::new("buffer"));
    dir.join(format!(
        ".{}.{}.{nanos}.{n}.duckx-tmp",
        stem.to_string_lossy(),
        std::process::id()
    ))
}

struct DeleteOnDrop(PathBuf);

impl Drop for DeleteOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// A file-tree change requested from the explorer. Paths are absolute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Delete(Vec<PathBuf>),
    Rename { from: PathBuf, to: PathBuf },
    Move { paths: Vec<PathBuf>, dest: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeOutcome {
    pub deleted: Vec<PathBuf>,
    pub relocated: Vec<(PathBuf, PathBuf)>,
    pub error: Option<String>,
    pub summary: String,
}

/// Applies `change` inside `root`. A path outside `root` is refused and left untouched.
pub fn apply(root: &Path, change: &Change) -> ChangeOutcome {
    let root = match std::path::absolute(root) {
        Ok(root) => root,
        Err(err) => {
            return ChangeOutcome {
                deleted: Vec::new(),
                relocated: Vec::new(),
                error: Some(format!("Cannot resolve the workspace: {err}")),
                summary: String::new(),
            };
        }
    };
    match change {
        Change::Delete(paths) => delete_all(&root, paths),
        Change::Rename { from, to } => rename_one(&root, from, to),
        Change::Move { paths, dest } => move_all(&root, paths, dest),
    }
}

fn delete_all(root: &Path, paths: &[PathBuf]) -> ChangeOutcome {
    let mut deleted = Vec::new();
    for path in paths {
        match guard(root, path, false).and_then(|path| remove_any(&path)) {
            Ok(path) => deleted.push(path),
            Err(message) => {
                return ChangeOutcome {
                    summary: String::new(),
                    error: Some(message),
                    deleted,
                    relocated: Vec::new(),
                };
            }
        }
    }
    let summary = if deleted.len() == 1 {
        format!("Deleted {}", leaf(&deleted[0]))
    } else {
        format!("Deleted {} items", deleted.len())
    };
    ChangeOutcome {
        deleted,
        relocated: Vec::new(),
        error: None,
        summary,
    }
}

fn rename_one(root: &Path, from: &Path, to: &Path) -> ChangeOutcome {
    let outcome = (|| {
        let from = guard(root, from, false)?;
        let to = guard(root, to, false)?;
        if to.exists() && !same_file(&from, &to) {
            return Err(format!("{} already exists", leaf(&to)));
        }
        fs::rename(&from, &to).map_err(|err| format!("Cannot rename {}: {err}", leaf(&from)))?;
        Ok((from, to))
    })();
    match outcome {
        Ok((from, to)) => ChangeOutcome {
            summary: format!("Renamed {} to {}", leaf(&from), leaf(&to)),
            relocated: vec![(from, to)],
            deleted: Vec::new(),
            error: None,
        },
        Err(message) => ChangeOutcome {
            deleted: Vec::new(),
            relocated: Vec::new(),
            error: Some(message),
            summary: String::new(),
        },
    }
}

fn move_all(root: &Path, paths: &[PathBuf], dest: &Path) -> ChangeOutcome {
    let dest = match guard(root, dest, true) {
        Ok(dest) => dest,
        Err(message) => {
            return failed(message);
        }
    };
    if !dest.is_dir() {
        return failed(format!("{} is not a folder", leaf(&dest)));
    }
    let mut relocated = Vec::new();
    for path in paths {
        match move_one(root, path, &dest) {
            Ok(pair) => relocated.push(pair),
            Err(message) => {
                return ChangeOutcome {
                    summary: String::new(),
                    error: Some(message),
                    deleted: Vec::new(),
                    relocated,
                };
            }
        }
    }
    let folder = leaf(&dest);
    let summary = if relocated.len() == 1 {
        format!("Moved {} to {folder}", leaf(&relocated[0].0))
    } else {
        format!("Moved {} items to {folder}", relocated.len())
    };
    ChangeOutcome {
        deleted: Vec::new(),
        relocated,
        error: None,
        summary,
    }
}

fn move_one(root: &Path, path: &Path, dest: &Path) -> Result<(PathBuf, PathBuf), String> {
    let path = guard(root, path, false)?;
    if dest.starts_with(&path) {
        return Err(format!("Cannot move {} into itself", leaf(&path)));
    }
    if path.parent().is_some_and(|parent| parent == dest) {
        return Err(format!("{} is already in that folder", leaf(&path)));
    }
    let name = path
        .file_name()
        .ok_or_else(|| format!("Cannot move {}", path.display()))?;
    let target = dest.join(name);
    if target.exists() {
        return Err(format!("{} already exists", leaf(&target)));
    }
    let target = guard(root, &target, false)?;
    fs::rename(&path, &target).map_err(|err| format!("Cannot move {}: {err}", leaf(&path)))?;
    Ok((path, target))
}

fn remove_any(path: &Path) -> Result<PathBuf, String> {
    let meta =
        fs::symlink_metadata(path).map_err(|err| format!("Cannot delete {}: {err}", leaf(path)))?;
    let result = if meta.file_type().is_symlink() || !meta.is_dir() {
        fs::remove_file(path)
    } else {
        fs::remove_dir_all(path)
    };
    result.map_err(|err| format!("Cannot delete {}: {err}", leaf(path)))?;
    Ok(path.to_path_buf())
}

/// `allow_root` permits the workspace directory itself (a move destination).
fn guard(root: &Path, path: &Path, allow_root: bool) -> Result<PathBuf, String> {
    let path = std::path::absolute(path)
        .map_err(|err| format!("Cannot resolve {}: {err}", path.display()))?;
    if path == root {
        if allow_root {
            return Ok(path);
        }
        return Err("That path is the workspace itself".to_string());
    }
    if !path.starts_with(root) {
        return Err(format!("{} is outside the workspace", leaf(&path)));
    }
    Ok(path)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (crate::agent::canonicalize(a), crate::agent::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn leaf(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

fn failed(message: String) -> ChangeOutcome {
    ChangeOutcome {
        deleted: Vec::new(),
        relocated: Vec::new(),
        error: Some(message),
        summary: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("duckx-fs-{}-{nanos}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn atomic_write_replaces_bytes_and_leaves_no_temp_file() {
        let dir = TempDir::new();
        let path = dir.path().join("main.rs");
        fs::write(&path, "old\r\n").unwrap();

        write_atomic(&path, "new\r\nline").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new\r\nline");

        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().contains("duckx-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn failed_replace_keeps_the_original_and_removes_the_temp_file() {
        let dir = TempDir::new();
        let dest = dir.path().join("blocked");
        fs::create_dir(&dest).unwrap();

        let err = write_atomic(&dest, "nope").unwrap_err();
        assert!(err.contains("Cannot save"), "{err}");
        assert!(dest.is_dir());
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().contains("duckx-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn read_text_rejects_invalid_utf8() {
        let dir = TempDir::new();
        let path = dir.path().join("bin");
        fs::write(&path, [0xA1, 0xA2]).unwrap();
        let err = read_text(&path).unwrap_err();
        assert!(err.contains("not valid UTF-8"), "{err}");
    }

    #[test]
    fn delete_rename_and_move_stay_inside_the_workspace() {
        let dir = TempDir::new();
        let hello = dir.path().join("hello.rs");
        let src = dir.path().join("src");
        fs::write(&hello, "hi\n").unwrap();
        fs::create_dir(&src).unwrap();
        fs::write(src.join("main.rs"), "lib\n").unwrap();

        let renamed = apply(
            dir.path(),
            &Change::Rename {
                from: hello.clone(),
                to: dir.path().join("notes.rs"),
            },
        );
        assert_eq!(renamed.error, None, "{renamed:?}");
        assert!(renamed.summary.contains("notes.rs"), "{renamed:?}");
        assert!(dir.path().join("notes.rs").is_file());

        let moved = apply(
            dir.path(),
            &Change::Move {
                paths: vec![dir.path().join("notes.rs")],
                dest: src.clone(),
            },
        );
        assert_eq!(moved.error, None, "{moved:?}");
        assert!(src.join("notes.rs").is_file());
        assert!(moved.summary.contains("src"), "{moved:?}");

        let deleted = apply(dir.path(), &Change::Delete(vec![src.clone()]));
        assert_eq!(deleted.error, None, "{deleted:?}");
        assert!(!src.exists());
        assert_eq!(deleted.summary, "Deleted src");

        let outside = std::env::temp_dir().join(format!("duckx-outside-{}", std::process::id()));
        let refused = apply(dir.path(), &Change::Delete(vec![outside]));
        assert!(
            refused
                .error
                .as_deref()
                .is_some_and(|e| e.contains("outside") || e.contains("Cannot delete")),
            "{refused:?}"
        );
    }

    #[test]
    fn move_into_itself_and_existing_name_are_errors() {
        let dir = TempDir::new();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("main.rs"), "lib\n").unwrap();
        fs::create_dir_all(src.join("nested")).unwrap();
        fs::write(dir.path().join("main.rs"), "root\n").unwrap();

        let into_self = apply(
            dir.path(),
            &Change::Move {
                paths: vec![src.clone()],
                dest: src.join("nested"),
            },
        );
        assert!(
            into_self
                .error
                .as_deref()
                .is_some_and(|e| e.contains("itself") || e.contains("not a folder")),
            "{into_self:?}"
        );
        assert!(src.join("main.rs").is_file());

        let clash = apply(
            dir.path(),
            &Change::Move {
                paths: vec![src.join("main.rs")],
                dest: dir.path().to_path_buf(),
            },
        );
        assert!(
            clash
                .error
                .as_deref()
                .is_some_and(|e| e.contains("already exists")),
            "{clash:?}"
        );
    }
}
