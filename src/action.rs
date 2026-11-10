use crate::buffer::Movement;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Sidebar,
    CommandBar,
    /// Incremental search. Printable keys search; they do not insert into the buffer.
    Find,
    /// Replacement. Printable keys edit the new text; they do not insert into the buffer.
    Replace,
    /// Modal dialog. Arrow keys move between its buttons.
    Prompt,
    /// Settings page. Arrow keys move through its sections and rows.
    Settings,
    /// About page. `Esc` closes it. Printable keys do not insert.
    About,
    /// File picker. Printable keys edit the query; arrows move through the list.
    FilePicker,
    /// Completion list. `Up`/`Down`/`Tab`/`Enter`/`Esc` move the list; other keys still type.
    CompletionPopup,
    /// Code action list. Arrows move, Enter applies, Esc closes.
    ActionMenu,
    /// Assistant prompt. Printable keys edit the prompt.
    Agent,
    /// The `preview` side picker. An arrow key opens the preview on that side.
    PreviewSide,
}

/// A side of the source window for `preview`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Up,
    Down,
    Left,
    Right,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::Up, Side::Down, Side::Left, Side::Right];

    /// The `preview` argument for this side.
    pub fn token(self) -> &'static str {
        match self {
            Side::Up => "split-up",
            Side::Down => "split-down",
            Side::Left => "split-left",
            Side::Right => "split-right",
        }
    }
}

/// Keys for the `preview` side picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewSideAction {
    /// An arrow key or its letter: open the preview on that side now.
    Choose(Side),
    /// `Enter`: open on the highlighted side.
    Confirm,
    /// `Tab` / `Shift + Tab`: move the highlight.
    Next,
    Prev,
    /// `Esc`: close the picker without a preview.
    Cancel,
}

/// Which way `Ctrl` + an arrow moves focus. Editor panes come from the split
/// tree; a direction with no surface is a no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticAction {
    Move {
        movement: Movement,
        extend: bool,
    },
    /// Page size depends on the viewport, so the app resolves it.
    PageUp {
        extend: bool,
    },
    PageDown {
        extend: bool,
    },
    InsertChar(char),
    InsertNewline,
    InsertText(String),
    DeleteBackward,
    DeleteForward,
    Indent,
    Outdent,
    /// Leave an active snippet, or close a hover that is still open.
    Dismiss,
    SelectAll,
    /// Clear the selection, including one the cursor moved away from.
    Deselect,
    Undo,
    Redo,
    Save,
    Copy,
    Cut,
    Paste,
    OpenCommandBar,
    /// Open FIND mode. Typed characters jump to that text.
    StartFind,
    /// Open the Command Bar with `replace `.
    StartReplace,
    FindNext,
    FindPrevious,
    Find(FindAction),
    Replace(ReplaceAction),
    CommandBar(CommandBarAction),
    ToggleSidebar,
    /// Grow or shrink the explorer column. The editor keeps at least half the screen.
    ResizeSidebar {
        wider: bool,
    },
    /// Grow or shrink the assistant panel by one row. The editor keeps six rows.
    ResizeAgent {
        taller: bool,
    },
    Sidebar(SidebarAction),
    /// Move focus to the neighboring surface. Does not move the text cursor.
    FocusAdjacent(PaneDirection),
    /// `Ctrl+W`: enter PANE mode, or confirm the candidate while already in it.
    FocusPrefix,
    /// A key while PANE mode is on. See [`PaneSelectAction`].
    PaneSelect(PaneSelectAction),
    /// Select the previous or next tab in the focused pane. Does not move the cursor.
    CycleTab {
        next: bool,
    },
    /// Move the cursor to the next or previous diagnostic in this file, wrapping around.
    GotoDiagnostic {
        next: bool,
    },
    /// Ask the language server what the symbol under the cursor is.
    Hover,
    /// Ask the language server where the symbol under the cursor is defined.
    GotoDefinition,
    /// Ask the language server for every place that symbol is used.
    GotoReferences,
    /// Move to the bracket paired with the one under the cursor.
    /// `extend` keeps the anchor and moves the head, the same way Shift does for navigation.
    MatchBracket {
        extend: bool,
    },
    /// Ask the language server for completions at the cursor.
    Complete,
    /// Move the current line or the selected lines up or down.
    MoveLine {
        down: bool,
    },
    /// Delete the current line, or every line the selection touches.
    DeleteLine,
    /// Copy the current line, or every line the selection touches, above or below.
    DuplicateLine {
        down: bool,
    },
    /// Add or remove the line comment marker.
    ToggleComment,
    /// Open the command bar ready to type a new symbol name.
    StartRename,
    /// Ask the language server to format this file.
    Format,
    /// Ask the language server for code actions at the cursor.
    CodeAction,
    /// Return to the location from before the last definition or references jump.
    JumpBack,
    /// The other way after [`SemanticAction::JumpBack`].
    JumpForward,
    Completion(CompletionAction),
    Quit,
    Prompt(PromptAction),
    Settings(SettingsAction),
    /// Close the About page.
    CloseAbout,
    PreviewSide(PreviewSideAction),
    /// Open the file picker over the editor.
    OpenFilePicker,
    Picker(PickerAction),
    /// Open, focus, or close the assistant panel.
    ToggleAgent,
    Agent(AgentAction),
}

/// Keys while the assistant prompt is focused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentAction {
    Insert(String),
    Backspace,
    Delete,
    /// Move one character. `extend` keeps the anchor, the same way Shift does in the editor.
    Move {
        right: bool,
        extend: bool,
    },
    /// Jump to the start or the end of the current wrap row (or the prompt while reading).
    Jump {
        end: bool,
        extend: bool,
    },
    /// Move to the wrapped row above or below. `extend` keeps the anchor.
    Line {
        down: bool,
        extend: bool,
    },
    /// `Ctrl+Left` / `Ctrl+Right`: move by word. `extend` keeps the anchor.
    Word {
        right: bool,
        extend: bool,
    },
    /// `Ctrl+Up`: read the messages above the prompt.
    Above,
    /// `Ctrl+Down`: return to the prompt.
    Below,
    /// Send the prompt.
    Submit,
    /// Leave the prompt. The panel stays open.
    Close,
    /// Clear transcript and tool memory (new chat).
    NewChat,
    /// Tab: accept the `/` or `@` popup, else open the command bar.
    Complete,
}

/// Keys that belong to the completion list. Typing is not one of these: it still edits the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionAction {
    Prev,
    Next,
    Accept,
    Close,
}

/// Keys while the file picker is open. Printable characters edit the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerAction {
    Insert(String),
    Backspace,
    DeleteForward,
    /// Delete back to the previous word.
    DeleteWordBack,
    CursorLeft,
    CursorRight,
    CursorWordLeft,
    CursorWordRight,
    CursorHome,
    CursorEnd,
    SelectPrev,
    SelectNext,
    PageUp,
    PageDown,
    First,
    Last,
    Submit,
    Close,
}

/// Input for a dialog. The buttons themselves live on the prompt, so the same
/// keys work for every confirmation, not only delete or unsaved changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptAction {
    Prev,
    Next,
    /// Activate the highlighted button.
    Confirm,
    /// Secondary shortcut. The prompt matches this to a button mnemonic.
    Accelerator(char),
    /// Activate the button that dismisses the dialog, without listing `Esc`.
    Cancel,
}

/// Keys for the settings page. Each menu opens on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAction {
    Up,
    Down,
    /// Open the highlighted menu, or start listening for a keybinding.
    Enter,
    /// Leave the current menu. From the top menu, this closes the page.
    Back,
    First,
    Last,
    /// Restore the highlighted row, or every row in the highlighted section.
    Reset,
    /// The key pressed while a keybinding row is listening.
    Assign(String),
    /// Characters typed into a text row.
    Insert(String),
    /// `Backspace` in a text row: the grapheme before the cursor.
    DeleteChar,
    /// `Delete` in a text row: the grapheme after the cursor.
    DeleteForward,
    /// `Ctrl+Backspace` / `Alt+Backspace` in a text row: back to the previous word stop.
    DeleteWordBack,
    /// Cursor moves inside a text row.
    Cursor(crate::command::LineMove),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarAction {
    Up {
        extend: bool,
    },
    Down {
        extend: bool,
    },
    PageUp {
        extend: bool,
    },
    PageDown {
        extend: bool,
    },
    First {
        extend: bool,
    },
    Last {
        extend: bool,
    },
    Expand {
        extend: bool,
    },
    Collapse {
        extend: bool,
    },
    Activate,
    /// Mark the covered rows, or toggle the focused row when nothing is covered.
    Mark,
    FilterInsert(String),
    FilterBackspace,
    /// Leave FILE mode, clear every file mark, and keep the sidebar focused.
    ExitSelect,
    /// Clears the filter, or returns focus to the editor when the filter is empty.
    Escape,
}

/// Keys while the status badge reads `REPLACE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceAction {
    Insert(String),
    Backspace,
    /// Undo the last replacement made in this mode.
    Undo,
    /// In the editor, leave REPLACE. In FILTER mode, replace the chosen files.
    Submit,
    /// Replace the current match, then select the next one.
    Next,
    /// Replace every match in the file.
    All,
    /// Leave REPLACE mode.
    Close,
}

/// Keys while the status badge reads `FIND`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindAction {
    Insert(String),
    Backspace,
    /// Previous match of the text typed so far.
    Prev,
    /// Next match of the text typed so far.
    Next,
    /// In the editor, leave FIND. In the explorer, search files for the typed text.
    Submit,
    /// Leave FIND mode. The match stays selected.
    Close,
    /// From a buffer FIND opened out of a file search, return to that file list.
    Files,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandBarAction {
    /// Typed or pasted text, inserted at the cursor.
    Insert(String),
    Backspace,
    /// `Delete`: the grapheme after the cursor.
    DeleteForward,
    /// `Ctrl+Backspace` / `Alt+Backspace`: back to the previous word stop.
    DeleteWordBack,
    /// Cursor moves. `Right` is `AcceptHint`, which moves when the cursor is not at the end.
    Cursor(crate::command::LineMove),
    SelectNext,
    SelectPrev,
    Complete,
    /// Leave the command bar and focus the assistant.
    ToChat,
    /// `Right`: at the end of the input, accept the dimmed file or folder name drawn
    /// after it; anywhere else, move the cursor one character right.
    AcceptHint,
    Submit,
    Close,
}

/// PANE mode (after `Ctrl+W`). Focus stays where it was until `Confirm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneSelectAction {
    /// Move the candidate highlight to the window on that side.
    Move(PaneDirection),
    /// `Enter` or `Ctrl+W`: focus the candidate.
    Confirm,
    /// `Esc`: leave focus where it was.
    Cancel,
}
