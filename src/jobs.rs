//! Background work requested by `App` and the results sent back to it.
//! `App` only queues `Job`s; the main loop runs them off the UI thread and feeds
//! each `AppEvent` back through `App::handle_event`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use ropey::Rope;

use crate::explorer::FileEntry;
use crate::layout::{EditorId, TabId};
use crate::picker::{self, IndexedFile};
use crate::services::encoding::{self, Decoded, FileStamp, OpenError, TextEncoding};
use crate::services::fs::{self, Change, ChangeOutcome};
use crate::services::git::{self, DiffResult, GitStatus};
use crate::services::tree;
use crate::syntax::{self, ParseInput, Parsed};

/// Which tabs a close command still has to finish after the current save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseBatch {
    /// Every tab in one editor window.
    View(EditorId),
    /// Every tab in every window.
    All,
}

/// What to do after a save finishes. Carried on the job so a later edit cannot mix up two saves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AfterSave {
    Nothing,
    Quit,
    Close,
    /// Continue `close-all` or `close-view` after this tab is saved.
    CloseBatch(CloseBatch),
    Open(PathBuf),
}

/// How a load should land in the buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadKind {
    /// First open. `None` detects UTF-8 or a BOM.
    /// `place` is a zero-based line and character column to put the cursor on.
    /// `focus` moves the editor to this tab when the bytes land.
    Open {
        encoding: Option<TextEncoding>,
        place: Option<(usize, usize)>,
        focus: bool,
    },
    /// Replace the buffer and put the cursor back at `(line, col)` in characters.
    /// `noticed` is the stamp that triggered an automatic reload. A failed read
    /// records it so the same bytes are not tried again every second.
    Reload {
        line: usize,
        col: usize,
        encoding: Option<TextEncoding>,
        noticed: Option<FileStamp>,
    },
}

/// One range in a file that is not open in the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskEdit {
    pub path: PathBuf,
    pub start_line: u32,
    pub start_char: u32,
    pub end_line: u32,
    pub end_char: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    ScanTree(PathBuf),
    GitStatus(PathBuf),
    /// `fingerprint` is the hash of `text`. `epoch` is the editor generation that owned those bytes.
    SaveFile {
        editor_id: EditorId,
        tab_id: TabId,
        path: PathBuf,
        text: String,
        encoding: TextEncoding,
        fingerprint: u64,
        epoch: u64,
        after: AfterSave,
    },
    LoadFile {
        editor_id: EditorId,
        tab_id: TabId,
        path: PathBuf,
        /// Bumped on the editor when this load was requested, so an older result is ignored.
        generation: u64,
        kind: LoadKind,
    },
    /// Compare the file's modified time and length with the stamp taken at load or save.
    StatFile {
        tab_id: TabId,
        path: PathBuf,
        stamp: FileStamp,
    },
    /// Delete, rename, or move paths inside `root`.
    Files {
        root: PathBuf,
        change: Change,
    },
    /// Write a small text file, such as `settings.json`.
    WriteText {
        path: PathBuf,
        text: String,
    },
    /// Count literal matches in explorer files. `Some` text replaces the bytes on disk.
    FindFiles {
        root: PathBuf,
        query: String,
        generation: u64,
        files: Vec<(PathBuf, Option<String>)>,
    },
    /// Apply language-server edits to files that are not open.
    PatchFiles {
        encoding: crate::lsp::position::Encoding,
        edits: Vec<DiskEdit>,
    },
    /// Replace `query` in closed files. Open buffers are edited by `App` itself.
    ReplaceFiles {
        root: PathBuf,
        query: String,
        replacement: String,
        generation: u64,
        files: Vec<PathBuf>,
    },
    /// Walk the workspace for the file picker.
    IndexFiles {
        root: PathBuf,
        generation: u64,
    },
    /// Score `files` against `query`. `recent` is most-recent first.
    FilterFiles {
        generation: u64,
        query: String,
        files: Arc<Vec<IndexedFile>>,
        recent: Vec<String>,
        active_dir: String,
    },
    /// Parse one tab. A result is stale when its generation or version no longer matches.
    Parse {
        tab_id: TabId,
        request: ParseInput,
    },
    /// Compare one buffer with `HEAD`. `text` is a rope clone, so the UI thread does not copy it.
    GitDiff {
        tab_id: TabId,
        path: PathBuf,
        text: Rope,
        fingerprint: u64,
        generation: u64,
    },
}

#[derive(Debug)]
pub enum AppEvent {
    TreeScanned {
        root: PathBuf,
        entries: Vec<FileEntry>,
    },
    GitStatus {
        root: PathBuf,
        statuses: HashMap<PathBuf, GitStatus>,
    },
    FileSaved {
        editor_id: EditorId,
        tab_id: TabId,
        path: PathBuf,
        fingerprint: u64,
        epoch: u64,
        after: AfterSave,
        result: Result<FileStamp, String>,
    },
    FileLoaded {
        editor_id: EditorId,
        tab_id: TabId,
        path: PathBuf,
        generation: u64,
        kind: LoadKind,
        result: Result<Decoded, OpenError>,
    },
    FileStatted {
        tab_id: TabId,
        path: PathBuf,
        /// Stamp the editor had when it asked. A newer save makes this result stale.
        stamp: FileStamp,
        result: Result<FileStamp, String>,
    },
    FilesChanged {
        root: PathBuf,
        outcome: ChangeOutcome,
    },
    TextWritten {
        path: PathBuf,
        result: Result<(), String>,
    },
    FilesFound {
        root: PathBuf,
        generation: u64,
        hits: Vec<(PathBuf, usize)>,
    },
    FilesPatched {
        files: usize,
        errors: Vec<String>,
    },
    FilesReplaced {
        root: PathBuf,
        generation: u64,
        matches: usize,
        files: usize,
        errors: Vec<String>,
    },
    FilesIndexed {
        root: PathBuf,
        generation: u64,
        files: Vec<IndexedFile>,
        truncated: bool,
    },
    PickerFiltered {
        generation: u64,
        indices: Vec<u32>,
        total: usize,
    },
    Parsed {
        tab_id: TabId,
        generation: u64,
        version: u64,
        parsed: Option<Parsed>,
    },
    GitDiff {
        tab_id: TabId,
        path: PathBuf,
        generation: u64,
        fingerprint: u64,
        result: DiffResult,
    },
}

/// Runs a job to completion. Blocking (file system walks, `git` subprocesses).
pub fn run(job: Job) -> AppEvent {
    match job {
        Job::ScanTree(root) => AppEvent::TreeScanned {
            entries: tree::scan(&root),
            root,
        },
        Job::GitStatus(root) => AppEvent::GitStatus {
            statuses: git::status(&root),
            root,
        },
        Job::SaveFile {
            editor_id,
            tab_id,
            path,
            text,
            encoding,
            fingerprint,
            epoch,
            after,
        } => AppEvent::FileSaved {
            result: encoding::write_encoded(&path, &text, encoding),
            editor_id,
            tab_id,
            path,
            fingerprint,
            epoch,
            after,
        },
        Job::LoadFile {
            editor_id,
            tab_id,
            path,
            generation,
            kind,
        } => AppEvent::FileLoaded {
            result: read_kind(&path, &kind),
            editor_id,
            tab_id,
            path,
            generation,
            kind,
        },
        Job::StatFile {
            tab_id,
            path,
            stamp,
        } => AppEvent::FileStatted {
            result: encoding::file_stamp(&path),
            tab_id,
            path,
            stamp,
        },
        Job::Files { root, change } => {
            let outcome = fs::apply(&root, &change);
            AppEvent::FilesChanged { root, outcome }
        }
        Job::WriteText { path, text } => AppEvent::TextWritten {
            result: fs::write_atomic(&path, &text),
            path,
        },
        Job::FindFiles {
            root,
            query,
            generation,
            files,
        } => AppEvent::FilesFound {
            hits: find_in_files(&root, &query, files),
            root,
            generation,
        },
        Job::PatchFiles { encoding, edits } => {
            let (files, errors) = patch_closed_files(encoding, edits);
            AppEvent::FilesPatched { files, errors }
        }
        Job::ReplaceFiles {
            root,
            query,
            replacement,
            generation,
            files,
        } => {
            let (matches, files, errors) = replace_in_files(&root, &query, &replacement, files);
            AppEvent::FilesReplaced {
                root,
                generation,
                matches,
                files,
                errors,
            }
        }
        Job::IndexFiles { root, generation } => {
            let (files, truncated) = picker::index_workspace(&root);
            AppEvent::FilesIndexed {
                root,
                generation,
                files,
                truncated,
            }
        }
        Job::FilterFiles {
            generation,
            query,
            files,
            recent,
            active_dir,
        } => {
            let indices = picker::filter(&files, &query, &recent, &active_dir);
            let total = indices.len();
            AppEvent::PickerFiltered {
                generation,
                indices,
                total,
            }
        }
        Job::Parse { tab_id, request } => AppEvent::Parsed {
            generation: request.generation,
            version: request.version,
            parsed: syntax::parse(request.language, &request.rope, request.old.as_ref()),
            tab_id,
        },
        Job::GitDiff {
            tab_id,
            path,
            text,
            fingerprint,
            generation,
        } => AppEvent::GitDiff {
            result: git::diff_against_head(&path, &text),
            tab_id,
            path,
            generation,
            fingerprint,
        },
    }
}

fn read_kind(path: &std::path::Path, kind: &LoadKind) -> Result<Decoded, OpenError> {
    let encoding = match kind {
        LoadKind::Open { encoding, .. } | LoadKind::Reload { encoding, .. } => *encoding,
    };
    match encoding {
        Some(encoding) => encoding::read_with(path, encoding),
        None => encoding::read_auto(path),
    }
}

fn patch_closed_files(
    encoding: crate::lsp::position::Encoding,
    edits: Vec<DiskEdit>,
) -> (usize, Vec<String>) {
    use std::collections::HashMap;
    let mut grouped: HashMap<PathBuf, Vec<DiskEdit>> = HashMap::new();
    for edit in edits {
        grouped.entry(edit.path.clone()).or_default().push(edit);
    }
    let mut files = 0;
    let mut errors = Vec::new();
    for (path, edits) in grouped {
        let decoded = match encoding::read_auto(&path) {
            Ok(decoded) => decoded,
            Err(err) => {
                errors.push(format!("{}: {}", path.display(), err.message(&path)));
                continue;
            }
        };
        let text = decoded.text;
        let mut rope = ropey::Rope::from_str(&text);
        let mut spans = Vec::new();
        for edit in &edits {
            let start =
                crate::lsp::position::byte_of(&rope, edit.start_line, edit.start_char, encoding);
            let end = crate::lsp::position::byte_of(&rope, edit.end_line, edit.end_char, encoding)
                .max(start);
            spans.push((start, end, edit.text.clone()));
        }
        spans.sort_by(|left, right| right.0.cmp(&left.0));
        for (start, end, text) in spans {
            let len = rope.len_bytes();
            let start = rope.byte_to_char(start.min(len));
            let end = rope.byte_to_char(end.min(len)).max(start);
            rope.remove(start..end);
            rope.insert(start, &text);
        }
        match encoding::write_encoded(&path, &rope.to_string(), decoded.encoding) {
            Ok(_) => files += 1,
            Err(err) => errors.push(format!("{}: {err}", path.display())),
        }
    }
    (files, errors)
}

fn find_in_files(
    root: &std::path::Path,
    query: &str,
    files: Vec<(PathBuf, Option<String>)>,
) -> Vec<(PathBuf, usize)> {
    files
        .into_iter()
        .filter_map(|(path, text)| {
            let count = match text {
                Some(text) => crate::buffer::Buffer::literal_match_count(&text, query),
                None => fs::read_text(&root.join(&path))
                    .map(|text| crate::buffer::Buffer::literal_match_count(&text, query))
                    .unwrap_or(0),
            };
            (count > 0).then_some((path, count))
        })
        .collect()
}

fn replace_in_files(
    root: &std::path::Path,
    query: &str,
    replacement: &str,
    files: Vec<PathBuf>,
) -> (usize, usize, Vec<String>) {
    let mut matches = 0;
    let mut changed = 0;
    let mut errors = Vec::new();
    for path in files {
        let abs = root.join(&path);
        let decoded = match encoding::read_auto(&abs) {
            Ok(decoded) => decoded,
            Err(_) => continue,
        };
        let (next, count) =
            crate::buffer::Buffer::replace_literal(&decoded.text, query, replacement);
        if count == 0 {
            continue;
        }
        if let Err(err) = encoding::write_encoded(&abs, &next, decoded.encoding) {
            errors.push(err);
            continue;
        }
        matches += count;
        changed += 1;
    }
    (matches, changed, errors)
}
