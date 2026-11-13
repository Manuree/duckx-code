//! Command chords. The settings page and the input router share this table.
//! Navigation, text insertion, and a few fallbacks are not in the table.

use std::collections::{BTreeMap, HashMap};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::action::SemanticAction;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    kind: Kind,
    ctrl: bool,
    alt: bool,
    shift: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Char(char),
    F(u8),
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    Enter,
    Backspace,
    Delete,
    Esc,
}

pub struct Binding {
    pub id: &'static str,
    pub label: &'static str,
    pub detail: &'static str,
    defaults: &'static [Chord],
    action: fn() -> SemanticAction,
}

#[derive(Clone)]
pub struct KeyMap {
    by_chord: HashMap<Chord, fn() -> SemanticAction>,
    owner: HashMap<Chord, &'static str>,
    shown: HashMap<&'static str, Chord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignError {
    /// The chord edits text or moves the cursor.
    Locked,
    /// `F1`, `Ctrl+;`, or `Ctrl+Q`.
    Reserved,
    /// Another command already uses this chord.
    Conflict(&'static str),
    Blank,
}

pub fn bindings() -> &'static [Binding] {
    const SHIFT_SPACE: Chord = Chord::mods(Kind::Char(' '), false, false, true);
    const fn ch(c: char, ctrl: bool, alt: bool, shift: bool) -> Chord {
        Chord::mods(Kind::Char(c), ctrl, alt, shift)
    }
    const fn f(n: u8, shift: bool) -> Chord {
        Chord::mods(Kind::F(n), false, false, shift)
    }
    const fn arrow(kind: Kind, ctrl: bool, alt: bool) -> Chord {
        Chord::mods(kind, ctrl, alt, false)
    }

    static BINDINGS: &[Binding] = &[
        Binding {
            id: "command-bar",
            label: "Command bar",
            detail: "Key that opens the command bar.",
            defaults: &[SHIFT_SPACE],
            action: || SemanticAction::OpenCommandBar,
        },
        Binding {
            id: "save",
            label: "Save",
            detail: "Write the focused file.",
            defaults: &[ch('s', true, false, false)],
            action: || SemanticAction::Save,
        },
        Binding {
            id: "copy",
            label: "Copy",
            detail: "Copy the selection, or the whole line.",
            defaults: &[ch('c', true, false, false)],
            action: || SemanticAction::Copy,
        },
        Binding {
            id: "cut",
            label: "Cut",
            detail: "Cut the selection, or the whole line.",
            defaults: &[ch('x', true, false, false)],
            action: || SemanticAction::Cut,
        },
        Binding {
            id: "paste",
            label: "Paste",
            detail: "Insert the clipboard.",
            defaults: &[ch('v', true, false, false)],
            action: || SemanticAction::Paste,
        },
        Binding {
            id: "undo",
            label: "Undo",
            detail: "Undo the last edit.",
            defaults: &[ch('z', true, false, false)],
            action: || SemanticAction::Undo,
        },
        Binding {
            id: "redo",
            label: "Redo",
            detail: "Redo the last undone edit.",
            defaults: &[ch('y', true, false, false), ch('z', true, false, true)],
            action: || SemanticAction::Redo,
        },
        Binding {
            id: "select-all",
            label: "Select all",
            detail: "Select the whole buffer.",
            defaults: &[ch('a', true, false, false)],
            action: || SemanticAction::SelectAll,
        },
        Binding {
            id: "deselect",
            label: "Deselect",
            detail: "Clear the selection, even after the cursor moved away from it.",
            defaults: &[ch('d', true, false, false)],
            action: || SemanticAction::Deselect,
        },
        Binding {
            id: "find",
            label: "Find",
            detail: "Search in the focused file.",
            defaults: &[ch('f', true, false, false)],
            action: || SemanticAction::StartFind,
        },
        Binding {
            id: "replace",
            label: "Replace",
            detail: "Replace text in the focused file.",
            defaults: &[ch('h', true, false, false)],
            action: || SemanticAction::StartReplace,
        },
        Binding {
            id: "find-next",
            label: "Find next",
            detail: "Jump to the next match.",
            defaults: &[f(3, false)],
            action: || SemanticAction::FindNext,
        },
        Binding {
            id: "find-previous",
            label: "Find previous",
            detail: "Jump to the previous match.",
            defaults: &[f(3, true)],
            action: || SemanticAction::FindPrevious,
        },
        Binding {
            id: "file-picker",
            label: "File picker",
            detail: "Open a file from the workspace.",
            defaults: &[ch('p', true, false, false)],
            action: || SemanticAction::OpenFilePicker,
        },
        Binding {
            id: "sidebar",
            label: "Sidebar",
            detail: "Show, hide, or focus the file tree.",
            defaults: &[ch('b', true, false, false)],
            action: || SemanticAction::ToggleSidebar,
        },
        Binding {
            id: "complete",
            label: "Complete",
            detail: "Open completions at the cursor.",
            defaults: &[ch(' ', true, false, false)],
            action: || SemanticAction::Complete,
        },
        Binding {
            id: "hover",
            label: "Hover",
            detail: "Show documentation for the symbol under the cursor.",
            defaults: &[f(2, false)],
            action: || SemanticAction::Hover,
        },
        Binding {
            id: "rename",
            label: "Rename",
            detail: "Rename the symbol under the cursor.",
            defaults: &[f(2, true)],
            action: || SemanticAction::StartRename,
        },
        Binding {
            id: "comment",
            label: "Comment",
            detail: "Add or remove a line comment.",
            defaults: &[f(4, false), ch('/', true, false, false)],
            action: || SemanticAction::ToggleComment,
        },
        Binding {
            id: "format",
            label: "Format",
            detail: "Ask the language server to format the file.",
            defaults: &[f(7, false)],
            action: || SemanticAction::Format,
        },
        Binding {
            id: "agent",
            label: "Assistant",
            detail: "Open or close the assistant panel.",
            defaults: &[f(6, false)],
            action: || SemanticAction::ToggleAgent,
        },
        Binding {
            id: "diagnostic-next",
            label: "Next diagnostic",
            detail: "Move to the next diagnostic in this file.",
            defaults: &[f(8, false)],
            action: || SemanticAction::GotoDiagnostic { next: true },
        },
        Binding {
            id: "diagnostic-previous",
            label: "Previous diagnostic",
            detail: "Move to the previous diagnostic in this file.",
            defaults: &[f(8, true)],
            action: || SemanticAction::GotoDiagnostic { next: false },
        },
        Binding {
            id: "code-action",
            label: "Code action",
            detail: "Show fixes and refactors at the cursor.",
            defaults: &[ch('.', true, false, false)],
            action: || SemanticAction::CodeAction,
        },
        Binding {
            id: "goto-definition",
            label: "Go to definition",
            detail: "Jump to where the symbol is defined.",
            defaults: &[f(12, false)],
            action: || SemanticAction::GotoDefinition,
        },
        Binding {
            id: "goto-references",
            label: "Find references",
            detail: "List every place the symbol under the cursor is used.",
            defaults: &[f(12, true)],
            action: || SemanticAction::GotoReferences,
        },
        Binding {
            id: "match-bracket",
            label: "Match bracket",
            detail: "Jump to the bracket that pairs with this one.",
            defaults: &[ch(']', true, false, false)],
            action: || SemanticAction::MatchBracket { extend: false },
        },
        Binding {
            id: "select-bracket",
            label: "Select to bracket",
            detail: "Extend the selection to the matching bracket.",
            defaults: &[ch(']', true, false, true)],
            action: || SemanticAction::MatchBracket { extend: true },
        },
        Binding {
            id: "jump-back",
            label: "Jump back",
            detail: "Return to the place from before the last jump.",
            defaults: &[ch('o', true, false, false)],
            action: || SemanticAction::JumpBack,
        },
        Binding {
            id: "jump-forward",
            label: "Jump forward",
            detail: "Go forward again after jump back.",
            defaults: &[ch('o', true, false, true)],
            action: || SemanticAction::JumpForward,
        },
        Binding {
            id: "move-line-up",
            label: "Move line up",
            detail: "Move the current line, or the selected lines, up.",
            defaults: &[arrow(Kind::Up, false, true)],
            action: || SemanticAction::MoveLine { down: false },
        },
        Binding {
            id: "move-line-down",
            label: "Move line down",
            detail: "Move the current line, or the selected lines, down.",
            defaults: &[arrow(Kind::Down, false, true)],
            action: || SemanticAction::MoveLine { down: true },
        },
        Binding {
            id: "duplicate-line-up",
            label: "Duplicate line up",
            detail: "Copy the current line, or the selected lines, above.",
            defaults: &[arrow(Kind::Up, true, true)],
            action: || SemanticAction::DuplicateLine { down: false },
        },
        Binding {
            id: "duplicate-line-down",
            label: "Duplicate line down",
            detail: "Copy the current line, or the selected lines, below.",
            defaults: &[arrow(Kind::Down, true, true)],
            action: || SemanticAction::DuplicateLine { down: true },
        },
        Binding {
            id: "tab-previous",
            label: "Previous tab",
            detail: "Cycle to the previous tab in this window.",
            defaults: &[arrow(Kind::Left, true, true)],
            action: || SemanticAction::CycleTab { next: false },
        },
        Binding {
            id: "tab-next",
            label: "Next tab",
            detail: "Cycle to the next tab in this window.",
            defaults: &[arrow(Kind::Right, true, true)],
            action: || SemanticAction::CycleTab { next: true },
        },
        Binding {
            id: "convert-keyboard",
            label: "Convert keyboard",
            detail: "Retype the selection as the other keyboard layout, e.g. ouj8nv8ezbf → นี่คือคำผิด.",
            // On the Thai layout the backtick key types `_`, so Alt+` arrives as Alt+_.
            defaults: &[ch('`', false, true, false), ch('_', false, true, false)],
            action: || SemanticAction::ConvertKeyboard,
        },
        Binding {
            id: "focus-prefix",
            label: "Pane select mode",
            detail: "Enter PANE mode: arrows move a highlight between windows, Enter focuses it, Esc cancels.",
            defaults: &[ch('w', true, false, false)],
            action: || SemanticAction::FocusPrefix,
        },
    ];
    BINDINGS
}

impl Chord {
    const fn mods(kind: Kind, ctrl: bool, alt: bool, shift: bool) -> Self {
        Self {
            kind,
            ctrl,
            alt,
            shift,
        }
    }

    pub fn from_key(key: &KeyEvent) -> Option<Self> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let mut shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let kind = match key.code {
            KeyCode::Char(c) => {
                if c.is_ascii_uppercase() && (ctrl || alt) {
                    shift = true;
                }
                Kind::Char(c.to_ascii_lowercase())
            }
            KeyCode::F(n) => Kind::F(n),
            KeyCode::Up => Kind::Up,
            KeyCode::Down => Kind::Down,
            KeyCode::Left => Kind::Left,
            KeyCode::Right => Kind::Right,
            KeyCode::Home => Kind::Home,
            KeyCode::End => Kind::End,
            KeyCode::PageUp => Kind::PageUp,
            KeyCode::PageDown => Kind::PageDown,
            KeyCode::Tab | KeyCode::BackTab => Kind::Tab,
            KeyCode::Enter => Kind::Enter,
            KeyCode::Backspace => Kind::Backspace,
            KeyCode::Delete => Kind::Delete,
            KeyCode::Esc => Kind::Esc,
            _ => return None,
        };
        Some(Self {
            kind,
            ctrl,
            alt,
            shift,
        })
    }

    pub fn parse(token: &str) -> Option<Self> {
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let mut kind = None;
        for part in token.split('+') {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "alt" => alt = true,
                "shift" => shift = true,
                "space" => kind = Some(Kind::Char(' ')),
                "up" => kind = Some(Kind::Up),
                "down" => kind = Some(Kind::Down),
                "left" => kind = Some(Kind::Left),
                "right" => kind = Some(Kind::Right),
                "home" => kind = Some(Kind::Home),
                "end" => kind = Some(Kind::End),
                "pgup" | "pageup" => kind = Some(Kind::PageUp),
                "pgdn" | "pagedown" => kind = Some(Kind::PageDown),
                "tab" => kind = Some(Kind::Tab),
                "enter" => kind = Some(Kind::Enter),
                "backspace" => kind = Some(Kind::Backspace),
                "delete" | "del" => kind = Some(Kind::Delete),
                "esc" | "escape" => kind = Some(Kind::Esc),
                other if other.len() > 1 && other.starts_with('f') => {
                    let n: u8 = other[1..].parse().ok()?;
                    if n == 0 {
                        return None;
                    }
                    kind = Some(Kind::F(n));
                }
                other => {
                    let mut chars = other.chars();
                    let ch = chars.next()?;
                    if chars.next().is_some() {
                        return None;
                    }
                    kind = Some(Kind::Char(ch.to_ascii_lowercase()));
                }
            }
        }
        Some(Self {
            kind: kind?,
            ctrl,
            alt,
            shift,
        })
    }

    fn kind_token(self) -> &'static str {
        match self.kind {
            Kind::Char(' ') => "space",
            Kind::Char(_) => "",
            Kind::F(_) => "",
            Kind::Up => "up",
            Kind::Down => "down",
            Kind::Left => "left",
            Kind::Right => "right",
            Kind::Home => "home",
            Kind::End => "end",
            Kind::PageUp => "pgup",
            Kind::PageDown => "pgdn",
            Kind::Tab => "tab",
            Kind::Enter => "enter",
            Kind::Backspace => "backspace",
            Kind::Delete => "delete",
            Kind::Esc => "esc",
        }
    }

    fn kind_display(self) -> &'static str {
        match self.kind {
            Kind::Char(' ') => "Space",
            Kind::Char(_) => "",
            Kind::F(_) => "",
            Kind::Up => "Up",
            Kind::Down => "Down",
            Kind::Left => "Left",
            Kind::Right => "Right",
            Kind::Home => "Home",
            Kind::End => "End",
            Kind::PageUp => "PgUp",
            Kind::PageDown => "PgDn",
            Kind::Tab => "Tab",
            Kind::Enter => "Enter",
            Kind::Backspace => "Backspace",
            Kind::Delete => "Delete",
            Kind::Esc => "Esc",
        }
    }

    /// `F1` and `Ctrl+;` open the command bar. `Ctrl+Q` quits.
    pub fn is_reserved(self) -> bool {
        match self.kind {
            Kind::F(1) if !self.ctrl && !self.alt => true,
            Kind::Char(';' | ':') if self.ctrl && !self.alt => true,
            Kind::Char('q') if self.ctrl && !self.alt => true,
            _ => false,
        }
    }

    /// Keys that insert text, move the cursor, or are reserved.
    pub fn is_locked(self) -> bool {
        if self.is_reserved() {
            return true;
        }
        match self.kind {
            Kind::Char(' ') if self.shift && !self.ctrl && !self.alt => false,
            Kind::Char(_) if self.ctrl && self.alt => true,
            Kind::Char(_) if !self.ctrl && !self.alt => true,
            Kind::Char(_) => false,
            Kind::F(_) if self.ctrl || self.alt => true,
            Kind::F(_) => false,
            Kind::Up | Kind::Down if self.alt && !self.ctrl && !self.shift => false,
            Kind::Left | Kind::Right | Kind::Up | Kind::Down
                if self.ctrl && self.alt && !self.shift =>
            {
                false
            }
            Kind::Left
            | Kind::Right
            | Kind::Up
            | Kind::Down
            | Kind::Home
            | Kind::End
            | Kind::PageUp
            | Kind::PageDown
            | Kind::Tab
            | Kind::Enter
            | Kind::Backspace
            | Kind::Delete
            | Kind::Esc => true,
        }
    }

    pub fn reserved_reason(self) -> Option<&'static str> {
        match self.kind {
            Kind::F(1) if !self.ctrl && !self.alt => Some("F1 always opens the command bar"),
            Kind::Char(';' | ':') if self.ctrl && !self.alt => {
                Some("Ctrl+; always opens the command bar")
            }
            Kind::Char('q') if self.ctrl && !self.alt => Some("Ctrl+Q always quits"),
            _ => None,
        }
    }
}

impl Chord {
    pub fn token(self) -> String {
        self.render(false)
    }

    pub fn display(self) -> String {
        self.render(true)
    }

    fn render(self, display: bool) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push(if display { "Ctrl" } else { "ctrl" }.to_string());
        }
        if self.alt {
            parts.push(if display { "Alt" } else { "alt" }.to_string());
        }
        if self.shift {
            parts.push(if display { "Shift" } else { "shift" }.to_string());
        }
        match self.kind {
            Kind::Char(c) if c != ' ' => parts.push(if display {
                c.to_ascii_uppercase().to_string()
            } else {
                c.to_string()
            }),
            Kind::F(n) => parts.push(format!("{}{n}", if display { "F" } else { "f" })),
            _ => {
                let name = if display {
                    self.kind_display()
                } else {
                    self.kind_token()
                };
                if !name.is_empty() {
                    parts.push(name.to_string());
                }
            }
        }
        parts.join("+")
    }
}

impl KeyMap {
    pub fn resolve(overrides: &BTreeMap<String, String>) -> Self {
        let mut map = Self {
            by_chord: HashMap::new(),
            owner: HashMap::new(),
            shown: HashMap::new(),
        };
        for binding in bindings() {
            let custom = overrides
                .get(binding.id)
                .and_then(|token| Chord::parse(token));
            let chords: Vec<Chord> = if let Some(chord) = custom {
                vec![chord]
            } else {
                binding.defaults.to_vec()
            };
            if let Some(shown) = chords.first().copied() {
                map.shown.insert(binding.id, shown);
            }
            for chord in chords {
                map.by_chord.insert(chord, binding.action);
                map.owner.insert(chord, binding.id);
            }
        }
        map
    }

    pub fn defaults() -> Self {
        Self::resolve(&BTreeMap::new())
    }

    pub fn lookup(&self, chord: Chord) -> Option<SemanticAction> {
        self.by_chord.get(&chord).map(|action| action())
    }

    pub fn owner(&self, chord: Chord) -> Option<&'static str> {
        self.owner.get(&chord).copied()
    }

    pub fn label(id: &str) -> &'static str {
        bindings()
            .iter()
            .find(|binding| binding.id == id)
            .map(|binding| binding.label)
            .unwrap_or("command")
    }

    pub fn display_of(&self, id: &str) -> String {
        self.shown
            .get(id)
            .map(|chord| chord.display())
            .unwrap_or_default()
    }
}

/// The action for a reserved chord or a bound command chord.
pub fn dispatch(key: &KeyEvent, map: &KeyMap) -> Option<SemanticAction> {
    let chord = Chord::from_key(key)?;
    let find = |chord: Chord| {
        reserved_action(chord)
            .or_else(|| map.lookup(chord))
            .or_else(|| unshifted(chord).and_then(|chord| map.lookup(chord)))
    };
    find(chord).or_else(|| on_us_keys(chord).and_then(find))
}

/// `Ctrl + ห` (Thai layout) as `Ctrl + D`: the same physical key on the US layout.
/// The kitty protocol, `modifyOtherKeys`, and Windows report the character of the
/// active layout. Only with `Ctrl` or `Alt`, so typing is never a shortcut.
fn on_us_keys(chord: Chord) -> Option<Chord> {
    let Kind::Char(c) = chord.kind else {
        return None;
    };
    if !chord.ctrl && !chord.alt {
        return None;
    }
    let us = crate::keyboard_layout::us_key(c)?;
    Some(Chord {
        kind: Kind::Char(us.to_ascii_lowercase()),
        shift: chord.shift || us.is_ascii_uppercase(),
        ..chord
    })
}

/// `Ctrl + }` as `Ctrl + Shift + ]`, and so on for the US keyboard's shifted symbols.
/// The kitty protocol (with alternate keys) and xterm's `modifyOtherKeys` report the
/// shifted character, while bindings name the key under it. Tried only after an
/// exact match, so a binding such as `Alt + _` still wins.
fn unshifted(chord: Chord) -> Option<Chord> {
    const PAIRS: &[(char, char)] = &[
        ('~', '`'),
        ('!', '1'),
        ('@', '2'),
        ('#', '3'),
        ('$', '4'),
        ('%', '5'),
        ('^', '6'),
        ('&', '7'),
        ('*', '8'),
        ('(', '9'),
        (')', '0'),
        ('_', '-'),
        ('+', '='),
        ('{', '['),
        ('}', ']'),
        ('|', '\\'),
        (':', ';'),
        ('"', '\''),
        ('<', ','),
        ('>', '.'),
        ('?', '/'),
    ];
    let Kind::Char(c) = chord.kind else {
        return None;
    };
    if !chord.ctrl && !chord.alt {
        return None;
    }
    let base = PAIRS.iter().find(|(shifted, _)| *shifted == c)?.1;
    Some(Chord {
        kind: Kind::Char(base),
        shift: true,
        ..chord
    })
}

fn reserved_action(chord: Chord) -> Option<SemanticAction> {
    match chord.kind {
        Kind::F(1) if !chord.ctrl && !chord.alt => Some(SemanticAction::OpenCommandBar),
        Kind::Char(';' | ':') if chord.ctrl && !chord.alt => Some(SemanticAction::OpenCommandBar),
        Kind::Char('q') if chord.ctrl && !chord.alt => Some(SemanticAction::Quit),
        _ => None,
    }
}

/// Stores `token` for `id`, or drops the override when it is the primary default.
pub fn assign(
    overrides: &mut BTreeMap<String, String>,
    id: &str,
    token: &str,
) -> Result<String, AssignError> {
    let chord = Chord::parse(token).ok_or(AssignError::Blank)?;
    if chord.is_reserved() {
        return Err(AssignError::Reserved);
    }
    if chord.is_locked() {
        return Err(AssignError::Locked);
    }
    let binding = bindings()
        .iter()
        .find(|binding| binding.id == id)
        .ok_or(AssignError::Blank)?;
    let current = KeyMap::resolve(overrides);
    if let Some(owner) = current.owner(chord)
        && owner != id
    {
        return Err(AssignError::Conflict(KeyMap::label(owner)));
    }
    if binding.defaults.first().copied() == Some(chord) {
        overrides.remove(id);
    } else {
        overrides.insert(id.to_string(), chord.token());
    }
    Ok(chord.display())
}

/// Commands the sidebar already ran from a chord, so a rebound key still works there.
pub fn sidebar_accepts(action: &SemanticAction) -> bool {
    matches!(
        action,
        SemanticAction::OpenCommandBar
            | SemanticAction::Save
            | SemanticAction::Copy
            | SemanticAction::Cut
            | SemanticAction::Paste
            | SemanticAction::StartFind
            | SemanticAction::StartReplace
            | SemanticAction::OpenFilePicker
            | SemanticAction::ToggleSidebar
            | SemanticAction::FindNext
            | SemanticAction::FindPrevious
            | SemanticAction::Quit
            | SemanticAction::CycleTab { .. }
            | SemanticAction::FocusPrefix
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shifted_symbols_reach_the_shift_binding() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let map = KeyMap::defaults();
        let ctrl = KeyModifiers::CONTROL;
        // The kitty protocol reports Ctrl + Shift + ] as Ctrl + } (Shift folded in);
        // xterm's modifyOtherKeys keeps Shift.
        for modifiers in [ctrl, ctrl | KeyModifiers::SHIFT] {
            assert_eq!(
                dispatch(&KeyEvent::new(KeyCode::Char('}'), modifiers), &map),
                Some(SemanticAction::MatchBracket { extend: true })
            );
        }
        assert_eq!(
            dispatch(&KeyEvent::new(KeyCode::Char('Z'), ctrl), &map),
            Some(SemanticAction::Redo)
        );
        // An exact binding still wins over the shifted form.
        assert_eq!(
            dispatch(&KeyEvent::new(KeyCode::Char('_'), KeyModifiers::ALT), &map),
            Some(SemanticAction::ConvertKeyboard)
        );
        // Plain typing is never a shortcut.
        assert_eq!(
            dispatch(
                &KeyEvent::new(KeyCode::Char('}'), KeyModifiers::SHIFT),
                &map
            ),
            None
        );
    }

    #[test]
    fn shortcuts_work_with_the_thai_layout_active() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let map = KeyMap::defaults();
        let ctrl = KeyModifiers::CONTROL;
        let on = |c: char, modifiers| dispatch(&KeyEvent::new(KeyCode::Char(c), modifiers), &map);
        // Kedmanee: ห is the `s` key, ผ `z`, แ `c`, ั `y`, ย `p`.
        assert_eq!(on('ห', ctrl), Some(SemanticAction::Save));
        assert_eq!(on('ผ', ctrl), Some(SemanticAction::Undo));
        assert_eq!(on('แ', ctrl), Some(SemanticAction::Copy));
        assert_eq!(on('ั', ctrl), Some(SemanticAction::Redo));
        assert_eq!(on('ย', ctrl), Some(SemanticAction::OpenFilePicker));
        assert_eq!(on('ว', ctrl), Some(SemanticAction::OpenCommandBar));
        assert_eq!(on('ๆ', ctrl), Some(SemanticAction::Quit));
        // Typing Thai is never a shortcut.
        assert_eq!(on('ห', KeyModifiers::NONE), None);
        assert_eq!(on('ฆ', KeyModifiers::SHIFT), None);
    }

    #[test]
    fn default_chords_do_not_overlap_and_round_trip() {
        let map = KeyMap::defaults();
        let mut seen = HashMap::new();
        for binding in bindings() {
            for chord in binding.defaults {
                let previous = seen.insert(*chord, binding.id);
                assert!(
                    previous.is_none(),
                    "{} and {} share {}",
                    previous.unwrap_or(""),
                    binding.id,
                    chord.token()
                );
                assert!(!chord.is_locked(), "{}", chord.token());
                let parsed = Chord::parse(&chord.token()).unwrap();
                assert_eq!(parsed, *chord);
                assert_eq!(map.owner(*chord), Some(binding.id));
            }
        }
    }

    #[test]
    fn assign_rejects_locked_reserved_and_conflicts() {
        let mut overrides = BTreeMap::new();
        assert_eq!(
            assign(&mut overrides, "save", "ctrl+s"),
            Ok("Ctrl+S".to_string())
        );
        assert!(overrides.is_empty(), "primary default is not stored");
        assert_eq!(
            assign(&mut overrides, "save", "ctrl+shift+s").unwrap(),
            "Ctrl+Shift+S"
        );
        assert_eq!(
            overrides.get("save").map(String::as_str),
            Some("ctrl+shift+s")
        );
        assert_eq!(
            assign(&mut overrides, "copy", "a"),
            Err(AssignError::Locked)
        );
        assert_eq!(
            assign(&mut overrides, "copy", "f1"),
            Err(AssignError::Reserved)
        );
        assert_eq!(
            assign(&mut overrides, "copy", "ctrl+shift+s"),
            Err(AssignError::Conflict("Save"))
        );
        assert!(assign(&mut overrides, "save", "ctrl+s").is_ok());
        assert!(overrides.is_empty());
    }
}
