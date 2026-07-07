//! `wks-tui` — a terminal client for workspacer.
//!
//! Talks directly to the **claudemon** daemon's REST + SSE API to monitor every
//! live Claude agent, read its transcript, approve permission prompts, answer
//! questions, send messages, interrupt, and spawn new agents — all with
//! vim-style keys. It needs only claudemon running; the Electron app does not
//! have to be open. (The hub bus's `agents.*` capabilities are registered by
//! the Electron main process, so a standalone client can't rely on them.)

mod app;
mod bus;
mod claudemon;
mod config;
mod daemons;
mod keys;
mod library;
mod names;
mod notes;
mod pins;
mod profiles;
mod render;
mod terminal;
mod theme;
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
    #[arg(
        long,
        env = "WKS_CLAUDEMON_URL",
        default_value = "http://127.0.0.1:7891"
    )]
    claudemon_url: String,

    /// Don't auto-launch claudemon if it isn't already listening. By default a
    /// loopback URL starts it and stops it on exit; pass this to connect only
    /// to a daemon started elsewhere (e.g. the Electron app).
    #[arg(long)]
    no_spawn: bool,

    /// Hub bus URL. By default the TUI is a thin client of the hub's brain
    /// provider — driving, the agent list, and terminals all flow over the bus
    /// (it auto-spawns the hub + brain for a loopback URL). This overrides the
    /// address; pass `--direct` to bypass the bus entirely.
    #[arg(long, env = "WKS_HUB_BUS", default_value = "ws://127.0.0.1:7895/bus")]
    bus: String,

    /// Bypass the hub bus and talk to claudemon directly (the standalone path).
    #[arg(long)]
    direct: bool,

    /// Auth token for the hub bus (when it requires one).
    #[arg(long, env = "HUB_TOKEN")]
    bus_token: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Bus by default; `--direct` opts back into talking to claudemon directly.
    let mut bus_url = (!cli.direct).then(|| cli.bus.clone());

    // When the desktop app is running it owns the hub and guards `/bus` with the
    // token it persists at `~/.config/workspacer/remote-token`. An explicit
    // `--bus-token`/`HUB_TOKEN` wins; otherwise discover that token so the TUI
    // can join a desktop-owned bus instead of being rejected with 401 and
    // hanging in reconnect. Harmless against a token-less hub (it's ignored).
    let bus_token = cli.bus_token.clone().or_else(config::hub_token);

    // Bring up claudemon (and, in bus mode, the hub + brain) if not already
    // running, before we take over the screen. The guard stops what we started.
    let _daemons = daemons::ensure(&cli.claudemon_url, bus_url.as_deref(), !cli.no_spawn);

    // Robustness: if we'd use a loopback bus but nothing's listening (e.g. the
    // hub binary isn't built), fall back to claudemon-direct so the TUI still
    // works. An explicitly-remote bus is the user's responsibility — respected.
    if let Some(url) = bus_url.clone() {
        if daemons::loopback_bus_unreachable(&url, bus_token.as_deref()) {
            eprintln!(
                "[wks-tui] hub bus not usable at {url}; using claudemon directly \
                 (build the hub with `make build-hub`, or run with --direct to silence this)"
            );
            bus_url = None;
        }
    }

    let claudemon = claudemon::Claudemon::new(cli.claudemon_url.clone());
    let profiles = profiles::load();
    let library = library::load();
    let config = config::load();

    // In bus mode the TUI is a thin bus client: driving routes through the brain
    // and terminals stream over pty.bytes. The agent *list* stays claudemon-owned
    // (the bus serves the desktop's enriched, camelCase shape the TUI can't read),
    // so agent.snapshot is just a nudge to re-pull. `--direct` (or an unreachable
    // bus) keeps the pure claudemon path.
    let (bus, bus_events) = match bus_url.as_ref() {
        Some(url) => {
            let (client, events) = bus::BusClient::connect(url.clone(), bus_token.clone());
            let _ = client.subscribe(vec![
                "agent.snapshot".into(),
                "agent.statusline".into(),
                "pty.bytes.*".into(),
            ]);
            (Some(client), Some(events))
        }
        None => (None, None),
    };

    let mut terminal = setup_terminal()?;
    let res = run(
        &mut terminal,
        cli.claudemon_url,
        claudemon,
        bus,
        bus_events,
        profiles,
        library,
        config,
    )
    .await;
    restore_terminal(&mut terminal)?;
    res
}

// Single top-level orchestration entry point wired up once from `main`; the
// arguments are the already-constructed subsystems, not worth a params struct.
#[allow(clippy::too_many_arguments)]
async fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    events_url: String,
    claudemon: claudemon::Claudemon,
    bus: Option<bus::BusClient>,
    mut bus_events: Option<mpsc::UnboundedReceiver<bus::BusEvent>>,
    profiles: Vec<profiles::Profile>,
    library: Vec<library::LibraryItem>,
    config: config::Config,
) -> Result<()> {
    let mut daemon_rx = claudemon::spawn_events(events_url.clone());
    let mut status_rx = claudemon::spawn_status_lines(events_url);
    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<AppMsg>();
    let (pty_tx, mut pty_rx) = mpsc::unbounded_channel::<claudemon::PtyChunk>();
    let mut app = App::new(claudemon, profiles, library, config, msg_tx, pty_tx);
    app.set_bus(bus);
    // Seed the agent list from the bus when in bus mode (works even with no
    // direct claudemon, e.g. a remote hub). Live updates arrive as events below.
    if app.has_bus() {
        app.refresh();
    }

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
            am = msg_rx.recv() => if let Some(msg) = am { app.apply_msg(msg) },
            chunk = pty_rx.recv() => if let Some(c) = chunk { app.feed_pty(c) },
            sl = status_rx.recv() => if let Some(msg) = sl { app.apply_status_line(msg.session_id, msg.status_line) },
            // Live agent view over the bus (snapshots + statusline). The pending()
            // arm never fires when there's no bus, so this is inert off-bus.
            bev = async {
                match bus_events.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => if let Some(ev) = bev { app.apply_bus_event(ev) },
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
