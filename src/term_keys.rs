//! Asks the terminal to report modified keys in full, for terminals without the kitty
//! keyboard protocol, and decodes the answer. See `docs/TERMINAL-KEYS.md`.
//!
//! xterm's `modifyOtherKeys` (level 2) makes the terminal send a key whose modifiers
//! do not fit the old encoding as `CSI 27 ; modifiers ; code ~`. `Ctrl + ;` becomes
//! `ESC [27;5;59~` instead of a plain `;`, and `Shift + Enter` becomes `ESC [27;2;13~`
//! instead of `Enter`. xterm, WezTerm, and tmux (with `extended-keys`) do this.
//! crossterm 0.28 drops these sequences, so the vendored crossterm hands them to
//! [`decode_csi`] instead (`vendor/README.md`). A terminal that does not know the
//! request ignores it and keeps sending legacy codes.
//!
//! The request is not sent when the kitty protocol is on (it already reports every
//! key), on Windows (crossterm reads console input records there, which carry every
//! modifier), or on the Linux text console.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

/// `CSI > 4 ; 2 m`: xterm `modifyOtherKeys` level 2. The `CSI 0 m` (reset colors and
/// styles) after it is for a terminal that ignores the `>` and reads the request as
/// "underline, dim".
pub const MODIFY_OTHER_KEYS_ON: &str = "\x1b[>4;2m\x1b[0m";
/// `CSI > 4 m`: back to the terminal's own setting (xterm), or off (tmux, WezTerm),
/// with the same `CSI 0 m` guard.
pub const MODIFY_OTHER_KEYS_OFF: &str = "\x1b[>4m\x1b[0m";

/// Whether to send [`MODIFY_OTHER_KEYS_ON`]. `kitty` is whether the kitty keyboard
/// protocol was turned on; `term` is `$TERM`.
pub fn wants_modify_other_keys(kitty: bool, term: Option<&str>) -> bool {
    if cfg!(windows) || kitty {
        return false;
    }
    !matches!(term, Some("linux" | "dumb"))
}

/// Lets crossterm hand over the sequences it cannot parse. Call once, before reading
/// input. Does nothing on Windows.
pub fn install() {
    #[cfg(unix)]
    {
        let _ = crossterm::event::set_csi_fallback(decode_csi);
    }
}

/// Decodes `ESC [ 27 ; modifiers ; code ~` (xterm `modifyOtherKeys`, also sent by
/// WezTerm and tmux). `None` for anything else, which crossterm then drops as before.
pub fn decode_csi(buffer: &[u8]) -> Option<Event> {
    let body = buffer.strip_prefix(b"\x1b[27;")?.strip_suffix(b"~")?;
    let body = std::str::from_utf8(body).ok()?;
    let (mask, code) = body.split_once(';')?;
    let mask = mask.parse::<u8>().ok()?.checked_sub(1)?;
    let code = code.parse::<u32>().ok()?;

    let mut modifiers = KeyModifiers::NONE;
    if mask & 1 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    // 2 is Alt; 8 is xterm's Meta, which is the Alt key on most setups.
    if mask & (2 | 8) != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if mask & 4 != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }

    let code = match code {
        9 if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
        9 => KeyCode::Tab,
        13 => KeyCode::Enter,
        27 => KeyCode::Esc,
        // 8 is what WezTerm sends for Ctrl + Backspace.
        8 | 127 => KeyCode::Backspace,
        _ => {
            let c = char::from_u32(code).filter(|c| !c.is_control())?;
            // Same as crossterm's legacy decoding: an uppercase letter carries Shift.
            if c.is_uppercase() {
                modifiers |= KeyModifiers::SHIFT;
            }
            KeyCode::Char(c)
        }
    };
    Some(Event::Key(KeyEvent::new_with_kind_and_state(
        code,
        modifiers,
        KeyEventKind::Press,
        KeyEventState::NONE,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    const CTRL: KeyModifiers = KeyModifiers::CONTROL;
    const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
    const ALT: KeyModifiers = KeyModifiers::ALT;

    #[test]
    fn decodes_modify_other_keys_for_the_risky_shortcuts() {
        let cases: &[(&[u8], Event)] = &[
            (b"\x1b[27;5;59~", key(KeyCode::Char(';'), CTRL)),
            (b"\x1b[27;2;32~", key(KeyCode::Char(' '), SHIFT)),
            (b"\x1b[27;2;13~", key(KeyCode::Enter, SHIFT)),
            (b"\x1b[27;3;13~", key(KeyCode::Enter, ALT)),
            (b"\x1b[27;5;47~", key(KeyCode::Char('/'), CTRL)),
            (b"\x1b[27;5;93~", key(KeyCode::Char(']'), CTRL)),
            (b"\x1b[27;6;125~", key(KeyCode::Char('}'), CTRL | SHIFT)),
            (b"\x1b[27;5;46~", key(KeyCode::Char('.'), CTRL)),
            (b"\x1b[27;6;90~", key(KeyCode::Char('Z'), CTRL | SHIFT)),
            (b"\x1b[27;6;79~", key(KeyCode::Char('O'), CTRL | SHIFT)),
            (b"\x1b[27;5;104~", key(KeyCode::Char('h'), CTRL)),
            (b"\x1b[27;5;127~", key(KeyCode::Backspace, CTRL)),
            (b"\x1b[27;5;8~", key(KeyCode::Backspace, CTRL)),
            (b"\x1b[27;5;32~", key(KeyCode::Char(' '), CTRL)),
            (b"\x1b[27;2;9~", key(KeyCode::BackTab, SHIFT)),
            (b"\x1b[27;5;9~", key(KeyCode::Tab, CTRL)),
            (b"\x1b[27;3;96~", key(KeyCode::Char('`'), ALT)),
            (b"\x1b[27;9;120~", key(KeyCode::Char('x'), ALT)),
            (b"\x1b[27;7;115~", key(KeyCode::Char('s'), CTRL | ALT)),
            // Shift + a Thai character (WezTerm sends it this way at level 1 and 2).
            ("\x1b[27;2;3590~".as_bytes(), key(KeyCode::Char('ฆ'), SHIFT)),
        ];
        for (bytes, want) in cases {
            assert_eq!(
                decode_csi(bytes).as_ref(),
                Some(want),
                "{}",
                String::from_utf8_lossy(bytes)
            );
        }
    }

    #[test]
    fn ignores_everything_else() {
        for bytes in [
            &b"\x1b[27;5~"[..],
            b"\x1b[27;x;59~",
            b"\x1b[27;0;59~",
            b"\x1b[27;5;0~",
            b"\x1b[27;5;59u",
            b"\x1b[28;5;59~",
            b"\x1b[>4;2m",
            b"\x1b[27;5;1114112~",
        ] {
            assert_eq!(
                decode_csi(bytes),
                None,
                "{}",
                String::from_utf8_lossy(bytes)
            );
        }
    }

    #[test]
    fn asks_only_terminals_that_need_it() {
        assert!(!wants_modify_other_keys(true, Some("xterm-256color")));
        assert!(!wants_modify_other_keys(false, Some("linux")));
        assert!(!wants_modify_other_keys(false, Some("dumb")));
        assert_eq!(
            wants_modify_other_keys(false, Some("xterm-256color")),
            !cfg!(windows)
        );
        assert_eq!(
            wants_modify_other_keys(false, Some("tmux-256color")),
            !cfg!(windows)
        );
        assert_eq!(wants_modify_other_keys(false, None), !cfg!(windows));
    }

    /// Through the vendored crossterm parser, the way the input reader sees bytes.
    #[cfg(unix)]
    #[test]
    fn crossterm_hands_modify_other_keys_to_the_decoder() {
        use crossterm::event::parse_input;
        install();
        assert_eq!(
            parse_input(b"\x1b[27;5;59~a\x1b[27;2;13~"),
            vec![
                key(KeyCode::Char(';'), CTRL),
                key(KeyCode::Char('a'), KeyModifiers::NONE),
                key(KeyCode::Enter, SHIFT),
            ]
        );
    }

    /// The fallback must not change anything crossterm already reads.
    #[cfg(unix)]
    #[test]
    fn crossterm_still_reads_everything_else_the_same() {
        use crossterm::event::parse_input;
        install();
        assert_eq!(
            parse_input(b"\x1b[15~\x1b[1;5D\x1b[Z\x08\x7f\x1d\x1f\x00\x1bx"),
            vec![
                key(KeyCode::F(5), KeyModifiers::NONE),
                key(KeyCode::Left, CTRL),
                key(KeyCode::BackTab, SHIFT),
                key(KeyCode::Char('h'), CTRL),
                key(KeyCode::Backspace, KeyModifiers::NONE),
                key(KeyCode::Char('5'), CTRL),
                key(KeyCode::Char('7'), CTRL),
                key(KeyCode::Char(' '), CTRL),
                key(KeyCode::Char('x'), ALT),
            ]
        );
        // kitty protocol / fixterms CSI u is crossterm's own decoding.
        assert_eq!(
            parse_input(b"\x1b[59;5u\x1b[32;2u"),
            vec![
                key(KeyCode::Char(';'), CTRL),
                key(KeyCode::Char(' '), SHIFT)
            ]
        );
        assert_eq!(
            parse_input("สวัสดี".as_bytes()),
            "สวัสดี"
                .chars()
                .map(|c| key(KeyCode::Char(c), KeyModifiers::NONE))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            parse_input(b"\x1b[200~a\x1b[27;5;59~b\x1b[201~"),
            vec![Event::Paste("a\x1b[27;5;59~b".to_string())]
        );
        // A lone Esc at the end of the input is still Esc.
        assert_eq!(
            parse_input(b"\x1b"),
            vec![key(KeyCode::Esc, KeyModifiers::NONE)]
        );
        // Unknown sequences are still dropped, and the next key survives.
        assert_eq!(
            parse_input(b"\x1b[99;5~q"),
            vec![key(KeyCode::Char('q'), KeyModifiers::NONE)]
        );
    }
}
