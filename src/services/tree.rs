use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::explorer::FileEntry;

/// Upper bound on scanned entries so huge directories can't stall the explorer.
pub const MAX_ENTRIES: usize = 20_000;

/// Walks `root` (respecting `.gitignore`, `.ignore`, and skipping hidden files) and returns
/// entries in display order: depth-first, directories before files, names case-insensitive.
/// Blocking; run it off the UI thread.
pub fn scan(root: &Path) -> Vec<FileEntry> {
    let mut children: HashMap<PathBuf, Vec<(PathBuf, bool)>> = HashMap::new();
    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .require_git(false)
        .follow_links(false)
        .build();

    for entry in walker.filter_map(Result::ok).take(MAX_ENTRIES + 1) {
        if entry.depth() == 0 {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let relative: PathBuf = relative.components().collect();
        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        let parent = relative.parent().map(Path::to_path_buf).unwrap_or_default();
        children.entry(parent).or_default().push((relative, is_dir));
    }

    for list in children.values_mut() {
        list.sort_by(|(a, a_dir), (b, b_dir)| {
            b_dir.cmp(a_dir).then_with(|| sort_key(a).cmp(&sort_key(b)))
        });
    }

    let mut entries = Vec::new();
    push_children(Path::new(""), 1, &children, &mut entries);
    entries
}

fn sort_key(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn push_children(
    parent: &Path,
    depth: usize,
    children: &HashMap<PathBuf, Vec<(PathBuf, bool)>>,
    out: &mut Vec<FileEntry>,
) {
    let Some(list) = children.get(parent) else {
        return;
    };
    for (path, is_dir) in list {
        out.push(FileEntry {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: path.clone(),
            is_dir: *is_dir,
            depth,
        });
        if *is_dir {
            push_children(path, depth + 1, children, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn respects_gitignore_hides_dotfiles_and_sorts_dirs_first() {
        let root = std::env::temp_dir().join(format!("duckx-tree-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for dir in ["src/ui", "target/debug", "Zeta"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in [
            ".gitignore",
            ".hidden",
            "b.txt",
            "A.md",
            "src/main.rs",
            "src/ui/mod.rs",
            "target/debug/app.exe",
            "Zeta/z.rs",
        ] {
            fs::write(root.join(file), "").unwrap();
        }
        fs::write(root.join(".gitignore"), "target/\n").unwrap();

        let listed: Vec<(String, usize)> = scan(&root)
            .into_iter()
            .map(|e| (e.path.to_string_lossy().replace('\\', "/"), e.depth))
            .collect();
        assert_eq!(
            listed,
            [
                ("src".to_string(), 1),
                ("src/ui".to_string(), 2),
                ("src/ui/mod.rs".to_string(), 3),
                ("src/main.rs".to_string(), 2),
                ("Zeta".to_string(), 1),
                ("Zeta/z.rs".to_string(), 2),
                ("A.md".to_string(), 1),
                ("b.txt".to_string(), 1),
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }
}
