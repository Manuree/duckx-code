//! File reads, searches, and new files the assistant runs off the UI thread.
//! Edits to existing files never happen here: they go through an editor buffer,
//! so each one is an undo step and stays unsaved until someone saves it.

use std::path::{Path, PathBuf};

use ropey::Rope;

use crate::services::{encoding, fs};

use super::edit;

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
    Create {
        path: PathBuf,
        text: String,
    },
    /// Literal text in workspace files. `open` holds the text of open buffers, which
    /// wins over the bytes on disk. `glob` narrows the files, like `*.rs` or `src/**`.
    Search {
        root: PathBuf,
        query: String,
        glob: Option<String>,
        open: Vec<(PathBuf, Rope)>,
    },
}

/// What the model should read, and a file to open when a create succeeded.
pub struct DiskReport {
    pub message: String,
    pub open: Option<PathBuf>,
    /// A successful read: the file and the fingerprint of the text the model saw.
    pub read: Option<(PathBuf, u64)>,
}

const MAX_NAMES: usize = 200;
pub const MAX_BYTES: usize = 2_000_000;
/// Matching lines a search returns.
const MAX_HITS: usize = 60;
/// Files a search opens before it stops.
const MAX_SEARCH_FILES: usize = 5_000;

/// The message is what the model reads, including a failure.
pub fn perform(task: DiskTask) -> DiskReport {
    match task {
        DiskTask::Read { path, start, end } => match read_decoded(&path) {
            Ok(text) => match edit::read_lines(&text, start, end) {
                Ok(lines) => DiskReport {
                    message: lines,
                    open: None,
                    read: Some((path, crate::buffer::fingerprint_bytes(text.as_bytes()))),
                },
                Err(err) => report(err),
            },
            Err(err) => report(err),
        },
        DiskTask::List { path } => report(match list_dir(&path) {
            Ok(text) => text,
            Err(err) => format!("Cannot list {}: {err}", path.display()),
        }),
        DiskTask::Create { path, text } => create_file(path, text),
        DiskTask::Search {
            root,
            query,
            glob,
            open,
        } => report(search(&root, &query, glob.as_deref(), &open)),
    }
}

fn report(message: String) -> DiskReport {
    DiskReport {
        message,
        open: None,
        read: None,
    }
}

/// The text the editor would load, so its fingerprint matches the buffer's.
fn read_decoded(path: &Path) -> Result<String, String> {
    check_file_size(path)?;
    encoding::read_auto(path)
        .map(|decoded| decoded.text)
        .map_err(|err| err.message(path))
}

/// `path:line: text` for each line holding `query`. Lowercase queries ignore case.
fn search(root: &Path, query: &str, glob: Option<&str>, open: &[(PathBuf, Rope)]) -> String {
    let query = query.trim_end_matches(['\r', '\n']);
    if query.trim().is_empty() {
        return "query is empty".to_string();
    }
    let fold = !query.chars().any(char::is_uppercase);
    let needle = if fold { query.to_lowercase() } else { query.to_string() };
    let mut builder = ignore::WalkBuilder::new(root);
    builder.hidden(true).git_ignore(true).require_git(false);
    if let Some(glob) = glob.map(str::trim).filter(|glob| !glob.is_empty()) {
        let mut overrides = ignore::overrides::OverrideBuilder::new(root);
        let pattern = if glob.contains('/') {
            glob.to_string()
        } else {
            format!("**/{glob}")
        };
        if let Err(err) = overrides.add(&pattern) {
            return format!("Bad glob {glob}: {err}");
        }
        match overrides.build() {
            Ok(built) => {
                builder.overrides(built);
            }
            Err(err) => return format!("Bad glob {glob}: {err}"),
        }
    }
    let mut hits = Vec::new();
    let mut files = 0usize;
    let mut more = false;
    for entry in builder.build().flatten() {
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        files += 1;
        if files > MAX_SEARCH_FILES {
            more = true;
            break;
        }
        let path = entry.path();
        let text = match open
            .iter()
            .find(|(open_path, _)| crate::session::same_directory(open_path, path))
        {
            Some((_, rope)) => rope.to_string(),
            None => {
                if check_file_size(path).is_err() {
                    continue;
                }
                match std::fs::read(path) {
                    Ok(bytes) if !bytes.contains(&0) => String::from_utf8_lossy(&bytes).into_owned(),
                    _ => continue,
                }
            }
        };
        let relative = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        for (index, line) in text.lines().enumerate() {
            let found = if fold {
                line.to_lowercase().contains(&needle)
            } else {
                line.contains(&needle)
            };
            if !found {
                continue;
            }
            if hits.len() >= MAX_HITS {
                more = true;
                break;
            }
            let line: String = line.trim().chars().take(160).collect();
            hits.push(format!("{relative}:{}: {line}", index + 1));
        }
        if more {
            break;
        }
    }
    if hits.is_empty() {
        return format!("No matches for {query}");
    }
    if more {
        hits.push(format!("… (stopped at {MAX_HITS} matches; narrow the query or glob)"));
    }
    hits.join("\n")
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

/// How many directory components may be created for a new file (parent must mostly exist).
pub const MAX_NEW_DIRS: usize = 1;

/// Refuses create_file paths whose missing parents are deeper than [`MAX_NEW_DIRS`].
pub fn create_parent_plan(path: &Path) -> Result<CreateParentPlan, String> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    let Some(parent) = parent else {
        return Ok(CreateParentPlan {
            parent: PathBuf::from("."),
            missing_dirs: 0,
        });
    };
    if parent.exists() {
        return Ok(CreateParentPlan {
            parent: parent.to_path_buf(),
            missing_dirs: 0,
        });
    }
    let mut missing = 0usize;
    let mut cursor = parent.to_path_buf();
    while !cursor.exists() {
        missing += 1;
        if missing > MAX_NEW_DIRS {
            return Err(format!(
                "Parent directory does not exist (refusing to create deep path): {}",
                parent.display()
            ));
        }
        match cursor.parent() {
            Some(next) if !next.as_os_str().is_empty() => cursor = next.to_path_buf(),
            _ => break,
        }
    }
    Ok(CreateParentPlan {
        parent: parent.to_path_buf(),
        missing_dirs: missing,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateParentPlan {
    pub parent: PathBuf,
    pub missing_dirs: usize,
}

fn create_file(path: PathBuf, text: String) -> DiskReport {
    if text.len() > MAX_BYTES {
        return report("File is too large".to_string());
    }
    if path.exists() {
        return report(format!("File already exists: {}", path.display()));
    }
    let plan = match create_parent_plan(&path) {
        Ok(plan) => plan,
        Err(err) => return report(err),
    };
    if plan.missing_dirs > 0 {
        if let Err(err) = std::fs::create_dir_all(&plan.parent) {
            return report(format!("Cannot create {}: {err}", plan.parent.display()));
        }
    }
    match fs::write_atomic(&path, &text) {
        Ok(()) => DiskReport {
            message: format!("Created {}", path.display()),
            open: Some(path),
            read: None,
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
    use super::{DiskTask, MAX_BYTES, check_file_size, create_parent_plan, perform};
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

    #[test]
    fn create_file_refuses_deep_missing_parents() {
        let root = temp_file("deep-root");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let deep = root.join("a").join("b").join("c").join("file.txt");
        let err = create_parent_plan(&deep).unwrap_err();
        assert!(err.contains("deep path") || err.contains("does not exist"), "{err}");
        let report = perform(DiskTask::Create {
            path: deep,
            text: "nope".into(),
        });
        assert!(report.open.is_none());
        assert!(
            report.message.contains("does not exist") || report.message.contains("deep"),
            "{}",
            report.message
        );
        let shallow = root.join("only").join("file.txt");
        let plan = create_parent_plan(&shallow).unwrap();
        assert_eq!(plan.missing_dirs, 1);
        let made = perform(DiskTask::Create {
            path: shallow.clone(),
            text: "ok\n".into(),
        });
        assert!(made.open.is_some(), "{}", made.message);
        assert_eq!(fs::read_to_string(&shallow).unwrap(), "ok\n");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_read_reports_the_fingerprint_of_the_text_it_returned() {
        let path = temp_file("read.txt");
        fs::write(&path, "one\ntwo\n").unwrap();
        let report = perform(DiskTask::Read {
            path: path.clone(),
            start: 1,
            end: 0,
        });
        assert!(report.message.contains("2|two"), "{}", report.message);
        let (seen, hash) = report.read.expect("read mark");
        assert_eq!(seen, path);
        assert_eq!(hash, crate::buffer::Buffer::from_text("one\ntwo\n").fingerprint());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn search_finds_lines_prefers_open_text_and_honors_the_glob() {
        let root = temp_file("search-root");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src").join("a.rs"), "fn alpha() {}\nfn beta() {}\n").unwrap();
        fs::write(root.join("notes.md"), "Alpha notes\n").unwrap();
        let found = perform(DiskTask::Search {
            root: root.clone(),
            query: "alpha".into(),
            glob: None,
            open: Vec::new(),
        })
        .message;
        assert!(found.contains("src/a.rs:1: fn alpha() {}"), "{found}");
        assert!(found.contains("notes.md:1: Alpha notes"), "lowercase ignores case: {found}");

        let only_rs = perform(DiskTask::Search {
            root: root.clone(),
            query: "alpha".into(),
            glob: Some("*.rs".into()),
            open: Vec::new(),
        })
        .message;
        assert!(!only_rs.contains("notes.md"), "{only_rs}");

        let open = vec![(
            root.join("src").join("a.rs"),
            ropey::Rope::from_str("fn gamma() {}\n"),
        )];
        let edited = perform(DiskTask::Search {
            root: root.clone(),
            query: "gamma".into(),
            glob: None,
            open,
        })
        .message;
        assert!(edited.contains("src/a.rs:1: fn gamma() {}"), "{edited}");
        let _ = fs::remove_dir_all(&root);
    }
}
