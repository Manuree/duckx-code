//! Workspace file picker.
//!
//! The file list is walked off the UI thread with the `ignore` crate (gitignore,
//! hidden files, no symlink follow), same policy as the explorer. Matching does
//! not reuse another editor's scorer: each whitespace-separated word must align
//! as a subsequence, exclusions are literal substrings, and a hit inside the
//! file name outranks the same hit in a directory. An empty query lists recently
//! opened files first, then the rest by file name.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use unicode_width::UnicodeWidthStr;

/// Files kept from one walk. Past this, the picker says the list was cut short.
pub const MAX_INDEXED: usize = 100_000;

const MODAL_ROWS: u16 = 18;
const NEG: i32 = i32::MIN / 4;

/// One workspace file. `relative` uses `/` separators. `name_at` is the byte
/// where the file name starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedFile {
    pub relative: String,
    pub name_at: u32,
}

/// Modal state. The catalog itself lives on `App`, because the walk result is
/// shared with the background filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePicker {
    pub query: String,
    /// Byte offset on a char boundary.
    pub cursor: usize,
    pub selected: usize,
    pub scroll: usize,
    /// Indexes into the catalog, best match first.
    pub rows: Vec<u32>,
    pub total: usize,
    pub scanning: bool,
    pub refreshing: bool,
    /// The walk stopped at [`MAX_INDEXED`].
    pub truncated: bool,
    pub(crate) pending_generation: u64,
    /// Relative path to keep highlighted when a refresh replaces the rows.
    pub(crate) preserve: Option<String>,
}

impl Default for FilePicker {
    fn default() -> Self {
        Self {
            query: String::new(),
            cursor: 0,
            selected: 0,
            scroll: 0,
            rows: Vec::new(),
            total: 0,
            scanning: true,
            refreshing: false,
            truncated: false,
            pending_generation: 0,
            preserve: None,
        }
    }
}

impl FilePicker {
    /// Query edits return true so the caller can schedule a new filter.
    pub fn edit(&mut self, action: crate::action::PickerAction, page: usize) -> bool {
        use crate::action::PickerAction;
        match action {
            PickerAction::Insert(text) => self.insert(&text),
            PickerAction::Backspace => self.backspace(),
            PickerAction::DeleteForward => self.delete_forward(),
            PickerAction::DeleteWordBack => self.delete_word_back(),
            PickerAction::CursorLeft => {
                self.move_cursor(-1);
                false
            }
            PickerAction::CursorRight => {
                self.move_cursor(1);
                false
            }
            PickerAction::CursorWordLeft => {
                self.move_word(false);
                false
            }
            PickerAction::CursorWordRight => {
                self.move_word(true);
                false
            }
            PickerAction::CursorHome => {
                self.cursor = 0;
                false
            }
            PickerAction::CursorEnd => {
                self.cursor = self.query.len();
                false
            }
            PickerAction::SelectPrev => {
                self.nudge(-1, page);
                false
            }
            PickerAction::SelectNext => {
                self.nudge(1, page);
                false
            }
            PickerAction::PageUp => {
                self.nudge(-(page.max(1) as isize), page);
                false
            }
            PickerAction::PageDown => {
                self.nudge(page.max(1) as isize, page);
                false
            }
            PickerAction::First => {
                self.jump(false, page);
                false
            }
            PickerAction::Last => {
                self.jump(true, page);
                false
            }
            PickerAction::Submit | PickerAction::Close => false,
        }
    }

    pub fn reveal(&mut self, page: usize) {
        let page = page.max(1);
        if self.rows.is_empty() {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        if self.selected >= self.rows.len() {
            self.selected = self.rows.len() - 1;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + page {
            self.scroll = self.selected + 1 - page;
        }
    }

    fn insert(&mut self, text: &str) -> bool {
        let text: String = text.chars().filter(|ch| !ch.is_control()).collect();
        if text.is_empty() || self.query.len() + text.len() > 512 {
            return false;
        }
        self.query.insert_str(self.cursor, &text);
        self.cursor += text.len();
        true
    }

    fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let previous = self.query[..self.cursor].chars().next_back().unwrap();
        self.cursor -= previous.len_utf8();
        self.query.remove(self.cursor);
        true
    }

    fn delete_forward(&mut self) -> bool {
        let Some(next) = self.query[self.cursor..].chars().next() else {
            return false;
        };
        self.query
            .replace_range(self.cursor..self.cursor + next.len_utf8(), "");
        true
    }

    fn delete_word_back(&mut self) -> bool {
        let end = self.cursor;
        self.move_word(false);
        if self.cursor == end {
            return false;
        }
        self.query.replace_range(self.cursor..end, "");
        true
    }

    fn move_cursor(&mut self, delta: isize) {
        if delta < 0 {
            if let Some(previous) = self.query[..self.cursor].chars().next_back() {
                self.cursor -= previous.len_utf8();
            }
        } else if let Some(next) = self.query[self.cursor..].chars().next() {
            self.cursor += next.len_utf8();
        }
    }

    fn move_word(&mut self, right: bool) {
        let chars: Vec<char> = self.query.chars().collect();
        let mut index = self.query[..self.cursor].chars().count();
        let word = |ch: char| ch.is_alphanumeric() || ch == '_';
        if right {
            index = skip(&chars, index, true, |ch| !word(ch));
            index = skip(&chars, index, true, word);
        } else {
            index = skip(&chars, index, false, |ch| !word(ch));
            index = skip(&chars, index, false, word);
        }
        self.cursor = chars[..index].iter().map(|ch| ch.len_utf8()).sum();
    }

    fn nudge(&mut self, delta: isize, page: usize) {
        if self.rows.is_empty() {
            self.selected = 0;
            self.scroll = 0;
            return;
        }
        let last = self.rows.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
        self.reveal(page);
    }

    fn jump(&mut self, last: bool, page: usize) {
        if self.rows.is_empty() {
            return;
        }
        self.selected = if last { self.rows.len() - 1 } else { 0 };
        self.reveal(page);
    }
}

fn skip(chars: &[char], mut index: usize, right: bool, pred: impl Fn(char) -> bool) -> usize {
    if right {
        while index < chars.len() && pred(chars[index]) {
            index += 1;
        }
    } else {
        while index > 0 && pred(chars[index - 1]) {
            index -= 1;
        }
    }
    index
}

/// Height of the picker panel for a terminal area of `area_height` rows.
pub fn modal_height(area_height: u16) -> u16 {
    if area_height < 6 {
        return area_height;
    }
    MODAL_ROWS.min(area_height.saturating_sub(2)).max(6)
}

/// File rows inside the panel. The border takes two rows and the query takes one.
pub fn list_rows(area_height: u16) -> usize {
    modal_height(area_height).saturating_sub(3).max(1) as usize
}

/// Byte range of `query` that fits in `columns`, keeping the cursor on screen.
/// When the cursor is at the end, one column is left blank for the block cursor.
pub fn query_window(query: &str, cursor: usize, columns: usize) -> (usize, usize) {
    let cursor = floor_boundary(query, cursor.min(query.len()));
    if columns == 0 {
        return (cursor, cursor);
    }
    let blank = usize::from(cursor == query.len());
    if query.width() + blank <= columns {
        return (0, query.len());
    }
    let next_width = query[cursor..]
        .chars()
        .next()
        .map(|ch| unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0))
        .unwrap_or(0);
    let text_budget = columns.saturating_sub(blank);
    let prefix_budget = text_budget.saturating_sub(next_width);
    let start = if query[..cursor].width() > prefix_budget {
        suffix_start(&query[..cursor], prefix_budget)
    } else {
        0
    };
    let mut used = query[start..cursor].width() + next_width;
    let base = (cursor
        + query[cursor..]
            .chars()
            .next()
            .map(|ch| ch.len_utf8())
            .unwrap_or(0))
    .min(query.len());
    if used > text_budget {
        return (start, cursor);
    }
    let mut end = base;
    for (offset, ch) in query[base..].char_indices() {
        let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + width > text_budget {
            break;
        }
        used += width;
        end = base + offset + ch.len_utf8();
    }
    (start, end)
}

fn floor_boundary(text: &str, index: usize) -> usize {
    crate::utf8::floor_char_boundary(text, index)
}

fn suffix_start(text: &str, columns: usize) -> usize {
    if text.width() <= columns {
        return 0;
    }
    let mut start = 0;
    let mut width = text.width();
    for (index, ch) in text.char_indices() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        start = index + ch.len_utf8();
        width = width.saturating_sub(ch_width);
        if width <= columns {
            break;
        }
    }
    start
}

pub fn absolute_path(root: &Path, relative: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for part in relative.split('/') {
        if !part.is_empty() {
            path.push(part);
        }
    }
    path
}

/// Blocking walk. Call it from a background job.
pub fn index_workspace(root: &Path) -> (Vec<IndexedFile>, bool) {
    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .require_git(false)
        .follow_links(false)
        .build();
    let mut files = Vec::new();
    let mut truncated = false;
    for entry in walker {
        let Ok(entry) = entry else {
            continue;
        };
        if entry.depth() == 0 || !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        if files.len() >= MAX_INDEXED {
            truncated = true;
            break;
        }
        let Some(relative) = crate::services::tree::relative_from(root, entry.path()) else {
            continue;
        };
        let relative = forward_slash(&relative);
        if relative.is_empty() {
            continue;
        }
        let name_at = relative.rfind('/').map(|index| index + 1).unwrap_or(0) as u32;
        files.push(IndexedFile { relative, name_at });
    }
    (files, truncated)
}

fn forward_slash(path: &Path) -> String {
    let mut out = String::new();
    for (index, component) in path.components().enumerate() {
        if index > 0 {
            out.push('/');
        }
        out.push_str(&component.as_os_str().to_string_lossy());
    }
    out
}

/// Best-first catalog indexes. `recent` is most-recent first. `active_dir` is
/// the workspace-relative directory of the focused file, using `/`.
pub fn filter(files: &[IndexedFile], query: &str, recent: &[String], active_dir: &str) -> Vec<u32> {
    let atoms = parse_query(query);
    if atoms.is_empty() {
        return empty_order(files, recent);
    }
    let rank: HashMap<&str, usize> = recent
        .iter()
        .enumerate()
        .map(|(index, path)| (path.as_str(), index))
        .collect();
    SCRATCH.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        let mut scored = Vec::new();
        for (index, file) in files.iter().enumerate() {
            let Some(score) = score_file(file, &atoms, &rank, active_dir, &mut scratch) else {
                continue;
            };
            scored.push((score, index as u32));
        }
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| {
                    files[a.1 as usize]
                        .relative
                        .len()
                        .cmp(&files[b.1 as usize].relative.len())
                })
                .then_with(|| {
                    files[a.1 as usize]
                        .relative
                        .cmp(&files[b.1 as usize].relative)
                })
        });
        scored.into_iter().map(|(_, index)| index).collect()
    })
}

/// Byte offsets of the characters the query aligned, for the rows on screen.
pub fn highlight(path: &str, name_at: u32, query: &str) -> Vec<u32> {
    let atoms = parse_query(query);
    if atoms.is_empty() || name_at as usize > path.len() {
        return Vec::new();
    }
    SCRATCH.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        let mut hits = Vec::new();
        let name = &path[name_at as usize..];
        for atom in &atoms {
            if atom.exclude {
                continue;
            }
            let mut local = Vec::new();
            let matched = if atom.path_shaped {
                align(
                    path,
                    &atom.text,
                    atom.sensitive,
                    &mut scratch,
                    Some(&mut local),
                )
                .is_some()
            } else if align(
                name,
                &atom.text,
                atom.sensitive,
                &mut scratch,
                Some(&mut local),
            )
            .is_some()
            {
                for hit in &mut local {
                    *hit += name_at;
                }
                true
            } else {
                local.clear();
                align(
                    path,
                    &atom.text,
                    atom.sensitive,
                    &mut scratch,
                    Some(&mut local),
                )
                .is_some()
            };
            if matched {
                hits.append(&mut local);
            }
        }
        hits.sort_unstable();
        hits.dedup();
        hits
    })
}

fn empty_order(files: &[IndexedFile], recent: &[String]) -> Vec<u32> {
    let rank: HashMap<&str, usize> = recent
        .iter()
        .enumerate()
        .map(|(index, path)| (path.as_str(), index))
        .collect();
    let mut indices: Vec<u32> = (0..files.len() as u32).collect();
    indices.sort_by(|&a, &b| {
        let left = &files[a as usize];
        let right = &files[b as usize];
        let left_rank = rank
            .get(left.relative.as_str())
            .copied()
            .unwrap_or(usize::MAX);
        let right_rank = rank
            .get(right.relative.as_str())
            .copied()
            .unwrap_or(usize::MAX);
        left_rank.cmp(&right_rank).then_with(|| {
            cmp_name(file_name(&left.relative), file_name(&right.relative))
                .then_with(|| left.relative.cmp(&right.relative))
        })
    });
    indices
}

fn file_name(relative: &str) -> &str {
    relative.rsplit('/').next().unwrap_or(relative)
}

fn cmp_name(left: &str, right: &str) -> Ordering {
    let mut left_chars = left.chars().map(|ch| ch.to_ascii_lowercase());
    let mut right_chars = right.chars().map(|ch| ch.to_ascii_lowercase());
    loop {
        match (left_chars.next(), right_chars.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left), Some(right)) => match left.cmp(&right) {
                Ordering::Equal => {}
                order => return order,
            },
        }
    }
}

struct Atom {
    text: String,
    sensitive: bool,
    exclude: bool,
    path_shaped: bool,
}

fn parse_query(query: &str) -> Vec<Atom> {
    query
        .split_whitespace()
        .filter_map(|raw| {
            let (exclude, body) = match raw.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => (false, raw),
            };
            if body.is_empty() {
                return None;
            }
            let text = body.replace('\\', "/");
            let sensitive = text.chars().any(|ch| ch.is_uppercase());
            let path_shaped = text.contains('/');
            Some(Atom {
                text,
                sensitive,
                exclude,
                path_shaped,
            })
        })
        .collect()
}

fn score_file(
    file: &IndexedFile,
    atoms: &[Atom],
    recent: &HashMap<&str, usize>,
    active_dir: &str,
    scratch: &mut Scratch,
) -> Option<i32> {
    let path = file.relative.as_str();
    let name_at = file.name_at as usize;
    if name_at > path.len() {
        return None;
    }
    let name = &path[name_at..];
    let mut score: i32 = 0;
    let mut positive = false;
    for atom in atoms {
        if atom.exclude {
            if contains_literal(path, &atom.text, atom.sensitive) {
                return None;
            }
            continue;
        }
        positive = true;
        let field = if atom.path_shaped {
            align(path, &atom.text, atom.sensitive, scratch, None)?
        } else if let Some(name_score) = align(name, &atom.text, atom.sensitive, scratch, None) {
            let mut name_score = name_score.saturating_mul(2).saturating_add(96);
            if equals_folded(name, &atom.text, atom.sensitive) {
                name_score += 500;
            } else if starts_folded(name, &atom.text, atom.sensitive) {
                name_score += 160;
            }
            if !atom.sensitive && name.contains(&atom.text) {
                name_score += 48;
            }
            name_score
        } else {
            let mut path_score = align(path, &atom.text, atom.sensitive, scratch, None)?;
            if !atom.sensitive && path.contains(&atom.text) {
                path_score += 24;
            }
            path_score
        };
        score = score.saturating_add(field);
    }
    if !positive {
        score = 1;
    }
    if let Some(&position) = recent.get(path) {
        score = score.saturating_add(56_i32.saturating_sub(position as i32 * 2).max(8));
    }
    if parent_dir(path) == active_dir {
        score = score.saturating_add(36);
    }
    Some(score)
}

fn parent_dir(relative: &str) -> &str {
    match relative.rfind('/') {
        Some(index) => &relative[..index],
        None => "",
    }
}

fn contains_literal(hay: &str, needle: &str, sensitive: bool) -> bool {
    if needle.is_empty() {
        return true;
    }
    if sensitive {
        return hay.contains(needle);
    }
    if hay.is_ascii() && needle.is_ascii() {
        let needle = needle.as_bytes();
        return hay
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle));
    }
    hay.to_lowercase().contains(&needle.to_lowercase())
}

fn starts_folded(hay: &str, needle: &str, sensitive: bool) -> bool {
    if sensitive {
        return hay.starts_with(needle);
    }
    let mut hay = hay.chars();
    for needle in needle.chars() {
        match hay.next() {
            Some(ch) if chars_eq(ch, needle, false) => {}
            _ => return false,
        }
    }
    true
}

fn equals_folded(hay: &str, needle: &str, sensitive: bool) -> bool {
    starts_folded(hay, needle, sensitive) && hay.chars().count() == needle.chars().count()
}

fn chars_eq(left: char, right: char, sensitive: bool) -> bool {
    if left == right {
        return true;
    }
    !sensitive
        && (left.eq_ignore_ascii_case(&right)
            || (!left.is_ascii() && left.to_lowercase().eq(right.to_lowercase())))
}

fn align(
    hay: &str,
    needle: &str,
    sensitive: bool,
    scratch: &mut Scratch,
    hits: Option<&mut Vec<u32>>,
) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    if hay.is_ascii() && needle.is_ascii() {
        return align_bytes(hay.as_bytes(), needle.as_bytes(), sensitive, scratch, hits);
    }
    let hay_chars: Vec<char> = hay.chars().collect();
    let needle_chars: Vec<char> = needle.chars().collect();
    if !subsequence_chars(&hay_chars, &needle_chars, sensitive) {
        return None;
    }
    let bytes: Vec<u32> = hay.char_indices().map(|(index, _)| index as u32).collect();
    align_seq(
        hay_chars.len(),
        needle_chars.len(),
        |hay_index, needle_index| {
            chars_eq(hay_chars[hay_index], needle_chars[needle_index], sensitive)
        },
        |hay_index| boundary_char(&hay_chars, hay_index),
        |hay_index| bytes[hay_index],
        scratch,
        hits,
    )
}

fn align_bytes(
    hay: &[u8],
    needle: &[u8],
    sensitive: bool,
    scratch: &mut Scratch,
    hits: Option<&mut Vec<u32>>,
) -> Option<i32> {
    if !subsequence_bytes(hay, needle, sensitive) {
        return None;
    }
    align_seq(
        hay.len(),
        needle.len(),
        |hay_index, needle_index| eq_byte(hay[hay_index], needle[needle_index], sensitive),
        |hay_index| boundary_byte(hay, hay_index),
        |hay_index| hay_index as u32,
        scratch,
        hits,
    )
}

fn subsequence_bytes(hay: &[u8], needle: &[u8], sensitive: bool) -> bool {
    let mut from = 0;
    for &needle in needle {
        let mut found = false;
        while from < hay.len() {
            let hay_byte = hay[from];
            from += 1;
            if eq_byte(hay_byte, needle, sensitive) {
                found = true;
                break;
            }
        }
        if !found {
            return false;
        }
    }
    true
}

fn eq_byte(hay: u8, needle: u8, sensitive: bool) -> bool {
    if sensitive {
        hay == needle
    } else {
        hay.to_ascii_lowercase() == needle.to_ascii_lowercase()
    }
}

fn boundary_byte(hay: &[u8], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    let previous = hay[index - 1];
    matches!(previous, b'/' | b'_' | b'-' | b'.' | b' ')
        || (previous.is_ascii_lowercase() && hay[index].is_ascii_uppercase())
}

fn boundary_char(hay: &[char], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    let previous = hay[index - 1];
    matches!(previous, '/' | '_' | '-' | '.' | ' ')
        || (previous.is_lowercase() && hay[index].is_uppercase())
}

/// `sensitive` is unused for the byte path because `eq` already closed over it.
/// Char alignment needs it to avoid folding.
fn subsequence_chars(hay: &[char], needle: &[char], sensitive: bool) -> bool {
    let mut from = 0;
    for &needle in needle {
        let mut found = false;
        while from < hay.len() {
            let hay_ch = hay[from];
            from += 1;
            if chars_eq(hay_ch, needle, sensitive) {
                found = true;
                break;
            }
        }
        if !found {
            return false;
        }
    }
    true
}

fn align_seq(
    hay_len: usize,
    needle_len: usize,
    eq: impl Fn(usize, usize) -> bool,
    boundary: impl Fn(usize) -> bool,
    byte_of: impl Fn(usize) -> u32,
    scratch: &mut Scratch,
    hits: Option<&mut Vec<u32>>,
) -> Option<i32> {
    if needle_len == 0 {
        return Some(0);
    }
    if hay_len < needle_len || hay_len > 4096 || needle_len > 256 {
        return None;
    }
    let stride = needle_len + 1;
    let cells = (hay_len + 1).saturating_mul(stride);
    scratch.score.clear();
    scratch.score.resize(cells, NEG);
    scratch.took.clear();
    scratch.took.resize(cells, 0);
    for row in 0..=hay_len {
        scratch.score[row * stride] = 0;
    }
    for hay_index in 1..=hay_len {
        for needle_index in 1..=needle_len {
            let skip = scratch.score[(hay_index - 1) * stride + needle_index];
            let mut best = skip;
            let mut used = 0u8;
            if eq(hay_index - 1, needle_index - 1) {
                let diagonal = scratch.score[(hay_index - 1) * stride + (needle_index - 1)];
                if diagonal > NEG / 2 {
                    let mut gained = diagonal.saturating_add(16);
                    if scratch.took[(hay_index - 1) * stride + (needle_index - 1)] == 1 {
                        gained = gained.saturating_add(32);
                    }
                    if boundary(hay_index - 1) {
                        gained = gained.saturating_add(24);
                    }
                    if gained >= best {
                        best = gained;
                        used = 1;
                    }
                }
            }
            let cell = hay_index * stride + needle_index;
            scratch.score[cell] = best;
            scratch.took[cell] = used;
        }
    }
    let total = scratch.score[hay_len * stride + needle_len];
    if total <= NEG / 2 {
        return None;
    }
    if let Some(hits) = hits {
        hits.clear();
        let mut hay_index = hay_len;
        let mut needle_index = needle_len;
        while needle_index > 0 && hay_index > 0 {
            if scratch.took[hay_index * stride + needle_index] == 1 {
                hits.push(byte_of(hay_index - 1));
                hay_index -= 1;
                needle_index -= 1;
            } else {
                hay_index -= 1;
            }
        }
        hits.reverse();
    }
    Some(total)
}

#[derive(Default)]
struct Scratch {
    score: Vec<i32>,
    took: Vec<u8>,
}

std::thread_local! {
    static SCRATCH: std::cell::RefCell<Scratch> = std::cell::RefCell::new(Scratch::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> IndexedFile {
        IndexedFile {
            name_at: path.rfind('/').map(|index| index as u32 + 1).unwrap_or(0),
            relative: path.to_string(),
        }
    }

    fn order(paths: &[&str], query: &str, recent: &[&str], active_dir: &str) -> Vec<String> {
        let files: Vec<IndexedFile> = paths.iter().copied().map(file).collect();
        let recent: Vec<String> = recent.iter().copied().map(str::to_string).collect();
        filter(&files, query, &recent, active_dir)
            .into_iter()
            .map(|index| files[index as usize].relative.clone())
            .collect()
    }

    fn spelled(path: &str, query: &str) -> String {
        let name_at = path.rfind('/').map(|index| index as u32 + 1).unwrap_or(0);
        highlight(path, name_at, query)
            .into_iter()
            .map(|index| path[index as usize..].chars().next().unwrap())
            .collect()
    }

    #[test]
    fn filename_beats_a_directory_hit_and_terms_are_anded() {
        let paths = [
            "src/app/widget.rs",
            "src/app.rs",
            "src/main.rs",
            "readme.md",
        ];
        assert_eq!(
            order(&paths, "app", &[], ""),
            ["src/app.rs", "src/app/widget.rs"]
        );
        assert_eq!(order(&paths, "main rs", &[], ""), ["src/main.rs"]);
        assert_eq!(
            order(&paths, "rs !widget", &[], ""),
            ["src/app.rs", "src/main.rs"]
        );
        assert!(
            order(&paths, "rs !app", &[], "")
                .iter()
                .all(|path| !path.contains("app"))
        );
    }

    #[test]
    fn smart_case_path_queries_and_exclusions_are_literal() {
        let paths = ["src/Main.rs", "src/main.rs", "src/ui/mod.rs", "modern.rs"];
        assert_eq!(order(&paths, "Main", &[], ""), ["src/Main.rs"]);
        assert_eq!(order(&paths, "main", &[], "")[0], "src/main.rs");
        assert_eq!(order(&paths, "ui/mod", &[], ""), ["src/ui/mod.rs"]);
        assert_eq!(order(&paths, r"ui\mod", &[], ""), ["src/ui/mod.rs"]);
        assert_eq!(
            order(&paths, "!mod", &[], ""),
            ["src/Main.rs", "src/main.rs"]
        );
        assert!(!order(&paths, "!mod", &[], "").contains(&"modern.rs".to_string()));
    }

    #[test]
    fn empty_query_puts_recent_files_first_and_same_folder_breaks_ties() {
        let paths = ["readme.md", "src/main.rs", "src/ui/mod.rs", "src/mod.rs"];
        assert_eq!(
            order(&paths, "", &["src/ui/mod.rs", "src/main.rs"], ""),
            ["src/ui/mod.rs", "src/main.rs", "src/mod.rs", "readme.md"]
        );
        assert_eq!(order(&paths, "mod", &[], "src/ui")[0], "src/ui/mod.rs");
    }

    #[test]
    fn contiguous_name_beats_a_scattered_subsequence() {
        let ranked = order(&["f_i_l_e.rs", "the_file.rs"], "file", &[], "");
        assert_eq!(ranked[0], "the_file.rs");
        assert_eq!(spelled("src/app.rs", "app"), "app");
        assert_eq!(spelled("src/เอกสาร.rs", "เอกสาร"), "เอกสาร");
    }

    #[test]
    fn query_window_keeps_the_cursor_visible() {
        assert_eq!(query_window("main", 4, 8), (0, 4));
        assert_eq!(query_window(&"abcdefghij".repeat(4), 40, 5), (36, 40));
        assert_eq!(query_window("abcdef", 0, 4), (0, 4));
        let long = "เอกสาร-ชื่อไฟล์.rs";
        let cursor = long.len();
        let (start, end) = query_window(long, cursor, 6);
        assert_eq!(end, cursor);
        assert!(long.is_char_boundary(start));
        assert!(long[start..end].width() <= 5);
    }

    #[test]
    fn cursor_edits_stay_on_char_boundaries() {
        let mut picker = FilePicker::default();
        assert!(picker.edit(crate::action::PickerAction::Insert("เอกสาร".into()), 10));
        picker.edit(crate::action::PickerAction::CursorLeft, 10);
        assert!(picker.query.is_char_boundary(picker.cursor));
        assert!(picker.edit(crate::action::PickerAction::Backspace, 10));
        assert!(picker.query.is_char_boundary(picker.cursor));
        assert_eq!(picker.query.chars().count(), 5);
        picker.edit(crate::action::PickerAction::CursorEnd, 10);
        assert!(picker.edit(crate::action::PickerAction::Insert(" src".into()), 10));
        assert!(picker.edit(crate::action::PickerAction::DeleteWordBack, 10));
        assert!(picker.query.ends_with(' '));
    }

    #[test]
    fn walk_skips_gitignored_and_hidden_files() {
        let root = std::env::temp_dir().join(format!("duckx-picker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(root.join(".hidden.rs"), "").unwrap();
        std::fs::write(root.join("src/app.rs"), "").unwrap();
        std::fs::write(root.join("target/secret.rs"), "").unwrap();

        let (files, truncated) = index_workspace(&root);
        let listed: Vec<&str> = files.iter().map(|file| file.relative.as_str()).collect();
        assert!(!truncated);
        assert_eq!(listed, ["src/app.rs"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_picker_command_filters_off_the_ui_thread_and_opens_the_match() {
        let root = std::env::temp_dir().join(format!("duckx-picker-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/app")).unwrap();
        std::fs::write(root.join("src/app.rs"), "fn app() {}\n").unwrap();
        std::fs::write(root.join("src/app/widget.rs"), "fn widget() {}\n").unwrap();

        let mut app = crate::app::App::new(crate::buffer::Buffer::from_text("stay\n"), None);
        app.explorer = crate::explorer::Explorer::new(root.clone());
        app.handle(crate::action::SemanticAction::OpenCommandBar);
        app.handle(crate::action::SemanticAction::CommandBar(
            crate::action::CommandBarAction::Insert("file-picker".into()),
        ));
        app.handle(crate::action::SemanticAction::CommandBar(
            crate::action::CommandBarAction::Submit,
        ));
        assert_eq!(app.focus(), crate::action::Focus::FilePicker);
        app.handle(crate::action::SemanticAction::Picker(
            crate::action::PickerAction::Insert("x".into()),
        ));
        assert_eq!(app.buffer().rope().to_string(), "stay\n");
        app.handle(crate::action::SemanticAction::Picker(
            crate::action::PickerAction::Backspace,
        ));

        for _ in 0..3 {
            let jobs = app.take_jobs();
            if jobs.is_empty() {
                break;
            }
            for job in jobs {
                let event = crate::jobs::run(job);
                app.handle_event(event);
            }
        }
        app.handle(crate::action::SemanticAction::Picker(
            crate::action::PickerAction::Insert("app".into()),
        ));
        let jobs = app.take_jobs();
        assert!(
            jobs.iter()
                .any(|job| matches!(job, crate::jobs::Job::FilterFiles { .. })),
            "filtering stays off the UI thread"
        );
        for job in jobs {
            let event = crate::jobs::run(job);
            app.handle_event(event);
        }
        let picker = app.file_picker.as_ref().unwrap();
        let top = app.picker_file(picker.rows[0]).unwrap();
        assert_eq!(top.relative, "src/app.rs");
        app.handle(crate::action::SemanticAction::Picker(
            crate::action::PickerAction::Submit,
        ));
        assert!(app.file_picker.is_none());
        assert_eq!(app.focus(), crate::action::Focus::Editor);
        let jobs = app.take_jobs();
        assert!(jobs.iter().any(|job| matches!(
            job,
            crate::jobs::Job::LoadFile { path, .. } if path.ends_with("app.rs") && !path.ends_with("widget.rs")
        )));
        std::fs::remove_dir_all(root).unwrap();
    }
}
