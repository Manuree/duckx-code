//! Assistant protocol, tool list, and the background HTTP worker.
//! The worker never touches a buffer. Edits land on the main loop.

mod disk;
mod edit;
mod path;
mod provider;
mod worker;
mod wrap;

pub use disk::{DiskTask, perform as perform_disk};
pub use edit::{FileEdit, apply_to_buffer, read_lines};
pub use path::within;
pub use worker::spawn;
pub use wrap::{column_at, index_at, next_boundary, prev_boundary, rows as wrap_rows};

use serde_json::{Value, json};

/// One line in the assistant panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptLine {
    pub user: bool,
    pub text: String,
}

/// Panel state. The core owns it; the worker only sees copies of the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Panel {
    pub open: bool,
    pub focused: bool,
    pub prompt: String,
    /// Character index of the prompt cursor.
    pub cursor: usize,
    /// Selection anchor. Equal to `cursor` when nothing is selected.
    pub anchor: usize,
    /// The transcript has the keys. `Ctrl+Down` returns to the prompt.
    pub reading: bool,
    /// Wrapped rows above the latest line while reading. Zero shows the latest text.
    pub scroll: usize,
    pub transcript: Vec<TranscriptLine>,
    pub status: String,
    pub busy: bool,
    /// Spinner frame while `busy` is set. The frames are `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`.
    pub spin: u8,
    pub turn: u64,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            open: false,
            focused: false,
            prompt: String::new(),
            cursor: 0,
            anchor: 0,
            reading: false,
            scroll: 0,
            transcript: Vec::new(),
            status: "ready".to_string(),
            busy: false,
            spin: 0,
            turn: 0,
        }
    }
}

/// `App` -> worker.
#[derive(Debug, Clone)]
pub enum AgentCommand {
    Turn {
        id: u64,
        server: String,
        model: String,
        history: Vec<(String, String)>,
        prompt: String,
    },
    ToolResult {
        id: u64,
        call_id: String,
        content: String,
    },
    Cancel {
        id: u64,
    },
}

/// Worker -> `App`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    Reply {
        id: u64,
        text: String,
    },
    Tool {
        id: u64,
        call_id: String,
        name: String,
        arguments: String,
    },
    Failed {
        id: u64,
        text: String,
    },
}

pub fn tool_specs() -> Vec<Value> {
    vec![
        tool(
            "read_file",
            "Read a line range from a workspace file. Lines are 1-based.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "start_line": { "type": "integer" },
                    "end_line": { "type": "integer" }
                },
                "required": ["path"]
            }),
        ),
        tool(
            "list_dir",
            "List names in one workspace directory.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        ),
        tool(
            "editor_state",
            "Focused file, cursor line, and the current selection.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            "diagnostics",
            "Errors and warnings for a file. Omit path for the focused file.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" } }
            }),
        ),
        tool(
            "apply_edit",
            "Replace a 1-based inclusive line range, or one unique snippet. An open file is one undo step.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "start_line": { "type": "integer" },
                    "end_line": { "type": "integer" },
                    "text": { "type": "string" },
                    "find": { "type": "string" },
                    "replace": { "type": "string" }
                },
                "required": ["path"]
            }),
        ),
        tool(
            "create_file",
            "Write a new file and open it. Fails if the path already exists.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "text": { "type": "string" }
                },
                "required": ["path", "text"]
            }),
        ),
        tool(
            "open_file",
            "Open a workspace file in the editor.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        ),
        tool(
            "save",
            "Save an open file. Omit path to save the focused file.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" } }
            }),
        ),
        tool(
            "goto",
            "Move the cursor to a 1-based line. path opens that file first.",
            json!({
                "type": "object",
                "properties": {
                    "line": { "type": "integer" },
                    "path": { "type": "string" }
                },
                "required": ["line"]
            }),
        ),
    ]
}

fn tool(name: &str, description: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "function": { "name": name, "description": description, "parameters": parameters }
    })
}

pub const SYSTEM: &str = "\
You are the assistant inside duckx-code. Use tools to read and change files in the workspace. \
Paths stay inside the workspace. apply_edit replaces a line range or one unique snippet, and an open file is one undo step. \
Each user message ends with editor_state: the focused file, the 1-based cursor line and column, and any selection. \
Edit at that cursor unless the user names another file or line. \
create_file writes a new file and opens it. It refuses a path that already exists; change an existing file with apply_edit. \
Do not invent paths. When you are finished, answer in short plain text.";
