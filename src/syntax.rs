//! Incremental syntax highlighting.
//!
//! Parsing runs off the UI thread. An edit only calls `tree.edit` (so the old
//! colors stay aligned) and queues a parse of a rope clone. Rendering queries
//! the visible line range and never mutates the tree.

use std::path::Path;
use std::sync::OnceLock;

use ropey::Rope;
use tree_sitter::{
    InputEdit, Node, Parser, Point as TsPoint, Query, QueryCursor, StreamingIterator, Tree,
};
use tree_sitter_md::{MarkdownParseOptions, MarkdownParser, MarkdownTree};

use crate::buffer::ByteEdit;
use crate::theme::SyntaxRole;

/// Files larger than this stay plain text. Parsing them would stall a worker.
pub const MAX_HIGHLIGHT_BYTES: usize = 8 * 1024 * 1024;

/// A worker gives up on a parse that runs longer than this.
const PARSE_TIMEOUT_MICROS: u64 = 2_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Python,
    Json,
    Markdown,
    Toml,
    JavaScript,
    TypeScript,
    Tsx,
    Bash,
    Yaml,
    Html,
    Css,
    Go,
    C,
    Cpp,
    Java,
    CSharp,
    Lua,
    Php,
    Ruby,
    Sql,
    Elixir,
    GraphQl,
    Scala,
    Haskell,
    Swift,
    Zig,
    Hcl,
    PowerShell,
    Xml,
    Make,
    Cmake,
}

impl Language {
    pub fn detect(path: &Path) -> Option<Self> {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match file_name.as_str() {
            "makefile" | "gnumakefile" => return Some(Self::Make),
            "cmakelists.txt" => return Some(Self::Cmake),
            "rakefile" | "gemfile" => return Some(Self::Ruby),
            _ => {}
        }
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "rs" => Self::Rust,
            "py" | "pyi" => Self::Python,
            "json" | "jsonc" => Self::Json,
            "md" | "markdown" => Self::Markdown,
            "toml" => Self::Toml,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "sh" | "bash" | "zsh" => Self::Bash,
            "yml" | "yaml" => Self::Yaml,
            "html" | "htm" => Self::Html,
            "css" => Self::Css,
            "go" => Self::Go,
            "c" | "h" => Self::C,
            "cc" | "cpp" | "cxx" | "c++" | "hh" | "hpp" | "hxx" | "h++" => Self::Cpp,
            "java" => Self::Java,
            "cs" => Self::CSharp,
            "lua" => Self::Lua,
            "php" | "phtml" => Self::Php,
            "rb" | "rake" | "gemspec" => Self::Ruby,
            "sql" => Self::Sql,
            "ex" | "exs" => Self::Elixir,
            "graphql" | "gql" => Self::GraphQl,
            "scala" | "sc" => Self::Scala,
            "hs" | "lhs" => Self::Haskell,
            "swift" => Self::Swift,
            "zig" => Self::Zig,
            "hcl" | "tf" | "tfvars" => Self::Hcl,
            "ps1" | "psm1" => Self::PowerShell,
            "xml" => Self::Xml,
            "mk" => Self::Make,
            "cmake" => Self::Cmake,
            _ => return None,
        })
    }

    /// Language tag of a Markdown fenced code block, such as `rust` or `ts`.
    pub fn from_info(info: &str) -> Option<Self> {
        let name = info.split_whitespace().next()?.to_ascii_lowercase();
        Some(match name.as_str() {
            "rust" | "rs" => Self::Rust,
            "python" | "py" => Self::Python,
            "json" | "jsonc" => Self::Json,
            "toml" => Self::Toml,
            "javascript" | "js" | "jsx" => Self::JavaScript,
            "typescript" | "ts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "bash" | "sh" | "shell" | "zsh" => Self::Bash,
            "yaml" | "yml" => Self::Yaml,
            "html" | "htm" => Self::Html,
            "css" => Self::Css,
            "go" | "golang" => Self::Go,
            "c" => Self::C,
            "cpp" | "c++" | "cxx" | "hpp" => Self::Cpp,
            "java" => Self::Java,
            "csharp" | "cs" | "c#" => Self::CSharp,
            "lua" => Self::Lua,
            "php" => Self::Php,
            "ruby" | "rb" => Self::Ruby,
            "sql" => Self::Sql,
            "elixir" | "ex" | "exs" => Self::Elixir,
            "graphql" | "gql" => Self::GraphQl,
            "scala" => Self::Scala,
            "haskell" | "hs" => Self::Haskell,
            "swift" => Self::Swift,
            "zig" => Self::Zig,
            "hcl" | "terraform" | "tf" => Self::Hcl,
            "powershell" | "ps1" => Self::PowerShell,
            "xml" => Self::Xml,
            "make" | "makefile" => Self::Make,
            "cmake" => Self::Cmake,
            _ => return None,
        })
    }
}

/// A parsed tree. Markdown keeps the block tree and its inline trees together.
#[derive(Clone, Debug)]
pub enum HighlightTree {
    Code(Tree),
    Markdown(MarkdownTree),
}

impl HighlightTree {
    fn edit(&mut self, input: &InputEdit) {
        match self {
            Self::Code(tree) => tree.edit(input),
            Self::Markdown(tree) => tree.edit(input),
        }
    }

    pub fn sexp(&self) -> String {
        match self {
            Self::Code(tree) => tree.root_node().to_sexp(),
            Self::Markdown(tree) => tree.block_tree().root_node().to_sexp(),
        }
    }
}

/// A fenced code block parsed with its own grammar. Byte offsets are document-absolute.
#[derive(Clone, Debug)]
pub struct Injection {
    language: Language,
    start_byte: usize,
    end_byte: usize,
    tree: Tree,
}

impl Injection {
    /// Shifts this fence across one edit. `None` boundaries mean the edit deleted the fence.
    fn shift(&mut self, input: &InputEdit, edit: &ByteEdit) -> bool {
        let (Some(start), Some(end)) = (
            map_boundary(self.start_byte, edit),
            map_boundary(self.end_byte, edit),
        ) else {
            return false;
        };
        if start >= end {
            return false;
        }
        self.start_byte = start;
        self.end_byte = end;
        self.tree.edit(input);
        true
    }
}

/// What a background parse produced for one buffer revision.
#[derive(Clone, Debug)]
pub struct Parsed {
    pub tree: HighlightTree,
    pub injections: Vec<Injection>,
}

/// The snapshot a worker parses. Equality ignores the tree; jobs are matched by revision.
#[derive(Clone, Debug)]
pub struct ParseInput {
    pub language: Language,
    pub generation: u64,
    pub version: u64,
    pub rope: Rope,
    pub old: Option<HighlightTree>,
}

impl PartialEq for ParseInput {
    fn eq(&self, other: &Self) -> bool {
        self.language == other.language
            && self.generation == other.generation
            && self.version == other.version
    }
}

impl Eq for ParseInput {}

/// Highlight state for one tab. `generation` changes when the buffer is replaced
/// or the language changes, so a parse of the previous buffer is ignored.
#[derive(Clone, Debug)]
pub struct SyntaxState {
    language: Language,
    generation: u64,
    version: u64,
    tree: Option<HighlightTree>,
    injections: Vec<Injection>,
    in_flight: bool,
    dirty: bool,
}

impl SyntaxState {
    pub fn fresh(language: Language, generation: u64, version: u64) -> Self {
        Self {
            language,
            generation,
            version,
            tree: None,
            injections: Vec::new(),
            in_flight: false,
            dirty: true,
        }
    }

    pub fn language(&self) -> Language {
        self.language
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn in_flight(&self) -> bool {
        self.in_flight
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn set_in_flight(&mut self, in_flight: bool) {
        self.in_flight = in_flight;
    }

    pub fn set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
    }

    pub fn set_version(&mut self, version: u64) {
        self.version = version;
    }

    pub fn snapshot(&self) -> Option<HighlightTree> {
        self.tree.clone()
    }

    pub fn apply_edits(&mut self, edits: &[ByteEdit]) {
        for edit in edits {
            let input = to_input(edit);
            if let Some(tree) = &mut self.tree {
                tree.edit(&input);
            }
            self.injections
                .retain_mut(|injection| injection.shift(&input, edit));
        }
    }

    pub fn install(&mut self, parsed: Parsed) {
        self.tree = Some(parsed.tree);
        self.injections = parsed.injections;
    }

    pub fn has_tree(&self) -> bool {
        self.tree.is_some()
    }

    /// Char index of the bracket paired with the one at `at`.
    /// A string or comment never matches. `<>` matches only when a node starts
    /// on one and ends on the other. Other brackets fall back to a rope scan
    /// when the tree has no pair.
    pub fn match_bracket(&self, rope: &Rope, at: usize) -> Option<usize> {
        if at >= rope.len_chars() {
            return None;
        }
        let bracket = rope.char(at);
        if !is_tree_bracket(bracket) {
            return None;
        }
        let byte = rope.char_to_byte(at);
        if let Some(mate) = self.structural_pair(rope, byte, bracket) {
            return byte_to_char(rope, mate);
        }
        if !is_plain_bracket(bracket) || self.in_literal(byte, bracket) {
            return None;
        }
        crate::buffer::scan_bracket(rope, at, bracket)
    }

    /// Char index of the bracket the tree pairs with the `()[]{}` bracket at `at`.
    /// `None` when there is no tree or the tree has no node from one to the other.
    pub fn tree_pair(&self, rope: &Rope, at: usize) -> Option<usize> {
        if at >= rope.len_chars() {
            return None;
        }
        let bracket = rope.char(at);
        if !is_plain_bracket(bracket) {
            return None;
        }
        let mate = self.structural_pair(rope, rope.char_to_byte(at), bracket)?;
        byte_to_char(rope, mate)
    }

    /// Char ranges of the strings, comments, and character literals that overlap
    /// chars `start..end`, sorted and merged. One walk of the tree over that range,
    /// so a caller can test many brackets without a tree lookup each. Empty without a tree.
    pub fn literal_ranges(&self, rope: &Rope, start: usize, end: usize) -> Vec<(usize, usize)> {
        let Some(tree) = self.tree.as_ref() else {
            return Vec::new();
        };
        let len = rope.len_chars();
        let start_byte = rope.char_to_byte(start.min(len));
        let end_byte = rope.char_to_byte(end.min(len));
        let mut bytes = Vec::new();
        match tree {
            HighlightTree::Code(tree) => {
                literal_nodes(tree.root_node(), start_byte, end_byte, &mut bytes);
            }
            HighlightTree::Markdown(tree) => {
                literal_nodes(
                    tree.block_tree().root_node(),
                    start_byte,
                    end_byte,
                    &mut bytes,
                );
                for inline in tree.inline_trees() {
                    literal_nodes(inline.root_node(), start_byte, end_byte, &mut bytes);
                }
            }
        }
        for injection in &self.injections {
            if injection.start_byte < end_byte && injection.end_byte > start_byte {
                literal_nodes(injection.tree.root_node(), start_byte, end_byte, &mut bytes);
            }
        }
        let mut ranges: Vec<(usize, usize)> = bytes
            .into_iter()
            .filter_map(|(from, to)| {
                let from = rope.try_byte_to_char(from.min(rope.len_bytes())).ok()?;
                let to = rope.try_byte_to_char(to.min(rope.len_bytes())).ok()?;
                (from < to).then_some((from, to))
            })
            .collect();
        ranges.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
        for (from, to) in ranges {
            match merged.last_mut() {
                Some(last) if from <= last.1 => last.1 = last.1.max(to),
                _ => merged.push((from, to)),
            }
        }
        merged
    }

    fn structural_pair(&self, rope: &Rope, byte: usize, bracket: char) -> Option<usize> {
        let tree = self.tree.as_ref()?;
        if let Some(injection) = self
            .injections
            .iter()
            .find(|injection| byte >= injection.start_byte && byte < injection.end_byte)
        {
            return pair_in(injection.tree.root_node(), rope, byte, bracket);
        }
        match tree {
            HighlightTree::Code(tree) => pair_in(tree.root_node(), rope, byte, bracket),
            HighlightTree::Markdown(tree) => {
                for inline in tree.inline_trees() {
                    let root = inline.root_node();
                    if byte >= root.start_byte()
                        && byte < root.end_byte()
                        && let Some(mate) = pair_in(root, rope, byte, bracket)
                    {
                        return Some(mate);
                    }
                }
                pair_in(tree.block_tree().root_node(), rope, byte, bracket)
            }
        }
    }

    fn in_literal(&self, byte: usize, bracket: char) -> bool {
        let Some(tree) = self.tree.as_ref() else {
            return false;
        };
        if let Some(injection) = self
            .injections
            .iter()
            .find(|injection| byte >= injection.start_byte && byte < injection.end_byte)
        {
            return node_in_literal(injection.tree.root_node(), byte, bracket);
        }
        match tree {
            HighlightTree::Code(tree) => node_in_literal(tree.root_node(), byte, bracket),
            HighlightTree::Markdown(tree) => {
                for inline in tree.inline_trees() {
                    let root = inline.root_node();
                    if byte >= root.start_byte() && byte < root.end_byte() {
                        return node_in_literal(root, byte, bracket);
                    }
                }
                node_in_literal(tree.block_tree().root_node(), byte, bracket)
            }
        }
    }
}

fn is_tree_bracket(ch: char) -> bool {
    is_plain_bracket(ch) || matches!(ch, '<' | '>')
}

fn is_plain_bracket(ch: char) -> bool {
    matches!(ch, '(' | ')' | '[' | ']' | '{' | '}')
}

fn closer_of(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '<' => Some('>'),
        _ => None,
    }
}

fn opener_of(close: char) -> Option<char> {
    match close {
        ')' => Some('('),
        ']' => Some('['),
        '}' => Some('{'),
        '>' => Some('<'),
        _ => None,
    }
}

fn pair_in(root: Node, rope: &Rope, byte: usize, bracket: char) -> Option<usize> {
    let end = byte + bracket.len_utf8();
    if byte < root.start_byte() || end > root.end_byte() {
        return None;
    }
    let mut current = root.descendant_for_byte_range(byte, end);
    while let Some(node) = current {
        if let Some(mate) = mate_byte(node, rope, byte, bracket) {
            return Some(mate);
        }
        current = node.parent();
    }
    None
}

fn mate_byte(node: Node, rope: &Rope, byte: usize, bracket: char) -> Option<usize> {
    let bracket_end = byte + bracket.len_utf8();
    if let Some(want_close) = closer_of(bracket)
        && node.start_byte() == byte
    {
        let (close_byte, close) = char_ending_at(rope, node.end_byte())?;
        if close == want_close && close_byte > byte {
            return Some(close_byte);
        }
    }
    if let Some(want_open) = opener_of(bracket)
        && node.end_byte() == bracket_end
    {
        let open = char_on_byte(rope, node.start_byte())?;
        if open == want_open && node.start_byte() < byte {
            return Some(node.start_byte());
        }
    }
    None
}

fn node_in_literal(root: Node, byte: usize, bracket: char) -> bool {
    let end = byte + bracket.len_utf8();
    if byte < root.start_byte() || end > root.end_byte() {
        return false;
    }
    let mut current = root.descendant_for_byte_range(byte, end);
    while let Some(node) = current {
        if literal_kind(node.kind()) {
            return true;
        }
        current = node.parent();
    }
    false
}

/// Push the byte ranges of literal nodes under `root` that overlap `start..end`,
/// without descending into a literal or into nodes outside the range.
fn literal_nodes(root: Node, start: usize, end: usize, out: &mut Vec<(usize, usize)>) {
    if root.start_byte() >= end || root.end_byte() <= start {
        return;
    }
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        let overlaps = node.start_byte() < end && node.end_byte() > start;
        if overlaps && literal_kind(node.kind()) {
            out.push((node.start_byte(), node.end_byte()));
        } else if overlaps && cursor.goto_first_child_for_byte(start).is_some() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() && cursor.node().start_byte() < end {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}

fn literal_kind(kind: &str) -> bool {
    let kind = kind.to_ascii_lowercase();
    kind.contains("comment")
        || kind.contains("string")
        || kind.contains("char_literal")
        || kind.contains("character")
}

fn char_on_byte(rope: &Rope, byte: usize) -> Option<char> {
    let index = rope.try_byte_to_char(byte).ok()?;
    if index >= rope.len_chars() || rope.char_to_byte(index) != byte {
        return None;
    }
    Some(rope.char(index))
}

fn char_ending_at(rope: &Rope, end_byte: usize) -> Option<(usize, char)> {
    if end_byte == 0 || end_byte > rope.len_bytes() {
        return None;
    }
    let end = rope.try_byte_to_char(end_byte).ok()?;
    if end == 0 {
        return None;
    }
    let index = end - 1;
    Some((rope.char_to_byte(index), rope.char(index)))
}

fn byte_to_char(rope: &Rope, byte: usize) -> Option<usize> {
    let index = rope.try_byte_to_char(byte).ok()?;
    if rope.char_to_byte(index) != byte {
        return None;
    }
    Some(index)
}

/// Parse `rope`, reusing `old` when it was built for this language and then edited.
/// `None` means the parser timed out or the text is too large.
pub fn parse(language: Language, rope: &Rope, old: Option<&HighlightTree>) -> Option<Parsed> {
    if rope.len_bytes() > MAX_HIGHLIGHT_BYTES {
        return None;
    }
    if language == Language::Markdown {
        return parse_markdown(rope, old);
    }
    let _ = code_query(language);
    let mut parser = Parser::new();
    parser.set_language(&grammar(language)).ok()?;
    let old_tree = match old {
        Some(HighlightTree::Code(tree)) => Some(tree),
        _ => None,
    };
    let mut callback = |byte: usize, _: TsPoint| chunk_at(rope, byte);
    let mut cancel = timeout_callback();
    let tree = parser.parse_with_options(
        &mut callback,
        old_tree,
        Some(tree_sitter::ParseOptions::new().progress_callback(&mut cancel)),
    )?;
    Some(Parsed {
        tree: HighlightTree::Code(tree),
        injections: Vec::new(),
    })
}

fn parse_markdown(rope: &Rope, old: Option<&HighlightTree>) -> Option<Parsed> {
    let _ = markdown_block_query();
    let _ = markdown_inline_query();
    let mut parser = MarkdownParser::default();
    let old_tree = match old {
        Some(HighlightTree::Markdown(tree)) => Some(tree),
        _ => None,
    };
    let mut callback = |byte: usize, _: TsPoint| chunk_at(rope, byte);
    let mut cancel_block = timeout_callback();
    let mut cancel_inline = timeout_callback();
    let tree = parser.parse_with_options(
        &mut callback,
        old_tree,
        MarkdownParseOptions::new(
            Some(tree_sitter::ParseOptions::new().progress_callback(&mut cancel_block)),
            Some(tree_sitter::ParseOptions::new().progress_callback(&mut cancel_inline)),
        ),
    )?;
    let injections = inject_fences(&tree, rope);
    Some(Parsed {
        tree: HighlightTree::Markdown(tree),
        injections,
    })
}

fn inject_fences(tree: &MarkdownTree, rope: &Rope) -> Vec<Injection> {
    let mut fences = Vec::new();
    let mut cursor = tree.block_tree().walk();
    collect_fences(&mut cursor, rope, &mut fences);
    let mut injections = Vec::new();
    for (language, start, end) in fences {
        if start >= end {
            continue;
        }
        if code_query(language).is_none() {
            continue;
        }
        let mut parser = Parser::new();
        if parser.set_language(&grammar(language)).is_err() {
            continue;
        }
        let node_range = tree_sitter::Range {
            start_byte: start,
            end_byte: end,
            start_point: point_at_byte(rope, start),
            end_point: point_at_byte(rope, end),
        };
        if parser.set_included_ranges(&[node_range]).is_err() {
            continue;
        }
        let mut callback = |byte: usize, _: TsPoint| chunk_at(rope, byte);
        let mut cancel = timeout_callback();
        let Some(parsed) = parser.parse_with_options(
            &mut callback,
            None,
            Some(tree_sitter::ParseOptions::new().progress_callback(&mut cancel)),
        ) else {
            continue;
        };
        injections.push(Injection {
            language,
            start_byte: start,
            end_byte: end,
            tree: parsed,
        });
    }
    injections
}

fn collect_fences(
    cursor: &mut tree_sitter::TreeCursor,
    rope: &Rope,
    out: &mut Vec<(Language, usize, usize)>,
) {
    let node = cursor.node();
    if node.kind() == "fenced_code_block"
        && let Some(language) = fence_language(node, rope)
        && let Some(content) = child_kind(node, "code_fence_content")
    {
        out.push((language, content.start_byte(), content.end_byte()));
    }
    if cursor.goto_first_child() {
        loop {
            collect_fences(cursor, rope, out);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

fn fence_language(node: Node, rope: &Rope) -> Option<Language> {
    let info = child_kind(node, "info_string")?;
    let text = node_text(rope, info)?;
    Language::from_info(&text)
}

fn child_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        if cursor.node().kind() == kind {
            return Some(cursor.node());
        }
        if !cursor.goto_next_sibling() {
            return None;
        }
    }
}

fn node_text(rope: &Rope, node: Node) -> Option<String> {
    let start = rope.try_byte_to_char(node.start_byte()).ok()?;
    let end = rope.try_byte_to_char(node.end_byte()).ok()?;
    Some(rope.slice(start..end).to_string())
}

/// Roles covering `start_line..end_line` (exclusive), in paint order.
/// Inner captures overwrite outer ones. Identical ranges keep the first pattern.
pub fn highlight_range(
    state: &SyntaxState,
    rope: &Rope,
    start_line: usize,
    end_line: usize,
) -> Vec<(usize, usize, SyntaxRole)> {
    let Some(tree) = &state.tree else {
        return Vec::new();
    };
    let last = rope.len_lines();
    let start_line = start_line.min(last);
    let end_line = end_line.max(start_line).min(last);
    let start_byte = line_byte(rope, start_line);
    let end_byte = line_byte(rope, end_line);
    let mut spans = Vec::new();
    match tree {
        HighlightTree::Code(tree) => {
            if let Some(query) = code_query(state.language) {
                collect(
                    query,
                    tree.root_node(),
                    rope,
                    start_byte,
                    end_byte,
                    &mut spans,
                );
            }
        }
        HighlightTree::Markdown(tree) => {
            if let Some(query) = markdown_block_query() {
                collect(
                    query,
                    tree.block_tree().root_node(),
                    rope,
                    start_byte,
                    end_byte,
                    &mut spans,
                );
            }
            if let Some(query) = markdown_inline_query() {
                for inline in tree.inline_trees() {
                    let root = inline.root_node();
                    if root.end_byte() < start_byte || root.start_byte() > end_byte {
                        continue;
                    }
                    collect(query, root, rope, start_byte, end_byte, &mut spans);
                }
            }
        }
    }
    for injection in &state.injections {
        if injection.end_byte < start_byte || injection.start_byte > end_byte {
            continue;
        }
        if let Some(query) = code_query(injection.language) {
            collect(
                query,
                injection.tree.root_node(),
                rope,
                start_byte,
                end_byte,
                &mut spans,
            );
        }
    }
    resolve(spans)
}

struct RawSpan {
    start: usize,
    end: usize,
    pattern: usize,
    role: SyntaxRole,
}

fn collect(
    query: &Query,
    node: Node,
    rope: &Rope,
    start_byte: usize,
    end_byte: usize,
    spans: &mut Vec<RawSpan>,
) {
    if start_byte >= end_byte {
        return;
    }
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(start_byte..end_byte);
    let mut matches = cursor.matches(query, node, RopeText(rope));
    while let Some(matched) = matches.next() {
        let pattern = matched.pattern_index;
        let captures: Vec<(usize, usize, usize)> = matched
            .captures()
            .iter()
            .map(|capture| {
                (
                    capture.node.start_byte(),
                    capture.node.end_byte(),
                    capture.index as usize,
                )
            })
            .collect();
        let names = query.capture_names();
        for (start_b, end_b, index) in captures {
            if start_b >= end_b {
                continue;
            }
            let Some(name) = names.get(index).copied() else {
                continue;
            };
            let Some(role) = role_for_capture(name) else {
                continue;
            };
            let (Ok(start), Ok(end)) =
                (rope.try_byte_to_char(start_b), rope.try_byte_to_char(end_b))
            else {
                continue;
            };
            if start >= end {
                continue;
            }
            spans.push(RawSpan {
                start,
                end,
                pattern,
                role,
            });
        }
    }
}

fn resolve(mut spans: Vec<RawSpan>) -> Vec<(usize, usize, SyntaxRole)> {
    spans.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then(b.end.cmp(&a.end))
            .then(a.pattern.cmp(&b.pattern))
    });
    let mut out = Vec::with_capacity(spans.len());
    for span in spans {
        if let Some(&(start, end, _)) = out.last()
            && start == span.start
            && end == span.end
        {
            continue;
        }
        out.push((span.start, span.end, span.role));
    }
    out
}

/// Maps a tree-sitter capture name onto a theme role.
/// The longest dotted prefix wins. Unknown names and `@none` paint nothing.
pub fn role_for_capture(name: &str) -> Option<SyntaxRole> {
    if name == "none" || name.starts_with("none.") {
        return None;
    }
    const MAP: &[(&str, SyntaxRole)] = &[
        ("variable.parameter", SyntaxRole::Parameter),
        ("variable.builtin", SyntaxRole::Builtin),
        ("variable", SyntaxRole::Variable),
        ("keyword", SyntaxRole::Keyword),
        ("conditional", SyntaxRole::Keyword),
        ("repeat", SyntaxRole::Keyword),
        ("include", SyntaxRole::Keyword),
        ("function.macro", SyntaxRole::Macro),
        ("function.builtin", SyntaxRole::Builtin),
        ("function", SyntaxRole::Function),
        ("method", SyntaxRole::Function),
        ("constructor", SyntaxRole::Function),
        ("type.builtin", SyntaxRole::Builtin),
        ("type.lifetime", SyntaxRole::Lifetime),
        ("type", SyntaxRole::Type),
        ("string.escape", SyntaxRole::Escape),
        ("string", SyntaxRole::String),
        ("character", SyntaxRole::String),
        ("escape", SyntaxRole::Escape),
        ("number", SyntaxRole::Number),
        ("float", SyntaxRole::Number),
        ("constant.builtin", SyntaxRole::Builtin),
        ("constant", SyntaxRole::Constant),
        ("boolean", SyntaxRole::Constant),
        ("property", SyntaxRole::Property),
        ("field", SyntaxRole::Property),
        ("operator", SyntaxRole::Operator),
        ("punctuation", SyntaxRole::Punctuation),
        ("tag", SyntaxRole::Tag),
        ("attribute", SyntaxRole::Attribute),
        ("label", SyntaxRole::Lifetime),
        ("comment.line.documentation", SyntaxRole::Documentation),
        ("comment.block.documentation", SyntaxRole::Documentation),
        ("comment.documentation", SyntaxRole::Documentation),
        ("comment", SyntaxRole::Comment),
        ("error", SyntaxRole::Error),
        ("text.title", SyntaxRole::Keyword),
        ("markup.heading", SyntaxRole::Keyword),
        ("text.literal", SyntaxRole::String),
        ("markup.raw", SyntaxRole::String),
        ("text.uri", SyntaxRole::Function),
        ("markup.link", SyntaxRole::Function),
        ("text.reference", SyntaxRole::Property),
    ];
    let mut best: Option<(usize, SyntaxRole)> = None;
    for &(key, role) in MAP {
        let matches = name == key
            || name
                .as_bytes()
                .get(key.len())
                .is_some_and(|byte| *byte == b'.' && name.starts_with(key));
        if matches && best.is_none_or(|(len, _)| key.len() > len) {
            best = Some((key.len(), role));
        }
    }
    best.map(|(_, role)| role)
}

fn grammar(language: Language) -> tree_sitter::Language {
    match language {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::Json => tree_sitter_json::LANGUAGE.into(),
        Language::Markdown => tree_sitter_md::LANGUAGE.into(),
        Language::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
        Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::Bash => tree_sitter_bash::LANGUAGE.into(),
        Language::Yaml => tree_sitter_yaml::LANGUAGE.into(),
        Language::Html => tree_sitter_html::LANGUAGE.into(),
        Language::Css => tree_sitter_css::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        Language::C => tree_sitter_c::LANGUAGE.into(),
        Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
        Language::Java => tree_sitter_java::LANGUAGE.into(),
        Language::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
        Language::Lua => tree_sitter_lua::LANGUAGE.into(),
        Language::Php => tree_sitter_php::LANGUAGE_PHP.into(),
        Language::Ruby => tree_sitter_ruby::LANGUAGE.into(),
        Language::Sql => tree_sitter_sequel::LANGUAGE.into(),
        Language::Elixir => tree_sitter_elixir::LANGUAGE.into(),
        Language::GraphQl => tree_sitter_graphql::LANGUAGE.into(),
        Language::Scala => tree_sitter_scala::LANGUAGE.into(),
        Language::Haskell => tree_sitter_haskell::LANGUAGE.into(),
        Language::Swift => tree_sitter_swift::LANGUAGE.into(),
        Language::Zig => tree_sitter_zig::LANGUAGE.into(),
        Language::Hcl => tree_sitter_hcl::LANGUAGE.into(),
        Language::PowerShell => tree_sitter_powershell::LANGUAGE.into(),
        Language::Xml => tree_sitter_xml::LANGUAGE_XML.into(),
        Language::Make => tree_sitter_make::LANGUAGE.into(),
        Language::Cmake => tree_sitter_cmake::LANGUAGE.into(),
    }
}

/// The published GraphQL crate ships a parser and no highlight query.
const GRAPHQL_HIGHLIGHTS: &str = r#"
[
  "query" "mutation" "subscription" "fragment" "on" "type" "interface"
  "union" "enum" "input" "scalar" "schema" "extend" "implements" "directive"
  "repeatable"
] @keyword

(string_value) @string
(named_type (name) @type)
(field_definition (name) @property)
"#;

/// The published HCL crate ships a parser and no highlight query.
const HCL_HIGHLIGHTS: &str = r#"
(attribute (identifier) @property)
(string_lit) @string
(numeric_lit) @number
(bool_lit) @constant.builtin
"#;

fn source_of(language: Language) -> &'static str {
    match language {
        Language::Rust => tree_sitter_rust::HIGHLIGHTS_QUERY,
        Language::Python => tree_sitter_python::HIGHLIGHTS_QUERY,
        Language::Json => tree_sitter_json::HIGHLIGHTS_QUERY,
        Language::Toml => tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
        Language::TypeScript | Language::Tsx => tree_sitter_typescript::HIGHLIGHTS_QUERY,
        Language::Bash => tree_sitter_bash::HIGHLIGHT_QUERY,
        Language::Yaml => tree_sitter_yaml::HIGHLIGHTS_QUERY,
        Language::Html => tree_sitter_html::HIGHLIGHTS_QUERY,
        Language::Css => tree_sitter_css::HIGHLIGHTS_QUERY,
        Language::JavaScript => tree_sitter_javascript::HIGHLIGHT_QUERY,
        Language::Markdown => "",
        Language::Go => tree_sitter_go::HIGHLIGHTS_QUERY,
        Language::C => tree_sitter_c::HIGHLIGHT_QUERY,
        Language::Cpp => tree_sitter_cpp::HIGHLIGHT_QUERY,
        Language::Java => tree_sitter_java::HIGHLIGHTS_QUERY,
        Language::CSharp => tree_sitter_c_sharp::HIGHLIGHTS_QUERY,
        Language::Lua => tree_sitter_lua::HIGHLIGHTS_QUERY,
        Language::Php => tree_sitter_php::HIGHLIGHTS_QUERY,
        Language::Ruby => tree_sitter_ruby::HIGHLIGHTS_QUERY,
        Language::Sql => tree_sitter_sequel::HIGHLIGHTS_QUERY,
        Language::Elixir => tree_sitter_elixir::HIGHLIGHTS_QUERY,
        Language::GraphQl => GRAPHQL_HIGHLIGHTS,
        Language::Scala => tree_sitter_scala::HIGHLIGHTS_QUERY,
        Language::Haskell => tree_sitter_haskell::HIGHLIGHTS_QUERY,
        Language::Swift => tree_sitter_swift::HIGHLIGHTS_QUERY,
        Language::Zig => tree_sitter_zig::HIGHLIGHTS_QUERY,
        Language::Hcl => HCL_HIGHLIGHTS,
        Language::PowerShell => tree_sitter_powershell::HIGHLIGHTS_QUERY,
        Language::Xml => tree_sitter_xml::XML_HIGHLIGHT_QUERY,
        Language::Make => tree_sitter_make::HIGHLIGHTS_QUERY,
        Language::Cmake => tree_sitter_cmake::HIGHLIGHTS_QUERY,
    }
}

fn code_query(language: Language) -> Option<&'static Query> {
    macro_rules! slot {
        ($name:ident) => {{
            static $name: OnceLock<Option<Query>> = OnceLock::new();
            $name.get_or_init(|| compile(language)).as_ref()
        }};
    }
    match language {
        Language::Rust => slot!(RUST),
        Language::Python => slot!(PYTHON),
        Language::Json => slot!(JSON),
        Language::Toml => slot!(TOML),
        Language::JavaScript => slot!(JAVASCRIPT),
        Language::TypeScript => slot!(TYPESCRIPT),
        Language::Tsx => slot!(TSX),
        Language::Bash => slot!(BASH),
        Language::Yaml => slot!(YAML),
        Language::Html => slot!(HTML),
        Language::Css => slot!(CSS),
        Language::Markdown => None,
        Language::Go => slot!(GO),
        Language::C => slot!(C),
        Language::Cpp => slot!(CPP),
        Language::Java => slot!(JAVA),
        Language::CSharp => slot!(CSHARP),
        Language::Lua => slot!(LUA),
        Language::Php => slot!(PHP),
        Language::Ruby => slot!(RUBY),
        Language::Sql => slot!(SQL),
        Language::Elixir => slot!(ELIXIR),
        Language::GraphQl => slot!(GRAPHQL),
        Language::Scala => slot!(SCALA),
        Language::Haskell => slot!(HASKELL),
        Language::Swift => slot!(SWIFT),
        Language::Zig => slot!(ZIG),
        Language::Hcl => slot!(HCL),
        Language::PowerShell => slot!(POWERSHELL),
        Language::Xml => slot!(XML),
        Language::Make => slot!(MAKE),
        Language::Cmake => slot!(CMAKE),
    }
}

fn compile(language: Language) -> Option<Query> {
    let owned;
    let text = match language {
        // TypeScript's query only adds types and modifiers. The keywords live in
        // the JavaScript query. JSX patterns are valid on the TSX grammar only.
        Language::JavaScript => {
            owned = format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
            );
            owned.as_str()
        }
        Language::TypeScript => {
            owned = format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            );
            owned.as_str()
        }
        Language::Tsx => {
            owned = format!(
                "{}\n{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            );
            owned.as_str()
        }
        _ => source_of(language),
    };
    Query::new(&grammar(language), text).ok()
}

fn line_byte(rope: &Rope, line: usize) -> usize {
    if line >= rope.len_lines() {
        rope.len_bytes()
    } else {
        rope.line_to_byte(line)
    }
}

fn markdown_block_query() -> Option<&'static Query> {
    static QUERY: OnceLock<Option<Query>> = OnceLock::new();
    QUERY
        .get_or_init(|| {
            Query::new(
                &tree_sitter_md::LANGUAGE.into(),
                tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
            )
            .ok()
        })
        .as_ref()
}

fn markdown_inline_query() -> Option<&'static Query> {
    static QUERY: OnceLock<Option<Query>> = OnceLock::new();
    QUERY
        .get_or_init(|| {
            Query::new(
                &tree_sitter_md::INLINE_LANGUAGE.into(),
                tree_sitter_md::HIGHLIGHT_QUERY_INLINE,
            )
            .ok()
        })
        .as_ref()
}

fn timeout_callback() -> impl FnMut(&tree_sitter::ParseState) -> std::ops::ControlFlow<()> {
    let started = std::time::Instant::now();
    move |_| {
        if started.elapsed().as_micros() > u128::from(PARSE_TIMEOUT_MICROS) {
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    }
}

fn chunk_at(rope: &Rope, byte: usize) -> &[u8] {
    if byte >= rope.len_bytes() {
        return b"";
    }
    let (chunk, chunk_byte, _, _) = rope.chunk_at_byte(byte);
    let bytes = chunk.as_bytes();
    let offset = byte - chunk_byte;
    if offset >= bytes.len() {
        b""
    } else {
        &bytes[offset..]
    }
}

fn point_at_byte(rope: &Rope, byte: usize) -> TsPoint {
    let byte = byte.min(rope.len_bytes());
    let char_idx = rope.try_byte_to_char(byte).unwrap_or(rope.len_chars());
    let row = rope.char_to_line(char_idx);
    let line_start = rope.line_to_char(row);
    TsPoint {
        row,
        column: rope.char_to_byte(char_idx) - rope.char_to_byte(line_start),
    }
}

fn to_input(edit: &ByteEdit) -> InputEdit {
    InputEdit {
        start_byte: edit.start_byte,
        old_end_byte: edit.old_end_byte,
        new_end_byte: edit.new_end_byte,
        start_position: TsPoint {
            row: edit.start.row,
            column: edit.start.column,
        },
        old_end_position: TsPoint {
            row: edit.old_end.row,
            column: edit.old_end.column,
        },
        new_end_position: TsPoint {
            row: edit.new_end.row,
            column: edit.new_end.column,
        },
    }
}

fn map_boundary(pos: usize, edit: &ByteEdit) -> Option<usize> {
    if pos < edit.start_byte {
        Some(pos)
    } else if pos >= edit.old_end_byte {
        Some(pos - edit.old_end_byte + edit.new_end_byte)
    } else {
        None
    }
}

struct RopeChunks<'a> {
    rope: &'a Rope,
    byte: usize,
    end: usize,
}

impl<'a> Iterator for RopeChunks<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.byte >= self.end || self.byte >= self.rope.len_bytes() {
            return None;
        }
        let (chunk, chunk_byte, _, _) = self.rope.chunk_at_byte(self.byte);
        let bytes = chunk.as_bytes();
        let offset = self.byte - chunk_byte;
        if offset >= bytes.len() {
            return None;
        }
        let mut slice = &bytes[offset..];
        let remaining = self.end - self.byte;
        if slice.len() > remaining {
            slice = &slice[..remaining];
        }
        if slice.is_empty() {
            return None;
        }
        self.byte += slice.len();
        Some(slice)
    }
}

struct RopeText<'a>(&'a Rope);

impl<'a> tree_sitter::TextProvider<&'a [u8]> for RopeText<'a> {
    type I = RopeChunks<'a>;

    fn text(&mut self, node: Node<'_>) -> Self::I {
        RopeChunks {
            rope: self.0,
            byte: node.start_byte(),
            end: node.end_byte(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{Buffer, Movement};

    fn slice_eq(
        rope: &Rope,
        span: &(usize, usize, SyntaxRole),
        text: &str,
        role: SyntaxRole,
    ) -> bool {
        span.2 == role && rope.slice(span.0..span.1).to_string() == text
    }

    fn painted(language: Language, text: &str) -> (Rope, Vec<(usize, usize, SyntaxRole)>) {
        let rope = Rope::from_str(text);
        let parsed = parse(language, &rope, None).expect("parse");
        let mut state = SyntaxState::fresh(language, 1, 0);
        state.install(parsed);
        let lines = rope.len_lines();
        let spans = highlight_range(&state, &rope, 0, lines);
        (rope, spans)
    }

    #[test]
    fn extensions_pick_a_language() {
        assert_eq!(
            Language::detect(Path::new("src/main.rs")),
            Some(Language::Rust)
        );
        assert_eq!(
            Language::detect(Path::new("app.py")),
            Some(Language::Python)
        );
        assert_eq!(
            Language::detect(Path::new("types.pyi")),
            Some(Language::Python)
        );
        assert_eq!(
            Language::detect(Path::new("note.md")),
            Some(Language::Markdown)
        );
        assert_eq!(
            Language::detect(Path::new("a.markdown")),
            Some(Language::Markdown)
        );
        assert_eq!(Language::detect(Path::new("c.json")), Some(Language::Json));
        assert_eq!(Language::detect(Path::new("c.jsonc")), Some(Language::Json));
        assert_eq!(
            Language::detect(Path::new("Cargo.toml")),
            Some(Language::Toml)
        );
        assert_eq!(Language::detect(Path::new("a.tsx")), Some(Language::Tsx));
        assert_eq!(
            Language::detect(Path::new("a.ts")),
            Some(Language::TypeScript)
        );
        assert_eq!(
            Language::detect(Path::new("a.jsx")),
            Some(Language::JavaScript)
        );
        assert_eq!(Language::detect(Path::new("run.sh")), Some(Language::Bash));
        assert_eq!(Language::detect(Path::new("a.yml")), Some(Language::Yaml));
        assert_eq!(Language::detect(Path::new("a.html")), Some(Language::Html));
        assert_eq!(Language::detect(Path::new("a.css")), Some(Language::Css));
        assert_eq!(Language::detect(Path::new("main.go")), Some(Language::Go));
        assert_eq!(Language::detect(Path::new("a.c")), Some(Language::C));
        assert_eq!(Language::detect(Path::new("a.h")), Some(Language::C));
        assert_eq!(Language::detect(Path::new("a.cpp")), Some(Language::Cpp));
        assert_eq!(Language::detect(Path::new("a.hpp")), Some(Language::Cpp));
        assert_eq!(Language::detect(Path::new("A.java")), Some(Language::Java));
        assert_eq!(Language::detect(Path::new("A.cs")), Some(Language::CSharp));
        assert_eq!(Language::detect(Path::new("a.lua")), Some(Language::Lua));
        assert_eq!(Language::detect(Path::new("a.php")), Some(Language::Php));
        assert_eq!(Language::detect(Path::new("a.rb")), Some(Language::Ruby));
        assert_eq!(Language::detect(Path::new("Gemfile")), Some(Language::Ruby));
        assert_eq!(Language::detect(Path::new("a.sql")), Some(Language::Sql));
        assert_eq!(Language::detect(Path::new("a.ex")), Some(Language::Elixir));
        assert_eq!(
            Language::detect(Path::new("a.graphql")),
            Some(Language::GraphQl)
        );
        assert_eq!(
            Language::detect(Path::new("a.scala")),
            Some(Language::Scala)
        );
        assert_eq!(Language::detect(Path::new("a.hs")), Some(Language::Haskell));
        assert_eq!(
            Language::detect(Path::new("a.swift")),
            Some(Language::Swift)
        );
        assert_eq!(Language::detect(Path::new("a.zig")), Some(Language::Zig));
        assert_eq!(Language::detect(Path::new("a.tf")), Some(Language::Hcl));
        assert_eq!(
            Language::detect(Path::new("a.ps1")),
            Some(Language::PowerShell)
        );
        assert_eq!(Language::detect(Path::new("a.xml")), Some(Language::Xml));
        assert_eq!(
            Language::detect(Path::new("Makefile")),
            Some(Language::Make)
        );
        assert_eq!(
            Language::detect(Path::new("CMakeLists.txt")),
            Some(Language::Cmake)
        );
        assert_eq!(Language::detect(Path::new("notes.txt")), None);
        assert_eq!(Language::from_info("go"), Some(Language::Go));
        assert_eq!(Language::from_info("c++"), Some(Language::Cpp));
        assert_eq!(Language::from_info("sql"), Some(Language::Sql));
        assert_eq!(Language::from_info("rust"), Some(Language::Rust));
        assert_eq!(Language::from_info("ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_info(""), None);
    }

    #[test]
    fn capture_names_map_by_longest_prefix() {
        assert_eq!(role_for_capture("keyword"), Some(SyntaxRole::Keyword));
        assert_eq!(
            role_for_capture("keyword.control"),
            Some(SyntaxRole::Keyword)
        );
        assert_eq!(role_for_capture("conditional"), Some(SyntaxRole::Keyword));
        assert_eq!(
            role_for_capture("function.method"),
            Some(SyntaxRole::Function)
        );
        assert_eq!(role_for_capture("constructor"), Some(SyntaxRole::Function));
        assert_eq!(role_for_capture("type.builtin"), Some(SyntaxRole::Builtin));
        assert_eq!(
            role_for_capture("string.special.key"),
            Some(SyntaxRole::String)
        );
        assert_eq!(role_for_capture("escape"), Some(SyntaxRole::Escape));
        assert_eq!(role_for_capture("string.escape"), Some(SyntaxRole::Escape));
        assert_eq!(role_for_capture("function.macro"), Some(SyntaxRole::Macro));
        assert_eq!(role_for_capture("number"), Some(SyntaxRole::Number));
        assert_eq!(role_for_capture("float"), Some(SyntaxRole::Number));
        assert_eq!(
            role_for_capture("constant.builtin"),
            Some(SyntaxRole::Builtin)
        );
        assert_eq!(role_for_capture("boolean"), Some(SyntaxRole::Constant));
        assert_eq!(
            role_for_capture("variable.parameter"),
            Some(SyntaxRole::Parameter)
        );
        assert_eq!(role_for_capture("variable"), Some(SyntaxRole::Variable));
        assert_eq!(
            role_for_capture("variable.builtin"),
            Some(SyntaxRole::Builtin)
        );
        assert_eq!(role_for_capture("property"), Some(SyntaxRole::Property));
        assert_eq!(role_for_capture("field"), Some(SyntaxRole::Property));
        assert_eq!(
            role_for_capture("punctuation.special"),
            Some(SyntaxRole::Punctuation)
        );
        assert_eq!(role_for_capture("tag"), Some(SyntaxRole::Tag));
        assert_eq!(role_for_capture("attribute"), Some(SyntaxRole::Attribute));
        assert_eq!(role_for_capture("label"), Some(SyntaxRole::Lifetime));
        assert_eq!(
            role_for_capture("comment.documentation"),
            Some(SyntaxRole::Documentation)
        );
        assert_eq!(role_for_capture("text.title"), Some(SyntaxRole::Keyword));
        assert_eq!(
            role_for_capture("markup.heading"),
            Some(SyntaxRole::Keyword)
        );
        assert_eq!(role_for_capture("text.literal"), Some(SyntaxRole::String));
        assert_eq!(role_for_capture("markup.raw"), Some(SyntaxRole::String));
        assert_eq!(role_for_capture("text.uri"), Some(SyntaxRole::Function));
        assert_eq!(
            role_for_capture("markup.link.url"),
            Some(SyntaxRole::Function)
        );
        assert_eq!(
            role_for_capture("text.reference"),
            Some(SyntaxRole::Property)
        );
        assert_eq!(role_for_capture("none"), None);
        assert_eq!(role_for_capture("spell"), None);
    }

    #[test]
    fn rust_keywords_and_strings_take_their_roles() {
        let (rope, spans) = painted(Language::Rust, "fn main() { \"hi\" }\n");
        assert!(
            spans
                .iter()
                .any(|span| slice_eq(&rope, span, "fn", SyntaxRole::Keyword))
        );
        assert!(
            spans
                .iter()
                .any(|span| slice_eq(&rope, span, "\"hi\"", SyntaxRole::String))
        );
    }

    #[test]
    fn python_def_is_a_keyword() {
        let (rope, spans) = painted(Language::Python, "def greet():\n    return 1\n");
        assert!(
            spans
                .iter()
                .any(|span| slice_eq(&rope, span, "def", SyntaxRole::Keyword))
        );
    }

    #[test]
    fn json_key_is_a_string() {
        let (rope, spans) = painted(Language::Json, "{\"name\": 1}\n");
        assert!(
            spans.iter().any(|span| {
                let text = rope.slice(span.0..span.1).to_string();
                text.contains("name") && matches!(span.2, SyntaxRole::String | SyntaxRole::Property)
            }),
            "{spans:?}"
        );
    }

    #[test]
    fn markdown_heading_is_a_keyword() {
        let (rope, spans) = painted(Language::Markdown, "# Title\n\nhello\n");
        assert!(
            spans.iter().any(|span| {
                rope.slice(span.0..span.1).to_string().contains("Title")
                    && span.2 == SyntaxRole::Keyword
            }),
            "{spans:?} text {}",
            rope
        );
    }

    #[test]
    fn markdown_rust_fence_is_highlighted() {
        let text = "# T\n\n```rust\nfn main() {}\n```\n";
        let (rope, spans) = painted(Language::Markdown, text);
        assert!(
            spans
                .iter()
                .any(|span| slice_eq(&rope, span, "fn", SyntaxRole::Keyword)),
            "{spans:?}"
        );
    }

    #[test]
    fn more_languages_paint_a_token() {
        let samples = [
            (
                Language::JavaScript,
                "function f() { return 1 }\n",
                "function",
                SyntaxRole::Keyword,
            ),
            (
                Language::TypeScript,
                "function f(): number { return 1 }\n",
                "function",
                SyntaxRole::Keyword,
            ),
            (
                Language::Tsx,
                "function f() { return 1 }\n",
                "function",
                SyntaxRole::Keyword,
            ),
            (Language::Bash, "echo hello\n", "echo", SyntaxRole::Function),
            (Language::Yaml, "name: duckx\n", "name", SyntaxRole::String),
            (Language::Html, "<p>hi</p>\n", "p", SyntaxRole::Tag),
            (
                Language::Css,
                "a { color: red; }\n",
                "color",
                SyntaxRole::Property,
            ),
            (
                Language::Toml,
                "name = \"duckx\"\n",
                "\"duckx\"",
                SyntaxRole::String,
            ),
            (
                Language::Go,
                "func main() {}\n",
                "func",
                SyntaxRole::Keyword,
            ),
            (
                Language::C,
                "int main() { return 0; }\n",
                "return",
                SyntaxRole::Keyword,
            ),
            (
                Language::Cpp,
                "class Foo {};\n",
                "class",
                SyntaxRole::Keyword,
            ),
            (
                Language::Java,
                "class Foo {}\n",
                "class",
                SyntaxRole::Keyword,
            ),
            (
                Language::CSharp,
                "class Foo {}\n",
                "class",
                SyntaxRole::Keyword,
            ),
            (
                Language::Lua,
                "function f() end\n",
                "function",
                SyntaxRole::Keyword,
            ),
            (
                Language::Php,
                "<?php function f() {}\n",
                "function",
                SyntaxRole::Keyword,
            ),
            (Language::Ruby, "def f\nend\n", "def", SyntaxRole::Keyword),
            (Language::Sql, "SELECT 1\n", "SELECT", SyntaxRole::Keyword),
            (
                Language::Elixir,
                "def hello do\nend\n",
                "do",
                SyntaxRole::Keyword,
            ),
            (
                Language::GraphQl,
                "type Query { name: String }\n",
                "type",
                SyntaxRole::Keyword,
            ),
            (Language::Scala, "class Foo\n", "class", SyntaxRole::Keyword),
            (
                Language::Haskell,
                "data Foo = Foo\n",
                "data",
                SyntaxRole::Keyword,
            ),
            (
                Language::Swift,
                "func f() {}\n",
                "func",
                SyntaxRole::Keyword,
            ),
            (
                Language::Zig,
                "pub fn main() void {}\n",
                "fn",
                SyntaxRole::Keyword,
            ),
            (
                Language::Hcl,
                "name = \"duckx\"\n",
                "name",
                SyntaxRole::Property,
            ),
            (
                Language::PowerShell,
                "function F {}\n",
                "function",
                SyntaxRole::Keyword,
            ),
            (Language::Xml, "<p>hi</p>\n", "p", SyntaxRole::Tag),
            (
                Language::Make,
                "all:\n\techo hi\n",
                "all",
                SyntaxRole::Constant,
            ),
            (
                Language::Cmake,
                "project(duckx)\n",
                "project",
                SyntaxRole::Function,
            ),
        ];
        let mut missed = Vec::new();
        for (language, text, token, role) in samples {
            let (rope, spans) = painted(language, text);
            if !spans.iter().any(|span| slice_eq(&rope, span, token, role)) {
                missed.push(format!("{language:?} {token} as {role:?}: {spans:?}"));
            }
        }
        assert!(missed.is_empty(), "{}", missed.join("\n"));
    }

    #[test]
    fn matching_bracket_uses_the_tree_and_skips_literals() {
        let call = "fn f(a) {}\n";
        let open = call.find('(').unwrap();
        let close = call.find(')').unwrap();
        assert_eq!(jump(Language::Rust, call, open), Some(close));
        assert_eq!(jump(Language::Rust, call, close), Some(open));

        let nested = "fn f() { { let x = (1); } }\n";
        let paren = nested.find('(').unwrap();
        assert_eq!(
            jump(Language::Rust, nested, paren),
            Some(nested.find(')').unwrap())
        );
        let first_brace = nested.find('{').unwrap();
        assert_eq!(
            jump(Language::Rust, nested, first_brace),
            Some(nested.rfind('}').unwrap())
        );

        let text = "fn f() {\n    let s = \"(\";\n    // (\n    let n = (1);\n}\n";
        let in_string = text.find("\"(").unwrap() + 1;
        assert_eq!(jump(Language::Rust, text, in_string), None);
        let in_comment = text.find("// (").unwrap() + 3;
        assert_eq!(jump(Language::Rust, text, in_comment), None);
        let real = text.find("(1)").unwrap();
        assert_eq!(jump(Language::Rust, text, real), Some(real + 2));

        let compare = "fn f() { let n = a < b; }\n";
        assert_eq!(
            jump(Language::Rust, compare, compare.find('<').unwrap()),
            None
        );
        let generic = "fn f() { let v: Vec<u8> = Vec::new(); }\n";
        let angle = generic.find('<').unwrap();
        assert_eq!(
            jump(Language::Rust, generic, angle),
            Some(generic.find('>').unwrap())
        );

        let fence = "```rust\nfn f(a) {}\n```\n";
        let fenced = fence.find('(').unwrap();
        assert_eq!(
            jump(Language::Markdown, fence, fenced),
            Some(fence.find(')').unwrap())
        );
    }

    fn jump(language: Language, text: &str, at: usize) -> Option<usize> {
        let rope = Rope::from_str(text);
        let parsed = parse(language, &rope, None).expect("parse");
        let mut state = SyntaxState::fresh(language, 1, 0);
        state.install(parsed);
        state.match_bracket(&rope, at)
    }

    #[test]
    fn an_edited_tree_reparses_like_a_fresh_parse() {
        let mut buf = Buffer::from_text("fn main() {}\n");
        let first = parse(Language::Rust, buf.rope(), None).expect("parse");
        let at = "fn main() {".chars().count();
        for _ in 0..at {
            buf.move_cursor(Movement::Right, false);
        }
        buf.insert_text("let x = 1; ");
        let (_, edits) = buf.take_edits();
        let mut tree = first.tree.clone();
        for edit in &edits {
            tree.edit(&to_input(edit));
        }
        let reparsed = parse(Language::Rust, buf.rope(), Some(&tree)).expect("reparse");
        let fresh = parse(Language::Rust, buf.rope(), None).expect("fresh");
        assert_eq!(reparsed.tree.sexp(), fresh.tree.sexp());
    }
}
