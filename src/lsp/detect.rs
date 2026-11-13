//! Finds a language server program on `PATH` without starting it.
//!
//! On Windows a bare command such as `pyright-langserver` is tried with every
//! `PATHEXT` extension (`.COM;.EXE;.BAT;.CMD` when unset), so npm's `.cmd`
//! launchers are found. Elsewhere the file must be executable. Nothing here runs
//! a program or a shell.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// The full path `command` would start, using this process's `PATH`.
pub fn find_program(command: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let pathext = std::env::var_os("PATHEXT");
    find_in(command, &path, pathext.as_deref(), cfg!(windows), runnable)
}

fn runnable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// [`find_program`] with the search path, `PATHEXT`, platform, and file test given,
/// so the Windows rules can be tested anywhere.
pub fn find_in(
    command: &str,
    path: &OsStr,
    pathext: Option<&OsStr>,
    windows: bool,
    runnable: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let command = command.trim();
    if command.is_empty() {
        return None;
    }
    let names = names_for(command, pathext, windows);
    let has_separator = command.contains('/') || (windows && command.contains('\\'));
    if has_separator || Path::new(command).is_absolute() {
        let dir = Path::new(command).parent().unwrap_or(Path::new(""));
        return names
            .iter()
            .map(|name| dir.join(name))
            .find(|candidate| runnable(candidate));
    }
    let separator = if windows { ';' } else { ':' };
    let path = path.to_string_lossy();
    for dir in path.split(separator) {
        let dir = dir.trim().trim_matches('"');
        if dir.is_empty() {
            continue;
        }
        for name in &names {
            let candidate = Path::new(dir).join(name);
            if runnable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// File names to look for. On Windows a name that already ends in a `PATHEXT`
/// extension is tried as it is first, then every extension is appended.
fn names_for(command: &str, pathext: Option<&OsStr>, windows: bool) -> Vec<String> {
    let file = Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command)
        .to_string();
    if !windows {
        return vec![file];
    }
    let exts: Vec<String> = pathext
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PATHEXT.to_string())
        .split(';')
        .map(|ext| ext.trim().to_ascii_lowercase())
        .filter(|ext| ext.starts_with('.') && ext.len() > 1)
        .collect();
    let lower = file.to_ascii_lowercase();
    let mut names = Vec::new();
    if exts.iter().any(|ext| lower.ends_with(ext.as_str())) {
        names.push(file.clone());
    }
    names.extend(exts.iter().map(|ext| format!("{file}{ext}")));
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn on(files: &[&str]) -> impl Fn(&Path) -> bool {
        let files: HashSet<PathBuf> = files.iter().map(PathBuf::from).collect();
        move |path| files.contains(path)
    }

    fn win(command: &str, path: &str, pathext: Option<&str>, files: &[&str]) -> Option<PathBuf> {
        find_in(
            command,
            OsStr::new(path),
            pathext.map(OsStr::new),
            true,
            on(files),
        )
    }

    #[test]
    fn windows_finds_npm_cmd_launchers_through_pathext() {
        let npm = Path::new(r"C:\npm").join("pyright-langserver.cmd");
        assert_eq!(
            win(
                "pyright-langserver",
                r"C:\bin;C:\npm",
                Some(".COM;.EXE;.BAT;.CMD"),
                &[npm.to_str().unwrap()]
            ),
            Some(npm)
        );
    }

    #[test]
    fn windows_prefers_the_earlier_folder_and_pathext_order() {
        let exe = Path::new(r"C:\bin").join("gopls.exe");
        let bat = Path::new(r"C:\bin").join("gopls.bat");
        let later = Path::new(r"D:\go").join("gopls.exe");
        let files = [
            later.to_str().unwrap(),
            bat.to_str().unwrap(),
            exe.to_str().unwrap(),
        ];
        assert_eq!(win("gopls", r"C:\bin;D:\go", None, &files), Some(exe));
        // A quoted folder and an empty entry are fine.
        let quoted = Path::new(r"C:\Program Files\LLVM\bin").join("clangd.exe");
        assert_eq!(
            win(
                "clangd",
                r#";"C:\Program Files\LLVM\bin";"#,
                Some(".exe"),
                &[quoted.to_str().unwrap()]
            ),
            Some(quoted)
        );
    }

    #[test]
    fn windows_takes_a_command_that_already_has_its_extension() {
        let cmd = Path::new(r"C:\npm").join("intelephense.CMD");
        assert_eq!(
            win(
                "intelephense.CMD",
                r"C:\npm",
                None,
                &[cmd.to_str().unwrap()]
            ),
            Some(cmd)
        );
        // A bare name never matches a file without a PATHEXT extension.
        let bare = Path::new(r"C:\npm").join("zls");
        assert_eq!(win("zls", r"C:\npm", None, &[bare.to_str().unwrap()]), None);
    }

    #[test]
    fn a_path_with_a_folder_is_checked_where_it_points() {
        assert_eq!(
            win(r"C:\tools\zls", "", None, &[r"C:\tools\zls.exe"]),
            Some(PathBuf::from(r"C:\tools\zls.exe"))
        );
        let unix = find_in(
            "/opt/zls/bin/zls",
            OsStr::new("/usr/bin"),
            None,
            false,
            on(&["/opt/zls/bin/zls"]),
        );
        assert_eq!(unix, Some(PathBuf::from("/opt/zls/bin/zls")));
    }

    #[test]
    fn unix_searches_path_in_order_without_extensions() {
        let found = find_in(
            "rust-analyzer",
            OsStr::new("/usr/bin::/home/u/.cargo/bin"),
            Some(OsStr::new(".EXE")),
            false,
            on(&[
                "/home/u/.cargo/bin/rust-analyzer",
                "/usr/bin/rust-analyzer.exe",
            ]),
        );
        assert_eq!(
            found,
            Some(PathBuf::from("/home/u/.cargo/bin/rust-analyzer"))
        );
        assert_eq!(
            find_in("", OsStr::new("/usr/bin"), None, false, |_| true),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_real_program_is_found_and_a_missing_one_is_not() {
        assert!(find_program("sh").is_some());
        assert!(find_program("duckx-no-such-server-7f3a").is_none());
    }
}
