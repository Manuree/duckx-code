//! Open files and window layout for each folder, from the last quit in that folder.
//!
//! Stored as `session.json` beside `settings.json`. One file holds every folder.
//! A start with no path argument restores the entry whose root is the current
//! directory. A different directory does not reopen another folder's files.
//! A path argument leaves the file alone until the next quit.
//! `workspace` writes the folder being left, then restores the folder being opened.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::layout::Axis;

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    pub root: PathBuf,
    pub sidebar: Sidebar,
    /// The sidebar had the keyboard when the session was saved.
    #[serde(default)]
    pub sidebar_focused: bool,
    pub layout: SessionLayout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidebar {
    pub open: bool,
    pub width: u16,
    /// Folder paths relative to `root`, with `/` between components.
    #[serde(default)]
    pub expanded: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTab {
    pub path: String,
    #[serde(default)]
    pub line: usize,
    #[serde(default)]
    pub col: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SessionLayout {
    Leaf {
        /// This pane was the active editor.
        #[serde(default)]
        selected: bool,
        /// Index into `tabs` of the tab that was showing.
        #[serde(default)]
        active: usize,
        #[serde(default)]
        tabs: Vec<SessionTab>,
    },
    Split {
        axis: Axis,
        first: Box<SessionLayout>,
        second: Box<SessionLayout>,
    },
}

/// How many folders `session.json` keeps. The folder just quit is first.
const MAX_FOLDERS: usize = 20;

#[derive(Debug, Serialize, Deserialize)]
struct Store {
    version: u32,
    workspaces: Vec<Session>,
}

pub fn file_in(dir: &Path) -> PathBuf {
    dir.join("session.json")
}

/// Every saved folder, newest first. A missing or unreadable file is an empty list.
pub fn read_all(path: &Path) -> Vec<Session> {
    load_all(path).unwrap_or_default()
}

/// The saved session for `root`, if this file has one.
/// An older file with a single session still matches that session's folder.
pub fn for_directory(path: &Path, root: &Path) -> Option<Session> {
    read_all(path)
        .into_iter()
        .find(|session| same_directory(&session.root, root))
}

/// Replaces the entry for `session.root` and leaves every other folder in place.
pub fn upsert(path: &Path, session: &Session) -> Result<(), String> {
    let mut workspaces = load_all(path)?;
    workspaces.retain(|saved| !same_directory(&saved.root, &session.root));
    workspaces.insert(0, session.clone());
    workspaces.truncate(MAX_FOLDERS);
    let store = Store {
        version: VERSION,
        workspaces,
    };
    let text = serde_json::to_string_pretty(&store)
        .map_err(|err| format!("Could not write {}: {err}", path.display()))?;
    crate::services::fs::write_atomic(path, &format!("{text}\n"))
}

fn load_all(path: &Path) -> Result<Vec<Session>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("Could not read {}: {err}", path.display())),
    };
    parse_store(&text).ok_or_else(|| format!("Could not read {}", path.display()))
}

/// A list of folders, or the older single-session document.
fn parse_store(text: &str) -> Option<Vec<Session>> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    if value.get("workspaces").is_some() {
        let store: Store = serde_json::from_value(value).ok()?;
        if store.version != VERSION {
            return None;
        }
        return Some(
            store
                .workspaces
                .into_iter()
                .filter(|session| session.version == VERSION)
                .collect(),
        );
    }
    let session: Session = serde_json::from_value(value).ok()?;
    if session.version != VERSION {
        return None;
    }
    Some(vec![session])
}

/// True when both paths are the same directory. Windows compares without case
/// and ignores a `\\?\` verbatim prefix from canonicalize.
pub fn same_directory(a: &Path, b: &Path) -> bool {
    let (Ok(a), Ok(b)) = (std::path::absolute(a), std::path::absolute(b)) else {
        return false;
    };
    let a = dunce::simplified(&a).to_path_buf();
    let b = dunce::simplified(&b).to_path_buf();
    let left: Vec<_> = a.components().collect();
    let right: Vec<_> = b.components().collect();
    if left.len() != right.len() {
        return false;
    }
    left.iter().zip(right.iter()).all(|(left, right)| {
        let left = left.as_os_str();
        let right = right.as_os_str();
        if cfg!(windows) {
            left.to_string_lossy()
                .eq_ignore_ascii_case(&right.to_string_lossy())
        } else {
            left == right
        }
    })
}

pub fn parse(text: &str) -> Option<Session> {
    let session: Session = serde_json::from_str(text).ok()?;
    if session.version != VERSION || !session.root.is_dir() {
        return None;
    }
    Some(session)
}

pub fn to_json(session: &Session) -> Option<String> {
    let text = serde_json::to_string_pretty(session).ok()?;
    Some(format!("{text}\n"))
}

/// Relative path with `/`, matching the strings stored in `session.json`.
pub fn display_rel(path: &Path) -> String {
    let mut out = String::new();
    for (index, component) in path.components().enumerate() {
        if index > 0 {
            out.push('/');
        }
        out.push_str(&component.as_os_str().to_string_lossy());
    }
    out
}

/// A session path such as `src/app.rs`. `..` is rejected so a file cannot point outside the folder.
pub fn rel_path(text: &str) -> Option<PathBuf> {
    if text.is_empty() {
        return None;
    }
    let mut out = PathBuf::new();
    for part in text.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return None;
        }
        out.push(part);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

pub fn join_rel(root: &Path, rel: &str) -> Option<PathBuf> {
    Some(root.join(rel_path(rel)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rejects_a_newer_version_and_a_missing_folder() {
        let root = std::env::temp_dir();
        let text = format!(
            r#"{{"version":2,"root":{},"sidebar":{{"open":false,"width":30}},"layout":{{"kind":"leaf"}}}}"#,
            serde_json::to_string(&root).unwrap()
        );
        assert!(parse(&text).is_none());
        let text = r#"{"version":1,"root":"__duckx_missing_session_dir__","sidebar":{"open":false,"width":30},"layout":{"kind":"leaf"}}"#;
        assert!(parse(text).is_none());
    }

    #[test]
    fn json_roundtrip_keeps_the_split_and_the_cursor() {
        let session = Session {
            version: VERSION,
            root: std::env::temp_dir(),
            sidebar: Sidebar {
                open: true,
                width: 34,
                expanded: vec!["src".to_string(), "src/ui".to_string()],
            },
            sidebar_focused: false,
            layout: SessionLayout::Split {
                axis: Axis::Horizontal,
                first: Box::new(SessionLayout::Leaf {
                    selected: false,
                    active: 0,
                    tabs: vec![SessionTab {
                        path: "src/app.rs".to_string(),
                        line: 12,
                        col: 4,
                    }],
                }),
                second: Box::new(SessionLayout::Leaf {
                    selected: true,
                    active: 0,
                    tabs: vec![SessionTab {
                        path: "DOCUMENTS.md".to_string(),
                        line: 1,
                        col: 0,
                    }],
                }),
            },
        };
        let restored = parse(&to_json(&session).unwrap()).unwrap();
        assert_eq!(restored, session);
    }

    #[test]
    fn relative_paths_reject_a_parent_segment() {
        let root = Path::new("E:/work");
        assert_eq!(
            join_rel(root, "src/app.rs"),
            Some(PathBuf::from("E:/work").join("src").join("app.rs"))
        );
        assert!(join_rel(root, "../secret").is_none());
        assert!(join_rel(root, "src/../../secret").is_none());
        assert!(rel_path("").is_none());
    }

    #[test]
    fn a_second_folder_keeps_its_own_session() {
        let dir = std::env::temp_dir().join(format!(
            "duckx-sessions-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _guard = DirGuard(dir.clone());
        let here = dir.join("here");
        let there = dir.join("there");
        std::fs::create_dir_all(&here).unwrap();
        std::fs::create_dir_all(&there).unwrap();
        let file = dir.join("session.json");
        let older = leaf_session(&there, "there.rs");
        std::fs::write(&file, to_json(&older).unwrap()).unwrap();

        assert!(for_directory(&file, &here).is_none());
        assert_eq!(for_directory(&file, &there).unwrap().root, there);

        upsert(&file, &leaf_session(&here, "here.rs")).unwrap();
        assert_eq!(
            for_directory(&file, &here).unwrap().layout,
            leaf_session(&here, "here.rs").layout
        );
        assert_eq!(for_directory(&file, &there).unwrap().root, there);
    }

    fn leaf_session(root: &Path, file: &str) -> Session {
        Session {
            version: VERSION,
            root: root.to_path_buf(),
            sidebar: Sidebar {
                open: false,
                width: 30,
                expanded: Vec::new(),
            },
            sidebar_focused: false,
            layout: SessionLayout::Leaf {
                selected: true,
                active: 0,
                tabs: vec![SessionTab {
                    path: file.to_string(),
                    line: 0,
                    col: 0,
                }],
            },
        }
    }

    #[test]
    fn restore_reopens_files_at_the_saved_cursor_and_skips_the_rest() {
        let dir = std::env::temp_dir().join(format!(
            "duckx-session-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _guard = DirGuard(dir.clone());
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let note = dir.join("src").join("note.rs");
        let other = dir.join("other.rs");
        std::fs::write(&note, "one\ntwo\nthree\n").unwrap();
        std::fs::write(&other, "alpha\nbeta\n").unwrap();

        let root = std::path::absolute(&dir).unwrap();
        let mut app = crate::app::App::new(
            crate::buffer::Buffer::from_text("one\ntwo\nthree\n"),
            Some(note),
        );
        app.explorer = crate::explorer::Explorer::new(root.clone());
        app.explorer.open = true;
        app.explorer.set_expanded([PathBuf::from("src")]);
        app.handle(crate::action::SemanticAction::Move {
            movement: crate::buffer::Movement::Down,
            extend: false,
        });
        app.handle(crate::action::SemanticAction::OpenCommandBar);
        app.handle(crate::action::SemanticAction::CommandBar(
            crate::action::CommandBarAction::Insert("split-right".into()),
        ));
        app.handle(crate::action::SemanticAction::CommandBar(
            crate::action::CommandBarAction::Submit,
        ));
        app.explorer.focused = true;

        let captured = app.session_snapshot().unwrap();
        assert!(captured.sidebar_focused);
        assert_eq!(captured.sidebar.expanded, ["src"]);
        match &captured.layout {
            SessionLayout::Split {
                axis: Axis::Horizontal,
                first,
                second,
            } => {
                match first.as_ref() {
                    SessionLayout::Leaf { selected, tabs, .. } => {
                        assert!(!selected);
                        assert_eq!(tabs.len(), 1);
                        assert_eq!(tabs[0].path, "src/note.rs");
                        assert_eq!((tabs[0].line, tabs[0].col), (1, 0));
                    }
                    other => panic!("first pane, got {other:?}"),
                }
                match second.as_ref() {
                    SessionLayout::Leaf { selected, tabs, .. } => {
                        assert!(*selected);
                        assert!(tabs.is_empty());
                    }
                    other => panic!("second pane, got {other:?}"),
                }
            }
            other => panic!("split, got {other:?}"),
        }

        let session = Session {
            version: VERSION,
            root: root.clone(),
            sidebar: Sidebar {
                open: true,
                width: 34,
                expanded: vec!["src".to_string()],
            },
            sidebar_focused: true,
            layout: SessionLayout::Split {
                axis: Axis::Horizontal,
                first: Box::new(SessionLayout::Leaf {
                    selected: false,
                    active: 1,
                    tabs: vec![
                        SessionTab {
                            path: "other.rs".into(),
                            line: 1,
                            col: 2,
                        },
                        SessionTab {
                            path: "src/note.rs".into(),
                            line: 1,
                            col: 0,
                        },
                        SessionTab {
                            path: "gone.rs".into(),
                            line: 0,
                            col: 0,
                        },
                        SessionTab {
                            path: "../secret.rs".into(),
                            line: 0,
                            col: 0,
                        },
                    ],
                }),
                second: Box::new(SessionLayout::Leaf {
                    selected: true,
                    active: 0,
                    tabs: Vec::new(),
                }),
            },
        };

        let mut restored = crate::app::App::new(crate::buffer::Buffer::new(), None);
        restored.restore_session(session);
        assert_eq!(restored.explorer.root, root);
        assert!(restored.explorer.open);
        assert!(restored.explorer.focused);
        assert_eq!(restored.sidebar_width(), 34);
        assert_eq!(
            restored
                .explorer
                .expanded_paths()
                .iter()
                .map(|path| display_rel(path))
                .collect::<Vec<_>>(),
            ["src"]
        );

        let mut ids = Vec::new();
        leaves(restored.layout(), &mut ids);
        assert_eq!(ids.len(), 2);
        assert_eq!(restored.active_id(), ids[1]);
        assert_eq!(restored.editor(ids[0]).tabs().len(), 2);
        assert_eq!(restored.editor(ids[0]).active_index(), 1);
        assert!(restored.editor(ids[1]).tab().path.is_none());

        let mut loads = Vec::new();
        for job in restored.take_jobs() {
            match &job {
                crate::jobs::Job::LoadFile { path, kind, .. } => {
                    assert!(
                        matches!(
                            kind,
                            crate::jobs::LoadKind::Open {
                                place: Some(_),
                                focus: false,
                                ..
                            }
                        ),
                        "session loads must not steal focus"
                    );
                    assert!(path.ends_with("note.rs") || path.ends_with("other.rs"));
                }
                crate::jobs::Job::ScanTree(..) | crate::jobs::Job::GitStatus(_) => continue,
                other => panic!("unexpected job {other:?}"),
            }
            loads.push(job);
        }
        assert_eq!(loads.len(), 2);
        for job in loads {
            restored.handle_event(crate::jobs::run(job));
        }

        assert!(restored.explorer.focused);
        assert_eq!(restored.active_id(), ids[1]);
        assert!(restored.editor(ids[1]).tab().path.is_none());
        let pane = restored.editor(ids[0]);
        assert_eq!(pane.tabs()[0].buffer.cursor_line_char(), (1, 2));
        assert_eq!(pane.tabs()[1].buffer.cursor_line_char(), (1, 0));
        assert!(pane.tabs()[0].path.as_ref().unwrap().ends_with("other.rs"));
        assert!(pane.tabs()[1].path.as_ref().unwrap().ends_with("note.rs"));
    }

    /// Keeps the temporary folder until the test ends, including on failure.
    struct DirGuard(PathBuf);

    impl Drop for DirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn leaves(layout: &crate::layout::Layout, out: &mut Vec<crate::layout::EditorId>) {
        match layout {
            crate::layout::Layout::Leaf(id) => out.push(*id),
            crate::layout::Layout::Split { first, second, .. } => {
                leaves(first, out);
                leaves(second, out);
            }
        }
    }
}
