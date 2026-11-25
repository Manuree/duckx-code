//! Bracket matching (highlight the pair at the cursor) and bracket pair colors.
//!
//! Only `()`, `[]` and `{}` count; `<>` never does. Both features take a
//! `literal` test so brackets inside strings and comments are skipped when the
//! file has a syntax tree (without a tree nothing is skipped).
//!
//! The matching rules follow Neovim's `matchparen` (`runtime/lua/nvim/matchparen.lua`):
//! the character under the cursor first, then the one before it (Neovim does
//! that in Insert mode, and this editor is always typing); brackets in strings
//! and comments are skipped, and a bracket in one pairs only with brackets in
//! strings or comments (Neovim stops skipping there instead); the
//! search is bounded (Neovim: visible lines and a 300 ms timeout; here
//! [`MATCH_SCAN_CHARS`]); and nothing is shown when the partner is off screen.

use crate::quotes::QuoteSpan;
use ropey::Rope;

/// Most characters the matching search reads from the cursor bracket.
pub const MATCH_SCAN_CHARS: usize = 50_000;
/// Lines read above the window to learn the nesting depth for colors.
pub const COLOR_LOOKBACK_LINES: usize = 500;
/// Lines read below the window to learn whether an opener is ever closed.
pub const COLOR_LOOKAHEAD_LINES: usize = 500;
/// Most characters one color pass reads, lookback and lookahead included.
pub const COLOR_SCAN_CHARS: usize = 200_000;

pub fn is_bracket(ch: char) -> bool {
    matches!(ch, '(' | ')' | '[' | ']' | '{' | '}')
}

fn closer_of(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

fn opener_of(close: char) -> Option<char> {
    match close {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        _ => None,
    }
}

/// What the matching search found for the bracket at the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Partner {
    /// Char index of the paired bracket.
    Found(usize),
    /// The search reached the start or end of the file without a pair.
    Unmatched,
    /// The search stopped at [`MATCH_SCAN_CHARS`]; nothing is shown.
    Unknown,
}

/// The cursor's bracket (char index) and its partner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BracketMatch {
    pub at: usize,
    pub partner: Partner,
    /// The cursor's bracket is in a string or comment. An unmatched one there is not an error.
    pub in_literal: bool,
}

/// The bracket under `head`, else the one just before it.
pub fn bracket_at_cursor(rope: &Rope, head: usize) -> Option<usize> {
    if head < rope.len_chars() && is_bracket(rope.char(head)) {
        return Some(head);
    }
    (head > 0 && head <= rope.len_chars() && is_bracket(rope.char(head - 1))).then(|| head - 1)
}

/// Match the bracket at the cursor. `tree_pair` is a syntax tree's answer (it may
/// know a pair the scan cannot see); `literal(i)` tells whether the bracket at char
/// `i` is inside a string or comment.
pub fn match_at_cursor(
    rope: &Rope,
    head: usize,
    tree_pair: &dyn Fn(usize) -> Option<usize>,
    literal: &dyn Fn(usize) -> bool,
    limit: usize,
) -> Option<BracketMatch> {
    let at = bracket_at_cursor(rope, head)?;
    let in_literal = literal(at);
    if !in_literal
        && let Some(mate) = tree_pair(at)
        && mate < rope.len_chars()
        && is_bracket(rope.char(mate))
    {
        return Some(BracketMatch {
            at,
            partner: Partner::Found(mate),
            in_literal,
        });
    }
    // Neovim pairs a bracket in a string or comment without skipping anything;
    // here it pairs only with brackets that are in a string or comment too.
    let outside = |at: usize| !literal(at);
    let skip: &dyn Fn(usize) -> bool = if in_literal { &outside } else { literal };
    let partner = scan_partner(rope, at, skip, limit);
    Some(BracketMatch {
        at,
        partner,
        in_literal,
    })
}

/// Scan for the partner of the bracket at `at`, counting nesting. A closer of
/// the wrong kind closes back to its own opener if one is open (the brackets in
/// between are unclosed); a stray one is ignored.
pub fn scan_partner(rope: &Rope, at: usize, skip: &dyn Fn(usize) -> bool, limit: usize) -> Partner {
    if at >= rope.len_chars() {
        return Partner::Unknown;
    }
    let origin = rope.char(at);
    if let Some(close) = closer_of(origin) {
        // Stack of closers we wait for; index 0 is the origin's.
        let mut stack = vec![close];
        for (read, ch) in rope.chars_at(at + 1).enumerate() {
            if read >= limit {
                return Partner::Unknown;
            }
            let index = at + 1 + read;
            if is_bracket(ch) && !skip(index) {
                if let Some(next) = closer_of(ch) {
                    stack.push(next);
                } else if let Some(depth) = stack.iter().rposition(|want| *want == ch) {
                    stack.truncate(depth);
                    if stack.is_empty() {
                        return Partner::Found(index);
                    }
                }
            }
        }
        Partner::Unmatched
    } else if let Some(open) = opener_of(origin) {
        let mut stack = vec![open];
        let mut index = at;
        let mut chars = rope.chars_at(at);
        let mut read = 0;
        while let Some(ch) = chars.prev() {
            if read >= limit {
                return Partner::Unknown;
            }
            read += 1;
            index -= 1;
            if is_bracket(ch) && !skip(index) {
                if let Some(next) = opener_of(ch) {
                    stack.push(next);
                } else if let Some(depth) = stack.iter().rposition(|want| *want == ch) {
                    stack.truncate(depth);
                    if stack.is_empty() {
                        return Partner::Found(index);
                    }
                }
            }
        }
        Partner::Unmatched
    } else {
        Partner::Unknown
    }
}

/// The nearest `open`/`close` pair around chars `start..end`, as the opener's and the
/// closer's char indices, for `select-inside-parens` and its siblings. Only that bracket
/// kind nests. With an empty range, a cursor on `open` or `close` takes that bracket's own
/// pair; a range that is exactly one pair (marks included) returns that pair.
/// `counts(i)` says whether the bracket at char `i` takes part (a string or comment
/// bracket is left out by the caller); the pair must close at or after `end`.
pub fn enclosing_pair(
    rope: &Rope,
    start: usize,
    end: usize,
    open: char,
    close: char,
    counts: &dyn Fn(usize) -> bool,
) -> Option<(usize, usize)> {
    let len = rope.len_chars();
    let (start, end) = (start.min(len), end.min(len));
    let is = |at: usize, ch: char| at < len && rope.char(at) == ch && counts(at);
    let closer = |from: usize| closer_from(rope, from, open, close, counts);
    if start == end {
        if is(start, open)
            && let Some(mate) = closer(start)
        {
            return Some((start, mate));
        }
        if is(start, close) {
            return opener_before(rope, start, open, close, counts, &|_| true);
        }
    } else if is(start, open) && end > 0 && closer(start) == Some(end - 1) {
        return Some((start, end - 1));
    }
    opener_before(rope, start, open, close, counts, &|mate| mate >= end)
}

/// Walks back from `before` to the first unmatched `open` whose closer passes `fits`.
fn opener_before(
    rope: &Rope,
    before: usize,
    open: char,
    close: char,
    counts: &dyn Fn(usize) -> bool,
    fits: &dyn Fn(usize) -> bool,
) -> Option<(usize, usize)> {
    let mut depth = 0usize;
    let mut chars = rope.chars_at(before);
    let mut at = before;
    while let Some(ch) = chars.prev() {
        at -= 1;
        if ch == close && counts(at) {
            depth += 1;
        } else if ch == open && counts(at) {
            if depth > 0 {
                depth -= 1;
                continue;
            }
            let mate = closer_from(rope, at, open, close, counts)?;
            if fits(mate) {
                return Some((at, mate));
            }
        }
    }
    None
}

fn closer_from(
    rope: &Rope,
    from: usize,
    open: char,
    close: char,
    counts: &dyn Fn(usize) -> bool,
) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in rope.chars_at(from + 1).enumerate() {
        let at = from + 1 + offset;
        if ch == open && counts(at) {
            depth += 1;
        } else if ch == close && counts(at) {
            if depth == 0 {
                return Some(at);
            }
            depth -= 1;
        }
    }
    None
}

/// Inner and outer char ranges of one enclosing pair for `select-inside`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InsideSpan {
    pub inner: (usize, usize),
    pub outer: (usize, usize),
}

impl InsideSpan {
    fn from_pair(open: usize, close: usize) -> Self {
        Self {
            inner: (open + 1, close),
            outer: (open, close + 1),
        }
    }

    fn from_quote(span: QuoteSpan) -> Self {
        Self {
            inner: span.inner,
            outer: span.outer,
        }
    }
}

/// Every enclosing `""` `''` `` ` ` `` `()` `{}` `[]` pair around char `at`,
/// innermost first, in the order they nest in the text. `<>` is left out: a
/// text scan would treat comparisons as pairs, and this module never pairs them.
///
/// `quotes` are quote spans that already contain `at` (the syntax tree's string
/// node, or every unescaped pair on the current line). `literal_of` is the
/// string or comment range holding a char, when the caller has a syntax tree.
pub fn enclosing_inside(
    rope: &Rope,
    at: usize,
    quotes: &[QuoteSpan],
    literal_of: &dyn Fn(usize) -> Option<(usize, usize)>,
) -> Vec<InsideSpan> {
    let mut pairs: Vec<InsideSpan> = quotes.iter().copied().map(InsideSpan::from_quote).collect();
    for (open, close) in [('(', ')'), ('{', '}'), ('[', ']')] {
        pairs.extend(pairs_of_kind(rope, at, open, close, literal_of));
    }
    pairs.sort_by_key(|span| {
        let (start, end) = span.outer;
        (end - start, start)
    });
    pairs.dedup();
    pairs
}

/// The next inner range `select-inside` should take. An empty or unmatched
/// `current` selection starts at the innermost pair; a selection that is
/// already the inside (or the delimiters) of a pair expands to the inside of
/// the next outer pair in the actual nest, and wraps back to the innermost
/// after the outermost.
pub fn next_inside(pairs: &[InsideSpan], current: (usize, usize)) -> Option<(usize, usize)> {
    if pairs.is_empty() {
        return None;
    }
    // The last match wins: the inside of `[]` around `(x)` is the same range as
    // the delimiters of `()`, and expand should leave that shared range outward.
    // An empty current still matches an empty inner (`()`), so a caret between
    // empty parens expands to the next outer pair instead of staying put.
    if let Some(i) = pairs
        .iter()
        .rposition(|span| span.inner == current || span.outer == current)
    {
        return Some(pairs.get(i + 1).unwrap_or(&pairs[0]).inner);
    }
    Some(pairs[0].inner)
}

fn pairs_of_kind(
    rope: &Rope,
    at: usize,
    open: char,
    close: char,
    literal_of: &dyn Fn(usize) -> Option<(usize, usize)>,
) -> Vec<InsideSpan> {
    let home = literal_of(at);
    let mut out = Vec::new();
    let mut walk = |home: Option<(usize, usize)>| {
        let counts = |i: usize| literal_of(i) == home;
        let mut found = enclosing_pair(rope, at, at, open, close, &counts);
        while let Some((o, c)) = found {
            let span = InsideSpan::from_pair(o, c);
            if out.last() == Some(&span) {
                break;
            }
            out.push(span);
            found = opener_before(rope, o, open, close, &counts, &|mate| mate > c);
        }
    };
    walk(home);
    if home.is_some() {
        walk(None);
    }
    out
}

/// Color of one bracket in the color pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BracketColor {
    /// Nesting depth from 0; the palette cycles over it.
    Depth(usize),
    /// Never closed, or closes nothing.
    Unmatched,
}

fn view_range(rope: &Rope, start_line: usize, end_line: usize) -> (usize, usize) {
    let lines = rope.len_lines();
    let start_line = start_line.min(lines);
    let end_line = end_line.clamp(start_line, lines);
    (rope.line_to_char(start_line), rope.line_to_char(end_line))
}

/// The chars [`colorize`] reads for lines `start_line..end_line`: the window plus
/// the lookback and lookahead, within [`COLOR_SCAN_CHARS`].
pub fn color_scan_range(rope: &Rope, start_line: usize, end_line: usize) -> (usize, usize) {
    let lines = rope.len_lines();
    let (view_start, view_end) = view_range(rope, start_line, end_line);
    let start_line = start_line.min(lines);
    let end_line = end_line.clamp(start_line, lines);
    let mut scan_start = rope.line_to_char(start_line.saturating_sub(COLOR_LOOKBACK_LINES));
    // Keep the lookback inside the character budget, leaving room for the window.
    let budget = COLOR_SCAN_CHARS.saturating_sub(view_end - view_start) / 2;
    if view_start - scan_start > budget {
        scan_start = view_start - budget;
    }
    let mut scan_end = rope.line_to_char((end_line + COLOR_LOOKAHEAD_LINES).min(lines));
    if scan_end - view_end > budget {
        scan_end = view_end + budget;
    }
    (scan_start, scan_end)
}

/// Colors for the brackets on lines `start_line..end_line`, as `(char index, color)`
/// in order. Reads at most [`COLOR_LOOKBACK_LINES`] above and
/// [`COLOR_LOOKAHEAD_LINES`] below, and [`COLOR_SCAN_CHARS`] in all. When the
/// lookback does not reach the top of the file the depth there is taken as 0,
/// and a closer with nothing open is not called an error (its opener may be
/// further up). Likewise an opener still open where the lookahead stops keeps
/// its depth color.
pub fn colorize(
    rope: &Rope,
    start_line: usize,
    end_line: usize,
    literal: &dyn Fn(usize) -> bool,
) -> Vec<(usize, BracketColor)> {
    let (view_start, view_end) = view_range(rope, start_line, end_line);
    let (scan_start, scan_end) = color_scan_range(rope, start_line, end_line);
    let from_top = scan_start == 0;
    let to_bottom = scan_end == rope.len_chars();

    // (char index, color) for brackets in the window, and the open stack
    // of (closer, slot in `out` or None when before the window, depth).
    let mut out: Vec<(usize, BracketColor)> = Vec::new();
    let mut stack: Vec<(char, Option<usize>)> = Vec::new();
    let mut index = scan_start;
    for ch in rope.chars_at(scan_start) {
        if index >= scan_end {
            break;
        }
        let in_view = index >= view_start && index < view_end;
        if index >= view_end && stack.iter().all(|(_, slot)| slot.is_none()) {
            break;
        }
        if is_bracket(ch) && !literal(index) {
            if let Some(close) = closer_of(ch) {
                let slot = in_view.then(|| {
                    out.push((index, BracketColor::Depth(stack.len())));
                    out.len() - 1
                });
                stack.push((close, slot));
            } else if let Some(depth) = stack.iter().rposition(|(want, _)| *want == ch) {
                // Openers between it and its own opener were never closed.
                for (_, slot) in stack.drain(depth + 1..) {
                    if let Some(slot) = slot {
                        out[slot].1 = BracketColor::Unmatched;
                    }
                }
                stack.pop();
                if in_view {
                    out.push((index, BracketColor::Depth(depth)));
                }
            } else if in_view {
                let color = if from_top {
                    BracketColor::Unmatched
                } else {
                    BracketColor::Depth(0)
                };
                out.push((index, color));
            }
        }
        index += 1;
    }
    if to_bottom && index >= scan_end {
        for (_, slot) in stack {
            if let Some(slot) = slot {
                out[slot].1 = BracketColor::Unmatched;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use BracketColor::{Depth, Unmatched};

    fn none(_: usize) -> Option<usize> {
        None
    }

    fn code(_: usize) -> bool {
        false
    }

    fn at(text: &str, head: usize) -> Option<BracketMatch> {
        match_at_cursor(&Rope::from_str(text), head, &none, &code, MATCH_SCAN_CHARS)
    }

    fn partner(text: &str, head: usize) -> Partner {
        at(text, head).expect("a bracket at the cursor").partner
    }

    #[test]
    fn matches_forward_and_backward_with_nesting() {
        let text = "f(a[b{c}d](e))";
        assert_eq!(partner(text, 1), Partner::Found(13));
        assert_eq!(partner(text, 13), Partner::Found(1));
        assert_eq!(partner(text, 3), Partner::Found(9));
        assert_eq!(partner(text, 5), Partner::Found(7));
        assert_eq!(partner(text, 7), Partner::Found(5));
        assert_eq!(partner(text, 10), Partner::Found(12));
    }

    #[test]
    fn the_char_under_the_cursor_wins_then_the_one_before() {
        // `(` under the cursor at 1; at 2 the `a` is not a bracket, so `(` before it.
        assert_eq!(at("f(a)", 1).unwrap().at, 1);
        assert_eq!(at("f(a)", 2).unwrap().at, 1);
        // `)(`: on the `(`, the `(` under the cursor is used, not the `)` before it.
        assert_eq!(at("()()", 2).unwrap().at, 2);
        // End of the line and end of the file: the bracket before the cursor.
        assert_eq!(partner("f(a)\nx", 4), Partner::Found(1));
        assert_eq!(partner("f(a)", 4), Partner::Found(1));
        assert_eq!(at("abc", 1), None);
        assert_eq!(at("", 0), None);
    }

    #[test]
    fn angle_brackets_are_not_brackets() {
        assert_eq!(at("Vec<u8>", 3), None);
        assert_eq!(at("a < b", 2), None);
        assert_eq!(partner("f(a > b)", 1), Partner::Found(7));
    }

    #[test]
    fn mismatched_kinds_close_back_to_their_own_opener() {
        // `]` closes nothing open in `( ] )`, so `(` still pairs with `)`.
        assert_eq!(partner("( ] )", 0), Partner::Found(4));
        // `)` closes `(` across the unclosed `[`.
        assert_eq!(partner("( [ )", 0), Partner::Found(4));
        assert_eq!(partner("( [ )", 2), Partner::Unmatched);
        assert_eq!(partner("(]", 0), Partner::Unmatched);
        assert_eq!(partner("[)", 1), Partner::Unmatched);
    }

    #[test]
    fn unmatched_at_either_end_of_the_file() {
        assert_eq!(partner("fn f( {", 4), Partner::Unmatched);
        assert_eq!(partner("x) y", 1), Partner::Unmatched);
    }

    #[test]
    fn the_search_stops_at_its_limit() {
        let text = format!("({})", "x".repeat(100));
        let rope = Rope::from_str(&text);
        assert_eq!(
            match_at_cursor(&rope, 0, &none, &code, 50).unwrap().partner,
            Partner::Unknown
        );
        assert_eq!(
            match_at_cursor(&rope, 101, &none, &code, 50)
                .unwrap()
                .partner,
            Partner::Unknown
        );
        assert_eq!(
            match_at_cursor(&rope, 0, &none, &code, 200)
                .unwrap()
                .partner,
            Partner::Found(101)
        );
    }

    #[test]
    fn a_huge_file_is_not_read_to_the_end() {
        let text = format!("(\n{}", "let x = 1;\n".repeat(200_000));
        let rope = Rope::from_str(&text);
        let started = std::time::Instant::now();
        let found = match_at_cursor(&rope, 0, &none, &code, MATCH_SCAN_CHARS).unwrap();
        assert_eq!(found.partner, Partner::Unknown);
        let colors = colorize(&rope, 100_000, 100_050, &code);
        assert!(colors.is_empty());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn thai_wide_characters_and_tabs_count_as_one_char_each() {
        // char indices: \t0 ก1 (2 ข3 [4 ค5 ]6 ง7 )8 界9 {10 }11
        let text = "\tก(ข[ค]ง)界{}";
        assert_eq!(partner(text, 2), Partner::Found(8));
        assert_eq!(partner(text, 8), Partner::Found(2));
        assert_eq!(partner(text, 4), Partner::Found(6));
        assert_eq!(partner(text, 11), Partner::Found(10));
        assert_eq!(
            colorize(&Rope::from_str(text), 0, 1, &code),
            vec![
                (2, Depth(0)),
                (4, Depth(1)),
                (6, Depth(1)),
                (8, Depth(0)),
                (10, Depth(0)),
                (11, Depth(0))
            ]
        );
    }

    #[test]
    fn brackets_in_strings_and_comments_are_skipped_and_pair_among_themselves() {
        // `literal` marks the `)` and `(` inside "…" at 3 and 4.
        let text = "f(\")(\")";
        let rope = Rope::from_str(text);
        let literal = |i: usize| i == 3 || i == 4;
        let found = match_at_cursor(&rope, 1, &none, &literal, MATCH_SCAN_CHARS).unwrap();
        assert_eq!(found.partner, Partner::Found(6));
        // From inside the string only brackets in strings or comments count.
        let inside = match_at_cursor(&rope, 4, &none, &literal, MATCH_SCAN_CHARS).unwrap();
        assert!(inside.in_literal);
        assert_eq!(inside.partner, Partner::Unmatched);
        let quoted = Rope::from_str("f(\"(a)\")");
        let in_quotes = |i: usize| (2..=6).contains(&i);
        let pair = match_at_cursor(&quoted, 3, &none, &in_quotes, MATCH_SCAN_CHARS).unwrap();
        assert_eq!(pair.partner, Partner::Found(5));
        assert_eq!(
            colorize(&rope, 0, 1, &literal),
            vec![(1, Depth(0)), (6, Depth(0))]
        );
    }

    #[test]
    fn a_tree_pair_is_used_first() {
        let rope = Rope::from_str("(a)");
        let tree = |at: usize| (at == 0).then_some(2);
        let found = match_at_cursor(&rope, 0, &tree, &code, 0).unwrap();
        assert_eq!(found.partner, Partner::Found(2));
    }

    #[test]
    fn colors_follow_depth_and_mark_unmatched() {
        let text = "a(b[c{d}e]f)g) {";
        let rope = Rope::from_str(text);
        assert_eq!(
            colorize(&rope, 0, 1, &code),
            vec![
                (1, Depth(0)),
                (3, Depth(1)),
                (5, Depth(2)),
                (7, Depth(2)),
                (9, Depth(1)),
                (11, Depth(0)),
                (13, Unmatched),
                (15, Unmatched),
            ]
        );
        // An opener left open by a closer of its outer pair is unmatched.
        assert_eq!(
            colorize(&Rope::from_str("([)"), 0, 1, &code),
            vec![(0, Depth(0)), (1, Unmatched), (2, Depth(0))]
        );
    }

    #[test]
    fn colors_for_a_window_carry_the_depth_from_above() {
        let text = "{\n  (\n    [x]\n  )\n}\n";
        let rope = Rope::from_str(text);
        // Only line 2 is visible; `[` and `]` are at depth 2.
        let colors = colorize(&rope, 2, 3, &code);
        let line = rope.line_to_char(2);
        assert_eq!(colors, vec![(line + 4, Depth(2)), (line + 6, Depth(2))]);
        // An opener on screen whose closer is below the window is not unmatched.
        assert_eq!(colorize(&rope, 0, 1, &code), vec![(0, Depth(0))]);
    }

    #[test]
    fn past_the_lookback_a_stray_closer_is_not_an_error() {
        let text = format!("(\n{})\n", "x\n".repeat(COLOR_LOOKBACK_LINES + 10));
        let rope = Rope::from_str(&text);
        let last = rope.len_lines() - 2;
        let colors = colorize(&rope, last, last + 1, &code);
        assert_eq!(colors.len(), 1);
        assert_eq!(colors[0].1, Depth(0));
        // Read from the top, the same closer is matched.
        let near = Rope::from_str("(\nx\n)\n");
        assert_eq!(colorize(&near, 2, 3, &code), vec![(4, Depth(0))]);
        assert_eq!(
            colorize(&Rope::from_str(")"), 0, 1, &code),
            vec![(0, Unmatched)]
        );
    }

    /// `text` with `|` at the cursor; the text inside the pair found there.
    fn inside(text: &str, open: char, close: char) -> Option<String> {
        inside_with(text, open, close, &|_| true)
    }

    fn inside_with(
        text: &str,
        open: char,
        close: char,
        counts: &dyn Fn(usize) -> bool,
    ) -> Option<String> {
        let at = text.chars().position(|ch| ch == '|').unwrap();
        let rope = Rope::from_str(&text.replacen('|', "", 1));
        let (o, c) = enclosing_pair(&rope, at, at, open, close, counts)?;
        Some(rope.slice(o + 1..c).to_string())
    }

    #[test]
    fn enclosing_pair_takes_the_nearest_pair_of_its_kind() {
        assert_eq!(
            inside("add(first(1|, 2), 3)", '(', ')').as_deref(),
            Some("1, 2")
        );
        assert_eq!(
            inside("add(first(1, 2), |3)", '(', ')').as_deref(),
            Some("first(1, 2), 3")
        );
        assert_eq!(
            inside("f(a, [b|, c])", '(', ')').as_deref(),
            Some("a, [b, c]")
        );
        assert_eq!(inside("f(a, [b|, c])", '[', ']').as_deref(), Some("b, c"));
        assert_eq!(
            inside(
                "fn f() {\n    if x {\n        y();\n    }\n  | z\n}",
                '{',
                '}'
            )
            .as_deref(),
            Some("\n    if x {\n        y();\n    }\n   z\n")
        );
    }

    #[test]
    fn enclosing_pair_on_a_bracket_uses_that_pair() {
        assert_eq!(inside("f(|(a), b)", '(', ')').as_deref(), Some("a"));
        assert_eq!(inside("f((a|), b)", '(', ')').as_deref(), Some("a"));
        assert_eq!(inside("f(|)", '(', ')').as_deref(), Some(""));
    }

    #[test]
    fn enclosing_pair_without_a_pair_is_none() {
        assert_eq!(inside("let x| = 1;", '(', ')'), None);
        assert_eq!(inside("f(a) |g(b)", '(', ')'), None);
        assert_eq!(inside("f(a, b|", '(', ')'), None);
        assert_eq!(inside("[a|]", '(', ')'), None);
    }

    #[test]
    fn enclosing_pair_skips_brackets_the_caller_leaves_out() {
        let text = "f(\")\", x|)";
        let quoted = |at: usize| !(2..5).contains(&at);
        assert_eq!(
            inside_with(text, '(', ')', &quoted).as_deref(),
            Some("\")\", x")
        );
        assert_eq!(inside(text, '(', ')'), None);
    }

    #[test]
    fn enclosing_pair_of_a_selected_pair_or_inside_grows_outward() {
        let rope = Rope::from_str("((a) b)");
        let any = |_: usize| true;
        assert_eq!(enclosing_pair(&rope, 1, 2, '(', ')', &any), Some((0, 6)));
        assert_eq!(enclosing_pair(&rope, 1, 4, '(', ')', &any), Some((1, 3)));
        assert_eq!(enclosing_pair(&rope, 2, 3, '(', ')', &any), Some((1, 3)));
        assert_eq!(enclosing_pair(&rope, 2, 5, '(', ')', &any), Some((0, 6)));
    }

    #[test]
    fn select_inside_follows_actual_nesting_not_delimiter_order() {
        let none = |_: usize| None;
        let nest = |text: &str| -> Vec<String> {
            let at = text.find('|').unwrap();
            let rope = Rope::from_str(&text.replacen('|', "", 1));
            let at = rope.byte_to_char(at);
            let quotes = crate::quotes::enclosing_on_line(&rope, at);
            enclosing_inside(&rope, at, &quotes, &none)
                .into_iter()
                .map(|span| rope.slice(span.inner.0..span.inner.1).to_string())
                .collect()
        };
        let walk = |text: &str| -> Vec<String> {
            let at = text.find('|').unwrap();
            let rope = Rope::from_str(&text.replacen('|', "", 1));
            let at = rope.byte_to_char(at);
            let quotes = crate::quotes::enclosing_on_line(&rope, at);
            let pairs = enclosing_inside(&rope, at, &quotes, &none);
            let mut current = (at, at);
            let mut got = Vec::new();
            for _ in 0..pairs.len() {
                let range = next_inside(&pairs, current).unwrap();
                got.push(rope.slice(range.0..range.1).to_string());
                current = range;
            }
            got
        };

        // `{[()]}` nests as () then [] then {}, not a fixed delimiter order.
        assert_eq!(nest("{[(|)]}"), ["", "()", "[()]"]);
        assert_eq!(nest("{[(x|)]}"), ["x", "(x)", "[(x)]"]);
        // A caret inside empty `()` is already that inner, so the first `si` expands out.
        assert_eq!(walk("{[(|)]}"), ["()", "[()]", ""]);
        assert_eq!(walk("{[(x|)]}"), ["x", "(x)", "[(x)]"]);
        // Quotes inside brackets, and brackets inside quotes, in either order.
        assert_eq!(nest(r#"{["(a|)"]}"#), ["a", "(a)", "\"(a)\"", "[\"(a)\"]"]);
        assert_eq!(walk(r#"{["(a|)"]}"#), ["a", "(a)", "\"(a)\"", "[\"(a)\"]"]);
        assert_eq!(nest(r#"("a [|b] c")"#), ["b", "a [b] c", "\"a [b] c\""]);
        assert_eq!(walk(r#"("a [|b] c")"#), ["b", "a [b] c", "\"a [b] c\""]);
        // Same kind can nest; backticks count as quotes; `<>` is ignored.
        assert_eq!(nest("f((x|))"), ["x", "(x)"]);
        assert_eq!(walk("f((x|))"), ["x", "(x)"]);
        assert_eq!(nest("run `cargo |test` now"), ["cargo test"]);
        assert_eq!(walk("if a < b && c > |d { }"), Vec::<String>::new());
        // Cursor on a delimiter uses that pair. On `]` the innermost pair is `[]`.
        assert_eq!(nest("{[()|]}"), ["()", "[()]"]);
        assert_eq!(walk("{[()|]}"), ["()", "[()]"]);
        // No enclosing pair.
        let rope = Rope::from_str("let x = 1;");
        assert!(enclosing_inside(&rope, 4, &[], &none).is_empty());
        assert_eq!(next_inside(&[], (4, 4)), None);
    }

    #[test]
    fn next_inside_from_an_outer_selection_still_follows_the_nest() {
        let rope = Rope::from_str("{[(x)]}");
        // Cursor on x (index 3).
        let quotes = crate::quotes::enclosing_on_line(&rope, 3);
        let pairs = enclosing_inside(&rope, 3, &quotes, &|_| None);
        // Already selected the delimiters of `()` (the characters `(x)`).
        assert_eq!(next_inside(&pairs, (2, 5)), Some((1, 6))); // inside []
        assert_eq!(rope.slice(1..6).to_string(), "[(x)]");
    }
}
