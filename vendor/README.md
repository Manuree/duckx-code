# vendor/

## crossterm (0.28.1, patched)

A copy of [crossterm 0.28.1](https://crates.io/crates/crossterm/0.28.1) (MIT, `crossterm/LICENSE`), used through `[patch.crates-io]` in the root `Cargo.toml`, so ratatui uses it too. The examples and dev-dependencies were removed. The only code change is marked `duckx-code patch`:

* `src/event/sys/unix/parse.rs`: when crossterm cannot parse a complete `CSI` sequence (`ESC [` … final byte), it asks a decoder the program installed before dropping the sequence. The old behavior, dropping it, is kept when no decoder is installed or the decoder returns `None`.
* `src/event.rs`: `crossterm::event::set_csi_fallback` installs that decoder (Unix only), and `crossterm::event::parse_input` (hidden) runs the parser on raw bytes for tests.

Why: duckx-code turns on xterm's `modifyOtherKeys` in terminals without the kitty keyboard protocol (`src/term_keys.rs`). Those terminals send keys like `Ctrl + ;` as `CSI 27 ; 5 ; 59 ~`, which crossterm 0.28 (and its master branch, as of November 2026) drops. See [`docs/TERMINAL-KEYS.md`](../docs/TERMINAL-KEYS.md).

To update crossterm: copy the new release here, apply the same patch, and run `cargo test --lib -- --skip live_ollama` (the `term_keys` tests go through the patched parser).
