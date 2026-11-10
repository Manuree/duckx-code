//! Where a one-shot message is shown: the snackbar, or the status line.
//!
//! Code sets [`App::message`] as before. Once per main-loop pass,
//! [`App::flush_notice`] moves a new transient message (a result, an error, a
//! warning, an info line) into the snackbar, which stays for a few seconds
//! whatever the user types next. Text that describes the state the editor is in
//! right now stays on the status line and is cleared by the next action:
//! progress (`Saving a.rs…`), the FIND / REPLACE position (`3 of 12`), FILE mode
//! marks and key hints, and anything waiting on the next key (`Press Enter
//! again…`, `F6 to answer`). While the command bar, FIND, or REPLACE is open,
//! its messages stay inline beside the input. See `DESIGN.md` §5, "Notices".

use std::time::{Duration, Instant};

use super::{App, Snackbar};

/// How a snackbar notice is colored and how long it stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    /// Something finished: saved, copied, replaced, deleted.
    Success,
    /// Nothing happened, or it was refused: `Nothing to replace`, `read-only`.
    Warning,
    /// Something failed: `Cannot save …`, `Unknown command`.
    Error,
}

impl NoticeKind {
    /// The snackbar title for a notice of this kind.
    pub fn title(self) -> &'static str {
        match self {
            NoticeKind::Info => " Info ",
            NoticeKind::Success => " Done ",
            NoticeKind::Warning => " Note ",
            NoticeKind::Error => " Error ",
        }
    }

    pub fn duration(self) -> Duration {
        Duration::from_secs(match self {
            NoticeKind::Info | NoticeKind::Success => 4,
            NoticeKind::Warning => 6,
            NoticeKind::Error => 8,
        })
    }
}

/// Where a message goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// State of the moment: stays on the status line until the next action.
    Status,
    Snackbar(NoticeKind),
}

/// Decides where `text` is shown. Everything is a snackbar notice except the
/// status-line cases listed in the module docs.
pub fn placement(text: &str) -> Placement {
    let text = text.trim();
    let status = text.ends_with('…')
        || is_position(text)
        || text.starts_with("Press Enter again")
        || text.ends_with("F6 to answer")
        || text.starts_with("space marks")
        || text.starts_with("Marked ")
        || text.starts_with("Unmarked ");
    if status {
        return Placement::Status;
    }
    Placement::Snackbar(kind_of(text))
}

/// `3 of 12` and `Replaced 1, 3 of 12`: where FIND / REPLACE is.
fn is_position(text: &str) -> bool {
    let rest = text
        .strip_prefix("Replaced ")
        .and_then(|rest| rest.split_once(", ").map(|(_, tail)| tail))
        .unwrap_or(text);
    let mut parts = rest.split(" of ");
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(a), Some(b), None)
            if !a.is_empty() && !b.is_empty()
                && a.chars().all(|c| c.is_ascii_digit())
                && b.chars().all(|c| c.is_ascii_digit())
    )
}

fn kind_of(text: &str) -> NoticeKind {
    let lower = text.to_lowercase();
    let starts = |words: &[&str]| words.iter().any(|word| text.starts_with(word));
    if starts(&[
        "Cannot",
        "Could not",
        "Couldn't",
        "Failed",
        "Unknown",
        "Error",
    ]) || lower.contains("failed")
        || lower.contains("error:")
        || lower.contains("denied")
    {
        return NoticeKind::Error;
    }
    if starts(&[
        "Saved",
        "Reloaded",
        "Opened",
        "Copied",
        "Cut ",
        "Deleted",
        "Renamed",
        "Created",
        "Moved",
        "Replaced",
        "Undid",
        "Converted",
        "Updated",
        "Restored",
        "Forgot",
        "Added",
        "Encoding set",
    ]) {
        return if lower.contains("could not") {
            NoticeKind::Warning
        } else {
            NoticeKind::Success
        };
    }
    if starts(&[
        "Nothing",
        "No ",
        "Not found",
        "Still ",
        "Only ",
        "Set a model",
        "Select ",
    ]) || lower.contains("cancelled")
        || lower.contains("read-only")
        || lower.contains(" is off")
        || lower.contains("not running")
        || lower.contains("already")
        || lower.contains("unsaved")
        || lower.contains("does not support")
        || lower.contains("not utf-8")
        || lower.contains("not loaded")
    {
        return NoticeKind::Warning;
    }
    NoticeKind::Info
}

impl App {
    /// Moves a new transient [`App::message`] into the snackbar. Returns `true`
    /// when the screen changed. The main loop calls this before every draw.
    pub fn flush_notice(&mut self) -> bool {
        let Some(text) = self.message.as_deref() else {
            self.notice_seen = None;
            return false;
        };
        if self.notice_seen.as_deref() == Some(text) {
            return false;
        }
        self.notice_seen = Some(text.to_string());
        if self.input_line_open() {
            return false;
        }
        let Placement::Snackbar(kind) = placement(text) else {
            return false;
        };
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        self.snackbar = Some(Snackbar {
            text,
            kind,
            title: kind.title(),
            assistant: false,
            until: Instant::now() + kind.duration(),
        });
        true
    }

    /// The message the status line draws: state-of-the-moment text, or any
    /// message while an input line is open (it sits beside the input).
    pub fn status_message(&self) -> Option<&str> {
        let text = self.message.as_deref()?;
        (self.input_line_open() || placement(text) == Placement::Status).then_some(text)
    }

    /// The command bar, FIND, or REPLACE is open: its messages answer what is
    /// being typed, so they stay beside the input.
    fn input_line_open(&self) -> bool {
        self.command_bar.is_some() || self.find_mode.is_some() || self.replace_mode.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::SemanticAction;
    use crate::buffer::Buffer;

    #[test]
    fn results_and_errors_go_to_the_snackbar_and_state_stays_on_the_status_line() {
        let snack = |text| match placement(text) {
            Placement::Snackbar(kind) => Some(kind),
            Placement::Status => None,
        };
        assert_eq!(
            snack(r"Saved E:\Work\Duckx Canvas\PRODUCT.md"),
            Some(NoticeKind::Success)
        );
        assert_eq!(snack("Copied 12 characters"), Some(NoticeKind::Success));
        assert_eq!(snack("Replaced 4"), Some(NoticeKind::Success));
        assert_eq!(
            snack("Cannot save a.rs: Permission denied (os error 5)"),
            Some(NoticeKind::Error)
        );
        assert_eq!(snack("Unknown command: frob"), Some(NoticeKind::Error));
        assert_eq!(snack("Nothing to replace"), Some(NoticeKind::Warning));
        assert_eq!(snack("The preview is read-only"), Some(NoticeKind::Warning));
        assert_eq!(
            snack("Select the text to convert first"),
            Some(NoticeKind::Warning)
        );
        assert_eq!(
            snack("Converted to Thai (Kedmanee); keyboard switched to Thai (Kedmanee)"),
            Some(NoticeKind::Success)
        );
        assert_eq!(
            snack("Converted to Thai (Kedmanee); could not switch the keyboard"),
            Some(NoticeKind::Warning)
        );
        assert_eq!(snack("3 matches in 2 files"), Some(NoticeKind::Info));
        assert_eq!(snack("Focus on"), Some(NoticeKind::Info));

        for status in [
            "Saving PRODUCT.md…",
            "Searching…",
            "3 of 12",
            "Replaced 1, 3 of 12",
            "Press Enter again to delete Lab",
            "The assistant asks to run write_file. F6 to answer",
            "space marks   shift+↑↓ covers   shift+space commands   esc clears",
            "Marked a.rs (2)",
        ] {
            assert_eq!(placement(status), Placement::Status, "{status}");
        }
    }

    #[test]
    fn a_new_message_is_flushed_once_and_outlives_the_next_key() {
        let mut app = App::new(Buffer::from_text("abc"), None);
        app.message = Some("Copied 3 characters".into());
        assert!(app.flush_notice());
        assert!(!app.flush_notice(), "only once");
        assert_eq!(app.status_message(), None, "not on the status line");
        let bar = app.snackbar.as_ref().unwrap();
        assert_eq!(bar.text, "Copied 3 characters");
        assert_eq!(bar.kind, NoticeKind::Success);
        assert!(!bar.assistant);
        app.handle(SemanticAction::InsertChar('x'));
        app.flush_notice();
        assert!(app.snackbar.is_some(), "typing does not hide it");
        app.snackbar.as_mut().unwrap().until = Instant::now();
        app.expire_snackbar();
        assert!(app.snackbar.is_none());

        // The same text again after another action is a new notice.
        app.message = Some("Copied 3 characters".into());
        assert!(app.flush_notice());
    }

    #[test]
    fn progress_stays_on_the_status_line_and_the_command_bar_keeps_its_own() {
        let mut app = App::new(Buffer::from_text(""), None);
        app.message = Some("Saving a.rs…".into());
        assert!(!app.flush_notice());
        assert!(app.snackbar.is_none());
        assert_eq!(app.status_message(), Some("Saving a.rs…"));

        app.handle(SemanticAction::OpenCommandBar);
        app.message = Some("Unknown command: frob".into());
        assert!(!app.flush_notice(), "the bar shows it beside the input");
        assert_eq!(app.status_message(), Some("Unknown command: frob"));
    }

    #[test]
    fn convert_keyboard_notices_use_the_snackbar() {
        let mut app = App::new(Buffer::from_text(""), None);
        app.handle(SemanticAction::ConvertKeyboard);
        assert!(app.flush_notice());
        let bar = app.snackbar.as_ref().unwrap();
        assert_eq!(bar.text, "Select the text to convert first");
        assert_eq!(bar.kind, NoticeKind::Warning);
        assert_eq!(app.status_message(), None);
    }

    #[test]
    fn an_error_notice_is_red_and_stays_longer() {
        let mut app = App::new(Buffer::from_text(""), None);
        app.message = Some("Cannot save a.rs: Permission denied".into());
        app.flush_notice();
        let bar = app.snackbar.as_ref().unwrap();
        assert_eq!(bar.kind, NoticeKind::Error);
        assert_eq!(bar.title, " Error ");
        assert!(bar.until > Instant::now() + Duration::from_secs(7));
    }
}
