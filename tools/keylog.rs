//! Records raw crossterm events to `keylog.txt` to diagnose keyboard input issues.
//! Run with `cargo run --example keylog`, type the problem keys, then press Esc.
//! It asks the terminal for keys the same way the editor does (kitty protocol, else
//! modifyOtherKeys; see `duckx_code::term_keys`), so what it logs is what the editor gets.

use std::fs::File;
use std::io::{self, Write};

use duckx_code::term_keys;

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
    term_keys::install();
    let enhanced = supports_keyboard_enhancement().unwrap_or(false);
    let term = std::env::var("TERM").ok();
    let modify_other_keys = term_keys::wants_modify_other_keys(enhanced, term.as_deref());
    writeln!(log, "keyboard_enhancement_supported: {enhanced}")?;
    writeln!(log, "modify_other_keys_requested: {modify_other_keys}")?;
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
    } else if modify_other_keys {
        stdout.write_all(term_keys::MODIFY_OTHER_KEYS_ON.as_bytes())?;
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
    } else if modify_other_keys {
        let _ = stdout.write_all(term_keys::MODIFY_OTHER_KEYS_OFF.as_bytes());
    }
    let _ = execute!(stdout, DisableBracketedPaste);
    disable_raw_mode()?;
    result
}
