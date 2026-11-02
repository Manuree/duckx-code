//! File reads and writes the assistant runs off the UI thread.

use std::path::PathBuf;

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
const MAX_BYTES: usize = 2_000_000;

/// The message is what the model reads, including a failure.
pub fn perform(task: DiskTask) -> DiskReport {
    match task {
        DiskTask::Read { path, start, end } => report(match std::fs::read_to_string(&path) {
            Ok(text) => edit::read_lines(&text, start, end).unwrap_or_else(|err| err),
            Err(err) => format!("Cannot read {}: {err}", path.display()),
        }),
        DiskTask::List { path } => report(match list_dir(&path) {
            Ok(text) => text,
            Err(err) => format!("Cannot list {}: {err}", path.display()),
        }),
        DiskTask::Create { path, text } => create_file(path, text),
        DiskTask::Edit { path, edit } => match std::fs::read_to_string(&path) {
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
            Err(err) => report(format!("Cannot read {}: {err}", path.display())),
        },
    }
}

fn report(message: String) -> DiskReport {
    DiskReport {
        message,
        open: None,
    }
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

fn list_dir(path: &std::path::Path) -> Result<String, String> {
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
    use super::{DiskTask, perform};

    #[test]
    fn create_file_writes_once_and_refuses_the_second_time() {
        let path = std::env::temp_dir().join(format!(
            "duckx-create-{}-{}.html",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&path);
        let made = perform(DiskTask::Create {
            path: path.clone(),
            text: "<p>hi</p>\n".into(),
        });
        assert!(made.open.is_some(), "{}", made.message);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "<p>hi</p>\n");
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
        let _ = std::fs::remove_file(&path);
    }
}
