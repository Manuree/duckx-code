//! `textDocument/semanticTokens/full`. The integer stream is the server's legend,
//! and each token becomes a syntax role that paints over the tree-sitter color.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ropey::Rope;

use super::diagnostics::Span;
use super::position::{self, Encoding};
use crate::theme::SyntaxRole;

#[derive(Debug, Default)]
pub struct SemanticTokens {
    files: HashMap<PathBuf, Vec<(usize, usize, SyntaxRole)>>,
}

impl SemanticTokens {
    pub fn set(&mut self, path: PathBuf, spans: Vec<(usize, usize, SyntaxRole)>) {
        if spans.is_empty() {
            self.files.remove(&path);
            return;
        }
        self.files.insert(path, spans);
    }

    pub fn clear(&mut self, path: &Path) {
        self.files.remove(path);
    }

    pub fn get(&self, path: &Path) -> &[(usize, usize, SyntaxRole)] {
        self.files.get(path).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn shift(&mut self, path: &Path, span: Span) {
        let Some(items) = self.files.get_mut(path) else {
            return;
        };
        for item in items.iter_mut() {
            item.0 = span.map(item.0, false);
            item.1 = span.map(item.1, true).max(item.0);
        }
        items.retain(|(start, end, _)| end > start);
        if items.is_empty() {
            self.files.remove(path);
        }
    }
}

/// `data` is repeating `(deltaLine, deltaStart, length, tokenType, tokenModifiers)`.
/// Ranges are bytes in `text`.
pub fn decode(
    data: &[u64],
    text: &Rope,
    encoding: Encoding,
    types: &[String],
    modifiers: &[String],
) -> Vec<(usize, usize, SyntaxRole)> {
    let mut line = 0u32;
    let mut start = 0u32;
    let mut spans = Vec::new();
    for chunk in data.chunks(5) {
        if chunk.len() < 5 {
            break;
        }
        let delta_line = chunk[0] as u32;
        line = line.saturating_add(delta_line);
        start = if delta_line == 0 {
            start.saturating_add(chunk[1] as u32)
        } else {
            chunk[1] as u32
        };
        let length = chunk[2] as u32;
        let type_name = types
            .get(chunk[3] as usize)
            .map(String::as_str)
            .unwrap_or("");
        let Some(role) = role_for(type_name, chunk[4], modifiers) else {
            continue;
        };
        let start_byte = position::byte_of(text, line, start, encoding);
        let end_byte = position::byte_of(text, line, start.saturating_add(length), encoding);
        if end_byte > start_byte {
            spans.push((start_byte, end_byte, role));
        }
    }
    spans
}

fn role_for(type_name: &str, bits: u64, modifiers: &[String]) -> Option<SyntaxRole> {
    if type_name.is_empty() {
        return None;
    }
    let documentation = modifier(bits, modifiers, "documentation");
    let library = modifier(bits, modifiers, "defaultLibrary");
    if documentation && type_name == "comment" {
        return Some(SyntaxRole::Documentation);
    }
    if library {
        return Some(SyntaxRole::Builtin);
    }
    Some(match type_name {
        "macro" => SyntaxRole::Macro,
        "lifetime" => SyntaxRole::Lifetime,
        "namespace" | "type" | "class" | "enum" | "interface" | "struct" | "typeParameter" => {
            SyntaxRole::Type
        }
        "parameter" => SyntaxRole::Parameter,
        "variable" => SyntaxRole::Variable,
        "property" | "enumMember" => SyntaxRole::Property,
        "function" | "method" => SyntaxRole::Function,
        "keyword" | "modifier" => SyntaxRole::Keyword,
        "comment" => SyntaxRole::Comment,
        "string" | "regexp" => SyntaxRole::String,
        "number" => SyntaxRole::Number,
        "operator" => SyntaxRole::Operator,
        "decorator" => SyntaxRole::Attribute,
        _ => return None,
    })
}

fn modifier(bits: u64, modifiers: &[String], name: &str) -> bool {
    modifiers
        .iter()
        .enumerate()
        .any(|(index, modifier)| modifier == name && index < 64 && bits & (1u64 << index) != 0)
}

#[cfg(test)]
mod tests {
    use super::decode;
    use crate::lsp::position::Encoding;
    use crate::theme::SyntaxRole;
    use ropey::Rope;

    #[test]
    fn a_relative_token_stream_picks_roles_from_the_legend() {
        let text = Rope::from_str("fn main() {}\n");
        let types = ["keyword", "function", "lifetime"]
            .map(str::to_string)
            .to_vec();
        let modifiers = ["documentation", "defaultLibrary"]
            .map(str::to_string)
            .to_vec();
        let data = vec![
            0, 0, 2, 0, 0, // fn
            0, 3, 4, 1, 2, // main, defaultLibrary
        ];
        let spans = decode(&data, &text, Encoding::Utf8, &types, &modifiers);
        assert_eq!(spans[0], (0, 2, SyntaxRole::Keyword));
        assert_eq!(spans[1].2, SyntaxRole::Builtin);
        assert!(spans[1].1 > spans[1].0);
    }
}
