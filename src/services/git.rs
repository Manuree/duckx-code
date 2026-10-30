use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use ropey::Rope;

/// Ordered by display priority: a directory shows the highest status among its descendants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GitStatus {
    Untracked,
    Renamed,
    Added,
    Deleted,
    Modified,
}

impl GitStatus {
    pub fn symbol(self) -> char {
        match self {
            GitStatus::Untracked => '?',
            GitStatus::Renamed => 'R',
            GitStatus::Added => 'A',
            GitStatus::Deleted => 'D',
            GitStatus::Modified => 'M',
        }
    }
}

/// Per-path git status relative to `root` (directories inherit their descendants' status).
/// Empty when `root` isn't in a repository or git isn't installed. Blocking; run it off the UI thread.
pub fn status(root: &Path) -> HashMap<PathBuf, GitStatus> {
    let git = |args: &[&str]| {
        git_command(root)
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
    };
    let Some(prefix) = git(&["rev-parse", "--show-prefix"]) else {
        return HashMap::new();
    };
    let Some(porcelain) = git(&["status", "--porcelain=v1", "-z", "--untracked-files=all"]) else {
        return HashMap::new();
    };
    parse_porcelain(
        &porcelain.stdout,
        String::from_utf8_lossy(&prefix.stdout).trim(),
    )
}

/// Parses `git status --porcelain=v1 -z`. Paths are repo-relative; `prefix` (from
/// `git rev-parse --show-prefix`) is stripped so results are relative to the explorer root.
pub fn parse_porcelain(raw: &[u8], prefix: &str) -> HashMap<PathBuf, GitStatus> {
    let text = String::from_utf8_lossy(raw);
    let mut fields = text.split('\0');
    let mut statuses = HashMap::new();

    while let Some(field) = fields.next() {
        let mut chars = field.chars();
        let (Some(x), Some(y), Some(' ')) = (chars.next(), chars.next(), chars.next()) else {
            continue;
        };
        if matches!(x, 'R' | 'C') {
            fields.next();
        }
        let status = match (x, y) {
            ('!', '!') => continue,
            ('?', '?') => GitStatus::Untracked,
            _ if x == 'D' || y == 'D' => GitStatus::Deleted,
            _ if x == 'M' || y == 'M' => GitStatus::Modified,
            ('A', _) => GitStatus::Added,
            ('R' | 'C', _) => GitStatus::Renamed,
            _ => GitStatus::Modified,
        };
        let Some(path) = field[3..].strip_prefix(prefix) else {
            continue;
        };
        let path: PathBuf = Path::new(path).components().collect();
        for target in path.ancestors() {
            if target.as_os_str().is_empty() {
                break;
            }
            let current = statuses.entry(target.to_path_buf()).or_insert(status);
            *current = (*current).max(status);
        }
    }
    statuses
}

/// Current branch name (or short commit hash when detached) of the repository containing `start`.
pub fn current_branch(start: &Path) -> Option<String> {
    let head = fs::read_to_string(find_git_dir(start)?.join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref: ") {
        Some(reference) => Some(
            reference
                .strip_prefix("refs/heads/")
                .unwrap_or(reference)
                .to_string(),
        ),
        None => Some(head.chars().take(7).collect()),
    }
}

/// Walks up from `start` to find `.git`, following `gitdir:` files used by worktrees and submodules.
fn find_git_dir(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return Some(dot_git);
        }
        if dot_git.is_file() {
            let content = fs::read_to_string(&dot_git).ok()?;
            let target = content.trim().strip_prefix("gitdir:")?.trim();
            return Some(dir.join(target));
        }
    }
    None
}

/// One changed line in the buffer, compared with `HEAD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Added,
    Modified,
}

/// Lines removed from `HEAD` that sit just above a buffer line.
/// `before_line == line_count` means they sat after the last line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deletion {
    pub before_line: usize,
    pub count: u32,
}

/// Line-level difference between `HEAD` and the buffer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileDiff {
    kinds: Vec<(usize, LineKind)>,
    deletions: Vec<Deletion>,
}

impl FileDiff {
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty() && self.deletions.is_empty()
    }

    pub fn kind(&self, line: usize) -> Option<LineKind> {
        self.kinds
            .binary_search_by_key(&line, |item| item.0)
            .ok()
            .map(|index| self.kinds[index].1)
    }

    pub fn deleted_above(&self, line: usize) -> u32 {
        self.deletions
            .binary_search_by_key(&line, |item| item.before_line)
            .ok()
            .map(|index| self.deletions[index].count)
            .unwrap_or(0)
    }
}

/// Why a diff could not be drawn, or the diff itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffResult {
    Diff(FileDiff),
    NotARepository,
    GitMissing,
    Binary,
    Failed(String),
}

impl DiffResult {
    pub fn as_diff(&self) -> Option<&FileDiff> {
        match self {
            DiffResult::Diff(diff) => Some(diff),
            _ => None,
        }
    }
}

/// Compares `rope` with `HEAD` of the file at `path`. Blocking; run it off the UI thread.
/// A path that is not in `HEAD` is entirely added. The buffer wins over the bytes on disk,
/// so unsaved edits are included.
pub fn diff_against_head(path: &Path, rope: &Rope) -> DiffResult {
    if rope.chunks().any(|chunk| chunk.as_bytes().contains(&0)) {
        return DiffResult::Binary;
    }
    let Some(start) = existing_dir(path) else {
        return DiffResult::Failed("Cannot compare this path".to_string());
    };
    let top = match git_command(start)
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        Ok(output) if output.status.success() => {
            PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
        }
        Ok(output) if is_not_repo(&output.stderr) => return DiffResult::NotARepository,
        Ok(output) => return DiffResult::Failed(short_error(&output.stderr)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return DiffResult::GitMissing,
        Err(err) => return DiffResult::Failed(err.to_string()),
    };
    let Some(relative) = repo_relative(path, &top) else {
        return DiffResult::Failed("Cannot compare this path".to_string());
    };
    if relative.is_empty() {
        return DiffResult::Failed("Cannot compare this path".to_string());
    }
    let spec = format!("HEAD:{relative}");
    let shown = match git_command(&top).args(["show", &spec]).output() {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return DiffResult::GitMissing,
        Err(err) => return DiffResult::Failed(err.to_string()),
    };
    let old = if shown.status.success() {
        if shown.stdout.contains(&0) {
            return DiffResult::Binary;
        }
        Some(rope_lines(&Rope::from_str(&String::from_utf8_lossy(
            &shown.stdout,
        ))))
    } else if is_not_repo(&shown.stderr) {
        return DiffResult::NotARepository;
    } else if missing_from_head(&shown.stderr) {
        None
    } else {
        return DiffResult::Failed(short_error(&shown.stderr));
    };
    DiffResult::Diff(diff_lines(old.as_deref().unwrap_or(&[]), &rope_lines(rope)))
}

/// Line diff of `old` against `new`. `old` is empty when the file is not in `HEAD`.
pub fn diff_lines(old: &[String], new: &[String]) -> FileDiff {
    diff_lines_limited(old, new, 400_000)
}

fn diff_lines_limited(old: &[String], new: &[String], limit: usize) -> FileDiff {
    let mut prefix = 0;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }
    let mut old_end = old.len();
    let mut new_end = new.len();
    while old_end > prefix && new_end > prefix && old[old_end - 1] == new[new_end - 1] {
        old_end -= 1;
        new_end -= 1;
    }
    let old_mid = &old[prefix..old_end];
    let new_mid = &new[prefix..new_end];
    if old_mid.is_empty() && new_mid.is_empty() {
        return FileDiff::default();
    }
    if old_mid.is_empty()
        || new_mid.is_empty()
        || old_mid.len().saturating_mul(new_mid.len()) > limit
    {
        return classify_fallback(old_mid, new_mid, prefix);
    }
    classify_ops(&lcs_ops(old_mid, new_mid), prefix)
}

fn classify_fallback(old: &[String], new: &[String], at: usize) -> FileDiff {
    let paired = old.len().min(new.len());
    let mut kinds = Vec::new();
    for index in 0..paired {
        if old[index] != new[index] {
            kinds.push((at + index, LineKind::Modified));
        }
    }
    for index in paired..new.len() {
        kinds.push((at + index, LineKind::Added));
    }
    let mut deletions = Vec::new();
    if old.len() > new.len() {
        deletions.push(Deletion {
            before_line: at + new.len(),
            count: (old.len() - new.len()) as u32,
        });
    }
    FileDiff { kinds, deletions }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Equal,
    Insert,
    Delete,
}

fn lcs_ops(old: &[String], new: &[String]) -> Vec<Op> {
    let width = new.len() + 1;
    let mut lengths = vec![0u32; (old.len() + 1) * width];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lengths[i * width + j] = if old[i] == new[j] {
                lengths[(i + 1) * width + (j + 1)] + 1
            } else {
                lengths[(i + 1) * width + j].max(lengths[i * width + (j + 1)])
            };
        }
    }
    let mut ops = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            ops.push(Op::Equal);
            i += 1;
            j += 1;
        } else if lengths[(i + 1) * width + j] >= lengths[i * width + (j + 1)] {
            ops.push(Op::Delete);
            i += 1;
        } else {
            ops.push(Op::Insert);
            j += 1;
        }
    }
    while i < old.len() {
        ops.push(Op::Delete);
        i += 1;
    }
    while j < new.len() {
        ops.push(Op::Insert);
        j += 1;
    }
    ops
}

/// A run of deletes and inserts between equal lines becomes modified, added, and deleted.
/// The first paired new lines are modifications; extra new lines are additions; extra old
/// lines were removed just above the next kept line.
fn classify_ops(ops: &[Op], mut new_line: usize) -> FileDiff {
    let mut kinds = Vec::new();
    let mut deletions = Vec::new();
    let mut index = 0;
    while index < ops.len() {
        if ops[index] == Op::Equal {
            new_line += 1;
            index += 1;
            continue;
        }
        let start = new_line;
        let mut deleted = 0usize;
        let mut inserted = 0usize;
        while index < ops.len() && ops[index] != Op::Equal {
            match ops[index] {
                Op::Delete => deleted += 1,
                Op::Insert => inserted += 1,
                Op::Equal => {}
            }
            index += 1;
        }
        let paired = deleted.min(inserted);
        for offset in 0..paired {
            kinds.push((start + offset, LineKind::Modified));
        }
        for offset in paired..inserted {
            kinds.push((start + offset, LineKind::Added));
        }
        if deleted > inserted {
            deletions.push(Deletion {
                before_line: start + inserted,
                count: (deleted - inserted) as u32,
            });
        }
        new_line = start + inserted;
    }
    FileDiff { kinds, deletions }
}

fn rope_lines(rope: &Rope) -> Vec<String> {
    (0..rope.len_lines())
        .map(|line| {
            let slice = rope.line(line);
            let mut end = slice.len_chars();
            if end > 0 && slice.char(end - 1) == '\n' {
                end -= 1;
            }
            if end > 0 && slice.char(end - 1) == '\r' {
                end -= 1;
            }
            slice.slice(..end).to_string()
        })
        .collect()
}

fn existing_dir(path: &Path) -> Option<&Path> {
    let dir = if path.is_dir() { path } else { path.parent()? };
    if dir.as_os_str().is_empty() {
        return None;
    }
    if dir.exists() {
        Some(dir)
    } else {
        dir.parent().filter(|parent| parent.exists())
    }
}

fn repo_relative(path: &Path, root: &Path) -> Option<String> {
    let file_keys = path_keys(path);
    let root_keys = path_keys(root);
    if root_keys.is_empty()
        || file_keys.len() < root_keys.len()
        || file_keys[..root_keys.len()] != root_keys[..]
    {
        return None;
    }
    let skip = root
        .components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .count();
    let names: Vec<String> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .skip(skip)
        .collect();
    Some(names.join("/"))
}

fn path_keys(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Prefix(prefix) => {
                Some(prefix.as_os_str().to_string_lossy().to_ascii_lowercase())
            }
            Component::Normal(name) => Some(name.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

fn is_not_repo(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr)
        .to_ascii_lowercase()
        .contains("not a git repository")
}

fn missing_from_head(stderr: &[u8]) -> bool {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    text.contains("does not exist")
        || text.contains("exists on disk")
        || text.contains("invalid object name")
}

fn short_error(stderr: &[u8]) -> String {
    let line = String::from_utf8_lossy(stderr)
        .lines()
        .next()
        .unwrap_or("git failed")
        .trim()
        .trim_start_matches("fatal: ")
        .to_string();
    if line.chars().count() > 120 {
        let mut cut: String = line.chars().take(119).collect();
        cut.push('…');
        cut
    } else if line.is_empty() {
        "git failed".to_string()
    } else {
        line
    }
}

fn git_command(cwd: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(cwd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_repo(name: &str, head: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("duckx-git-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("src/nested")).unwrap();
        fs::write(root.join(".git/HEAD"), head).unwrap();
        root
    }

    #[test]
    fn reads_branch_from_nested_directory() {
        let root = temp_repo("branch", "ref: refs/heads/feature/status-bar\n");
        assert_eq!(
            current_branch(&root.join("src/nested")).as_deref(),
            Some("feature/status-bar")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn porcelain_statuses_propagate_to_parent_dirs() {
        let raw = b"?? src/new.rs\0 M src/ui/mod.rs\0A  docs/a.md\0R  docs/b.md\0docs/old.md\0!! target/\0";
        let statuses = parse_porcelain(raw, "");
        let get = |p: &str| {
            statuses
                .get(&Path::new(p).components().collect::<PathBuf>())
                .copied()
        };

        assert_eq!(get("src/new.rs"), Some(GitStatus::Untracked));
        assert_eq!(get("src/ui/mod.rs"), Some(GitStatus::Modified));
        assert_eq!(get("src/ui"), Some(GitStatus::Modified));
        assert_eq!(get("src"), Some(GitStatus::Modified));
        assert_eq!(get("docs/b.md"), Some(GitStatus::Renamed));
        assert_eq!(get("docs"), Some(GitStatus::Added));
        assert_eq!(get("docs/old.md"), None);
        assert_eq!(get("target"), None);
    }

    #[test]
    fn porcelain_prefix_limits_to_subdirectory() {
        let raw = b"?? sub/a.rs\0?? other/b.rs\0";
        let statuses = parse_porcelain(raw, "sub/");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses.get(Path::new("a.rs")), Some(&GitStatus::Untracked));
    }

    #[test]
    fn detached_head_shows_short_hash() {
        let root = temp_repo("detached", "0123456789abcdef0123456789abcdef01234567\n");
        assert_eq!(current_branch(&root).as_deref(), Some("0123456"));
        fs::remove_dir_all(root).unwrap();
    }

    fn owned(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| (*line).to_string()).collect()
    }

    #[test]
    fn line_diff_marks_changes_additions_and_deletions() {
        let same = diff_lines(&owned(&["a", "b"]), &owned(&["a", "b"]));
        assert!(same.is_empty());

        let added = diff_lines(&owned(&["a", "c"]), &owned(&["a", "b", "c"]));
        assert_eq!(added.kind(1), Some(LineKind::Added));
        assert_eq!(added.deleted_above(1), 0);
        assert!(added.kind(0).is_none());

        let removed = diff_lines(&owned(&["a", "b", "c"]), &owned(&["a", "c"]));
        assert_eq!(removed.deleted_above(1), 1);
        assert!(removed.kind(1).is_none());

        let replaced = diff_lines(&owned(&["a", "b", "c"]), &owned(&["a", "B", "c"]));
        assert_eq!(replaced.kind(1), Some(LineKind::Modified));
        assert_eq!(replaced.deleted_above(2), 0);

        let grown = diff_lines(&owned(&["a", "b", "c"]), &owned(&["a", "B", "D", "c"]));
        assert_eq!(grown.kind(1), Some(LineKind::Modified));
        assert_eq!(grown.kind(2), Some(LineKind::Added));

        let shrunk = diff_lines(&owned(&["a", "b", "d", "c"]), &owned(&["a", "B", "c"]));
        assert_eq!(shrunk.kind(1), Some(LineKind::Modified));
        assert_eq!(shrunk.deleted_above(2), 1);

        let at_end = diff_lines(&owned(&["a", "b"]), &owned(&["a"]));
        assert_eq!(at_end.deleted_above(1), 1);

        let at_start = diff_lines(&owned(&["b", "a"]), &owned(&["a"]));
        assert_eq!(at_start.deleted_above(0), 1);

        let untracked = diff_lines(&[], &owned(&["a", "b"]));
        assert_eq!(untracked.kind(0), Some(LineKind::Added));
        assert_eq!(untracked.kind(1), Some(LineKind::Added));
    }

    #[test]
    fn a_huge_middle_falls_back_without_pairing_equal_lines() {
        let old = owned(&["same", "old"]);
        let new = owned(&["same", "new", "extra"]);
        let diff = diff_lines_limited(&old, &new, 0);
        assert!(diff.kind(0).is_none());
        assert_eq!(diff.kind(1), Some(LineKind::Modified));
        assert_eq!(diff.kind(2), Some(LineKind::Added));
    }

    #[test]
    fn repo_relative_drops_the_toplevel_prefix() {
        let root = std::env::temp_dir().join("Repo");
        let file = root.join("src").join("main.rs");
        assert_eq!(repo_relative(&file, &root).as_deref(), Some("src/main.rs"));
    }

    #[test]
    fn diff_against_head_reads_a_commit_and_rejects_a_plain_folder() {
        let version = Command::new("git").arg("--version").output();
        if version.is_err() || !version.unwrap().status.success() {
            return;
        }
        let root = std::env::temp_dir().join(format!("duckx-diff-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        let file = root.join("src").join("a.txt");
        fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(&root)
                .status()
                .unwrap();
            assert!(status.success(), "{args:?}");
        };
        git(&["init"]);
        git(&["config", "user.email", "duckx@example.com"]);
        git(&["config", "user.name", "duckx"]);
        git(&["add", "src/a.txt"]);
        git(&["commit", "-m", "init"]);

        let rope = Rope::from_str("alpha\nBETA\ngamma\nextra\n");
        match diff_against_head(&file, &rope) {
            DiffResult::Diff(diff) => {
                assert!(diff.kind(0).is_none());
                assert_eq!(diff.kind(1), Some(LineKind::Modified));
                assert!(diff.kind(2).is_none());
                assert_eq!(diff.kind(3), Some(LineKind::Added));
            }
            other => panic!("expected a diff, got {other:?}"),
        }

        let plain = std::env::temp_dir().join(format!("duckx-diff-plain-{}", std::process::id()));
        let _ = fs::remove_dir_all(&plain);
        fs::create_dir_all(&plain).unwrap();
        let lone = plain.join("a.txt");
        fs::write(&lone, "hi\n").unwrap();
        assert!(matches!(
            diff_against_head(&lone, &Rope::from_str("hi\n")),
            DiffResult::NotARepository
        ));
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(plain).unwrap();
    }
}
