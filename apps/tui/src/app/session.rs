//! One session's conversation: transcript folds, delta application, the
//! optimistic echo, and which transport it runs on.

use super::*;

impl App {
    /// Apply one delta frame to its session's fold.
    ///
    /// Only sessions we have a fold for are tracked — the feed is global, so
    /// most frames belong to some other agent and are dropped here rather than
    /// growing state for a chat nobody opened. A frame that can't be sequenced
    /// onto what we have (`false` from `apply_delta`) means we missed one, so we
    /// resync from the snapshot endpoint instead of rendering a hole.
    pub(in crate::app) fn apply_conv_delta(&mut self, delta: crate::claudemon::ConvDelta) {
        let sid = delta.session_id;
        let Some(fold) = self.folds.get_mut(&sid) else {
            return;
        };
        let sequenced = fold.apply_delta(delta.seq, delta.reset, &delta.items);
        if !sequenced {
            // At most one resync in flight per session — see `resyncing`.
            if self.begin_resync(&sid) {
                self.load_transcript(sid.clone());
            }
            return;
        }
        if self.chat_session_id().as_deref() == Some(sid.as_str()) {
            self.retire_echo();
            // Deliberately no invalidate: the render memo keys on the fold's
            // commit counter, so a committed change rebuilds and a streamed
            // token does not.
        }
    }

    /// Drop the optimistic send echo once the real conversation carries it, so
    /// the message isn't shown twice for a beat.
    pub(in crate::app) fn retire_echo(&mut self) {
        let Some(echo) = self.pending_echo.as_deref() else {
            return;
        };
        let landed = self
            .turns()
            .iter()
            .rev()
            .find(|t| t.role == crate::types::Role::User)
            .is_some_and(|t| {
                t.parts
                    .iter()
                    .any(|p| matches!(p, crate::types::Part::Text(s) if s == echo))
            });
        if landed {
            self.pending_echo = None;
        }
    }

    /// The folded turns of the open chat, or empty when there is no chat.
    pub fn turns(&self) -> &[Turn] {
        self.chat_session_id()
            .and_then(|sid| self.folds.get(&sid))
            .map(|f| f.turns())
            .unwrap_or(&[])
    }

    /// Structural-change counter for the open chat's committed turns. The
    /// transcript render memo keys off this, so a streamed token (which only
    /// moves `pending_text`) doesn't invalidate the wrapped render of the whole
    /// conversation. Distinct from a session switch, which changes which fold
    /// answers here — hence the session id in the cache key too.
    pub fn commits(&self) -> u64 {
        self.chat_session_id()
            .and_then(|sid| self.folds.get(&sid))
            .map(|f| f.commits())
            .unwrap_or(0)
    }

    /// The open chat's uncommitted assistant text — the live tail of a message
    /// still streaming in. Rendered after the committed turns.
    pub fn pending_text(&self) -> Option<&str> {
        self.chat_session_id()
            .and_then(|sid| self.folds.get(&sid))
            .and_then(|f| f.pending_text())
    }

    /// A session's current plan, if it has published one. Rides the conversation
    /// stream as last-write-wins session state, so it's whatever the fold last
    /// saw — no separate fetch.
    pub fn plan_for(&self, sid: &str) -> Option<&crate::types::Plan> {
        self.folds.get(sid).and_then(|f| f.plan())
    }

    /// Adopt a full snapshot into a session's fold (open, or a delta gap).
    pub(in crate::app) fn adopt_snapshot(&mut self, sid: &str, snapshot: &serde_json::Value) {
        let stream = self.transport_for(sid) == "stream";
        self.folds
            .entry(sid.to_string())
            .or_insert_with(|| crate::types::ConvFold::new(stream))
            .adopt_snapshot(snapshot);
    }

    /// Seed a session's fold from a wire snapshot. Test-only: the app itself
    /// only ever gets one from the daemon, but a renderer test needs a transcript
    /// without standing up an HTTP server for it.
    #[cfg(test)]
    pub fn seed_fold(&mut self, sid: &str, snapshot: &serde_json::Value) {
        self.adopt_snapshot(sid, snapshot);
    }

    /// A session's wire transport (`"pty"`/`"stream"`), defaulting to PTY for
    /// sessions not (yet) in the live list.
    pub(in crate::app) fn transport_for(&self, sid: &str) -> String {
        self.all_agents
            .iter()
            .find(|a| a.session_id == sid)
            .map(|a| a.transport.clone())
            .unwrap_or_else(|| "pty".to_string())
    }

    /// Which transport to respawn an EXISTING session on.
    ///
    /// Its own, whenever we know it. A conversation resumes fine on either
    /// transport, but flipping one under the user is a behaviour change, not a
    /// detail: a PTY session would lose the terminal view it had, and a stream
    /// session would be handed a terminal that doesn't exist. The configured
    /// transport decides *fresh* spawns only — for a row we've never seen (not in
    /// the live list) that's also the best guess available.
    pub(in crate::app) fn respawn_transport(&self, sid: &str) -> crate::config::Transport {
        match self.all_agents.iter().find(|a| a.session_id == sid) {
            Some(a) if a.is_stream() => crate::config::Transport::Stream,
            Some(_) => crate::config::Transport::Pty,
            None => self.transport,
        }
    }

    /// A driver pinned to a specific spawn transport (see `respawn_transport`).
    /// Spawns are local-only, so this never hub-qualifies.
    pub(in crate::app) fn driver_on(
        &self,
        transport: crate::config::Transport,
    ) -> crate::bus::Driver {
        crate::bus::Driver {
            claudemon: self.claudemon.clone(),
            bus: self.bus.clone(),
            transport,
            hub: None,
        }
    }

    /// True for headless stream-transport sessions — no PTY to warm or attach.
    pub fn is_stream_session(&self, sid: &str) -> bool {
        self.all_agents
            .iter()
            .find(|a| a.session_id == sid)
            .is_some_and(|a| a.is_stream())
    }

    /// Pull a full snapshot for a session's fold. Needed on open and after a
    /// delta gap; the delta feed covers everything in between. A remote
    /// session's conversation comes over its federation link
    /// (`hub:<peer>/sessions.conversation` — same `{items, seq}` shape the
    /// fold adopts); local ones keep the claudemon path untouched.
    pub(in crate::app) fn load_transcript(&self, session_id: String) {
        if let Some(hub) = self.hub_of(&session_id) {
            let Some(bus) = self.bus.clone() else { return };
            let tx = self.tx.clone();
            tokio::spawn(crate::federation::fetch_remote_conversation(
                bus, hub, session_id, tx,
            ));
            return;
        }
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move { fetch_transcript(&cm, &tx, session_id).await });
    }

    pub(in crate::app) fn maybe_load_transcript(&self) {
        if self.chat_mode == ChatMode::Transcript {
            if let Some(sid) = self.chat_session_id() {
                self.load_transcript(sid);
            }
        }
    }

    /// Drop the memoized transcript render. Must be called whenever `turns`
    /// or `pending_echo` change; the renderer rebuilds it on the next draw.
    pub(in crate::app) fn invalidate_transcript_cache(&mut self) {
        self.transcript_cache = None;
    }

    // ── inbound messages ──────────────────────────────────────────────────

    /// Fold a live statusLine tick into the per-session map (the renderer reads
    /// it via `derive_stats`).
    pub fn apply_status_line(&mut self, session_id: String, status_line: StatusLine) {
        self.status_lines.insert(session_id, status_line);
    }

    /// The session the active tab shows (its content). May be a shell.
    pub fn chat_session_id(&self) -> Option<String> {
        self.active_tab().map(|t| t.session_id.clone())
    }
}
