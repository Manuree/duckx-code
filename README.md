# Duckx Code

**CLI Code Editor For 75% keyboards. No vim motions. Write code with Ollama or LM Studio.**

duckx-code is a modeless terminal editor for a 75% keyboard. The cursor moves with the dedicated arrow keys, Home, End, Page Up, and Page Down. There are no vim motions: no `hjkl`, no `w`/`b`, and no operator-pending commands. Printable keys insert text immediately. Hold Shift with any navigation key to select. `F6` opens an assistant that writes and edits code through a local model on Ollama or LM Studio.

Keybindings and Command Bar commands are listed in [DOCUMENTS.md](DOCUMENTS.md). The design notes are in [DESIGN.md](DESIGN.md).

## Install

Download a build from [Releases](https://github.com/Manuree/duckx-code/releases):

| System | Archive |
| :--- | :--- |
| Windows (x86_64) | `duckx-code-windows-x86_64.zip` |
| Linux (x86_64) | `duckx-code-linux-x86_64.tar.gz` |
| macOS (Apple silicon) | `duckx-code-macos-aarch64.tar.gz` |

On Windows, unzip and run `duckx-code.exe`. On Linux and macOS, extract the archive and run `duckx-code`. A terminal that can draw a full-screen TUI is required. The status bar uses [Nerd Font](https://www.nerdfonts.com/) icons.

From source, with Rust 1.85 or newer (edition 2024):

```powershell
cargo install --git https://github.com/Manuree/duckx-code
```

## Usage

```powershell
duckx-code                 # reopen the last folder, files, and layout
duckx-code path\to\file.rs # open a file; a missing path opens as a new file
```

`Ctrl+S` saves. `Ctrl+Q` quits, and asks first when a file has unsaved changes. `Ctrl+C` copies; it does not quit. `Shift+Space` or `F1` opens the Command Bar.

Settings and the last session are stored in `%APPDATA%\duckx-code` on Windows, and in `$XDG_CONFIG_HOME/duckx-code` or `~/.config/duckx-code` elsewhere.

## Release

A push to `master` publishes a GitHub Release only when `version` in `Cargo.toml` differs from the previous tip of the branch. The workflow builds Windows, Linux, and macOS binaries, runs the tests, and tags `v<version>`.

```toml
version = "0.2.0"
```

Commit that change and push to `master`. Pushing other files with the same version does not start a build.

## License

[MIT](LICENSE)
