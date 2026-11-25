# Duckx Code

[![Built With Ratatui](https://img.shields.io/badge/Built_With_Ratatui-000?logo=ratatui&logoColor=fff)](https://ratatui.rs/) [![License: MIT](https://img.shields.io/github/license/Manuree/duckx-code)](LICENSE) [![Release](https://github.com/Manuree/duckx-code/actions/workflows/release.yml/badge.svg?branch=master)](https://github.com/Manuree/duckx-code/actions/workflows/release.yml) [![Rust edition 2024](https://img.shields.io/badge/rust-edition%202024-orange?logo=rust)](https://doc.rust-lang.org/edition-guide/rust-2024/)

**CLI Code Editor For 75% keyboards. Modeless by default, with optional Normal / Insert / Visual modes. Write code with Ollama, LM Studio, any OpenAI-compatible server, or OpenRouter.**

duckx-code is a terminal editor for a 75% keyboard, modeless by default. The cursor moves with the dedicated arrow keys, Home, End, Page Up, and Page Down. By default there are no vim motions: no `hjkl`, no `w`/`b`, and no operator-pending commands. Printable keys insert text immediately. If you prefer modes, or your terminal drops some `Ctrl`, `Alt`, or `Shift` combinations, turn off Settings → Editor → **Modeless** for Normal, Insert, and Visual modes driven by plain letter keys, which always show which mode you are in and which keys it takes (see [Optional modes](#optional-modes-for-any-terminal)). Hold Shift with any navigation key to select. `F6` opens an assistant that writes and edits code through a local model on Ollama or LM Studio, or a cloud model through an OpenAI-compatible server or OpenRouter.

## Supported terminals

> [!IMPORTANT]
> **duckx-code supports only terminals that deliver every duckx-code shortcut.** Many shortcuts use `Shift`, `Ctrl + Shift`, or `Ctrl` with a symbol (`Shift + Space`, `Shift + Enter`, `Ctrl + Shift + Z`, `Ctrl + ]`, `Ctrl + /`, `Ctrl + .`, `Ctrl + Shift + O`). A terminal that only sends the old (legacy VT) key encoding cannot send these, so it is not supported.

duckx-code turns on the [kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/) at startup when the terminal says it supports it. On Windows, crossterm reads console input records, which carry the full modifier state.

| Status | Terminals |
| :--- | :--- |
| **Supported** | Windows Terminal, the Windows console (conhost: cmd, PowerShell), Alacritty 0.13+, kitty, Ghostty, foot |
| **Supported with a setting** | WezTerm, with `config.enable_kitty_keyboard = true` in `wezterm.lua` (it is off by default) |
| **Not supported** | xterm, GNOME Terminal and other VTE terminals, macOS Terminal.app, PuTTY, tmux (even with `extended-keys` some shortcuts still do not arrive) |
| **Untested** | VS Code's integrated terminal, Konsole, Rio, Warp, iTerm2 |

Notes:

* **Expected, not yet confirmed on every terminal.** The list follows from how each terminal encodes keys. It has not yet been tried on each terminal with a real keyboard. [docs/TERMINAL-KEYS.md](docs/TERMINAL-KEYS.md) has the per-key details and the fallbacks.
* **Some terminals and desktops keep a few keys by default.** Windows Terminal keeps `Alt + Enter` and `Alt + arrows`; kitty, Ghostty, and foot keep some `Ctrl + Shift` keys; macOS keeps `Ctrl + arrows` and `Ctrl + Space`; Linux desktops may keep `Ctrl + Alt + arrows`. Free them in that program's settings, or change the duckx-code key in Settings → Keybindings.
* **macOS:** set the Option key to act as Alt (Meta) in the terminal.
* **Older or limited terminal?** Turn on the [optional modes](#optional-modes-for-any-terminal). Their keys are plain letters (`i`, `v`, `dd`, `yy`, `gg`, `:`), which every terminal sends, so editing does not depend on the combinations listed above.
* **Known gap:** even in a supported terminal, `Ctrl + Shift + ]` (select to the matching bracket) can arrive as `Ctrl + }` and is not matched yet.

## Features

* **Modeless editing** with arrow-key navigation, Shift selection, multi-window splits, and tabs per window ([DOCUMENTS.md](DOCUMENTS.md)).
* **Optional modes** (Settings → Editor → **Modeless** Off): Normal, Insert, Visual, and Visual Line, with a colored mode badge, the keys of the current mode after that badge in the status bar, and a `?` list of every key. The keys are plain letters, so the editor stays fully usable in older terminals that drop some `Ctrl` / `Alt` / `Shift` combinations. Each mode key is also a Command Bar command ([Optional modes](#optional-modes-for-any-terminal)).
* **Mouse:** click to move the cursor, drag to select buffer text (never the line numbers), double-click for a word, triple-click for a line, wheel to scroll, and click to switch tabs or focus the sidebar and assistant ([DOCUMENTS.md](DOCUMENTS.md#mouse)).
* **Command Bar** (`F1`, `Shift + Space`, or `Esc` twice when there is nothing to close): every action also runs by name, with fuzzy matching.
* **File explorer** sidebar (`Ctrl + B`) and a workspace **file picker** (`Ctrl + P`).
* **Syntax highlighting** with tree-sitter for Rust, Python, JavaScript, TypeScript, Go, C, C++, C#, Java, and more.
* **Language servers:** diagnostics, hover, go to definition, references, rename, format, code actions, and completions.
* **Assistant** (`F6`) with Agent, Plan, and Ask modes, slash commands, and `@file` mentions. Providers: Ollama, LM Studio, OpenAI-compatible servers, and OpenRouter.
* Markdown preview, git changes in the gutter, file encodings (UTF-8, UTF-16, Windows-1252, Windows-874), color themes, and editable keybindings in Settings.

## Optional modes (for any terminal)

duckx-code is modeless by default: every printable key types, and actions use shortcuts such as `Ctrl + Shift + Z` or `Shift + Space`. Older or limited terminals (legacy VT encoding, some SSH clients, tmux) do not send every one of those combinations. For them, and for anyone who simply likes modes, duckx-code has optional Normal, Insert, and Visual modes whose keys are plain letters that arrive in every terminal.

**Turn it on:** open Settings (`setting` in the Command Bar) → Editor → **Modeless** → Off, or run the `modeless` command. **Start mode** picks Normal or Insert at startup. Turn **Modeless** back On for the usual editor; nothing else changes.

* **Normal:** move with `h j k l`, `w b`, `0 $`, `gg G`; `Ctrl + D` / `Ctrl + U` move half a page. Edit with `x`, `u` / `U`, `>>`, `gc`, `=` (format), and whole lines with `yy` / `dd` then `p` (below) or `P` (above) on a new line; counts like `5j` and `3dd`. `/` finds, `gd` goes to the definition, `gr` lists references, `ga` shows code actions, `gR` renames, `K` shows hover, `]e` / `[e` move the line, `]]` / `[[` jump to the next / previous `( ) [ ] { }` character, `gt` / `gT` switch tabs, `Tab` jumps forward, and `:` or `Shift + Space` opens the Command Bar, which runs every other action by name.
* **Insert:** `i`, `a`, `o` and friends start typing; `Esc` (or `Ctrl + [`) goes back to Normal. A terminal that sends `Esc` and the next letter together as `Alt` + letter is handled as `Esc`, then that letter, so no key is lost.
* **Visual / Visual Line:** `v` or `V`, then `i` enters Insert with the selection kept: the first typed character or paste replaces it, Backspace or Delete deletes it, and a move or `Esc` clears it without deleting (the replaced text is not copied; `c` cuts and types). Then `d` cut, `y` copy, `c` change, `>` / `<` or `Tab` / `Shift + Tab` indent and outdent, `gc` comment. Select inside quotes or brackets with `s"` `s(` `s{` `s[` or `si"` `si(` `si{` `si[` (or the closing bracket). `y` and `d` in Visual Line take whole lines, so `p` / `P` bring them back as lines.
* **Always clear which mode you are in:** a colored badge (`NORMAL`, `INSERT`, `VISUAL`, `V-LINE`) in the status bar, a mode-colored cursor, the keys that work right now after the badge in the status bar (Settings → Editor → **Mode hints**), and `?` for a searchable list of every key in the mode. The hints show each command's name in lowercase, with the letters to press highlighted (mode color, bold), such as **c**opy **l**ine for `cl`. The highlighted letter is uppercase only when that key needs Shift, so `v` is **v**isual and `V` is **V**isual line. A key whose letters are not in the name, or a symbol or `Ctrl` key, is shown before the name instead (`Tab  jump forward`).

**Key style: Vim or Mnemonic.** Settings → Editor → **Key style** (or the `key-style` command) picks the keys of Normal and Visual. **Vim** (the default) is the table above. **Mnemonic** names the command: a verb letter, then what it acts on. It applies only while Modeless is Off; motions (`h j k l w b 0 $ gg G { }`, `]]` / `[[`), `i a I A o O`, `v V`, `:`, `?`, `Esc`, and every `Ctrl` key stay the same.

| Mnemonic keys | Does |
| :--- | :--- |
| `cl` (`cc`) `cw` `ca` | Copy the line, the word, the whole file |
| `xl` (`xx`) `xw` | Cut the line, the word (`x` as in `Ctrl + X`) |
| `p` `P` | Paste below / above |
| `sc` `sl` (`ss`) `sw` `sa` `sb` `sq` `sp` | Select characters (Visual), lines (Visual Line), the word, all, to the bracket, inside quotes, inside `( )` |
| `si"` `si(` `si{` `si[` | Select inside quotes, `( )`, `{ }`, `[ ]` (or the closing bracket; also `si'`) |
| `dc` `dl` (`dd`) | Delete the character, the line (the clipboard is untouched) |
| `u` `r` | Undo / redo |
| `ff` (`/`) `fn` (`n`) `fP` (`N`) `fo` `R` | Find, next, previous, open a file, replace |
| `md` `mu` | Move the line down / up |
| `gd` `gr` `ga` `gR` `gt` `gT` `gl` `gh` (`H`) | Definition, references, actions, rename, next / previous tab, go to line (`12gl`), hover |
| `e` `q` | Explorer, close the tab (or quit on the last one) |
| Visual: `i` `c` `x` `e` `p` | Insert over the selection (type or paste to replace; not copied), copy, cut, edit (cut and type), paste |

Counts work the same way (`3cl` copies three lines), and `Esc` cancels a started sequence. Save stays `Ctrl + S` or `:w`.

**Command Bar aliases:** `:w` saves, `:q` closes the tab (or quits when it is the last one), and `:wq` saves and then does that. They match exactly, so `:q` never picks another command.

**Keys that differ from vim** (they keep their duckx-code meaning in every mode):

| Key | Here | In vim |
| :--- | :--- | :--- |
| `Ctrl + V` | Paste | Block selection |
| `Ctrl + C` | Copy (it never quits) | Cancel |
| `Ctrl + W` | PANE mode: pick a window | Window prefix |
| `Ctrl + R` | Not used; redo is `U` or `Ctrl + Y` | Redo |
| `Ctrl + D` / `Ctrl + U` | Half page down / up in Normal (in Insert they keep their usual meaning) | Same |
| `Ctrl + H` | Many terminals send Backspace as `^H`; both mean left in Normal | Backspace |

All keys and details: [DOCUMENTS.md → Modal editing](DOCUMENTS.md#modal-editing-modeless-off). Which shortcuts each terminal can send: [docs/TERMINAL-KEYS.md](docs/TERMINAL-KEYS.md).

## Install

There are no prebuilt downloads yet; they will come with the first GitHub Release. Until then, install with Cargo. duckx-code is not on crates.io, so Cargo builds it from this repository:

```sh
cargo install --git https://github.com/Manuree/duckx-code --locked
```

This builds one program, `duckx-code`, into Cargo's bin folder (`~/.cargo/bin`, or `%USERPROFILE%\.cargo\bin` on Windows), which rustup puts on `PATH`. `--locked` uses the dependency versions in `Cargo.lock`.

From a clone instead:

```sh
git clone https://github.com/Manuree/duckx-code
cd duckx-code
cargo install --path . --locked
```

Requirements:

* **Rust 1.91 or newer**, installed with [rustup](https://rustup.rs/). Edition 2024 needs 1.85, the locked `tree-sitter` 0.27 needs 1.90, and duckx-code itself uses `str::floor_char_boundary`, stable since 1.91. `rustup update` brings an older toolchain up to date.
* **A C compiler.** The tree-sitter grammars and `ring` (TLS for the assistant) compile C code. On Windows, install the [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with **Desktop development with C++** (rustup asks for these too). On macOS, run `xcode-select --install`. On Linux, install `gcc` or `clang` (for example `sudo apt install build-essential`).
* **A [supported terminal](#supported-terminals).** The status bar uses [Nerd Font](https://www.nerdfonts.com/) icons.

To update, run the same `cargo install` command again with `--force`. To remove it:

```sh
cargo uninstall duckx-code
```

## Usage

```powershell
duckx-code                 # reopen the last folder, files, and layout
duckx-code path\to\file.rs # open a file; a missing path opens as a new file
```

`Ctrl+S` saves. `Ctrl+Q` quits, and asks first when a file has unsaved changes. `Ctrl+C` copies; it does not quit. `Shift+Space` or `F1` opens the Command Bar, and so does `Esc` twice in the editor when there is nothing left to close (Settings → Editor → **Double Esc opens Command Bar**).

duckx-code captures the mouse by default. To use the terminal's own selection instead, hold `Shift` while dragging (Windows Terminal supports this, as do most terminals), or turn off Settings → Editor → **Mouse**.

## Configuration

Settings and the last session are stored in `%APPDATA%\duckx-code` on Windows, and in `$XDG_CONFIG_HOME/duckx-code` or `~/.config/duckx-code` elsewhere. The `setting` command opens the Settings page, where you can change the theme, the assistant provider and model, language servers, and keybindings.

## Documentation

* [DOCUMENTS.md](DOCUMENTS.md): keybindings and Command Bar commands.
* [docs/TERMINAL-KEYS.md](docs/TERMINAL-KEYS.md): which shortcuts can fail in which terminal, and what to use instead.
* [DESIGN.md](DESIGN.md): design notes.

## Release

A push to `master` publishes a GitHub Release only when `version` in `Cargo.toml` differs from the previous tip of the branch. The workflow builds Windows, Linux, and macOS binaries, runs the tests, and tags `v<version>`.

```toml
version = "0.2.0"
```

Commit that change and push to `master`. Pushing other files with the same version does not start a build.

## License

[MIT](LICENSE)
