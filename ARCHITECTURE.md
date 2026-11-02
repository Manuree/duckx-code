# ARCHITECTURE.md

## 1. System Overview

**duckx-code** is a terminal-based code editor written in **Rust**. It is engineered around a strict **Unidirectional Data Flow** on the main UI thread coupled with **Asynchronous Message Passing** for background workloads (LSP and File I/O), guaranteeing zero-latency typing and rendering.

> For product philosophy, 75% keyboard ergonomics, keybindings, and UI/UX specifications, see [`DESIGN.md`](./DESIGN.md).  
> For AI coding assistant guidelines, Rust conventions, and development rules, see [`AGENTS.md`](./AGENTS.md).

---

## 2. High-Level System Architecture

```text
[Crossterm KeyEvents]                                 [LSP Server Process]
          │                                                    ▲
          ▼                                                    │ (JSON-RPC stdio)
┌────────────────────┐                                ┌────────┴───────────┐
│  1. Input Router   │                                │  5. Async LSP      │
│  (Focus-Aware)     │                                │     Client Worker  │
└─────────┬──────────┘                                └────────┬───────────┘
          │ Semantic Action                                    │ LSP Events (mpsc)
          ▼                                                    ▼
┌──────────────────────────────────────────────────────────────────────────┐
│                         2. Application Core                              │
│          (Text Buffer, Selection, Command Engine, Diagnostics)           │
└─────────┬────────────────────────────────────────────────────▲───────────┘
          │                                                    │
          ▼                                                    ▼
┌────────────────────┐                                ┌────────────────────┐
│  4. UI & Viewport  │                                │  3. OS Services    │
│  (Ratatui Render)  │                                │  (Arboard / FS)    │
└────────────────────┘                                └────────────────────┘
```

---

## 3. Component Specifications

### 3.1 Input Router (Focus-Aware)
* **Role:** Captures raw terminal input events (`crossterm::event::Event`) and translates them into strongly-typed `SemanticAction` enums based on the active focus state.
* **Focus Context Routing:**
  * `Focus::Editor` — Maps printable chars to buffer insertion, navigation keys to cursor movement, and `Shift`-modified navigation keys to selection extension. `Ctrl + Arrow` is focus movement between surfaces, not cursor movement; `Alt + Left / Right` is word movement. `Ctrl + Alt + Left / Right` cycles tabs in the focused pane. `Ctrl + Alt + Up / Down` duplicates the current line or the selected lines. `Shift + Delete` deletes those lines.
  * `Focus::Sidebar` — Routes navigation keys to the file tree (`Up`/`Down`/`PgUp`/`PgDn`/`Home`/`End`, `Right` expand, `Left` collapse, `Enter` activate) and printable keys to the explorer filter. In FILE mode, `Shift` plus those movement keys covers a range, `Space` marks the cover or the focused row, and `Esc` returns to NORMAL and clears every mark. `Shift + Space` opens the Command Bar from here too, including in FILE mode. Otherwise `Esc` clears the filter or returns focus to the editor. `Ctrl + Arrow` moves focus between surfaces from here too. `Ctrl + Alt + Left / Right` cycles the active pane's tabs without leaving the sidebar.
  * `Focus::CommandBar` — Routes keystrokes to the command input line, `Up`/`Down` to candidate selection, `Enter` to command dispatch, and `Esc` to close the overlay.
  * `Focus::Settings` — The section list stays on the left and its rows on the right. `Up`/`Down` move in the focused column. `Enter` or `Right` moves into the rows, and `Esc`, `Left`, or Backspace steps back (closing the page from the section list). Printable keys do not reach the buffer.
  * `Focus::FilePicker` — `file-picker` / `Ctrl+P`. Printable keys edit the query. `Up`/`Down`/`PgUp`/`PgDn` move the file list. `Left`/`Right`/`Home`/`End` move the query cursor. `Enter` opens the highlighted file. `Esc` closes the picker. `Ctrl+Arrow` does not leave the modal.
  * `Focus::CompletionPopup` — Intercepts `Up`, `Down`, `Tab`, `Enter`, and `Esc` while forwarding standard typing events to the underlying editor buffer.
* **Terminal Protocol Layer:** Negotiates `PushKeyboardEnhancementFlags` (Kitty Keyboard Protocol) via `crossterm` where available to disambiguate modifier combinations such as `Shift + Space` (Command Bar trigger), while maintaining fallback bindings (`F1`, `Ctrl + ;`) for terminals that cannot report them.

### 3.2 Application Core
* **Role:** Single source of truth for editor state. Processes `SemanticAction` and `LspEvent` messages deterministically and synchronously on the main loop.
* **Core Subsystems:**
  * **Text Buffer (`ropey::Rope`):** UTF-8 rope data structure providing $O(\log N)$ insertions, deletions, and line/char indexing on large files.
  * **Selection Engine:** Models selections as `{ anchor: Position, head: Position }`. Unmodified movement sets `anchor = head`; `Shift`-modified movement updates `head` while preserving `anchor`.
  * **Transaction History (Undo / Redo):** Records atomic buffer deltas paired with pre- and post-edit selection states.
  * **Command Engine:** Maintains a prefix-searchable registry of commands and tracks the most recently executed command per focus context (`Top-1 Recent`) for instant re-execution. The registry is filtered by the surface that opened the Command Bar: editor commands and explorer file commands are never listed together. For `move`, `rename`, and `save`, the closest file or folder name from the scanned tree is drawn as dim ghost text; `Right` copies it into the input. That lookup reads the in-memory tree only.
  * **Diagnostics Store (`lsp/diagnostics.rs`):** LSP diagnostics (`Error`, `Warning`, `Info`, `Hint`) per file as buffer byte ranges, sorted by start. Each change the core sends shifts the stored ranges, and a result computed for an older document version is moved forward through the last 64 changes before it is stored.
  * **LSP Sync (`app/lsp_sync.rs`):** Once per frame, `take_lsp_requests` compares every file tab with what the worker was last told. A new eligible tab sends `Open`; a closed, renamed, or re-languaged tab sends `Close`; an edited tab sends one `Change` whose byte span is merged from the buffer's `ByteEdit` log (the same log tree-sitter uses), or the whole text when the buffer was replaced. The request carries a rope clone, so no text is copied on the UI thread.
  * **Explorer Model:** Holds the scanned file tree as a flat, depth-first list plus expanded directories, filter text, selection, and git status per path. Visibility, tree guides, and filtering are pure computations over that list. A content search replaces those rows with matching files, their parent folders, and a match count. The file picker keeps a flat catalog of those same ignore rules and scores it off the UI thread.
* **Background Jobs:** Actions never block. Work such as scanning the file tree, indexing and filtering the file picker, searching or replacing file contents, running `git status`, comparing a buffer with `HEAD`, parsing a buffer for syntax highlighting, reading and atomically saving a file, or checking whether an open file changed on disk is queued as a `Job` in `App`; the main loop drains the queue, runs each job on `tokio::task::spawn_blocking`, and feeds the resulting `AppEvent` back through an `mpsc` channel into `App::handle_event`. About once a second the loop queues a stamp check for each open file. Results for a stale root, a replaced buffer, an older search, an older picker generation, an older syntax parse, an older diff generation, an older open, or a stamp from before the latest save are dropped.

### 3.3 OS Services (Arboard / FS)
* **Role:** Abstracts host operating system interactions away from the pure state transitions in `Application Core`.
* **System Clipboard (`arboard`):** Provides direct read/write access to the native OS clipboard for Copy, Cut, and Paste actions.
* **File System & Atomic Writes:** Manages text loading (UTF-8, a UTF-8 or UTF-16 BOM, and an explicit Windows-1252 or Windows-874 encoding), line-ending normalization (`LF` / `CRLF`), dirty-flag tracking, atomic saves in that encoding, and reload when a clean open file changes on disk. The rope stays Unicode; encoding exists only at the read and write boundary.
* **Tree Scan (`ignore`):** Walks the workspace respecting `.gitignore` and hiding dotfiles, sorted directories-first (capped at 20,000 entries). The file picker uses the same rules for a flat file list, capped at 100,000 files.
* **Git Status:** Runs `git status --porcelain=v1 -z` and propagates the strongest status of each file up to its parent directories. `diff` runs `git show HEAD:path` and compares those lines with the buffer off the UI thread.

### 3.4 UI & Viewport (Ratatui Render)
* **Role:** Projects an immutable view of `Application Core` onto the terminal alternate screen at each render frame without mutating application state.
* **Viewport Projection:** Computes `scroll_x` and `scroll_y` offsets from cursor position, terminal dimensions, and `scrolloff` margins.
* **Incremental Syntax Highlighting (`tree-sitter`):** On an edit the core calls `tree.edit` so the previous colors stay aligned, then queues `Job::Parse` with a rope clone. The worker parses off the UI thread (one parse in flight per tab; a stale generation or revision is discarded). Rendering queries only the visible line range `[scroll_y .. scroll_y + height]`. Files larger than 8 MB stay plain text. Markdown fenced code blocks are parsed with the fence's language.
* **Render Pipeline:**
  1. Base editor canvas (line number gutter, optional HEAD diff column, diagnostic markers, syntax-highlighted text, selection background spans), with the explorer sidebar on the left when open.
  2. Status bar (file name, dirty state, cursor `Ln:Col`, LSP state).
  3. Modal overlays (Semantic Command Bar, file picker, settings page, command hints, LSP completion menu).
* **Themes:** `theme::Appearance` on the app holds the color theme (interface) and the syntax theme (code). Widgets read `UiStyles` derived from the color theme and never load files while drawing. Theme documents are JSON. Built-ins are embedded from `themes/`; the user config directory can add or replace them. Changing a theme queues a background write of `settings.json`.
* **Session:** On quit, after the frame loop, a background task updates `session.json` in that same config directory. The file keeps one entry per folder (the latest 20): each saved file's path and cursor (line and character column), the split tree, and the sidebar's open state, width, and expanded folders. A start with no path argument uses the current directory as the folder and restores only the entry for that directory, through `Job::LoadFile`. A different directory is not substituted. A path argument skips the restore. Untitled text is not stored. A missing file, or a path that climbs out of the folder with `..`, is skipped. An older file that stored a single folder is still read.

### 3.5 Async LSP Client Worker
* **Role:** Manages external Language Server child processes (`stdin`/`stdout` JSON-RPC 2.0) completely off the UI thread.
* **Concurrency & Channels:**
  * One background `tokio` task (`lsp/client.rs`) owns every server process, the text each server was last sent, and all position conversion. Each process has its own reader, writer, stderr, and wait tasks that feed the worker through an internal channel.
  * **Outbound (`Core -> LSP`):** `mpsc::UnboundedSender<LspRequest>` carries `Open`, `Change`, `Save`, `Close`, `Shutdown`, `Query` (hover, definition, completion), and `Cancel`, with positions as rope byte offsets. Document sync is queued first; a query from the same frame follows it.
  * **Inbound (`LSP -> Core`):** `mpsc::UnboundedReceiver<LspEvent>` delivers `Diagnostics` (byte ranges plus the document version they refer to), `Status` (starting, ready, busy with progress text, failed), `Notice`, `Features`, `Hover`, `Definition`, and `Completion` to the main loop's `tokio::select!`, which calls `App::handle_lsp`. A reply whose token is not the current request is ignored.
* **Servers (`lsp/registry.rs`):** Each language lists candidate programs in order; one that is not installed is skipped, and a group with none is not retried this session. Servers are keyed by group and root folder. The root is the highest folder with a marker file inside the workspace, so one server covers a Cargo or npm workspace.
* **Protocol details:**
  * `initialize` offers `positionEncodings: ["utf-8", "utf-16"]`, hover (`plaintext` and `markdown`), definition (`linkSupport`), and completion (`contextSupport`, no snippets). `lsp/position.rs` converts byte offsets to whichever the server picks (UTF-16 by default, UTF-32 also handled).
  * Hover markdown is shown as plain text. Completion snippets keep tab stops: `Tab` / `Shift+Tab` move through them and `Esc` leaves the snippet. Typing waits 50 ms and cancels the previous request; `Ctrl+Space`, hover, and definition are sent at once. A reply is applied to the text the request was about. The completion insert, including an import that ends before the cursor, is one undo step. The list is filtered by the word before the cursor and can show a resolved documentation line. `(` and `,` request signature help. Definition jumps record a location (64 deep) so `Ctrl+O` / `Ctrl+Shift+O` can walk back and forward. A jump into a file that is not open yet is applied when that load finishes. Semantic tokens, when offered, are requested 200 ms after a change and paint over tree-sitter colors.
  * `publishDiagnostics` keeps the document version the server named, and positions are taken from that version's text. A server that offers `diagnosticProvider` is asked again 200 ms after a change. That pull is stored separately from `publishDiagnostics`. An empty pull does not clear type errors that arrived on save. It does drop syntax errors from that save, and the pull's own syntax errors take their place, so a quote closed in the buffer clears without saving. An edit that rewrites a diagnostic's range drops that marker immediately, including an insertion at either edge of the token, so turning `print` into `println!` clears the saved compiler error without waiting for the next check. rust-analyzer's cargo check still runs on save for a type error the edit did not touch.
  * Documents opened while a server initializes are held and sent with their latest text once it is ready. Changes are incremental (one range per `didChange`) when the server supports it, otherwise the full text.
  * Every server-to-client request gets an answer (`workspace/configuration`, `workDoneProgress/create`, `registerCapability`, …; unknown methods get `-32601`), because servers block on them. `settings.json` `lsp` chooses whether servers run, whether rust-analyzer checks on save, whether a save formats first, and pyright's `typeCheckingMode` (`off`, `basic`, or `standard`) with `diagnosticMode: openFilesOnly`. A change notifies servers with `workspace/didChangeConfiguration`. Rename, format, and code action are ordinary queries; their edits land in open buffers as one undo step, and in files that are not open as a background write.
  * `$/progress` becomes `Status::Busy`, rate-limited to one update per 100 ms per server.
  * A server that exits after it was ready restarts up to 3 times with 1/2/4 s back-off and reopens its documents; otherwise it is marked failed with the last stderr line. stderr is copied to `logs/lsp-<name>.log` in the config directory. On Windows, servers run with `CREATE_NO_WINDOW` so they cannot change the editor's console modes, and `<name>.cmd` is tried when `<name>` is not found.
  * Quit sends `shutdown`, waits up to 1.5 s for the reply, sends `exit`, waits 0.5 s, then kills what remains.

### 3.6 Assistant worker
* **Role:** Talks to an OpenAI-compatible server (Ollama at `http://127.0.0.1:11434/v1`, or LM Studio and later hosts at their `/v1` address). The HTTP loop lives on a `tokio` task.
* **Channels:** `AgentCommand` (a prompt, a tool result, or cancel) goes to the worker. `AgentEvent` (a reply, a tool request, or an error) comes back through `mpsc` into `App::handle_agent` on the main loop, beside LSP events.
* **Edits:** The worker never touches a `Rope`. A tool that changes an open file becomes one undo step on the core, the same way a language-server patch does. A file that is not open is read or written on `spawn_blocking`. A path that leaves the workspace is an error string returned to the model.
* **Place:** Every prompt sent to the model ends with `editor_state` for the focused file: path, 1-based cursor line and column, and the selection. The chat line stays the text the user typed. The same tool called with the same arguments three times in a row ends the turn, and so does a 33rd tool call. `create_file` writes a new workspace file on the background job and the core opens it. An existing path is refused.

---

## 4. Recommended Rust Crate Stack

| Layer | Crate | Purpose |
| :--- | :--- | :--- |
| **Terminal Backend** | `crossterm` | Cross-platform raw mode, alternate screen, and keyboard event streaming |
| **TUI Rendering** | `ratatui` | Immediate-mode terminal UI layout and double-buffered cell rendering |
| **Text Rope Buffer** | `ropey` | High-performance UTF-8 Rope data structure for text storage and slicing |
| **Syntax Highlighting** | `tree-sitter` | Incremental parsing and syntax tree queries |
| **Async Runtime** | `tokio` | Event loop orchestration, async process I/O, and mpsc channels |
| **LSP Protocol** | `lsp-types` + `serde_json` | Strongly-typed LSP structures and JSON-RPC 2.0 framing |
| **Assistant HTTP** | `reqwest` | OpenAI-compatible chat completions for Ollama and other local servers |
| **OS Clipboard** | `arboard` | Cross-platform native OS clipboard integration |
| **Unicode Handling** | `unicode-segmentation` / `unicode-width` | Grapheme cluster boundary detection and display column width calculation |
| **File Tree Walk** | `ignore` | `.gitignore`-aware directory walking for the explorer sidebar |

---

## 5. Proposed Module Hierarchy

```text
src/
├── main.rs            # Entry point, terminal setup/teardown, main event loop
├── lib.rs             # Library root exposing core modules (testable without a terminal)
├── app.rs             # Application Core state and SemanticAction dispatcher
├── app/
│   ├── lsp_sync.rs    # Tab <-> language server sync, LspEvent handling, F8 navigation
│   └── agent.rs       # Assistant tool results applied on the core
├── agent/             # Assistant protocol, HTTP worker, path check, edits
├── action.rs          # SemanticAction and Focus enums
├── input.rs           # Focus-aware Input Router (KeyEvent -> SemanticAction)
├── buffer.rs          # Rope wrapper, Selection (anchor/head), and Undo/Redo history
├── command.rs         # Command registry, prefix matcher, and Top-1 Recent tracker
├── settings.rs        # Settings page sections and row movement
├── syntax.rs          # Tree-sitter grammars, incremental parse, visible-range highlights
├── theme.rs           # Color themes, syntax themes, JSON load/save
├── explorer.rs        # File tree model: expansion, filter, selection, tree guides
├── picker.rs          # File picker index, fuzzy ranking, and query editing
├── jobs.rs            # Background Job / AppEvent definitions and blocking runners
├── services/
│   ├── clipboard.rs   # Arboard OS clipboard wrapper (in-memory fallback)
│   ├── git.rs         # Branch, `git status`, and buffer-vs-HEAD line diff
│   ├── tree.rs        # .gitignore-aware workspace scan (ignore crate)
│   └── fs.rs          # File I/O and atomic save operations
├── lsp/
│   ├── client.rs      # Async tokio worker: server processes, sync, diagnostics, restarts
│   ├── protocol.rs    # LspRequest / LspEvent message definitions
│   ├── transport.rs   # JSON-RPC Content-Length framing
│   ├── position.rs    # Byte offset <-> UTF-8 / UTF-16 / UTF-32 positions
│   ├── uri.rs         # file:// URIs, including Windows drives and UNC shares
│   ├── registry.rs    # Language -> server programs, project root detection
│   └── diagnostics.rs # Diagnostics store, edit merging, range shifting
└── ui/
    ├── mod.rs         # Top-level Ratatui render coordinator
    ├── viewport.rs    # Code canvas, gutter, selection & cursor rendering
    ├── command_bar.rs # Vertical command hints above the status line
    ├── picker.rs      # File picker modal
    ├── icons.rs       # Nerd Font glyphs and file-type icons
    ├── sidebar.rs     # Explorer sidebar: filter box, tree, git markers
    └── status_bar.rs  # lualine-style bottom status line
```
