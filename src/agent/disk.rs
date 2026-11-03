//! File reads and writes the assistant runs off the UI thread.

use std::path::{Path, PathBuf};

use crate::services::fs;

use super::edit::{self, FileEdit};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskTask {
    Read {
        path: PathBuf,
        start: usize,
        end: usize,
    },
    List {
        path: PathBuf,
    },
    Edit {
        path: PathBuf,
        edit: FileEdit,
    },
    Create {
        path: PathBuf,
        text: String,
    },
}

/// What the model should read, and a file to open when a create succeeded.
pub struct DiskReport {
    pub message: String,
    pub open: Option<PathBuf>,
}

const MAX_NAMES: usize = 200;
pub const MAX_BYTES: usize = 2_000_000;

/// The message is what the model reads, including a failure.
pub fn perform(task: DiskTask) -> DiskReport {
    match task {
        DiskTask::Read { path, start, end } => report(match read_text_capped(&path) {
            Ok(text) => edit::read_lines(&text, start, end).unwrap_or_else(|err| err),
            Err(err) => err,
        }),
        DiskTask::List { path } => report(match list_dir(&path) {
            Ok(text) => text,
            Err(err) => format!("Cannot list {}: {err}", path.display()),
        }),
        DiskTask::Create { path, text } => create_file(path, text),
        DiskTask::Edit { path, edit } => match read_text_capped(&path) {
            Ok(text) => match edit::apply_str(&text, &edit) {
                Ok(new_text) => match fs::write_atomic(&path, &new_text) {
                    Ok(()) => report(match &edit {
                        FileEdit::Lines { start, end, .. } => {
                            format!("Replaced lines {start}-{end} in {}", path.display())
                        }
                        FileEdit::Snippet { .. } => {
                            format!("Replaced one snippet in {}", path.display())
                        }
                    }),
                    Err(err) => report(err),
                },
                Err(err) => report(err),
            },
            Err(err) => report(err),
        },
    }
}

fn report(message: String) -> DiskReport {
    DiskReport {
        message,
        open: None,
    }
}

/// Refuse oversized files before loading them into memory.
pub fn check_file_size(path: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|err| {
        format!("Cannot read {}: {err}", path.display())
    })?;
    let len = meta.len();
    if len > MAX_BYTES as u64 {
        return Err(format!(
            "File is too large ({} bytes; limit is {MAX_BYTES})",
            len
        ));
    }
    Ok(())
}

fn read_text_capped(path: &Path) -> Result<String, String> {
    check_file_size(path)?;
    std::fs::read_to_string(path).map_err(|err| format!("Cannot read {}: {err}", path.display()))
}

fn create_file(path: PathBuf, text: String) -> DiskReport {
    if text.len() > MAX_BYTES {
        return report("File is too large".to_string());
    }
    if path.exists() {
        return report(format!("File already exists: {}", path.display()));
    }
    match fs::write_atomic(&path, &text) {
        Ok(()) => DiskReport {
            message: format!("Created {}", path.display()),
            open: Some(path),
        },
        Err(err) => report(err),
    }
}

fn list_dir(path: &Path) -> Result<String, String> {
    let mut names = Vec::new();
    let read = std::fs::read_dir(path).map_err(|err| err.to_string())?;
    for entry in read.flatten() {
        let mut name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            name.push('/');
        }
        names.push(name);
        if names.len() >= 2_000 {
            break;
        }
    }
    names.sort();
    let truncated = names.len() > MAX_NAMES;
    names.truncate(MAX_NAMES);
    if truncated {
        names.push("…".to_string());
    }
    Ok(names.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::{DiskTask, MAX_BYTES, check_file_size, perform};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_file(label: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "duckx-disk-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn create_file_writes_once_and_refuses_the_second_time() {
        let path = temp_file("create.html");
        let _ = fs::remove_file(&path);
        let made = perform(DiskTask::Create {
            path: path.clone(),
            text: "<p>hi</p>\n".into(),
        });
        assert!(made.open.is_some(), "{}", made.message);
        assert_eq!(fs::read_to_string(&path).unwrap(), "<p>hi</p>\n");
        let again = perform(DiskTask::Create {
            path: path.clone(),
            text: "nope".into(),
        });
        assert!(again.open.is_none());
        assert!(
            again.message.contains("already exists"),
            "{}",
            again.message
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn oversized_files_are_refused_before_read() {
        let path = temp_file("big.bin");
        // Sparse-ish write without spending forever: write a small file then lie via check.
        fs::write(&path, b"tiny").unwrap();
        assert!(check_file_size(&path).is_ok());

        let over = temp_file("over.bin");
        let chunk = vec![b'x'; 64 * 1024];
        let mut file = fs::File::create(&over).unwrap();
        use std::io::Write;
        let mut written = 0usize;
        while written <= MAX_BYTES {
            file.write_all(&chunk).unwrap();
            written += chunk.len();
        }
        drop(file);
        let err = check_file_size(&over).unwrap_err();
        assert!(err.contains("too large"), "{err}");
        let report = perform(DiskTask::Read {
            path: over.clone(),
            start: 1,
            end: 10,
        });
        assert!(
            report.message.contains("too large"),
            "{}",
            report.message
        );
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&over);
    }
}
