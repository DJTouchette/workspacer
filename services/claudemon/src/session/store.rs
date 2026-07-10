use std::collections::VecDeque;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use dashmap::DashMap;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};

use time::OffsetDateTime;

use super::conversation::{ConversationItem, ConversationStore};
use super::permission_mode::{classify_screen, PermissionMode, PermissionSwitchError};
use super::state::{HookEvent, Pending, Plan, SessionMode, SessionState, StatusLine, Transport};
use crate::protocol::WrapperMessage;

const BROADCAST_CAPACITY: usize = 256;
const HOOK_BROADCAST_CAPACITY: usize = 256;
const STATUS_BROADCAST_CAPACITY: usize = 256;
const OUTPUT_BUFFER_CAP: usize = 256 * 1024; // 256 KiB per session
const BYTE_BROADCAST_CAPACITY: usize = 1024;
/// Max chat messages held per session while it isn't yet accepting input.
/// Bounds memory if a session never reaches `Input` (e.g. stuck on startup);
/// the oldest is dropped (with a warning) past this. In practice the queue
/// holds 0–1, so the cap only bites on a genuinely wedged session.
const MAX_PENDING_MESSAGES: usize = 32;
/// Delay between an `Input` transition and the pending-message flush. The
/// transition is announced by a *hook* (Stop / SessionStart), and Claude Code
/// runs hooks while the turn is still closing — its composer isn't back at the
/// prompt yet. Injecting at that instant types the message into the box but
/// the submitting Enter is treated as mid-turn input and swallowed, leaving
/// the text stranded in the TUI (the "GUI send lands in the TUI box" bug).
const FLUSH_DELAY_MS: u64 = 300;
/// Grace period after a flushed send before verifying the submit took. A
/// successful submit flips the session to `Responding` (UserPromptSubmit
/// hook); still `Input` after this long means the Enter was swallowed and the
/// text is sitting in the composer — a bare CR then submits it.
const SUBMIT_VERIFY_DELAY_MS: u64 = 1000;
/// How many corrective bare-CR passes the verify loop makes before giving up.
/// Each pass waits [`SUBMIT_VERIFY_DELAY_MS`] and only fires while the session
/// is still `Input` (a CR on an empty prompt is a no-op, so a spurious pass is
/// harmless); two passes cover an Enter swallowed twice in a row.
const SUBMIT_VERIFY_ATTEMPTS: u32 = 2;
/// Shift+Tab presses a permission-mode switch may make before concluding the
/// target mode isn't in this session's cycle. Claude Code cycles at most four
/// modes, so six covers a full loop with slack for a double-draw.
const MODE_MAX_PRESSES: u32 = 6;
/// How often the mode switch re-reads the screen while waiting for the footer
/// to react to a press.
const MODE_POLL_MS: u64 = 50;
/// How long a press may go without an observable footer change before the
/// switch gives up (`Unverified`). TUI redraw after Shift+Tab is near-instant;
/// this is generous slack for a loaded machine.
const MODE_CHANGE_TIMEOUT_MS: u64 = 1200;
/// Terminal size assumed for sessions whose size was never reported.
const DEFAULT_TERM_SIZE: (u16, u16) = (80, 24);

/// Tracks the child's bracketed-paste (DECSET 2004) state from its output
/// stream. `enabled` is `None` until either toggle sequence has been seen.
/// `tail` holds the last few bytes of the previous chunk so a toggle sequence
/// split across chunk boundaries is still recognized.
#[derive(Default)]
struct PasteModeTracker {
    enabled: Option<bool>,
    tail: Vec<u8>,
}

const PASTE_ON: &[u8] = b"\x1b[?2004h";
const PASTE_OFF: &[u8] = b"\x1b[?2004l";

impl PasteModeTracker {
    /// Scan a chunk (prefixed with the retained tail) for the *last* paste
    /// toggle it contains and update `enabled`. Returns the new state when a
    /// toggle was seen in this chunk.
    fn scan(&mut self, chunk: &[u8]) -> Option<bool> {
        let mut hay = std::mem::take(&mut self.tail);
        hay.extend_from_slice(chunk);
        let last_on = find_last(&hay, PASTE_ON);
        let last_off = find_last(&hay, PASTE_OFF);
        let seen = match (last_on, last_off) {
            (Some(on), Some(off)) => Some(on > off),
            (Some(_), None) => Some(true),
            (None, Some(_)) => Some(false),
            (None, None) => None,
        };
        if seen.is_some() {
            self.enabled = seen;
        }
        // Keep one sequence-length-minus-one of tail for boundary spanning.
        let keep = hay.len().min(PASTE_ON.len() - 1);
        self.tail = hay[hay.len() - keep..].to_vec();
        seen
    }
}

fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

/// Per-session ring buffer of raw PTY bytes the child has produced so far.
#[derive(Default)]
pub struct OutputBuffer {
    bytes: VecDeque<u8>,
    cap: usize,
}

impl OutputBuffer {
    fn new(cap: usize) -> Self {
        Self {
            bytes: VecDeque::with_capacity(cap.min(8192)),
            cap,
        }
    }

    fn push(&mut self, chunk: &[u8]) {
        if chunk.len() >= self.cap {
            self.bytes.clear();
            let tail = &chunk[chunk.len() - self.cap..];
            self.bytes.extend(tail.iter().copied());
            return;
        }
        let overflow = (self.bytes.len() + chunk.len()).saturating_sub(self.cap);
        for _ in 0..overflow {
            self.bytes.pop_front();
        }
        self.bytes.extend(chunk.iter().copied());
    }

    fn snapshot(&self) -> Vec<u8> {
        self.bytes.iter().copied().collect()
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SessionUpdate {
    pub session_id: String,
    pub event: String,
    pub state: SessionState,
}

/// A statusLine tick for one session. Broadcast on its own channel (not the
/// hook fanout) because the statusLine command fires very frequently — routing
/// it through `hook_tx` would flood the SQLite persistence task.
#[derive(Clone, Debug, serde::Serialize)]
pub struct StatusLineUpdate {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub status_line: StatusLine,
}

/// Handle the daemon keeps for each connected wrapper. Sending into `tx`
/// reaches the wrapper's WebSocket and ultimately the child's stdin / signals.
#[derive(Clone)]
pub struct WrapperHandle {
    pub tx: mpsc::UnboundedSender<WrapperMessage>,
}

#[derive(Clone)]
pub struct SessionStore {
    states: Arc<DashMap<String, SessionState>>,
    wrappers: Arc<DashMap<String, WrapperHandle>>,
    buffers: Arc<DashMap<String, Arc<Mutex<OutputBuffer>>>>,
    bytes_tx: Arc<DashMap<String, broadcast::Sender<Vec<u8>>>>,
    update_tx: broadcast::Sender<SessionUpdate>,
    /// Raw hook events fanout — every inbound HookEvent is broadcast here
    /// before state-machine processing, so clients that want the unaggregated
    /// stream (e.g. a richer external session store) can subscribe.
    hook_tx: broadcast::Sender<HookEvent>,
    /// StatusLine fanout — kept separate from `hook_tx` so the high-frequency
    /// statusLine ticks never reach the SQLite persistence task.
    status_tx: broadcast::Sender<StatusLineUpdate>,
    /// Per-session opt-in for the deferred-hook gateway. When `true`,
    /// PreToolUse hook responses are parked until a client decides
    /// (or until the daemon's timeout fires).
    gates: Arc<DashMap<String, bool>>,
    /// Currently-parked decision for a session, keyed by session_id.
    /// At most one is outstanding because Claude Code is blocked on it.
    decisions: Arc<DashMap<String, oneshot::Sender<Value>>>,
    /// Pending in-daemon spawns indexed by cwd: when claude's `SessionStart`
    /// hook arrives with a matching cwd, we rewrite the hook's session_id to
    /// the spawn's pre-assigned UUID and stash an alias so subsequent hook
    /// events resolve to the same entry.
    pending_spawns_by_cwd: Arc<DashMap<String, String>>,
    /// Alias map: claude's hook session_id → our canonical (spawn) session_id.
    aliases: Arc<DashMap<String, String>>,
    /// Chat messages received via `/message` while the session wasn't yet in
    /// `Input` mode — cold-start `Unknown`, or mid-turn `Responding`. Flushed
    /// in order the instant the session transitions to `Input`, then sent as a
    /// single atomic `line + \r` frame. This is what makes the first message
    /// after spawn reliable instead of racing a raw PTY write against the TUI's
    /// cold-start render (the "typed but not sent" bug).
    pending_messages: Arc<DashMap<String, Vec<String>>>,
    /// When each session last transitioned into `Input`. The scheduled flush
    /// settles [`FLUSH_DELAY_MS`] past this instant, so a send into a prompt
    /// that has been idle for a while injects immediately while a send racing
    /// a just-closed turn waits out the TUI's composer redraw.
    input_since: Arc<DashMap<String, tokio::time::Instant>>,
    /// Monotonic per-session flush generation. Every (re)schedule and every
    /// queue clear bumps it; an in-flight flush/verify task re-checks its
    /// captured epoch at each step and aborts when superseded, so corrective
    /// CRs can never stack up from overlapping tasks.
    flush_epochs: Arc<DashMap<String, u64>>,
    /// When a client last wrote *raw* bytes to the session (terminal
    /// keystrokes via `/input`, picker answers via `/answer`). The verify
    /// ladder aborts if this postdates its flush: the composer content is no
    /// longer known-ours, and a corrective CR could submit a user's draft.
    client_input_at: Arc<DashMap<String, tokio::time::Instant>>,
    /// Bracketed-paste (DECSET 2004) state per session, tracked from the PTY
    /// output stream in [`Self::record_output`]. `send_message_now` frames
    /// chat as a bracketed paste; if the TUI has paste mode *off* (cold-start
    /// trust/OAuth screens), the markers would land as literal text — so the
    /// flush holds while this is explicitly `false` and reschedules when the
    /// enable sequence appears. `None` (never observed) does not gate.
    paste_modes: Arc<DashMap<String, PasteModeTracker>>,
    /// Managed (adapter-driven) sessions route user prompts here instead of to a
    /// PTY: the provider adapter's driver task owns the receiver and forwards
    /// each prompt to the agent's own API (e.g. OpenCode's POST message).
    managed_inputs: Arc<DashMap<String, mpsc::UnboundedSender<String>>>,
    /// Managed approval decisions (true = approve, false = deny). `/approve`
    /// routes the user's decision here for managed sessions; the adapter's
    /// driver forwards it to the provider (OpenCode permission reply / Codex
    /// JSON-RPC approval response).
    managed_decisions: Arc<DashMap<String, mpsc::UnboundedSender<bool>>>,
    /// Live in-daemon PTY children, keyed by session_id, so daemon shutdown can
    /// kill them (they have no `kill_on_drop`, unlike the managed providers'
    /// tokio children) and their exit can be reaped. Without this, quitting the
    /// launcher orphans every `claude` PTY it spawned.
    ptys: Arc<DashMap<String, Arc<crate::wrapper::pty::PtyHandle>>>,
    /// Last-known PTY size (cols, rows) per session — set at spawn/register and
    /// on `/resize`. The live permission-mode switch reconstructs the screen
    /// from the output ring with `vt100`, which needs the real grid to place
    /// the footer rows correctly.
    term_sizes: Arc<DashMap<String, (u16, u16)>>,
    /// Live-switchable auto-approve policy for managed sessions whose adapter
    /// mediates approvals (codex over the app-server ws). The adapter registers
    /// its shared flag at session start; `/permission-mode` flips it. Sessions
    /// without an entry (opencode/pi, codex rollout fallback) can't switch live.
    managed_yolo: Arc<DashMap<String, ManagedYoloHandle>>,
    /// Live model/effort switch channels for managed sessions whose adapter can
    /// apply one mid-thread (codex over the app-server ws:
    /// `thread/settings/update`; the claude stream driver: `set_model`).
    /// Registered by the adapter at session start;
    /// `POST /sessions/:id/model` sends here. Sessions without an entry
    /// (opencode/pi, codex rollout fallback) can't switch live — the caller
    /// falls back to the restart path.
    managed_model: Arc<DashMap<String, mpsc::UnboundedSender<ModelSwitch>>>,
    /// Structural AskUserQuestion answers for managed sessions whose driver can
    /// resolve a parked question over its own protocol (the claude stream
    /// driver's `can_use_tool` allow-with-answers). `/answer` routes here when
    /// present instead of writing picker keystrokes to a PTY.
    managed_answers: Arc<DashMap<String, mpsc::UnboundedSender<ManagedAnswer>>>,
    /// Structural permission-mode switches for managed sessions whose driver
    /// speaks Claude's own mode vocabulary (`set_permission_mode` on the
    /// stream control protocol). Present only for the stream driver — codex
    /// keeps its ask/yolo flag in `managed_yolo`.
    managed_permission_modes: Arc<DashMap<String, mpsc::UnboundedSender<ManagedPermissionSwitch>>>,
    /// Interrupt channels for managed sessions with a structural
    /// SIGINT-equivalent (the stream driver's `interrupt` control request).
    /// `/signal {sigint}` routes here when present.
    managed_interrupts: Arc<DashMap<String, mpsc::UnboundedSender<()>>>,
}

/// A live model/effort switch request for a managed adapter's driver loop.
/// Either field may be absent — absent means "leave as is".
#[derive(Debug, Clone)]
pub struct ModelSwitch {
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// A structural answer to a managed session's pending `AskUserQuestion` —
/// the same vocabulary `POST /sessions/:id/answer` accepts for PTY sessions
/// (option number, free text, or one answer per question), forwarded to the
/// driver instead of being typed into a picker.
#[derive(Debug, Clone)]
pub struct ManagedAnswer {
    /// 1-indexed option for the current (or only) question.
    pub option: Option<u8>,
    /// Free-form text answer.
    pub text: Option<String>,
    /// For multi-question prompts: one answer per question in order — an
    /// option number rendered as a string (`"2"`) or free-form text.
    pub answers: Option<Vec<String>>,
}

/// A live permission-mode switch bound for a managed driver that applies
/// Claude's own modes structurally. The driver resolves `reply` with the mode
/// the CLI confirmed, or the CLI's error string — so `/permission-mode` can
/// answer with verified truth rather than fire-and-forget.
#[derive(Debug)]
pub struct ManagedPermissionSwitch {
    pub mode: String,
    pub reply: oneshot::Sender<Result<String, String>>,
}

/// A managed session's approval policy, shared with its driver task.
#[derive(Clone)]
pub struct ManagedYoloHandle {
    /// Read by the adapter at each approval request: `true` = auto-approve.
    pub live: Arc<std::sync::atomic::AtomicBool>,
    /// Whether the provider's own process was spawned in bypass mode. If so,
    /// approvals are skipped at the source and flipping `live` off can't bring
    /// them back — yolo→ask needs a restart.
    pub spawned_yolo: bool,
}

/// Outcome of a `/message` submission. Keeping the policy here (rather than in
/// the HTTP handler) keeps the handler thin and lets the buffering behavior be
/// unit-tested without standing up axum.
#[derive(Debug, PartialEq, Eq)]
pub enum MessageOutcome {
    /// Accepted for delivery — the session is at the prompt (`Input`). The
    /// write happens via the guarded flush: immediately once the prompt has
    /// settled, and verified afterwards (see `schedule_pending_flush`).
    Sent,
    /// Held until the session reaches `Input` (was `Unknown`/`Responding`,
    /// or paused on `Approval`/`Question`), then flushed through the same
    /// guarded pipeline.
    Queued,
    /// Session has ended (`Stopped`) — there is no prompt to deliver to.
    Rejected(SessionMode),
    /// No session with this id.
    NoSession,
    /// Session exists but has no wrapper attached to receive input.
    NoWrapper,
    /// The wrapper's channel is closed (process gone).
    WrapperGone,
}

impl SessionStore {
    pub fn new() -> Self {
        let (update_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (hook_tx, _) = broadcast::channel(HOOK_BROADCAST_CAPACITY);
        let (status_tx, _) = broadcast::channel(STATUS_BROADCAST_CAPACITY);
        Self {
            states: Arc::new(DashMap::new()),
            wrappers: Arc::new(DashMap::new()),
            buffers: Arc::new(DashMap::new()),
            bytes_tx: Arc::new(DashMap::new()),
            update_tx,
            hook_tx,
            status_tx,
            gates: Arc::new(DashMap::new()),
            decisions: Arc::new(DashMap::new()),
            pending_spawns_by_cwd: Arc::new(DashMap::new()),
            aliases: Arc::new(DashMap::new()),
            pending_messages: Arc::new(DashMap::new()),
            input_since: Arc::new(DashMap::new()),
            flush_epochs: Arc::new(DashMap::new()),
            client_input_at: Arc::new(DashMap::new()),
            paste_modes: Arc::new(DashMap::new()),
            managed_inputs: Arc::new(DashMap::new()),
            managed_decisions: Arc::new(DashMap::new()),
            ptys: Arc::new(DashMap::new()),
            term_sizes: Arc::new(DashMap::new()),
            managed_yolo: Arc::new(DashMap::new()),
            managed_model: Arc::new(DashMap::new()),
            managed_answers: Arc::new(DashMap::new()),
            managed_permission_modes: Arc::new(DashMap::new()),
            managed_interrupts: Arc::new(DashMap::new()),
        }
    }

    // --- in-daemon PTY child lifecycle --------------------------------------

    /// Register a spawned PTY child so it can be killed on daemon shutdown and
    /// reaped on exit.
    pub fn register_pty(&self, session_id: &str, handle: Arc<crate::wrapper::pty::PtyHandle>) {
        self.ptys.insert(session_id.to_string(), handle);
    }

    /// Reap and forget a PTY child after its reader loop ended. Usually the reader
    /// stopped because the child exited (EOF) and `wait()` returns immediately —
    /// but the reader can also break on a transient read *error* while the child
    /// is still alive. To avoid blocking a runtime worker on `wait()` in that case
    /// (and starving `signal_child`/`has_exited`, which take the same mutex), reap
    /// on the blocking pool and kill first if the child hasn't already exited.
    pub fn reap_pty(&self, session_id: &str) {
        let Some((_, handle)) = self.ptys.remove(session_id) else {
            return;
        };
        tokio::task::spawn_blocking(move || {
            let mut child = handle.child.lock().expect("PTY child mutex poisoned");
            // Already exited (the common EOF path) → try_wait reaps it right away.
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            // Reader broke while the child lives → kill so the following wait()
            // returns promptly instead of blocking indefinitely.
            let _ = child.kill();
            let _ = child.wait();
        });
    }

    /// Kill every live in-daemon PTY child. Called on daemon shutdown so the
    /// `claude` processes it spawned don't outlive the daemon (and the launcher).
    pub fn kill_all_ptys(&self) {
        for entry in self.ptys.iter() {
            let _ =
                crate::wrapper::pty::signal_child(entry.value(), crate::protocol::Signal::Sigkill);
        }
    }

    /// Repopulate the in-memory session list from persisted rows at startup,
    /// marking each as [`SessionMode::Stopped`]. The processes themselves are
    /// gone (they were the previous daemon's children), but the rows let clients
    /// see prior agents again and resume them — a respawn launches
    /// `claude --resume <id>`, and because we pin `--session-id` at spawn the row
    /// id doubles as claude's transcript uuid, so the conversation reopens rather
    /// than starting blank. A live entry (none exist at boot) always wins.
    pub fn hydrate(&self, sessions: Vec<crate::store::RestoredSession>) {
        for s in sessions {
            self.states.entry(s.id.clone()).or_insert_with(|| {
                let mut st = SessionState::new(s.id.clone(), s.cwd.clone());
                st.mode = SessionMode::Stopped;
                st.tool_calls = s.tool_calls;
                if let Ok(t) = OffsetDateTime::from_unix_timestamp(s.created_at) {
                    st.started_at = t;
                }
                if let Ok(t) = OffsetDateTime::from_unix_timestamp(s.last_event_at) {
                    st.updated_at = t;
                }
                st
            });
        }
    }

    // --- deferred-hook gateway ----------------------------------------------

    pub fn set_gate(&self, session_id: &str, on: bool) {
        if on {
            self.gates.insert(session_id.to_string(), true);
        } else {
            self.gates.remove(session_id);
            // If we're disabling the gate while a decision is parked, drop the
            // sender so the hook handler falls through to passthrough.
            self.decisions.remove(session_id);
        }
    }

    pub fn gate_enabled(&self, session_id: &str) -> bool {
        self.gates.get(session_id).map(|e| *e).unwrap_or(false)
    }

    /// Park a decision channel for this session and flip mode to Approval.
    /// Returns a receiver the caller awaits; another caller (typically
    /// `/decide` or `/approve`) resolves it via `resolve_decision`.
    pub fn park_decision(
        &self,
        session_id: &str,
        tool: Option<String>,
        raw: Value,
    ) -> oneshot::Receiver<Value> {
        let (tx, rx) = oneshot::channel();
        // If there's already a pending decision, drop the old sender so
        // its waiter falls through. Shouldn't happen in practice because
        // Claude blocks on the hook, but keeps us safe under re-entrancy.
        self.decisions.insert(session_id.to_string(), tx);

        // Flip the observable state to Approval and surface the tool info
        // in `pending` so clients can render the right picker.
        let updated = {
            if let Some(mut state) = self.states.get_mut(session_id) {
                state.mode = SessionMode::Approval;
                let summary = raw
                    .get("tool_input")
                    .and_then(|ti| ti.get("command").or_else(|| ti.get("description")))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                state.pending = Some(Pending::Approval {
                    tool: tool.clone(),
                    summary,
                    raw,
                });
                Some(state.clone())
            } else {
                None
            }
        };
        if let Some(state) = updated {
            let _ = self.update_tx.send(SessionUpdate {
                session_id: session_id.to_string(),
                event: "PreToolUse".to_string(),
                state,
            });
        }
        rx
    }

    pub fn resolve_decision(&self, session_id: &str, decision: Value) -> bool {
        match self.decisions.remove(session_id) {
            Some((_, tx)) => tx.send(decision).is_ok(),
            None => false,
        }
    }

    pub fn has_pending_decision(&self, session_id: &str) -> bool {
        self.decisions.contains_key(session_id)
    }

    pub fn clear_pending_decision(&self, session_id: &str) {
        self.decisions.remove(session_id);
    }

    pub fn ingest(&self, mut event: HookEvent) -> SessionState {
        // Alias resolution: if claude's session_id has already been mapped to
        // our canonical (spawn-side) id, rewrite. For the first SessionStart of
        // a spawn, register the alias by looking up the pending spawn by cwd.
        if let Some(canonical) = self.aliases.get(&event.session_id).map(|e| e.clone()) {
            event.session_id = canonical;
        } else if !self.states.contains_key(&event.session_id) && event.event == "SessionStart" {
            // Only guess by cwd when we don't already know this id. When the
            // caller pinned `--session-id`, claude's hook id *is* our spawn id
            // (already in `states`), so we must skip the cwd guess — otherwise a
            // sibling spawn sharing the cwd could steal this session's hooks.
            if let Some(cwd) = event.cwd.clone() {
                if let Some((_, canonical)) = self.pending_spawns_by_cwd.remove(&cwd) {
                    self.aliases
                        .insert(event.session_id.clone(), canonical.clone());
                    event.session_id = canonical;
                }
            }
        }

        // Broadcast the *post-aliasing* event so subscribers see the canonical
        // session_id Workspacer (and other clients) already know about.
        let _ = self.hook_tx.send(event.clone());

        // Managed sessions own their mode state machine in the driver (the
        // stream driver via the control protocol; codex/opencode/pi via their
        // native events) — a hook must not fight it, and the PTY flush
        // pipeline it feeds has no PTY here. Claude Code still runs the user's
        // hooks for headless stream sessions, so this path is hit routinely:
        // keep the hooks as enrichment only — capture `transcript_path` (the
        // `/transcript` endpoint needs it) and rely on the rebroadcast above.
        let managed = self.managed_inputs.contains_key(&event.session_id)
            || self
                .states
                .get(&event.session_id)
                .is_some_and(|s| s.transport == Transport::Stream);
        if managed {
            if let Some(mut entry) = self.states.get_mut(&event.session_id) {
                if let Some(tp) = event.payload.get("transcript_path").and_then(Value::as_str) {
                    entry.transcript_path = Some(tp.to_string());
                }
                entry.updated_at = OffsetDateTime::now_utc();
                return entry.clone();
            }
            // Managed input registered but no state row (teardown race) —
            // fall through to the normal path, which creates one.
        }

        let (state, became_input, became_stopped) = {
            let mut entry = self
                .states
                .entry(event.session_id.clone())
                .or_insert_with(|| SessionState::new(event.session_id.clone(), event.cwd.clone()));
            let prev_mode = entry.mode;
            entry.apply(&event);
            let became_input = entry.mode == SessionMode::Input && prev_mode != SessionMode::Input;
            let became_stopped = entry.mode == SessionMode::Stopped;
            (entry.clone(), became_input, became_stopped)
        };
        // Drain or drop any queued chat messages — done outside the `states`
        // entry lock above (flush touches `wrappers`, not `states`).
        if became_input {
            // Stamp the transition so the scheduled flush can settle relative
            // to it: a send into a long-idle prompt injects immediately, a send
            // racing this very transition waits out the composer redraw.
            self.input_since
                .insert(event.session_id.clone(), tokio::time::Instant::now());
            self.schedule_pending_flush(&event.session_id);
        } else if became_stopped {
            self.clear_pending_messages(&event.session_id);
        }
        let _ = self.update_tx.send(SessionUpdate {
            session_id: event.session_id.clone(),
            event: event.event.clone(),
            state: state.clone(),
        });
        state
    }

    /// Apply a Claude Code statusLine payload to its session.
    ///
    /// The statusLine JSON carries Claude's *own* session id (same id its hooks
    /// use), so we resolve it through the same alias map `ingest` builds —
    /// landing on the canonical (spawn-side) id Workspacer knows. No-op if the
    /// session isn't registered yet: the statusLine command fires repeatedly,
    /// so the next tick lands once `SessionStart` has created the alias. Returns
    /// the updated state (and broadcasts a `StatusLine` update) when matched.
    pub fn ingest_status_line(&self, raw: &Value) -> Option<SessionState> {
        let sid = raw.get("session_id").and_then(Value::as_str)?;
        let canonical = self
            .aliases
            .get(sid)
            .map(|e| e.clone())
            .unwrap_or_else(|| sid.to_string());

        let status = StatusLine::from_claude_json(raw);
        let state = {
            let mut entry = self.states.get_mut(&canonical)?;
            let session = entry.value_mut();
            session.status_line = Some(status.clone());
            session.updated_at = OffsetDateTime::now_utc();
            session.clone()
        };
        let _ = self.status_tx.send(StatusLineUpdate {
            session_id: canonical,
            cwd: state.cwd.clone(),
            status_line: status,
        });
        Some(state)
    }

    pub fn list(&self) -> Vec<SessionState> {
        self.states.iter().map(|e| e.value().clone()).collect()
    }

    pub fn get(&self, session_id: &str) -> Option<SessionState> {
        self.states.get(session_id).map(|e| e.clone())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionUpdate> {
        self.update_tx.subscribe()
    }

    pub fn subscribe_hooks(&self) -> broadcast::Receiver<HookEvent> {
        self.hook_tx.subscribe()
    }

    pub fn subscribe_status_lines(&self) -> broadcast::Receiver<StatusLineUpdate> {
        self.status_tx.subscribe()
    }

    // --- wrapper-driven session lifecycle -----------------------------------

    pub fn register_wrapper(
        &self,
        session_id: &str,
        cwd: &str,
        handle: WrapperHandle,
    ) -> SessionState {
        // Treat wrapper registration as a synthetic SessionStart so the state
        // machine produces the same observable behavior as hook-driven starts.
        let synthetic = HookEvent {
            event: "SessionStart".to_string(),
            session_id: session_id.to_string(),
            cwd: Some(cwd.to_string()),
            timestamp: None,
            payload: serde_json::Map::new(),
        };
        let state = self.ingest(synthetic);
        self.wrappers.insert(session_id.to_string(), handle);
        self.buffers
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(OutputBuffer::new(OUTPUT_BUFFER_CAP))));
        let (tx, _) = broadcast::channel(BYTE_BROADCAST_CAPACITY);
        self.bytes_tx.insert(session_id.to_string(), tx);
        // Fresh child, unknown terminal state — a stale paste-mode reading from
        // a previous life of this session id must not gate (or ungate) sends.
        self.paste_modes.remove(session_id);
        state
    }

    pub fn deregister_wrapper(&self, session_id: &str) {
        self.wrappers.remove(session_id);
        self.buffers.remove(session_id);
        self.bytes_tx.remove(session_id);
        let synthetic = HookEvent {
            event: "SessionEnd".to_string(),
            session_id: session_id.to_string(),
            cwd: None,
            timestamp: None,
            payload: serde_json::Map::new(),
        };
        let _ = self.ingest(synthetic);
        // Sweep the per-session auxiliary maps so they don't accrue a permanent
        // entry per session across spawn/stop churn (drop_pending_spawn does the
        // same). Done after the SessionEnd ingest so anything it touches (e.g.
        // the flush epoch) is cleared too. `states` is intentionally kept — the
        // session lingers as a resumable Stopped row.
        self.input_since.remove(session_id);
        self.flush_epochs.remove(session_id);
        self.client_input_at.remove(session_id);
        self.paste_modes.remove(session_id);
        self.term_sizes.remove(session_id);
    }

    /// Register an in-daemon spawn before claude's SessionStart hook fires.
    /// The session is created upfront with our chosen session_id so clients
    /// can immediately subscribe to bytes, send input, etc. We also remember
    /// the cwd so that when claude's SessionStart arrives later we can alias
    /// claude's session_id to ours (see `ingest`).
    pub fn register_spawn(
        &self,
        session_id: &str,
        cwd: &str,
        handle: WrapperHandle,
    ) -> SessionState {
        let state = {
            let entry = self
                .states
                .entry(session_id.to_string())
                .or_insert_with(|| {
                    SessionState::new(session_id.to_string(), Some(cwd.to_string()))
                });
            entry.clone()
        };
        self.wrappers.insert(session_id.to_string(), handle);
        self.buffers
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(OutputBuffer::new(OUTPUT_BUFFER_CAP))));
        self.bytes_tx
            .entry(session_id.to_string())
            .or_insert_with(|| broadcast::channel(BYTE_BROADCAST_CAPACITY).0);
        self.pending_spawns_by_cwd
            .insert(cwd.to_string(), session_id.to_string());
        // Fresh child — drop any paste-mode reading from a previous life of
        // this session id (resume reuses it).
        self.paste_modes.remove(session_id);
        let _ = self.update_tx.send(SessionUpdate {
            session_id: session_id.to_string(),
            event: "Spawn".to_string(),
            state: state.clone(),
        });
        state
    }

    // --- managed (adapter-driven) sessions ----------------------------------
    //
    // A "managed" session is one whose telemetry comes from a provider adapter
    // (OpenCode `serve`, Codex `app-server`) rather than Claude's hooks +
    // transcript + statusLine. The adapter drives the state machine and
    // conversation directly via the methods below; there is no PTY wrapper or
    // byte buffer (observation is structured, not a terminal stream).

    /// Register a managed session and announce it like a spawn. Starts in
    /// `Input` (ready to accept a prompt). Idempotent on the id.
    ///
    /// A managed session emits no PTY bytes, but we still create an (empty)
    /// output buffer + byte channel so the renderer's viewer-attach path
    /// (`/sessions/:id/stream`) works uniformly — it simply never receives any
    /// bytes; the conversation/state/status streams carry the real telemetry.
    pub fn register_managed(&self, session_id: &str, cwd: &str, provider: &str) -> SessionState {
        let state = {
            let mut entry = self
                .states
                .entry(session_id.to_string())
                .or_insert_with(|| {
                    SessionState::new(session_id.to_string(), Some(cwd.to_string()))
                });
            entry.mode = SessionMode::Input;
            entry.provider = provider.to_string();
            entry.clone()
        };
        self.buffers
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(OutputBuffer::new(OUTPUT_BUFFER_CAP))));
        self.bytes_tx
            .entry(session_id.to_string())
            .or_insert_with(|| broadcast::channel(BYTE_BROADCAST_CAPACITY).0);
        let _ = self.update_tx.send(SessionUpdate {
            session_id: session_id.to_string(),
            event: "Spawn".to_string(),
            state: state.clone(),
        });
        state
    }

    /// Attach a PTY wrapper to an already-registered (managed) session so its
    /// terminal input (`POST /sessions/:id/input`) reaches the child and its
    /// output flows through `record_output` onto the byte stream. Used by hybrid
    /// agents (e.g. OpenCode `attach`) that pair a structured GUI adapter with a
    /// live TUI in a PTY — the GUI and Term are then two views of one session.
    /// The byte buffer + channel already exist from `register_managed`; this just
    /// adds the input wrapper (and is defensive about the buffer/channel).
    pub fn attach_pty(&self, session_id: &str, handle: WrapperHandle) {
        self.wrappers.insert(session_id.to_string(), handle);
        self.buffers
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(OutputBuffer::new(OUTPUT_BUFFER_CAP))));
        self.bytes_tx
            .entry(session_id.to_string())
            .or_insert_with(|| broadcast::channel(BYTE_BROADCAST_CAPACITY).0);
    }

    /// Drive a managed session's mode (and optional pending) directly, since
    /// managed backends don't emit Claude hooks. Broadcasts a SessionUpdate.
    /// Returns None if the session isn't registered.
    pub fn set_managed_mode(
        &self,
        session_id: &str,
        mode: SessionMode,
        pending: Option<Pending>,
    ) -> Option<SessionState> {
        let state = {
            let mut entry = self.states.get_mut(session_id)?;
            entry.mode = mode;
            entry.pending = pending;
            entry.updated_at = OffsetDateTime::now_utc();
            entry.clone()
        };
        let _ = self.update_tx.send(SessionUpdate {
            session_id: session_id.to_string(),
            event: "Managed".to_string(),
            state: state.clone(),
        });
        Some(state)
    }

    /// Mark which transport drives this session. Set at spawn time (before the
    /// driver task starts) so `ingest`'s hooks guard and every snapshot see it
    /// from the session's first instant. No-op for unknown ids.
    pub fn set_transport(&self, session_id: &str, transport: Transport) {
        if let Some(mut entry) = self.states.get_mut(session_id) {
            entry.transport = transport;
        }
    }

    /// Register the prompt channel for a managed session. Prompts submitted via
    /// `submit_message` are forwarded here (the adapter's driver task owns the
    /// receiver).
    pub fn register_managed_input(&self, session_id: &str, tx: mpsc::UnboundedSender<String>) {
        self.managed_inputs.insert(session_id.to_string(), tx);
    }

    /// Register the structural AskUserQuestion answer channel for a managed
    /// session (the stream driver resolves the parked `can_use_tool` with the
    /// user's choices).
    pub fn register_managed_answer(
        &self,
        session_id: &str,
        tx: mpsc::UnboundedSender<ManagedAnswer>,
    ) {
        self.managed_answers.insert(session_id.to_string(), tx);
    }

    /// Forward an AskUserQuestion answer to a managed session's driver.
    /// Returns false (so `/answer` falls through to the PTY keystroke path)
    /// when this session has no structural answer channel.
    pub fn submit_managed_answer(&self, session_id: &str, answer: ManagedAnswer) -> bool {
        match self.managed_answers.get(session_id) {
            Some(tx) => tx.send(answer).is_ok(),
            None => false,
        }
    }

    /// Register the structural permission-mode channel for a managed session
    /// whose driver speaks Claude's own mode vocabulary (stream driver).
    pub fn register_managed_permission_mode(
        &self,
        session_id: &str,
        tx: mpsc::UnboundedSender<ManagedPermissionSwitch>,
    ) {
        self.managed_permission_modes
            .insert(session_id.to_string(), tx);
    }

    /// Whether this managed session can switch Claude permission modes
    /// structurally (drives `/permission-mode` routing).
    pub fn has_managed_permission_mode(&self, session_id: &str) -> bool {
        self.managed_permission_modes.contains_key(session_id)
    }

    /// Forward a structural permission-mode switch to the driver. Returns
    /// false when the session has no such channel (or the driver is gone).
    pub fn submit_managed_permission_mode(
        &self,
        session_id: &str,
        switch: ManagedPermissionSwitch,
    ) -> bool {
        match self.managed_permission_modes.get(session_id) {
            Some(tx) => tx.send(switch).is_ok(),
            None => false,
        }
    }

    /// Register the structural interrupt channel for a managed session (the
    /// stream driver's SIGINT equivalent — an `interrupt` control request).
    pub fn register_managed_interrupt(&self, session_id: &str, tx: mpsc::UnboundedSender<()>) {
        self.managed_interrupts.insert(session_id.to_string(), tx);
    }

    /// Interrupt a managed session's current turn. Returns false when the
    /// session has no structural interrupt (caller falls back to the PTY /
    /// terminate paths).
    pub fn interrupt_managed(&self, session_id: &str) -> bool {
        match self.managed_interrupts.get(session_id) {
            Some(tx) => tx.send(()).is_ok(),
            None => false,
        }
    }

    /// Register the approval-decision channel for a managed session. `/approve`
    /// routes the user's yes/no here (the adapter forwards it to the provider).
    pub fn register_managed_decision(&self, session_id: &str, tx: mpsc::UnboundedSender<bool>) {
        self.managed_decisions.insert(session_id.to_string(), tx);
    }

    /// Register the live model-switch channel for a managed session whose
    /// adapter can apply one mid-thread (codex: `thread/settings/update`).
    pub fn register_managed_model_switch(
        &self,
        session_id: &str,
        tx: mpsc::UnboundedSender<ModelSwitch>,
    ) {
        self.managed_model.insert(session_id.to_string(), tx);
    }

    /// Live-switch a managed session's model/effort without a restart. Err when
    /// the session has no switch channel (provider can't do it live — opencode/
    /// pi, or codex running on the rollout fallback) so the caller can offer
    /// the restart path instead.
    pub fn set_managed_model(
        &self,
        session_id: &str,
        switch: ModelSwitch,
    ) -> Result<(), &'static str> {
        match self.managed_model.get(session_id) {
            Some(tx) if tx.send(switch).is_ok() => Ok(()),
            _ => {
                Err("this session's provider can't switch models live — restart with the new model")
            }
        }
    }

    /// Forward an approval decision to a managed session's adapter. Returns
    /// false (so the caller falls through to the Claude hook path) when this
    /// isn't a managed session.
    pub fn submit_managed_decision(&self, session_id: &str, approve: bool) -> bool {
        match self.managed_decisions.get(session_id) {
            Some(tx) => tx.send(approve).is_ok(),
            None => false,
        }
    }

    /// Whether this session is adapter-driven (OpenCode/Codex/Pi), i.e. it has a
    /// managed prompt channel rather than a Claude hook + PTY lifecycle.
    pub fn is_managed(&self, session_id: &str) -> bool {
        self.managed_inputs.contains_key(session_id)
    }

    /// Externally terminate a managed session. Dropping its prompt channel makes
    /// the adapter's driver loop see `rx.recv() == None` and break, which runs its
    /// cleanup (kills the provider server + TUI child) and then calls
    /// `deregister_managed`. This is the only external kill path for managed
    /// sessions — without it, closing a pane leaves the `codex app-server` /
    /// `opencode serve` process and its driver task running forever.
    pub fn terminate_managed(&self, session_id: &str) -> bool {
        // Removing the sender drops it (submit_message only holds transient
        // clones), so the driver's `rx.recv()` resolves to None and the loop exits.
        let existed = self.managed_inputs.remove(session_id).is_some();
        self.managed_decisions.remove(session_id);
        self.managed_model.remove(session_id);
        self.managed_yolo.remove(session_id);
        self.managed_answers.remove(session_id);
        self.managed_permission_modes.remove(session_id);
        self.managed_interrupts.remove(session_id);
        existed
    }

    /// Tear down a managed session: drop its prompt + decision channels
    /// (signalling the driver to stop), release its terminal resources (the
    /// attached TUI's byte buffer + broadcast + input wrapper), and mark it
    /// Stopped. Idempotent — safe whether reached via `terminate_managed` or the
    /// driver loop exiting on its own.
    pub fn deregister_managed(&self, session_id: &str) {
        self.managed_inputs.remove(session_id);
        self.managed_decisions.remove(session_id);
        self.managed_model.remove(session_id);
        self.managed_yolo.remove(session_id);
        self.managed_answers.remove(session_id);
        self.managed_permission_modes.remove(session_id);
        self.managed_interrupts.remove(session_id);
        // Release the hybrid Term view's resources (attached by `attach_pty`).
        // The 256 KiB byte ring per session is the bulk of a managed session's
        // memory; leaving it (and the input wrapper + broadcast) around after the
        // session ends is a slow leak across spawn/stop churn.
        self.wrappers.remove(session_id);
        self.buffers.remove(session_id);
        self.bytes_tx.remove(session_id);
        if let Some(mut entry) = self.states.get_mut(session_id) {
            entry.mode = SessionMode::Stopped;
            entry.updated_at = OffsetDateTime::now_utc();
            let state = entry.clone();
            drop(entry);
            let _ = self.update_tx.send(SessionUpdate {
                session_id: session_id.to_string(),
                event: "SessionEnd".to_string(),
                state,
            });
        }
    }

    fn managed_input(&self, session_id: &str) -> Option<mpsc::UnboundedSender<String>> {
        self.managed_inputs.get(session_id).map(|e| e.clone())
    }

    /// Attach model/usage/cost telemetry to a managed session (the adapter's
    /// equivalent of Claude's statusLine). Broadcasts on the status channel.
    pub fn apply_status_line(&self, session_id: &str, status: StatusLine) -> Option<SessionState> {
        let state = {
            let mut entry = self.states.get_mut(session_id)?;
            entry.status_line = Some(status.clone());
            entry.updated_at = OffsetDateTime::now_utc();
            entry.clone()
        };
        let _ = self.status_tx.send(StatusLineUpdate {
            session_id: session_id.to_string(),
            cwd: state.cwd.clone(),
            status_line: status,
        });
        Some(state)
    }

    /// Record the agent's current plan and surface it to clients.
    ///
    /// Two effects, one call so there's a single emission path: (1) store the
    /// plan on the session state (auto-serialized in `GET /sessions/:id`), and
    /// (2) push a `plan` conversation item onto `conv` so the live SSE delta and
    /// any resync replay both deliver it. Last-write-wins: each call fully
    /// replaces the prior plan. Storing on the state is skipped (but the item is
    /// still pushed) if the session isn't registered — the conversation log is
    /// keyed independently, so a plan never gets lost on a timing edge.
    pub fn set_plan(
        &self,
        conv: &ConversationStore,
        session_id: &str,
        plan: Plan,
    ) -> Option<SessionState> {
        let item = ConversationItem::Plan {
            steps: plan.steps.clone(),
            updated_at: plan.updated_at.clone(),
        };
        let state = self.states.get_mut(session_id).map(|mut entry| {
            entry.plan = Some(plan);
            entry.updated_at = OffsetDateTime::now_utc();
            entry.clone()
        });
        conv.push(session_id, vec![item]);
        if let Some(state) = &state {
            let _ = self.update_tx.send(SessionUpdate {
                session_id: session_id.to_string(),
                event: "Plan".to_string(),
                state: state.clone(),
            });
        }
        state
    }

    /// Drop a previously-registered spawn that has not yet bound to a claude
    /// hook session. Used when /sessions/spawn fails after partial setup or
    /// the child exits before SessionStart fires.
    pub fn drop_pending_spawn(&self, session_id: &str, cwd: &str) {
        self.pending_spawns_by_cwd.remove(cwd);
        self.wrappers.remove(session_id);
        self.buffers.remove(session_id);
        self.bytes_tx.remove(session_id);
        self.pending_messages.remove(session_id);
        self.input_since.remove(session_id);
        self.flush_epochs.remove(session_id);
        self.client_input_at.remove(session_id);
        self.paste_modes.remove(session_id);
        self.term_sizes.remove(session_id);
        self.states.remove(session_id);
        // Drop any hook-id → canonical-id aliases pointing at this session, so the
        // alias map doesn't accrue a permanent entry per spawn across churn.
        self.aliases.retain(|_, canonical| canonical != session_id);
    }

    pub fn wrapper(&self, session_id: &str) -> Option<WrapperHandle> {
        self.wrappers.get(session_id).map(|h| h.clone())
    }

    // --- terminal size ------------------------------------------------------

    /// Record the session's PTY size — called at spawn/register and on
    /// `/resize` so screen reconstruction uses the real grid.
    pub fn note_term_size(&self, session_id: &str, cols: u16, rows: u16) {
        if cols == 0 || rows == 0 {
            return;
        }
        self.term_sizes.insert(session_id.to_string(), (cols, rows));
    }

    fn term_size(&self, session_id: &str) -> (u16, u16) {
        self.term_sizes
            .get(session_id)
            .map(|s| *s)
            .unwrap_or(DEFAULT_TERM_SIZE)
    }

    // --- live permission-mode switch ------------------------------------------

    /// The session's current permission mode as shown by its TUI footer,
    /// reconstructed from the output ring buffer. `None` when the session has
    /// no output buffer (no wrapper ever attached).
    pub async fn screen_permission_mode(&self, session_id: &str) -> Option<PermissionMode> {
        let bytes = self.output_snapshot(session_id).await?;
        let (cols, rows) = self.term_size(session_id);
        let mut parser = vt100::Parser::new(rows, cols, 0);
        parser.process(&bytes);
        Some(classify_screen(&parser.screen().contents()))
    }

    /// Wait for the footer's mode classification to move off `prev` after a
    /// Shift+Tab press. Polls the reconstructed screen every [`MODE_POLL_MS`]
    /// for up to [`MODE_CHANGE_TIMEOUT_MS`]; `None` = no observable change.
    async fn await_mode_change(
        &self,
        session_id: &str,
        prev: PermissionMode,
    ) -> Option<PermissionMode> {
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(MODE_CHANGE_TIMEOUT_MS);
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(MODE_POLL_MS)).await;
            if let Some(mode) = self.screen_permission_mode(session_id).await {
                if mode != prev {
                    return Some(mode);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
        }
    }

    /// Switch a live PTY session's permission mode without a restart, the way
    /// a human would: press Shift+Tab (the TUI's mode cycle) and watch the
    /// footer marker until the target mode is showing. Every press is verified
    /// against the reconstructed screen, so the loop stops on the target,
    /// detects a full cycle without it (`Unavailable` — conveniently already
    /// back at the starting mode), and never sprays blind keystrokes
    /// (`Unverified` after one unacknowledged press).
    ///
    /// Only `Input`/`Responding` sessions are eligible: while a dialog is up
    /// (`Approval`/`Question`) Shift+Tab could act on the dialog instead.
    pub async fn set_permission_mode(
        &self,
        session_id: &str,
        target: PermissionMode,
    ) -> Result<PermissionMode, PermissionSwitchError> {
        if self.is_managed(session_id) {
            return Err(PermissionSwitchError::Managed);
        }
        let Some(mode) = self.states.get(session_id).map(|s| s.mode) else {
            return Err(PermissionSwitchError::NoSession);
        };
        if !matches!(mode, SessionMode::Input | SessionMode::Responding) {
            return Err(PermissionSwitchError::Busy(mode));
        }
        let Some(handle) = self.wrapper(session_id) else {
            return Err(PermissionSwitchError::NoWrapper);
        };
        let Some(start) = self.screen_permission_mode(session_id).await else {
            return Err(PermissionSwitchError::NoWrapper);
        };
        if start == target {
            return Ok(start);
        }
        let mut current = start;
        for _ in 0..MODE_MAX_PRESSES {
            if handle
                .tx
                .send(WrapperMessage::Input {
                    bytes: B64.encode(b"\x1b[Z"),
                })
                .is_err()
            {
                return Err(PermissionSwitchError::NoWrapper);
            }
            let Some(next) = self.await_mode_change(session_id, current).await else {
                return Err(PermissionSwitchError::Unverified(current));
            };
            current = next;
            if current == target {
                return Ok(current);
            }
            if current == start {
                return Err(PermissionSwitchError::Unavailable(current));
            }
        }
        Err(PermissionSwitchError::Unavailable(current))
    }

    /// Register a managed session's live approval-policy flag. The adapter
    /// keeps the `Arc` and reads it at every approval request; `spawned_yolo`
    /// records whether the provider process itself was started in bypass mode
    /// (in which case approvals can't be re-enabled live).
    pub fn register_managed_yolo(
        &self,
        session_id: &str,
        live: Arc<std::sync::atomic::AtomicBool>,
        spawned_yolo: bool,
    ) {
        self.managed_yolo.insert(
            session_id.to_string(),
            ManagedYoloHandle { live, spawned_yolo },
        );
    }

    /// Live-switch a managed session's permission mode (`"ask"` / `"yolo"`).
    ///
    /// `ask → yolo` always works: the adapter starts auto-approving the
    /// provider's approval requests (already-parked requests stay parked for
    /// the user — only *new* ones auto-approve). `yolo → ask` works only when
    /// the provider process wasn't itself spawned in bypass mode; otherwise it
    /// never sends approval requests and the switch would be a silent no-op —
    /// reported as `ManagedUnavailable` so the caller can offer a restart.
    pub fn set_managed_permission_mode(
        &self,
        session_id: &str,
        mode: &str,
    ) -> Result<&'static str, PermissionSwitchError> {
        if !self.is_managed(session_id) {
            return if self.states.contains_key(session_id) {
                Err(PermissionSwitchError::NoWrapper)
            } else {
                Err(PermissionSwitchError::NoSession)
            };
        }
        let Some(handle) = self.managed_yolo.get(session_id).map(|h| h.clone()) else {
            return Err(PermissionSwitchError::Managed);
        };
        match mode {
            "yolo" => {
                handle
                    .live
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                Ok("yolo")
            }
            "ask" => {
                if handle.spawned_yolo {
                    return Err(PermissionSwitchError::ManagedUnavailable { current: "yolo" });
                }
                handle
                    .live
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                Ok("ask")
            }
            _ => Err(PermissionSwitchError::Managed),
        }
    }

    // --- chat message submission --------------------------------------------

    /// Submit a chat message. Every live mode accepts it: at the prompt
    /// (`Input`) it goes through the guarded flush right away; in any other
    /// live mode (`Unknown`/`Responding`/`Approval`/`Question`) it is held and
    /// flushed when the session next reaches `Input`. Queuing during
    /// `Approval`/`Question` matters: typing into an open dialog would answer
    /// the dialog, not deliver the message — the old `Rejected` outcome pushed
    /// callers into exactly that raw-PTY fallback. Only a `Stopped` session
    /// rejects. See [`MessageOutcome`].
    pub fn submit_message(&self, session_id: &str, text: String) -> MessageOutcome {
        // Managed (adapter-driven) sessions forward the prompt to the provider's
        // own API via the driver task — no PTY, no Input-mode gating.
        if let Some(tx) = self.managed_input(session_id) {
            return if tx.send(text).is_ok() {
                MessageOutcome::Sent
            } else {
                MessageOutcome::WrapperGone
            };
        }
        let Some(mode) = self.states.get(session_id).map(|s| s.mode) else {
            return MessageOutcome::NoSession;
        };
        match mode {
            SessionMode::Stopped => MessageOutcome::Rejected(SessionMode::Stopped),
            SessionMode::Input => {
                if self.wrapper(session_id).is_none() {
                    return MessageOutcome::NoWrapper;
                }
                // Even at the prompt the send is routed through the scheduled
                // flush rather than written inline: the hook that announced
                // `Input` fires while the TUI is still closing the turn, so an
                // instant injection can have its Enter swallowed exactly like a
                // queued one. The flush settles FLUSH_DELAY_MS past the Input
                // transition (a no-op wait when the prompt has been idle) and
                // then verifies the submit took.
                self.enqueue_message(session_id, text);
                self.schedule_pending_flush(session_id);
                MessageOutcome::Sent
            }
            _ => {
                self.enqueue_message(session_id, text);
                // Guard the read-then-enqueue against a concurrent transition:
                // if `ingest` flipped the session to `Input` after our mode read
                // but before the enqueue, drain here. Both drains are atomic
                // (`mem::take` under the shard lock), so this and the `ingest`
                // flush can never double-send or lose the message.
                if self.states.get(session_id).map(|s| s.mode) == Some(SessionMode::Input) {
                    self.schedule_pending_flush(session_id);
                }
                MessageOutcome::Queued
            }
        }
    }

    fn enqueue_message(&self, session_id: &str, text: String) {
        let mut q = self
            .pending_messages
            .entry(session_id.to_string())
            .or_default();
        if q.len() >= MAX_PENDING_MESSAGES {
            // Bound memory under a stuck session. The caller was already told
            // "queued", so this is silent loss from its perspective — warn so
            // a wedged session is at least visible in the daemon log.
            let dropped = q.remove(0);
            tracing::warn!(
                session_id,
                dropped = %dropped.chars().take(80).collect::<String>(),
                "pending-message queue full; dropping oldest"
            );
        }
        q.push(text);
    }

    /// Bump the session's flush generation, invalidating any in-flight
    /// flush/verify task, and return the fresh value for a new task to carry.
    fn bump_flush_epoch(&self, session_id: &str) -> u64 {
        let mut entry = self.flush_epochs.entry(session_id.to_string()).or_insert(0);
        *entry += 1;
        *entry
    }

    fn flush_epoch_is(&self, session_id: &str, epoch: u64) -> bool {
        self.flush_epochs.get(session_id).map(|e| *e) == Some(epoch)
    }

    /// Record that a client wrote raw bytes to this session (terminal
    /// keystrokes, picker answers). Called by the `/input` and `/answer`
    /// handlers so an in-flight verify ladder knows the composer is no longer
    /// exclusively ours and stands down.
    pub fn note_client_input(&self, session_id: &str) {
        self.client_input_at
            .insert(session_id.to_string(), tokio::time::Instant::now());
    }

    /// Whether the session's TUI has bracketed paste *explicitly* disabled.
    /// Unknown (never observed either toggle) does not gate — some transports
    /// attach mid-stream and would otherwise never flush.
    fn paste_mode_off(&self, session_id: &str) -> bool {
        self.paste_modes
            .get(session_id)
            .is_some_and(|t| t.enabled == Some(false))
    }

    fn client_typed_since(&self, session_id: &str, when: tokio::time::Instant) -> bool {
        // `>=`, not `>`: input landing at the same instant as the flush is
        // exactly the ambiguity the guard exists for (and the paused test
        // clock only moves during sleeps, so simultaneous stamps are common).
        self.client_input_at
            .get(session_id)
            .is_some_and(|t| *t >= when)
    }

    /// Flush queued messages once the TUI is actually ready. The `Input`
    /// transition is announced by a hook, which Claude Code runs *before* its
    /// composer is back at the prompt — flushing synchronously there types the
    /// message into the box but the submitting Enter gets swallowed as mid-turn
    /// input, stranding the text in the TUI (seen when a GUI send raced a
    /// terminal-driven turn). So: settle until the mode has been `Input` for
    /// [`FLUSH_DELAY_MS`] (no wait when the prompt has been idle longer than
    /// that), re-check the session is still ready (if not, the next `Input`
    /// transition reschedules), flush, then verify the submit actually flipped
    /// the session to `Responding` — if it didn't, a bare CR submits whatever
    /// is sitting in the composer (a no-op on an empty prompt), retried up to
    /// [`SUBMIT_VERIFY_ATTEMPTS`] times.
    ///
    /// Each call bumps the session's flush epoch and the spawned task
    /// re-checks it at every step, so overlapping schedules (rapid sends,
    /// back-to-back `Input` transitions) collapse to a single live task and
    /// corrective CRs never stack.
    ///
    /// Outside a tokio runtime (unit tests drive the state machine
    /// synchronously) this degrades to the immediate flush.
    fn schedule_pending_flush(&self, session_id: &str) {
        if self
            .pending_messages
            .get(session_id)
            .is_none_or(|q| q.is_empty())
        {
            return;
        }
        let epoch = self.bump_flush_epoch(session_id);
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            self.flush_pending_messages(session_id);
            return;
        };
        let store = self.clone();
        let sid = session_id.to_string();
        rt.spawn(async move {
            let settled = store
                .input_since
                .get(&sid)
                .map(|i| i.elapsed())
                .unwrap_or_default();
            let remaining =
                std::time::Duration::from_millis(FLUSH_DELAY_MS).saturating_sub(settled);
            if !remaining.is_zero() {
                tokio::time::sleep(remaining).await;
            }
            if !store.flush_epoch_is(&sid, epoch) {
                return; // superseded by a newer schedule (or the queue was cleared)
            }
            if store.states.get(&sid).map(|s| s.mode) != Some(SessionMode::Input) {
                return; // no longer ready — the queue survives for the next transition
            }
            if store.paste_mode_off(&sid) {
                // The TUI has bracketed paste explicitly disabled (cold-start
                // trust/OAuth screens) — a paste now would land as literal
                // marker text. Hold; `record_output` reschedules on the enable
                // sequence.
                return;
            }
            let sent = store.flush_pending_messages(&sid);
            if sent.is_empty() {
                return;
            }
            let flushed_at = tokio::time::Instant::now();
            // Slash commands (e.g. `/model opus`) can complete without a
            // UserPromptSubmit hook, so "still Input" is not evidence the Enter
            // was swallowed — and a corrective CR could activate whatever picker
            // the command opened. Only verify sends that must start a turn.
            if sent.iter().all(|t| t.trim_start().starts_with('/')) {
                return;
            }
            for _ in 0..SUBMIT_VERIFY_ATTEMPTS {
                tokio::time::sleep(std::time::Duration::from_millis(SUBMIT_VERIFY_DELAY_MS))
                    .await;
                if !store.flush_epoch_is(&sid, epoch) {
                    return;
                }
                if store.states.get(&sid).map(|s| s.mode) != Some(SessionMode::Input) {
                    return; // the submit took (UserPromptSubmit flipped the mode)
                }
                if store.client_typed_since(&sid, flushed_at) {
                    return; // someone typed raw bytes since the flush — the
                            // composer is no longer known-ours, a CR could
                            // submit their draft
                }
                // No UserPromptSubmit arrived — the Enter was swallowed and the
                // text is sitting in the composer. Submit it.
                let Some(handle) = store.wrapper(&sid) else { return };
                let _ = handle
                    .tx
                    .send(WrapperMessage::Input { bytes: B64.encode(b"\r") });
            }
            // Ladder exhausted with the session still at `Input` and no
            // client typing — the text is most likely stranded in the
            // composer. Loud log rather than a re-paste: re-sending the text
            // risks doubling it if a submit actually took but its hook was
            // lost, which is worse than a visible strand.
            tokio::time::sleep(std::time::Duration::from_millis(SUBMIT_VERIFY_DELAY_MS)).await;
            if store.flush_epoch_is(&sid, epoch)
                && store.states.get(&sid).map(|s| s.mode) == Some(SessionMode::Input)
                && !store.client_typed_since(&sid, flushed_at)
            {
                tracing::warn!(
                    session_id = %sid,
                    "chat send not confirmed after verify retries; text may be stranded in the composer"
                );
            }
        });
    }

    /// Drain and send queued messages in order, returning the texts that were
    /// actually written to the child (so the caller can decide whether the
    /// batch needs submit verification). Called via
    /// [`Self::schedule_pending_flush`] on the `Input` transition. No-op when
    /// the queue is empty.
    fn flush_pending_messages(&self, session_id: &str) -> Vec<String> {
        let queued: Vec<String> = self
            .pending_messages
            .get_mut(session_id)
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default();
        let mut sent = Vec::with_capacity(queued.len());
        for text in queued {
            if self.send_message_now(session_id, text.clone()) == MessageOutcome::Sent {
                sent.push(text);
            }
        }
        sent
    }

    fn clear_pending_messages(&self, session_id: &str) {
        self.pending_messages.remove(session_id);
        // Abort any in-flight flush/verify task — its corrective CR must not
        // fire on whatever state the session is in now.
        self.bump_flush_epoch(session_id);
    }

    /// Encode a chat line — appending the `\r` that Claude Code's input field
    /// treats as submit — and write it to the child as a single atomic input
    /// frame, so the submit can't race a mid-flight redraw and get dropped.
    fn send_message_now(&self, session_id: &str, text: String) -> MessageOutcome {
        let Some(handle) = self.wrapper(session_id) else {
            return MessageOutcome::NoWrapper;
        };
        // Inject the prompt as a *bracketed paste* followed by a separate Enter.
        // Writing raw `text\r` as one burst makes the TUI fold the trailing CR
        // into the "paste" (a newline in the composer) instead of submitting — you
        // get the text plus a stray unsubmitted newline. Bracketed paste
        // (ESC[200~ … ESC[201~) delivers the whole text as one paste event; the CR
        // *after* the end marker is a real Enter that submits. Typed input already
        // works because each keystroke arrives as its own event. Any trailing
        // CR/LF in `text` is stripped so it doesn't add an extra blank line inside
        // the paste.
        let body = text.trim_end_matches(['\r', '\n']);
        let mut bytes = Vec::with_capacity(body.len() + 8);
        bytes.extend_from_slice(b"\x1b[200~");
        bytes.extend_from_slice(body.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~\r");
        if handle
            .tx
            .send(WrapperMessage::Input {
                bytes: B64.encode(&bytes),
            })
            .is_err()
        {
            return MessageOutcome::WrapperGone;
        }
        MessageOutcome::Sent
    }

    pub async fn record_output(&self, session_id: &str, chunk: &[u8]) {
        // Track the child's bracketed-paste state. When the TUI (re-)enables
        // paste mode — the strongest available signal that its composer is
        // mounted and accepting input — release any messages held behind the
        // paste-mode gate in `schedule_pending_flush`.
        let toggled_on = {
            let mut tracker = self.paste_modes.entry(session_id.to_string()).or_default();
            tracker.scan(chunk) == Some(true)
        };
        if toggled_on && self.states.get(session_id).map(|s| s.mode) == Some(SessionMode::Input) {
            self.schedule_pending_flush(session_id);
        }
        // Hold the buffer lock across both the ring-buffer push and the
        // broadcast send so a concurrent snapshot_and_subscribe can't see a
        // chunk in the snapshot *and* receive it again via the broadcast.
        let Some(buf) = self.buffers.get(session_id).map(|e| e.clone()) else {
            return;
        };
        let tx = self.bytes_tx.get(session_id).map(|e| e.clone());
        let mut guard = buf.lock().await;
        guard.push(chunk);
        if let Some(tx) = tx {
            let _ = tx.send(chunk.to_vec());
        }
    }

    pub async fn output_snapshot(&self, session_id: &str) -> Option<Vec<u8>> {
        let buf = self.buffers.get(session_id).map(|e| e.clone())?;
        let snapshot = buf.lock().await.snapshot();
        Some(snapshot)
    }

    pub fn subscribe_bytes(&self, session_id: &str) -> Option<broadcast::Receiver<Vec<u8>>> {
        self.bytes_tx.get(session_id).map(|e| e.subscribe())
    }

    /// Atomically take a snapshot of the ring buffer and subscribe to live
    /// bytes. The buffer mutex is held across both operations, and
    /// `record_output` holds the same mutex across its push+broadcast, so the
    /// returned snapshot and receiver are gap-free and duplicate-free: any
    /// chunk written before this call is in the snapshot only; any chunk
    /// written after is delivered via the receiver only.
    pub async fn snapshot_and_subscribe(
        &self,
        session_id: &str,
    ) -> Option<(Vec<u8>, broadcast::Receiver<Vec<u8>>)> {
        let buf = self.buffers.get(session_id).map(|e| e.clone())?;
        let tx = self.bytes_tx.get(session_id).map(|e| e.clone())?;
        let guard = buf.lock().await;
        let snapshot = guard.snapshot();
        let rx = tx.subscribe();
        drop(guard);
        Some((snapshot, rx))
    }
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(event: &str, session_id: &str, cwd: &str) -> HookEvent {
        HookEvent {
            event: event.into(),
            session_id: session_id.into(),
            cwd: Some(cwd.into()),
            timestamp: None,
            payload: serde_json::Map::new(),
        }
    }

    fn handle() -> WrapperHandle {
        let (tx, _rx) = mpsc::unbounded_channel();
        WrapperHandle { tx }
    }

    /// A wrapper handle whose receiver is returned so tests can inspect the
    /// input frames the store writes to the child.
    fn handle_with_rx() -> (WrapperHandle, mpsc::UnboundedReceiver<WrapperMessage>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (WrapperHandle { tx }, rx)
    }

    /// Decode the next `Input` frame's bytes, asserting one is present.
    fn next_input(rx: &mut mpsc::UnboundedReceiver<WrapperMessage>) -> Vec<u8> {
        match rx.try_recv().expect("expected an Input frame") {
            WrapperMessage::Input { bytes } => B64.decode(bytes).expect("valid base64"),
            other => panic!("expected Input frame, got {other:?}"),
        }
    }

    /// A chat line as it's injected into the PTY: a bracketed paste of the text
    /// followed by a submitting CR (see `send_message_now`).
    fn pasted(text: &str) -> Vec<u8> {
        let mut b = b"\x1b[200~".to_vec();
        b.extend_from_slice(text.as_bytes());
        b.extend_from_slice(b"\x1b[201~\r");
        b
    }

    #[test]
    fn deregister_wrapper_sweeps_per_session_aux_maps() {
        let store = SessionStore::new();
        store.register_wrapper("s1", "/w", handle());
        // Populate the per-session auxiliary maps the way live traffic would.
        store.flush_epochs.insert("s1".into(), 3);
        store
            .input_since
            .insert("s1".into(), tokio::time::Instant::now());
        store
            .client_input_at
            .insert("s1".into(), tokio::time::Instant::now());
        store.term_sizes.insert("s1".into(), (80, 24));
        store
            .paste_modes
            .insert("s1".into(), PasteModeTracker::default());

        store.deregister_wrapper("s1");

        // Transport maps AND the auxiliary maps must all be swept so they don't
        // accrue a permanent entry per session across spawn/stop churn.
        assert!(!store.wrappers.contains_key("s1"), "wrappers leaked");
        assert!(
            !store.flush_epochs.contains_key("s1"),
            "flush_epochs leaked"
        );
        assert!(!store.input_since.contains_key("s1"), "input_since leaked");
        assert!(
            !store.client_input_at.contains_key("s1"),
            "client_input_at leaked"
        );
        assert!(!store.term_sizes.contains_key("s1"), "term_sizes leaked");
        assert!(!store.paste_modes.contains_key("s1"), "paste_modes leaked");
    }

    #[test]
    fn message_sent_immediately_when_input() {
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h); // synthetic SessionStart → Input
        assert_eq!(
            store.submit_message("s1", "hello".into()),
            MessageOutcome::Sent
        );
        assert_eq!(
            next_input(&mut rx),
            pasted("hello"),
            "line submitted as bracketed paste + CR"
        );
    }

    #[test]
    fn message_queued_in_unknown_flushes_on_session_start() {
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_spawn("s1", "/w", h); // mode stays Unknown until SessionStart
        assert_eq!(
            store.submit_message("s1", "hi".into()),
            MessageOutcome::Queued
        );
        assert!(rx.try_recv().is_err(), "nothing written while queued");

        // Claude's real SessionStart lands → Input → the queued line flushes.
        store.ingest(hook("SessionStart", "s1", "/w"));
        assert_eq!(next_input(&mut rx), pasted("hi"));
    }

    // Outside a tokio runtime the flush degrades to synchronous (the daemon
    // always runs inside one — there it goes through the delayed schedule,
    // covered by the paused-clock tests below).
    #[test]
    fn message_queued_in_responding_flushes_on_stop() {
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h); // Input
        store.ingest(hook("UserPromptSubmit", "s1", "/w")); // → Responding
        assert_eq!(
            store.submit_message("s1", "followup".into()),
            MessageOutcome::Queued
        );
        assert!(rx.try_recv().is_err(), "held while Claude is responding");

        store.ingest(hook("Stop", "s1", "/w")); // turn ends → Input → flush
        assert_eq!(next_input(&mut rx), pasted("followup"));
    }

    // The real (in-runtime) path: an Input transition *schedules* the flush —
    // the hook that announced it fires while the TUI is still closing the
    // turn, so an immediate injection strands the text in the composer. After
    // the settle delay the message flushes; if no UserPromptSubmit follows,
    // the Enter was swallowed and a bare CR re-submits the composer content.
    #[tokio::test(start_paused = true)]
    async fn scheduled_flush_delays_then_verifies_submit() {
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h); // Input
        store.ingest(hook("UserPromptSubmit", "s1", "/w")); // → Responding
        assert_eq!(
            store.submit_message("s1", "followup".into()),
            MessageOutcome::Queued
        );

        store.ingest(hook("Stop", "s1", "/w")); // → Input; flush scheduled, not immediate
        assert!(
            rx.try_recv().is_err(),
            "no injection while the TUI is still closing the turn"
        );

        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS + 50)).await;
        assert_eq!(
            next_input(&mut rx),
            pasted("followup"),
            "flushed once the prompt settles"
        );

        // No UserPromptSubmit arrives → verify pass submits the stranded text.
        tokio::time::sleep(std::time::Duration::from_millis(
            SUBMIT_VERIFY_DELAY_MS + 50,
        ))
        .await;
        assert_eq!(
            next_input(&mut rx),
            b"\r".to_vec(),
            "bare CR submits the composer"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn scheduled_flush_skips_verify_when_submit_took() {
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h);
        store.ingest(hook("UserPromptSubmit", "s1", "/w"));
        assert_eq!(
            store.submit_message("s1", "followup".into()),
            MessageOutcome::Queued
        );
        store.ingest(hook("Stop", "s1", "/w"));

        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS + 50)).await;
        assert_eq!(next_input(&mut rx), pasted("followup"));
        store.ingest(hook("UserPromptSubmit", "s1", "/w")); // the submit took

        tokio::time::sleep(std::time::Duration::from_millis(
            SUBMIT_VERIFY_DELAY_MS + 50,
        ))
        .await;
        assert!(
            rx.try_recv().is_err(),
            "no stray CR when the message submitted"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn scheduled_flush_holds_message_when_a_turn_starts_in_the_window() {
        // The user submits from the TUI during the settle window — the queued
        // message must not be injected into the now-running turn; it stays
        // queued and flushes after the *next* Input transition.
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h);
        store.ingest(hook("UserPromptSubmit", "s1", "/w"));
        assert_eq!(
            store.submit_message("s1", "queued".into()),
            MessageOutcome::Queued
        );
        store.ingest(hook("Stop", "s1", "/w")); // schedules the flush
        store.ingest(hook("UserPromptSubmit", "s1", "/w")); // TUI turn starts inside the window

        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS + 50)).await;
        assert!(rx.try_recv().is_err(), "flush aborted — a turn is running");

        store.ingest(hook("Stop", "s1", "/w")); // next turn end reschedules
        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS + 50)).await;
        assert_eq!(next_input(&mut rx), pasted("queued"));
    }

    // A *direct* send (mode already `Input`) rides the same guarded pipeline:
    // right after an Input transition it settles first — the hook that
    // announced the mode fires while the TUI is still closing the turn, and an
    // instant injection gets its Enter swallowed exactly like a queued one.
    #[tokio::test(start_paused = true)]
    async fn direct_send_settles_after_a_fresh_input_transition() {
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h); // Input transition stamped now
        assert_eq!(
            store.submit_message("s1", "hi".into()),
            MessageOutcome::Sent
        );
        tokio::task::yield_now().await;
        assert!(rx.try_recv().is_err(), "held while the composer settles");

        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS + 50)).await;
        assert_eq!(next_input(&mut rx), pasted("hi"));

        store.ingest(hook("UserPromptSubmit", "s1", "/w")); // the submit took
        tokio::time::sleep(std::time::Duration::from_millis(
            2 * SUBMIT_VERIFY_DELAY_MS + 100,
        ))
        .await;
        assert!(
            rx.try_recv().is_err(),
            "no corrective CR after a clean submit"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn direct_send_into_an_idle_prompt_flushes_without_settle_wait() {
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h);
        // The prompt has been idle far longer than the settle window.
        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS * 10)).await;
        assert_eq!(
            store.submit_message("s1", "hi".into()),
            MessageOutcome::Sent
        );
        tokio::task::yield_now().await;
        assert_eq!(
            next_input(&mut rx),
            pasted("hi"),
            "no settle wait on an idle prompt"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn swallowed_enter_gets_bounded_corrective_crs() {
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h);
        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS * 10)).await;
        assert_eq!(
            store.submit_message("s1", "hi".into()),
            MessageOutcome::Sent
        );
        tokio::task::yield_now().await;
        assert_eq!(next_input(&mut rx), pasted("hi"));

        // No UserPromptSubmit ever arrives → each verify pass submits, capped.
        tokio::time::sleep(std::time::Duration::from_millis(
            SUBMIT_VERIFY_DELAY_MS + 50,
        ))
        .await;
        assert_eq!(next_input(&mut rx), b"\r".to_vec(), "first corrective CR");
        tokio::time::sleep(std::time::Duration::from_millis(
            SUBMIT_VERIFY_DELAY_MS + 50,
        ))
        .await;
        assert_eq!(next_input(&mut rx), b"\r".to_vec(), "second corrective CR");
        tokio::time::sleep(std::time::Duration::from_millis(SUBMIT_VERIFY_DELAY_MS * 4)).await;
        assert!(rx.try_recv().is_err(), "the ladder is bounded — no CR spam");
    }

    #[tokio::test(start_paused = true)]
    async fn flush_holds_while_bracketed_paste_is_disabled() {
        // Cold-start screens (trust prompt, OAuth) run with DECSET 2004 off —
        // a paste there lands as literal marker text. The flush holds until
        // the TUI enables paste mode, which `record_output` observes.
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h);
        store.record_output("s1", b"\x1b[?2004l").await; // paste explicitly off
        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS * 10)).await;

        assert_eq!(
            store.submit_message("s1", "hi".into()),
            MessageOutcome::Sent
        );
        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS + 50)).await;
        assert!(rx.try_recv().is_err(), "held while paste mode is off");

        // The composer mounts and enables paste mode → release.
        store.record_output("s1", b"\x1b[?2004h").await;
        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS + 50)).await;
        assert_eq!(next_input(&mut rx), pasted("hi"));
    }

    #[tokio::test(start_paused = true)]
    async fn paste_toggle_split_across_chunks_is_detected() {
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h);
        // Disable arrives split across two output chunks.
        store.record_output("s1", b"\x1b[?20").await;
        store.record_output("s1", b"04l").await;
        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS * 10)).await;

        assert_eq!(
            store.submit_message("s1", "hi".into()),
            MessageOutcome::Sent
        );
        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS + 50)).await;
        assert!(rx.try_recv().is_err(), "split 2004l still gates the flush");

        store.record_output("s1", b"\x1b[?2004").await;
        store.record_output("s1", b"h").await;
        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS + 50)).await;
        assert_eq!(next_input(&mut rx), pasted("hi"), "split 2004h releases it");
    }

    #[tokio::test(start_paused = true)]
    async fn verify_stands_down_when_the_user_types_in_the_window() {
        // Raw terminal keystrokes land in the composer during the verify
        // window — a corrective CR would submit the user's draft, so the
        // ladder aborts instead.
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h);
        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS * 10)).await;
        assert_eq!(
            store.submit_message("s1", "hi".into()),
            MessageOutcome::Sent
        );
        tokio::task::yield_now().await;
        assert_eq!(next_input(&mut rx), pasted("hi"));

        store.note_client_input("s1"); // user starts typing in the terminal
        tokio::time::sleep(std::time::Duration::from_millis(SUBMIT_VERIFY_DELAY_MS * 4)).await;
        assert!(
            rx.try_recv().is_err(),
            "no corrective CR while the user is typing"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn slash_command_send_skips_submit_verification() {
        // Built-in commands (e.g. `/model`) may not fire UserPromptSubmit, so
        // "still Input" is not evidence of a swallowed Enter — and a corrective
        // CR could activate whatever picker the command opened.
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h);
        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS * 10)).await;
        assert_eq!(
            store.submit_message("s1", "/model opus".into()),
            MessageOutcome::Sent
        );
        tokio::task::yield_now().await;
        assert_eq!(next_input(&mut rx), pasted("/model opus"));

        tokio::time::sleep(std::time::Duration::from_millis(SUBMIT_VERIFY_DELAY_MS * 4)).await;
        assert!(
            rx.try_recv().is_err(),
            "no corrective CR for a slash command"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn rapid_sends_coalesce_to_one_verify_task() {
        // A second send supersedes the first send's flush/verify task (epoch
        // bump), so corrective CRs never stack up from overlapping tasks.
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h);
        tokio::time::sleep(std::time::Duration::from_millis(FLUSH_DELAY_MS * 10)).await;
        assert_eq!(
            store.submit_message("s1", "one".into()),
            MessageOutcome::Sent
        );
        tokio::task::yield_now().await;
        assert_eq!(next_input(&mut rx), pasted("one"));
        assert_eq!(
            store.submit_message("s1", "two".into()),
            MessageOutcome::Sent
        );
        tokio::task::yield_now().await;
        assert_eq!(next_input(&mut rx), pasted("two"));

        // Both tasks' timers elapse, but only the latest epoch may correct.
        tokio::time::sleep(std::time::Duration::from_millis(
            SUBMIT_VERIFY_DELAY_MS + 50,
        ))
        .await;
        assert_eq!(next_input(&mut rx), b"\r".to_vec());
        assert!(rx.try_recv().is_err(), "exactly one CR per verify window");
    }

    #[test]
    fn message_queued_while_awaiting_approval_flushes_after_the_dialog() {
        // Typing into an open permission dialog would answer the dialog, so a
        // send during `Approval` queues and delivers once the prompt is back.
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h); // Input
        store.ingest(hook("PermissionRequest", "s1", "/w")); // → Approval
        assert_eq!(
            store.submit_message("s1", "also do X".into()),
            MessageOutcome::Queued,
        );
        assert!(
            rx.try_recv().is_err(),
            "chat is not written while awaiting approval"
        );

        store.ingest(hook("PostToolUse", "s1", "/w")); // approved → Responding
        assert!(rx.try_recv().is_err(), "still held mid-turn");
        store.ingest(hook("Stop", "s1", "/w")); // turn ends → Input → flush
        assert_eq!(next_input(&mut rx), pasted("also do X"));
    }

    #[test]
    fn message_rejected_when_stopped() {
        let store = SessionStore::new();
        let (h, _rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h);
        store.ingest(hook("SessionEnd", "s1", "/w")); // → Stopped
        assert_eq!(
            store.submit_message("s1", "hi".into()),
            MessageOutcome::Rejected(SessionMode::Stopped),
        );
    }

    #[test]
    fn queued_message_dropped_when_session_ends() {
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_spawn("s1", "/w", h); // Unknown
        assert_eq!(
            store.submit_message("s1", "later".into()),
            MessageOutcome::Queued
        );

        store.ingest(hook("SessionEnd", "s1", "/w")); // → Stopped, queue cleared
        assert!(
            rx.try_recv().is_err(),
            "a stopped session drops its queued message"
        );
    }

    #[test]
    fn message_for_unknown_session_reports_no_session() {
        let store = SessionStore::new();
        assert_eq!(
            store.submit_message("ghost", "hi".into()),
            MessageOutcome::NoSession
        );
    }

    #[test]
    fn hydrate_restores_sessions_as_stopped_without_clobbering_live() {
        let store = SessionStore::new();
        // A live session already present before hydration (e.g. an early hook).
        store.ingest(hook("SessionStart", "live", "/work/live"));

        store.hydrate(vec![
            crate::store::RestoredSession {
                id: "restored".into(),
                cwd: Some("/work/restored".into()),
                tool_calls: 7,
                created_at: 1000,
                last_event_at: 2000,
            },
            // Same id as the live one — must NOT overwrite it back to stopped.
            crate::store::RestoredSession {
                id: "live".into(),
                cwd: Some("/work/live".into()),
                tool_calls: 0,
                created_at: 1,
                last_event_at: 2,
            },
        ]);

        let restored = store.get("restored").expect("restored session present");
        assert_eq!(restored.mode, SessionMode::Stopped);
        assert_eq!(restored.cwd.as_deref(), Some("/work/restored"));
        assert_eq!(restored.tool_calls, 7);

        let live = store.get("live").expect("live session present");
        assert_ne!(
            live.mode,
            SessionMode::Stopped,
            "live entry must win over hydrate"
        );
    }

    // A pinned spawn (claude launched with `--session-id` == our id) must keep
    // its own hooks even when a sibling spawn shares the cwd. Without the
    // `states.contains_key` guard, SessionStart would consume the cwd's pending
    // entry and re-alias to the sibling, stealing the session — the root cause
    // of "wrong transcript" with several agents in one repo.
    #[test]
    fn pinned_session_id_not_stolen_by_cwd_sibling() {
        let store = SessionStore::new();
        let cwd = "/work/repo";
        // The later spawn overwrites pending_spawns_by_cwd[cwd].
        store.register_spawn("AAA", cwd, handle());
        store.register_spawn("BBB", cwd, handle());

        let state = store.ingest(hook("SessionStart", "AAA", cwd));
        assert_eq!(
            state.session_id, "AAA",
            "pinned hook must apply to its own state"
        );
        assert!(store.get("AAA").is_some());
        assert!(
            !store.aliases.contains_key("AAA"),
            "pinned id must not be aliased away"
        );
    }

    // Legacy path: a spawn with no pinned id (claude picks its own session id)
    // still correlates by cwd on the first SessionStart.
    #[test]
    fn legacy_unpinned_session_aliases_by_cwd() {
        let store = SessionStore::new();
        let cwd = "/work/solo";
        store.register_spawn("canonical-uuid", cwd, handle());

        let state = store.ingest(hook("SessionStart", "claude-own-id", cwd));
        assert_eq!(state.session_id, "canonical-uuid");
        assert_eq!(
            store.aliases.get("claude-own-id").map(|e| e.clone()),
            Some("canonical-uuid".to_string()),
        );
    }

    // A statusLine payload arrives with Claude's own session id; it must resolve
    // through the alias map to the canonical (spawn) id and land on that state.
    #[test]
    fn status_line_resolves_alias_and_lands_on_canonical_session() {
        let store = SessionStore::new();
        let cwd = "/work/repo";
        store.register_spawn("canonical-uuid", cwd, handle());
        // SessionStart binds claude's id → canonical via cwd.
        store.ingest(hook("SessionStart", "claude-own-id", cwd));

        let raw = serde_json::json!({
            "session_id": "claude-own-id",
            "workspace": { "current_dir": cwd },
            "model": { "display_name": "Opus 4.8 (1M context)" },
            "context_window": { "used_percentage": 22, "total_input_tokens": 220_000, "total_output_tokens": 700 },
            "cost": { "total_cost_usd": 3.34 },
            "rate_limits": {
                "five_hour": { "used_percentage": 1.0, "resets_at": 1_738_425_600i64 },
                "seven_day": { "used_percentage": 35.0 }
            }
        });
        let state = store
            .ingest_status_line(&raw)
            .expect("should match canonical session");
        assert_eq!(state.session_id, "canonical-uuid");
        let sl = state.status_line.expect("status_line set");
        assert_eq!(sl.model_display.as_deref(), Some("Opus 4.8 (1M context)"));
        assert_eq!(sl.context_used_pct, Some(22.0));
        assert_eq!(sl.cost_usd, Some(3.34));
        assert_eq!(sl.five_hour_pct, Some(1.0));
        assert_eq!(sl.five_hour_resets_at, Some(1_738_425_600));
        assert_eq!(sl.seven_day_pct, Some(35.0));
        assert_eq!(sl.seven_day_resets_at, None);
    }

    // Before any SessionStart, a statusLine for an unknown id is a silent no-op
    // (it fires repeatedly, so the next tick lands once the session registers).
    #[test]
    fn status_line_for_unknown_session_is_noop() {
        let store = SessionStore::new();
        let raw = serde_json::json!({ "session_id": "nobody", "context_window": { "used_percentage": 5 } });
        assert!(store.ingest_status_line(&raw).is_none());
        assert!(
            store.get("nobody").is_none(),
            "must not create a phantom session"
        );
    }

    // terminate_managed drops the managed prompt channel so the adapter's driver
    // loop (which selects on the receiver) sees the stream close and exits —
    // the only external kill path for an adapter-driven session.
    #[test]
    fn terminate_managed_closes_the_prompt_channel() {
        let store = SessionStore::new();
        store.register_managed("m1", "/tmp", "codex");
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        store.register_managed_input("m1", tx);
        assert!(store.is_managed("m1"));

        assert!(
            store.terminate_managed("m1"),
            "should report the session existed"
        );
        assert!(!store.is_managed("m1"), "prompt channel is gone");
        // The driver's receiver now observes the closed channel (loop-break signal).
        assert!(
            rx.try_recv().is_err(),
            "recv resolves to closed, not a value"
        );
        // Terminating an unknown / already-gone session is a no-op, not a panic.
        assert!(!store.terminate_managed("m1"));
    }

    // terminate_managed must clean up every managed registration, including the
    // yolo handle — leaking it kept the approval-policy flag alive past teardown.
    #[test]
    fn terminate_managed_cleans_up_the_yolo_handle() {
        let store = SessionStore::new();
        let _live = managed_with_yolo(&store, "m1", false);
        // The yolo flag is registered, so a live policy switch is accepted.
        assert_eq!(store.set_managed_permission_mode("m1", "yolo"), Ok("yolo"));

        assert!(store.terminate_managed("m1"));
        // Re-register just the prompt channel so `is_managed` is true again but
        // the yolo handle is NOT — isolating that terminate cleaned it up. With
        // the handle gone the switch reports the session frozen rather than
        // silently mutating a leaked handle.
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        store.register_managed_input("m1", tx);
        assert_eq!(
            store.set_managed_permission_mode("m1", "yolo"),
            Err(PermissionSwitchError::Managed),
            "yolo handle should be gone after terminate"
        );
    }

    // The serialized session carries the provider so clients don't guess: PTY
    // sessions default to claude; managed sessions report their adapter.
    #[test]
    fn session_state_carries_provider() {
        let store = SessionStore::new();
        let managed = store.register_managed("m1", "/tmp", "opencode");
        assert_eq!(managed.provider, "opencode");
        // A brand-new (PTY-style) state defaults to claude, and survives a
        // serde round-trip via the default when the field is absent.
        let claude = SessionState::new("c1".to_string(), Some("/tmp".to_string()));
        assert_eq!(claude.provider, "claude");
        let json = serde_json::to_value(&claude).unwrap();
        assert_eq!(json["provider"], "claude");
        let mut obj = json.as_object().unwrap().clone();
        obj.remove("provider");
        let restored: SessionState = serde_json::from_value(Value::Object(obj)).unwrap();
        assert_eq!(
            restored.provider, "claude",
            "absent field defaults to claude"
        );
    }

    // deregister_managed reclaims the hybrid Term view's byte resources (the bulk
    // of a managed session's memory) rather than leaking them past session end.
    #[test]
    fn deregister_managed_releases_terminal_resources() {
        let store = SessionStore::new();
        store.register_managed("m2", "/tmp", "codex");
        store.attach_pty("m2", handle());
        assert!(store.wrapper("m2").is_some());
        assert!(store.subscribe_bytes("m2").is_some());

        store.deregister_managed("m2");
        assert!(store.wrapper("m2").is_none(), "input wrapper released");
        assert!(
            store.subscribe_bytes("m2").is_none(),
            "byte broadcast released"
        );
        // The lightweight state row is kept (marked Stopped) for history.
        assert_eq!(store.get("m2").map(|s| s.mode), Some(SessionMode::Stopped));
    }

    // ── live permission-mode switch ──

    use crate::session::permission_mode::{PermissionMode, PermissionSwitchError};
    use std::sync::atomic::{AtomicU32, Ordering};

    const SHIFT_TAB: &[u8] = b"\x1b[Z";

    /// Bottom-row footer redraw for a mode marker: park the cursor on the last
    /// default row (24), erase it, write the marker — what the classifier sees.
    fn footer(marker: &str) -> Vec<u8> {
        format!("\x1b[24;1H\x1b[2K{marker}").into_bytes()
    }

    fn marker_for(mode: &str) -> &'static str {
        match mode {
            "acceptEdits" => "⏵⏵ accept edits on (shift+tab to cycle)",
            "plan" => "⏸ plan mode on (shift+tab to cycle)",
            "bypassPermissions" => "⏵⏵ bypass permissions on (shift+tab to cycle)",
            _ => "? for shortcuts",
        }
    }

    /// A fake Claude TUI: consumes wrapper input frames and, on each Shift+Tab,
    /// advances through `cycle` and redraws the footer accordingly. Returns a
    /// press counter the test can assert on.
    fn fake_tui(
        store: &SessionStore,
        sid: &'static str,
        mut rx: mpsc::UnboundedReceiver<WrapperMessage>,
        cycle: &'static [&'static str],
    ) -> Arc<AtomicU32> {
        let presses = Arc::new(AtomicU32::new(0));
        let presses_out = presses.clone();
        let store = store.clone();
        tokio::spawn(async move {
            let mut idx = 0usize;
            while let Some(msg) = rx.recv().await {
                let WrapperMessage::Input { bytes } = msg else {
                    continue;
                };
                let Ok(decoded) = B64.decode(bytes.as_bytes()) else {
                    continue;
                };
                if decoded == SHIFT_TAB {
                    presses.fetch_add(1, Ordering::SeqCst);
                    idx = (idx + 1) % cycle.len();
                    store
                        .record_output(sid, &footer(marker_for(cycle[idx])))
                        .await;
                }
            }
        });
        presses_out
    }

    #[tokio::test(start_paused = true)]
    async fn mode_switch_cycles_to_target_and_stops() {
        let store = SessionStore::new();
        let (h, rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h); // synthetic SessionStart → Input
        store.record_output("s1", &footer("? for shortcuts")).await;
        let presses = fake_tui(&store, "s1", rx, &["default", "acceptEdits", "plan"]);

        let got = store.set_permission_mode("s1", PermissionMode::Plan).await;
        assert_eq!(got, Ok(PermissionMode::Plan));
        assert_eq!(
            presses.load(Ordering::SeqCst),
            2,
            "default → acceptEdits → plan"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn mode_switch_is_a_noop_when_already_on_target() {
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h);
        store
            .record_output("s1", &footer(marker_for("acceptEdits")))
            .await;

        let got = store
            .set_permission_mode("s1", PermissionMode::AcceptEdits)
            .await;
        assert_eq!(got, Ok(PermissionMode::AcceptEdits));
        assert!(rx.try_recv().is_err(), "no keystrokes for a no-op switch");
    }

    #[tokio::test(start_paused = true)]
    async fn mode_switch_reports_unavailable_after_a_full_cycle() {
        let store = SessionStore::new();
        let (h, rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h);
        store.record_output("s1", &footer("? for shortcuts")).await;
        // Bypass isn't in this session's cycle.
        let presses = fake_tui(&store, "s1", rx, &["default", "acceptEdits", "plan"]);

        let got = store
            .set_permission_mode("s1", PermissionMode::BypassPermissions)
            .await;
        assert_eq!(
            got,
            Err(PermissionSwitchError::Unavailable(PermissionMode::Default)),
            "full loop ends back at the starting mode"
        );
        assert_eq!(
            presses.load(Ordering::SeqCst),
            3,
            "one full cycle, then stop"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn mode_switch_gives_up_when_the_tui_ignores_the_keystroke() {
        let store = SessionStore::new();
        let (h, _rx) = handle_with_rx(); // nobody redraws the footer
        store.register_wrapper("s1", "/w", h);
        store.record_output("s1", &footer("? for shortcuts")).await;

        let got = store.set_permission_mode("s1", PermissionMode::Plan).await;
        assert_eq!(
            got,
            Err(PermissionSwitchError::Unverified(PermissionMode::Default))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn mode_switch_rejected_while_a_dialog_is_open() {
        let store = SessionStore::new();
        let (h, mut rx) = handle_with_rx();
        store.register_wrapper("s1", "/w", h);
        store.ingest(hook("PermissionRequest", "s1", "/w")); // Approval pause

        let got = store
            .set_permission_mode("s1", PermissionMode::AcceptEdits)
            .await;
        assert_eq!(got, Err(PermissionSwitchError::Busy(SessionMode::Approval)));
        assert!(rx.try_recv().is_err(), "no keystrokes near an open dialog");
    }

    #[tokio::test(start_paused = true)]
    async fn mode_switch_rejected_for_managed_sessions() {
        let store = SessionStore::new();
        store.register_managed("m1", "/tmp", "codex");
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        store.register_managed_input("m1", tx);
        let got = store
            .set_permission_mode("m1", PermissionMode::AcceptEdits)
            .await;
        assert_eq!(got, Err(PermissionSwitchError::Managed));
    }

    // ── managed (codex) live approval-policy switch ──

    fn managed_with_yolo(
        store: &SessionStore,
        sid: &str,
        spawned_yolo: bool,
    ) -> Arc<std::sync::atomic::AtomicBool> {
        store.register_managed(sid, "/tmp", "codex");
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        store.register_managed_input(sid, tx);
        let live = Arc::new(std::sync::atomic::AtomicBool::new(spawned_yolo));
        store.register_managed_yolo(sid, live.clone(), spawned_yolo);
        live
    }

    #[test]
    fn managed_switch_ask_to_yolo_flips_the_adapter_flag() {
        let store = SessionStore::new();
        let live = managed_with_yolo(&store, "m1", false);
        assert_eq!(store.set_managed_permission_mode("m1", "yolo"), Ok("yolo"));
        assert!(live.load(Ordering::SeqCst), "adapter now auto-approves");
        // …and back: the provider wasn't spawned in bypass mode, so approvals
        // still flow and 'ask' is reachable live.
        assert_eq!(store.set_managed_permission_mode("m1", "ask"), Ok("ask"));
        assert!(!live.load(Ordering::SeqCst));
    }

    #[test]
    fn managed_switch_to_ask_unavailable_when_spawned_yolo() {
        let store = SessionStore::new();
        let live = managed_with_yolo(&store, "m1", true);
        assert_eq!(
            store.set_managed_permission_mode("m1", "ask"),
            Err(PermissionSwitchError::ManagedUnavailable { current: "yolo" }),
            "a bypass-spawned provider never asks — flipping the flag would be a silent no-op"
        );
        assert!(live.load(Ordering::SeqCst), "flag untouched on refusal");
    }

    #[test]
    fn managed_switch_without_registered_flag_reports_frozen() {
        // opencode/pi and the codex rollout fallback never register a live
        // flag — the switch must refuse rather than pretend.
        let store = SessionStore::new();
        store.register_managed("m1", "/tmp", "codex");
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        store.register_managed_input("m1", tx);
        assert_eq!(
            store.set_managed_permission_mode("m1", "yolo"),
            Err(PermissionSwitchError::Managed)
        );
    }
}
