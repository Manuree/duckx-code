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
    Deselect,
    Indent,
    Outdent,
    GotoLine,
    Find,
    Replace,
    Copy,
    Cut,
    Paste,
    /// Copy the focused file or folder name.
    CopyName,
    /// Copy the focused file or folder as a full native path.
    CopyPath,
    /// Copy the focused file or folder path relative to the workspace, with `/`.
    CopyRelativePath,
    Save,
    ToggleExplorer,
    /// Grow the explorer column.
    SidebarWider,
    /// Shrink the explorer column.
    SidebarNarrower,
    /// Show or hide changes against HEAD in the gutter.
    Diff,
    /// Show or hide dot files and folders (Settings → Files → Show hidden files).
    ShowHidden,
    /// Turn bracket matching on or off (Settings → Editor → Match brackets).
    BracketMatch,
    /// Turn bracket pair colors on or off (Settings → Editor → Colorize bracket pairs).
    BracketColors,
    /// Show only the focused buffer, without the title or status bar.
    Focus,
    /// Open a read-only Markdown preview beside the focused file.
    Preview,
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
    /// Retype the selection as the other keyboard layout.
    ConvertKeyboard,
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
    /// Create an empty file and open it. Runs without FILE mode.
    NewFile,
    /// Create a folder from the explorer. Runs without FILE mode.
    NewFolder,
    /// Enter FILE mode in the explorer.
    Select,
    /// Move explorer entries into a directory.
    Move,
    /// Delete explorer entries.
    Delete,
    /// Rename (or, with a path, move) the focused explorer entry or the file being edited.
    Rename,
    /// Open the settings page.
    Settings,
    /// Open the about page.
    About,
    /// Open, focus, or close the assistant panel.
    Agent,
    /// Clear assistant transcript and tool memory (new chat).
    AgentNew,
    /// Open the workspace file picker.
    FilePicker,
    /// Leave this folder and open another one.
    Workspace,
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
        id: CommandId::Workspace,
        name: "workspace",
        description: "Open another folder",
        shortcut: "",
        arg: Some("<folder>"),
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
        id: CommandId::ShowHidden,
        name: "hidden",
        description: "Show or hide dot files and folders like .claude",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::BracketMatch,
        name: "bracket-match",
        description: "Highlight the bracket at the cursor and its pair, or stop",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::BracketColors,
        name: "bracket-colors",
        description: "Color brackets by nesting depth, or stop",
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
        id: CommandId::Preview,
        name: "preview",
        description: "Preview this Markdown file (read-only): pick ↑ ↓ ← → for the side",
        shortcut: "",
        arg: Some("[up|down|left|right]"),
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
        id: CommandId::ConvertKeyboard,
        name: "convert-keyboard",
        description: "Retype the selection as the other keyboard layout (ouj8 → นี่ค)",
        shortcut: "Alt+`",
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
        id: CommandId::CopyName,
        name: "copy-name",
        description: "Copy the file or folder name",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::CopyPath,
        name: "copy-path",
        description: "Copy the full path of the file or folder",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::CopyRelativePath,
        name: "copy-relative-path",
        description: "Copy the path from the workspace folder",
        shortcut: "",
        arg: None,
        editor: true,
        explorer: true,
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
        id: CommandId::Deselect,
        name: "deselect",
        description: "Clear the selection",
        shortcut: "Ctrl+D",
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
        id: CommandId::Agent,
        name: "agent",
        description: "Open the assistant panel",
        shortcut: "F6",
        arg: None,
        editor: true,
        explorer: false,
    },
    CommandSpec {
        id: CommandId::AgentNew,
        name: "clear",
        description: "Start a new assistant chat",
        shortcut: "Ctrl+L",
        arg: None,
        editor: true,
        explorer: false,
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
        id: CommandId::NewFile,
        name: "new-file",
        description: "Create a file and open it, making missing folders",
        shortcut: "",
        arg: Some("<path>"),
        editor: true,
        explorer: true,
    },
    CommandSpec {
        id: CommandId::NewFolder,
        name: "new-folder",
        description: "Create a folder in the focused folder",
        shortcut: "",
        arg: Some("<path>"),
        editor: false,
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
        description: "Rename the file or folder; a path with / moves it",
        shortcut: "",
        arg: Some("<name>"),
        editor: true,
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
        CommandId::Workspace => 15,
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
        CommandId::CopyName => 102,
        CommandId::CopyPath => 104,
        CommandId::CopyRelativePath => 106,
        CommandId::GotoDefinition => 110,
        CommandId::GotoReferences => 112,
        CommandId::MatchBracket => 115,
        CommandId::Complete => 120,
        CommandId::Hover => 130,
        CommandId::Format => 140,
        CommandId::CodeAction => 150,
        CommandId::Comment => 160,
        CommandId::ConvertKeyboard => 165,
        CommandId::DiagnosticNext => 170,
        CommandId::DiagnosticPrev => 180,
        CommandId::RenameSymbol => 190,
        CommandId::JumpBack => 200,
        CommandId::JumpForward => 210,
        CommandId::Diff => 255,
        CommandId::ShowHidden => 266,
        CommandId::BracketMatch => 267,
        CommandId::BracketColors => 268,
        CommandId::Focus => 258,
        CommandId::Preview => 257,
        CommandId::Reveal => 259,
        CommandId::Select => 220,
        CommandId::Move => 230,
        CommandId::Delete => 240,
        CommandId::Rename => 250,
        CommandId::NewFile => 251,
        CommandId::NewFolder => 252,
        CommandId::ToggleExplorer => 260,
        CommandId::SidebarWider => 262,
        CommandId::SidebarNarrower => 264,
        CommandId::SelectAll => 270,
        CommandId::Deselect => 272,
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
        CommandId::Agent => 392,
        CommandId::AgentNew => 393,
        CommandId::About => 395,
        CommandId::Quit => 400,
    }
}

/// Sort key on one surface. In the explorer the copy commands follow the file
/// commands (`select` … `new-folder`), so `move`, `delete`, and `rename` stay on the
/// first page of hints in FILE mode. Everywhere else this is [`importance`].
fn surface_rank(id: CommandId, context: CommandContext) -> u32 {
    let base = u32::from(importance(id)) * 10;
    match (context, id) {
        (CommandContext::Explorer, CommandId::CopyName) => 2525,
        (CommandContext::Explorer, CommandId::CopyPath) => 2526,
        (CommandContext::Explorer, CommandId::CopyRelativePath) => 2527,
        _ => base,
    }
}

/// Extra names that run the same command. The hint list shows the canonical name.
pub fn aliases(id: CommandId) -> &'static [&'static str] {
    match id {
        CommandId::Settings => &["config"],
        CommandId::CloseView => &["clean-view"],
        CommandId::AgentNew => &["agent-new"],
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

/// `move` and `delete` run only while FILE mode is on. `rename`, `new-file`, and
/// `new-folder` are explorer file commands too, but they never need it.
pub fn manages_files(id: CommandId) -> bool {
    matches!(id, CommandId::Move | CommandId::Delete)
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
    commands.sort_by_key(|spec| surface_rank(spec.id, context));
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

/// Commands whose argument may be left out. `preview` alone opens the side picker
/// instead of waiting for a typed side.
pub fn optional_arg(id: CommandId) -> bool {
    matches!(id, CommandId::Preview)
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
    /// Saved folder paths (`workspace`).
    Workspace,
    /// `split-up`, `split-down`, `split-left`, and `split-right` (`preview`).
    PreviewSide,
    /// Folders a new entry can go into (`new-file`, `new-folder`). A leading `/`
    /// lists workspace-relative folders; otherwise folders under the focused folder.
    NewEntry,
}

/// Commands whose argument is a file or folder name.
pub fn name_source(id: CommandId) -> Option<NameSource> {
    match id {
        CommandId::Move => Some(NameSource::Directory),
        CommandId::Rename => Some(NameSource::Sibling),
        CommandId::Save => Some(NameSource::Path),
        CommandId::Workspace => Some(NameSource::Workspace),
        CommandId::Preview => Some(NameSource::PreviewSide),
        CommandId::NewFile | CommandId::NewFolder => Some(NameSource::NewEntry),
        _ => None,
    }
}

/// True when `candidate` starts with `typed`. Case and `\` / `/` are ignored.
pub fn name_prefix(typed: &str, candidate: &str) -> bool {
    split_suffix(typed, candidate).is_some()
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
    /// Byte offset of the cursor in `input`. `None` keeps it at the end, so code that
    /// replaces the input with `set_input` (or builds a new bar) types at the end.
    pub cursor: Option<usize>,
}

/// Cursor moves and edits inside the command bar's input line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineMove {
    Left,
    Right,
    WordLeft,
    WordRight,
    Home,
    End,
}

impl CommandBar {
    /// The cursor's byte offset, always on a grapheme boundary inside `input`.
    pub fn cursor(&self) -> usize {
        match self.cursor {
            Some(at) => crate::line_edit::snap(&self.input, at),
            None => self.input.len(),
        }
    }

    pub fn at_end(&self) -> bool {
        self.cursor() == self.input.len()
    }

    /// Replace the whole input; the cursor goes to the end.
    pub fn set_input(&mut self, text: impl Into<String>) {
        self.input = text.into();
        self.cursor = None;
    }

    fn put_cursor(&mut self, at: usize) {
        self.cursor = (at < self.input.len()).then_some(at);
    }

    pub fn move_cursor(&mut self, movement: LineMove) {
        use crate::line_edit;
        let at = self.cursor();
        let to = match movement {
            LineMove::Left => line_edit::prev(&self.input, at),
            LineMove::Right => line_edit::next(&self.input, at),
            LineMove::WordLeft => line_edit::word_left(&self.input, at),
            LineMove::WordRight => line_edit::word_right(&self.input, at),
            LineMove::Home => 0,
            LineMove::End => self.input.len(),
        };
        self.put_cursor(to);
    }

    /// Insert at the cursor. Returns true when the text changed.
    pub fn insert(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        let at = self.cursor();
        self.input.insert_str(at, text);
        self.put_cursor(at + text.len());
        true
    }

    /// Delete from the cursor back to `to` (`to` before the cursor) or forward to `to`.
    fn delete_to(&mut self, to: usize) -> bool {
        let at = self.cursor();
        let (start, end) = if to < at { (to, at) } else { (at, to) };
        if start == end {
            return false;
        }
        self.input.replace_range(start..end, "");
        self.put_cursor(start);
        true
    }

    pub fn backspace(&mut self) -> bool {
        self.delete_to(crate::line_edit::prev(&self.input, self.cursor()))
    }

    pub fn delete_forward(&mut self) -> bool {
        self.delete_to(crate::line_edit::next(&self.input, self.cursor()))
    }

    pub fn delete_word_back(&mut self) -> bool {
        self.delete_to(crate::line_edit::word_left(&self.input, self.cursor()))
    }

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
                "workspace",
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
                "copy-name",
                "copy-path",
                "copy-relative-path",
                "goto-definition",
                "goto-references",
                "match-bracket",
                "complete",
                "hover",
                "format",
                "code-action",
                "comment",
                "convert-keyboard",
                "diagnostic-next",
                "diagnostic-prev",
                "rename-symbol",
                "jump-back",
                "jump-forward",
                "rename",
                "new-file",
                "diff",
                "preview",
                "focus",
                "reveal",
                "explorer",
                "hidden",
                "bracket-match",
                "bracket-colors",
                "select-all",
                "deselect",
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
                "agent",
                "clear",
                "about",
                "quit",
            ]
        );
        assert_eq!(
            names_in("", None, CommandContext::Explorer),
            [
                "file-picker",
                "workspace",
                "find",
                "replace",
                "select",
                "rename",
                "new-file",
                "new-folder",
                "copy-name",
                "copy-path",
                "copy-relative-path",
                "reveal",
                "explorer",
                "wider",
                "narrower",
                "hidden",
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
                "workspace",
                "find",
                "replace",
                "select",
                "move",
                "rename",
                "new-file",
                "new-folder",
                "copy-name",
                "copy-path",
                "copy-relative-path",
                "reveal",
                "explorer",
                "wider",
                "narrower",
                "hidden",
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
        assert_eq!(
            names("cop", None),
            ["copy", "copy-name", "copy-path", "copy-relative-path"]
        );
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
    fn new_file_and_new_folder_do_not_need_file_mode() {
        for files in [false, true] {
            let explorer = catalog(CommandContext::Explorer, files);
            assert!(explorer.iter().any(|spec| spec.id == CommandId::NewFile));
            assert!(explorer.iter().any(|spec| spec.id == CommandId::NewFolder));
        }
        let editor = catalog(CommandContext::Editor, false);
        assert!(editor.iter().any(|spec| spec.id == CommandId::NewFile));
        assert!(!editor.iter().any(|spec| spec.id == CommandId::NewFolder));
        assert!(!manages_files(CommandId::NewFile));
        assert!(!manages_files(CommandId::NewFolder));
        assert_eq!(
            find_command("new-folder", CommandContext::Explorer, false).map(|spec| spec.id),
            Some(CommandId::NewFolder)
        );
        assert_eq!(
            find_command("new-file", CommandContext::Editor, false).map(|spec| spec.id),
            Some(CommandId::NewFile)
        );
        assert!(find_command("new-folder", CommandContext::Editor, false).is_none());
        assert_eq!(
            unavailable_reason("new-folder", CommandContext::Editor),
            "new-folder applies to the explorer"
        );
        assert_eq!(
            names_in("new", None, CommandContext::Explorer)[..2],
            ["new-file", "new-folder"]
        );
        assert_eq!(names("new", None)[0], "new-file");
        assert_eq!(spec(CommandId::NewFile).arg, Some("<path>"));
        assert_eq!(spec(CommandId::NewFolder).arg, Some("<path>"));
        assert_eq!(name_source(CommandId::NewFile), Some(NameSource::NewEntry));
        assert_eq!(
            name_source(CommandId::NewFolder),
            Some(NameSource::NewEntry)
        );
    }

    #[test]
    fn copy_path_commands_are_listed_on_both_surfaces_without_file_mode() {
        for id in [
            CommandId::CopyName,
            CommandId::CopyPath,
            CommandId::CopyRelativePath,
        ] {
            assert!(
                catalog(CommandContext::Editor, false)
                    .iter()
                    .any(|c| c.id == id)
            );
            assert!(
                catalog(CommandContext::Explorer, false)
                    .iter()
                    .any(|c| c.id == id)
            );
            assert!(!manages_files(id));
            assert!(spec(id).arg.is_none());
        }
        assert_eq!(
            names_in("copy-", None, CommandContext::Explorer),
            ["copy-name", "copy-path", "copy-relative-path"]
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
        assert_eq!(
            name_source(CommandId::Workspace),
            Some(NameSource::Workspace)
        );
        assert_eq!(name_source(CommandId::GotoLine), None);
        assert!(name_prefix("E:/Wo", "e:\\work"));
        assert!(!name_prefix("zzz", "e:\\work"));
    }
}
