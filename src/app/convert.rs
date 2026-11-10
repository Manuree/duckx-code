//! `convert-keyboard` (`Alt+\``): retype the selection as the same keys give on
//! another keyboard layout. See [`crate::keyboard_layout`].

use super::App;
use crate::action::CompletionAction;
use crate::keyboard_layout::{self, LAYOUTS, Layout};

/// Where the text to convert lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertTarget {
    /// The focused editor buffer.
    Editor,
    /// The assistant prompt.
    Agent,
}

/// The `Convert to` dropdown, shown when the OS cannot say which layout is the
/// other one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutMenu {
    pub target: ConvertTarget,
    /// Char range of the text, and the text itself, checked again before it is replaced.
    pub start: usize,
    pub end: usize,
    pub text: String,
    /// True when the user selected the text, false for the word before the cursor.
    pub selected_text: bool,
    pub from: &'static Layout,
    pub choices: Vec<&'static Layout>,
    pub selected: usize,
}

/// One row of the dropdown: the layout and what the text becomes on it.
pub struct LayoutMenuView {
    pub target: ConvertTarget,
    pub rows: Vec<(String, String)>,
    pub selected: usize,
}

/// The OS layout list and the layout switch, swappable in tests.
#[derive(Clone, Copy)]
pub struct LayoutOs {
    pub installed: fn() -> Option<Vec<Option<&'static Layout>>>,
    pub switch_to: fn(&Layout) -> bool,
}

impl Default for LayoutOs {
    fn default() -> Self {
        Self {
            installed: keyboard_layout::os::installed,
            switch_to: keyboard_layout::os::switch_to,
        }
    }
}

/// Longest preview shown beside a layout in the dropdown.
const PREVIEW_CHARS: usize = 24;

impl App {
    pub(super) fn convert_keyboard(&mut self) {
        self.layout_menu = None;
        let target = if self.agent.open && self.agent.focused {
            ConvertTarget::Agent
        } else {
            ConvertTarget::Editor
        };
        let Some((start, end, text, selected_text)) = self.convert_span(target) else {
            self.message = Some("Select the text to convert first".to_string());
            return;
        };
        let Some(from) = keyboard_layout::detect(&text) else {
            self.message = Some("Nothing to convert in the selection".to_string());
            return;
        };
        let installed = (self.layout_os.installed)();
        if let Some(to) = installed
            .as_deref()
            .and_then(|list| keyboard_layout::os::target(from, list))
        {
            self.apply_conversion(target, start, end, &text, selected_text, from, to, true);
            return;
        }
        let choices: Vec<&'static Layout> = LAYOUTS
            .iter()
            .copied()
            .filter(|layout| !std::ptr::eq(*layout, from))
            .collect();
        self.dismiss_lsp_popups();
        self.layout_menu = Some(LayoutMenu {
            target,
            start,
            end,
            text,
            selected_text,
            from,
            choices,
            selected: 0,
        });
    }

    /// The selection, or the word right before the cursor on its line.
    fn convert_span(&self, target: ConvertTarget) -> Option<(usize, usize, String, bool)> {
        match target {
            ConvertTarget::Editor => {
                let buffer = self.buffer();
                let highlight = buffer.highlight();
                if !highlight.is_empty() {
                    return Some((
                        highlight.from(),
                        highlight.to(),
                        buffer.highlighted_text(),
                        true,
                    ));
                }
                let rope = buffer.rope();
                let end = buffer.selection().head.min(rope.len_chars());
                let mut start = end;
                while start > 0 && !rope.char(start - 1).is_whitespace() {
                    start -= 1;
                }
                (start < end).then(|| (start, end, rope.slice(start..end).to_string(), false))
            }
            ConvertTarget::Agent => {
                let chars: Vec<char> = self.agent.prompt.chars().collect();
                let cursor = self.agent.cursor.min(chars.len());
                let anchor = self.agent.anchor.min(chars.len());
                if cursor != anchor {
                    let (start, end) = (cursor.min(anchor), cursor.max(anchor));
                    return Some((start, end, chars[start..end].iter().collect(), true));
                }
                let mut start = cursor;
                while start > 0 && !chars[start - 1].is_whitespace() {
                    start -= 1;
                }
                (start < cursor)
                    .then(|| (start, cursor, chars[start..cursor].iter().collect(), false))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_conversion(
        &mut self,
        target: ConvertTarget,
        start: usize,
        end: usize,
        text: &str,
        selected_text: bool,
        from: &'static Layout,
        to: &'static Layout,
        from_os: bool,
    ) {
        let out = keyboard_layout::convert(text, from, to);
        if out == text {
            self.message = Some(format!("Nothing changes on {}", to.name));
            return;
        }
        let len = out.chars().count();
        match target {
            ConvertTarget::Editor => {
                let rope = self.buffer().rope();
                if end > rope.len_chars() || rope.slice(start..end) != text {
                    self.message = Some("The text changed; run convert-keyboard again".to_string());
                    return;
                }
                self.clear_snippet();
                self.dismiss_lsp_popups();
                self.edit(|buffer| {
                    buffer.replace_span(start, end, &out);
                    // A selection stays over the new text with the cursor at its end.
                    buffer.move_to(if selected_text { start } else { start + len }, false);
                    if selected_text {
                        buffer.move_to(start + len, true);
                    }
                });
                self.scroll_to_cursor();
            }
            ConvertTarget::Agent => {
                let mut chars: Vec<char> = self.agent.prompt.chars().collect();
                if end > chars.len() || chars[start..end].iter().collect::<String>() != text {
                    self.message = Some("The text changed; run convert-keyboard again".to_string());
                    return;
                }
                chars.splice(start..end, out.chars());
                self.agent.prompt = chars.into_iter().collect();
                self.agent.cursor = start + len;
                self.agent.anchor = if selected_text { start } else { start + len };
            }
        }
        // Switch the OS layout when the OS has it, so typing goes on in `to`.
        let has_to = from_os
            || (self.layout_os.installed)().is_some_and(|list| {
                list.iter()
                    .any(|layout| layout.is_some_and(|l| std::ptr::eq(l, to)))
            });
        let switched = has_to && (self.layout_os.switch_to)(to);
        self.message = Some(if switched {
            format!("Converted to {}; keyboard switched to {}", to.name, to.name)
        } else if from_os {
            format!("Converted to {}; could not switch the keyboard", to.name)
        } else {
            format!("Converted to {}", to.name)
        });
    }

    pub(super) fn handle_layout_menu(&mut self, action: CompletionAction) {
        let Some(menu) = self.layout_menu.as_mut() else {
            return;
        };
        let len = menu.choices.len().max(1);
        match action {
            CompletionAction::Prev => menu.selected = (menu.selected + len - 1) % len,
            CompletionAction::Next => menu.selected = (menu.selected + 1) % len,
            CompletionAction::Close => self.layout_menu = None,
            CompletionAction::Accept => {
                let Some(menu) = self.layout_menu.take() else {
                    return;
                };
                let Some(to) = menu.choices.get(menu.selected).copied() else {
                    return;
                };
                self.apply_conversion(
                    menu.target,
                    menu.start,
                    menu.end,
                    &menu.text,
                    menu.selected_text,
                    menu.from,
                    to,
                    false,
                );
            }
        }
    }

    pub fn layout_menu_view(&self) -> Option<LayoutMenuView> {
        let menu = self.layout_menu.as_ref()?;
        let rows = menu
            .choices
            .iter()
            .map(|layout| {
                let preview = keyboard_layout::convert(&menu.text, menu.from, layout);
                let line = preview.lines().next().unwrap_or("");
                let mut short: String = line.chars().take(PREVIEW_CHARS).collect();
                if line.chars().count() > PREVIEW_CHARS || preview.lines().nth(1).is_some() {
                    short.push('…');
                }
                (layout.name.to_string(), short)
            })
            .collect();
        Some(LayoutMenuView {
            target: menu.target,
            rows,
            selected: menu.selected,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Focus, SemanticAction};
    use crate::buffer::Buffer;
    use crate::keyboard_layout::{THAI, US};

    fn no_os() -> Option<Vec<Option<&'static Layout>>> {
        None
    }

    fn us_and_thai() -> Option<Vec<Option<&'static Layout>>> {
        Some(vec![Some(&US), Some(&THAI)])
    }

    fn three() -> Option<Vec<Option<&'static Layout>>> {
        Some(vec![Some(&US), Some(&THAI), None])
    }

    fn switch_ok(_: &Layout) -> bool {
        true
    }

    fn switch_fails(_: &Layout) -> bool {
        false
    }

    fn app(text: &str, installed: fn() -> Option<Vec<Option<&'static Layout>>>) -> App {
        let mut app = App::new(Buffer::from_text(text), None);
        app.layout_os = LayoutOs {
            installed,
            switch_to: switch_ok,
        };
        app
    }

    fn text(app: &App) -> String {
        app.buffer().rope().to_string()
    }

    #[test]
    fn without_the_os_a_dropdown_picks_the_layout_and_undo_is_one_step() {
        let mut app = app("ouj8nv8ezbf", no_os);
        app.buffer_mut().select_all();
        app.handle(SemanticAction::ConvertKeyboard);
        assert_eq!(app.focus(), Focus::LayoutMenu);
        let view = app.layout_menu_view().unwrap();
        assert_eq!(
            view.rows,
            [("Thai (Kedmanee)".to_string(), "นี่คือคำผิด".to_string())]
        );
        assert_eq!(text(&app), "ouj8nv8ezbf", "nothing changes before Enter");
        app.handle(SemanticAction::LayoutMenu(CompletionAction::Accept));
        assert_eq!(text(&app), "นี่คือคำผิด");
        assert_eq!(app.buffer().highlighted_text(), "นี่คือคำผิด", "still selected");
        assert_eq!(app.buffer().selection().head, 11, "cursor at the end");
        assert_eq!(app.focus(), Focus::Editor);
        assert_eq!(app.message.as_deref(), Some("Converted to Thai (Kedmanee)"));
        app.handle(SemanticAction::Undo);
        assert_eq!(text(&app), "ouj8nv8ezbf");
    }

    #[test]
    fn esc_closes_the_dropdown_and_leaves_the_text() {
        let mut app = app("ouj8", no_os);
        app.buffer_mut().select_all();
        app.handle(SemanticAction::ConvertKeyboard);
        app.handle(SemanticAction::LayoutMenu(CompletionAction::Next));
        app.handle(SemanticAction::LayoutMenu(CompletionAction::Close));
        assert_eq!(app.focus(), Focus::Editor);
        assert_eq!(text(&app), "ouj8");
    }

    #[test]
    fn two_os_layouts_convert_at_once_both_ways_and_switch_the_keyboard() {
        let mut app = app("ouj8nv8ezbf", us_and_thai);
        app.buffer_mut().select_all();
        app.handle(SemanticAction::ConvertKeyboard);
        assert!(app.layout_menu.is_none(), "no prompt");
        assert_eq!(text(&app), "นี่คือคำผิด");
        assert_eq!(
            app.message.as_deref(),
            Some("Converted to Thai (Kedmanee); keyboard switched to Thai (Kedmanee)")
        );
        app.handle(SemanticAction::ConvertKeyboard);
        assert_eq!(text(&app), "ouj8nv8ezbf", "Thai goes back to English");

        app.layout_os.switch_to = switch_fails;
        app.handle(SemanticAction::ConvertKeyboard);
        assert_eq!(text(&app), "นี่คือคำผิด");
        assert_eq!(
            app.message.as_deref(),
            Some("Converted to Thai (Kedmanee); could not switch the keyboard")
        );
    }

    #[test]
    fn more_than_two_os_layouts_ask_instead() {
        let mut app = app("ouj8", three);
        app.buffer_mut().select_all();
        app.handle(SemanticAction::ConvertKeyboard);
        assert_eq!(app.focus(), Focus::LayoutMenu);
        app.handle(SemanticAction::LayoutMenu(CompletionAction::Accept));
        assert_eq!(text(&app), "นี่ค");
        assert!(
            app.message
                .as_deref()
                .unwrap()
                .contains("keyboard switched"),
            "Thai is installed, so the OS is still asked to switch"
        );
    }

    #[test]
    fn with_no_selection_the_word_before_the_cursor_is_converted() {
        let mut app = app("hello ouj8nv8ezbf", us_and_thai);
        app.buffer_mut().move_to(17, false);
        app.handle(SemanticAction::ConvertKeyboard);
        assert_eq!(text(&app), "hello นี่คือคำผิด");
        let len = text(&app).chars().count();
        assert_eq!(app.buffer().selection().head, len, "caret after the word");
        assert!(app.buffer().highlight().is_empty());

        let mut app = app_with_space();
        app.handle(SemanticAction::ConvertKeyboard);
        assert_eq!(
            app.message.as_deref(),
            Some("Select the text to convert first")
        );
        assert_eq!(text(&app), "ouj8 ");
    }

    fn app_with_space() -> App {
        let mut app = app("ouj8 ", us_and_thai);
        app.buffer_mut().move_to(5, false);
        app
    }

    #[test]
    fn the_assistant_prompt_converts_its_selection_too() {
        let mut app = app("", us_and_thai);
        app.agent.open = true;
        app.agent.focused = true;
        app.agent.prompt = "hi ouj8nv8ezbf".into();
        app.agent.anchor = 3;
        app.agent.cursor = 14;
        app.handle(SemanticAction::ConvertKeyboard);
        assert_eq!(app.agent.prompt, "hi นี่คือคำผิด");
        assert_eq!((app.agent.anchor, app.agent.cursor), (3, 14));
        assert_eq!(text(&app), "", "the editor is untouched");
    }

    #[test]
    fn text_with_nothing_to_convert_says_so() {
        let mut app = app("   \n", us_and_thai);
        app.buffer_mut().select_all();
        app.handle(SemanticAction::ConvertKeyboard);
        assert_eq!(
            app.message.as_deref(),
            Some("Nothing to convert in the selection")
        );
    }
}
