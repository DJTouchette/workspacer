//! Terminals: warming a session's PTY, pruning dead ones, and the
//! transcript/terminal toggle.

use super::*;

impl App {
    /// Record that a session has no PTY and drop its (useless) warm terminal. If
    /// it's the open chat in terminal mode, fall back to the transcript.
    pub(in crate::app) fn mark_no_terminal(&mut self, session_id: String) {
        self.no_terminal.insert(session_id.clone());
        self.terms.remove(&session_id);
        if let Some(h) = self.term_tasks.remove(&session_id) {
            h.abort();
        }
        if self.open_session_id().as_deref() == Some(session_id.as_str()) {
            self.chat_mode = ChatMode::Transcript;
            self.term_attached = false;
            self.chat_follow = true;
            self.load_transcript(session_id);
        }
    }

    /// Feed a PTY chunk into its session's emulator. Feeds background terminals
    /// too, so every cached agent stays current even while you're elsewhere.
    pub fn feed_pty(&mut self, chunk: PtyChunk) {
        if let Some(term) = self.terms.get_mut(&chunk.session_id) {
            term.feed(&chunk.bytes);
        }
    }

    /// Push a pending PTY resize to claudemon (called after each draw, since the
    /// renderer is what learns the pane size). Reflows Claude's TUI to the pane.
    pub fn flush_term_resize(&mut self) {
        if self.term_resizes.is_empty() {
            return;
        }
        let pending: Vec<(String, (u16, u16))> = self.term_resizes.drain().collect();
        let drv = self.driver();
        tokio::spawn(async move {
            for (sid, (cols, rows)) in pending {
                let _ = drv.resize(&sid, cols, rows).await;
            }
        });
    }

    // ── workspace / tab selectors ─────────────────────────────────────────

    /// Ensure a warm emulator + PTY stream exists for this session. Idempotent:
    /// re-opening an agent reuses the already-current terminal instead of
    /// re-attaching (which is what caused the blank re-open).
    pub(in crate::app) fn ensure_terminal(&mut self, session_id: String) {
        if self.terms.contains_key(&session_id) || self.no_terminal.contains(&session_id) {
            return;
        }
        // Headless stream sessions have no PTY by construction — never open
        // (and endlessly retry) a stream for them. Recording the fact keeps
        // watch panes labelled "transcript only" instead of "starting…".
        if self.is_stream_session(&session_id) {
            self.no_terminal.insert(session_id);
            return;
        }
        self.terms.insert(session_id.clone(), Term::new());

        // Bus mode: attach the lease (which replays the ring buffer) and keep it
        // alive; the bytes arrive as pty.bytes.<id> events (see apply_bus_event).
        if let Some(bus) = self.bus.clone() {
            let sid = session_id.clone();
            let handle = tokio::spawn(async move {
                let _ = bus
                    .call(
                        "sessions.attachTerminal",
                        serde_json::json!({ "sessionId": sid }),
                    )
                    .await;
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    // A false keepalive means the lease lapsed — re-attach to re-prime.
                    let lapsed = matches!(
                        bus.call("sessions.terminalKeepalive", serde_json::json!({ "sessionId": sid })).await,
                        Ok(v) if v.get("ok").and_then(|b| b.as_bool()) == Some(false)
                    );
                    if lapsed {
                        let _ = bus
                            .call(
                                "sessions.attachTerminal",
                                serde_json::json!({ "sessionId": sid }),
                            )
                            .await;
                    }
                }
            });
            self.term_tasks.insert(session_id, handle.abort_handle());
            return;
        }

        let cm = self.claudemon.clone();
        let pty_tx = self.pty_tx.clone();
        let msg_tx = self.tx.clone();
        let sid = session_id.clone();
        let handle = tokio::spawn(async move {
            use crate::claudemon::StreamEnd;
            const MIN_BACKOFF: std::time::Duration = std::time::Duration::from_millis(300);
            const MAX_BACKOFF: std::time::Duration = std::time::Duration::from_secs(5);
            /// A stream that stayed up this long was a working connection, not
            /// a failed retry — the next drop starts over from the bottom.
            const HEALTHY_AFTER: std::time::Duration = std::time::Duration::from_secs(10);
            let mut backoff = MIN_BACKOFF;
            loop {
                let started = std::time::Instant::now();
                match cm.read_pty_stream(&sid, &pty_tx).await {
                    // No PTY for this session — tell the app to use the
                    // transcript and stop trying.
                    Ok(StreamEnd::NoPty) => {
                        let _ = msg_tx.send(AppMsg::TerminalUnavailable(sid.clone()));
                        return;
                    }
                    Ok(StreamEnd::Disconnected) | Err(_) => {}
                }
                // Reset after a healthy run. Without this the backoff only ever
                // grew: a handful of drops over a long session pinned it at the
                // 5s ceiling forever, so every later blip froze the terminal
                // pane for a full five seconds before the replay arrived, even
                // though the daemon was answering in milliseconds.
                if started.elapsed() >= HEALTHY_AFTER {
                    backoff = MIN_BACKOFF;
                }
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        });
        self.term_tasks.insert(session_id, handle.abort_handle());
    }

    /// Drop cached terminals (and their stream tasks) for sessions that no
    /// longer exist, so we don't leak connections for ended agents.
    pub(in crate::app) fn prune_terminals(&mut self, live: &HashSet<String>) {
        self.terms.retain(|sid, _| live.contains(sid));
        self.term_tasks.retain(|sid, handle| {
            let keep = live.contains(sid);
            if !keep {
                handle.abort();
            }
            keep
        });
    }

    /// Switch between the raw terminal and the parsed transcript. The terminal
    /// stays warm in the background either way.
    pub(in crate::app) fn toggle_chat_mode(&mut self) {
        let Some(sid) = self.chat_session_id() else {
            return;
        };
        match self.chat_mode {
            ChatMode::Terminal => {
                self.chat_mode = ChatMode::Transcript;
                self.term_attached = false;
                self.chat_follow = true;
                self.load_transcript(sid);
            }
            ChatMode::Transcript => {
                // Headless stream sessions have no PTY — the toggle is a no-op.
                if self.is_stream_session(&sid) {
                    self.set_toast("headless session — no terminal");
                    return;
                }
                if self.no_terminal.contains(&sid) {
                    self.set_toast("no terminal — external session (transcript only)");
                    return;
                }
                self.chat_mode = ChatMode::Terminal;
                self.ensure_terminal(sid);
            }
        }
    }

    /// Add a freshly-spawned shell as a tab under its agent and switch to it.
    pub(in crate::app) fn add_shell_tab(&mut self, agent_id: String, session_id: String) {
        if let Some(ws) = self.workspaces.get_mut(&agent_id) {
            let n = ws.tabs.iter().filter(|t| t.kind == TabKind::Shell).count() + 1;
            ws.tabs.push(Tab {
                title: format!("sh{n}"),
                session_id,
                kind: TabKind::Shell,
            });
            ws.active = ws.tabs.len() - 1;
            // Only switch into it if that agent is the one on screen.
            if self.open_agent_id() == Some(agent_id.as_str()) {
                self.enter_active_tab();
            }
            // The new shell may already be in the agent list — re-filter so it
            // drops out of the sidebar immediately, not on the next poll.
            self.apply_filter();
        }
    }

    pub fn term_attached(&self) -> bool {
        self.term_attached && self.open_session_id().is_some()
    }

    // ── sidebar (row 0 = Dashboard, rows 1.. = agents) ──────────────────────
}
