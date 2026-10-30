//! Hover, go to definition, and completion. Requests go out with the document
//! sync, and a reply for an older request is ignored.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ropey::Rope;

use super::App;
use crate::lsp::position;
use crate::lsp::protocol::{CompletionTrigger, JumpTarget, QueryKind};
use crate::lsp::{self, LspRequest};
use crate::syntax::Language;

const JUMPS: usize = 64;
const VISIBLE: usize = 8;

#[derive(Debug, Default)]
pub(super) struct Nav {
    next_token: u64,
    hover_token: Option<u64>,
    pub(super) hover: Option<String>,
    definition_token: Option<u64>,
    back: Vec<JumpPoint>,
    forward: Vec<JumpPoint>,
    pending: Option<(PathBuf, PendingPlace)>,
    pub(super) completion: Option<Menu>,
    pub(super) signature: Option<String>,
    signature_token: Option<u64>,
    resolve_token: Option<u64>,
    resolve_index: usize,
    pub(super) snippet: Option<SnippetSession>,
    features: HashMap<&'static str, Features>,
    pub(super) actions: Option<ActionList>,
    pub(super) edit_token: Option<u64>,
    /// Set when a format reply should be written to disk.
    pub(super) format_save: Option<FormatSave>,
}

#[derive(Debug)]
pub(super) struct FormatSave {
    pub editor: crate::layout::EditorId,
    pub tab: crate::layout::TabId,
    pub path: PathBuf,
    pub after: crate::jobs::AfterSave,
}

#[derive(Debug)]
pub(super) struct ActionList {
    pub selected: usize,
    pub encoding: crate::lsp::position::Encoding,
    pub items: Vec<crate::lsp::protocol::ActionOffer>,
}

pub struct ActionView {
    pub titles: Vec<String>,
    pub selected: usize,
}

#[derive(Debug)]
pub(super) struct SnippetSession {
    stops: Vec<(usize, usize)>,
    index: usize,
}

#[derive(Debug)]
struct JumpPoint {
    path: PathBuf,
    char_idx: usize,
}

#[derive(Debug)]
enum PendingPlace {
    Char(usize),
    Target(JumpTarget),
}

#[derive(Debug, Clone)]
struct Features {
    hover: bool,
    definition: bool,
    completion: bool,
    triggers: Vec<String>,
    resolve: bool,
    signature: bool,
    rename: bool,
    format: bool,
    code_action: bool,
    action_resolve: bool,
}

#[derive(Debug)]
pub(super) struct Menu {
    token: u64,
    /// Char index the prefix starts at, once a reply has arrived.
    origin: Option<usize>,
    incomplete: bool,
    items: Vec<Item>,
    /// Index into the filtered list.
    selected: usize,
    loading: bool,
    /// `Ctrl+Space`, so an empty result is still told to the user.
    explicit: bool,
    /// Text from `origin` to the cursor, used to filter without borrowing the buffer.
    prefix: String,
}

#[derive(Debug)]
struct Item {
    label: String,
    detail: String,
    filter: String,
    insert: String,
    kind: String,
    docs: String,
    snippet: bool,
    resolve: Option<String>,
    replace_at: Option<usize>,
    extra: Vec<(usize, usize, String)>,
}

/// What the completion popup draws. At most [`VISIBLE`] rows, with the selection on screen.
pub struct CompletionView {
    pub rows: Vec<CompletionRow>,
    pub selected: usize,
    pub docs: String,
}

pub struct CompletionRow {
    pub label: String,
    pub detail: String,
}

impl App {
    pub fn hover_text(&self) -> Option<&str> {
        self.nav.hover.as_deref()
    }

    /// The rows the popup shows. `None` while an automatic request has not answered yet.
    pub fn completion_view(&self) -> Option<CompletionView> {
        let menu = self.nav.completion.as_ref()?;
        if menu.loading && menu.items.is_empty() {
            return menu.explicit.then(|| CompletionView {
                rows: vec![CompletionRow {
                    label: "…".into(),
                    detail: String::new(),
                }],
                selected: 0,
                docs: String::new(),
            });
        }
        let filtered = filtered_indexes(menu);
        if filtered.is_empty() {
            return menu.explicit.then(|| CompletionView {
                rows: vec![CompletionRow {
                    label: "No matches".into(),
                    detail: String::new(),
                }],
                selected: 0,
                docs: String::new(),
            });
        }
        let selected = menu.selected.min(filtered.len() - 1);
        let start = selected
            .saturating_sub(VISIBLE / 2)
            .min(filtered.len().saturating_sub(VISIBLE));
        let end = (start + VISIBLE).min(filtered.len());
        let rows = filtered[start..end]
            .iter()
            .map(|&index| {
                let item = &menu.items[index];
                let label = if item.kind.is_empty() {
                    item.label.clone()
                } else {
                    format!("{} {}", item.kind, item.label)
                };
                CompletionRow {
                    label,
                    detail: item.detail.clone(),
                }
            })
            .collect();
        let docs = filtered
            .get(selected)
            .map(|&index| menu.items[index].docs.clone())
            .unwrap_or_default();
        Some(CompletionView {
            rows,
            selected: selected - start,
            docs,
        })
    }

    pub fn signature_text(&self) -> Option<&str> {
        self.nav.signature.as_deref()
    }

    pub(super) fn show_hover(&mut self) {
        self.dismiss_lsp_popups();
        if self.server_lacks(|features| features.hover, "hover") {
            return;
        }
        self.nav.hover = None;
        let token = self.enqueue(QueryKind::Hover);
        self.nav.hover_token = Some(token);
    }

    pub(super) fn goto_definition(&mut self) {
        self.dismiss_lsp_popups();
        if self.server_lacks(|features| features.definition, "go to definition") {
            return;
        }
        let token = self.enqueue(QueryKind::Definition);
        self.nav.definition_token = Some(token);
        if self.message.is_none() {
            self.message = Some("Finding definition…".to_string());
        }
    }

    pub(super) fn complete_now(&mut self) {
        self.nav.hover = None;
        if self.server_lacks(|features| features.completion, "completion") {
            return;
        }
        self.begin_completion(CompletionTrigger::Invoked, true);
    }

    pub(super) fn jump_back(&mut self) {
        self.step_jump(true);
    }

    pub(super) fn jump_forward(&mut self) {
        self.step_jump(false);
    }

    pub(super) fn completion_move(&mut self, next: bool) {
        let Some(menu) = self.nav.completion.as_mut() else {
            return;
        };
        let len = filtered_indexes(menu).len();
        if len == 0 {
            return;
        }
        menu.selected = if next {
            (menu.selected + 1) % len
        } else {
            (menu.selected + len - 1) % len
        };
        self.request_resolve();
    }

    pub(super) fn accept_completion(&mut self) {
        let chosen = {
            let Some(menu) = self.nav.completion.as_ref() else {
                return;
            };
            if menu.loading && menu.items.is_empty() {
                return;
            }
            let filtered = filtered_indexes(menu);
            if let Some(&index) = filtered.get(menu.selected.min(filtered.len().saturating_sub(1)))
            {
                let item = &menu.items[index];
                Some((
                    menu.origin,
                    item.replace_at,
                    item.insert.clone(),
                    item.snippet,
                    item.extra.clone(),
                ))
            } else {
                None
            }
        };
        let Some((origin, replace_at, insert, snippet, extra)) = chosen else {
            self.message = Some("No matches".to_string());
            self.close_completion();
            return;
        };
        let parsed = if snippet {
            crate::snippet::parse(&insert)
        } else {
            crate::snippet::ParsedSnippet {
                text: insert,
                stops: Vec::new(),
            }
        };
        let head = self.buffer().selection().head;
        let start = replace_at.unwrap_or(origin.unwrap_or(head)).min(head);
        let mut edits = vec![(start, head - start, parsed.text)];
        for (from, to, text) in extra {
            if from <= to && to <= start {
                edits.push((from, to - from, text));
            }
        }
        edits.sort_by_key(|edit| edit.0);
        let mut kept: Vec<(usize, usize, String)> = Vec::new();
        for edit in edits {
            if kept.last().is_some_and(|prev| edit.0 < prev.0 + prev.1) {
                continue;
            }
            kept.push(edit);
        }
        let shape: Vec<(usize, usize, usize)> = kept
            .iter()
            .map(|(pos, removed, text)| (*pos, *removed, text.chars().count()))
            .collect();
        let inserted_at = map_pos(start, &shape);
        self.edit(|buffer| buffer.edit_batch(kept, head));
        self.close_completion();
        self.arm_snippet(inserted_at, &parsed.stops);
    }

    fn arm_snippet(&mut self, inserted_at: usize, stops: &[(usize, usize)]) {
        let stops: Vec<(usize, usize)> = stops
            .iter()
            .map(|(start, end)| (inserted_at + start, inserted_at + end))
            .filter(|(start, end)| start <= end)
            .collect();
        if stops.is_empty() {
            self.nav.snippet = None;
            return;
        }
        let (start, end) = stops[0];
        self.buffer_mut().select_span(start, end);
        self.scroll_to_cursor();
        self.nav.snippet = Some(SnippetSession { stops, index: 0 });
    }

    pub(super) fn snippet_jump(&mut self, next: bool) -> bool {
        let Some(session) = self.nav.snippet.as_mut() else {
            return false;
        };
        if session.stops.is_empty() {
            self.nav.snippet = None;
            return false;
        }
        if next {
            if session.index + 1 >= session.stops.len() {
                self.nav.snippet = None;
                return true;
            }
            session.index += 1;
        } else if session.index == 0 {
            return true;
        } else {
            session.index -= 1;
        }
        let (start, end) = session.stops[session.index];
        self.buffer_mut().select_span(start, end);
        self.scroll_to_cursor();
        true
    }

    pub(super) fn clear_snippet(&mut self) {
        self.nav.snippet = None;
    }

    pub(super) fn shift_snippet(&mut self, pos: usize, removed: usize, inserted: usize) {
        let Some(session) = self.nav.snippet.as_mut() else {
            return;
        };
        let end = pos + removed;
        let delta = inserted as isize - removed as isize;
        for stop in session.stops.iter_mut() {
            if stop.0 == stop.1 && stop.0 == pos {
                stop.0 = pos + inserted;
                stop.1 = stop.0;
                continue;
            }
            if stop.1 <= pos {
                continue;
            }
            if stop.0 >= end {
                stop.0 = shift_char(stop.0, delta);
                stop.1 = shift_char(stop.1, delta);
                continue;
            }
            stop.0 = pos;
            stop.1 = pos + inserted;
        }
    }

    pub(super) fn close_completion(&mut self) {
        if let Some(token) = self.nav.resolve_token.take() {
            self.lsp_nav_requests.push(LspRequest::Cancel { token });
        }
        if let Some(menu) = self.nav.completion.take() {
            self.lsp_nav_requests
                .push(LspRequest::Cancel { token: menu.token });
        }
    }

    pub(super) fn dismiss_lsp_popups(&mut self) {
        self.nav.hover = None;
        self.nav.hover_token = None;
        self.nav.actions = None;
        self.close_completion();
    }

    pub fn action_view(&self) -> Option<ActionView> {
        let menu = self.nav.actions.as_ref()?;
        if menu.items.is_empty() {
            return None;
        }
        Some(ActionView {
            titles: menu.items.iter().map(|item| item.title.clone()).collect(),
            selected: menu.selected,
        })
    }

    /// After a character was inserted or deleted in the buffer.
    pub(super) fn note_typed(&mut self, inserted: Option<char>) {
        self.nav.hover = None;
        self.nav.hover_token = None;
        self.note_signature(inserted);
        if self.nav.completion.is_some() {
            if !self.completion_prefix_ok() {
                self.close_completion();
                return;
            }
            let loading = self
                .nav
                .completion
                .as_ref()
                .is_some_and(|menu| menu.loading || menu.incomplete);
            let refresh = loading || inserted.is_some_and(|_| self.trigger_suffix().is_some());
            if refresh {
                self.retrigger(inserted);
            } else {
                self.store_prefix();
                let hide = self.nav.completion.as_ref().is_some_and(|menu| {
                    !menu.explicit && !menu.loading && filtered_indexes(menu).is_empty()
                });
                if hide {
                    self.close_completion();
                }
            }
            return;
        }
        let Some(ch) = inserted else {
            return;
        };
        if !self
            .features_of()
            .is_some_and(|features| features.completion)
        {
            return;
        }
        if let Some(trigger) = self.trigger_suffix() {
            self.begin_completion(CompletionTrigger::Character(trigger), false);
        } else if is_word(ch) {
            self.begin_completion(CompletionTrigger::Invoked, false);
        }
    }

    pub(super) fn apply_pending_place(&mut self, path: &Path) {
        let Some((wanted, place)) = self.nav.pending.take() else {
            return;
        };
        if !super::same_path(&wanted, path) {
            self.nav.pending = Some((wanted, place));
            return;
        }
        match place {
            PendingPlace::Char(index) => {
                self.buffer_mut().select_span(index, index);
                self.scroll_to_cursor();
            }
            PendingPlace::Target(target) => self.place_target(&target),
        }
    }

    pub(super) fn handle_features(
        &mut self,
        group: &'static str,
        hover: bool,
        definition: bool,
        completion: bool,
        triggers: Vec<String>,
        resolve: bool,
        signature: bool,
        rename: bool,
        format: bool,
        code_action: bool,
        action_resolve: bool,
    ) {
        self.nav.features.insert(
            group,
            Features {
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
            },
        );
    }

    pub(super) fn handle_signature(&mut self, token: u64, text: Option<String>) {
        if self.nav.signature_token != Some(token) {
            return;
        }
        self.nav.signature_token = None;
        self.nav.signature = text.filter(|text| !text.is_empty());
    }

    pub(super) fn handle_resolved(&mut self, token: u64, detail: String, docs: String) {
        if self.nav.resolve_token != Some(token) {
            return;
        }
        self.nav.resolve_token = None;
        let index = self.nav.resolve_index;
        if let Some(item) = self
            .nav
            .completion
            .as_mut()
            .and_then(|menu| menu.items.get_mut(index))
        {
            if !detail.is_empty() {
                item.detail = detail;
            }
            if !docs.is_empty() {
                item.docs = docs;
            }
        }
    }

    fn note_signature(&mut self, inserted: Option<char>) {
        match inserted {
            Some('(' | ',' | ')') => self.show_signature(),
            Some(_) => {
                self.nav.signature = None;
                self.nav.signature_token = None;
            }
            None => {}
        }
    }

    fn show_signature(&mut self) {
        if !self
            .features_of()
            .is_some_and(|features| features.signature)
        {
            return;
        }
        let token = self.enqueue(QueryKind::Signature);
        self.nav.signature_token = Some(token);
    }

    fn request_resolve(&mut self) {
        if !self.features_of().is_some_and(|features| features.resolve) {
            return;
        }
        let chosen = {
            let Some(menu) = self.nav.completion.as_ref() else {
                return;
            };
            let filtered = filtered_indexes(menu);
            let Some(&index) = filtered.get(menu.selected.min(filtered.len().saturating_sub(1)))
            else {
                return;
            };
            let item = &menu.items[index];
            if !item.docs.is_empty() {
                return;
            }
            item.resolve.clone().map(|raw| (index, raw))
        };
        let Some((index, raw)) = chosen else {
            return;
        };
        if raw.is_empty() {
            return;
        }
        let Some(path) = self.path().map(std::path::Path::to_path_buf) else {
            return;
        };
        self.nav.next_token += 1;
        let token = self.nav.next_token;
        self.nav.resolve_token = Some(token);
        self.nav.resolve_index = index;
        self.lsp_nav_requests.push(LspRequest::Query {
            token,
            path,
            at: 0,
            kind: QueryKind::Resolve { item: raw },
        });
    }

    pub(super) fn handle_hover(&mut self, token: u64, text: Option<String>) {
        if self.nav.hover_token != Some(token) {
            return;
        }
        self.nav.hover_token = None;
        match text {
            Some(text) => self.nav.hover = Some(text),
            None => self.message = Some("No hover information".to_string()),
        }
    }

    pub(super) fn handle_definition(&mut self, token: u64, locations: Vec<JumpTarget>) {
        if self.nav.definition_token != Some(token) {
            return;
        }
        self.nav.definition_token = None;
        let Some(first) = locations.first().cloned() else {
            self.message = Some("No definition".to_string());
            return;
        };
        if locations.len() > 1 {
            self.message = Some(format!(
                "{} definitions, showing the first",
                locations.len()
            ));
        } else {
            self.message = None;
        }
        self.jump_to_target(first);
    }

    pub(super) fn handle_completion(
        &mut self,
        token: u64,
        incomplete: bool,
        items: Vec<lsp::protocol::CompletionItem>,
    ) {
        let (matches, explicit) = {
            let Some(menu) = self.nav.completion.as_ref() else {
                return;
            };
            (menu.token == token, menu.explicit)
        };
        if !matches {
            return;
        }
        let rope = self.buffer().rope().clone();
        let head = self.buffer().selection().head;
        // Filter by the word the user typed. A completion's replace range can start
        // earlier (an import, or a path), and using that as the prefix hides every item.
        let origin = word_start(&rope, head);
        let prefix = self.typed_prefix(Some(origin));
        let mapped = items
            .into_iter()
            .map(|item| Item {
                label: item.label,
                detail: item.detail,
                filter: item.filter,
                insert: item.insert,
                kind: item.kind,
                docs: item.docs,
                snippet: item.snippet,
                resolve: item.resolve,
                replace_at: item
                    .replace
                    .map(|(start, _)| rope.byte_to_char(start.min(rope.len_bytes()))),
                extra: item
                    .extra
                    .into_iter()
                    .filter_map(|(start, end, text)| {
                        let from = rope.byte_to_char(start.min(rope.len_bytes()));
                        let to = rope.byte_to_char(end.min(rope.len_bytes())).max(from);
                        (to <= origin).then_some((from, to, text))
                    })
                    .collect(),
            })
            .collect();
        if let Some(menu) = self.nav.completion.as_mut() {
            menu.items = mapped;
            menu.origin = Some(origin);
            menu.incomplete = incomplete;
            menu.loading = false;
            menu.selected = 0;
            menu.explicit = explicit;
            menu.prefix = prefix;
        }
        let empty = self
            .nav
            .completion
            .as_ref()
            .is_some_and(|menu| filtered_indexes(menu).is_empty());
        if empty && !explicit {
            self.close_completion();
        } else {
            self.request_resolve();
        }
    }

    fn store_prefix(&mut self) {
        let origin = self.nav.completion.as_ref().and_then(|menu| menu.origin);
        let prefix = self.typed_prefix(origin);
        if let Some(menu) = self.nav.completion.as_mut() {
            menu.prefix = prefix;
        }
    }

    fn typed_prefix(&self, origin: Option<usize>) -> String {
        let Some(origin) = origin else {
            return String::new();
        };
        let head = self.buffer().selection().head;
        if head < origin {
            return String::new();
        }
        self.buffer().rope().slice(origin..head).to_string()
    }

    fn enqueue(&mut self, kind: QueryKind) -> u64 {
        let token = {
            self.nav.next_token = self.nav.next_token.wrapping_add(1);
            self.nav.next_token
        };
        let Some(path) = self.path().map(Path::to_path_buf) else {
            self.message = Some("Save the file before using the language server".to_string());
            return token;
        };
        if Language::detect(&path).is_none() {
            self.message = Some("This file has no language server".to_string());
            return token;
        }
        let at = {
            let buffer = self.buffer();
            buffer.rope().char_to_byte(buffer.selection().head)
        };
        self.lsp_nav_requests.push(LspRequest::Query {
            token,
            path,
            at,
            kind,
        });
        token
    }

    fn begin_completion(&mut self, trigger: CompletionTrigger, explicit: bool) {
        let token = self.enqueue(QueryKind::Completion {
            trigger,
            immediate: explicit,
        });
        self.nav.completion = Some(Menu {
            token,
            origin: None,
            incomplete: false,
            items: Vec::new(),
            selected: 0,
            loading: true,
            explicit,
            prefix: String::new(),
        });
    }

    fn retrigger(&mut self, inserted: Option<char>) {
        let explicit = self
            .nav
            .completion
            .as_ref()
            .is_some_and(|menu| menu.explicit);
        let trigger = match inserted.and_then(|_| self.trigger_suffix()) {
            Some(trigger) => CompletionTrigger::Character(trigger),
            None => CompletionTrigger::Invoked,
        };
        self.begin_completion(trigger, explicit);
    }

    fn server_lacks(&mut self, has: impl Fn(&Features) -> bool, feature: &str) -> bool {
        match self.features_of() {
            Some(features) if !has(&features) => {
                self.message = Some(format!("The language server does not support {feature}"));
                true
            }
            _ => false,
        }
    }

    fn features_of(&self) -> Option<Features> {
        let path = self.path()?;
        let language = Language::detect(path)?;
        let group = lsp::registry::servers_for(language).first()?.name;
        self.nav.features.get(group).cloned()
    }

    fn trigger_suffix(&self) -> Option<String> {
        let triggers = self.features_of()?.triggers;
        if triggers.is_empty() {
            return None;
        }
        let buffer = self.buffer();
        let head = buffer.selection().head;
        let line = buffer.rope().char_to_line(head);
        let start = buffer.rope().line_to_char(line);
        let prefix: String = buffer.rope().slice(start..head).chars().collect();
        triggers
            .into_iter()
            .filter(|trigger| !trigger.is_empty() && prefix.ends_with(trigger.as_str()))
            .max_by_key(String::len)
    }

    fn completion_prefix_ok(&self) -> bool {
        let Some(origin) = self.nav.completion.as_ref().and_then(|menu| menu.origin) else {
            return self.nav.completion.is_some();
        };
        let head = self.buffer().selection().head;
        if head < origin || self.buffer().selection().anchor != head {
            return false;
        }
        !self
            .buffer()
            .rope()
            .slice(origin..head)
            .chars()
            .any(|ch| ch == '\n' || ch == '\r')
    }

    pub(super) fn drop_pending_place(&mut self, path: &Path) {
        if self
            .nav
            .pending
            .as_ref()
            .is_some_and(|(wanted, _)| super::same_path(wanted, path))
        {
            self.nav.pending = None;
        }
    }

    fn step_jump(&mut self, back: bool) {
        self.dismiss_lsp_popups();
        let target = if back {
            self.nav.back.pop()
        } else {
            self.nav.forward.pop()
        };
        let Some(target) = target else {
            self.message = Some(if back {
                "No earlier location".to_string()
            } else {
                "No later location".to_string()
            });
            return;
        };
        if let Some(point) = self.current_jump() {
            let stack = if back {
                &mut self.nav.forward
            } else {
                &mut self.nav.back
            };
            push_point(stack, point);
        }
        self.go_to_point(target);
    }

    fn jump_to_target(&mut self, target: JumpTarget) {
        if let Some(point) = self.current_jump() {
            push_point(&mut self.nav.back, point);
        }
        self.nav.forward.clear();
        if self
            .path()
            .is_some_and(|path| super::same_path(path, &target.path))
        {
            self.place_target(&target);
            return;
        }
        if let Some(tab) = self.tab_with_path(&target.path) {
            self.focus_tab(tab);
            self.place_target(&target);
            return;
        }
        let path = target.path.clone();
        self.nav.pending = Some((path.clone(), PendingPlace::Target(target)));
        self.open_path(path);
    }

    fn go_to_point(&mut self, point: JumpPoint) {
        if self
            .path()
            .is_some_and(|path| super::same_path(path, &point.path))
        {
            self.buffer_mut()
                .select_span(point.char_idx, point.char_idx);
            self.scroll_to_cursor();
            return;
        }
        if let Some(tab) = self.tab_with_path(&point.path) {
            self.focus_tab(tab);
            self.buffer_mut()
                .select_span(point.char_idx, point.char_idx);
            self.scroll_to_cursor();
            return;
        }
        let path = point.path.clone();
        self.nav.pending = Some((path.clone(), PendingPlace::Char(point.char_idx)));
        self.open_path(path);
    }

    fn current_jump(&self) -> Option<JumpPoint> {
        let path = self.path()?.to_path_buf();
        Some(JumpPoint {
            path,
            char_idx: self.buffer().selection().head,
        })
    }

    fn place_target(&mut self, target: &JumpTarget) {
        let rope = self.buffer().rope().clone();
        let byte = position::byte_of(&rope, target.line, target.character, target.encoding);
        let index = rope.byte_to_char(byte.min(rope.len_bytes()));
        self.buffer_mut().select_span(index, index);
        self.scroll_to_cursor();
    }

    pub(super) fn rename_symbol(&mut self, new_name: &str) {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            self.message = Some("Type the new name after rename-symbol".to_string());
            return;
        }
        self.dismiss_lsp_popups();
        if !self.appearance.lsp.enabled {
            self.message = Some("The language server is off".to_string());
            return;
        }
        if self.server_lacks(|features| features.rename, "rename") {
            return;
        }
        let token = self.enqueue(QueryKind::Rename {
            new_name: new_name.to_string(),
        });
        self.nav.edit_token = Some(token);
        self.nav.format_save = None;
        self.message = Some(format!("Renaming to {new_name}…"));
    }

    pub(super) fn format_document(&mut self) {
        self.dismiss_lsp_popups();
        if !self.appearance.lsp.enabled {
            self.message = Some("The language server is off".to_string());
            return;
        }
        if self.server_lacks(|features| features.format, "format") {
            return;
        }
        let token = self.enqueue(QueryKind::Format);
        self.nav.edit_token = Some(token);
        self.nav.format_save = None;
        self.message = Some("Formatting…".to_string());
    }

    /// Format, then save. Returns false when the save should happen immediately.
    pub(super) fn format_before_save(
        &mut self,
        editor: crate::layout::EditorId,
        tab: crate::layout::TabId,
        path: PathBuf,
        after: crate::jobs::AfterSave,
    ) -> bool {
        if !self.appearance.lsp.enabled || !self.appearance.lsp.format_on_save {
            return false;
        }
        if !self.features_of().is_some_and(|features| features.format) {
            return false;
        }
        let token = self.enqueue(QueryKind::Format);
        self.nav.edit_token = Some(token);
        self.nav.format_save = Some(FormatSave {
            editor,
            tab,
            path,
            after,
        });
        self.message = Some("Formatting…".to_string());
        true
    }

    pub(super) fn code_action(&mut self) {
        self.dismiss_lsp_popups();
        if !self.appearance.lsp.enabled {
            self.message = Some("The language server is off".to_string());
            return;
        }
        if self.server_lacks(|features| features.code_action, "code action") {
            return;
        }
        let token = self.enqueue(QueryKind::CodeAction);
        self.nav.edit_token = Some(token);
        self.message = Some("Looking for code actions…".to_string());
    }

    pub(super) fn show_actions(
        &mut self,
        token: u64,
        encoding: crate::lsp::position::Encoding,
        actions: Vec<crate::lsp::protocol::ActionOffer>,
    ) {
        if self.nav.edit_token != Some(token) {
            return;
        }
        self.nav.edit_token = None;
        if actions.is_empty() {
            self.nav.actions = None;
            self.message = Some("No code actions".to_string());
            return;
        }
        self.message = None;
        self.nav.actions = Some(ActionList {
            selected: 0,
            encoding,
            items: actions,
        });
    }

    pub(super) fn move_action(&mut self, next: bool) {
        let Some(menu) = self.nav.actions.as_mut() else {
            return;
        };
        let len = menu.items.len();
        if len == 0 {
            return;
        }
        menu.selected = if next {
            (menu.selected + 1) % len
        } else {
            (menu.selected + len - 1) % len
        };
    }

    pub(super) fn accept_action(&mut self) {
        let Some(menu) = self.nav.actions.take() else {
            return;
        };
        let Some(action) = menu.items.get(menu.selected) else {
            return;
        };
        let encoding = menu.encoding;
        let title = action.title.clone();
        let edit = action.edit.clone();
        let raw = action.raw.clone();
        drop(menu);
        if let Some(edit) = edit {
            let parsed = serde_json::from_str(&edit).unwrap_or(serde_json::Value::Null);
            let edits = crate::lsp::plain::workspace_edits(&parsed);
            self.message = Some(title);
            self.apply_described_edits(encoding, edits);
            return;
        }
        if self
            .features_of()
            .is_some_and(|features| features.action_resolve)
        {
            let token = self.enqueue(QueryKind::CodeActionResolve { action: raw });
            self.nav.edit_token = Some(token);
            self.message = Some(title);
            return;
        }
        self.message = Some(format!("{title} has no edit"));
    }
}

fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn map_pos(pos: usize, edits: &[(usize, usize, usize)]) -> usize {
    let mut delta = 0isize;
    for &(at, removed, inserted) in edits {
        if pos < at {
            break;
        }
        if pos < at + removed {
            return shift_char(at, delta);
        }
        delta += inserted as isize - removed as isize;
    }
    shift_char(pos, delta)
}

fn shift_char(pos: usize, delta: isize) -> usize {
    (pos as isize + delta).max(0) as usize
}

fn word_start(rope: &Rope, head: usize) -> usize {
    let mut index = head;
    while index > 0 && is_word(rope.char(index - 1)) {
        index -= 1;
    }
    index
}

fn filtered_indexes(menu: &Menu) -> Vec<usize> {
    menu.items
        .iter()
        .enumerate()
        .filter(|(_, item)| matches_prefix(&item.filter, &menu.prefix))
        .map(|(index, _)| index)
        .collect()
}

fn push_point(stack: &mut Vec<JumpPoint>, point: JumpPoint) {
    if stack
        .last()
        .is_some_and(|last| last.path == point.path && last.char_idx == point.char_idx)
    {
        return;
    }
    stack.push(point);
    if stack.len() > JUMPS {
        stack.remove(0);
    }
}

fn matches_prefix(filter: &str, prefix: &str) -> bool {
    if prefix.is_empty() || filter.starts_with(prefix) {
        return true;
    }
    prefix.chars().all(|ch| !ch.is_uppercase())
        && filter.to_lowercase().starts_with(&prefix.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::SemanticAction;
    use crate::buffer::Buffer;
    use crate::lsp::position::Encoding;
    use crate::lsp::protocol::{CompletionItem, JumpTarget};
    use crate::lsp::{LspEvent, LspRequest, ServerStatus};

    fn app() -> App {
        App::new(
            Buffer::from_text("fn main() {}\n"),
            Some(PathBuf::from("src/main.rs")),
        )
    }

    fn ready(app: &mut App) {
        app.handle_lsp(LspEvent::Features {
            group: "rust-analyzer",
            hover: true,
            definition: true,
            completion: true,
            triggers: vec![".".into(), "::".into()],
            resolve: false,
            signature: false,
            rename: true,
            format: true,
            code_action: true,
            action_resolve: false,
        });
        app.handle_lsp(LspEvent::Status {
            group: "rust-analyzer",
            name: "rust-analyzer",
            status: ServerStatus::Ready,
        });
    }

    fn queries(app: &mut App) -> Vec<LspRequest> {
        app.take_lsp_requests()
            .into_iter()
            .filter(|request| {
                matches!(
                    request,
                    LspRequest::Query { .. } | LspRequest::Cancel { .. }
                )
            })
            .collect()
    }

    #[test]
    fn hover_stays_until_the_cursor_moves_and_ignores_a_stale_reply() {
        let mut app = app();
        ready(&mut app);
        app.handle(SemanticAction::Hover);
        let requests = queries(&mut app);
        let LspRequest::Query {
            token,
            kind: QueryKind::Hover,
            ..
        } = &requests[0]
        else {
            panic!("{requests:?}");
        };
        app.handle_lsp(LspEvent::Hover {
            token: *token,
            text: Some("fn main".into()),
        });
        assert_eq!(app.hover_text(), Some("fn main"));
        app.handle_lsp(LspEvent::Hover {
            token: token + 1,
            text: Some("stale".into()),
        });
        assert_eq!(app.hover_text(), Some("fn main"));
        app.handle(SemanticAction::Move {
            movement: crate::buffer::Movement::Right,
            extend: false,
        });
        assert_eq!(app.hover_text(), None);
    }

    #[test]
    fn definition_jumps_and_jump_back_returns() {
        let mut app = App::new(
            Buffer::from_text("fn main() {\n    work();\n}\nfn work() {}\n"),
            Some(PathBuf::from("src/main.rs")),
        );
        ready(&mut app);
        for _ in 0..16 {
            app.handle(SemanticAction::Move {
                movement: crate::buffer::Movement::Right,
                extend: false,
            });
        }
        let before = app.buffer().selection().head;
        app.handle(SemanticAction::GotoDefinition);
        let requests = queries(&mut app);
        let LspRequest::Query { token, .. } = &requests[0] else {
            panic!("{requests:?}");
        };
        app.handle_lsp(LspEvent::Definition {
            token: *token,
            locations: vec![JumpTarget {
                path: PathBuf::from("src/main.rs"),
                line: 3,
                character: 3,
                end_line: 3,
                end_character: 7,
                encoding: Encoding::Utf8,
            }],
        });
        assert!(app.buffer().selection().head > before);
        app.handle(SemanticAction::JumpBack);
        assert_eq!(app.buffer().selection().head, before);
        app.handle(SemanticAction::JumpForward);
        assert!(app.buffer().selection().head > before);
    }

    #[test]
    fn typing_a_word_asks_for_completion_and_accept_is_one_undo() {
        let mut app = app();
        ready(&mut app);
        app.handle(SemanticAction::Move {
            movement: crate::buffer::Movement::DocumentEnd,
            extend: false,
        });
        app.handle(SemanticAction::InsertChar('p'));
        let requests = queries(&mut app);
        let LspRequest::Query { token, kind, .. } = &requests[0] else {
            panic!("{requests:?}");
        };
        assert!(matches!(
            kind,
            QueryKind::Completion {
                immediate: false,
                ..
            }
        ));
        app.handle_lsp(LspEvent::Completion {
            token: *token,
            incomplete: false,
            items: vec![
                CompletionItem {
                    label: "println".into(),
                    detail: "fn".into(),
                    filter: "println".into(),
                    insert: "println!".into(),
                    kind: String::new(),
                    docs: String::new(),
                    snippet: false,
                    resolve: None,
                    replace: None,
                    extra: Vec::new(),
                },
                CompletionItem {
                    label: "print".into(),
                    detail: String::new(),
                    filter: "print".into(),
                    insert: "print!".into(),
                    kind: String::new(),
                    docs: String::new(),
                    snippet: false,
                    resolve: None,
                    replace: None,
                    extra: Vec::new(),
                },
            ],
        });
        let view = app.completion_view().expect("menu");
        assert_eq!(view.rows[0].label, "println");
        assert_eq!(view.rows[1].label, "print");
        app.handle(SemanticAction::Completion(
            crate::action::CompletionAction::Next,
        ));
        app.handle(SemanticAction::Completion(
            crate::action::CompletionAction::Accept,
        ));
        assert!(app.buffer().rope().to_string().contains("print!"));
        assert!(app.completion_view().is_none());
        app.handle(SemanticAction::Undo);
        assert!(!app.buffer().rope().to_string().contains("print!"));
    }

    #[test]
    fn a_replace_range_before_the_word_does_not_hide_matches() {
        let mut app = app();
        ready(&mut app);
        app.handle(SemanticAction::Move {
            movement: crate::buffer::Movement::DocumentEnd,
            extend: false,
        });
        app.handle(SemanticAction::InsertChar('p'));
        let requests = queries(&mut app);
        let LspRequest::Query { token, .. } = &requests[0] else {
            panic!("{requests:?}");
        };
        app.handle_lsp(LspEvent::Completion {
            token: *token,
            incomplete: false,
            items: vec![CompletionItem {
                label: "println".into(),
                detail: String::new(),
                filter: "println".into(),
                insert: "println!".into(),
                kind: String::new(),
                docs: String::new(),
                snippet: false,
                resolve: None,
                replace: Some((0, 1)),
                extra: Vec::new(),
            }],
        });
        let view = app
            .completion_view()
            .expect("the typed letter still matches");
        assert_eq!(view.rows[0].label, "println");
    }

    #[test]
    fn a_trigger_character_is_sent_as_a_trigger() {
        let mut app = app();
        ready(&mut app);
        app.handle(SemanticAction::InsertChar('.'));
        let requests = queries(&mut app);
        let LspRequest::Query { kind, .. } = &requests[0] else {
            panic!("{requests:?}");
        };
        assert_eq!(
            kind,
            &QueryKind::Completion {
                trigger: CompletionTrigger::Character(".".into()),
                immediate: false,
            }
        );
    }

    #[test]
    fn an_unsupported_feature_is_refused_before_a_request() {
        let mut app = app();
        app.handle_lsp(LspEvent::Features {
            group: "rust-analyzer",
            hover: false,
            definition: false,
            completion: false,
            triggers: Vec::new(),
            resolve: false,
            signature: false,
            rename: false,
            format: false,
            code_action: false,
            action_resolve: false,
        });
        app.handle(SemanticAction::Hover);
        assert!(queries(&mut app).is_empty());
        assert_eq!(
            app.message.as_deref(),
            Some("The language server does not support hover")
        );
    }
}
