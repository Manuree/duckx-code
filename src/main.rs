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
    app.clipboard = Clipboard::system();
    if let Ok(cwd) = env::current_dir() {
        app.explorer = Explorer::new(cwd.clone());
        if path.is_none()
            && let Some(saved) = session::for_directory(&session::file_in(&config), &cwd)
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

async fn run_loop(terminal: &mut Tui, app: &mut App) -> io::Result<()> {
    let (lsp, mut lsp_events) =
        lsp::client::spawn(Some(duckx_code::theme::config_dir().join("logs")));
    let result = drive(terminal, app, &lsp, &mut lsp_events).await;
    lsp.shutdown(Duration::from_secs(2)).await;
    result
}

async fn drive(
    terminal: &mut Tui,
    app: &mut App,
    lsp: &lsp::client::LspHandle,
    lsp_events: &mut mpsc::UnboundedReceiver<lsp::LspEvent>,
) -> io::Result<()> {
    let mut events = EventStream::new();
    let (job_results, mut job_events) = mpsc::unbounded_channel::<AppEvent>();
    let mut watch = tokio::time::interval(Duration::from_secs(1));
    watch.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while !app.should_quit {
        for job in app.take_jobs() {
            let job_results = job_results.clone();
            tokio::task::spawn_blocking(move || {
                let _ = job_results.send(jobs::run(job));
            });
        }
        for request in app.take_lsp_requests() {
            lsp.send(request);
        }

        let size = terminal.size()?;
        app.set_screen_size(size.width, size.height);
        let area = Rect::new(0, 0, size.width, size.height);
        let layout = ui::frame_layout(&app, area);
        app.resize_panes(
            &layout
                .panes
                .iter()
                .map(|pane| (pane.id, pane.text.width, pane.text.height))
                .collect::<Vec<_>>(),
        );
        app.explorer.height = ui::sidebar_tree_height(layout.sidebar);
        app.explorer.scroll_into_view();
        terminal.draw(|frame| ui::render(frame, &*app))?;

        tokio::select! {
            event = events.next() => {
                let Some(event) = event else {
                    break;
                };
                if let Some(action) =
                    input::route_flags(
                        &event?,
                        app.focus(),
                        app.explorer.selecting,
                        app.filtering(),
                        app.keys(),
                        app.capturing(),
                    )
                {
                    app.handle(action);
                }
            }
            Some(event) = job_events.recv() => app.handle_event(event),
            Some(event) = lsp_events.recv() => app.handle_lsp(event),
            _ = watch.tick() => app.watch_files(),
        }
    }
    Ok(())
}
