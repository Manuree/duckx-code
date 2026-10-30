//! Diagnostics per file, kept aligned with the buffer while the user types.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::buffer::ByteEdit;

use super::protocol::{Diagnostic, Severity};

/// One replaced byte range: `start..old_end` of the old text is `start..new_end` of the new text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub old_end: usize,
    pub new_end: usize,
}

impl Span {
    /// Where byte `pos` of the old text is in the new text. `end` picks the end of
    /// the replacement for positions inside the replaced range.
    pub(crate) fn map(self, pos: usize, end: bool) -> usize {
        if pos <= self.start {
            pos
        } else if pos >= self.old_end {
            pos - self.old_end + self.new_end
        } else if end {
            self.new_end
        } else {
            self.start
        }
    }

    /// Change in total length.
    pub fn delta(self) -> isize {
        self.new_end as isize - self.old_end as isize
    }
}

/// Folds consecutive edits (each in the text left by the one before) into one span
/// of the text before the first edit. `None` when there are no edits.
pub fn merge_edits(edits: &[ByteEdit]) -> Option<Span> {
    let (first, rest) = edits.split_first()?;
    let mut span = Span {
        start: first.start_byte,
        old_end: first.old_end_byte,
        new_end: first.new_end_byte,
    };
    for edit in rest {
        if edit.old_end_byte >= span.new_end {
            span.old_end += edit.old_end_byte - span.new_end;
            span.new_end = edit.new_end_byte;
        } else {
            span.new_end = span.new_end + edit.new_end_byte - edit.old_end_byte;
        }
        span.start = span.start.min(edit.start_byte);
    }
    Some(span)
}

#[derive(Debug, Default)]
struct Layers {
    /// `textDocument/publishDiagnostics`, including `cargo check` after a save.
    pushed: Vec<Diagnostic>,
    /// `textDocument/diagnostic`. rust-analyzer's native checks live here and are often empty.
    pulled: Vec<Diagnostic>,
    shown: Vec<Diagnostic>,
}

impl Layers {
    fn store(&mut self, items: Vec<Diagnostic>, pulled: bool) {
        if pulled {
            self.pulled = items;
        } else {
            self.pushed = items;
        }
        self.shown.clear();
        self.shown.extend(self.pushed.iter().cloned());
        self.shown.extend(self.pulled.iter().cloned());
        self.shown.sort_by_key(|item| (item.start, item.severity));
    }
}

#[derive(Debug, Default)]
pub struct Diagnostics {
    files: HashMap<PathBuf, Layers>,
}

impl Diagnostics {
    /// Replaces the diagnostics `publishDiagnostics` reported for `path`.
    pub fn set(&mut self, path: PathBuf, items: Vec<Diagnostic>) {
        self.set_layer(path, items, false);
    }

    /// Replaces one source. An empty pull does not remove type errors that arrived from a save.
    /// It does remove syntax errors from that save: the pull is the parse of the unsaved buffer.
    pub fn set_layer(&mut self, path: PathBuf, items: Vec<Diagnostic>, pulled: bool) {
        let layers = self.files.entry(path.clone()).or_default();
        if pulled {
            layers.pushed.retain(|item| !item.is_syntax());
        }
        layers.store(items, pulled);
        if layers.shown.is_empty() {
            self.files.remove(&path);
        }
    }

    pub fn clear(&mut self, path: &Path) {
        self.files.remove(path);
    }

    pub fn get(&self, path: &Path) -> &[Diagnostic] {
        self.files
            .get(path)
            .map(|layers| layers.shown.as_slice())
            .unwrap_or(&[])
    }

    /// Moves every range in `path` through an edit. A diagnostic whose text the edit
    /// rewrote is dropped, so fixing `print` into `println!` clears that marker now.
    pub fn shift(&mut self, path: &Path, span: Span) {
        if let Some(layers) = self.files.get_mut(path) {
            shift_all(&mut layers.pushed, span);
            shift_all(&mut layers.pulled, span);
            let pushed = std::mem::take(&mut layers.pushed);
            layers.store(pushed, false);
            if layers.shown.is_empty() {
                self.files.remove(path);
            }
        }
    }

    /// Errors, warnings, infos, and hints in `path`.
    pub fn counts(&self, path: &Path) -> [usize; 4] {
        let mut counts = [0; 4];
        for item in self.get(path) {
            counts[item.severity as usize] += 1;
        }
        counts
    }
}

/// The edit replaced this range, or inserted text at either edge of it.
fn rewritten(span: Span, start: usize, end: usize) -> bool {
    if span.start == span.old_end {
        span.start >= start && span.start <= end
    } else if start == end {
        start >= span.start && start < span.old_end
    } else {
        span.start < end && start < span.old_end
    }
}

/// Slides ranges through `span`. Drops one when the edit rewrote its text, including
/// an insertion at either edge. An edit strictly before or after only moves it.
pub fn shift_all(items: &mut Vec<Diagnostic>, span: Span) {
    items.retain(|item| !rewritten(span, item.start, item.end));
    for item in items {
        item.start = span.map(item.start, false);
        item.end = span.map(item.end, true).max(item.start);
    }
}

/// The diagnostic `F8` moves to: the first that starts after `byte`, wrapping to the first.
/// `Shift + F8` (`forward == false`) is the last that starts before `byte`, wrapping to the last.
pub fn adjacent(items: &[Diagnostic], byte: usize, forward: bool) -> Option<&Diagnostic> {
    if forward {
        items
            .iter()
            .find(|item| item.start > byte)
            .or_else(|| items.first())
    } else {
        items
            .iter()
            .rev()
            .find(|item| item.start < byte)
            .or_else(|| items.last())
    }
}

/// The most severe diagnostic that covers `byte`, or else the most severe on bytes `line_start..line_end`.
pub fn at_cursor(
    items: &[Diagnostic],
    byte: usize,
    line_start: usize,
    line_end: usize,
) -> Option<&Diagnostic> {
    let touches =
        |item: &&Diagnostic| item.start <= line_end && item.end.max(item.start + 1) > line_start;
    let covering = items
        .iter()
        .filter(touches)
        .filter(|item| item.start <= byte && byte <= item.end)
        .min_by_key(|item| item.severity);
    covering.or_else(|| {
        items
            .iter()
            .filter(touches)
            .min_by_key(|item| item.severity)
    })
}

/// Most severe level per line, for the gutter.
pub fn worst(items: &[Diagnostic]) -> Option<Severity> {
    items.iter().map(|item| item.severity).min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Point;

    fn edit(start: usize, old_end: usize, new_end: usize) -> ByteEdit {
        let point = Point { row: 0, column: 0 };
        ByteEdit {
            start_byte: start,
            old_end_byte: old_end,
            new_end_byte: new_end,
            start: point,
            old_end: point,
            new_end: point,
        }
    }

    fn diag(start: usize, end: usize, severity: Severity) -> Diagnostic {
        Diagnostic {
            start,
            end,
            severity,
            message: format!("{start}..{end}"),
            source: None,
            code: None,
        }
    }

    /// Applies `(start, old_end, replacement)` edits one after another and returns the
    /// `ByteEdit` log, the way `Buffer` records it.
    fn replay(text: &str, steps: &[(usize, usize, &str)]) -> (String, Vec<ByteEdit>) {
        let mut text = text.to_string();
        let mut edits = Vec::new();
        for &(start, old_end, insert) in steps {
            text.replace_range(start..old_end, insert);
            edits.push(edit(start, old_end, start + insert.len()));
        }
        (text, edits)
    }

    fn apply(text: &str, span: Span, after: &str) -> String {
        let mut out = text[..span.start].to_string();
        out.push_str(&after[span.start..span.new_end]);
        out.push_str(&text[span.old_end..]);
        out
    }

    #[test]
    fn merged_span_rebuilds_the_new_text_from_the_old() {
        let old = "hello world, goodbye";
        let cases: &[&[(usize, usize, &str)]] = &[
            &[(5, 5, "!"), (6, 6, "!")],
            &[(0, 5, "HI"), (10, 15, "")],
            &[(10, 11, ""), (2, 3, "xyz")],
            &[(3, 3, "abc"), (4, 5, ""), (0, 1, "")],
            &[(19, 20, "Y"), (0, 0, ">")],
        ];
        for steps in cases {
            let (new, edits) = replay(old, steps);
            let span = merge_edits(&edits).unwrap();
            assert_eq!(apply(old, span, &new), new, "steps {steps:?}");
            assert_eq!(
                old.len() as isize + span.delta(),
                new.len() as isize,
                "steps {steps:?}"
            );
        }
    }

    #[test]
    fn typing_before_a_diagnostic_moves_it_right() {
        let mut items = vec![diag(10, 14, Severity::Error)];
        shift_all(
            &mut items,
            Span {
                start: 2,
                old_end: 2,
                new_end: 5,
            },
        );
        assert_eq!((items[0].start, items[0].end), (13, 17));
    }

    #[test]
    fn deleting_across_a_diagnostic_drops_it_and_keeps_one_outside() {
        let mut items = vec![diag(10, 14, Severity::Error), diag(1, 2, Severity::Hint)];
        shift_all(
            &mut items,
            Span {
                start: 8,
                old_end: 20,
                new_end: 8,
            },
        );
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].start, items[0].end), (1, 2));
    }

    #[test]
    fn inserting_at_the_end_of_a_token_drops_that_diagnostic() {
        let mut items = vec![diag(4, 9, Severity::Error), diag(20, 24, Severity::Warning)];
        shift_all(
            &mut items,
            Span {
                start: 9,
                old_end: 9,
                new_end: 12,
            },
        );
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].start, items[0].end), (23, 27));
    }

    #[test]
    fn f8_order_wraps_both_ways() {
        let items = vec![diag(5, 6, Severity::Error), diag(20, 25, Severity::Warning)];
        assert_eq!(adjacent(&items, 0, true).unwrap().start, 5);
        assert_eq!(adjacent(&items, 5, true).unwrap().start, 20);
        assert_eq!(adjacent(&items, 30, true).unwrap().start, 5);
        assert_eq!(adjacent(&items, 20, false).unwrap().start, 5);
        assert_eq!(adjacent(&items, 0, false).unwrap().start, 20);
        assert!(adjacent(&[], 0, true).is_none());
    }

    #[test]
    fn cursor_prefers_the_diagnostic_under_it_then_the_worst_on_the_line() {
        let items = vec![diag(0, 3, Severity::Error), diag(5, 8, Severity::Warning)];
        assert_eq!(
            at_cursor(&items, 6, 0, 10).unwrap().severity,
            Severity::Warning
        );
        assert_eq!(
            at_cursor(&items, 4, 0, 10).unwrap().severity,
            Severity::Error
        );
        assert!(at_cursor(&items, 15, 11, 20).is_none());
    }

    #[test]
    fn store_counts_by_severity_and_drops_empty_sets() {
        let mut store = Diagnostics::default();
        let path = PathBuf::from("a.rs");
        store.set(
            path.clone(),
            vec![
                diag(9, 9, Severity::Warning),
                diag(1, 2, Severity::Error),
                diag(4, 4, Severity::Error),
            ],
        );
        assert_eq!(store.counts(&path), [2, 1, 0, 0]);
        assert_eq!(store.get(&path)[0].start, 1);
        store.set(path.clone(), Vec::new());
        assert!(store.get(&path).is_empty());
    }

    #[test]
    fn an_empty_pull_keeps_diagnostics_from_a_save() {
        let mut store = Diagnostics::default();
        let path = PathBuf::from("a.rs");
        store.set(path.clone(), vec![diag(1, 2, Severity::Error)]);
        store.set_layer(path.clone(), Vec::new(), true);
        assert_eq!(store.get(&path).len(), 1);
        store.set_layer(path.clone(), vec![diag(4, 5, Severity::Warning)], true);
        assert_eq!(store.counts(&path), [1, 1, 0, 0]);
        let insert = Span {
            start: 0,
            old_end: 0,
            new_end: 1,
        };
        store.shift(&path, insert);
        let shown = store.get(&path);
        assert_eq!(shown[0].start, 2);
        assert_eq!(shown[1].start, 5);
        store.set(path.clone(), Vec::new());
        assert_eq!(store.get(&path).len(), 1);
        assert_eq!(store.get(&path)[0].severity, Severity::Warning);
    }

    #[test]
    fn a_live_pull_drops_a_fixed_syntax_error_and_keeps_a_type_error() {
        let mut store = Diagnostics::default();
        let path = PathBuf::from("a.rs");
        let mut syntax = diag(1, 2, Severity::Error);
        syntax.message = "unterminated double quote string".to_string();
        syntax.code = Some("E0765".to_string());
        let mut typed = diag(8, 9, Severity::Error);
        typed.message = "mismatched types".to_string();
        typed.code = Some("E0308".to_string());
        store.set(path.clone(), vec![syntax, typed]);
        store.set_layer(path.clone(), Vec::new(), true);
        let shown = store.get(&path);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].code.as_deref(), Some("E0308"));

        let mut again = diag(1, 4, Severity::Error);
        again.message = "unterminated double quote string".to_string();
        again.code = Some("E0765".to_string());
        let mut still_typed = diag(8, 9, Severity::Error);
        still_typed.message = "mismatched types".to_string();
        still_typed.code = Some("E0308".to_string());
        store.set(path.clone(), vec![again, still_typed]);
        let mut live = diag(1, 4, Severity::Error);
        live.message = "Syntax Error: Missing trailing `\"` symbol to terminate the string literal"
            .to_string();
        live.code = Some("syntax-error".to_string());
        store.set_layer(path.clone(), vec![live], true);
        let shown = store.get(&path);
        assert_eq!(shown.len(), 2);
        assert!(
            shown
                .iter()
                .all(|item| item.code.as_deref() != Some("E0765"))
        );
        assert!(
            shown
                .iter()
                .any(|item| item.code.as_deref() == Some("syntax-error"))
        );
        assert!(
            shown
                .iter()
                .any(|item| item.code.as_deref() == Some("E0308"))
        );
    }
}
