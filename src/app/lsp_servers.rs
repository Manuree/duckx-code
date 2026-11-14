//! Settings → Language servers: one row per server with where its program was
//! found and how it is running, On/Off, restart, and the custom server fields.

use std::path::PathBuf;

use super::App;
use crate::lsp::ServerStatus;
use crate::lsp::registry;
use crate::settings::{
    CustomServer, ServerRow, SettingId, join_args, split_args, split_extensions, split_list,
};

/// Where a server's program is, as the row shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// The background search has not answered yet.
    Checking,
    At(PathBuf),
    Missing,
}

/// The dot in front of a server row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dot {
    /// On, and found on `PATH` or already running: ready to use.
    Ready,
    /// On and starting.
    Starting,
    /// On and stopped for good this session.
    Failed,
    /// Off, not on `PATH`, still being looked for, or no command yet.
    Idle,
}

/// What one Language servers row shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerView {
    pub name: String,
    pub command: String,
    pub extensions: String,
    pub on: bool,
    pub custom: bool,
    /// The custom server's fields are open below.
    pub editing: bool,
    pub found: Found,
    pub status: Option<ServerStatus>,
    /// Install hint, shown only when the program was not found.
    pub install: String,
}

impl ServerView {
    /// Off always reads as idle, whatever was found or last ran.
    pub fn dot(&self) -> Dot {
        if !self.on {
            return Dot::Idle;
        }
        match &self.status {
            Some(ServerStatus::Ready | ServerStatus::Busy(_)) => Dot::Ready,
            Some(ServerStatus::Starting) => Dot::Starting,
            Some(ServerStatus::Failed(_)) => Dot::Failed,
            None => match self.found {
                Found::At(_) => Dot::Ready,
                Found::Checking | Found::Missing => Dot::Idle,
            },
        }
    }

    /// The second line: where it is, then how it is doing.
    pub fn detail(&self) -> String {
        let place = if self.command.is_empty() {
            "needs a command and a file extension · e edits".to_string()
        } else if self.extensions.is_empty() {
            "needs a file extension · e edits".to_string()
        } else {
            match &self.found {
                Found::Checking => format!("{} · looking on PATH…", self.command),
                Found::At(path) => path.display().to_string(),
                Found::Missing if self.install.is_empty() => {
                    format!("{} not found on PATH", self.command)
                }
                Found::Missing => {
                    format!("{} not found on PATH · {}", self.command, self.install)
                }
            }
        };
        let state = match (&self.status, self.on) {
            (_, false) => Some("off".to_string()),
            (Some(ServerStatus::Starting), _) => Some("starting".to_string()),
            (Some(ServerStatus::Ready), _) => Some("running".to_string()),
            (Some(ServerStatus::Busy(work)), _) => Some(work.clone()),
            (Some(ServerStatus::Failed(reason)), _) => Some(format!(
                "stopped: {}",
                reason.lines().next().unwrap_or("exited")
            )),
            (None, true) => None,
        };
        match state {
            Some(state) => format!("{place} · {state}"),
            None => place,
        }
    }
}

impl App {
    /// The server a Language servers row stands for.
    pub fn server_view(&self, row: usize) -> Option<ServerView> {
        let prefs = &self.appearance.lsp;
        let (name, command, extensions, install, custom) = match prefs.server_row(row)? {
            ServerRow::Builtin(index) => {
                let spec = &registry::builtins()[index];
                (
                    spec.name.clone(),
                    spec.command.clone(),
                    spec.extensions_label(),
                    spec.install.clone(),
                    false,
                )
            }
            ServerRow::Custom(index) => {
                let server = &prefs.custom[index];
                (
                    server.name.clone(),
                    server.command.trim().to_string(),
                    server.extensions.join(", "),
                    String::new(),
                    true,
                )
            }
        };
        let found = match self.lsp_found.get(&command) {
            _ if command.is_empty() => Found::Missing,
            Some(Some(path)) => Found::At(path.clone()),
            Some(None) => Found::Missing,
            None => Found::Checking,
        };
        let status = self
            .lsp_servers
            .values()
            .filter(|(running, _)| *running == name)
            .map(|(_, status)| status.clone())
            .max_by_key(status_rank);
        Some(ServerView {
            on: prefs.server_enabled(&name),
            editing: custom && self.lsp_editing.as_deref() == Some(name.as_str()),
            name,
            command,
            extensions,
            custom,
            found,
            status,
            install,
        })
    }

    /// Looks for every server program on `PATH` in the background. `force` looks
    /// again for ones already found or missing.
    pub(super) fn find_lsp_programs(&mut self, force: bool) {
        let mut commands: Vec<String> = registry::builtins()
            .iter()
            .map(|spec| spec.command.clone())
            .chain(
                self.appearance
                    .lsp
                    .custom
                    .iter()
                    .map(|server| server.command.trim().to_string()),
            )
            .filter(|command| !command.is_empty())
            .collect();
        commands.sort();
        commands.dedup();
        self.find_programs(commands, force);
    }

    fn find_programs(&mut self, commands: Vec<String>, force: bool) {
        let wanted: Vec<String> = commands
            .into_iter()
            .filter(|command| !self.lsp_finding.contains(command))
            .filter(|command| force || !self.lsp_found.contains_key(command))
            .collect();
        if wanted.is_empty() {
            return;
        }
        for command in &wanted {
            if force {
                self.lsp_found.remove(command);
            }
            self.lsp_finding.insert(command.clone());
        }
        self.jobs.push(crate::jobs::Job::FindPrograms(wanted));
    }

    pub(super) fn programs_found(&mut self, found: Vec<(String, Option<PathBuf>)>) {
        for (command, path) in found {
            self.lsp_finding.remove(&command);
            self.lsp_found.insert(command, path);
        }
    }

    /// `Enter` on a server row: On ↔ Off. Off stops it at once; the next server
    /// for its files takes over. On lets the next sync start it.
    pub(super) fn toggle_server_row(&mut self, row: usize) {
        let Some(view) = self.server_view(row) else {
            return;
        };
        let on = !view.on;
        self.appearance.lsp.set_server_enabled(&view.name, on);
        if !on {
            self.stop_lsp_server(&view.name, false);
        }
        self.lsp_prefs_dirty = true;
        self.queue_settings_save();
        self.message = Some(format!(
            "{} {}",
            view.name,
            if on { "on" } else { "off; stopped" }
        ));
    }

    /// `r` on a server row: look for the program again and restart the server.
    pub(super) fn restart_server_row(&mut self, row: usize) {
        let Some(view) = self.server_view(row) else {
            return;
        };
        if !view.command.is_empty() {
            self.find_programs(vec![view.command.clone()], true);
        }
        self.restart_lsp_server(&view.name);
    }

    /// Stops `name` and opens its files again, looking for missing programs anew.
    fn restart_lsp_server(&mut self, name: &str) {
        if !self.appearance.lsp.enabled {
            self.message = Some("Language servers are off in Settings".to_string());
            return;
        }
        if !self.appearance.lsp.server_enabled(name) {
            self.message = Some(format!("{name} is off; Enter turns it on"));
            return;
        }
        self.stop_lsp_server(name, true);
        self.message = Some(format!("Restarting {name}"));
    }

    /// `lsp-restart`: restarts the server of the focused file.
    pub(super) fn restart_current_server(&mut self) {
        let Some(path) = self.path().map(std::path::Path::to_path_buf) else {
            self.message = Some("This file has no language server".to_string());
            return;
        };
        let Some(group) = self.lsp_registry.group(&path) else {
            self.message = Some("This file has no language server".to_string());
            return;
        };
        let name = self
            .lsp_servers
            .get(&group)
            .map(|(running, _)| running.clone())
            .unwrap_or(group);
        if let Some(spec) = self.lsp_registry.find(&name) {
            let command = spec.command.clone();
            self.find_programs(vec![command], true);
        }
        self.restart_lsp_server(&name);
    }

    /// `Delete` on a server row: a built-in goes back to On; a custom server is
    /// deleted after a second `Delete`.
    pub(super) fn reset_server_row(&mut self, row: usize) {
        let Some(view) = self.server_view(row) else {
            return;
        };
        let armed = self.server_delete_armed.take();
        if !view.custom {
            let was_off = !view.on;
            self.appearance.lsp.set_server_enabled(&view.name, true);
            self.lsp_prefs_dirty = true;
            if was_off {
                self.queue_settings_save();
            }
            self.message = Some(format!("Restored {}", view.name));
            return;
        }
        if armed.as_deref() != Some(view.name.as_str()) {
            self.message = Some(format!("Press Delete again to delete {}", view.name));
            self.server_delete_armed = Some(view.name);
            return;
        }
        let Some(index) = self.appearance.lsp.find_custom(&view.name) else {
            return;
        };
        self.stop_lsp_server(&view.name, false);
        self.appearance.lsp.delete_custom(index);
        if self.lsp_editing.as_deref() == Some(view.name.as_str()) {
            self.lsp_editing = None;
        }
        self.lsp_prefs_dirty = true;
        self.queue_settings_save();
        self.settle_settings_row();
        self.message = Some(format!("Deleted {}", view.name));
    }

    /// `e` on a custom server row opens its fields; on a built-in it says why not.
    pub(super) fn edit_server_row(&mut self, row: usize) {
        let Some(view) = self.server_view(row) else {
            return;
        };
        if !view.custom {
            self.message = Some(format!(
                "{} is built in; Add server makes your own",
                view.name
            ));
            return;
        }
        self.lsp_editing = Some(view.name.clone());
        self.jump_to_setting(SettingId::LspCustomCommand);
        self.message = Some(format!("Editing {}", view.name));
    }

    /// Moves the settings highlight onto `id` in the open section.
    fn jump_to_setting(&mut self, id: SettingId) {
        if let Some(page) = self.settings.as_mut()
            && let Some(row) = page.section().items.iter().position(|item| item.id == id)
        {
            page.row = row;
        }
    }

    /// The custom server whose fields are open.
    pub(crate) fn editing_custom(&self) -> Option<(usize, &CustomServer)> {
        let name = self.lsp_editing.as_deref()?;
        let index = self.appearance.lsp.find_custom(name)?;
        Some((index, &self.appearance.lsp.custom[index]))
    }

    /// The custom server fields are shown only while one is being edited (R5).
    pub(super) fn server_field_visible(&self, id: SettingId) -> bool {
        match id {
            SettingId::LspCustomName
            | SettingId::LspCustomCommand
            | SettingId::LspCustomArgs
            | SettingId::LspCustomExtensions
            | SettingId::LspCustomLanguageId
            | SettingId::LspCustomRootMarkers => self.editing_custom().is_some(),
            _ => true,
        }
    }

    /// The text a custom server field starts with when it is opened.
    pub(super) fn server_field_text(&self, id: SettingId) -> Option<String> {
        if id == SettingId::LspAddServer {
            return Some(String::new());
        }
        let (_, server) = self.editing_custom()?;
        Some(match id {
            SettingId::LspCustomName => server.name.clone(),
            SettingId::LspCustomCommand => server.command.clone(),
            SettingId::LspCustomArgs => join_args(&server.args),
            SettingId::LspCustomExtensions => server.extensions.join(", "),
            SettingId::LspCustomLanguageId => server.language_id.clone(),
            SettingId::LspCustomRootMarkers => server.root_markers.join(", "),
            _ => return None,
        })
    }

    /// What a custom server field row shows.
    pub fn server_field_label(&self, id: SettingId) -> String {
        if id == SettingId::LspAddServer {
            return "enter, then type a name".to_string();
        }
        let Some((_, server)) = self.editing_custom() else {
            return String::new();
        };
        let or = |text: String, empty: &str| {
            if text.trim().is_empty() {
                empty.to_string()
            } else {
                text
            }
        };
        match id {
            SettingId::LspCustomName => server.name.clone(),
            SettingId::LspCustomCommand => or(server.command.clone(), "(required)"),
            SettingId::LspCustomArgs => or(join_args(&server.args), "(none)"),
            SettingId::LspCustomExtensions => or(server.extensions.join(", "), "(required)"),
            SettingId::LspCustomLanguageId => match server.language_id.trim() {
                "" => match server.extensions.first() {
                    Some(ext) => format!("{ext} (first extension)"),
                    None => "(first extension)".to_string(),
                },
                id => id.to_string(),
            },
            SettingId::LspCustomRootMarkers => or(server.root_markers.join(", "), "(none)"),
            _ => String::new(),
        }
    }

    /// Validates and stores Add server or a custom server field. `Ok(true)` when it changed.
    pub(super) fn store_server_field(
        &mut self,
        id: SettingId,
        value: String,
    ) -> Result<bool, String> {
        if id == SettingId::LspAddServer {
            if value.is_empty() {
                return Ok(false);
            }
            self.appearance.lsp.add_custom(&value)?;
            self.lsp_editing = Some(value.clone());
            self.jump_to_setting(SettingId::LspCustomCommand);
            self.message = Some(format!(
                "Added {value}; set its command and file extensions below"
            ));
            self.lsp_prefs_dirty = true;
            return Ok(true);
        }
        let Some((index, _)) = self.editing_custom() else {
            return Ok(false);
        };
        let lsp = &mut self.appearance.lsp;
        let server = lsp.custom[index].clone();
        let mut next = server.clone();
        match id {
            SettingId::LspCustomName => {
                let changed = lsp.rename_custom(index, &value)?;
                if changed {
                    self.lsp_editing = Some(value);
                    self.lsp_prefs_dirty = true;
                }
                return Ok(changed);
            }
            SettingId::LspCustomCommand => {
                if value.is_empty() {
                    return Err("A server needs a command".to_string());
                }
                if value.chars().any(char::is_control) {
                    return Err("A command is one line of text".to_string());
                }
                next.command = value;
            }
            SettingId::LspCustomArgs => next.args = split_args(&value)?,
            SettingId::LspCustomExtensions => {
                let extensions = split_extensions(&value);
                if extensions.is_empty() {
                    return Err("A server needs at least one file extension".to_string());
                }
                next.extensions = extensions;
            }
            SettingId::LspCustomLanguageId => {
                if value.chars().any(char::is_whitespace) {
                    return Err("A language id has no spaces".to_string());
                }
                next.language_id = value;
            }
            SettingId::LspCustomRootMarkers => next.root_markers = split_list(&value),
            _ => return Ok(false),
        }
        if next == server {
            return Ok(false);
        }
        let command = next.command.trim().to_string();
        lsp.custom[index] = next;
        if !command.is_empty() {
            self.find_programs(vec![command], true);
        }
        // Start it again with the new values, and look for a missing program anew.
        self.stop_lsp_server(&server.name, true);
        self.lsp_prefs_dirty = true;
        Ok(true)
    }
}

/// Which of several copies' states a row shows: running beats starting beats stopped.
fn status_rank(status: &ServerStatus) -> u8 {
    match status {
        ServerStatus::Ready | ServerStatus::Busy(_) => 3,
        ServerStatus::Starting => 2,
        ServerStatus::Failed(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::action::{SemanticAction, SettingsAction};
    use crate::buffer::Buffer;
    use crate::jobs::Job;
    use crate::lsp::{LspEvent, LspRequest};
    use crate::settings::{Page, sections};

    fn file_app(name: &str) -> App {
        App::new(Buffer::from_text("x\n"), Some(PathBuf::from(name)))
    }

    fn servers_section() -> usize {
        sections()
            .iter()
            .position(|section| section.name == "Language servers")
            .unwrap()
    }

    /// Settings open inside Language servers with `id` highlighted.
    fn open_at(app: &mut App, id: SettingId) {
        let mut page = Page::new();
        page.section = servers_section();
        page.inside = true;
        page.row = page
            .section()
            .items
            .iter()
            .position(|item| item.id == id)
            .unwrap();
        app.settings = Some(page);
    }

    fn slot_of(app: &App, name: &str) -> SettingId {
        let slots = registry::builtins().len() + crate::settings::MAX_CUSTOM_SERVERS;
        (0..slots)
            .map(SettingId::LspServer)
            .find(|id| match id {
                SettingId::LspServer(row) => {
                    app.appearance.lsp.server_row_name(*row).as_deref() == Some(name)
                }
                _ => false,
            })
            .unwrap()
    }

    fn open_row(app: &mut App, name: &str) {
        let id = slot_of(app, name);
        open_at(app, id);
    }

    fn press(app: &mut App, action: SettingsAction) {
        app.handle(SemanticAction::Settings(action));
    }

    fn type_in(app: &mut App, text: &str) {
        press(app, SettingsAction::Enter);
        assert!(
            app.settings.as_ref().unwrap().text.is_some(),
            "{:?}",
            app.message
        );
        press(app, SettingsAction::Insert(text.into()));
        press(app, SettingsAction::Enter);
    }

    /// Like [`type_in`], replacing what the row held.
    fn retype(app: &mut App, text: &str) {
        press(app, SettingsAction::Enter);
        press(app, SettingsAction::Cursor(crate::command::LineMove::End));
        for _ in 0..64 {
            press(app, SettingsAction::DeleteChar);
        }
        press(app, SettingsAction::Insert(text.into()));
        press(app, SettingsAction::Enter);
    }

    fn running(app: &mut App, group: &str, name: &str) {
        app.handle_lsp(LspEvent::Status {
            group: group.into(),
            name: name.into(),
            status: ServerStatus::Ready,
        });
    }

    fn first_server(requests: &[LspRequest]) -> Option<String> {
        requests.iter().find_map(|request| match request {
            LspRequest::Open { servers, .. } => servers.first().map(|spec| spec.name.clone()),
            _ => None,
        })
    }

    fn highlighted(app: &App) -> SettingId {
        let page = app.settings.as_ref().unwrap();
        page.section().items[page.row].id
    }

    #[test]
    fn switching_a_server_off_stops_it_and_on_starts_it_again() {
        let mut app = file_app("main.rs");
        assert_eq!(
            first_server(&app.take_lsp_requests()).as_deref(),
            Some("rust-analyzer")
        );
        running(&mut app, "rust-analyzer", "rust-analyzer");
        open_row(&mut app, "rust-analyzer");
        press(&mut app, SettingsAction::Enter);
        assert!(!app.appearance.lsp.server_enabled("rust-analyzer"));
        assert!(app.server_view(0).is_some_and(|view| !view.on));
        let requests = app.take_lsp_requests();
        assert!(
            requests.iter().any(
                |request| matches!(request, LspRequest::Stop { name } if name == "rust-analyzer")
            ),
            "{requests:?}"
        );
        assert!(
            requests
                .iter()
                .any(|request| matches!(request, LspRequest::Close { .. }))
        );
        assert_eq!(first_server(&requests), None, "no other server takes .rs");
        assert!(app.lsp_label().is_none());

        press(&mut app, SettingsAction::Enter);
        assert!(app.appearance.lsp.server_enabled("rust-analyzer"));
        assert_eq!(
            first_server(&app.take_lsp_requests()).as_deref(),
            Some("rust-analyzer")
        );
    }

    #[test]
    fn the_next_python_server_takes_over_when_the_first_is_switched_off() {
        let mut app = file_app("hello.py");
        assert_eq!(
            first_server(&app.take_lsp_requests()).as_deref(),
            Some("pyright")
        );
        running(&mut app, "pyright", "pyright");
        open_row(&mut app, "pyright");
        press(&mut app, SettingsAction::Enter);
        let requests = app.take_lsp_requests();
        assert!(
            requests
                .iter()
                .any(|request| matches!(request, LspRequest::Stop { name } if name == "pyright"))
        );
        assert_eq!(first_server(&requests).as_deref(), Some("basedpyright"));
    }

    #[test]
    fn r_looks_again_and_restarts_and_lsp_restart_does_the_same_for_the_file() {
        let mut app = file_app("main.rs");
        app.take_lsp_requests();
        app.take_jobs();
        running(&mut app, "rust-analyzer", "rust-analyzer");
        open_row(&mut app, "rust-analyzer");
        press(&mut app, SettingsAction::Restart);
        let jobs = app.take_jobs();
        assert!(
            jobs.iter().any(|job| matches!(
                job,
                Job::FindPrograms(commands) if commands == &["rust-analyzer".to_string()]
            )),
            "{jobs:?}"
        );
        let requests = app.take_lsp_requests();
        assert!(
            matches!(requests.first(), Some(LspRequest::Restart { name }) if name == "rust-analyzer"),
            "{requests:?}"
        );
        assert_eq!(first_server(&requests).as_deref(), Some("rust-analyzer"));
        assert_eq!(app.message.as_deref(), Some("Restarting rust-analyzer"));

        // The command restarts the server running the file, here a fallback.
        app.settings = None;
        let mut app = file_app("hello.py");
        app.take_lsp_requests();
        running(&mut app, "pyright", "basedpyright");
        app.run_command(crate::command::CommandId::LspRestart, None)
            .unwrap();
        let requests = app.take_lsp_requests();
        assert!(
            matches!(requests.first(), Some(LspRequest::Restart { name }) if name == "basedpyright"),
            "{requests:?}"
        );
        assert_eq!(first_server(&requests).as_deref(), Some("pyright"));

        let mut app = file_app("notes.txt");
        app.run_command(crate::command::CommandId::LspRestart, None)
            .unwrap();
        assert_eq!(
            app.message.as_deref(),
            Some("This file has no language server")
        );
    }

    #[test]
    fn the_add_form_validates_and_a_custom_server_takes_its_files() {
        let mut app = file_app("main.foo");
        assert!(app.take_lsp_requests().is_empty(), "no server for .foo yet");
        open_at(&mut app, SettingId::LspAddServer);
        // The fields stay hidden until a custom server is being edited (R5).
        assert!(!app.setting_visible(SettingId::LspCustomCommand));

        type_in(&mut app, "gopls");
        assert_eq!(app.message.as_deref(), Some("gopls is a built-in server"));
        press(&mut app, SettingsAction::Back);
        type_in(&mut app, "foo-ls");
        assert_eq!(app.lsp_editing.as_deref(), Some("foo-ls"));
        assert!(app.setting_visible(SettingId::LspCustomCommand));
        assert_eq!(highlighted(&app), SettingId::LspCustomCommand);
        let requests = app.take_lsp_requests();
        assert_eq!(
            first_server(&requests),
            None,
            "not complete yet: {requests:?}"
        );

        type_in(&mut app, "  ");
        assert_eq!(app.message.as_deref(), Some("A server needs a command"));
        press(&mut app, SettingsAction::Back);
        type_in(&mut app, "foo-ls");
        assert_eq!(app.appearance.lsp.custom[0].command, "foo-ls");

        open_at(&mut app, SettingId::LspCustomArgs);
        type_in(&mut app, "--stdio \"--log=a b");
        assert_eq!(
            app.message.as_deref(),
            Some("Arguments have an unclosed \"")
        );
        press(&mut app, SettingsAction::Back);
        type_in(&mut app, "--stdio \"--log=a b\"");
        assert_eq!(app.appearance.lsp.custom[0].args, ["--stdio", "--log=a b"]);

        open_at(&mut app, SettingId::LspCustomExtensions);
        type_in(&mut app, " , ");
        assert_eq!(
            app.message.as_deref(),
            Some("A server needs at least one file extension")
        );
        press(&mut app, SettingsAction::Back);
        type_in(&mut app, ".FOO, bar");
        assert_eq!(app.appearance.lsp.custom[0].extensions, ["foo", "bar"]);
        assert_eq!(
            app.server_field_label(SettingId::LspCustomLanguageId),
            "foo (first extension)"
        );

        let requests = app.take_lsp_requests();
        let spec = requests
            .iter()
            .find_map(|request| match request {
                LspRequest::Open { servers, .. } => servers.first().cloned(),
                _ => None,
            })
            .expect("the custom server opens main.foo");
        assert_eq!(spec.name, "foo-ls");
        assert_eq!(spec.args, ["--stdio", "--log=a b"]);
        assert_eq!(
            spec.language_id(std::path::Path::new("main.foo")),
            Some("foo")
        );

        open_at(&mut app, SettingId::LspCustomName);
        retype(&mut app, "foo");
        assert_eq!(app.lsp_editing.as_deref(), Some("foo"));
        assert_eq!(app.appearance.lsp.custom[0].name, "foo");
    }

    #[test]
    fn e_opens_a_custom_servers_fields_and_delete_needs_two_presses() {
        let mut app = file_app("main.rs");
        app.appearance.lsp.add_custom("mine").unwrap();
        open_row(&mut app, "rust-analyzer");
        press(&mut app, SettingsAction::Edit);
        assert_eq!(app.lsp_editing, None);
        assert_eq!(
            app.message.as_deref(),
            Some("rust-analyzer is built in; Add server makes your own")
        );

        // Del on a built-in turns it back on.
        app.appearance
            .lsp
            .set_server_enabled("rust-analyzer", false);
        press(&mut app, SettingsAction::Reset);
        assert!(app.appearance.lsp.server_enabled("rust-analyzer"));

        open_row(&mut app, "mine");
        press(&mut app, SettingsAction::Edit);
        assert_eq!(app.lsp_editing.as_deref(), Some("mine"));
        assert_eq!(highlighted(&app), SettingId::LspCustomCommand);

        open_row(&mut app, "mine");
        press(&mut app, SettingsAction::Reset);
        assert_eq!(app.appearance.lsp.custom.len(), 1);
        assert_eq!(
            app.message.as_deref(),
            Some("Press Delete again to delete mine")
        );
        // Any other key disarms it.
        press(&mut app, SettingsAction::Down);
        open_row(&mut app, "mine");
        press(&mut app, SettingsAction::Reset);
        assert_eq!(app.appearance.lsp.custom.len(), 1);
        press(&mut app, SettingsAction::Reset);
        assert!(app.appearance.lsp.custom.is_empty());
        assert_eq!(app.lsp_editing, None);
        assert!(!app.setting_visible(SettingId::LspCustomCommand));
        assert_eq!(app.message.as_deref(), Some("Deleted mine"));
        assert!(
            app.setting_visible(highlighted(&app)),
            "the highlight moved to a shown row"
        );
    }

    #[test]
    fn detection_runs_in_the_background_and_fills_in_each_row() {
        let mut app = file_app("main.rs");
        app.take_jobs();
        app.run_command(crate::command::CommandId::Settings, None)
            .unwrap();
        let jobs = app.take_jobs();
        let commands = jobs
            .iter()
            .find_map(|job| match job {
                Job::FindPrograms(commands) => Some(commands.clone()),
                _ => None,
            })
            .expect("opening Settings looks for the servers");
        assert!(commands.contains(&"zls".to_string()));
        assert!(
            commands.contains(&"pyright-langserver".to_string())
                || commands.contains(&"pyright".to_string())
        );
        let zls = registry::builtins()
            .iter()
            .position(|spec| spec.name == "zls")
            .unwrap();
        let view = app.server_view(zls).unwrap();
        assert_eq!(view.found, Found::Checking);
        assert!(
            view.detail().contains("looking on PATH"),
            "{}",
            view.detail()
        );

        // Already looked for: opening Settings again does not ask twice.
        app.run_command(crate::command::CommandId::Settings, None)
            .unwrap();
        assert!(
            !app.take_jobs()
                .iter()
                .any(|job| matches!(job, Job::FindPrograms(_)))
        );

        app.handle_event(crate::jobs::AppEvent::ProgramsFound(vec![
            ("zls".into(), None),
            (
                "rust-analyzer".into(),
                Some(PathBuf::from("/usr/bin/rust-analyzer")),
            ),
        ]));
        let view = app.server_view(zls).unwrap();
        assert_eq!(view.dot(), Dot::Idle);
        assert!(
            view.detail().starts_with("zls not found on PATH · "),
            "{}",
            view.detail()
        );
        assert!(view.detail().contains(&view.install));
        let rust = app.server_view(0).unwrap();
        assert_eq!(rust.dot(), Dot::Ready);
        assert_eq!(rust.detail(), "/usr/bin/rust-analyzer");
        running(&mut app, "rust-analyzer", "rust-analyzer");
        let rust = app.server_view(0).unwrap();
        assert_eq!(rust.dot(), Dot::Ready);
        assert_eq!(rust.detail(), "/usr/bin/rust-analyzer · running");
    }

    #[test]
    fn the_section_is_hidden_while_the_language_server_is_off() {
        let mut app = file_app("main.rs");
        let shown = |app: &App| Page::shown_sections(|id| app.setting_visible(id));
        assert!(shown(&app).contains(&servers_section()));
        app.appearance.lsp.enabled = false;
        assert!(!shown(&app).contains(&servers_section()));
    }
}
