//! Optional modal editing: Normal, Insert, Visual, and Visual Line.
//!
//! Off by default. Settings → Editor → Modeless is on, and then the editor routes
//! keys through [`crate::input::route_flags`] exactly as before; nothing here runs.
//! With Modeless off, [`route`] takes the editor's keys instead:
//!
//! - `Ctrl`, `Alt`, and F-key chords, arrows, Home/End, and PgUp/PgDn behave as
//!   they do without modes (the same [`crate::input::route_editor_key`] call), in
//!   every mode. In Visual a move extends the selection.
//! - Insert mode is the modeless editor, except that `Esc` returns to Normal.
//! - In Normal and Visual, plain keys come from [`KEYS`] (Key style Vim, the default)
//!   or [`MNEMONIC_KEYS`] (Key style Mnemonic: `cl` copies the line). Every entry
//!   either runs a Command Bar command or behaves like a modeless key (`h` is
//!   `Left`), so nothing here is reachable only by a letter.
//!
//! The selling point is clarity, so the same table feeds the hints in the status
//! bar ([`hints`]) and the `?` key list ([`key_list`]). Both show each key's
//! name with the letters to press marked in it ([`marks`]), or the key before the
//! name when the name does not hold them.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::action::SemanticAction;
use crate::command::CommandId;
use crate::keys::KeyMap;

/// The editor's mode while Modeless is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditMode {
    /// Keys are commands and motions. Nothing is typed.
    #[default]
    Normal,
    /// Keys type, the same as the modeless editor. `Esc` returns to Normal.
    Insert,
    /// Motions extend a selection; `line` selects whole lines (`V`).
    Visual { line: bool },
}

impl EditMode {
    /// The status bar badge, padded to one width so the bar does not jump.
    pub fn badge(self) -> &'static str {
        match self {
            Self::Normal => "  NORMAL  ",
            Self::Insert => "  INSERT  ",
            Self::Visual { line: false } => "  VISUAL  ",
            Self::Visual { line: true } => "  V-LINE  ",
        }
    }

    pub fn is_visual(self) -> bool {
        matches!(self, Self::Visual { .. })
    }
}

/// Which keys Normal and Visual use (Settings → Editor → Key style). Only while
/// Modeless is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyStyle {
    /// The vim keys ([`KEYS`]): `yy` copies a line, `dd` cuts it.
    #[default]
    Vim,
    /// A verb letter, then what it acts on ([`MNEMONIC_KEYS`]): `cl` copies the line.
    Mnemonic,
}

impl KeyStyle {
    pub fn index(self) -> usize {
        match self {
            Self::Vim => 0,
            Self::Mnemonic => 1,
        }
    }

    pub fn from_index(index: usize) -> Self {
        if index == 1 {
            Self::Mnemonic
        } else {
            Self::Vim
        }
    }

    /// The saved token: `vim` or `mnemonic`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vim => "vim",
            Self::Mnemonic => "mnemonic",
        }
    }

    /// `vim` or `mnemonic` (any case); `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "vim" => Some(Self::Vim),
            "mnemonic" => Some(Self::Mnemonic),
            _ => None,
        }
    }

    /// The name Settings and messages show.
    pub fn label(self) -> &'static str {
        match self {
            Self::Vim => "Vim",
            Self::Mnemonic => "Mnemonic",
        }
    }

    /// Every key of this style.
    pub fn keys(self) -> &'static [ModalKey] {
        match self {
            Self::Vim => KEYS,
            Self::Mnemonic => MNEMONIC_KEYS,
        }
    }
}

/// Which modes a [`ModalKey`] works in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modes {
    Normal,
    Visual,
    Both,
}

impl Modes {
    fn has(self, mode: EditMode) -> bool {
        matches!(
            (self, mode),
            (Self::Both, EditMode::Normal | EditMode::Visual { .. })
                | (Self::Normal, EditMode::Normal)
                | (Self::Visual, EditMode::Visual { .. })
        )
    }
}

/// What a [`ModalKey`] does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Do {
    /// Exactly what this key does without modes. In Visual a move extends.
    Key(KeyCode, KeyModifiers),
    /// Run this Command Bar command.
    Command(CommandId),
}

/// One key (or two-key sequence) in Normal or Visual mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModalKey {
    /// The characters to press, or a key name (`Enter`, `Backspace`, `Esc`).
    pub keys: &'static str,
    pub modes: Modes,
    pub run: Do,
    /// What it does, for the `?` key list.
    pub label: &'static str,
    /// The name the hint row and the `?` list show, with the letters of `keys`
    /// highlighted in it when they are all there (see [`marks`]).
    pub name: &'static str,
}

const NONE: KeyModifiers = KeyModifiers::NONE;
const CTRL: KeyModifiers = KeyModifiers::CONTROL;
const SHIFT: KeyModifiers = KeyModifiers::SHIFT;

const fn key(
    keys: &'static str,
    modes: Modes,
    run: Do,
    label: &'static str,
    name: &'static str,
) -> ModalKey {
    ModalKey {
        keys,
        modes,
        run,
        label,
        name,
    }
}

const fn cmd(id: CommandId) -> Do {
    Do::Command(id)
}

const fn like(code: KeyCode, modifiers: KeyModifiers) -> Do {
    Do::Key(code, modifiers)
}

/// Every key of Normal and Visual mode in the Vim key style (the default), in the
/// order the `?` list shows them.
pub const KEYS: &[ModalKey] = &[
    // Changing mode.
    key(
        "i",
        Modes::Normal,
        cmd(CommandId::ModeInsert),
        "Type at the cursor (Insert)",
        "Insert",
    ),
    key(
        "a",
        Modes::Normal,
        cmd(CommandId::InsertAfter),
        "Type after the cursor",
        "Append",
    ),
    key(
        "I",
        Modes::Normal,
        cmd(CommandId::InsertLineStart),
        "Type at the start of the line",
        "Insert at line start",
    ),
    key(
        "A",
        Modes::Normal,
        cmd(CommandId::InsertLineEnd),
        "Type at the end of the line",
        "Append at line end",
    ),
    key(
        "o",
        Modes::Normal,
        cmd(CommandId::OpenLineBelow),
        "Open a line below and type",
        "Open line below",
    ),
    key(
        "O",
        Modes::Normal,
        cmd(CommandId::OpenLineAbove),
        "Open a line above and type",
        "Open line above",
    ),
    key(
        "v",
        Modes::Both,
        cmd(CommandId::ModeVisual),
        "Select characters (Visual)",
        "Visual",
    ),
    key(
        "V",
        Modes::Both,
        cmd(CommandId::ModeVisualLine),
        "Select whole lines (Visual Line)",
        "Visual Line",
    ),
    key(
        "Esc",
        Modes::Both,
        cmd(CommandId::ModeNormal),
        "Back to Normal; in Normal, close popups",
        "Normal",
    ),
    // Motions: the same moves as the arrow and Ctrl keys.
    key(
        "h",
        Modes::Both,
        like(KeyCode::Left, NONE),
        "Left (←)",
        "Left",
    ),
    key(
        "j",
        Modes::Both,
        like(KeyCode::Down, NONE),
        "Down (↓)",
        "Down",
    ),
    key("k", Modes::Both, like(KeyCode::Up, NONE), "Up (↑)", "Up"),
    key(
        "l",
        Modes::Both,
        like(KeyCode::Right, NONE),
        "Right (→)",
        "Right",
    ),
    key(
        "w",
        Modes::Both,
        like(KeyCode::Right, CTRL),
        "Next word (Ctrl+→)",
        "Word",
    ),
    key(
        "b",
        Modes::Both,
        like(KeyCode::Left, CTRL),
        "Previous word (Ctrl+←)",
        "Back a word",
    ),
    key(
        "0",
        Modes::Both,
        like(KeyCode::Home, NONE),
        "Start of the line (Home)",
        "Line start",
    ),
    key(
        "$",
        Modes::Both,
        like(KeyCode::End, NONE),
        "End of the line (End)",
        "Line end",
    ),
    key(
        "gg",
        Modes::Both,
        like(KeyCode::Home, CTRL),
        "Top of the file; 5gg: line 5 (Ctrl+Home)",
        "Goto beginning",
    ),
    key(
        "G",
        Modes::Both,
        like(KeyCode::End, CTRL),
        "End of the file; 5G: line 5 (Ctrl+End)",
        "Goto end",
    ),
    key(
        "{",
        Modes::Both,
        like(KeyCode::Up, CTRL),
        "Previous blank line (Ctrl+↑)",
        "Previous blank line",
    ),
    key(
        "}",
        Modes::Both,
        like(KeyCode::Down, CTRL),
        "Next blank line (Ctrl+↓)",
        "Next blank line",
    ),
    key(
        "Enter",
        Modes::Both,
        like(KeyCode::Down, NONE),
        "Down (↓)",
        "Down",
    ),
    key(
        "Backspace",
        Modes::Both,
        like(KeyCode::Left, NONE),
        "Left (←)",
        "Left",
    ),
    // Editing in Normal.
    key(
        "x",
        Modes::Normal,
        like(KeyCode::Delete, NONE),
        "Delete the character (Delete)",
        "Delete character",
    ),
    key(
        "dd",
        Modes::Normal,
        cmd(CommandId::CutLine),
        "Cut the line (3dd: three lines)",
        "Cut line",
    ),
    key(
        "yy",
        Modes::Normal,
        cmd(CommandId::CopyLine),
        "Copy the line (3yy: three lines)",
        "Copy line",
    ),
    key(
        "p",
        Modes::Normal,
        cmd(CommandId::PasteAfter),
        "Paste: a copied line goes on a new line below",
        "Paste",
    ),
    key(
        "P",
        Modes::Normal,
        cmd(CommandId::PasteBefore),
        "Paste: a copied line goes on a new line above",
        "Paste before",
    ),
    key(
        "p",
        Modes::Visual,
        cmd(CommandId::Paste),
        "Paste over the selection",
        "Paste",
    ),
    key("u", Modes::Normal, cmd(CommandId::Undo), "Undo", "Undo"),
    key("U", Modes::Normal, cmd(CommandId::Redo), "Redo", "Redo"),
    key(
        ">>",
        Modes::Normal,
        cmd(CommandId::Indent),
        "Indent the line",
        "Indent",
    ),
    key(
        "<<",
        Modes::Normal,
        cmd(CommandId::Outdent),
        "Outdent the line",
        "Outdent",
    ),
    key(
        "gc",
        Modes::Both,
        cmd(CommandId::Comment),
        "Comment or uncomment",
        "toggle Comment",
    ),
    key(
        "Ctrl+D",
        Modes::Normal,
        cmd(CommandId::HalfPageDown),
        "Half a screen down",
        "Half page down",
    ),
    key(
        "Ctrl+U",
        Modes::Normal,
        cmd(CommandId::HalfPageUp),
        "Half a screen up",
        "Half page up",
    ),
    key(
        "]e",
        Modes::Normal,
        cmd(CommandId::MoveLineDown),
        "Move the line down (Alt+↓)",
        "Move line down",
    ),
    key(
        "[e",
        Modes::Normal,
        cmd(CommandId::MoveLineUp),
        "Move the line up (Alt+↑)",
        "Move line up",
    ),
    // Editing the Visual selection.
    key(
        "Tab",
        Modes::Visual,
        cmd(CommandId::Indent),
        "Indent the lines",
        "Indent",
    ),
    key(
        "Shift+Tab",
        Modes::Visual,
        cmd(CommandId::Outdent),
        "Outdent the lines",
        "Outdent",
    ),
    key(
        "d",
        Modes::Visual,
        cmd(CommandId::Cut),
        "Cut the selection",
        "Delete (cut)",
    ),
    // Editing in Normal.
    key(
        "x",
        Modes::Visual,
        cmd(CommandId::Cut),
        "Cut the selection",
        "Cut",
    ),
    key(
        "y",
        Modes::Visual,
        cmd(CommandId::Copy),
        "Copy the selection",
        "Yank (copy)",
    ),
    key(
        "c",
        Modes::Visual,
        cmd(CommandId::ChangeSelection),
        "Delete the selection and type",
        "Change",
    ),
    key(
        ">",
        Modes::Visual,
        cmd(CommandId::Indent),
        "Indent the lines",
        "Indent",
    ),
    key(
        "<",
        Modes::Visual,
        cmd(CommandId::Outdent),
        "Outdent the lines",
        "Outdent",
    ),
    key(
        "o",
        Modes::Visual,
        cmd(CommandId::SwapSelectionEnds),
        "Cursor to the other end",
        "Other end",
    ),
    key(
        "i\"",
        Modes::Visual,
        cmd(CommandId::SelectInsideQuotes),
        "Select inside quotes",
        "Inside quotes",
    ),
    key(
        "i'",
        Modes::Visual,
        cmd(CommandId::SelectInsideQuotes),
        "Select inside quotes",
        "Inside quotes",
    ),
    key(
        "i(",
        Modes::Visual,
        cmd(CommandId::SelectInsideParens),
        "Select inside ( )",
        "Inside ( )",
    ),
    key(
        "i)",
        Modes::Visual,
        cmd(CommandId::SelectInsideParens),
        "Select inside ( )",
        "Inside ( )",
    ),
    key(
        "i{",
        Modes::Visual,
        cmd(CommandId::SelectInsideBraces),
        "Select inside { }",
        "Inside { }",
    ),
    key(
        "i}",
        Modes::Visual,
        cmd(CommandId::SelectInsideBraces),
        "Select inside { }",
        "Inside { }",
    ),
    key(
        "i[",
        Modes::Visual,
        cmd(CommandId::SelectInsideBrackets),
        "Select inside [ ]",
        "Inside [ ]",
    ),
    key(
        "i]",
        Modes::Visual,
        cmd(CommandId::SelectInsideBrackets),
        "Select inside [ ]",
        "Inside [ ]",
    ),
    key(
        "%",
        Modes::Normal,
        cmd(CommandId::MatchBracket),
        "Matching bracket",
        "Matching bracket",
    ),
    key(
        "%",
        Modes::Visual,
        cmd(CommandId::SelectBracket),
        "Select to the matching bracket",
        "Select to bracket",
    ),
    // Search, language server, and the rest.
    key(
        "/",
        Modes::Normal,
        cmd(CommandId::Find),
        "Find in the file",
        "Find",
    ),
    key(
        "n",
        Modes::Normal,
        cmd(CommandId::FindNext),
        "Next match",
        "Next match",
    ),
    key(
        "N",
        Modes::Normal,
        cmd(CommandId::FindPrevious),
        "Previous match",
        "Previous match",
    ),
    key(
        "gd",
        Modes::Normal,
        cmd(CommandId::GotoDefinition),
        "Go to definition",
        "Goto Definition",
    ),
    key(
        "K",
        Modes::Normal,
        cmd(CommandId::Hover),
        "Hover documentation",
        "Hover",
    ),
    key(
        "gr",
        Modes::Normal,
        cmd(CommandId::GotoReferences),
        "Find references",
        "Goto References",
    ),
    key(
        "ga",
        Modes::Normal,
        cmd(CommandId::CodeAction),
        "Code actions",
        "Goto Actions",
    ),
    key(
        "gR",
        Modes::Normal,
        like(KeyCode::F(2), SHIFT),
        "Rename the symbol (Shift+F2)",
        "Goto Rename",
    ),
    key(
        "gt",
        Modes::Normal,
        cmd(CommandId::TabNext),
        "Next tab",
        "Goto next tab",
    ),
    key(
        "gT",
        Modes::Normal,
        cmd(CommandId::TabPrev),
        "Previous tab",
        "Goto previous Tab",
    ),
    key(
        "=",
        Modes::Normal,
        cmd(CommandId::Format),
        "Format the file",
        "Format",
    ),
    key(
        "Tab",
        Modes::Normal,
        cmd(CommandId::JumpForward),
        "Jump forward (Ctrl+O goes back)",
        "Jump forward",
    ),
    key(
        "]d",
        Modes::Normal,
        cmd(CommandId::DiagnosticNext),
        "Next diagnostic",
        "Next diagnostic",
    ),
    key(
        "[d",
        Modes::Normal,
        cmd(CommandId::DiagnosticPrev),
        "Previous diagnostic",
        "Previous diagnostic",
    ),
    key(
        ":",
        Modes::Both,
        like(KeyCode::F(1), NONE),
        "Command Bar (F1)",
        "Commands",
    ),
    key(
        "?",
        Modes::Both,
        cmd(CommandId::Keys),
        "This list of keys",
        "Keys",
    ),
    // Ctrl keys that differ from vim. They keep their duckx-code meaning in every mode.
    key(
        "Shift+Space",
        Modes::Both,
        like(KeyCode::F(1), NONE),
        "Command Bar (same as :)",
        "Commands",
    ),
    key(
        "Ctrl+V",
        Modes::Both,
        like(KeyCode::Char('v'), CTRL),
        "Pastes (in vim it starts a block selection)",
        "Paste",
    ),
    key(
        "Ctrl+C",
        Modes::Both,
        like(KeyCode::Char('c'), CTRL),
        "Copies (in vim it cancels; it never quits here)",
        "Copy",
    ),
    key(
        "Ctrl+W",
        Modes::Both,
        like(KeyCode::Char('w'), CTRL),
        "PANE: pick a window (in vim it is the window prefix)",
        "Pane",
    ),
    key(
        "Ctrl+R",
        Modes::Both,
        like(KeyCode::Char('y'), CTRL),
        "Not used; redo is U (or Ctrl+Y, shown here)",
        "Not used",
    ),
    key(
        "Ctrl+H",
        Modes::Both,
        like(KeyCode::Left, NONE),
        "Same as Backspace: left (many terminals send Backspace as ^H)",
        "Left",
    ),
];

/// Every key of Normal and Visual mode in the Mnemonic key style: a verb letter, then
/// what it acts on (`cl` copies the line). Same order rules as [`KEYS`].
pub const MNEMONIC_KEYS: &[ModalKey] = &[
    // Changing mode: the same keys as Vim.
    key(
        "i",
        Modes::Normal,
        cmd(CommandId::ModeInsert),
        "Type at the cursor (Insert)",
        "Insert",
    ),
    key(
        "a",
        Modes::Normal,
        cmd(CommandId::InsertAfter),
        "Type after the cursor",
        "insert After",
    ),
    key(
        "I",
        Modes::Normal,
        cmd(CommandId::InsertLineStart),
        "Type at the start of the line",
        "Insert start",
    ),
    key(
        "A",
        Modes::Normal,
        cmd(CommandId::InsertLineEnd),
        "Type at the end of the line",
        "insert At end",
    ),
    key(
        "o",
        Modes::Normal,
        cmd(CommandId::OpenLineBelow),
        "Open a line below and type",
        "Open below",
    ),
    key(
        "O",
        Modes::Normal,
        cmd(CommandId::OpenLineAbove),
        "Open a line above and type",
        "Open above",
    ),
    key(
        "v",
        Modes::Both,
        cmd(CommandId::ModeVisual),
        "Select characters (Visual; same as sc)",
        "Visual",
    ),
    key(
        "V",
        Modes::Both,
        cmd(CommandId::ModeVisualLine),
        "Select whole lines (Visual Line; same as sl)",
        "Visual Line",
    ),
    key(
        "Esc",
        Modes::Both,
        cmd(CommandId::ModeNormal),
        "Back to Normal; in Normal, close popups",
        "Normal",
    ),
    // Motions: the same as Vim.
    key(
        "h",
        Modes::Both,
        like(KeyCode::Left, NONE),
        "Left (←)",
        "Left",
    ),
    key(
        "j",
        Modes::Both,
        like(KeyCode::Down, NONE),
        "Down (↓)",
        "Down",
    ),
    key("k", Modes::Both, like(KeyCode::Up, NONE), "Up (↑)", "Up"),
    key(
        "l",
        Modes::Both,
        like(KeyCode::Right, NONE),
        "Right (→)",
        "Right",
    ),
    key(
        "w",
        Modes::Both,
        like(KeyCode::Right, CTRL),
        "Next word (Ctrl+→)",
        "Word",
    ),
    key(
        "b",
        Modes::Both,
        like(KeyCode::Left, CTRL),
        "Previous word (Ctrl+←)",
        "Back a word",
    ),
    key(
        "0",
        Modes::Both,
        like(KeyCode::Home, NONE),
        "Start of the line (Home)",
        "Line start",
    ),
    key(
        "$",
        Modes::Both,
        like(KeyCode::End, NONE),
        "End of the line (End)",
        "Line end",
    ),
    key(
        "gg",
        Modes::Both,
        like(KeyCode::Home, CTRL),
        "Top of the file; 5gg: line 5 (Ctrl+Home)",
        "Goto beginning",
    ),
    key(
        "G",
        Modes::Both,
        like(KeyCode::End, CTRL),
        "End of the file; 5G: line 5 (Ctrl+End)",
        "Goto end",
    ),
    key(
        "{",
        Modes::Both,
        like(KeyCode::Up, CTRL),
        "Previous blank line (Ctrl+↑)",
        "Previous blank line",
    ),
    key(
        "}",
        Modes::Both,
        like(KeyCode::Down, CTRL),
        "Next blank line (Ctrl+↓)",
        "Next blank line",
    ),
    key(
        "Enter",
        Modes::Both,
        like(KeyCode::Down, NONE),
        "Down (↓)",
        "Down",
    ),
    key(
        "Backspace",
        Modes::Both,
        like(KeyCode::Left, NONE),
        "Left (←)",
        "Left",
    ),
    // c: copy. x: cut (as Ctrl+X). p: paste.
    key(
        "cl",
        Modes::Normal,
        cmd(CommandId::CopyLine),
        "Copy the line (3cl: three lines)",
        "Copy Line",
    ),
    key(
        "cc",
        Modes::Normal,
        cmd(CommandId::CopyLine),
        "Copy the line (3cc: three lines)",
        "Copy Line",
    ),
    key(
        "cw",
        Modes::Normal,
        cmd(CommandId::CopyWord),
        "Copy the word at the cursor",
        "Copy Word",
    ),
    key(
        "ca",
        Modes::Normal,
        cmd(CommandId::CopyAll),
        "Copy the whole file",
        "Copy All",
    ),
    key(
        "xl",
        Modes::Normal,
        cmd(CommandId::CutLine),
        "Cut the line (3xl: three lines)",
        "Cut Line",
    ),
    key(
        "xx",
        Modes::Normal,
        cmd(CommandId::CutLine),
        "Cut the line (3xx: three lines)",
        "Cut Line",
    ),
    key(
        "xw",
        Modes::Normal,
        cmd(CommandId::CutWord),
        "Cut the word at the cursor",
        "Cut Word",
    ),
    key(
        "p",
        Modes::Normal,
        cmd(CommandId::PasteAfter),
        "Paste: a copied line goes on a new line below",
        "Paste",
    ),
    key(
        "P",
        Modes::Normal,
        cmd(CommandId::PasteBefore),
        "Paste: a copied line goes on a new line above",
        "Paste Before",
    ),
    // s: select. In Visual these change the selection.
    key(
        "sc",
        Modes::Both,
        cmd(CommandId::ModeVisual),
        "Select characters (Visual)",
        "Select Characters",
    ),
    key(
        "sl",
        Modes::Both,
        cmd(CommandId::ModeVisualLine),
        "Select whole lines (Visual Line)",
        "Select Line",
    ),
    key(
        "ss",
        Modes::Both,
        cmd(CommandId::ModeVisualLine),
        "Select whole lines (Visual Line)",
        "Select Line",
    ),
    key(
        "sw",
        Modes::Both,
        cmd(CommandId::SelectWord),
        "Select the word at the cursor",
        "Select Word",
    ),
    key(
        "sa",
        Modes::Both,
        cmd(CommandId::SelectAll),
        "Select the whole file",
        "Select All",
    ),
    key(
        "sb",
        Modes::Both,
        cmd(CommandId::SelectBracket),
        "Select to the matching bracket",
        "Select Bracket",
    ),
    key(
        "sq",
        Modes::Both,
        cmd(CommandId::SelectInsideQuotes),
        "Select inside quotes; again takes the quotes",
        "Select Quotes",
    ),
    key(
        "sp",
        Modes::Both,
        cmd(CommandId::SelectInsideParens),
        "Select inside ( ); again takes the parentheses",
        "Select Parens",
    ),
    key(
        "s{",
        Modes::Both,
        cmd(CommandId::SelectInsideBraces),
        "Select inside { }; again takes the braces",
        "Select inside { }",
    ),
    key(
        "s[",
        Modes::Both,
        cmd(CommandId::SelectInsideBrackets),
        "Select inside [ ]; again takes the brackets",
        "Select inside [ ]",
    ),
    // d: delete without the clipboard. u / r: undo / redo.
    key(
        "dc",
        Modes::Normal,
        like(KeyCode::Delete, NONE),
        "Delete the character (Delete)",
        "Delete Character",
    ),
    key(
        "dl",
        Modes::Normal,
        cmd(CommandId::DeleteLine),
        "Delete the line, clipboard untouched (3dl: three lines)",
        "Delete Line",
    ),
    key(
        "dd",
        Modes::Normal,
        cmd(CommandId::DeleteLine),
        "Delete the line, clipboard untouched (3dd: three lines)",
        "Delete Line",
    ),
    key("u", Modes::Normal, cmd(CommandId::Undo), "Undo", "Undo"),
    key("r", Modes::Normal, cmd(CommandId::Redo), "Redo", "Redo"),
    key(
        ">>",
        Modes::Normal,
        cmd(CommandId::Indent),
        "Indent the line",
        "Indent",
    ),
    key(
        "<<",
        Modes::Normal,
        cmd(CommandId::Outdent),
        "Outdent the line",
        "Outdent",
    ),
    key(
        "gc",
        Modes::Both,
        cmd(CommandId::Comment),
        "Comment or uncomment",
        "toggle Comment",
    ),
    key(
        "md",
        Modes::Normal,
        cmd(CommandId::MoveLineDown),
        "Move the line down (Alt+↓)",
        "Move line Down",
    ),
    key(
        "mu",
        Modes::Normal,
        cmd(CommandId::MoveLineUp),
        "Move the line up (Alt+↑)",
        "Move line Up",
    ),
    key(
        "Ctrl+D",
        Modes::Normal,
        cmd(CommandId::HalfPageDown),
        "Half a screen down",
        "Half page down",
    ),
    key(
        "Ctrl+U",
        Modes::Normal,
        cmd(CommandId::HalfPageUp),
        "Half a screen up",
        "Half page up",
    ),
    // Editing the Visual selection.
    key(
        "c",
        Modes::Visual,
        cmd(CommandId::Copy),
        "Copy the selection",
        "Copy",
    ),
    key(
        "x",
        Modes::Visual,
        cmd(CommandId::Cut),
        "Cut the selection",
        "Cut",
    ),
    key(
        "e",
        Modes::Visual,
        cmd(CommandId::ChangeSelection),
        "Edit: delete the selection and type",
        "Edit",
    ),
    key(
        "p",
        Modes::Visual,
        cmd(CommandId::Paste),
        "Paste over the selection",
        "Paste",
    ),
    key(
        ">",
        Modes::Visual,
        cmd(CommandId::Indent),
        "Indent the lines",
        "Indent",
    ),
    key(
        "<",
        Modes::Visual,
        cmd(CommandId::Outdent),
        "Outdent the lines",
        "Outdent",
    ),
    key(
        "Tab",
        Modes::Visual,
        cmd(CommandId::Indent),
        "Indent the lines",
        "Indent",
    ),
    key(
        "Shift+Tab",
        Modes::Visual,
        cmd(CommandId::Outdent),
        "Outdent the lines",
        "Outdent",
    ),
    key(
        "o",
        Modes::Visual,
        cmd(CommandId::SwapSelectionEnds),
        "Cursor to the other end",
        "Other end",
    ),
    key(
        "%",
        Modes::Normal,
        cmd(CommandId::MatchBracket),
        "Matching bracket",
        "Matching bracket",
    ),
    // f: find. g: go to. The rest.
    key(
        "ff",
        Modes::Normal,
        cmd(CommandId::Find),
        "Find in the file (same as /)",
        "Find in File",
    ),
    key(
        "fn",
        Modes::Normal,
        cmd(CommandId::FindNext),
        "Next match (same as n)",
        "Find Next",
    ),
    key(
        "fP",
        Modes::Normal,
        cmd(CommandId::FindPrevious),
        "Previous match (same as N)",
        "Find Previous",
    ),
    key(
        "fo",
        Modes::Normal,
        cmd(CommandId::FilePicker),
        "Open a file by name (Ctrl+P)",
        "File Open",
    ),
    key(
        "/",
        Modes::Normal,
        cmd(CommandId::Find),
        "Find in the file",
        "Find",
    ),
    key(
        "n",
        Modes::Normal,
        cmd(CommandId::FindNext),
        "Next match",
        "Next match",
    ),
    key(
        "N",
        Modes::Normal,
        cmd(CommandId::FindPrevious),
        "Previous match",
        "Previous match",
    ),
    key(
        "R",
        Modes::Normal,
        cmd(CommandId::Replace),
        "Find and replace",
        "Replace",
    ),
    key(
        "gd",
        Modes::Normal,
        cmd(CommandId::GotoDefinition),
        "Go to definition",
        "Goto Definition",
    ),
    key(
        "gr",
        Modes::Normal,
        cmd(CommandId::GotoReferences),
        "Find references",
        "Goto References",
    ),
    key(
        "ga",
        Modes::Normal,
        cmd(CommandId::CodeAction),
        "Code actions",
        "Goto Action",
    ),
    key(
        "gR",
        Modes::Normal,
        like(KeyCode::F(2), SHIFT),
        "Rename the symbol (Shift+F2)",
        "Goto Rename",
    ),
    key(
        "gt",
        Modes::Normal,
        cmd(CommandId::TabNext),
        "Next tab",
        "Goto tab",
    ),
    key(
        "gT",
        Modes::Normal,
        cmd(CommandId::TabPrev),
        "Previous tab",
        "Goto previous Tab",
    ),
    key(
        "gl",
        Modes::Normal,
        cmd(CommandId::GotoLine),
        "Go to a line: 5gl goes to line 5, gl asks",
        "Goto Line",
    ),
    key(
        "gh",
        Modes::Normal,
        cmd(CommandId::Hover),
        "Hover documentation",
        "Goto Hover",
    ),
    key(
        "H",
        Modes::Normal,
        cmd(CommandId::Hover),
        "Hover documentation",
        "Hover",
    ),
    key(
        "]d",
        Modes::Normal,
        cmd(CommandId::DiagnosticNext),
        "Next diagnostic",
        "Next diagnostic",
    ),
    key(
        "[d",
        Modes::Normal,
        cmd(CommandId::DiagnosticPrev),
        "Previous diagnostic",
        "Previous diagnostic",
    ),
    key(
        "=",
        Modes::Normal,
        cmd(CommandId::Format),
        "Format the file",
        "Format",
    ),
    key(
        "Tab",
        Modes::Normal,
        cmd(CommandId::JumpForward),
        "Jump forward (Ctrl+O goes back)",
        "Jump forward",
    ),
    key(
        "e",
        Modes::Normal,
        cmd(CommandId::ToggleExplorer),
        "Show or hide the explorer",
        "Explorer",
    ),
    key(
        "q",
        Modes::Normal,
        cmd(CommandId::CloseOrQuit),
        "Close the tab, or quit on the last one",
        "close or Quit",
    ),
    key(
        ":",
        Modes::Both,
        like(KeyCode::F(1), NONE),
        "Command Bar (F1)",
        "Commands",
    ),
    key(
        "?",
        Modes::Both,
        cmd(CommandId::Keys),
        "This list of keys",
        "Keys",
    ),
    // Ctrl keys that keep their duckx-code meaning in every mode.
    key(
        "Shift+Space",
        Modes::Both,
        like(KeyCode::F(1), NONE),
        "Command Bar (same as :)",
        "Commands",
    ),
    key(
        "Ctrl+V",
        Modes::Both,
        like(KeyCode::Char('v'), CTRL),
        "Pastes (in vim it starts a block selection)",
        "Paste",
    ),
    key(
        "Ctrl+C",
        Modes::Both,
        like(KeyCode::Char('c'), CTRL),
        "Copies (in vim it cancels; it never quits here)",
        "Copy",
    ),
    key(
        "Ctrl+W",
        Modes::Both,
        like(KeyCode::Char('w'), CTRL),
        "PANE: pick a window (in vim it is the window prefix)",
        "Pane",
    ),
    key(
        "Ctrl+R",
        Modes::Both,
        like(KeyCode::Char('y'), CTRL),
        "Not used; redo is r (or Ctrl+Y, shown here)",
        "Not used",
    ),
    key(
        "Ctrl+H",
        Modes::Both,
        like(KeyCode::Left, NONE),
        "Same as Backspace: left (many terminals send Backspace as ^H)",
        "Left",
    ),
];

/// Hint row order in Normal (Vim keys). The last one always stays when the row is narrow.
const NORMAL_HINTS: &[&str] = &["i", "v", "V", "dd", "yy", "p", "u", "/", ":", "?"];
/// Hint row order in Visual (Vim keys). The last one always stays when the row is narrow.
const VISUAL_HINTS: &[&str] = &["d", "y", "c", ">", "<", "gc", "o", "p", "?", "Esc"];
/// Hint row order in Normal (Mnemonic keys). `s`, `c`, `x`, and `f` start a sequence.
const MNEMONIC_NORMAL_HINTS: &[&str] = &["i", "s", "c", "x", "p", "u", "r", "f", ":", "?"];
/// Hint row order in Visual (Mnemonic keys).
const MNEMONIC_VISUAL_HINTS: &[&str] = &["c", "x", "e", "p", ">", "<", "gc", "o", "?", "Esc"];

/// The first key of a Mnemonic sequence and the verb it stands for, for the hint row.
const MNEMONIC_PREFIXES: &[(char, &str)] = &[
    ('c', "Copy"),
    ('x', "Cut"),
    ('s', "Select"),
    ('d', "Delete"),
    ('f', "Find"),
    ('m', "Move line"),
    ('g', "Goto"),
];

/// Most a count can reach (`9999j`); more digits are ignored.
pub const MAX_COUNT: u32 = 9999;

/// A Visual Line selection: the line it started on, and where the cursor is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSelect {
    pub anchor_line: usize,
    pub cursor: usize,
}

/// The `?` list: the typed filter and the highlighted row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyList {
    pub query: String,
    pub selected: usize,
}

/// Everything modal editing remembers. The app keeps one for the whole window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModalState {
    pub mode: EditMode,
    /// The first key of a two-key sequence (`g`, `d`, `y`, `>`, `<`, `]`, `[`, `i`).
    pub pending: Option<char>,
    /// Digits typed before a motion or command (`5j`, `3dd`).
    pub count: Option<u32>,
    /// The `?` key list, while it is open.
    pub key_list: Option<KeyList>,
    /// One line for the hint row, such as a key with no command. Cleared by the next key.
    pub notice: Option<String>,
    /// Set in Visual Line mode.
    pub line: Option<LineSelect>,
    /// The key table Normal and Visual use.
    pub style: KeyStyle,
}

/// Actions only modal editing produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModalAction {
    /// Run a command the way its key does in the current mode (Visual-aware).
    Run(CommandId),
    /// A count before a motion or command: do it this many times.
    Repeat(u32, Box<SemanticAction>),
    /// The first key of a two-key sequence.
    Pend(char),
    /// One digit of a count.
    Count(u8),
    /// `Esc` with a count or sequence started: forget it.
    Cancel,
    /// `Esc` in Insert or Visual: back to Normal (after closing a popup or snippet).
    Escape,
    /// A key with no command here. The hint row says so; nothing is typed.
    Unbound(String),
    /// `5G` / `5gg`: go to that line.
    GotoLine(usize),
    /// `Ctrl+Space` in Normal: enter Insert, then do this.
    EnterInsertThen(Box<SemanticAction>),
    /// A chord in Visual that ends the selection (`Ctrl+C`, `Ctrl+X`, …): do it, then Normal.
    ThenNormal(Box<SemanticAction>),
    /// `Esc` and a letter that arrived together as `Alt+letter` (a terminal that does not
    /// tell them apart): leave the mode as `Esc` does, then press the letter in Normal.
    EscapeThen(char),
    /// Keys while the `?` list is open.
    KeyList(KeyListAction),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyListAction {
    Prev,
    Next,
    Insert(char),
    Backspace,
    /// Do the highlighted key.
    Run,
    Close,
}

fn modal(action: ModalAction) -> Option<SemanticAction> {
    Some(SemanticAction::Modal(action))
}

/// Route one event while Modeless is off and the editor has focus.
pub fn route(event: &Event, state: &ModalState, keys: &KeyMap) -> Option<SemanticAction> {
    match event {
        // A paste always inserts its text, in every mode.
        Event::Paste(text) if state.mode.is_visual() => modal(ModalAction::ThenNormal(Box::new(
            SemanticAction::InsertText(text.clone()),
        ))),
        Event::Paste(text) => Some(SemanticAction::InsertText(text.clone())),
        Event::Key(key) if key.kind != KeyEventKind::Release => route_key(key, state, keys),
        _ => None,
    }
}

/// `Ctrl+[` is `Esc` in every terminal that sends it as its own key, as vim users expect.
/// A terminal that already sends `Esc` for it never gets here.
fn ctrl_bracket_as_esc(key: &KeyEvent, keys: &KeyMap) -> Option<KeyEvent> {
    let chord = crate::keys::Chord::from_key(key);
    (key.code == KeyCode::Char('[')
        && key.modifiers == KeyModifiers::CONTROL
        && chord.is_none_or(|chord| keys.lookup(chord).is_none()))
    .then(|| KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
}

/// The letter of an `Alt+letter` that nothing is bound to: a terminal without the kitty
/// keyboard protocol sends a quick `Esc` then letter this way.
fn esc_then_letter(key: &KeyEvent, keys: &KeyMap) -> Option<char> {
    let KeyCode::Char(c) = key.code else {
        return None;
    };
    let mods = key.modifiers;
    (mods.contains(KeyModifiers::ALT)
        && !mods.contains(KeyModifiers::CONTROL)
        && !c.is_control()
        && c != ' '
        && crate::input::route_editor_key(key, keys).is_none())
    .then_some(c)
}

/// `Ctrl+D` and `Ctrl+U` in Normal: half a page, as in vim. A chord the user bound to
/// something else keeps that meaning.
fn half_page_token(key: &KeyEvent, keys: &KeyMap) -> Option<&'static str> {
    if key.modifiers != KeyModifiers::CONTROL {
        return None;
    }
    let free = || {
        crate::keys::Chord::from_key(key).is_none_or(|chord| {
            keys.lookup(chord)
                .is_none_or(|action| action == SemanticAction::Deselect)
        })
    };
    match key.code {
        KeyCode::Char('d') if free() => Some("Ctrl+D"),
        KeyCode::Char('u') if free() => Some("Ctrl+U"),
        _ => None,
    }
}

fn route_key(key: &KeyEvent, state: &ModalState, keys: &KeyMap) -> Option<SemanticAction> {
    if let Some(esc) = ctrl_bracket_as_esc(key, keys) {
        return route_key(&esc, state, keys);
    }
    if state.key_list.is_some() {
        return route_key_list(key);
    }
    if let Some(letter) = esc_then_letter(key, keys) {
        return modal(ModalAction::EscapeThen(letter));
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let esc = key.code == KeyCode::Esc && !ctrl && !alt;
    if state.mode == EditMode::Insert {
        if esc {
            return modal(ModalAction::Escape);
        }
        return crate::input::route_editor_key(key, keys);
    }
    if esc {
        return if state.pending.is_some() || state.count.is_some() {
            modal(ModalAction::Cancel)
        } else if state.mode.is_visual() {
            modal(ModalAction::Escape)
        } else {
            Some(SemanticAction::Dismiss)
        };
    }
    // `Shift+Space` opens the Command Bar; it is not a space to look up.
    if key.code == KeyCode::Char(' ') && key.modifiers.contains(KeyModifiers::SHIFT) {
        return passthrough(key, state, keys);
    }
    if state.mode == EditMode::Normal
        && state.pending.is_none()
        && let Some(token) = half_page_token(key, keys)
        && let Some(entry) = lookup(state.style, state.mode, token)
    {
        let action = action_for(entry, state.mode, keys)?;
        return Some(with_count(action, entry, state.count));
    }
    let Some(token) = plain_token(key) else {
        return passthrough(key, state, keys);
    };
    if state.pending.is_none()
        && let Some(digit) = single_digit(&token)
        && (digit != 0 || state.count.is_some())
    {
        return modal(ModalAction::Count(digit));
    }
    let seq = match state.pending {
        Some(first) => format!("{first}{token}"),
        None => token.clone(),
    };
    if let Some(entry) = lookup(state.style, state.mode, &seq) {
        let action = action_for(entry, state.mode, keys)?;
        return Some(with_count(action, entry, state.count));
    }
    if state.pending.is_none()
        && let Some(first) = single_char(&token)
        && starts_sequence(state.style, state.mode, first)
    {
        return modal(ModalAction::Pend(first));
    }
    modal(ModalAction::Unbound(seq))
}

/// A key that types or names a table entry: a plain character (or `AltGr`, which is
/// `Ctrl+Alt`), or a bare `Enter`, `Backspace`, or `Tab`.
fn plain_token(key: &KeyEvent) -> Option<String> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let bare = key.modifiers.is_empty();
    match key.code {
        KeyCode::Char(c) if ctrl == alt && !c.is_control() => Some(c.to_string()),
        KeyCode::Enter if bare => Some("Enter".into()),
        KeyCode::Backspace if bare => Some("Backspace".into()),
        KeyCode::Tab if bare => Some("Tab".into()),
        KeyCode::BackTab if !ctrl && !alt => Some("Shift+Tab".into()),
        _ => None,
    }
}

fn single_char(token: &str) -> Option<char> {
    let mut chars = token.chars();
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

fn single_digit(token: &str) -> Option<u8> {
    single_char(token)
        .and_then(|c| c.to_digit(10))
        .map(|digit| digit as u8)
}

/// A key name such as `Enter`, never the start of a two-key sequence.
fn is_named(keys: &str) -> bool {
    keys.chars().count() > 1 && keys.starts_with(|c: char| c.is_ascii_uppercase())
}

/// The entry for `keys` in `mode` with the `style` keys.
pub fn lookup(style: KeyStyle, mode: EditMode, keys: &str) -> Option<&'static ModalKey> {
    style
        .keys()
        .iter()
        .find(|entry| entry.keys == keys && entry.modes.has(mode))
}

/// True when some two-key sequence in `mode` starts with `first`.
fn starts_sequence(style: KeyStyle, mode: EditMode, first: char) -> bool {
    style.keys().iter().any(|entry| {
        entry.modes.has(mode)
            && !is_named(entry.keys)
            && entry.keys.chars().count() > 1
            && entry.keys.starts_with(first)
    })
}

/// The action a table entry produces in `mode`. `None` only when its modeless key
/// does nothing.
pub fn action_for(entry: &ModalKey, mode: EditMode, keys: &KeyMap) -> Option<SemanticAction> {
    match entry.run {
        Do::Command(id) => modal(ModalAction::Run(id)),
        Do::Key(code, modifiers) => {
            let action = crate::input::route_editor_key(&KeyEvent::new(code, modifiers), keys)?;
            Some(if mode.is_visual() {
                extend(action)
            } else {
                action
            })
        }
    }
}

/// A move or page in Visual extends the selection.
fn extend(action: SemanticAction) -> SemanticAction {
    match action {
        SemanticAction::Move { movement, .. } => SemanticAction::Move {
            movement,
            extend: true,
        },
        SemanticAction::PageUp { .. } => SemanticAction::PageUp { extend: true },
        SemanticAction::PageDown { .. } => SemanticAction::PageDown { extend: true },
        other => other,
    }
}

/// True for the commands that only change the mode, so a count does not repeat them.
fn changes_mode(id: CommandId) -> bool {
    crate::command::modal_only(id)
        && !matches!(
            id,
            CommandId::CopyLine
                | CommandId::CutLine
                | CommandId::HalfPageDown
                | CommandId::HalfPageUp
        )
}

/// Keys where a count is a line number (`5G`, `5gg`, `5gl`) rather than a repeat.
fn goes_to_line(entry: &ModalKey) -> bool {
    matches!(entry.keys, "gg" | "G") || matches!(entry.run, Do::Command(CommandId::GotoLine))
}

fn with_count(action: SemanticAction, entry: &ModalKey, count: Option<u32>) -> SemanticAction {
    let Some(count) = count.filter(|n| *n > 1 || goes_to_line(entry)) else {
        return action;
    };
    if goes_to_line(entry) {
        return SemanticAction::Modal(ModalAction::GotoLine(count.max(1) as usize));
    }
    match &action {
        SemanticAction::Modal(ModalAction::Run(id)) if changes_mode(*id) => action,
        _ => SemanticAction::Modal(ModalAction::Repeat(count, Box::new(action))),
    }
}

/// Chords, arrows, Home/End, PgUp/PgDn, Delete: the modeless meaning, fitted to the mode.
fn passthrough(key: &KeyEvent, state: &ModalState, keys: &KeyMap) -> Option<SemanticAction> {
    let action = crate::input::route_editor_key(key, keys)?;
    let unbound = || {
        let name = crate::keys::Chord::from_key(key)
            .map(|chord| chord.display())
            .unwrap_or_else(|| format!("{:?}", key.code));
        modal(ModalAction::Unbound(name))
    };
    match action {
        // Tab and Shift+Tab indent the selected lines in Visual.
        SemanticAction::Indent if state.mode.is_visual() => {
            modal(ModalAction::Run(CommandId::Indent))
        }
        SemanticAction::Outdent if state.mode.is_visual() => {
            modal(ModalAction::Run(CommandId::Outdent))
        }
        // Typing belongs to Insert.
        SemanticAction::InsertChar(_)
        | SemanticAction::InsertNewline
        | SemanticAction::Indent
        | SemanticAction::Outdent => unbound(),
        SemanticAction::Complete if state.mode == EditMode::Normal => modal(
            ModalAction::EnterInsertThen(Box::new(SemanticAction::Complete)),
        ),
        SemanticAction::Complete => unbound(),
        action if state.mode.is_visual() => {
            let action = extend(action);
            if ends_visual(&action) {
                modal(ModalAction::ThenNormal(Box::new(action)))
            } else {
                Some(repeat(action, state.count))
            }
        }
        action => Some(repeat(action, state.count)),
    }
}

/// Chords that finish with the selection in Visual, so the mode returns to Normal.
fn ends_visual(action: &SemanticAction) -> bool {
    matches!(
        action,
        SemanticAction::Copy
            | SemanticAction::Cut
            | SemanticAction::Paste
            | SemanticAction::Undo
            | SemanticAction::Redo
            | SemanticAction::Deselect
            | SemanticAction::DeleteForward
            | SemanticAction::DeleteBackward
            | SemanticAction::DeleteLine
            | SemanticAction::InsertText(_)
            | SemanticAction::ToggleComment
    )
}

fn repeat(action: SemanticAction, count: Option<u32>) -> SemanticAction {
    let repeats = matches!(
        action,
        SemanticAction::Move { .. }
            | SemanticAction::PageUp { .. }
            | SemanticAction::PageDown { .. }
    );
    match count {
        Some(n) if n > 1 && repeats => {
            SemanticAction::Modal(ModalAction::Repeat(n, Box::new(action)))
        }
        _ => action,
    }
}

fn route_key_list(key: &KeyEvent) -> Option<SemanticAction> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let action = match key.code {
        KeyCode::Up => KeyListAction::Prev,
        KeyCode::Down => KeyListAction::Next,
        KeyCode::Tab if !ctrl && !alt => KeyListAction::Next,
        KeyCode::BackTab => KeyListAction::Prev,
        KeyCode::Enter => KeyListAction::Run,
        KeyCode::Esc => KeyListAction::Close,
        KeyCode::Backspace => KeyListAction::Backspace,
        KeyCode::Char(c) if ctrl == alt && !c.is_control() => KeyListAction::Insert(c),
        _ => return None,
    };
    modal(ModalAction::KeyList(action))
}

/// The `?` list rows for `mode` (Insert lists Normal, where its keys work) with the
/// `style` keys, filtered by `query` against the keys, the name, the description,
/// and the command name.
pub fn key_list(style: KeyStyle, mode: EditMode, query: &str) -> Vec<&'static ModalKey> {
    let mode = if mode == EditMode::Insert {
        EditMode::Normal
    } else {
        mode
    };
    let query = query.to_lowercase();
    style
        .keys()
        .iter()
        .filter(|entry| entry.modes.has(mode))
        .filter(|entry| {
            query.is_empty()
                || entry.keys.to_lowercase().contains(&query)
                || entry.name.to_lowercase().contains(&query)
                || entry.label.to_lowercase().contains(&query)
                || command_name(entry).is_some_and(|name| name.contains(&query))
        })
        .collect()
}

/// The Command Bar command that does the same as `entry`, when it runs one.
pub fn command_name(entry: &ModalKey) -> Option<&'static str> {
    match entry.run {
        Do::Command(id) => Some(crate::command::spec(id).name),
        Do::Key(..) => None,
    }
}

/// The modeless key `entry` behaves like, as the Settings page writes it.
pub fn mirror_name(entry: &ModalKey) -> Option<String> {
    match entry.run {
        Do::Key(code, modifiers) => {
            crate::keys::Chord::from_key(&KeyEvent::new(code, modifiers)).map(|c| c.display())
        }
        Do::Command(_) => None,
    }
}

/// The keys that run `id` while Modeless is off with the `style` keys, for the
/// Command Bar, such as `dd`.
pub fn keys_for(style: KeyStyle, id: CommandId) -> Vec<&'static str> {
    let mut found: Vec<&'static str> = Vec::new();
    for entry in style.keys() {
        if entry.run == Do::Command(id) && !found.contains(&entry.keys) {
            found.push(entry.keys);
        }
    }
    found
}

/// One entry of the hint row: the name with the letters to press highlighted, or,
/// when the name does not hold them, the key and then the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub key: String,
    pub word: String,
    /// Char indices into `word` to highlight. Empty: show `key` before `word`.
    pub marks: Vec<usize>,
}

impl Hint {
    /// True when the name carries the key (the letters are highlighted in it).
    pub fn marked(&self) -> bool {
        !self.marks.is_empty()
    }

    /// `c̲opy l̲ine` as plain text: `[c]opy [l]ine`, or `key word` when not marked.
    /// For tests and messages.
    pub fn plain(&self) -> String {
        if !self.marked() {
            return format!("{} {}", self.key, self.word);
        }
        let mut out = String::new();
        for (index, c) in self.word.chars().enumerate() {
            if self.marks.contains(&index) {
                out.push('[');
                out.push(c);
                out.push(']');
            } else {
                out.push(c);
            }
        }
        out
    }
}

/// A hint with no highlighted letters: `key word`.
fn hint(key: impl Into<String>, word: impl Into<String>) -> Hint {
    Hint {
        key: key.into(),
        word: word.into(),
        marks: Vec::new(),
    }
}

/// A hint for `keys` named `name`: the letters highlighted when `name` holds them
/// all, else `keys` shown before it. The name is lowercase; each highlighted letter
/// is the exact case of its key, so a Shift key stands out (`v` → `[v]isual`,
/// `V` → `[V]isual line`). `typed` letters are already pressed (a started
/// sequence): they are not highlighted and show in lower case.
fn named_hint(keys: &str, name: &str, typed: usize) -> Hint {
    let shown_key: String = keys.chars().skip(typed).collect();
    let name = name.to_lowercase();
    match marks(&name, keys) {
        Some(found) => {
            let mut word: Vec<char> = name.chars().collect();
            for (n, &(index, c)) in found.iter().enumerate() {
                word[index] = if n < typed { c.to_ascii_lowercase() } else { c };
            }
            Hint {
                key: shown_key,
                word: word.into_iter().collect(),
                marks: found.iter().skip(typed).map(|&(index, _)| index).collect(),
            }
        }
        None => hint(shown_key, name),
    }
}

/// True when `name[index]` begins a word: the first char, or after a space, `-`, or `_`.
fn word_start(name: &[char], index: usize) -> bool {
    index == 0 || matches!(name[index - 1], ' ' | '-' | '_' | '(')
}

/// Where the letters of `keys` sit in `name`, in order, and how each is shown: for each
/// letter the first word start that has it, else its first place anywhere, always after
/// the letter before. The shown character is the key itself, so its case is the case
/// of the key (`p` → `[p]aste`, `P` → `[P]aste before`, `fP` → `[f]ind [P]revious`).
/// `None` when a letter is missing or the key is not only letters (`Tab`, `Ctrl+D`,
/// `>>`, `]d`), so the key is shown instead.
pub fn marks(name: &str, keys: &str) -> Option<Vec<(usize, char)>> {
    if keys.is_empty() || is_named(keys) || !keys.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let chars: Vec<char> = name.chars().collect();
    let mut from = 0;
    let mut found = Vec::new();
    for key in keys.chars() {
        let lower = key.to_ascii_lowercase();
        let hit = |i: &usize| chars[*i].to_ascii_lowercase() == lower;
        let index = (from..chars.len())
            .find(|i| word_start(&chars, *i) && hit(i))
            .or_else(|| (from..chars.len()).find(hit))?;
        found.push((index, key));
        from = index + 1;
    }
    Some(found)
}

/// The hint row for `state`: what the next key can do. The last entry is the one to
/// keep when the row is too narrow for all of them.
pub fn hints(state: &ModalState, keys: &KeyMap) -> Vec<Hint> {
    let table = state.style.keys();
    if let Some(first) = state.pending {
        let mut out: Vec<Hint> = Vec::new();
        for entry in table
            .iter()
            .filter(|entry| entry.modes.has(state.mode) && !is_named(entry.keys))
        {
            let Some(rest) = entry.keys.strip_prefix(first) else {
                continue;
            };
            if rest.is_empty() {
                continue;
            }
            let next = named_hint(entry.keys, entry.name, 1);
            // `cc` after `cl` (both Copy Line): one highlighted name is enough.
            let same_name = out
                .iter()
                .position(|shown| shown.word.eq_ignore_ascii_case(&next.word));
            match same_name {
                Some(at) if !out[at].marked() && next.marked() => out[at] = next,
                Some(_) => {}
                None => out.push(next),
            }
        }
        out.push(hint("Esc", "cancel"));
        return out;
    }
    if let Some(count) = state.count {
        return vec![
            hint(format!("{count}×"), "then a motion or command"),
            hint("Esc", "cancel"),
        ];
    }
    let order = match (state.mode, state.style) {
        (EditMode::Insert, _) => {
            let display = |id: &str| keys.display_of(id);
            return vec![
                hint(display("save"), "save"),
                hint(display("complete"), "complete"),
                hint(display("undo"), "undo"),
                hint(display("copy"), "copy"),
                hint(display("paste"), "paste"),
                hint("Esc", "normal"),
            ];
        }
        (EditMode::Normal, KeyStyle::Vim) => NORMAL_HINTS,
        (EditMode::Visual { .. }, KeyStyle::Vim) => VISUAL_HINTS,
        (EditMode::Normal, KeyStyle::Mnemonic) => MNEMONIC_NORMAL_HINTS,
        (EditMode::Visual { .. }, KeyStyle::Mnemonic) => MNEMONIC_VISUAL_HINTS,
    };
    order
        .iter()
        .filter_map(|keys| {
            if let Some(entry) = lookup(state.style, state.mode, keys) {
                return Some(named_hint(entry.keys, entry.name, 0));
            }
            let first = single_char(keys)?;
            prefix_name(state.style, state.mode, first).map(|name| named_hint(keys, name, 0))
        })
        .collect()
}

/// The verb a sequence's first key stands for, for the hint row (`c` → Copy).
fn prefix_name(style: KeyStyle, mode: EditMode, first: char) -> Option<&'static str> {
    if style != KeyStyle::Mnemonic || !starts_sequence(style, mode, first) {
        return None;
    }
    MNEMONIC_PREFIXES
        .iter()
        .find(|(c, _)| *c == first)
        .map(|(_, name)| *name)
}

/// The `?` list name of `entry`, highlighted the same way as the hint row.
pub fn list_hint(entry: &ModalKey) -> Hint {
    named_hint(entry.keys, entry.name, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Focus;
    use crate::buffer::Movement;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    fn ch(c: char) -> Event {
        let modifiers = if c.is_ascii_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        press(KeyCode::Char(c), modifiers)
    }

    fn state(mode: EditMode) -> ModalState {
        ModalState {
            mode,
            ..ModalState::default()
        }
    }

    fn normal(event: &Event) -> Option<SemanticAction> {
        route(event, &state(EditMode::Normal), &KeyMap::defaults())
    }

    fn visual(event: &Event) -> Option<SemanticAction> {
        route(
            event,
            &state(EditMode::Visual { line: false }),
            &KeyMap::defaults(),
        )
    }

    fn run(id: CommandId) -> Option<SemanticAction> {
        Some(SemanticAction::Modal(ModalAction::Run(id)))
    }

    fn mv(movement: Movement, extend: bool) -> Option<SemanticAction> {
        Some(SemanticAction::Move { movement, extend })
    }

    /// Every chord, arrow, F-key, and text key a modeless editor knows.
    fn sample_keys() -> Vec<Event> {
        let mut events = Vec::new();
        let mods = [
            KeyModifiers::NONE,
            KeyModifiers::SHIFT,
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            KeyModifiers::CONTROL | KeyModifiers::ALT,
            KeyModifiers::ALT | KeyModifiers::SHIFT,
        ];
        let mut codes: Vec<KeyCode> = (' '..='~').map(KeyCode::Char).collect();
        codes.extend([
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Enter,
            KeyCode::Backspace,
            KeyCode::Delete,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Esc,
            KeyCode::Insert,
        ]);
        codes.extend((1..=12).map(KeyCode::F));
        for code in codes {
            for modifiers in mods {
                events.push(press(code, modifiers));
            }
        }
        events.push(Event::Paste("pasted\ntext".into()));
        events
    }

    #[test]
    fn insert_mode_routes_every_key_like_the_modeless_editor_except_esc() {
        let keys = KeyMap::defaults();
        let insert = state(EditMode::Insert);
        for event in sample_keys() {
            let modeless = crate::input::route(&event, Focus::Editor, false);
            let modal = route(&event, &insert, &keys);
            match &event {
                Event::Key(key)
                    if key.code == KeyCode::Esc
                        && !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    assert_eq!(modal, Some(SemanticAction::Modal(ModalAction::Escape)));
                }
                // Esc and a letter that arrive as one Alt+letter are both kept.
                Event::Key(key) if modeless.is_none() && alt_letter(key).is_some() => {
                    assert_eq!(
                        modal,
                        Some(SemanticAction::Modal(ModalAction::EscapeThen(
                            alt_letter(key).unwrap()
                        )))
                    );
                }
                Event::Key(key) if is_ctrl_bracket(key) => {
                    assert_eq!(modal, Some(SemanticAction::Modal(ModalAction::Escape)));
                }
                _ => assert_eq!(modal, modeless, "{event:?}"),
            }
        }
    }

    /// The letter of an unbound `Alt+letter` (Shift allowed), as the router reads it.
    fn alt_letter(key: &KeyEvent) -> Option<char> {
        match key.code {
            KeyCode::Char(c)
                if key.modifiers.contains(KeyModifiers::ALT)
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && c != ' ' =>
            {
                Some(c)
            }
            _ => None,
        }
    }

    fn is_ctrl_bracket(key: &KeyEvent) -> bool {
        key.code == KeyCode::Char('[') && key.modifiers == KeyModifiers::CONTROL
    }

    #[test]
    fn every_bound_ctrl_alt_and_f_key_does_the_same_in_normal_mode() {
        let keys = KeyMap::defaults();
        let normal_state = state(EditMode::Normal);
        for event in sample_keys() {
            let Event::Key(key) = &event else { continue };
            let chord = key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && !(key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.modifiers.contains(KeyModifiers::ALT)
                    && matches!(key.code, KeyCode::Char(_)));
            let fkey = matches!(key.code, KeyCode::F(_));
            if !(chord || fkey) {
                continue;
            }
            let modeless = crate::input::route(&event, Focus::Editor, false);
            let modal = route(&event, &normal_state, &keys);
            // These are the documented differences, each with its own test below.
            if is_ctrl_bracket(key) {
                assert_eq!(modal, Some(SemanticAction::Dismiss));
                continue;
            }
            if key.modifiers == KeyModifiers::CONTROL
                && matches!(key.code, KeyCode::Char('d' | 'u'))
            {
                assert!(matches!(
                    modal,
                    Some(SemanticAction::Modal(ModalAction::Run(
                        CommandId::HalfPageDown | CommandId::HalfPageUp
                    )))
                ));
                continue;
            }
            if modeless.is_none() && alt_letter(key).is_some() {
                assert_eq!(
                    modal,
                    Some(SemanticAction::Modal(ModalAction::EscapeThen(
                        alt_letter(key).unwrap()
                    )))
                );
                continue;
            }
            match modeless {
                Some(SemanticAction::Complete) => assert_eq!(
                    modal,
                    Some(SemanticAction::Modal(ModalAction::EnterInsertThen(
                        Box::new(SemanticAction::Complete)
                    )))
                ),
                Some(
                    SemanticAction::InsertChar(_)
                    | SemanticAction::InsertNewline
                    | SemanticAction::Indent
                    | SemanticAction::Outdent,
                ) => assert!(matches!(
                    modal,
                    Some(SemanticAction::Modal(ModalAction::Unbound(_)))
                )),
                other => assert_eq!(modal, other, "{event:?}"),
            }
        }
    }

    #[test]
    fn normal_mode_never_types_a_plain_key() {
        for event in sample_keys() {
            let typed = matches!(
                normal(&event),
                Some(SemanticAction::InsertChar(_) | SemanticAction::InsertNewline)
            );
            assert!(!typed, "{event:?} typed in Normal");
        }
    }

    #[test]
    fn motions_move_in_normal_and_extend_in_visual() {
        assert_eq!(normal(&ch('h')), mv(Movement::Left, false));
        assert_eq!(normal(&ch('j')), mv(Movement::Down, false));
        assert_eq!(normal(&ch('w')), mv(Movement::WordRight, false));
        assert_eq!(normal(&ch('$')), mv(Movement::LineEnd, false));
        assert_eq!(normal(&ch('G')), mv(Movement::DocumentEnd, false));
        assert_eq!(normal(&ch('}')), mv(Movement::ParagraphDown, false));
        assert_eq!(visual(&ch('l')), mv(Movement::Right, true));
        assert_eq!(
            visual(&press(KeyCode::Down, KeyModifiers::NONE)),
            mv(Movement::Down, true)
        );
        assert_eq!(visual(&ch('b')), mv(Movement::WordLeft, true));
    }

    #[test]
    fn two_key_sequences_wait_for_their_second_key() {
        assert_eq!(
            normal(&ch('g')),
            Some(SemanticAction::Modal(ModalAction::Pend('g')))
        );
        let pending = ModalState {
            pending: Some('d'),
            ..state(EditMode::Normal)
        };
        let keys = KeyMap::defaults();
        assert_eq!(route(&ch('d'), &pending, &keys), run(CommandId::CutLine));
        assert_eq!(
            route(&ch('q'), &pending, &keys),
            Some(SemanticAction::Modal(ModalAction::Unbound("dq".into())))
        );
        assert_eq!(
            route(&press(KeyCode::Esc, KeyModifiers::NONE), &pending, &keys),
            Some(SemanticAction::Modal(ModalAction::Cancel))
        );
        let g = ModalState {
            pending: Some('g'),
            ..state(EditMode::Normal)
        };
        assert_eq!(
            route(&ch('g'), &g, &keys),
            mv(Movement::DocumentStart, false)
        );
        assert_eq!(route(&ch('d'), &g, &keys), run(CommandId::GotoDefinition));
    }

    #[test]
    fn counts_repeat_motions_and_commands_and_pick_a_line_for_g() {
        let keys = KeyMap::defaults();
        assert_eq!(
            normal(&ch('5')),
            Some(SemanticAction::Modal(ModalAction::Count(5)))
        );
        // A lone 0 is the start of the line; after a digit it is part of the count.
        assert_eq!(normal(&ch('0')), mv(Movement::LineStart, false));
        let five = ModalState {
            count: Some(5),
            ..state(EditMode::Normal)
        };
        assert_eq!(
            route(&ch('0'), &five, &keys),
            Some(SemanticAction::Modal(ModalAction::Count(0)))
        );
        assert_eq!(
            route(&ch('j'), &five, &keys),
            Some(SemanticAction::Modal(ModalAction::Repeat(
                5,
                Box::new(SemanticAction::Move {
                    movement: Movement::Down,
                    extend: false
                })
            )))
        );
        assert_eq!(
            route(&ch('G'), &five, &keys),
            Some(SemanticAction::Modal(ModalAction::GotoLine(5)))
        );
        let three_d = ModalState {
            count: Some(3),
            pending: Some('d'),
            ..state(EditMode::Normal)
        };
        assert_eq!(
            route(&ch('d'), &three_d, &keys),
            Some(SemanticAction::Modal(ModalAction::Repeat(
                3,
                Box::new(SemanticAction::Modal(ModalAction::Run(CommandId::CutLine)))
            )))
        );
        // A count does not repeat a mode change.
        assert_eq!(route(&ch('i'), &five, &keys), run(CommandId::ModeInsert));
    }

    #[test]
    fn esc_returns_to_normal_and_is_dismiss_in_normal() {
        let esc = press(KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(normal(&esc), Some(SemanticAction::Dismiss));
        assert_eq!(
            visual(&esc),
            Some(SemanticAction::Modal(ModalAction::Escape))
        );
        assert_eq!(
            route(&esc, &state(EditMode::Insert), &KeyMap::defaults()),
            Some(SemanticAction::Modal(ModalAction::Escape))
        );
    }

    #[test]
    fn visual_chords_that_finish_the_selection_return_to_normal() {
        let copy = press(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(
            visual(&copy),
            Some(SemanticAction::Modal(ModalAction::ThenNormal(Box::new(
                SemanticAction::Copy
            ))))
        );
        assert_eq!(visual(&ch('y')), run(CommandId::Copy));
        assert_eq!(visual(&ch('o')), run(CommandId::SwapSelectionEnds));
        assert_eq!(normal(&ch('o')), run(CommandId::OpenLineBelow));
    }

    #[test]
    fn unbound_letters_are_reported_not_typed() {
        assert_eq!(
            normal(&ch('q')),
            Some(SemanticAction::Modal(ModalAction::Unbound("q".into())))
        );
        assert_eq!(
            normal(&press(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(SemanticAction::Modal(ModalAction::Unbound(
                "Shift+Tab".into()
            )))
        );
    }

    #[test]
    fn a_paste_inserts_its_text_in_every_mode() {
        let paste = Event::Paste("dd".into());
        assert_eq!(
            normal(&paste),
            Some(SemanticAction::InsertText("dd".into()))
        );
        assert_eq!(
            visual(&paste),
            Some(SemanticAction::Modal(ModalAction::ThenNormal(Box::new(
                SemanticAction::InsertText("dd".into())
            ))))
        );
    }

    #[test]
    fn the_key_list_takes_every_key_while_open() {
        let open = ModalState {
            key_list: Some(KeyList::default()),
            ..state(EditMode::Normal)
        };
        let keys = KeyMap::defaults();
        assert_eq!(
            route(&ch('d'), &open, &keys),
            Some(SemanticAction::Modal(ModalAction::KeyList(
                KeyListAction::Insert('d')
            )))
        );
        assert_eq!(
            route(&press(KeyCode::Enter, KeyModifiers::NONE), &open, &keys),
            Some(SemanticAction::Modal(ModalAction::KeyList(
                KeyListAction::Run
            )))
        );
        assert_eq!(key_list(KeyStyle::Vim, EditMode::Normal, "dd").len(), 1);
        assert!(key_list(KeyStyle::Vim, EditMode::Normal, "").len() > 30);
        assert!(key_list(KeyStyle::Vim, EditMode::Visual { line: false }, "i(").len() == 1);
    }

    /// Every key does something reachable without modes: a Command Bar command that
    /// the bar offers while Modeless is off, or the very action of a modeless key.
    #[test]
    fn every_mode_key_has_a_command_or_is_a_modeless_key() {
        let keys = KeyMap::defaults();
        crate::command::show_modal_commands(true);
        for style in [KeyStyle::Vim, KeyStyle::Mnemonic] {
            for entry in style.keys() {
                assert!(!entry.name.is_empty(), "{} has no name", entry.keys);
                match entry.run {
                    Do::Command(id) => {
                        let spec = crate::command::spec(id);
                        assert!(
                            crate::command::find_command(
                                spec.name,
                                crate::command::CommandContext::Editor,
                                false
                            )
                            .is_some(),
                            "{style:?} {}: {} is not offered in the editor",
                            entry.keys,
                            spec.name
                        );
                    }
                    Do::Key(code, modifiers) => {
                        let modeless =
                            crate::input::route(&press(code, modifiers), Focus::Editor, false);
                        assert!(
                            modeless.is_some(),
                            "{style:?} {} mirrors a key that does nothing",
                            entry.keys
                        );
                        let mode = if entry.modes == Modes::Visual {
                            EditMode::Visual { line: false }
                        } else {
                            EditMode::Normal
                        };
                        let expected = if mode.is_visual() {
                            modeless.clone().map(extend)
                        } else {
                            modeless.clone()
                        };
                        assert_eq!(action_for(entry, mode, &keys), expected, "{}", entry.keys);
                    }
                }
            }
        }
        crate::command::show_modal_commands(false);
    }

    #[test]
    fn keys_are_unique_per_mode_and_never_both_a_key_and_a_prefix() {
        for style in [KeyStyle::Vim, KeyStyle::Mnemonic] {
            for mode in [EditMode::Normal, EditMode::Visual { line: false }] {
                let entries: Vec<_> = style.keys().iter().filter(|e| e.modes.has(mode)).collect();
                for (i, a) in entries.iter().enumerate() {
                    for b in &entries[i + 1..] {
                        assert_ne!(a.keys, b.keys, "{style:?}: {} twice in {mode:?}", a.keys);
                    }
                    if let Some(first) = single_char(a.keys) {
                        assert!(
                            !starts_sequence(style, mode, first),
                            "{style:?}: {} is a key and the start of a sequence in {mode:?}",
                            a.keys
                        );
                    }
                    assert!(
                        a.keys.chars().count() <= 2 || is_named(a.keys),
                        "{style:?}: {} is longer than two keys",
                        a.keys
                    );
                }
            }
        }
    }

    #[test]
    fn hints_follow_the_mode_and_the_pending_key() {
        let keys = KeyMap::defaults();
        let words = |state: &ModalState| -> Vec<String> {
            hints(state, &keys).iter().map(Hint::plain).collect()
        };
        let normal_hints = words(&state(EditMode::Normal));
        assert_eq!(
            normal_hints,
            [
                "[i]nsert",
                "[v]isual",
                "[V]isual line",
                "dd cut line",
                "yy copy line",
                "[p]aste",
                "[u]ndo",
                "/ find",
                ": commands",
                "? keys"
            ]
        );
        let visual_hints = words(&state(EditMode::Visual { line: true }));
        assert_eq!(
            visual_hints.first().map(String::as_str),
            Some("[d]elete (cut)")
        );
        assert_eq!(visual_hints.last().map(String::as_str), Some("Esc normal"));
        let insert_hints = words(&state(EditMode::Insert));
        assert_eq!(
            insert_hints.first().map(String::as_str),
            Some("Ctrl+S save")
        );
        assert_eq!(insert_hints.last().map(String::as_str), Some("Esc normal"));
        let g = ModalState {
            pending: Some('g'),
            ..state(EditMode::Normal)
        };
        assert_eq!(
            words(&g),
            [
                "goto be[g]inning",
                "toggle [c]omment",
                "goto [d]efinition",
                "goto [r]eferences",
                "goto [a]ctions",
                "goto [R]ename",
                "goto next [t]ab",
                "goto previous [T]ab",
                "Esc cancel"
            ]
        );
    }

    fn mnemonic(mode: EditMode) -> ModalState {
        ModalState {
            mode,
            style: KeyStyle::Mnemonic,
            ..ModalState::default()
        }
    }

    fn type_keys(state: &ModalState, text: &str) -> Option<SemanticAction> {
        let keys = KeyMap::defaults();
        let mut state = state.clone();
        let mut last = None;
        for c in text.chars() {
            last = route(&ch(c), &state, &keys);
            match &last {
                Some(SemanticAction::Modal(ModalAction::Pend(first))) => {
                    state.pending = Some(*first);
                }
                Some(SemanticAction::Modal(ModalAction::Count(digit))) => {
                    state.count = Some(state.count.unwrap_or(0) * 10 + u32::from(*digit));
                }
                _ => {
                    state.pending = None;
                    state.count = None;
                }
            }
        }
        last
    }

    // --- The hint matcher ---

    fn shown(name: &str, keys: &str) -> Option<String> {
        marks(name, keys)?;
        Some(named_hint(keys, name, 0).plain())
    }

    #[test]
    fn the_matcher_prefers_word_starts_then_any_letter_in_order() {
        assert_eq!(shown("Copy Line", "cl").as_deref(), Some("[c]opy [l]ine"));
        assert_eq!(shown("Copy Word", "cw").as_deref(), Some("[c]opy [w]ord"));
        // A letter that starts no word is found inside one.
        assert_eq!(
            shown("Goto beginning", "gg").as_deref(),
            Some("[g]oto be[g]inning")
        );
        assert_eq!(
            shown("toggle Comment", "gc").as_deref(),
            Some("to[g]gle [c]omment")
        );
        // Letters keep their order: the second never sits before the first.
        assert_eq!(shown("Line Copy", "cl"), None);
        // Case-insensitive; `-` and `_` start words too.
        assert_eq!(
            shown("close or Quit", "q").as_deref(),
            Some("close or [q]uit")
        );
        assert_eq!(shown("copy-line", "cl").as_deref(), Some("[c]opy-[l]ine"));
        // The same letter is never used twice.
        assert_eq!(shown("Copy Line", "cc"), None);
    }

    #[test]
    fn hint_letters_match_the_key_case_and_the_rest_is_lowercase() {
        // Names are lowercase; a highlighted letter is uppercase only when the key is.
        assert_eq!(
            shown("Paste Before", "P").as_deref(),
            Some("[P]aste before")
        );
        assert_eq!(shown("Paste", "p").as_deref(), Some("[p]aste"));
        assert_eq!(shown("example", "M").as_deref(), Some("exa[M]ple"));
        assert_eq!(
            shown("Goto Rename", "gR").as_deref(),
            Some("[g]oto [R]ename")
        );
        assert_eq!(
            shown("Goto previous Tab", "gT").as_deref(),
            Some("[g]oto previous [T]ab")
        );
        assert_eq!(
            shown("Find Previous", "fP").as_deref(),
            Some("[f]ind [P]revious")
        );
        // `v` and `V` both start with "visual": the case of the mark is the key.
        assert_eq!(shown("Visual", "v").as_deref(), Some("[v]isual"));
        assert_eq!(shown("Visual Line", "V").as_deref(), Some("[V]isual line"));
    }

    #[test]
    fn the_matcher_falls_back_for_symbols_named_keys_and_missing_letters() {
        for (name, keys) in [
            ("Cut", "x"),
            ("Cut Line", "xl"),
            ("Jump forward", "Tab"),
            ("Format", "="),
            ("Keys", "?"),
            ("Normal", "Esc"),
            ("Half page down", "Ctrl+D"),
            ("Left", "h"),
            ("Indent", ">>"),
            ("Next diagnostic", "]d"),
            ("Anything", ""),
        ] {
            assert_eq!(marks(name, keys), None, "{keys} in {name}");
        }
        let fallback = named_hint("x", "Cut", 0);
        assert!(!fallback.marked());
        assert_eq!(fallback.plain(), "x cut");
        assert_eq!(
            named_hint("Tab", "Jump forward", 0).plain(),
            "Tab jump forward"
        );
    }

    #[test]
    fn every_hint_row_entry_is_highlighted_where_its_letters_allow() {
        let keys = KeyMap::defaults();
        for style in [KeyStyle::Vim, KeyStyle::Mnemonic] {
            for mode in [EditMode::Normal, EditMode::Visual { line: false }] {
                let state = ModalState {
                    mode,
                    style,
                    ..ModalState::default()
                };
                for hint in hints(&state, &keys) {
                    let letters = !hint.key.is_empty()
                        && !is_named(&hint.key)
                        && hint.key.chars().all(|c| c.is_ascii_alphabetic());
                    if hint.marked() {
                        assert!(letters, "{style:?} {} is marked", hint.key);
                    } else if letters {
                        assert!(
                            marks(&hint.word, &hint.key).is_none(),
                            "{style:?} {} could be marked in {}",
                            hint.key,
                            hint.word
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn mnemonic_hint_rows_match_the_design() {
        let keys = KeyMap::defaults();
        let words = |state: &ModalState| -> Vec<String> {
            hints(state, &keys).iter().map(Hint::plain).collect()
        };
        assert_eq!(
            words(&mnemonic(EditMode::Normal)),
            [
                "[i]nsert",
                "[s]elect",
                "[c]opy",
                "x cut",
                "[p]aste",
                "[u]ndo",
                "[r]edo",
                "[f]ind",
                ": commands",
                "? keys"
            ]
        );
        assert_eq!(
            words(&mnemonic(EditMode::Visual { line: false })),
            [
                "[c]opy",
                "x cut",
                "[e]dit",
                "[p]aste",
                "> indent",
                "< outdent",
                "to[g]gle [c]omment",
                "[o]ther end",
                "? keys",
                "Esc normal"
            ]
        );
        let after = |first: char| ModalState {
            pending: Some(first),
            ..mnemonic(EditMode::Normal)
        };
        assert_eq!(
            words(&after('c')),
            ["copy [l]ine", "copy [w]ord", "copy [a]ll", "Esc cancel"]
        );
        assert_eq!(
            words(&after('s')),
            [
                "select [c]haracters",
                "select [l]ine",
                "select [w]ord",
                "select [a]ll",
                "select [b]racket",
                "select [q]uotes",
                "select [p]arens",
                "{ select inside { }",
                "[ select inside [ ]",
                "Esc cancel"
            ]
        );
        assert_eq!(
            words(&after('x')),
            // `x` is not in "cut": the keys show before the name (`xx` is the same line).
            ["l cut line", "w cut word", "Esc cancel"]
        );
        assert_eq!(
            words(&after('f')),
            [
                "find in [f]ile",
                "find [n]ext",
                "find [P]revious",
                "file [o]pen",
                "Esc cancel"
            ]
        );
    }

    // --- The Mnemonic keys ---

    #[test]
    fn vim_stays_the_default_key_style() {
        assert_eq!(ModalState::default().style, KeyStyle::Vim);
        assert_eq!(KeyStyle::default(), KeyStyle::Vim);
        assert_eq!(normal(&ch('x')), Some(SemanticAction::DeleteForward));
        assert_eq!(
            type_keys(&state(EditMode::Normal), "yy"),
            run(CommandId::CopyLine)
        );
        assert_eq!(
            type_keys(&state(EditMode::Normal), "dd"),
            run(CommandId::CutLine)
        );
        // `c` is not a Normal key in Vim style, so `cl` is not a Mnemonic copy.
        assert!(matches!(
            normal(&ch('c')),
            Some(SemanticAction::Modal(ModalAction::Unbound(_)))
        ));
    }

    #[test]
    fn mnemonic_keys_name_the_command() {
        let n = mnemonic(EditMode::Normal);
        for (typed, id) in [
            ("cl", CommandId::CopyLine),
            ("cc", CommandId::CopyLine),
            ("cw", CommandId::CopyWord),
            ("ca", CommandId::CopyAll),
            ("xl", CommandId::CutLine),
            ("xx", CommandId::CutLine),
            ("xw", CommandId::CutWord),
            ("p", CommandId::PasteAfter),
            ("P", CommandId::PasteBefore),
            ("sc", CommandId::ModeVisual),
            ("sl", CommandId::ModeVisualLine),
            ("ss", CommandId::ModeVisualLine),
            ("sw", CommandId::SelectWord),
            ("sa", CommandId::SelectAll),
            ("sb", CommandId::SelectBracket),
            ("sq", CommandId::SelectInsideQuotes),
            ("sp", CommandId::SelectInsideParens),
            ("dl", CommandId::DeleteLine),
            ("dd", CommandId::DeleteLine),
            ("u", CommandId::Undo),
            ("r", CommandId::Redo),
            ("md", CommandId::MoveLineDown),
            ("mu", CommandId::MoveLineUp),
            ("ff", CommandId::Find),
            ("fn", CommandId::FindNext),
            ("fP", CommandId::FindPrevious),
            ("fo", CommandId::FilePicker),
            ("R", CommandId::Replace),
            ("gd", CommandId::GotoDefinition),
            ("gr", CommandId::GotoReferences),
            ("ga", CommandId::CodeAction),
            ("gt", CommandId::TabNext),
            ("gT", CommandId::TabPrev),
            ("gl", CommandId::GotoLine),
            ("gh", CommandId::Hover),
            ("H", CommandId::Hover),
            ("gc", CommandId::Comment),
            ("q", CommandId::CloseOrQuit),
            ("e", CommandId::ToggleExplorer),
            ("i", CommandId::ModeInsert),
            ("v", CommandId::ModeVisual),
            ("?", CommandId::Keys),
        ] {
            assert_eq!(type_keys(&n, typed), run(id), "{typed}");
        }
        assert_eq!(type_keys(&n, "dc"), Some(SemanticAction::DeleteForward));
        // Motions are the Vim ones.
        assert_eq!(type_keys(&n, "j"), mv(Movement::Down, false));
        assert_eq!(type_keys(&n, "w"), normal(&ch('w')));
        assert_eq!(type_keys(&n, "G"), normal(&ch('G')));
        // The verb letters start a sequence; they do nothing alone.
        for first in ['c', 'x', 's', 'd', 'f', 'm', 'g'] {
            assert_eq!(
                type_keys(&n, &first.to_string()),
                Some(SemanticAction::Modal(ModalAction::Pend(first))),
                "{first}"
            );
        }
        // Unbound in Mnemonic: no `yy`, `U`, `K`, or `]e`.
        assert!(matches!(
            type_keys(&n, "y"),
            Some(SemanticAction::Modal(ModalAction::Unbound(_)))
        ));
    }

    #[test]
    fn mnemonic_visual_keys_copy_cut_edit_and_paste() {
        let v = mnemonic(EditMode::Visual { line: false });
        assert_eq!(type_keys(&v, "c"), run(CommandId::Copy));
        assert_eq!(type_keys(&v, "x"), run(CommandId::Cut));
        assert_eq!(type_keys(&v, "e"), run(CommandId::ChangeSelection));
        assert_eq!(type_keys(&v, "p"), run(CommandId::Paste));
        assert_eq!(type_keys(&v, ">"), run(CommandId::Indent));
        assert_eq!(type_keys(&v, "<"), run(CommandId::Outdent));
        assert_eq!(type_keys(&v, "gc"), run(CommandId::Comment));
        assert_eq!(type_keys(&v, "o"), run(CommandId::SwapSelectionEnds));
        assert_eq!(type_keys(&v, "sw"), run(CommandId::SelectWord));
        assert_eq!(type_keys(&v, "j"), mv(Movement::Down, true));
    }

    #[test]
    fn mnemonic_counts_and_esc_work_on_sequences() {
        let n = mnemonic(EditMode::Normal);
        assert_eq!(
            type_keys(&n, "3cl"),
            Some(SemanticAction::Modal(ModalAction::Repeat(
                3,
                Box::new(SemanticAction::Modal(ModalAction::Run(CommandId::CopyLine)))
            )))
        );
        assert_eq!(
            type_keys(&n, "3xx"),
            Some(SemanticAction::Modal(ModalAction::Repeat(
                3,
                Box::new(SemanticAction::Modal(ModalAction::Run(CommandId::CutLine)))
            )))
        );
        assert_eq!(
            type_keys(&n, "12gl"),
            Some(SemanticAction::Modal(ModalAction::GotoLine(12)))
        );
        let pending = ModalState {
            pending: Some('c'),
            ..n.clone()
        };
        assert_eq!(
            route(
                &press(KeyCode::Esc, KeyModifiers::NONE),
                &pending,
                &KeyMap::defaults()
            ),
            Some(SemanticAction::Modal(ModalAction::Cancel))
        );
    }

    #[test]
    fn the_key_list_and_command_keys_follow_the_style() {
        assert_eq!(
            key_list(KeyStyle::Mnemonic, EditMode::Normal, "copy line").len(),
            2
        );
        assert!(key_list(KeyStyle::Mnemonic, EditMode::Normal, "yy").is_empty());
        assert_eq!(keys_for(KeyStyle::Vim, CommandId::CopyLine), ["yy"]);
        assert_eq!(
            keys_for(KeyStyle::Mnemonic, CommandId::CopyLine),
            ["cl", "cc"]
        );
        assert_eq!(
            keys_for(KeyStyle::Mnemonic, CommandId::CutLine),
            ["xl", "xx"]
        );
        // With Vim keys a mode command shows the same key as before.
        for spec in crate::command::COMMANDS {
            if crate::command::modal_only(spec.id) && !spec.shortcut.is_empty() {
                assert_eq!(
                    keys_for(KeyStyle::Vim, spec.id).first().copied(),
                    Some(spec.shortcut),
                    "{}",
                    spec.name
                );
            }
        }
        // The `?` list shows the name with its letters marked.
        let entry = lookup(KeyStyle::Mnemonic, EditMode::Normal, "cw").unwrap();
        assert_eq!(list_hint(entry).plain(), "[c]opy [w]ord");
    }

    // --- Shortcut fixes for Modeless off ---

    #[test]
    fn shift_space_opens_the_command_bar_in_normal_and_visual() {
        let shift_space = press(KeyCode::Char(' '), KeyModifiers::SHIFT);
        let open = Some(SemanticAction::OpenCommandBar);
        assert_eq!(normal(&shift_space), open);
        assert_eq!(visual(&shift_space), open);
        // A plain space is still not a command.
        assert!(matches!(
            normal(&press(KeyCode::Char(' '), KeyModifiers::NONE)),
            Some(SemanticAction::Modal(ModalAction::Unbound(_)))
        ));
    }

    #[test]
    fn esc_and_a_letter_merged_into_alt_letter_keep_both_keys() {
        let keys = KeyMap::defaults();
        let alt_j = press(KeyCode::Char('j'), KeyModifiers::ALT);
        for mode in [
            EditMode::Insert,
            EditMode::Normal,
            EditMode::Visual { line: false },
        ] {
            assert_eq!(
                route(&alt_j, &state(mode), &keys),
                Some(SemanticAction::Modal(ModalAction::EscapeThen('j'))),
                "{mode:?}"
            );
        }
        // A capital arrives with Shift as well.
        let alt_g = press(KeyCode::Char('G'), KeyModifiers::ALT | KeyModifiers::SHIFT);
        assert_eq!(
            route(&alt_g, &state(EditMode::Insert), &keys),
            Some(SemanticAction::Modal(ModalAction::EscapeThen('G')))
        );
        // Bound Alt chords keep their meaning, and AltGr (Ctrl+Alt) still types in Insert.
        let convert = press(KeyCode::Char('`'), KeyModifiers::ALT);
        assert_eq!(
            route(&convert, &state(EditMode::Insert), &keys),
            Some(SemanticAction::ConvertKeyboard)
        );
        let altgr = press(
            KeyCode::Char('@'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert_eq!(
            route(&altgr, &state(EditMode::Insert), &keys),
            Some(SemanticAction::InsertChar('@'))
        );
    }

    #[test]
    fn ctrl_left_bracket_is_esc_in_every_mode() {
        let keys = KeyMap::defaults();
        let ctrl_bracket = press(KeyCode::Char('['), KeyModifiers::CONTROL);
        assert_eq!(
            route(&ctrl_bracket, &state(EditMode::Insert), &keys),
            Some(SemanticAction::Modal(ModalAction::Escape))
        );
        assert_eq!(
            route(&ctrl_bracket, &state(EditMode::Normal), &keys),
            Some(SemanticAction::Dismiss)
        );
        assert_eq!(
            route(
                &ctrl_bracket,
                &state(EditMode::Visual { line: true }),
                &keys
            ),
            Some(SemanticAction::Modal(ModalAction::Escape))
        );
        let pending = ModalState {
            pending: Some('g'),
            ..state(EditMode::Normal)
        };
        assert_eq!(
            route(&ctrl_bracket, &pending, &keys),
            Some(SemanticAction::Modal(ModalAction::Cancel))
        );
    }

    #[test]
    fn plain_keys_for_actions_that_need_a_shortcut_a_terminal_may_not_send() {
        let keys = KeyMap::defaults();
        let seq = |first: char, second: char| {
            let pending = ModalState {
                pending: Some(first),
                ..state(EditMode::Normal)
            };
            route(&ch(second), &pending, &keys)
        };
        assert_eq!(seq('g', 'r'), run(CommandId::GotoReferences));
        assert_eq!(seq('g', 'a'), run(CommandId::CodeAction));
        assert_eq!(seq('g', 't'), run(CommandId::TabNext));
        assert_eq!(seq('g', 'T'), run(CommandId::TabPrev));
        assert_eq!(seq(']', 'e'), run(CommandId::MoveLineDown));
        assert_eq!(seq('[', 'e'), run(CommandId::MoveLineUp));
        // gR is Shift+F2, which starts the rename prompt.
        assert_eq!(seq('g', 'R'), Some(SemanticAction::StartRename));
        assert_eq!(normal(&ch('=')), run(CommandId::Format));
        assert_eq!(
            normal(&press(KeyCode::Tab, KeyModifiers::NONE)),
            run(CommandId::JumpForward)
        );
    }

    #[test]
    fn ctrl_d_and_ctrl_u_are_half_pages_in_normal_only() {
        let keys = KeyMap::defaults();
        let down = press(KeyCode::Char('d'), KeyModifiers::CONTROL);
        let up = press(KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert_eq!(normal(&down), run(CommandId::HalfPageDown));
        assert_eq!(normal(&up), run(CommandId::HalfPageUp));
        // Insert keeps Ctrl+D (deselect), exactly as Modeless does.
        assert_eq!(
            route(&down, &state(EditMode::Insert), &keys),
            crate::input::route(&down, Focus::Editor, false)
        );
        assert_eq!(
            route(&down, &state(EditMode::Insert), &keys),
            Some(SemanticAction::Deselect)
        );
        // Modeless routing is not touched by any of this.
        assert_eq!(
            crate::input::route(&down, Focus::Editor, false),
            Some(SemanticAction::Deselect)
        );
        assert_eq!(crate::input::route(&up, Focus::Editor, false), None);
        // A user who bound Ctrl+U to something keeps it.
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("cut".to_string(), "ctrl+u".to_string());
        let rebound = KeyMap::resolve(&overrides);
        assert_eq!(
            route(&up, &state(EditMode::Normal), &rebound),
            Some(SemanticAction::Cut)
        );
    }

    #[test]
    fn tab_and_shift_tab_indent_and_outdent_in_visual() {
        let keys = KeyMap::defaults();
        for mode in [
            EditMode::Visual { line: false },
            EditMode::Visual { line: true },
        ] {
            assert_eq!(
                route(
                    &press(KeyCode::Tab, KeyModifiers::NONE),
                    &state(mode),
                    &keys
                ),
                run(CommandId::Indent)
            );
            assert_eq!(
                route(
                    &press(KeyCode::BackTab, KeyModifiers::SHIFT),
                    &state(mode),
                    &keys
                ),
                run(CommandId::Outdent)
            );
            assert_eq!(
                route(
                    &press(KeyCode::Tab, KeyModifiers::SHIFT),
                    &state(mode),
                    &keys
                ),
                run(CommandId::Outdent)
            );
        }
    }

    #[test]
    fn line_keys_use_the_line_commands_and_visual_p_stays_a_plain_paste() {
        assert_eq!(visual(&ch('p')), run(CommandId::Paste));
        assert_eq!(normal(&ch('p')), run(CommandId::PasteAfter));
        assert_eq!(normal(&ch('P')), run(CommandId::PasteBefore));
        let y = ModalState {
            pending: Some('y'),
            ..state(EditMode::Normal)
        };
        assert_eq!(
            route(&ch('y'), &y, &KeyMap::defaults()),
            run(CommandId::CopyLine)
        );
    }

    #[test]
    fn the_key_list_documents_the_ctrl_keys_that_differ_from_vim() {
        for mode in [EditMode::Normal, EditMode::Visual { line: false }] {
            let rows = key_list(KeyStyle::Vim, mode, "ctrl");
            let keys: Vec<_> = rows.iter().map(|row| row.keys).collect();
            for expected in ["Ctrl+V", "Ctrl+C", "Ctrl+W", "Ctrl+R", "Ctrl+H"] {
                assert!(keys.contains(&expected), "{expected} in {mode:?}");
            }
        }
        // They are notes: the keys themselves still do what they do without modes.
        let keys = KeyMap::defaults();
        for (c, expected) in [
            ('v', SemanticAction::Paste),
            ('c', SemanticAction::Copy),
            ('w', SemanticAction::FocusPrefix),
        ] {
            assert_eq!(
                route(
                    &press(KeyCode::Char(c), KeyModifiers::CONTROL),
                    &state(EditMode::Normal),
                    &keys
                ),
                Some(expected)
            );
        }
        // Ctrl+R is unused: it does not redo.
        assert_ne!(
            route(
                &press(KeyCode::Char('r'), KeyModifiers::CONTROL),
                &state(EditMode::Normal),
                &keys
            ),
            Some(SemanticAction::Redo)
        );
    }
}
