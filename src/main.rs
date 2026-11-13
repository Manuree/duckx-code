use std::env;
use std::io::{self, Stdout};
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::{
    cursor::SetCursorStyle,
    event::{
        DisableBracketedPaste, EnableBracketedPaste, EventStream, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, SetTitle, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use duckx_code::{
    app::App,
    buffer::Buffer,
    explorer::Explorer,
    input,
    jobs::{self, AppEvent},
    key_burst::{self, KeyBurst},
    lsp,
    services::{
        clipboard::Clipboard,
        encoding::{self, OpenError},
        git,
        watch::{self, RootWatch},
    },
    session, ui,
};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use tokio::sync::mpsc;

type Tui = Terminal<CrosstermBackend<Stdout>>;

fn init_panic_hook() {
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let _ = restore_terminal();
        original_hook(panic_info);
    }));
}

fn setup_terminal() -> io::Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableBracketedPaste,
        SetCursorStyle::SteadyBlock
    )?;

    // Ask for every modifier with the kitty keyboard protocol when the terminal answers
    // its query (always "no" on Windows, where console input records already carry them).
    let kitty = supports_keyboard_enhancement().unwrap_or(false)
        && execute!(
            stdout,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            )
        )
        .is_ok();
    KITTY_KEYS.store(kitty, Ordering::SeqCst);

    let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    // After the alternate screen, so a host that clears the title on entry still shows it.
    show_app_title();
    Ok(terminal)
}

fn restore_terminal() -> io::Result<()> {
    let mut stdout = io::stdout();
    // Undo only what setup turned on: a terminal without the kitty protocol may read
    // the pop sequence (`CSI < 1 u`) as "restore cursor".
    if KITTY_KEYS.swap(false, Ordering::SeqCst) {
        let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(
        stdout,
        SetCursorStyle::DefaultUserShape,
        DisableBracketedPaste,
        LeaveAlternateScreen
    );
    restore_app_title();
    let _ = disable_raw_mode();
    Ok(())
}

/// Set by `setup_terminal` when it pushed kitty keyboard flags, so quit and the panic
/// hook pop them only then.
static KITTY_KEYS: AtomicBool = AtomicBool::new(false);

/// Name the terminal shows on this tab while the editor is open.
const APP_TITLE: &str = env!("CARGO_PKG_NAME");

/// Title to put back on quit, including after a panic. `None` means it could not be read.
static PREVIOUS_CONSOLE_TITLE: OnceLock<Option<String>> = OnceLock::new();

fn show_app_title() {
    let _ = PREVIOUS_CONSOLE_TITLE.set(read_console_title());
    apply_console_title(APP_TITLE);
}

fn restore_app_title() {
    let Some(Some(title)) = PREVIOUS_CONSOLE_TITLE.get() else {
        return;
    };
    apply_console_title(title);
}

fn apply_console_title(title: &str) {
    let title = osc_title(title);
    #[cfg(windows)]
    let _ = set_console_title_winapi(&title);
    // OSC 0. On Windows 10+ this is what crossterm writes; ConPTY forwards it to the tab.
    let _ = execute!(io::stdout(), SetTitle(title));
}

/// Drops bytes that would end an OSC 0 title sequence early.
fn osc_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| !matches!(c, '\u{0}' | '\u{1b}' | '\u{07}' | '\u{9c}'))
        .collect()
}

fn read_console_title() -> Option<String> {
    #[cfg(windows)]
    {
        read_console_title_winapi()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
fn read_console_title_winapi() -> Option<String> {
    unsafe extern "system" {
        fn GetConsoleTitleW(buffer: *mut u16, size: u32) -> u32;
    }

    let mut buf = vec![0u16; 4096];
    let n = unsafe { GetConsoleTitleW(buf.as_mut_ptr(), buf.len() as u32) };
    if n == 0 {
        return None;
    }
    let n = (n as usize).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]))
}

#[cfg(windows)]
fn set_console_title_winapi(title: &str) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    unsafe extern "system" {
        fn SetConsoleTitleW(title: *const u16) -> i32;
    }

    let wide: Vec<u16> = std::ffi::OsStr::new(title)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let ok = unsafe { SetConsoleTitleW(wide.as_ptr()) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn load_app(path: Option<PathBuf>) -> io::Result<App> {
    let mut app = match &path {
        Some(path) => match encoding::read_auto(path) {
            Ok(decoded) => {
                let mut app = App::new(Buffer::from_text(&decoded.text), Some(path.clone()));
                app.note_opened(decoded.encoding, decoded.stamp);
                app
            }
            Err(OpenError::Missing) => App::new(Buffer::new(), Some(path.clone())),
            Err(OpenError::Undecoded(message)) => {
                let mut app = App::new(Buffer::new(), Some(path.clone()));
                app.mark_unreadable(message);
                app
            }
            Err(OpenError::Failed(message)) => return Err(io::Error::other(message)),
        },
        None => App::new(Buffer::new(), None),
    };
    let opened_parent = path.as_ref().and_then(|path| {
        std::path::absolute(path)
            .ok()
            .and_then(|parent| parent.parent().map(Path::to_path_buf))
    });

    let config = duckx_code::theme::config_dir();
    app.load_appearance(&config);
    app.load_sessions(&session::file_in(&config));
    app.clipboard = Clipboard::system();
    if let Ok(cwd) = env::current_dir() {
        app.explorer = Explorer::new(cwd.clone());
        if path.is_none()
            && let Some(saved) = app.saved_session(&cwd)
        {
            app.restore_session(saved);
        }
    }
    let branch_dir = if path.is_some() {
        opened_parent.or_else(|| env::current_dir().ok())
    } else {
        env::current_dir().ok()
    };
    app.branch = branch_dir.as_deref().and_then(git::current_branch);
    Ok(app)
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let path = env::args_os().nth(1).map(PathBuf::from);
    let mut app = load_app(path)?;

    init_panic_hook();
    let mut terminal = setup_terminal()?;
    let result = run_loop(&mut terminal, &mut app).await;
    persist_session(&app).await;
    restore_terminal()?;
    result
}

/// Writes this folder's entry in `session.json` after the frame loop.
/// Other folders already in the file stay there.
async fn persist_session(app: &App) {
    let Some(dir) = app.appearance.config_dir().map(Path::to_path_buf) else {
        return;
    };
    let Some(session) = app.session_snapshot() else {
        return;
    };
    let path = session::file_in(&dir);
    let _ = tokio::task::spawn_blocking(move || session::upsert(&path, &session)).await;
}

struct FolderWatch {
    root: PathBuf,
    watcher: Option<RootWatch>,
    failed: bool,
}

/// Starts a watch when the workspace folder changes. A folder that cannot be watched
/// stays on the one-second refresh until the folder changes again.
fn sync_folder_watch(
    slot: &mut Option<FolderWatch>,
    root: &Path,
    tx: &mpsc::UnboundedSender<AppEvent>,
    due: &mut Option<tokio::time::Instant>,
) {
    let same = slot
        .as_ref()
        .is_some_and(|watch| session::same_directory(&watch.root, root));
    if same {
        return;
    }
    *due = None;
    match watch::watch_root(root, tx.clone()) {
        Ok(watcher) => {
            *slot = Some(FolderWatch {
                root: root.to_path_buf(),
                watcher: Some(watcher),
                failed: false,
            });
        }
        Err(()) => {
            *slot = Some(FolderWatch {
                root: root.to_path_buf(),
                watcher: None,
                failed: true,
            });
        }
    }
}

async fn run_loop(terminal: &mut Tui, app: &mut App) -> io::Result<()> {
    let (lsp, mut lsp_events) =
        lsp::client::spawn(Some(duckx_code::theme::config_dir().join("logs")));
    let (agent_tx, mut agent_events) = duckx_code::agent::spawn();
    app.attach_agent(agent_tx);
    let result = drive(terminal, app, &lsp, &mut lsp_events, &mut agent_events).await;
    lsp.shutdown(Duration::from_secs(2)).await;
    result
}

/// Route one terminal event to the focused surface. Returns true when it did something.
fn route_event(
    app: &mut App,
    event: &crossterm::event::Event,
    paste_burst: &mut duckx_code::agent::PasteBurst,
) -> bool {
    let action = if app.pane_select.is_some() {
        input::route_pane_select(event, app.keys())
    } else {
        input::route_flags(
            event,
            app.focus(),
            app.explorer.selecting,
            app.filtering(),
            app.keys(),
            app.capturing(),
            app.drafting(),
            paste_burst,
        )
    };
    let Some(action) = action else {
        return false;
    };
    app.handle(action);
    true
}

async fn drive(
    terminal: &mut Tui,
    app: &mut App,
    lsp: &lsp::client::LspHandle,
    lsp_events: &mut mpsc::UnboundedReceiver<lsp::LspEvent>,
    agent_events: &mut mpsc::UnboundedReceiver<duckx_code::agent::AgentEvent>,
) -> io::Result<()> {
    let mut events = EventStream::new();
    let (job_results, mut job_events) = mpsc::unbounded_channel::<AppEvent>();
    let (fs_tx, mut fs_events) = mpsc::unbounded_channel::<AppEvent>();
    let mut folder_watch: Option<FolderWatch> = None;
    let mut fs_due: Option<tokio::time::Instant> = None;
    let mut watch = tokio::time::interval(Duration::from_secs(1));
    watch.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // First frame and every input/job/LSP/agent/fs/spinner change must redraw.
    // Idle ticks that queue nothing must not.
    let mut needs_draw = true;
    let mut last_explorer_height: Option<usize> = None;
    let mut paste_burst = duckx_code::agent::PasteBurst::default();
    let mut burst = KeyBurst::default();
    while !app.should_quit {
        sync_folder_watch(&mut folder_watch, &app.explorer.root, &fs_tx, &mut fs_due);
        let queued = app.take_jobs();
        if !queued.is_empty() {
            needs_draw = true;
        }
        for job in queued {
            let job_results = job_results.clone();
            tokio::task::spawn_blocking(move || {
                let _ = job_results.send(jobs::run(job));
            });
        }
        for request in app.take_lsp_requests() {
            lsp.send(request);
        }
        if app.flush_notice() {
            needs_draw = true;
        }

        if needs_draw {
            let size = terminal.size()?;
            app.set_screen_size(size.width, size.height);
            let area = Rect::new(0, 0, size.width, size.height);
            let layout = ui::frame_layout(app, area);
            app.resize_panes(&layout.pane_sizes());
            let tree_height = ui::sidebar_tree_height(layout.sidebar);
            app.explorer.height = tree_height;
            if last_explorer_height != Some(tree_height) {
                app.explorer.scroll_into_view();
                last_explorer_height = Some(tree_height);
            }
            ui::draw(terminal, app)?;
            needs_draw = false;
        }

        let snack_at = app.snackbar_deadline().map(|until| {
            tokio::time::Instant::now() + until.saturating_duration_since(std::time::Instant::now())
        });

        let paste_at = burst.deadline().map(|due| {
            tokio::time::Instant::now() + due.saturating_duration_since(std::time::Instant::now())
        });

        tokio::select! {
            event = events.next() => {
                let Some(event) = event else {
                    break;
                };
                // Drain what is already queued so a Windows paste (one key per character)
                // is seen as one batch. See `key_burst`.
                let mut batch = vec![event?];
                while batch.len() < key_burst::MAX_DRAIN
                    && crossterm::event::poll(Duration::ZERO)?
                {
                    batch.push(crossterm::event::read()?);
                }
                for event in burst.push(batch, std::time::Instant::now()) {
                    needs_draw |= route_event(app, &event, &mut paste_burst);
                }
            }
            _ = tokio::time::sleep_until(paste_at.unwrap_or_else(tokio::time::Instant::now)), if paste_at.is_some() => {
                if let Some(event) = burst.flush() {
                    needs_draw |= route_event(app, &event, &mut paste_burst);
                }
            }
            Some(event) = job_events.recv() => {
                app.handle_event(event);
                while let Ok(more) = job_events.try_recv() {
                    app.handle_event(more);
                }
                needs_draw = true;
            }
            Some(event) = lsp_events.recv() => {
                app.handle_lsp(event);
                while let Ok(more) = lsp_events.try_recv() {
                    app.handle_lsp(more);
                }
                needs_draw = true;
            }
            Some(event) = agent_events.recv() => {
                app.handle_agent(event);
                while let Ok(more) = agent_events.try_recv() {
                    app.handle_agent(more);
                }
                needs_draw = true;
            }
            Some(event) = fs_events.recv() => {
                match event {
                    AppEvent::FsChanged { root } => {
                        let live = folder_watch.as_ref().is_some_and(|watch| {
                            !watch.failed && session::same_directory(&watch.root, &root)
                        });
                        if live {
                            fs_due = Some(tokio::time::Instant::now() + Duration::from_millis(200));
                        }
                    }
                    AppEvent::FsWatchFailed { root } => {
                        if folder_watch
                            .as_ref()
                            .is_some_and(|watch| session::same_directory(&watch.root, &root))
                        {
                            if let Some(watch) = folder_watch.as_mut() {
                                watch.watcher = None;
                                watch.failed = true;
                            }
                            fs_due = None;
                        }
                    }
                    other => {
                        app.handle_event(other);
                        needs_draw = true;
                    }
                }
            }
            _ = tokio::time::sleep_until(
                fs_due.unwrap_or(tokio::time::Instant::now() + Duration::from_secs(86_400)),
            ), if fs_due.is_some() => {
                fs_due = None;
                let root = app.explorer.root.clone();
                app.note_fs_changed(&root);
                // Queued ScanTree/GitStatus must paint (loading / new rows) without a keypress.
                needs_draw = true;
            }
            _ = watch.tick() => {
                let stats = app.watch_files();
                let mut tree = false;
                if folder_watch.as_ref().is_some_and(|watch| watch.failed) {
                    let root = app.explorer.root.clone();
                    app.note_fs_changed(&root);
                    tree = true;
                }
                if stats > 0 || tree {
                    needs_draw = true;
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(80)), if app.agent.busy => {
                app.advance_agent_spin();
                needs_draw = true;
            }
            _ = tokio::time::sleep_until(snack_at.unwrap_or_else(tokio::time::Instant::now)), if snack_at.is_some() => {
                app.expire_snackbar();
                needs_draw = true;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{APP_TITLE, osc_title};

    #[test]
    fn app_title_is_the_package_name_and_safe_for_the_terminal() {
        assert_eq!(APP_TITLE, "duckx-code");
        assert_eq!(osc_title(APP_TITLE), "duckx-code");
        assert_eq!(osc_title("a\u{1b}]0;x\u{7}b"), "a]0;xb");
    }

    #[cfg(windows)]
    #[test]
    fn windows_console_title_round_trip() {
        let Some(previous) = super::read_console_title() else {
            return;
        };
        struct PutBack(String);
        impl Drop for PutBack {
            fn drop(&mut self) {
                let _ = super::set_console_title_winapi(&self.0);
            }
        }
        let _put_back = PutBack(previous.clone());
        let probe = "duckx-code-title-probe";
        super::set_console_title_winapi(probe).expect("set title");
        let got = super::read_console_title();
        assert_eq!(got.as_deref(), Some(probe));
        drop(_put_back);
        assert_eq!(
            super::read_console_title().as_deref(),
            Some(previous.as_str())
        );
    }
}
