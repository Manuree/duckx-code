use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::action::{
    AgentAction, CommandBarAction, CompletionAction, FindAction, Focus, PaneDirection,
    PaneSelectAction, PickerAction, PromptAction, ReplaceAction, SemanticAction, SettingsAction,
    SidebarAction,
};
use crate::buffer::Movement;

pub fn route(event: &Event, focus: Focus, selecting: bool) -> Option<SemanticAction> {
    let mut burst = crate::agent::PasteBurst::default();
    route_flags(
        event,
        focus,
        selecting,
        false,
        &crate::keys::KeyMap::defaults(),
        false,
        false,
        &mut burst,
    )
}

/// `filtering` is the explorer FILTER state. `Space` and `Shift` mark files there,
/// the same way they do in FILE mode. `capturing` is the settings page waiting for a chord.
pub fn route_flags(
    event: &Event,
    focus: Focus,
    selecting: bool,
    filtering: bool,
    keys: &crate::keys::KeyMap,
    capturing: bool,
    drafting: bool,
    burst: &mut crate::agent::PasteBurst,
) -> Option<SemanticAction> {
    match focus {
        Focus::Editor => route_editor(event, keys),
        Focus::Sidebar => route_sidebar(event, selecting, filtering, keys),
        Focus::Prompt => route_prompt(event),
        Focus::Settings => route_settings(event, capturing, drafting),
        Focus::Agent => route_agent(event, keys, burst),
        Focus::About => route_about(event),
        Focus::PreviewSide => route_preview_side(event),
        Focus::LayoutMenu => route_layout_menu(event, keys),
        Focus::Find => route_find(event),
        Focus::Replace => route_replace(event),
        Focus::FilePicker => route_picker(event),
        Focus::CompletionPopup | Focus::ActionMenu => route_completion(event, keys),
        Focus::CommandBar => match route_command_bar(event)? {
            CommandBarAction::Insert(text) if text.is_empty() => None,
            action => Some(SemanticAction::CommandBar(action)),
        },
    }
}

/// Navigation keys drive the tree; printable keys type into the filter box.
/// In FILE mode, `Space` marks files, `Shift+Space` opens the Command Bar, and `Esc` returns to NORMAL.
fn route_sidebar(
    event: &Event,
    selecting: bool,
    filtering: bool,
    keys: &crate::keys::KeyMap,
) -> Option<SemanticAction> {
    let key = match event {
        Event::Paste(text) => {
            let text: String = text.chars().filter(|c| !c.is_control()).collect();
            return (!text.is_empty())
                .then_some(SemanticAction::Sidebar(SidebarAction::FilterInsert(text)));
        }
        Event::Key(key) if key.kind != KeyEventKind::Release => key,
        _ => return None,
    };
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let marking = selecting || filtering;
    let extend = shift && marking;

    if let Some(action) = crate::keys::dispatch(key, keys)
        && crate::keys::sidebar_accepts(&action)
    {
        return Some(action);
    }
    if ctrl && !alt && matches!(key.code, KeyCode::Char(_)) {
        return None;
    }
    if alt && !ctrl && !shift {
        match key.code {
            KeyCode::Right => return Some(SemanticAction::ResizeSidebar { wider: true }),
            KeyCode::Left => return Some(SemanticAction::ResizeSidebar { wider: false }),
            _ => {}
        }
    }

    let action = match key.code {
        KeyCode::Up => SidebarAction::Up { extend },
        KeyCode::Down => SidebarAction::Down { extend },
        KeyCode::PageUp => SidebarAction::PageUp { extend },
        KeyCode::PageDown => SidebarAction::PageDown { extend },
        KeyCode::Home => SidebarAction::First { extend },
        KeyCode::End => SidebarAction::Last { extend },
        KeyCode::Right => SidebarAction::Expand { extend },
        KeyCode::Left => SidebarAction::Collapse { extend },
        KeyCode::Enter => SidebarAction::Activate,
        KeyCode::Backspace => SidebarAction::FilterBackspace,
        KeyCode::Esc if selecting => SidebarAction::ExitSelect,
        KeyCode::Esc if filtering => SidebarAction::Escape,
        KeyCode::Esc => SidebarAction::Escape,
        KeyCode::Char(' ') if marking && !ctrl && !alt && !shift => SidebarAction::Mark,
        KeyCode::Char(c) if !alt || ctrl => SidebarAction::FilterInsert(c.to_string()),
        _ => return None,
    };
    Some(SemanticAction::Sidebar(action))
}

/// FIND mode. Letters search immediately. `Up` / `Down` move between matches. `Esc` leaves.
fn route_find(event: &Event) -> Option<SemanticAction> {
    let key = match event {
        Event::Paste(text) => {
            let text: String = text.chars().filter(|c| !c.is_control()).collect();
            return (!text.is_empty()).then_some(SemanticAction::Find(FindAction::Insert(text)));
        }
        Event::Key(key) if key.kind != KeyEventKind::Release => key,
        _ => return None,
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    if ctrl && !alt {
        if let KeyCode::Char(c) = key.code {
            match c.to_ascii_lowercase() {
                'q' => return Some(SemanticAction::Quit),
                'p' => return Some(SemanticAction::OpenFilePicker),
                'b' => return Some(SemanticAction::Find(FindAction::Files)),
                _ => {}
            }
        }
    }
    let action = match key.code {
        KeyCode::Esc => FindAction::Close,
        KeyCode::Enter => FindAction::Submit,
        KeyCode::Up => FindAction::Prev,
        KeyCode::Down => FindAction::Next,
        KeyCode::F(3) if shift => FindAction::Prev,
        KeyCode::F(3) => FindAction::Next,
        KeyCode::Backspace => FindAction::Backspace,
        KeyCode::Char(c) if !ctrl && !alt || ctrl && alt => FindAction::Insert(c.to_string()),
        _ => return None,
    };
    Some(SemanticAction::Find(action))
}

/// REPLACE mode. Letters edit the new text. `Down` replaces the current match.
fn route_replace(event: &Event) -> Option<SemanticAction> {
    let key = match event {
        Event::Paste(text) => {
            let text: String = text.chars().filter(|c| !c.is_control()).collect();
            return (!text.is_empty())
                .then_some(SemanticAction::Replace(ReplaceAction::Insert(text)));
        }
        Event::Key(key) if key.kind != KeyEventKind::Release => key,
        _ => return None,
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    if ctrl && !alt && matches!(key.code, KeyCode::Char('q' | 'Q')) {
        return Some(SemanticAction::Quit);
    }
    if ctrl && !alt && matches!(key.code, KeyCode::Char('p' | 'P')) {
        return Some(SemanticAction::OpenFilePicker);
    }
    let action = match key.code {
        KeyCode::Esc => ReplaceAction::Close,
        KeyCode::Enter => ReplaceAction::Submit,
        KeyCode::Up => ReplaceAction::Undo,
        KeyCode::Down if shift && !ctrl && !alt => ReplaceAction::All,
        KeyCode::Down if !ctrl && !alt => ReplaceAction::Next,
        KeyCode::Backspace => ReplaceAction::Backspace,
        KeyCode::Char(c) if !ctrl && !alt || ctrl && alt => ReplaceAction::Insert(c.to_string()),
        _ => return None,
    };
    Some(SemanticAction::Replace(action))
}

fn route_command_bar(event: &Event) -> Option<CommandBarAction> {
    use crate::command::LineMove;
    let key = match event {
        Event::Paste(text) => {
            return Some(CommandBarAction::Insert(
                text.chars().filter(|c| !c.is_control()).collect(),
            ));
        }
        Event::Key(key) if key.kind != KeyEventKind::Release => key,
        _ => return None,
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    match key.code {
        KeyCode::Esc | KeyCode::F(1) => Some(CommandBarAction::Close),
        KeyCode::Enter => Some(CommandBarAction::Submit),
        KeyCode::Up | KeyCode::BackTab => Some(CommandBarAction::SelectPrev),
        KeyCode::Down => Some(CommandBarAction::SelectNext),
        KeyCode::Tab if !ctrl && !alt && !shift => Some(CommandBarAction::ToChat),
        KeyCode::Right if !ctrl && !alt && !shift => Some(CommandBarAction::AcceptHint),
        KeyCode::Right if ctrl && !alt && !shift => {
            Some(CommandBarAction::Cursor(LineMove::WordRight))
        }
        KeyCode::Left if !ctrl && !alt && !shift => Some(CommandBarAction::Cursor(LineMove::Left)),
        KeyCode::Left if ctrl && !alt && !shift => {
            Some(CommandBarAction::Cursor(LineMove::WordLeft))
        }
        KeyCode::Home if !shift => Some(CommandBarAction::Cursor(LineMove::Home)),
        KeyCode::End if !shift => Some(CommandBarAction::Cursor(LineMove::End)),
        KeyCode::Char('a') if ctrl && !alt => Some(CommandBarAction::Cursor(LineMove::Home)),
        KeyCode::Char('e') if ctrl && !alt => Some(CommandBarAction::Cursor(LineMove::End)),
        KeyCode::Backspace if ctrl || alt => Some(CommandBarAction::DeleteWordBack),
        KeyCode::Backspace => Some(CommandBarAction::Backspace),
        KeyCode::Delete if !ctrl && !alt => Some(CommandBarAction::DeleteForward),
        KeyCode::Char(c) if !ctrl && !alt || ctrl && alt => {
            Some(CommandBarAction::Insert(c.to_string()))
        }
        _ => None,
    }
}

fn route_picker(event: &Event) -> Option<SemanticAction> {
    let key = match event {
        Event::Paste(text) => {
            let text: String = text.chars().filter(|ch| !ch.is_control()).collect();
            return (!text.is_empty())
                .then_some(SemanticAction::Picker(PickerAction::Insert(text)));
        }
        Event::Key(key) if key.kind != KeyEventKind::Release => key,
        _ => return None,
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    if ctrl && !alt && matches!(key.code, KeyCode::Char('q' | 'Q')) {
        return Some(SemanticAction::Quit);
    }
    if key.code == KeyCode::F(1)
        || (ctrl && !alt && matches!(key.code, KeyCode::Char(';' | ':')))
        || (shift && !ctrl && !alt && matches!(key.code, KeyCode::Char(' ')))
    {
        return Some(SemanticAction::OpenCommandBar);
    }
    let action = match key.code {
        KeyCode::Esc => PickerAction::Close,
        KeyCode::Enter => PickerAction::Submit,
        KeyCode::Up => PickerAction::SelectPrev,
        KeyCode::Down => PickerAction::SelectNext,
        KeyCode::PageUp => PickerAction::PageUp,
        KeyCode::PageDown => PickerAction::PageDown,
        KeyCode::Home if ctrl => PickerAction::First,
        KeyCode::End if ctrl => PickerAction::Last,
        KeyCode::Home => PickerAction::CursorHome,
        KeyCode::End => PickerAction::CursorEnd,
        KeyCode::Left if ctrl => PickerAction::CursorWordLeft,
        KeyCode::Right if ctrl => PickerAction::CursorWordRight,
        KeyCode::Left => PickerAction::CursorLeft,
        KeyCode::Right => PickerAction::CursorRight,
        KeyCode::Backspace if ctrl => PickerAction::DeleteWordBack,
        KeyCode::Backspace => PickerAction::Backspace,
        KeyCode::Delete => PickerAction::DeleteForward,
        KeyCode::Char('w') if ctrl && !alt => PickerAction::DeleteWordBack,
        KeyCode::Char(c) if !ctrl && !alt || ctrl && alt => PickerAction::Insert(c.to_string()),
        _ => return None,
    };
    Some(SemanticAction::Picker(action))
}

/// Arrows pick a side at once. `Enter` takes the highlighted side, `Tab` moves it,
/// and `Esc` cancels. `u` / `d` / `l` / `r` mirror the arrows.
/// The `convert-keyboard` layout list. `Up` / `Down` (or `Tab`) move, `Enter`
/// or the convert key again converts, `Esc` closes. Other keys, paste included,
/// are swallowed so nothing is typed behind the list.
fn route_layout_menu(event: &Event, keys: &crate::keys::KeyMap) -> Option<SemanticAction> {
    let Event::Key(key) = event else {
        return None;
    };
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if crate::keys::dispatch(key, keys) == Some(SemanticAction::ConvertKeyboard) {
        return Some(SemanticAction::LayoutMenu(CompletionAction::Accept));
    }
    let plain = !key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    let action = match key.code {
        KeyCode::Up | KeyCode::BackTab if plain => CompletionAction::Prev,
        KeyCode::Tab if plain && key.modifiers.contains(KeyModifiers::SHIFT) => {
            CompletionAction::Prev
        }
        KeyCode::Down | KeyCode::Tab if plain => CompletionAction::Next,
        KeyCode::Enter if plain => CompletionAction::Accept,
        KeyCode::Esc => CompletionAction::Close,
        _ => return None,
    };
    Some(SemanticAction::LayoutMenu(action))
}

fn route_preview_side(event: &Event) -> Option<SemanticAction> {
    use crate::action::{PreviewSideAction, Side};
    let key = match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => key,
        _ => return None,
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    if ctrl && !alt && matches!(key.code, KeyCode::Char('q' | 'Q')) {
        return Some(SemanticAction::Quit);
    }
    if ctrl || alt {
        return None;
    }
    let action = match key.code {
        KeyCode::Up | KeyCode::Char('u' | 'U') => PreviewSideAction::Choose(Side::Up),
        KeyCode::Down | KeyCode::Char('d' | 'D') => PreviewSideAction::Choose(Side::Down),
        KeyCode::Left | KeyCode::Char('l' | 'L') => PreviewSideAction::Choose(Side::Left),
        KeyCode::Right | KeyCode::Char('r' | 'R') => PreviewSideAction::Choose(Side::Right),
        KeyCode::Enter => PreviewSideAction::Confirm,
        KeyCode::Tab => PreviewSideAction::Next,
        KeyCode::BackTab => PreviewSideAction::Prev,
        KeyCode::Esc => PreviewSideAction::Cancel,
        _ => return None,
    };
    Some(SemanticAction::PreviewSide(action))
}

fn route_about(event: &Event) -> Option<SemanticAction> {
    let key = match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => key,
        _ => return None,
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    if ctrl && !alt && matches!(key.code, KeyCode::Char('q' | 'Q')) {
        return Some(SemanticAction::Quit);
    }
    if ctrl || alt {
        return None;
    }
    match key.code {
        KeyCode::Esc | KeyCode::F(1) | KeyCode::Backspace => Some(SemanticAction::CloseAbout),
        _ => None,
    }
}

fn route_settings(event: &Event, capturing: bool, drafting: bool) -> Option<SemanticAction> {
    if drafting && !capturing {
        return route_settings_text(event);
    }
    let key = match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => key,
        _ => return None,
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    if ctrl && !alt && matches!(key.code, KeyCode::Char('q' | 'Q')) {
        return Some(SemanticAction::Quit);
    }
    if capturing {
        if matches!(
            key.code,
            KeyCode::Esc | KeyCode::F(1) | KeyCode::Left | KeyCode::Backspace
        ) && !ctrl
            && !alt
        {
            return Some(SemanticAction::Settings(SettingsAction::Back));
        }
        if matches!(key.code, KeyCode::Delete) && !ctrl && !alt && !shift {
            return Some(SemanticAction::Settings(SettingsAction::Reset));
        }
        if let Some(chord) = crate::keys::Chord::from_key(key) {
            return Some(SemanticAction::Settings(SettingsAction::Assign(
                chord.token(),
            )));
        }
        return None;
    }
    if matches!(key.code, KeyCode::Delete) && !ctrl && !alt && !shift {
        return Some(SemanticAction::Settings(SettingsAction::Reset));
    }
    let action = match key.code {
        KeyCode::Esc | KeyCode::F(1) | KeyCode::Left | KeyCode::Backspace => SettingsAction::Back,
        KeyCode::Up => SettingsAction::Up,
        KeyCode::Down => SettingsAction::Down,
        KeyCode::Enter | KeyCode::Right => SettingsAction::Enter,
        KeyCode::Home | KeyCode::PageUp => SettingsAction::First,
        KeyCode::End | KeyCode::PageDown => SettingsAction::Last,
        KeyCode::Char('r' | 'R') if !ctrl && !alt => SettingsAction::Restart,
        KeyCode::Char('e' | 'E') if !ctrl && !alt => SettingsAction::Edit,
        _ => return None,
    };
    Some(SemanticAction::Settings(action))
}

/// A text row is being typed. Printable keys edit that value at the cursor.
fn route_settings_text(event: &Event) -> Option<SemanticAction> {
    use crate::command::LineMove;
    let text = match event {
        Event::Paste(text) => text.clone(),
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let alt = key.modifiers.contains(KeyModifiers::ALT);
            let shift = key.modifiers.contains(KeyModifiers::SHIFT);
            if ctrl && !alt && matches!(key.code, KeyCode::Char('q' | 'Q')) {
                return Some(SemanticAction::Quit);
            }
            let edit = |action| Some(SemanticAction::Settings(action));
            // The same line keys as the command bar.
            return match key.code {
                KeyCode::Esc | KeyCode::F(1) => edit(SettingsAction::Back),
                KeyCode::Enter => edit(SettingsAction::Enter),
                KeyCode::Backspace if ctrl || alt => edit(SettingsAction::DeleteWordBack),
                KeyCode::Backspace => edit(SettingsAction::DeleteChar),
                KeyCode::Delete if !ctrl && !alt => edit(SettingsAction::DeleteForward),
                KeyCode::Left if ctrl && !alt && !shift => {
                    edit(SettingsAction::Cursor(LineMove::WordLeft))
                }
                KeyCode::Right if ctrl && !alt && !shift => {
                    edit(SettingsAction::Cursor(LineMove::WordRight))
                }
                KeyCode::Left if !ctrl && !alt && !shift => {
                    edit(SettingsAction::Cursor(LineMove::Left))
                }
                KeyCode::Right if !ctrl && !alt && !shift => {
                    edit(SettingsAction::Cursor(LineMove::Right))
                }
                KeyCode::Home if !shift => edit(SettingsAction::Cursor(LineMove::Home)),
                KeyCode::End if !shift => edit(SettingsAction::Cursor(LineMove::End)),
                KeyCode::Char('a') if ctrl && !alt => edit(SettingsAction::Cursor(LineMove::Home)),
                KeyCode::Char('e') if ctrl && !alt => edit(SettingsAction::Cursor(LineMove::End)),
                KeyCode::Up => edit(SettingsAction::Up),
                KeyCode::Down => edit(SettingsAction::Down),
                KeyCode::Char(c) if !ctrl && !alt || ctrl && alt => {
                    edit(SettingsAction::Insert(c.to_string()))
                }
                _ => None,
            };
        }
        _ => return None,
    };
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    (!text.is_empty()).then_some(SemanticAction::Settings(SettingsAction::Insert(text)))
}

fn route_agent(
    event: &Event,
    keys: &crate::keys::KeyMap,
    burst: &mut crate::agent::PasteBurst,
) -> Option<SemanticAction> {
    let key = match event {
        Event::Paste(text) => {
            burst.reset();
            let text = crate::agent::normalize_paste(text);
            return (!text.is_empty()).then_some(SemanticAction::Agent(AgentAction::Insert(text)));
        }
        Event::Key(key) if key.kind != KeyEventKind::Release => key,
        _ => return None,
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    // Ctrl+J inserts a newline (fallback when Shift+Enter is invisible).
    if ctrl && !alt && matches!(key.code, KeyCode::Char('j') | KeyCode::Char('J')) {
        burst.note();
        return Some(SemanticAction::Agent(AgentAction::Insert("\n".into())));
    }
    // Alt+Enter inserts a newline (another fallback for terminals without Shift+Enter).
    if alt && !ctrl && matches!(key.code, KeyCode::Enter) {
        burst.note();
        return Some(SemanticAction::Agent(AgentAction::Insert("\n".into())));
    }
    // Alt+Up / Alt+Down change the panel height. In the editor those keys move a line.
    if alt && !ctrl && !shift {
        match key.code {
            KeyCode::Up => return Some(SemanticAction::ResizeAgent { taller: true }),
            KeyCode::Down => return Some(SemanticAction::ResizeAgent { taller: false }),
            _ => {}
        }
    }
    if ctrl && !alt {
        match key.code {
            KeyCode::Left | KeyCode::Right => {
                return Some(SemanticAction::Agent(AgentAction::Word {
                    right: key.code == KeyCode::Right,
                    extend: shift,
                }));
            }
            KeyCode::Up => return Some(SemanticAction::Agent(AgentAction::Above)),
            KeyCode::Down => return Some(SemanticAction::Agent(AgentAction::Below)),
            KeyCode::Char('l') | KeyCode::Char('L') => {
                return Some(SemanticAction::Agent(AgentAction::NewChat));
            }
            _ => {}
        }
    }
    if let Some(action) = crate::keys::dispatch(key, keys) {
        return Some(action);
    }
    if ctrl || alt {
        return None;
    }
    // Track printable / navigation keys for the conhost paste-burst heuristic.
    let tracks_burst = matches!(
        key.code,
        KeyCode::Char(_) | KeyCode::Enter | KeyCode::Backspace | KeyCode::Delete | KeyCode::Tab
    );
    if tracks_burst {
        burst.note();
    } else {
        burst.reset();
    }
    match key.code {
        KeyCode::Tab => Some(SemanticAction::Agent(AgentAction::Complete)),
        KeyCode::Esc => Some(SemanticAction::Agent(AgentAction::Close)),
        KeyCode::Enter if shift || burst.enter_as_newline() => {
            Some(SemanticAction::Agent(AgentAction::Insert("\n".into())))
        }
        KeyCode::Enter => Some(SemanticAction::Agent(AgentAction::Submit)),
        KeyCode::Left => Some(SemanticAction::Agent(AgentAction::Move {
            right: false,
            extend: shift,
        })),
        KeyCode::Right => Some(SemanticAction::Agent(AgentAction::Move {
            right: true,
            extend: shift,
        })),
        KeyCode::Home => Some(SemanticAction::Agent(AgentAction::Jump {
            end: false,
            extend: shift,
        })),
        KeyCode::End => Some(SemanticAction::Agent(AgentAction::Jump {
            end: true,
            extend: shift,
        })),
        KeyCode::Up => Some(SemanticAction::Agent(AgentAction::Line {
            down: false,
            extend: shift,
        })),
        KeyCode::Down => Some(SemanticAction::Agent(AgentAction::Line {
            down: true,
            extend: shift,
        })),
        KeyCode::Backspace => Some(SemanticAction::Agent(AgentAction::Backspace)),
        KeyCode::Delete => Some(SemanticAction::Agent(AgentAction::Delete)),
        KeyCode::Char(c) => Some(SemanticAction::Agent(AgentAction::Insert(c.to_string()))),
        _ => None,
    }
}

fn route_prompt(event: &Event) -> Option<SemanticAction> {
    let key = match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => key,
        _ => return None,
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let action = match key.code {
        KeyCode::Esc => PromptAction::Cancel,
        KeyCode::Enter => PromptAction::Confirm,
        KeyCode::Left | KeyCode::Up => PromptAction::Prev,
        KeyCode::Right | KeyCode::Down => PromptAction::Next,
        KeyCode::Char(c) if !ctrl && !alt => PromptAction::Accelerator(c),
        _ => return None,
    };
    Some(SemanticAction::Prompt(action))
}

fn route_completion(event: &Event, keys: &crate::keys::KeyMap) -> Option<SemanticAction> {
    let Event::Key(key) = event else {
        return route_editor(event, keys);
    };
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Up if !ctrl && !alt => Some(SemanticAction::Completion(CompletionAction::Prev)),
        KeyCode::Down if !ctrl && !alt => Some(SemanticAction::Completion(CompletionAction::Next)),
        KeyCode::Tab | KeyCode::Enter if !ctrl && !alt => {
            Some(SemanticAction::Completion(CompletionAction::Accept))
        }
        KeyCode::Esc => Some(SemanticAction::Completion(CompletionAction::Close)),
        _ => route_editor(event, keys),
    }
}

/// Keys while PANE mode is on. Arrows (with or without `Ctrl` held) move the
/// candidate, `Enter` or the mode key again (`Ctrl+W`) confirms, `Esc` cancels.
/// Everything else, paste included, is swallowed so nothing is typed.
pub fn route_pane_select(event: &Event, keys: &crate::keys::KeyMap) -> Option<SemanticAction> {
    let Event::Key(key) = event else {
        return None;
    };
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let action = match key.code {
        KeyCode::Left => PaneSelectAction::Move(PaneDirection::Left),
        KeyCode::Right => PaneSelectAction::Move(PaneDirection::Right),
        KeyCode::Up => PaneSelectAction::Move(PaneDirection::Up),
        KeyCode::Down => PaneSelectAction::Move(PaneDirection::Down),
        KeyCode::Enter => PaneSelectAction::Confirm,
        KeyCode::Esc => PaneSelectAction::Cancel,
        _ if crate::keys::dispatch(key, keys) == Some(SemanticAction::FocusPrefix) => {
            PaneSelectAction::Confirm
        }
        _ => return None,
    };
    Some(SemanticAction::PaneSelect(action))
}

fn route_editor(event: &Event, keys: &crate::keys::KeyMap) -> Option<SemanticAction> {
    match event {
        Event::Key(key) => route_editor_key(key, keys),
        Event::Paste(text) => Some(SemanticAction::InsertText(text.clone())),
        _ => None,
    }
}

fn route_editor_key(key: &KeyEvent, keys: &crate::keys::KeyMap) -> Option<SemanticAction> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl_arrow = ctrl
        && !alt
        && matches!(
            key.code,
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
        );
    if !ctrl_arrow {
        if let Some(action) = crate::keys::dispatch(key, keys) {
            return Some(action);
        }
    }
    let movement = |movement| {
        Some(SemanticAction::Move {
            movement,
            extend: shift,
        })
    };

    match key.code {
        KeyCode::Up if ctrl && !alt => movement(Movement::ParagraphUp),
        KeyCode::Down if ctrl && !alt => movement(Movement::ParagraphDown),
        KeyCode::Left if ctrl != alt => movement(Movement::WordLeft),
        KeyCode::Left => movement(Movement::Left),
        KeyCode::Right if ctrl != alt => movement(Movement::WordRight),
        KeyCode::Right => movement(Movement::Right),
        KeyCode::Up => movement(Movement::Up),
        KeyCode::Down => movement(Movement::Down),
        KeyCode::Home if ctrl => movement(Movement::DocumentStart),
        KeyCode::Home => movement(Movement::LineStart),
        KeyCode::End if ctrl => movement(Movement::DocumentEnd),
        KeyCode::End => movement(Movement::LineEnd),
        KeyCode::PageUp => Some(SemanticAction::PageUp { extend: shift }),
        KeyCode::PageDown => Some(SemanticAction::PageDown { extend: shift }),
        KeyCode::Enter => Some(SemanticAction::InsertNewline),
        KeyCode::Backspace => Some(SemanticAction::DeleteBackward),
        KeyCode::Delete if shift && !ctrl && !alt => Some(SemanticAction::DeleteLine),
        KeyCode::Delete => Some(SemanticAction::DeleteForward),
        KeyCode::Esc if !ctrl && !alt => Some(SemanticAction::Dismiss),
        KeyCode::BackTab if !ctrl && !alt => Some(SemanticAction::Outdent),
        KeyCode::Tab if shift && !ctrl && !alt => Some(SemanticAction::Outdent),
        KeyCode::Tab if !ctrl && !alt => Some(SemanticAction::Indent),
        KeyCode::Char(c) if ctrl && alt => Some(SemanticAction::InsertChar(c)),
        KeyCode::Char(c) if !ctrl && !alt => Some(SemanticAction::InsertChar(c)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::LineMove;
    use crossterm::event::KeyEventState;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    fn editor(event: Event) -> Option<SemanticAction> {
        route(&event, Focus::Editor, false)
    }

    #[test]
    fn alt_backtick_converts_the_keyboard_and_the_list_swallows_typing() {
        let convert = Some(SemanticAction::ConvertKeyboard);
        assert_eq!(editor(key(KeyCode::Char('`'), KeyModifiers::ALT)), convert);
        // The same key on the Thai layout types `_`.
        assert_eq!(editor(key(KeyCode::Char('_'), KeyModifiers::ALT)), convert);
        assert_eq!(
            route(
                &key(KeyCode::Char('`'), KeyModifiers::ALT),
                Focus::Agent,
                false
            ),
            convert
        );
        assert_eq!(
            editor(key(KeyCode::Char('`'), KeyModifiers::NONE)),
            Some(SemanticAction::InsertChar('`')),
            "a plain backtick still types"
        );
        let menu = |event| route(&event, Focus::LayoutMenu, false);
        assert_eq!(
            menu(key(KeyCode::Down, KeyModifiers::NONE)),
            Some(SemanticAction::LayoutMenu(CompletionAction::Next))
        );
        assert_eq!(
            menu(key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(SemanticAction::LayoutMenu(CompletionAction::Accept))
        );
        assert_eq!(
            menu(key(KeyCode::Char('`'), KeyModifiers::ALT)),
            Some(SemanticAction::LayoutMenu(CompletionAction::Accept))
        );
        assert_eq!(
            menu(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(SemanticAction::LayoutMenu(CompletionAction::Close))
        );
        assert_eq!(menu(key(KeyCode::Char('a'), KeyModifiers::NONE)), None);
        assert_eq!(menu(Event::Paste("x".into())), None);
    }

    #[test]
    fn ctrl_p_opens_the_file_picker_and_letters_type_the_query() {
        let ctrl_p = key(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert_eq!(editor(ctrl_p.clone()), Some(SemanticAction::OpenFilePicker));
        let picker = |event| route(&event, Focus::FilePicker, false);
        assert_eq!(
            picker(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some(SemanticAction::Picker(PickerAction::Insert("a".into())))
        );
        assert_eq!(
            picker(key(KeyCode::Down, KeyModifiers::SHIFT)),
            Some(SemanticAction::Picker(PickerAction::SelectNext))
        );
        assert_eq!(
            picker(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(SemanticAction::Picker(PickerAction::Close))
        );
    }

    #[test]
    fn shift_turns_navigation_into_selection() {
        assert_eq!(
            editor(key(KeyCode::Right, KeyModifiers::SHIFT)),
            Some(SemanticAction::Move {
                movement: Movement::Right,
                extend: true
            })
        );
        assert_eq!(
            editor(key(KeyCode::Left, KeyModifiers::ALT | KeyModifiers::SHIFT)),
            Some(SemanticAction::Move {
                movement: Movement::WordLeft,
                extend: true
            })
        );
        assert_eq!(
            editor(key(KeyCode::PageDown, KeyModifiers::SHIFT)),
            Some(SemanticAction::PageDown { extend: true })
        );
    }

    #[test]
    fn ctrl_bracket_jumps_to_the_match_and_shift_extends() {
        assert_eq!(
            editor(key(KeyCode::Char(']'), KeyModifiers::CONTROL)),
            Some(SemanticAction::MatchBracket { extend: false })
        );
        assert_eq!(
            editor(key(
                KeyCode::Char(']'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            Some(SemanticAction::MatchBracket { extend: true })
        );
    }

    #[test]
    fn ctrl_home_end_jump_to_document_edges() {
        assert_eq!(
            editor(key(KeyCode::Home, KeyModifiers::CONTROL)),
            Some(SemanticAction::Move {
                movement: Movement::DocumentStart,
                extend: false
            })
        );
        assert_eq!(
            editor(key(KeyCode::End, KeyModifiers::NONE)),
            Some(SemanticAction::Move {
                movement: Movement::LineEnd,
                extend: false
            })
        );
    }

    #[test]
    fn printable_chars_insert_including_altgr() {
        assert_eq!(
            editor(key(KeyCode::Char('A'), KeyModifiers::SHIFT)),
            Some(SemanticAction::InsertChar('A'))
        );
        assert_eq!(
            editor(key(
                KeyCode::Char('@'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )),
            Some(SemanticAction::InsertChar('@'))
        );
        assert_eq!(editor(key(KeyCode::Char('x'), KeyModifiers::ALT)), None);
    }

    #[test]
    fn ctrl_shortcuts() {
        assert_eq!(
            editor(key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            Some(SemanticAction::Undo)
        );
        assert_eq!(
            editor(key(
                KeyCode::Char('Z'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            Some(SemanticAction::Redo)
        );
        assert_eq!(
            editor(key(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            Some(SemanticAction::Quit)
        );
        assert_eq!(
            editor(key(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            Some(SemanticAction::Save)
        );
        assert_eq!(
            editor(key(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            Some(SemanticAction::JumpBack)
        );
        assert_eq!(
            editor(key(
                KeyCode::Char('o'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            Some(SemanticAction::JumpForward)
        );
        assert_eq!(
            editor(key(KeyCode::Char(' '), KeyModifiers::CONTROL)),
            Some(SemanticAction::Complete)
        );
        assert_eq!(
            editor(key(KeyCode::F(2), KeyModifiers::NONE)),
            Some(SemanticAction::Hover)
        );
        assert_eq!(
            editor(key(KeyCode::F(12), KeyModifiers::NONE)),
            Some(SemanticAction::GotoDefinition)
        );
        assert_eq!(
            editor(key(KeyCode::F(12), KeyModifiers::SHIFT)),
            Some(SemanticAction::GotoReferences)
        );
        assert_eq!(
            editor(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(SemanticAction::Copy)
        );
        assert_eq!(
            editor(key(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            Some(SemanticAction::Cut)
        );
        assert_eq!(
            editor(key(KeyCode::Char('v'), KeyModifiers::CONTROL)),
            Some(SemanticAction::Paste)
        );
        assert_eq!(
            editor(key(KeyCode::Char('f'), KeyModifiers::CONTROL)),
            Some(SemanticAction::StartFind)
        );
        assert_eq!(
            editor(key(KeyCode::Char('h'), KeyModifiers::CONTROL)),
            Some(SemanticAction::StartReplace)
        );
        assert_eq!(
            editor(key(KeyCode::F(3), KeyModifiers::NONE)),
            Some(SemanticAction::FindNext)
        );
        assert_eq!(
            editor(key(KeyCode::F(3), KeyModifiers::SHIFT)),
            Some(SemanticAction::FindPrevious)
        );
        assert_eq!(
            editor(key(KeyCode::F(8), KeyModifiers::NONE)),
            Some(SemanticAction::GotoDiagnostic { next: true })
        );
        assert_eq!(
            editor(key(KeyCode::F(8), KeyModifiers::SHIFT)),
            Some(SemanticAction::GotoDiagnostic { next: false })
        );
        assert_eq!(
            route(
                &key(KeyCode::F(3), KeyModifiers::NONE),
                Focus::Sidebar,
                true
            ),
            Some(SemanticAction::FindNext)
        );
        assert_eq!(
            route(
                &key(KeyCode::Char('f'), KeyModifiers::CONTROL),
                Focus::Sidebar,
                false
            ),
            Some(SemanticAction::StartFind)
        );
    }

    #[test]
    fn find_mode_types_searches_and_esc_leaves() {
        let find = |event| route(&event, Focus::Find, false);
        assert_eq!(
            find(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some(SemanticAction::Find(FindAction::Insert("a".into())))
        );
        assert_eq!(
            find(key(KeyCode::Up, KeyModifiers::NONE)),
            Some(SemanticAction::Find(FindAction::Prev))
        );
        assert_eq!(
            find(key(KeyCode::Down, KeyModifiers::NONE)),
            Some(SemanticAction::Find(FindAction::Next))
        );
        assert_eq!(
            find(key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(SemanticAction::Find(FindAction::Submit))
        );
        assert_eq!(
            find(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(SemanticAction::Find(FindAction::Close))
        );
        assert_eq!(
            find(key(KeyCode::Char('b'), KeyModifiers::CONTROL)),
            Some(SemanticAction::Find(FindAction::Files))
        );
        assert_eq!(
            {
                let mut burst = crate::agent::PasteBurst::default();
                route_flags(
                    &key(KeyCode::Char(' '), KeyModifiers::NONE),
                    Focus::Sidebar,
                    false,
                    true,
                    &crate::keys::KeyMap::defaults(),
                    false,
                    false,
                    &mut burst,
                )
            },
            Some(SemanticAction::Sidebar(SidebarAction::Mark))
        );
        assert_eq!(
            find(key(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            Some(SemanticAction::Quit)
        );
    }

    #[test]
    fn replace_mode_edits_the_new_text_and_down_replaces() {
        let replace = |event| route(&event, Focus::Replace, false);
        assert_eq!(
            replace(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some(SemanticAction::Replace(ReplaceAction::Insert("a".into())))
        );
        assert_eq!(
            replace(key(KeyCode::Down, KeyModifiers::NONE)),
            Some(SemanticAction::Replace(ReplaceAction::Next))
        );
        assert_eq!(
            replace(key(KeyCode::Down, KeyModifiers::SHIFT)),
            Some(SemanticAction::Replace(ReplaceAction::All))
        );
        assert_eq!(
            replace(key(KeyCode::Up, KeyModifiers::NONE)),
            Some(SemanticAction::Replace(ReplaceAction::Undo))
        );
        assert_eq!(
            replace(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(SemanticAction::Replace(ReplaceAction::Close))
        );
    }

    #[test]
    fn tab_and_shift_tab_indent() {
        assert_eq!(
            editor(key(KeyCode::Tab, KeyModifiers::NONE)),
            Some(SemanticAction::Indent)
        );
        assert_eq!(
            editor(key(KeyCode::BackTab, KeyModifiers::SHIFT)),
            Some(SemanticAction::Outdent)
        );
        assert_eq!(
            editor(key(KeyCode::BackTab, KeyModifiers::NONE)),
            Some(SemanticAction::Outdent)
        );
        assert_eq!(
            editor(key(KeyCode::Tab, KeyModifiers::SHIFT)),
            Some(SemanticAction::Outdent)
        );
    }

    #[test]
    fn command_bar_openers() {
        assert_eq!(
            editor(key(KeyCode::Char(' '), KeyModifiers::SHIFT)),
            Some(SemanticAction::OpenCommandBar)
        );
        assert_eq!(
            editor(key(KeyCode::Char(' '), KeyModifiers::NONE)),
            Some(SemanticAction::InsertChar(' '))
        );
        assert_eq!(
            editor(key(KeyCode::F(1), KeyModifiers::NONE)),
            Some(SemanticAction::OpenCommandBar)
        );
        assert_eq!(
            editor(key(KeyCode::Char(';'), KeyModifiers::CONTROL)),
            Some(SemanticAction::OpenCommandBar)
        );
    }

    #[test]
    fn a_rebound_save_leaves_ctrl_s_free_and_keeps_quit() {
        use std::collections::BTreeMap;
        let mut overrides = BTreeMap::new();
        crate::keys::assign(&mut overrides, "save", "ctrl+shift+s").unwrap();
        let map = crate::keys::KeyMap::resolve(&overrides);
        let route = |event| {
            let mut burst = crate::agent::PasteBurst::default();
            route_flags(
                &event,
                Focus::Editor,
                false,
                false,
                &map,
                false,
                false,
                &mut burst,
            )
        };
        assert_eq!(route(key(KeyCode::Char('s'), KeyModifiers::CONTROL)), None);
        assert_eq!(
            route(key(
                KeyCode::Char('s'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            Some(SemanticAction::Save)
        );
        assert_eq!(
            route(key(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            Some(SemanticAction::Quit)
        );
        assert_eq!(
            route(key(KeyCode::F(1), KeyModifiers::NONE)),
            Some(SemanticAction::OpenCommandBar)
        );
    }

    #[test]
    fn command_bar_focus_routes_to_bar() {
        let bar = |event| route(&event, Focus::CommandBar, false);
        assert_eq!(
            bar(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(SemanticAction::CommandBar(CommandBarAction::Close))
        );
        assert_eq!(
            bar(key(KeyCode::Char(' '), KeyModifiers::SHIFT)),
            Some(SemanticAction::CommandBar(CommandBarAction::Insert(
                " ".into()
            )))
        );
        assert_eq!(
            bar(key(KeyCode::Left, KeyModifiers::NONE)),
            Some(SemanticAction::CommandBar(CommandBarAction::Cursor(
                LineMove::Left
            )))
        );
        for (code, mods, expected) in [
            (
                KeyCode::Left,
                KeyModifiers::CONTROL,
                CommandBarAction::Cursor(LineMove::WordLeft),
            ),
            (
                KeyCode::Right,
                KeyModifiers::CONTROL,
                CommandBarAction::Cursor(LineMove::WordRight),
            ),
            (
                KeyCode::Home,
                KeyModifiers::NONE,
                CommandBarAction::Cursor(LineMove::Home),
            ),
            (
                KeyCode::End,
                KeyModifiers::NONE,
                CommandBarAction::Cursor(LineMove::End),
            ),
            (
                KeyCode::Char('a'),
                KeyModifiers::CONTROL,
                CommandBarAction::Cursor(LineMove::Home),
            ),
            (
                KeyCode::Char('e'),
                KeyModifiers::CONTROL,
                CommandBarAction::Cursor(LineMove::End),
            ),
            (
                KeyCode::Delete,
                KeyModifiers::NONE,
                CommandBarAction::DeleteForward,
            ),
            (
                KeyCode::Backspace,
                KeyModifiers::NONE,
                CommandBarAction::Backspace,
            ),
            (
                KeyCode::Backspace,
                KeyModifiers::CONTROL,
                CommandBarAction::DeleteWordBack,
            ),
            (
                KeyCode::Backspace,
                KeyModifiers::ALT,
                CommandBarAction::DeleteWordBack,
            ),
        ] {
            assert_eq!(
                bar(key(code, mods)),
                Some(SemanticAction::CommandBar(expected.clone())),
                "{code:?} {mods:?}"
            );
        }
        assert_eq!(bar(key(KeyCode::Left, KeyModifiers::SHIFT)), None);
        assert_eq!(
            bar(key(KeyCode::Right, KeyModifiers::NONE)),
            Some(SemanticAction::CommandBar(CommandBarAction::AcceptHint))
        );
        assert_eq!(
            bar(key(KeyCode::Tab, KeyModifiers::NONE)),
            Some(SemanticAction::CommandBar(CommandBarAction::ToChat))
        );
        assert_eq!(
            route(&key(KeyCode::Tab, KeyModifiers::NONE), Focus::Agent, false),
            Some(SemanticAction::Agent(AgentAction::Complete))
        );
        assert_eq!(
            route(
                &key(KeyCode::Left, KeyModifiers::SHIFT),
                Focus::Agent,
                false
            ),
            Some(SemanticAction::Agent(AgentAction::Move {
                right: false,
                extend: true
            }))
        );
        assert_eq!(
            route(
                &key(KeyCode::Right, KeyModifiers::NONE),
                Focus::Agent,
                false
            ),
            Some(SemanticAction::Agent(AgentAction::Move {
                right: true,
                extend: false
            }))
        );
        assert_eq!(
            route(
                &key(KeyCode::Up, KeyModifiers::CONTROL),
                Focus::Agent,
                false
            ),
            Some(SemanticAction::Agent(AgentAction::Above))
        );
        assert_eq!(
            route(
                &key(KeyCode::Down, KeyModifiers::CONTROL),
                Focus::Agent,
                false
            ),
            Some(SemanticAction::Agent(AgentAction::Below))
        );
        assert_eq!(bar(key(KeyCode::Right, KeyModifiers::SHIFT)), None);
        assert_eq!(bar(Event::Paste("\n".into())), None);
    }

    #[test]
    fn agent_paste_keeps_newlines_and_normalizes_crlf() {
        assert_eq!(
            route(&Event::Paste("a\r\nb\nc".into()), Focus::Agent, false),
            Some(SemanticAction::Agent(AgentAction::Insert("a\nb\nc".into())))
        );
    }

    #[test]
    fn agent_shift_enter_and_fallbacks_insert_newline() {
        assert_eq!(
            route(
                &key(KeyCode::Enter, KeyModifiers::SHIFT),
                Focus::Agent,
                false
            ),
            Some(SemanticAction::Agent(AgentAction::Insert("\n".into())))
        );
        assert_eq!(
            route(&key(KeyCode::Enter, KeyModifiers::ALT), Focus::Agent, false),
            Some(SemanticAction::Agent(AgentAction::Insert("\n".into())))
        );
        assert_eq!(
            route(
                &key(KeyCode::Char('j'), KeyModifiers::CONTROL),
                Focus::Agent,
                false
            ),
            Some(SemanticAction::Agent(AgentAction::Insert("\n".into())))
        );
        assert_eq!(
            route(
                &key(KeyCode::Enter, KeyModifiers::NONE),
                Focus::Agent,
                false
            ),
            Some(SemanticAction::Agent(AgentAction::Submit))
        );
    }

    #[test]
    fn agent_burst_enter_inserts_newline() {
        let mut burst = crate::agent::PasteBurst::default();
        let map = crate::keys::KeyMap::defaults();
        let note = |burst: &mut crate::agent::PasteBurst, code| {
            route_flags(
                &key(code, KeyModifiers::NONE),
                Focus::Agent,
                false,
                false,
                &map,
                false,
                false,
                burst,
            )
        };
        assert_eq!(
            note(&mut burst, KeyCode::Char('a')),
            Some(SemanticAction::Agent(AgentAction::Insert("a".into())))
        );
        assert_eq!(
            note(&mut burst, KeyCode::Char('b')),
            Some(SemanticAction::Agent(AgentAction::Insert("b".into())))
        );
        assert_eq!(
            note(&mut burst, KeyCode::Enter),
            Some(SemanticAction::Agent(AgentAction::Insert("\n".into())))
        );
    }

    #[test]
    fn ctrl_and_alt_arrows_move_by_word_and_ctrl_up_down_by_paragraph() {
        assert_eq!(
            editor(key(KeyCode::Left, KeyModifiers::CONTROL)),
            Some(SemanticAction::Move {
                movement: Movement::WordLeft,
                extend: false
            })
        );
        assert_eq!(
            editor(key(
                KeyCode::Right,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            Some(SemanticAction::Move {
                movement: Movement::WordRight,
                extend: true
            })
        );
        assert_eq!(
            editor(key(KeyCode::Up, KeyModifiers::CONTROL)),
            Some(SemanticAction::Move {
                movement: Movement::ParagraphUp,
                extend: false
            })
        );
        assert_eq!(
            editor(key(KeyCode::Down, KeyModifiers::CONTROL)),
            Some(SemanticAction::Move {
                movement: Movement::ParagraphDown,
                extend: false
            })
        );
        assert_eq!(
            editor(key(KeyCode::Left, KeyModifiers::ALT | KeyModifiers::SHIFT)),
            Some(SemanticAction::Move {
                movement: Movement::WordLeft,
                extend: true
            })
        );
        assert_eq!(
            editor(key(KeyCode::Right, KeyModifiers::ALT)),
            Some(SemanticAction::Move {
                movement: Movement::WordRight,
                extend: false
            })
        );
        assert_eq!(
            editor(key(
                KeyCode::Left,
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )),
            Some(SemanticAction::CycleTab { next: false })
        );
        assert_eq!(
            editor(key(KeyCode::Up, KeyModifiers::CONTROL | KeyModifiers::ALT)),
            Some(SemanticAction::DuplicateLine { down: false })
        );
        assert_eq!(
            editor(key(
                KeyCode::Down,
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )),
            Some(SemanticAction::DuplicateLine { down: true })
        );
        assert_eq!(
            editor(key(KeyCode::Delete, KeyModifiers::SHIFT)),
            Some(SemanticAction::DeleteLine)
        );
        assert_eq!(
            editor(key(KeyCode::Delete, KeyModifiers::NONE)),
            Some(SemanticAction::DeleteForward)
        );
        assert_eq!(
            editor(key(
                KeyCode::Right,
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )),
            Some(SemanticAction::CycleTab { next: true })
        );
        assert_eq!(
            route(
                &key(KeyCode::Right, KeyModifiers::CONTROL | KeyModifiers::ALT),
                Focus::Sidebar,
                false,
            ),
            Some(SemanticAction::CycleTab { next: true })
        );
        assert_eq!(
            editor(key(
                KeyCode::Left,
                KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT
            )),
            Some(SemanticAction::Move {
                movement: Movement::Left,
                extend: true
            })
        );
    }

    #[test]
    fn ctrl_b_toggles_sidebar_from_editor_and_sidebar() {
        let ctrl_b = || key(KeyCode::Char('b'), KeyModifiers::CONTROL);
        assert_eq!(editor(ctrl_b()), Some(SemanticAction::ToggleSidebar));
        assert_eq!(
            route(&ctrl_b(), Focus::Sidebar, false),
            Some(SemanticAction::ToggleSidebar)
        );
    }

    #[test]
    fn sidebar_focus_routes_navigation_and_filter_typing() {
        let sidebar = |event| route(&event, Focus::Sidebar, false);
        assert_eq!(
            sidebar(key(KeyCode::Left, KeyModifiers::NONE)),
            Some(SemanticAction::Sidebar(SidebarAction::Collapse {
                extend: false
            }))
        );
        assert_eq!(
            sidebar(key(KeyCode::Down, KeyModifiers::SHIFT)),
            Some(SemanticAction::Sidebar(SidebarAction::Down {
                extend: false
            }))
        );
        assert_eq!(
            sidebar(key(KeyCode::Char(' '), KeyModifiers::NONE)),
            Some(SemanticAction::Sidebar(SidebarAction::FilterInsert(
                " ".into()
            )))
        );
        assert_eq!(
            sidebar(key(KeyCode::Right, KeyModifiers::CONTROL)),
            Some(SemanticAction::Sidebar(SidebarAction::Expand {
                extend: false
            }))
        );
        assert_eq!(
            sidebar(key(KeyCode::Left, KeyModifiers::CONTROL)),
            Some(SemanticAction::Sidebar(SidebarAction::Collapse {
                extend: false
            }))
        );
        assert_eq!(
            sidebar(key(KeyCode::Right, KeyModifiers::ALT)),
            Some(SemanticAction::ResizeSidebar { wider: true })
        );
        assert_eq!(
            sidebar(key(KeyCode::Left, KeyModifiers::ALT)),
            Some(SemanticAction::ResizeSidebar { wider: false })
        );
        assert_eq!(
            editor(key(KeyCode::Right, KeyModifiers::ALT)),
            Some(SemanticAction::Move {
                movement: Movement::WordRight,
                extend: false
            })
        );
        assert_eq!(
            sidebar(key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(SemanticAction::Sidebar(SidebarAction::Activate))
        );
        assert_eq!(
            sidebar(key(KeyCode::Char('m'), KeyModifiers::NONE)),
            Some(SemanticAction::Sidebar(SidebarAction::FilterInsert(
                "m".into()
            )))
        );
        assert_eq!(
            sidebar(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(SemanticAction::Sidebar(SidebarAction::Escape))
        );
        assert_eq!(
            sidebar(key(KeyCode::Char(' '), KeyModifiers::SHIFT)),
            Some(SemanticAction::OpenCommandBar)
        );
        assert_eq!(
            sidebar(key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            None
        );
        assert_eq!(
            sidebar(key(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            Some(SemanticAction::Save)
        );
        assert_eq!(
            sidebar(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(SemanticAction::Copy)
        );
    }

    #[test]
    fn select_mode_marks_with_space_and_leaves_on_esc() {
        let selecting = |event| route(&event, Focus::Sidebar, true);
        assert_eq!(
            selecting(key(KeyCode::Down, KeyModifiers::SHIFT)),
            Some(SemanticAction::Sidebar(SidebarAction::Down {
                extend: true
            }))
        );
        assert_eq!(
            selecting(key(KeyCode::Char(' '), KeyModifiers::NONE)),
            Some(SemanticAction::Sidebar(SidebarAction::Mark))
        );
        assert_eq!(
            selecting(key(KeyCode::Char(' '), KeyModifiers::SHIFT)),
            Some(SemanticAction::OpenCommandBar)
        );
        assert_eq!(
            selecting(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(SemanticAction::Sidebar(SidebarAction::ExitSelect))
        );
        assert_eq!(
            selecting(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some(SemanticAction::Sidebar(SidebarAction::FilterInsert(
                "a".into()
            )))
        );
    }

    #[test]
    fn dialog_uses_arrows_and_keeps_letters_as_shortcuts() {
        let prompt = |event| route(&event, Focus::Prompt, false);
        assert_eq!(
            prompt(key(KeyCode::Right, KeyModifiers::NONE)),
            Some(SemanticAction::Prompt(PromptAction::Next))
        );
        assert_eq!(
            prompt(key(KeyCode::Left, KeyModifiers::NONE)),
            Some(SemanticAction::Prompt(PromptAction::Prev))
        );
        assert_eq!(
            prompt(key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(SemanticAction::Prompt(PromptAction::Confirm))
        );
        assert_eq!(
            prompt(key(KeyCode::Char('y'), KeyModifiers::NONE)),
            Some(SemanticAction::Prompt(PromptAction::Accelerator('y')))
        );
        assert_eq!(
            prompt(key(KeyCode::Char('N'), KeyModifiers::SHIFT)),
            Some(SemanticAction::Prompt(PromptAction::Accelerator('N')))
        );
        assert_eq!(
            prompt(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(SemanticAction::Prompt(PromptAction::Cancel))
        );
        assert_eq!(prompt(key(KeyCode::Char('s'), KeyModifiers::CONTROL)), None);
    }

    #[test]
    fn key_release_is_ignored() {
        let release = Event::Key(KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        });
        assert_eq!(editor(release), None);
    }

    #[test]
    fn paste_inserts_text() {
        assert_eq!(
            editor(Event::Paste("hi".into())),
            Some(SemanticAction::InsertText("hi".into()))
        );
    }

    #[test]
    fn settings_page_moves_by_arrow_and_ignores_typing() {
        let settings = |event| route(&event, Focus::Settings, false);
        assert_eq!(
            settings(key(KeyCode::Down, KeyModifiers::NONE)),
            Some(SemanticAction::Settings(SettingsAction::Down))
        );
        assert_eq!(
            settings(key(KeyCode::Right, KeyModifiers::NONE)),
            Some(SemanticAction::Settings(SettingsAction::Enter))
        );
        assert_eq!(
            settings(key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(SemanticAction::Settings(SettingsAction::Enter))
        );
        assert_eq!(
            settings(key(KeyCode::Left, KeyModifiers::NONE)),
            Some(SemanticAction::Settings(SettingsAction::Back))
        );
        assert_eq!(
            settings(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(SemanticAction::Settings(SettingsAction::Back))
        );
        assert_eq!(
            settings(key(KeyCode::Backspace, KeyModifiers::NONE)),
            Some(SemanticAction::Settings(SettingsAction::Back))
        );
        assert_eq!(settings(key(KeyCode::Char('a'), KeyModifiers::NONE)), None);
        let about = |event| route(&event, Focus::About, false);
        assert_eq!(
            about(key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(SemanticAction::CloseAbout)
        );
        assert_eq!(
            about(key(KeyCode::Backspace, KeyModifiers::NONE)),
            Some(SemanticAction::CloseAbout)
        );
        assert_eq!(about(key(KeyCode::Char('a'), KeyModifiers::NONE)), None);
        assert_eq!(
            about(key(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            Some(SemanticAction::Quit)
        );
        assert_eq!(
            settings(key(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            Some(SemanticAction::Quit)
        );
    }

    #[test]
    fn ctrl_d_only_deselects_where_a_buffer_takes_keys() {
        let ctrl_d = key(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(editor(ctrl_d.clone()), Some(SemanticAction::Deselect));
        assert_eq!(
            route(&ctrl_d, Focus::CompletionPopup, false),
            Some(SemanticAction::Deselect)
        );
        for focus in [
            Focus::Sidebar,
            Focus::Find,
            Focus::Replace,
            Focus::FilePicker,
            Focus::PreviewSide,
            Focus::About,
            Focus::Settings,
            Focus::Prompt,
        ] {
            assert_eq!(route(&ctrl_d, focus, false), None, "{focus:?}");
        }
        assert_eq!(route(&ctrl_d, Focus::Sidebar, true), None);
        assert_eq!(route(&ctrl_d, Focus::CommandBar, false), None);
    }

    #[test]
    fn ctrl_w_starts_the_focus_prefix_where_arrows_used_to_move_focus() {
        let ctrl_w = key(KeyCode::Char('w'), KeyModifiers::CONTROL);
        for focus in [
            Focus::Editor,
            Focus::Sidebar,
            Focus::Agent,
            Focus::CompletionPopup,
        ] {
            assert_eq!(
                route(&ctrl_w, focus, false),
                Some(SemanticAction::FocusPrefix),
                "{focus:?}"
            );
        }
        assert_eq!(
            route(&ctrl_w, Focus::Sidebar, true),
            Some(SemanticAction::FocusPrefix)
        );
        assert_eq!(
            route(&ctrl_w, Focus::FilePicker, false),
            Some(SemanticAction::Picker(PickerAction::DeleteWordBack))
        );
        for focus in [
            Focus::Find,
            Focus::Replace,
            Focus::CommandBar,
            Focus::PreviewSide,
            Focus::Prompt,
            Focus::About,
            Focus::Settings,
        ] {
            assert_eq!(route(&ctrl_w, focus, false), None, "{focus:?}");
        }
    }

    #[test]
    fn pane_mode_keys_move_confirm_cancel_and_swallow_the_rest() {
        use crossterm::event::{KeyEventKind, ModifierKeyCode};
        let keys = crate::keys::KeyMap::defaults();
        let pane = |code, modifiers| route_pane_select(&key(code, modifiers), &keys);
        let action = |action| Some(SemanticAction::PaneSelect(action));
        assert_eq!(
            pane(KeyCode::Left, KeyModifiers::NONE),
            action(PaneSelectAction::Move(PaneDirection::Left))
        );
        assert_eq!(
            pane(KeyCode::Right, KeyModifiers::CONTROL),
            action(PaneSelectAction::Move(PaneDirection::Right))
        );
        assert_eq!(
            pane(KeyCode::Up, KeyModifiers::CONTROL),
            action(PaneSelectAction::Move(PaneDirection::Up))
        );
        assert_eq!(
            pane(KeyCode::Down, KeyModifiers::NONE),
            action(PaneSelectAction::Move(PaneDirection::Down))
        );
        assert_eq!(
            pane(KeyCode::Enter, KeyModifiers::NONE),
            action(PaneSelectAction::Confirm)
        );
        assert_eq!(
            pane(KeyCode::Char('w'), KeyModifiers::CONTROL),
            action(PaneSelectAction::Confirm)
        );
        assert_eq!(
            pane(KeyCode::Esc, KeyModifiers::NONE),
            action(PaneSelectAction::Cancel)
        );
        for (code, modifiers) in [
            (KeyCode::Char('x'), KeyModifiers::NONE),
            (KeyCode::Char('w'), KeyModifiers::NONE),
            (KeyCode::Char('q'), KeyModifiers::CONTROL),
            (KeyCode::Backspace, KeyModifiers::NONE),
            (KeyCode::Tab, KeyModifiers::NONE),
            (
                KeyCode::Modifier(ModifierKeyCode::LeftControl),
                KeyModifiers::CONTROL,
            ),
        ] {
            assert_eq!(pane(code, modifiers), None, "{code:?}");
        }
        assert_eq!(route_pane_select(&Event::Paste("text".into()), &keys), None);
        let mut release = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        assert_eq!(route_pane_select(&Event::Key(release), &keys), None);
    }

    #[test]
    fn agent_ctrl_left_right_move_by_word_and_ctrl_up_down_still_read() {
        let agent = |code, modifiers| route(&key(code, modifiers), Focus::Agent, false);
        assert_eq!(
            agent(KeyCode::Left, KeyModifiers::CONTROL),
            Some(SemanticAction::Agent(AgentAction::Word {
                right: false,
                extend: false
            }))
        );
        assert_eq!(
            agent(KeyCode::Right, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
            Some(SemanticAction::Agent(AgentAction::Word {
                right: true,
                extend: true
            }))
        );
        assert_eq!(
            agent(KeyCode::Up, KeyModifiers::CONTROL),
            Some(SemanticAction::Agent(AgentAction::Above))
        );
        assert_eq!(
            agent(KeyCode::Down, KeyModifiers::CONTROL),
            Some(SemanticAction::Agent(AgentAction::Below))
        );
        assert_eq!(
            agent(KeyCode::Up, KeyModifiers::ALT),
            Some(SemanticAction::ResizeAgent { taller: true })
        );
        assert_eq!(
            agent(KeyCode::Down, KeyModifiers::ALT),
            Some(SemanticAction::ResizeAgent { taller: false })
        );
    }

    #[test]
    fn a_settings_text_row_takes_the_command_bar_line_keys() {
        use crate::command::LineMove;
        let keys = crate::keys::KeyMap::defaults();
        let mut burst = crate::agent::PasteBurst::default();
        let mut draft = |code, modifiers| {
            route_flags(
                &key(code, modifiers),
                Focus::Settings,
                false,
                false,
                &keys,
                false,
                true,
                &mut burst,
            )
        };
        let edit = |action| Some(SemanticAction::Settings(action));
        assert_eq!(
            draft(KeyCode::Left, KeyModifiers::NONE),
            edit(SettingsAction::Cursor(LineMove::Left))
        );
        assert_eq!(
            draft(KeyCode::Right, KeyModifiers::NONE),
            edit(SettingsAction::Cursor(LineMove::Right))
        );
        assert_eq!(
            draft(KeyCode::Left, KeyModifiers::CONTROL),
            edit(SettingsAction::Cursor(LineMove::WordLeft))
        );
        assert_eq!(
            draft(KeyCode::Right, KeyModifiers::CONTROL),
            edit(SettingsAction::Cursor(LineMove::WordRight))
        );
        assert_eq!(
            draft(KeyCode::Home, KeyModifiers::NONE),
            edit(SettingsAction::Cursor(LineMove::Home))
        );
        assert_eq!(
            draft(KeyCode::Char('e'), KeyModifiers::CONTROL),
            edit(SettingsAction::Cursor(LineMove::End))
        );
        assert_eq!(
            draft(KeyCode::Delete, KeyModifiers::NONE),
            edit(SettingsAction::DeleteForward)
        );
        assert_eq!(
            draft(KeyCode::Backspace, KeyModifiers::NONE),
            edit(SettingsAction::DeleteChar)
        );
        assert_eq!(
            draft(KeyCode::Backspace, KeyModifiers::ALT),
            edit(SettingsAction::DeleteWordBack)
        );
        assert_eq!(
            draft(KeyCode::Char('x'), KeyModifiers::NONE),
            edit(SettingsAction::Insert("x".into()))
        );
        assert_eq!(draft(KeyCode::Char('w'), KeyModifiers::CONTROL), None);
        assert_eq!(
            draft(KeyCode::Char('q'), KeyModifiers::CONTROL),
            Some(SemanticAction::Quit)
        );
    }
}
