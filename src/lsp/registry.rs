//! Which language server serves which language, and which folder it is rooted at.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::settings::{CustomServer, LspPrefs};

/// One language server program and the files it is used for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSpec {
    /// Unique key: the row in Settings → Language servers, `lsp.servers` in
    /// `settings.json`, the status bar, and the log file. Built-ins use the program name.
    pub name: String,
    /// What the server is for, such as `Rust` or `Python`.
    pub title: String,
    pub command: String,
    pub args: Vec<String>,
    /// Lowercase file extension, or a whole lowercase file name such as `gemfile`,
    /// with the LSP `languageId` sent for it.
    pub languages: Vec<(String, String)>,
    /// Files that mark a project root, checked from the file's folder upward.
    pub root_markers: Vec<String>,
    /// How to install it. Empty for a custom server.
    pub install: String,
    /// Added by the user in Settings.
    pub custom: bool,
}

impl ServerSpec {
    /// The `languageId` for `path`, when this server takes that file.
    pub fn language_id(&self, path: &Path) -> Option<&str> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_ascii_lowercase);
        let ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase);
        self.languages
            .iter()
            .find(|(key, _)| Some(key) == ext.as_ref())
            .or_else(|| {
                self.languages
                    .iter()
                    .find(|(key, _)| Some(key) == name.as_ref())
            })
            .map(|(_, id)| id.as_str())
    }

    pub fn handles(&self, path: &Path) -> bool {
        self.language_id(path).is_some()
    }

    /// Extensions for the settings row: `ts, tsx, js`.
    pub fn extensions_label(&self) -> String {
        let mut seen = Vec::new();
        for (key, _) in &self.languages {
            if !seen.contains(key) {
                seen.push(key.clone());
            }
        }
        seen.join(", ")
    }

    /// A user's server from `settings.json`. `None` until it has a command and an extension.
    pub fn from_custom(server: &CustomServer) -> Option<Self> {
        let command = server.command.trim();
        if command.is_empty() || server.extensions.is_empty() || server.name.trim().is_empty() {
            return None;
        }
        let language_id = match server.language_id.trim() {
            "" => server.extensions[0].clone(),
            id => id.to_string(),
        };
        Some(Self {
            name: server.name.trim().to_string(),
            title: server.name.trim().to_string(),
            command: command.to_string(),
            args: server.args.clone(),
            languages: server
                .extensions
                .iter()
                .map(|ext| (ext.clone(), language_id.clone()))
                .collect(),
            root_markers: server.root_markers.clone(),
            install: String::new(),
            custom: true,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn builtin(
    name: &str,
    title: &str,
    command: &str,
    args: &[&str],
    languages: &[(&str, &str)],
    root_markers: &[&str],
    install: &str,
) -> Arc<ServerSpec> {
    Arc::new(ServerSpec {
        name: name.to_string(),
        title: title.to_string(),
        command: command.to_string(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
        languages: languages
            .iter()
            .map(|(ext, id)| (ext.to_string(), id.to_string()))
            .collect(),
        root_markers: root_markers.iter().map(|m| m.to_string()).collect(),
        install: install.to_string(),
        custom: false,
    })
}

const PYTHON: &[(&str, &str)] = &[("py", "python"), ("pyi", "python")];
const PYTHON_MARKERS: &[&str] = &[
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "requirements.txt",
    "pyrightconfig.json",
];
const NODE_LANGSERVERS: &str = "npm install -g vscode-langservers-extracted";

/// Servers that come with duckx-code, in the order they are tried for a file.
/// Several may take the same file; the first enabled one that is installed runs.
pub fn builtins() -> &'static [Arc<ServerSpec>] {
    static BUILTINS: OnceLock<Vec<Arc<ServerSpec>>> = OnceLock::new();
    BUILTINS.get_or_init(|| {
        vec![
            builtin(
                "rust-analyzer",
                "Rust",
                "rust-analyzer",
                &[],
                &[("rs", "rust")],
                &["Cargo.toml"],
                "rustup component add rust-analyzer",
            ),
            builtin(
                "pyright",
                "Python",
                "pyright-langserver",
                &["--stdio"],
                PYTHON,
                PYTHON_MARKERS,
                "npm install -g pyright",
            ),
            builtin(
                "basedpyright",
                "Python",
                "basedpyright-langserver",
                &["--stdio"],
                PYTHON,
                PYTHON_MARKERS,
                "pip install basedpyright",
            ),
            builtin(
                "pylsp",
                "Python",
                "pylsp",
                &[],
                PYTHON,
                PYTHON_MARKERS,
                "pip install python-lsp-server",
            ),
            builtin(
                "ruff",
                "Python (Ruff)",
                "ruff",
                &["server"],
                PYTHON,
                &[
                    "pyproject.toml",
                    "ruff.toml",
                    ".ruff.toml",
                    "setup.py",
                    "requirements.txt",
                ],
                "pip install ruff",
            ),
            builtin(
                "typescript-language-server",
                "TypeScript",
                "typescript-language-server",
                &["--stdio"],
                &[
                    ("ts", "typescript"),
                    ("mts", "typescript"),
                    ("cts", "typescript"),
                    ("tsx", "typescriptreact"),
                    ("js", "javascript"),
                    ("mjs", "javascript"),
                    ("cjs", "javascript"),
                    ("jsx", "javascriptreact"),
                ],
                &["tsconfig.json", "jsconfig.json", "package.json"],
                "npm install -g typescript-language-server typescript",
            ),
            builtin(
                "gopls",
                "Go",
                "gopls",
                &[],
                &[("go", "go")],
                &["go.work", "go.mod"],
                "go install golang.org/x/tools/gopls@latest",
            ),
            builtin(
                "clangd",
                "C/C++",
                "clangd",
                &[],
                &[
                    ("c", "c"),
                    ("h", "c"),
                    ("cc", "cpp"),
                    ("cpp", "cpp"),
                    ("cxx", "cpp"),
                    ("c++", "cpp"),
                    ("hh", "cpp"),
                    ("hpp", "cpp"),
                    ("hxx", "cpp"),
                    ("h++", "cpp"),
                ],
                &[
                    "compile_commands.json",
                    "compile_flags.txt",
                    ".clangd",
                    "CMakeLists.txt",
                ],
                if cfg!(windows) {
                    "winget install LLVM.LLVM"
                } else {
                    "install clangd (LLVM): clangd.llvm.org/installation"
                },
            ),
            builtin(
                "zls",
                "Zig",
                "zls",
                &[],
                &[("zig", "zig"), ("zon", "zig")],
                &["build.zig"],
                "see zigtools.org/zls/install",
            ),
            builtin(
                "lua-language-server",
                "Lua",
                "lua-language-server",
                &[],
                &[("lua", "lua")],
                &[".luarc.json", ".luarc.jsonc"],
                "see luals.github.io/#install",
            ),
            builtin(
                "ruby-lsp",
                "Ruby",
                "ruby-lsp",
                &[],
                &[
                    ("rb", "ruby"),
                    ("rake", "ruby"),
                    ("gemspec", "ruby"),
                    ("gemfile", "ruby"),
                    ("rakefile", "ruby"),
                ],
                &["Gemfile"],
                "gem install ruby-lsp",
            ),
            builtin(
                "intelephense",
                "PHP",
                "intelephense",
                &["--stdio"],
                &[("php", "php"), ("phtml", "php")],
                &["composer.json"],
                "npm install -g intelephense",
            ),
            builtin(
                "yaml-language-server",
                "YAML",
                "yaml-language-server",
                &["--stdio"],
                &[("yaml", "yaml"), ("yml", "yaml")],
                &[".git"],
                "npm install -g yaml-language-server",
            ),
            builtin(
                "bash-language-server",
                "Shell",
                "bash-language-server",
                &["start"],
                &[
                    ("sh", "shellscript"),
                    ("bash", "shellscript"),
                    ("zsh", "shellscript"),
                ],
                &[".git"],
                "npm install -g bash-language-server",
            ),
            builtin(
                "vscode-json-language-server",
                "JSON",
                "vscode-json-language-server",
                &["--stdio"],
                &[("json", "json"), ("jsonc", "jsonc")],
                &[".git"],
                NODE_LANGSERVERS,
            ),
            builtin(
                "vscode-css-language-server",
                "CSS",
                "vscode-css-language-server",
                &["--stdio"],
                &[("css", "css"), ("scss", "scss"), ("less", "less")],
                &["package.json", ".git"],
                NODE_LANGSERVERS,
            ),
            builtin(
                "vscode-html-language-server",
                "HTML",
                "vscode-html-language-server",
                &["--stdio"],
                &[("html", "html"), ("htm", "html")],
                &["package.json", ".git"],
                NODE_LANGSERVERS,
            ),
            builtin(
                "svelteserver",
                "Svelte",
                "svelteserver",
                &["--stdio"],
                &[("svelte", "svelte")],
                &["svelte.config.js", "package.json"],
                "npm install -g svelte-language-server",
            ),
            builtin(
                "vue-language-server",
                "Vue",
                "vue-language-server",
                &["--stdio"],
                &[("vue", "vue")],
                &["vite.config.ts", "vite.config.js", "package.json"],
                "npm install -g @vue/language-server",
            ),
            builtin(
                "sourcekit-lsp",
                "Swift",
                "sourcekit-lsp",
                &[],
                &[("swift", "swift")],
                &["Package.swift"],
                if cfg!(target_os = "macos") {
                    "xcode-select --install"
                } else {
                    "comes with the Swift toolchain: swift.org/install"
                },
            ),
        ]
    })
}

/// Every server and whether it is used, built from the Language server settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    /// Built-ins, then custom servers: the order rows are shown in Settings.
    listing: Vec<Arc<ServerSpec>>,
    /// Names switched off in Settings.
    off: HashSet<String>,
}

/// Every built-in, all on.
impl Default for Registry {
    fn default() -> Self {
        Self::new(&LspPrefs::default())
    }
}

impl Registry {
    pub fn new(prefs: &LspPrefs) -> Self {
        let mut listing: Vec<Arc<ServerSpec>> = builtins().to_vec();
        listing.extend(
            prefs
                .custom
                .iter()
                .filter_map(ServerSpec::from_custom)
                .map(Arc::new),
        );
        let off = listing
            .iter()
            .filter(|spec| !prefs.server_enabled(&spec.name))
            .map(|spec| spec.name.clone())
            .collect();
        Self { listing, off }
    }

    /// Built-ins first, then complete custom servers.
    pub fn listing(&self) -> &[Arc<ServerSpec>] {
        &self.listing
    }

    pub fn find(&self, name: &str) -> Option<&Arc<ServerSpec>> {
        self.listing.iter().find(|spec| spec.name == name)
    }

    pub fn enabled(&self, name: &str) -> bool {
        !self.off.contains(name)
    }

    /// Enabled servers that take `path`, most preferred first: custom servers win
    /// over built-ins, then the order of [`builtins`]. The worker starts the first
    /// one that is installed.
    pub fn candidates(&self, path: &Path) -> Vec<Arc<ServerSpec>> {
        let usable = |spec: &&Arc<ServerSpec>| spec.handles(path) && self.enabled(&spec.name);
        let custom = self
            .listing
            .iter()
            .filter(|spec| spec.custom)
            .filter(usable);
        let builtin = self
            .listing
            .iter()
            .filter(|spec| !spec.custom)
            .filter(usable);
        custom.chain(builtin).cloned().collect()
    }

    /// The key a file's server is known by: the first candidate's name.
    pub fn group(&self, path: &Path) -> Option<String> {
        let custom_first = self
            .listing
            .iter()
            .filter(|spec| spec.custom)
            .chain(self.listing.iter().filter(|spec| !spec.custom));
        custom_first
            .filter(|spec| spec.handles(path) && self.enabled(&spec.name))
            .map(|spec| spec.name.clone())
            .next()
    }

    pub fn supports(&self, path: &Path) -> bool {
        self.group(path).is_some()
    }
}

/// Root folder for a server handling `file`.
///
/// Inside `workspace`, this is the highest folder with a marker, up to the workspace
/// itself, or the workspace when none has one; one server then covers a whole
/// Cargo or npm workspace. Outside it, the highest marked folder above the file, or the file's folder.
pub fn find_root<M: AsRef<str>>(file: &Path, workspace: &Path, markers: &[M]) -> PathBuf {
    find_root_with(file, workspace, markers, |path| path.exists())
}

fn find_root_with<M: AsRef<str>>(
    file: &Path,
    workspace: &Path,
    markers: &[M],
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    let parent = file.parent().unwrap_or(file).to_path_buf();
    let inside = file.starts_with(workspace);
    let mut top = None;
    for dir in file.ancestors().skip(1) {
        if markers
            .iter()
            .any(|marker| exists(&dir.join(marker.as_ref())))
        {
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

    fn reg(prefs: &LspPrefs) -> Registry {
        Registry::new(prefs)
    }

    fn names(specs: &[Arc<ServerSpec>]) -> Vec<&str> {
        specs.iter().map(|spec| spec.name.as_str()).collect()
    }

    fn custom(name: &str, command: &str, exts: &[&str]) -> CustomServer {
        CustomServer {
            name: name.into(),
            command: command.into(),
            extensions: exts.iter().map(|e| e.to_string()).collect(),
            ..CustomServer::default()
        }
    }

    #[test]
    fn files_are_routed_by_extension_and_keep_the_old_fallbacks() {
        let r = reg(&LspPrefs::default());
        assert_eq!(names(&r.candidates(Path::new("a.rs"))), ["rust-analyzer"]);
        assert_eq!(
            names(&r.candidates(Path::new("x/a.PY"))),
            ["pyright", "basedpyright", "pylsp", "ruff"]
        );
        let c = r.candidates(Path::new("a.c"));
        assert_eq!(names(&c), names(&r.candidates(Path::new("a.hpp"))));
        assert_eq!(c[0].language_id(Path::new("a.hpp")), Some("cpp"));
        assert_eq!(
            r.candidates(Path::new("a.jsx"))[0].language_id(Path::new("a.jsx")),
            Some("javascriptreact")
        );
        assert_eq!(names(&r.candidates(Path::new("Gemfile"))), ["ruby-lsp"]);
        assert_eq!(
            names(&r.candidates(Path::new("a.vue"))),
            ["vue-language-server"]
        );
        assert!(r.candidates(Path::new("README.md")).is_empty());
        assert!(r.candidates(Path::new("noext")).is_empty());
        assert!(!r.supports(Path::new("a.toml")));
        assert_eq!(r.group(Path::new("a.py")).as_deref(), Some("pyright"));
    }

    #[test]
    fn a_switched_off_server_is_skipped_and_the_next_one_wins() {
        let mut prefs = LspPrefs::default();
        prefs.servers.insert("pyright".into(), false);
        let r = reg(&prefs);
        assert_eq!(
            names(&r.candidates(Path::new("a.py"))),
            ["basedpyright", "pylsp", "ruff"]
        );
        assert_eq!(r.group(Path::new("a.py")).as_deref(), Some("basedpyright"));
        prefs.servers.insert("rust-analyzer".into(), false);
        assert!(reg(&prefs).candidates(Path::new("a.rs")).is_empty());
    }

    #[test]
    fn a_custom_server_wins_over_built_ins_and_works_without_a_grammar() {
        let mut prefs = LspPrefs::default();
        prefs.custom.push(custom("my-pyls", "mypy-ls", &["py"]));
        prefs.custom.push(custom("templ", "templ", &["templ"]));
        prefs.custom.push(custom("half done", "", &["x"]));
        let r = reg(&prefs);
        assert_eq!(
            names(&r.candidates(Path::new("a.py")))[..2],
            ["my-pyls", "pyright"]
        );
        let templ = r.candidates(Path::new("views/a.templ"));
        assert_eq!(names(&templ), ["templ"]);
        assert_eq!(templ[0].language_id(Path::new("a.templ")), Some("templ"));
        // No command yet: listed nowhere, routes nothing.
        assert!(r.find("half done").is_none());
        assert!(r.candidates(Path::new("a.x")).is_empty());
        assert_eq!(r.listing().len(), builtins().len() + 2);
    }

    #[test]
    fn every_built_in_has_a_command_an_extension_and_an_install_hint() {
        let mut seen = HashSet::new();
        for spec in builtins() {
            assert!(seen.insert(spec.name.clone()), "{} twice", spec.name);
            assert!(!spec.command.is_empty() && !spec.languages.is_empty());
            assert!(
                !spec.install.is_empty(),
                "{} has no install hint",
                spec.name
            );
            assert!(!spec.custom);
        }
        for program in [
            "zls",
            "lua-language-server",
            "ruby-lsp",
            "intelephense",
            "yaml-language-server",
            "bash-language-server",
            "vscode-json-language-server",
            "vscode-css-language-server",
            "vscode-html-language-server",
            "svelteserver",
            "vue-language-server",
            "sourcekit-lsp",
            "ruff",
        ] {
            assert!(
                builtins().iter().any(|spec| spec.command == program),
                "{program}"
            );
        }
    }
}
