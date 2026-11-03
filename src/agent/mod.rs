//! Assistant protocol, tool list, and the background HTTP worker.
//! The worker never touches a buffer. Edits land on the main loop.

mod disk;
mod edit;
mod path;
mod provider;
mod worker;
mod wrap;

pub use disk::{DiskTask, MAX_BYTES, check_file_size, create_parent_plan, perform as perform_disk};
pub use edit::{FileEdit, apply_to_buffer, read_lines, read_lines_rope};
pub use path::{canonicalize, strip_verbatim, within};
pub use provider::{content_looks_like_tool_prefix, normalize_tool_arguments};
pub use worker::spawn;
pub use wrap::{column_at, index_at, next_boundary, prev_boundary, rows as wrap_rows};

use serde_json::{Value, json};

/// One line in the assistant panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptLine {
    pub user: bool,
    pub text: String,
}

/// A mutating tool waiting for Allow / Deny in the assistant panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTool {
    pub turn: u64,
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    /// Short summary shown above the prompt (tool, path, preview).
    pub summary: String,
}

/// Tools that change files need Allow / Deny unless auto-approve is on.
pub fn needs_approval(name: &str) -> bool {
    matches!(name, "apply_edit" | "create_file" | "save")
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
    /// Mutating tool waiting for Allow (y/Enter) or Deny (n/Esc).
    pub pending: Option<PendingTool>,
    /// Prior turn messages (user / assistant / tool) kept under a char budget.
    pub memory: Vec<serde_json::Value>,
    /// Live streamed assistant text for the current turn, shown until Reply.
    pub stream_draft: Option<String>,
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
            pending: None,
            memory: Vec::new(),
            stream_draft: None,
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
        /// Optional Bearer token. Empty means no Authorization header.
        api_key: String,
        /// When true the worker tries SSE first, then falls back.
        stream: bool,
        /// Ollama `num_ctx`. Ignored on OpenAI-compat / LM Studio.
        num_ctx: Option<u32>,
        /// Use Ollama native `/api/chat` so `num_ctx` is honored.
        ollama_native: bool,
        /// Prior OpenAI-compatible messages (user / assistant / tool), already budgeted.
        history: Vec<serde_json::Value>,
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
    /// Incremental assistant text while a streamed completion is running.
    Delta {
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
    /// Drop poisoned tool history so the next turn does not keep failing with 400.
    ResetMemory {
        id: u64,
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
                    "text": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "text"]
            }),
        ),
        tool(
            "write_file",
            "Same as create_file. content is the full new file.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                    "text": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
        ),
        tool(
            "search_replace",
            "Same as apply_edit. old_string is the unique snippet and new_string replaces it.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        ),
        tool(
            "list_files",
            "Same as list_dir.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "directory": { "type": "string" }
                },
                "required": ["path"]
            }),
        ),
        tool(
            "bash",
            "Unavailable. Do not call it. Use create_file, write_file, or apply_edit.",
            json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "required": ["command"]
            }),
        ),
        tool(
            "shell",
            "Unavailable. Do not call it. Use create_file, write_file, or apply_edit.",
            json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "required": ["command"]
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
You are the duckx-code assistant. Act with tools. Never tell the user to run steps you can do yourself. \
Greetings and short chat: plain text, no tools. Each user message already ends with editor_state — do not call editor_state to greet. \
Rules: paths stay in the workspace; no bash/shell; do not invent paths. \
create_file / write_file: write a new file and open it (one new parent folder max; refuse if path exists). \
apply_edit / search_replace: change an existing file (line range or unique snippet; open file = one undo). Edit at the cursor unless another path/line is named. \
save: save an open file. Approval for apply_edit/create_file/save is handled by the app — call the tool; do not hesitate. \
Tool call format examples (emit a real tool call, not these as prose): \
{\"name\":\"create_file\",\"arguments\":{\"path\":\"src/hi.rs\",\"text\":\"fn main() {}\"}} \
{\"name\":\"apply_edit\",\"arguments\":{\"path\":\"a.rs\",\"find\":\"old\",\"replace\":\"new\"}} \
When done, reply in short plain text.";

/// Maps names and argument keys local models already emit onto the tools the editor runs.
pub fn normalize_call(name: &str, arguments: &str) -> (String, String) {
    let name = match name.trim() {
        "write" | "write_file" | "save_file" | "create" => "create_file",
        "read" | "read_text" | "read_text_file" => "read_file",
        "list" | "list_files" | "ls" | "dir" => "list_dir",
        "edit" | "edit_file" | "search_replace" | "str_replace" | "replace_in_file" => "apply_edit",
        "open" => "open_file",
        "goto_line" => "goto",
        "bash" | "shell" | "run_command" | "run_terminal_cmd" | "execute" | "powershell"
        | "cmd" => "shell",
        other => other,
    };
    let Ok(mut value) = serde_json::from_str::<Value>(arguments) else {
        return (name.to_string(), arguments.to_string());
    };
    let Some(obj) = value.as_object_mut() else {
        return (name.to_string(), arguments.to_string());
    };
    move_alias(
        obj,
        &["file", "file_path", "filepath", "filename", "directory"],
        "path",
    );
    move_alias(obj, &["content", "contents", "file_text", "body"], "text");
    move_alias(obj, &["old_string", "old_str", "old_text"], "find");
    move_alias(obj, &["new_string", "new_str", "new_text"], "replace");
    move_alias(obj, &["start", "offset"], "start_line");
    move_alias(obj, &["end"], "end_line");
    if name == "read_file" && !obj.contains_key("end_line") {
        if let Some(limit) = obj.remove("limit").and_then(|value| value.as_u64()) {
            let start = obj
                .get("start_line")
                .and_then(|value| value.as_u64())
                .unwrap_or(1)
                .max(1);
            obj.insert("end_line".into(), json!(start + limit.saturating_sub(1)));
        }
    }
    (name.to_string(), Value::Object(obj.clone()).to_string())
}

fn move_alias(obj: &mut serde_json::Map<String, Value>, from: &[&str], to: &str) {
    let occupied = obj.get(to).is_some_and(|value| match value {
        Value::String(text) => !text.is_empty(),
        Value::Null => false,
        Value::Number(number) => number.as_u64().unwrap_or(1) != 0,
        _ => true,
    });
    if occupied {
        return;
    }
    for key in from {
        if let Some(value) = obj.remove(*key) {
            obj.insert(to.to_string(), value);
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_call;
    use serde_json::Value;

    #[test]
    fn write_file_content_is_create_file_text() {
        let (name, arguments) = normalize_call(
            "write_file",
            r#"{"path":"index.html","content":"<p>hi</p>"}"#,
        );
        assert_eq!(name, "create_file");
        let value: Value = serde_json::from_str(&arguments).unwrap();
        assert_eq!(value["path"], "index.html");
        assert_eq!(value["text"], "<p>hi</p>");
    }

    #[test]
    fn search_replace_and_shell_map_onto_editor_tools() {
        let (name, arguments) = normalize_call(
            "search_replace",
            r#"{"file_path":"a.rs","old_string":"foo","new_string":"bar"}"#,
        );
        assert_eq!(name, "apply_edit");
        let value: Value = serde_json::from_str(&arguments).unwrap();
        assert_eq!(value["path"], "a.rs");
        assert_eq!(value["find"], "foo");
        assert_eq!(value["replace"], "bar");

        let (name, _) = normalize_call("bash", r#"{"command":"ls"}"#);
        assert_eq!(name, "shell");
    }
}
