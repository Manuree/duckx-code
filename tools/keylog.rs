//! Records raw crossterm events to `keylog.txt` to diagnose keyboard input issues.
//! Run with `cargo run --example keylog`, type the problem keys, then press Esc.
//! It asks the terminal for keys the same way the editor does (the kitty keyboard
//! protocol when supported, same flags), so what it logs is what the editor gets.

use std::fs::File;
use std::io::{self, Write};

use crossterm::{
    event::{
        DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags, read,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement},
};

fn main() -> io::Result<()> {
    let mut log = File::create("keylog.txt")?;
    let enhanced = supports_keyboard_enhancement().unwrap_or(false);
    writeln!(log, "keyboard_enhancement_supported: {enhanced}")?;
    for var in [
        "TERM",
        "TERM_PROGRAM",
        "TERM_PROGRAM_VERSION",
        "TMUX",
        "WT_SESSION",
        "ConEmuANSI",
    ] {
        writeln!(log, "{var}={:?}", std::env::var(var).ok())?;
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnableBracketedPaste)?;
    if enhanced {
        execute!(
            stdout,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            )
        )?;
    }
    print!("Type the keys that fail, then press Esc to finish.\r\n");
    stdout.flush()?;

    let result = (|| -> io::Result<()> {
        loop {
            let event = read()?;
            writeln!(log, "{event:?}")?;
            print!("{event:?}\r\n");
            stdout.flush()?;
            if let Event::Key(key) = event
                && key.code == KeyCode::Esc
                && key.kind == KeyEventKind::Press
            {
                return Ok(());
            }
        }
    })();

    if enhanced {
        let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(stdout, DisableBracketedPaste);
    disable_raw_mode()?;
    result
}
