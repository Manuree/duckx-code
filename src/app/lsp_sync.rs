//! Keeps the language server worker in step with the open tabs, and applies what it reports.
//!
//! Each frame the main loop calls [`App::take_lsp_requests`], which compares every
//! file tab with what the worker was last told: new tabs are opened, closed or
//! renamed tabs are closed, and edited tabs send one merged change.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use super::{App, Tab};
use crate::layout::TabId;
use crate::lsp::diagnostics::{self, Span};
use crate::lsp::{self, LspEvent, LspRequest, ServerStatus, Severity, TextChange, registry};
use crate::syntax::Language;

/// Changes remembered so diagnostics computed against an older version can be moved forward.
const HISTORY: usize = 64;

#[derive(Debug)]
pub(super) struct SyncedDoc {
    path: PathBuf,
    language: Language,
    epoch: u64,
    revision: u64,
    version: u64,
    len_bytes: usize,
    /// `(version, span)` for each change sent; `None` was a whole-file change.
    history: VecDeque<(u64, Option<Span>)>,
}

/// The status bar text for the active file's language server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LspLabel {
    pub name: &'static str,
    pub status: ServerStatus,
}

/// A tab as the sync pass sees it: id, the file and language when it should have a server, epoch, revision.
type TabState = (TabId, Option<(PathBuf, Language)>, u64, u64);

fn eligible(tab: &Tab, enabled: bool) -> Option<(PathBuf, Language)> {
    if !enabled {
        return None;
    }
    let path = tab.path.as_ref()?;
    let language = Language::detect(path).filter(|language| registry::supports(*language))?;
    (tab.buffer.rope().len_bytes() <= lsp::MAX_DOCUMENT_BYTES).then(|| (path.clone(), language))
}

impl App {
    /// Brings the worker up to date with every tab, then hands over the queued requests.
    pub fn take_lsp_requests(&mut self) -> Vec<LspRequest> {
        self.sync_lsp();
        let mut requests = std::mem::take(&mut self.lsp_requests);
        if self.lsp_prefs_dirty {
            self.lsp_prefs_dirty = false;
            requests.insert(0, LspRequest::Configure(self.appearance.lsp.clone()));
        }
        requests.append(&mut self.lsp_nav_requests);
        requests
    }

    fn sync_lsp(&mut self) {
        let enabled = self.appearance.lsp.enabled;
        let tabs: Vec<TabState> = self
            .editors
            .iter()
            .flat_map(|editor| editor.tabs.iter())
            .map(|tab| {
                (
                    tab.id,
                    eligible(tab, enabled),
                    tab.epoch,
                    tab.buffer.revision(),
                )
            })
            .collect();

        let stale: Vec<TabId> = self
            .lsp_docs
            .iter()
            .filter(|(id, doc)| {
                !tabs.iter().any(|(tab_id, wanted, _, _)| {
                    tab_id == *id
                        && wanted.as_ref().is_some_and(|(path, language)| {
                            *path == doc.path && *language == doc.language
                        })
                })
            })
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            if let Some(doc) = self.lsp_docs.remove(&id) {
                self.diagnostics.clear(&doc.path);
                self.semantic.clear(&doc.path);
                self.lsp_requests.push(LspRequest::Close { path: doc.path });
            }
        }

        for (id, wanted, epoch, revision) in tabs {
            let Some((path, language)) = wanted else {
                if let Some(tab) = self.find_tab_mut(id) {
                    tab.lsp_edits.clear();
                }
                continue;
            };
            let known = self.lsp_docs.get(&id).map(|doc| (doc.epoch, doc.revision));
            match known {
                None => {
                    self.sync_tab_syntax(id);
                    let Some(tab) = self.find_tab_mut(id) else {
                        continue;
                    };
                    tab.lsp_edits.clear();
                    let text = tab.buffer.rope().clone();
                    self.lsp_docs.insert(
                        id,
                        SyncedDoc {
                            path: path.clone(),
                            language,
                            epoch,
                            revision,
                            version: 1,
                            len_bytes: text.len_bytes(),
                            history: VecDeque::new(),
                        },
                    );
                    self.lsp_requests.push(LspRequest::Open {
                        path,
                        language,
                        text,
                        version: 1,
                        workspace: self.explorer.root.clone(),
                    });
                }
                Some(seen) if seen != (epoch, revision) => {
                    self.sync_tab_syntax(id);
                    let Some(tab) = self.find_tab_mut(id) else {
                        continue;
                    };
                    let edits = std::mem::take(&mut tab.lsp_edits);
                    let text = tab.buffer.rope().clone();
                    let Some(doc) = self.lsp_docs.get_mut(&id) else {
                        continue;
                    };
                    let span = (doc.epoch == epoch)
                        .then(|| diagnostics::merge_edits(&edits))
                        .flatten()
                        .filter(|span| {
                            doc.len_bytes as isize + span.delta() == text.len_bytes() as isize
                        });
                    doc.version += 1;
                    doc.epoch = epoch;
                    doc.revision = revision;
                    doc.len_bytes = text.len_bytes();
                    doc.history.push_back((doc.version, span));
                    if doc.history.len() > HISTORY {
                        doc.history.pop_front();
                    }
                    let version = doc.version;
                    let change = match span {
                        Some(span) => {
                            self.diagnostics.shift(&path, span);
                            self.semantic.shift(&path, span);
                            TextChange::Range {
                                start: span.start,
                                old_end: span.old_end,
                                new_end: span.new_end,
                            }
                        }
                        None => TextChange::Full,
                    };
                    self.lsp_requests.push(LspRequest::Change {
                        path,
                        text,
                        version,
                        change,
                    });
                }
                Some(_) => {}
            }
        }
    }

    /// `didSave` after the bytes are on disk. Pending edits go first so the server sees them in order.
    pub(super) fn notify_lsp_saved(&mut self, tab_id: TabId) {
        self.sync_lsp();
        if let Some(doc) = self.lsp_docs.get(&tab_id) {
            self.lsp_requests.push(LspRequest::Save {
                path: doc.path.clone(),
            });
        }
    }

    pub fn handle_lsp(&mut self, event: LspEvent) {
        match event {
            LspEvent::Diagnostics {
                path,
                version,
                mut diagnostics,
            } => {
                let Some(doc) = self.lsp_docs.values().find(|doc| doc.path == path) else {
                    return;
                };
                for (_, span) in doc.history.iter().filter(|(sent, _)| *sent > version) {
                    match span {
                        Some(span) => diagnostics::shift_all(&mut diagnostics, *span),
                        None => break,
                    }
                }
                self.diagnostics.set(path, diagnostics);
            }
            LspEvent::PulledDiagnostics {
                path,
                version,
                mut diagnostics,
            } => {
                let Some(doc) = self.lsp_docs.values().find(|doc| doc.path == path) else {
                    return;
                };
                for (_, span) in doc.history.iter().filter(|(sent, _)| *sent > version) {
                    match span {
                        Some(span) => diagnostics::shift_all(&mut diagnostics, *span),
                        None => break,
                    }
                }
                self.diagnostics.set_layer(path, diagnostics, true);
            }
            LspEvent::Status {
                group,
                name,
                status,
            } => {
                self.lsp_servers.insert(group, (name, status));
            }
            LspEvent::Notice(text) => self.message = Some(text),
            LspEvent::Features {
                group,
                hover,
                definition,
                completion,
                triggers,
                resolve,
                signature,
                rename,
                format,
                code_action,
                action_resolve,
            } => self.handle_features(
                group,
                hover,
                definition,
                completion,
                triggers,
                resolve,
                signature,
                rename,
                format,
                code_action,
                action_resolve,
            ),
            LspEvent::WorkspaceEdit {
                token,
                encoding,
                edits,
            } => self.apply_workspace_edit(token, encoding, edits),
            LspEvent::Actions {
                token,
                encoding,
                actions,
            } => self.show_actions(token, encoding, actions),
            LspEvent::Signature { token, text } => self.handle_signature(token, text),
            LspEvent::Resolved {
                token,
                detail,
                docs,
            } => self.handle_resolved(token, detail, docs),
            LspEvent::SemanticTokens {
                path,
                version,
                mut spans,
            } => {
                let Some(doc) = self.lsp_docs.values().find(|doc| doc.path == path) else {
                    return;
                };
                for (_, span) in doc.history.iter().filter(|(sent, _)| *sent > version) {
                    match span {
                        Some(span) => {
                            for item in spans.iter_mut() {
                                item.0 = span.map(item.0, false);
                                item.1 = span.map(item.1, true).max(item.0);
                            }
                        }
                        None => break,
                    }
                }
                self.semantic.set(path, spans);
            }
            LspEvent::Hover { token, text } => self.handle_hover(token, text),
            LspEvent::Definition { token, locations } => self.handle_definition(token, locations),
            LspEvent::Completion {
                token,
                incomplete,
                items,
            } => self.handle_completion(token, incomplete, items),
        }
    }

    pub(super) fn goto_diagnostic(&mut self, next: bool) {
        let Some(path) = self.path().map(Path::to_path_buf) else {
            self.message = Some("No diagnostics in this file".to_string());
            return;
        };
        let rope = self.buffer().rope();
        let cursor = rope.char_to_byte(self.buffer().selection().head);
        let target = diagnostics::adjacent(self.diagnostics.get(&path), cursor, next)
            .map(|item| rope.byte_to_char(item.start.min(rope.len_bytes())));
        match target {
            Some(start) => self.buffer_mut().select_span(start, start),
            None => self.message = Some("No diagnostics in this file".to_string()),
        }
    }

    /// The diagnostic the status bar shows for the cursor line.
    pub fn cursor_diagnostic(&self) -> Option<&lsp::Diagnostic> {
        let path = self.path()?;
        let rope = self.buffer().rope();
        let head = self.buffer().selection().head;
        let line = rope.char_to_line(head);
        let line_start = rope.line_to_byte(line);
        let line_end = rope
            .line_to_byte((line + 1).min(rope.len_lines()))
            .max(line_start);
        let line_end = if line + 1 < rope.len_lines() {
            line_end.saturating_sub(1)
        } else {
            line_end
        };
        diagnostics::at_cursor(
            self.diagnostics.get(path),
            rope.char_to_byte(head),
            line_start,
            line_end,
        )
    }

    /// Server for the active file, once one has reported in.
    pub fn lsp_label(&self) -> Option<LspLabel> {
        let language = Language::detect(self.path()?)?;
        let group = registry::servers_for(language).first()?.name;
        let (name, status) = self.lsp_servers.get(group)?;
        Some(LspLabel {
            name,
            status: status.clone(),
        })
    }

    /// Errors, warnings, infos, and hints in the active file.
    pub fn diagnostic_counts(&self) -> [usize; 4] {
        self.path()
            .map(|path| self.diagnostics.counts(path))
            .unwrap_or_default()
    }

    pub(super) fn apply_workspace_edit(
        &mut self,
        token: u64,
        encoding: crate::lsp::position::Encoding,
        edits: Vec<crate::lsp::protocol::TextPatch>,
    ) {
        if self.nav.edit_token != Some(token) {
            return;
        }
        self.nav.edit_token = None;
        let save = self.nav.format_save.take();
        self.apply_patches(encoding, &edits);
        if let Some(save) = save {
            self.queue_save(save.editor, save.tab, save.path, save.after, false);
            return;
        }
        if edits.is_empty() {
            self.message = Some("No changes".to_string());
        }
    }

    /// Edits parsed in the editor, for a code action that already included them.
    pub(super) fn apply_described_edits(
        &mut self,
        encoding: crate::lsp::position::Encoding,
        edits: Vec<crate::lsp::plain::RawTextEdit>,
    ) {
        let patches = edits
            .into_iter()
            .filter_map(|edit| {
                let path = crate::lsp::uri::to_path(&edit.uri)?;
                Some(crate::lsp::protocol::TextPatch {
                    path,
                    version: None,
                    start: 0,
                    end: 0,
                    start_line: edit.range.line,
                    start_char: edit.range.character,
                    end_line: edit.range.end_line,
                    end_char: edit.range.end_character,
                    text: edit.text,
                })
            })
            .collect::<Vec<_>>();
        self.apply_patches(encoding, &patches);
    }

    fn apply_patches(
        &mut self,
        encoding: crate::lsp::position::Encoding,
        edits: &[crate::lsp::protocol::TextPatch],
    ) {
        use std::collections::BTreeMap;
        let mut by_path: BTreeMap<PathBuf, Vec<crate::lsp::protocol::TextPatch>> = BTreeMap::new();
        for edit in edits {
            by_path
                .entry(edit.path.clone())
                .or_default()
                .push(edit.clone());
        }
        let mut closed = Vec::new();
        let mut changed = 0usize;
        for (path, patches) in by_path {
            if self.apply_patches_to_open(&path, encoding, &patches) {
                changed += 1;
            } else {
                closed.extend(patches);
            }
        }
        if !closed.is_empty() {
            changed += closed.len();
            self.jobs.push(crate::jobs::Job::PatchFiles {
                encoding,
                edits: closed
                    .into_iter()
                    .map(|edit| crate::jobs::DiskEdit {
                        path: edit.path,
                        start_line: edit.start_line,
                        start_char: edit.start_char,
                        end_line: edit.end_line,
                        end_char: edit.end_char,
                        text: edit.text,
                    })
                    .collect(),
            });
        }
        if changed > 0 {
            self.message = Some(if changed == 1 {
                "Updated 1 file".to_string()
            } else {
                format!("Updated {changed} files")
            });
        }
    }

    fn apply_patches_to_open(
        &mut self,
        path: &Path,
        encoding: crate::lsp::position::Encoding,
        patches: &[crate::lsp::protocol::TextPatch],
    ) -> bool {
        let Some(doc) = self
            .lsp_docs
            .values()
            .find(|doc| super::same_path(&doc.path, path))
        else {
            return false;
        };
        let history: Vec<(u64, Option<Span>)> = doc.history.iter().cloned().collect();
        let located = self
            .editors
            .iter()
            .flat_map(|editor| editor.tabs.iter())
            .find(|tab| {
                tab.path
                    .as_deref()
                    .is_some_and(|tab_path| super::same_path(tab_path, path))
            })
            .map(|tab| tab.id);
        let Some(tab_id) = located else {
            return false;
        };
        let mut char_edits = Vec::new();
        {
            let Some(tab) = self.find_tab_mut(tab_id) else {
                return false;
            };
            let rope = tab.buffer.rope();
            for patch in patches {
                let (start, end) = if let Some(version) = patch.version {
                    let mut start = patch.start;
                    let mut end = patch.end;
                    let mut lost = false;
                    for (_sent, span) in history.iter().filter(|(sent, _)| *sent > version) {
                        let Some(span) = span else {
                            lost = true;
                            break;
                        };
                        start = span.map(start, false);
                        end = span.map(end, true).max(start);
                    }
                    if lost {
                        continue;
                    }
                    (start, end)
                } else {
                    let start = crate::lsp::position::byte_of(
                        rope,
                        patch.start_line,
                        patch.start_char,
                        encoding,
                    );
                    let end = crate::lsp::position::byte_of(
                        rope,
                        patch.end_line,
                        patch.end_char,
                        encoding,
                    )
                    .max(start);
                    (start, end)
                };
                let len = rope.len_bytes();
                let start = rope.byte_to_char(start.min(len));
                let end = rope.byte_to_char(end.min(len)).max(start);
                char_edits.push((start, end - start, patch.text.clone()));
            }
        }
        char_edits.sort_by_key(|edit| edit.0);
        let mut kept = Vec::new();
        let mut next = 0usize;
        for edit in char_edits {
            if edit.0 < next {
                continue;
            }
            next = edit.0 + edit.1;
            kept.push(edit);
        }
        if kept.is_empty() {
            return true;
        }
        let Some(tab) = self.find_tab_mut(tab_id) else {
            return false;
        };
        let cursor = tab.buffer.selection().head;
        tab.buffer.edit_batch(kept, cursor);
        tab.dirty = true;
        true
    }
}

impl Tab {
    /// Diagnostics touching `start_line..end_line`, as char ranges with their level.
    /// An empty range still covers one character so it can be underlined.
    pub fn diagnostic_spans(
        &self,
        store: &diagnostics::Diagnostics,
        start_line: usize,
        end_line: usize,
    ) -> Vec<(usize, usize, Severity)> {
        let Some(path) = self.path.as_deref() else {
            return Vec::new();
        };
        let rope = self.buffer.rope();
        let lines = rope.len_lines();
        let first = rope.line_to_byte(start_line.min(lines));
        let last = rope.line_to_byte(end_line.min(lines));
        let len = rope.len_bytes();
        store
            .get(path)
            .iter()
            .filter(|item| item.start <= last && item.end >= first)
            .map(|item| {
                let start = rope.byte_to_char(item.start.min(len));
                let end = rope.byte_to_char(item.end.min(len)).max(start + 1);
                (start, end, item.severity)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::SemanticAction;
    use crate::buffer::{Buffer, Movement};
    use crate::lsp::Diagnostic;

    fn rust_app(text: &str) -> App {
        App::new(Buffer::from_text(text), Some(PathBuf::from("main.rs")))
    }

    fn diag(start: usize, end: usize, severity: Severity, message: &str) -> Diagnostic {
        Diagnostic {
            start,
            end,
            severity,
            message: message.to_string(),
            source: None,
            code: None,
        }
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            app.handle(SemanticAction::InsertChar(c));
        }
    }

    #[test]
    fn a_file_tab_opens_once_and_typing_sends_one_merged_change() {
        let mut app = rust_app("fn main() {}\n");
        let requests = app.take_lsp_requests();
        assert!(matches!(
            requests.as_slice(),
            [LspRequest::Open {
                version: 1,
                language: Language::Rust,
                ..
            }]
        ));
        assert!(app.take_lsp_requests().is_empty());

        type_text(&mut app, "ab");
        let requests = app.take_lsp_requests();
        let [
            LspRequest::Change {
                version,
                change,
                text,
                ..
            },
        ] = requests.as_slice()
        else {
            panic!("expected one change, got {requests:?}");
        };
        assert_eq!(*version, 2);
        assert_eq!(
            *change,
            TextChange::Range {
                start: 0,
                old_end: 0,
                new_end: 2
            }
        );
        assert_eq!(text.to_string(), "abfn main() {}\n");
    }

    #[test]
    fn unsupported_and_unnamed_tabs_are_not_sent() {
        let mut notes = App::new(Buffer::from_text("# hi"), Some(PathBuf::from("notes.md")));
        assert!(notes.take_lsp_requests().is_empty());
        let mut scratch = App::new(Buffer::from_text("x"), None);
        assert!(scratch.take_lsp_requests().is_empty());
    }

    #[test]
    fn undo_is_sent_as_a_change_too() {
        let mut app = rust_app("x");
        app.take_lsp_requests();
        type_text(&mut app, "y");
        app.take_lsp_requests();
        app.handle(SemanticAction::Undo);
        let requests = app.take_lsp_requests();
        let [LspRequest::Change { text, change, .. }] = requests.as_slice() else {
            panic!("expected a change, got {requests:?}");
        };
        assert_eq!(text.to_string(), "x");
        assert_eq!(
            *change,
            TextChange::Range {
                start: 0,
                old_end: 1,
                new_end: 0
            }
        );
    }

    #[test]
    fn diagnostics_for_an_older_version_move_with_later_edits() {
        let mut app = rust_app("let x = 1;\n");
        app.take_lsp_requests();
        type_text(&mut app, "  ");
        app.take_lsp_requests();
        let path = app.path().unwrap().to_path_buf();
        app.handle_lsp(LspEvent::Diagnostics {
            path: path.clone(),
            version: 1,
            diagnostics: vec![diag(4, 5, Severity::Warning, "unused")],
        });
        assert_eq!(app.diagnostics.get(&path)[0].start, 6);

        // Typing in front again shifts the stored range locally.
        app.handle(SemanticAction::Move {
            movement: Movement::DocumentStart,
            extend: false,
        });
        type_text(&mut app, "#");
        app.take_lsp_requests();
        assert_eq!(
            (
                app.diagnostics.get(&path)[0].start,
                app.diagnostics.get(&path)[0].end
            ),
            (7, 8)
        );
    }

    #[test]
    fn f8_walks_diagnostics_and_the_status_bar_shows_the_one_on_the_cursor_line() {
        let mut app = rust_app("aaa\nbbb\nccc\n");
        app.take_lsp_requests();
        let path = app.path().unwrap().to_path_buf();
        app.handle_lsp(LspEvent::Diagnostics {
            path,
            version: 1,
            diagnostics: vec![
                diag(9, 10, Severity::Error, "third line"),
                diag(4, 7, Severity::Warning, "second line"),
            ],
        });
        assert!(app.cursor_diagnostic().is_none());
        assert_eq!(app.diagnostic_counts(), [1, 1, 0, 0]);

        app.handle(SemanticAction::GotoDiagnostic { next: true });
        assert_eq!(app.buffer().cursor_line_col(), (1, 0));
        assert_eq!(app.cursor_diagnostic().unwrap().message, "second line");
        app.handle(SemanticAction::GotoDiagnostic { next: true });
        assert_eq!(app.buffer().cursor_line_col(), (2, 1));
        app.handle(SemanticAction::GotoDiagnostic { next: true });
        assert_eq!(app.buffer().cursor_line_col(), (1, 0), "wraps to the first");
        app.handle(SemanticAction::GotoDiagnostic { next: false });
        assert_eq!(app.buffer().cursor_line_col(), (2, 1), "wraps to the last");
    }

    #[test]
    fn f8_without_diagnostics_says_so() {
        let mut app = rust_app("x");
        app.handle(SemanticAction::GotoDiagnostic { next: true });
        assert_eq!(app.message.as_deref(), Some("No diagnostics in this file"));
    }

    #[test]
    fn closing_a_tab_closes_the_document_and_drops_its_diagnostics() {
        let mut app = rust_app("x");
        app.take_lsp_requests();
        let path = app.path().unwrap().to_path_buf();
        app.handle_lsp(LspEvent::Diagnostics {
            path: path.clone(),
            version: 1,
            diagnostics: vec![diag(0, 1, Severity::Error, "bad")],
        });
        app.close_active().unwrap();
        let requests = app.take_lsp_requests();
        assert!(
            requests.iter().any(
                |request| matches!(request, LspRequest::Close { path: closed } if *closed == path)
            ),
            "{requests:?}"
        );
        assert!(app.diagnostics.get(&path).is_empty());
        app.handle_lsp(LspEvent::Diagnostics {
            path: path.clone(),
            version: 1,
            diagnostics: vec![diag(0, 1, Severity::Error, "late")],
        });
        assert!(
            app.diagnostics.get(&path).is_empty(),
            "late results for a closed file are dropped"
        );
    }

    #[test]
    fn server_status_is_shown_for_the_active_files_language() {
        let mut app = rust_app("x");
        assert!(app.lsp_label().is_none());
        app.handle_lsp(LspEvent::Status {
            group: "rust-analyzer",
            name: "rust-analyzer",
            status: ServerStatus::Busy("Indexing 3%".into()),
        });
        assert_eq!(
            app.lsp_label(),
            Some(LspLabel {
                name: "rust-analyzer",
                status: ServerStatus::Busy("Indexing 3%".into())
            })
        );
    }
}
