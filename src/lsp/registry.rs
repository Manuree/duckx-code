//! Which language server serves which language, and which folder it is rooted at.

use std::path::{Path, PathBuf};

use crate::syntax::Language;

#[derive(Debug, PartialEq, Eq)]
pub struct ServerSpec {
    pub name: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
    /// Files that mark a project root, checked from the file's folder upward.
    pub root_markers: &'static [&'static str],
}

const RUST: &[ServerSpec] = &[ServerSpec {
    name: "rust-analyzer",
    command: "rust-analyzer",
    args: &[],
    root_markers: &["Cargo.toml"],
}];

const PYTHON_MARKERS: &[&str] = &[
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "requirements.txt",
    "pyrightconfig.json",
];

const PYTHON: &[ServerSpec] = &[
    ServerSpec {
        name: "pyright",
        command: "pyright-langserver",
        args: &["--stdio"],
        root_markers: PYTHON_MARKERS,
    },
    ServerSpec {
        name: "basedpyright",
        command: "basedpyright-langserver",
        args: &["--stdio"],
        root_markers: PYTHON_MARKERS,
    },
    ServerSpec {
        name: "pylsp",
        command: "pylsp",
        args: &[],
        root_markers: PYTHON_MARKERS,
    },
];

const TYPESCRIPT: &[ServerSpec] = &[ServerSpec {
    name: "typescript-language-server",
    command: "typescript-language-server",
    args: &["--stdio"],
    root_markers: &["tsconfig.json", "jsconfig.json", "package.json"],
}];

const GO: &[ServerSpec] = &[ServerSpec {
    name: "gopls",
    command: "gopls",
    args: &[],
    root_markers: &["go.work", "go.mod"],
}];

const CLANGD: &[ServerSpec] = &[ServerSpec {
    name: "clangd",
    command: "clangd",
    args: &[],
    root_markers: &[
        "compile_commands.json",
        "compile_flags.txt",
        ".clangd",
        "CMakeLists.txt",
    ],
}];

/// Programs to try for `language`, most preferred first. Empty when none is configured.
pub fn servers_for(language: Language) -> &'static [ServerSpec] {
    match language {
        Language::Rust => RUST,
        Language::Python => PYTHON,
        Language::JavaScript | Language::TypeScript | Language::Tsx => TYPESCRIPT,
        Language::Go => GO,
        Language::C | Language::Cpp => CLANGD,
        _ => &[],
    }
}

/// True when some server is configured for `language`.
pub fn supports(language: Language) -> bool {
    !servers_for(language).is_empty()
}

/// The LSP `languageId` for `path`.
pub fn language_id(language: Language, path: &Path) -> &'static str {
    let jsx = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("jsx"));
    match language {
        Language::Rust => "rust",
        Language::Python => "python",
        Language::JavaScript if jsx => "javascriptreact",
        Language::JavaScript => "javascript",
        Language::TypeScript => "typescript",
        Language::Tsx => "typescriptreact",
        Language::Go => "go",
        Language::C => "c",
        Language::Cpp => "cpp",
        _ => "plaintext",
    }
}

/// Root folder for a server handling `file`.
///
/// Inside `workspace`, this is the highest folder with a marker, up to the workspace
/// itself, or the workspace when none has one; one server then covers a whole
/// Cargo or npm workspace. Outside it, the highest marked folder above the file, or the file's folder.
pub fn find_root(file: &Path, workspace: &Path, markers: &[&str]) -> PathBuf {
    find_root_with(file, workspace, markers, |path| path.exists())
}

fn find_root_with(
    file: &Path,
    workspace: &Path,
    markers: &[&str],
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    let parent = file.parent().unwrap_or(file).to_path_buf();
    let inside = file.starts_with(workspace);
    let mut top = None;
    for dir in file.ancestors().skip(1) {
        if markers.iter().any(|marker| exists(&dir.join(marker))) {
            top = Some(dir.to_path_buf());
        }
        if inside && dir == workspace {
            break;
        }
    }
    match top {
        Some(dir) => dir,
        None if inside => workspace.to_path_buf(),
        None => parent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn root(file: &str, workspace: &str, present: &[&str]) -> PathBuf {
        let present: HashSet<PathBuf> = present.iter().map(PathBuf::from).collect();
        find_root_with(
            Path::new(file),
            Path::new(workspace),
            &["Cargo.toml"],
            |path| present.contains(path),
        )
    }

    #[test]
    fn cargo_workspace_root_wins_over_the_member_crate() {
        assert_eq!(
            root(
                "/w/crates/a/src/lib.rs",
                "/w",
                &["/w/Cargo.toml", "/w/crates/a/Cargo.toml"]
            ),
            PathBuf::from("/w")
        );
    }

    #[test]
    fn markers_above_the_workspace_are_ignored() {
        assert_eq!(
            root("/w/proj/src/main.rs", "/w/proj", &["/w/Cargo.toml"]),
            PathBuf::from("/w/proj")
        );
    }

    #[test]
    fn a_nested_project_inside_the_workspace_is_found() {
        assert_eq!(
            root(
                "/w/tools/gen/src/main.rs",
                "/w",
                &["/w/tools/gen/Cargo.toml"]
            ),
            PathBuf::from("/w/tools/gen")
        );
    }

    #[test]
    fn files_outside_the_workspace_use_their_own_markers_or_folder() {
        assert_eq!(
            root("/other/p/src/x.rs", "/w", &["/other/p/Cargo.toml"]),
            PathBuf::from("/other/p")
        );
        assert_eq!(root("/tmp/x.rs", "/w", &[]), PathBuf::from("/tmp"));
    }

    #[test]
    fn languages_share_servers_where_one_program_serves_both() {
        assert_eq!(servers_for(Language::C), servers_for(Language::Cpp));
        assert_eq!(
            servers_for(Language::Tsx)[0].name,
            servers_for(Language::JavaScript)[0].name
        );
        assert!(!supports(Language::Markdown));
        assert_eq!(
            language_id(Language::JavaScript, Path::new("a.jsx")),
            "javascriptreact"
        );
    }
}
