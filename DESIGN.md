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
| `Ctrl + Left` / `Ctrl + Right` / `Ctrl + Up` / `Ctrl + Down` | Focus adjacent surface | Moves focus to the open window that shares an edge on that side. A closed window, including a closed sidebar, is not a destination. See §4. |
| `Ctrl + Alt + Left` / `Ctrl + Alt + Right` | Previous / next tab | Cycles the tabs of the focused window. Does not move the cursor. Also works while the sidebar is focused. See §4. |
| `Alt + Left` / `Alt + Right` | Move by Word | Jumps to the start/end of the previous or next word token. While the sidebar is focused, these keys narrow and widen it instead |
| `Home` | Line Start (Smart Home) | Jumps to the first non-whitespace character on the line; pressing again jumps to column 0 |
| `End` | Line End | Jumps to the end of the current line |
| `Ctrl + Home` / `Ctrl + End` | Document Start / End | Jumps to the very beginning or very end of the document |
| `PgUp` / `PgDn` | Page Scroll | Scrolls the viewport and moves the cursor up or down by one screen height |

### 2.2 Shift-Driven Selection
Holding `Shift` with any navigation key anchors the selection at the starting cursor position (`anchor`) and extends the active cursor (`head`):

| Keybinding | Selection Behavior |
| :--- | :--- |
| `Shift + Up / Down / Left / Right` | Character-by-character or line-by-line selection extension |
| `Alt + Shift + Left / Right` | Word-by-word selection extension |
| `Shift + Home` / `Shift + End` | Select from cursor to start or end of the current line |
| `Shift + PgUp` / `Shift + PgDn` | Select an entire viewport page up or down |
| `Ctrl + Shift + Home / End` | Select from cursor to the beginning or end of the document |
| `Ctrl + A` | Select the entire document buffer |

*Typing any printable character, `Backspace`, or `Delete` while a selection is active replaces the entire selected region atomically.*

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
   * `Right`: Accept the dimmed file or folder name drawn after the typed argument. The block cursor sits on the first character of that hint, the same way a shell draws an inline suggestion. `move` hints folders, `rename` hints names in the same folder, `save` hints files and folders, `new-file` and `new-folder` hint folders (under the focused folder, or from the workspace root after a leading `/`), and `workspace` hints saved folders. The closest name is the shortest match, then A–Z. A name already typed in full stays as it is.
   * `Enter`: Execute the selected command immediately and return focus to the editor. An exact command name in the input (e.g. `goto 42`) wins over the highlighted hint. If the command needs an argument that is missing, `Enter` completes the name instead of failing.
   * `Esc` / `F1`: Dismiss the Command Bar without side effects.
5. **Focus context:** the bar only lists commands for the surface that was focused when it opened. An editor shows editing commands. The explorer shows `file-picker`, `workspace`, `find`, `replace`, `select`, `new-file`, `new-folder`, `reveal`, `explorer`, `setting`, `about`, and `quit`. In FILE mode it also shows `move`, `delete`, and `rename`, in that order after `select` and before `new-file`. `new-file` and `new-folder` never need FILE mode, and `new-file` is listed for an editor too. A command from the other surface is not hinted, and typing its name does not run it. A bare line number is `goto` only while an editor is focused. The ★ recent command is the last one run on this surface, and it is the first hint whenever it is listed.

### 3.3 Built-in Commands

While an editor is focused:

| Command | Argument | Shortcut |
| :--- | :--- | :--- |
| `goto` (or just type the number) | `<line>` (1-based) | — |
| `copy` / `cut` / `paste` | — | `Ctrl + C` / `Ctrl + X` / `Ctrl + V` |
| `save` | optional path | `Ctrl + S` |
| `new-file` | `<path>` (nested, e.g. `images/images.rs`) | — |
| `undo` / `redo` | — | `Ctrl + Z` / `Ctrl + Y` |
| `select-all` | — | `Ctrl + A` |
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
| `close` | — | — |
| `close-all` | — | — |
| `close-view` (`clean-view` is the same command) | — | — |
| `tab-prev` / `tab-next` | — | `Ctrl + Alt + Left` / `Ctrl + Alt + Right` |
| `setting` (`config` is the same command) | — | — |
| `about` | — | — |
| `quit` | — | `Ctrl + Q` |

`new-file` creates an empty file and opens it, without FILE mode. From an editor the path is relative to the workspace root, and a leading `/` or `\` also means the workspace root (never the disk root). Missing folders in a nested path are made. It refuses an existing path, a path outside the workspace, or no open folder.

While the explorer is focused:

| Command | Argument | What it does |
| :--- | :--- | :--- |
| `select` | — | Enter FILE mode. `Space` marks or unmarks the focused file. Hold `Shift` and move to cover a range, then `Space` (without `Shift`) marks that range. `Shift + Space` opens the Command Bar. `Esc` returns to NORMAL and clears every mark. |
| `move` | `<dir>` | FILE mode only. Move the marked files (or the focused file) into a folder inside the workspace. |
| `delete` | — | FILE mode only. Delete those files. Asks first. Refuses while an open editor has unsaved changes for that file. |
| `rename` | `<name>` | FILE mode only. Rename the focused file or folder. The new name stays in the same folder. |
| `new-file` | `<path>` | No FILE mode needed. Create an empty file and open it. A relative path starts in the focused folder (or the folder of the focused file); a leading `/` or `\` starts at the workspace root. Nested paths such as `images/images.rs` make the missing folders. Refuses an existing path or a path outside the workspace. |
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
 split-right   Open an empty editor to the right
 split-down    Open an empty editor below          ↑↓  tab  enter  esc
 COMMAND  ❯ spli█                                   Top   3:1    4 LF
```

### Command Bar
The composer is the status line. While it is open the badge reads `COMMAND` and the `❯` input replaces the file name. Matching commands stack upward from that line, one per row, in list order: name, description, and shortcut. The highlighted row is the one `Up` / `Down` moves, so the highlight travels the same direction as the arrow. `↑↓  tab  enter  esc` sits at the right of the highlighted row, and gains `→` while a file or folder hint is showing. The hint itself is dim text immediately after the characters already typed. The list is hidden when the bar closes, and the file name returns. These labels name where input goes; they are not vim modes, and printable keys still insert text whenever the bar is closed.

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
* **Move between surfaces (`Ctrl + Arrows`):** focus moves in a straight line from the cursor (or the middle of the sidebar) to the nearest open window on that side. A wider window underneath does not take the move when the cursor lines up with a narrower one. A closed sidebar is ignored, so `Ctrl + Left` in the leftmost editor does nothing until the sidebar is opened with `Ctrl + B`. From the sidebar, `Ctrl + Right` enters the editor beside it and leaves the sidebar open.
* **Width (`Alt + Right` / `Alt + Left`):** while the sidebar is focused, `Alt + Right` makes it four columns wider and `Alt + Left` makes it four columns narrower. It starts at 30 columns, never narrower than 16, and never wider than half the screen, so the editor keeps the other half. `wider` and `narrower` do the same from the Command Bar. The width lasts until duckx-code quits. In the editor those keys still move by word.
* **Navigation uses the same keys as the editor:** `Up`/`Down`/`PgUp`/`PgDn`/`Home`/`End` move, `Right` expands a folder (then steps into it), `Left` collapses (then steps out to the parent), `Enter` toggles a folder.
* **Type to filter:** there are no letter shortcuts in the tree, so every printable key goes to the filter box. The box has no title. A magnifying-glass icon marks the input, and the counter reads `matches/total`. Matches show with their ancestor folders.
* **What is shown:** `.gitignore` is respected and dotfiles are hidden. Folders come first, then files, both sorted case-insensitively.
* **Git markers:** `M` modified, `A` added, `D` deleted, `R` renamed, `?` untracked, colored at the right edge. A folder shows the strongest status found inside it.
* **`Enter` opens a file** as a tab in the focused window and moves focus there. A file that is already open is focused instead of loaded again. A clean untitled tab is reused; a tab that already has a path, or unsaved edits, stays open and the file loads in a new tab beside it. `Enter` on a folder still expands or collapses it.
* **File commands:** `select`, `new-file`, and `new-folder` are listed while the explorer is focused. `move`, `delete`, and `rename` are listed only in FILE mode. Editor commands such as `goto` and `indent` are not listed there. See §3.2.
* **Marks:** `select` enters FILE mode (the status badge reads `FILE`). `Space` toggles a mark on the focused row (a green `●`). Hold `Shift` and move to cover rows (a green `○` and a selection background); release `Shift` and press `Space` to mark that cover, or to unmark it when every covered row is already marked. `Shift + Space` opens the Command Bar, so `move`, `delete`, and `rename` run without leaving the mode. Movement without `Shift` clears the cover and leaves marks in place. `Esc`, closing the sidebar, or moving focus to an editor returns to NORMAL and clears every mark. A marked folder includes everything inside it. `rename` always renames the focused row.
* The scan and `git status` run in the background, so the editor stays responsive on large trees (`Loading…` is shown meanwhile). A file or folder created, removed, or renamed on disk refreshes an open tree the same way. The rows stay up until that scan arrives, so a later refresh does not flash `Loading…`. Deletes, renames, moves, and `new-file` / `new-folder` also run in the background and then refresh the tree. An open editor follows a rename or move. Deleting a file that is open keeps its text and marks that editor unsaved.

### Editor Windows
* **`split-right` / `split-down`:** split only the focused editor in half. The new pane is an empty `[No Name]` buffer with its own cursor and undo history, and it receives focus. There is no shortcut yet.
* **`close`:** closes the focused tab. If that tab has unsaved changes, duckx-code asks before discarding them. When other tabs remain, the window stays and the next tab takes focus. The last tab of a split window closes that window. Closing the last tab of the last window leaves a blank untitled page.
* **`close-all`:** closes every tab in every window. Each unsaved tab is asked with the same `Yes` / `No` / `Cancel` dialog. `Yes` saves, `No` discards that tab and continues, and `Cancel` leaves the rest open. The screen ends on one blank untitled page.
* **`close-view`:** closes every tab in the focused window. `clean-view` is the same command. Unsaved tabs in that window are asked the same way. Other windows stay open. The last window becomes a blank untitled page; a split window closes.
* **Which pane is active:** its line number is yellow, its active tab is bold, and it owns the block cursor. Other panes keep their text and scroll position, with the active tab dimmed. A dim `│` or `─` separates panes. The editor canvas itself has no border.
* **`focus`:** the screen is only the buffer in use. The tab row, status bar, diagnostic line, sidebar, and other windows hide. The command bar is drawn only while it is open, and find or replace use that line while they are open. `Ctrl + Arrows` still moves to another editor window, which then fills the screen. `Ctrl + B` and a move onto the sidebar do nothing in this view. Only `focus` again restores the hidden chrome.
* **`diff`:** toggles a gutter column comparing the buffer, including unsaved edits, with `HEAD`. `┃` marks an added or modified line in that git color. `‾`, a digit, or `+` marks lines removed just above that row. The status line says `diff` while this is on. The comparison runs in the background.
* **Tabs:** one row at the top of every pane. Each tab is a file-type icon when the name is a recognized type, then the file name, or `[No Name]`, with `[+]` when that tab has unsaved edits. The active tab uses the title color (bold while the pane is focused). Other tabs are dim. A row that does not fit shows `‹` and `›` for the tabs scrolled out of view. No background or border. `Ctrl + Alt + Left` / `Right` cycle the tabs of the focused window, wrapping, without moving the cursor. `tab-prev` and `tab-next` do the same from the Command Bar.
* **Status bar:** path and cursor position always describe the focused editor.

### Settings
`setting` or `config` covers the editor and the sidebar with one rounded page. The section list stays on the left (`Editor`, `Appearance`, `Files`, `Keybindings`) and the rows of the highlighted section stay on the right. `Up` / `Down` move in the column that has focus. `Enter` or `Right` moves into that section's rows. `Esc`, `Left`, or `Backspace` steps back to the section list, and from the section list closes the page. The status badge reads `SETTINGS` while the page is open. Typing does not insert into the buffer.

**Appearance** has two saved choices. **Color theme** colors the interface (editor, sidebar, status bar, dialogs). **Syntax colors** colors code. They are chosen separately, so a dark interface can sit under a different syntax palette. `Enter` or `Right` on a row opens the choices in the same rows as the command list: full width of that column, no border, the highlighted name in the accent color, and `↑↓  enter  esc` at the right of that row. `Up` / `Down` moves through the list and a theme or an editor value updates immediately. `Enter` keeps the highlighted value and saves it. `Esc`, `Left`, or `Backspace` closes the list and restores the value that was selected when it opened. The syntax row shows a short colored sample (`fn main() { "hi" }`). Built-in themes are Tokyo Night and the four Catppuccin flavors (Latte, Frappé, Macchiato, Mocha). A JSON file in the user theme folder replaces a built-in when the `id` matches, and a new `id` is added to the list.

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

### R2. Block Cursor
* The text cursor is always a steady (non-blinking) block (`SetCursorStyle::SteadyBlock`), not a bar or underline. A blinking terminal cursor fights each frame: `Terminal::draw` hides the cursor, moves it across changed cells, then shows it again, which restarts the blink.
* On exit, the terminal's own cursor shape is restored (`SetCursorStyle::DefaultUserShape`).
