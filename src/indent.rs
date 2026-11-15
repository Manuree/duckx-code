//! Auto indent for `Enter` and for a typed closing bracket.
//!
//! The rules follow what VS Code does with `onEnterRules` / `indentationRules` and what
//! Neovim's `smartindent` does: keep the line's indent, add one level after an opening
//! bracket or a block opener, split `{|}` onto three lines, and move a closing bracket
//! typed on a blank line back to its opener's indent. The grammars bundled here ship no
//! `indents.scm`, so the syntax tree is used only to ignore brackets and `:` inside
//! strings and comments.

use crate::syntax::Language;
use ropey::Rope;

/// One edit: replace chars `from..to` with `text`, then put the caret `caret` chars into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub from: usize,
    pub to: usize,
    pub text: String,
    pub caret: usize,
}

/// What the indent rules need to know about the buffer.
pub struct Context<'a> {
    pub language: Option<Language>,
    /// One indent level: a tab, or the file's number of spaces.
    pub unit: &'a str,
    /// The file's line break.
    pub ending: &'a str,
    /// True when the char at this index is inside a string or comment.
    pub literal: &'a dyn Fn(usize) -> bool,
}

const VOID_TAGS: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// The edit for `Enter` with the selection `from..to`.
pub fn on_enter(rope: &Rope, from: usize, to: usize, cx: &Context) -> Edit {
    let line = rope.char_to_line(from);
    let start = rope.line_to_char(line);
    let before: String = rope.slice(start..from).chars().collect();
    let after_end = line_content_end(rope, rope.char_to_line(to)).max(to);
    let after: String = rope.slice(to..after_end).chars().collect();
    let indent: String = before
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .collect();
    let plain = Edit {
        from,
        to,
        text: format!("{}{indent}", cx.ending),
        caret: cx.ending.chars().count() + indent.chars().count(),
    };
    let trimmed = before.trim_end();
    let code = trimmed.trim_start();
    if code.is_empty() {
        return plain;
    }
    let trimmed_len = trimmed.chars().count();
    let last_at = start + trimmed_len - 1;
    let last = rope.char(last_at);
    let after_code = after.trim_start();
    // Whitespace around the break is dropped when the indent changes.
    let cut_from = (start + trimmed_len).min(from);
    let cut_to = to + (after.chars().count() - after_code.chars().count());

    if after.trim().is_empty()
        && let Some(leader) = comment_leader(code, cx.language)
    {
        let text = format!("{}{indent}{leader}", cx.ending);
        let caret = text.chars().count();
        return Edit {
            from,
            to,
            text,
            caret,
        };
    }

    let in_literal = (cx.literal)(last_at);
    let opens_bracket = matches!(last, '{' | '(' | '[') && !in_literal;
    let opens_tag = is_markup(cx.language) && opening_tag(code);
    let opens_block = !in_literal && opens_block(code, cx.language);
    if opens_bracket || opens_tag || opens_block {
        let inner = format!("{indent}{}", cx.unit);
        let closes_here = (opens_bracket && after_code.starts_with(closer_of(last)))
            || (opens_tag && after_code.starts_with("</"));
        let mut text = format!("{}{inner}", cx.ending);
        let caret = text.chars().count();
        if closes_here {
            text.push_str(cx.ending);
            text.push_str(&indent);
        }
        return Edit {
            from: cut_from,
            to: cut_to,
            text,
            caret,
        };
    }

    if !in_literal && ends_block(code, cx.language) {
        let outer = strip_unit(&indent, cx.unit);
        let text = format!("{}{outer}", cx.ending);
        let caret = text.chars().count();
        return Edit {
            from: cut_from,
            to: cut_to,
            text,
            caret,
        };
    }
    plain
}

/// The edit for typing `close` at `at`: on a line that is blank up to the cursor, the line
/// takes the indent of the line holding the matching opener. `None` leaves typing as it is.
pub fn on_closer(rope: &Rope, at: usize, close: char, cx: &Context) -> Option<Edit> {
    let open = opener_of(close)?;
    let line = rope.char_to_line(at);
    let start = rope.line_to_char(line);
    if !rope
        .slice(start..at)
        .chars()
        .all(|ch| ch == ' ' || ch == '\t')
    {
        return None;
    }
    // Inside a multi-line string or comment the blank prefix is text, not indent.
    if (cx.literal)(if at > start { at - 1 } else { at }) {
        return None;
    }
    let mut stack = Vec::new();
    let mut chars = rope.chars_at(start);
    let mut index = start;
    let opener = loop {
        if start - index >= crate::brackets::MATCH_SCAN_CHARS {
            return None;
        }
        let ch = chars.prev()?;
        index -= 1;
        if !matches!(ch, '(' | ')' | '[' | ']' | '{' | '}') || (cx.literal)(index) {
            continue;
        }
        if let Some(want) = opener_of(ch) {
            stack.push(want);
        } else if let Some(want) = stack.pop() {
            if want != ch {
                return None;
            }
        } else if ch == open {
            break index;
        } else {
            return None;
        }
    };
    let opener_line = rope.char_to_line(opener);
    let target: String = rope
        .line(opener_line)
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .collect();
    let current: String = rope.slice(start..at).chars().collect();
    if target == current {
        return None;
    }
    let text = format!("{target}{close}");
    let caret = text.chars().count();
    Some(Edit {
        from: start,
        to: at,
        text,
        caret,
    })
}

fn line_content_end(rope: &Rope, line: usize) -> usize {
    let start = rope.line_to_char(line);
    let content = rope
        .line(line)
        .chars()
        .take_while(|ch| *ch != '\n' && *ch != '\r')
        .count();
    start + content
}

fn closer_of(open: char) -> char {
    match open {
        '{' => '}',
        '(' => ')',
        _ => ']',
    }
}

fn opener_of(close: char) -> Option<char> {
    match close {
        '}' => Some('{'),
        ')' => Some('('),
        ']' => Some('['),
        _ => None,
    }
}

/// `indent` less one level, for a line after `return` in Python.
fn strip_unit(indent: &str, unit: &str) -> String {
    if let Some(rest) = indent.strip_suffix(unit) {
        return rest.to_string();
    }
    if let Some(rest) = indent.strip_suffix('\t') {
        return rest.to_string();
    }
    let spaces = indent.chars().rev().take_while(|ch| *ch == ' ').count();
    let drop = spaces.min(unit.chars().count().max(1));
    indent[..indent.len() - drop].to_string()
}

fn last_word(code: &str) -> &str {
    code.rsplit(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .next()
        .unwrap_or("")
}

fn first_word(code: &str) -> &str {
    code.split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .next()
        .unwrap_or("")
}

/// Line comment text with any trailing `# …` or `-- …` comment removed, when the marker
/// is plainly outside quotes. Good enough for keyword checks at the end of a line.
fn without_trailing_comment<'a>(code: &'a str, marker: &str) -> &'a str {
    let mut quote = None;
    for (i, ch) in code.char_indices() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if code[i..].starts_with(marker) => return code[..i].trim_end(),
            None => {}
        }
    }
    code
}

/// Lines after which the next line is indented one level, besides an opening bracket.
fn opens_block(code: &str, language: Option<Language>) -> bool {
    match language {
        Some(Language::Python) => without_trailing_comment(code, "#").ends_with(':'),
        Some(Language::Yaml) => {
            let code = without_trailing_comment(code, " #");
            code.ends_with(':')
                || [": |", ": >", ": |-", ": >-", ": |+", ": >+", "- |", "- >"]
                    .iter()
                    .any(|tail| code.ends_with(tail))
                || code == "-"
        }
        Some(Language::Lua) => {
            let code = without_trailing_comment(code, "--");
            let word = last_word(code);
            matches!(word, "then" | "do" | "else" | "repeat")
                || (code.ends_with(')')
                    && code
                        .split_whitespace()
                        .any(|w| w == "function" || w.starts_with("function("))
                    && !code.contains(" end"))
        }
        Some(Language::Ruby) => {
            let code = without_trailing_comment(code, " #");
            matches!(last_word(code), "do" | "then" | "else" | "begin")
                || code
                    .strip_suffix('|')
                    .and_then(|code| code.rfind('|').map(|bar| code[..bar].trim_end()))
                    .is_some_and(|head| last_word(head) == "do" || head.ends_with('{'))
                || (matches!(
                    first_word(code),
                    "def"
                        | "class"
                        | "module"
                        | "if"
                        | "unless"
                        | "while"
                        | "until"
                        | "case"
                        | "elsif"
                        | "when"
                        | "rescue"
                        | "ensure"
                ) && !code.ends_with(" end")
                    && !code.ends_with(';'))
        }
        Some(Language::Bash) => {
            let code = without_trailing_comment(code, " #");
            matches!(last_word(code), "then" | "do" | "else")
                && !code.ends_with("fi")
                && !code.ends_with("done")
        }
        Some(Language::Elixir) => {
            let code = without_trailing_comment(code, " #");
            last_word(code) == "do" || code.ends_with("->")
        }
        Some(Language::Haskell) => {
            matches!(last_word(code), "where" | "do" | "of" | "let") || code.ends_with('=')
        }
        _ => false,
    }
}

/// Python lines after which the block is over, so the next line steps out one level.
fn ends_block(code: &str, language: Option<Language>) -> bool {
    language == Some(Language::Python)
        && matches!(
            first_word(code),
            "return" | "pass" | "break" | "continue" | "raise"
        )
}

fn is_markup(language: Option<Language>) -> bool {
    matches!(
        language,
        Some(Language::Html | Language::Xml | Language::Php | Language::Tsx)
    )
}

/// The line ends with an opening tag such as `<div class="a">` that is not closed on it.
fn opening_tag(code: &str) -> bool {
    let Some(body) = code.strip_suffix('>') else {
        return false;
    };
    if body.ends_with('/') || body.ends_with('?') || body.ends_with('-') {
        return false;
    }
    let Some(open) = body.rfind('<') else {
        return false;
    };
    let tag = &body[open + 1..];
    let name: String = tag
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || matches!(ch, '-' | ':' | '_'))
        .collect();
    !name.is_empty()
        && !tag.contains('<')
        && !VOID_TAGS.contains(&name.to_ascii_lowercase().as_str())
}

/// Continue a doc comment: `///`, `//!`, and `/** … */` blocks in C-like languages.
fn comment_leader(code: &str, language: Option<Language>) -> Option<&'static str> {
    let c_like = !matches!(
        language,
        Some(
            Language::Python
                | Language::Yaml
                | Language::Toml
                | Language::Bash
                | Language::Ruby
                | Language::Elixir
                | Language::Lua
                | Language::Sql
                | Language::Haskell
                | Language::Make
                | Language::Cmake
                | Language::Markdown
                | Language::Json
                | Language::Html
                | Language::Xml
        )
    ) && language.is_some();
    if !c_like {
        return None;
    }
    if code.starts_with("///") {
        Some("/// ")
    } else if code.starts_with("//!") {
        Some("//! ")
    } else if code.starts_with("/**") && !code.contains("*/") {
        Some(" * ")
    } else if (code == "*" || code.starts_with("* ")) && !code.contains("*/") {
        Some("* ")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` with its last `|` at the cursor, after `Enter`, with the new caret shown as `|`.
    fn enter(text: &str, language: Option<Language>) -> String {
        enter_with(text, language, "    ", &|_| false)
    }

    fn enter_with(
        text: &str,
        language: Option<Language>,
        unit: &str,
        literal: &dyn Fn(usize) -> bool,
    ) -> String {
        let byte = text.rfind('|').unwrap();
        let at = text[..byte].chars().count();
        let mut rope = Rope::from_str(&format!("{}{}", &text[..byte], &text[byte + 1..]));
        let cx = Context {
            language,
            unit,
            ending: "\n",
            literal,
        };
        let edit = on_enter(&rope, at, at, &cx);
        rope.remove(edit.from..edit.to);
        let mut text = edit.text.clone();
        let byte = text
            .char_indices()
            .nth(edit.caret)
            .map_or(text.len(), |(i, _)| i);
        text.insert(byte, '|');
        rope.insert(edit.from, &text);
        rope.to_string()
    }

    fn close(text: &str, ch: char) -> String {
        let at = text.chars().position(|c| c == '|').unwrap();
        let mut rope = Rope::from_str(&text.replacen('|', "", 1));
        let cx = Context {
            language: None,
            unit: "    ",
            ending: "\n",
            literal: &|_| false,
        };
        match on_closer(&rope, at, ch, &cx) {
            Some(edit) => {
                rope.remove(edit.from..edit.to);
                rope.insert(edit.from, &format!("{}|", edit.text));
            }
            None => rope.insert(at, &format!("{ch}|")),
        }
        rope.to_string()
    }

    const RUST: Option<Language> = Some(Language::Rust);
    const PYTHON: Option<Language> = Some(Language::Python);

    #[test]
    fn rust_braces_split_and_indent() {
        assert_eq!(enter("fn main() {|}", RUST), "fn main() {\n    |\n}");
        assert_eq!(enter("    if x {|}", RUST), "    if x {\n        |\n    }");
        assert_eq!(enter("fn main() {   |", RUST), "fn main() {\n    |");
        assert_eq!(enter("    let x = 1;|", RUST), "    let x = 1;\n    |");
        assert_eq!(enter("    foo(a,|", RUST), "    foo(a,\n    |");
        assert_eq!(enter("let v = vec![|];", RUST), "let v = vec![\n    |\n];");
    }

    #[test]
    fn brackets_in_strings_and_comments_do_not_indent() {
        let text = "let s = \"{|";
        let literal = |at: usize| at >= 8;
        assert_eq!(enter_with(text, RUST, "    ", &literal), "let s = \"{\n|");
    }

    #[test]
    fn tabs_follow_the_file() {
        assert_eq!(
            enter_with("\tif x {|}", RUST, "\t", &|_| false),
            "\tif x {\n\t\t|\n\t}"
        );
    }

    #[test]
    fn python_blocks_indent_and_return_dedents() {
        assert_eq!(enter("def f():|", PYTHON), "def f():\n    |");
        assert_eq!(
            enter("    if x:  # check|", PYTHON),
            "    if x:  # check\n        |"
        );
        assert_eq!(
            enter("        return 1|", PYTHON),
            "        return 1\n    |"
        );
        assert_eq!(enter("    x = 1|", PYTHON), "    x = 1\n    |");
        assert_eq!(enter("d = {|}", PYTHON), "d = {\n    |\n}");
        assert_eq!(enter("x = \"a:\"|", PYTHON), "x = \"a:\"\n|");
    }

    #[test]
    fn javascript_and_typescript() {
        let js = Some(Language::JavaScript);
        assert_eq!(enter("function f() {|}", js), "function f() {\n    |\n}");
        assert_eq!(enter("const a = [|", js), "const a = [\n    |");
        assert_eq!(
            enter("  foo(() => {|});", Some(Language::TypeScript)),
            "  foo(() => {\n      |\n  });"
        );
        assert_eq!(enter("/**|", js), "/**\n * |");
        assert_eq!(enter(" * text|", js), " * text\n * |");
        assert_eq!(enter("/// docs|", RUST), "/// docs\n/// |");
        assert_eq!(enter("// plain|", js), "// plain\n|");
    }

    #[test]
    fn json_objects_and_arrays() {
        let json = Some(Language::Json);
        assert_eq!(enter("{|}", json), "{\n    |\n}");
        assert_eq!(enter("  \"a\": [|]", json), "  \"a\": [\n      |\n  ]");
        assert_eq!(enter("  \"a\": 1,|", json), "  \"a\": 1,\n  |");
    }

    #[test]
    fn lua_keywords() {
        let lua = Some(Language::Lua);
        assert_eq!(enter("if x then|", lua), "if x then\n    |");
        assert_eq!(enter("for i = 1, 3 do|", lua), "for i = 1, 3 do\n    |");
        assert_eq!(
            enter("local function f(a)|", lua),
            "local function f(a)\n    |"
        );
        assert_eq!(enter("local t = {|}", lua), "local t = {\n    |\n}");
        assert_eq!(enter("print(x)|", lua), "print(x)\n|");
        assert_eq!(enter("f(function() end)|", lua), "f(function() end)\n|");
    }

    #[test]
    fn yaml_keys_and_block_scalars() {
        let yaml = Some(Language::Yaml);
        assert_eq!(enter("jobs:|", yaml), "jobs:\n    |");
        assert_eq!(enter("  run: ||", yaml), "  run: |\n      |");
        assert_eq!(enter("  name: build|", yaml), "  name: build\n  |");
        assert_eq!(enter("  - name: x|", yaml), "  - name: x\n  |");
        assert_eq!(
            enter("url: http://x # note:|", yaml),
            "url: http://x # note:\n|"
        );
    }

    #[test]
    fn plain_text_keeps_the_indent_only() {
        assert_eq!(enter("  hello {|}", None), "  hello {\n      |\n  }");
        assert_eq!(enter("  note: x|", None), "  note: x\n  |");
        assert_eq!(enter("  |text", None), "  \n  |text");
        assert_eq!(enter("|", None), "\n|");
    }

    #[test]
    fn other_block_openers() {
        assert_eq!(
            enter("if [ -f x ]; then|", Some(Language::Bash)),
            "if [ -f x ]; then\n    |"
        );
        assert_eq!(enter("def f|", Some(Language::Ruby)), "def f\n    |");
        assert_eq!(
            enter("x.each do |i||", Some(Language::Ruby)),
            "x.each do |i|\n    |"
        );
        assert_eq!(
            enter("def f do|", Some(Language::Elixir)),
            "def f do\n    |"
        );
        assert_eq!(
            enter("<div class=\"a\">|</div>", Some(Language::Html)),
            "<div class=\"a\">\n    |\n</div>"
        );
        assert_eq!(enter("<br>|", Some(Language::Html)), "<br>\n|");
        assert_eq!(enter("<p>hi</p>|", Some(Language::Html)), "<p>hi</p>\n|");
    }

    #[test]
    fn closers_on_a_blank_line_go_back_to_the_opener() {
        assert_eq!(
            close("fn f() {\n    x();\n    |", '}'),
            "fn f() {\n    x();\n}|"
        );
        assert_eq!(
            close("  a = [\n        1,\n        |", ']'),
            "  a = [\n        1,\n  ]|"
        );
        assert_eq!(close("f(\n  g(1),\n    |", ')'), "f(\n  g(1),\n)|");
        assert_eq!(close("    x|", '}'), "    x}|");
        assert_eq!(close("fn f() {\n|", '}'), "fn f() {\n}|");
        assert_eq!(close("foo(\n    |", '}'), "foo(\n    }|");
    }
}
