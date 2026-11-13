# DOCUMENTS.md — Keybindings & Command Reference

This is the user reference for every keybinding and Command Bar command that **duckx-code currently supports**. [Planned](#12-planned) lists designed behavior that is not built yet.

> For the design rationale behind these bindings, see [`DESIGN.md`](./DESIGN.md).
>
> Some shortcuts cannot reach duckx-code in every terminal (for example `Shift + Space`, `Shift + Enter`, `Ctrl + Shift + Z`, or `Ctrl + Alt + arrows`). At startup duckx-code turns on the kitty keyboard protocol where the terminal supports it, so every modifier is reported. [`docs/TERMINAL-KEYS.md`](./docs/TERMINAL-KEYS.md) has a per-terminal table of what still does not work, the setting to change in each terminal, and the fallback to use.

## Getting Started

```powershell
cargo run                    # open the current folder, with its last files and layout
cargo run -- path\to\file.rs # open a file (a missing path opens as a new file)
```

Starting with no path opens the folder the command was started from. Files, cursors, split windows, and the sidebar (open or closed, its width, and which folders were expanded) come back from the last quit in that same folder. Another folder keeps its own session and is not opened in its place. Both live in `session.json`, next to `settings.json`. A file that was deleted is skipped. Text that was never saved is not restored. Passing a file path opens that file instead, in the folder the command was started from.

There are no modes. Every printable key types text immediately. `Ctrl + S` saves the file. `Ctrl + Q` quits; if a file has unsaved changes, duckx-code asks first. `Ctrl + C` copies — it does not quit.

The status bar uses Nerd Font icons, so set your terminal font to a [Nerd Font](https://www.nerdfonts.com/) (e.g. *JetBrainsMono Nerd Font*, *CaskaydiaCove Nerd Font*).

---

## 1. Navigation

All movement uses the dedicated navigation cluster of a 75% keyboard.

| Keys | Action | Notes |
| :--- | :--- | :--- |
| `Left` / `Right` | Move one character | Moves by visible character (grapheme), so combining marks, Thai vowels/tone marks, and emoji count as one. Crosses line boundaries. |
| `Up` / `Down` | Move one line | Remembers the visual column across lines of different length, including wide (CJK) characters and tabs. On the first or last line, jumps to the line start or end. |
| `Alt + Left` / `Alt + Right` | Move by word | Stops at boundaries between words, punctuation, and whitespace. While the sidebar is focused, these keys narrow and widen it instead. See [Explorer Sidebar](#8-explorer-sidebar). |
| `Ctrl + Alt + Left` / `Ctrl + Alt + Right` | Previous / next tab | Cycles the tabs in the focused window and wraps. Does not move the cursor. The same keys work while the sidebar is focused. |
| `Home` | Smart line start | First press goes to the first non-whitespace character; the next press goes to column 1, and further presses toggle between the two. |
| `End` | Line end | |
| `Ctrl + Home` / `Ctrl + End` | Document start / end | |
| `PgUp` / `PgDn` | Move one screen | The viewport scrolls with the cursor, so the cursor keeps its row on screen. |

The view keeps 3 lines of context above and below the cursor and scrolls horizontally for long lines.

---

## 2. Selection

Hold `Shift` with **any** navigation key to select. The selection starts where the cursor was (anchor) and extends to where it moves (head).

| Keys | Selects |
| :--- | :--- |
| `Shift + Left / Right` | Character by character |
| `Shift + Up / Down` | Line by line |
| `Alt + Shift + Left / Right` | Word by word |
| `Shift + Home` / `Shift + End` | To line start / end |
| `Shift + PgUp` / `Shift + PgDn` | One screen up / down |
| `Ctrl + Shift + Home / End` | To document start / end |
| `Ctrl + Shift + ]` | Through the matching bracket |
| `Ctrl + A` | Entire document |

Behavior while a selection is active:
* Typing, `Enter`, `Backspace`, `Delete`, or pasting replaces the selected text.
* `Left` / `Right` without `Shift` collapses the selection to its start / end.
* Other movement keys without `Shift` drop the selection and move from the cursor.
* The status bar shows how many characters are selected.

---

## 3. Editing

| Keys | Action | Notes |
| :--- | :--- | :--- |
| Any printable key | Insert character | Includes characters typed with `AltGr` on international layouts. |
| `Enter` | New line | Carries over the indentation of the current line. |
| `Backspace` | Delete backward | Inside leading spaces, removes one indent level back to the previous tab stop. |
| `Delete` | Delete forward | |
| `Shift + Delete` or `delete-line` | Delete line | Deletes the current line, or every line the selection touches. A selection that ends at the start of a line does not include that line. The caret moves to the start of the next line, or to the end of the previous line when those lines were the end of the file. One undo step. The text is not copied to the clipboard. |
| `Ctrl + Alt + Up` / `Ctrl + Alt + Down` or `duplicate-line-up` / `duplicate-line-down` | Duplicate line | Copies the current line, or every line the selection touches, above or below. The caret keeps its column on the copy. A selection covers that same span on the copy. One undo step. |
| Terminal paste | Insert text | Inserted as copied: no auto-indent, no auto-close, tabs stay tabs. Line endings are converted to the file's style (`LF` or `CRLF`). The whole paste is one undo step. On Windows, where the terminal sends a paste as separate key presses, a fast run of keys that holds a line break or tab (or is 32 keys long) is treated as one paste. |
| `Ctrl + C` | Copy | Copies the selection. With no selection, copies the whole current line including its line ending. |
| `Ctrl + X` | Cut | Cuts the same text `Ctrl + C` would copy, as one undo step. |
| `Ctrl + V` | Paste | Inserts the OS clipboard at the cursor, replacing any selection. |
| `Ctrl + S` | Save | Writes the file atomically and clears the modified marker. See [Saving](#saving). |
| ``Alt + ` `` or `convert-keyboard` | Fix the keyboard layout | Retypes the selection as the same keys give on the other layout, so `ouj8nv8ezbf` becomes `นี่คือคำผิด`, and back. See [Wrong keyboard layout](#wrong-keyboard-layout). One undo step. |

`Ctrl + C`, `Ctrl + X`, `Ctrl + V`, and `Ctrl + S` also work while the sidebar is focused. They act on the focused editor. In a terminal that cannot read the OS clipboard, copy and paste use an internal clipboard that only works inside duckx-code.

### Wrong keyboard layout

Typed a line with the wrong layout on? Select it and press ``Alt + ` `` (or run `convert-keyboard`).

* **What changes:** every character is looked up on the layout it was typed on and replaced by the character the same key gives on the other layout, Shift included. US QWERTY ↔ Thai Kedmanee is built in: `ouj8nv8ezbf` → `นี่คือคำผิด`, `ทำหะ` → `mest`. Thai vowels above or below a letter and tone marks are their own keys, so each one converts on its own. Spaces, new lines, and characters neither layout types stay as they are.
* **Which way:** decided from the text. Mostly Thai letters convert to English; Latin letters, digits, or nothing either layout owns (`/-`) convert to Thai.
* **No selection:** the word right before the cursor (back to the previous space on that line) is converted and the cursor stays after it, so you can keep typing. With a space or the line start before the cursor, a notice says `Select the text to convert first`.
* **After converting:** a selection stays selected over the new text, so a second ``Alt + ` `` turns it back. The whole change is one undo step.
* **Windows:** the installed input languages are read from Windows. When there are exactly two and they are US English (QWERTY) and Thai (Kedmanee), the text converts at once and Windows is asked to switch the terminal to the target layout, so you keep typing in the right language. The notice says `keyboard switched to …`, or `could not switch the keyboard` when Windows refused. Other keyboards for the same language (US Dvorak, Thai Pattachote) are not treated as these two.
* **Elsewhere, or more layouts:** on Linux and macOS, or on Windows with more than two layouts or one duckx-code has no table for, a `Convert to` list opens under the selection. Each row shows a layout and what the text becomes on it. `Up` / `Down` move, `Enter` (or ``Alt + ` `` again) converts, `Esc` closes. Other keys are ignored while it is open. On Windows the keyboard still switches when the chosen layout is installed.
* **Assistant prompt:** the same key works on the selection, or the word before the cursor, in the assistant prompt.
* **Shortcuts on the Thai layout:** `Ctrl` or `Alt` with a Thai character runs the shortcut of the same key on the US layout, so `Ctrl + ห` saves like `Ctrl + S` in terminals that report the Thai character (the kitty protocol, Windows). A key with a binding of its own keeps it, and Thai typing is never a shortcut.
* **Keys:** on the Thai layout the backtick key types `_`, so `Alt + _` does the same. Some setups use the backtick key (Grave accent) as the Windows language-switch hotkey; if ``Alt + ` `` never reaches duckx-code, change it under Settings → Keybindings → Convert keyboard.

---

## 4. Indentation

| Keys | No selection, or selection within one line | Selection across multiple lines |
| :--- | :--- | :--- |
| `Tab` | Inserts spaces up to the next tab stop (or replaces the selection) | Indents every selected line, skipping empty lines |
| `Shift + Tab` | Outdents the line or the selection. Inside a snippet, moves to the previous placeholder. | Outdents every selected line. |

* **Indent style is detected per file:** if more lines start with a tab than with spaces, `Tab` inserts a tab character; otherwise it uses 4 spaces.
* **Partial-line rule:** a selection that ends at the very start of a line does not include that line.
* **Undo:** a multi-line indent or outdent is undone in one step.

---

## 5. Undo & Redo

| Keys | Action |
| :--- | :--- |
| `Ctrl + Z` | Undo |
| `Ctrl + Y` or `Ctrl + Shift + Z` | Redo |

* **Typing is grouped:** consecutive typing is undone as one step. Moving the cursor starts a new step.
* **Selection is restored:** undo puts the selection back the way it was before the edit.
* **Redo history is cleared by new edits:** making a new edit after undoing discards the redo history.

`Ctrl + Shift + Z` only works in terminals that report the `Shift` modifier with `Ctrl`; `Ctrl + Y` works everywhere.

---

## Search

`find` opens **FIND** mode. The status badge reads `FIND`, and the text you type replaces the file name, the same way the Command Bar does. Each character jumps to the first match of the text so far. The search is literal and case-sensitive. A row above the status line shows which match this is (`1 of 3`, or `Not found`) and `↑↓  esc`.

| Keys | Action |
| :--- | :--- |
| `Ctrl + F` or `find` | Enter FIND mode. The field starts with this tab's last search. `find hello` starts with that text instead and jumps immediately. |
| Type | Append to the search and jump to its first match. A space is part of the search. |
| `Backspace` | Delete the last character and jump again |
| `Down` or `F3` | Next match. After the last match, wraps to the first. |
| `Up` or `Shift + F3` | Previous match. Before the first match, wraps to the last. |
| `Esc` or `Enter` | Leave FIND mode in the editor. The match stays selected. |
| `Ctrl + H` or `replace` | Enter REPLACE mode |

`Ctrl + F` also works while the sidebar is focused, and then focuses the editor.

The match is selected and the cursor in the file sits on its first character. Each tab keeps the text from the last search and puts it back in the field the next time FIND opens. After you leave FIND mode, `F3` and `Shift + F3` still move through that text. With no search yet, a notice says `Nothing to find`.

From the explorer, `Ctrl + F` or `find` searches files instead of the open buffer. The field starts with the last file search. `Enter` lists every file that contains the text, with the match count on that row, and the keys go back to moving through the tree. The status badge then reads `FILTER`. `Space` marks a file and `Shift` plus the movement keys covers a range, the same way FILE mode does. `Enter` on a file opens it, enters FIND, and selects the match. `Ctrl + B` from that FIND returns to the file list. `Esc` leaves FILTER, clears those marks, and restores the tree. `find hello` from the explorer starts the search immediately. The search is the same literal, case-sensitive match, and a file that is already open is searched from its buffer.

`replace` while the badge reads `FILTER` opens REPLACE for that search. The field is `glowworm → `, and `Enter` writes the new word. Marked files are the only ones changed. A marked folder covers the matching files inside it. With no marks, every matching file is changed. An open file is edited in its buffer. A closed file is written on disk. The list then refreshes.

`replace` opens **REPLACE** mode. The badge reads `REPLACE`. The field always shows the selected text, then an arrow, then the new word: `cat → kitten`. The cursor starts just after the arrow, so typing edits the new word. The selected text is dimmed and cut off with `...` when it is wider than 24 columns, so a whole line does not fill the bar.

| Keys | Action |
| :--- | :--- |
| Type | Edit the replacement. It is not written into the file yet. |
| `Backspace` | Delete the last character of the replacement |
| `Down` | Replace this match, then select the next one. An empty replacement deletes the match. |
| `Shift + Down` | Replace every match in the file, as one undo step |
| `Up` | Undo the last replacement made in this mode |
| `Esc` or `Enter` | Leave REPLACE mode |

The text to replace is the selection, or the last search when nothing is selected. Each tab also keeps the last replacement and puts it back after the arrow the next time REPLACE opens. `replace kitten` starts with that new word instead. `Down` with no selection and no search says `Nothing to replace`. `Up` with nothing replaced yet says `Nothing to undo`.

---

## Opening a file

`file-picker` or `Ctrl + P` opens a rounded list over the editor. The status badge reads `OPEN`. Printable keys edit the query and do not change the file.

The list is the workspace, with `.gitignore` respected. Dot files and folders are skipped unless **Show hidden files** is On (Settings → Files, or the `hidden` command); `.git` is never listed. The scan runs in the background. Opening the picker again shows the previous list immediately, then replaces it when the new scan finishes. A `+` after the count means the walk stopped at 100,000 files.

| Keys | Action |
| :--- | :--- |
| Type | Narrow the list. Several words must all match. `!word` hides paths that contain `word`. Lower case ignores case; a capital letter matches case. `\` matches `/`. |
| `Up` / `Down` | Move the highlight |
| `PgUp` / `PgDn` | Move by one page of the list |
| `Left` / `Right` | Move the cursor in the query |
| `Home` / `End` | Start or end of the query |
| `Ctrl + Left` / `Ctrl + Right` | Previous or next word in the query |
| `Ctrl + Backspace` or `Ctrl + W` | Delete back to the previous word |
| `Ctrl + Home` / `Ctrl + End` | First or last file |
| `Enter` | Open the highlighted file in the focused window |
| `Esc` | Close the list |
| `Shift + Space` or `F1` | Close the list and open the Command Bar |

With an empty query, files opened earlier in this session come first, then everything else by file name. A typed query prefers the file name over a directory, and prefers the folder of the file you are editing when the scores are otherwise equal. Matched characters are highlighted. The counter on the query row is `highlight/matches`.

## Opening a folder

`workspace` opens a different folder without quitting. It is listed from the editor and from the explorer.

`Enter` on `workspace` with no path shows the folders saved in `session.json`, newest first, except the folder you are in. `Up` / `Down` move through that list and `Enter` opens the highlighted folder. A typed path is used as it is: a full path, `~`, or a path relative to the current folder. `Right` accepts the dimmed remainder of a saved folder. The folder must exist. The current folder says `Already here`. A path that is not a folder says `Not a folder`.

Each file with unsaved edits asks `Yes` / `No` / `Cancel` before the switch. `Yes` saves that file, `No` leaves it unsaved on disk, and `Cancel` stays in the current folder. The folder you leave is written to `session.json` first. The folder you open restores its tabs, cursors, splits, and sidebar from that file. A folder with no saved session opens one empty tab. Files from the folder you left do not stay open, and a load that was already running for that folder cannot put them back. The file tree, git status, and branch refresh in the background. Language servers for the previous folder stop, and files in the new folder start their own. The assistant transcript is cleared. Move, delete, rename, the file picker, and a relative `save` all use the folder you opened.

---

## Saving

| Keys / Command | Action |
| :--- | :--- |
| `Ctrl + S` or `save` | Save the focused editor |
| `save <path>` | Save the focused editor to that path and remember it. A relative path is inside the open folder. |

* **Atomic write:** the new text is written to a temporary file in the same directory, flushed, then renamed over the real file. A failed save leaves the previous file as it was.
* **Encoding:** the status bar shows the encoding after the line ending (`4 LF UTF-8`). A file opens as UTF-8, or as UTF-8 or UTF-16 when a BOM is present. `encoding utf-16le` makes the next save write that encoding. `reload` reads the file again with the encoding it already has. `reload windows-874` reads those bytes as that encoding. A character the encoding cannot store is not written, and a notice says so. Names: `utf-8`, `utf-8-bom`, `utf-16le`, `utf-16be`, `windows-1252`, `windows-874`.
* **Changed on disk:** about once a second, a clean file whose copy on disk changed is reloaded and the cursor stays on the same line. A file with unsaved edits asks first. `Reload` discards those edits. `Keep` and `Esc` leave the editor text; the next save writes over the disk copy. A save from duckx-code itself is not treated as an outside change.
* **Modified marker:** `[+]` appears after the file name on that tab and in the status bar while the text differs from the last successful load or save. Undoing or deleting back to that text clears it.
* **No path yet:** `Ctrl + S` opens the Command Bar with `save ` so you can type a path. A name with no folder, such as `hello.rs`, is saved in the directory duckx-code was started from. The status bar then shows that folder, dimmed, in front of the file name, and `Saved` includes the full path. A missing directory is created.
* **Sidebar:** saving while the sidebar is open reloads the tree, so the new file shows up there.
* **Quit:** `Ctrl + Q` or `quit` asks once per tab that still has unsaved changes. The dialog is the shared confirmation box: `Left` / `Right` move between `Yes`, `No`, and `Cancel`, and `Enter` confirms the highlighted one. `Y` and `N` are shortcuts for `Yes` and `No`. `Esc` cancels and is not shown. `Yes` saves (and asks for a path when there is none), `No` quits without saving that tab, `Cancel` stays in the editor.
* **Close:** `close` asks the same way before discarding a dirty tab. Other tabs in that window stay open. Closing the only tab of the only window leaves a blank `[No Name]` page. `close-all` and `close-view` ask once per dirty tab they would close. `Yes` saves, `No` discards that tab and continues, and `Cancel` leaves the remaining tabs open.

---

## Language servers & diagnostics

Opening a file in a supported language starts its language server in the background. Nothing waits for it: typing, scrolling, and saving work the same while the server starts, indexes, or is missing. Every open file of that language in the same project shares one server.

| Language | Server program (first one switched On and installed wins) | Project root marker |
| :--- | :--- | :--- |
| Rust | `rust-analyzer` | `Cargo.toml` |
| Python | `pyright-langserver --stdio`, `basedpyright-langserver --stdio`, `pylsp`, `ruff server` | `pyproject.toml`, `setup.py`, `setup.cfg`, `requirements.txt`, `pyrightconfig.json` (Ruff: `pyproject.toml`, `ruff.toml`, `.ruff.toml`) |
| JavaScript / TypeScript / JSX / TSX | `typescript-language-server --stdio` | `tsconfig.json`, `jsconfig.json`, `package.json` |
| Go | `gopls` | `go.work`, `go.mod` |
| C / C++ | `clangd` | `compile_commands.json`, `compile_flags.txt`, `.clangd`, `CMakeLists.txt` |
| Zig (`.zig`, `.zon`) | `zls` | `build.zig` |
| Lua | `lua-language-server` | `.luarc.json`, `.luarc.jsonc` |
| Ruby (`.rb`, `.rake`, `.gemspec`, `Gemfile`, `Rakefile`) | `ruby-lsp` | `Gemfile` |
| PHP | `intelephense --stdio` | `composer.json` |
| YAML | `yaml-language-server --stdio` | `.git` |
| Shell (`.sh`, `.bash`, `.zsh`) | `bash-language-server start` | `.git` |
| JSON / JSONC | `vscode-json-language-server --stdio` | `.git` |
| CSS / SCSS / Less | `vscode-css-language-server --stdio` | `package.json`, `.git` |
| HTML | `vscode-html-language-server --stdio` | `package.json`, `.git` |
| Svelte | `svelteserver --stdio` | `svelte.config.js`, `package.json` |
| Vue | `vue-language-server --stdio` | `vite.config.ts`, `vite.config.js`, `package.json` |
| Swift | `sourcekit-lsp` | `Package.swift` |

Files are matched to a server by their extension (or, for `Gemfile` / `Rakefile`, their name), so a server needs no syntax colors to work. The program must be on `PATH`. On Windows, `PATHEXT` is used, so npm's `.cmd` launchers (such as `typescript-language-server.cmd`) and `.bat` files are found too. The program is started directly from the folder it was found in, never through a shell (a `.cmd` or `.bat` launcher is run by Windows' own `cmd.exe`, the only way Windows runs one). `rustup component add rust-analyzer` installs the Rust server; Settings → [Language servers](#language-servers) shows the install command of every server that is missing.

* **Project root:** for a file inside the folder duckx-code was started from, the highest folder with a marker, up to that folder, or the folder itself when none has one. A Cargo or npm workspace therefore gets one server, not one per member. For a file outside it, the highest marked folder above the file, or the file's own folder.
* **Python checking:** pyright and basedpyright are given `typeCheckingMode: basic` for open files. That reports an undefined name, such as a misspelled call. Left unset, those servers report syntax errors only. A `pyrightconfig.json` or `pyproject.toml` in the project still configures them. `pylsp` uses its own defaults.
* **Diagnostics:** errors, warnings, infos, and hints from the server. The gutter shows `E`, `W`, `I`, or `H` in that color before the line number on the line where a diagnostic starts; the most severe one wins. The text it covers is drawn bold in that same color, the color of the diagnostic bar. Ranges move with your edits until the server sends fresh results. Editing the text a diagnostic covers, including typing at either edge of that token, clears that marker immediately. A result computed for an older version of the file is moved the same way, and dropped when that edit rewrote its range. rust-analyzer's compiler check runs when the file is saved. A type error you have not touched stays while you keep typing. Syntax errors follow the unsaved buffer: after a short pause the editor asks again, and a quote you already closed disappears without saving. That live result replaces the compiler's syntax error from the last save.
* **Cursor line:** the most severe diagnostic under the cursor (or else on the cursor line) is a full-width bar just above the status bar. The bar uses that level's color, the message is on the left, and the number of diagnostics in the file is on the right. The status bar itself keeps only the per-level counts.
* **Completion:** the menu shows the symbol kind, the detail, and one line of documentation when the server sends it. Accepting a snippet selects the first placeholder. `Tab` and `Shift + Tab` move through the placeholders, and `Esc` leaves the snippet. `(` and `,` ask for the function signature and show it above the cursor.
* **Semantic tokens:** when the server offers them, those colors paint over the tree-sitter colors, so a built-in type, a macro, or a lifetime can differ from a name the grammar could not tell apart. A syntax theme may set `documentation`, `macro`, `builtin`, `escape`, and `lifetime`. Missing keys fall back to comment, function, type, constant, and attribute.
* **Restart:** `lsp-restart` stops the server of the focused file, looks for its program on `PATH` again, and starts it, reopening every file that used it. It also retries a server that was missing or gave up after crashing. `r` on a row of Settings → [Language servers](#language-servers) does the same for that server.
* **Missing server:** a notice says `No language server found. Install … for code diagnostics` once per session, and the file stays plain.
* **Crash:** a server that stops after it was ready is restarted up to 3 times, waiting 1, 2, then 4 seconds. A server that stops before it is ready, or after the third restart, stays stopped until duckx-code restarts. The status bar shows the last line it printed.
* **Large files:** files over 8 MB are not sent to a server.
* **Logs:** each server's stderr is written to `%APPDATA%\duckx-code\logs\lsp-<name>.log` (elsewhere, `logs/` under the config directory from [Settings](#10-settings)). The file is replaced each time the server starts.
* **Quit:** servers are asked to shut down and are stopped after 2 seconds at most.

| Keys / Command | Action |
| :--- | :--- |
| `F8` or `diagnostic-next` | Move the cursor to the start of the next diagnostic in this file. After the last one, wraps to the first. |
| `Shift + F8` or `diagnostic-prev` | Move to the previous diagnostic, wrapping to the last |
| `F2` or `hover` | Show documentation for the symbol under the cursor. The popup is plain text. Moving the cursor or editing the file closes it. |
| `Shift + F2` or `rename-symbol <name>` | Rename that symbol everywhere the language server knows it. The command bar asks for the new name. |
| `F4`, `Ctrl + /`, or `comment` | Add a line comment to the current line or the selected lines, or remove one when those lines are already commented. |
| `Alt + Up` / `Alt + Down` or `move-line-up` / `move-line-down` | Move the current line, or the selected lines, up or down. |
| `Shift + Delete` or `delete-line` | Delete the current line, or every line the selection touches. |
| `Ctrl + Alt + Up` / `Ctrl + Alt + Down` or `duplicate-line-up` / `duplicate-line-down` | Duplicate the current line, or the selected lines, above or below. |
| `F7` or `format` | Ask the language server to format the file. |
| `lsp-restart` | Restart the focused file's language server, looking for its program again. |
| `Ctrl + .` or `code-action` | Show fixes and refactors at the cursor. `Up` / `Down` move, `Enter` applies, `Esc` closes. |
| `F12` or `goto-definition` | Jump to where that symbol is defined. When the server returns several places, the first one is opened and a notice says how many there were. |
| `Shift + F12` or `goto-references` | List every place that symbol is used, including its declaration when the server sends it. One place jumps there. Several places open a list: `Up` / `Down` move, `Enter` or `Tab` jumps, `Esc` closes. |
| `Ctrl + ]` or `match-bracket` | Jump to the bracket paired with the one under the cursor, or the one just before it. `Ctrl + Shift + ]` selects through that bracket. |
| `Ctrl + O` or `jump-back` | Return to the place from before the last definition or references jump |
| `Ctrl + Shift + O` or `jump-forward` | Go forward again after jump-back |
| `Ctrl + Space` or `complete` | Open completions at the cursor. Typing a word or a character the server listed (such as `.`) opens the same list on its own. A snippet selects its first placeholder. `Tab` moves to the next placeholder and `Shift + Tab` to the previous one. `Esc` leaves the snippet. |

With no diagnostics in the file, both diagnostic keys say `No diagnostics in this file`. With no references, `Shift + F12` says `No references`. A reference row shows the file and line, and the text of that line when the file is already open. `Ctrl + O` returns from that jump the same way it returns from a definition.

**Bracket highlights.** When the cursor is on a bracket, or just after one, that bracket and its pair are drawn bold on a soft background. Brackets in strings and comments are skipped, and a bracket in a string pairs only with one in a string. Nothing is shown when the pair is off screen or more than 50 000 characters away, and only the focused window shows it. A bracket that has no pair is drawn in the error color. Brackets are also colored by nesting depth; a bracket with no partner uses the error color. `<` and `>` are never treated as brackets here. The selection draws over both, and a diagnostic's color wins over a depth color. Both are rows under Settings → Editor (on by default) and the commands `bracket-match` and `bracket-colors`.

`Ctrl + ]` pairs `()` `[]` and `{}`. With a syntax tree, `<>` also jumps when that tree treats the two ends as one pair, such as a generic argument, and a bracket in a string or comment stays put. A file with no tree yet pairs `()` `[]` `{}` by scanning, so a bracket in a string can match. Nothing happens when there is no pair, and this jump is not part of the definition jump list. `match-bracket` does the same jump when a terminal does not report `Ctrl + ]`.

The completion list filters as you keep typing. `Up` and `Down` move the highlight, `Tab` or `Enter` inserts the highlighted item as one undo step, and `Esc` closes the list. Other keys still type into the file. A file that has not been saved yet says so instead of asking a server.

---

## 6. Command Bar

### 6.1 Opening & Using the Bar

| Keys | Action |
| :--- | :--- |
| `Shift + Space` | Open the Command Bar |
| `F1` / `Ctrl + ;` / `Ctrl + Shift + :` | Open the Command Bar (fallbacks) |
| Type | Filter commands in real time |
| `Up` / `Down` (or `Shift + Tab` for up) | Move the highlight |
| `Tab` | Leave the bar and open the assistant. `Tab` in the assistant opens the bar again |
| `Right` | Accept the dimmed file or folder name after the cursor |
| `Enter` | Run the command |
| `Esc` / `F1` | Close without running anything |
| `Backspace` | Delete the last input character |

* **Where it sits:** the bar is the status line. The badge reads `COMMAND` and you type after `❯`. The file name is hidden until the bar closes.
* **Hints:** matching commands sit in a rounded `Commands` list that floats over the bottom of the editor, just above the status line. The editor and previews keep their full height under it. The list is one command per row, in list order. `Up` moves the highlight to the row above and `Down` to the row below. The highlighted row is a full-width bar marked `›`. `↑↓  tab  enter  esc` sits in the list's bottom border, and `n/len` in the top border when there are more matches than rows. The list disappears when the bar closes.
* **Focus context:** the list is only the commands for the surface that was focused when the bar opened. From an editor, file commands (`select`, `move`, `delete`, `rename`) are absent. From the explorer, editing commands (`goto`, `save`, `undo`, …) are absent, and `move`, `delete`, and `rename` appear only in FILE mode. `file-picker`, `workspace`, `reveal`, `explorer`, `setting`, `about`, and `quit` are in both. Typing a command from the other surface reports where it applies and does not run it. Typing `move`, `delete`, or `rename` outside FILE mode reports that it applies in FILE mode.
* **Terminals that hide `Shift + Space`:** some terminals, including some integrated editor terminals, send `Shift + Space` as a plain space. If the bar doesn't open, use `F1`.
* **Space inside the bar:** pressing `Shift + Space` while the bar is open types a normal space.

### 6.2 How Matching Works

1. **The list is the focused surface.** Editor commands and explorer file commands are never mixed. A bare number goes to that line only while an editor is focused: typing `42` shows `goto 42` as the only hint, and `Enter` jumps there. `Tab` leaves the bar and opens the assistant. In the explorer, a number matches nothing.
2. **Name prefix matches come next:** `un` → `undo` in the editor, `del` → `delete-line` in the editor and `delete` in the explorer while FILE mode is on.
3. **Then word matches:** commands where a word in the name or description starts with the input, so `line` in the editor → `goto`, `copy`, `cut`, `comment`, `indent`, `outdent`, `move-line-up`, `move-line-down`, `delete-line`, `duplicate-line-up`, `duplicate-line-down`.
4. **Each of those groups follows importance.** An empty list in the editor starts `file-picker`, `workspace`, `find`, `replace`, `goto`, `save`, `undo`, `redo`, then clipboard, language tools, layout, `reveal`, `explorer`, `setting`, `agent`, `about`, and `quit`. The explorer uses the same rank: `file-picker`, `workspace`, `find`, `replace`, `select`, then `move`, `delete`, and `rename` in FILE mode, then `reveal`, `explorer`, `setting`, `about`, and `quit`.
5. **The most recent command on this surface is always first** when it is one of the matches, and marked with ★. That holds for an empty input and for a typed filter, including when the recent command only matches a later word. Opening the bar and pressing `Enter` immediately repeats it. A command last run in the editor is not pinned while the explorer is focused, and the reverse.

### 6.3 Running Commands

* **Arguments:** put them after the command name, separated by a space, e.g. `goto 42`.
* **File and folder names:** `move`, `rename`, `save`, and `workspace` draw the rest of the closest matching name in dim text after what you typed. The block cursor sits on the first character of that hint, and `Right` accepts the whole hint. The keys in the list's border show `→` while the hint is visible. The closest name is the shortest match, then A–Z, ignoring case and treating `\` like `/`. A name you have already finished is left alone, so `move src` does not grow into `move src/ui`. `move`, `rename`, and `save` take names from the workspace tree the explorer has scanned. `move` hints folders. `rename` hints names in the same folder as the focused row. `save` hints files and folders. `workspace` hints folders saved in `session.json`. `Right` does nothing when there is no hint.
* **Exact names win:** a fully typed command name runs even if another suggestion is highlighted.
* **Missing arguments:** pressing `Enter` on a command that needs an argument fills in `goto ` and waits for you to type it. `find` and `replace` take no argument: `Enter` opens FIND or REPLACE mode. `replace kitten` opens REPLACE mode with that new word already typed.
* **Errors:** messages such as `Unknown command` or `Invalid line number` appear in yellow beside the input while the bar is open; after the bar closes, they are an `Error` [notice](#notices).

### 6.4 Command List

#### Navigation
| Command | Argument | Description | Shortcut |
| :--- | :--- | :--- | :--- |
| `goto` | `<line>` | Go to a line number (1-based; numbers past the end go to the last line). Typing just the number works too. | — |
| `diagnostic-next` | — | Jump to the next error or warning in this file. See [Language servers & diagnostics](#language-servers--diagnostics). | `F8` |
| `diagnostic-prev` | — | Jump to the previous error or warning in this file | `Shift + F8` |
| `hover` | — | Show documentation for the symbol under the cursor | `F2` |
| `goto-definition` | — | Jump to where the symbol under the cursor is defined | `F12` |
| `goto-references` | — | Show every place this symbol is used | `Shift + F12` |
| `match-bracket` | — | Jump to the matching bracket | `Ctrl + ]` |
| `complete` | — | Open completions at the cursor | `Ctrl + Space` |
| `comment` | — | Add or remove a line comment | `F4` |
| `convert-keyboard` | — | Retype the selection, or the word before the cursor, as the other keyboard layout (US QWERTY ↔ Thai Kedmanee). See [Wrong keyboard layout](#wrong-keyboard-layout). | ``Alt + ` `` |
| `move-line-up` | — | Move the current line or selection up | `Alt + Up` |
| `move-line-down` | — | Move the current line or selection down | `Alt + Down` |
| `delete-line` | — | Delete the current line, or every line the selection touches. See [Editing](#3-editing). | `Shift + Delete` |
| `duplicate-line-up` | — | Duplicate the current line or the selected lines above | `Ctrl + Alt + Up` |
| `duplicate-line-down` | — | Duplicate the current line or the selected lines below | `Ctrl + Alt + Down` |
| `rename-symbol` | `<name>` | Rename the symbol under the cursor | `Shift + F2` |
| `format` | — | Format the current file | `F7` |
| `lsp-restart` | — | Restart this file's language server and look for it on `PATH` again. See [Language servers & diagnostics](#language-servers--diagnostics). | — |
| `code-action` | — | Show fixes and refactors at the cursor | `Ctrl + .` |
| `jump-back` | — | Return to the place from before the last definition or references jump | `Ctrl + O` |
| `jump-forward` | — | Go forward again after jump-back | `Ctrl + Shift + O` |

#### Search
See [Search](#search) for wrap, replace, and empty-replacement behavior.

| Command | Argument | Description | Shortcut |
| :--- | :--- | :--- | :--- |
| `find` | — | Enter FIND mode. In the explorer, `Enter` searches files. See [Search](#search). | `Ctrl + F` |
| `file-picker` | — | Open a file from the workspace. A rounded list appears over the editor. Also listed while the explorer is focused. See [Opening a file](#opening-a-file). | `Ctrl + P` |
| `workspace` | `<folder>` | Open another folder. See [Opening a folder](#opening-a-folder). Also listed while the explorer is focused. | — |
| `replace` | optional text | Enter REPLACE mode. In FILTER mode, `Enter` replaces the marked files, or every match when nothing is marked. See [Search](#search). | `Ctrl + H` |

#### Clipboard
| Command | Argument | Description | Shortcut |
| :--- | :--- | :--- | :--- |
| `copy` | — | Copy the selection to the OS clipboard. With no selection, copies the whole current line including its line ending. Shows `Copied N characters`. | `Ctrl + C` |
| `cut` | — | Cut that same text to the OS clipboard. Shows `Cut N characters`. | `Ctrl + X` |
| `paste` | — | Insert OS clipboard text at the cursor, replacing any selection. Line endings are converted to the file's style. | `Ctrl + V` |

If no OS clipboard is available (e.g. a headless session), `copy`, `cut`, and `paste` fall back to an internal clipboard that only works within the editor.

#### File
| Command | Argument | Description | Shortcut |
| :--- | :--- | :--- | :--- |
| `save` | optional path | Save the focused editor. With no path yet, the bar stays on `save ` until you type one. Shows `Saved` and the full path. | `Ctrl + S` |
| `reload` | optional encoding | Read the focused file from disk again. With an encoding, those bytes are decoded that way. A dirty file asks before the editor text is replaced. | — |
| `encoding` | optional name | Show the encoding, or set the one the next save writes. | — |

#### Editing
| Command | Argument | Description | Shortcut |
| :--- | :--- | :--- | :--- |
| `undo` | — | Undo the last edit | `Ctrl + Z` |
| `redo` | — | Redo the last undone edit | `Ctrl + Y` |
| `indent` | — | Indent the current line or selection | `Tab` |
| `outdent` | — | Outdent the current line or selection | `Shift + Tab` |

#### Selection
| Command | Argument | Description | Shortcut |
| :--- | :--- | :--- | :--- |
| `select-all` | — | Select the entire document | `Ctrl + A` |

#### View
| Command | Argument | Description | Shortcut |
| :--- | :--- | :--- | :--- |
| `explorer` | — | Toggle the file explorer sidebar (same cycle as `Ctrl + B`) | `Ctrl + B` |
| `reveal` | — | Show the focused file in the explorer. See [Explorer Sidebar](#8-explorer-sidebar). | — |
| `diff` | — | Show or hide changes against HEAD in the gutter. See [Editor Windows](#7-editor-windows). | — |
| `bracket-match` | — | Turn bracket matching on or off. Flips and saves **Match brackets** (Settings → Editor). | — |
| `bracket-colors` | — | Turn bracket pair colors on or off. Flips and saves **Colorize bracket pairs** (Settings → Editor). | — |
| `hidden` | — | Show or hide dot files and folders (`.claude`, `.env`) in the explorer, the file picker, and searches. Flips and saves **Show hidden files** (Settings → Files). `.git` stays hidden. | — |
| `focus` | — | Show only the buffer in use. The title, status bar, diagnostic line, sidebar, and other windows hide. The command bar, and find or replace, appear only while they are open. Only `focus` again brings the rest back. See [Editor Windows](#7-editor-windows). | — |
| `split-right` | — | Open an empty editor to the right of the focused one | — |
| `split-down` | — | Open an empty editor below the focused one | — |
| `close` | — | Close the focused tab. The last tab of a split window closes that window. The last tab of the last window becomes a blank page. | — |
| `close-all` | — | Close every tab in every window. One blank page remains. | — |
| `close-view` | — | Close every tab in the focused window. `clean-view` is another name for this command. A split window closes; the last window becomes a blank page. | — |
| `tab-prev` | — | Select the previous tab in the focused window | `Ctrl + Alt + Left` |
| `tab-next` | — | Select the next tab in the focused window | `Ctrl + Alt + Right` |

#### Application
| Command | Argument | Description | Shortcut |
| :--- | :--- | :--- | :--- |
| `setting` | — | Open the settings page. `config` is another name for the same command. | — |
| `agent` | — | Open the assistant panel. It is drawn only while the prompt is focused. `F6` while the prompt is focused closes it. From the command bar, `Tab` opens it, and `Tab` in the prompt (when no `/` or `@` popup is open) returns to the command bar and hides the panel. New chat is `/new` or `Ctrl+L` in the panel — bare `agent new` is no longer a command-bar argument. See [Assistant](#assistant). | `F6` |
| `clear` | — | Start a new assistant chat (clears transcript and tool memory). Prefer `/new` or `/clear` in the assistant prompt. Alias `agent-new`. | `Ctrl+L` (in panel) |
| `about` | — | Open the about page. It shows the duck logo, the name, and the version. `Esc` closes it. | — |
| `quit` | — | Exit duckx-code. Asks before leaving unsaved changes | `Ctrl + Q` |

#### Explorer
These are listed only while the explorer sidebar is focused. `explorer`, `setting`, `about`, and `quit` are also listed here.

| Command | Argument | Description | Shortcut |
| :--- | :--- | :--- | :--- |
| `select` | — | Enter FILE mode. The status badge reads `FILE`. `Space` marks or unmarks the focused file. Hold `Shift` and move to cover a range, then `Space` without `Shift` marks that range (or unmarks it when every row is already marked). `Shift + Space` opens the Command Bar. `Esc` returns to NORMAL and clears every mark. A green `●` is a mark; a green `○` is only the cover. | — |
| `move` | `<dir>` | FILE mode only. Move the marked files, or the focused file when nothing is marked, into a folder. The folder is inside the workspace (`src`, or the workspace root with `.`). Shows `Moved`. | — |
| `delete` | — | FILE mode only. Delete those files. Asks with the shared `Yes` / `No` / `Cancel` dialog (`Left` / `Right` and `Enter`; `Y` and `N` are shortcuts). A folder deletes everything inside it. Refuses when an open editor still has unsaved changes for that path. An open editor keeps its text and is marked unsaved. | — |
| `rename` | `<name>` | FILE mode only. Rename the focused file or folder. The name stays in the same folder and cannot contain `\` or `/`. An open editor follows the new path. | — |
| `wider` | — | Make the sidebar four columns wider, up to half the screen | `Alt + Right` |
| `narrower` | — | Make the sidebar four columns narrower, down to 16 columns | `Alt + Left` |

---

## 7. Editor Windows

Each window has its own tabs. A new window starts with one empty `[No Name]` tab. `split-right`, `split-down`, `close`, `close-all`, and `close-view` have no shortcut yet.

| Command | Action |
| :--- | :--- |
| `split-right` | Split the focused window in half, left and right, and focus the empty window on the right |
| `split-down` | Split the focused window in half, top and bottom, and focus the empty window below |
| `close` | Close the focused tab. If it is the only tab, close the window and focus the window that takes its place |
| `close-all` | Close every tab in every window, then leave one blank `[No Name]` page |
| `close-view` | Close every tab in the focused window. `clean-view` is the same command. Other windows stay as they are |
| `tab-prev` | Select the previous tab in the focused window (`Ctrl + Alt + Left`) |
| `tab-next` | Select the next tab in the focused window (`Ctrl + Alt + Right`) |

* **The split is only the focused window.** A window on the left stays full height when the right side is split top and bottom.
* **Closing asks when that tab has unsaved changes.** The same `Yes` / `No` / `Cancel` dialog is used. `Yes` saves, `No` discards, `Cancel` keeps the tab. Closing one tab of several leaves the window open and selects the next tab. Closing the last tab of a split window closes that window. Closing the last tab of the last window leaves a blank `[No Name]` page.
* **`close-all` closes every tab.** Each dirty tab is asked in turn, starting with the focused one. `Yes` saves that tab (and asks for a path when it has none), `No` discards it, and `Cancel` stops so the tabs not yet closed stay open. When the questions are done, every window collapses to one blank `[No Name]` page.
* **`close-view` closes the focused window.** `clean-view` runs the same command. Dirty tabs in that window are asked the same way; tabs in other windows are left alone, including unsaved ones. The last tab of a split window closes that window. The last window becomes a blank `[No Name]` page.
* **Moving between windows** uses PANE mode. `Ctrl + W` starts it on the focused window, and `Left / Right / Up / Down` move the yellow PANE border to the nearest open window on that side; it stays put when there is none. `Enter`, or `Ctrl + W` again, focuses the highlighted window, and `Esc` cancels. It does not open a closed sidebar. (`Ctrl + Left / Right` move by word and `Ctrl + Up / Down` jump between blank lines; they do not change windows.)
* **The focused window** has a yellow line number, a bold active tab, and the block cursor. Other windows stay where you left them, with a dim active tab. A dim `│` or `─` separates windows.
* **`focus`** fills the screen with the buffer in use. The tab row, the status bar, the diagnostic line under the buffer, the sidebar, and every other window are hidden. The gutter stays. `Shift + Space` (or `F1`, or `Ctrl + ;`) still opens the command bar, and it is drawn only while it is open; find and replace use that same line while they are open. PANE mode (`Ctrl + W`, an arrow, `Enter`) moves to another editor window, which then fills the screen. `Ctrl + B`, `reveal`, and a move onto the sidebar do nothing while this view is on. Only `focus` again restores the title, status bar, sidebar, and the other windows.
* **`diff`** compares the buffer, including unsaved edits, with `HEAD`. It toggles. While it is on, the status line says `diff`, and a column appears to the left of the diagnostic mark. `┃` is an added line (the added git color) or a modified line (the modified git color). `‾` means one line was removed just above this row. A digit `2`–`9`, or `+` past that, is how many lines were removed. `~` means this row also changed. A removal after the last line marks the first `~` row. The comparison runs in the background and refreshes after an edit. `Diff on` means marks are showing. `No changes`, `Not a git repository`, `git was not found`, and `Binary file` explain an empty gutter. A tab with no path says `No file to compare`. `diff` again hides the column and says `Diff hidden`.
* **Each window has its own tabs** on one row at the top: a file-type icon when the name is recognized, then the file name, or `[No Name]`, plus `[+]` when that tab has unsaved edits. The active tab is bold while the window is focused. Tabs that do not fit show `‹` and `›`. There is no background or border. `Ctrl + Alt + Left` / `Right` cycle them. A file opened from the explorer becomes a tab in the focused window. A file that is already open is focused instead of opened twice. A clean untitled tab is reused; anything else stays put and the new file opens beside it.
* **The status bar** shows the path and cursor of the focused window.

---

## 8. Explorer Sidebar

A file tree of the directory duckx-code was started from, on the left side of the screen.

### 8.1 Opening, Focusing & Closing

| Keys | When | Action |
| :--- | :--- | :--- |
| `Ctrl + B` | Sidebar closed | Open the sidebar and focus it |
| `Ctrl + B` | Sidebar open, editor focused | Move focus to the sidebar |
| `Ctrl + B` | Sidebar focused | Close the sidebar |
| `Ctrl + W`, `Left`, `Enter` | Sidebar is open and this editor is on its right edge | Focus the sidebar (PANE mode). Does nothing while the sidebar is closed |
| `Ctrl + W`, `Right`, `Enter` | Sidebar focused | Focus the editor window directly to the right (the sidebar stays open) |
| `Alt + Right` | Sidebar focused | Make the sidebar four columns wider |
| `Alt + Left` | Sidebar focused | Make the sidebar four columns narrower |
| `Esc` | FILE mode | Return to NORMAL and clear every file mark. The sidebar stays focused. A filter stays until the next `Esc` |
| `Esc` | Sidebar focused, not in FILE mode | Clear the filter; if it is already empty, return focus to the editor (the sidebar stays open) |

* **Moving between surfaces:** editor windows and an open sidebar share PANE mode (`Ctrl + W`, then arrows and `Enter`). See [Editor Windows](#7-editor-windows). `Left` from the sidebar stays on the sidebar, and PANE mode does not open a closed sidebar. Leaving the sidebar this way does not close it; `Ctrl + B` still does.
* **Width:** the sidebar opens at 30 columns. `Alt + Right` adds four columns and `Alt + Left` removes four. It stops at 16 columns and at half the screen, and a notice says `Sidebar is at its widest` or `Sidebar is at its narrowest`. The editor keeps the other half. `wider` and `narrower` do the same thing. The width lasts until duckx-code quits. In the editor, `Alt + Left` / `Alt + Right` still move by word.
* **Terminals that take `Ctrl + B`:** some integrated editor terminals and tmux (default prefix) intercept `Ctrl + B`. Use the `explorer` command from the Command Bar instead.
* `Shift + Space` / `F1` open the Command Bar and `Ctrl + Q` quits from the sidebar too, including in FILE mode. `Space` without `Shift` marks files only in that mode.

### 8.2 Moving in the Tree

| Keys | Action |
| :--- | :--- |
| `Up` / `Down` | Previous / next item |
| `PgUp` / `PgDn` | One screen up / down |
| `Home` / `End` | First / last item |
| `Right` | Expand a folder; on an expanded folder, move to its first child |
| `Left` | Collapse a folder; on a file or collapsed folder, move to the parent folder |
| `Shift` + `Up` / `Down` / `PgUp` / `PgDn` / `Home` / `End` / `Left` / `Right` | In FILE mode, cover rows from where `Shift` started. Release `Shift`, then `Space` marks them |
| `Enter` | Expand / collapse a folder, or open a file as a tab in the focused window |

`Enter` on a file opens it as a tab in the focused window and moves focus there. If that file is already open, that tab is focused. A clean untitled tab is reused. A tab that already has a path, or unsaved edits, stays open and the file loads in a new tab. A file that is not valid UTF-8 and has no UTF-16 BOM stays as an empty tab, and a notice says to run `reload` with an encoding, for example `reload windows-874`. Saving that tab is refused until the reload succeeds. An empty tab created for a different open error is removed again.

`reveal` selects the file of the focused tab in the tree. Folders above that file open. Other open folders stay open. A filter or a file search is cleared. In FILE mode the `Shift` cover clears and marks stay. The text cursor does not move, and focus stays where it was. A closed sidebar opens without taking focus, so typing still inserts in the editor. While focus mode is on, `reveal` does nothing. A tab with no path says `No file to reveal`. A file outside the tree, including one hidden by `.gitignore`, says `File is not in the explorer`. If the tree is still loading, the row moves when the scan finishes.

A file or folder created, deleted, or renamed on disk shows up while the sidebar is open. The rows stay in place until the new list is ready. A closed sidebar shows the change the next time it opens.

### 8.3 Filtering

Type any printable character to filter. The box has no title. A magnifying-glass icon sits at the left of the input. Every name containing the text (case-insensitive) is shown together with its parent folders, and the first match is selected. `Backspace` deletes the last filter character, and pasting adds to the filter. In FILE mode, `Space` marks files instead of typing a space into the filter.

The counter in the filter box shows `visible/visible` with no filter and `matches/total` while filtering.

In FILE mode, movement without `Shift` clears the cover. Marks last until you `Space` that file again, run `move` / `delete`, or leave FILE mode. Leaving the mode — `Esc`, closing the sidebar, or focusing an editor — clears every mark.

### 8.4 Changing Files

Open the Command Bar while the explorer is focused (`Shift + Space` works in FILE mode too). Outside FILE mode the hints are `file-picker`, `workspace`, `find`, `replace`, `select`, `reveal`, `explorer`, `setting`, `about`, and `quit`. Inside FILE mode they also include `move`, `delete`, and `rename`, after `select`. `replace` outside FILTER mode reports that it applies in FILTER mode.

| Command | Action |
| :--- | :--- |
| `find` | Search file contents. See [Search](#search) |
| `replace` | In FILTER mode, replace the search text in the marked files, or in every match |
| `select` | Enter FILE mode. `Space` marks the focused row or the `Shift` cover. `Esc` returns to NORMAL and clears every mark |
| `move <dir>` | Move marked rows, or the focused row, into that folder |
| `delete` | Ask, then delete marked rows, or the focused row |
| `rename <name>` | Rename the focused row |

`move` and `rename` update any editor that has the file open. `delete` asks with `Yes` / `No` / `Cancel` before it removes anything. It does not discard unsaved editor text: it refuses while that text is unsaved, and after a delete the editor is marked unsaved so you can save the text back. Paths outside the workspace are refused. The tree refreshes when the change finishes.

### 8.5 What Is Shown

* **Hidden entries:** files and folders matched by `.gitignore` (and `.ignore`) are hidden. Dot files and folders (`.claude`, `.github`, `.env`, `.gitignore`) are hidden too unless **Show hidden files** is On; then they appear in their usual sorted place with a dim name. `.git` is always hidden. Turning the setting on or off (Settings → Files, or `hidden` in the Command Bar) refreshes the open tree at once, and folders that were open stay open.
* **Order:** folders first, then files, alphabetically (case-insensitive).
* **Size limit:** very large trees are cut off after 20,000 entries.
* **Refresh:** the tree and git status are reloaded each time the sidebar is opened, after a file is saved while it is open, and after `move`, `rename`, or `delete`.

| Marker | Meaning |
| :--- | :--- |
| `M` | Modified |
| `A` | Added (staged) |
| `D` | Deleted |
| `R` | Renamed |
| `?` | Untracked |

The letter is fixed. The color comes from the active color theme. Tokyo Night uses the [VS Code git decoration colors](https://github.com/tokyo-night/tokyo-night-vscode-theme): modified `#6183bb`, added, renamed, and untracked `#449dab`, deleted `#914c54`. Catppuccin uses yellow, green, red, blue, and gray for those five markers.

A folder shows the strongest marker of anything inside it. Markers need `git` on your `PATH`; outside a repository none are shown.

---

## 9. Status Bar

The layout follows Neovim's lualine (Tokyo Night colors), with powerline separators between sections:

```text
 NORMAL  master  src/DESIGN.md                    12 selected   8%   8:209   4 LF
```

While the Command Bar is open the file name is hidden, and matching commands float over the editor, just above the status line:

```text
 ╭ Commands ─────────────────────────────────────────────────╮
 │› split-right      Open an empty editor to the right       │
 │  split-down       Open an empty editor below              │
 ╰─────────────────────────────────── ↑↓  tab  enter  esc ───╯
 COMMAND  ❯ spli                                     12 selected   8%   8:209   4 LF
```

| Section | Element | Meaning |
| :--- | :--- | :--- |
| Left | `NORMAL` / `FILE` / `COMMAND` / `FIND` / `REPLACE` / `OPEN` / `SETTINGS` / `ABOUT` | Where keystrokes go. `NORMAL` (blue) while editing the file or the sidebar. `FILE` (orange) while the explorer is in FILE mode, entered with `select`. `FILTER` (teal) while the explorer is showing a file search. `COMMAND` (purple) while the Command Bar is open. `FIND` (yellow) while searching; printable keys jump to that text instead of inserting. `REPLACE` (orange) while typing a replacement. `OPEN` (purple) while the file picker is open. `SETTINGS` (green) while the settings page is open. `ABOUT` (yellow) while the about page is open. Printable keys insert text while the badge reads `NORMAL`. |
| Left | Git branch | Shown while the bar is closed and the working directory is inside a repository. The branch icon and the branch name sit between the badge and the file. A detached HEAD shows the first 7 characters of the commit hash. Outside a repository this section is omitted. Hidden while the Command Bar, FIND mode, or REPLACE mode is open. |
| Left | Icon + path | Shown while the Command Bar, FIND mode, and REPLACE mode are closed. File-type icon, the directory dimmed, and the file name in bold. A file saved as a bare name shows the folder duckx-code was started from. `[No Name]` when the buffer has no path. `[+]` after the name while there are unsaved changes. A yellow message after it is state of the moment only (`Saving hello.rs…`, `3 of 12`, `Press Enter again to delete Lab`) and goes at the next key press. Results and errors (`Saved E:\Work\duckx-code\hello.rs`, `Copied 5 characters`) are [notices](#notices). |
| Left | `❯` input | Shown while the bar is open, in place of the file name. Matching commands are stacked on the rows above this line. |
| Right | `E 2 W 1` | Diagnostics in this file per level (`E`, `W`, `I`, `H`), each in its color. A level with none is omitted. The message for the cursor line is the colored bar above this line, with the file's diagnostic count at its right edge. |
| Right | Language server | Dim name of the server for this file's language once it has started: `rust-analyzer starting`, `rust-analyzer` when ready, `rust-analyzer: Indexing 40%` while it reports work (cut to 28 characters), `rust-analyzer stopped` after it gave up. |
| Right | `N selected` | Number of selected characters (only while selecting). |
| Right | `Top` / `N%` / `Bot` | Cursor position through the file: `Top` on the first line, `Bot` on the last, otherwise a percentage. |
| Right | `line:col` | Cursor position (1-based; the column is the on-screen column). |
| Right | `4 LF UTF-8` | Indent, line ending, and encoding of the focused file. A number is how many spaces one Tab inserts (`4 LF UTF-8`). `Tab 4` means Tab inserts a tab character, drawn that many columns wide (`Tab 4 CRLF UTF-8`). `LF` or `CRLF` is the ending a new line break uses. With line endings set to Auto, that is the ending detected in the file. LF or CRLF in settings is what the next save writes. The encoding is what the next save writes. `encoding` changes it. |

The icons and separators are Nerd Font glyphs; use a [Nerd Font](https://www.nerdfonts.com/) in your terminal or they render as empty boxes.

### Notices

Results, errors, and notes appear in a rounded snackbar at the bottom right, just above the status bar (or the diagnostic bar), instead of in the status line:

```text
                                        ╭ Done ───────────────────────────────╮
                                        │ Saved E:\Work\Duckx Canvas\PRODUCT.md │
                                        ╰─────────────────────────────────────╯
 NORMAL  master  PRODUCT.md                                    Top   1:1   4 LF UTF-8
```

| Title and color | For | Stays |
| :--- | :--- | :--- |
| `Done`, green | Something finished: `Saved …`, `Copied 5 characters`, `Replaced 4`, `Deleted a.rs`, `Converted to Thai (Kedmanee)` | 4 seconds |
| `Info`, blue | Information: `3 matches in 2 files`, `Focus on`, language server messages | 4 seconds |
| `Note`, yellow | Nothing happened or it was refused: `Nothing to replace`, `The preview is read-only`, `Select the text to convert first` | 6 seconds |
| `Error`, red | Something failed: `Cannot save a.rs: …`, `Unknown command: frob` | 8 seconds |
| `Assistant · F6` | A reply that arrived while the assistant panel was hidden (red when the turn failed). Opening the panel clears it. | 6 seconds |

Typing does not hide a notice; a newer one replaces it. What stays on the status line is the state you are in right now, cleared by the next key: progress (`Saving a.rs…`, `Searching…`, `Formatting…`), the FIND / REPLACE position (`3 of 12`), FILE mode marks and key hints, and anything that waits for your next key (`Press Enter again to delete Lab`, `The assistant asks to run apply_edit. F6 to answer`). While the Command Bar, FIND, or REPLACE is open, its messages (`Unknown command`, `Not found`) stay beside the input you are typing.

---

## 10. Settings

`setting` or `config` opens a settings page over the editor and the sidebar. The status bar badge reads `SETTINGS`. Sections stay in the left column and the rows of the highlighted section stay on the right.

Editor, Files, and Keybindings are saved in the same `settings.json` as the themes. `Delete` restores the highlighted row. On the section list, `Delete` restores every row in that section.

| Row | Values | Effect |
| :--- | :--- | :--- |
| Tab width | 2, 4, 8 | Spaces inserted for one Tab, and the on-screen width of a tab character. The default is 4. |
| Indent with spaces | On, Off | On inserts spaces. Off inserts a tab character. Text already in the file is left as it is. |
| Line endings | Auto, LF, CRLF | Auto keeps the ending detected in the file. LF or CRLF is the ending written the next time the file is saved. |
| Match brackets | On, Off | On draws the bracket at the cursor (or just before it) and its pair bold on a soft background, in the focused window, when both are on screen. A bracket with no pair is drawn in the error color. Brackets in strings and comments are skipped. The default is On. The `bracket-match` command flips it. |
| Colorize bracket pairs | On, Off | On colors `()` `[]` `{}` by how deep they are nested, cycling through the syntax theme's bracket colors. Unmatched ones use the error color. The default is On. The `bracket-colors` command flips it. |
| Trim trailing space | On, Off | On removes spaces and tabs at the end of a line when the file is saved. The default is Off. |
| Final newline | On, Off | On adds a newline at the end of a non-empty file when it is saved. The default is Off. |
| Show hidden files | On, Off | On lists dot files and folders (`.claude`, `.github`, `.env`) in the explorer, the file picker, `@` mentions, find in files, and the assistant's `search` tool. `.git` stays hidden either way, and `.gitignore` rules still apply. Saved as `files.show_hidden`; the default is Off. The `hidden` command flips it. |

**Keybindings** is one row per command. The value is the chord, such as `Ctrl+S`. `Enter` waits for the next key, and `Esc` cancels. The new chord is saved. `Delete` on that row restores its built-in chord. A chord another command already uses is refused, and a notice names that command.

These keys stay where they are, so the editor can always be used and closed:

* Letters, numbers, and other printable keys still insert text.
* `Up` / `Down` / `Left` / `Right`, `Home`, `End`, `PgUp`, and `PgDn`, including `Shift` for a selection, `Alt + Left` / `Alt + Right` for words, and `Ctrl + Home` / `Ctrl + End` for the document.
* `Ctrl + Left` / `Right` still move by word, and `Ctrl + Up` / `Down` still jump between blank lines. `Ctrl + W` (PANE mode) moves focus to another window; that chord can be changed (Pane select mode).
* `F1` and `Ctrl + ;` still open the command bar, even when the Command bar row is a different chord.
* `Ctrl + Q` still quits.

Appearance is saved. **Color theme** is the interface. **Syntax colors** is the code. The value on the right is the theme name, with `▾` to show that the row opens a list. The open list is a rounded panel right under the row, aligned to the value: the highlighted name is a full-width bar in the accent color marked `›`, and `↑↓ move  enter select  esc` sits in the bottom border. Every other list in the app (recent values, command matches, the assistant's `/` and `@` lists, completion, references, actions) looks the same. The syntax row also draws `fn main() { "hi" }` in the keyword, function, punctuation, and string colors of the selected syntax theme.

| Keys | Action |
| :--- | :--- |
| `Up` / `Down` | Move through the column that has focus. The highlight wraps. On the section list, the right column follows. Inside an open list, move through the names and preview that value. |
| `Enter` / `Right` | Move into the rows of the highlighted section. On a choice row, open the list. Inside the list, keep the highlighted value and save it. On a keybinding, wait for the next key. |
| `Esc` / `Left` / `Backspace` / `F1` | Close an open list and restore the previous value, or cancel a keybinding that is waiting for a key. Otherwise step back to the section list. From the section list, this closes the page and returns to the editor, or to the explorer if that had focus. |
| `Delete` | Restore the highlighted row to its built-in value and save. On the section list, restore every row in that section. |
| `Home` / `PgUp` | First row of the open menu |
| `End` / `PgDn` | Last row of the open menu |
| `Ctrl + Q` | Quit, the same as elsewhere |

Typing does not change the file while this page is open.

### Color theme and syntax colors

Both are JSON documents. Built-ins, in cycle order:

| `id` | Name |
| :--- | :--- |
| `tokyo-night` | Tokyo Night |
| `catppuccin-latte` | Catppuccin Latte |
| `catppuccin-frappe` | Catppuccin Frappé |
| `catppuccin-macchiato` | Catppuccin Macchiato |
| `catppuccin-mocha` | Catppuccin Mocha |

Tokyo Night follows the [VS Code theme](https://github.com/tokyo-night/tokyo-night-vscode-theme) (`themes/tokyo-night-color-theme.json`). The focused line number stays yellow, and the status bar keeps the lualine sections. A selected span uses the opaque mix of `editor.selectionBackground` (`#515c7e` at 30%) over the editor background, `#2b2f41`.

Catppuccin colors follow the [Catppuccin palette](https://github.com/catppuccin/palette). Interface roles use `base`, `mantle`, `surface`, `overlay`, and the accent colors (blue, mauve, green, yellow, red, peach). Syntax roles use mauve for keywords, blue for functions, yellow for types, green for strings, peach for numbers, sky for operators, maroon for parameters, and lavender for properties.

The choice is written to `settings.json`:

```json
{
  "color_theme": "tokyo-night",
  "syntax_theme": "tokyo-night",
  "editor": { "tab_width": 4, "indent_with_spaces": true, "line_ending": "auto", "match_brackets": true, "colorize_brackets": true },
  "files": { "trim_trailing_space": false, "final_newline": false },
  "lsp": { "enabled": true, "check_on_save": true, "format_on_save": false, "python": "basic", "servers": { "pylsp": false }, "custom": [ { "name": "taplo", "command": "taplo", "args": ["lsp", "stdio"], "extensions": ["toml"], "language_id": "toml", "root_markers": [".taplo.toml"] } ] },
  "ai": { "enabled": true, "provider": "ollama", "server": "", "model": "", "auto_approve": false, "streaming": true, "context_size": "16384", "think": true, "max_tokens": "", "request_timeout": 1800, "retries": 3, "openrouter_app_name": "duckx-code", "openrouter_referer": "https://github.com/Manuree/duckx-code" }
}
```

A changed keybinding is added only when it differs from the built-in chord, under `keybindings`, for example `"save": "ctrl+shift+s"`. Restoring that row removes the entry. A file written by an older version, with only the theme ids, still loads.

On Windows that file is `%APPDATA%\duckx-code\settings.json`. Elsewhere it is `$XDG_CONFIG_HOME/duckx-code/settings.json`, or `~/.config/duckx-code/settings.json` when `XDG_CONFIG_HOME` is unset.

User themes live next to it:

* `%APPDATA%\duckx-code\themes\color\*.json` — interface
* `%APPDATA%\duckx-code\themes\syntax\*.json` — code

A file with the same `id` as a built-in replaces that built-in. A new `id` is added at the end of the list. A file that is not valid JSON is skipped, and a notice says which file failed. The shipped documents are in `themes/color` and `themes/syntax` in the repository; those are the shape to copy. Colors are `#rrggbb`.

A syntax theme may add `brackets`, a list of colors used in turn for each nesting depth, and `bracket_error` for unmatched brackets. A color theme may add `bracket_match` under `editor`, the background behind a matched pair. Without them the depth colors are the theme's `constant`, `keyword`, and `function` colors, unmatched brackets use its `error` color, and the pair background is `editor.divider`, so an older theme file still loads. Every built-in theme sets all three.

A color theme may add a `diagnostics` section with `error`, `warning`, `info`, and `hint`. Without it, the Tokyo Night colors are used (`#db4b4b`, `#e0af68`, `#0db9d7`, `#1abc9c`). The Catppuccin themes use red, yellow, sky, and teal.

**Language** holds the language-server choices. `Enter` opens the list, `Up` / `Down` preview a value, `Enter` keeps it, and `Esc` restores the previous one. The choice is written to `settings.json` under `lsp`.

| Row | Values | Effect |
| :--- | :--- | :--- |
| Language server | On, Off | Off stops sending open files to a server. Diagnostics and completion for those files go away. |
| Check on save | On, Off | Off tells rust-analyzer not to run `cargo check` when a Rust file is saved. |
| Format on save | On, Off | On asks the language server to format the file before it is written. |
| Python checking | Off, Basic, Standard | How strictly pyright checks an open Python file. Basic reports an undefined name. |

With Language server Off, the other three rows are hidden; they keep their values and return when it is turned On.

### Language servers

**Language servers** comes right after Language and lists every server, built-in first, then the ones you added. With Language server Off the whole section is hidden. Opening Settings looks for each program on `PATH` in the background, once per session, so the page opens at once and each row fills in when the search answers (`looking on PATH…` until then).

Each server takes two lines. The first has a dot, the server name, the file extensions it is used for, and `On` / `Off`. The second says where the program was found, or `<program> not found on PATH · <install command>`, then how it is doing: `starting`, `running`, what it is busy with, `stopped: <reason>`, or `off`.

| Dot | Meaning |
| :--- | :--- |
| green `●` | Running (ready or busy) |
| yellow `●` | Starting |
| dim `●` | Installed, not running now |
| hollow `○` | Not found on `PATH` yet, or a custom server without a command |
| red `●` | Stopped (crashed too often, or failed to start) |

| Keys | Action |
| :--- | :--- |
| `Enter` / `Right` | Switch the server On or Off and save. Off stops it at once; its files go to the next server for them that is On and installed (pyright → basedpyright → pylsp → Ruff for Python), or stay plain. |
| `r` | Look for the program again and restart the server. A server that was missing or gave up is tried again. |
| `e` | On a custom server, show its fields below. A built-in cannot be edited. |
| `Delete` | On a built-in, turn it back On. On a custom server, press twice to delete it; any other key in between cancels. |

When two servers handle the same extension, a custom server comes before the built-ins, and among the rest the list order decides: the first one that is On and installed is used.

**Add server** asks for a name (at most 40 characters, not a built-in's name and not used by another custom server). The new server's fields then show below it, the same way a named provider's do under AI. They are shown only while a custom server is being edited:

| Field | Typed as | Notes |
| :--- | :--- | :--- |
| Name | text | Renaming keeps its On/Off choice. |
| Command | program name or full path | Required. Found on `PATH` (with `PATHEXT` on Windows). |
| Arguments | split at spaces | `"…"` or `'…'` keeps a space inside one argument; a backslash stays as it is, so Windows paths need no escaping. An unclosed quote is refused. |
| File extensions | comma-separated | Required, at least one. Dots and case are dropped: `.TOML, tml` → `toml, tml`. |
| LSP language id | text | `languageId` sent with each file. Empty uses the first extension. |
| Root markers | comma-separated | Files or folders that mark the project root, such as `.taplo.toml`. Empty uses the folder duckx-code was started from, or the file's own folder for a file outside it. |

A custom server is used once it has a command and an extension. Changing its command or arguments restarts it. At most 16 custom servers are kept. They are saved under `lsp.custom`, and a server switched Off is saved under `lsp.servers` as `"name": false` (On is the default and is not written). Both keys are optional, so an older `settings.json` loads unchanged; a hand-edited custom server with a blank, repeated, or built-in name is skipped with a notice.

**AI** holds the assistant. `Enter` on a list row (Assistant, Provider, Streaming, Auto-approve tools, Thinking) opens its list. `Enter` on a text row types that value with the same line keys as the command bar: `Left` / `Right` move by letter, `Ctrl + Left` / `Ctrl + Right` by word, `Home` / `End` (or `Ctrl + A` / `Ctrl + E`) jump to the ends, `Backspace` / `Delete` remove a letter, `Ctrl + Backspace` or `Alt + Backspace` removes a word, `Enter` keeps the value, and `Esc` drops the draft. A long value scrolls sideways to keep the cursor on screen. A number that is out of range keeps the draft open and says the allowed range. `Del` restores the highlighted row. Rows that do not apply are hidden, not greyed, and appear as soon as the choice they wait for is made: Thinking shows for Ollama, the two OpenRouter rows for OpenRouter, the model rows (Model context, Model max output, Model tools, Detected limits) once a Model is set, Keep tokens and Compact buffer while Auto compact is On, and the named-provider rows (Provider name, Provider kind, API key variable, Saved models, Delete provider) while a named provider is selected. With Assistant Off, only the Assistant row is shown. Hidden rows keep their values, and the arrows skip them. Every choice is written to `settings.json` under `ai`.

| Row | Values | Effect |
| :--- | :--- | :--- |
| Assistant | On, Off | Off refuses to send a prompt. |
| Provider | Ollama, LM Studio, OpenAI-compatible, OpenRouter | Ollama defaults to `http://127.0.0.1:11434/v1`, LM Studio to `http://127.0.0.1:1234/v1`, OpenAI-compatible to `https://api.openai.com/v1`, and OpenRouter to `https://openrouter.ai/api/v1`. Each provider keeps its own Server, Model, and API key: switching away saves them under `ai.profiles` and switching back restores them. Providers you add (Add provider) follow the built-ins in the list as `Lab · Ollama`, are saved under `ai.named` (name, kind, server, model, api_key, api_key_env, models), and the selected one's name is `ai.active`. A file without them loads and saves exactly as before. Moving through the open list previews a provider; `Esc` goes back to the one you had. An older file's `"openai"` loads as LM Studio and is saved as `"lmstudio"`. |
| Server | text | This provider's base address, including `/v1`. Empty shows `(default)` and uses the provider's address. Ollama talks to native `/api/chat` (derived from this URL) so Context size works; the others use `/v1/chat/completions`. |
| Model | text | This provider's model name, sent as `model` (for OpenRouter a slug such as `anthropic/claude-sonnet-4`). |
| API key | text, shown as dots | This provider's Bearer token. Empty (default) sends no Authorization header — right for local Ollama and LM Studio. `OPENROUTER_API_KEY` (OpenRouter) or `OPENAI_API_KEY` (OpenAI-compatible) wins over the saved key when it is set, and the row then reads `•••••••• (from OPENROUTER_API_KEY)`. The key is also sent when listing models. It is stored in plain text in `settings.json`; use the environment variable to keep it out of the file. |
| Model context | blank, or 1024 to 2000000 | Token window for the current provider and model only (`8_000` and `8,000` are accepted). Blank (default) uses Context size and shows it as `16384 (Context size)`. Ollama gets it as `num_ctx`; for the other providers it is the budget history is fitted to. Saved under `ai.model_limits` as `"<provider> <model>": {"context": …}`. With no Model set, the row says `(set a model first)`. `Del` clears it. |
| Model max output | blank, or 1 to 1000000 | Longest answer for the current provider and model only. Blank (default) uses Max tokens and shows what that gives (`16384 (Max tokens)`, or `(omit · Max tokens)`). Saved as `"max_output"` next to Model context. |
| Model tools | Auto, On, Off | For the current provider and model only. Auto (default) sends native `tools` and, if the server says this model cannot use them, switches it to a text description of the tools. On always sends native tools, even to a model detected without them, and shows the server's refusal instead of switching. Off never sends them and always uses the text prompt. Saved under `ai.model_tools` as `"<provider> <model>": "on"`; Auto removes the entry. |
| Detected limits | `Enter` | What a server said the current model cannot do (`no tools`, `no think`, or `none`). It is saved under `ai.detected` (keyed by chat URL and model) so a restart does not ask again. `Enter` or `Del` forgets it for the current server and model, and the next turn tries tools and think again. |
| Max tokens | blank, `off`, or 1 to 1000000 | Longest answer (`max_tokens`, or `num_predict` on Ollama). Blank (default) keeps the 16384 cap on Ollama and LM Studio and leaves it out on OpenAI-compatible and OpenRouter, so the model's own limit applies. `off` leaves it out everywhere. |
| Request timeout | 10 to 7200 seconds | Time one request may take, streaming included. Default 1800 (30 minutes) so a slow local model can finish a large `create_file`. |
| Retries | 0 to 10 | Extra tries after a 408, 429, or 5xx answer. Default 3. The wait doubles from one second (at most 30 seconds) and follows the server's `Retry-After` when it sends one. A refused connection, a bad key (401), no credits (402), or a forbidden model (403) is reported at once. |
| Streaming | On, Off | On (default) streams tokens (SSE on `/v1`; NDJSON on Ollama native) and shows text as it arrives; falls back to one JSON reply if streaming fails. An error the server sends inside the stream, and a 401 / 402 / 403 / 408 / 429 / 5xx answer, is shown as the error instead of retrying without streaming. Off always uses one JSON reply. |
| Auto-approve tools | On, Off | On runs `apply_edit`, `create_file`, and `save` without asking. Off (default) shows Yes/No buttons in the assistant panel. |
| Thinking | On, Off | Shown for Ollama only. On (default) sends `think: true` when the model supports it and drops it for models that reject it. |
| Context size | text (`16384`, `auto`, or a number) | Token window. Default `16384`. `auto` uses 16384. Ollama gets it as `options.num_ctx` (OpenAI-compat `/v1` ignores `num_ctx` on current Ollama — that is why the Ollama provider uses native `/api/chat`; host env `OLLAMA_CONTEXT_LENGTH` is a fallback). Every provider uses it as the budget auto compact works to and as the limit in the `ctx` counter at the assistant panel's bottom-right corner; nothing extra is sent to the others. LM Studio: set the real context in LM Studio and match it here. With another provider and `auto`, the counter shows `?` for the limit. A model's own Model context wins over this row. |
| Auto compact | On, Off | On (default): when a request would pass Context size less Compact buffer, older turns are summarized by the same model into one summary sent with the system prompt, and the newest Keep tokens stay word for word. The cut always lands at the start of a turn, so a tool call is never separated from its result. If the summary request fails, those turns are dropped instead and the transcript says so. Off keeps the earlier behavior: a small trail (about 2500 characters, the last 16 messages) with the oldest dropped. |
| Keep tokens | 1000 to 200000 | Newest history kept verbatim when auto compact runs. Default 15000. Never more than half the room left in the window, so a small context still has space for the summary. `8_000` and `8,000` are accepted. |
| Compact buffer | 5 to 50 percent | Head room left free before auto compact runs. Default 15 (`%` is accepted). |
| OpenRouter app name | text | Shown for OpenRouter only. Sent as `X-Title` so requests show under this name on openrouter.ai. Default `duckx-code`. Empty sends none. |
| OpenRouter referer | text | Shown for OpenRouter only. Sent as `HTTP-Referer`. Default `https://github.com/Manuree/duckx-code`. Empty sends none. |
| Add provider | text | Type a name and press `Enter` to add your own provider (for example two OpenAI-compatible servers, or a second Ollama box). It starts with the selected provider's kind and server, no model, and no key, and is selected at once. Names are up to 40 characters, unique in any case, and cannot be a built-in name or contain `:`. |
| Provider name | text | Named providers only. Renames the selected one. |
| Provider kind | Ollama, LM Studio, OpenAI-compatible, OpenRouter | Named providers only. How it is talked to: Ollama uses native `/api/chat` with Context size and Thinking, LM Studio and Ollama keep the local 16384 Max tokens default, OpenRouter sends its app headers. Its server, model, and key stay as they are. |
| API key variable | text (letters, digits, `_`) | Named providers only. An environment variable whose value wins over its saved key, such as `WORK_OPENAI_KEY`. Empty (default): only the saved key is used — a named provider never reads `OPENAI_API_KEY` / `OPENROUTER_API_KEY` unless you name it here. |
| Saved models | text, comma separated | Named providers only. Offered in the Model list with a `saved` tag, after `recent` and `current`. Repeats are dropped; at most 20. |
| Delete provider | `Enter` twice | Named providers only. The first `Enter` asks (`Press Enter again to delete Lab`), the second removes it and selects the built-in provider of its kind, with that provider's own values. |
| Clear recent history | `Enter` | Forgets every remembered server and model, for all providers. The row shows how many are saved. Current values stay. |

**Recent values.** Server and Model each keep the last 10 values, newest first: a value goes to the front when you save the row, when `/model name` switches to it, and when a turn with it gets an answer. Servers are kept per provider, and models per provider and server, so OpenRouter's list never shows Ollama tags. While typing one of these rows, a rounded `Recent` list opens under it, with `recent` or `server` at the right of each entry and `↑↓ pick  del forget  enter save` in its border: `Down` / `Up` walk it and put the highlighted value in the row (past either end returns to what you typed), typing filters it by any part of the name in any case, `Delete` on a highlighted recent value forgets it, and `Enter` keeps the row. The Model list also shows the models the server reports (`server` on the right, after the `recent` ones); they are fetched in the background when the row opens. The list opens as soon as you start editing the row, even before anything is remembered: it always offers the value saved now (`current`) and, for Server, the provider's built-in address (`default`), so a fresh install or an upgrade never shows an empty list. With nothing to offer it says `no recent values yet · saved when you press enter`. Under the Model list a line says `loading models from …` while the server is asked, or `server list unavailable: …` when it cannot answer (Ollama uses `/api/tags`, LM Studio and the others `/v1/models`); opening the row again asks again. `Delete` only forgets `recent` entries. While Server or Model is highlighted, the footer shows how many values are remembered (`enter edit · 3 recent`). API keys and the other text rows keep no list. API keys are never remembered. The lists are saved under `ai.recent` in `settings.json`; a hand-edited list is trimmed to 10 without repeats when it loads.

A model the server rejects `tools` or `think` for is remembered for that server and model only, and the memory survives a restart (Detected limits). A 401, 402, 403, 408, 429, or 5xx answer never marks a model that way, whatever its text says.

Code uses the syntax theme's `text` color. The other syntax roles are stored for highlighting and are what the sample on the Syntax colors row shows.

---

## 11. About

`about` opens a page over the editor and the sidebar. The status bar badge reads `ABOUT`. The page shows the duck logo, the name **duckx-code**, and the version from this build. `Esc`, `Backspace`, or `F1` closes it and returns to the surface that was focused. Printable keys do not insert while it is open. `Ctrl + Q` still quits.

---

## Assistant

`agent` or `F6` opens a rounded panel that floats over the bottom of the editor, just above the status line, and the panel is drawn only while its prompt is focused. The editor, previews, and sidebar keep their full height under it; the editor scrolls so its cursor line stays in sight above the panel, and keys in the prompt do not move the editor cursor. `Tab` in the command bar closes the bar and shows the panel. `Tab` in the prompt opens the command bar and hides the panel when no completion popup is open; with a `/` or `@` popup open, `Tab` completes the highlighted entry instead. `Esc` closes an open popup first, then leaves the prompt and hides the panel. Hiding the panel, with `Esc` or `F6`, does not stop a running turn. Only `/stop` stops it. A reply that arrives while the panel is hidden shows a note above the status bar, at the bottom right, for 6 seconds. The note is the first line of the answer and says `F6`. Opening the panel clears it. A failed turn uses the error color. `/new`, `/clear`, and `Ctrl + L` wait until the turn is done, and say so. `F6` from the editor shows it again, and `F6` while the prompt is focused closes it. From the prompt, PANE mode (`Ctrl + W`, an arrow, `Enter`) moves to another window. Inside the panel, `Ctrl + Up` reads the messages and `Ctrl + Down` returns to the prompt. `Alt + Up` makes the panel one row taller and `Alt + Down` makes it one row shorter. At least three rows of the editor stay in sight above it. At either limit a notice says `Assistant is at its tallest` or `Assistant is at its shortest`. The size lasts until duckx-code quits. In the editor those two keys still move a line. `Up` and `Down` move a completion popup when one is open; otherwise they scroll messages or move between wrapped prompt rows. `Ctrl + L` in the panel, or the slash commands `/new` and `/clear`, starts a new chat: transcript and tool-trail memory are cleared so a long session cannot push the system prompt and tools out of a small context window. Bare prompt words `clear` / `agent new` are no longer treated as commands (they would collide with normal chat); the command-bar entry `clear` (alias `agent-new`) remains as a shortcut outside the prompt.

### Slash commands

A line is a slash command only when `/` is the first character of the prompt (position 0 — leading spaces do not count). Text such as `please /help` or a path like `/usr/bin` or `src/a.rs` is sent to the model as normal chat. Typing `/` at the start opens command hints above the prompt (same style as the command bar: name, short description, ↑↓ tab enter esc on the selected row); further typing fuzzy-filters the list. `Up` / `Down` move the highlight, `Tab` completes the name (and leaves a trailing space for commands that take arguments), `Enter` completes and runs commands that need no argument (or shows status for `/model`, `/provider`, `/context`), and `Esc` closes the popup. An unknown `/xyz` shows an inline error in the transcript and is not sent to the model.

| Command | Description |
| :--- | :--- |
| `/new` | Start a new chat (also `Ctrl+L`). While a turn is running it does nothing and says to use `/stop` first. |
| `/clear` | Same as `/new`. |
| `/help` | List slash commands and key hints. |
| `/model [name]` | Show the current model, list models from the server (`/api/tags` or `/v1/models`, sent with the provider's key), or switch to `name`. The list is fetched in the background and appears in the transcript when it arrives. |
| `/provider [ollama, lmstudio, openai-compatible, openrouter, or a provider you added]` | Show or switch the AI provider. A name you added matches in any case (`/provider lab`). Each provider keeps its own server, model, and key. |
| `/context [n or auto]` | Show or set context size (`num_ctx`). |
| `/stream on or off` | Toggle response streaming. |
| `/auto-approve on or off` | Toggle tool auto-approve. |
| `/think on or off` | Toggle sending `think: true` to Ollama when supported. |
| `/stop` | Stop the current assistant run. This is the only way to stop it. `Esc` and `F6` only hide the panel. |
| `/retry` | Resend the last user prompt. |
| `/compact` | Summarize everything before the latest turn now, with the current model, and keep that turn. The title says `compacting` until the summary arrives. With no model set, or the assistant Off, it only drops the older messages. |
| `/plan` | Plan mode: the assistant reads code and writes a plan, but cannot edit. `/plan` again (or `/agent`) turns it off. |
| `/ask` | Ask mode: the assistant reads code and answers, but cannot edit. `/ask` again (or `/agent`) turns it off. |
| `/agent` | Back to Agent mode, where the assistant can edit. |
| `/changes [n]` | List the files the assistant changed in this chat (path, line, what changed, `· unsaved`). `/changes 2` opens the second one at its line and moves focus to the editor. |
| `/settings` | Open Settings → AI. |

### Modes

The editor has no modes. Only the assistant panel does, and a mode changes only what the assistant may do; keys in the editor stay the same.

| Mode | Title | Border | Tools |
| :--- | :--- | :--- | :--- |
| Agent (default) | `ready` | Sidebar border | All tools. |
| Plan | `ready : plan` | Yellow, the same warning color as the PANE border | Read-only tools only. The model is asked for a short step-by-step plan. |
| Ask | `ready : ask` | The theme's info color | Read-only tools only. The model is asked to answer, not to plan or edit. |

The mode stays across `/new` and lasts until you change it. If a model still calls an editing tool in Plan or Ask, the call is refused with a tool result that says which mode is on.

### Editor first

The assistant never takes the editor away from you while you type. While focus is in the editor (the panel hidden, or open but not focused):

- An edit changes the text around your cursor, but your cursor and selection stay on the same text.
- `open_file` and `create_file` open the file in a background tab. The visible tab does not change. `goto` does not move the cursor; the model is told to name the place instead.
- A tool that needs approval waits in the panel. Focus does not move, so a `y` you type goes into your file, not into the approval. The status bar says `The assistant asks to run apply_edit. F6 to answer`.

While the prompt is focused, `open_file`, `create_file`, and `goto` show the file and move the cursor as before.

### @file mentions

Typing `@` at the start of the prompt or after whitespace opens file hints above the prompt (an email-like `a@b.com` does not). The list is fuzzy-matched (case-insensitive) against the workspace file index (up to about 10 rows), ranking basename matches and recent/open files first, and ignoring `.git` / `target` the same way the file picker does. `Tab` or `Enter` inserts `@relative/path` (quoted when the path contains spaces). Unicode names (including Thai) work. On send, each mentioned file is attached as context: read with the usual size limits under a shared attach budget (large files are truncated with a note), path-safety checks still apply, and the `@path` text stays visible in the transcript.

Messages and the prompt wrap onto the next row when they are wider than the panel. The prompt holds multiple lines (up to 8 visible rows, then it scrolls). Printable keys edit the prompt. `Left` and `Right` move the cursor, and `Shift` with either key extends the selection. `Up` and `Down` move between wrapped rows. `Home` and `End` jump to the start or end of the current wrap row. `Backspace` and `Delete` remove the selection, or one character (Backspace at the start of a line joins it to the previous line). `Enter` sends the prompt. `Shift + Enter` inserts a newline. When the terminal cannot report Shift with Enter, `Alt + Enter` or `Ctrl + J` do the same. Bracketed paste keeps newlines (CRLF is normalized). While a mutating tool waits for approval, the prompt is replaced by `[ Yes ]` / `[ No ]` buttons (default Yes); Left/Right or Tab move the highlight and Enter confirms. Keys typed then do not edit the chat prompt. On terminals without bracketed paste (Windows Terminal and conhost on Windows), a fast run of keys is taken as one paste, so a multi-line paste keeps its newlines and is not sent; as a backup, Enter right after other fast keys is a newline. `Ctrl + Up` leaves the prompt to read the messages above it. `Up` and `Down` then scroll those messages, `Home` and `End` jump to the top and the latest line, and `Ctrl + Down` returns to the prompt. The model also receives the focused file, the cursor line and column, and any selection, and it is told to edit at that cursor unless the prompt names another place. The chat line stays the text you typed. The panel title is only the status: `ready`, `waiting`, `running read_file` (or another tool), `approve apply_edit`, or a short server error — not the word Assistant and not key hints. Key hints stay in this document (`Enter` sends, `Shift + Enter` newline). While the model is reasoning, the title shows a spinner and `thinking…`, optionally with a short live snippet of the latest reasoning line (reasoning is not written into the transcript). While the assistant is waiting or running a tool, `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` spins in front of that word. The same tool with the same arguments three times in a row stops the turn, and so does a 33rd tool call. A missing model, or Assistant set to Off, is a status-bar message.

The assistant can run these tools. A path that climbs out of the workspace with `..`, or through a symlink that leaves the folder, is refused. Files larger than 2 MB are not read or edited on disk. `create_file` refuses to create more than one new folder level under a missing parent. `apply_edit`, `create_file`, and `save` wait in the assistant panel with a Yes/No button row under a short card (tool name, path, and a compact `-`/`+` preview). Left/Right, `h`/`l`, or Tab move the highlight; Enter confirms the selected button (Yes is selected by default for a fast allow flow). Shortcuts `y` (Allow) and `n` / Esc (Deny) still work. Unless Auto-approve tools is On. Deny returns a tool result to the model so the turn can continue. Read-only tools (`read_file`, `list_dir`, `editor_state`, `diagnostics`, `search`, `lsp`, `open_file`, `goto`) run without asking. The system prompt also carries the workspace's `AGENTS.md` (read in the background, up to 6000 characters) when there is one. Later turns keep prior messages and tool calls. With Auto compact On, the newest 4 tool results stay whole and older ones are cut to a short head; with it Off, tool results are clipped. Before each request the client estimates tokens (chars ÷ 3.5) against Context size. With Auto compact On (default), older turns past Context size less Compact buffer are folded into a summary as described under Settings → AI; `/new` clears the summary too. With Auto compact Off, only a small trail is kept (about 2500 characters, oldest dropped first) and the oldest history is dropped to fit, always keeping the system prompt, tool specs, and the current user message.

**Context gauge.** The assistant panel's bottom border shows `ctx 12.4k/32k 37%` at its right corner whenever the panel is showing: tokens in use, the Context size, and the share. Tokens come from the server's last answer (`usage`, or Ollama's `prompt_eval_count` + `eval_count`) when it reports them, otherwise from the same estimate auto compact uses (before the first turn, the estimate of what would be sent). It updates after every request and after compaction. The text is dim under 70 %, the theme's warning color from 70 %, and its error color from 90 %. A limit that is not known reads `?`. In a narrow panel it shortens to `12.4k/32k`, then disappears; the corners stay. The `/` and `@` lists open above the prompt, inside the panel, so they never cover it. The status line is unchanged.

| Tool | What it does |
| :--- | :--- |
| `read_file` | Read a line range. An open file is read from the buffer. |
| `list_dir` | List one directory. |
| `editor_state` | Focused path, cursor line, and the selection. |
| `diagnostics` | Errors and warnings for a file. |
| `search` | Find text in the workspace (`path:line: text`, up to 60 hits). A lowercase query ignores case. `glob` limits the files (`*.rs`). Files in `.gitignore` are skipped, and so are dot files unless Show hidden files is On. `.git` is never searched. An open file is searched as it is in the editor, unsaved text included. |
| `lsp` | Ask the language server: `definition`, `references`, or `hover` at a line and a symbol (or column). A closed file opens in a background tab first. Needs a running language server for that file. |
| `apply_edit` | Replace a line range, or a snippet. A snippet must be unique, or set `replace_all` to change every copy. A snippet still matches when only line endings, curly quotes, or `12\|` line-number prefixes differ. The edit always goes to an editor buffer, never straight to disk: an open file, or a closed file opened in a background tab. It is one undo step (`Ctrl + Z`) and stays unsaved until you or `save` save it. A line-range edit is refused when the model has not read the file, or the file changed since it read it (you typed). The result shows the changed lines, and, when a language server runs for the file, the errors and warnings after the edit (it waits up to 4 seconds; the title says `checking`). |
| `create_file` | Write a new file and open it. A path that already exists is refused. |
| `open_file` | Open a workspace file. While you are in the editor it opens in a background tab. |
| `save` | Save an open file. |
| `goto` | Move to a line. A path opens that file first. While you are in the editor the cursor does not move. |

`write_file` is `create_file`. `search_replace` is `apply_edit`. `list_files` is `list_dir`. `read` is `read_file`. `grep`, `rg`, and `find` are `search`. `definition`, `goto_definition`, `references`, and `hover` are `lsp`. `bash` and `shell` are refused in the tool result, so the turn keeps going. A reply that arrives only in the server's reasoning field is shown. The request allows 8192 tokens. An empty reply says so in the panel. Set the server and the model under [Settings](#10-settings), section AI, before the first prompt.

## 12. Planned

Nothing in [`DESIGN.md`](./DESIGN.md) is waiting on a keybinding. New editing commands are listed with the keys above.
