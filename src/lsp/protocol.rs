//! Messages between `App` and the language server worker. Positions here are
//! rope byte offsets; the worker converts them to the encoding each server chose.

use std::path::PathBuf;

use ropey::Rope;

use crate::syntax::Language;

/// `App` -> worker.
#[derive(Debug, Clone)]
pub enum LspRequest {
    /// A file tab became eligible for a language server. `workspace` is the
    /// directory duckx-code treats as the project; servers for files inside it use it as their root.
    Open {
        path: PathBuf,
        language: Language,
        text: Rope,
        version: u64,
        workspace: PathBuf,
    },
    /// `text` is the whole buffer after the change. Cloning a rope is cheap.
    Change {
        path: PathBuf,
        text: Rope,
        version: u64,
        change: TextChange,
    },
    Save {
        path: PathBuf,
    },
    Close {
        path: PathBuf,
    },
    /// Stop every server. The worker exits after this.
    Shutdown,
    /// Settings the next `workspace/configuration` answer should use.
    Configure(crate::settings::LspPrefs),
    /// Ask the server about one cursor position. `token` is echoed on the reply.
    Query {
        token: u64,
        path: PathBuf,
        /// Byte offset in the text the server has already been sent.
        at: usize,
        kind: QueryKind,
    },
    /// Drop a query that is no longer wanted. A request already sent is cancelled.
    Cancel {
        token: u64,
    },
}

/// What [`LspRequest::Query`] asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryKind {
    Hover,
    Definition,
    Completion {
        trigger: CompletionTrigger,
        /// `Ctrl+Space` is sent at once. Typing waits so a burst becomes one request.
        immediate: bool,
    },
    /// `textDocument/signatureHelp` at the cursor.
    Signature,
    /// `completionItem/resolve`. `item` is the completion item JSON.
    Resolve {
        item: String,
    },
    /// `textDocument/rename`.
    Rename {
        new_name: String,
    },
    /// `textDocument/formatting` for the whole file.
    Format,
    /// `textDocument/references` at the cursor.
    References,
    /// `textDocument/codeAction` at the cursor.
    CodeAction,
    /// `codeAction/resolve`. `action` is the action JSON.
    CodeActionResolve {
        action: String,
    },
}

/// How a completion request was started. Matches the protocol's trigger kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionTrigger {
    /// Typed an identifier, or asked with `Ctrl+Space`.
    Invoked,
    /// Typed one of the server's trigger characters.
    Character(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextChange {
    /// The buffer was replaced; send all of it.
    Full,
    /// Bytes `start..old_end` of the previous text became `start..new_end` of the new text.
    Range {
        start: usize,
        old_end: usize,
        new_end: usize,
    },
}

/// Worker -> `App`.
#[derive(Debug, Clone, PartialEq)]
pub enum LspEvent {
    /// The full set `publishDiagnostics` sent for one file.
    /// `version` is the document version the byte offsets refer to.
    Diagnostics {
        path: PathBuf,
        version: u64,
        diagnostics: Vec<Diagnostic>,
    },
    /// Native diagnostics from `textDocument/diagnostic`. Kept beside [`LspEvent::Diagnostics`].
    PulledDiagnostics {
        path: PathBuf,
        version: u64,
        diagnostics: Vec<Diagnostic>,
    },
    /// `group` names the server family (the first candidate in the registry);
    /// `name` is the program actually running.
    Status {
        group: &'static str,
        name: &'static str,
        status: ServerStatus,
    },
    /// Something the user should read in the status bar.
    Notice(String),
    /// What the server offered in `initialize`. Sent once, when it becomes ready.
    Features {
        group: &'static str,
        hover: bool,
        definition: bool,
        completion: bool,
        /// Characters that should open completion, such as `.` or `:`.
        triggers: Vec<String>,
        /// `completionItem/resolve` for documentation that was not in the list.
        resolve: bool,
        /// `textDocument/signatureHelp`.
        signature: bool,
        /// `textDocument/rename`.
        rename: bool,
        /// `textDocument/formatting`.
        format: bool,
        /// `textDocument/references`.
        references: bool,
        /// `textDocument/codeAction`.
        code_action: bool,
        /// `codeAction/resolve`, so an action without an edit can be filled in.
        action_resolve: bool,
    },
    /// `token` matches the [`LspRequest::Query`] that asked. `None` is an empty hover.
    Hover { token: u64, text: Option<String> },
    /// Places `textDocument/definition` returned, in the order the server sent them.
    Definition {
        token: u64,
        locations: Vec<JumpTarget>,
    },
    /// Places `textDocument/references` returned, in the order the server sent them.
    References {
        token: u64,
        locations: Vec<JumpTarget>,
    },
    /// Completion items for `token`. `incomplete` means a later keystroke should ask again.
    Completion {
        token: u64,
        incomplete: bool,
        items: Vec<CompletionItem>,
    },
    /// The active signature at the cursor. `None` is an empty answer.
    Signature { token: u64, text: Option<String> },
    /// Documentation filled in for one completion item.
    Resolved {
        token: u64,
        detail: String,
        docs: String,
    },
    /// Semantic tokens for one file, as byte ranges in `version`'s text.
    SemanticTokens {
        path: PathBuf,
        version: u64,
        spans: Vec<(usize, usize, crate::theme::SyntaxRole)>,
    },
    /// Edits from rename, format, or a code action.
    /// `version` is the document the byte offsets refer to. `None` means the file
    /// was not open, so `start_line` / `start_char` are applied to the bytes on disk.
    WorkspaceEdit {
        token: u64,
        encoding: crate::lsp::position::Encoding,
        edits: Vec<TextPatch>,
    },
    /// Code actions the user can pick from.
    Actions {
        token: u64,
        encoding: crate::lsp::position::Encoding,
        actions: Vec<ActionOffer>,
    },
}

/// One text replacement. Byte offsets are in `version`'s text when `version` is set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPatch {
    pub path: PathBuf,
    pub version: Option<u64>,
    pub start: usize,
    pub end: usize,
    pub start_line: u32,
    pub start_char: u32,
    pub end_line: u32,
    pub end_char: u32,
    pub text: String,
}

/// One code action. `edit` is a workspace edit JSON when the server sent one.
/// `raw` is the whole action, for `codeAction/resolve`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionOffer {
    pub title: String,
    pub edit: Option<String>,
    pub raw: String,
}

/// A definition, in the server's position encoding, so it can be resolved in any buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpTarget {
    pub path: PathBuf,
    pub line: u32,
    pub character: u32,
    pub end_line: u32,
    pub end_character: u32,
    pub encoding: crate::lsp::position::Encoding,
}

/// One completion candidate. Ranges are bytes in the text the request was about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub detail: String,
    pub filter: String,
    pub insert: String,
    /// Short kind, such as `fn` or `struct`. Empty when the server omitted it.
    pub kind: String,
    /// Plain documentation. Empty until the item or a resolve reply provides it.
    pub docs: String,
    /// `insert` is an LSP snippet with tab stops.
    pub snippet: bool,
    /// Original item JSON, sent to `completionItem/resolve` when `docs` is empty.
    pub resolve: Option<String>,
    /// Bytes the insert replaces. `None` means the word before the cursor.
    pub replace: Option<(usize, usize)>,
    /// Other byte edits in that same text, such as an import above the cursor.
    pub extra: Vec<(usize, usize, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStatus {
    Starting,
    Ready,
    /// Work in progress reported by the server, e.g. `Indexing 40%`.
    Busy(String),
    /// Stopped for good this session.
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

impl Severity {
    /// LSP `DiagnosticSeverity`. A missing severity is shown as an error.
    pub fn from_lsp(value: Option<lsp_types::DiagnosticSeverity>) -> Self {
        match value {
            Some(lsp_types::DiagnosticSeverity::WARNING) => Self::Warning,
            Some(lsp_types::DiagnosticSeverity::INFORMATION) => Self::Info,
            Some(lsp_types::DiagnosticSeverity::HINT) => Self::Hint,
            _ => Self::Error,
        }
    }

    /// Gutter letter.
    pub fn letter(self) -> char {
        match self {
            Self::Error => 'E',
            Self::Warning => 'W',
            Self::Info => 'I',
            Self::Hint => 'H',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Byte range in the buffer. May be empty.
    pub start: usize,
    pub end: usize,
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
}

impl Diagnostic {
    /// A parse failure. The live `textDocument/diagnostic` pull owns these.
    /// A `cargo check` of the file on disk reports the same class (`E0765`, an
    /// unclosed delimiter) and would otherwise stay until the next save.
    pub fn is_syntax(&self) -> bool {
        if self.code.as_deref() == Some("syntax-error") || self.message.starts_with("Syntax Error")
        {
            return true;
        }
        let message = self.message.as_str();
        message.starts_with("unterminated ")
            || message.starts_with("unexpected closing delimiter")
            || message.starts_with("mismatched closing delimiter")
            || message.starts_with("this file contains an unclosed delimiter")
    }
}
