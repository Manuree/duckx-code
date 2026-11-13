# TERMINAL-KEYS.md — Which shortcuts work in which terminal

duckx-code runs inside a terminal, so it only sees the keys the terminal sends. This page says what duckx-code does to get every shortcut through, which shortcuts still cannot work in which terminal, what to change in each terminal, and what to use instead. The bindings themselves are not changed for any terminal (DESIGN.md R9).

> **Mostly not tested on real terminals.** The matrix is based on how each protocol works, on crossterm 0.28's decoder (stock crates.io release), on the terminals' published default shortcuts, and on reading their source where noted. Only tmux 3.5a was run (keys sent with `tmux send-keys` into the `keylog` example, not typed on a keyboard). No Windows, macOS, or desktop Linux terminal was tried. `?` means unknown. The cells are meant to be corrected as terminals are checked: see [Checking a terminal](#checking-a-terminal).

For the full list of shortcuts, see [`DOCUMENTS.md`](../DOCUMENTS.md).

## What duckx-code asks the terminal for

At startup (`main.rs`, `setup_terminal`):

1. **kitty keyboard protocol.** duckx-code asks the terminal whether it supports it (`supports_keyboard_enhancement`). If it does, duckx-code turns it on with `DISAMBIGUATE_ESCAPE_CODES`, `REPORT_ALTERNATE_KEYS`, and `REPORT_ALL_KEYS_AS_ESCAPE_CODES`. Every key then arrives with all of its modifiers. It is turned off again on quit and after a panic, and only if it was turned on: a terminal that never turned it on is not sent the "off" sequence (`CSI < 1 u`), which it could read as "restore cursor".
2. **Shifted symbols and other layouts.** The kitty protocol reports `Ctrl + Shift + ]` as `Ctrl + }`. A shortcut lookup that finds nothing tries the key under it on a US keyboard (`Ctrl + }` → `Ctrl + Shift + ]`). With the Thai (Kedmanee) layout active, `Ctrl` or `Alt` plus a Thai character is looked up as the same physical key on the US layout (`Ctrl + ห` = `Ctrl + S`). An exact binding always wins, and keys without `Ctrl` or `Alt` are never shortcuts.

Windows needs nothing more. crossterm reads Win32 console input records there, not escape codes, and every record carries the key and the full modifier state.

Any other terminal gets the legacy encoding below. CSI u sequences (`ESC [ code ; modifiers u`, as sent by iTerm2's "Report keys using CSI u" or by tmux with `extended-keys-format csi-u`) are decoded by crossterm too.

> **Not done: xterm `modifyOtherKeys`.** xterm, WezTerm (by default), and tmux (`extended-keys-format xterm`) can send full modifiers as `CSI 27 ; modifiers ; code ~` after the program asks with `CSI > 4 ; 2 m`. crossterm 0.28 drops that form without a trace, and at level 2 those terminals send even `Ctrl + C` that way, so asking for it would break `Ctrl` keys. Supporting it needs a change in crossterm, and duckx-code uses crossterm as published. If crossterm adds a decoder for it, it is the next step for xterm and WezTerm.

## Why some keys get lost

* **Legacy (VT / xterm) encoding.** A terminal without the protocols above sends a printable key as its character, `Ctrl` + a letter as a control byte (`Ctrl + A` = `0x01`), arrows and F-keys as escape sequences that can carry modifiers (`ESC [1;5D` = `Ctrl + Left`), and `Alt` as `ESC` before the key. Everything else has no code of its own:
  * `Shift + Space` is a space, and `Shift + Enter` is `Enter`.
  * `Ctrl + Shift + letter` is the same byte as `Ctrl + letter`.
  * `Ctrl + ;` and `Ctrl + .` send the character, or nothing.
  * `Ctrl + ]` is `0x1D` and `Ctrl + /` is `0x1F`; crossterm reads them as `Ctrl + 5` and `Ctrl + 7`.
  * `Ctrl + H` and many terminals' `Ctrl + Backspace` are both `0x08`.
* **The terminal or the OS takes the key first.** This happens whatever the protocol. See the footnotes and [Taken by the OS or desktop](#taken-by-the-os-or-desktop).
* **Raw mode.** duckx-code turns off flow control and signal keys, so `Ctrl + S`, `Ctrl + Q`, `Ctrl + C`, and `Ctrl + Z` reach it. `0x0A` is read as `Ctrl + J`.

## Compatibility matrix

With each terminal's default settings, unless a footnote says otherwise. Cross-platform terminals (Alacritty, kitty, WezTerm, Ghostty, Rio, Warp) are rated on Linux and Windows; on macOS also read the Option-key note [o]. Shortcuts taken by the OS or the desktop are not in the cells.

**Cells:** ✅ expected to work · ⚠️ works after a setting change, or the terminal takes it by default (see the footnote) · ❌ cannot reach duckx-code · `?` unknown.

**Columns:** **WT** Windows Terminal (native Windows build) · **conhost** Windows console (cmd, PowerShell) · **VS Code** integrated terminal · **GNOME** GNOME Terminal and other VTE terminals · **Konsole** · **xterm** · **Alacr.** Alacritty 0.13+ · **kitty** · **WezTerm** · **Ghostty** · **foot** · **Rio** · **Warp** · **iTerm2** · **Term.app** macOS Terminal · **tmux** (inside any terminal) · **PuTTY**.

<!-- One row per shortcut. Keep the cell order of the header. Footnote letters are in [brackets]. -->

| Shortcut (action) | WT | conhost | VS Code | GNOME | Konsole | xterm | Alacr. | kitty | WezTerm | Ghostty | foot | Rio | Warp | iTerm2 | Term.app | tmux | PuTTY |
| :--- | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: | :-: |
| `Shift + Space` (Command Bar) | ✅ | ✅ | ? [v] | ❌ | ? | ❌ | ✅ | ✅ | ⚠️ [wz] | ✅ | ✅ | ? | ? | ? [i] | ❌ | ⚠️ [t] | ❌ |
| `Ctrl + ;` (Command Bar) | ✅ [w] | ✅ [w] | ? [v] | ❌ | ? | ❌ | ✅ | ✅ | ⚠️ [wz] | ✅ | ✅ | ? | ? | ? [i] | ❌ | ⚠️ [t] | ❌ |
| `F1` (Command Bar) | ✅ | ✅ | ⚠️ [v] | ⚠️ [g] | ? | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ? | ? | ✅ | ✅ | ✅ | ✅ |
| `Shift + Enter` (assistant newline) | ✅ | ✅ | ? [v] | ❌ | ? | ❌ | ✅ | ✅ | ⚠️ [wz] | ✅ | ✅ | ? | ? | ? [i] | ❌ | ⚠️ [t] | ❌ |
| `Alt + Enter` (assistant newline) | ⚠️ [wt] | ⚠️ [c] | ? [v] | ✅ | ? | ⚠️ [x] | ✅ | ✅ | ⚠️ [wz] | ✅ | ✅ | ? | ? | ⚠️ [o] | ⚠️ [o] | ✅ | ✅ |
| `Ctrl + J` (assistant newline) | ✅ | ✅ | ? [v] | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| `Ctrl + Shift + Z` (redo) | ✅ | ✅ | ? [v] | ❌ | ? | ❌ | ✅ | ⚠️ [k] | ⚠️ [wz] | ✅ | ✅ | ? | ? | ? [i] | ❌ | ❌ [t] | ❌ |
| `Ctrl + ]` (matching bracket) | ✅ | ✅ | ? [v] | ❌ | ? | ❌ | ✅ | ✅ | ⚠️ [wz] | ✅ | ✅ | ? | ? | ? [i] | ❌ | ❌ [t] | ❌ |
| `Ctrl + Shift + ]` (select to bracket) | ✅ | ✅ | ? [v] | ❌ | ? | ❌ | ✅ | ⚠️ [k] | ⚠️ [wz] | ✅ | ✅ | ? | ? | ? [i] | ❌ | ❌ [t] | ❌ |
| `Ctrl + /` (comment) | ✅ | ✅ | ? [v] | ❌ | ? | ❌ | ✅ | ✅ | ⚠️ [wz] | ✅ | ✅ | ? | ? | ? [i] | ❌ | ❌ [t] | ❌ |
| `Ctrl + .` (code action) | ✅ | ✅ | ? [v] | ❌ | ? | ❌ | ✅ | ✅ | ⚠️ [wz] | ✅ | ✅ | ? | ? | ? [i] | ❌ | ⚠️ [t] | ❌ |
| `Ctrl + Shift + O` (jump forward) | ✅ | ✅ | ? [v] | ❌ | ? | ❌ | ✅ | ⚠️ [k] | ⚠️ [wz] | ⚠️ [gh] | ⚠️ [f] | ? | ? | ? [i] | ❌ | ❌ [t] | ❌ |
| `Ctrl + Backspace` (word delete; not `Ctrl + H`) | ✅ | ✅ | ? [v] | ❌ [bs] | ? | ❌ [bs] | ✅ | ✅ | ⚠️ [wz] | ✅ | ✅ | ? | ? | ? [i] | ❌ [bs] | ⚠️ [t] | ❌ [bs] |
| `Ctrl + H` (replace) | ✅ | ✅ | ? [v] | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| `Ctrl + Space` (completions) | ✅ | ✅ | ? [v] | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| `Ctrl + Alt + Left / Right` (tabs) | ✅ | ✅ | ? [v] | ✅ | ? | ✅ | ✅ | ✅ | ✅ | ? [gh] | ✅ | ? | ? | ? [o] | ? [o] | ✅ | ⚠️ [p] |
| `Ctrl + Alt + Up / Down` (duplicate line) | ✅ | ✅ | ? [v] | ✅ | ? | ✅ | ✅ | ✅ | ✅ | ? [gh] | ✅ | ? | ? | ? [o] | ? [o] | ✅ | ⚠️ [p] |
| `Ctrl + Left / Right` (word) | ✅ | ✅ | ? [v] | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ? | ✅ | ✅ | ✅ | ⚠️ [p] |
| `Ctrl + Up / Down` (blank line; assistant reading) | ✅ | ✅ | ? [v] | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ? | ✅ | ✅ | ✅ | ⚠️ [p] |
| `Alt + arrows` (move line, word, sizes) | ⚠️ [wt] | ✅ | ? [v] | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ? | ? | ⚠️ [o] | ⚠️ [o] | ✅ | ⚠️ [p] |
| ``Alt + ` `` / `Alt + _` (convert keyboard) | ✅ | ✅ | ? [v] | ✅ | ✅ | ⚠️ [x] | ✅ | ✅ | ✅ | ✅ | ✅ | ? | ? | ⚠️ [o] | ⚠️ [o] | ✅ | ✅ |
| `Alt + Backspace` (word delete) | ✅ | ✅ | ? [v] | ✅ | ✅ | ⚠️ [x] | ✅ | ✅ | ✅ | ✅ | ✅ | ? | ? | ⚠️ [o] | ⚠️ [o] | ✅ | ✅ |
| `Shift + PgUp / PgDn` (select a page) | ✅ | ✅ | ⚠️ [v] | ⚠️ [g] | ✅ | ⚠️ [x] | ✅ | ✅ | ⚠️ [wz] | ? | ? | ? | ? | ? | ✅ | ✅ | ⚠️ [p] |
| `PgUp`, `PgDn`, `Home`, `End` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ? | ? | ⚠️ [ta] | ✅ | ✅ |
| `F2`–`F8`, `F12`, with `Shift` | ✅ | ✅ | ? [v] | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ? | ? | ✅ | ? | ✅ | ⚠️ [p] |
| `Ctrl + B` (sidebar) | ✅ | ✅ | ⚠️ [v] | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ? | ✅ | ✅ | ⚠️ [tp] | ✅ |
| `Ctrl + P` (file picker) | ✅ | ✅ | ⚠️ [v] | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ? | ✅ | ✅ | ✅ | ✅ |
| `Ctrl + W` (PANE mode) | ✅ | ✅ | ? [v] | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ? | ✅ | ✅ | ✅ | ✅ |

### Footnotes

* **[w] Windows layouts.** The key record gives the character of the active layout. With the Thai layout, `Ctrl + ;` arrives as `Ctrl + ว`, which duckx-code now reads as `Ctrl + ;`. Other non-US layouts are not mapped: a shortcut on a symbol key may need the US layout.
* **[wt] Windows Terminal** binds `Alt + Enter` (full screen), `Alt + arrows` (move between its own split panes), and `Alt + Shift + arrows` (resize them) by default. Remove them in Settings → Actions, or in `settings.json` with `{ "command": "unbound", "keys": "alt+enter" }`.
* **[c] conhost** uses `Alt + Enter` for full screen.
* **[v] VS Code.** Keys listed in `terminal.integrated.commandsToSkipShell` go to VS Code, not to the terminal: `F1`, `Ctrl + P`, `Ctrl + B`, `Shift + PgUp / PgDn`, and others. Set `"terminal.integrated.sendKeybindingsToShell": true`, or add `"-workbench.action.quickOpen"` and similar entries to that list. Whether VS Code's terminal (xterm.js) supports the kitty protocol is unknown, so the protocol rows are `?`. On Windows it runs through ConPTY.
* **[g] GNOME Terminal** uses `F1` for Help and `Shift + PgUp / PgDn` for scrollback (Preferences → Shortcuts). VTE has no kitty protocol, as far as is known, so the legacy rows are ❌.
* **[x] xterm** has no kitty protocol, and duckx-code does not use its `modifyOtherKeys` (see above), so it gets the legacy encoding. `Alt` + a character sends `ESC` first only with `XTerm*metaSendsEscape: true` (many distributions set it; upstream xterm sends an 8-bit character instead). xterm also scrolls its own buffer with `Shift + PgUp / PgDn`.
* **[k] kitty** maps `Ctrl + Shift + Z` (previous prompt), `Ctrl + Shift + O` (pass selection to a program), and `Ctrl + Shift + ]` (next window) by default, according to its shortcut list. Unmap them in `kitty.conf`.
* **[wz] WezTerm.** Its kitty protocol is off by default; with `config.enable_kitty_keyboard = true` in `wezterm.lua`, the protocol rows work. Without it WezTerm sends the legacy encoding. It also maps `Ctrl + Shift + Z` (zoom pane), `Alt + Enter` (full screen), and `Shift + PgUp / PgDn` (scroll) by default; disable them with `act.DisableDefaultAssignment`.
* **[gh] Ghostty** binds `Ctrl + Shift + O` (new split) on Linux, and may bind `Ctrl + Alt + arrows` to move between splits, according to its default keybind list. Not checked.
* **[f] foot** binds `Ctrl + Shift + O` (URL mode, `show-urls-launch`). Change it in `foot.ini`.
* **[i] iTerm2.** Works when iTerm2 reports keys with the kitty protocol, or with Profiles → Keys → General → "Report keys using CSI u" turned on (crossterm decodes CSI u). Which one is active by default is not known.
* **[o] macOS Option key.** Option types special characters (`å`, `∫`) unless it is set to act as Meta: Terminal.app Profiles → Keyboard → "Use Option as Meta key"; iTerm2 Profiles → Keys → Left / Right Option key → `Esc+`; kitty `macos_option_as_alt yes`; Alacritty `option_as_alt = "Both"`; Ghostty `macos-option-as-alt = true`. Terminal.app also maps Option + Left / Right to `ESC b` / `ESC f`, which duckx-code reads as `Alt + B` / `Alt + F`; remove those two entries to get `Alt + Left / Right`.
* **[ta] Terminal.app** scrolls its window with `PgUp`, `PgDn`, `Home`, and `End`, and sends them to the program only with `Shift`. Change it in Profiles → Keyboard.
* **[bs] `Ctrl + Backspace`** arrives as `0x08` (`Ctrl + H`) or as a plain `Backspace`. As `0x08` it opens Replace in the editor and the assistant prompt.
* **[t] tmux** passes modified keys on only with extended keys. tmux turns them on for a program that asks with `modifyOtherKeys`, which duckx-code does not do, so use `set -g extended-keys always` with `set -g extended-keys-format csi-u` (tmux 3.5+; set both in `.tmux.conf`, before the pane starts). Checked with tmux 3.5a: then `Shift + Space`, `Ctrl + ;`, `Shift + Enter`, `Ctrl + .`, and `Ctrl + Backspace` arrive correctly (⚠️), but `always` is level 1, so keys that have a control byte stay legacy: `Ctrl + Shift + Z` arrives as `Ctrl + Z` (undo), `Ctrl + ]` and `Ctrl + /` as `Ctrl + 5` / `Ctrl + 7`, and `Ctrl + Shift + O` as `Ctrl + O` (❌). Do not use the default format `xterm` with `always`: those keys are then dropped. The terminal around tmux must also send the key to tmux (tmux detects this for some terminals; otherwise `set -as terminal-features 'xterm*:extkeys'`). With `always`, other programs in tmux, such as the shell, also receive CSI u for these keys. With the default `extended-keys off` (or `on`), tmux is legacy: the ⚠️ cells are ❌.
* **[tp] tmux** uses `Ctrl + B` as its prefix. Press it twice to send it, or change `prefix`. GNU screen uses `Ctrl + A` (select all) the same way.
* **[p] PuTTY.** Configuration → Terminal → Keyboard: set "Shift/Ctrl/Alt with the arrow keys" to "xterm-style bitmap" (PuTTY 0.75+) so modifiers are sent with arrows, `Home`, `End`, and page keys, and pick a function-key mode that sends `Shift` with F-keys (such as "Xterm R6"). PuTTY uses `Shift + PgUp / PgDn` for scrollback.

## Taken by the OS or desktop

These apply in every terminal on that system.

| Key | Taken by (default) | In duckx-code |
| :--- | :--- | :--- |
| `Ctrl + Left / Right / Up / Down` | macOS Mission Control (Spaces, App Exposé) | word, blank line, assistant reading |
| `Ctrl + Space` | macOS (input source); Windows IMEs for Chinese and others | completions |
| `F1`–`F12` | macOS media keys unless `Fn` is held or "Use F1, F2, etc. keys as standard function keys" is on | F-key shortcuts |
| `Ctrl + Alt + Left / Right / Up / Down` | GNOME and other Linux desktops (workspaces); Intel graphics on Windows (screen rotation) | tabs, duplicate line |
| ``Alt + ` `` | GNOME (switch windows of one app); Windows when the grave key switches the input language (common with Thai) | convert keyboard (`Alt + _` also works) |
| `F12` | Guake and Yakuake drop-down terminals | definition |

## Fallbacks

| Shortcut | Use instead |
| :--- | :--- |
| `Shift + Space`, `Ctrl + ;`, `F1` | Any of the three, or rebind `command-bar` in Settings → Keybindings (the file picker knows only the three built-in keys) |
| `Shift + Enter` | `Ctrl + J` (every terminal), or `Alt + Enter` |
| `Ctrl + Shift + Z` | `Ctrl + Y` |
| `Ctrl + ]`, `Ctrl + .`, `Ctrl + Shift + O` | the `match-bracket`, `code-action`, `jump-forward` commands |
| `Ctrl + /` | `F4` |
| `Ctrl + Backspace` | `Alt + Backspace` (Command Bar, Settings), `Ctrl + W` (file picker) |
| `Ctrl + Alt + arrows` | the `tab-prev` / `tab-next` / `duplicate-line-up` / `duplicate-line-down` commands, or another chord in Settings → Keybindings |
| `Ctrl + Up / Down` in the assistant | none yet |
| Any key | every action in the Command Bar list also runs by name |

## Checking a terminal

**1.** Run `cargo run --example keylog` in the terminal. It sets the terminal up exactly as duckx-code does, writes `keylog.txt`, and prints each key. The first line says whether the kitty protocol was found.
**2.** Press the shortcut. It works when the line shows the expected event. Nothing at all means the terminal or the OS kept the key.

| Shortcut | Expected line |
| :--- | :--- |
| `Shift + Space` | `Char(' '), modifiers: SHIFT` |
| `Ctrl + ;` | `Char(';'), modifiers: CONTROL` |
| `Shift + Enter` | `Enter, modifiers: SHIFT` |
| `Alt + Enter` | `Enter, modifiers: ALT` |
| `Ctrl + Shift + Z` | `Char('z')` with `SHIFT \| CONTROL`, or `Char('Z')` with `CONTROL` |
| `Ctrl + ]` | `Char(']'), modifiers: CONTROL` (`Char('5')` means legacy) |
| `Ctrl + Shift + ]` | `Char(']')` with `SHIFT \| CONTROL`, or `Char('}')` with `CONTROL` |
| `Ctrl + /` | `Char('/'), modifiers: CONTROL` (`Char('7')` means legacy) |
| `Ctrl + .` | `Char('.'), modifiers: CONTROL` |
| `Ctrl + Shift + O` | `Char('o')` with `SHIFT \| CONTROL`, or `Char('O')` with `CONTROL` |
| `Ctrl + Backspace` | `Backspace, modifiers: CONTROL` (`Char('h'), modifiers: CONTROL` means it is `0x08`) |
| Arrows with modifiers | `Left`, `Up`, … with the modifiers pressed |

**3.** Change the cell in the matrix, and add a line to the table below.

### Checked

| Terminal | Version | OS | Date | How | Result |
| :--- | :--- | :--- | :--- | :--- | :--- |
| tmux | 3.5a | Debian 13 | 2026-11-13 | `tmux send-keys` into `keylog`, stock crossterm 0.28.1; `extended-keys` off, on, always (`xterm` and `csi-u` formats, set in the config file) | Off and on: legacy (`Ctrl + ;` = `;`, `Shift + Enter` = `Enter`, `Ctrl + ]` = `Ctrl + 5`). Always + `csi-u`: `Ctrl + ;`, `Shift + Enter`, `Shift + Space`, `Ctrl + .`, `Ctrl + Backspace` correct; `Ctrl + Shift + Z`, `Ctrl + Shift + O`, `Ctrl + ]`, `Ctrl + Shift + ]`, `Ctrl + /` still legacy. Always + `xterm`: those keys dropped. Not typed on a keyboard, so the outer-terminal part [t] is unchecked |
