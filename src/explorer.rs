use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::services::git::GitStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Relative to the explorer root.
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    /// 1 for direct children of the root.
    pub depth: usize,
}

/// A visible tree row with its precomputed guide prefix (`│ ├╴`).
#[derive(Debug)]
pub struct Row<'a> {
    pub entry: &'a FileEntry,
    pub guide: String,
    pub open: bool,
    pub matched: bool,
    /// Matches of the explorer content search. `None` on folders and outside that search.
    pub hits: Option<usize>,
}

/// File explorer sidebar state. `entries` holds the whole scanned tree in display order;
/// expansion and filtering only change which entries are visible.
#[derive(Debug)]
pub struct Explorer {
    pub root: PathBuf,
    pub root_name: String,
    pub open: bool,
    pub focused: bool,
    /// FILE mode, entered with `select`. `Space` marks files and `Esc` returns to NORMAL.
    pub selecting: bool,
    pub loading: bool,
    pub filter: String,
    /// Query shown in the filter box while a content search is active.
    pub file_query: Option<String>,
    pub git: HashMap<PathBuf, GitStatus>,
    /// Relative path to match count. `Some` limits the tree to those files and their parents.
    file_hits: Option<HashMap<PathBuf, usize>>,
    entries: Vec<FileEntry>,
    expanded: HashSet<PathBuf>,
    selected: Option<PathBuf>,
    /// Files marked with the `select` command. These stay put while the cursor moves.
    pinned: HashSet<PathBuf>,
    /// The cursor position where `Shift` selection started.
    range_anchor: Option<PathBuf>,
    /// Files covered by the current `Shift` range. Cleared by movement without `Shift`.
    shift_paths: HashSet<PathBuf>,
    pub scroll: usize,
    /// Number of tree rows on screen, used for paging and scrolling.
    pub height: usize,
}

impl Explorer {
    pub fn new(root: PathBuf) -> Self {
        let root = crate::agent::strip_verbatim(&root);
        let root_name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root.display().to_string());
        Self {
            root,
            root_name,
            open: false,
            focused: false,
            selecting: false,
            loading: false,
            filter: String::new(),
            file_query: None,
            git: HashMap::new(),
            file_hits: None,
            entries: Vec::new(),
            expanded: HashSet::new(),
            selected: None,
            pinned: HashSet::new(),
            range_anchor: None,
            shift_paths: HashSet::new(),
            scroll: 0,
            height: 0,
        }
    }

    pub fn entries(&self) -> &[FileEntry] {
        &self.entries
    }

    /// Directories the tree is showing open, relative to the root, in path order.
    pub fn expanded_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<_> = self.expanded.iter().cloned().collect();
        paths.sort();
        paths
    }

    /// Remembers which directories to draw open. A later scan drops any path that is gone.
    pub fn set_expanded(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        self.expanded = paths.into_iter().collect();
    }

    pub fn set_entries(&mut self, entries: Vec<FileEntry>) {
        self.entries = entries;
        self.loading = false;
        self.scroll_into_view();
        let known: HashSet<&Path> = self.entries.iter().map(|e| e.path.as_path()).collect();
        self.expanded.retain(|p| known.contains(p.as_path()));
        self.pinned.retain(|p| known.contains(p.as_path()));
        self.shift_paths.retain(|p| known.contains(p.as_path()));
        if self
            .range_anchor
            .as_ref()
            .is_some_and(|p| !known.contains(p.as_path()))
        {
            self.range_anchor = None;
            self.shift_paths.clear();
        }
        if self
            .selected
            .as_ref()
            .is_none_or(|p| !known.contains(p.as_path()))
        {
            self.selected = self.entries.first().map(|e| e.path.clone());
        }
    }

    pub fn is_marked(&self, path: &Path) -> bool {
        self.pinned.contains(path)
    }

    /// Rows painted while `Shift` is held in FILE mode. Not marks until `Space`.
    pub fn is_covered(&self, path: &Path) -> bool {
        self.shift_paths.contains(path)
    }

    pub fn enter_select(&mut self) {
        self.selecting = true;
        self.range_anchor = None;
        self.shift_paths.clear();
    }

    /// Returns to NORMAL and drops every file mark.
    pub fn leave_file_mode(&mut self) {
        self.selecting = false;
        self.pinned.clear();
        self.range_anchor = None;
        self.shift_paths.clear();
    }

    /// Drops sidebar focus. FILE mode stays until the caller leaves it.
    pub fn blur(&mut self) {
        self.focused = false;
    }

    pub fn marked_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| self.is_marked(&entry.path))
            .count()
    }

    /// Marks the covered rows, or toggles the focused row when nothing is covered.
    /// A cover is marked when any row in it is unmarked, and unmarked when every row
    /// is already marked. Returns `(label, now marked)`.
    pub fn commit_mark(&mut self) -> Option<(String, bool)> {
        let paths = if self.shift_paths.is_empty() {
            self.selected.iter().cloned().collect::<Vec<_>>()
        } else {
            self.visible_rows()
                .iter()
                .filter(|row| self.shift_paths.contains(&row.entry.path))
                .map(|row| row.entry.path.clone())
                .collect::<Vec<_>>()
        };
        if paths.is_empty() {
            return None;
        }
        let marked = paths.iter().any(|path| !self.pinned.contains(path));
        for path in &paths {
            if marked {
                self.pinned.insert(path.clone());
            } else {
                self.pinned.remove(path);
            }
        }
        self.range_anchor = None;
        self.shift_paths.clear();
        let label = if paths.len() == 1 {
            paths[0].display().to_string()
        } else {
            format!("{} files", paths.len())
        };
        Some((label, marked))
    }

    /// Marked files, or the focused file when nothing is marked.
    /// A marked folder stands in for the files inside it.
    pub fn targets(&self) -> Vec<PathBuf> {
        let marked: Vec<PathBuf> = self
            .entries
            .iter()
            .filter(|entry| self.is_marked(&entry.path))
            .map(|entry| entry.path.clone())
            .collect();
        let paths = if marked.is_empty() {
            self.selected.iter().cloned().collect()
        } else {
            marked
        };
        roots_only(&paths)
    }

    /// Drops entries under `rels` (relative to the root) and moves the cursor to a neighbor.
    pub fn remove_entries(&mut self, rels: &[PathBuf]) {
        let gone = |path: &Path| path_is_under(path, rels);
        let rows = self.visible_rows();
        let index = self.selected_index().unwrap_or(0);
        let next = rows
            .iter()
            .skip(index + 1)
            .find(|row| !gone(&row.entry.path))
            .or_else(|| {
                rows.iter()
                    .take(index)
                    .rev()
                    .find(|row| !gone(&row.entry.path))
            })
            .map(|row| row.entry.path.clone());
        self.entries.retain(|entry| !gone(&entry.path));
        self.expanded.retain(|path| !gone(path));
        self.pinned.retain(|path| !gone(path));
        self.shift_paths.retain(|path| !gone(path));
        self.git.retain(|path, _| !gone(path));
        if self.range_anchor.as_deref().is_some_and(gone) {
            self.range_anchor = None;
            self.shift_paths.clear();
        }
        if self.selected.as_deref().is_some_and(gone) {
            self.selected = next.or_else(|| self.entries.first().map(|entry| entry.path.clone()));
        }
    }

    /// Points `from` and everything inside it at `to`. Both paths are relative to the root.
    pub fn relocate(&mut self, from: &Path, to: &Path) {
        let map = |path: &Path| -> PathBuf {
            if path == from {
                return to.to_path_buf();
            }
            match path.strip_prefix(from) {
                Ok(rest) if !rest.as_os_str().is_empty() => to.join(rest),
                _ => path.to_path_buf(),
            }
        };
        for entry in &mut self.entries {
            if entry.path == from || entry.path.starts_with(from) {
                entry.path = map(&entry.path);
                entry.depth = entry.path.components().count();
                if entry.path == to
                    && let Some(name) = entry.path.file_name()
                {
                    entry.name = name.to_string_lossy().into_owned();
                }
            }
        }
        self.expanded = self.expanded.iter().map(|path| map(path)).collect();
        self.pinned = self.pinned.iter().map(|path| map(path)).collect();
        self.shift_paths = self.shift_paths.iter().map(|path| map(path)).collect();
        self.git = std::mem::take(&mut self.git)
            .into_iter()
            .map(|(path, status)| (map(&path), status))
            .collect();
        if let Some(path) = self.selected.clone() {
            self.selected = Some(map(&path));
        }
        if let Some(path) = self.range_anchor.clone() {
            self.range_anchor = Some(map(&path));
        }
    }

    pub fn selected(&self) -> Option<&Path> {
        self.selected.as_deref()
    }

    fn matches_filter(&self, entry: &FileEntry) -> bool {
        entry
            .name
            .to_lowercase()
            .contains(&self.filter.to_lowercase())
    }

    /// Shows `hits` in the tree and moves the cursor to the first matching file.
    pub fn show_file_hits(&mut self, hits: HashMap<PathBuf, usize>) {
        self.file_hits = Some(hits);
        if let Some(path) = self
            .visible_rows()
            .iter()
            .find(|row| row.hits.is_some())
            .map(|row| row.entry.path.clone())
        {
            self.selected = Some(path);
        }
        self.scroll_into_view();
    }

    /// Selects `path` in the full tree. Ancestor folders open, a filter or content
    /// search is cleared, and a `Shift` cover is dropped. Marks stay. Returns false
    /// when `path` is not a file in the scanned tree.
    pub fn reveal_file(&mut self, path: &Path) -> bool {
        let path: PathBuf = path.components().collect();
        let found = self
            .entries
            .iter()
            .any(|entry| !entry.is_dir && entry.path == path);
        if !found {
            return false;
        }
        self.filter.clear();
        self.file_hits = None;
        self.file_query = None;
        let mut parent = path.as_path();
        while let Some(dir) = parent.parent() {
            if dir.as_os_str().is_empty() {
                break;
            }
            if self
                .entries
                .iter()
                .any(|entry| entry.is_dir && entry.path == dir)
            {
                self.expanded.insert(dir.to_path_buf());
            }
            parent = dir;
        }
        self.selected = Some(path);
        self.range_anchor = None;
        self.shift_paths.clear();
        self.scroll_into_view();
        true
    }

    /// Restores the normal tree after a content search.
    pub fn clear_file_search(&mut self) {
        self.file_hits = None;
        self.file_query = None;
    }

    pub fn has_file_hits(&self) -> bool {
        self.file_hits.is_some()
    }

    /// Drops file marks without leaving FILE mode.
    pub fn clear_marks(&mut self) {
        self.pinned.clear();
        self.range_anchor = None;
        self.shift_paths.clear();
    }

    /// Files a FILTER replace should touch. Marks limit the set to those files
    /// and the files inside marked folders. With no marks, every match is included.
    pub fn files_to_replace(&self) -> Vec<PathBuf> {
        let Some(hits) = &self.file_hits else {
            return Vec::new();
        };
        let matched: Vec<PathBuf> = self
            .entries
            .iter()
            .filter(|entry| !entry.is_dir && hits.get(&entry.path).is_some_and(|count| *count > 0))
            .map(|entry| entry.path.clone())
            .collect();
        if self.pinned.is_empty() {
            return matched;
        }
        matched
            .into_iter()
            .filter(|path| {
                self.pinned
                    .iter()
                    .any(|mark| path == mark || path.starts_with(mark))
            })
            .collect()
    }

    /// Rows currently shown: all expanded branches, or — while filtering — every
    /// match plus its ancestor directories. A content search uses the same shape.
    pub fn visible_rows(&self) -> Vec<Row<'_>> {
        if self.file_hits.is_some() {
            return self.file_hit_rows();
        }
        let mut visible: Vec<(&FileEntry, bool, bool)> = Vec::new();
        if self.filter.is_empty() {
            let mut collapsed_at: Option<usize> = None;
            for entry in &self.entries {
                if let Some(depth) = collapsed_at {
                    if entry.depth > depth {
                        continue;
                    }
                    collapsed_at = None;
                }
                let open = entry.is_dir && self.expanded.contains(&entry.path);
                if entry.is_dir && !open {
                    collapsed_at = Some(entry.depth);
                }
                visible.push((entry, open, false));
            }
        } else {
            let mut ancestors: HashSet<&Path> = HashSet::new();
            for entry in self.entries.iter().filter(|e| self.matches_filter(e)) {
                ancestors.extend(entry.path.ancestors().skip(1));
            }
            for entry in &self.entries {
                let matched = self.matches_filter(entry);
                let is_ancestor = ancestors.contains(entry.path.as_path());
                if matched || is_ancestor {
                    visible.push((entry, is_ancestor, matched));
                }
            }
        }
        with_guides(visible)
    }

    fn file_hit_rows(&self) -> Vec<Row<'_>> {
        let Some(hits) = &self.file_hits else {
            return Vec::new();
        };
        let mut ancestors: HashSet<&Path> = HashSet::new();
        for entry in self
            .entries
            .iter()
            .filter(|entry| hits.get(&entry.path).is_some_and(|count| *count > 0))
        {
            ancestors.extend(entry.path.ancestors().skip(1));
        }
        let mut visible = Vec::new();
        for entry in &self.entries {
            let matched = hits.get(&entry.path).is_some_and(|count| *count > 0);
            let is_ancestor = ancestors.contains(entry.path.as_path());
            if matched || is_ancestor {
                visible.push((entry, is_ancestor, matched));
            }
        }
        let mut rows = with_guides(visible);
        for row in &mut rows {
            row.hits = hits
                .get(&row.entry.path)
                .copied()
                .filter(|count| *count > 0);
        }
        rows
    }

    /// `(shown, total)` for the filter box: visible rows when not filtering,
    /// otherwise matches out of every scanned entry. During a content search the
    /// pair is matching files out of every file.
    pub fn counts(&self) -> (usize, usize) {
        if let Some(hits) = &self.file_hits {
            let matched = self
                .entries
                .iter()
                .filter(|entry| hits.get(&entry.path).is_some_and(|count| *count > 0))
                .count();
            let files = self.entries.iter().filter(|entry| !entry.is_dir).count();
            return (matched, files);
        }
        if self.filter.is_empty() {
            let shown = self.visible_rows().len();
            (shown, shown)
        } else {
            let matched = self
                .entries
                .iter()
                .filter(|e| self.matches_filter(e))
                .count();
            (matched, self.entries.len())
        }
    }

    pub fn selected_index(&self) -> Option<usize> {
        let selected = self.selected.as_deref()?;
        self.visible_rows()
            .iter()
            .position(|row| row.entry.path == selected)
    }

    fn select_index(&mut self, index: usize) {
        let rows = self.visible_rows();
        let path = rows
            .get(index.min(rows.len().saturating_sub(1)))
            .map(|row| row.entry.path.clone());
        if path.is_some() {
            self.selected = path;
        }
    }

    pub fn move_selection(&mut self, delta: isize, extend: bool) {
        self.begin_nav(extend);
        let current = self.selected_index().unwrap_or(0) as isize;
        self.select_index((current + delta).max(0) as usize);
        self.end_nav(extend);
    }

    pub fn select_first(&mut self, extend: bool) {
        self.begin_nav(extend);
        self.select_index(0);
        self.end_nav(extend);
    }

    pub fn select_last(&mut self, extend: bool) {
        self.begin_nav(extend);
        self.select_index(usize::MAX);
        self.end_nav(extend);
    }

    fn begin_nav(&mut self, extend: bool) {
        if extend && self.range_anchor.is_none() {
            self.range_anchor = self.selected.clone();
        }
    }

    fn end_nav(&mut self, extend: bool) {
        if extend {
            self.rebuild_shift_range();
        } else {
            self.range_anchor = None;
            self.shift_paths.clear();
        }
    }

    fn rebuild_shift_range(&mut self) {
        let Some(anchor) = self.range_anchor.clone() else {
            self.shift_paths.clear();
            return;
        };
        let selected = self.selected.clone();
        let paths = {
            let rows = self.visible_rows();
            let head = selected
                .as_ref()
                .and_then(|path| rows.iter().position(|row| row.entry.path == *path));
            match (rows.iter().position(|row| row.entry.path == anchor), head) {
                (Some(start), Some(head)) => {
                    let (lo, hi) = if start <= head {
                        (start, head)
                    } else {
                        (head, start)
                    };
                    rows[lo..=hi]
                        .iter()
                        .map(|row| row.entry.path.clone())
                        .collect::<Vec<_>>()
                }
                (_, Some(_)) => selected.into_iter().collect(),
                _ => Vec::new(),
            }
        };
        self.shift_paths.clear();
        self.shift_paths.extend(paths);
    }

    fn selected_entry(&self) -> Option<&FileEntry> {
        let selected = self.selected.as_deref()?;
        self.entries.iter().find(|e| e.path == selected)
    }

    /// `Right`: open a collapsed directory, or step into an open one.
    pub fn expand(&mut self, extend: bool) {
        self.begin_nav(extend);
        let Some(entry) = self.selected_entry().filter(|e| e.is_dir).cloned() else {
            self.end_nav(extend);
            return;
        };
        if self.expanded.insert(entry.path.clone()) {
            self.end_nav(extend);
            return;
        }
        let rows = self.visible_rows();
        if let Some(index) = self.selected_index()
            && let Some(child) = rows.get(index + 1)
            && child.entry.depth > entry.depth
        {
            let path = child.entry.path.clone();
            self.selected = Some(path);
        }
        self.end_nav(extend);
    }

    /// `Left`: close an open directory, or jump to the parent directory.
    pub fn collapse(&mut self, extend: bool) {
        self.begin_nav(extend);
        let Some(entry) = self.selected_entry().cloned() else {
            self.end_nav(extend);
            return;
        };
        if entry.is_dir && self.expanded.remove(&entry.path) {
            self.end_nav(extend);
            return;
        }
        if let Some(parent) = entry.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            self.selected = Some(parent.to_path_buf());
        }
        self.end_nav(extend);
    }

    /// `Enter`: toggles a directory; returns the path when a file is chosen.
    pub fn activate(&mut self) -> Option<PathBuf> {
        let entry = self.selected_entry()?.clone();
        if entry.is_dir {
            if !self.expanded.remove(&entry.path) {
                self.expanded.insert(entry.path);
            }
            None
        } else {
            Some(entry.path)
        }
    }

    pub fn insert_filter(&mut self, text: &str) {
        self.filter.push_str(text);
        self.on_filter_changed();
    }

    pub fn pop_filter(&mut self) {
        if self.filter.pop().is_some() {
            self.on_filter_changed();
        }
    }

    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.on_filter_changed();
    }

    fn on_filter_changed(&mut self) {
        self.scroll = 0;
        let first_match = self
            .visible_rows()
            .iter()
            .find(|row| row.matched)
            .map(|row| row.entry.path.clone());
        match first_match {
            Some(path) => self.selected = Some(path),
            None if self.selected_index().is_none() => self.select_first(false),
            None => {}
        }
        if self.range_anchor.is_some() {
            self.rebuild_shift_range();
        }
    }

    /// Keeps the selected row within the visible `height` rows.
    pub fn scroll_into_view(&mut self) {
        let total = self.visible_rows().len();
        let height = self.height.max(1);
        let index = self.selected_index().unwrap_or(0);
        if index < self.scroll {
            self.scroll = index;
        } else if index >= self.scroll + height {
            self.scroll = index + 1 - height;
        }
        self.scroll = self.scroll.min(total.saturating_sub(height));
    }
}

/// Adds `│ ├╴ └╴` guides. Relies on depth-first order: the next row at depth `<= d`
/// is a sibling exactly when it has depth `d`.
fn with_guides(visible: Vec<(&FileEntry, bool, bool)>) -> Vec<Row<'_>> {
    let mut ancestors_last: Vec<bool> = Vec::new();
    let mut rows = Vec::with_capacity(visible.len());
    for (i, &(entry, open, matched)) in visible.iter().enumerate() {
        let depth = entry.depth;
        let is_last = visible[i + 1..]
            .iter()
            .find(|(next, _, _)| next.depth <= depth)
            .is_none_or(|(next, _, _)| next.depth < depth);

        ancestors_last.truncate(depth - 1);
        let mut guide: String = ancestors_last
            .iter()
            .map(|last| if *last { "  " } else { "│ " })
            .collect();
        guide.push_str(if is_last { "└╴" } else { "├╴" });
        ancestors_last.push(is_last);

        rows.push(Row {
            entry,
            guide,
            open,
            matched,
            hits: None,
        });
    }
    rows
}

fn path_is_under(path: &Path, roots: &[PathBuf]) -> bool {
    roots
        .iter()
        .any(|root| path == root.as_path() || path.starts_with(root))
}

/// Drops a path when another path in the list is its parent.
fn roots_only(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|path| {
            !paths
                .iter()
                .any(|other| other != *path && path.starts_with(other))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, is_dir: bool) -> FileEntry {
        let path: PathBuf = Path::new(path).components().collect();
        FileEntry {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            depth: path.components().count(),
            path,
            is_dir,
        }
    }

    fn sample() -> Explorer {
        let mut explorer = Explorer::new(PathBuf::from("/work/duckx-code"));
        explorer.set_entries(vec![
            entry("examples", true),
            entry("examples/keylog.rs", false),
            entry("src", true),
            entry("src/ui", true),
            entry("src/ui/mod.rs", false),
            entry("src/main.rs", false),
            entry("Cargo.toml", false),
            entry("README.md", false),
        ]);
        explorer
    }

    fn shown(explorer: &Explorer) -> Vec<String> {
        explorer
            .visible_rows()
            .iter()
            .map(|row| format!("{}{}", row.guide, row.entry.name))
            .collect()
    }

    fn selected_name(explorer: &Explorer) -> String {
        explorer
            .selected()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn reveal_file_opens_ancestors_and_keeps_marks() {
        let mut explorer = sample();
        explorer.height = 2;
        explorer.enter_select();
        explorer.commit_mark();
        explorer.move_selection(1, true);
        assert!(explorer.is_covered(Path::new("src")));
        assert!(explorer.is_marked(Path::new("examples")));
        explorer.insert_filter("toml");

        assert!(explorer.reveal_file(Path::new("src/ui/mod.rs")));
        assert!(explorer.selecting);
        assert!(explorer.filter.is_empty());
        assert_eq!(selected_name(&explorer), "mod.rs");
        assert!(explorer.expanded_paths().contains(&PathBuf::from("src")));
        assert!(explorer.expanded_paths().contains(&PathBuf::from("src/ui")));
        assert!(
            !explorer
                .expanded_paths()
                .contains(&PathBuf::from("examples"))
        );
        assert!(explorer.is_marked(Path::new("examples")));
        assert!(!explorer.is_covered(Path::new("src")));
        let index = explorer.selected_index().unwrap();
        assert!(index >= explorer.scroll && index < explorer.scroll + explorer.height);
        assert!(shown(&explorer).iter().any(|row| row.ends_with("mod.rs")));

        assert!(!explorer.reveal_file(Path::new("src")));
        assert!(!explorer.reveal_file(Path::new("missing.rs")));
    }

    #[test]
    fn collapsed_tree_shows_top_level_with_guides() {
        let explorer = sample();
        assert_eq!(explorer.root_name, "duckx-code");
        assert_eq!(
            shown(&explorer),
            ["├╴examples", "├╴src", "├╴Cargo.toml", "└╴README.md"]
        );
        assert_eq!(explorer.counts(), (4, 4));
        assert_eq!(selected_name(&explorer), "examples");
    }

    #[test]
    fn right_expands_then_enters_and_left_walks_back_out() {
        let mut explorer = sample();
        explorer.move_selection(1, false);
        explorer.expand(false);
        assert_eq!(
            shown(&explorer),
            [
                "├╴examples",
                "├╴src",
                "│ ├╴ui",
                "│ └╴main.rs",
                "├╴Cargo.toml",
                "└╴README.md"
            ]
        );
        explorer.expand(false);
        assert_eq!(selected_name(&explorer), "ui");
        explorer.expand(false);
        explorer.expand(false);
        assert_eq!(selected_name(&explorer), "mod.rs");
        assert!(shown(&explorer).contains(&"│ │ └╴mod.rs".to_string()));

        explorer.collapse(false);
        assert_eq!(selected_name(&explorer), "ui");
        explorer.collapse(false);
        assert!(!shown(&explorer).iter().any(|r| r.ends_with("mod.rs")));
        explorer.collapse(false);
        assert_eq!(selected_name(&explorer), "src");
    }

    #[test]
    fn enter_toggles_dirs_and_returns_files() {
        let mut explorer = sample();
        assert_eq!(explorer.activate(), None);
        assert_eq!(shown(&explorer).len(), 5);
        assert_eq!(explorer.activate(), None);
        assert_eq!(shown(&explorer).len(), 4);

        explorer.select_last(false);
        assert_eq!(explorer.activate(), Some(PathBuf::from("README.md")));
    }

    #[test]
    fn filter_shows_matches_with_ancestors_and_selects_first_match() {
        let mut explorer = sample();
        explorer.insert_filter("MOD");
        assert_eq!(shown(&explorer), ["└╴src", "  └╴ui", "    └╴mod.rs"]);
        assert_eq!(explorer.counts(), (1, 8));
        assert_eq!(selected_name(&explorer), "mod.rs");

        explorer.pop_filter();
        explorer.pop_filter();
        explorer.pop_filter();
        assert_eq!(explorer.counts(), (4, 4));

        explorer.insert_filter("zzz");
        assert!(shown(&explorer).is_empty());
        assert_eq!(explorer.counts(), (0, 8));
        explorer.clear_filter();
        assert_eq!(shown(&explorer).len(), 4);
    }

    #[test]
    fn file_hits_list_matching_files_with_their_parents() {
        let mut explorer = sample();
        let mut hits = HashMap::new();
        hits.insert(PathBuf::from("src/main.rs"), 2);
        hits.insert(PathBuf::from("README.md"), 1);
        explorer.file_query = Some("fn".to_string());
        explorer.show_file_hits(hits);
        assert_eq!(shown(&explorer), ["├╴src", "│ └╴main.rs", "└╴README.md"]);
        let counts: Vec<_> = explorer.visible_rows().iter().map(|row| row.hits).collect();
        assert_eq!(counts, [None, Some(2), Some(1)]);
        assert_eq!(selected_name(&explorer), "main.rs");
        assert_eq!(explorer.counts(), (2, 5));
        explorer.clear_file_search();
        assert_eq!(shown(&explorer).len(), 4);
    }

    #[test]
    fn files_to_replace_uses_marks_or_every_match() {
        let mut explorer = sample();
        let mut hits = HashMap::new();
        hits.insert(PathBuf::from("src/main.rs"), 2);
        hits.insert(PathBuf::from("README.md"), 1);
        explorer.show_file_hits(hits);
        assert_eq!(
            explorer.files_to_replace(),
            [PathBuf::from("src/main.rs"), PathBuf::from("README.md")]
        );
        explorer.commit_mark();
        assert_eq!(explorer.files_to_replace(), [PathBuf::from("src/main.rs")]);
        explorer.clear_marks();
        assert_eq!(explorer.files_to_replace().len(), 2);
    }

    #[test]
    fn scroll_keeps_selection_visible() {
        let mut explorer = sample();
        explorer.height = 2;
        explorer.select_last(false);
        explorer.scroll_into_view();
        assert_eq!(explorer.scroll, 2);
        explorer.select_first(false);
        explorer.scroll_into_view();
        assert_eq!(explorer.scroll, 0);
    }

    #[test]
    fn rescan_keeps_selection_and_expansion() {
        let mut explorer = sample();
        explorer.move_selection(1, false);
        explorer.expand(false);
        let entries = explorer.entries().to_vec();
        explorer.set_entries(entries);
        assert_eq!(selected_name(&explorer), "src");
        assert_eq!(shown(&explorer).len(), 6);
    }

    #[test]
    fn select_marks_stick_until_space_commits_the_shift_cover() {
        let mut explorer = sample();
        let (name, marked) = explorer.commit_mark().unwrap();
        assert_eq!(name, "examples");
        assert!(marked);
        assert!(explorer.is_marked(Path::new("examples")));

        explorer.move_selection(1, false);
        assert!(explorer.is_marked(Path::new("examples")));
        assert!(!explorer.is_marked(Path::new("src")));
        assert_eq!(explorer.marked_count(), 1);

        explorer.move_selection(1, true);
        explorer.move_selection(1, true);
        assert!(explorer.is_covered(Path::new("src")));
        assert!(explorer.is_covered(Path::new("Cargo.toml")));
        assert!(explorer.is_covered(Path::new("README.md")));
        assert!(!explorer.is_marked(Path::new("src")));
        assert_eq!(explorer.targets(), vec![PathBuf::from("examples")]);

        let (name, marked) = explorer.commit_mark().unwrap();
        assert_eq!(name, "3 files");
        assert!(marked);
        assert!(explorer.is_marked(Path::new("src")));
        assert!(explorer.is_marked(Path::new("README.md")));
        assert!(!explorer.is_covered(Path::new("src")));
        assert_eq!(
            explorer.targets(),
            vec![
                PathBuf::from("examples"),
                PathBuf::from("src"),
                PathBuf::from("Cargo.toml"),
                PathBuf::from("README.md"),
            ]
        );

        explorer.move_selection(0, false);
        assert!(explorer.is_marked(Path::new("examples")));
        assert!(explorer.is_marked(Path::new("src")));

        explorer.select_first(true);
        let (name, marked) = explorer.commit_mark().unwrap();
        assert_eq!(name, "4 files");
        assert!(!marked);
        assert_eq!(explorer.marked_count(), 0);
    }

    #[test]
    fn marked_folder_covers_its_children_and_rename_follows() {
        let mut explorer = sample();
        explorer.move_selection(1, false);
        explorer.expand(false);
        explorer.commit_mark();
        explorer.move_selection(2, false);
        explorer.commit_mark();
        assert_eq!(explorer.targets(), vec![PathBuf::from("src")]);

        explorer.relocate(Path::new("src"), Path::new("lib"));
        assert_eq!(selected_name(&explorer), "main.rs");
        assert!(
            explorer
                .entries()
                .iter()
                .any(|e| e.path == Path::new("lib"))
        );
        assert!(explorer.is_marked(Path::new("lib")));
        assert_eq!(explorer.targets(), vec![PathBuf::from("lib")]);

        explorer.remove_entries(&[PathBuf::from("lib")]);
        assert!(!explorer.entries().iter().any(|e| e.path.starts_with("lib")));
        assert_eq!(explorer.marked_count(), 0);
        assert_eq!(selected_name(&explorer), "Cargo.toml");
    }
}
