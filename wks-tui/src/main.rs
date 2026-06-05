//! `wks-tui` — a terminal client for workspacer.
//!
//! Talks directly to the **claudemon** daemon's REST + SSE API to monitor every
//! live Claude agent, read its transcript, approve permission prompts, answer
//! questions, send messages, interrupt, and spawn new agents — all with
//! vim-style keys. It needs only claudemon running; the Electron app does not
//! have to be open. (The hub bus's `agents.*` capabilities are registered by
//! the Electron main process, so a standalone client can't rely on them.)

mod app;
mod claudemon;
mod daemons;
mod library;
mod profiles;
mod terminal;
mod types;
mod ui;

use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use crossterm::event::{Event, EventStream, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures_util::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

use app::{App, AppMsg};
use claudemon::DaemonEvent;

#[derive(Parser, Debug)]
#[command(name = "wks-tui", about = "Terminal client for workspacer agents")]
struct Cli {
    /// claudemon REST base URL — the daemon the TUI reads and controls.
    #[arg(long, env = "WKS_CLAUDEMON_URL", default_value = "http://127.0.0.1:7891")]
    claudemon_url: String,

    /// Don't auto-launch claudemon if it isn't already listening. By default a
    /// loopback URL starts it and stops it on exit; pass this to connect only
    /// to a daemon started elsewhere (e.g. the Electron app).
    #[arg(long)]
    no_spawn: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Bring up claudemon if it isn't running, before we take over the screen so
    // diagnostics print normally and the first request lands. The guard stops
    // it again on exit (only if we started it).
    let _daemons = daemons::ensure(&cli.claudemon_url, !cli.no_spawn);

    let claudemon = claudemon::Claudemon::new(cli.claudemon_url.clone());
    let profiles = profiles::load();
    let library = library::load();

    let mut terminal = setup_terminal()?;
    let res = run(&mut terminal, cli.claudemon_url, claudemon, profiles, library).await;
    restore_terminal(&mut terminal)?;
    res
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    events_url: String,
    claudemon: claudemon::Claudemon,
    profiles: Vec<profiles::Profile>,
    library: Vec<library::LibraryItem>,
) -> Result<()> {
    let mut daemon_rx = claudemon::spawn_events(events_url);
    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<AppMsg>();
    let (pty_tx, mut pty_rx) = mpsc::unbounded_channel::<claudemon::PtyChunk>();
    let mut app = App::new(claudemon, profiles, library, msg_tx, pty_tx);

    let mut keys = EventStream::new();
    // A steady tick so toasts expire and the "working…" indicator stays live
    // even when no events arrive.
    let mut tick = tokio::time::interval(Duration::from_millis(1000));

    loop {
        terminal.draw(|f| ui::render(f, &mut app))?;
        // The renderer learns the pane size; push any pending PTY resize now.
        app.flush_term_resize();
        if app.should_quit {
            break;
        }

        tokio::select! {
            ev = keys.next() => match ev {
                Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => app.handle_key(key),
                Some(Ok(_)) => {}
                Some(Err(_)) | None => break,
            },
            de = daemon_rx.recv() => match de {
                Some(DaemonEvent::Connected) => app.on_connected(),
                Some(DaemonEvent::Disconnected) => app.on_disconnected(),
                Some(DaemonEvent::Changed) => app.on_changed(),
                None => break,
            },
            am = msg_rx.recv() => match am {
                Some(msg) => app.apply_msg(msg),
                None => {}
            },
            chunk = pty_rx.recv() => match chunk {
                Some(c) => app.feed_pty(c),
                None => {}
            },
            _ = tick.tick() => {}
        }
    }
    Ok(())
}

// ── terminal lifecycle ────────────────────────────────────────────────────

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    // Restore the terminal even if we panic, so the user isn't left in raw mode.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
