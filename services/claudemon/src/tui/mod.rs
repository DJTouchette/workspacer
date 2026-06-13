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

pub mod app;
pub mod editor;
pub mod preview;
mod sse;
mod syntax;
pub mod view;

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

    // Best-effort: ask the terminal to disambiguate Shift/Ctrl/Alt+Enter
    // from plain Enter via the kitty keyboard protocol. Terminals that
    // don't support it ignore the request and we silently fall back to
    // Alt+Enter / Ctrl+J for newlines.
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::event::PushKeyboardEnhancementFlags(
            crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        )
    );

    // Keyboard event stream from crossterm, polled in the same select.
    let mut keys = EventStream::new();
    let mut last_in_chat = app.in_chat();

    loop {
        // Chat <-> dashboard switches change the layout shape, which can
        // leave ghost cells from the previous view. A one-shot clear on
        // the transition avoids that without the flicker of clearing
        // every frame.
        if app.in_chat() != last_in_chat {
            terminal.clear().context("clear")?;
            last_in_chat = app.in_chat();
        }

        terminal
            .draw(|frame| view::render(frame, &app))
            .context("draw")?;

        tokio::select! {
            // External events from the SSE task.
            maybe_evt = rx.recv() => match maybe_evt {
                Some(evt) => {
                    let should_refresh = app.apply_event(evt);
                    if should_refresh {
                        app.fetch_transcript_for_chat().await;
                    }
                }
                None => break,
            },
            // Keyboard input.
            maybe_key = keys.next() => {
                let Some(Ok(Event::Key(key))) = maybe_key else { continue };
                if key.kind != KeyEventKind::Press { continue }
                if let KeyCode::Char('c') = key.code {
                    if key.modifiers.contains(KeyModifiers::CONTROL) { break }
                }
                let cont = if app.in_chat() {
                    handle_chat_key(&mut app, key).await
                } else {
                    handle_dashboard_key(&mut app, key.code).await
                };
                if !cont { break; }
            },
            // Periodic redraw so "n seconds ago" stays fresh. In chat view,
            // also refetch the JSONL transcript so output appears while Claude
            // is still working, not only after a final Stop event.
            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                if app.in_chat() {
                    app.fetch_transcript_for_chat().await;
                }
            }
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

async fn handle_dashboard_key(app: &mut App, code: KeyCode) -> bool {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return false,
        KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
        KeyCode::Enter => app.enter_chat().await,
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

async fn handle_chat_key(app: &mut App, key: crossterm::event::KeyEvent) -> bool {
    // In chat view, plain text keys feed the editor. Quick-actions for
    // pending state (a/d/1-9/r/q) fire only when the input buffer is
    // empty so typing "approve allow this" doesn't accidentally trigger
    // /approve.
    use crossterm::event::KeyModifiers;

    let code = key.code;
    let mods = key.modifiers;
    let input_empty = app.editor_is_empty();
    let mode = app
        .chat_session_id()
        .and_then(|id| app.sessions.get(&id))
        .map(|s| s.mode);
    let transcript_focus = match &app.view {
        app::View::Chat(chat) => chat.transcript_focus,
        _ => false,
    };

    match code {
        KeyCode::Esc => app.leave_chat(),
        KeyCode::Tab => app.toggle_transcript_focus(),

        KeyCode::Char('j') if transcript_focus => app.chat_scroll(1),
        KeyCode::Char('k') if transcript_focus => app.chat_scroll(-1),
        KeyCode::Down if transcript_focus => app.chat_scroll(1),
        KeyCode::Up if transcript_focus => app.chat_scroll(-1),
        KeyCode::Char('l') if transcript_focus => app.set_tool_results_expanded(true),
        KeyCode::Char('h') if transcript_focus => app.set_tool_results_expanded(false),

        // Enter behavior depends on modifiers: Alt/Shift/Ctrl+Enter → newline,
        // plain Enter → send. We also accept Ctrl+J as a universal newline
        // (Ctrl+J is the LF byte; terminals that don't disambiguate Enter
        // can still produce it via Ctrl+J).
        KeyCode::Enter
            if mods.contains(KeyModifiers::ALT)
                || mods.contains(KeyModifiers::SHIFT)
                || mods.contains(KeyModifiers::CONTROL) =>
        {
            app.with_editor(|e| e.insert_newline());
        }
        KeyCode::Enter => app.act_send_message().await,
        KeyCode::Char('j') if mods.contains(KeyModifiers::CONTROL) => {
            app.with_editor(|e| e.insert_newline());
        }

        // History + cursor navigation
        KeyCode::Up => app.with_editor(|e| e.history_prev()),
        KeyCode::Down => app.with_editor(|e| e.history_next()),
        KeyCode::Left => app.with_editor(|e| e.move_left()),
        KeyCode::Right => app.with_editor(|e| e.move_right()),
        KeyCode::Home => app.with_editor(|e| e.move_home()),
        KeyCode::End => app.with_editor(|e| e.move_end()),

        // Editing
        KeyCode::Backspace => app.with_editor(|e| e.backspace()),
        KeyCode::Delete => app.with_editor(|e| e.delete_forward()),
        KeyCode::Char('w') if mods.contains(KeyModifiers::CONTROL) => {
            app.with_editor(|e| e.delete_word_back())
        }
        KeyCode::Char('u') if mods.contains(KeyModifiers::CONTROL) => {
            app.with_editor(|e| e.clear())
        }

        // Scroll the transcript
        KeyCode::PageUp => app.chat_scroll(-5),
        KeyCode::PageDown => app.chat_scroll(5),

        // Mode-aware quick-actions (only when the input is empty)
        KeyCode::Char('r') if input_empty => {
            app.fetch_transcript_for_chat().await;
        }
        KeyCode::Char(c)
            if c.is_ascii_digit()
                && c != '0'
                && input_empty
                && mode == Some(crate::session::SessionMode::Question) =>
        {
            let option = c.to_digit(10).unwrap() as u8;
            app.act_answer(option).await;
        }
        KeyCode::Char('a')
            if input_empty && mode == Some(crate::session::SessionMode::Approval) =>
        {
            app.act_approve(true).await;
        }
        KeyCode::Char('d')
            if input_empty && mode == Some(crate::session::SessionMode::Approval) =>
        {
            app.act_approve(false).await;
        }
        KeyCode::Char('q') if input_empty => return false,

        // Default: insert the character into the editor.
        KeyCode::Char(c) => app.with_editor(|e| e.insert(c)),

        _ => {}
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[tokio::test]
    async fn tab_focuses_transcript_and_jk_scroll() {
        let mut app = App::new("http://test".into());
        app.view = app::View::Chat(app::ChatState {
            session_id: "s".into(),
            transcript: Default::default(),
            editor: crate::tui::editor::Editor::new(),
            transcript_focus: false,
            expand_tool_results: false,
            scroll_offset: 0,
            last_seen_mode: crate::session::SessionMode::Input,
            render_cache: std::cell::RefCell::new(None),
        });

        assert!(handle_chat_key(&mut app, key(KeyCode::Tab)).await);
        assert!(matches!(&app.view, app::View::Chat(c) if c.transcript_focus));

        assert!(handle_chat_key(&mut app, key(KeyCode::Char('j'))).await);
        assert!(matches!(&app.view, app::View::Chat(c) if c.scroll_offset == 0));

        assert!(handle_chat_key(&mut app, key(KeyCode::Char('k'))).await);
        assert!(matches!(&app.view, app::View::Chat(c) if c.scroll_offset == 1));

        assert!(handle_chat_key(&mut app, key(KeyCode::Char('l'))).await);
        assert!(matches!(&app.view, app::View::Chat(c) if c.expand_tool_results));

        assert!(handle_chat_key(&mut app, key(KeyCode::Char('h'))).await);
        assert!(matches!(&app.view, app::View::Chat(c) if !c.expand_tool_results));
    }
}
