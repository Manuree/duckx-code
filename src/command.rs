/// Which surface the Command Bar was opened from. Hints and execution stay inside it:
/// editor commands are not offered while the explorer is focused, and file commands
/// are not offered while an editor is focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommandContext {
    #[default]
    Editor,
    Explorer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandId {
    Quit,
    Undo,
    Redo,
    SelectAll,
    Indent,
    Outdent,
    GotoLine,
    Find,
    Replace,
    Copy,
    Cut,
    Paste,
    Save,
    ToggleExplorer,
    /// Grow the explorer column.
    SidebarWider,
    /// Shrink the explorer column.
    SidebarNarrower,
    /// Show or hide changes against HEAD in the gutter.
    Diff,
    /// Show only the focused buffer, without the title or status bar.
    Focus,
    /// Scroll the explorer to the file in the focused tab.
    Reveal,
    SplitRight,
    SplitDown,
    Close,
    /// Close every tab in every window.
    CloseAll,
    /// Close every tab in the focused window.
    CloseView,
    /// Select the previous tab in the focused pane.
    TabPrev,
    /// Select the next tab in the focused pane.
    TabNext,
    /// Move to the next diagnostic in this file.
    DiagnosticNext,
    /// Move to the previous diagnostic in this file.
    DiagnosticPrev,
    /// Show documentation for the symbol under the cursor.
    Hover,
    /// Jump to where the symbol under the cursor is defined.
    GotoDefinition,
    /// List every place the symbol under the cursor is used.
    GotoReferences,
    /// Jump to the bracket paired with the one under the cursor.
    MatchBracket,
    /// Open the completion list at the cursor.
    Complete,
    /// Add or remove a line comment.
    Comment,
    /// Move the selected lines up.
    MoveLineUp,
    /// Move the selected lines down.
    MoveLineDown,
    /// Delete the current line or the selected lines.
    DeleteLine,
    /// Duplicate the current line or the selected lines above.
    DuplicateLineUp,
    /// Duplicate the current line or the selected lines below.
    DuplicateLineDown,
    /// Rename the symbol under the cursor. The argument is the new name.
    RenameSymbol,
    /// Format the current file.
    Format,
    /// Show code actions at the cursor.
    CodeAction,
    /// Return to the place from before the last definition or references jump.
    JumpBack,
    /// Undo [`CommandId::JumpBack`].
    JumpForward,
    /// Enter FILE mode in the explorer.
    Select,
    /// Move explorer entries into a directory.
    Move,
    /// Delete explorer entries.
    Delete,
    /// Rename the focused explorer entry.
    Rename,
    /// Open the settings page.
    Settings,
    /// Open the about page.
    About,
    /// Open the workspace file picker.
    FilePicker,
    /// Read the focused file from disk again.
    Reload,
    /// Show or set the encoding the next save writes.
    Encoding,
}

#[derive(Debug)]
pub struct CommandSpec {
    pub id: CommandId,
    pub name: &'static str,
    pub description: &'static str,
    pub shortcut: &'static str,
    /// Placeholder shown in hints when the command requires an argument.
    pub arg: Option<&'static str>,
    /// Offered while an editor is focused.
    editor: bool,
    /// Offered while the explorer is focused.
    explorer: bool,
}

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: CommandId::GotoLine,
        name: "goto",
        description: "Go to line number",
        shortcut: "",
        arg: Some("<line>"),
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Find,
        name: "find",
        description: "Find text in the file or explorer",
        shortcut: "Ctrl+F",
        arg: None,
        editor: true,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::FilePicker,
        name: "file-picker",
        description: "Open a workspace file by name",
        shortcut: "Ctrl+P",
        arg: None,
        editor: true,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::Replace,
        name: "replace",
        description: "Replace text in the file or FILTER",
        shortcut: "Ctrl+H",
        arg: None,
        editor: true,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::ToggleExplorer,
        name: "explorer",
        description: "Toggle the file explorer sidebar",
        shortcut: "Ctrl+B",
        arg: None,
        editor: true,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::SidebarWider,
        name: "wider",
        description: "Make the explorer sidebar wider",
        shortcut: "Alt+Right",
        arg: None,
        editor: false,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::SidebarNarrower,
        name: "narrower",
        description: "Make the explorer sidebar narrower",
        shortcut: "Alt+Left",
        arg: None,
        editor: false,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::Diff,
        name: "diff",
        description: "Show changes against HEAD in the gutter",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Focus,
        name: "focus",
        description: "Show only this buffer, without the title or status bar",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Reveal,
        name: "reveal",
        description: "Show the focused file in the explorer",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::SplitRight,
        name: "split-right",
        description: "Open an empty editor to the right",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::SplitDown,
        name: "split-down",
        description: "Open an empty editor below",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Close,
        name: "close",
        description: "Close the focused tab",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::CloseAll,
        name: "close-all",
        description: "Close every tab",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::CloseView,
        name: "close-view",
        description: "Close every tab in this window",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::TabPrev,
        name: "tab-prev",
        description: "Select the previous tab in this window",
        shortcut: "Ctrl+Alt+Left",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::TabNext,
        name: "tab-next",
        description: "Select the next tab in this window",
        shortcut: "Ctrl+Alt+Right",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::DiagnosticNext,
        name: "diagnostic-next",
        description: "Jump to the next error or warning in this file",
        shortcut: "F8",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::DiagnosticPrev,
        name: "diagnostic-prev",
        description: "Jump to the previous error or warning in this file",
        shortcut: "Shift+F8",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Hover,
        name: "hover",
        description: "Show documentation for the symbol under the cursor",
        shortcut: "F2",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::GotoDefinition,
        name: "goto-definition",
        description: "Jump to where the symbol under the cursor is defined",
        shortcut: "F12",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::GotoReferences,
        name: "goto-references",
        description: "Show every place this symbol is used",
        shortcut: "Shift+F12",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::MatchBracket,
        name: "match-bracket",
        description: "Jump to the matching bracket",
        shortcut: "Ctrl+]",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Complete,
        name: "complete",
        description: "Open completions at the cursor",
        shortcut: "Ctrl+Space",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Comment,
        name: "comment",
        description: "Add or remove a comment on the selected lines",
        shortcut: "F4",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::MoveLineUp,
        name: "move-line-up",
        description: "Move the current line or selection up",
        shortcut: "Alt+Up",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::MoveLineDown,
        name: "move-line-down",
        description: "Move the current line or selection down",
        shortcut: "Alt+Down",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::DeleteLine,
        name: "delete-line",
        description: "Delete the current line or the selected lines",
        shortcut: "Shift+Delete",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::DuplicateLineUp,
        name: "duplicate-line-up",
        description: "Duplicate the current line or the selected lines above",
        shortcut: "Ctrl+Alt+Up",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::DuplicateLineDown,
        name: "duplicate-line-down",
        description: "Duplicate the current line or the selected lines below",
        shortcut: "Ctrl+Alt+Down",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::RenameSymbol,
        name: "rename-symbol",
        description: "Rename the symbol under the cursor everywhere the language server knows it",
        shortcut: "Shift+F2",
        arg: Some("name"),
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Format,
        name: "format",
        description: "Format the current file",
        shortcut: "F7",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::CodeAction,
        name: "code-action",
        description: "Show fixes and refactors at the cursor",
        shortcut: "Ctrl+.",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::JumpBack,
        name: "jump-back",
        description: "Return to the place from before the last definition or references jump",
        shortcut: "Ctrl+O",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::JumpForward,
        name: "jump-forward",
        description: "Go forward again after jump-back",
        shortcut: "Ctrl+Shift+O",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Copy,
        name: "copy",
        description: "Copy selection (or current line) to clipboard",
        shortcut: "Ctrl+C",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Cut,
        name: "cut",
        description: "Cut selection (or current line) to clipboard",
        shortcut: "Ctrl+X",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Paste,
        name: "paste",
        description: "Paste from clipboard",
        shortcut: "Ctrl+V",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Save,
        name: "save",
        description: "Save the current file",
        shortcut: "Ctrl+S",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Undo,
        name: "undo",
        description: "Undo last edit",
        shortcut: "Ctrl+Z",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Redo,
        name: "redo",
        description: "Redo last undone edit",
        shortcut: "Ctrl+Y",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::SelectAll,
        name: "select-all",
        description: "Select the entire document",
        shortcut: "Ctrl+A",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Indent,
        name: "indent",
        description: "Indent current line or selection",
        shortcut: "Tab",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Outdent,
        name: "outdent",
        description: "Outdent current line or selection",
        shortcut: "Shift+Tab",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Settings,
        name: "setting",
        description: "Open the settings page (config)",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::About,
        name: "about",
        description: "Show the version and logo",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::Quit,
        name: "quit",
        description: "Exit duckx-code",
        shortcut: "Ctrl+Q",
        arg: None,
        editor: true,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::Select,
        name: "select",
        description: "Enter FILE mode to mark files with space",
        shortcut: "",
        arg: None,
        editor: false,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::Move,
        name: "move",
        description: "Move focused or marked files into a folder",
        shortcut: "",
        arg: Some("<dir>"),
        editor: false,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::Delete,
        name: "delete",
        description: "Delete focused or marked files",
        shortcut: "",
        arg: None,
        editor: false,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::Rename,
        name: "rename",
        description: "Rename the focused file or folder",
        shortcut: "",
        arg: Some("<name>"),
        editor: false,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::Reload,
        name: "reload",
        description: "Reload the file from disk",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::Encoding,
        name: "encoding",
        description: "Show or choose the file encoding",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: false,
    },
];

/// Lower comes first in the Command Bar. Opening and search lead, then everyday
/// edits, language tools, explorer file commands, layout, and finally settings,
/// about, and quit. [`suggestions`] still pins the most recent command on this surface
/// ahead of this order whenever that command is one of the matches.
fn importance(id: CommandId) -> u16 {
    match id {
        CommandId::FilePicker => 10,
        CommandId::Find => 20,
        CommandId::Replace => 30,
        CommandId::GotoLine => 40,
        CommandId::Save => 50,
        CommandId::Reload => 55,
        CommandId::Encoding => 56,
        CommandId::Undo => 60,
        CommandId::Redo => 70,
        CommandId::Copy => 80,
        CommandId::Cut => 90,
        CommandId::Paste => 100,
        CommandId::GotoDefinition => 110,
        CommandId::GotoReferences => 112,
        CommandId::MatchBracket => 115,
        CommandId::Complete => 120,
        CommandId::Hover => 130,
        CommandId::Format => 140,
        CommandId::CodeAction => 150,
        CommandId::Comment => 160,
        CommandId::DiagnosticNext => 170,
        CommandId::DiagnosticPrev => 180,
        CommandId::RenameSymbol => 190,
        CommandId::JumpBack => 200,
        CommandId::JumpForward => 210,
        CommandId::Diff => 255,
        CommandId::Focus => 258,
        CommandId::Reveal => 259,
        CommandId::Select => 220,
        CommandId::Move => 230,
        CommandId::Delete => 240,
        CommandId::Rename => 250,
        CommandId::ToggleExplorer => 260,
        CommandId::SidebarWider => 262,
        CommandId::SidebarNarrower => 264,
        CommandId::SelectAll => 270,
        CommandId::Indent => 280,
        CommandId::Outdent => 290,
        CommandId::MoveLineUp => 300,
        CommandId::MoveLineDown => 310,
        CommandId::DeleteLine => 315,
        CommandId::DuplicateLineUp => 316,
        CommandId::DuplicateLineDown => 317,
        CommandId::TabPrev => 320,
        CommandId::TabNext => 330,
        CommandId::SplitRight => 340,
        CommandId::SplitDown => 350,
        CommandId::Close => 360,
        CommandId::CloseView => 370,
        CommandId::CloseAll => 380,
        CommandId::Settings => 390,
        CommandId::About => 395,
        CommandId::Quit => 400,
    }
}

/// Extra names that run the same command. The hint list shows the canonical name.
pub fn aliases(id: CommandId) -> &'static [&'static str] {
    match id {
        CommandId::Settings => &["config"],
        CommandId::CloseView => &["clean-view"],
        _ => &[],
    }
}

fn matches_prefix(spec: &CommandSpec, query: &str) -> bool {
    spec.name.starts_with(query)
        || aliases(spec.id)
            .iter()
            .any(|alias| alias.starts_with(query))
}

fn exact_name(spec: &CommandSpec, name: &str) -> bool {
    spec.name == name || aliases(spec.id).contains(&name)
}

/// True when `name` is a command or one of its aliases, on any surface.
pub fn is_known_name(name: &str) -> bool {
    COMMANDS.iter().any(|spec| exact_name(spec, name))
}

pub fn spec(id: CommandId) -> &'static CommandSpec {
    COMMANDS
        .iter()
        .find(|c| c.id == id)
        .expect("every CommandId has a CommandSpec")
}

/// `move`, `delete`, and `rename` run only while FILE mode is on.
pub fn manages_files(id: CommandId) -> bool {
    matches!(id, CommandId::Move | CommandId::Delete | CommandId::Rename)
}

fn allows(spec: &CommandSpec, context: CommandContext, files: bool) -> bool {
    match context {
        CommandContext::Editor => spec.editor,
        CommandContext::Explorer => spec.explorer && (files || !manages_files(spec.id)),
    }
}

/// Commands the Command Bar may show or run for `context`, most important first.
/// `files` is true while the explorer is in FILE mode.
pub fn catalog(context: CommandContext, files: bool) -> Vec<&'static CommandSpec> {
    let mut commands: Vec<_> = COMMANDS
        .iter()
        .filter(|spec| allows(spec, context, files))
        .collect();
    commands.sort_by_key(|spec| importance(spec.id));
    commands
}

pub fn find_command(
    name: &str,
    context: CommandContext,
    files: bool,
) -> Option<&'static CommandSpec> {
    catalog(context, files)
        .into_iter()
        .find(|c| exact_name(c, name))
}

/// Bare numeric input in the Command Bar is a shorthand for `goto <line>`.
pub fn is_line_number(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit())
}

/// Which names a command argument can complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameSource {
    /// Workspace-relative directories (`move`).
    Directory,
    /// File and folder names in the focused entry's folder (`rename`).
    Sibling,
    /// Workspace-relative files and directories (`save`).
    Path,
}

/// Commands whose argument is a file or folder name.
pub fn name_source(id: CommandId) -> Option<NameSource> {
    match id {
        CommandId::Move => Some(NameSource::Directory),
        CommandId::Rename => Some(NameSource::Sibling),
        CommandId::Save => Some(NameSource::Path),
        _ => None,
    }
}

/// Dim remainder of the closest name, drawn after the characters already typed.
///
/// The closest name is the one with the fewest path segments, then the shortest,
/// then A–Z. Comparison ignores case and treats `\` as `/`. An exact name produces
/// no remainder, so a finished folder is not extended into a child path.
pub fn ghost_suffix(typed: &str, candidates: &[impl AsRef<str>]) -> Option<String> {
    if typed.is_empty() {
        return None;
    }
    let mut best: Option<&str> = None;
    for candidate in candidates {
        let candidate = candidate.as_ref();
        let Some(suffix) = split_suffix(typed, candidate) else {
            continue;
        };
        if suffix.is_empty() {
            return None;
        }
        best = Some(match best {
            Some(current) if name_rank(current) <= name_rank(candidate) => current,
            _ => candidate,
        });
    }
    best.and_then(|candidate| split_suffix(typed, candidate))
        .filter(|suffix| !suffix.is_empty())
        .map(str::to_string)
}

fn split_suffix<'a>(typed: &str, candidate: &'a str) -> Option<&'a str> {
    let mut rest = candidate.chars();
    for typed_char in typed.chars() {
        let candidate_char = rest.next()?;
        if !same_name_char(typed_char, candidate_char) {
            return None;
        }
    }
    Some(rest.as_str())
}

fn same_name_char(a: char, b: char) -> bool {
    fold_name_char(a) == fold_name_char(b)
}

fn fold_name_char(c: char) -> Vec<char> {
    let c = if c == '\\' { '/' } else { c };
    c.to_lowercase().collect()
}

fn name_rank(name: &str) -> (usize, usize, String) {
    let norm: String = name
        .chars()
        .flat_map(|c| {
            let c = if c == '\\' { '/' } else { c };
            c.to_lowercase()
        })
        .collect();
    let parts = norm.split('/').filter(|part| !part.is_empty()).count();
    (parts, norm.chars().count(), norm)
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CommandBar {
    pub input: String,
    pub selected: usize,
    pub context: CommandContext,
    /// True when the bar was opened from FILE mode, so file commands are listed.
    pub files: bool,
    /// The command whose argument prompt is already showing.
    /// A second submit with an empty argument confirms that empty text.
    pub prompted: Option<CommandId>,
}

impl CommandBar {
    /// The command-name part of the input (first word).
    pub fn query(&self) -> &str {
        self.input.split_whitespace().next().unwrap_or("")
    }

    /// Text after the command name. `None` when the name has no space after it.
    /// An empty string means a space was typed and nothing follows it.
    pub fn argument_slot(&self) -> Option<&str> {
        self.input
            .trim_start()
            .split_once(char::is_whitespace)
            .map(|(_, rest)| rest.trim())
    }

    /// Everything after the command name, if non-empty.
    pub fn argument(&self) -> Option<&str> {
        self.argument_slot().filter(|arg| !arg.is_empty())
    }

    pub fn selected(&self, recent: Option<CommandId>) -> Option<&'static CommandSpec> {
        suggestions(&self.input, recent, self.context, self.files)
            .get(self.selected)
            .copied()
    }
}

/// Prefix matches first, then commands where a later name segment or a description
/// word starts with the query. Each group keeps importance order. The most recent
/// command on this surface is then moved to the top whenever it is one of those
/// matches. Commands from another surface are omitted entirely.
pub fn suggestions(
    input: &str,
    recent: Option<CommandId>,
    context: CommandContext,
    files: bool,
) -> Vec<&'static CommandSpec> {
    let query = input.split_whitespace().next().unwrap_or("").to_lowercase();
    if context == CommandContext::Editor && is_line_number(&query) {
        return vec![spec(CommandId::GotoLine)];
    }

    let available = catalog(context, files);
    let mut matches: Vec<&'static CommandSpec> = available
        .iter()
        .copied()
        .filter(|c| matches_prefix(c, &query))
        .collect();
    matches.extend(available.iter().copied().filter(|c| {
        let description = c.description.to_lowercase();
        let mut words = c.name.split('-').chain(description.split_whitespace());
        !matches_prefix(c, &query) && words.any(|w| w.starts_with(&query))
    }));
    if let Some(recent) = recent.filter(|id| allows(spec(*id), context, files))
        && let Some(i) = matches.iter().position(|c| c.id == recent)
    {
        let pinned = matches.remove(i);
        matches.insert(0, pinned);
    }
    matches
}

/// Shown when a real command was typed on the surface it does not belong to.
pub fn unavailable_reason(name: &str, context: CommandContext) -> String {
    if context == CommandContext::Explorer
        && catalog(CommandContext::Explorer, true)
            .iter()
            .any(|spec| manages_files(spec.id) && exact_name(spec, name))
    {
        return format!("{name} applies in FILE mode");
    }
    match context {
        CommandContext::Editor => format!("{name} applies to the explorer"),
        CommandContext::Explorer => format!("{name} applies to the editor"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(input: &str, recent: Option<CommandId>) -> Vec<&'static str> {
        names_in(input, recent, CommandContext::Editor)
    }

    fn names_in(
        input: &str,
        recent: Option<CommandId>,
        context: CommandContext,
    ) -> Vec<&'static str> {
        suggestions(input, recent, context, false)
            .iter()
            .map(|c| c.name)
            .collect()
    }

    #[test]
    fn empty_input_lists_the_focused_surface_with_recent_on_top() {
        let editor = names("", None);
        assert_eq!(editor.len(), catalog(CommandContext::Editor, false).len());
        assert!(!editor.contains(&"delete"));
        assert!(!editor.contains(&"select"));
        assert_eq!(names("", Some(CommandId::Quit))[0], "quit");

        assert_eq!(
            names("", None),
            [
                "file-picker",
                "find",
                "replace",
                "goto",
                "save",
                "reload",
                "encoding",
                "undo",
                "redo",
                "copy",
                "cut",
                "paste",
                "goto-definition",
                "goto-references",
                "match-bracket",
                "complete",
                "hover",
                "format",
                "code-action",
                "comment",
                "diagnostic-next",
                "diagnostic-prev",
                "rename-symbol",
                "jump-back",
                "jump-forward",
                "diff",
                "focus",
                "reveal",
                "explorer",
                "select-all",
                "indent",
                "outdent",
                "move-line-up",
                "move-line-down",
                "delete-line",
                "duplicate-line-up",
                "duplicate-line-down",
                "tab-prev",
                "tab-next",
                "split-right",
                "split-down",
                "close",
                "close-view",
                "close-all",
                "setting",
                "about",
                "quit",
            ]
        );
        assert_eq!(
            names_in("", None, CommandContext::Explorer),
            [
                "file-picker",
                "find",
                "replace",
                "select",
                "reveal",
                "explorer",
                "wider",
                "narrower",
                "setting",
                "about",
                "quit"
            ]
        );
        assert_eq!(
            suggestions("", Some(CommandId::Delete), CommandContext::Explorer, true)
                .iter()
                .map(|spec| spec.name)
                .collect::<Vec<_>>(),
            [
                "delete",
                "file-picker",
                "find",
                "replace",
                "select",
                "move",
                "rename",
                "reveal",
                "explorer",
                "wider",
                "narrower",
                "setting",
                "about",
                "quit"
            ]
        );
        assert_eq!(
            names_in("", Some(CommandId::GotoLine), CommandContext::Explorer)[0],
            "file-picker"
        );
    }

    #[test]
    fn prefix_matches_come_before_contains_matches() {
        assert_eq!(
            names("u", None),
            [
                "undo",
                "redo",
                "goto-definition",
                "goto-references",
                "hover",
                "rename-symbol",
                "move-line-up",
                "duplicate-line-up"
            ]
        );
        assert_eq!(
            names("u", Some(CommandId::Redo)),
            [
                "redo",
                "undo",
                "goto-definition",
                "goto-references",
                "hover",
                "rename-symbol",
                "move-line-up",
                "duplicate-line-up"
            ]
        );
        assert_eq!(names("cop", None), ["copy"]);
        assert_eq!(names("set", None), ["setting"]);
        assert_eq!(names("about", None), ["about"]);
        assert_eq!(names_in("about", None, CommandContext::Explorer), ["about"]);
        assert_eq!(names("config", None), ["setting"]);
        assert_eq!(
            names_in("conf", None, CommandContext::Explorer),
            ["setting"]
        );
        assert_eq!(names("all", None), ["select-all", "close-all"]);
        assert_eq!(names("close", None), ["close", "close-view", "close-all"]);
        assert_eq!(names("clean", None), ["close-view"]);
        assert_eq!(names("clean-view", None), ["close-view"]);
        assert_eq!(names("find", None), ["find"]);
        assert_eq!(names("diff", None), ["diff"]);
        assert_eq!(names("rep", None), ["replace"]);
        assert_eq!(
            names("line", None),
            [
                "goto",
                "copy",
                "cut",
                "comment",
                "indent",
                "outdent",
                "move-line-up",
                "move-line-down",
                "delete-line",
                "duplicate-line-up",
                "duplicate-line-down"
            ]
        );
        assert_eq!(names("line", Some(CommandId::Copy))[0], "copy");
        assert!(names("zzz", None).is_empty());
        assert_eq!(names("del", None), ["delete-line"]);
        assert!(names_in("del", None, CommandContext::Explorer).is_empty());
        assert_eq!(
            suggestions("del", None, CommandContext::Explorer, true)
                .iter()
                .map(|spec| spec.name)
                .collect::<Vec<_>>(),
            ["delete"]
        );
        assert!(names_in("goto", None, CommandContext::Explorer).is_empty());
        assert_eq!(names("select", None)[0], "select-all");
        assert!(!names("select", None).contains(&"select"));
        assert_eq!(
            names_in("select", None, CommandContext::Explorer),
            ["select"]
        );
    }

    #[test]
    fn numeric_input_suggests_only_goto_in_the_editor() {
        assert_eq!(names("42", Some(CommandId::Quit)), ["goto"]);
        assert!(names_in("42", Some(CommandId::Quit), CommandContext::Explorer).is_empty());
        assert!(is_line_number("007"));
        assert!(!is_line_number(""));
        assert!(!is_line_number("4a"));
        assert!(!is_line_number("-1"));
    }

    #[test]
    fn every_command_belongs_to_a_surface_and_has_a_spec() {
        for c in COMMANDS {
            assert_eq!(spec(c.id).name, c.name);
            assert!(c.editor || c.explorer, "{}", c.name);
        }
        let mut ranks: Vec<_> = COMMANDS.iter().map(|c| importance(c.id)).collect();
        let count = ranks.len();
        ranks.sort();
        ranks.dedup();
        assert_eq!(ranks.len(), count, "importance ranks must be unique");
        assert_eq!(
            catalog(CommandContext::Explorer, true).len(),
            COMMANDS.iter().filter(|c| c.explorer).count()
        );
    }

    #[test]
    fn query_and_argument_split_on_whitespace() {
        let bar = CommandBar {
            input: "  goto   42 ".into(),
            ..CommandBar::default()
        };
        assert_eq!(bar.query(), "goto");
        assert_eq!(bar.argument(), Some("42"));

        let bar = CommandBar {
            input: "goto ".into(),
            ..CommandBar::default()
        };
        assert_eq!(bar.argument(), None);
        assert_eq!(bar.argument_slot(), Some(""));

        let bar = CommandBar {
            input: "move My Folder".into(),
            context: CommandContext::Explorer,
            ..CommandBar::default()
        };
        assert_eq!(bar.argument(), Some("My Folder"));
    }

    #[test]
    fn ghost_suffix_is_the_rest_of_the_closest_name() {
        let names = ["examples", "extra", "src", "src/ui", "src/main.rs"];
        assert_eq!(ghost_suffix("ex", &names).as_deref(), Some("tra"));
        assert_eq!(ghost_suffix("exa", &names).as_deref(), Some("mples"));
        assert_eq!(ghost_suffix("s", &names).as_deref(), Some("rc"));
        assert_eq!(ghost_suffix("src/", &names).as_deref(), Some("ui"));
        assert_eq!(ghost_suffix("SRC", &names), None);
        assert_eq!(ghost_suffix("src\\u", &names).as_deref(), Some("i"));
        assert_eq!(ghost_suffix("", &names), None);
        assert_eq!(ghost_suffix("zzz", &names), None);
        assert_eq!(name_source(CommandId::Move), Some(NameSource::Directory));
        assert_eq!(name_source(CommandId::Rename), Some(NameSource::Sibling));
        assert_eq!(name_source(CommandId::Save), Some(NameSource::Path));
        assert_eq!(name_source(CommandId::GotoLine), None);
    }
}
