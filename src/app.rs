use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::action::{
    CommandBarAction, CompletionAction, FindAction, Focus, PaneDirection, PickerAction,
    PromptAction, ReplaceAction, SemanticAction, SettingsAction, SidebarAction,
};
use ropey::Rope;
use crate::buffer::{Buffer, Movement};
use crate::command::{
    CommandBar, CommandContext, CommandId, CommandSpec, NameSource, find_command, ghost_suffix,
    is_known_name, is_line_number, name_prefix, name_source, spec, suggestions, unavailable_reason,
};
use crate::explorer::Explorer;
use crate::jobs::{AfterSave, AppEvent, CloseBatch, Job, LoadKind};
use crate::layout::{self, Axis, EditorId, Layout, Surface, TabId};
use crate::picker::{self, FilePicker, IndexedFile};
use crate::services::clipboard::Clipboard;
use crate::services::encoding::{Decoded, FileStamp, OpenError, TextEncoding};
use crate::services::fs::Change;
use crate::services::git::DiffResult;
use crate::syntax::{self, SyntaxState};
use crate::theme::SyntaxRole;

mod agent;
mod lsp_nav;
mod lsp_sync;

pub use lsp_nav::{ActionView, ReferenceView};

pub use lsp_nav::{CompletionRow, CompletionView};
pub use lsp_sync::LspLabel;

/// Keeps the start of `text` and ends with `...` once it is wider than `max_cols`.
/// Line breaks are shown as spaces so the status line stays on one row.
fn replaced_message(matches: usize, files: usize) -> String {
    let match_word = if matches == 1 { "match" } else { "matches" };
    let file_word = if files == 1 { "file" } else { "files" };
    format!("Replaced {matches} {match_word} in {files} {file_word}")
}

fn truncate_end(text: &str, max_cols: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    if flat.width() <= max_cols {
        return flat;
    }
    let budget = max_cols.saturating_sub(3);
    let mut out = String::new();
    for grapheme in flat.graphemes(true) {
        if out.width() + grapheme.width() > budget {
            break;
        }
        out.push_str(grapheme);
    }
    out.push_str("...");
    out
}

fn completed_input(spec: &CommandSpec) -> String {
    if spec.arg.is_some() {
        format!("{} ", spec.name)
    } else {
        spec.name.to_string()
    }
}

fn wrap_index(index: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    (index as isize + delta).rem_euclid(len as isize) as usize
}

/// Lines kept visible above/below the cursor while scrolling.
pub const SCROLLOFF: usize = 3;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct View {
    pub scroll_y: usize,
    pub scroll_x: usize,
    pub width: usize,
    pub height: usize,
}

impl View {
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width as usize;
        self.height = height as usize;
    }

    /// Adjusts scroll offsets so `(line, visual col)` stays on screen.
    pub fn scroll_to(&mut self, line: usize, col: usize) {
        if self.height > 0 {
            let off = SCROLLOFF.min(self.height.saturating_sub(1) / 2);
            if line < self.scroll_y + off {
                self.scroll_y = line.saturating_sub(off);
            } else if line + off >= self.scroll_y + self.height {
                self.scroll_y = line + off + 1 - self.height;
            }
        }
        if self.width > 0 {
            if col < self.scroll_x {
                self.scroll_x = col;
            } else if col >= self.scroll_x + self.width {
                self.scroll_x = col + 1 - self.width;
            }
        }
    }
}

/// Cached visible syntax spans for one paint range.
#[derive(Debug, Clone)]
struct HighlightCache {
    generation: u64,
    version: u64,
    start_line: usize,
    end_line: usize,
    spans: Vec<(usize, usize, SyntaxRole)>,
}

/// One open document. Several tabs share a pane, and each keeps its own cursor.
#[derive(Debug)]
pub struct Tab {
    pub id: TabId,
    pub buffer: Buffer,
    pub path: Option<PathBuf>,
    pub view: View,
    /// Bytes differ from the last load or successful save.
    pub dirty: bool,
    saved_fingerprint: u64,
    /// Bumped when a load replaces the buffer, so an in-flight save of the old text is ignored.
    epoch: u64,
    load_generation: u64,
    load_baseline: u64,
    /// Bytes this tab will write on the next save.
    encoding: TextEncoding,
    /// Stamp from the last successful load or save. `None` until the file has been seen.
    disk: Option<FileStamp>,
    /// Disk stamp the user chose to keep editing over.
    kept: Option<FileStamp>,
    /// A failed decode. Saving is refused until a reload succeeds.
    readable: bool,
    loading: bool,
    saving: bool,
    stat_pending: bool,
    noted_missing: bool,
    /// Last `find` text in this tab. `find-next` and `replace` reuse it.
    find_query: Option<String>,
    /// Last replacement typed in REPLACE mode.
    replace_text: String,
    /// Highlight tree for `path`. `None` for plain text and files over the size cap.
    syntax: Option<SyntaxState>,
    /// Bumped when the buffer is replaced or the language changes, so an older parse is ignored.
    syntax_generation: u64,
    /// Last `highlights` result for the visible range. Cleared when the tree or range changes.
    highlight_cache: RefCell<Option<HighlightCache>>,
    /// Edits not yet sent to the language server.
    lsp_edits: Vec<crate::buffer::ByteEdit>,
    /// Last comparison with `HEAD`. Kept while a newer one is in flight so the gutter does not blink.
    diff: Option<CachedDiff>,
}

impl Tab {
    /// Syntax roles for `start_line..end_line`. Empty until a parse has landed.
    /// Tree-sitter spans are cached for the same generation/version/range; semantic tokens merge each call.
    pub fn highlights(
        &self,
        start_line: usize,
        end_line: usize,
        semantic: &[(usize, usize, SyntaxRole)],
    ) -> Vec<(usize, usize, SyntaxRole)> {
        let generation = self.syntax.as_ref().map(|s| s.generation()).unwrap_or(0);
        let version = self.syntax.as_ref().map(|s| s.version()).unwrap_or(0);
        let tree_spans = {
            let cache = self.highlight_cache.borrow();
            let reuse = cache.as_ref().is_some_and(|cache| {
                cache.generation == generation
                    && cache.version == version
                    && cache.start_line == start_line
                    && cache.end_line == end_line
            });
            if reuse {
                cache.as_ref().unwrap().spans.clone()
            } else {
                drop(cache);
                let spans = self
                    .syntax
                    .as_ref()
                    .map(|state| {
                        syntax::highlight_range(state, self.buffer.rope(), start_line, end_line)
                    })
                    .unwrap_or_default();
                *self.highlight_cache.borrow_mut() = Some(HighlightCache {
                    generation,
                    version,
                    start_line,
                    end_line,
                    spans: spans.clone(),
                });
                spans
            }
        };
        let mut spans = tree_spans;
        let rope = self.buffer.rope();
        for &(start, end, role) in semantic {
            let Ok(start) = rope.try_byte_to_char(start.min(rope.len_bytes())) else {
                continue;
            };
            let Ok(end) = rope.try_byte_to_char(end.min(rope.len_bytes())) else {
                continue;
            };
            if start < end {
                spans.push((start, end, role));
            }
        }
        spans.sort_by_key(|span| span.0);
        spans
    }

    /// The diff to draw, when one has landed for this path.
    pub fn file_diff(&self) -> Option<&crate::services::git::FileDiff> {
        let cached = self.diff.as_ref()?;
        if self.path.as_ref() != Some(&cached.path) {
            return None;
        }
        cached.result.as_diff()
    }
}

#[derive(Debug)]
struct CachedDiff {
    path: PathBuf,
    fingerprint: u64,
    result: DiffResult,
}

/// One editor pane. The layout addresses the pane; tabs live inside it.
#[derive(Debug)]
pub struct Editor {
    pub id: EditorId,
    tabs: Vec<Tab>,
    active: usize,
}

impl Editor {
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    fn tab_mut(&mut self) -> &mut Tab {
        let active = self.active;
        &mut self.tabs[active]
    }
}

fn fresh_tab(id: TabId, buffer: Buffer, path: Option<PathBuf>) -> Tab {
    let saved_fingerprint = buffer.fingerprint();
    Tab {
        id,
        buffer,
        path,
        view: View::default(),
        dirty: false,
        saved_fingerprint,
        epoch: 0,
        load_generation: 0,
        load_baseline: 0,
        encoding: TextEncoding::Utf8,
        disk: None,
        kept: None,
        readable: true,
        loading: false,
        saving: false,
        stat_pending: false,
        noted_missing: false,
        find_query: None,
        replace_text: String::new(),
        syntax: None,
        syntax_generation: 0,
        highlight_cache: RefCell::new(None),
        lsp_edits: Vec::new(),
        diff: None,
    }
}

fn fresh_editor(id: EditorId, tab: Tab) -> Editor {
    Editor {
        id,
        tabs: vec![tab],
        active: 0,
    }
}

/// One button on a confirmation dialog. The label is drawn as-is. `mnemonic` is
/// the underlined letter and the secondary shortcut; arrow keys do the choosing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogButton {
    pub label: &'static str,
    pub mnemonic: Option<char>,
    effect: ButtonEffect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonEffect {
    Affirm,
    Deny,
    Dismiss,
}

/// A confirmation dialog. Callers supply the title, message, and buttons.
/// The widget does not know whether it is asking about deleting a file or unsaved edits.
pub struct Prompt {
    pub title: String,
    pub message: String,
    pub buttons: Vec<DialogButton>,
    pub selected: usize,
    action: Pending,
}

fn reload_buttons() -> Vec<DialogButton> {
    vec![
        DialogButton {
            label: "Reload",
            mnemonic: Some('r'),
            effect: ButtonEffect::Affirm,
        },
        DialogButton {
            label: "Keep",
            mnemonic: Some('k'),
            effect: ButtonEffect::Deny,
        },
    ]
}

fn confirm_buttons() -> Vec<DialogButton> {
    vec![
        DialogButton {
            label: "Yes",
            mnemonic: Some('y'),
            effect: ButtonEffect::Affirm,
        },
        DialogButton {
            label: "No",
            mnemonic: Some('n'),
            effect: ButtonEffect::Deny,
        },
        DialogButton {
            label: "Cancel",
            mnemonic: None,
            effect: ButtonEffect::Dismiss,
        },
    ]
}

/// Where FIND is aimed. The explorer searches files; the editor searches the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FindScope {
    Buffer,
    Files,
}

/// The query typed while the status badge reads `FIND`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindMode {
    pub query: String,
    scope: FindScope,
}

impl FindMode {
    fn searches_files(&self) -> bool {
        self.scope == FindScope::Files
    }
}

/// A file search whose matches are listed in the explorer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFind {
    query: String,
    generation: u64,
}

/// Where REPLACE writes. Files replaces the FILTER search across files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplaceScope {
    Buffer,
    Files,
}

/// The selected text and the replacement typed while the badge reads `REPLACE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceMode {
    /// Text captured from the selection, or from the last search.
    needle: String,
    /// Replacement typed in this mode. Empty deletes the match.
    replacement: String,
    /// Replacements made here that `Up` can still undo.
    undos: u32,
    scope: ReplaceScope,
}

impl ReplaceMode {
    fn searches_files(&self) -> bool {
        self.scope == ReplaceScope::Files
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchSeek {
    First,
    Next,
    Prev,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pending {
    Quit,
    Close,
    /// `close-all` or `close-view` still has more tabs to close.
    CloseBatch(CloseBatch),
    Open(PathBuf),
    /// Open another folder after every unsaved tab has been saved or skipped.
    Workspace(PathBuf),
    Delete(Vec<PathBuf>),
    /// Read the tab from disk again. `stamp` is the outside change the user can choose to keep.
    Reload {
        encoding: Option<TextEncoding>,
        stamp: Option<FileStamp>,
    },
}

pub struct App {
    editors: Vec<Editor>,
    active: EditorId,
    next_id: u64,
    next_tab: u64,
    layout: Layout,
    pub command_bar: Option<CommandBar>,
    /// Last command run while an editor was focused.
    pub recent_command: Option<CommandId>,
    /// Last command run while the explorer was focused.
    recent_explorer: Option<CommandId>,
    /// One-shot status message, cleared by the next action.
    pub message: Option<String>,
    pub clipboard: Clipboard,
    pub branch: Option<String>,
    pub explorer: Explorer,
    /// Workspace-relative file to select once the tree scan that `reveal` started finishes.
    reveal_file: Option<PathBuf>,
    /// Preferred explorer column width. Drawing clamps it to half the terminal.
    sidebar_width: u16,
    /// File picker modal. `None` while it is closed.
    pub file_picker: Option<FilePicker>,
    /// Last workspace walk, reused so the next picker opens before the refresh finishes.
    file_index: Option<CachedFiles>,
    index_generation: u64,
    /// Workspace-relative paths, most recently opened first.
    recent_files: Vec<String>,
    /// Last terminal size, so `Ctrl` + arrow uses the same rectangles as the screen.
    screen_width: u16,
    screen_height: u16,
    /// Background work queued by actions; drained and run by the main loop.
    jobs: Vec<Job>,
    pub should_quit: bool,
    pub prompt: Option<Prompt>,
    /// Settings page. `None` while it is closed.
    pub settings: Option<crate::settings::Page>,
    /// About page. Closed when false.
    pub about: bool,
    /// Color theme, syntax theme, and the styles widgets read.
    pub appearance: crate::theme::Appearance,
    pub editor_prefs: crate::settings::EditorPrefs,
    pub file_prefs: crate::settings::FilePrefs,
    pub ai: crate::settings::AiPrefs,
    /// Sends prompts and tool results to the assistant worker. `None` in tests.
    agent_tx: Option<tokio::sync::mpsc::UnboundedSender<crate::agent::AgentCommand>>,
    pub agent: crate::agent::Panel,
    key_overrides: std::collections::BTreeMap<String, String>,
    keys: crate::keys::KeyMap,
    /// A `settings.json` write is already queued.
    settings_save_inflight: bool,
    /// The themes changed again while that write was queued.
    settings_save_pending: bool,
    /// A save that still needs a path before `Pending` can continue.
    resume: Option<Pending>,
    /// Incremental search. The badge reads `FIND` while this is set.
    pub find_mode: Option<FindMode>,
    /// File search started from the explorer. The tree lists its matches.
    file_find: Option<FileFind>,
    /// Last file-search text. Opening FIND in the explorer puts it back.
    file_find_query: String,
    file_find_generation: u64,
    /// Last replacement typed for a FILTER replace.
    file_replace_text: String,
    file_replace_generation: u64,
    /// Shown after the file list refreshes, instead of the match count.
    replace_notice: Option<String>,
    /// Matches already applied to open buffers, added to the disk job's total.
    replace_buffers: (usize, usize),
    /// After this file finishes loading, enter buffer FIND for the query.
    reveal_find: Option<(PathBuf, String)>,
    /// Replacement. The badge reads `REPLACE` while this is set.
    pub replace_mode: Option<ReplaceMode>,
    /// Tabs the user chose not to save during quit or a workspace switch.
    quit_skipped: Vec<TabId>,
    /// Focus returns to the sidebar when the unsaved-changes flow is cancelled.
    prompt_restore_sidebar: bool,
    /// Messages for the language server worker; drained by the main loop.
    lsp_requests: Vec<crate::lsp::LspRequest>,
    /// Hover, definition, and completion requests. Appended after document sync.
    lsp_nav_requests: Vec<crate::lsp::LspRequest>,
    nav: lsp_nav::Nav,
    /// What the worker was last told about each file tab.
    lsp_docs: HashMap<TabId, lsp_sync::SyncedDoc>,
    pub diagnostics: crate::lsp::diagnostics::Diagnostics,
    /// Semantic tokens, byte ranges, painted over the tree-sitter colors.
    pub(crate) semantic: crate::lsp::semantic::SemanticTokens,
    /// Latest state per server group, with the program name that reported it.
    lsp_servers: HashMap<&'static str, (&'static str, crate::lsp::ServerStatus)>,
    /// The worker has not yet been told the language-server settings.
    lsp_prefs_dirty: bool,
    /// `diff` is showing gutter marks for every open file.
    show_diff: bool,
    /// `focus` fills the screen with the active buffer and hides the other chrome.
    focus_mode: bool,
    diff_generation: u64,
    /// The job already running for a tab: generation and the fingerprint it was given.
    diff_inflight: HashMap<TabId, (u64, u64)>,
    /// Announce the result of the toggle for this tab and fingerprint.
    diff_announce: Option<(TabId, u64)>,
    /// Folders remembered in `session.json`, newest first.
    sessions: Vec<crate::session::Session>,
    /// Language-server events from the previous folder are ignored until `ResetDone`.
    lsp_resetting: bool,
    /// Tree scan that has been queued or is running. `dirty` asks for one more pass.
    tree_scan: Option<TreeScan>,
}

struct TreeScan {
    root: PathBuf,
    dirty: bool,
}

struct CachedFiles {
    root: PathBuf,
    files: Arc<Vec<IndexedFile>>,
    truncated: bool,
}

impl App {
    /// Starts with an in-memory clipboard; the binary swaps in `Clipboard::system()`.
    pub fn new(buffer: Buffer, path: Option<PathBuf>) -> Self {
        let id = EditorId(0);
        let path = path.map(|path| locate(Path::new("."), path.clone()).unwrap_or(path));
        let mut app = Self {
            editors: vec![fresh_editor(id, fresh_tab(TabId(0), buffer, path))],
            active: id,
            next_id: 1,
            next_tab: 1,
            layout: Layout::Leaf(id),
            command_bar: None,
            recent_command: None,
            recent_explorer: None,
            message: None,
            clipboard: Clipboard::in_memory(),
            branch: None,
            explorer: Explorer::new(PathBuf::from(".")),
            reveal_file: None,
            sidebar_width: layout::SIDEBAR_WIDTH,
            file_picker: None,
            file_index: None,
            index_generation: 0,
            recent_files: Vec::new(),
            screen_width: 120,
            screen_height: 40,
            jobs: Vec::new(),
            should_quit: false,
            prompt: None,
            settings: None,
            about: false,
            appearance: crate::theme::Appearance::builtin(),
            editor_prefs: crate::settings::EditorPrefs::default(),
            file_prefs: crate::settings::FilePrefs::default(),
            ai: crate::settings::AiPrefs::default(),
            agent_tx: None,
            agent: crate::agent::Panel::default(),
            key_overrides: std::collections::BTreeMap::new(),
            keys: crate::keys::KeyMap::defaults(),
            settings_save_inflight: false,
            settings_save_pending: false,
            resume: None,
            quit_skipped: Vec::new(),
            prompt_restore_sidebar: false,
            find_mode: None,
            file_find: None,
            file_find_query: String::new(),
            file_find_generation: 0,
            file_replace_text: String::new(),
            file_replace_generation: 0,
            replace_notice: None,
            replace_buffers: (0, 0),
            reveal_find: None,
            replace_mode: None,
            lsp_requests: Vec::new(),
            lsp_nav_requests: Vec::new(),
            nav: lsp_nav::Nav::default(),
            lsp_docs: HashMap::new(),
            diagnostics: Default::default(),
            semantic: Default::default(),
            lsp_servers: HashMap::new(),
            lsp_prefs_dirty: false,
            show_diff: false,
            focus_mode: false,
            diff_generation: 1,
            diff_inflight: HashMap::new(),
            diff_announce: None,
            sessions: Vec::new(),
            lsp_resetting: false,
            tree_scan: None,
        };
        app.configure_syntax(TabId(0));
        app
    }

    pub fn show_diff(&self) -> bool {
        self.show_diff
    }

    /// The screen is only the active buffer. The title, status bar, sidebar, and
    /// other windows stay hidden until `focus` is run again.
    pub fn focus_mode(&self) -> bool {
        self.focus_mode
    }

    /// The status line is the command, find, or replace input.
    pub fn composer_open(&self) -> bool {
        self.command_bar.is_some() || self.find_mode.is_some() || self.replace_mode.is_some()
    }

    pub fn is_dirty(&self) -> bool {
        self.active_editor().tab().dirty
    }

    pub fn buffer(&self) -> &Buffer {
        &self.active_editor().tab().buffer
    }

    /// Encoding the focused file will be saved as.
    pub fn encoding_label(&self) -> &'static str {
        self.active_editor().tab().encoding.label()
    }

    /// Records the encoding and disk stamp of a file that was read before the loop started.
    pub fn note_opened(&mut self, encoding: TextEncoding, stamp: FileStamp) {
        if let Some(tab) = self.find_tab_mut(self.active_tab_id()) {
            tab.encoding = encoding;
            tab.disk = Some(stamp);
            tab.readable = true;
            tab.noted_missing = false;
        }
    }

    /// The focused tab has a path whose bytes could not be decoded.
    pub fn mark_unreadable(&mut self, message: String) {
        if let Some(tab) = self.find_tab_mut(self.active_tab_id()) {
            tab.readable = false;
            tab.disk = None;
        }
        self.message = Some(message);
    }

    /// Queues a stamp check for every open file that is not already being read, saved, or checked.
    /// Returns how many checks were queued so the UI loop can skip a redraw when nothing changed.
    pub fn watch_files(&mut self) -> usize {
        if self.prompt.is_some() {
            return 0;
        }
        let mut jobs = Vec::new();
        for editor in &mut self.editors {
            for tab in &mut editor.tabs {
                let Some(path) = tab.path.clone() else {
                    continue;
                };
                let Some(stamp) = tab.disk else {
                    continue;
                };
                if tab.loading || tab.saving || tab.stat_pending || !tab.readable {
                    continue;
                }
                tab.stat_pending = true;
                jobs.push(Job::StatFile {
                    tab_id: tab.id,
                    path,
                    stamp,
                });
            }
        }
        let n = jobs.len();
        self.jobs.extend(jobs);
        n
    }

    pub fn buffer_mut(&mut self) -> &mut Buffer {
        &mut self.active_editor_mut().tab_mut().buffer
    }

    pub fn path(&self) -> Option<&std::path::Path> {
        self.active_editor().tab().path.as_deref()
    }

    pub fn view(&self) -> &View {
        &self.active_editor().tab().view
    }

    pub fn view_mut(&mut self) -> &mut View {
        &mut self.active_editor_mut().tab_mut().view
    }

    fn active_tab_id(&self) -> TabId {
        self.active_editor().tab().id
    }

    pub(crate) fn sidebar_width(&self) -> u16 {
        self.sidebar_width
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn active_id(&self) -> EditorId {
        self.active
    }

    pub fn editor(&self, id: EditorId) -> &Editor {
        self.editors
            .iter()
            .find(|editor| editor.id == id)
            .expect("editor id is in the layout")
    }

    /// Sizes every visible pane and scrolls it so its own cursor stays in view.
    pub fn resize_panes(&mut self, panes: &[(EditorId, u16, u16)]) {
        for &(id, width, height) in panes {
            let Some(editor) = self.editors.iter_mut().find(|editor| editor.id == id) else {
                continue;
            };
            let tab = editor.tab_mut();
            tab.view.resize(width, height);
            let (line, col) = tab.buffer.cursor_line_col();
            tab.view.scroll_to(line, col);
        }
    }

    fn active_editor(&self) -> &Editor {
        self.editor(self.active)
    }

    fn active_editor_mut(&mut self) -> &mut Editor {
        let active = self.active;
        self.editors
            .iter_mut()
            .find(|editor| editor.id == active)
            .expect("active editor exists")
    }

    pub fn set_screen_size(&mut self, width: u16, height: u16) {
        self.screen_width = width;
        self.screen_height = height;
    }

    pub fn styles(&self) -> &crate::theme::UiStyles {
        &self.appearance.styles
    }

    /// Reads `settings.json` and user theme files from `dir`. Built-ins stay in place
    /// when the directory is empty.
    pub fn load_appearance(&mut self, dir: &Path) {
        let (appearance, file, warning) = crate::theme::Appearance::load(dir);
        self.appearance = appearance;
        self.editor_prefs = file.editor;
        self.file_prefs = file.files;
        self.ai = file.ai;
        self.key_overrides = file.keybindings;
        self.keys = crate::keys::KeyMap::resolve(&self.key_overrides);
        self.apply_editor_prefs();
        self.lsp_prefs_dirty = true;
        if let Some(warning) = warning {
            self.message = Some(warning);
        }
    }

    /// Reads `session.json` into memory. The command bar lists these folders without touching disk.
    pub fn load_sessions(&mut self, path: &Path) {
        self.sessions = crate::session::read_all(path);
    }

    /// The remembered session for `root`, if this process has loaded one.
    pub fn saved_session(&self, root: &Path) -> Option<crate::session::Session> {
        self.sessions
            .iter()
            .find(|session| crate::session::same_directory(&session.root, root))
            .cloned()
    }

    /// Folder, saved files, cursors, splits, and the sidebar. Untitled text is left out.
    pub fn session_snapshot(&self) -> Option<crate::session::Session> {
        let root = std::path::absolute(&self.explorer.root).ok()?;
        if !root.is_dir() {
            return None;
        }
        Some(crate::session::Session {
            version: crate::session::VERSION,
            root: root.clone(),
            sidebar: crate::session::Sidebar {
                open: self.explorer.open,
                width: self.sidebar_width,
                expanded: self
                    .explorer
                    .expanded_paths()
                    .iter()
                    .map(|path| crate::session::display_rel(path))
                    .collect(),
            },
            sidebar_focused: self.explorer.open && self.explorer.focused,
            layout: self.snapshot_layout(&self.layout, &root),
        })
    }

    fn snapshot_layout(&self, layout: &Layout, root: &Path) -> crate::session::SessionLayout {
        match layout {
            Layout::Leaf(id) => {
                let editor = self.editor(*id);
                let mut tabs = Vec::new();
                let mut active = 0;
                for (index, tab) in editor.tabs().iter().enumerate() {
                    let Some(path) = tab.path.as_deref() else {
                        continue;
                    };
                    let Some(rel) = relative_to(root, path) else {
                        continue;
                    };
                    let text = crate::session::display_rel(&rel);
                    if text.is_empty() {
                        continue;
                    }
                    if index == editor.active_index() {
                        active = tabs.len();
                    }
                    let (line, col) = tab.buffer.cursor_line_char();
                    tabs.push(crate::session::SessionTab {
                        path: text,
                        line,
                        col,
                    });
                }
                crate::session::SessionLayout::Leaf {
                    selected: *id == self.active,
                    active,
                    tabs,
                }
            }
            Layout::Split {
                axis,
                first,
                second,
            } => crate::session::SessionLayout::Split {
                axis: *axis,
                first: Box::new(self.snapshot_layout(first, root)),
                second: Box::new(self.snapshot_layout(second, root)),
            },
        }
    }

    /// Replaces the editors with `session`. Files that are gone are skipped.
    /// Each remaining file is queued to load off the UI thread.
    pub fn restore_session(&mut self, session: crate::session::Session) {
        self.jobs.clear();
        self.install_session(session);
    }

    /// Replaces the editors with `session`. Files that are gone are skipped.
    /// Each remaining file is queued to load off the UI thread.
    /// Jobs already queued, such as the session write, stay in place.
    /// `false` when the session is the wrong version or its folder is gone.
    fn install_session(&mut self, session: crate::session::Session) -> bool {
        if session.version != crate::session::VERSION || !session.root.is_dir() {
            return false;
        }
        let mut explorer = Explorer::new(session.root.clone());
        explorer.open = session.sidebar.open;
        explorer.focused = session.sidebar_focused && session.sidebar.open;
        explorer.set_expanded(
            session
                .sidebar
                .expanded
                .iter()
                .filter_map(|path| crate::session::rel_path(path)),
        );
        self.explorer = explorer;
        if session.sidebar.width >= layout::SIDEBAR_MIN_WIDTH {
            self.sidebar_width = session.sidebar.width;
        }
        self.editors.clear();
        let mut loads = Vec::new();
        let (layout, selected) = self.materialize_session(&session.layout, &mut loads);
        self.layout = layout;
        self.active = selected.unwrap_or_else(|| first_leaf(&self.layout));
        for (id, tab_id, path, line, col) in loads {
            self.start_load(
                id,
                tab_id,
                path,
                LoadKind::Open {
                    encoding: None,
                    place: Some((line, col)),
                    focus: false,
                },
                true,
            );
        }
        if self.explorer.open {
            self.queue_tree_scan(true);
        }
        true
    }

    fn materialize_session(
        &mut self,
        node: &crate::session::SessionLayout,
        loads: &mut Vec<(EditorId, TabId, PathBuf, usize, usize)>,
    ) -> (Layout, Option<EditorId>) {
        match node {
            crate::session::SessionLayout::Leaf {
                selected,
                active,
                tabs,
            } => {
                let id = EditorId(self.next_id);
                self.next_id += 1;
                let mut built = Vec::new();
                let mut active_index = 0;
                let mut saw_active = false;
                for (index, tab) in tabs.iter().enumerate() {
                    let Some(path) = crate::session::join_rel(&self.explorer.root, &tab.path)
                    else {
                        continue;
                    };
                    if !path.is_file() {
                        continue;
                    }
                    if index == *active {
                        active_index = built.len();
                        saw_active = true;
                    }
                    let fresh = self.alloc_tab(self.style_buffer(Buffer::new()), None);
                    loads.push((id, fresh.id, path, tab.line, tab.col));
                    built.push(fresh);
                }
                if built.is_empty() {
                    built.push(self.alloc_tab(self.style_buffer(Buffer::new()), None));
                    active_index = 0;
                } else if !saw_active {
                    active_index = 0;
                }
                self.editors.push(Editor {
                    id,
                    tabs: built,
                    active: active_index,
                });
                (Layout::Leaf(id), selected.then_some(id))
            }
            crate::session::SessionLayout::Split {
                axis,
                first,
                second,
            } => {
                let (first, focus_first) = self.materialize_session(first, loads);
                let (second, focus_second) = self.materialize_session(second, loads);
                (
                    Layout::Split {
                        axis: *axis,
                        first: Box::new(first),
                        second: Box::new(second),
                    },
                    focus_first.or(focus_second),
                )
            }
        }
    }

    pub fn keys(&self) -> &crate::keys::KeyMap {
        &self.keys
    }

    pub fn capturing(&self) -> bool {
        self.settings.as_ref().is_some_and(|page| page.capture)
    }

    /// A settings row is accepting typed text.
    pub fn drafting(&self) -> bool {
        self.settings
            .as_ref()
            .is_some_and(|page| page.text.is_some())
    }

    pub fn attach_agent(
        &mut self,
        tx: tokio::sync::mpsc::UnboundedSender<crate::agent::AgentCommand>,
    ) {
        self.agent_tx = Some(tx);
    }

    /// Rows the assistant panel takes. Zero unless the prompt itself is focused.
    /// Grows with the prompt (up to 8 wrapped lines) so multi-line input stays visible.
    pub fn agent_rows(&self) -> u16 {
        if self.focus() != Focus::Agent {
            return 0;
        }
        let height = self.screen_height;
        if height < 10 {
            return 0;
        }
        let base = 8.min(height / 3).max(4);
        let width = self.screen_width.saturating_sub(2).saturating_sub(3).max(1) as usize;
        let prompt_lines = crate::agent::wrap_rows(&self.agent.prompt, width)
            .len()
            .clamp(1, 8) as u16;
        // Keep the previous minimum panel size; grow when the prompt needs more rows.
        let grown = 3u16.saturating_add(prompt_lines);
        grown.max(base).min(height.saturating_sub(6)).max(4)
    }

    pub fn focus(&self) -> Focus {
        if self.prompt.is_some() {
            Focus::Prompt
        } else if self.command_bar.is_some() {
            Focus::CommandBar
        } else if self.about {
            Focus::About
        } else if self.settings.is_some() {
            Focus::Settings
        } else if self.file_picker.is_some() {
            Focus::FilePicker
        } else if self.replace_mode.is_some() {
            Focus::Replace
        } else if self.find_mode.is_some() {
            Focus::Find
        } else if self.completion_view().is_some() {
            Focus::CompletionPopup
        } else if self.reference_view().is_some() || self.action_view().is_some() {
            Focus::ActionMenu
        } else if self.agent.open && self.agent.focused {
            Focus::Agent
        } else if self.explorer.open && self.explorer.focused {
            Focus::Sidebar
        } else {
            Focus::Editor
        }
    }

    pub fn recent_for(&self, context: CommandContext) -> Option<CommandId> {
        match context {
            CommandContext::Editor => self.recent_command,
            CommandContext::Explorer => self.recent_explorer,
        }
    }

    pub fn take_jobs(&mut self) -> Vec<Job> {
        std::mem::take(&mut self.jobs)
    }

    /// Results from background jobs. Stale results for a different root are dropped.
    pub fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::TreeScanned { root, entries } if self.same_root(&root) => {
                self.explorer.set_entries(entries);
                self.finish_reveal_file();
                if self.file_find.is_some() {
                    self.refresh_file_find();
                }
                if self.file_picker.is_some() {
                    self.queue_index();
                }
                if self.take_tree_follow_up(&root) {
                    self.queue_tree_scan(true);
                }
            }
            AppEvent::GitStatus { root, statuses } if self.same_root(&root) => {
                self.explorer.git = statuses;
            }
            AppEvent::FileSaved {
                editor_id,
                tab_id,
                path,
                fingerprint,
                epoch,
                after,
                result,
            } => self.finish_save(editor_id, tab_id, path, fingerprint, epoch, after, result),
            AppEvent::FileLoaded {
                editor_id,
                tab_id,
                path,
                generation,
                kind,
                result,
            } => self.finish_load(editor_id, tab_id, path, generation, kind, result),
            AppEvent::FileStatted {
                tab_id,
                path,
                stamp,
                result,
            } => self.finish_stat(tab_id, path, stamp, result),
            AppEvent::FilesChanged { root, outcome } if self.same_root(&root) => {
                self.finish_files(outcome);
            }
            AppEvent::FilesFound {
                root,
                generation,
                hits,
            } if self.same_root(&root) => self.finish_file_find(generation, hits),
            AppEvent::FilesReplaced {
                root,
                generation,
                matches,
                files,
                errors,
            } if self.same_root(&root) => {
                self.finish_file_replace(generation, matches, files, errors)
            }
            AppEvent::FilesPatched { files, errors } => {
                let mut message = if files == 1 {
                    "Updated 1 file".to_string()
                } else {
                    format!("Updated {files} files")
                };
                if let Some(err) = errors.first() {
                    message = format!("{message}. {err}");
                }
                self.message = Some(message);
            }
            AppEvent::TextWritten { result, .. } => {
                self.settings_save_inflight = false;
                if let Err(message) = result {
                    self.message = Some(message);
                }
                if self.settings_save_pending {
                    self.queue_settings_save();
                }
            }
            AppEvent::FilesIndexed {
                root,
                generation,
                files,
                truncated,
            } if self.same_root(&root) && generation == self.index_generation => {
                self.file_index = Some(CachedFiles {
                    root,
                    files: Arc::new(files),
                    truncated,
                });
                if let Some(picker) = &mut self.file_picker {
                    picker.truncated = truncated;
                    picker.refreshing = !picker.rows.is_empty();
                    picker.scanning = picker.rows.is_empty();
                }
                if self.file_picker.is_some() {
                    let preserve = self
                        .file_picker
                        .as_ref()
                        .is_some_and(|picker| !picker.rows.is_empty());
                    self.queue_filter(!preserve);
                }
            }
            AppEvent::PickerFiltered {
                generation,
                indices,
                total,
            } if self
                .file_picker
                .as_ref()
                .is_some_and(|picker| picker.pending_generation == generation) =>
            {
                self.apply_picker_rows(indices, total);
            }
            AppEvent::Parsed {
                tab_id,
                generation,
                version,
                parsed,
            } => self.finish_parse(tab_id, generation, version, parsed),
            AppEvent::GitDiff {
                tab_id,
                path,
                generation,
                fingerprint,
                result,
            } => self.finish_diff(tab_id, path, generation, fingerprint, result),
            AppEvent::AgentDisk {
                turn,
                call_id,
                result,
                open,
            } => {
                if turn == self.agent.turn && self.agent.busy {
                    if let Some(path) = open {
                        self.open_path(path);
                    }
                }
                self.reply_tool(turn, call_id, result);
            }
            AppEvent::SessionSaved {
                result: Err(message),
            } => {
                self.message = Some(message);
            }
            AppEvent::GitBranch { root, name } if self.same_root(&root) => {
                self.branch = name;
            }
            _ => {}
        }
    }

    /// Drops the current tree, then attaches a highlighter for the tab's path.
    /// Used when the buffer text is replaced wholesale.
    fn reset_syntax(&mut self, tab_id: TabId) {
        if let Some(tab) = self.find_tab_mut(tab_id) {
            tab.syntax = None;
        }
        self.configure_syntax(tab_id);
    }

    /// Attaches a highlighter when `path` names a supported language, and queues a parse.
    fn configure_syntax(&mut self, tab_id: TabId) {
        let Some(tab) = self.find_tab(tab_id) else {
            return;
        };
        let language = tab
            .path
            .as_deref()
            .and_then(syntax::Language::detect)
            .filter(|_| tab.buffer.rope().len_bytes() <= syntax::MAX_HIGHLIGHT_BYTES);
        let Some(language) = language else {
            if let Some(tab) = self.find_tab_mut(tab_id) {
                tab.syntax = None;
            }
            return;
        };
        let version = self
            .find_tab(tab_id)
            .map(|tab| tab.buffer.revision())
            .unwrap_or(0);
        let same = self
            .find_tab(tab_id)
            .and_then(|tab| tab.syntax.as_ref())
            .is_some_and(|state| state.language() == language);
        if !same {
            let tab = self.find_tab_mut(tab_id).expect("tab exists");
            tab.syntax_generation = tab.syntax_generation.wrapping_add(1);
            let generation = tab.syntax_generation;
            tab.syntax = Some(SyntaxState::fresh(language, generation, version));
        }
        self.queue_parse(tab_id);
    }

    fn sync_tab_syntax(&mut self, tab_id: TabId) {
        let Some(tab) = self.find_tab_mut(tab_id) else {
            return;
        };
        let (version, edits) = tab.buffer.take_edits();
        if edits.is_empty() {
            return;
        }
        tab.lsp_edits.extend_from_slice(&edits);
        if tab.buffer.rope().len_bytes() > syntax::MAX_HIGHLIGHT_BYTES {
            tab.syntax = None;
            return;
        }
        let Some(state) = tab.syntax.as_mut() else {
            return;
        };
        state.apply_edits(&edits);
        state.set_version(version);
        state.set_dirty(true);
        self.queue_parse(tab_id);
    }

    fn queue_parse(&mut self, tab_id: TabId) {
        let Some(request) = self.parse_request(tab_id) else {
            return;
        };
        if let Some(state) = self
            .find_tab_mut(tab_id)
            .and_then(|tab| tab.syntax.as_mut())
        {
            state.set_in_flight(true);
            state.set_dirty(false);
        }
        self.jobs.push(Job::Parse { tab_id, request });
    }

    fn parse_request(&self, tab_id: TabId) -> Option<syntax::ParseInput> {
        let tab = self.find_tab(tab_id)?;
        let state = tab.syntax.as_ref()?;
        if state.in_flight() || !state.dirty() {
            return None;
        }
        if tab.buffer.rope().len_bytes() > syntax::MAX_HIGHLIGHT_BYTES {
            return None;
        }
        // Rope::clone is O(1) (shared chunks); no to_string on the UI thread.
        Some(syntax::ParseInput {
            language: state.language(),
            generation: state.generation(),
            version: state.version(),
            rope: tab.buffer.rope().clone(),
            old: state.snapshot(),
        })
    }

    fn finish_parse(
        &mut self,
        tab_id: TabId,
        generation: u64,
        version: u64,
        parsed: Option<syntax::Parsed>,
    ) {
        let current = self
            .find_tab(tab_id)
            .and_then(|tab| tab.syntax.as_ref())
            .is_some_and(|state| state.generation() == generation);
        if !current {
            return;
        }
        let state = self
            .find_tab_mut(tab_id)
            .and_then(|tab| tab.syntax.as_mut())
            .expect("syntax generation matched");
        state.set_in_flight(false);
        if state.version() != version {
            state.set_dirty(true);
        } else if let Some(parsed) = parsed {
            state.install(parsed);
        }
        self.queue_parse(tab_id);
    }

    /// Queues one write of `settings.json`. A change that arrives while a write
    /// is already queued is saved when that write finishes, so the file ends on
    /// the latest choice.
    fn queue_settings_save(&mut self) {
        if self.settings_save_inflight {
            self.settings_save_pending = true;
            return;
        }
        let Some(dir) = self.appearance.config_dir().map(Path::to_path_buf) else {
            return;
        };
        let file = crate::settings::SettingsFile {
            color_theme: self.appearance.color.id.clone(),
            syntax_theme: self.appearance.syntax.id.clone(),
            editor: self.editor_prefs,
            files: self.file_prefs,
            keybindings: self.key_overrides.clone(),
            lsp: self.appearance.lsp.clone(),
            ai: self.ai.clone(),
        };
        let (path, text) = file.document(&dir);
        self.settings_save_inflight = true;
        self.settings_save_pending = false;
        self.jobs.push(Job::WriteText { path, text });
    }

    fn remember_file(&mut self, path: &Path) {
        let Some(relative) = relative_to(&self.explorer.root, path) else {
            return;
        };
        let text = display_rel(&relative);
        if text.is_empty() {
            return;
        }
        self.recent_files.retain(|item| item != &text);
        self.recent_files.insert(0, text);
        self.recent_files.truncate(32);
    }

    fn ranking_paths(&self) -> Vec<String> {
        let mut paths = self.recent_files.clone();
        for editor in &self.editors {
            for tab in editor.tabs() {
                let Some(path) = tab.path.as_deref() else {
                    continue;
                };
                let Some(relative) = relative_to(&self.explorer.root, path) else {
                    continue;
                };
                let text = display_rel(&relative);
                if !text.is_empty() && !paths.iter().any(|item| item == &text) {
                    paths.push(text);
                }
            }
        }
        paths
    }

    fn active_dir(&self) -> String {
        let Some(path) = self.path() else {
            return String::new();
        };
        let Some(relative) = relative_to(&self.explorer.root, path) else {
            return String::new();
        };
        let text = display_rel(&relative);
        text.rfind('/')
            .map(|index| text[..index].to_string())
            .unwrap_or_default()
    }

    pub fn picker_file(&self, index: u32) -> Option<&IndexedFile> {
        self.file_index.as_ref()?.files.get(index as usize)
    }

    fn open_file_picker(&mut self) {
        self.remember_sidebar();
        self.explorer.blur();
        self.command_bar = None;
        self.find_mode = None;
        self.replace_mode = None;
        self.settings = None;
        self.about = false;
        let cached = self
            .file_index
            .as_ref()
            .is_some_and(|index| self.same_root(&index.root));
        self.file_picker = Some(FilePicker {
            scanning: !cached,
            refreshing: cached,
            truncated: self
                .file_index
                .as_ref()
                .is_some_and(|index| cached && index.truncated),
            ..FilePicker::default()
        });
        if cached {
            self.queue_filter(true);
        }
        self.queue_index();
    }

    fn close_picker(&mut self) {
        if self.file_picker.take().is_none() {
            return;
        }
        self.jobs
            .retain(|job| !matches!(job, Job::FilterFiles { .. }));
        self.restore_sidebar_focus();
    }

    fn handle_picker(&mut self, action: PickerAction) {
        match action {
            PickerAction::Close => self.close_picker(),
            PickerAction::Submit => self.accept_picker(),
            action => {
                let page = picker::list_rows(self.screen_height);
                let edited = self
                    .file_picker
                    .as_mut()
                    .is_some_and(|picker| picker.edit(action, page));
                if edited {
                    self.queue_filter(true);
                }
            }
        }
    }

    fn accept_picker(&mut self) {
        let Some(path) = self.selected_picker_path() else {
            return;
        };
        self.file_picker = None;
        self.prompt_restore_sidebar = false;
        self.jobs
            .retain(|job| !matches!(job, Job::FilterFiles { .. }));
        self.open_path(path);
    }

    fn selected_picker_path(&self) -> Option<PathBuf> {
        let picker = self.file_picker.as_ref()?;
        let row = *picker.rows.get(picker.selected)?;
        let file = self.file_index.as_ref()?.files.get(row as usize)?;
        Some(picker::absolute_path(&self.explorer.root, &file.relative))
    }

    fn queue_index(&mut self) {
        self.index_generation = self.index_generation.wrapping_add(1);
        let generation = self.index_generation;
        let root = self.explorer.root.clone();
        self.jobs
            .retain(|job| !matches!(job, Job::IndexFiles { .. }));
        self.jobs.push(Job::IndexFiles { root, generation });
        let cached = self
            .file_index
            .as_ref()
            .is_some_and(|index| self.same_root(&index.root));
        if let Some(picker) = &mut self.file_picker {
            picker.scanning = !cached && picker.rows.is_empty();
            picker.refreshing = cached || !picker.rows.is_empty();
        }
    }

    fn refresh_picker_index(&mut self) {
        if self.file_picker.is_none() && self.file_index.is_none() {
            return;
        }
        self.queue_index();
    }

    fn queue_filter(&mut self, reset: bool) {
        let ready = self
            .file_index
            .as_ref()
            .is_some_and(|index| self.same_root(&index.root))
            && self.file_picker.is_some();
        if !ready {
            return;
        }
        let preserve = if reset {
            None
        } else {
            let row = self
                .file_picker
                .as_ref()
                .and_then(|picker| picker.rows.get(picker.selected).copied());
            row.and_then(|row| {
                self.file_index.as_ref().and_then(|index| {
                    index
                        .files
                        .get(row as usize)
                        .map(|file| file.relative.clone())
                })
            })
        };
        let files = Arc::clone(&self.file_index.as_ref().unwrap().files);
        let recent = self.ranking_paths();
        let active_dir = self.active_dir();
        let (generation, query) = {
            let picker = self.file_picker.as_mut().unwrap();
            picker.pending_generation = picker.pending_generation.wrapping_add(1);
            picker.preserve = preserve;
            if reset {
                picker.selected = 0;
                picker.scroll = 0;
            }
            (picker.pending_generation, picker.query.clone())
        };
        self.jobs
            .retain(|job| !matches!(job, Job::FilterFiles { .. }));
        self.jobs.push(Job::FilterFiles {
            generation,
            query,
            files,
            recent,
            active_dir,
        });
    }

    fn apply_picker_rows(&mut self, indices: Vec<u32>, total: usize) {
        let preserve = self
            .file_picker
            .as_ref()
            .and_then(|picker| picker.preserve.clone());
        let files = self
            .file_index
            .as_ref()
            .map(|index| Arc::clone(&index.files));
        let page = picker::list_rows(self.screen_height);
        let Some(picker) = &mut self.file_picker else {
            return;
        };
        picker.rows = indices;
        picker.total = total;
        picker.scanning = false;
        picker.refreshing = false;
        if let Some(path) = preserve {
            picker.selected = files
                .as_ref()
                .and_then(|files| {
                    picker.rows.iter().position(|&row| {
                        files
                            .get(row as usize)
                            .is_some_and(|file| file.relative == path)
                    })
                })
                .unwrap_or(0);
        } else {
            picker.selected = 0;
            picker.scroll = 0;
        }
        picker.reveal(page);
    }

    /// `Ctrl+B`: closed → open and focus; open but unfocused → focus; focused → close.
    /// Focus mode stays on; only `focus` again leaves it, so the sidebar stays hidden.
    fn toggle_sidebar(&mut self) {
        if self.focus_mode {
            return;
        }
        if !self.explorer.open {
            self.reveal_sidebar();
            self.explorer.focused = true;
        } else if self.explorer.focused {
            self.explorer.open = false;
            self.explorer.blur();
            self.end_file_mode();
        } else {
            self.explorer.focused = true;
        }
    }

    /// Grow or shrink the open sidebar by one step. A closed sidebar, and focus mode, stay put.
    /// At either limit the status line says so.
    fn resize_sidebar(&mut self, wider: bool) {
        if !self.explorer.open || self.focus_mode {
            return;
        }
        let shown = layout::clamp_sidebar_width(self.screen_width, self.sidebar_width);
        let next = layout::step_sidebar_width(self.screen_width, shown, wider);
        if next == shown {
            self.message = Some(
                if wider {
                    "Sidebar is at its widest"
                } else {
                    "Sidebar is at its narrowest"
                }
                .to_string(),
            );
            return;
        }
        self.sidebar_width = next;
        self.message = None;
    }

    /// Open the sidebar on the focused file without moving focus. A closed sidebar
    /// opens and keeps the editor focused. Focus mode stays as it is.
    fn reveal_focused_file(&mut self) -> Result<(), String> {
        if self.focus_mode {
            return Ok(());
        }
        let path = self
            .find_tab(self.active_tab_id())
            .and_then(|tab| tab.path.clone())
            .ok_or_else(|| "No file to reveal".to_string())?;
        let rel = self
            .relative_to_root(&path)
            .filter(|rel| !rel.as_os_str().is_empty())
            .ok_or_else(|| "File is not in the explorer".to_string())?;
        if !self.explorer.open {
            self.reveal_sidebar();
        }
        if self.explorer.reveal_file(&rel) {
            self.reveal_file = None;
            return Ok(());
        }
        if self.explorer.loading {
            self.reveal_file = Some(rel);
            return Ok(());
        }
        Err("File is not in the explorer".to_string())
    }

    /// Applies a `reveal` that waited for the tree scan.
    fn finish_reveal_file(&mut self) {
        let Some(path) = self.reveal_file.take() else {
            return;
        };
        if !self.explorer.reveal_file(&path) {
            self.message = Some("File is not in the explorer".to_string());
        }
    }

    /// Open the sidebar and queue a scan. A sidebar that is already open is left as-is.
    fn reveal_sidebar(&mut self) {
        if self.explorer.open {
            return;
        }
        self.explorer.open = true;
        self.queue_tree_scan(true);
    }

    /// The folder watcher, after its quiet period, asks for one scan of `root`.
    /// A different folder and a closed sidebar are ignored.
    pub fn note_fs_changed(&mut self, root: &Path) {
        if !self.same_root(root) {
            return;
        }
        self.queue_tree_scan(true);
    }

    /// Queues one tree scan and one git status.
    /// A scan already in the queue is left alone, because it has not read the disk yet.
    /// A scan that is already running remembers one later pass in `tree_scan.dirty`.
    /// `Loading…` is only shown when the tree has no rows yet.
    /// `require_open` skips the scan while the sidebar is closed.
    fn queue_tree_scan(&mut self, require_open: bool) {
        if require_open && !self.explorer.open {
            return;
        }
        let root = self.explorer.root.clone();
        let queued = self.jobs.iter().any(
            |job| matches!(job, Job::ScanTree(path) if crate::session::same_directory(path, &root)),
        );
        if queued {
            return;
        }
        if let Some(scan) = &mut self.tree_scan
            && crate::session::same_directory(&scan.root, &root)
        {
            scan.dirty = true;
            return;
        }
        self.tree_scan = Some(TreeScan {
            root: root.clone(),
            dirty: false,
        });
        if self.explorer.entries().is_empty() {
            self.explorer.loading = true;
        }
        self.jobs.push(Job::ScanTree(root.clone()));
        let status_queued = self.jobs.iter().any(|job| {
            matches!(job, Job::GitStatus(path) if crate::session::same_directory(path, &root))
        });
        if !status_queued {
            self.jobs.push(Job::GitStatus(root));
        }
    }

    /// Clears the tracker for this scan. `true` when one more scan should run.
    fn take_tree_follow_up(&mut self, root: &Path) -> bool {
        let matches = self
            .tree_scan
            .as_ref()
            .is_some_and(|scan| crate::session::same_directory(&scan.root, root));
        if !matches {
            return false;
        }
        let dirty = self.tree_scan.take().is_some_and(|scan| scan.dirty);
        dirty && self.explorer.open
    }

    /// Open windows, placed the way the screen draws them. A closed sidebar is absent.
    fn focus_surfaces(&self) -> Vec<(Surface, ratatui::layout::Rect)> {
        let area = ratatui::layout::Rect::new(0, 0, self.screen_width, self.screen_height);
        let agent = self.agent_rows();
        let (sidebar, editors) =
            layout::split_above(area, self.explorer.open, 1 + agent, self.sidebar_width);
        let mut surfaces = Vec::new();
        if self.explorer.open && sidebar.width > 0 && sidebar.height > 0 {
            surfaces.push((Surface::Sidebar, sidebar));
        }
        surfaces.extend(
            self.layout
                .place(editors)
                .panes
                .into_iter()
                .filter(|(_, rect)| rect.width > 0 && rect.height > 0)
                .map(|(id, rect)| (Surface::Editor(id), rect)),
        );
        if agent > 0 {
            let y = editors.y.saturating_add(editors.height);
            surfaces.push((
                Surface::Agent,
                ratatui::layout::Rect::new(area.x, y, area.width, agent),
            ));
        }
        surfaces
    }

    /// A cell inside `from`: the text cursor for an editor, the middle of the sidebar.
    fn focus_anchor(
        &self,
        from: Surface,
        surfaces: &[(Surface, ratatui::layout::Rect)],
    ) -> (u16, u16) {
        let Some(rect) = surfaces
            .iter()
            .find(|(surface, _)| *surface == from)
            .map(|(_, rect)| *rect)
        else {
            return (0, 0);
        };
        match from {
            Surface::Sidebar | Surface::Agent => (
                rect.x.saturating_add(rect.width / 2),
                rect.y.saturating_add(rect.height / 2),
            ),
            Surface::Editor(id) => {
                let tab = self.editor(id).tab();
                let (line, col) = tab.buffer.cursor_line_col();
                let row = line.saturating_sub(tab.view.scroll_y) as u16;
                let col = col.saturating_sub(tab.view.scroll_x) as u16;
                let gutter =
                    (tab.buffer.len_lines().to_string().len().max(3) as u16 + 2).min(rect.width);
                (
                    rect.x.saturating_add(gutter).saturating_add(col),
                    rect.y
                        .saturating_add(layout::text_row_offset(rect.height))
                        .saturating_add(row),
                )
            }
        }
    }

    /// `Ctrl` + arrow. Moves along the cursor's row or column to the nearest open
    /// window on that side. A closed sidebar is not a destination and is not opened.
    fn focus_adjacent(&mut self, direction: PaneDirection) {
        let from = if self.agent.open && self.agent.focused {
            Surface::Agent
        } else if self.explorer.open && self.explorer.focused {
            Surface::Sidebar
        } else {
            Surface::Editor(self.active)
        };
        let surfaces = self.focus_surfaces();
        let anchor = self.focus_anchor(from, &surfaces);
        let Some(next) = layout::adjacent_surface(&surfaces, from, anchor, direction) else {
            return;
        };
        match next {
            Surface::Sidebar if self.focus_mode => {}
            Surface::Sidebar => {
                self.agent.focused = false;
                self.explorer.focused = true;
            }
            Surface::Editor(id) => {
                self.agent.focused = false;
                self.active = id;
                self.explorer.blur();
                self.end_file_mode();
            }
            Surface::Agent => {
                self.agent.focused = true;
                self.explorer.blur();
                self.end_file_mode();
            }
        }
    }

    fn split(&mut self, axis: Axis) {
        let id = EditorId(self.next_id);
        self.next_id += 1;
        let buffer = self.style_buffer(Buffer::new());
        let tab = self.alloc_tab(buffer, None);
        self.editors.push(fresh_editor(id, tab));
        self.layout =
            std::mem::replace(&mut self.layout, Layout::Leaf(id)).split(self.active, axis, id);
        self.active = id;
        self.explorer.blur();
        self.end_file_mode();
    }

    fn close_active(&mut self) -> Result<(), String> {
        let closed_pane = self.active;
        if self.editor(closed_pane).tabs.len() > 1 {
            let index = self.editor(closed_pane).active;
            let removed = self.editor(closed_pane).tabs[index].id;
            let editor = self.editor_mut(closed_pane).expect("pane");
            editor.tabs.remove(index);
            editor.active = index.min(editor.tabs.len() - 1);
            self.quit_skipped.retain(|id| *id != removed);
            self.explorer.blur();
            self.end_file_mode();
            return Ok(());
        }
        let closed = closed_pane;
        let Some(next) = self.layout.focus_after_close(closed) else {
            self.blank_editor(closed);
            self.explorer.blur();
            self.end_file_mode();
            return Ok(());
        };
        let current = std::mem::replace(&mut self.layout, Layout::Leaf(next));
        match current.close(closed) {
            Ok(layout) => self.layout = layout,
            Err(layout) => {
                self.layout = layout;
                return Err("Cannot close the last editor window".to_string());
            }
        }
        let dropped: Vec<TabId> = self.editor(closed).tabs.iter().map(|tab| tab.id).collect();
        self.editors.retain(|editor| editor.id != closed);
        self.quit_skipped.retain(|id| !dropped.contains(id));
        self.active = next;
        self.explorer.blur();
        self.end_file_mode();
        Ok(())
    }

    /// The last window stays on screen as an empty untitled page so typing still has a buffer.
    fn blank_editor(&mut self, id: EditorId) {
        let kept = self
            .editors
            .iter()
            .find(|editor| editor.id == id)
            .map(|editor| {
                let tab = editor.tab();
                (
                    tab.view.width,
                    tab.view.height,
                    tab.epoch.wrapping_add(1),
                    tab.load_generation.wrapping_add(1),
                    tab.id,
                )
            });
        let Some((width, height, epoch, load_generation, tab_id)) = kept else {
            return;
        };
        let buffer = self.style_buffer(Buffer::new());
        let Some(editor) = self.editor_mut(id) else {
            return;
        };
        let tab = editor.tab_mut();
        *tab = fresh_tab(tab_id, buffer, None);
        tab.view.resize(width as u16, height as u16);
        tab.epoch = epoch;
        tab.load_generation = load_generation;
    }

    fn alloc_tab(&mut self, buffer: Buffer, path: Option<PathBuf>) -> Tab {
        let id = TabId(self.next_tab);
        self.next_tab += 1;
        fresh_tab(id, buffer, path)
    }

    fn push_tab(&mut self, pane: EditorId) -> TabId {
        let buffer = self.style_buffer(Buffer::new());
        let tab = self.alloc_tab(buffer, None);
        let id = tab.id;
        let editor = self.editor_mut(pane).expect("pane");
        editor.tabs.push(tab);
        editor.active = editor.tabs.len() - 1;
        id
    }

    fn cycle_tab(&mut self, next: bool) {
        let editor = self.active_editor_mut();
        let len = editor.tabs.len();
        if len < 2 {
            return;
        }
        let delta = if next { 1 } else { -1 };
        editor.active = (editor.active as isize + delta).rem_euclid(len as isize) as usize;
    }

    fn focus_tab(&mut self, id: TabId) -> bool {
        let found = self.editors.iter_mut().find_map(|editor| {
            let index = editor.tabs.iter().position(|tab| tab.id == id)?;
            editor.active = index;
            Some(editor.id)
        });
        if let Some(pane) = found {
            self.active = pane;
            true
        } else {
            false
        }
    }

    fn handle_sidebar(&mut self, action: SidebarAction) {
        if self.file_find.is_some()
            && matches!(
                action,
                SidebarAction::FilterInsert(_) | SidebarAction::FilterBackspace
            )
        {
            return;
        }
        if matches!(action, SidebarAction::Escape) && self.file_find.is_some() {
            self.exit_file_find();
            return;
        }
        if matches!(action, SidebarAction::Activate) {
            if let Some(path) = self.explorer.activate() {
                let path = self.explorer.root.join(path);
                let query = self.file_find.as_ref().map(|search| search.query.clone());
                let already = query.is_some() && self.tab_with_path(&path).is_some();
                if let Some(query) = query.clone()
                    && !already
                {
                    self.reveal_find = Some((path.clone(), query));
                }
                self.open_path(path);
                if already && let Some(query) = query {
                    self.open_find_mode(&query);
                }
            }
            return;
        }
        if matches!(action, SidebarAction::Mark) {
            if let Err(message) = self.commit_marks() {
                self.message = Some(message);
            }
            return;
        }
        if matches!(action, SidebarAction::ExitSelect) {
            self.end_file_mode();
            self.message = Some("Exited FILE".to_string());
            return;
        }
        let explorer = &mut self.explorer;
        let page = explorer.height.max(1) as isize;
        match action {
            SidebarAction::Up { extend } => explorer.move_selection(-1, extend),
            SidebarAction::Down { extend } => explorer.move_selection(1, extend),
            SidebarAction::PageUp { extend } => explorer.move_selection(-page, extend),
            SidebarAction::PageDown { extend } => explorer.move_selection(page, extend),
            SidebarAction::First { extend } => explorer.select_first(extend),
            SidebarAction::Last { extend } => explorer.select_last(extend),
            SidebarAction::Expand { extend } => explorer.expand(extend),
            SidebarAction::Collapse { extend } => explorer.collapse(extend),
            SidebarAction::Activate => {}
            SidebarAction::FilterInsert(text) => explorer.insert_filter(&text),
            SidebarAction::FilterBackspace => explorer.pop_filter(),
            SidebarAction::Escape if !explorer.filter.is_empty() => explorer.clear_filter(),
            SidebarAction::Escape => explorer.blur(),
            SidebarAction::Mark | SidebarAction::ExitSelect => {}
        }
    }

    pub fn handle(&mut self, action: SemanticAction) {
        self.message = None;
        match action {
            SemanticAction::Move { movement, extend } => {
                self.dismiss_lsp_popups();
                self.clear_snippet();
                self.buffer_mut().move_cursor(movement, extend);
            }
            SemanticAction::PageUp { extend } => {
                self.dismiss_lsp_popups();
                self.clear_snippet();
                self.page(false, extend);
            }
            SemanticAction::PageDown { extend } => {
                self.dismiss_lsp_popups();
                self.clear_snippet();
                self.page(true, extend);
            }
            SemanticAction::InsertChar(c) => {
                let selection = self.buffer().selection();
                let before = self.buffer().rope().len_chars();
                self.edit(|buffer| buffer.insert_char(c));
                let after = self.buffer().rope().len_chars();
                if after != before {
                    let removed = selection.to() - selection.from();
                    let pos = if selection.is_empty() {
                        selection.head
                    } else {
                        selection.from()
                    };
                    self.shift_snippet(pos, removed, 1);
                }
                self.note_typed(Some(c));
            }
            SemanticAction::InsertNewline => {
                self.dismiss_lsp_popups();
                self.clear_snippet();
                self.edit(|buffer| buffer.insert_newline());
            }
            SemanticAction::InsertText(text) => {
                self.dismiss_lsp_popups();
                self.clear_snippet();
                self.edit(|buffer| buffer.insert_text(&text));
            }
            SemanticAction::DeleteBackward => {
                let selection = self.buffer().selection();
                let before = self.buffer().rope().len_chars();
                self.edit(|buffer| buffer.delete_backward());
                let after = self.buffer().rope().len_chars();
                if after != before {
                    if selection.is_empty() {
                        self.shift_snippet(selection.head.saturating_sub(1), 1, 0);
                    } else {
                        self.shift_snippet(selection.from(), selection.to() - selection.from(), 0);
                    }
                }
                self.note_typed(None);
            }
            SemanticAction::DeleteForward => {
                let selection = self.buffer().selection();
                let before = self.buffer().rope().len_chars();
                self.edit(|buffer| buffer.delete_forward());
                let after = self.buffer().rope().len_chars();
                if after != before {
                    let removed = if selection.is_empty() {
                        1
                    } else {
                        selection.to() - selection.from()
                    };
                    self.shift_snippet(selection.from(), removed, 0);
                }
                self.note_typed(None);
            }
            SemanticAction::Indent => {
                if self.snippet_jump(true) {
                    return;
                }
                self.dismiss_lsp_popups();
                self.edit(|buffer| buffer.indent());
            }
            SemanticAction::Outdent => {
                if self.snippet_jump(false) {
                    return;
                }
                self.dismiss_lsp_popups();
                self.edit(|buffer| buffer.outdent());
            }
            SemanticAction::Dismiss => {
                self.clear_snippet();
                self.dismiss_lsp_popups();
            }
            SemanticAction::SelectAll => {
                self.dismiss_lsp_popups();
                self.buffer_mut().select_all();
            }
            SemanticAction::Undo => {
                self.dismiss_lsp_popups();
                self.edit(|buffer| {
                    buffer.undo();
                });
            }
            SemanticAction::Redo => {
                self.dismiss_lsp_popups();
                self.edit(|buffer| {
                    buffer.redo();
                });
            }
            SemanticAction::Save => {
                if let Err(message) = self.save_focused(None, None) {
                    self.message = Some(message);
                }
            }
            SemanticAction::Copy => {
                if let Err(message) = self.copy() {
                    self.message = Some(message);
                }
            }
            SemanticAction::Cut => {
                if let Err(message) = self.cut() {
                    self.message = Some(message);
                }
            }
            SemanticAction::Paste => {
                self.dismiss_lsp_popups();
                if let Err(message) = self.paste() {
                    self.message = Some(message);
                }
            }
            SemanticAction::OpenCommandBar => {
                self.dismiss_lsp_popups();
                self.open_command_bar();
            }
            SemanticAction::OpenFilePicker => {
                self.dismiss_lsp_popups();
                self.open_file_picker();
            }
            SemanticAction::Picker(action) => self.handle_picker(action),
            SemanticAction::StartFind => {
                self.dismiss_lsp_popups();
                self.open_find_mode("");
            }
            SemanticAction::Find(action) => self.handle_find(action),
            SemanticAction::StartReplace => {
                self.dismiss_lsp_popups();
                self.open_replace_mode("");
            }
            SemanticAction::Replace(action) => self.handle_replace(action),
            SemanticAction::FindNext => {
                if let Err(message) = self.find_adjacent(true) {
                    self.message = Some(message);
                }
            }
            SemanticAction::FindPrevious => {
                if let Err(message) = self.find_adjacent(false) {
                    self.message = Some(message);
                }
            }
            SemanticAction::CommandBar(action) => self.handle_command_bar(action),
            SemanticAction::ToggleSidebar => self.toggle_sidebar(),
            SemanticAction::ResizeSidebar { wider } => self.resize_sidebar(wider),
            SemanticAction::Sidebar(action) => self.handle_sidebar(action),
            SemanticAction::FocusAdjacent(direction) => self.focus_adjacent(direction),
            SemanticAction::CycleTab { next } => self.cycle_tab(next),
            SemanticAction::GotoDiagnostic { next } => {
                self.dismiss_lsp_popups();
                self.goto_diagnostic(next);
            }
            SemanticAction::Hover => self.show_hover(),
            SemanticAction::GotoDefinition => self.goto_definition(),
            SemanticAction::GotoReferences => self.goto_references(),
            SemanticAction::MatchBracket { extend } => self.match_bracket(extend),
            SemanticAction::Complete => self.complete_now(),
            SemanticAction::MoveLine { down } => {
                self.clear_snippet();
                if !self.buffer_mut().move_block(down) {
                    self.message = Some("The line is already at the edge".to_string());
                }
            }
            SemanticAction::DeleteLine => {
                self.clear_snippet();
                self.edit(|buffer| buffer.delete_lines());
            }
            SemanticAction::DuplicateLine { down } => {
                self.clear_snippet();
                self.edit(|buffer| buffer.duplicate_block(down));
            }
            SemanticAction::ToggleComment => self.toggle_comment(),
            SemanticAction::StartRename => self.start_rename(),
            SemanticAction::Format => self.format_document(),
            SemanticAction::CodeAction => self.code_action(),
            SemanticAction::JumpBack => self.jump_back(),
            SemanticAction::JumpForward => self.jump_forward(),
            SemanticAction::Completion(action) if self.nav.references.is_some() => match action {
                CompletionAction::Prev => self.move_reference(false),
                CompletionAction::Next => self.move_reference(true),
                CompletionAction::Accept => self.accept_reference(),
                CompletionAction::Close => self.close_references(),
            },
            SemanticAction::Completion(action) if self.nav.actions.is_some() => match action {
                CompletionAction::Prev => self.move_action(false),
                CompletionAction::Next => self.move_action(true),
                CompletionAction::Accept => self.accept_action(),
                CompletionAction::Close => self.nav.actions = None,
            },
            SemanticAction::Completion(CompletionAction::Prev) => self.completion_move(false),
            SemanticAction::Completion(CompletionAction::Next) => self.completion_move(true),
            SemanticAction::Completion(CompletionAction::Accept) => self.accept_completion(),
            SemanticAction::Completion(CompletionAction::Close) => self.close_completion(),
            SemanticAction::Quit => self.request_quit(),
            SemanticAction::Prompt(action) => self.handle_prompt(action),
            SemanticAction::Settings(action) => self.handle_settings(action),
            SemanticAction::CloseAbout => self.about = false,
            SemanticAction::ToggleAgent => self.toggle_agent(),
            SemanticAction::Agent(action) => self.handle_agent_input(action),
        }
    }

    fn handle_settings(&mut self, action: SettingsAction) {
        if matches!(action, SettingsAction::Reset) {
            self.reset_highlighted();
            return;
        }
        if let SettingsAction::Assign(token) = &action {
            self.assign_highlighted(token);
            return;
        }
        if self.settings.as_ref().is_some_and(|page| page.capture) {
            if matches!(action, SettingsAction::Back) {
                if let Some(page) = self.settings.as_mut() {
                    page.capture = false;
                }
            }
            return;
        }
        if self
            .settings
            .as_ref()
            .is_some_and(|page| page.text.is_some())
        {
            self.edit_text_setting(action);
            return;
        }
        if self
            .settings
            .as_ref()
            .is_some_and(|page| page.menu.is_some())
        {
            self.handle_choice_menu(action);
            return;
        }
        let opens_menu = self.settings.as_ref().and_then(|page| {
            (page.inside && matches!(action, SettingsAction::Enter))
                .then(|| page.section().items.get(page.row).map(|item| item.id))
                .flatten()
        });
        if let Some(crate::settings::SettingId::Binding(_)) = opens_menu {
            if let Some(page) = self.settings.as_mut() {
                page.capture = true;
            }
            return;
        }
        if let Some(id) = opens_menu.filter(|id| crate::settings::AiPrefs::is_text(*id)) {
            self.begin_text_setting(id);
            return;
        }
        if let Some(id) = opens_menu {
            self.open_choice_menu(id);
            return;
        }
        let Some(page) = self.settings.as_mut() else {
            return;
        };
        let close = match action {
            SettingsAction::Up => {
                page.move_cursor(-1);
                false
            }
            SettingsAction::Down => {
                page.move_cursor(1);
                false
            }
            SettingsAction::Enter => {
                page.enter();
                false
            }
            SettingsAction::Back => page.back(),
            SettingsAction::First => {
                page.first();
                false
            }
            SettingsAction::Last => {
                page.last();
                false
            }
            SettingsAction::Reset | SettingsAction::Assign(_) => false,
            SettingsAction::Insert(_) | SettingsAction::DeleteChar => false,
        };
        if close {
            self.settings = None;
        }
    }

    fn open_choice_menu(&mut self, id: crate::settings::SettingId) {
        let Some((index, origin, count)) = self.choice_state(id) else {
            return;
        };
        if count == 0 {
            return;
        }
        if let Some(page) = self.settings.as_mut() {
            page.menu = Some(crate::settings::ChoiceMenu::new(index, origin));
        }
    }

    /// `Up` / `Down` preview a theme. `Enter` keeps it and saves. `Back` restores
    /// the theme that was selected when the list opened.
    fn handle_choice_menu(&mut self, action: SettingsAction) {
        let (id, index, origin) = {
            let Some(page) = self.settings.as_ref() else {
                return;
            };
            let Some(menu) = page.menu.as_ref() else {
                return;
            };
            let Some(item) = page.section().items.get(page.row) else {
                return;
            };
            (item.id, menu.index, menu.origin().to_string())
        };
        let len = self.choice_state(id).map(|(_, _, len)| len).unwrap_or(0);
        if len == 0 {
            if let Some(page) = self.settings.as_mut() {
                page.menu = None;
            }
            return;
        }
        match action {
            SettingsAction::Up => self.preview_choice(id, wrap_index(index, len, -1)),
            SettingsAction::Down => self.preview_choice(id, wrap_index(index, len, 1)),
            SettingsAction::First => self.preview_choice(id, 0),
            SettingsAction::Last => self.preview_choice(id, len - 1),
            SettingsAction::Enter => {
                let current = self.current_choice_id(id);
                if let Some(page) = self.settings.as_mut() {
                    page.menu = None;
                }
                if current != origin {
                    self.queue_settings_save();
                    self.lsp_prefs_dirty = true;
                }
            }
            SettingsAction::Back => {
                self.restore_choice(id, &origin);
                if let Some(page) = self.settings.as_mut() {
                    page.menu = None;
                }
            }
            SettingsAction::Reset | SettingsAction::Assign(_) => {}
            SettingsAction::Insert(_) | SettingsAction::DeleteChar => {}
        }
    }

    fn edit_text_setting(&mut self, action: SettingsAction) {
        match action {
            SettingsAction::Insert(text) => {
                if let Some(draft) = self.settings.as_mut().and_then(|page| page.text.as_mut()) {
                    let room = 512usize.saturating_sub(draft.value.chars().count());
                    draft.value.extend(text.chars().take(room));
                }
            }
            SettingsAction::DeleteChar => {
                if let Some(draft) = self.settings.as_mut().and_then(|page| page.text.as_mut()) {
                    draft.value.pop();
                }
            }
            SettingsAction::Enter => self.commit_text_setting(),
            SettingsAction::Back => {
                if let Some(page) = self.settings.as_mut() {
                    page.text = None;
                }
            }
            _ => {}
        }
    }

    fn begin_text_setting(&mut self, id: crate::settings::SettingId) {
        use crate::settings::SettingId;
        let value = match id {
            SettingId::AiServer => self.ai.endpoint(),
            SettingId::AiModel => self.ai.model.clone(),
            SettingId::AiApiKey => self.ai.api_key.clone(),
            SettingId::AiContextSize => {
                if self.ai.context_size.trim().is_empty() {
                    self.ai.resolved_context_size().to_string()
                } else {
                    self.ai.context_size.clone()
                }
            }
            _ => return,
        };
        if let Some(page) = self.settings.as_mut() {
            page.text = Some(crate::settings::TextDraft::new(value));
        }
    }

    fn commit_text_setting(&mut self) {
        use crate::settings::SettingId;
        let Some(value) = self.settings.as_ref().and_then(|page| {
            page.text
                .as_ref()
                .map(|draft| draft.value.trim().to_string())
        }) else {
            return;
        };
        let id = self
            .settings
            .as_ref()
            .and_then(|page| page.section().items.get(page.row).map(|item| item.id));
        let changed = match id {
            Some(SettingId::AiServer) => {
                let default = self.ai.provider.default_server();
                let stored = if value == default {
                    String::new()
                } else {
                    value
                };
                let changed = stored != self.ai.server;
                self.ai.server = stored;
                changed
            }
            Some(SettingId::AiModel) => {
                let changed = value != self.ai.model;
                self.ai.model = value;
                changed
            }
            Some(SettingId::AiApiKey) => {
                let changed = value != self.ai.api_key;
                self.ai.api_key = value;
                changed
            }
            Some(SettingId::AiContextSize) => {
                let stored = if value.is_empty() {
                    "16384".to_string()
                } else {
                    value
                };
                let changed = stored != self.ai.context_size;
                self.ai.context_size = stored;
                changed
            }
            _ => false,
        };
        if let Some(page) = self.settings.as_mut() {
            page.text = None;
        }
        if changed {
            self.queue_settings_save();
        }
    }

    fn preview_choice(&mut self, id: crate::settings::SettingId, index: usize) {
        use crate::settings::SettingId;
        match id {
            SettingId::ColorTheme => {
                self.appearance.select_color_index(index);
            }
            SettingId::SyntaxTheme => {
                self.appearance.select_syntax_index(index);
            }
            SettingId::TabWidth | SettingId::IndentWithSpaces | SettingId::LineEnding => {
                self.editor_prefs.set_choice(id, index);
                self.apply_editor_prefs();
            }
            SettingId::TrimTrailingSpace | SettingId::FinalNewline => {
                self.file_prefs.set_choice(id, index);
            }
            id @ (SettingId::LanguageServer
            | SettingId::CheckOnSave
            | SettingId::FormatOnSave
            | SettingId::PythonChecking) => {
                self.appearance.lsp.set_choice(id, index);
            }
            id @ (SettingId::AiEnabled | SettingId::AiAutoApprove | SettingId::AiStreaming | SettingId::AiProvider) => {
                self.ai.set_choice(id, index);
            }
            SettingId::AiServer | SettingId::AiModel | SettingId::AiApiKey | SettingId::AiContextSize | SettingId::Binding(_) => return,
        }
        if let Some(menu) = self.settings.as_mut().and_then(|page| page.menu.as_mut()) {
            menu.index = index;
        }
    }

    fn restore_choice(&mut self, id: crate::settings::SettingId, origin: &str) {
        use crate::settings::SettingId;
        match id {
            SettingId::ColorTheme => {
                self.appearance.select_color_id(origin);
            }
            SettingId::SyntaxTheme => {
                self.appearance.select_syntax_id(origin);
            }
            SettingId::TabWidth | SettingId::IndentWithSpaces | SettingId::LineEnding => {
                self.editor_prefs.restore_choice(id, origin);
                self.apply_editor_prefs();
            }
            SettingId::TrimTrailingSpace | SettingId::FinalNewline => {
                self.file_prefs.restore_choice(id, origin);
            }
            id @ (SettingId::LanguageServer
            | SettingId::CheckOnSave
            | SettingId::FormatOnSave
            | SettingId::PythonChecking) => {
                self.appearance.lsp.restore_choice(id, origin);
            }
            id @ (SettingId::AiEnabled | SettingId::AiAutoApprove | SettingId::AiStreaming | SettingId::AiProvider) => {
                self.ai.restore_choice(id, origin);
            }
            SettingId::AiServer | SettingId::AiModel | SettingId::AiApiKey | SettingId::AiContextSize | SettingId::Binding(_) => {}
        }
    }

    fn current_choice_id(&self, id: crate::settings::SettingId) -> String {
        use crate::settings::SettingId;
        match id {
            SettingId::ColorTheme => self.appearance.color.id.clone(),
            SettingId::SyntaxTheme => self.appearance.syntax.id.clone(),
            SettingId::TabWidth | SettingId::IndentWithSpaces | SettingId::LineEnding => {
                self.editor_prefs.choice_token(id)
            }
            SettingId::TrimTrailingSpace | SettingId::FinalNewline => {
                self.file_prefs.choice_token(id)
            }
            id @ (SettingId::LanguageServer
            | SettingId::CheckOnSave
            | SettingId::FormatOnSave
            | SettingId::PythonChecking) => self.appearance.lsp.choice_token(id),
            id @ (SettingId::AiEnabled | SettingId::AiAutoApprove | SettingId::AiStreaming | SettingId::AiProvider) => self.ai.choice_token(id),
            SettingId::AiServer | SettingId::AiModel | SettingId::AiApiKey | SettingId::AiContextSize | SettingId::Binding(_) => String::new(),
        }
    }

    fn choice_state(&self, id: crate::settings::SettingId) -> Option<(usize, String, usize)> {
        use crate::settings::{AiPrefs, EditorPrefs, FilePrefs, LspPrefs, SettingId};
        let (index, token, len) = match id {
            SettingId::ColorTheme => (
                self.appearance.color_index(),
                self.appearance.color.id.clone(),
                self.appearance.color_count(),
            ),
            SettingId::SyntaxTheme => (
                self.appearance.syntax_index(),
                self.appearance.syntax.id.clone(),
                self.appearance.syntax_count(),
            ),
            SettingId::TabWidth | SettingId::IndentWithSpaces | SettingId::LineEnding => (
                self.editor_prefs.choice_index(id),
                self.editor_prefs.choice_token(id),
                EditorPrefs::choice_names(id).len(),
            ),
            SettingId::TrimTrailingSpace | SettingId::FinalNewline => (
                self.file_prefs.choice_index(id),
                self.file_prefs.choice_token(id),
                FilePrefs::choice_names(id).len(),
            ),
            id @ (SettingId::LanguageServer
            | SettingId::CheckOnSave
            | SettingId::FormatOnSave
            | SettingId::PythonChecking) => (
                self.appearance.lsp.choice_index(id),
                self.appearance.lsp.choice_token(id),
                LspPrefs::choice_names(id).len(),
            ),
            id @ (SettingId::AiEnabled | SettingId::AiAutoApprove | SettingId::AiStreaming | SettingId::AiProvider) => (
                self.ai.choice_index(id),
                self.ai.choice_token(id),
                AiPrefs::choice_names(id).len(),
            ),
            SettingId::AiServer | SettingId::AiModel | SettingId::AiApiKey | SettingId::AiContextSize | SettingId::Binding(_) => return None,
        };
        Some((index, token, len))
    }

    fn apply_editor_prefs(&mut self) {
        let spaces = self.editor_prefs.indent_with_spaces;
        let width = usize::from(self.editor_prefs.tab_width);
        let forced = self.editor_prefs.line_ending.forced();
        for editor in &mut self.editors {
            for tab in &mut editor.tabs {
                tab.buffer.apply_editor_style(spaces, width, forced);
            }
        }
    }

    fn style_buffer(&self, mut buffer: crate::buffer::Buffer) -> crate::buffer::Buffer {
        buffer.apply_editor_style(
            self.editor_prefs.indent_with_spaces,
            usize::from(self.editor_prefs.tab_width),
            self.editor_prefs.line_ending.forced(),
        );
        buffer
    }

    fn reset_highlighted(&mut self) {
        let Some(page) = self.settings.clone() else {
            return;
        };
        if let Some(page) = self.settings.as_mut() {
            page.capture = false;
            page.menu = None;
            page.text = None;
        }
        if page.inside {
            if let Some(item) = page.section().items.get(page.row) {
                self.reset_one(item.id);
                self.message = Some(format!("Restored {}", item.label));
            }
        } else {
            let name = page.section().name;
            let ids: Vec<_> = page.section().items.iter().map(|item| item.id).collect();
            for id in ids {
                self.reset_one(id);
            }
            self.message = Some(format!("Restored {name}"));
        }
        self.keys = crate::keys::KeyMap::resolve(&self.key_overrides);
        self.apply_editor_prefs();
        self.queue_settings_save();
        self.lsp_prefs_dirty = true;
    }

    fn reset_one(&mut self, id: crate::settings::SettingId) {
        use crate::settings::{PythonCheck, SettingId};
        match id {
            SettingId::TabWidth | SettingId::IndentWithSpaces | SettingId::LineEnding => {
                self.editor_prefs.reset(id);
            }
            SettingId::TrimTrailingSpace | SettingId::FinalNewline => self.file_prefs.reset(id),
            SettingId::ColorTheme => {
                self.appearance.select_color_id("tokyo-night");
            }
            SettingId::SyntaxTheme => {
                self.appearance.select_syntax_id("tokyo-night");
            }
            SettingId::Binding(id) => {
                self.key_overrides.remove(id);
            }
            SettingId::LanguageServer => self.appearance.lsp.enabled = true,
            SettingId::CheckOnSave => self.appearance.lsp.check_on_save = true,
            SettingId::FormatOnSave => self.appearance.lsp.format_on_save = false,
            SettingId::PythonChecking => self.appearance.lsp.python = PythonCheck::Basic,
            id @ (SettingId::AiEnabled
            | SettingId::AiAutoApprove
            | SettingId::AiStreaming
            | SettingId::AiProvider
            | SettingId::AiServer
            | SettingId::AiModel
            | SettingId::AiApiKey
            | SettingId::AiContextSize) => self.ai.reset(id),
        }
    }

    fn assign_highlighted(&mut self, token: &str) {
        let Some(id) = self.settings.as_ref().and_then(|page| {
            page.section()
                .items
                .get(page.row)
                .and_then(|item| match item.id {
                    crate::settings::SettingId::Binding(id) => Some(id),
                    _ => None,
                })
        }) else {
            return;
        };
        let label = crate::keys::KeyMap::label(id);
        match crate::settings::assign_binding(&mut self.key_overrides, id, token) {
            Ok(shown) => {
                if let Some(page) = self.settings.as_mut() {
                    page.capture = false;
                }
                self.keys = crate::keys::KeyMap::resolve(&self.key_overrides);
                self.queue_settings_save();
                self.message = Some(format!("{label} is {shown}"));
            }
            Err(message) => self.message = Some(message),
        }
    }

    fn handle_command_bar(&mut self, action: CommandBarAction) {
        if matches!(action, CommandBarAction::ToChat) {
            self.command_bar = None;
            self.cancel_resume();
            self.focus_agent();
            return;
        }
        let ghost = if matches!(action, CommandBarAction::AcceptHint) {
            self.argument_ghost()
        } else {
            None
        };
        let Some(context) = self.command_bar.as_ref().map(|bar| bar.context) else {
            return;
        };
        let files = self.command_bar.as_ref().is_some_and(|bar| bar.files);
        let recent = self.recent_for(context);
        let folders = self.workspace_choices();
        let listing = folders.is_some();
        let Some(bar) = self.command_bar.as_mut() else {
            return;
        };
        let count = folders
            .as_ref()
            .map(|rows| rows.len())
            .unwrap_or_else(|| suggestions(&bar.input, recent, context, files).len());
        match action {
            CommandBarAction::Insert(text) => {
                bar.input.push_str(&text);
                bar.selected = 0;
                bar.prompted = None;
            }
            CommandBarAction::Backspace => {
                bar.input.pop();
                bar.selected = 0;
                bar.prompted = None;
            }
            CommandBarAction::SelectNext if count > 0 => bar.selected = (bar.selected + 1) % count,
            CommandBarAction::SelectPrev if count > 0 => {
                bar.selected = (bar.selected + count - 1) % count;
            }
            CommandBarAction::SelectNext | CommandBarAction::SelectPrev => {}
            CommandBarAction::Complete
                if listing
                    || (context == CommandContext::Editor && is_line_number(bar.query())) => {}
            CommandBarAction::Complete => {
                if let Some(spec) = bar.selected(recent) {
                    bar.input = completed_input(spec);
                    bar.selected = 0;
                    bar.prompted = None;
                }
            }
            CommandBarAction::AcceptHint => {
                if let Some(ghost) = ghost {
                    bar.input.push_str(&ghost);
                    bar.selected = 0;
                    bar.prompted = None;
                }
            }
            CommandBarAction::Submit => self.submit_command(),
            CommandBarAction::Close => {
                self.command_bar = None;
                self.cancel_resume();
            }
            CommandBarAction::ToChat => {}
        }
    }

    /// Dim remainder of a file or folder name, drawn after the typed argument.
    /// Names come from the explorer's scanned tree, so nothing is read from disk here.
    pub fn argument_ghost(&self) -> Option<String> {
        let bar = self.command_bar.as_ref()?;
        let spec = find_command(&bar.query().to_lowercase(), bar.context, bar.files)?;
        let source = name_source(spec.id)?;
        let typed = bar.argument()?;
        ghost_suffix(typed, &self.name_candidates(source))
    }

    /// Saved folders while `workspace` is waiting for a path. `None` for every other command.
    pub fn workspace_choices(&self) -> Option<Vec<String>> {
        let bar = self.command_bar.as_ref()?;
        let spec = find_command(&bar.query().to_lowercase(), bar.context, bar.files)?;
        if spec.id != CommandId::Workspace || bar.argument_slot().is_none() {
            return None;
        }
        let typed = bar.argument().unwrap_or("");
        Some(
            self.name_candidates(NameSource::Workspace)
                .into_iter()
                .filter(|path| typed.is_empty() || name_prefix(typed, path))
                .collect(),
        )
    }

    fn name_candidates(&self, source: NameSource) -> Vec<String> {
        let entries = self.explorer.entries();
        match source {
            NameSource::Directory => entries
                .iter()
                .filter(|entry| entry.is_dir)
                .map(|entry| display_rel(&entry.path))
                .collect(),
            NameSource::Path => entries
                .iter()
                .map(|entry| display_rel(&entry.path))
                .collect(),
            NameSource::Workspace => self
                .sessions
                .iter()
                .filter(|session| {
                    !crate::session::same_directory(&session.root, &self.explorer.root)
                })
                .map(|session| session.root.display().to_string())
                .collect(),
            NameSource::Sibling => {
                let Some(selected) = self.explorer.selected() else {
                    return Vec::new();
                };
                let parent = selected.parent().unwrap_or(Path::new(""));
                entries
                    .iter()
                    .filter(|entry| entry.path.parent().unwrap_or(Path::new("")) == parent)
                    .map(|entry| entry.name.clone())
                    .collect()
            }
        }
    }

    fn focus_editor(&mut self) {
        if self.explorer.focused {
            self.explorer.blur();
            self.end_file_mode();
        }
    }

    fn find_query(&self) -> Result<String, String> {
        self.active_editor()
            .tab()
            .find_query
            .clone()
            .filter(|query| !query.is_empty())
            .ok_or_else(|| "Nothing to find".to_string())
    }

    /// True while explorer FIND results are on screen. The badge reads `FILTER`.
    pub fn filtering(&self) -> bool {
        self.file_find.is_some()
    }

    /// True while REPLACE is aimed at the files in FILTER mode.
    pub fn replacing_files(&self) -> bool {
        self.replace_mode
            .as_ref()
            .is_some_and(|mode| mode.searches_files())
    }

    /// True while the FIND badge is collecting a file search.
    pub fn finding_files(&self) -> bool {
        self.find_mode
            .as_ref()
            .is_some_and(|mode| mode.searches_files())
    }

    fn open_find_mode(&mut self, query: &str) {
        if self.explorer.open && self.explorer.focused {
            self.open_file_find(query);
            if !query.is_empty() {
                self.submit_file_find();
            }
            return;
        }
        let query = if query.is_empty() {
            self.active_editor()
                .tab()
                .find_query
                .clone()
                .unwrap_or_default()
        } else {
            query.to_string()
        };
        self.focus_editor();
        self.command_bar = None;
        self.replace_mode = None;
        self.find_mode = Some(FindMode {
            query: query.clone(),
            scope: FindScope::Buffer,
        });
        self.live_query(&query);
    }

    fn open_file_find(&mut self, query: &str) {
        let query = if query.is_empty() {
            self.file_find_query.clone()
        } else {
            query.to_string()
        };
        if !query.is_empty() {
            self.file_find_query = query.clone();
        }
        self.command_bar = None;
        self.replace_mode = None;
        self.find_mode = Some(FindMode {
            query,
            scope: FindScope::Files,
        });
    }

    fn handle_find(&mut self, action: FindAction) {
        match action {
            FindAction::Close => {
                let files = self.finding_files();
                self.find_mode = None;
                if files {
                    self.exit_file_find();
                }
            }
            FindAction::Submit => {
                if self.finding_files() {
                    self.submit_file_find();
                } else {
                    self.find_mode = None;
                }
            }
            FindAction::Files => self.show_file_find(),
            FindAction::Insert(text) => {
                let Some((query, files)) = self.push_find_text(&text) else {
                    return;
                };
                if files {
                    if !query.is_empty() {
                        self.file_find_query = query;
                    }
                } else {
                    self.live_query(&query);
                }
            }
            FindAction::Backspace => {
                let Some((query, files)) = self.pop_find_text() else {
                    return;
                };
                if !files {
                    self.live_query(&query);
                }
            }
            FindAction::Next => {
                if self.finding_files() {
                    return;
                }
                if let Err(message) = self.find_adjacent(true) {
                    self.message = Some(message);
                }
            }
            FindAction::Prev => {
                if self.finding_files() {
                    return;
                }
                if let Err(message) = self.find_adjacent(false) {
                    self.message = Some(message);
                }
            }
        }
    }

    fn push_find_text(&mut self, text: &str) -> Option<(String, bool)> {
        let mode = self.find_mode.as_mut()?;
        mode.query.push_str(text);
        Some((mode.query.clone(), mode.searches_files()))
    }

    fn pop_find_text(&mut self) -> Option<(String, bool)> {
        let mode = self.find_mode.as_mut()?;
        mode.query.pop();
        Some((mode.query.clone(), mode.searches_files()))
    }

    fn submit_file_find(&mut self) {
        let Some(query) = self.find_mode.as_ref().map(|mode| mode.query.clone()) else {
            return;
        };
        if query.is_empty() {
            self.message = Some("Nothing to find".to_string());
            return;
        }
        self.file_find_query = query.clone();
        self.file_find_generation = self.file_find_generation.wrapping_add(1);
        let generation = self.file_find_generation;
        let texts = self.open_file_ropes();
        let files = self
            .explorer
            .entries()
            .iter()
            .filter(|entry| !entry.is_dir)
            .map(|entry| (entry.path.clone(), texts.get(&entry.path).cloned()))
            .collect();
        self.file_find = Some(FileFind {
            query: query.clone(),
            generation,
        });
        self.explorer.clear_file_search();
        self.explorer.file_query = Some(query.clone());
        self.find_mode = None;
        self.explorer.open = true;
        self.explorer.focused = true;
        self.jobs.push(Job::FindFiles {
            root: self.explorer.root.clone(),
            query,
            generation,
            files,
        });
        self.message = Some("Searching…".to_string());
    }

    fn show_file_find(&mut self) {
        if self.file_find.is_none() {
            return;
        }
        self.find_mode = None;
        self.replace_mode = None;
        self.command_bar = None;
        self.explorer.open = true;
        self.explorer.focused = true;
    }

    fn exit_file_find(&mut self) {
        self.file_find = None;
        self.reveal_find = None;
        self.find_mode = None;
        self.replace_notice = None;
        self.explorer.clear_marks();
        self.explorer.clear_file_search();
    }

    fn finish_file_find(&mut self, generation: u64, hits: Vec<(PathBuf, usize)>) {
        let Some(search) = &self.file_find else {
            return;
        };
        if search.generation != generation {
            return;
        }
        let matches: usize = hits.iter().map(|(_, count)| *count).sum();
        let files = hits.len();
        self.explorer.show_file_hits(hits.into_iter().collect());
        if let Some(notice) = self.replace_notice.take() {
            self.message = Some(notice);
            return;
        }
        self.message = Some(if files == 0 {
            "Not found".to_string()
        } else {
            let match_word = if matches == 1 { "match" } else { "matches" };
            let file_word = if files == 1 { "file" } else { "files" };
            format!("{matches} {match_word} in {files} {file_word}")
        });
    }

    fn refresh_file_find(&mut self) {
        let Some(query) = self.file_find.as_ref().map(|search| search.query.clone()) else {
            return;
        };
        self.file_find_generation = self.file_find_generation.wrapping_add(1);
        let generation = self.file_find_generation;
        let texts = self.open_file_ropes();
        let files = self
            .explorer
            .entries()
            .iter()
            .filter(|entry| !entry.is_dir)
            .map(|entry| (entry.path.clone(), texts.get(&entry.path).cloned()))
            .collect();
        if let Some(search) = self.file_find.as_mut() {
            search.generation = generation;
        }
        self.explorer.file_query = Some(query.clone());
        self.jobs.push(Job::FindFiles {
            root: self.explorer.root.clone(),
            query,
            generation,
            files,
        });
    }

    /// Open buffers keyed by path relative to the explorer root.
    /// Rope clones are cheap; the FindFiles worker stringifies or scans chunks.
    /// Includes every open file so project search matches the buffer (dirty or not).
    fn open_file_ropes(&self) -> HashMap<PathBuf, Rope> {
        let mut texts = HashMap::new();
        for editor in &self.editors {
            for tab in editor.tabs() {
                let Some(path) = &tab.path else {
                    continue;
                };
                let Some(rel) = self.relative_to_root(path) else {
                    continue;
                };
                texts.insert(rel, tab.buffer.rope().clone());
            }
        }
        texts
    }

    /// The same folder, including when one path is `E:\Work\app` and the other differs by case or a `\\?\` prefix that `absolute` folds.
    fn same_root(&self, root: &Path) -> bool {
        root == self.explorer.root || crate::session::same_directory(root, &self.explorer.root)
    }

    fn relative_to_root(&self, path: &Path) -> Option<PathBuf> {
        relative_to(&self.explorer.root, path)
    }

    /// Selects the first match of `query`. An empty query leaves the cursor where it is.
    fn live_query(&mut self, query: &str) {
        if query.is_empty() {
            self.message = None;
            return;
        }
        self.active_editor_mut().tab_mut().find_query = Some(query.to_string());
        if let Err(message) = self.seek_match(MatchSeek::First) {
            self.message = Some(message);
        }
    }

    fn find_adjacent(&mut self, forward: bool) -> Result<(), String> {
        self.focus_editor();
        if let Some(query) = self.find_mode.as_ref().map(|mode| mode.query.clone()) {
            if query.is_empty() {
                return Err("Nothing to find".to_string());
            }
            self.active_editor_mut().tab_mut().find_query = Some(query);
        }
        self.seek_match(if forward {
            MatchSeek::Next
        } else {
            MatchSeek::Prev
        })
    }

    fn seek_match(&mut self, seek: MatchSeek) -> Result<(), String> {
        let query = self.find_query()?;
        let spans = self.buffer().find_spans(&query);
        if spans.is_empty() {
            return Err("Not found".to_string());
        }
        let index = match seek {
            MatchSeek::First => 0,
            MatchSeek::Next => {
                let origin = self.match_origin(&query, true);
                spans
                    .iter()
                    .position(|(start, _)| *start >= origin)
                    .unwrap_or(0)
            }
            MatchSeek::Prev => {
                let origin = self.match_origin(&query, false);
                spans
                    .iter()
                    .rposition(|(start, _)| *start < origin)
                    .unwrap_or(spans.len() - 1)
            }
        };
        let (start, end) = spans[index];
        self.buffer_mut().select_span(start, end);
        self.message = Some(format!("{} of {}", index + 1, spans.len()));
        Ok(())
    }

    /// End of the selected match when moving forward, start when moving backward.
    /// A caret that is not on the search text uses the cursor itself.
    fn match_origin(&self, query: &str, forward: bool) -> usize {
        let buffer = self.buffer();
        let sel = buffer.selection();
        if !sel.is_empty() && buffer.selected_text() == query {
            if forward { sel.to() } else { sel.from() }
        } else {
            sel.head
        }
    }

    fn replace_query(&mut self, replacement: &str, all: bool) -> Result<(), String> {
        self.focus_editor();
        let query = self.find_query()?;
        if all {
            let count = self.buffer().find_spans(&query).len();
            if count == 0 {
                return Err("Not found".to_string());
            }
            let replacement = replacement.to_string();
            self.edit(|buffer| {
                buffer.replace_all(&query, &replacement);
            });
            self.message = Some(format!("Replaced {count}"));
            return Ok(());
        }

        let spans = self.buffer().find_spans(&query);
        if spans.is_empty() {
            return Err("Not found".to_string());
        }
        let index = self.replace_index(&query, &spans);
        let (start, end) = spans[index];
        let replacement = replacement.to_string();
        self.edit(|buffer| buffer.replace_span(start, end, &replacement));
        let cursor = start + replacement.chars().count();
        let spans = self.buffer().find_spans(&query);
        if spans.is_empty() {
            self.message = Some("Replaced 1".to_string());
            return Ok(());
        }
        let (index, (start, end)) = spans
            .iter()
            .enumerate()
            .find(|(_, (start, _))| *start >= cursor)
            .map(|(index, span)| (index, *span))
            .unwrap_or((0, spans[0]));
        self.buffer_mut().select_span(start, end);
        self.message = Some(format!("Replaced 1, {} of {}", index + 1, spans.len()));
        Ok(())
    }

    /// The selected match when it is the search text, otherwise the match that
    /// contains the cursor, otherwise the next match at or after the cursor.
    fn replace_index(&self, query: &str, spans: &[(usize, usize)]) -> usize {
        let buffer = self.buffer();
        let sel = buffer.selection();
        if !sel.is_empty()
            && buffer.selected_text() == query
            && let Some(index) = spans
                .iter()
                .position(|(start, end)| *start == sel.from() && *end == sel.to())
        {
            return index;
        }
        let head = sel.head;
        if let Some(index) = spans
            .iter()
            .position(|(start, end)| *start <= head && head < *end)
        {
            return index;
        }
        spans
            .iter()
            .position(|(start, _)| *start >= head)
            .unwrap_or(0)
    }

    /// How many columns of the selected text are shown before `...`.
    const REPLACE_PREVIEW: usize = 24;

    fn open_replace_mode(&mut self, replacement: &str) {
        if self.explorer.open && self.explorer.focused {
            if self.filtering() {
                self.open_file_replace(replacement);
            } else {
                self.message = Some("replace applies in FILTER mode".to_string());
            }
            return;
        }
        let replacement = if replacement.is_empty() {
            self.active_editor().tab().replace_text.clone()
        } else {
            replacement.to_string()
        };
        self.focus_editor();
        self.command_bar = None;
        self.find_mode = None;
        let selected = self.buffer().selected_text();
        let needle = if !selected.is_empty() {
            selected
        } else {
            self.active_editor()
                .tab()
                .find_query
                .clone()
                .unwrap_or_default()
        };
        if !needle.is_empty() {
            self.active_editor_mut().tab_mut().find_query = Some(needle.clone());
            if self.buffer().selected_text() != needle {
                let spans = self.buffer().find_spans(&needle);
                if !spans.is_empty() {
                    let index = self.replace_index(&needle, &spans);
                    let (start, end) = spans[index];
                    self.buffer_mut().select_span(start, end);
                }
            }
            let spans = self.buffer().find_spans(&needle);
            if spans.is_empty() {
                self.message = Some("Not found".to_string());
            } else {
                let index = self.replace_index(&needle, &spans);
                self.message = Some(format!("{} of {}", index + 1, spans.len()));
            }
        }
        self.replace_mode = Some(ReplaceMode {
            needle,
            replacement,
            undos: 0,
            scope: ReplaceScope::Buffer,
        });
        self.remember_replacement();
    }

    fn open_file_replace(&mut self, replacement: &str) {
        if !self.explorer.has_file_hits() {
            self.message = Some("Still searching".to_string());
            return;
        }
        let Some(needle) = self.file_find.as_ref().map(|search| search.query.clone()) else {
            self.message = Some("Nothing to replace".to_string());
            return;
        };
        let replacement = if replacement.is_empty() {
            self.file_replace_text.clone()
        } else {
            replacement.to_string()
        };
        self.file_replace_text = replacement.clone();
        self.command_bar = None;
        self.find_mode = None;
        self.replace_mode = Some(ReplaceMode {
            needle,
            replacement,
            undos: 0,
            scope: ReplaceScope::Files,
        });
    }

    /// Keeps this tab's last replacement, including an empty one used to delete.
    fn remember_replacement(&mut self) {
        let Some(mode) = self.replace_mode.as_ref() else {
            return;
        };
        let text = mode.replacement.clone();
        if mode.searches_files() {
            self.file_replace_text = text;
        } else {
            self.active_editor_mut().tab_mut().replace_text = text;
        }
    }

    fn handle_replace(&mut self, action: ReplaceAction) {
        match action {
            ReplaceAction::Close => self.replace_mode = None,
            ReplaceAction::Submit => {
                if self.replacing_files() {
                    self.apply_file_replace();
                } else {
                    self.replace_mode = None;
                }
            }
            ReplaceAction::Insert(text) => {
                if let Some(mode) = self.replace_mode.as_mut() {
                    mode.replacement.push_str(&text);
                }
                self.remember_replacement();
            }
            ReplaceAction::Backspace => {
                if let Some(mode) = self.replace_mode.as_mut() {
                    mode.replacement.pop();
                }
                self.remember_replacement();
            }
            ReplaceAction::Undo => self.undo_replace(),
            ReplaceAction::Next => self.replace_from_mode(false),
            ReplaceAction::All => self.replace_from_mode(true),
        }
    }

    fn replace_from_mode(&mut self, all: bool) {
        let Some(mode) = self.replace_mode.clone() else {
            return;
        };
        if mode.searches_files() {
            self.apply_file_replace();
            return;
        }
        if mode.needle.is_empty() {
            self.message = Some("Nothing to replace".to_string());
            return;
        }
        self.active_editor_mut().tab_mut().find_query = Some(mode.needle.clone());
        match self.replace_query(&mode.replacement, all) {
            Ok(()) => {
                if let Some(mode) = self.replace_mode.as_mut() {
                    mode.undos += 1;
                }
            }
            Err(message) => self.message = Some(message),
        }
    }

    fn undo_replace(&mut self) {
        let pending = self
            .replace_mode
            .as_ref()
            .map(|mode| mode.undos)
            .unwrap_or(0);
        if pending == 0 {
            self.message = Some("Nothing to undo".to_string());
            return;
        }
        let undone = self.buffer_mut().undo();
        self.note_buffer_change();
        if undone {
            if let Some(mode) = self.replace_mode.as_mut() {
                mode.undos = mode.undos.saturating_sub(1);
            }
            self.message = Some("Undid replace".to_string());
        }
    }

    fn apply_file_replace(&mut self) {
        let Some(mode) = self.replace_mode.clone() else {
            return;
        };
        if !self.explorer.has_file_hits() {
            self.message = Some("Still searching".to_string());
            return;
        }
        let marked = self.explorer.marked_count() > 0;
        let paths = self.explorer.files_to_replace();
        if paths.is_empty() {
            self.message = Some(if marked {
                "Nothing selected matches".to_string()
            } else {
                "Nothing to replace".to_string()
            });
            return;
        }
        self.file_replace_text = mode.replacement.clone();
        let mut buffer_matches = 0;
        let mut buffer_files = 0;
        let mut disk = Vec::new();
        for rel in paths {
            match self.replace_open_file(&rel, &mode.needle, &mode.replacement) {
                Some(0) => {}
                Some(count) => {
                    buffer_matches += count;
                    buffer_files += 1;
                }
                None => disk.push(rel),
            }
        }
        self.replace_mode = None;
        if disk.is_empty() {
            self.replace_notice = Some(replaced_message(buffer_matches, buffer_files));
            self.refresh_file_find();
            return;
        }
        self.replace_buffers = (buffer_matches, buffer_files);
        self.file_replace_generation = self.file_replace_generation.wrapping_add(1);
        self.jobs.push(Job::ReplaceFiles {
            root: self.explorer.root.clone(),
            query: mode.needle,
            replacement: mode.replacement,
            generation: self.file_replace_generation,
            files: disk,
        });
        self.message = Some("Replacing…".to_string());
    }

    /// `Some` when `rel` is open in a tab. The count is how many matches changed.
    fn replace_open_file(&mut self, rel: &Path, query: &str, replacement: &str) -> Option<usize> {
        let abs = self.explorer.root.join(rel);
        let tab_id = self.tab_with_path(&abs)?;
        let tab = self.find_tab_mut(tab_id)?;
        let count = tab.buffer.replace_all(query, replacement);
        if count > 0 {
            tab.dirty = tab.buffer.fingerprint() != tab.saved_fingerprint;
        }
        self.sync_tab_syntax(tab_id);
        Some(count)
    }

    fn finish_file_replace(
        &mut self,
        generation: u64,
        matches: usize,
        files: usize,
        errors: Vec<String>,
    ) {
        if generation != self.file_replace_generation {
            return;
        }
        let (buffer_matches, buffer_files) = self.replace_buffers;
        self.replace_buffers = (0, 0);
        let mut message = replaced_message(buffer_matches + matches, buffer_files + files);
        if let Some(err) = errors.first() {
            message = format!("{message}. {err}");
        }
        self.replace_notice = Some(message);
        self.refresh_file_find();
    }

    /// Selected text and the replacement being typed. The selected text is cut
    /// off with `...` when it is long. The arrow is always between them.
    pub fn replace_parts(&self) -> Option<(String, String)> {
        let mode = self.replace_mode.as_ref()?;
        let selected = self.buffer().selected_text();
        let source = if mode.searches_files() || selected.is_empty() {
            mode.needle.as_str()
        } else {
            selected.as_str()
        };
        Some((
            truncate_end(source, Self::REPLACE_PREVIEW),
            mode.replacement.clone(),
        ))
    }

    /// `selected → new`, ready to type even when the new word is still empty.
    pub fn replace_field(&self) -> String {
        self.replace_parts()
            .map(|(head, replacement)| format!("{head} → {replacement}"))
            .unwrap_or_default()
    }

    fn open_command_bar(&mut self) {
        if self.file_picker.is_some() {
            self.close_picker();
        }
        self.find_mode = None;
        self.replace_mode = None;
        let context = if self.explorer.open && self.explorer.focused {
            CommandContext::Explorer
        } else {
            CommandContext::Editor
        };
        self.command_bar = Some(CommandBar {
            context,
            files: context == CommandContext::Explorer && self.explorer.selecting,
            ..CommandBar::default()
        });
    }

    /// An exact name match wins over the highlighted suggestion, so `goto 42` works without arrowing.
    /// A bare number is shorthand for `goto <number>` while an editor is focused.
    /// Commands that belong to the other surface are refused instead of being hinted.
    fn submit_command(&mut self) {
        let Some(bar) = self.command_bar.as_ref() else {
            return;
        };
        let context = bar.context;
        let input = bar.input.trim().to_string();
        let name = bar.query().to_lowercase();
        let arg = bar.argument_slot().map(str::to_string);
        let files = bar.files;
        let recent = self.recent_for(context);
        let highlighted = bar.selected(recent).map(|spec| spec.id);

        if context == CommandContext::Editor && is_line_number(&input) {
            self.command_bar = None;
            self.finish_command(spec(CommandId::GotoLine), Some(input.trim()), context);
            return;
        }

        if let Some(spec) = find_command(&name, context, files) {
            if spec.id == CommandId::Workspace {
                self.submit_workspace(arg, context);
                return;
            }
            self.dispatch_spec(spec, arg, context);
            return;
        }
        if !name.is_empty() && is_known_name(&name) {
            self.message = Some(unavailable_reason(&name, context));
            return;
        }
        if let Some(id) = highlighted {
            self.dispatch_spec(spec(id), arg, context);
            return;
        }
        self.message = Some(format!("Unknown command: {name}"));
    }

    /// `workspace` with no path lists saved folders. A typed path opens that folder.
    fn submit_workspace(&mut self, arg: Option<String>, context: CommandContext) {
        let Some(arg) = arg else {
            if let Some(bar) = self.command_bar.as_mut() {
                bar.input = "workspace ".to_string();
                bar.selected = 0;
                bar.prompted = Some(CommandId::Workspace);
            }
            return;
        };
        let chosen = if arg.is_empty() {
            let rows = self.workspace_choices().unwrap_or_default();
            let index = self
                .command_bar
                .as_ref()
                .map(|bar| bar.selected)
                .unwrap_or(0);
            match rows.get(index.min(rows.len().saturating_sub(1))) {
                Some(path) if !rows.is_empty() => path.clone(),
                _ => {
                    self.message = Some("Enter a folder".to_string());
                    return;
                }
            }
        } else {
            arg
        };
        self.command_bar = None;
        self.finish_command(spec(CommandId::Workspace), Some(&chosen), context);
    }

    fn dispatch_spec(&mut self, spec: &CommandSpec, arg: Option<String>, context: CommandContext) {
        if spec.arg.is_some() && arg.as_ref().is_none_or(|text| text.is_empty()) {
            if let Some(bar) = self.command_bar.as_mut() {
                bar.input = completed_input(spec);
                bar.selected = 0;
                bar.prompted = Some(spec.id);
            }
            return;
        }
        self.command_bar = None;
        self.finish_command(spec, arg.as_deref(), context);
    }

    fn finish_command(&mut self, spec: &CommandSpec, arg: Option<&str>, context: CommandContext) {
        if spec.id != CommandId::Save {
            self.cancel_resume();
        }
        match self.run_command(spec.id, arg) {
            Ok(()) => match context {
                CommandContext::Editor => self.recent_command = Some(spec.id),
                CommandContext::Explorer => self.recent_explorer = Some(spec.id),
            },
            Err(message) => self.message = Some(message),
        }
    }

    fn run_command(&mut self, id: CommandId, arg: Option<&str>) -> Result<(), String> {
        if crate::command::manages_files(id) && !self.explorer.selecting {
            return Err(format!("{} applies in FILE mode", spec(id).name));
        }
        match id {
            CommandId::Quit => self.request_quit(),
            CommandId::Undo => self.edit(|buffer| {
                buffer.undo();
            }),
            CommandId::Redo => self.edit(|buffer| {
                buffer.redo();
            }),
            CommandId::SelectAll => self.buffer_mut().select_all(),
            CommandId::Indent => self.edit(|buffer| buffer.indent()),
            CommandId::Outdent => self.edit(|buffer| buffer.outdent()),
            CommandId::GotoLine => {
                let line = arg
                    .and_then(|a| a.parse::<usize>().ok())
                    .filter(|n| *n > 0)
                    .ok_or_else(|| format!("Invalid line number: {}", arg.unwrap_or("")))?;
                self.buffer_mut().goto_line(line - 1);
            }
            CommandId::Find => self.open_find_mode(arg.unwrap_or("")),
            CommandId::Replace => self.open_replace_mode(arg.unwrap_or("")),
            CommandId::ToggleExplorer => self.toggle_sidebar(),
            CommandId::Reveal => self.reveal_focused_file()?,
            CommandId::SidebarWider => self.resize_sidebar(true),
            CommandId::SidebarNarrower => self.resize_sidebar(false),
            CommandId::SplitRight => self.split(Axis::Horizontal),
            CommandId::SplitDown => self.split(Axis::Vertical),
            CommandId::Close => self.request_close()?,
            CommandId::CloseAll => self.request_close_batch(CloseBatch::All),
            CommandId::CloseView => self.request_close_batch(CloseBatch::View(self.active)),
            CommandId::TabPrev => self.cycle_tab(false),
            CommandId::TabNext => self.cycle_tab(true),
            CommandId::DiagnosticNext => self.goto_diagnostic(true),
            CommandId::DiagnosticPrev => self.goto_diagnostic(false),
            CommandId::Hover => self.show_hover(),
            CommandId::GotoDefinition => self.goto_definition(),
            CommandId::GotoReferences => self.goto_references(),
            CommandId::MatchBracket => self.match_bracket(false),
            CommandId::Complete => self.complete_now(),
            CommandId::Comment => self.toggle_comment(),
            CommandId::MoveLineUp => self.handle(SemanticAction::MoveLine { down: false }),
            CommandId::MoveLineDown => self.handle(SemanticAction::MoveLine { down: true }),
            CommandId::DeleteLine => self.handle(SemanticAction::DeleteLine),
            CommandId::DuplicateLineUp => {
                self.handle(SemanticAction::DuplicateLine { down: false })
            }
            CommandId::DuplicateLineDown => {
                self.handle(SemanticAction::DuplicateLine { down: true })
            }
            CommandId::RenameSymbol => self.rename_symbol(arg.unwrap_or("")),
            CommandId::Format => self.format_document(),
            CommandId::CodeAction => self.code_action(),
            CommandId::JumpBack => self.jump_back(),
            CommandId::JumpForward => self.jump_forward(),
            CommandId::Copy => self.copy()?,
            CommandId::Cut => self.cut()?,
            CommandId::Paste => self.paste()?,
            CommandId::Save => {
                let follow = self.resume.take();
                self.save_focused(arg.map(PathBuf::from), follow)?;
            }
            CommandId::Reload => self.reload_file(arg),
            CommandId::Encoding => self.set_encoding(arg),
            CommandId::Select => self.enter_select(),
            CommandId::Move => self.begin_move(arg.unwrap_or(""))?,
            CommandId::Delete => self.begin_delete()?,
            CommandId::Rename => self.begin_rename(arg.unwrap_or(""))?,
            CommandId::Settings => {
                self.close_picker();
                self.about = false;
                self.settings = Some(crate::settings::Page::new());
            }
            CommandId::About => {
                self.close_picker();
                self.settings = None;
                self.about = true;
            }
            CommandId::Agent => match arg.map(str::trim) {
                Some("new") | Some("clear") => self.clear_agent_chat(),
                _ => self.toggle_agent(),
            },
            CommandId::AgentNew => self.clear_agent_chat(),
            CommandId::FilePicker => self.open_file_picker(),
            CommandId::Workspace => self.request_workspace(arg.unwrap_or(""))?,
            CommandId::Diff => self.toggle_diff(),
            CommandId::Focus => self.toggle_focus(),
        }
        Ok(())
    }

    fn toggle_focus(&mut self) {
        self.focus_mode = !self.focus_mode;
        if self.focus_mode {
            self.explorer.blur();
            self.end_file_mode();
            self.message = Some("Focus on".to_string());
        } else {
            self.message = Some("Focus off".to_string());
        }
    }

    fn toggle_diff(&mut self) {
        if self.show_diff {
            self.show_diff = false;
            self.diff_announce = None;
            self.message = Some("Diff hidden".to_string());
            return;
        }
        self.show_diff = true;
        for editor in &mut self.editors {
            for tab in &mut editor.tabs {
                tab.diff = None;
            }
        }
        self.diff_inflight.clear();
        let tab_id = self.active_tab_id();
        let fingerprint = self.find_tab(tab_id).map(|tab| tab.buffer.fingerprint());
        if self.find_tab(tab_id).is_some_and(|tab| tab.path.is_some()) {
            self.message = Some("Diff on".to_string());
            self.diff_announce = fingerprint.map(|fingerprint| (tab_id, fingerprint));
        } else {
            self.message = Some("No file to compare".to_string());
            self.diff_announce = None;
        }
        let ids: Vec<TabId> = self
            .editors
            .iter()
            .flat_map(|editor| editor.tabs.iter().map(|tab| tab.id))
            .collect();
        for id in ids {
            self.request_diff(id);
        }
    }

    fn request_diff(&mut self, tab_id: TabId) {
        if !self.show_diff || self.diff_inflight.contains_key(&tab_id) {
            return;
        }
        let generation = self.diff_generation;
        let Some((path, fingerprint, text)) = self.find_tab(tab_id).and_then(|tab| {
            let path = tab.path.clone()?;
            let fingerprint = tab.buffer.fingerprint();
            let fresh = tab
                .diff
                .as_ref()
                .is_some_and(|cached| cached.path == path && cached.fingerprint == fingerprint);
            if fresh {
                None
            } else {
                Some((path, fingerprint, tab.buffer.rope().clone()))
            }
        }) else {
            return;
        };
        self.diff_generation = generation.wrapping_add(1);
        self.diff_inflight.insert(tab_id, (generation, fingerprint));
        self.jobs.push(Job::GitDiff {
            tab_id,
            path,
            text,
            fingerprint,
            generation,
        });
    }

    fn finish_diff(
        &mut self,
        tab_id: TabId,
        path: PathBuf,
        generation: u64,
        fingerprint: u64,
        result: DiffResult,
    ) {
        if self
            .diff_inflight
            .get(&tab_id)
            .is_none_or(|(current, _)| *current != generation)
        {
            return;
        }
        self.diff_inflight.remove(&tab_id);
        if !self.show_diff {
            return;
        }
        let matches = self.find_tab(tab_id).is_some_and(|tab| {
            tab.path.as_ref() == Some(&path) && tab.buffer.fingerprint() == fingerprint
        });
        if matches {
            if let Some(tab) = self.find_tab_mut(tab_id) {
                tab.diff = Some(CachedDiff {
                    path,
                    fingerprint,
                    result: result.clone(),
                });
            }
        }
        self.request_diff(tab_id);
        if self
            .diff_announce
            .is_some_and(|(id, announced)| id == tab_id && announced != fingerprint)
        {
            self.diff_announce = None;
        }
        if self.diff_announce == Some((tab_id, fingerprint)) && matches {
            self.diff_announce = None;
            if let DiffResult::Diff(diff) = &result
                && !diff.is_empty()
            {
                return;
            }
            self.message = Some(diff_message(&result));
        }
    }

    fn edit(&mut self, f: impl FnOnce(&mut Buffer)) {
        f(&mut self.active_editor_mut().tab_mut().buffer);
        self.note_buffer_change();
    }

    fn note_buffer_change(&mut self) {
        let tab_id = self.active_tab_id();
        self.note_tab_change(tab_id);
    }

    fn note_tab_change(&mut self, tab_id: TabId) {
        if let Some(tab) = self.find_tab_mut(tab_id) {
            tab.dirty = tab.buffer.fingerprint() != tab.saved_fingerprint;
        }
        self.sync_tab_syntax(tab_id);
        self.request_diff(tab_id);
    }

    fn editor_mut(&mut self, id: EditorId) -> Option<&mut Editor> {
        self.editors.iter_mut().find(|editor| editor.id == id)
    }

    fn copy(&mut self) -> Result<(), String> {
        let text = self.buffer().copy_text();
        let count = text.chars().count();
        self.clipboard.set_text(text)?;
        self.message = Some(format!("Copied {count} characters"));
        Ok(())
    }

    fn cut(&mut self) -> Result<(), String> {
        let text = self.buffer().copy_text();
        let count = text.chars().count();
        self.clipboard.set_text(text)?;
        self.edit(|buffer| {
            buffer.cut_text();
        });
        self.message = Some(format!("Cut {count} characters"));
        Ok(())
    }

    fn paste(&mut self) -> Result<(), String> {
        let text = self.clipboard.get_text()?;
        if text.is_empty() {
            return Err("Clipboard is empty".to_string());
        }
        self.edit(|buffer| buffer.insert_text(&text));
        Ok(())
    }

    fn enter_select(&mut self) {
        self.explorer.enter_select();
        self.message =
            Some("space marks   shift+↑↓ covers   shift+space commands   esc clears".to_string());
    }

    fn end_file_mode(&mut self) {
        self.explorer.leave_file_mode();
    }

    fn commit_marks(&mut self) -> Result<(), String> {
        if !self.explorer.selecting && !self.filtering() {
            return Ok(());
        }
        let Some((name, marked)) = self.explorer.commit_mark() else {
            return Err("Nothing to select".to_string());
        };
        let count = self.explorer.marked_count();
        self.message = Some(match (marked, count) {
            (true, 0 | 1) => format!("Marked {name}"),
            (true, n) => format!("Marked {name} ({n})"),
            (false, 0) => format!("Unmarked {name}"),
            (false, n) => format!("Unmarked {name} ({n})"),
        });
        Ok(())
    }

    fn begin_rename(&mut self, name: &str) -> Result<(), String> {
        let name = single_name(name)?;
        let rel = self
            .explorer
            .selected()
            .ok_or("Nothing to rename")?
            .to_path_buf();
        let from = abs_in(&self.explorer.root, &rel)?;
        let to = from
            .parent()
            .map(|parent| parent.join(name))
            .unwrap_or_else(|| self.explorer.root.join(name));
        if same_path(&from, &to) {
            return Err("That is already the name".to_string());
        }
        let to = ensure_inside(&self.explorer.root, &to, false)?;
        self.message = Some(format!("Renaming to {name}…"));
        self.jobs.push(Job::Files {
            root: self.explorer.root.clone(),
            change: Change::Rename { from, to },
        });
        Ok(())
    }

    fn begin_move(&mut self, dest_arg: &str) -> Result<(), String> {
        let rels = self.explorer.targets();
        if rels.is_empty() {
            return Err("Nothing to move".to_string());
        }
        let dest = resolve_in_workspace(&self.explorer.root, dest_arg)?;
        let paths = rels
            .iter()
            .map(|rel| abs_in(&self.explorer.root, rel))
            .collect::<Result<Vec<_>, _>>()?;
        self.message = Some("Moving…".to_string());
        self.jobs.push(Job::Files {
            root: self.explorer.root.clone(),
            change: Change::Move { paths, dest },
        });
        Ok(())
    }

    fn begin_delete(&mut self) -> Result<(), String> {
        let rels = self.explorer.targets();
        if rels.is_empty() {
            return Err("Nothing to delete".to_string());
        }
        let paths = rels
            .iter()
            .map(|rel| abs_in(&self.explorer.root, rel))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(name) = self.unsaved_under(&paths) {
            return Err(format!("Cannot delete {name}: unsaved changes"));
        }
        self.remember_sidebar();
        self.explorer.blur();
        self.prompt = Some(Prompt {
            title: " Delete ".to_string(),
            message: delete_message(&self.explorer, &rels),
            buttons: confirm_buttons(),
            selected: 0,
            action: Pending::Delete(paths),
        });
        Ok(())
    }

    fn queue_delete(&mut self, paths: Vec<PathBuf>) {
        self.message = Some(if paths.len() == 1 {
            format!(
                "Deleting {}…",
                file_label(paths.first().map(PathBuf::as_path))
            )
        } else {
            format!("Deleting {} items…", paths.len())
        });
        self.jobs.push(Job::Files {
            root: self.explorer.root.clone(),
            change: Change::Delete(paths),
        });
        self.restore_sidebar_focus();
    }

    fn unsaved_under(&self, paths: &[PathBuf]) -> Option<String> {
        self.editors.iter().find_map(|editor| {
            editor.tabs.iter().find_map(|tab| {
                let path = tab.path.as_deref()?;
                if tab.dirty && paths.iter().any(|target| covers(target, path)) {
                    Some(file_label(Some(path)))
                } else {
                    None
                }
            })
        })
    }

    fn finish_files(&mut self, outcome: crate::services::fs::ChangeOutcome) {
        for (from, to) in &outcome.relocated {
            if let (Some(from_rel), Some(to_rel)) = (
                relative_to(&self.explorer.root, from),
                relative_to(&self.explorer.root, to),
            ) {
                self.explorer.relocate(&from_rel, &to_rel);
            }
            self.retarget_editors(from, to);
        }
        if !outcome.deleted.is_empty() {
            let rels: Vec<PathBuf> = outcome
                .deleted
                .iter()
                .filter_map(|path| relative_to(&self.explorer.root, path))
                .collect();
            self.explorer.remove_entries(&rels);
            for path in &outcome.deleted {
                self.mark_deleted_editors(path);
            }
        }
        self.message = Some(outcome.error.unwrap_or(outcome.summary));
        self.refresh_picker_index();
        self.queue_tree_scan(true);
    }

    fn retarget_editors(&mut self, from: &Path, to: &Path) {
        let mut changed = Vec::new();
        for editor in &mut self.editors {
            for tab in &mut editor.tabs {
                let Some(path) = tab.path.clone() else {
                    continue;
                };
                if let Some(next) = retarget_path(&path, from, to) {
                    tab.path = Some(next);
                    changed.push(tab.id);
                }
            }
        }
        for tab_id in changed {
            self.configure_syntax(tab_id);
            self.request_diff(tab_id);
        }
    }

    fn mark_deleted_editors(&mut self, deleted: &Path) {
        for editor in &mut self.editors {
            for tab in &mut editor.tabs {
                let Some(path) = tab.path.as_deref() else {
                    continue;
                };
                if covers(deleted, path) {
                    tab.saved_fingerprint = tab.buffer.fingerprint().wrapping_add(1);
                    tab.dirty = true;
                }
            }
        }
    }

    /// `explicit` is a path from the `save` command. Without a path, the command bar asks for one.
    fn save_focused(
        &mut self,
        explicit: Option<PathBuf>,
        follow: Option<Pending>,
    ) -> Result<(), String> {
        let path = explicit.or_else(|| self.path().map(Path::to_path_buf));
        let Some(path) = path else {
            self.resume = follow;
            self.command_bar = Some(CommandBar {
                input: "save ".to_string(),
                ..CommandBar::default()
            });
            return Ok(());
        };
        let path = locate(&self.explorer.root, path)?;
        let after = follow.map(pending_after).unwrap_or(AfterSave::Nothing);
        self.queue_save(self.active, self.active_tab_id(), path, after, true);
        Ok(())
    }

    fn queue_save(
        &mut self,
        id: EditorId,
        tab_id: TabId,
        path: PathBuf,
        after: AfterSave,
        format: bool,
    ) {
        if format && self.format_before_save(id, tab_id, path.clone(), after.clone()) {
            return;
        }
        let forced = self.editor_prefs.line_ending.forced();
        let trim = self.file_prefs.trim_trailing_space;
        let final_newline = self.file_prefs.final_newline;
        let Some(readable) = self.find_tab(tab_id).map(|tab| tab.readable) else {
            return;
        };
        if !readable {
            self.message = Some(
                "This file is not loaded. reload <encoding> reads it, for example reload windows-874"
                    .to_string(),
            );
            return;
        }
        let Some(editor) = self.editor_mut(id) else {
            return;
        };
        let Some(tab) = editor.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let encoding = tab.encoding;
        tab.buffer.prepare_for_save(forced, trim, final_newline);
        let fingerprint = tab.buffer.fingerprint();
        let text = tab.buffer.rope().clone();
        let epoch = tab.epoch;
        tab.saving = true;
        tab.path = Some(path.clone());
        let name = file_label(Some(&path));
        self.configure_syntax(tab_id);
        self.request_diff(tab_id);
        self.jobs.push(Job::SaveFile {
            editor_id: id,
            tab_id,
            path,
            text,
            encoding,
            fingerprint,
            epoch,
            after,
        });
        self.message = Some(format!("Saving {name}…"));
    }

    fn match_bracket(&mut self, extend: bool) {
        self.dismiss_lsp_popups();
        self.clear_snippet();
        let target = {
            let tab = self.active_editor().tab();
            let rope = tab.buffer.rope();
            let head = tab.buffer.selection().head;
            let Some((at, bracket)) = crate::buffer::bracket_near(rope, head) else {
                return;
            };
            if let Some(state) = tab.syntax.as_ref().filter(|state| state.has_tree()) {
                state.match_bracket(rope, at)
            } else {
                crate::buffer::scan_bracket(rope, at, bracket)
            }
        };
        let Some(target) = target else {
            return;
        };
        self.buffer_mut().move_to(target, extend);
        self.scroll_to_cursor();
    }

    fn toggle_comment(&mut self) {
        self.clear_snippet();
        let Some(marker) = self.comment_marker() else {
            self.message = Some("This file has no line comment".to_string());
            return;
        };
        if !self.buffer_mut().toggle_comment(marker) {
            self.message = Some("Nothing to comment".to_string());
        }
    }

    fn comment_marker(&self) -> Option<&'static str> {
        use crate::syntax::Language;
        match Language::detect(self.path()?)? {
            Language::Json
            | Language::Markdown
            | Language::Html
            | Language::Css
            | Language::Xml => None,
            Language::Python
            | Language::Toml
            | Language::Yaml
            | Language::Bash
            | Language::Ruby
            | Language::Elixir
            | Language::PowerShell
            | Language::Make
            | Language::Cmake
            | Language::Hcl
            | Language::GraphQl => Some("#"),
            Language::Lua | Language::Sql | Language::Haskell => Some("--"),
            _ => Some("//"),
        }
    }

    fn start_rename(&mut self) {
        self.dismiss_lsp_popups();
        self.open_command_bar();
        if let Some(bar) = self.command_bar.as_mut() {
            bar.input = "rename-symbol ".to_string();
        }
    }

    fn queue_load(&mut self, id: EditorId, tab_id: TabId, path: PathBuf) {
        self.start_load(
            id,
            tab_id,
            path,
            LoadKind::Open {
                encoding: None,
                place: None,
                focus: true,
            },
            false,
        );
    }

    fn reload_file(&mut self, arg: Option<&str>) {
        let encoding = match arg.map(str::trim).filter(|text| !text.is_empty()) {
            None => None,
            Some(name) => match TextEncoding::parse(name) {
                Some(encoding) => Some(encoding),
                None => {
                    self.message = Some(format!(
                        "Unknown encoding {name}. Use {}",
                        TextEncoding::names()
                    ));
                    return;
                }
            },
        };
        let tab_id = self.active_tab_id();
        let Some(tab) = self.find_tab(tab_id) else {
            return;
        };
        if tab.path.is_none() {
            self.message = Some("No file to reload".to_string());
            return;
        }
        if tab.dirty {
            self.show_prompt(
                tab_id,
                Pending::Reload {
                    encoding,
                    stamp: None,
                },
            );
            return;
        }
        let encoding = encoding.or(Some(tab.encoding));
        self.queue_reload(tab_id, encoding, None);
    }

    fn set_encoding(&mut self, arg: Option<&str>) {
        let Some(name) = arg.map(str::trim).filter(|text| !text.is_empty()) else {
            self.message = Some(format!("Encoding {}", self.encoding_label()));
            return;
        };
        let Some(encoding) = TextEncoding::parse(name) else {
            self.message = Some(format!(
                "Unknown encoding {name}. Use {}",
                TextEncoding::names()
            ));
            return;
        };
        let tab_id = self.active_tab_id();
        let Some(readable) = self.find_tab(tab_id).map(|tab| tab.readable) else {
            return;
        };
        if !readable {
            self.message = Some(
                "reload <encoding> reads this file, for example reload windows-874".to_string(),
            );
            return;
        }
        if let Some(tab) = self.find_tab_mut(tab_id) {
            tab.encoding = encoding;
        }
        self.message = Some(format!(
            "Encoding set to {}. The next save writes it",
            encoding.label()
        ));
    }

    fn queue_reload(
        &mut self,
        tab_id: TabId,
        encoding: Option<TextEncoding>,
        noticed: Option<FileStamp>,
    ) {
        let Some(tab) = self.find_tab(tab_id) else {
            return;
        };
        let Some(path) = tab.path.clone() else {
            self.message = Some("No file to reload".to_string());
            return;
        };
        if !tab.readable && encoding.is_none() {
            self.message = Some(
                "This file is not UTF-8. reload <encoding> reads it, for example reload windows-874"
                    .to_string(),
            );
            return;
        }
        let (line, col) = tab.buffer.cursor_line_char();
        let Some(editor_id) = self.pane_of(tab_id) else {
            return;
        };
        self.start_load(
            editor_id,
            tab_id,
            path,
            LoadKind::Reload {
                line,
                col,
                encoding,
                noticed,
            },
            false,
        );
    }

    /// `quiet` skips the status line. Session restore opens several files at once.
    fn start_load(
        &mut self,
        id: EditorId,
        tab_id: TabId,
        path: PathBuf,
        kind: LoadKind,
        quiet: bool,
    ) {
        let Some(editor) = self.editor_mut(id) else {
            return;
        };
        let Some(index) = editor.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        let focus = matches!(kind, LoadKind::Open { focus: true, .. });
        if focus {
            editor.active = index;
        }
        let tab = &mut editor.tabs[index];
        tab.loading = true;
        tab.load_generation = tab.load_generation.wrapping_add(1);
        tab.load_baseline = tab.buffer.fingerprint();
        let generation = tab.load_generation;
        let name = file_label(Some(&path));
        let reloading = matches!(kind, LoadKind::Reload { .. });
        self.jobs.push(Job::LoadFile {
            editor_id: id,
            tab_id,
            path,
            generation,
            kind,
        });
        if focus {
            self.active = id;
            self.explorer.blur();
            self.end_file_mode();
        }
        if !quiet {
            self.message = Some(if reloading {
                format!("Reloading {name}…")
            } else {
                format!("Opening {name}…")
            });
        }
    }

    fn open_path(&mut self, path: PathBuf) {
        self.open_at(path, None);
    }

    /// Opens `path`. `line` is 1-based and moves the cursor when the file is already open.
    /// A file that still has to load keeps the line in the load request.
    fn open_at(&mut self, path: PathBuf, line: Option<usize>) {
        let path = crate::agent::strip_verbatim(&path);
        self.remember_file(&path);
        if let Some(tab_id) = self.tab_with_path(&path) {
            self.focus_tab(tab_id);
            self.explorer.blur();
            self.end_file_mode();
            if let Some(line) = line.filter(|n| *n > 0) {
                self.buffer_mut().goto_line(line - 1);
            }
            return;
        }
        self.remember_sidebar();
        let reuse = {
            let tab = self.active_editor().tab();
            tab.path.is_none() && !tab.dirty
        };
        let pane = self.active;
        let tab_id = if reuse {
            self.active_tab_id()
        } else {
            self.push_tab(pane)
        };
        self.start_load(
            pane,
            tab_id,
            path,
            LoadKind::Open {
                encoding: None,
                place: line.filter(|n| *n > 0).map(|line| (line - 1, 0)),
                focus: true,
            },
            false,
        );
    }

    fn tab_with_path(&self, path: &Path) -> Option<TabId> {
        self.editors.iter().find_map(|editor| {
            editor
                .tabs
                .iter()
                .find(|tab| {
                    tab.path
                        .as_deref()
                        .is_some_and(|open| same_path(open, path))
                })
                .map(|tab| tab.id)
        })
    }

    fn request_workspace(&mut self, raw: &str) -> Result<(), String> {
        let path = resolve_workspace(&self.explorer.root, raw)?;
        if crate::session::same_directory(&path, &self.explorer.root) {
            self.message = Some("Already here".to_string());
            return Ok(());
        }
        self.remember_sidebar();
        self.quit_skipped.clear();
        self.begin_workspace(path);
        Ok(())
    }

    fn begin_workspace(&mut self, path: PathBuf) {
        self.continue_workspace(path);
    }

    fn continue_workspace(&mut self, path: PathBuf) {
        if let Some(id) = self.next_unsaved_for_quit() {
            self.show_prompt(id, Pending::Workspace(path));
        } else {
            self.finish_workspace(path);
        }
    }

    /// Writes the folder being left, then opens `path` and restores its session.
    fn finish_workspace(&mut self, path: PathBuf) {
        self.prompt_restore_sidebar = false;
        self.quit_skipped.clear();
        self.resume = None;
        self.queue_session_save();
        self.jobs
            .retain(|job| matches!(job, Job::SaveSession { .. }));
        self.reset_workspace_runtime();
        let restored = self
            .saved_session(&path)
            .is_some_and(|saved| self.install_session(saved));
        if !restored {
            self.install_empty_workspace(&path);
        }
        self.detach_outside_files();
        self.branch = None;
        self.queue_workspace_refresh();
        let name = file_label(Some(&self.explorer.root));
        self.message = Some(format!("Opened {name}"));
    }

    /// A file from the folder just left must not stay open in the folder just opened.
    fn detach_outside_files(&mut self) {
        let root = self.explorer.root.clone();
        let stale: Vec<TabId> = self
            .editors
            .iter()
            .flat_map(|editor| editor.tabs.iter())
            .filter(|tab| {
                tab.path
                    .as_deref()
                    .is_some_and(|path| !inside_workspace(&root, path))
            })
            .map(|tab| tab.id)
            .collect();
        for id in stale {
            let blank = self.style_buffer(Buffer::new());
            let Some(tab) = self.find_tab_mut(id) else {
                continue;
            };
            let width = tab.view.width;
            let height = tab.view.height;
            tab.buffer = blank;
            tab.path = None;
            tab.dirty = false;
            tab.saved_fingerprint = tab.buffer.fingerprint();
            tab.epoch = tab.epoch.wrapping_add(1);
            tab.load_generation = tab.load_generation.wrapping_add(1);
            tab.disk = None;
            tab.loading = false;
            tab.saving = false;
            tab.stat_pending = false;
            tab.readable = true;
            tab.noted_missing = false;
            tab.lsp_edits.clear();
            tab.syntax = None;
            tab.diff = None;
            tab.view.resize(width as u16, height as u16);
        }
    }

    fn queue_session_save(&mut self) {
        let Some(session) = self.session_snapshot() else {
            return;
        };
        self.sessions
            .retain(|saved| !crate::session::same_directory(&saved.root, &session.root));
        self.sessions.insert(0, session.clone());
        self.sessions.truncate(20);
        let Some(dir) = self.appearance.config_dir().map(Path::to_path_buf) else {
            return;
        };
        self.jobs.push(Job::SaveSession {
            path: crate::session::file_in(&dir),
            session,
        });
    }

    fn reset_workspace_runtime(&mut self) {
        self.close_picker();
        self.find_mode = None;
        self.replace_mode = None;
        self.file_find = None;
        self.reveal_find = None;
        self.reveal_file = None;
        self.replace_notice = None;
        self.settings = None;
        self.about = false;
        self.focus_mode = false;
        self.command_bar = None;
        self.prompt = None;
        self.recent_files.clear();
        self.file_index = None;
        self.diff_inflight.clear();
        self.diff_announce = None;
        self.diff_generation = self.diff_generation.wrapping_add(1);
        self.reset_agent();
        self.lsp_docs.clear();
        self.diagnostics = Default::default();
        self.semantic = Default::default();
        self.lsp_servers.clear();
        self.nav = lsp_nav::Nav::default();
        self.lsp_nav_requests.clear();
        self.lsp_requests.clear();
        self.lsp_requests.push(crate::lsp::LspRequest::Reset);
        self.lsp_resetting = true;
    }

    fn reset_agent(&mut self) {
        let turn = self.agent.turn;
        if self.agent.busy {
            if let Some(tx) = &self.agent_tx {
                let _ = tx.send(crate::agent::AgentCommand::Cancel { id: turn });
            }
        }
        self.agent = crate::agent::Panel::default();
        self.agent.turn = turn.wrapping_add(1);
    }

    fn install_empty_workspace(&mut self, path: &Path) {
        let open = self.explorer.open;
        let mut explorer = Explorer::new(path.to_path_buf());
        explorer.open = open;
        self.explorer = explorer;
        self.editors.clear();
        let id = EditorId(self.next_id);
        self.next_id += 1;
        let tab = self.alloc_tab(self.style_buffer(Buffer::new()), None);
        self.editors = vec![Editor {
            id,
            tabs: vec![tab],
            active: 0,
        }];
        self.layout = Layout::Leaf(id);
        self.active = id;
    }

    fn queue_workspace_refresh(&mut self) {
        self.queue_tree_scan(false);
        let root = self.explorer.root.clone();
        if !self
            .jobs
            .iter()
            .any(|job| matches!(job, Job::GitBranch(path) if path == &root))
        {
            self.jobs.push(Job::GitBranch(root));
        }
    }

    fn request_quit(&mut self) {
        self.remember_sidebar();
        self.quit_skipped.clear();
        if self.next_unsaved_for_quit().is_some() {
            self.continue_quit();
        } else {
            self.prompt_restore_sidebar = false;
            self.should_quit = true;
        }
    }

    fn continue_quit(&mut self) {
        if let Some(id) = self.next_unsaved_for_quit() {
            self.show_prompt(id, Pending::Quit);
        } else {
            self.should_quit = true;
        }
    }

    fn next_unsaved_for_quit(&self) -> Option<TabId> {
        self.editors.iter().find_map(|editor| {
            editor
                .tabs
                .iter()
                .find(|tab| tab.dirty && !self.quit_skipped.contains(&tab.id))
                .map(|tab| tab.id)
        })
    }

    fn request_close(&mut self) -> Result<(), String> {
        if self.active_editor().tab().dirty {
            self.remember_sidebar();
            self.show_prompt(self.active_tab_id(), Pending::Close);
            return Ok(());
        }
        self.close_active()
    }

    /// `close-all` and `close-view`. Dirty tabs are asked one at a time; clean tabs close after that.
    fn request_close_batch(&mut self, batch: CloseBatch) {
        self.remember_sidebar();
        self.continue_close_batch(batch);
    }

    fn continue_close_batch(&mut self, batch: CloseBatch) {
        if let Some(id) = self.next_dirty_for(batch) {
            self.show_prompt(id, Pending::CloseBatch(batch));
            return;
        }
        self.finish_close_batch(batch);
    }

    /// The focused dirty tab comes first, so the dialog matches the file on screen.
    fn next_dirty_for(&self, batch: CloseBatch) -> Option<TabId> {
        let in_scope = |id: EditorId| match batch {
            CloseBatch::View(pane) => id == pane,
            CloseBatch::All => true,
        };
        if in_scope(self.active) && self.active_editor().tab().dirty {
            return Some(self.active_tab_id());
        }
        self.editors.iter().find_map(|editor| {
            if !in_scope(editor.id) {
                return None;
            }
            editor.tabs.iter().find(|tab| tab.dirty).map(|tab| tab.id)
        })
    }

    fn finish_close_batch(&mut self, batch: CloseBatch) {
        let ids: Vec<TabId> = match batch {
            CloseBatch::View(pane) => self
                .editors
                .iter()
                .find(|editor| editor.id == pane)
                .map(|editor| editor.tabs.iter().map(|tab| tab.id).collect())
                .unwrap_or_default(),
            CloseBatch::All => self
                .editors
                .iter()
                .flat_map(|editor| editor.tabs.iter().map(|tab| tab.id))
                .collect(),
        };
        for id in ids {
            if self.find_tab(id).is_none() {
                continue;
            }
            if let Err(message) = self.close_tab(id) {
                self.message = Some(message);
                break;
            }
        }
        self.restore_sidebar_focus();
    }

    fn remember_sidebar(&mut self) {
        if self.explorer.open && self.explorer.focused {
            self.prompt_restore_sidebar = true;
        }
    }

    fn keep_external(&mut self, tab_id: TabId, stamp: Option<FileStamp>) {
        if let Some(stamp) = stamp
            && let Some(tab) = self.find_tab_mut(tab_id)
        {
            tab.kept = Some(stamp);
        }
        self.message = Some("Kept the editor text".to_string());
    }

    fn show_prompt(&mut self, tab_id: TabId, action: Pending) {
        self.focus_tab(tab_id);
        self.command_bar = None;
        self.explorer.blur();
        let name = file_label(self.find_tab(tab_id).and_then(|tab| tab.path.as_deref()));
        let (title, message, buttons) = match &action {
            Pending::Quit => (
                " Unsaved ".to_string(),
                format!("Save changes to {name} before quitting?"),
                confirm_buttons(),
            ),
            Pending::Close | Pending::CloseBatch(_) => (
                " Unsaved ".to_string(),
                format!("Save changes to {name} before closing?"),
                confirm_buttons(),
            ),
            Pending::Open(path) => (
                " Unsaved ".to_string(),
                format!(
                    "Save changes to {name} before opening {}?",
                    file_label(Some(path))
                ),
                confirm_buttons(),
            ),
            Pending::Workspace(path) => (
                " Unsaved ".to_string(),
                format!(
                    "Save changes to {name} before opening {}?",
                    file_label(Some(path))
                ),
                confirm_buttons(),
            ),
            Pending::Delete(_) => (
                " Delete ".to_string(),
                "Delete?".to_string(),
                confirm_buttons(),
            ),
            Pending::Reload { .. } => (
                " Changed ".to_string(),
                format!("{name} changed on disk. Reload it and discard unsaved edits?"),
                reload_buttons(),
            ),
        };
        self.prompt = Some(Prompt {
            title,
            message,
            buttons,
            selected: 0,
            action,
        });
    }

    fn handle_prompt(&mut self, action: PromptAction) {
        match action {
            PromptAction::Prev => self.move_prompt(-1),
            PromptAction::Next => self.move_prompt(1),
            PromptAction::Confirm => {
                let Some(index) = self.prompt.as_ref().map(|prompt| prompt.selected) else {
                    return;
                };
                self.activate_prompt(index);
            }
            PromptAction::Accelerator(letter) => {
                let Some(index) = self.prompt.as_ref().and_then(|prompt| {
                    prompt.buttons.iter().position(|button| {
                        button
                            .mnemonic
                            .is_some_and(|mnemonic| mnemonic.eq_ignore_ascii_case(&letter))
                    })
                }) else {
                    return;
                };
                self.activate_prompt(index);
            }
            PromptAction::Cancel => {
                let reload = self
                    .prompt
                    .as_ref()
                    .and_then(|prompt| match &prompt.action {
                        Pending::Reload { stamp, .. } => Some(*stamp),
                        _ => None,
                    });
                if let Some(stamp) = reload {
                    let tab_id = self.active_tab_id();
                    self.prompt = None;
                    self.keep_external(tab_id, stamp);
                    return;
                }
                let Some(index) = self.prompt.as_ref().and_then(|prompt| {
                    prompt
                        .buttons
                        .iter()
                        .position(|button| button.effect == ButtonEffect::Dismiss)
                }) else {
                    self.prompt = None;
                    self.restore_sidebar_focus();
                    return;
                };
                self.activate_prompt(index);
            }
        }
    }

    fn move_prompt(&mut self, delta: isize) {
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        let count = prompt.buttons.len() as isize;
        if count == 0 {
            return;
        }
        prompt.selected = (prompt.selected as isize + delta).rem_euclid(count) as usize;
    }

    fn activate_prompt(&mut self, index: usize) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        let Some(effect) = prompt.buttons.get(index).map(|button| button.effect) else {
            self.prompt = Some(prompt);
            return;
        };
        let tab_id = self.active_tab_id();
        match effect {
            ButtonEffect::Dismiss => {
                let deleting = matches!(prompt.action, Pending::Delete(_));
                self.quit_skipped.clear();
                self.resume = None;
                self.restore_sidebar_focus();
                if deleting {
                    self.message = Some("Delete cancelled".to_string());
                }
            }
            ButtonEffect::Deny => match prompt.action {
                Pending::Quit => {
                    self.quit_skipped.push(tab_id);
                    self.continue_quit();
                }
                Pending::Close => {
                    if let Err(message) = self.close_active() {
                        self.message = Some(message);
                    } else {
                        self.restore_sidebar_focus();
                    }
                }
                Pending::CloseBatch(batch) => {
                    if let Err(message) = self.close_tab(tab_id) {
                        self.message = Some(message);
                        self.restore_sidebar_focus();
                    } else {
                        self.continue_close_batch(batch);
                    }
                }
                Pending::Open(path) => self.queue_load(self.active, tab_id, path),
                Pending::Workspace(path) => {
                    self.quit_skipped.push(tab_id);
                    self.continue_workspace(path);
                }
                Pending::Reload { stamp, .. } => self.keep_external(tab_id, stamp),
                Pending::Delete(_) => {
                    self.message = Some("Delete cancelled".to_string());
                    self.restore_sidebar_focus();
                }
            },
            ButtonEffect::Affirm => match prompt.action {
                Pending::Delete(paths) => self.queue_delete(paths),
                Pending::Reload { encoding, .. } => {
                    let encoding =
                        encoding.or_else(|| self.find_tab(tab_id).map(|tab| tab.encoding));
                    self.queue_reload(tab_id, encoding, None);
                }
                action => {
                    if let Err(message) = self.save_focused(None, Some(action)) {
                        self.message = Some(message);
                    }
                }
            },
        }
    }

    fn cancel_resume(&mut self) {
        if self.resume.take().is_some() {
            self.quit_skipped.clear();
            self.restore_sidebar_focus();
        }
    }

    fn restore_sidebar_focus(&mut self) {
        if self.prompt_restore_sidebar && self.explorer.open {
            self.explorer.focused = true;
        }
        self.prompt_restore_sidebar = false;
    }

    fn drop_reveal(&mut self, path: &Path) {
        if self
            .reveal_find
            .as_ref()
            .is_some_and(|(wanted, _)| same_path(wanted, path))
        {
            self.reveal_find = None;
        }
    }

    fn find_tab(&self, id: TabId) -> Option<&Tab> {
        self.editors
            .iter()
            .find_map(|editor| editor.tabs.iter().find(|tab| tab.id == id))
    }

    fn find_tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.editors
            .iter_mut()
            .find_map(|editor| editor.tabs.iter_mut().find(|tab| tab.id == id))
    }

    fn finish_save(
        &mut self,
        _pane: EditorId,
        tab_id: TabId,
        path: PathBuf,
        fingerprint: u64,
        epoch: u64,
        after: AfterSave,
        result: Result<FileStamp, String>,
    ) {
        if let Some(tab) = self.find_tab_mut(tab_id) {
            tab.saving = false;
        }
        let stamp = match result {
            Ok(stamp) => stamp,
            Err(message) => {
                self.message = Some(message);
                if let Some(action) = pending_from_after(after)
                    && self.find_tab(tab_id).is_some()
                {
                    self.show_prompt(tab_id, action);
                }
                return;
            }
        };

        let applied = self.find_tab(tab_id).is_some_and(|tab| tab.epoch == epoch);
        let keep_path = applied
            && (inside_workspace(&self.explorer.root, &path)
                || self.find_tab(tab_id).is_some_and(|tab| {
                    tab.path
                        .as_deref()
                        .is_some_and(|open| same_path(open, &path))
                }));
        if keep_path {
            let tab = self.find_tab_mut(tab_id).expect("tab exists");
            tab.path = Some(path.clone());
            tab.saved_fingerprint = fingerprint;
            tab.dirty = tab.buffer.fingerprint() != fingerprint;
            tab.disk = Some(stamp);
            tab.kept = None;
            tab.noted_missing = false;
            self.message = Some(format!("Saved {}", path.display()));
            self.notify_lsp_saved(tab_id);
        }
        self.queue_tree_scan(true);
        self.refresh_picker_index();

        if !applied {
            match after {
                AfterSave::Quit => self.continue_quit(),
                AfterSave::CloseBatch(batch) => self.continue_close_batch(batch),
                AfterSave::Workspace(path) => self.continue_workspace(path),
                AfterSave::Nothing | AfterSave::Close | AfterSave::Open(_) => {}
            }
            return;
        }

        let dirty = self.find_tab(tab_id).is_some_and(|tab| tab.dirty);
        match after {
            AfterSave::Nothing => {}
            AfterSave::Quit => self.continue_quit(),
            AfterSave::Close if dirty => self.show_prompt(tab_id, Pending::Close),
            AfterSave::Close => {
                if let Err(message) = self.close_tab(tab_id) {
                    self.message = Some(message);
                }
            }
            AfterSave::CloseBatch(batch) if dirty => {
                self.show_prompt(tab_id, Pending::CloseBatch(batch));
            }
            AfterSave::CloseBatch(batch) => {
                if let Err(message) = self.close_tab(tab_id) {
                    self.message = Some(message);
                    self.restore_sidebar_focus();
                } else {
                    self.continue_close_batch(batch);
                }
            }
            AfterSave::Open(path) if dirty => self.show_prompt(tab_id, Pending::Open(path)),
            AfterSave::Workspace(path) if dirty => {
                self.show_prompt(tab_id, Pending::Workspace(path))
            }
            AfterSave::Workspace(path) => self.continue_workspace(path),
            AfterSave::Open(path) => {
                if let Some(pane) = self.pane_of(tab_id) {
                    self.queue_load(pane, tab_id, path);
                }
            }
        }
    }

    fn pane_of(&self, tab_id: TabId) -> Option<EditorId> {
        self.editors
            .iter()
            .find(|editor| editor.tabs.iter().any(|tab| tab.id == tab_id))
            .map(|editor| editor.id)
    }

    fn close_tab(&mut self, tab_id: TabId) -> Result<(), String> {
        if !self.focus_tab(tab_id) {
            return Ok(());
        }
        self.close_active()
    }

    fn finish_load(
        &mut self,
        _pane: EditorId,
        tab_id: TabId,
        path: PathBuf,
        generation: u64,
        kind: LoadKind,
        result: Result<Decoded, OpenError>,
    ) {
        let baseline = {
            let Some(tab) = self.find_tab(tab_id) else {
                return;
            };
            if tab.load_generation != generation {
                return;
            }
            tab.load_baseline
        };
        if let Some(tab) = self.find_tab_mut(tab_id) {
            tab.loading = false;
        }
        let reloading = matches!(kind, LoadKind::Reload { .. });
        let place = match &kind {
            LoadKind::Reload { line, col, .. } => Some((*line, *col)),
            LoadKind::Open { place, .. } => *place,
        };
        let focus_editor = matches!(kind, LoadKind::Open { focus: true, .. });
        match result {
            Ok(decoded) => {
                let unchanged = self
                    .find_tab(tab_id)
                    .is_some_and(|tab| tab.buffer.fingerprint() == baseline);
                if !unchanged {
                    self.message = Some(if reloading {
                        "Reload cancelled because the buffer changed".to_string()
                    } else {
                        "Open cancelled because the buffer changed".to_string()
                    });
                    self.prompt_restore_sidebar = false;
                    self.drop_reveal(&path);
                    self.drop_pending_place(&path);
                    return;
                }
                let same = self
                    .find_tab(tab_id)
                    .is_some_and(|tab| tab.buffer.rope().to_string() == decoded.text);
                if reloading && same {
                    let tab = self.find_tab_mut(tab_id).expect("tab exists");
                    tab.encoding = decoded.encoding;
                    tab.disk = Some(decoded.stamp);
                    tab.saved_fingerprint = tab.buffer.fingerprint();
                    tab.dirty = false;
                    tab.readable = true;
                    tab.kept = None;
                    tab.noted_missing = false;
                    tab.path = Some(path);
                    self.message = None;
                    return;
                }
                let mut buffer = self.style_buffer(Buffer::from_text(&decoded.text));
                if let Some((line, col)) = place {
                    buffer.place_cursor(line, col);
                }
                let (scroll_line, scroll_col) = buffer.cursor_line_col();
                let saved_fingerprint = buffer.fingerprint();
                let tab = self.find_tab_mut(tab_id).expect("tab exists");
                let width = tab.view.width;
                let height = tab.view.height;
                tab.buffer = buffer;
                tab.path = Some(path.clone());
                tab.saved_fingerprint = saved_fingerprint;
                tab.dirty = false;
                tab.epoch = tab.epoch.wrapping_add(1);
                tab.encoding = decoded.encoding;
                tab.disk = Some(decoded.stamp);
                tab.kept = None;
                tab.readable = true;
                tab.noted_missing = false;
                tab.lsp_edits.clear();
                if reloading {
                    tab.view.scroll_to(scroll_line, scroll_col);
                    self.reset_syntax(tab_id);
                    self.request_diff(tab_id);
                    self.message = Some(format!("Reloaded {}", path.display()));
                    return;
                }
                tab.view.resize(width as u16, height as u16);
                if place.is_some() {
                    tab.view.scroll_to(scroll_line, scroll_col);
                } else {
                    tab.view.scroll_x = 0;
                    tab.view.scroll_y = 0;
                }
                self.reset_syntax(tab_id);
                self.request_diff(tab_id);
                if focus_editor {
                    self.focus_tab(tab_id);
                    self.explorer.blur();
                    self.end_file_mode();
                }
                self.prompt_restore_sidebar = false;
                self.message = None;
                if self
                    .reveal_find
                    .as_ref()
                    .is_some_and(|(wanted, _)| same_path(wanted, &path))
                {
                    let query = self.reveal_find.take().map(|(_, query)| query);
                    if let Some(query) = query {
                        self.open_find_mode(&query);
                    }
                }
                self.apply_pending_place(&path);
            }
            Err(error) => {
                self.drop_reveal(&path);
                self.drop_pending_place(&path);
                if let LoadKind::Reload { noticed, .. } = kind {
                    self.message = Some(error.message(&path));
                    if let Some(stamp) = noticed
                        && let Some(tab) = self.find_tab_mut(tab_id)
                    {
                        tab.disk = Some(stamp);
                        tab.kept = None;
                    }
                    return;
                }
                let undecoded = matches!(error, OpenError::Undecoded(_));
                self.message = Some(error.message(&path));
                if undecoded {
                    if let Some(tab) = self.find_tab_mut(tab_id) {
                        tab.path = Some(path);
                        tab.readable = false;
                        tab.disk = None;
                    }
                    self.focus_tab(tab_id);
                    self.explorer.blur();
                    self.end_file_mode();
                    self.prompt_restore_sidebar = false;
                    return;
                }
                let unused = self.find_tab(tab_id).is_some_and(|tab| {
                    tab.path.is_none() && !tab.dirty && tab.buffer.rope().len_chars() == 0
                });
                let extra = self
                    .pane_of(tab_id)
                    .is_some_and(|pane| self.editor(pane).tabs.len() > 1);
                if unused && extra {
                    let _ = self.close_tab(tab_id);
                }
                self.restore_sidebar_focus();
            }
        }
    }

    fn finish_stat(
        &mut self,
        tab_id: TabId,
        path: PathBuf,
        known: FileStamp,
        result: Result<FileStamp, String>,
    ) {
        enum Seen {
            Ignore,
            Missing(String),
            Same,
            Changed {
                dirty: bool,
                kept: bool,
                stamp: FileStamp,
                encoding: TextEncoding,
            },
        }
        let seen = {
            let Some(tab) = self.find_tab_mut(tab_id) else {
                return;
            };
            tab.stat_pending = false;
            if tab.path.as_ref() != Some(&path)
                || tab.disk != Some(known)
                || tab.loading
                || tab.saving
            {
                Seen::Ignore
            } else {
                match result {
                    Err(message) => {
                        if tab.noted_missing {
                            Seen::Ignore
                        } else {
                            tab.noted_missing = true;
                            Seen::Missing(message)
                        }
                    }
                    Ok(stamp) if stamp == known => {
                        tab.noted_missing = false;
                        Seen::Same
                    }
                    Ok(stamp) => {
                        tab.noted_missing = false;
                        Seen::Changed {
                            dirty: tab.dirty,
                            kept: tab.kept == Some(stamp),
                            encoding: tab.encoding,
                            stamp,
                        }
                    }
                }
            }
        };
        match seen {
            Seen::Ignore | Seen::Same => {}
            Seen::Missing(message) => self.message = Some(message),
            Seen::Changed {
                dirty: true,
                kept: false,
                stamp,
                ..
            } if self.prompt.is_none() => self.show_prompt(
                tab_id,
                Pending::Reload {
                    encoding: None,
                    stamp: Some(stamp),
                },
            ),
            Seen::Changed {
                dirty: false,
                stamp,
                encoding,
                ..
            } if self.prompt.is_none() => {
                self.queue_reload(tab_id, Some(encoding), Some(stamp));
            }
            Seen::Changed { .. } => {}
        }
    }

    pub fn scroll_to_cursor(&mut self) {
        let tab = self.active_editor_mut().tab_mut();
        let (line, col) = tab.buffer.cursor_line_col();
        tab.view.scroll_to(line, col);
    }

    /// Moves the cursor and the viewport together so the cursor keeps its screen row.
    fn page(&mut self, down: bool, extend: bool) {
        let lines = self.view().height.max(1);
        if down {
            let last_line = self.buffer().len_lines() - 1;
            self.buffer_mut()
                .move_cursor(Movement::PageDown(lines), extend);
            let scroll_y = (self.view().scroll_y + lines).min(last_line);
            self.view_mut().scroll_y = scroll_y;
        } else {
            self.buffer_mut()
                .move_cursor(Movement::PageUp(lines), extend);
            let scroll_y = self.view().scroll_y.saturating_sub(lines);
            self.view_mut().scroll_y = scroll_y;
        }
    }
}

fn delete_message(explorer: &Explorer, rels: &[PathBuf]) -> String {
    if rels.len() == 1 {
        let path = &rels[0];
        let is_dir = explorer
            .entries()
            .iter()
            .any(|entry| entry.path == *path && entry.is_dir);
        if is_dir {
            format!("Delete {} and everything inside?", path.display())
        } else {
            format!("Delete {}?", path.display())
        }
    } else {
        format!("Delete {} items?", rels.len())
    }
}

fn diff_message(result: &DiffResult) -> String {
    match result {
        DiffResult::Diff(diff) if diff.is_empty() => "No changes".to_string(),
        DiffResult::Diff(_) => "Diff on".to_string(),
        DiffResult::NotARepository => "Not a git repository".to_string(),
        DiffResult::GitMissing => "git was not found".to_string(),
        DiffResult::Binary => "Binary file".to_string(),
        DiffResult::Failed(message) => message.clone(),
    }
}

fn single_name(arg: &str) -> Result<&str, String> {
    let name = arg.trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || name.chars().any(|c| c.is_control())
    {
        return Err("Enter a file name with no folders".to_string());
    }
    Ok(name)
}

fn abs_in(root: &Path, rel: &Path) -> Result<PathBuf, String> {
    std::path::absolute(root.join(rel))
        .map_err(|err| format!("Cannot resolve {}: {err}", rel.display()))
}

fn ensure_inside(root: &Path, path: &Path, allow_root: bool) -> Result<PathBuf, String> {
    let root =
        std::path::absolute(root).map_err(|err| format!("Cannot resolve the workspace: {err}"))?;
    let path = std::path::absolute(path)
        .map_err(|err| format!("Cannot resolve {}: {err}", path.display()))?;
    if crate::session::same_directory(&root, &path) {
        if allow_root {
            return Ok(path);
        }
        return Err("That path is the workspace itself".to_string());
    }
    if crate::services::tree::relative_from(&root, &path).is_none() {
        return Err("That path is outside the workspace".to_string());
    }
    Ok(path)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

/// `~` and `~/rest` expand to the home folder. Every other path is left as typed.
fn expand_user(arg: &str) -> Result<PathBuf, String> {
    if arg == "~" {
        return home_dir().ok_or_else(|| "No home folder".to_string());
    }
    if let Some(rest) = arg.strip_prefix("~/").or_else(|| arg.strip_prefix("~\\")) {
        let home = home_dir().ok_or_else(|| "No home folder".to_string())?;
        return Ok(home.join(rest));
    }
    Ok(PathBuf::from(arg))
}

/// A folder to open. Absolute paths, `~`, and paths relative to `current` are accepted.
fn resolve_workspace(current: &Path, arg: &str) -> Result<PathBuf, String> {
    let arg = arg.trim();
    if arg.is_empty() {
        return Err("Enter a folder".to_string());
    }
    let raw = expand_user(arg)?;
    let joined = if raw.is_absolute() {
        raw
    } else {
        current.join(raw)
    };
    let path = std::path::absolute(&joined)
        .map_err(|err| format!("Cannot resolve {}: {err}", joined.display()))?;
    if !path.is_dir() {
        return Err(format!("Not a folder: {}", path.display()));
    }
    Ok(path)
}

fn resolve_in_workspace(root: &Path, arg: &str) -> Result<PathBuf, String> {
    let arg = arg.trim();
    if arg.is_empty() {
        return Err("Enter a folder".to_string());
    }
    let raw = PathBuf::from(arg);
    let joined = if raw.is_absolute() {
        raw
    } else {
        root.join(raw)
    };
    ensure_inside(root, &joined, true)
}

fn first_leaf(layout: &Layout) -> EditorId {
    match layout {
        Layout::Leaf(id) => *id,
        Layout::Split { first, .. } => first_leaf(first),
    }
}

fn relative_to(root: &Path, path: &Path) -> Option<PathBuf> {
    let root = std::path::absolute(root).ok()?;
    let path = std::path::absolute(path).ok()?;
    crate::services::tree::relative_from(&root, &path)
}

fn covers(parent: &Path, child: &Path) -> bool {
    inside_workspace(parent, child)
}

fn retarget_path(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    let path = std::path::absolute(path).ok()?;
    let from = std::path::absolute(from).ok()?;
    let to = std::path::absolute(to).ok()?;
    if crate::session::same_directory(&path, &from) {
        return Some(to);
    }
    crate::services::tree::relative_from(&from, &path).map(|rel| to.join(rel))
}

fn pending_after(action: Pending) -> AfterSave {
    match action {
        Pending::Quit => AfterSave::Quit,
        Pending::Close => AfterSave::Close,
        Pending::CloseBatch(batch) => AfterSave::CloseBatch(batch),
        Pending::Open(path) => AfterSave::Open(path),
        Pending::Workspace(path) => AfterSave::Workspace(path),
        Pending::Delete(_) | Pending::Reload { .. } => AfterSave::Nothing,
    }
}

fn pending_from_after(after: AfterSave) -> Option<Pending> {
    match after {
        AfterSave::Nothing => None,
        AfterSave::Quit => Some(Pending::Quit),
        AfterSave::Close => Some(Pending::Close),
        AfterSave::CloseBatch(batch) => Some(Pending::CloseBatch(batch)),
        AfterSave::Open(path) => Some(Pending::Open(path)),
        AfterSave::Workspace(path) => Some(Pending::Workspace(path)),
    }
}

fn display_rel(path: &Path) -> String {
    let mut out = String::new();
    for (index, component) in path.components().enumerate() {
        if index > 0 {
            out.push('/');
        }
        out.push_str(&component.as_os_str().to_string_lossy());
    }
    out
}

fn file_label(path: Option<&Path>) -> String {
    path.and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "[No Name]".to_string())
}

/// `path` joined onto the open folder when it is relative. An absolute path is kept.
/// A relative path that climbs out of `root` is an error, so a save stays in that folder.
fn locate(root: &Path, path: PathBuf) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return std::path::absolute(&path)
            .map_err(|err| format!("Cannot resolve {}: {err}", path.display()));
    }
    ensure_inside(root, &root.join(path), false)
}

/// True when `path` is `root` or a file inside it. Windows ignores case.
fn inside_workspace(root: &Path, path: &Path) -> bool {
    let (Ok(root), Ok(path)) = (std::path::absolute(root), std::path::absolute(path)) else {
        return false;
    };
    crate::session::same_directory(&root, &path)
        || crate::services::tree::relative_from(&root, &path).is_some()
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::path::absolute(a), std::path::absolute(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered_lines(count: usize) -> Buffer {
        let lines: Vec<String> = (0..count).map(|i| i.to_string()).collect();
        Buffer::from_text(&lines.join("\n"))
    }

    #[test]
    fn match_bracket_moves_the_cursor_and_shift_keeps_the_anchor() {
        let mut app = App::new(Buffer::from_text("f(a)"), None);
        app.buffer_mut().move_to(1, false);
        app.handle(SemanticAction::MatchBracket { extend: false });
        assert_eq!(app.buffer().selection().head, 3);
        app.handle(SemanticAction::MatchBracket { extend: false });
        assert_eq!(app.buffer().selection().head, 1);
        assert!(app.buffer().selection().is_empty());

        app.handle(SemanticAction::MatchBracket { extend: true });
        assert_eq!(
            app.buffer().selection(),
            crate::buffer::Selection { anchor: 1, head: 3 }
        );

        app.buffer_mut().move_to(0, false);
        app.handle(SemanticAction::MatchBracket { extend: false });
        assert_eq!(app.buffer().selection().head, 0);

        app.buffer_mut().move_to(1, false);
        app.run_command(crate::command::CommandId::MatchBracket, None)
            .unwrap();
        assert_eq!(app.buffer().selection().head, 3);
    }

    #[test]
    fn scrolling_keeps_scrolloff_margin() {
        let mut app = App::new(numbered_lines(100), None);
        app.view_mut().resize(80, 10);
        for _ in 0..50 {
            app.handle(SemanticAction::Move {
                movement: Movement::Down,
                extend: false,
            });
        }
        app.scroll_to_cursor();
        assert_eq!(app.view().scroll_y, 50 + SCROLLOFF + 1 - 10);
    }

    #[test]
    fn page_down_moves_by_viewport_height() {
        let mut app = App::new(numbered_lines(100), None);
        app.view_mut().resize(80, 20);
        app.handle(SemanticAction::PageDown { extend: false });
        assert_eq!(app.buffer().cursor_line_col().0, 20);
        assert_eq!(app.view().scroll_y, 20);

        app.handle(SemanticAction::PageUp { extend: true });
        assert_eq!(app.buffer().cursor_line_col().0, 0);
        assert_eq!(
            app.buffer().selection().anchor,
            app.buffer().rope().line_to_char(20)
        );
    }

    fn bar(app: &mut App, action: CommandBarAction) {
        app.handle(SemanticAction::CommandBar(action));
    }

    fn type_in_bar(app: &mut App, text: &str) {
        bar(app, CommandBarAction::Insert(text.into()));
    }

    #[test]
    fn goto_completes_argument_then_runs_and_becomes_recent() {
        let mut app = App::new(numbered_lines(10), None);
        app.handle(SemanticAction::OpenCommandBar);
        assert_eq!(app.focus(), Focus::CommandBar);

        type_in_bar(&mut app, "go");
        bar(&mut app, CommandBarAction::Submit);
        assert_eq!(app.command_bar.as_ref().unwrap().input, "goto ");

        type_in_bar(&mut app, "5");
        bar(&mut app, CommandBarAction::Submit);
        assert_eq!(app.focus(), Focus::Editor);
        assert_eq!(app.buffer().cursor_line_col(), (4, 0));
        assert_eq!(app.recent_command, Some(CommandId::GotoLine));
    }

    #[test]
    fn diff_runs_off_the_ui_thread_and_follows_an_edit() {
        use crate::services::git::{DiffResult, LineKind, diff_lines};

        let mut app = App::new(Buffer::from_text("one\n"), Some("a.rs".into()));
        app.handle(SemanticAction::OpenCommandBar);
        type_in_bar(&mut app, "diff");
        bar(&mut app, CommandBarAction::Submit);
        assert!(app.show_diff());
        assert_eq!(app.message.as_deref(), Some("Diff on"));

        let queued: Vec<_> = app
            .take_jobs()
            .into_iter()
            .filter(|job| matches!(job, Job::GitDiff { .. }))
            .collect();
        assert_eq!(queued.len(), 1);
        let Job::GitDiff {
            tab_id,
            path,
            generation,
            fingerprint,
            ..
        } = queued.into_iter().next().unwrap()
        else {
            unreachable!("filtered");
        };

        app.handle(SemanticAction::InsertChar('x'));
        assert!(
            !app.take_jobs()
                .into_iter()
                .any(|job| matches!(job, Job::GitDiff { .. }))
        );

        app.handle_event(AppEvent::GitDiff {
            tab_id,
            path: path.clone(),
            generation,
            fingerprint,
            result: DiffResult::Diff(diff_lines(&[], &["one".into()])),
        });
        assert!(app.buffer().line_content(0).starts_with('x'));
        assert!(app.editor(app.active_id()).tab().file_diff().is_none());

        let follow: Vec<_> = app
            .take_jobs()
            .into_iter()
            .filter(|job| matches!(job, Job::GitDiff { .. }))
            .collect();
        assert_eq!(follow.len(), 1);
        let Job::GitDiff {
            tab_id,
            path,
            generation,
            fingerprint,
            ..
        } = follow.into_iter().next().unwrap()
        else {
            unreachable!("filtered");
        };
        let diff = diff_lines(&["one".into()], &["xone".into()]);
        assert_eq!(diff.kind(0), Some(LineKind::Modified));
        app.handle_event(AppEvent::GitDiff {
            tab_id,
            path,
            generation,
            fingerprint,
            result: DiffResult::Diff(diff),
        });
        assert_eq!(
            app.editor(app.active_id())
                .tab()
                .file_diff()
                .unwrap()
                .kind(0),
            Some(LineKind::Modified)
        );
    }

    #[test]
    fn diff_without_a_path_does_not_call_git() {
        let mut app = App::new(Buffer::new(), None);
        app.handle(SemanticAction::OpenCommandBar);
        type_in_bar(&mut app, "diff");
        bar(&mut app, CommandBarAction::Submit);
        assert!(app.show_diff());
        assert_eq!(app.message.as_deref(), Some("No file to compare"));
        assert!(
            !app.take_jobs()
                .into_iter()
                .any(|job| matches!(job, Job::GitDiff { .. }))
        );
    }

    #[test]
    fn enter_on_empty_input_reruns_recent_command() {
        let mut app = App::new(Buffer::from_text("abc"), None);
        app.recent_command = Some(CommandId::SelectAll);
        app.handle(SemanticAction::OpenCommandBar);
        bar(&mut app, CommandBarAction::Submit);
        assert_eq!(app.buffer().selected_text(), "abc");
    }

    #[test]
    fn arrows_pick_suggestion_and_errors_surface_as_message() {
        let mut app = App::new(Buffer::new(), None);
        app.handle(SemanticAction::OpenCommandBar);
        type_in_bar(&mut app, "zzz");
        bar(&mut app, CommandBarAction::Submit);
        assert_eq!(app.message.as_deref(), Some("Unknown command: zzz"));
        assert_eq!(app.focus(), Focus::CommandBar);

        bar(&mut app, CommandBarAction::Close);
        app.handle(SemanticAction::OpenCommandBar);
        type_in_bar(&mut app, "goto abc");
        bar(&mut app, CommandBarAction::Submit);
        assert_eq!(app.message.as_deref(), Some("Invalid line number: abc"));
        assert_eq!(app.recent_command, None);

        app.handle(SemanticAction::OpenCommandBar);
        bar(&mut app, CommandBarAction::SelectPrev);
        bar(&mut app, CommandBarAction::Submit);
        assert!(app.should_quit);
    }

    #[test]
    fn find_jumps_to_the_first_match_then_next_and_previous() {
        let mut app = App::new(Buffer::from_text("one two one two"), None);
        for _ in 0..8 {
            app.handle(SemanticAction::Move {
                movement: Movement::Right,
                extend: false,
            });
        }
        run(&mut app, "find one");
        assert_eq!(app.buffer().selected_text(), "one");
        assert_eq!(app.buffer().selection().head, 0);
        assert_eq!(app.message.as_deref(), Some("1 of 2"));
        assert_eq!(app.recent_command, Some(CommandId::Find));

        app.handle(SemanticAction::FindNext);
        assert_eq!(app.buffer().selection().from(), 8);
        assert_eq!(app.message.as_deref(), Some("2 of 2"));
        app.handle(SemanticAction::FindNext);
        assert_eq!(app.message.as_deref(), Some("1 of 2"));
        app.handle(SemanticAction::FindPrevious);
        assert_eq!(app.message.as_deref(), Some("2 of 2"));

        run(&mut app, "find One");
        assert_eq!(app.message.as_deref(), Some("Not found"));
        assert_eq!(app.buffer().selection().from(), 8);
        app.handle(SemanticAction::FindNext);
        assert_eq!(app.message.as_deref(), Some("Not found"));

        let mut fresh = App::new(Buffer::from_text("one"), None);
        fresh.handle(SemanticAction::FindNext);
        assert_eq!(fresh.message.as_deref(), Some("Nothing to find"));
        fresh.handle(SemanticAction::StartReplace);
        fresh.handle(SemanticAction::Replace(ReplaceAction::Next));
        assert_eq!(fresh.message.as_deref(), Some("Nothing to replace"));
        assert_eq!(fresh.buffer().rope().to_string(), "one");
    }

    #[test]
    fn replace_mode_types_the_new_word_then_down_and_up() {
        let mut app = App::new(Buffer::from_text("cat cat cat"), None);
        run(&mut app, "find cat");
        app.handle(SemanticAction::StartReplace);
        assert_eq!(app.focus(), Focus::Replace);
        assert_eq!(app.replace_field(), "cat → ");

        app.handle(SemanticAction::Replace(ReplaceAction::Insert(
            "kitten".into(),
        )));
        assert_eq!(app.replace_field(), "cat → kitten");
        app.handle(SemanticAction::Replace(ReplaceAction::Next));
        assert_eq!(app.buffer().rope().to_string(), "kitten cat cat");
        assert_eq!(app.buffer().selected_text(), "cat");
        assert_eq!(app.message.as_deref(), Some("Replaced 1, 1 of 2"));
        assert!(app.is_dirty());

        app.handle(SemanticAction::Replace(ReplaceAction::Undo));
        assert_eq!(app.buffer().rope().to_string(), "cat cat cat");
        assert_eq!(app.message.as_deref(), Some("Undid replace"));
        assert!(!app.is_dirty());

        app.handle(SemanticAction::Replace(ReplaceAction::All));
        assert_eq!(app.buffer().rope().to_string(), "kitten kitten kitten");
        assert_eq!(app.message.as_deref(), Some("Replaced 3"));
        app.handle(SemanticAction::Replace(ReplaceAction::Undo));
        assert_eq!(app.buffer().rope().to_string(), "cat cat cat");
        assert_eq!(app.buffer().selected_text(), "cat");
    }

    #[test]
    fn replace_field_cuts_a_long_selection_with_an_ellipsis() {
        let mut app = App::new(Buffer::from_text(&"a".repeat(40)), None);
        app.buffer_mut().select_all();
        app.handle(SemanticAction::StartReplace);
        let field = app.replace_field();
        let head = field.split(" → ").next().unwrap_or("");
        assert!(head.ends_with("..."), "{field}");
        assert!(head.width() <= 24, "{field}");
        app.handle(SemanticAction::Replace(ReplaceAction::Insert("b".into())));
        assert!(
            app.replace_field().ends_with(" → b"),
            "{}",
            app.replace_field()
        );
    }

    #[test]
    fn replace_with_nothing_deletes_the_match() {
        let mut app = App::new(Buffer::from_text("aa"), None);
        run(&mut app, "find aa");
        app.handle(SemanticAction::StartReplace);
        assert_eq!(app.replace_field(), "aa → ");
        app.handle(SemanticAction::Replace(ReplaceAction::Next));
        assert_eq!(app.buffer().rope().to_string(), "");
        assert_eq!(app.message.as_deref(), Some("Replaced 1"));
    }

    #[test]
    fn find_query_stays_on_its_own_editor() {
        let mut app = App::new(Buffer::from_text("alpha beta alpha"), None);
        run(&mut app, "find alpha");
        run(&mut app, "split-right");
        app.handle(SemanticAction::FindNext);
        assert_eq!(app.message.as_deref(), Some("Nothing to find"));
        assert_eq!(app.buffer().rope().to_string(), "");

        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Left));
        app.handle(SemanticAction::FindNext);
        assert_eq!(app.buffer().selected_text(), "alpha");
        assert_eq!(app.buffer().selection().from(), 11);
        assert_eq!(app.message.as_deref(), Some("2 of 2"));
    }

    #[test]
    fn find_mode_jumps_as_each_character_is_typed() {
        let mut app = App::new(Buffer::from_text("cat cot cat"), None);
        app.handle(SemanticAction::StartFind);
        assert_eq!(app.focus(), Focus::Find);
        assert_eq!(
            app.find_mode.as_ref().map(|mode| mode.query.as_str()),
            Some("")
        );

        app.handle(SemanticAction::Find(FindAction::Insert("c".into())));
        assert_eq!(app.buffer().selected_text(), "c");
        assert_eq!(app.buffer().selection().head, 0);
        assert_eq!(app.message.as_deref(), Some("1 of 3"));

        app.handle(SemanticAction::Find(FindAction::Insert("a".into())));
        assert_eq!(app.buffer().selected_text(), "ca");
        assert_eq!(app.message.as_deref(), Some("1 of 2"));

        app.handle(SemanticAction::Find(FindAction::Insert("t".into())));
        assert_eq!(app.buffer().selected_text(), "cat");
        assert_eq!(app.message.as_deref(), Some("1 of 2"));

        app.handle(SemanticAction::Find(FindAction::Next));
        assert_eq!(app.buffer().selection().from(), 8);
        assert_eq!(app.message.as_deref(), Some("2 of 2"));
        app.handle(SemanticAction::Find(FindAction::Prev));
        assert_eq!(app.buffer().selection().from(), 0);
        assert_eq!(app.message.as_deref(), Some("1 of 2"));

        app.handle(SemanticAction::Find(FindAction::Backspace));
        assert_eq!(app.find_mode.as_ref().unwrap().query, "ca");
        assert_eq!(app.buffer().selected_text(), "ca");

        app.handle(SemanticAction::Find(FindAction::Close));
        assert_eq!(app.focus(), Focus::Editor);
        assert_eq!(app.buffer().selected_text(), "ca");

        app.handle(SemanticAction::FindNext);
        assert_eq!(app.buffer().selected_text(), "ca");
        assert_eq!(app.buffer().selection().from(), 8);

        app.handle(SemanticAction::StartFind);
        assert_eq!(app.find_mode.as_ref().unwrap().query, "ca");
        assert_eq!(app.buffer().selected_text(), "ca");
        assert_eq!(app.buffer().selection().from(), 0);
        assert_eq!(app.message.as_deref(), Some("1 of 2"));
        app.handle(SemanticAction::Find(FindAction::Submit));
        assert_eq!(app.focus(), Focus::Editor);
    }

    #[test]
    fn file_find_lists_matches_then_opens_the_file_in_find_mode() {
        let (dir, _guard) = temp_dir();
        std::fs::create_dir(dir.join("src")).unwrap();
        std::fs::write(dir.join("src").join("lib.rs"), "cat").unwrap();
        std::fs::write(dir.join("one.txt"), "cat cat").unwrap();
        std::fs::write(dir.join("two.txt"), "dog").unwrap();

        let mut app = App::new(Buffer::from_text("untouched"), None);
        app.explorer = Explorer::new(dir.clone());
        app.explorer.open = true;
        app.explorer.focused = true;
        let mut src = file_entry("src");
        src.is_dir = true;
        app.explorer.set_entries(vec![
            src,
            file_entry("src/lib.rs"),
            file_entry("one.txt"),
            file_entry("two.txt"),
        ]);

        app.handle(SemanticAction::StartFind);
        assert_eq!(app.focus(), Focus::Find);
        assert_eq!(app.buffer().rope().to_string(), "untouched");
        app.handle(SemanticAction::Find(FindAction::Insert("cat".into())));
        app.handle(SemanticAction::Find(FindAction::Submit));
        assert_eq!(app.focus(), Focus::Sidebar);
        assert_eq!(app.message.as_deref(), Some("Searching…"));
        finish_jobs(&mut app);

        let rows: Vec<_> = app
            .explorer
            .visible_rows()
            .iter()
            .map(|row| (row.entry.name.as_str(), row.hits))
            .collect();
        assert_eq!(
            rows,
            [("src", None), ("lib.rs", Some(1)), ("one.txt", Some(2))]
        );
        assert_eq!(app.message.as_deref(), Some("3 matches in 2 files"));
        assert_eq!(app.explorer.file_query.as_deref(), Some("cat"));

        app.handle(SemanticAction::Sidebar(SidebarAction::Down {
            extend: false,
        }));
        app.handle(SemanticAction::Sidebar(SidebarAction::Activate));
        finish_jobs(&mut app);
        assert_eq!(app.focus(), Focus::Find);
        assert_eq!(app.find_mode.as_ref().unwrap().query, "cat");
        assert_eq!(app.buffer().selected_text(), "cat");
        assert_eq!(app.buffer().rope().to_string(), "cat cat");

        app.handle(SemanticAction::Find(FindAction::Files));
        assert_eq!(app.focus(), Focus::Sidebar);
        assert_eq!(app.explorer.file_query.as_deref(), Some("cat"));
        assert_eq!(app.explorer.visible_rows().len(), 3);

        app.handle(SemanticAction::Sidebar(SidebarAction::Escape));
        assert_eq!(app.focus(), Focus::Sidebar);
        assert!(app.explorer.file_query.is_none());
        let names: Vec<_> = app
            .explorer
            .visible_rows()
            .iter()
            .map(|row| row.entry.name.as_str())
            .collect();
        assert_eq!(names, ["src", "one.txt", "two.txt"]);

        app.handle(SemanticAction::StartFind);
        assert_eq!(app.find_mode.as_ref().unwrap().query, "cat");
        app.handle(SemanticAction::Find(FindAction::Close));
        assert_eq!(app.focus(), Focus::Sidebar);
        assert!(app.find_mode.is_none());
    }

    #[test]
    fn filter_replace_changes_marked_files_or_every_match() {
        let (dir, _guard) = temp_dir();
        std::fs::write(dir.join("one.txt"), "glowworm glowworm").unwrap();
        std::fs::write(dir.join("two.txt"), "glowworm").unwrap();
        let mut app = App::new(Buffer::from_text("untouched"), None);
        app.explorer = Explorer::new(dir.clone());
        app.explorer.open = true;
        app.explorer.focused = true;
        app.explorer
            .set_entries(vec![file_entry("one.txt"), file_entry("two.txt")]);

        app.handle(SemanticAction::StartFind);
        app.handle(SemanticAction::Find(FindAction::Insert("glowworm".into())));
        app.handle(SemanticAction::Find(FindAction::Submit));
        finish_jobs(&mut app);
        assert!(app.filtering());
        app.handle(SemanticAction::Sidebar(SidebarAction::Mark));
        assert!(app.explorer.is_marked(Path::new("one.txt")));

        app.handle(SemanticAction::StartReplace);
        assert_eq!(app.focus(), Focus::Replace);
        assert_eq!(app.replace_field(), "glowworm → ");
        app.handle(SemanticAction::Replace(ReplaceAction::Insert(
            "firefly".into(),
        )));
        app.handle(SemanticAction::Replace(ReplaceAction::Submit));
        finish_jobs(&mut app);
        finish_jobs(&mut app);
        assert_eq!(
            std::fs::read_to_string(dir.join("one.txt")).unwrap(),
            "firefly firefly"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("two.txt")).unwrap(),
            "glowworm"
        );
        assert_eq!(app.message.as_deref(), Some("Replaced 2 matches in 1 file"));
        assert_eq!(app.focus(), Focus::Sidebar);

        app.explorer.clear_marks();
        app.handle(SemanticAction::StartReplace);
        assert_eq!(app.replace_field(), "glowworm → firefly");
        for _ in 0.."firefly".len() {
            app.handle(SemanticAction::Replace(ReplaceAction::Backspace));
        }
        app.handle(SemanticAction::Replace(ReplaceAction::Insert(
            "lamp".into(),
        )));
        app.handle(SemanticAction::Replace(ReplaceAction::Submit));
        finish_jobs(&mut app);
        finish_jobs(&mut app);
        assert_eq!(
            std::fs::read_to_string(dir.join("two.txt")).unwrap(),
            "lamp"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("one.txt")).unwrap(),
            "firefly firefly"
        );
    }

    #[test]
    fn replace_mode_reopens_with_the_last_replacement() {
        let mut app = App::new(Buffer::from_text("cat cat"), None);
        run(&mut app, "find cat");
        app.handle(SemanticAction::StartReplace);
        app.handle(SemanticAction::Replace(ReplaceAction::Insert(
            "kitten".into(),
        )));
        app.handle(SemanticAction::Replace(ReplaceAction::Close));
        app.handle(SemanticAction::StartReplace);
        assert_eq!(app.replace_field(), "cat → kitten");

        app.handle(SemanticAction::Replace(ReplaceAction::Next));
        app.handle(SemanticAction::Replace(ReplaceAction::Close));
        app.handle(SemanticAction::StartReplace);
        assert_eq!(app.replace_field(), "cat → kitten");

        run(&mut app, "split-right");
        app.handle(SemanticAction::StartReplace);
        assert_eq!(app.replace_field(), " → ");
    }

    #[test]
    fn find_keeps_spaces_inside_the_text() {
        let mut app = App::new(Buffer::from_text("one two one"), None);
        run(&mut app, "find two one");
        assert_eq!(app.buffer().selected_text(), "two one");
        assert_eq!(app.message.as_deref(), Some("1 of 1"));
    }

    fn run(app: &mut App, input: &str) {
        app.handle(SemanticAction::OpenCommandBar);
        type_in_bar(app, input);
        bar(app, CommandBarAction::Submit);
    }

    #[test]
    fn bare_number_goes_to_line() {
        let mut app = App::new(numbered_lines(10), None);
        run(&mut app, " 7 ");
        assert_eq!(app.focus(), Focus::Editor);
        assert_eq!(app.buffer().cursor_line_col(), (6, 0));
        assert_eq!(app.recent_command, Some(CommandId::GotoLine));

        run(&mut app, "0");
        assert_eq!(app.message.as_deref(), Some("Invalid line number: 0"));
    }

    #[test]
    fn right_accepts_the_closest_folder_or_file_name() {
        let mut app = App::new(Buffer::new(), None);
        let mut examples = file_entry("examples");
        examples.is_dir = true;
        let mut extra = file_entry("extra");
        extra.is_dir = true;
        let mut src = file_entry("src");
        src.is_dir = true;
        let mut ui = file_entry("src/ui");
        ui.is_dir = true;
        let main = file_entry("src/main.rs");
        focus_tree(
            &mut app,
            PathBuf::from("/work/duckx-code"),
            vec![examples, extra, src, ui, main],
        );
        app.explorer.enter_select();
        app.handle(SemanticAction::OpenCommandBar);
        type_in_bar(&mut app, "move ex");
        assert_eq!(app.argument_ghost().as_deref(), Some("tra"));
        bar(&mut app, CommandBarAction::AcceptHint);
        assert_eq!(app.command_bar.as_ref().unwrap().input, "move extra");
        assert_eq!(app.argument_ghost(), None);

        app.command_bar = None;
        app.explorer.move_selection(2, false);
        app.explorer.expand(false);
        app.explorer.expand(false);
        assert_eq!(app.explorer.selected(), Some(Path::new("src/ui")));
        app.handle(SemanticAction::OpenCommandBar);
        type_in_bar(&mut app, "rename m");
        assert_eq!(app.argument_ghost().as_deref(), Some("ain.rs"));
        bar(&mut app, CommandBarAction::AcceptHint);
        assert_eq!(app.command_bar.as_ref().unwrap().input, "rename main.rs");

        app.explorer.blur();
        app.command_bar = None;
        app.handle(SemanticAction::OpenCommandBar);
        type_in_bar(&mut app, "save src/m");
        assert_eq!(app.argument_ghost().as_deref(), Some("ain.rs"));

        app.command_bar = None;
        app.handle(SemanticAction::OpenCommandBar);
        type_in_bar(&mut app, "goto 4");
        assert_eq!(app.argument_ghost(), None);
        bar(&mut app, CommandBarAction::AcceptHint);
        assert_eq!(app.command_bar.as_ref().unwrap().input, "goto 4");
    }

    #[test]
    fn tab_keeps_numeric_input() {
        let mut app = App::new(numbered_lines(10), None);
        app.handle(SemanticAction::OpenCommandBar);
        type_in_bar(&mut app, "42");
        bar(&mut app, CommandBarAction::Complete);
        assert_eq!(app.command_bar.as_ref().unwrap().input, "42");
    }

    #[test]
    fn tab_switches_the_command_bar_and_the_assistant() {
        let mut app = App::new(Buffer::from_text("hi"), None);
        app.handle(SemanticAction::OpenCommandBar);
        assert_eq!(app.focus(), Focus::CommandBar);
        bar(&mut app, CommandBarAction::ToChat);
        assert!(app.command_bar.is_none());
        assert!(app.agent.open && app.agent.focused);
        assert_eq!(app.focus(), Focus::Agent);
        app.handle(SemanticAction::OpenCommandBar);
        assert_eq!(app.focus(), Focus::CommandBar);
        assert!(app.agent.open);
        bar(&mut app, CommandBarAction::ToChat);
        assert_eq!(app.focus(), Focus::Agent);
    }

    #[test]
    fn a_prompt_tells_the_model_the_cursor_line() {
        let mut app = App::new(
            Buffer::from_text("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n"),
            Some(PathBuf::from("examples/lang/python.py")),
        );
        for _ in 0..7 {
            app.handle(SemanticAction::Move {
                movement: Movement::Down,
                extend: false,
            });
        }
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        app.attach_agent(tx);
        app.ai.model = "qwen".into();
        app.agent.prompt = "add hello".into();
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Submit));
        let crate::agent::AgentCommand::Turn { prompt, .. } = rx.try_recv().unwrap() else {
            panic!("prompt was not sent");
        };
        assert!(prompt.starts_with("add hello\n"), "{prompt}");
        assert!(prompt.contains("editor_state:"), "{prompt}");
        assert!(prompt.contains("python.py"), "{prompt}");
        assert!(prompt.contains("line: 8"), "{prompt}");
        assert_eq!(app.agent.transcript.last().unwrap().text, "add hello");
    }

    #[test]
    fn mutating_tools_wait_for_allow_or_deny() {
        let root = std::env::temp_dir().join(format!(
            "duckx-approve-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("demo.txt");
        std::fs::write(&file, "hello\n").unwrap();
        let mut app = App::new(Buffer::from_text("hello\n"), Some(file.clone()));
        app.explorer = crate::explorer::Explorer::new(root.clone());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        app.attach_agent(tx);
        app.ai.model = "qwen".into();
        app.agent.busy = true;
        app.agent.turn = 1;
        let args = format!(
            r#"{{"path":"{}","find":"hello","replace":"hi"}}"#,
            file.display()
        );
        app.handle_agent(crate::agent::AgentEvent::Tool {
            id: 1,
            call_id: "c1".into(),
            name: "apply_edit".into(),
            arguments: args.clone(),
        });
        assert!(app.agent.pending.is_some());
        assert!(app.agent.status.contains("approve"), "{}", app.agent.status);
        assert!(rx.try_recv().is_err(), "tool must not run before Allow");

        app.handle(SemanticAction::Agent(crate::action::AgentAction::Insert(
            "n".into(),
        )));
        let crate::agent::AgentCommand::ToolResult { content, .. } = rx.try_recv().unwrap() else {
            panic!("deny must reply to the worker");
        };
        assert!(content.contains("denied"), "{content}");
        assert!(app.agent.pending.is_none());
        assert_eq!(app.buffer().rope().to_string(), "hello\n");

        app.agent.busy = true;
        app.handle_agent(crate::agent::AgentEvent::Tool {
            id: 1,
            call_id: "c2".into(),
            name: "apply_edit".into(),
            arguments: args,
        });
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Submit));
        assert!(app.agent.pending.is_none());
        assert_eq!(app.buffer().rope().to_string(), "hi\n");
        let crate::agent::AgentCommand::ToolResult { content, .. } = rx.try_recv().unwrap() else {
            panic!("allow must reply");
        };
        assert!(content.contains("Replaced"), "{content}");

        app.ai.auto_approve = true;
        app.agent.busy = true;
        app.handle_agent(crate::agent::AgentEvent::Tool {
            id: 1,
            call_id: "c3".into(),
            name: "apply_edit".into(),
            arguments: format!(
                r#"{{"path":"{}","find":"hi","replace":"hey"}}"#,
                file.display()
            ),
        });
        assert!(app.agent.pending.is_none());
        assert_eq!(app.buffer().rope().to_string(), "hey\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_prompt_moves_and_shift_selects() {
        let mut app = App::new(Buffer::from_text("hi"), None);
        let type_prompt = |app: &mut App, text: &str| {
            app.handle(SemanticAction::Agent(crate::action::AgentAction::Insert(
                text.into(),
            )));
        };
        type_prompt(&mut app, "hello");
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Move {
            right: false,
            extend: false,
        }));
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Move {
            right: false,
            extend: true,
        }));
        assert_eq!(app.agent.prompt, "hello");
        assert_eq!(app.agent.cursor, 3);
        assert_eq!(app.agent.anchor, 4);
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Backspace));
        assert_eq!(app.agent.prompt, "helo");
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Jump {
            end: false,
            extend: false,
        }));
        type_prompt(&mut app, "X");
        assert_eq!(app.agent.prompt, "Xhelo");
        assert_eq!(app.agent.cursor, 1);
    }

    #[test]
    fn ctrl_up_reads_the_transcript_and_ctrl_down_returns() {
        let mut app = App::new(Buffer::from_text("hi"), None);
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Above));
        assert!(app.agent.reading);
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Line {
            down: false,
            extend: false,
        }));
        assert_eq!(app.agent.scroll, 1);
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Insert(
            "x".into(),
        )));
        assert!(app.agent.prompt.is_empty());
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Below));
        assert!(!app.agent.reading);
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Insert(
            "x".into(),
        )));
        assert_eq!(app.agent.prompt, "x");
    }

    #[test]
    fn the_prompt_keeps_pasted_newlines_and_joins_on_backspace() {
        let mut app = App::new(Buffer::from_text("hi"), None);
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Insert(
            "a\nb".into(),
        )));
        assert_eq!(app.agent.prompt, "a\nb");
        assert_eq!(app.agent.cursor, 3);
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Move {
            right: false,
            extend: false,
        }));
        assert_eq!(app.agent.cursor, 2);
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Backspace));
        assert_eq!(app.agent.prompt, "ab");
        assert_eq!(app.agent.cursor, 1);
    }

    #[test]
    fn the_prompt_home_end_stay_on_the_current_wrap_row() {
        let mut app = App::new(Buffer::from_text("hi"), None);
        app.set_screen_size(40, 24);
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Insert(
            "one\ntwo".into(),
        )));
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Jump {
            end: false,
            extend: false,
        }));
        assert_eq!(app.agent.cursor, 4); // start of "two"
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Jump {
            end: true,
            extend: false,
        }));
        assert_eq!(app.agent.cursor, 7); // end of "two"
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Line {
            down: false,
            extend: false,
        }));
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Jump {
            end: false,
            extend: false,
        }));
        assert_eq!(app.agent.cursor, 0);
    }

    #[test]
    fn the_prompt_steps_over_a_thai_cluster() {
        let mut app = App::new(Buffer::from_text("hi"), None);
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Insert(
            "ที่a".into(),
        )));
        assert_eq!(app.agent.cursor, 4);
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Move {
            right: false,
            extend: false,
        }));
        assert_eq!(app.agent.cursor, 3);
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Move {
            right: false,
            extend: false,
        }));
        assert_eq!(app.agent.cursor, 0);
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Move {
            right: true,
            extend: false,
        }));
        app.handle(SemanticAction::Agent(crate::action::AgentAction::Backspace));
        assert_eq!(app.agent.prompt, "a");
    }

    #[test]
    fn copy_and_paste_through_clipboard() {
        let mut app = App::new(Buffer::from_text("hello world"), None);
        run(&mut app, "paste");
        assert_eq!(app.message.as_deref(), Some("Clipboard is empty"));

        for _ in 0..5 {
            app.handle(SemanticAction::Move {
                movement: Movement::Right,
                extend: true,
            });
        }
        run(&mut app, "copy");
        assert_eq!(app.message.as_deref(), Some("Copied 5 characters"));
        assert_eq!(app.buffer().selected_text(), "hello");

        app.handle(SemanticAction::Move {
            movement: Movement::DocumentEnd,
            extend: false,
        });
        run(&mut app, "paste");
        assert_eq!(app.buffer().rope().to_string(), "hello worldhello");
        assert_eq!(app.recent_command, Some(CommandId::Paste));
    }

    #[test]
    fn ctrl_b_cycles_open_focus_close_and_queues_scan() {
        let mut app = App::new(Buffer::new(), None);
        app.explorer = Explorer::new(PathBuf::from("/work/repo"));

        app.handle(SemanticAction::ToggleSidebar);
        assert_eq!(app.focus(), Focus::Sidebar);
        assert_eq!(
            app.take_jobs(),
            [
                Job::ScanTree(PathBuf::from("/work/repo")),
                Job::GitStatus(PathBuf::from("/work/repo"))
            ]
        );
        assert!(app.take_jobs().is_empty());

        app.handle(SemanticAction::Sidebar(SidebarAction::Escape));
        assert_eq!(app.focus(), Focus::Editor);
        assert!(app.explorer.open);

        app.handle(SemanticAction::ToggleSidebar);
        assert_eq!(app.focus(), Focus::Sidebar);
        assert!(app.take_jobs().is_empty());

        app.handle(SemanticAction::ToggleSidebar);
        assert_eq!(app.focus(), Focus::Editor);
        assert!(!app.explorer.open);
    }

    #[test]
    fn sidebar_grows_and_shrinks_until_the_screen_limit() {
        let mut app = App::new(Buffer::new(), None);
        app.handle(SemanticAction::ResizeSidebar { wider: true });
        assert_eq!(app.sidebar_width(), layout::SIDEBAR_WIDTH);

        app.handle(SemanticAction::ToggleSidebar);
        app.handle(SemanticAction::ResizeSidebar { wider: true });
        assert_eq!(app.sidebar_width(), 34);
        let sidebar = app
            .focus_surfaces()
            .into_iter()
            .find(|(surface, _)| *surface == Surface::Sidebar)
            .unwrap()
            .1;
        assert_eq!(sidebar.width, 34);

        app.sidebar_width = 58;
        app.handle(SemanticAction::ResizeSidebar { wider: true });
        assert_eq!(app.sidebar_width(), 60);
        app.handle(SemanticAction::ResizeSidebar { wider: true });
        assert_eq!(app.message.as_deref(), Some("Sidebar is at its widest"));

        app.sidebar_width = 18;
        app.handle(SemanticAction::ResizeSidebar { wider: false });
        assert_eq!(app.sidebar_width(), 16);
        app.handle(SemanticAction::ResizeSidebar { wider: false });
        assert_eq!(app.message.as_deref(), Some("Sidebar is at its narrowest"));
    }

    #[test]
    fn explorer_command_toggles_like_ctrl_b() {
        let mut app = App::new(Buffer::new(), None);
        run(&mut app, "explorer");
        assert!(app.explorer.open);
        assert_eq!(app.focus(), Focus::Sidebar);
    }

    #[test]
    fn scan_results_for_another_root_are_ignored_and_enter_opens_a_file() {
        use crate::explorer::FileEntry;

        let mut app = App::new(Buffer::new(), None);
        app.handle(SemanticAction::ToggleSidebar);
        let file = FileEntry {
            path: PathBuf::from("main.rs"),
            name: "main.rs".into(),
            is_dir: false,
            depth: 1,
        };
        app.handle_event(AppEvent::TreeScanned {
            root: PathBuf::from("/elsewhere"),
            entries: vec![file.clone()],
        });
        assert!(app.explorer.entries().is_empty());

        app.handle_event(AppEvent::TreeScanned {
            root: app.explorer.root.clone(),
            entries: vec![file],
        });
        assert!(!app.explorer.loading);
        app.take_jobs();
        app.handle(SemanticAction::Sidebar(SidebarAction::Activate));
        let jobs = app.take_jobs();
        assert!(
            matches!(jobs.as_slice(), [Job::LoadFile { .. }]),
            "{jobs:?}"
        );
        let event = crate::jobs::run(jobs.into_iter().next().unwrap());
        app.handle_event(event);
        assert!(
            app.message
                .as_deref()
                .is_some_and(|message| message.contains("Cannot open")),
            "{:?}",
            app.message
        );
        assert_eq!(app.focus(), Focus::Sidebar);
    }

    #[test]
    fn an_outside_change_scans_an_open_tree_once_and_keeps_the_rows() {
        let root = PathBuf::from("/work/repo");
        let mut app = App::new(Buffer::new(), None);
        app.explorer = Explorer::new(root.clone());
        app.explorer.open = true;
        app.explorer.set_entries(vec![file_entry("keep.rs")]);
        assert_eq!(app.explorer.selected(), Some(Path::new("keep.rs")));

        app.note_fs_changed(Path::new("/elsewhere"));
        assert!(app.take_jobs().is_empty());

        app.explorer.open = false;
        app.note_fs_changed(&root);
        assert!(app.take_jobs().is_empty());
        app.explorer.open = true;

        app.note_fs_changed(&root);
        app.note_fs_changed(&root);
        assert_eq!(
            app.take_jobs(),
            [Job::ScanTree(root.clone()), Job::GitStatus(root.clone())]
        );
        assert!(!app.explorer.loading);
        assert_eq!(app.explorer.selected(), Some(Path::new("keep.rs")));

        app.note_fs_changed(&root);
        assert!(app.take_jobs().is_empty());
        app.handle_event(AppEvent::TreeScanned {
            root: root.clone(),
            entries: vec![file_entry("fresh.rs"), file_entry("keep.rs")],
        });
        assert!(!app.explorer.loading);
        assert_eq!(app.explorer.selected(), Some(Path::new("keep.rs")));
        assert!(
            app.explorer
                .entries()
                .iter()
                .any(|entry| entry.name == "fresh.rs")
        );
        assert_eq!(
            app.take_jobs(),
            [Job::ScanTree(root.clone()), Job::GitStatus(root)]
        );
    }

    /// Regression: disk create / delete / rename must refresh the open Explorer
    /// through note_fs_changed → ScanTree → TreeScanned → set_entries without
    /// closing and reopening the sidebar (commit 517ca8a).
    #[test]
    fn disk_create_delete_rename_refresh_open_explorer_without_reopen() {
        let (dir, _guard) = temp_dir();
        std::fs::write(dir.join("keep.txt"), "keep\n").unwrap();
        std::fs::write(dir.join("gone.txt"), "gone\n").unwrap();

        let mut app = App::new(Buffer::new(), None);
        app.explorer = Explorer::new(dir.clone());
        app.explorer.open = true;
        // Seed the tree the way a first scan would.
        let first = crate::jobs::run(Job::ScanTree(dir.clone()));
        app.handle_event(first);
        app.take_jobs();
        let names: Vec<_> = app
            .explorer
            .entries()
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert!(names.contains(&"keep.txt"), "{names:?}");
        assert!(names.contains(&"gone.txt"), "{names:?}");

        // Create, delete, and rename on disk while the sidebar stays open.
        std::fs::write(dir.join("fresh.txt"), "fresh\n").unwrap();
        std::fs::remove_file(dir.join("gone.txt")).unwrap();
        std::fs::rename(dir.join("keep.txt"), dir.join("kept.txt")).unwrap();

        app.note_fs_changed(&dir);
        let jobs = app.take_jobs();
        assert!(
            jobs.iter().any(|j| matches!(j, Job::ScanTree(_))),
            "disk change must queue ScanTree while Explorer is open: {jobs:?}"
        );
        let scan = jobs
            .into_iter()
            .find(|j| matches!(j, Job::ScanTree(_)))
            .expect("ScanTree");
        let event = crate::jobs::run(scan);
        app.handle_event(event);

        let names: Vec<_> = app
            .explorer
            .entries()
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert!(
            names.contains(&"fresh.txt"),
            "create must appear without reopening Explorer: {names:?}"
        );
        assert!(
            names.contains(&"kept.txt"),
            "rename must appear without reopening Explorer: {names:?}"
        );
        assert!(
            !names.contains(&"gone.txt"),
            "delete must disappear without reopening Explorer: {names:?}"
        );
        assert!(
            !names.contains(&"keep.txt"),
            "old rename source must be gone: {names:?}"
        );
        assert!(app.explorer.open, "sidebar must stay open");
    }


    #[test]
    fn focus_shows_one_buffer_and_leaves_the_rest_in_place() {
        let mut app = App::new(Buffer::from_text("alpha"), None);
        run(&mut app, "explorer");
        app.handle(SemanticAction::Sidebar(SidebarAction::Escape));
        run(&mut app, "split-right");
        app.handle(SemanticAction::InsertChar('R'));
        run(&mut app, "focus");
        assert!(app.focus_mode());
        assert_eq!(app.message.as_deref(), Some("Focus on"));
        assert_eq!(app.buffer().rope().to_string(), "R");

        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Left));
        assert!(app.focus_mode());
        assert_eq!(app.focus(), Focus::Editor);
        assert_eq!(app.buffer().rope().to_string(), "alpha");

        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Left));
        run(&mut app, "explorer");
        app.handle(SemanticAction::ToggleSidebar);
        assert!(app.focus_mode());
        assert!(app.explorer.open);
        assert_eq!(app.focus(), Focus::Editor);
        assert_eq!(app.buffer().rope().to_string(), "alpha");

        run(&mut app, "focus");
        assert!(!app.focus_mode());
        assert_eq!(app.message.as_deref(), Some("Focus off"));
        app.handle(SemanticAction::ToggleSidebar);
        assert_eq!(app.focus(), Focus::Sidebar);
    }

    #[test]
    fn split_right_keeps_text_and_ctrl_left_returns() {
        let mut app = App::new(Buffer::from_text("alpha"), None);
        let original = app.active_id();
        run(&mut app, "split-right");
        assert_eq!(app.focus(), Focus::Editor);
        assert_eq!(app.buffer().rope().to_string(), "");
        assert_ne!(app.active_id(), original);

        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Left));
        assert_eq!(app.active_id(), original);
        assert_eq!(app.buffer().rope().to_string(), "alpha");

        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Right));
        assert_eq!(app.buffer().rope().to_string(), "");
    }

    #[test]
    fn split_down_returns_up_to_the_original() {
        let mut app = App::new(Buffer::from_text("alpha"), None);
        let original = app.active_id();
        run(&mut app, "split-down");
        app.handle(SemanticAction::InsertChar('b'));
        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Up));
        assert_eq!(app.active_id(), original);
        assert_eq!(app.buffer().rope().to_string(), "alpha");
    }

    #[test]
    fn close_middle_of_a_nested_split_promotes_the_sibling() {
        let mut app = App::new(Buffer::from_text("alpha"), None);
        let left = app.active_id();
        run(&mut app, "split-right");
        let middle = app.active_id();
        run(&mut app, "split-down");
        let bottom = app.active_id();

        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Up));
        assert_eq!(app.active_id(), middle);
        run(&mut app, "close");

        assert_eq!(app.active_id(), bottom);
        assert_eq!(
            app.layout(),
            &Layout::Split {
                axis: Axis::Horizontal,
                first: Box::new(Layout::Leaf(left)),
                second: Box::new(Layout::Leaf(bottom)),
            }
        );
        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Left));
        assert_eq!(app.buffer().rope().to_string(), "alpha");
    }

    #[test]
    fn close_last_editor_leaves_a_blank_page() {
        let mut app = App::new(Buffer::from_text("alpha"), Some("src/main.rs".into()));
        let id = app.active_id();
        run(&mut app, "close");
        assert!(app.message.is_none());
        assert_eq!(app.active_id(), id);
        assert_eq!(app.buffer().rope().to_string(), "");
        assert!(app.path().is_none());
        assert!(!app.is_dirty());
        assert!(matches!(app.layout(), Layout::Leaf(_)));
    }

    #[test]
    fn split_right_runs_from_the_editor_and_not_from_the_explorer() {
        let mut app = App::new(Buffer::from_text("alpha"), None);
        app.handle(SemanticAction::ToggleSidebar);
        assert_eq!(app.focus(), Focus::Sidebar);
        run(&mut app, "split-right");
        assert_eq!(
            app.message.as_deref(),
            Some("split-right applies to the editor")
        );
        assert_eq!(app.focus(), Focus::CommandBar);
        assert_eq!(app.buffer().rope().to_string(), "alpha");

        app.handle(SemanticAction::CommandBar(CommandBarAction::Close));
        app.handle(SemanticAction::Sidebar(SidebarAction::Escape));
        assert_eq!(app.focus(), Focus::Editor);
        run(&mut app, "split-right");
        assert_eq!(app.focus(), Focus::Editor);
        assert!(app.explorer.open);
        assert_eq!(app.buffer().rope().to_string(), "");
    }

    #[test]
    fn ctrl_left_does_not_open_a_closed_sidebar() {
        let mut app = App::new(Buffer::new(), None);
        app.explorer = Explorer::new(PathBuf::from("/work/repo"));
        let editor = app.active_id();

        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Left));
        assert_eq!(app.focus(), Focus::Editor);
        assert_eq!(app.active_id(), editor);
        assert!(!app.explorer.open);
        assert!(app.take_jobs().is_empty());

        app.handle(SemanticAction::ToggleSidebar);
        assert_eq!(app.focus(), Focus::Sidebar);
        app.handle(SemanticAction::Sidebar(SidebarAction::Escape));
        assert_eq!(app.focus(), Focus::Editor);
        app.take_jobs();

        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Left));
        assert_eq!(app.focus(), Focus::Sidebar);
        assert!(app.explorer.open);
        assert!(app.take_jobs().is_empty());

        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Left));
        assert_eq!(app.focus(), Focus::Sidebar);

        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Right));
        assert_eq!(app.focus(), Focus::Editor);
        assert_eq!(app.active_id(), editor);
        assert!(app.explorer.open);
    }

    #[test]
    fn ctrl_arrow_follows_the_pane_under_the_current_one() {
        let mut app = App::new(Buffer::from_text("alpha"), None);
        let top_left = app.active_id();
        run(&mut app, "split-down");
        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Up));
        run(&mut app, "split-right");
        let top_right = app.active_id();
        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Down));
        run(&mut app, "split-right");
        let bottom_right = app.active_id();
        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Left));
        run(&mut app, "split-right");

        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Up));
        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Right));
        assert_eq!(app.active_id(), top_right);

        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Down));
        assert_eq!(app.active_id(), bottom_right);

        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Up));
        assert_eq!(app.active_id(), top_right);
        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Left));
        assert_eq!(app.active_id(), top_left);
        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Left));
        assert_eq!(app.focus(), Focus::Editor);
        assert!(!app.explorer.open);
        assert_eq!(app.buffer().rope().to_string(), "alpha");
    }

    #[test]
    fn escape_clears_filter_before_leaving_sidebar() {
        let mut app = App::new(Buffer::new(), None);
        app.handle(SemanticAction::ToggleSidebar);
        app.handle(SemanticAction::Sidebar(SidebarAction::FilterInsert(
            "rs".into(),
        )));
        app.handle(SemanticAction::Sidebar(SidebarAction::Escape));
        assert_eq!(app.explorer.filter, "");
        assert_eq!(app.focus(), Focus::Sidebar);
        app.handle(SemanticAction::Sidebar(SidebarAction::Escape));
        assert_eq!(app.focus(), Focus::Editor);
    }

    #[test]
    fn horizontal_scroll_follows_cursor() {
        let mut app = App::new(Buffer::from_text(&"x".repeat(200)), None);
        app.view_mut().resize(50, 10);
        app.handle(SemanticAction::Move {
            movement: Movement::LineEnd,
            extend: false,
        });
        app.scroll_to_cursor();
        assert_eq!(app.view().scroll_x, 200 + 1 - 50);
    }

    fn temp_dir() -> (PathBuf, impl Drop) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("duckx-app-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        struct Guard(PathBuf);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        (path.clone(), Guard(path))
    }

    fn file_entry(path: &str) -> crate::explorer::FileEntry {
        let path = PathBuf::from(path);
        crate::explorer::FileEntry {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            depth: path.components().count(),
            path,
            is_dir: false,
        }
    }

    fn finish_jobs(app: &mut App) {
        for job in app.take_jobs() {
            let event = crate::jobs::run(job);
            app.handle_event(event);
        }
    }

    #[test]
    fn edits_mark_the_buffer_dirty_until_the_text_matches_the_save() {
        let mut app = App::new(Buffer::from_text("hi"), Some("a.rs".into()));
        assert!(!app.is_dirty());
        app.handle(SemanticAction::InsertChar('!'));
        assert!(app.is_dirty());
        app.handle(SemanticAction::DeleteBackward);
        assert!(!app.is_dirty());
        app.handle(SemanticAction::InsertChar('!'));
        app.handle(SemanticAction::Undo);
        assert!(!app.is_dirty());
    }

    #[test]
    fn save_writes_atomically_and_clears_the_dirty_flag() {
        let (dir, _guard) = temp_dir();
        let path = dir.join("main.rs");
        std::fs::write(&path, "old\n").unwrap();
        let mut app = App::new(Buffer::from_text("old\n"), Some(path.clone()));
        app.handle(SemanticAction::InsertChar('!'));
        app.handle(SemanticAction::Save);
        let jobs = app.take_jobs();
        let save = jobs
            .into_iter()
            .find(|job| matches!(job, Job::SaveFile { .. }))
            .expect("save must leave the UI thread");
        let event = crate::jobs::run(save);
        app.handle_event(event);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "!old\n");
        assert!(!app.is_dirty());
        assert!(
            app.message.as_deref().is_some_and(|message| {
                message.starts_with("Saved ") && message.contains(&path.display().to_string())
            }),
            "{:?}",
            app.message
        );
        app.handle(SemanticAction::Quit);
        assert!(app.should_quit);
    }

    #[test]
    fn relative_save_keeps_the_folder_and_refreshes_an_open_sidebar() {
        let (dir, _guard) = temp_dir();
        let mut app = App::new(Buffer::from_text("hi"), None);
        app.explorer = Explorer::new(dir.clone());
        app.explorer.open = true;

        run(&mut app, "save hello.rs");
        let jobs = app.take_jobs();
        let Some(path) = jobs.iter().find_map(|job| match job {
            Job::SaveFile { path, .. } => Some(path),
            _ => None,
        }) else {
            panic!("expected a save job, got {jobs:?}");
        };
        assert!(path.is_absolute(), "{path:?}");
        assert_eq!(
            path.parent(),
            Some(std::path::absolute(&dir).unwrap().as_path())
        );
        assert!(path.ends_with("hello.rs"));

        let saved = dir.join("note.rs");
        run(&mut app, &format!("save {}", saved.display()));
        finish_jobs(&mut app);
        assert_eq!(app.path(), Some(saved.as_path()));
        let jobs = app.take_jobs();
        assert!(
            jobs.iter()
                .any(|job| matches!(job, Job::ScanTree(root) if root == &dir)),
            "{jobs:?}"
        );
        assert!(
            jobs.iter()
                .any(|job| matches!(job, Job::GitStatus(root) if root == &dir)),
            "{jobs:?}"
        );
    }

    #[test]
    fn quit_asks_before_discarding_unsaved_edits() {
        let (dir, _guard) = temp_dir();
        let path = dir.join("main.rs");
        std::fs::write(&path, "old\n").unwrap();
        let mut app = App::new(Buffer::from_text("old\n"), Some(path.clone()));
        app.handle(SemanticAction::InsertChar('!'));

        app.handle(SemanticAction::Quit);
        assert!(!app.should_quit);
        assert_eq!(app.focus(), Focus::Prompt);
        app.handle(SemanticAction::Prompt(PromptAction::Cancel));
        assert!(!app.should_quit);
        assert!(app.is_dirty());
        assert_eq!(app.buffer().rope().to_string(), "!old\n");

        app.handle(SemanticAction::Quit);
        app.handle(SemanticAction::Prompt(PromptAction::Accelerator('n')));
        assert!(app.should_quit);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\n");

        let mut app = App::new(Buffer::from_text("old\n"), Some(path.clone()));
        app.handle(SemanticAction::InsertChar('!'));
        app.handle(SemanticAction::Quit);
        app.handle(SemanticAction::Prompt(PromptAction::Accelerator('y')));
        finish_jobs(&mut app);
        assert!(app.should_quit);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "!old\n");
    }

    #[test]
    fn dialog_arrows_wrap_and_enter_confirms_the_highlight() {
        let mut app = App::new(Buffer::from_text("hi"), Some("a.rs".into()));
        app.handle(SemanticAction::InsertChar('!'));
        app.handle(SemanticAction::Quit);
        assert_eq!(app.prompt.as_ref().unwrap().selected, 0);
        assert_eq!(
            app.prompt
                .as_ref()
                .unwrap()
                .buttons
                .iter()
                .map(|button| button.label)
                .collect::<Vec<_>>(),
            ["Yes", "No", "Cancel"]
        );

        app.handle(SemanticAction::Prompt(PromptAction::Next));
        app.handle(SemanticAction::Prompt(PromptAction::Next));
        app.handle(SemanticAction::Prompt(PromptAction::Next));
        assert_eq!(app.prompt.as_ref().unwrap().selected, 0);
        app.handle(SemanticAction::Prompt(PromptAction::Prev));
        assert_eq!(app.prompt.as_ref().unwrap().selected, 2);
        app.handle(SemanticAction::Prompt(PromptAction::Confirm));
        assert!(app.prompt.is_none());
        assert!(!app.should_quit);
        assert!(app.is_dirty());
    }

    #[test]
    fn untitled_save_asks_for_a_path() {
        let (dir, _guard) = temp_dir();
        let mut app = App::new(Buffer::new(), None);
        app.handle(SemanticAction::InsertChar('a'));
        app.handle(SemanticAction::Save);
        assert_eq!(app.command_bar.as_ref().unwrap().input, "save ");
        assert!(app.take_jobs().is_empty());

        let path = dir.join("note.txt");
        run(&mut app, &format!("save {}", path.display()));
        finish_jobs(&mut app);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a");
        assert!(!app.is_dirty());
        assert!(app.path().is_some_and(|open| open.ends_with("note.txt")));
    }

    #[test]
    fn quit_walks_every_dirty_editor() {
        let mut app = App::new(Buffer::from_text("a"), None);
        app.handle(SemanticAction::InsertChar('!'));
        run(&mut app, "split-right");
        app.handle(SemanticAction::InsertChar('x'));
        app.handle(SemanticAction::Quit);
        assert_eq!(app.focus(), Focus::Prompt);
        app.handle(SemanticAction::Prompt(PromptAction::Accelerator('n')));
        assert!(!app.should_quit);
        assert_eq!(app.focus(), Focus::Prompt);
        app.handle(SemanticAction::Prompt(PromptAction::Accelerator('n')));
        assert!(app.should_quit);
    }

    #[test]
    fn close_asks_when_the_pane_is_dirty_and_the_last_pane_becomes_blank() {
        let mut app = App::new(Buffer::from_text("alpha"), None);
        app.handle(SemanticAction::InsertChar('!'));
        run(&mut app, "close");
        assert_eq!(app.focus(), Focus::Prompt);
        app.handle(SemanticAction::Prompt(PromptAction::Accelerator('n')));
        assert_eq!(app.buffer().rope().to_string(), "");
        assert!(!app.is_dirty());
        assert!(matches!(app.layout(), Layout::Leaf(_)));

        app.handle(SemanticAction::InsertText("alpha".into()));
        let original = app.active_id();
        run(&mut app, "split-right");
        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Left));
        assert_eq!(app.active_id(), original);
        run(&mut app, "close");
        assert_eq!(app.focus(), Focus::Prompt);
        app.handle(SemanticAction::Prompt(PromptAction::Accelerator('n')));
        assert_ne!(app.active_id(), original);
        assert_eq!(app.buffer().rope().to_string(), "");
    }

    #[test]
    fn close_all_asks_before_discarding_and_leaves_one_blank_page() {
        let (dir, _guard) = temp_dir();
        let path = dir.join("main.rs");
        std::fs::write(&path, "old\n").unwrap();
        let mut app = App::new(Buffer::from_text("old\n"), Some(path.clone()));
        app.handle(SemanticAction::ToggleSidebar);
        run(&mut app, "close-all");
        assert_eq!(
            app.message.as_deref(),
            Some("close-all applies to the editor")
        );
        app.handle(SemanticAction::CommandBar(CommandBarAction::Close));
        app.handle(SemanticAction::Sidebar(SidebarAction::Escape));

        app.handle(SemanticAction::InsertChar('!'));
        run(&mut app, "split-right");
        app.handle(SemanticAction::InsertChar('x'));

        run(&mut app, "close-all");
        assert_eq!(app.focus(), Focus::Prompt);
        assert!(
            app.prompt
                .as_ref()
                .unwrap()
                .message
                .contains("Save changes to [No Name] before closing?")
        );
        app.handle(SemanticAction::Prompt(PromptAction::Cancel));
        assert_eq!(app.buffer().rope().to_string(), "x");
        assert!(app.is_dirty());
        assert!(matches!(app.layout(), Layout::Split { .. }));

        run(&mut app, "close-all");
        app.handle(SemanticAction::Prompt(PromptAction::Accelerator('n')));
        assert_eq!(app.focus(), Focus::Prompt);
        assert!(app.prompt.as_ref().unwrap().message.contains("main.rs"));
        app.handle(SemanticAction::Prompt(PromptAction::Accelerator('y')));
        finish_jobs(&mut app);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "!old\n");
        assert!(matches!(app.layout(), Layout::Leaf(_)));
        assert_eq!(app.buffer().rope().to_string(), "");
        assert!(!app.is_dirty());
        assert!(app.prompt.is_none());
    }

    #[test]
    fn close_all_asks_for_a_path_when_the_dirty_tab_is_untitled() {
        let (dir, _guard) = temp_dir();
        let mut app = App::new(Buffer::new(), None);
        app.handle(SemanticAction::InsertChar('a'));
        run(&mut app, "split-right");
        app.handle(SemanticAction::InsertChar('b'));
        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Left));

        run(&mut app, "close-all");
        app.handle(SemanticAction::Prompt(PromptAction::Accelerator('y')));
        assert_eq!(app.command_bar.as_ref().unwrap().input, "save ");
        let path = dir.join("note.txt");
        run(&mut app, &format!("save {}", path.display()));
        finish_jobs(&mut app);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a");
        assert_eq!(app.focus(), Focus::Prompt);
        app.handle(SemanticAction::Prompt(PromptAction::Accelerator('n')));
        assert!(matches!(app.layout(), Layout::Leaf(_)));
        assert_eq!(app.buffer().rope().to_string(), "");
        assert!(!app.is_dirty());
    }

    #[test]
    fn close_view_closes_only_the_focused_window() {
        let (dir, _guard) = temp_dir();
        std::fs::write(dir.join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.join("lib.rs"), "fn lib() {}\n").unwrap();
        let mut app = App::new(Buffer::new(), None);
        focus_tree(
            &mut app,
            dir,
            vec![file_entry("main.rs"), file_entry("lib.rs")],
        );
        app.handle(SemanticAction::Sidebar(SidebarAction::Activate));
        finish_jobs(&mut app);
        app.explorer.focused = true;
        app.handle(SemanticAction::Sidebar(SidebarAction::Down {
            extend: false,
        }));
        app.handle(SemanticAction::Sidebar(SidebarAction::Activate));
        finish_jobs(&mut app);
        let pane = app.active_id();
        assert_eq!(app.editor(pane).tabs().len(), 2);
        app.handle(SemanticAction::InsertChar('!'));

        run(&mut app, "split-right");
        app.handle(SemanticAction::InsertText("keep".into()));
        let other = app.active_id();
        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Left));
        assert_eq!(app.active_id(), pane);

        run(&mut app, "clean-view");
        assert_eq!(app.focus(), Focus::Prompt);
        assert!(app.prompt.as_ref().unwrap().message.contains("lib.rs"));
        app.handle(SemanticAction::Prompt(PromptAction::Cancel));
        assert_eq!(app.editor(pane).tabs().len(), 2);
        assert_eq!(app.buffer().rope().to_string(), "!fn lib() {}\n");
        assert_eq!(
            app.editor(other).tabs()[0].buffer.rope().to_string(),
            "keep"
        );

        run(&mut app, "close-view");
        app.handle(SemanticAction::Prompt(PromptAction::Accelerator('n')));
        assert_eq!(app.active_id(), other);
        assert!(matches!(app.layout(), Layout::Leaf(_)));
        assert_eq!(app.buffer().rope().to_string(), "keep");
        assert!(app.is_dirty());
        assert!(app.prompt.is_none());

        run(&mut app, "close-view");
        assert!(app.prompt.as_ref().unwrap().message.contains("[No Name]"));
        app.handle(SemanticAction::Prompt(PromptAction::Accelerator('n')));
        assert!(matches!(app.layout(), Layout::Leaf(_)));
        assert_eq!(app.buffer().rope().to_string(), "");
        assert!(!app.is_dirty());
    }

    #[test]
    fn cut_and_paste_round_trip_through_the_clipboard() {
        let mut app = App::new(Buffer::from_text("ab\ncd"), None);
        app.handle(SemanticAction::Cut);
        assert_eq!(app.buffer().rope().to_string(), "cd");
        assert_eq!(app.message.as_deref(), Some("Cut 3 characters"));
        app.handle(SemanticAction::Paste);
        assert_eq!(app.buffer().rope().to_string(), "ab\ncd");
        assert!(!app.is_dirty());
    }

    #[test]
    fn sidebar_opens_a_file_and_reuses_an_editor_that_already_has_it() {
        let (dir, _guard) = temp_dir();
        let path = dir.join("main.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();

        let mut app = App::new(Buffer::new(), None);
        app.explorer = Explorer::new(dir.clone());
        app.explorer.open = true;
        app.explorer.focused = true;
        app.handle_event(AppEvent::TreeScanned {
            root: dir.clone(),
            entries: vec![file_entry("main.rs")],
        });
        app.handle(SemanticAction::Sidebar(SidebarAction::Activate));
        finish_jobs(&mut app);
        assert_eq!(app.buffer().rope().to_string(), "fn main() {}\n");
        assert!(!app.is_dirty());
        assert_eq!(app.focus(), Focus::Editor);
        assert!(app.path().is_some_and(|open| open.ends_with("main.rs")));
        // The open queued a syntax parse. The next assert checks that focusing
        // the same file does not load it again.
        let _ = app.take_jobs();

        app.handle(SemanticAction::ToggleSidebar);
        app.handle(SemanticAction::Sidebar(SidebarAction::Activate));
        assert!(app.take_jobs().is_empty());
        assert_eq!(app.buffer().rope().to_string(), "fn main() {}\n");
        assert_eq!(app.focus(), Focus::Editor);
    }

    #[test]
    fn each_pane_keeps_its_own_tabs_and_close_removes_one_tab() {
        let (dir, _guard) = temp_dir();
        std::fs::write(dir.join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.join("lib.rs"), "fn lib() {}\n").unwrap();
        let mut app = App::new(Buffer::new(), None);
        focus_tree(
            &mut app,
            dir.clone(),
            vec![file_entry("main.rs"), file_entry("lib.rs")],
        );
        app.handle(SemanticAction::Sidebar(SidebarAction::Activate));
        finish_jobs(&mut app);
        assert_eq!(app.editor(app.active_id()).tabs().len(), 1);

        app.explorer.focused = true;
        app.handle(SemanticAction::Sidebar(SidebarAction::Down {
            extend: false,
        }));
        app.handle(SemanticAction::Sidebar(SidebarAction::Activate));
        finish_jobs(&mut app);
        let pane = app.active_id();
        assert_eq!(app.editor(pane).tabs().len(), 2);
        assert_eq!(app.editor(pane).active_index(), 1);
        assert!(app.path().is_some_and(|path| path.ends_with("lib.rs")));

        app.handle(SemanticAction::CycleTab { next: false });
        assert!(app.path().is_some_and(|path| path.ends_with("main.rs")));
        assert_eq!(app.buffer().rope().to_string(), "fn main() {}\n");
        run(&mut app, "tab-next");
        assert!(app.path().is_some_and(|path| path.ends_with("lib.rs")));

        run(&mut app, "split-right");
        let other = app.active_id();
        assert_ne!(other, pane);
        assert_eq!(app.editor(other).tabs().len(), 1);
        assert_eq!(app.editor(pane).tabs().len(), 2);

        app.handle(SemanticAction::FocusAdjacent(PaneDirection::Left));
        run(&mut app, "close");
        assert_eq!(app.active_id(), pane);
        assert_eq!(app.editor(pane).tabs().len(), 1);
        assert!(app.path().is_some_and(|path| path.ends_with("main.rs")));
    }

    #[test]
    fn opening_a_dirty_tab_keeps_it_and_an_edit_cancels_a_load() {
        let (dir, _guard) = temp_dir();
        let path = dir.join("main.rs");
        std::fs::write(&path, "from disk\n").unwrap();
        let mut app = App::new(Buffer::from_text("local"), None);
        app.handle(SemanticAction::InsertChar('!'));
        focus_tree(&mut app, dir, vec![file_entry("main.rs")]);

        app.handle(SemanticAction::Sidebar(SidebarAction::Activate));
        assert_ne!(app.focus(), Focus::Prompt);
        assert_eq!(app.editor(app.active_id()).tabs().len(), 2);
        app.handle(SemanticAction::CycleTab { next: false });
        assert_eq!(app.buffer().rope().to_string(), "!local");
        app.handle(SemanticAction::CycleTab { next: true });
        assert_eq!(app.buffer().rope().to_string(), "");

        let jobs = app.take_jobs();
        app.handle(SemanticAction::InsertChar('x'));
        let event = crate::jobs::run(jobs.into_iter().next().unwrap());
        app.handle_event(event);
        assert_eq!(app.buffer().rope().to_string(), "x");
        assert_eq!(
            app.message.as_deref(),
            Some("Open cancelled because the buffer changed")
        );

        app.handle(SemanticAction::Undo);
        assert_eq!(app.buffer().rope().to_string(), "");
        run(&mut app, "close");
        assert_eq!(app.editor(app.active_id()).tabs().len(), 1);
        assert_eq!(app.buffer().rope().to_string(), "!local");
        assert!(app.is_dirty());
    }

    #[test]
    fn a_clean_untitled_tab_is_replaced_unless_it_changes_first() {
        let (dir, _guard) = temp_dir();
        std::fs::write(dir.join("main.rs"), "from disk\n").unwrap();
        let mut app = App::new(Buffer::from_text("local"), None);
        focus_tree(&mut app, dir, vec![file_entry("main.rs")]);
        app.handle(SemanticAction::Sidebar(SidebarAction::Activate));
        assert_eq!(app.editor(app.active_id()).tabs().len(), 1);
        let jobs = app.take_jobs();
        app.handle(SemanticAction::InsertChar('x'));
        let event = crate::jobs::run(jobs.into_iter().next().unwrap());
        app.handle_event(event);
        assert_eq!(app.buffer().rope().to_string(), "xlocal");
        assert_eq!(
            app.message.as_deref(),
            Some("Open cancelled because the buffer changed")
        );
    }

    fn focus_tree(app: &mut App, root: PathBuf, entries: Vec<crate::explorer::FileEntry>) {
        app.explorer = Explorer::new(root);
        app.explorer.open = true;
        app.explorer.focused = true;
        app.explorer.set_entries(entries);
    }

    #[test]
    fn reveal_selects_the_focused_file_without_moving_the_cursor() {
        let (dir, _guard) = temp_dir();
        std::fs::create_dir(dir.join("src")).unwrap();
        let file = dir.join("src").join("lib.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let mut app = App::new(Buffer::from_text("fn main() {}\n"), Some(file));
        app.handle(SemanticAction::Move {
            movement: Movement::Right,
            extend: false,
        });
        let cursor = app.buffer().selection().head;

        app.explorer = Explorer::new(dir.clone());
        let mut src = file_entry("src");
        src.is_dir = true;
        app.explorer
            .set_entries(vec![src, file_entry("src/lib.rs"), file_entry("one.txt")]);
        app.explorer.insert_filter("one");
        app.explorer.enter_select();
        app.explorer.commit_mark();
        app.explorer.open = false;

        app.run_command(CommandId::Reveal, None).unwrap();
        assert!(app.explorer.open);
        assert!(!app.explorer.focused);
        assert_eq!(app.focus(), Focus::Editor);
        assert_eq!(app.buffer().selection().head, cursor);
        assert!(app.explorer.filter.is_empty());
        assert!(app.explorer.selecting);
        assert!(app.explorer.is_marked(Path::new("one.txt")));
        assert_eq!(
            app.explorer.selected().unwrap(),
            Path::new("src").join("lib.rs")
        );
        assert!(
            app.explorer
                .expanded_paths()
                .iter()
                .any(|path| path == Path::new("src"))
        );
        assert!(app.reveal_file.is_none());
    }

    #[test]
    fn reveal_waits_for_the_scan_and_reports_a_missing_file() {
        let (dir, _guard) = temp_dir();
        std::fs::create_dir(dir.join("src")).unwrap();
        let file = dir.join("src").join("lib.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let mut app = App::new(Buffer::from_text("fn main() {}\n"), Some(file));
        app.explorer = Explorer::new(dir.clone());
        app.explorer.open = true;
        app.explorer.loading = true;

        app.run_command(CommandId::Reveal, None).unwrap();
        assert!(app.message.is_none());
        assert_eq!(
            app.reveal_file.as_deref(),
            Some(Path::new("src").join("lib.rs").as_path())
        );

        let mut src = file_entry("src");
        src.is_dir = true;
        app.handle_event(AppEvent::TreeScanned {
            root: dir.clone(),
            entries: vec![src, file_entry("src/lib.rs")],
        });
        assert!(app.reveal_file.is_none());
        assert_eq!(
            app.explorer.selected().unwrap(),
            Path::new("src").join("lib.rs")
        );
        assert!(app.message.is_none());

        app.explorer.loading = false;
        app.explorer.set_entries(vec![file_entry("one.txt")]);
        let error = app.run_command(CommandId::Reveal, None).unwrap_err();
        assert_eq!(error, "File is not in the explorer");

        let mut unnamed = App::new(Buffer::from_text("draft"), None);
        unnamed.explorer = Explorer::new(dir);
        let error = unnamed.run_command(CommandId::Reveal, None).unwrap_err();
        assert_eq!(error, "No file to reveal");

        app.run_command(CommandId::Focus, None).unwrap();
        app.explorer.open = false;
        app.run_command(CommandId::Reveal, None).unwrap();
        assert!(!app.explorer.open);
        assert!(app.focus_mode());
    }

    #[test]
    fn command_bar_only_offers_the_focused_surface() {
        let mut app = App::new(Buffer::from_text("one\ntwo\nthree\n"), None);
        run(&mut app, "delete");
        assert_eq!(
            app.message.as_deref(),
            Some("delete applies to the explorer")
        );
        assert_eq!(app.focus(), Focus::CommandBar);

        app.command_bar = None;
        app.explorer.open = true;
        app.explorer.focused = true;
        run(&mut app, "goto 2");
        assert_eq!(app.message.as_deref(), Some("goto applies to the editor"));
        assert_eq!(app.buffer().cursor_line_col().0, 0);

        app.command_bar = None;
        app.handle(SemanticAction::OpenCommandBar);
        let names: Vec<_> = crate::command::suggestions(
            "",
            app.recent_for(crate::command::CommandContext::Explorer),
            crate::command::CommandContext::Explorer,
            false,
        )
        .into_iter()
        .map(|spec| spec.name)
        .collect();
        assert_eq!(
            names,
            [
                "file-picker",
                "workspace",
                "find",
                "replace",
                "select",
                "reveal",
                "explorer",
                "wider",
                "narrower",
                "setting",
                "about",
                "quit"
            ]
        );
    }

    #[test]
    fn setting_and_config_open_the_same_page() {
        let mut app = App::new(Buffer::new(), None);
        run(&mut app, "setting");
        assert_eq!(app.focus(), Focus::Settings);
        assert_eq!(
            app.settings
                .as_ref()
                .map(|page| (page.section, page.row, page.inside)),
            Some((0, 0, false))
        );
        assert_eq!(app.recent_command, Some(CommandId::Settings));

        app.handle(SemanticAction::Settings(SettingsAction::Down));
        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        assert_eq!(
            app.settings
                .as_ref()
                .map(|page| (page.section, page.row, page.inside)),
            Some((1, 0, true))
        );
        app.handle(SemanticAction::Settings(SettingsAction::Back));
        assert_eq!(app.settings.as_ref().map(|page| page.inside), Some(false));
        app.handle(SemanticAction::Settings(SettingsAction::Back));
        assert_eq!(app.focus(), Focus::Editor);

        app.explorer.open = true;
        app.explorer.focused = true;
        run(&mut app, "config");
        assert_eq!(app.focus(), Focus::Settings);
        app.handle(SemanticAction::Settings(SettingsAction::Back));
        assert_eq!(app.focus(), Focus::Sidebar);
    }

    #[test]
    fn about_opens_over_either_surface_and_esc_closes_it() {
        let mut app = App::new(Buffer::from_text("keep\n"), None);
        app.settings = Some(crate::settings::Page::new());
        run(&mut app, "about");
        assert!(app.about);
        assert!(app.settings.is_none());
        assert_eq!(app.focus(), Focus::About);
        assert_eq!(app.recent_command, Some(CommandId::About));
        assert_eq!(app.buffer().rope().to_string(), "keep\n");

        app.handle(SemanticAction::CloseAbout);
        assert!(!app.about);
        assert_eq!(app.focus(), Focus::Editor);
        assert_eq!(app.buffer().rope().to_string(), "keep\n");

        app.explorer.open = true;
        app.explorer.focused = true;
        run(&mut app, "about");
        assert_eq!(app.focus(), Focus::About);
        assert_eq!(app.recent_explorer, Some(CommandId::About));
        app.handle(SemanticAction::CloseAbout);
        assert_eq!(app.focus(), Focus::Sidebar);
    }

    #[test]
    fn appearance_cycles_themes_and_saves_settings_json() {
        let (dir, _guard) = temp_dir();
        let mut app = App::new(Buffer::new(), None);
        app.load_appearance(&dir);
        app.settings = Some(crate::settings::Page::new());
        app.handle(SemanticAction::Settings(SettingsAction::Down));
        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        assert!(
            app.settings
                .as_ref()
                .is_some_and(|page| page.menu.is_some())
        );
        assert_eq!(app.appearance.color.id, "tokyo-night");
        app.handle(SemanticAction::Settings(SettingsAction::Down));
        assert_eq!(app.appearance.color.id, "catppuccin-latte");
        app.handle(SemanticAction::Settings(SettingsAction::Back));
        assert_eq!(app.appearance.color.id, "tokyo-night");
        assert!(app.take_jobs().is_empty());

        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        app.handle(SemanticAction::Settings(SettingsAction::Down));
        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        assert_eq!(app.appearance.color.id, "catppuccin-latte");
        assert_eq!(app.appearance.syntax.id, "tokyo-night");
        finish_jobs(&mut app);
        app.handle(SemanticAction::Settings(SettingsAction::Down));
        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        app.handle(SemanticAction::Settings(SettingsAction::Down));
        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        assert_eq!(app.appearance.syntax.id, "catppuccin-latte");
        finish_jobs(&mut app);

        let saved = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        assert!(
            saved.contains("\"color_theme\": \"catppuccin-latte\""),
            "{saved}"
        );
        assert!(
            saved.contains("\"syntax_theme\": \"catppuccin-latte\""),
            "{saved}"
        );
        let (loaded, _file, warning) = crate::theme::Appearance::load(&dir);
        assert!(warning.is_none(), "{warning:?}");
        assert_eq!(loaded.color.id, "catppuccin-latte");
        assert_eq!(loaded.syntax.id, "catppuccin-latte");
    }

    #[test]
    fn editor_tab_width_is_saved_and_delete_restores_it() {
        let (dir, _guard) = temp_dir();
        let mut app = App::new(Buffer::from_text("ab"), None);
        app.load_appearance(&dir);
        app.settings = Some(crate::settings::Page::new());
        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        app.handle(SemanticAction::Settings(SettingsAction::Down));
        app.handle(SemanticAction::Settings(SettingsAction::Enter));
        assert_eq!(app.editor_prefs.tab_width, 8);
        app.buffer_mut().indent();
        assert_eq!(app.buffer().rope().to_string(), "        ab");
        finish_jobs(&mut app);
        let saved = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        assert!(saved.contains("\"tab_width\": 8"), "{saved}");

        app.handle(SemanticAction::Settings(SettingsAction::Reset));
        assert_eq!(app.editor_prefs.tab_width, 4);
        finish_jobs(&mut app);
        let saved = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        assert!(saved.contains("\"tab_width\": 4"), "{saved}");
        assert!(
            !saved.contains("\"keybindings\""),
            "default chords are not written: {saved}"
        );
    }

    #[test]
    fn explorer_rename_and_move_retarget_the_open_file() {
        let (dir, _guard) = temp_dir();
        let hello = dir.join("hello.rs");
        let src = dir.join("src");
        std::fs::write(&hello, "hi\n").unwrap();
        std::fs::create_dir(&src).unwrap();

        let mut src_entry = file_entry("src");
        src_entry.is_dir = true;
        let mut app = App::new(Buffer::from_text("hi\n"), Some(hello.clone()));
        focus_tree(
            &mut app,
            dir.clone(),
            vec![file_entry("hello.rs"), src_entry],
        );
        run(&mut app, "select");

        run(&mut app, "rename notes.rs");
        finish_jobs(&mut app);
        assert!(dir.join("notes.rs").is_file(), "renamed file should exist");
        assert!(!hello.exists());
        assert!(app.path().is_some_and(|path| path.ends_with("notes.rs")));
        assert!(
            app.message
                .as_deref()
                .is_some_and(|message| message.contains("Renamed") && message.contains("notes.rs")),
            "{:?}",
            app.message
        );
        assert_eq!(app.explorer.selected(), Some(Path::new("notes.rs")));

        run(&mut app, "move src");
        finish_jobs(&mut app);
        assert!(src.join("notes.rs").is_file());
        assert!(!dir.join("notes.rs").exists());
        assert!(app.path().is_some_and(|path| path.ends_with("notes.rs")));
        assert!(
            app.message
                .as_deref()
                .is_some_and(|message| message.contains("Moved")),
            "{:?}",
            app.message
        );
    }

    #[test]
    fn explorer_delete_confirms_and_keeps_unsaved_editor_text() {
        let (dir, _guard) = temp_dir();
        let hello = dir.join("hello.rs");
        std::fs::write(&hello, "hi\n").unwrap();
        let mut app = App::new(Buffer::from_text("hi\n"), Some(hello.clone()));
        focus_tree(&mut app, dir.clone(), vec![file_entry("hello.rs")]);
        run(&mut app, "select");

        app.handle(SemanticAction::InsertChar('!'));
        run(&mut app, "delete");
        assert!(
            app.message
                .as_deref()
                .is_some_and(|message| message.contains("unsaved")),
            "{:?}",
            app.message
        );
        assert!(hello.is_file());
        assert!(app.prompt.is_none());

        app.handle(SemanticAction::Undo);
        run(&mut app, "delete");
        assert_eq!(app.focus(), Focus::Prompt);
        assert!(app.prompt.as_ref().unwrap().message.contains("hello.rs"));
        app.handle(SemanticAction::Prompt(PromptAction::Cancel));
        assert_eq!(app.message.as_deref(), Some("Delete cancelled"));
        assert!(hello.is_file());
        assert_eq!(app.focus(), Focus::Sidebar);

        run(&mut app, "delete");
        app.handle(SemanticAction::Prompt(PromptAction::Accelerator('y')));
        finish_jobs(&mut app);
        assert!(!hello.exists());
        assert_eq!(app.buffer().rope().to_string(), "hi\n");
        assert!(app.is_dirty());
        assert!(
            app.message
                .as_deref()
                .is_some_and(|message| message.contains("Deleted")),
            "{:?}",
            app.message
        );
    }

    #[test]
    fn select_mode_marks_with_space_and_leaves_on_esc() {
        let mut app = App::new(Buffer::new(), None);
        focus_tree(
            &mut app,
            PathBuf::from("workspace"),
            vec![file_entry("a.rs"), file_entry("b.rs"), file_entry("c.rs")],
        );

        run(&mut app, "select");
        assert!(app.explorer.selecting);
        assert_eq!(app.focus(), Focus::Sidebar);
        assert_eq!(app.explorer.marked_count(), 0);

        app.handle(SemanticAction::Sidebar(SidebarAction::Mark));
        assert!(app.explorer.is_marked(Path::new("a.rs")));
        assert!(!app.explorer.is_marked(Path::new("b.rs")));

        app.handle(SemanticAction::Sidebar(SidebarAction::Down {
            extend: true,
        }));
        app.handle(SemanticAction::Sidebar(SidebarAction::Down {
            extend: true,
        }));
        assert!(app.explorer.is_covered(Path::new("b.rs")));
        assert!(app.explorer.is_covered(Path::new("c.rs")));
        assert!(!app.explorer.is_marked(Path::new("c.rs")));

        app.handle(SemanticAction::Sidebar(SidebarAction::Mark));
        assert!(app.explorer.is_marked(Path::new("a.rs")));
        assert!(app.explorer.is_marked(Path::new("b.rs")));
        assert!(app.explorer.is_marked(Path::new("c.rs")));
        assert!(!app.explorer.is_covered(Path::new("a.rs")));
        assert_eq!(app.explorer.targets().len(), 3);

        app.handle(SemanticAction::Sidebar(SidebarAction::FilterInsert(
            "x".into(),
        )));
        app.handle(SemanticAction::Sidebar(SidebarAction::ExitSelect));
        assert!(!app.explorer.selecting);
        assert_eq!(app.focus(), Focus::Sidebar);
        assert_eq!(app.explorer.filter, "x");
        assert_eq!(app.explorer.marked_count(), 0);
        assert!(!app.explorer.is_marked(Path::new("a.rs")));
        assert_eq!(app.message.as_deref(), Some("Exited FILE"));

        app.handle(SemanticAction::Sidebar(SidebarAction::Escape));
        assert!(app.explorer.filter.is_empty());
        assert_eq!(app.focus(), Focus::Sidebar);

        run(&mut app, "delete");
        assert_eq!(app.message.as_deref(), Some("delete applies in FILE mode"));

        run(&mut app, "select");
        assert!(app.explorer.selecting);
        app.handle(SemanticAction::Sidebar(SidebarAction::Mark));
        app.handle(SemanticAction::OpenCommandBar);
        assert!(app.command_bar.as_ref().is_some_and(|bar| bar.files));
        app.command_bar = None;
        app.handle(SemanticAction::ToggleSidebar);
        assert!(!app.explorer.open);
        assert!(!app.explorer.selecting);
        assert_eq!(app.explorer.marked_count(), 0);
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("duckx-app-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn opened(path: &Path, text: &str) -> App {
        let mut app = App::new(Buffer::from_text(text), Some(path.to_path_buf()));
        let stamp = crate::services::encoding::file_stamp(path).unwrap();
        app.note_opened(TextEncoding::Utf8, stamp);
        app
    }

    #[test]
    fn a_clean_file_reloads_and_keeps_the_cursor_line() {
        let dir = Scratch::new();
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "alpha\nbeta\n").unwrap();
        let mut app = opened(&path, "alpha\nbeta\n");
        app.buffer_mut().goto_line(1);

        std::fs::write(&path, "alpha\nBETA\ngamma\n").unwrap();
        app.watch_files();
        finish_jobs(&mut app);
        finish_jobs(&mut app);

        assert_eq!(app.buffer().rope().to_string(), "alpha\nBETA\ngamma\n");
        assert_eq!(app.buffer().cursor_line_char().0, 1);
        assert!(!app.is_dirty());
        assert_eq!(app.encoding_label(), "UTF-8");
    }

    #[test]
    fn a_dirty_file_asks_before_reload_and_keep_holds_the_text() {
        let dir = Scratch::new();
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "one\n").unwrap();
        let mut app = opened(&path, "one\n");
        app.handle(SemanticAction::InsertChar('!'));

        std::fs::write(&path, "two two\n").unwrap();
        app.watch_files();
        finish_jobs(&mut app);
        assert_eq!(app.focus(), Focus::Prompt);
        assert!(
            app.take_jobs()
                .iter()
                .all(|job| !matches!(job, Job::LoadFile { .. }))
        );

        app.handle(SemanticAction::Prompt(PromptAction::Cancel));
        assert_eq!(app.buffer().rope().to_string(), "!one\n");
        assert_eq!(app.message.as_deref(), Some("Kept the editor text"));

        app.watch_files();
        finish_jobs(&mut app);
        assert!(app.prompt.is_none());
        assert_eq!(app.buffer().rope().to_string(), "!one\n");
    }

    #[test]
    fn encoding_is_what_the_next_save_writes() {
        let dir = Scratch::new();
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "hi\n").unwrap();
        let mut app = opened(&path, "hi\n");
        run(&mut app, "encoding utf-16le");
        assert_eq!(app.encoding_label(), "UTF-16 LE");

        app.handle(SemanticAction::Save);
        finish_jobs(&mut app);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xFE]);
        let decoded = crate::services::encoding::read_auto(&path).unwrap();
        assert_eq!(decoded.text, "hi\n");
        assert_eq!(decoded.encoding, TextEncoding::Utf16Le);
    }

    #[test]
    fn reload_with_an_encoding_reads_bytes_that_are_not_utf8() {
        let dir = Scratch::new();
        let path = dir.path().join("thai.txt");
        std::fs::write(&path, [0xA1]).unwrap();
        let mut app = App::new(Buffer::new(), Some(path.clone()));
        app.mark_unreadable("Cannot open".into());

        run(&mut app, "reload windows-874");
        finish_jobs(&mut app);
        assert_eq!(app.buffer().rope().to_string(), "ก");
        assert_eq!(app.encoding_label(), "Windows-874");
        assert!(!app.is_dirty());
    }

    #[test]
    fn a_character_the_encoding_cannot_store_is_not_written() {
        let dir = Scratch::new();
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "hi\n").unwrap();
        let mut app = opened(&path, "hi\n");
        run(&mut app, "encoding windows-874");
        app.handle(SemanticAction::InsertChar('🙂'));
        app.handle(SemanticAction::Save);
        finish_jobs(&mut app);
        assert_eq!(std::fs::read(&path).unwrap(), b"hi\n");
        assert!(
            app.message
                .as_deref()
                .is_some_and(|message| message.contains("cannot represent")),
            "{:?}",
            app.message
        );
        assert!(app.is_dirty());
    }

    fn leaf_session(root: &Path, file: &str) -> crate::session::Session {
        crate::session::Session {
            version: crate::session::VERSION,
            root: root.to_path_buf(),
            sidebar: crate::session::Sidebar {
                open: true,
                width: 30,
                expanded: Vec::new(),
            },
            sidebar_focused: false,
            layout: crate::session::SessionLayout::Leaf {
                selected: true,
                active: 0,
                tabs: vec![crate::session::SessionTab {
                    path: file.to_string(),
                    line: 0,
                    col: 0,
                }],
            },
        }
    }

    #[test]
    fn workspace_opens_the_saved_folder_and_drops_the_old_tree() {
        let (left, _left_guard) = temp_dir();
        let (right, _right_guard) = temp_dir();
        std::fs::write(right.join("note.txt"), "from the other folder\n").unwrap();
        let mut app = App::new(Buffer::from_text("here\n"), None);
        app.explorer = Explorer::new(left.clone());
        app.explorer.open = true;
        app.sessions.push(leaf_session(&right, "note.txt"));

        run(&mut app, &format!("workspace {}", right.display()));
        assert_eq!(app.explorer.root, std::path::absolute(&right).unwrap());
        let opened = app.message.clone();
        assert!(
            opened
                .as_deref()
                .is_some_and(|message| message.starts_with("Opened ")),
            "{opened:?}"
        );
        assert!(crate::session::same_directory(&app.sessions[0].root, &left));

        let requests = app.take_lsp_requests();
        assert!(matches!(
            requests.first(),
            Some(crate::lsp::LspRequest::Reset)
        ));
        app.handle_lsp(crate::lsp::LspEvent::Notice("rust-analyzer stopped".into()));
        assert_eq!(app.message, opened);
        app.handle_lsp(crate::lsp::LspEvent::ResetDone);

        app.handle_event(AppEvent::TreeScanned {
            root: left,
            entries: vec![file_entry("secret.txt")],
        });
        assert!(
            app.explorer
                .entries()
                .iter()
                .all(|entry| entry.name != "secret.txt")
        );

        finish_jobs(&mut app);
        assert_eq!(app.buffer().rope().to_string(), "from the other folder\n");
    }

    #[test]
    fn workspace_asks_before_discarding_unsaved_edits() {
        let (left, _left_guard) = temp_dir();
        let (right, _right_guard) = temp_dir();
        let mut app = App::new(Buffer::from_text("old\n"), None);
        app.explorer = Explorer::new(left.clone());
        app.handle(SemanticAction::InsertChar('!'));

        run(&mut app, &format!("workspace {}", right.display()));
        assert_eq!(app.focus(), Focus::Prompt);
        assert_eq!(app.explorer.root, left);
        app.handle(SemanticAction::Prompt(PromptAction::Cancel));
        assert_eq!(app.explorer.root, left);
        assert!(app.is_dirty());
        assert_eq!(app.buffer().rope().to_string(), "!old\n");

        run(&mut app, &format!("workspace {}", right.display()));
        app.handle(SemanticAction::Prompt(PromptAction::Accelerator('n')));
        assert_eq!(app.explorer.root, std::path::absolute(&right).unwrap());
        assert_eq!(app.buffer().rope().to_string(), "");
    }

    #[test]
    fn workspace_rejects_the_current_folder_and_a_missing_path() {
        let (left, _left_guard) = temp_dir();
        let mut app = App::new(Buffer::new(), None);
        app.explorer = Explorer::new(left.clone());

        run(&mut app, &format!("workspace {}", left.display()));
        assert_eq!(app.message.as_deref(), Some("Already here"));
        assert_eq!(app.explorer.root, left);

        run(&mut app, "workspace missing-folder");
        assert!(
            app.message
                .as_deref()
                .is_some_and(|message| message.starts_with("Not a folder")),
            "{:?}",
            app.message
        );
        assert_eq!(app.explorer.root, left);

        let child = left.join("child");
        std::fs::create_dir(&child).unwrap();
        run(&mut app, "workspace child");
        assert_eq!(app.explorer.root, std::path::absolute(&child).unwrap());
    }

    #[test]
    fn workspace_lists_saved_folders_and_writes_the_one_being_left() {
        let (left, _left_guard) = temp_dir();
        let (right, _right_guard) = temp_dir();
        let (config, _config_guard) = temp_dir();
        std::fs::write(left.join("stay.txt"), "stay\n").unwrap();
        let mut app = App::new(Buffer::from_text("stay\n"), Some(left.join("stay.txt")));
        app.explorer = Explorer::new(left.clone());
        app.load_appearance(&config);
        app.sessions.push(leaf_session(&right, "note.txt"));

        app.handle(SemanticAction::OpenCommandBar);
        type_in_bar(&mut app, "workspace");
        bar(&mut app, CommandBarAction::Submit);
        assert_eq!(
            app.command_bar.as_ref().map(|bar| bar.input.as_str()),
            Some("workspace ")
        );
        let choices = app.workspace_choices().unwrap();
        assert_eq!(choices, vec![right.display().to_string()]);
        bar(&mut app, CommandBarAction::Submit);

        assert_eq!(app.explorer.root, std::path::absolute(&right).unwrap());
        let saved = app.take_jobs().into_iter().find_map(|job| match job {
            Job::SaveSession { path, session } => Some((path, session)),
            _ => None,
        });
        let (path, session) = saved.expect("session write");
        assert!(crate::session::same_directory(&session.root, &left));
        app.handle_event(crate::jobs::run(Job::SaveSession { path, session }));
        let text = std::fs::read_to_string(config.join("session.json")).unwrap();
        assert!(text.contains("stay.txt"), "{text}");
    }

    #[test]
    fn workspace_drops_the_old_file_and_a_late_load_cannot_bring_it_back() {
        let (left, _left_guard) = temp_dir();
        let (right, _right_guard) = temp_dir();
        std::fs::write(right.join("here.txt"), "new folder\n").unwrap();
        let old = left.join("AGENTS.md");
        std::fs::write(&old, "from the first folder\n").unwrap();
        let mut app = opened(&old, "from the first folder\n");
        app.explorer = Explorer::new(left);
        let tab_id = app.active_tab_id();

        run(&mut app, &format!("workspace {}", right.display()));
        assert!(app.path().is_none(), "{:?}", app.path());
        assert_eq!(app.buffer().rope().to_string(), "");

        app.handle_event(AppEvent::FileLoaded {
            editor_id: crate::layout::EditorId(0),
            tab_id,
            path: old.clone(),
            generation: 0,
            kind: crate::jobs::LoadKind::Open {
                encoding: None,
                place: None,
                focus: true,
            },
            result: Ok(crate::services::encoding::Decoded {
                text: "from the first folder\n".to_string(),
                encoding: TextEncoding::Utf8,
                stamp: crate::services::encoding::FileStamp {
                    modified: std::time::SystemTime::UNIX_EPOCH,
                    len: 0,
                },
            }),
        });
        assert!(app.path().is_none(), "{:?}", app.path());
        assert_eq!(app.buffer().rope().to_string(), "");

        run(&mut app, "save note.txt");
        let saved = app.take_jobs().into_iter().find_map(|job| match job {
            Job::SaveFile { path, .. } => Some(path),
            _ => None,
        });
        assert_eq!(
            saved.as_deref().and_then(|path| path.parent()),
            Some(std::path::absolute(&right).unwrap().as_path())
        );
    }
}
