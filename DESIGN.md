# DESIGN.md

## 1. Product Vision & Design Philosophy

**duckx-code** is a modern, modeless CLI code editor built specifically for the **75% Mechanical Keyboard Layout**. It brings the approachable, intuitive editing experience of modern GUI editors into the terminal without sacrificing raw speed or lightweight efficiency.

### Why 75% Keyboard Layout?
Legacy terminal editors (`vi`/`vim`, `helix`, `emacs`) were designed in an era before compact keyboards standardized a dedicated right-hand navigation column. A 75% keyboard provides physical, dedicated keys for `Up`, `Down`, `Left`, `Right`, `Home`, `End`, `PgUp`, `PgDn`, and the `F1`–`F12` function row within immediate reach.

**duckx-code** treats these dedicated hardware keys as first-class citizens:
1. **Zero Mnemonic Overhead:** Users never need to memorize arbitrary letter keys (`hjkl`, `w`, `b`, `d`, `y`) that do not visually correspond to their actions.
2. **True Modeless Editing:** Opening a file places the user directly in an editable state. Every printable key inserts text immediately—no switching between Normal, Insert, or Visual modes.
3. **Muscle-Memory Compatibility:** Navigation and selection work identically to standard desktop applications while remaining 100% keyboard-driven.

---

## 2. Ergonomics & Keybinding Specification

### 2.1 Core Navigation (Right-Hand Cluster)
| Keybinding | Action | Description |
| :--- | :--- | :--- |
| `Up` / `Down` | Move Cursor Vertically | Moves cursor up or down one line, preserving target visual column |
| `Left` / `Right` | Move Cursor Horizontally | Moves cursor left or right by one Unicode grapheme cluster |
| `Ctrl + W` | Pane select mode | Enters PANE mode from an editor, a preview, the sidebar, or the assistant panel. Arrows (with or without `Ctrl`) move a candidate highlight to the open window that shares an edge on that side, as many times as needed; a closed window, including a closed sidebar, is not a destination. `Enter` or `Ctrl + W` again focuses the candidate. `Esc` leaves focus where it was. Every other key and paste is swallowed, never typed. The binding is `focus-prefix` ("Pane select mode") in Settings → keys. See §4. |
| `Ctrl + Left` / `Ctrl + Right` | Move by Word | Same stops as `Alt + Left` / `Alt + Right` (below). With `Shift`, extends the selection by word |
| `Ctrl + Up` / `Ctrl + Down` | Paragraph jump | Moves to the start (column 0) of the previous or next blank line (empty or only spaces and tabs). On a blank line, its run of blank lines is skipped first, so repeated presses go from gap to gap. With no blank line left, goes to the start or the end of the document. With `Shift`, extends the selection; without it, a selection stays highlighted (sticky). In a focused preview these scroll one line; in the assistant panel they read the messages above or return to the prompt |
| `Ctrl + Alt + Left` / `Ctrl + Alt + Right` | Previous / next tab | Cycles the tabs of the focused window. Does not move the cursor. Also works while the sidebar is focused. See §4. |
| `Alt + Left` / `Alt + Right` | Move by Word | Skips spaces, then one run of word characters (letters, digits, `_`) or one run of punctuation, like VS Code: `Right` stops at the end of that run, `Left` at its start. In `let  total = foo(bar);`, `Right` stops at `let|`, `total|`, `=|`, `foo|`, `(|`, `bar|`, `);|`. At a line edge it crosses to the next or previous line. While the sidebar is focused, these keys narrow and widen it instead |
| `Home` | Line Start (Smart Home) | Jumps to the first non-whitespace character on the line; pressing again jumps to column 0 |
| `End` | Line End | Jumps to the end of the current line |
| `Ctrl + Home` / `Ctrl + End` | Document Start / End | Jumps to the very beginning or very end of the document |
| `PgUp` / `PgDn` | Page Scroll | Scrolls the viewport and moves the cursor up or down by one screen height |

### 2.2 Shift-Driven Selection
Holding `Shift` with any navigation key anchors the selection at the starting cursor position (`anchor`) and extends the active cursor (`head`):

| Keybinding | Selection Behavior |
| :--- | :--- |
| `Shift + Up / Down / Left / Right` | Character-by-character or line-by-line selection extension |
| `Ctrl + Shift + Left / Right` or `Alt + Shift + Left / Right` | Word-by-word selection extension |
| `Ctrl + Shift + Up / Down` | Extend the selection to the previous or next blank line (paragraph) |
| `Shift + Home` / `Shift + End` | Select from cursor to start or end of the current line |
| `Shift + PgUp` / `Shift + PgDn` | Select an entire viewport page up or down |
| `Ctrl + Shift + Home / End` | Select from cursor to the beginning or end of the document |
| `Ctrl + A` | Select the entire document buffer |
| `Ctrl + D` | Deselect: clear the selection (also the `deselect` command) |

*Typing any printable character, `Backspace`, or `Delete` while a selection is active replaces the entire selected region atomically.*

**Sticky selection.** Moving without `Shift` does not clear the selection or change its range. This covers the arrows, `Home`/`End`, `PgUp`/`PgDn`, word jumps, `goto`, and jumps such as go to definition. The range stays highlighted and keeps counting in the status bar while the cursor moves around to read. Only `Ctrl + D` / `deselect`, a new selection (`Shift` + move, `select-all`, a find match), an edit, or undo/redo clears it. `Esc` does not clear it.
* `Shift` + move continues the selection when the cursor is back on its active end (the `head` where the cursor was when selecting). Anywhere else it starts a new selection anchored at the cursor.
* `copy` and `cut` act on the sticky selection wherever the cursor is.
* Typing, `Enter`, `Backspace`, `Delete`, paste, `Tab`, and the line commands: when the cursor is inside or at an edge of the selection, it is taken back and replaced or deleted as before. When the cursor has moved away, the selection is cleared first and the edit happens at the cursor. Text out of view is never deleted by a keystroke.
* Other edits (find & replace, formatting, save trimming) keep the sticky range on the same text and clamp it to the buffer. Undo and redo restore their own selection and drop the sticky one.

### 2.3 Editing & Clipboard Shortcuts
Avoids terminal control-code traps while remaining intuitive:

| Keybinding | Action |
| :--- | :--- |
| `Ctrl + C` | Copy active selection (or current line if nothing is selected) to the OS clipboard |
| `Ctrl + X` | Cut active selection (or current line) to the OS clipboard |
| `Ctrl + V` | Paste from the OS clipboard at the cursor, replacing an active selection |
| `Ctrl + Z` | Undo last edit transaction |
| `Ctrl + Y` / `Ctrl + Shift + Z` | Redo last undone transaction |
| `Ctrl + S` | Save the current file (atomic write). A buffer with no path asks for one |
| `Alt + Up` / `Alt + Down` | Move current line or selected block up/down |
| `Shift + Delete` | Delete the current line, or every line the selection touches |
| `Ctrl + Alt + Up` / `Ctrl + Alt + Down` | Duplicate the current line or the selected lines above or below |
| `F4` or `Ctrl + /` | Toggle a line comment on the current line or selection |
| `Ctrl + ]` | Jump to the matching bracket. `Ctrl + Shift + ]` selects through it |
| `Shift + F2` | Rename the symbol under the cursor |
| `F7` | Format the current file |
| `Ctrl + .` | Show code actions at the cursor |
| `Tab` | Indent the active line or multi-line selection |
| `Shift + Tab` | Outdent the active line or multi-line selection. Inside a snippet, move to the previous placeholder |
| `Backspace` (inside leading spaces) | Removes one indentation level back to the previous tab stop |

---

## 3. Semantic Command Bar UX

### 3.1 Invocation & Conflict Prevention
Traditional editors either require exiting Insert Mode (`Esc` then `:`) or rely on `Ctrl + Shift + P` (which conflicts with terminal emulator shortcuts).
* **Primary Trigger:** `Shift + Space` — both keys sit under the thumbs and left pinky, so the bar opens without leaving the home row.
* **Fallback Triggers:** `F1` (top-left of a 75% layout), `Ctrl + ;`, and `Ctrl + Shift + :`.
  * Legacy VT terminals send `Shift + Space` as a plain space, so the modifier cannot be detected there. `F1` works everywhere.
  * Known trade-off: holding `Shift` through a space while typing capitals opens the bar. `Esc` closes it with no side effects, and `Shift + Space` typed *inside* the bar inserts a normal space.
* **Benefit:** Typing `:` inside the editor always inserts a literal colon character without mode confusion.

### 3.2 Real-Time Prefix Match & Top-1 Recent Priority
When the Command Bar opens:
1. **Top-1 Recent Pinning:** The most recently executed command on this surface is always the first hint whenever it matches the input, including when the input is empty, so a single `Enter` repeats it.
2. **Importance order:** Every other hint follows a fixed importance rank. Opening and search come first (`file-picker`, `workspace`, `find`, `replace`, `goto`), then everyday edits (`save`, `undo`, `redo`, clipboard), then language tools, then explorer file commands, then layout, then `setting`, `about`, and `quit`. The explorer uses the same rank.
3. **Zero-Latency Prefix Filtering:** As the user types, commands are filtered in real time by prefix/keyword match with zero debounce delay. Names that start with the input stay ahead of commands that only match a later word. Importance orders each of those groups, and the recent command is then moved to the top.
4. **Keyboard Flow:**
   * `Up` / `Down`: Cycle through matching command hints.
   * `Tab`: Leave the bar and open the assistant. `Tab` in the assistant opens the command bar again.
   * **Line editing:** the input is a one-line editor for every command. `Left` / `Right` move one character (a Thai letter and its marks, or an emoji, move as one). `Home` / `End` and `Ctrl + A` / `Ctrl + E` go to the start or end. `Ctrl + Left` / `Ctrl + Right` jump by word with the editor's rule. `Backspace` / `Delete` remove the character before or after the cursor, and `Ctrl + Backspace` (or `Alt + Backspace`) the previous word. Typing and paste insert at the cursor. `Shift` selection is not supported in the bar.
   * `Right`: With the cursor at the end of the input, accept the dimmed file or folder name drawn after the typed argument; anywhere else it moves the cursor. The hint is only drawn while the cursor is at the end. The block cursor sits on the first character of that hint, the same way a shell draws an inline suggestion. `move` hints folders, `rename` hints names in the same folder, `save` hints files and folders, `new-file` and `new-folder` hint folders (under the focused folder, or from the workspace root after a leading `/`), and `workspace` hints saved folders. The closest name is the shortest match, then A–Z. A name already typed in full stays as it is.
   * `Enter`: Execute the selected command immediately and return focus to the editor. An exact command name in the input (e.g. `goto 42`) wins over the highlighted hint. If the command needs an argument that is missing, `Enter` completes the name instead of failing.
   * `Esc` / `F1`: Dismiss the Command Bar without side effects.
5. **Focus context:** the bar only lists commands for the surface that was focused when it opened. An editor shows editing commands. The explorer shows `file-picker`, `workspace`, `find`, `replace`, `select`, `rename`, `new-file`, `new-folder`, `copy-name`, `copy-path`, `copy-relative-path`, `reveal`, `explorer`, `setting`, `about`, and `quit`. In FILE mode it also shows `move` and `delete`, in that order after `select` and before `rename`. `rename`, `new-file`, `new-folder`, and the three copy commands never need FILE mode; `rename`, `new-file`, and the copy commands are listed for an editor too (there the copy commands follow `paste`). A command from the other surface is not hinted, and typing its name does not run it. A bare line number is `goto` only while an editor is focused. The ★ recent command is the last one run on this surface, and it is the first hint whenever it is listed.

### 3.3 Built-in Commands

While an editor is focused:

| Command | Argument | Shortcut |
| :--- | :--- | :--- |
| `goto` (or just type the number) | `<line>` (1-based) | — |
| `copy` / `cut` / `paste` | — | `Ctrl + C` / `Ctrl + X` / `Ctrl + V` |
| `copy-name` / `copy-path` / `copy-relative-path` | — | — |
| `save` | optional path | `Ctrl + S` |
| `new-file` | `<path>` (nested, e.g. `images/images.rs`) | — |
| `rename` | `<name>` or a path; bare `rename` + `Enter` fills in the current name. Renames this buffer's file (in a focused Markdown preview, its source). Save first | — |
| `undo` / `redo` | — | `Ctrl + Z` / `Ctrl + Y` |
| `select-all` | — | `Ctrl + A` |
| `deselect` | — | `Ctrl + D`. Clear the selection, including a sticky one the cursor moved away from |
| `indent` | — | `Tab` |
| `outdent` | — | `Shift + Tab` |
| `agent` | — | `F6` |
| `delete-line` | — | `Shift + Delete` |
| `duplicate-line-up` / `duplicate-line-down` | — | `Ctrl + Alt + Up` / `Ctrl + Alt + Down` |
| `explorer` | — | `Ctrl + B` |
| `focus` | — | — |
| `file-picker` | — | `Ctrl + P` |
| `workspace` | `<folder>` | — |
| `split-right` / `split-down` | — | — |
| `preview` | none (opens the ↑ ↓ ← → side picker), or `split-up` / `split-down` / `split-left` / `split-right` (`up` / `down` / `left` / `right` also work) | — |
| `close` | — | — |
| `close-all` | — | — |
| `close-view` (`clean-view` is the same command) | — | — |
| `tab-prev` / `tab-next` | — | `Ctrl + Alt + Left` / `Ctrl + Alt + Right` |
| `setting` (`config` is the same command) | — | — |
| `about` | — | — |
| `quit` | — | `Ctrl + Q` |

`new-file` creates an empty file and opens it, without FILE mode. From an editor the path is relative to the workspace root, and a leading `/` or `\` also means the workspace root (never the disk root). Missing folders in a nested path are made. It refuses an existing path, a path outside the workspace, or no open folder.

The copy commands put the focused tab's file on the clipboard (system clipboard, or the in-memory one when the system clipboard is unavailable) and say `Copied <text>`, with a long path cut from the front behind a leading `…` so the file name stays visible. In a focused Markdown preview they use the preview's source file. `copy-name` gives `ARCHITECTURE.md`; `copy-path` gives the full path in the system's own form, without the Windows `\\?\` prefix; `copy-relative-path` gives `docs/ARCHITECTURE.md`. Relative paths always use `/`, the same as the command bar and the session file, so they paste into Markdown links and shells alike. The workspace folder itself copies as `.`. An untitled buffer is refused (`This buffer has no file yet; save it first`), and `copy-relative-path` refuses a file outside the workspace (`ARCHITECTURE.md is outside the workspace`) while the other two still work.

While the explorer is focused:

| Command | Argument | What it does |
| :--- | :--- | :--- |
| `select` | — | Enter FILE mode. `Space` marks or unmarks the focused file. Hold `Shift` and move to cover a range, then `Space` (without `Shift`) marks that range. `Shift + Space` opens the Command Bar. `Esc` returns to NORMAL and clears every mark. |
| `move` | `<dir>` | FILE mode only. Move the marked files (or the focused file) into a folder inside the workspace. |
| `delete` | — | FILE mode only. Delete those files. Asks first. Refuses while an open editor has unsaved changes for that file. |
| `rename` | `<name>` | No FILE mode needed. Rename the focused file or folder. A plain name stays in the same folder; a path with `/` moves it (relative to its folder, or from the workspace root after a leading `/`), making missing folders like `new-file`. Bare `rename` + `Enter` fills the bar with `rename <current name>` to edit. Refuses an empty name, an existing target (a case-only change such as `a.md` → `A.md` is allowed), a path outside the workspace, and moving a folder into itself. Open tabs follow, including files inside a renamed folder, and unsaved edits stay unsaved. The status reads `Renamed X → Y`. From an editor it renames that buffer's file and from a focused Markdown preview its source; an untitled or unsaved buffer must be saved first. |
| `new-file` | `<path>` | No FILE mode needed. Create an empty file and open it. A relative path starts in the focused folder (or the folder of the focused file); a leading `/` or `\` starts at the workspace root. Nested paths such as `images/images.rs` make the missing folders. Refuses an existing path or a path outside the workspace. |
| `copy-name` | — | No FILE mode needed. Copy the focused row's file or folder name, e.g. `ARCHITECTURE.md`. |
| `copy-path` | — | No FILE mode needed. Copy the focused row's full path in the system's own form, e.g. `D:\_Way\My Assets\Photalt\docs\ARCHITECTURE.md` on Windows (never the `\\?\` prefix). |
| `copy-relative-path` | — | No FILE mode needed. Copy the path from the workspace folder with `/` separators on every system, e.g. `docs/ARCHITECTURE.md`; a folder copies its folder path, e.g. `docs`. |
| `new-folder` | `<path>` | No FILE mode needed; explorer only. Create a folder in the focused folder (or the folder of the focused file), for example `dogs`. A leading `/` starts at the workspace root, so `/images/dogs` makes `images/dogs` and any missing parent. The tree refreshes and selects the new folder. Refuses an existing path or a path outside the workspace. |
| `explorer` | — | Toggle the sidebar (`Ctrl + B`) |
| `file-picker` | — | Open a workspace file by name (`Ctrl + P`) |
| `workspace` | `<folder>` | Open another folder. An empty argument lists saved folders. |
| `setting` (`config` is the same command) | — | Open the settings page |
| `about` | — | Show the version and the duck logo |
| `quit` | — | Exit duckx-code |

---

## 4. Visual Layout & UI Design

```text
 main.rs
   1  fn main() {
   2      println!("{}", message);█
   3  }
 ╭ Commands ─────────────────────────────────────────────────╮
 │› split-right      Open an empty editor to the right       │
 │  split-down       Open an empty editor below              │
 ╰─────────────────────────────────── ↑↓  tab  enter  esc ───╯
 COMMAND  ❯ spli█                                   Top   3:1    4 LF
```

### Command Bar
The composer is the status line. While it is open the badge reads `COMMAND` and the `❯` input replaces the file name. Matching commands sit in a rounded `Commands` list right above that line, one per row, in list order: name, description, and shortcut at the right edge. The highlighted row is a full-width bar with a `›` marker, and it is the one `Up` / `Down` moves, so the highlight travels the same direction as the arrow. `↑↓  tab  enter  esc` sits in the list's bottom border, and gains `→` while a file or folder hint is showing. When there are more matches than rows, the top border shows the position (`2/31`). The hint itself is dim text immediately after the characters already typed. The block cursor is drawn where the next character goes. Input wider than the line scrolls sideways (measured in display columns, so wide and Thai text line up) to keep the cursor on screen, and the position and encoding labels on the right step aside while the input needs their room. A bare `rename` fills in the current name with the cursor before the extension (`rename index|.html`); a name without an extension, a dot file, or a folder puts it at the end. The list is hidden when the bar closes, and the file name returns. These labels name where input goes; they are not vim modes, and printable keys still insert text whenever the bar is closed.

### Status Bar
Modeled on Neovim's lualine (LazyVim defaults, Tokyo Night palette) so it feels familiar to Neovim users:
* **Left:** `NORMAL` badge → git branch → file-type icon + path (directory dimmed, file name bold). The branch (branch icon plus name, or a short hash when HEAD is detached) is shown only inside a repository, in its own section between the badge and the file. One-shot messages appear after the path in yellow. While the command bar is open this side is `COMMAND` → `❯` input, and the branch and file name are hidden. While FIND mode is open it is `FIND` → `❯` search text, with one row above showing the match count and `↑↓  esc`. From the explorer the hint is `enter  esc`, and `Enter` lists matching files in the tree with a count on each row. The badge then reads `FILTER`. `replace` from there edits those files. While REPLACE mode is open it is `REPLACE` → `❯` selected text, then `→`, then the new text, with the cursor after the arrow from the start. The selected text is dimmed and cut off with `...` past 24 columns. The row above reads `↑ undo  ↓ next  ⇧↓ all  esc`. While the explorer is in FILE mode the badge reads `FILE`. Hints stack on the rows above, not on this line.
* **Right:** selection count (when selecting) → `Top` / `N%` / `Bot` progress + `line:col` → indent, line ending, and encoding. A number is how many spaces one Tab inserts (`4 LF UTF-8`). `Tab 4` means Tab inserts a tab character drawn that many columns wide. `LF` or `CRLF` is the ending a new line break uses, which is also what the next save writes when the line-ending setting is not Auto. The encoding is UTF-8 unless the file has a BOM or `encoding` / `reload` selected another one.
* Sections are joined with powerline separators and use Nerd Font icons.
* Deliberately omitted from the lualine original: showcmd (there are no multi-key sequences) and plugin-update counts (there is no plugin system).

### Explorer Sidebar
Modeled on the snacks.nvim explorer (LazyVim):

```text
╭────────────────────────────╮
│  src                 3/24 │
╰────────────────────────────╯
  duckx-code
 └╴ src
   ├╴ app.rs              M
   └╴ main.rs
```

* **Toggle with focus (`Ctrl + B`):** closed → open and focused; open but editor focused → focus the sidebar; sidebar focused → close. `Esc` returns to the editor without closing. In FILE mode the first `Esc` returns to NORMAL and clears every mark; the next `Esc` returns to the editor. The `explorer` command does the same cycle as `Ctrl + B` for terminals that intercept it.
* **Move between surfaces (PANE mode, `Ctrl + W`):** the status badge reads `PANE` with `← ↑ ↓ → move  Enter select  Esc cancel`. Focus does not move yet: each arrow moves a *candidate* in a straight line from the candidate's cursor (or the middle of the sidebar or assistant panel) to the nearest open window on that side. The candidate gets a bold amber rounded border (the theme's warning color) titled `▸ select`, drawn over that window's edge cells, including windows that normally have no border. The window that still has focus keeps its normal look, so "here" and "going there" read differently. `Enter` or `Ctrl + W` focuses the candidate; `Esc` cancels. If the candidate window closes while the mode is on, the candidate falls back to the window focus started from. A wider window underneath does not take the move when the cursor lines up with a narrower one. A closed sidebar is ignored, so `Left` in PANE mode from the leftmost editor does nothing until the sidebar is opened with `Ctrl + B`. From the sidebar, `Right` then `Enter` enters the editor beside it and leaves the sidebar open. In the sidebar, `Ctrl + Left` / `Ctrl + Right` / `Ctrl + Up` / `Ctrl + Down` act like the plain arrows (collapse, expand, move).
* **Width (`Alt + Right` / `Alt + Left`):** while the sidebar is focused, `Alt + Right` makes it four columns wider and `Alt + Left` makes it four columns narrower. It starts at 30 columns, never narrower than 16, and never wider than half the screen, so the editor keeps the other half. `wider` and `narrower` do the same from the Command Bar. The width lasts until duckx-code quits. In the editor those keys still move by word.
* **Navigation uses the same keys as the editor:** `Up`/`Down`/`PgUp`/`PgDn`/`Home`/`End` move, `Right` expands a folder (then steps into it), `Left` collapses (then steps out to the parent), `Enter` toggles a folder.
* **Type to filter:** there are no letter shortcuts in the tree, so every printable key goes to the filter box. The box has no title. A magnifying-glass icon marks the input, and the counter reads `matches/total`. Matches show with their ancestor folders.
* **What is shown:** `.gitignore` is respected and dotfiles are hidden. Folders come first, then files, both sorted case-insensitively.
* **Git markers:** `M` modified, `A` added, `D` deleted, `R` renamed, `?` untracked, colored at the right edge. A folder shows the strongest status found inside it.
* **`Enter` opens a file** as a tab in the focused window and moves focus there. A file that is already open is focused instead of loaded again. A clean untitled tab is reused; a tab that already has a path, or unsaved edits, stays open and the file loads in a new tab beside it. `Enter` on a folder still expands or collapses it.
* **File commands:** `select`, `rename`, `new-file`, and `new-folder` are listed while the explorer is focused. `move` and `delete` are listed only in FILE mode. Editor commands such as `goto` and `indent` are not listed there. See §3.2.
* **Marks:** `select` enters FILE mode (the status badge reads `FILE`). `Space` toggles a mark on the focused row (a green `●`). Hold `Shift` and move to cover rows (a green `○` and a selection background); release `Shift` and press `Space` to mark that cover, or to unmark it when every covered row is already marked. `Shift + Space` opens the Command Bar, so `move`, `delete`, and `rename` run without leaving the mode. Movement without `Shift` clears the cover and leaves marks in place. `Esc`, closing the sidebar, or moving focus to an editor returns to NORMAL and clears every mark. A marked folder includes everything inside it. `rename` always renames the focused row.
* The scan and `git status` run in the background, so the editor stays responsive on large trees (`Loading…` is shown meanwhile). A file or folder created, removed, or renamed on disk refreshes an open tree the same way. The rows stay up until that scan arrives, so a later refresh does not flash `Loading…`. Deletes, renames, moves, and `new-file` / `new-folder` also run in the background and then refresh the tree. An open editor follows a rename or move. Deleting a file that is open keeps its text and marks that editor unsaved.

### Editor Windows
* **`split-right` / `split-down`:** split only the focused editor in half. The new pane is an empty `[No Name]` buffer with its own cursor and undo history, and it receives focus. There is no shortcut yet.
* **`preview`:** a read-only Markdown preview of the focused `.md` (or `.markdown`) file, opened as a real window above, below, left of, or right of the source window. Other file types are refused (`preview works on Markdown (.md) files`) before any popup appears. Focus stays on the source.
  * **Side picker:** `preview` with no argument opens a small rounded panel (R1, `ui::panel`) titled `Preview <file>`, laid out like a compass: `↑ Up` on top, `← Left` and `Right →` in the middle row, `↓ Down` below, and the hint `←↑↓→ open · enter · esc close`. An arrow key opens the preview on that side at once (`u` / `d` / `l` / `r` do the same). `Enter` opens the highlighted side, which starts on the last side used (right the first time); `Tab` / `Shift + Tab` move the highlight. `Esc` closes the picker without a preview.
  * **Typed side:** `preview split-up`, `split-down`, `split-left`, or `split-right` (or `up` / `down` / `left` / `right`) skips the picker; `Right` accepts the hinted side while typing.
  * **The window:** The preview is a rounded panel (R1) titled `<file> · preview` with a light blue border; the source canvas stays borderless. Rendering (theme colors throughout):
    * Headings get a clear hierarchy: H1 is bold with a full-width `━` rule under it, H2 and H3 are bold in their own accent colors, and H4–H6 step down from there. Every block is separated by one blank line.
    * Paragraphs wrap to the window by display width, so Thai text (marks stay on their letter) and emoji such as 🎨 line up. Words break at spaces; a run wider than the line splits between characters.
    * Lists indent per level with `•` `◦` `▪`, numbered lists right-align their numbers (`9.` and `10.`), and task items show `☐` / `☑` in place of the bullet. A wrapped item continues under its text, not under the bullet.
    * Block quotes carry a `│` bar on every line, nested quotes one bar per level.
    * Fenced code sits in a rounded frame with the language in its top edge, on a raised background, colored by the editor's tree-sitter highlighter for the languages it knows. Long code lines wrap at spaces inside the frame.
    * Tables draw as a rounded box-drawing grid with a header separator, honoring `:--`, `:-:`, and `--:` alignment. Columns keep their natural width when they fit and otherwise share the width fairly, wrapping cells.
    * Horizontal rules span the width. Links are underlined in the link color with the URL hidden (a link with no text shows its URL). Bold, italic, strikethrough, and inline code (its own background) are styled.
    * Images show as `[image: alt]`; nothing is loaded.
    * The rendered lines are cached per source revision, width, and theme, so drawing a frame does not lay out the file again; an edit (unsaved ones included) shows at once. A thin scrollbar sits inside the right padding when the document is taller than the window.
    * The preview follows its source window: switching that window to another Markdown tab shows that file (a non-Markdown tab leaves the last Markdown file showing).
    * One preview per file: the same side again only says it is already open, and any other side moves it there instead of adding a second one.
  * In the focused preview, `Up` / `Down` scroll one row, `Page Up` / `Page Down` a page, and `Ctrl + Home` / `Ctrl + End` jump to the ends. Typing, paste, undo, save, find, and editor commands are refused with `The preview is read-only`. There is no cursor and nothing to save: the window holds no document of its own, and it is left out of the saved session. The status badge reads `PREVIEW` and the path shows `<file> · preview`.
  * `close` in the preview closes only the preview. Closing the source tab (or its window) also closes its preview. A file opened while the preview is focused opens beside the source, never inside the preview.
* **`close`:** closes the focused tab. If that tab has unsaved changes, duckx-code asks before discarding them. When other tabs remain, the window stays and the next tab takes focus. The last tab of a split window closes that window. Closing the last tab of the last window leaves a blank untitled page.
* **`close-all`:** closes every tab in every window. Each unsaved tab is asked with the same `Yes` / `No` / `Cancel` dialog. `Yes` saves, `No` discards that tab and continues, and `Cancel` leaves the rest open. The screen ends on one blank untitled page.
* **`close-view`:** closes every tab in the focused window. `clean-view` is the same command. Unsaved tabs in that window are asked the same way. Other windows stay open. The last window becomes a blank untitled page; a split window closes.
* **Which pane is active:** its line number is yellow, its active tab is bold, and it owns the block cursor. Other panes keep their text and scroll position, with the active tab dimmed. A dim `│` or `─` separates panes. The editor canvas itself has no border.
* **`focus`:** the screen is only the buffer in use. The tab row, status bar, diagnostic line, sidebar, and other windows hide. The command bar is drawn only while it is open, and find or replace use that line while they are open. PANE mode still works: the candidate editor window fills the screen with the amber border while you choose, `Enter` keeps it, and `Esc` returns to the window you started from. The sidebar is not a candidate in this view. `Ctrl + B` and a move onto the sidebar do nothing in this view. Only `focus` again restores the hidden chrome.
* **`diff`:** toggles a gutter column comparing the buffer, including unsaved edits, with `HEAD`. `┃` marks an added or modified line in that git color. `‾`, a digit, or `+` marks lines removed just above that row. The status line says `diff` while this is on. The comparison runs in the background.
* **Tabs:** one row at the top of every pane. Each tab is a file-type icon when the name is a recognized type, then the file name, or `[No Name]`, with `[+]` when that tab has unsaved edits. The active tab uses the title color (bold while the pane is focused). Other tabs are dim. A row that does not fit shows `‹` and `›` for the tabs scrolled out of view. No background or border. `Ctrl + Alt + Left` / `Right` cycle the tabs of the focused window, wrapping, without moving the cursor. `tab-prev` and `tab-next` do the same from the Command Bar.
* **Status bar:** path and cursor position always describe the focused editor.

### Settings
`setting` or `config` covers the editor and the sidebar with one rounded page. The section list stays on the left (`Editor`, `Appearance`, `Files`, `Keybindings`) and the rows of the highlighted section stay on the right. `Up` / `Down` move in the column that has focus. `Enter` or `Right` moves into that section's rows. `Esc`, `Left`, or `Backspace` steps back to the section list, and from the section list closes the page. The status badge reads `SETTINGS` while the page is open. Typing does not insert into the buffer.

**Appearance** has two saved choices. **Color theme** colors the interface (editor, sidebar, status bar, dialogs). **Syntax colors** colors code. They are chosen separately, so a dark interface can sit under a different syntax palette. `Enter` or `Right` on a row opens the choices in a rounded list (see **Dropdown lists** under R1) right under that row and aligned to the value: the highlighted name is a full-width bar in the accent color, and `↑↓ move  enter select  esc` sits in the bottom border. `Up` / `Down` moves through the list and a theme or an editor value updates immediately. `Enter` keeps the highlighted value and saves it. `Esc`, `Left`, or `Backspace` closes the list and restores the value that was selected when it opened. The syntax row shows a short colored sample (`fn main() { "hi" }`). Built-in themes are Tokyo Night and the four Catppuccin flavors (Latte, Frappé, Macchiato, Mocha). A JSON file in the user theme folder replaces a built-in when the `id` matches, and a new `id` is added to the list.

**Editor** stores tab width (2, 4, or 8), whether Tab inserts spaces, and the line ending written on save (`Auto`, `LF`, or `CRLF`). Those values apply to the next Tab and to new line breaks. Text already in the file is not rewritten until a save that uses a forced line ending. **Files** stores whether a save trims trailing whitespace and whether it adds a final newline. **Keybindings** lists each command chord. `Enter` waits for the next key. `Delete` on a row restores that row. `Delete` on a section name restores every row in the section. Arrow keys, plain characters, `Ctrl` + arrows, `F1`, `Ctrl + ;`, and `Ctrl + Q` cannot be reassigned. A chord that another command already uses is refused.

### About
`about` covers the editor and the sidebar with one rounded page. It shows the duck logo, the name, and the version. The status badge reads `ABOUT`. `Esc`, `Backspace`, or `F1` closes the page. Typing does not insert into the buffer. `Ctrl + Q` still quits.

### File picker
`file-picker` or `Ctrl + P` opens a rounded list over the editor. The status badge reads `OPEN`. The walk respects `.gitignore` and skips hidden files, on a background job, and the previous list stays up while a refresh runs. An empty query lists files opened in this session first, then the rest by file name. Words separated by spaces must all match. A word that starts with `!` drops any path that contains that text. A lowercase query ignores case; any capital letter matches case. `\` is the same as `/`. A match in the file name ranks above the same match in a directory, and a file in the focused file's folder ranks above an equal match elsewhere. `Up` / `Down` / `PgUp` / `PgDn` move the highlight. `Left` / `Right` / `Home` / `End` move the cursor in the query. `Ctrl + Left` / `Ctrl + Right` move by word, and `Ctrl + Backspace` deletes the previous word. `Ctrl + Home` / `Ctrl + End` jump to the first or last file. `Enter` opens the highlighted file. `Esc` closes the list. `Shift + Space` or `F1` closes it and opens the Command Bar.

### Dialogs
Every confirmation uses one dialog. The caller supplies the title, the message, and the buttons. Nothing in the widget is specific to delete or to unsaved edits. The usual buttons are `Yes`, `No`, and `Cancel`. `Left` / `Right` (and `Up` / `Down`) move the highlight, `Enter` confirms it, and an underlined letter is only a shortcut for that button. `Esc` confirms `Cancel` and is not drawn. The message stays one column in from the border and sits on the row under the top edge. The buttons are centered on their row and sit on the row above the bottom edge, with one blank row between the message and the buttons.

### UI Principles
* **Distraction-Free Canvas:** Each pane has a one-line tab strip instead of a heavy tab bar. The explorer sidebar is hidden until toggled with `Ctrl + B`; by default all screen real estate is dedicated to code.
* **Clear Visual Feedback:**
  * Active selection rendered with high-contrast background highlighting.
  * Inline diagnostic indicators in the gutter (`E`, `W`, `I`, `H`) and subtle diagnostic underlines/highlights.
  * Unsaved changes clearly marked (`[+]`) after the file name in the status bar, and on that tab.
* **Non-Intrusive Completion:** Autocomplete popups appear anchored directly below the cursor and never block typing or arrow navigation when dismissed with `Esc`.

---

## 5. UI Requirements

These are **mandatory** rules for every UI element. A change that breaks one of them is a bug.

### R1. Rounded Borders Only
* Every bordered element (popups, overlays, panels, dialogs, completion menus) uses rounded corners: `╭ ╮ ╰ ╯` with `─ │` edges.
* Square (`┌ ┐ └ ┘`), double (`╔ ╗`), and thick (`┏ ┓`) borders are not allowed.
* **Implementation:** create bordered blocks only through `ui::panel(title)`, which applies `BorderType::Rounded`. Never call `Block::bordered()` / `Block::default().borders(..)` directly in UI code.
* **Enforced by:** the `panel_borders_are_rounded` render test in `src/ui/mod.rs`.
* **Scope:** the main editor canvas and status bar have no border; this rule applies to anything that does.
* **Dropdown lists:** every list that opens over other content is drawn by `ui::dropdown`: settings choices, the Server / Model recent list, the command-bar matches and saved folders, the assistant's `/` and `@` lists, and the editor's completion, references, and actions popups. Each one is a rounded panel on the editor background with the accent border, clears what is behind it, marks the highlighted row with `›` and a full-width bar in the editor selection color, puts its keys in the bottom border, aligns tags (`recent`, `server`, shortcuts) to the right edge, and shows `n/len` in the top border when it scrolls. It opens under its row and flips above when there is more room there. An area too small for a frame (under 3 rows or 6 columns) gets plain rows instead, never a panic.
* **Context gauge:** the assistant panel's otherwise empty bottom border carries `ctx used/limit percent`, right-aligned with one border dash before the `╯` corner, so the rounded frame and its focus color are untouched. Text is the panel's dim color, the theme's warning color from 70 %, and its error color from 90 %. A narrow panel drops the `ctx` word and percent, then the whole counter. Server-reported usage wins over the chars ÷ 3.5 estimate. Auto compact folds older turns into a summary in the system prompt, cutting only at a user message so tool calls and results stay paired; a failed summary falls back to dropping those turns.
* **Per-model limits:** `ai.model_limits` maps `"<provider> <model>"` to an optional `context` and `max_output`. Lookups go model entry first, then the global Context size / Max tokens rows, so a file without the map, and a model without an entry, behave exactly as before. An entry with nothing set is removed, and the map is left out of `settings.json` when empty. The Settings rows always edit the model that is selected now. `AiPrefs::model_key()` is the single key helper for both `ai.model_limits` and `ai.model_tools`; it uses the provider kind, so a named provider shares per-model entries with the built-in of its kind.
* **Model capabilities:** `Tuning.tools` carries the per-model mode (`Auto` / `On` / `Off`) to every request path, so no worker signature changes. Only `Auto` consults or updates the learned no-tools set. The learned sets stay process-wide in the provider; after each turn the worker reports what is known for that server and model (`AgentEvent::Detected`), `App` saves it under `ai.detected`, and `attach_agent` seeds it back at start-up. Keys are `"<chat url> <model>"`.
* **Named providers:** `AiPrefs` keeps one live set of `provider` / `server` / `model` / `api_key`. A built-in parks its values in `profiles`; a named provider parks them in its `ai.named` entry, and `ai.active` says which named one is live (empty: a built-in). Every switch first stashes the live values where they belong, then loads the target, so nothing is copied twice and an older file (no `named`, no `active`) is untouched. While a named provider is live, `provider` is its kind, so native Ollama, the local token cap, and OpenRouter headers follow the kind with no extra branches. Recent servers and models are keyed by kind and server, so a named provider shares that history with the built-in of its kind at the same server.

### R2. Block Cursor
* The text cursor is always a steady (non-blinking) block (`SetCursorStyle::SteadyBlock`), not a bar or underline. A blinking terminal cursor fights each frame: `Terminal::draw` hides the cursor, moves it across changed cells, then shows it again, which restarts the blink.
* On exit, the terminal's own cursor shape is restored (`SetCursorStyle::DefaultUserShape`).
