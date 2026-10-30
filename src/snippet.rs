//! LSP snippets. Placeholders keep their default text, and each tab stop is a
//! range the editor can select. `$0` is the last stop. A transform is dropped.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSnippet {
    pub text: String,
    /// Char ranges in [`Self::text`], in the order `Tab` visits them.
    pub stops: Vec<(usize, usize)>,
}

pub fn parse(source: &str) -> ParsedSnippet {
    let chars: Vec<char> = source.chars().collect();
    let mut cursor = 0;
    let mut text = String::new();
    let mut marks: Vec<(u32, usize, usize)> = Vec::new();
    scan(&chars, &mut cursor, &mut text, &mut marks, false);
    let mut stops: Vec<(u32, usize, usize)> = Vec::new();
    for mark in marks {
        if stops.iter().any(|(index, _, _)| *index == mark.0) {
            continue;
        }
        stops.push(mark);
    }
    let has_final = stops.iter().any(|(index, _, _)| *index == 0);
    stops.sort_by_key(|(index, _, _)| (*index == 0, *index));
    if !stops.is_empty() && !has_final {
        let end = text.chars().count();
        stops.push((0, end, end));
    }
    ParsedSnippet {
        text,
        stops: stops
            .into_iter()
            .map(|(_, start, end)| (start, end))
            .collect(),
    }
}

fn scan(
    chars: &[char],
    cursor: &mut usize,
    text: &mut String,
    marks: &mut Vec<(u32, usize, usize)>,
    inside: bool,
) {
    while *cursor < chars.len() {
        let ch = chars[*cursor];
        if inside && ch == '}' {
            return;
        }
        if inside && ch == '\\' {
            if let Some(next) = chars.get(*cursor + 1).copied() {
                if next == '\\' || next == '}' || next == '|' || next == ',' || next == '$' {
                    text.push(next);
                    *cursor += 2;
                    continue;
                }
            }
        }
        if ch != '$' {
            text.push(ch);
            *cursor += 1;
            continue;
        }
        let Some(next) = chars.get(*cursor + 1).copied() else {
            text.push('$');
            *cursor += 1;
            return;
        };
        if next == '$' {
            text.push('$');
            *cursor += 2;
            continue;
        }
        if next == '{' {
            *cursor += 2;
            placeholder(chars, cursor, text, marks);
            continue;
        }
        if next.is_ascii_digit() {
            *cursor += 1;
            let index = read_number(chars, cursor);
            let at = text.chars().count();
            marks.push((index, at, at));
            continue;
        }
        text.push('$');
        *cursor += 1;
    }
}

fn placeholder(
    chars: &[char],
    cursor: &mut usize,
    text: &mut String,
    marks: &mut Vec<(u32, usize, usize)>,
) {
    let index = read_number(chars, cursor);
    let start = text.chars().count();
    match chars.get(*cursor).copied() {
        Some(':') => {
            *cursor += 1;
            scan(chars, cursor, text, marks, true);
            if chars.get(*cursor) == Some(&'}') {
                *cursor += 1;
            }
        }
        Some('|') => {
            *cursor += 1;
            let mut choice = String::new();
            while let Some(ch) = chars.get(*cursor).copied() {
                *cursor += 1;
                if ch == '|' || ch == ',' || ch == '}' {
                    break;
                }
                if ch == '\\' {
                    if let Some(next) = chars.get(*cursor).copied() {
                        choice.push(next);
                        *cursor += 1;
                        continue;
                    }
                }
                choice.push(ch);
            }
            if chars.get(cursor.saturating_sub(1)) != Some(&'}') {
                while chars.get(*cursor).is_some_and(|ch| *ch != '}') {
                    *cursor += 1;
                }
                if chars.get(*cursor) == Some(&'}') {
                    *cursor += 1;
                }
            }
            text.push_str(&choice);
        }
        Some('/') => {
            while chars.get(*cursor).is_some_and(|ch| *ch != '}') {
                *cursor += 1;
            }
            if chars.get(*cursor) == Some(&'}') {
                *cursor += 1;
            }
        }
        Some('}') => *cursor += 1,
        _ => {}
    }
    let end = text.chars().count();
    marks.push((index, start, end));
}

fn read_number(chars: &[char], cursor: &mut usize) -> u32 {
    let mut value = 0u32;
    while let Some(ch) = chars.get(*cursor).copied() {
        if !ch.is_ascii_digit() {
            break;
        }
        value = value
            .saturating_mul(10)
            .saturating_add(ch as u32 - '0' as u32);
        *cursor += 1;
    }
    value
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn placeholders_keep_defaults_and_tab_order() {
        let parsed = parse("${1:name}: ${2:Type}$0");
        assert_eq!(parsed.text, "name: Type");
        assert_eq!(parsed.stops, vec![(0, 4), (6, 10), (10, 10)]);
    }

    #[test]
    fn a_final_stop_is_added_when_the_snippet_omits_it() {
        let parsed = parse("vec![$1]");
        assert_eq!(parsed.text, "vec![]");
        assert_eq!(parsed.stops, vec![(5, 5), (6, 6)]);
    }

    #[test]
    fn dollars_choices_and_plain_text() {
        assert_eq!(parse("$$").text, "$");
        assert!(parse("hello").stops.is_empty());
        let choice = parse("${1|alpha,beta|}");
        assert_eq!(choice.text, "alpha");
        assert_eq!(choice.stops[0], (0, 5));
    }
}
