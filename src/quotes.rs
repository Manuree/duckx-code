//! Finding the quoted text around the cursor for `select-inside-quotes`.

use ropey::Rope;

/// The quote characters `select-inside-quotes` pairs.
pub const QUOTES: [char; 3] = ['"', '\'', '`'];

/// A quoted string in char indices: `inner` is the text between the marks, `outer` includes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuoteSpan {
    pub inner: (usize, usize),
    pub outer: (usize, usize),
}

/// Splits a string literal node spanning chars `start..end` into its quote marks and contents.
/// Prefixes such as `b"`, `r#"`, `f'` are skipped; a run of three marks counts as one triple quote.
pub fn from_literal(rope: &Rope, start: usize, end: usize) -> Option<QuoteSpan> {
    let chars: Vec<char> = rope.slice(start..end).chars().collect();
    let open = chars.iter().position(|ch| QUOTES.contains(ch))?;
    let quote = chars[open];
    let close = chars.iter().rposition(|&ch| ch == quote)?;
    if close <= open {
        return None;
    }
    let open_run = chars[open..].iter().take_while(|&&ch| ch == quote).count();
    let close_run = chars[..=close]
        .iter()
        .rev()
        .take_while(|&&ch| ch == quote)
        .count();
    let width = if open_run >= 3 && close_run >= 3 && close - open >= 5 {
        3
    } else {
        1
    };
    Some(QuoteSpan {
        inner: (start + open + width, start + close + 1 - width),
        outer: (start + open, start + close + 1),
    })
}

/// Every quoted span on `at`'s line that contains `at`, innermost first, pairing
/// unescaped marks of each quote kind left to right. A cursor on a mark includes
/// that mark's pair. Backticks are included with `"` and `'`.
pub fn enclosing_on_line(rope: &Rope, at: usize) -> Vec<QuoteSpan> {
    let at = at.min(rope.len_chars());
    let line = rope.char_to_line(at);
    let line_start = rope.line_to_char(line);
    let chars: Vec<char> = rope
        .line(line)
        .chars()
        .take_while(|&ch| ch != '\n' && ch != '\r')
        .collect();
    let col = at - line_start;
    let mut found = Vec::new();
    for quote in QUOTES {
        let marks: Vec<usize> = (0..chars.len())
            .filter(|&i| chars[i] == quote && !escaped(&chars, i))
            .collect();
        for &[open, close] in marks.as_chunks::<2>().0 {
            if open <= col && col <= close {
                found.push((open, close));
            }
        }
    }
    found.sort_by_key(|&(open, close)| (close - open, open));
    found
        .into_iter()
        .map(|(open, close)| QuoteSpan {
            inner: (line_start + open + 1, line_start + close),
            outer: (line_start + open, line_start + close + 1),
        })
        .collect()
}

/// The innermost quoted span on `at`'s line that contains `at`, pairing unescaped marks of each
/// quote kind left to right. A cursor on a mark selects that mark's pair.
pub fn scan_line(rope: &Rope, at: usize) -> Option<QuoteSpan> {
    enclosing_on_line(rope, at).into_iter().next()
}

/// Whether the char at `i` follows an odd run of backslashes.
fn escaped(chars: &[char], i: usize) -> bool {
    chars[..i]
        .iter()
        .rev()
        .take_while(|&&ch| ch == '\\')
        .count()
        % 2
        == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` with `|` marking the cursor; returns the inner text the scan selects.
    fn inner(text: &str) -> Option<String> {
        let at = text.find('|').unwrap();
        let rope = Rope::from_str(&text.replacen('|', "", 1));
        let at = rope.byte_to_char(at);
        scan_line(&rope, at).map(|span| rope.slice(span.inner.0..span.inner.1).to_string())
    }

    #[test]
    fn double_quotes() {
        assert_eq!(
            inner("println!(\"Hello,| world!\");").as_deref(),
            Some("Hello, world!")
        );
    }

    #[test]
    fn single_quotes() {
        assert_eq!(inner("x = 'ab|c' + y").as_deref(), Some("abc"));
    }

    #[test]
    fn backticks() {
        assert_eq!(
            inner("run `cargo| test` now").as_deref(),
            Some("cargo test")
        );
    }

    #[test]
    fn escaped_quotes_do_not_pair() {
        assert_eq!(
            inner(r#"s = "say \"hi|\" ok";"#).as_deref(),
            Some(r#"say \"hi\" ok"#)
        );
        assert_eq!(inner(r#"s = "a\\" + "b|";"#).as_deref(), Some("b"));
    }

    #[test]
    fn cursor_on_a_quote_mark_uses_its_pair() {
        assert_eq!(inner("f(|\"one\", \"two\")").as_deref(), Some("one"));
        assert_eq!(inner("f(\"one|\", \"two\")").as_deref(), Some("one"));
        assert_eq!(inner("f(\"one\", \"two|\")").as_deref(), Some("two"));
    }

    #[test]
    fn no_enclosing_quotes() {
        assert_eq!(inner("let x| = 1;"), None);
        assert_eq!(inner("a(\"one\") |b(\"two\")"), None);
        assert_eq!(inner("only \"one| mark"), None);
    }

    #[test]
    fn multiple_strings_on_one_line() {
        assert_eq!(inner("[\"a\", \"b|b\", \"c\"]").as_deref(), Some("bb"));
        assert_eq!(inner("[\"a\", \"bb\", \"|c\"]").as_deref(), Some("c"));
    }

    #[test]
    fn innermost_kind_wins() {
        assert_eq!(inner("\"it's |here' ok\"").as_deref(), Some("s here"));
        assert_eq!(inner("\"say `x|y` now\"").as_deref(), Some("xy"));
    }

    #[test]
    fn empty_string() {
        assert_eq!(inner("x = \"|\";").as_deref(), Some(""));
    }

    #[test]
    fn literal_nodes_strip_prefixes_and_triple_quotes() {
        let check = |text: &str, want: &str, outer: &str| {
            let rope = Rope::from_str(text);
            let span = from_literal(&rope, 0, rope.len_chars()).unwrap();
            assert_eq!(rope.slice(span.inner.0..span.inner.1).to_string(), want);
            assert_eq!(rope.slice(span.outer.0..span.outer.1).to_string(), outer);
        };
        check("\"Hello\"", "Hello", "\"Hello\"");
        check("b\"bytes\"", "bytes", "\"bytes\"");
        check("r#\"raw \"q\" \"#", "raw \"q\" ", "\"raw \"q\" \"");
        check(
            "\"\"\"doc\nstring\"\"\"",
            "doc\nstring",
            "\"\"\"doc\nstring\"\"\"",
        );
        check("\"\"", "", "\"\"");
        check("'x'", "x", "'x'");
    }
}
