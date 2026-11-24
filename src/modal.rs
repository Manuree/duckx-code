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
//! - In Normal and Visual, plain keys come from [`KEYS`]. Every entry either runs a
//!   Command Bar command or behaves like a modeless key (`h` is `Left`), so nothing
//!   here is reachable only by a letter.
//!
//! The selling point is clarity, so the same table feeds the hint row above the
//! status bar ([`hints`]) and the `?` key list ([`key_list`]).

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
    /// Short word for the hint row. Empty: only in the key list.
    pub hint: &'static str,
}

const NONE: KeyModifiers = KeyModifiers::NONE;
const CTRL: KeyModifiers = KeyModifiers::CONTROL;

const fn key(
    keys: &'static str,
    modes: Modes,
    run: Do,
    label: &'static str,
    hint: &'static str,
) -> ModalKey {
    ModalKey {
        keys,
        modes,
        run,
        label,
        hint,
    }
}

const fn cmd(id: CommandId) -> Do {
    Do::Command(id)
}

const fn like(code: KeyCode, modifiers: KeyModifiers) -> Do {
    Do::Key(code, modifiers)
}

/// Every key of Normal and Visual mode, in the order the `?` list shows them.
pub const KEYS: &[ModalKey] = &[
    // Changing mode.
    key(
        "i",
        Modes::Normal,
        cmd(CommandId::ModeInsert),
        "Type at the cursor (Insert)",
        "insert",
    ),
    key(
        "a",
        Modes::Normal,
        cmd(CommandId::InsertAfter),
        "Type after the cursor",
        "",
    ),
    key(
        "I",
        Modes::Normal,
        cmd(CommandId::InsertLineStart),
        "Type at the start of the line",
        "",
    ),
    key(
        "A",
        Modes::Normal,
        cmd(CommandId::InsertLineEnd),
        "Type at the end of the line",
        "",
    ),
    key(
        "o",
        Modes::Normal,
        cmd(CommandId::OpenLineBelow),
        "Open a line below and type",
        "",
    ),
    key(
        "O",
        Modes::Normal,
        cmd(CommandId::OpenLineAbove),
        "Open a line above and type",
        "",
    ),
    key(
        "v",
        Modes::Both,
        cmd(CommandId::ModeVisual),
        "Select characters (Visual)",
        "select",
    ),
    key(
        "V",
        Modes::Both,
        cmd(CommandId::ModeVisualLine),
        "Select whole lines (Visual Line)",
        "lines",
    ),
    key(
        "Esc",
        Modes::Both,
        cmd(CommandId::ModeNormal),
        "Back to Normal; in Normal, close popups",
        "normal",
    ),
    // Motions: the same moves as the arrow and Ctrl keys.
    key("h", Modes::Both, like(KeyCode::Left, NONE), "Left (←)", ""),
    key("j", Modes::Both, like(KeyCode::Down, NONE), "Down (↓)", ""),
    key("k", Modes::Both, like(KeyCode::Up, NONE), "Up (↑)", ""),
    key(
        "l",
        Modes::Both,
        like(KeyCode::Right, NONE),
        "Right (→)",
        "",
    ),
    key(
        "w",
        Modes::Both,
        like(KeyCode::Right, CTRL),
        "Next word (Ctrl+→)",
        "",
    ),
    key(
        "b",
        Modes::Both,
        like(KeyCode::Left, CTRL),
        "Previous word (Ctrl+←)",
        "",
    ),
    key(
        "0",
        Modes::Both,
        like(KeyCode::Home, NONE),
        "Start of the line (Home)",
        "",
    ),
    key(
        "$",
        Modes::Both,
        like(KeyCode::End, NONE),
        "End of the line (End)",
        "",
    ),
    key(
        "gg",
        Modes::Both,
        like(KeyCode::Home, CTRL),
        "Top of the file; 5gg: line 5 (Ctrl+Home)",
        "top",
    ),
    key(
        "G",
        Modes::Both,
        like(KeyCode::End, CTRL),
        "End of the file; 5G: line 5 (Ctrl+End)",
        "end",
    ),
    key(
        "{",
        Modes::Both,
        like(KeyCode::Up, CTRL),
        "Previous blank line (Ctrl+↑)",
        "",
    ),
    key(
        "}",
        Modes::Both,
        like(KeyCode::Down, CTRL),
        "Next blank line (Ctrl+↓)",
        "",
    ),
    key(
        "Enter",
        Modes::Both,
        like(KeyCode::Down, NONE),
        "Down (↓)",
        "",
    ),
    key(
        "Backspace",
        Modes::Both,
        like(KeyCode::Left, NONE),
        "Left (←)",
        "",
    ),
    // Editing in Normal.
    key(
        "x",
        Modes::Normal,
        like(KeyCode::Delete, NONE),
        "Delete the character (Delete)",
        "",
    ),
    key(
        "dd",
        Modes::Normal,
        cmd(CommandId::DeleteLine),
        "Delete the line",
        "del line",
    ),
    key(
        "yy",
        Modes::Normal,
        cmd(CommandId::Copy),
        "Copy the line",
        "copy line",
    ),
    key(
        "p",
        Modes::Both,
        cmd(CommandId::Paste),
        "Paste (over the selection in Visual)",
        "paste",
    ),
    key("u", Modes::Normal, cmd(CommandId::Undo), "Undo", "undo"),
    key("U", Modes::Normal, cmd(CommandId::Redo), "Redo", ""),
    key(
        ">>",
        Modes::Normal,
        cmd(CommandId::Indent),
        "Indent the line",
        "indent",
    ),
    key(
        "<<",
        Modes::Normal,
        cmd(CommandId::Outdent),
        "Outdent the line",
        "outdent",
    ),
    key(
        "gc",
        Modes::Both,
        cmd(CommandId::Comment),
        "Comment or uncomment",
        "comment",
    ),
    // Editing the Visual selection.
    key(
        "d",
        Modes::Visual,
        cmd(CommandId::Cut),
        "Cut the selection",
        "cut",
    ),
    key(
        "x",
        Modes::Visual,
        cmd(CommandId::Cut),
        "Cut the selection",
        "",
    ),
    key(
        "y",
        Modes::Visual,
        cmd(CommandId::Copy),
        "Copy the selection",
        "copy",
    ),
    key(
        "c",
        Modes::Visual,
        cmd(CommandId::ChangeSelection),
        "Delete the selection and type",
        "change",
    ),
    key(
        ">",
        Modes::Visual,
        cmd(CommandId::Indent),
        "Indent the lines",
        "indent",
    ),
    key(
        "<",
        Modes::Visual,
        cmd(CommandId::Outdent),
        "Outdent the lines",
        "outdent",
    ),
    key(
        "o",
        Modes::Visual,
        cmd(CommandId::SwapSelectionEnds),
        "Cursor to the other end",
        "other end",
    ),
    key(
        "i\"",
        Modes::Visual,
        cmd(CommandId::SelectInsideQuotes),
        "Select inside quotes",
        "inside quotes",
    ),
    key(
        "i'",
        Modes::Visual,
        cmd(CommandId::SelectInsideQuotes),
        "Select inside quotes",
        "inside quotes",
    ),
    key(
        "i(",
        Modes::Visual,
        cmd(CommandId::SelectInsideParens),
        "Select inside ( )",
        "inside ( )",
    ),
    key(
        "i)",
        Modes::Visual,
        cmd(CommandId::SelectInsideParens),
        "Select inside ( )",
        "inside ( )",
    ),
    key(
        "i{",
        Modes::Visual,
        cmd(CommandId::SelectInsideBraces),
        "Select inside { }",
        "inside { }",
    ),
    key(
        "i}",
        Modes::Visual,
        cmd(CommandId::SelectInsideBraces),
        "Select inside { }",
        "inside { }",
    ),
    key(
        "i[",
        Modes::Visual,
        cmd(CommandId::SelectInsideBrackets),
        "Select inside [ ]",
        "inside [ ]",
    ),
    key(
        "i]",
        Modes::Visual,
        cmd(CommandId::SelectInsideBrackets),
        "Select inside [ ]",
        "inside [ ]",
    ),
    key(
        "%",
        Modes::Normal,
        cmd(CommandId::MatchBracket),
        "Matching bracket",
        "",
    ),
    key(
        "%",
        Modes::Visual,
        cmd(CommandId::SelectBracket),
        "Select to the matching bracket",
        "",
    ),
    // Search, language server, and the rest.
    key(
        "/",
        Modes::Normal,
        cmd(CommandId::Find),
        "Find in the file",
        "find",
    ),
    key(
        "n",
        Modes::Normal,
        cmd(CommandId::FindNext),
        "Next match",
        "",
    ),
    key(
        "N",
        Modes::Normal,
        cmd(CommandId::FindPrevious),
        "Previous match",
        "",
    ),
    key(
        "gd",
        Modes::Normal,
        cmd(CommandId::GotoDefinition),
        "Go to definition",
        "definition",
    ),
    key(
        "K",
        Modes::Normal,
        cmd(CommandId::Hover),
        "Hover documentation",
        "",
    ),
    key(
        "]d",
        Modes::Normal,
        cmd(CommandId::DiagnosticNext),
        "Next diagnostic",
        "next diagnostic",
    ),
    key(
        "[d",
        Modes::Normal,
        cmd(CommandId::DiagnosticPrev),
        "Previous diagnostic",
        "previous diagnostic",
    ),
    key(
        ":",
        Modes::Both,
        like(KeyCode::F(1), NONE),
        "Command Bar (F1)",
        "commands",
    ),
    key(
        "?",
        Modes::Both,
        cmd(CommandId::Keys),
        "This list of keys",
        "keys",
    ),
];

/// Hint row order in Normal. The last one always stays when the row is narrow.
const NORMAL_HINTS: &[&str] = &["i", "v", "V", "dd", "yy", "p", "u", "/", ":", "?"];
/// Hint row order in Visual. The last one always stays when the row is narrow.
const VISUAL_HINTS: &[&str] = &["d", "y", "c", ">", "<", "gc", "o", "p", "?", "Esc"];

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

fn route_key(key: &KeyEvent, state: &ModalState, keys: &KeyMap) -> Option<SemanticAction> {
    if state.key_list.is_some() {
        return route_key_list(key);
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
    if let Some(entry) = lookup(state.mode, &seq) {
        let action = action_for(entry, state.mode, keys)?;
        return Some(with_count(action, entry, state.count));
    }
    if state.pending.is_none()
        && let Some(first) = single_char(&token)
        && starts_sequence(state.mode, first)
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

/// The entry for `keys` in `mode`.
pub fn lookup(mode: EditMode, keys: &str) -> Option<&'static ModalKey> {
    KEYS.iter()
        .find(|entry| entry.keys == keys && entry.modes.has(mode))
}

/// True when some two-key sequence in `mode` starts with `first`.
fn starts_sequence(mode: EditMode, first: char) -> bool {
    KEYS.iter().any(|entry| {
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
}

fn with_count(action: SemanticAction, entry: &ModalKey, count: Option<u32>) -> SemanticAction {
    let Some(count) = count.filter(|n| *n > 1 || matches!(entry.keys, "gg" | "G")) else {
        return action;
    };
    if matches!(entry.keys, "gg" | "G") {
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

/// The `?` list rows for `mode` (Insert lists Normal, where its keys work), filtered
/// by `query` against the keys, the description, and the command name.
pub fn key_list(mode: EditMode, query: &str) -> Vec<&'static ModalKey> {
    let mode = if mode == EditMode::Insert {
        EditMode::Normal
    } else {
        mode
    };
    let query = query.to_lowercase();
    KEYS.iter()
        .filter(|entry| entry.modes.has(mode))
        .filter(|entry| {
            query.is_empty()
                || entry.keys.to_lowercase().contains(&query)
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

/// The keys that run `id` while Modeless is off, for the Command Bar, such as `dd`.
pub fn keys_for(id: CommandId) -> Vec<&'static str> {
    let mut found: Vec<&'static str> = Vec::new();
    for entry in KEYS {
        if entry.run == Do::Command(id) && !found.contains(&entry.keys) {
            found.push(entry.keys);
        }
    }
    found
}

/// One `key word` pair of the hint row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub key: String,
    pub word: String,
}

fn hint(key: impl Into<String>, word: impl Into<String>) -> Hint {
    Hint {
        key: key.into(),
        word: word.into(),
    }
}

/// The hint row for `state`: what the next key can do. The last entry is the one to
/// keep when the row is too narrow for all of them.
pub fn hints(state: &ModalState, keys: &KeyMap) -> Vec<Hint> {
    if let Some(first) = state.pending {
        let mut out: Vec<Hint> = KEYS
            .iter()
            .filter(|entry| entry.modes.has(state.mode) && !is_named(entry.keys))
            .filter_map(|entry| {
                let rest = entry.keys.strip_prefix(first)?;
                (!rest.is_empty()).then(|| hint(rest, short(entry)))
            })
            .collect();
        out.dedup();
        out.push(hint("Esc", "cancel"));
        return out;
    }
    if let Some(count) = state.count {
        return vec![
            hint(format!("{count}×"), "then a motion or command"),
            hint("Esc", "cancel"),
        ];
    }
    let order = match state.mode {
        EditMode::Insert => {
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
        EditMode::Normal => NORMAL_HINTS,
        EditMode::Visual { .. } => VISUAL_HINTS,
    };
    order
        .iter()
        .filter_map(|keys| lookup(state.mode, keys))
        .map(|entry| hint(entry.keys, short(entry)))
        .collect()
}

fn short(entry: &ModalKey) -> String {
    if entry.hint.is_empty() {
        entry.label.to_lowercase()
    } else {
        entry.hint.to_string()
    }
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
                _ => assert_eq!(modal, modeless, "{event:?}"),
            }
        }
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
        assert_eq!(route(&ch('d'), &pending, &keys), run(CommandId::DeleteLine));
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
                Box::new(SemanticAction::Modal(ModalAction::Run(
                    CommandId::DeleteLine
                )))
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
            normal(&press(KeyCode::Tab, KeyModifiers::NONE)),
            Some(SemanticAction::Modal(ModalAction::Unbound("Tab".into())))
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
        assert_eq!(key_list(EditMode::Normal, "dd").len(), 1);
        assert!(key_list(EditMode::Normal, "").len() > 30);
        assert!(key_list(EditMode::Visual { line: false }, "i(").len() == 1);
    }

    /// Every key does something reachable without modes: a Command Bar command that
    /// the bar offers while Modeless is off, or the very action of a modeless key.
    #[test]
    fn every_mode_key_has_a_command_or_is_a_modeless_key() {
        let keys = KeyMap::defaults();
        crate::command::show_modal_commands(true);
        for entry in KEYS {
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
                        "{}: {} is not offered in the editor",
                        entry.keys,
                        spec.name
                    );
                }
                Do::Key(code, modifiers) => {
                    let modeless =
                        crate::input::route(&press(code, modifiers), Focus::Editor, false);
                    assert!(
                        modeless.is_some(),
                        "{} mirrors a key that does nothing",
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
        crate::command::show_modal_commands(false);
    }

    #[test]
    fn keys_are_unique_per_mode_and_never_both_a_key_and_a_prefix() {
        for mode in [EditMode::Normal, EditMode::Visual { line: false }] {
            let entries: Vec<_> = KEYS.iter().filter(|e| e.modes.has(mode)).collect();
            for (i, a) in entries.iter().enumerate() {
                for b in &entries[i + 1..] {
                    assert_ne!(a.keys, b.keys, "{} twice in {mode:?}", a.keys);
                }
                if let Some(first) = single_char(a.keys) {
                    assert!(
                        !starts_sequence(mode, first),
                        "{} is a key and the start of a sequence in {mode:?}",
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
            hints(state, &keys)
                .into_iter()
                .map(|h| format!("{} {}", h.key, h.word))
                .collect()
        };
        let normal_hints = words(&state(EditMode::Normal));
        assert_eq!(normal_hints.first().map(String::as_str), Some("i insert"));
        assert_eq!(normal_hints.last().map(String::as_str), Some("? keys"));
        let visual_hints = words(&state(EditMode::Visual { line: true }));
        assert_eq!(visual_hints.first().map(String::as_str), Some("d cut"));
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
            ["g top", "c comment", "d definition", "Esc cancel"]
        );
    }
}
