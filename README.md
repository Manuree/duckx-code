# Duckx Code

[![Built With Ratatui](https://img.shields.io/badge/Built_With_Ratatui-000?logo=ratatui&logoColor=fff)](https://ratatui.rs/) [![License: MIT](https://img.shields.io/github/license/Manuree/duckx-code)](LICENSE) [![Release](https://github.com/Manuree/duckx-code/actions/workflows/release.yml/badge.svg?branch=master)](https://github.com/Manuree/duckx-code/actions/workflows/release.yml) [![Rust edition 2024](https://img.shields.io/badge/rust-edition%202024-orange?logo=rust)](https://doc.rust-lang.org/edition-guide/rust-2024/)

**CLI Code Editor For 75% keyboards. No vim motions. Write code with Ollama, LM Studio, any OpenAI-compatible server, or OpenRouter.**

duckx-code is a modeless terminal editor for a 75% keyboard. The cursor moves with the dedicated arrow keys, Home, End, Page Up, and Page Down. There are no vim motions: no `hjkl`, no `w`/`b`, and no operator-pending commands. Printable keys insert text immediately. Hold Shift with any navigation key to select. `F6` opens an assistant that writes and edits code through a local model on Ollama or LM Studio, or a cloud model through an OpenAI-compatible server or OpenRouter.

Keybindings and Command Bar commands are listed in [DOCUMENTS.md](DOCUMENTS.md). The design notes are in [DESIGN.md](DESIGN.md).

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
* A terminal that can draw a full-screen TUI. The status bar uses [Nerd Font](https://www.nerdfonts.com/) icons.

To update, run the same `cargo install` command again with `--force`. To remove it:

```sh
cargo uninstall duckx-code
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
