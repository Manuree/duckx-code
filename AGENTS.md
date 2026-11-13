# AGENTS.md

This file provides persistent context, architectural guardrails, and coding conventions for AI coding agents working on **duckx-code**.

## 1. Project Context & Documentation Map

Before making structural, UX, or architectural changes, always consult:
* [`ARCHITECTURE.md`](./ARCHITECTURE.md) — System data flow, component boundaries, concurrency model, and Rust crate stack.
* [`DESIGN.md`](./DESIGN.md) — Product philosophy, 75% keyboard layout ergonomics, Shift-driven selection rules, and Semantic Command Bar UX.
* [`DOCUMENTS.md`](./DOCUMENTS.md) — User reference of every implemented keybinding and Command Bar command, grouped by category. **Update it in the same change** whenever you add, remove, or alter a keybinding (`src/input.rs`) or a command (`src/command.rs`), and move items out of its "Planned" section once they ship.

---

## 2. Core Invariants (Must Never Be Violated)

1. **True Modeless Operation:**
   * Never introduce modal editing states (`Normal`, `Insert`, `Visual` modes) or `hjkl` navigation bindings.
   * When the editor canvas has focus, printable characters must always insert text directly into the buffer.
 * Modeless is the editor's rule. The assistant panel may have its own modes (`/plan`, `/ask`, `/agent`), but they must never change editor keys or state.
 * The assistant must not move the user's cursor or selection, switch the visible tab, or take focus while the editor has focus. Its edits go to buffers (undoable, unsaved), never straight to disk.
2. **75% Keyboard Navigation & Shift-Selection:**
   * All cursor movement must center around `Up`, `Down`, `Left`, `Right`, `Home`, `End`, `PgUp`, and `PgDn` (plus `Alt` word and `Ctrl` document modifiers).
   * `Ctrl + Left / Right / Up / Down` moves focus to the open surface that borders the current one on that side. It must not open a closed surface and must not move the text cursor.
   * Holding `Shift` with any navigation key must extend/modify the active selection (`anchor` stays fixed, `head` moves).
   * Unmodified navigation keys must collapse any active selection (`anchor = head`).
3. **0ms Non-Blocking UI Thread:**
   * Never perform blocking I/O, synchronous network/stdio reads, or heavy synchronous computation on the main UI/render thread.
   * All LSP communication (`stdin`/`stdout` JSON-RPC) must live inside the asynchronous `tokio` background worker and communicate with `Application Core` strictly via `tokio::sync::mpsc` channels.
4. **Unidirectional Data Flow:**
   * `Input Router` converts raw `crossterm::event::KeyEvent` into `SemanticAction`.
   * `Application Core` mutates state exclusively in response to `SemanticAction` or `LspEvent`.
   * `UI & Viewport` (`ratatui`) must remain a pure, read-only projection of `Application Core` state—never mutate editor state inside rendering functions.
5. **UI Requirements ([`DESIGN.md` §5](./DESIGN.md#5-ui-requirements)):**
   * **R1 Rounded Borders:** every bordered element must be built with `ui::panel(title)` (rounded `╭╮╰╯` corners). Never construct `Block::bordered()` or set another `BorderType` directly. Any new bordered widget needs a render test asserting rounded corners.
   * **R2 Block Cursor:** the visible cursor is a reversed cell in the frame. The terminal cursor stays hidden. Never switch it to a bar or underline. `SetCursorStyle::SteadyBlock` is set at startup and restored on exit.
   * **R3 Notices:** report results, errors, and warnings by setting `App::message`; they appear as snackbar notices. Only state of the moment (progress ending in `…`, FIND/REPLACE position, prompts waiting for a key) stays on the status line, decided by `app::notice::placement`. Do not draw transient text in the status bar directly.
   * **R4 Floating panels:** the Commands list and the assistant panel are drawn over the bottom of the windows (`EditorLayout::command` / `agent`), never by shrinking the editor tree. Clear the area first, use `ui::panel`, and size editor scrolling with `EditorLayout::pane_sizes`. Only the status line, the diagnostic bar, and the FIND/REPLACE hint row reserve rows.
   * **R5 Settings rows:** a row that does not apply is hidden, not greyed. Add its condition to `AiPrefs::visible` / `LspPrefs::visible` (read through `App::setting_visible`) with a test; keep `Page::row` an index into `section().items` and move it with the `Page` methods that skip hidden rows.
   * **R7 Brackets:** bracket highlights reach the editor only through `viewport::Decorations` and keep its documented draw order. Colors come from theme keys (`brackets`, `bracket_error`, `editor.bracket_match`) with fallbacks, never hard-coded. Every bracket scan is bounded (`brackets::MATCH_SCAN_CHARS`, `COLOR_LOOKBACK_LINES`, `COLOR_SCAN_CHARS`) and cached on the tab; never scan the whole file per frame. `<>` is not a bracket.
   * **R8 Language servers:** route files to servers only through `lsp::registry::Registry` (by extension, custom first, first enabled and installed wins); never pick a server by grammar. Look programs up only with `lsp::detect::find_program`, in a background job, and spawn the resolved path without a shell (keep `CREATE_NO_WINDOW`). A new built-in needs args, language ids, root markers, and an install hint, and stays On by default. New settings keys stay optional so older `settings.json` files load.
   * **Workspace walks:** walk the workspace only through `services::tree::walker(root, show_hidden)` so `.gitignore`, the Show hidden files setting (`FilePrefs::show_hidden`), and the always-hidden `.git` apply everywhere. Pass the setting into the job; do not build a `WalkBuilder` by hand.

---

## 3. Rust Coding Standards & Conventions

* **Text Indexing & Unicode Safety:**
  * Always use `ropey::Rope` for buffer storage and manipulation.
  * Never slice strings by raw byte indices (`&str[a..b]`) without verifying UTF-8 char/grapheme boundaries.
  * Use `unicode-segmentation` for grapheme cluster movement (`Left`/`Right`) and `unicode-width` for visual column calculations (`Up`/`Down` target column and horizontal scrolling).
* **Error Handling:**
  * Do not use `.unwrap()` or `.expect()` in runtime code paths where user input, file I/O, clipboard access, or LSP responses can fail.
  * Propagate recoverable errors gracefully and surface user-facing errors in the Status Bar or notification overlay instead of panicking.
  * Ensure a panic hook restores terminal state (`disable_raw_mode`, `LeaveAlternateScreen`) before printing panic backtraces so the user's terminal is never left corrupted.
* **Terminal Compatibility:**
  * In `crossterm`, enable `EnableBracketedPaste` and attempt `PushKeyboardEnhancementFlags` where supported, but always provide fallback key matches (`F1` and `Ctrl + ;` alongside `Shift + Space`) so Windows Terminal, Alacritty, WezTerm, and standard terminals all work out of the box.
  * Windows gets no `Event::Paste` from crossterm; a paste arrives as key presses. Route terminal events through `key_burst::KeyBurst` (as `main.rs` does) so a key flood becomes one `Event::Paste`, and handle `Event::Paste` in every new text input instead of relying on per-key handling. See DESIGN.md R6.
* **Testing:**
  * Write unit tests for `buffer.rs` (cursor movement, `Shift` selection spans, grapheme boundaries, Undo/Redo transactions) and `command.rs` (prefix matching and `Top-1 Recent` ordering) independent of the terminal UI.
* **Feature examples:**
  * When you add a feature, add a file under `examples/<folder>/` with comments that teach the keys. One short English folder per feature. English only, in short easy words. Add one line to `examples/start.txt`. Do not put Cargo binaries in `examples/`.
