//! Application state and input handling.
//!
//! The app owns no socket directly — it holds a [`Claudemon`] client and an
//! `AppMsg` sender, and every action that needs the network spawns a task that
//! calls claudemon and posts the outcome back as an [`AppMsg`]. The main loop
//! (in `main.rs`) drives `draw` + `handle_key` + `apply_msg`. This keeps
//! rendering synchronous and the network fully async, fire-and-refresh.

mod input;
pub mod tasks;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

use crate::claudemon::{Claudemon, PtyChunk};
use crate::config::Config;
use crate::keys::{Chord, Keymap};
use crate::library::LibraryItem;
use crate::profiles::Profile;
use crate::terminal::Term;
use crate::theme::Theme;
use crate::types::{Agent, FileStatus, StatusLine, Turn};
use base64::Engine as _;

use tasks::{fetch_agents, fetch_git_diff, fetch_git_status, fetch_transcript};

mod state;
pub use state::*;
use state::{fuzzy_match, TOAST_TTL};

mod agents;
mod git;
mod runs;
mod session;
mod terminal;

pub struct App {
    pub(super) claudemon: Claudemon,
    /// Optional hub-bus client. When set, agent-driving calls route through it
    /// (the TUI as a thin bus client); otherwise everything uses claudemon.
    pub(super) bus: Option<crate::bus::BusClient>,
    pub(super) tx: UnboundedSender<AppMsg>,
    /// Sender the PTY stream task pushes chunks into; the main loop drains it
    /// and calls [`App::feed_pty`].
    pub(super) pty_tx: UnboundedSender<PtyChunk>,

    pub profiles: Vec<Profile>,
    pub library: Vec<LibraryItem>,
    /// Resolved color theme; every renderer references it instead of literals.
    /// Transport for Claude sessions we spawn. Never consulted for a session
    /// that already exists — `transport_for` reads that off the wire.
    pub transport: crate::config::Transport,
    pub theme: Theme,
    /// Resolved keybindings; `input.rs` dispatches every key through this.
    pub keymap: Keymap,
    /// Whether the keybinding/help overlay is open.
    pub help: bool,
    pub spawn_form: Option<SpawnForm>,
    pub palette: Option<Palette>,
    /// The model / handoff-provider picker modal, when open.
    pub picker: Option<Picker>,
    /// Fallback provider map for managed sessions this TUI spawned (session_id →
    /// `"codex"`/`"opencode"`/`"pi"`). claudemon's `/sessions` list now carries
    /// an authoritative `provider` field ([`Agent::provider`]) that
    /// [`App::provider_for`] prefers; this map only bridges the gap before the
    /// first post-spawn refresh lands (and daemons too old to emit the field).
    pub managed_providers: HashMap<String, String>,
    /// Last known permission mode per session, updated from each successful
    /// switch — the cycle advances from here.
    pub perm_modes: HashMap<String, String>,
    /// The git review pane, when open (a modal over the agent view).
    pub review: Option<ReviewState>,
    /// A pane docked beside the agent tiles: the git review, or the agent's own
    /// changed files. `None` when the content column is all agent.
    ///
    /// Deliberately its own slot rather than another entry in `tiles`: every tile
    /// is an agent, and the split/focus/close logic is written around that
    /// (`focus_agent` on every move, "no other agent to split"). Threading a kind
    /// through all of it to express "one docked pane" would put those invariants
    /// at risk for no user-visible gain — nobody wants three reviews side by side.
    pub side: Option<SidePane>,
    /// The runs overlay: which session's subagents + workflows are on screen.
    /// `None` when closed.
    pub runs_open: Option<String>,
    /// Last-read runs per session. Read off disk on a refresh tick while the
    /// overlay is open (see [`crate::runs`]) — nothing is read for a session
    /// nobody is looking at.
    pub runs: std::collections::HashMap<String, crate::runs::SessionRuns>,
    /// The rename overlay, when open.
    pub rename: Option<RenameForm>,
    /// Custom per-cwd display names (persisted); empty when none set.
    pub names: HashMap<String, String>,
    /// The notes scratchpad overlay, when open.
    pub notes_view: Option<NotesState>,
    /// Per-cwd scratchpad text (persisted).
    pub notes: HashMap<String, String>,
    /// Inspector cache: cwd → (branch, changed-file count), for the open agent.
    pub git_summary: HashMap<String, (Option<String>, usize)>,

    pub connected: bool,
    pub should_quit: bool,

    /// Chat rendering mode.
    pub chat_mode: ChatMode,
    /// Live terminal emulators, kept warm per session so re-opening an agent is
    /// instant and correct (no re-attach / blank screen). Created lazily the
    /// first time you open an agent; fed continuously in the background by their
    /// PTY stream tasks; pruned when the session disappears.
    pub terms: HashMap<String, Term>,
    pub(super) term_tasks: HashMap<String, tokio::task::AbortHandle>,
    /// Sessions known to have no PTY (external/observed-only); they render the
    /// transcript instead of a terminal.
    pub no_terminal: HashSet<String>,
    /// Tabs per open agent, keyed by the agent's (Claude) session id.
    pub workspaces: HashMap<String, Workspace>,
    pub term_attached: bool,
    /// Pending `(cols, rows)` per session id, pushed to claudemon after the next
    /// draw whenever a (focused or watch) pane resized that session's emulator.
    pub term_resizes: HashMap<String, (u16, u16)>,

    /// Agents tiled in the content area (their session ids). 0/1 entries render
    /// as a single pane (today's behavior); 2+ tile the content. `tile_focus`
    /// is the interactive pane — it stays in sync with `View::Agent { id }`; the
    /// others render their live terminal read-only. See [`SplitDir`].
    pub tiles: Vec<String>,
    pub tile_focus: usize,
    pub split_dir: SplitDir,

    /// Harpoon-style pinned agents (session ids), in slot order. `<leader>1..9`
    /// teleports to a slot; the sidebar shows each pin's number. Derived each
    /// poll from `pinned_cwds` resolved against the live sessions.
    pub harpoon: Vec<String>,
    /// The persisted pins, as an ordered list of cwds (stable across restarts;
    /// see [`crate::pins`]). The source of truth `harpoon` is rebuilt from.
    pub pinned_cwds: Vec<String>,
    /// The agent focused just before the current one — vim's alternate buffer
    /// (`Ctrl-^`).
    pub prev_focus: Option<String>,
    /// Visited-agent history for the jumplist (`Ctrl-o` / `<leader>i`);
    /// `jump_idx` is the current position within it.
    pub jumplist: Vec<String>,
    pub jump_idx: usize,

    /// Session ids seen in a non-stopped mode this run. Used to hide hydrated
    /// "orphan" sessions (stopped history claudemon replays on restart) while
    /// keeping sessions that stopped while we watched them.
    pub seen_live: HashSet<String>,
    /// When true, the sidebar shows every session including stopped history.
    pub show_all_sessions: bool,
    /// How many stopped orphans the last [`set_agents`] hid (for the title).
    pub hidden_count: usize,

    /// Full live session set (orphan-filtered) — the source of truth for
    /// lifecycle and by-id lookups. `agents` is the `/`-filtered projection of
    /// this that the sidebar and selection use.
    pub all_agents: Vec<Agent>,
    /// Active sidebar filter query (`/`); `None` means no filter. `filter_editing`
    /// is true while the query is being typed.
    pub filter: Option<String>,
    pub filter_editing: bool,
    /// The `:` ex-command line buffer; `Some` while it's open and capturing.
    pub cmdline: Option<String>,
    /// The cross-agent content-search modal; `Some` while it's open.
    pub search: Option<SearchState>,
    /// A pending vim count prefix (e.g. `3` before `j`), applied to the next
    /// motion and then cleared. `None` when no count is being typed.
    pub count: Option<usize>,

    /// Agents, in a stable order: existing rows stay put across polls and new
    /// sessions are appended at the end (matches the Electron app).
    pub agents: Vec<Agent>,
    /// Claude's authoritative statusLine per session (context%/cost/model/rate
    /// limits), streamed live; preferred over transcript usage when present.
    pub status_lines: HashMap<String, StatusLine>,
    pub selected: usize,

    pub view: View,
    /// Per-session incremental conversation folds, keyed by session id. Holding
    /// the fold (rather than a flat turns vec) is what lets the delta feed apply
    /// one item without re-deriving the whole transcript: joining a tool result
    /// to its call, deduping replays, and gluing token fragments all need the
    /// state that came before. Populated when a chat is opened; sessions we've
    /// never looked at cost nothing.
    pub folds: std::collections::HashMap<String, crate::types::ConvFold>,
    /// Memoized transcript render: the fully folded + wrapped lines for the
    /// current `turns`/`pending_echo` at a given width. The main loop draws on
    /// every event (PTY chunks, SSE nudges, keystrokes, the tick), so without
    /// this every draw would re-parse the whole conversation's markdown.
    /// Sessions with a full-conversation resync in flight, and when it started.
    ///
    /// A delta that can't be sequenced triggers a resync; without a guard every
    /// further delta arriving during that round trip triggered another, each
    /// response re-folding the whole conversation, and an older response
    /// landing after a newer one would adopt a stale snapshot. The timestamp
    /// self-heals: a failed fetch sends nothing back, so the guard must not
    /// latch forever.
    pub resyncing: std::collections::HashMap<String, Instant>,
    /// Cleared by [`App::invalidate_transcript_cache`] whenever the inputs
    /// change; the renderer refills it when the width differs.
    pub transcript_cache: Option<TranscriptCache>,
    /// Top-line offset of the transcript viewport. Authoritative after each
    /// render, which clamps it to the content height. `usize` on purpose: a
    /// very long transcript can wrap to more than `u16::MAX` lines.
    pub chat_scroll: usize,
    /// When true, the transcript sticks to the bottom as new content streams
    /// in; the renderer keeps `chat_scroll` pinned to the max. Any manual
    /// scroll clears it.
    pub chat_follow: bool,
    /// True when the composer is capturing keystrokes (vim insert mode).
    pub insert_mode: bool,
    pub input: String,
    /// Optimistic local echo of the last sent chat message: rendered as a
    /// pending user turn until a refold's trailing user message matches it
    /// (or the send fails, which restores the composer).
    pub pending_echo: Option<String>,
    /// Progress through the open agent's multi-question / multi-select set;
    /// `None` until the user starts answering (renderers fall back to Q1).
    pub question_flow: Option<QuestionFlow>,

    /// Chords typed so far toward a multi-key binding (e.g. after the leader).
    /// Empty when no sequence is in flight; drives the which-key popup.
    pub pending_keys: Vec<Chord>,

    pub(super) toast: Option<(String, Instant)>,
}

impl App {
    pub fn new(
        claudemon: Claudemon,
        profiles: Vec<Profile>,
        library: Vec<LibraryItem>,
        config: Config,
        tx: UnboundedSender<AppMsg>,
        pty_tx: UnboundedSender<PtyChunk>,
    ) -> Self {
        Self {
            claudemon,
            bus: None,
            tx,
            pty_tx,
            profiles,
            library,
            theme: config.theme,
            keymap: config.keymap,
            transport: config.transport,
            help: false,
            spawn_form: None,
            palette: None,
            picker: None,
            managed_providers: HashMap::new(),
            perm_modes: HashMap::new(),
            review: None,
            rename: None,
            names: crate::names::load(),
            notes_view: None,
            notes: crate::notes::load(),
            git_summary: HashMap::new(),
            connected: false,
            should_quit: false,
            chat_mode: ChatMode::Terminal,
            terms: HashMap::new(),
            term_tasks: HashMap::new(),
            no_terminal: HashSet::new(),
            workspaces: HashMap::new(),
            term_attached: false,
            term_resizes: HashMap::new(),
            tiles: Vec::new(),
            tile_focus: 0,
            split_dir: SplitDir::Columns,
            harpoon: Vec::new(),
            pinned_cwds: crate::pins::load(),
            prev_focus: None,
            jumplist: Vec::new(),
            jump_idx: 0,
            seen_live: HashSet::new(),
            show_all_sessions: false,
            hidden_count: 0,
            all_agents: Vec::new(),
            filter: None,
            filter_editing: false,
            cmdline: None,
            search: None,
            count: None,
            agents: Vec::new(),
            status_lines: HashMap::new(),
            selected: 0,
            view: View::List,
            folds: std::collections::HashMap::new(),
            side: None,
            runs_open: None,
            runs: std::collections::HashMap::new(),
            resyncing: std::collections::HashMap::new(),
            transcript_cache: None,
            chat_scroll: 0,
            chat_follow: false,
            insert_mode: false,
            input: String::new(),
            pending_echo: None,
            question_flow: None,
            pending_keys: Vec::new(),
            toast: None,
        }
    }

    /// Attach (or clear) the hub-bus client after construction.
    pub fn set_bus(&mut self, bus: Option<crate::bus::BusClient>) {
        self.bus = bus;
    }

    /// A cheap driver bound to the current claudemon + optional bus, for
    /// agent-driving calls (message/approve/answer/signal).
    pub(super) fn driver(&self) -> crate::bus::Driver {
        crate::bus::Driver {
            claudemon: self.claudemon.clone(),
            bus: self.bus.clone(),
            transport: self.transport,
        }
    }

    // ── live toast ──────────────────────────────────────────────────────────

    pub fn toast(&self) -> Option<&str> {
        match &self.toast {
            Some((msg, at)) if at.elapsed() < TOAST_TTL => Some(msg),
            _ => None,
        }
    }

    pub(super) fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    /// Claim the resync slot for a session. Returns false when one is already
    /// in flight (and hasn't gone stale), so the caller skips the fetch.
    pub(super) fn begin_resync(&mut self, session_id: &str) -> bool {
        /// Long enough to cover a slow round trip, short enough that a failed
        /// fetch doesn't wedge resync for the session.
        const RESYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
        let now = Instant::now();
        if let Some(started) = self.resyncing.get(session_id) {
            if now.duration_since(*started) < RESYNC_TIMEOUT {
                return false;
            }
        }
        self.resyncing.insert(session_id.to_string(), now);
        true
    }

    pub fn apply_msg(&mut self, msg: AppMsg) {
        match msg {
            AppMsg::Agents(list) => self.set_agents(list),
            AppMsg::Transcript {
                session_id,
                snapshot,
            } => {
                self.resyncing.remove(&session_id);
                self.adopt_snapshot(&session_id, &snapshot);
                if self.chat_session_id().as_deref() == Some(session_id.as_str()) {
                    self.retire_echo();
                    self.invalidate_transcript_cache();
                }
            }
            AppMsg::ConvDelta(delta) => self.apply_conv_delta(*delta),
            AppMsg::Runs { session_id, runs } => {
                self.runs.insert(session_id, *runs);
            }
            AppMsg::Toast(t) => self.set_toast(t),
            AppMsg::SendFailed { text, error } => {
                self.pending_echo = None;
                self.invalidate_transcript_cache();
                // Give the user their message back to retry (unless they've
                // already started typing something else).
                if self.input.is_empty() {
                    self.input = text;
                }
                self.set_toast(format!("Failed: {error}"));
            }
            AppMsg::TerminalUnavailable(sid) => self.mark_no_terminal(sid),
            AppMsg::ShellSpawned {
                agent_id,
                session_id,
            } => self.add_shell_tab(agent_id, session_id),
            AppMsg::SearchEntries {
                session_id,
                name,
                lines,
            } => {
                // Fold a session's lines into the open search index (ignore if
                // the modal was closed before the fetch returned).
                if let Some(s) = self.search.as_mut() {
                    for line in lines {
                        s.entries.push(SearchHit {
                            session_id: session_id.clone(),
                            name: name.clone(),
                            line,
                        });
                    }
                    s.pending = s.pending.saturating_sub(1);
                    s.rematch();
                }
            }
            AppMsg::GitStatus { cwd, branch, files } => self.apply_git_status(cwd, branch, files),
            AppMsg::GitDiff {
                cwd,
                path,
                staged,
                diff,
            } => self.apply_git_diff(cwd, path, staged, diff),
            AppMsg::GitSummary {
                cwd,
                branch,
                changed,
            } => {
                self.git_summary.insert(cwd, (branch, changed));
            }
            AppMsg::GitError { cwd, message } => {
                if let Some(r) = self.review.as_mut() {
                    if r.cwd == cwd {
                        r.error = Some(message);
                    }
                }
            }
            AppMsg::PermissionMode { session_id, mode } => {
                self.perm_modes.insert(session_id, mode.clone());
                self.set_toast(format!("Mode: {mode}"));
            }
            AppMsg::PickerModels { session_id, models } => {
                if let Some(p) = self.picker.as_mut() {
                    if p.session_id == session_id {
                        // Highlight the provider's default model, if it flagged one.
                        let default_idx = models.iter().position(|m| m.default);
                        p.items = models
                            .into_iter()
                            .map(|m| PickerItem {
                                label: match &m.label {
                                    Some(l) if !l.is_empty() && l != &m.id => {
                                        format!("{}  ({})", l, m.id)
                                    }
                                    _ => m.id.clone(),
                                },
                                id: m.id,
                            })
                            .collect();
                        p.pending = false;
                        p.rematch();
                        if let Some(i) = default_idx {
                            if let Some(pos) = p.matched.iter().position(|&mi| mi == i) {
                                p.selected = pos;
                            }
                        }
                    }
                }
            }
            AppMsg::ManagedSpawned {
                session_id,
                provider,
            } => {
                self.managed_providers.insert(session_id, provider);
            }
        }
    }

    pub fn on_connected(&mut self) {
        self.connected = true;
        self.refresh();
        self.maybe_load_transcript();
    }

    pub fn on_disconnected(&mut self) {
        self.connected = false;
    }

    /// A session changed — re-pull the list and, if we're reading a transcript,
    /// refresh it. (Terminal mode updates live over its own PTY stream.)
    pub fn on_changed(&mut self) {
        // Session-list state only. The conversation used to be refetched here on
        // every daemon nudge — that was the polling model the delta feed
        // replaces, and doing both would mean a full refetch per streamed token.
        self.refresh();
    }

    pub(super) fn open_agent_id(&self) -> Option<&str> {
        match &self.view {
            View::Agent { id } => Some(id),
            View::List => None,
        }
    }

    pub fn workspace(&self) -> Option<&Workspace> {
        self.workspaces.get(self.open_agent_id()?)
    }

    pub(super) fn workspace_mut(&mut self) -> Option<&mut Workspace> {
        let id = self.open_agent_id()?.to_string();
        self.workspaces.get_mut(&id)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.workspace()?.active_tab()
    }

    /// The session whose terminal is on screen: a Shell tab always, or a Claude
    /// tab in terminal mode.
    pub fn open_session_id(&self) -> Option<String> {
        let tab = self.active_tab()?;
        match tab.kind {
            TabKind::Shell => Some(tab.session_id.clone()),
            TabKind::Claude => {
                (self.chat_mode == ChatMode::Terminal).then(|| tab.session_id.clone())
            }
        }
    }

    /// Re-pull the agent list from claudemon. The TUI's `Agent` model is
    /// claudemon's REST shape (snake_case, `mode`/`pending`/`usage`); the hub
    /// bus instead serves the desktop's enriched, camelCase view, which doesn't
    /// deserialize here — so even in bus mode the list is owned by claudemon
    /// (always reachable in the loopback setups). The bus stays the transport
    /// for driving (the [`Driver`]), PTY bytes, and statuslines.
    pub fn refresh(&self) {
        let tx = self.tx.clone();
        let cm = self.claudemon.clone();
        tokio::spawn(async move { fetch_agents(&cm, &tx).await });
    }

    pub fn has_bus(&self) -> bool {
        self.bus.is_some()
    }

    /// Route a delivered bus event into the agent view.
    pub fn apply_bus_event(&mut self, ev: crate::bus::BusEvent) {
        // Live terminal bytes: pty.bytes.<sessionId>, base64 in the event data.
        if let Some(sid) = ev.topic.strip_prefix("pty.bytes.") {
            if let Some(b64) = ev.data.as_str() {
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                    self.feed_pty(crate::claudemon::PtyChunk {
                        session_id: sid.to_string(),
                        bytes,
                    });
                }
            }
            return;
        }
        match ev.topic.as_str() {
            // The list itself is refreshed from claudemon (see `refresh`); a bus
            // snapshot is just a nudge that something changed, so re-pull.
            "agent.snapshot" => self.refresh(),
            "agent.statusline" => {
                let sid = ev
                    .data
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let sl = ev.data.get("statusLine").cloned();
                if let (Some(sid), Some(sl)) = (sid, sl) {
                    if let Ok(status) = serde_json::from_value::<StatusLine>(sl) {
                        self.apply_status_line(sid, status);
                    }
                }
            }
            _ => {}
        }
    }

    /// Run a control future, toast the outcome, then refresh the list (and the
    /// open transcript). The universal "do something to an agent" path; pass a
    /// future built from a cloned [`Claudemon`].
    pub(super) fn dispatch<F>(&self, ok_msg: &str, fut: F)
    where
        F: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let cm = self.claudemon.clone();
        let tx = self.tx.clone();
        let ok_msg = ok_msg.to_string();
        let reopen = self.chat_session_id();
        tokio::spawn(async move {
            match fut.await {
                Ok(_) => {
                    let _ = tx.send(AppMsg::Toast(ok_msg));
                    fetch_agents(&cm, &tx).await;
                    if let Some(sid) = reopen {
                        fetch_transcript(&cm, &tx, sid).await;
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppMsg::Toast(format!("Failed: {e}")));
                }
            }
        });
    }

    // ── git review pane actions ─────────────────────────────────────────────
}

#[cfg(test)]
mod tests;
