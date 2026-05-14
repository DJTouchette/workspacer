//! `claudemon watch` — terminal UI for live session monitoring + approvals.
//!
//! Single-screen layout:
//!   - top header: connection state + session count
//!   - session list: one row per session with mode badge + one-line context
//!   - details panel for the selected session: mode, cwd, last event,
//!     pending approval / question payload, gate state
//!   - key hint footer
//!
//! Drives off the daemon's REST + SSE API only — no daemon-internal coupling.
//! That means the TUI works against any compatible daemon (remote, future
//! re-impl in another language, etc.).

mod app;
mod sse;
mod view;

use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use tokio::sync::mpsc;

use app::{App, AppEvent};

pub async fn run(api_url: String) -> Result<()> {
    let mut app = App::new(api_url.clone());
    // Initial fetch is best-effort. If the daemon isn't reachable we still
    // come up — the header will show "disconnected" and the SSE loop keeps
    // retrying. The user can press `r` once the daemon is back.
    if let Err(err) = app.refresh_initial().await {
        app.toast(format!("initial fetch failed: {err}"));
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();

    // SSE subscriber task.
    {
        let url = format!("{}/events", api_url.trim_end_matches('/'));
        let tx = tx.clone();
        tokio::spawn(async move {
            sse::run(url, tx).await;
        });
    }

    // Set up the terminal in raw mode + alternate screen. Install a panic
    // hook so a crash anywhere in here restores the terminal — otherwise
    // the user is left with a mangled shell.
    let mut terminal = ratatui::init();
    let _restore_on_drop = TerminalGuard;

    // Keyboard event stream from crossterm, polled in the same select.
    let mut keys = EventStream::new();

    loop {
        terminal
            .draw(|frame| view::render(frame, &app))
            .context("draw")?;

        tokio::select! {
            // External events from the SSE task.
            maybe_evt = rx.recv() => match maybe_evt {
                Some(evt) => app.apply_event(evt),
                None => break,
            },
            // Keyboard input.
            maybe_key = keys.next() => {
                let Some(Ok(Event::Key(key))) = maybe_key else { continue };
                if key.kind != KeyEventKind::Press { continue }
                if let KeyCode::Char('c') = key.code {
                    if key.modifiers.contains(KeyModifiers::CONTROL) { break }
                }
                if !handle_key(&mut app, key.code).await { break; }
            },
            // Periodic redraw so "n seconds ago" stays fresh.
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
    }

    Ok(())
}

/// RAII guard that restores the terminal even on panic.
struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

async fn handle_key(app: &mut App, code: KeyCode) -> bool {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return false,
        KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
        KeyCode::Char('a') => app.act_approve(true).await,
        KeyCode::Char('d') => app.act_approve(false).await,
        KeyCode::Char('g') => app.act_toggle_gate().await,
        KeyCode::Char('r') => {
            let _ = app.refresh_initial().await;
        }
        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
            let option = c.to_digit(10).unwrap() as u8;
            app.act_answer(option).await;
        }
        _ => {}
    }
    true
}

