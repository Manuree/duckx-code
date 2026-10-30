//! Turns LSP markup and snippets into the plain text the editor can show and insert.
//!
//! Hover and completion docs are shown as plain text. Snippet inserts stay in
//! snippet form so the editor can keep their tab stops.

use serde_json::Value;

/// A position range as the server sent it, before it is turned into rope bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRange {
    pub line: u32,
    pub character: u32,
    pub end_line: u32,
    pub end_character: u32,
}

/// One place `textDocument/definition` pointed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawLocation {
    pub uri: String,
    pub range: RawRange,
}

/// One completion row, with the text that should replace the prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCompletion {
    pub label: String,
    pub detail: String,
    pub filter: String,
    pub insert: String,
    pub kind: String,
    pub docs: String,
    pub snippet: bool,
    /// The item as JSON when the editor may ask `completionItem/resolve`.
    pub resolve: Option<String>,
    /// The range `textEdit` replaces. `None` means the editor's own word prefix.
    pub replace: Option<RawRange>,
    /// Edits outside the prefix, such as an added import. Applied with the insert.
    pub extra: Vec<(RawRange, String)>,
}

/// Markdown and MarkedString contents, as one plain block. `None` when there is nothing to show.
pub fn hover_text(result: &Value) -> Option<String> {
    let contents = result.get("contents").unwrap_or(result);
    let text = flatten_contents(contents);
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

pub fn definition_targets(result: &Value) -> Vec<RawLocation> {
    if result.is_null() {
        return Vec::new();
    }
    if let Some(list) = result.as_array() {
        return list.iter().filter_map(one_location).collect();
    }
    one_location(result).into_iter().collect()
}

pub fn completion_items(result: &Value) -> (bool, Vec<RawCompletion>) {
    if result.is_null() {
        return (false, Vec::new());
    }
    let (incomplete, items) = if let Some(list) = result.as_array() {
        (false, list.as_slice())
    } else {
        let incomplete = result["isIncomplete"].as_bool().unwrap_or(false);
        let items = result["items"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        (incomplete, items)
    };
    let items = items.iter().filter_map(one_completion).collect();
    (incomplete, items)
}

/// `${1:name}` becomes `name`, `$1` and `${1}` become nothing, `$$` becomes `$`.
pub fn plain_snippet(text: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '$' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        if chars.get(i + 1) == Some(&'$') {
            out.push('$');
            i += 2;
            continue;
        }
        if chars.get(i + 1) == Some(&'{') {
            let rest = &chars[i + 2..];
            let Some(end) = rest.iter().position(|c| *c == '}') else {
                out.push('$');
                i += 1;
                continue;
            };
            let body: String = rest[..end].iter().collect();
            let tabstop = body.chars().next().is_some_and(|c| c.is_ascii_digit());
            if tabstop {
                if let Some((_, default)) = body.split_once(':') {
                    out.push_str(default);
                }
            } else {
                out.push('$');
                out.push('{');
                out.push_str(&body);
                out.push('}');
            }
            i += end + 3;
            continue;
        }
        let digits = chars[i + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        if digits > 0 {
            i += 1 + digits;
            continue;
        }
        out.push('$');
        i += 1;
    }
    out
}

fn flatten_contents(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return plain_markup(text);
    }
    if let Some(list) = value.as_array() {
        return list
            .iter()
            .map(flatten_contents)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    if let Some(value_text) = value.get("value").and_then(Value::as_str) {
        if value.get("language").and_then(Value::as_str).is_some() {
            return value_text.to_string();
        }
        return plain_markup(value_text);
    }
    String::new()
}

/// Drops the markup characters and keeps the words. Fenced code stays, without the fences.
pub fn plain_markup(text: &str) -> String {
    let mut out = String::new();
    let mut fence = false;
    for line in text.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            fence = !fence;
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        if fence {
            out.push_str(line);
        } else {
            out.push_str(&plain_markup_line(trimmed));
        }
    }
    collapse_blank_lines(&out)
}

fn plain_markup_line(line: &str) -> String {
    let mut line = line.to_string();
    while line.starts_with('#') {
        line = line.trim_start_matches('#').trim_start().to_string();
    }
    let mut out = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            i += 1;
            while i < chars.len() && chars[i] != '`' {
                out.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            continue;
        }
        if chars[i] == '['
            && let Some(close) = chars[i + 1..].iter().position(|c| *c == ']')
        {
            let label: String = chars[i + 1..i + 1 + close].iter().collect();
            let after = i + 1 + close + 1;
            if chars.get(after) == Some(&'(')
                && let Some(end) = chars[after + 1..].iter().position(|c| *c == ')')
            {
                out.push_str(&label);
                i = after + 1 + end + 1;
                continue;
            }
        }
        if chars[i] == '*' || chars[i] == '_' {
            i += 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in text.split('\n') {
        if line.trim().is_empty() {
            blank += 1;
            if blank <= 1 && !out.is_empty() {
                out.push('\n');
            }
            continue;
        }
        blank = 0;
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim().to_string()
}

fn one_location(value: &Value) -> Option<RawLocation> {
    let uri = value
        .get("uri")
        .or_else(|| value.get("targetUri"))
        .and_then(Value::as_str)?;
    let range = value
        .get("targetSelectionRange")
        .or_else(|| value.get("targetRange"))
        .or_else(|| value.get("range"))
        .and_then(raw_range)?;
    Some(RawLocation {
        uri: uri.to_string(),
        range,
    })
}

fn one_completion(value: &Value) -> Option<RawCompletion> {
    let label = value.get("label").and_then(Value::as_str)?.to_string();
    let filter = value
        .get("filterText")
        .and_then(Value::as_str)
        .unwrap_or(&label)
        .to_string();
    let detail = value
        .get("detail")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let snippet = value
        .get("insertTextFormat")
        .and_then(Value::as_u64)
        .is_some_and(|kind| kind == 2);
    let kind = kind_label(value.get("kind").and_then(Value::as_u64));
    let docs = documentation_text(value.get("documentation"));
    let (insert, replace) = text_edit(value).unwrap_or_else(|| {
        (
            value
                .get("insertText")
                .and_then(Value::as_str)
                .unwrap_or(&label)
                .to_string(),
            None,
        )
    });
    let extra = value
        .get("additionalTextEdits")
        .and_then(Value::as_array)
        .map(|edits| {
            edits
                .iter()
                .filter_map(|edit| {
                    let range = edit.get("range").and_then(raw_range)?;
                    let text = edit.get("newText").and_then(Value::as_str)?.to_string();
                    Some((range, text))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(RawCompletion {
        label,
        detail,
        filter,
        insert,
        kind,
        docs,
        snippet,
        resolve: Some(value.to_string()),
        replace,
        extra,
    })
}

fn kind_label(kind: Option<u64>) -> String {
    match kind {
        Some(2) => "method",
        Some(3) => "fn",
        Some(4) => "new",
        Some(5 | 10) => "field",
        Some(6) => "var",
        Some(7 | 8 | 25) => "type",
        Some(9) => "mod",
        Some(13) => "enum",
        Some(14) => "keyword",
        Some(15) => "snippet",
        Some(21) => "const",
        Some(22) => "struct",
        _ => "",
    }
    .to_string()
}

fn documentation_text(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if let Some(text) = value.as_str() {
        return text.lines().next().unwrap_or("").trim().to_string();
    }
    hover_text(&serde_json::json!({ "contents": value }))
        .map(|text| text.lines().next().unwrap_or("").trim().to_string())
        .unwrap_or_default()
}

/// The active signature label. `None` when the server sent no signatures.
pub fn signature_text(result: &Value) -> Option<String> {
    let signatures = result.get("signatures").and_then(Value::as_array)?;
    let active = result
        .get("activeSignature")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let signature = signatures.get(active).or_else(|| signatures.first())?;
    let label = signature.get("label").and_then(Value::as_str)?.trim();
    (!label.is_empty()).then(|| label.to_string())
}

fn text_edit(value: &Value) -> Option<(String, Option<RawRange>)> {
    let edit = value.get("textEdit")?;
    let text = edit.get("newText").and_then(Value::as_str)?.to_string();
    let range = edit
        .get("replace")
        .or_else(|| edit.get("insert"))
        .or_else(|| edit.get("range"))
        .and_then(raw_range);
    Some((text, range))
}

/// One replacement from a workspace edit or a formatting result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTextEdit {
    pub uri: String,
    pub range: RawRange,
    pub text: String,
}

/// A code action the editor can show. `edit` is present when the server included one.
#[derive(Debug, Clone, PartialEq)]
pub struct RawAction {
    pub title: String,
    pub edit: Option<Value>,
    pub raw: String,
}

/// `WorkspaceEdit`, either `documentChanges` or `changes`.
pub fn workspace_edits(value: &Value) -> Vec<RawTextEdit> {
    if let Some(changes) = value.get("documentChanges").and_then(Value::as_array) {
        let mut edits = Vec::new();
        for change in changes {
            let Some(uri) = change
                .get("textDocument")
                .and_then(|doc| doc.get("uri"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            edits.extend(edits_for(uri, change.get("edits")));
        }
        if !edits.is_empty() {
            return edits;
        }
    }
    let Some(changes) = value.get("changes").and_then(Value::as_object) else {
        return Vec::new();
    };
    changes
        .iter()
        .flat_map(|(uri, edits)| edits_for(uri, Some(edits)))
        .collect()
}

/// `textDocument/formatting` returns a bare list of edits for one file.
pub fn formatting_edits(uri: &str, value: &Value) -> Vec<RawTextEdit> {
    edits_for(uri, Some(value))
}

/// Code actions, skipping entries that have no title.
pub fn code_actions(value: &Value) -> Vec<RawAction> {
    let Some(list) = value.as_array() else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| {
            let title = item.get("title").and_then(Value::as_str)?.to_string();
            if title.is_empty() {
                return None;
            }
            let edit = item.get("edit").cloned();
            Some(RawAction {
                title,
                edit,
                raw: item.to_string(),
            })
        })
        .collect()
}

fn edits_for(uri: &str, value: Option<&Value>) -> Vec<RawTextEdit> {
    let Some(list) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|edit| {
            let range = raw_range(edit.get("range")?)?;
            let text = edit.get("newText").and_then(Value::as_str)?.to_string();
            Some(RawTextEdit {
                uri: uri.to_string(),
                range,
                text,
            })
        })
        .collect()
}

fn raw_range(value: &Value) -> Option<RawRange> {
    let start = value.get("start")?;
    let end = value.get("end")?;
    Some(RawRange {
        line: start.get("line").and_then(Value::as_u64)? as u32,
        character: start.get("character").and_then(Value::as_u64)? as u32,
        end_line: end.get("line").and_then(Value::as_u64)? as u32,
        end_character: end.get("character").and_then(Value::as_u64)? as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn markup_keeps_words_and_code() {
        let text = plain_markup(
            "# Title\n\nSee `Vec` and [docs](https://example.com).\n\n```rust\nlet x = 1;\n```\n",
        );
        assert!(text.contains("Title"), "{text}");
        assert!(text.contains("Vec"), "{text}");
        assert!(text.contains("docs"), "{text}");
        assert!(!text.contains("https://"), "{text}");
        assert!(text.contains("let x = 1;"), "{text}");
        assert!(!text.contains("```"), "{text}");
    }

    #[test]
    fn snippets_keep_the_default_text_only() {
        assert_eq!(plain_snippet("vec![$1]"), "vec![]");
        assert_eq!(plain_snippet("${1:name}: ${2:Type}"), "name: Type");
        assert_eq!(plain_snippet("$$"), "$");
    }

    #[test]
    fn hover_reads_markup_and_marked_strings() {
        let markup = json!({"contents": {"kind": "markdown", "value": "**bold**"}});
        assert_eq!(hover_text(&markup).as_deref(), Some("bold"));
        let marked = json!({"contents": {"language": "rust", "value": "fn f()"}});
        assert_eq!(hover_text(&marked).as_deref(), Some("fn f()"));
        assert_eq!(hover_text(&json!(null)), None);
    }

    #[test]
    fn definition_reads_a_location_or_a_link() {
        let one = json!({"uri": "file:///a.rs", "range": {"start": {"line": 1, "character": 2}, "end": {"line": 1, "character": 5}}});
        assert_eq!(definition_targets(&one).len(), 1);
        let link = json!({"targetUri": "file:///b.rs", "targetSelectionRange": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}, "targetRange": {"start": {"line": 0, "character": 0}, "end": {"line": 3, "character": 0}}});
        let got = definition_targets(&link);
        assert_eq!(got[0].uri, "file:///b.rs");
        assert_eq!(got[0].range.character, 0);
        assert_eq!(got[0].range.end_character, 1);
    }

    #[test]
    fn completion_prefers_the_text_edit_and_flattens_snippets() {
        let result = json!({
            "isIncomplete": true,
            "items": [{
                "label": "println",
                "filterText": "println",
                "insertTextFormat": 2,
                "textEdit": {"range": {"start": {"line": 0, "character": 1}, "end": {"line": 0, "character": 4}}, "newText": "println!($0)"},
                "additionalTextEdits": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "use std::io;\n"}]
            }]
        });
        let (incomplete, items) = completion_items(&result);
        assert!(incomplete);
        assert_eq!(items[0].insert, "println!($0)");
        assert!(items[0].snippet);
        assert_eq!(items[0].extra.len(), 1);
        assert!(items[0].replace.is_some());
    }
}
