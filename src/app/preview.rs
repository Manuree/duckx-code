//! `preview`: a read-only Markdown pane beside the source editor.
//!
//! The preview is a real window in the split layout. Its editor holds one empty
//! placeholder tab that is never shown, saved, or edited; the screen draws the
//! rendered source tab instead. The source buffer is read on every frame, so unsaved
//! edits show up at once.

use std::path::Path;

use super::App;
use crate::action::{Focus, PreviewSideAction, SemanticAction, Side};
use crate::buffer::{Buffer, Movement};
use crate::command::CommandId;
use crate::layout::{Axis, EditorId, Layout, TabId};
use crate::syntax::Language;

/// One preview window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// The window the preview is drawn in.
    pub pane: EditorId,
    /// The Markdown tab it renders.
    pub source: TabId,
    /// Which side of the source window the preview sits on.
    pub side: Side,
    /// First rendered row on screen.
    pub scroll: usize,
    /// Text area inside the border, from the last layout.
    pub(super) width: u16,
    pub(super) height: u16,
}

/// The side picker that bare `preview` opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewPicker {
    /// The Markdown tab the preview will render.
    pub source: TabId,
    /// The side `Enter` picks. Starts on the last side used.
    pub selected: Side,
}

/// The direction typed after `preview`.
pub fn preview_side(arg: &str) -> Result<Side, String> {
    match arg.trim().to_ascii_lowercase().as_str() {
        "split-right" | "right" => Ok(Side::Right),
        "split-down" | "down" => Ok(Side::Down),
        "split-left" | "left" => Ok(Side::Left),
        "split-up" | "up" => Ok(Side::Up),
        other => Err(format!(
            "Unknown preview side: {other} (use split-up, split-down, split-left, or split-right)"
        )),
    }
}

fn side_axis(side: Side) -> Axis {
    match side {
        Side::Left | Side::Right => Axis::Horizontal,
        Side::Up | Side::Down => Axis::Vertical,
    }
}

pub fn is_markdown(path: Option<&Path>) -> bool {
    path.and_then(Language::detect) == Some(Language::Markdown)
}

/// Commands that still make sense while the preview window is focused.
fn allowed_in_preview(id: CommandId) -> bool {
    matches!(
        id,
        CommandId::Quit
            | CommandId::Close
            | CommandId::CloseAll
            | CommandId::CloseView
            | CommandId::ToggleExplorer
            | CommandId::SidebarWider
            | CommandId::SidebarNarrower
            | CommandId::SplitRight
            | CommandId::SplitDown
            | CommandId::TabPrev
            | CommandId::TabNext
            | CommandId::Focus
            | CommandId::Preview
            | CommandId::Settings
            | CommandId::About
            | CommandId::Agent
            | CommandId::AgentNew
            | CommandId::FilePicker
            | CommandId::Workspace
            | CommandId::NewFile
            | CommandId::CopyName
            | CommandId::CopyPath
            | CommandId::CopyRelativePath
    )
}

impl App {
    /// The preview drawn in `pane`, if that window is one.
    pub fn preview(&self, pane: EditorId) -> Option<&Preview> {
        self.previews.iter().find(|preview| preview.pane == pane)
    }

    pub fn previews(&self) -> &[Preview] {
        &self.previews
    }

    /// The source text and title for the preview in `pane`.
    pub fn preview_source(&self, pane: EditorId) -> Option<(&Buffer, String)> {
        let preview = self.preview(pane)?;
        let tab = self.find_tab(preview.source)?;
        let name = tab
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "[No Name]".to_string());
        Some((&tab.buffer, format!(" {name} · preview ")))
    }

    pub(super) fn is_preview(&self, pane: EditorId) -> bool {
        self.preview(pane).is_some()
    }

    /// True while key presses go to a focused preview window.
    pub fn preview_focused(&self) -> bool {
        self.is_preview(self.active) && self.focus() == Focus::Editor
    }

    /// The Markdown tab `preview` works on: the focused tab, or the source of a
    /// focused preview. Anything that is not Markdown is refused here, before any popup.
    fn preview_target(&self) -> Result<TabId, String> {
        let source = match self.preview(self.active) {
            Some(preview) => preview.source,
            None => self.active_tab_id(),
        };
        let tab = self
            .find_tab(source)
            .ok_or_else(|| "Nothing to preview".to_string())?;
        if !is_markdown(tab.path.as_deref()) {
            return Err("preview works on Markdown (.md) files".to_string());
        }
        Ok(source)
    }

    /// `preview` opens the side picker. `preview split-left` and the other sides
    /// open at once. Focus stays on the source.
    pub(super) fn open_preview(&mut self, arg: &str) -> Result<(), String> {
        let source = self.preview_target()?;
        if arg.trim().is_empty() {
            self.preview_picker = Some(PreviewPicker {
                source,
                selected: self.preview_side,
            });
            return Ok(());
        }
        let side = preview_side(arg)?;
        self.open_preview_on(source, side)
    }

    /// File name of `tab`, for titles.
    pub fn tab_label(&self, tab: TabId) -> Option<String> {
        let path = self.find_tab(tab)?.path.as_deref()?;
        Some(path.file_name()?.to_string_lossy().into_owned())
    }

    pub fn preview_picker(&self) -> Option<&PreviewPicker> {
        self.preview_picker.as_ref()
    }

    /// Keys while the side picker is open. An arrow opens the preview on that side.
    pub(super) fn handle_preview_side(&mut self, action: PreviewSideAction) {
        let Some(picker) = self.preview_picker else {
            return;
        };
        let side = match action {
            PreviewSideAction::Choose(side) => side,
            PreviewSideAction::Confirm => picker.selected,
            PreviewSideAction::Next | PreviewSideAction::Prev => {
                let at = Side::ALL
                    .iter()
                    .position(|side| *side == picker.selected)
                    .unwrap_or(0);
                let step = if action == PreviewSideAction::Next {
                    1
                } else {
                    3
                };
                if let Some(picker) = self.preview_picker.as_mut() {
                    picker.selected = Side::ALL[(at + step) % 4];
                }
                return;
            }
            PreviewSideAction::Cancel => {
                self.preview_picker = None;
                return;
            }
        };
        self.preview_picker = None;
        if let Err(message) = self.open_preview_on(picker.source, side) {
            self.message = Some(message);
        }
    }

    fn open_preview_on(&mut self, source: TabId, side: Side) -> Result<(), String> {
        let source_pane = self
            .pane_of(source)
            .ok_or_else(|| "Nothing to preview".to_string())?;
        self.preview_side = side;
        if let Some(existing) = self.previews.iter().find(|p| p.source == source) {
            if existing.side == side {
                self.active = source_pane;
                self.explorer.blur();
                self.end_file_mode();
                self.message = Some("Preview is already open".to_string());
                return Ok(());
            }
            let pane = existing.pane;
            self.close_preview_pane(pane);
        }
        let id = EditorId(self.next_id);
        self.next_id += 1;
        let buffer = self.style_buffer(Buffer::new());
        let tab = self.alloc_tab(buffer, None);
        self.editors.push(super::fresh_editor(id, tab));
        let axis = side_axis(side);
        let layout = std::mem::replace(&mut self.layout, Layout::Leaf(id));
        self.layout = match side {
            Side::Right | Side::Down => layout.split(source_pane, axis, id),
            Side::Left | Side::Up => layout.split_before(source_pane, axis, id),
        };
        self.previews.push(Preview {
            pane: id,
            source,
            side,
            scroll: 0,
            width: 0,
            height: 0,
        });
        self.active = source_pane;
        self.explorer.blur();
        self.end_file_mode();
        Ok(())
    }

    /// Removes the preview window `pane`. The last window becomes an empty page instead.
    pub(super) fn close_preview_pane(&mut self, pane: EditorId) {
        self.previews.retain(|preview| preview.pane != pane);
        if !self.editors.iter().any(|editor| editor.id == pane) {
            return;
        }
        let next = self.layout.focus_after_close(pane);
        let current = std::mem::replace(&mut self.layout, Layout::Leaf(pane));
        match current.close(pane) {
            Ok(layout) => {
                self.layout = layout;
                self.editors.retain(|editor| editor.id != pane);
                if self.active == pane
                    && let Some(next) = next
                {
                    self.active = next;
                }
            }
            Err(layout) => {
                self.layout = layout;
                self.blank_editor(pane);
            }
        }
    }

    /// Drops previews whose window or source tab is gone. Closing the source closes its preview.
    pub(super) fn prune_previews(&mut self) {
        let stale: Vec<EditorId> = self
            .previews
            .iter()
            .filter(|preview| {
                !self.editors.iter().any(|editor| editor.id == preview.pane)
                    || self.find_tab(preview.source).is_none()
                    || self.pane_of(preview.source) == Some(preview.pane)
            })
            .map(|preview| preview.pane)
            .collect();
        for pane in stale {
            self.close_preview_pane(pane);
        }
    }

    /// Keeps the inner size of each preview for paging and scroll limits.
    pub(super) fn size_preview(&mut self, pane: EditorId, width: u16, height: u16) {
        if let Some(preview) = self.previews.iter_mut().find(|p| p.pane == pane) {
            // Rounded border on every side plus one column of padding left and right.
            preview.width = width.saturating_sub(4);
            preview.height = height.saturating_sub(2);
        }
    }

    /// Number of rendered rows for the preview in `pane` at its last width.
    fn preview_rows(&self, pane: EditorId) -> usize {
        let Some(preview) = self.preview(pane) else {
            return 0;
        };
        let Some((buffer, _)) = self.preview_source(pane) else {
            return 0;
        };
        let styles = self.markdown_styles();
        crate::markdown::render(&buffer.rope().to_string(), preview.width.max(1), &styles).len()
    }

    pub fn markdown_styles(&self) -> crate::markdown::MarkdownStyles {
        crate::markdown::MarkdownStyles::from_theme(
            &self.appearance.styles,
            &self.appearance.syntax,
        )
    }

    fn scroll_preview(&mut self, to: impl FnOnce(usize, usize) -> isize) {
        let pane = self.active;
        let rows = self.preview_rows(pane);
        let Some(preview) = self.previews.iter_mut().find(|p| p.pane == pane) else {
            return;
        };
        let page = usize::from(preview.height.max(1));
        let max = rows.saturating_sub(page);
        let next = to(preview.scroll, page).clamp(0, max as isize);
        preview.scroll = next as usize;
    }

    /// Key handling while the preview window is focused. Movement scrolls; anything
    /// that would edit or save is refused. Returns `true` when the action was used.
    pub(super) fn handle_preview_action(&mut self, action: &SemanticAction) -> bool {
        let line = |delta: isize| move |scroll: usize, _: usize| scroll as isize + delta;
        match action {
            SemanticAction::Move { movement, .. } => match movement {
                Movement::Up => self.scroll_preview(line(-1)),
                Movement::Down => self.scroll_preview(line(1)),
                Movement::PageUp(_) => self.scroll_preview(|s, p| s as isize - p as isize),
                Movement::PageDown(_) => self.scroll_preview(|s, p| (s + p) as isize),
                Movement::DocumentStart => self.scroll_preview(|_, _| 0),
                Movement::DocumentEnd => self.scroll_preview(|_, _| isize::MAX),
                _ => {}
            },
            SemanticAction::ScrollLine { down } => {
                self.scroll_preview(line(if *down { 1 } else { -1 }))
            }
            SemanticAction::PageUp { .. } => self.scroll_preview(|s, p| s as isize - p as isize),
            SemanticAction::PageDown { .. } => self.scroll_preview(|s, p| (s + p) as isize),
            SemanticAction::OpenCommandBar
            | SemanticAction::OpenFilePicker
            | SemanticAction::Picker(_)
            | SemanticAction::CommandBar(_)
            | SemanticAction::ToggleSidebar
            | SemanticAction::ResizeSidebar { .. }
            | SemanticAction::Sidebar(_)
            | SemanticAction::FocusAdjacent(_)
            | SemanticAction::FocusPrefix
            | SemanticAction::CancelFocusPrefix
            | SemanticAction::CycleTab { .. }
            | SemanticAction::Quit
            | SemanticAction::Prompt(_)
            | SemanticAction::Settings(_)
            | SemanticAction::CloseAbout
            | SemanticAction::ToggleAgent
            | SemanticAction::Agent(_)
            | SemanticAction::Dismiss => return false,
            SemanticAction::Save => {
                self.message =
                    Some("The preview is read-only. Save the Markdown file instead".to_string());
            }
            _ => self.message = Some("The preview is read-only".to_string()),
        }
        true
    }

    /// Refuses editor commands aimed at the preview window.
    pub(super) fn check_preview_command(&self, id: CommandId) -> Result<(), String> {
        if self.is_preview(self.active)
            && !(self.explorer.open && self.explorer.focused)
            && !allowed_in_preview(id)
        {
            return Err("The preview is read-only".to_string());
        }
        Ok(())
    }

    /// Before a file opens into the focused window, move off a preview so the file
    /// lands beside its source instead of inside the preview.
    pub(super) fn leave_preview_window(&mut self) {
        let Some(preview) = self.preview(self.active) else {
            return;
        };
        if let Some(pane) = self.pane_of(preview.source) {
            self.active = pane;
        }
    }
}
