//! Paste normalization and a burst heuristic for terminals without bracketed paste.

use std::time::{Duration, Instant};

/// Keep newlines; turn CRLF/CR into `\n`; drop other control characters.
pub fn normalize_paste(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    let _ = chars.next();
                }
                out.push('\n');
            }
            '\n' => out.push('\n'),
            '\t' => out.push('\t'),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Detects a rapid key flood (Windows conhost paste without bracketed paste).
/// Enter inside an active burst becomes a newline instead of Submit.
#[derive(Debug, Default, Clone)]
pub struct PasteBurst {
    last: Option<Instant>,
    /// Keys seen in the current burst, including the one just noted.
    count: u32,
}

impl PasteBurst {
    const WINDOW: Duration = Duration::from_millis(30);

    /// Record a key. Returns true when this key continues an active burst.
    pub fn note(&mut self) -> bool {
        let now = Instant::now();
        match self.last {
            Some(prev) if now.duration_since(prev) <= Self::WINDOW => {
                self.count = self.count.saturating_add(1);
                self.last = Some(now);
                true
            }
            _ => {
                self.count = 1;
                self.last = Some(now);
                false
            }
        }
    }

    /// After [`note`] on Enter: true when prior keys in this burst make Enter a newline.
    pub fn enter_as_newline(&self) -> bool {
        self.count >= 2
    }

    pub fn reset(&mut self) {
        self.last = None;
        self.count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::{PasteBurst, normalize_paste};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn normalize_paste_keeps_newlines_and_crlf() {
        assert_eq!(normalize_paste("a\r\nb\rc\n"), "a\nb\nc\n");
        assert_eq!(normalize_paste("x\u{0000}y"), "xy");
        assert_eq!(normalize_paste("line\tkept"), "line\tkept");
    }

    #[test]
    fn burst_enter_is_newline_only_inside_a_flood() {
        let mut burst = PasteBurst::default();
        assert!(!burst.note()); // first key
        assert!(!burst.enter_as_newline());
        // Immediate follow-ups stay inside the window.
        assert!(burst.note());
        assert!(burst.note());
        assert!(burst.enter_as_newline());
        // Idle then lone Enter.
        thread::sleep(Duration::from_millis(40));
        assert!(!burst.note());
        assert!(!burst.enter_as_newline());
    }
}
