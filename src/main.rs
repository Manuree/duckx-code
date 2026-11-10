use std::env;
use std::io::{self, Stdout};
use std::panic;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::{
    cursor::SetCursorStyle,
    event::{
        DisableBracketedPaste, EnableBracketedPaste, EventStream, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        supports_keyboard_enhancement,
    },
};
use duckx_code::{
    app::App,
    buffer::Buffer,
    explorer::Explorer,
    input,
    jobs::{self, AppEvent},
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

    if supports_keyboard_enhancement().unwrap_or(false) {
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            )
        );
    }

    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore_terminal() -> io::Result<()> {
    let mut stdout = io::stdout();
    let _ = execute!(stdout, PopKeyboardEnhancementFlags);
    let _ = execute!(
        stdout,
        SetCursorStyle::DefaultUserShape,
        DisableBracketedPaste,
        LeaveAlternateScreen
    );
    let _ = disable_raw_mode();
    Ok(())
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

        if needs_draw {
            let size = terminal.size()?;
            app.set_screen_size(size.width, size.height);
            let area = Rect::new(0, 0, size.width, size.height);
            let layout = ui::frame_layout(app, area);
            app.resize_panes(
                &layout
                    .panes
                    .iter()
                    .map(|pane| (pane.id, pane.text.width, pane.text.height))
                    .collect::<Vec<_>>(),
            );
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
            tokio::time::Instant::now()
                + until.saturating_duration_since(std::time::Instant::now())
        });

        tokio::select! {
            event = events.next() => {
                let Some(event) = event else {
                    break;
                };
                let event = event?;
                let action = if app.pane_select.is_some() {
                    input::route_pane_select(&event, app.keys())
                } else {
                    input::route_flags(
                        &event,
                        app.focus(),
                        app.explorer.selecting,
                        app.filtering(),
                        app.keys(),
                        app.capturing(),
                        app.drafting(),
                        &mut paste_burst,
                    )
                };
                if let Some(action) = action {
                    app.handle(action);
                    needs_draw = true;
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
