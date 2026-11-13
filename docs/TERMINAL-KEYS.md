# TERMINAL-KEYS.md — Shortcuts that may not work in every terminal

duckx-code runs inside a terminal, so it only sees the keys the terminal sends. Some chords cannot be encoded the old way, and some never reach the program because the terminal or the OS keeps them. This page lists every duckx-code shortcut that is at risk, what to use instead today, and a suggested portable key.

> **Not tested on real hardware.** Nothing here was tried in a real Windows Terminal, conhost, or macOS terminal. It is based on how the terminal protocols work, on crossterm 0.28's decoder (`crossterm/src/event/sys/unix/parse.rs`), and on the default key settings of the terminals and desktops named. Your terminal's settings can differ.
>
> **Proposals are only proposals.** The "Suggested key" column is not implemented. Each one was checked against the current bindings in `src/keys.rs` and `src/input.rs` so that it does not take a key that is already used.

For the full list of shortcuts, see [`DOCUMENTS.md`](../DOCUMENTS.md).

## Why some keys get lost

* **Legacy (VT / xterm) encoding.** A plain terminal sends a key as bytes. A printable key is its character. `Ctrl` plus a letter is a control byte (`Ctrl + A` = `0x01` … `Ctrl + Z` = `0x1A`). Arrows, `Home`, F-keys and similar keys are escape sequences, and these can carry `Shift` / `Alt` / `Ctrl` (`ESC [1;5D` is `Ctrl + Left`). `Alt` is sent as `ESC` before the key. Everything else has no code of its own:
  * `Shift + Space` is a space, and `Shift + Enter` is `Enter` (`\r`).
  * `Ctrl + Shift + letter` is the same byte as `Ctrl + letter`, so `Shift` is lost.
  * `Ctrl + ;` and `Ctrl + .` send the plain character, or nothing.
  * Some chords share a byte with another key. `Ctrl + H` and many terminals' `Ctrl + Backspace` are both `0x08`. `Ctrl + ]` is `0x1D` and `Ctrl + /` is `0x1F`, and crossterm decodes those two as `Ctrl + 5` and `Ctrl + 7`.
  * An `Esc` followed quickly by another key can be read as `Alt` plus that key.
* **The kitty keyboard protocol (CSI u).** It sends every key with all of its modifiers. duckx-code turns it on at startup when the terminal says it supports it (`main.rs`: `PushKeyboardEnhancementFlags` with `DISAMBIGUATE_ESCAPE_CODES`, `REPORT_ALTERNATE_KEYS`, `REPORT_ALL_KEYS_AS_ESCAPE_CODES`). In terminals that implement it, such as kitty, WezTerm, foot, Ghostty, recent Alacritty, and iTerm2 with the option on, the legacy problems on this page do not happen.
* **xterm's `modifyOtherKeys`.** This is another way to send modified keys. duckx-code does not turn it on, and crossterm 0.28 does not decode it. A terminal that supports only `modifyOtherKeys`, and not the kitty protocol, gets the legacy behavior.
* **tmux and screen.** Inside tmux, keys go through tmux. tmux sends legacy codes unless `extended-keys` is configured, so treat it as a legacy terminal. tmux's prefix is `Ctrl + B`, and GNU screen's is `Ctrl + A`.
* **Windows.** crossterm does not read escape codes on Windows. It reads Win32 console input records, which carry the key and the modifier state. In principle `Shift + Space`, `Shift + Enter`, and `Ctrl + Shift + Z` are therefore visible there. This is untested: the console still turns some chords into characters first, and a non-US layout can change what `Ctrl` plus a symbol gives. On Windows, `Ctrl + Alt` together is `AltGr` and types a character (`key_burst.rs`), so `Ctrl + Alt + letter` chords are never used.
* **The raw-mode basics.** duckx-code puts the terminal in raw mode. That turns off flow control (`IXON`), so `Ctrl + S` and `Ctrl + Q` reach the editor. It also turns off signal keys (`ISIG`), so `Ctrl + C` and `Ctrl + Z` do too. In raw mode crossterm reads `0x0A` as `Ctrl + J`, not `Enter`.

## 1. Fails under legacy VT / xterm encoding

These work in terminals with the kitty protocol, and are expected to work on Windows (untested). They fail in a terminal that only speaks the legacy encoding, which is the default in GNOME Terminal and other VTE terminals, xterm, Konsole, macOS Terminal.app, PuTTY, and inside tmux.

| Key | Action | What the program gets | Fallback in duckx-code today | Suggested key (proposal) |
| :--- | :--- | :--- | :--- | :--- |
| `Shift + Space` | Open the Command Bar (editor, sidebar, assistant prompt, file picker) | A plain space, so it types a space | `F1`. `Ctrl + ;` too, but it fails in the same terminals. The `command-bar` key can be changed in `settings.json` (`"keybindings": { "command-bar": "ctrl+k" }`); the editor, sidebar, and assistant prompt use the new key, but the file picker knows only the built-in keys | `Ctrl + K`, which is not used anywhere |
| `Ctrl + ;` | Open the Command Bar (reserved; cannot be rebound) | `;` or nothing. In the editor a `;` may be typed | `F1` | `Ctrl + K` |
| `Shift + Enter` | Newline in the assistant prompt | `Enter`, so the prompt is **sent** | `Ctrl + J` and `Alt + Enter` both insert a newline (`route_agent`). On Windows, `Enter` right after a fast run of keys is a newline too (paste-burst backup) | Use `Ctrl + J`: it is `0x0A`, so it works in every terminal. `Alt + Enter` is taken by Windows Terminal (see section 2) |
| `Ctrl + Shift + Z` | Redo | `Ctrl + Z`, so it **undoes** | `Ctrl + Y`, or the `redo` command | none needed |
| `Ctrl + ]` | Jump to the matching bracket | `0x1D`, which crossterm reads as `Ctrl + 5`, so nothing happens | the `match-bracket` command | Also accept `Ctrl + 5`, which is what these terminals deliver |
| `Ctrl + Shift + ]` | Select through the matching bracket | `0x1D` (`Ctrl + 5`) or nothing; `Shift` is lost | none: there is no command for it | Add a `select-bracket` command. The name is free |
| `Ctrl + /` | Toggle a line comment | `0x1F`, which crossterm reads as `Ctrl + 7`, so nothing happens | `F4`, or the `comment` command | Also accept `Ctrl + 7` |
| `Ctrl + .` | Code action | `.` or nothing. In the editor a `.` may be typed | the `code-action` command | `F9`, which is not used |
| `Ctrl + Shift + O` | Jump forward | `Ctrl + O`, so it **jumps back** | the `jump-forward` command | `Ctrl + T`, which is not used |
| `Ctrl + Backspace` | Delete the previous word in the Command Bar, the file picker, and Settings text rows | Usually `0x08` = `Ctrl + H`. In the editor and the assistant prompt that opens **Replace**. In those text inputs it does nothing. Some terminals send plain Backspace instead | Command Bar and Settings: `Alt + Backspace`. File picker: `Ctrl + W`. The editor, the assistant prompt, and find / replace have no word delete. When the terminal does send `Ctrl + Backspace`, the editor and find / replace treat it as a plain Backspace and the assistant prompt ignores it | `Alt + Backspace` as word delete in every text input, including the editor (`ESC DEL` decodes everywhere). Do not use `Ctrl + W` in the editor: it is PANE mode |
| `Ctrl + H` | Replace | Works: `0x08` is decoded as `Ctrl + H`, not Backspace. The risk is the other way round: `Ctrl + Backspace`, and `Backspace` on a terminal set to send `^H`, arrive as `Ctrl + H` and open Replace | the `replace` command | `Ctrl + R` for Replace, which is not used. Keep `Ctrl + H` as a second key |

## 2. May be taken by the OS or the terminal

These keys encode fine, but by default something in front of duckx-code may use them first. Most can be freed in the OS or terminal settings.

| Key | Action in duckx-code | Taken by (by default) | Fallback in duckx-code today | Suggested key (proposal) |
| :--- | :--- | :--- | :--- | :--- |
| `Ctrl + Alt + Left / Right` | Previous / next tab | GNOME and other Linux desktops (switch workspace); Intel graphics on Windows (rotate the screen) | `tab-prev` / `tab-next` commands; can be changed in Settings → Keybindings | `Alt + PgUp` / `Alt + PgDn` (today these are a plain `PgUp` / `PgDn`) |
| `Ctrl + Alt + Up / Down` | Duplicate line up / down | Older GNOME, Xfce, Cinnamon (switch workspace); Intel graphics on Windows | `duplicate-line-up` / `duplicate-line-down` commands; Settings → Keybindings | `Alt + Shift + Up / Down` (VS Code's keys; today they are the same as `Shift + Up / Down`). Windows Terminal may use them to resize split panes |
| `Ctrl + Left / Right` | Move by word in the editor, Command Bar, file picker, assistant prompt, and Settings text rows | macOS Mission Control (switch Space) | Editor: `Alt + Left / Right`. Other inputs: `Home` / `End` only | `Alt + Left / Right` for words in every text input (free in the Command Bar and the assistant prompt; a plain character move in the file picker today) |
| `Ctrl + Up / Down` | Editor: jump between blank lines (or scroll a preview). Assistant: read the messages above the prompt and return | macOS Mission Control and App Exposé | Editor: `PgUp` / `PgDn`. Assistant: none | Assistant: `PgUp` to read the messages and `PgDn` to return (`PgUp` / `PgDn` are not used in the prompt) |
| `Ctrl + Shift + Left / Right` | Select by word (editor, assistant prompt) | macOS Mission Control may also take it | `Alt + Shift + Left / Right` (editor only) | Same as `Ctrl + Left / Right` |
| `Shift + PgUp` / `Shift + PgDn` | Select one screen up / down (editor, sidebar) | xterm and GNOME Terminal (scroll back). Konsole only outside the full-screen view, so it is fine there | repeat `Shift + Up / Down` | none; unbind it in the terminal |
| `PgUp`, `PgDn`, `Home`, `End` (all uses) | Page / line moves everywhere | macOS Terminal.app keeps them for its own scrolling and sends them to the program only with `Shift` (Profiles → Keyboard) | `Shift + PgUp` and so on reach the program, but then select | none; change the Terminal.app profile, or use iTerm2 |
| Any `Alt + …` on macOS | Word moves, `Alt + Up / Down` (move line, resize the assistant), ``Alt + ` `` (convert keyboard), `Alt + Enter`, `Alt + Backspace`, `Alt + Left / Right` (sidebar width) | Option types special characters (`å`, `∫`, …) unless "Use Option as Meta key" (Terminal.app) or "Esc+" (iTerm2) is set | Commands: `move-line-up` / `move-line-down`, `convert-keyboard`, `wider` / `narrower`. Newline: `Ctrl + J`. Words: `Ctrl + Left / Right`, which Mission Control may take | none; set Option as Meta |
| `F1` | Open the Command Bar; close the Command Bar, Settings, and About | GNOME Terminal (Help). On a Mac, brightness unless `Fn` is held or F-keys are set as standard keys | `Shift + Space` and `Ctrl + ;`, but both fail in GNOME Terminal (section 1). **In a default GNOME Terminal none of the three may reach duckx-code.** Rebind `command-bar` in `settings.json`, or turn off the F1 shortcut in GNOME Terminal | `Ctrl + K` |
| `F2`–`F8`, `F12` | Hover, rename, find next, comment, assistant, format, diagnostics, definition / references | macOS media keys unless `Fn` is held. `F12` toggles Guake and Yakuake drop-down terminals | The matching commands (`hover`, `rename-symbol`, `comment`, `agent`, `format`, `diagnostic-next` / `diagnostic-prev`, `goto-definition`, `goto-references`) | none |
| `Alt + Enter` | Newline in the assistant prompt | Windows Terminal (toggle full screen) | `Ctrl + J` | none |
| `Ctrl + Space` | Open completions | macOS (switch input source); Windows IMEs for Chinese and other languages (toggle the IME) | Completions also open while typing; the `complete` command; Settings → Keybindings | none |
| ``Alt + ` `` | Convert keyboard layout | Windows when the backtick key is the language-switch hotkey (see [Wrong keyboard layout](../DOCUMENTS.md#wrong-keyboard-layout)) | `Alt + _`; the `convert-keyboard` command; Settings → Keybindings | none |
| `Ctrl + B` | Sidebar | tmux prefix; VS Code's integrated terminal | the `explorer` command | none |
| `Ctrl + A` | Select all | GNU screen prefix | the `select-all` command | none |
| `Ctrl + P` | File picker | VS Code's integrated terminal (Quick Open) | the `file-picker` command | none |
| `Ctrl + W` | PANE mode (move between windows) | Terminals running in a web browser (close the browser tab) | can be changed in Settings → Keybindings (`Pane select mode`) | none |
| `Ctrl + C` / `Ctrl + V` | Copy / paste | Windows Terminal copies when it has its own mouse selection, and pastes on `Ctrl + V`. The pasted text still arrives, as typed keys | — | none; harmless |

## 3. Safe almost everywhere

* **`Ctrl` + a letter:** `Ctrl + S`, `C`, `X`, `V`, `Z`, `Y`, `A`, `D`, `F`, `P`, `B`, `O`, `W`, `Q`, plus `Ctrl + J` and `Ctrl + L` in the assistant prompt and `Ctrl + A` / `Ctrl + E` in text rows. Each one is a single control byte, and raw mode stops the terminal from acting on `Ctrl + S` / `Q` / `C` / `Z`. The exceptions are a multiplexer prefix (section 2) and `Ctrl + H` (section 1).
* **`Ctrl + Space`:** it is `0x00`, which crossterm decodes. It is affected only by the input-source shortcuts in section 2.
* **`Shift` with the arrows, `Home`, `End`, and `Delete`:** selection, `Shift + Delete` (delete line), and `Shift + Down` (replace all). These are sent as modifier escape sequences. The exceptions are `Shift + PgUp` / `PgDn` (section 2). The Linux text console does not send `Shift` with these keys.
* **`Alt + arrows` outside macOS:** sent as `ESC [1;3A` and so on.
* **`Ctrl + Home` / `Ctrl + End`, with or without `Shift`.**
* **`Tab` and `Shift + Tab`:** indent and outdent, snippet placeholders, accepting a completion, and the Command Bar / assistant switch. `Shift + Tab` is `ESC [Z` everywhere.
* **`Enter`, `Esc`, and `Backspace`:** dialogs, pickers, menus, Settings.
* **F-keys:** `F2`–`F8` and `F12`, and `Shift` with them (`Shift + F2`, `F3`, `F8`, `F12`), apart from the cases in section 2. A few terminals, such as rxvt and the Linux console, send `Shift + F-key` as a higher F-key.
* **Plain letters in Settings and dialogs:** `r` and `e` on a language server row, and the dialog letters (`Y` / `N` / `C`).

## What to do when a key does not work

1. **Check what your terminal sends.** Open Settings → Keybindings, press `Enter` on a row, then press the key. What the row shows is what duckx-code received.
2. **Use the fallback** from the tables above. Every action in the Command Bar list also works as a command, by name.
3. **Change the key.** Settings → Keybindings, or `keybindings` in `settings.json`, accepts `Ctrl` or `Alt` plus a character, plain and `Shift` F-keys, `Shift + Space`, `Alt + Up / Down`, and `Ctrl + Alt` + arrows. `F1`, `Ctrl + ;`, and `Ctrl + Q` are reserved.
4. **Use a terminal with the kitty keyboard protocol** for full modifier support, and set Option as Meta on macOS.
