use std::collections::VecDeque;
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use dashmap::{DashMap, DashSet};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};

use time::OffsetDateTime;

use super::account_usage::{AccountUsage, UsageError, UsageFailure};
use super::conversation::{ConversationItem, ConversationStore};
use super::permission_mode::{classify_screen, PermissionMode, PermissionSwitchError};
use super::state::{
    HookEvent, Pending, PendingWrite, Plan, SessionMode, SessionState, StatusLine, SubagentInfo,
    SubagentStatus, SubagentUpdate, Transport, CLAUDE_FIVE_HOUR_WINDOW_MINUTES,
    CLAUDE_SEVEN_DAY_WINDOW_MINUTES,
};
use crate::protocol::WrapperMessage;

const BROADCAST_CAPACITY: usize = 256;
const HOOK_BROADCAST_CAPACITY: usize = 256;
const STATUS_BROADCAST_CAPACITY: usize = 256;
const OUTPUT_BUFFER_CAP: usize = 256 * 1024; // 256 KiB per session
const BYTE_BROADCAST_CAPACITY: usize = 1024;
/// Max deferred inputs held per session while it isn't yet accepting input.
/// Bounds memory if a session never reaches `Input` (e.g. stuck on startup);
/// the oldest ordinary chat is dropped (with a warning) past this, while an
/// accepted PTY model control is retained. A queue full of controls refuses
/// new input. In practice the queue holds 0–1, so the cap only bites on a
/// genuinely wedged session.
const MAX_PENDING_MESSAGES: usize = 32;

/// Private provenance for deferred input. This deliberately never reaches a
/// session snapshot or SQLite: only the structural PTY model route may mint a
/// protected entry, while ordinary chat remains ordinary regardless of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingMessageKind {
    Chat,
    PtyModelControl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingMessage {
    text: String,
    kind: PendingMessageKind,
}

impl PendingMessage {
    fn chat(text: String) -> Self {
        Self {
            text,
            kind: PendingMessageKind::Chat,
        }
    }

    fn pty_model_control(text: String) -> Self {
        Self {
            text,
            kind: PendingMessageKind::PtyModelControl,
        }
    }

    fn is_protected(&self) -> bool {
        self.kind == PendingMessageKind::PtyModelControl
    }
}

/// At most this many incompatible provider status frames are allowed to look
/// stale after an accepted model switch. Counting frames rather than time makes
/// the release deterministic in tests and in a suspended process. Three covers
/// repeated status ticks already in flight without letting an unmatchable
/// display name (for example a concrete Haiku label after an alias switch) hide
/// provider truth indefinitely.
const MODEL_CONFIRMATION_MAX_SUPPRESSIONS: u8 = 3;
/// Delay between an `Input` transition and the pending-message flush. The
/// transition is announced by a *hook* (Stop / SessionStart), and Claude Code
/// runs hooks while the turn is still closing — its composer isn't back at the
/// prompt yet. Injecting at that instant types the message into the box but
/// the submitting Enter is treated as mid-turn input and swallowed, leaving
/// the text stranded in the TUI (the "GUI send lands in the TUI box" bug).
const FLUSH_DELAY_MS: u64 = 300;

fn now_millis() -> i64 {
    let now = OffsetDateTime::now_utc();
    now.unix_timestamp() * 1000 + i64::from(now.millisecond())
}
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
/// Upper bound on a reported PTY grid. `screen_permission_mode` rebuilds the
/// session's screen with `vt100::Parser::new(rows, cols, 0)`, which eagerly
/// allocates `cols * rows` cells — so an unvalidated `65535x65535` from any of
/// the three reporting paths (`/resize`, wrapper Register, spawn) turns one
/// request into a multi-gigabyte allocation. No real terminal is anywhere near
/// this; clamping (rather than rejecting) keeps an oversized report harmless
/// instead of failing a resize the child itself accepted.
const MAX_TERM_SIZE: (u16, u16) = (1000, 500);

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

/// A classified account-usage poll failure, with when it happened. `kind` is
/// what a surface must branch on: `NeedsReauth` is actionable, `NoCredentials`
/// means this account was never a source, `Unreachable` is transient. None of
/// them is zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountUsageFailure {
    pub kind: UsageFailure,
    pub detail: String,
    pub at: OffsetDateTime,
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
    pending_messages: Arc<DashMap<String, Vec<PendingMessage>>>,
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
    /// Monotonic spawn generation per session id.
    ///
    /// A restart reuses the id on purpose — the desktop, the TUI and the brain
    /// all close a session and immediately respawn with `resume` pinned to the
    /// same id — and every close path is fire-and-forget: `terminate_managed`
    /// only drops the input sender, `POST /signal` only sends SIGTERM. Neither
    /// waits for the child to die, so the *old* life's teardown routinely runs
    /// after its successor has already registered under that id. Teardown is
    /// therefore gated on still owning the generation it was born with; a stale
    /// caller reaps only its own child and leaves the store alone.
    generations: Arc<DashMap<String, u64>>,
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
    /// Latest account-level rate-limit reading from the OAuth usage endpoint
    /// (see `session::account_usage`), keyed by Claude config root (`""` = the
    /// daemon's default). One entry per logged-in account: a profile spawn
    /// with its own `CLAUDE_CONFIG_DIR` is a different login with different
    /// windows, and a single global copy used to stamp the default account's
    /// gauges onto its sessions. Patched into each Claude session's status
    /// line at ingest time from ITS root's entry — the stream transport's
    /// `rate_limit_event` rarely carries `utilization`, so without this a
    /// stream session never shows a usage percentage.
    account_usage: Arc<std::sync::RwLock<std::collections::HashMap<String, AccountUsage>>>,
    /// Why a root's last poll FAILED, keyed the same way. Kept beside the
    /// readings rather than folded into them because the two answer different
    /// questions and a consumer needs both: a root can have a stale reading
    /// AND a current failure, and "we asked and were refused" is not the same
    /// answer as "we never asked". Without this, a failed poll was silent, and
    /// silence at the surface is indistinguishable from 0% — which reads as
    /// "plenty of headroom" and is the one thing the gauge must never lie
    /// about. Cleared on the next success.
    account_usage_errors:
        Arc<std::sync::RwLock<std::collections::HashMap<String, AccountUsageFailure>>>,
    /// Session ids for daemon-owned keep-warm pings (see `daemon::heartbeat`).
    /// A warm ping runs a real headless `claude`, whose Claude Code hooks would
    /// otherwise register a stray session and surface it in the sidebar / recent
    /// / fleet. The ping is pinned to `claude --session-id <uuid>` and that uuid
    /// is marked here, so `ingest` drops its hooks entirely: a warm is recorded
    /// in the `heartbeats` table, never as a session.
    heartbeat_ids: Arc<DashSet<String>>,
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
    /// Optional per-answer kind tags, index-aligned with `answers`: each entry
    /// is `"option"` (the string is a 1-indexed option number to map to its
    /// label) or `"text"` (the string is a literal free-text answer, never
    /// numerically remapped). Absent/empty → fall back to the numeric-guess
    /// heuristic (`answered_input`), which is what the TUI and older clients
    /// rely on since they only send bare `answers`.
    pub answer_kinds: Option<Vec<String>>,
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
    /// The bounded PTY queue is full. Control operations use this outcome
    /// instead of evicting an already-accepted chat message.
    QueueFull,
}

fn comparable_model_identity(value: &str) -> String {
    let normalized: String = value
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    normalized
        .strip_prefix("claude")
        .unwrap_or(&normalized)
        .to_string()
}

fn status_confirms_selection(
    status: &StatusLine,
    selection: &super::windows::ModelSelection,
) -> bool {
    let model_match = status.model_display.as_deref().map(|reported| {
        let reported = comparable_model_identity(reported);
        let selected = comparable_model_identity(&selection.model);
        !reported.is_empty()
            && !selected.is_empty()
            && (reported.contains(&selected) || selected.contains(&reported))
    });
    match selection.context_window {
        Some(window) => status.context_window_size == Some(window) && model_match.unwrap_or(true),
        None => model_match == Some(true),
    }
}

/// Keep asynchronous provider telemetry on the epoch it can prove. A frame
/// with no command id cannot overwrite an accepted owner selection merely by
/// arriving later; incompatible model/window fields are withheld while cost,
/// token totals and rate-limit readings continue to flow. A compatible frame
/// advances telemetry to the accepted epoch and releases the fence, after
/// which later provider-driven model changes remain truthful and visible.
fn reconcile_model_telemetry(session: &mut SessionState, status: &mut StatusLine) {
    let Some(pending) = session.pending_model_confirmation.as_ref() else {
        return;
    };
    if status_confirms_selection(status, pending) {
        session.model_telemetry_epoch = session.model_selection_epoch;
        session.pending_model_confirmation = None;
        session.model_confirmation_suppressions_remaining = 0;
        return;
    }

    // The frame is incompatible, so it may still have been emitted before the
    // accepted switch reached the provider. Suppress only the bounded stale
    // budget. This frame still counts when model/window fields are absent:
    // providers are not required to report a comparable display name or a
    // window, and waiting for evidence they cannot produce recreates the
    // permanent fence this bound exists to prevent.
    status.model_display = None;
    status.context_window_size = None;
    status.context_used_pct = None;
    session.model_confirmation_suppressions_remaining = session
        .model_confirmation_suppressions_remaining
        .saturating_sub(1);
    if session.model_confirmation_suppressions_remaining == 0 {
        session.model_telemetry_epoch = session.model_selection_epoch;
        session.pending_model_confirmation = None;
    }
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
            generations: Arc::new(DashMap::new()),
            term_sizes: Arc::new(DashMap::new()),
            managed_yolo: Arc::new(DashMap::new()),
            managed_model: Arc::new(DashMap::new()),
            managed_answers: Arc::new(DashMap::new()),
            managed_permission_modes: Arc::new(DashMap::new()),
            managed_interrupts: Arc::new(DashMap::new()),
            account_usage: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
            account_usage_errors: Arc::new(
                std::sync::RwLock::new(std::collections::HashMap::new()),
            ),
            heartbeat_ids: Arc::new(DashSet::new()),
        }
    }

    // --- keep-warm heartbeats -----------------------------------------------

    /// Mark a session id as a keep-warm ping so its Claude Code hooks are
    /// dropped by `ingest` (never registered or broadcast as a session). Called
    /// by `daemon::heartbeat` before it spawns the pinned `claude --session-id`.
    pub fn mark_heartbeat(&self, session_id: &str) {
        self.heartbeat_ids.insert(session_id.to_string());
    }

    /// Stop suppressing a session id's hooks. Called once the warm ping's child
    /// has exited (all its hooks have fired) so the set can't grow unbounded.
    pub fn unmark_heartbeat(&self, session_id: &str) {
        self.heartbeat_ids.remove(session_id);
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
    /// Claim the next spawn generation for a session id.
    ///
    /// Every spawn path calls this and hands the token to whatever will tear the
    /// session down. See [`SessionStore::generations`] for why teardown cannot
    /// key on the id alone.
    pub fn claim_generation(&self, session_id: &str) -> u64 {
        let mut entry = self.generations.entry(session_id.to_string()).or_insert(0);
        *entry += 1;
        *entry
    }

    /// Whether `generation` is still the live one for this session. False once a
    /// restart has claimed a newer one — the caller is a previous lifetime and
    /// must not touch shared state.
    ///
    /// An id nobody has claimed answers **true**: there is no successor to
    /// protect, and the alternative default would silently skip teardown and
    /// leak the session's plumbing. The guard exists to stop one lifetime
    /// clobbering another, not to gate teardown in general.
    pub fn owns_generation(&self, session_id: &str, generation: u64) -> bool {
        match self.generations.get(session_id) {
            Some(live) => *live == generation,
            None => true,
        }
    }

    /// The registered PTY handle for a session, if any. Test/diagnostic accessor.
    pub fn pty_handle(&self, session_id: &str) -> Option<Arc<crate::wrapper::pty::PtyHandle>> {
        self.ptys.get(session_id).map(|h| h.clone())
    }

    /// Reap the caller's own PTY child, and clear the registry slot only if it
    /// still holds that child.
    ///
    /// Ownership is by handle identity rather than generation because the reader
    /// task already holds its `Arc` — and because it must reap its child either
    /// way. A previous life's reader reaching EOF after a restart still needs to
    /// collect its zombie; what it must not do is remove and SIGKILL the
    /// successor's child, which is exactly what keying on the id alone did.
    pub fn reap_pty_owned(&self, session_id: &str, mine: &Arc<crate::wrapper::pty::PtyHandle>) {
        self.ptys
            .remove_if(session_id, |_, live| Arc::ptr_eq(live, mine));
        self.reap_handle(mine.clone());
    }

    fn reap_handle(&self, handle: Arc<crate::wrapper::pty::PtyHandle>) {
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
                st.user_prompts = s.user_prompt_count;
                // The window signals. `hydrate` used to drop BOTH of these on
                // the floor — `RestoredSession` carried neither, and the
                // `sessions.model` column it could have read was ignored — so a
                // restarted daemon rebuilt a 1M session with no idea it was
                // one. `requested_model` is the load-bearing half: Claude Code
                // strips the `[1m]` marker from the id it writes into the
                // transcript, so nothing else can ever tell a resumed 1M
                // session from a 200k one until its provider speaks again.
                //
                // `s.model` (the concrete id the transcript reported) is
                // deliberately NOT restored onto the state: the transcript is
                // re-read on resume and is its own authority for that. It is
                // carried on `RestoredSession` because the resolver's fallback
                // rank takes a model id, and a caller that has the row already
                // has it.
                st.requested_selection = s.requested_selection.clone();
                if let Some(selection) = s.requested_selection.clone() {
                    // A restarted daemon has durable owner truth but no proof
                    // that a newly arriving status frame belongs to it. Re-arm
                    // the same FINITE confirmation fence used by a live
                    // acceptance; hydration never extends its bound.
                    st.model_selection_epoch = 1;
                    st.pending_model_confirmation = Some(selection);
                    st.model_confirmation_suppressions_remaining =
                        MODEL_CONFIRMATION_MAX_SUPPRESSIONS;
                }
                // Preserve an exact stored companion: equality with the
                // canonical identity is the provider-neutral proof that an
                // opaque id such as `vendor/model-1m` must stay untouched. If
                // restore normalized a historical Claude spelling (for example
                // `sonnet-1m` -> identity `sonnet` + 1M), derive the current
                // executable `[1m]` projection so the next event heals SQLite.
                st.requested_model = s.requested_selection.as_ref().map_or_else(
                    || s.requested_model.clone(),
                    |selection| {
                        Some(
                            s.requested_model
                                .as_ref()
                                .filter(|legacy| legacy.trim() == selection.model)
                                .cloned()
                                .unwrap_or_else(|| {
                                    super::windows::PersistedModelSelection::from_selection(
                                        selection.clone(),
                                    )
                                    .legacy_model
                                }),
                        )
                    },
                );
                // The transcript path, which is what `usage::usage_for_session`
                // folds cost and tokens out of. `hydrate` could not restore it
                // before v6 — the column did not exist — so EVERY rehydrated
                // row folded from `None` and reported $0.00 / 0 tokens / no
                // model for the rest of the daemon's life. It also carries the
                // account attribution (`claude_config_root`), which likewise
                // went blank across a restart.
                st.transcript_path = s.transcript_path.clone();
                // The ACCOUNT this session billed against, as recorded at
                // spawn. Restored rather than re-derived: re-deriving it from
                // the transcript path is exactly the thing that merges two
                // logins the moment anything resolves that path (see
                // `SessionState::config_root`). `None` here is honest — a row
                // written before v8, or a session the daemon did not spawn.
                st.config_root = s.config_root.clone();
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

    /// Drop archived (Stopped + past [`ARCHIVE_AFTER_SECONDS`]) sessions from the
    /// in-memory `states` map so it doesn't grow without bound across long daemon
    /// uptime with many spawn/stop cycles. Returns the number evicted.
    ///
    /// Safe against resume: an archived session is already hidden from the
    /// default list (`SessionState::is_archived`), and the resume paths
    /// (`register_spawn` / `register_managed`, both `states.entry().or_insert`)
    /// re-create the state row from the session id regardless of whether it's
    /// resident — the conversation is re-derived from Claude's on-disk transcript
    /// (`--resume <id>`) or the codex rollout sidecar. Its SQLite row is
    /// untouched here, so a daemon restart also rehydrates it via
    /// `load_recent_sessions`. Only live and recently-stopped sessions are kept.
    pub fn evict_stale_stopped(&self) -> usize {
        self.evict_stale_stopped_at(OffsetDateTime::now_utc().unix_timestamp())
    }

    /// [`evict_stale_stopped`](Self::evict_stale_stopped) with an injected clock,
    /// so tests can force the archive threshold without waiting a real week.
    fn evict_stale_stopped_at(&self, now_unix: i64) -> usize {
        let mut removed = 0usize;
        self.states.retain(|_, st| {
            let archived = st.is_archived(now_unix);
            if archived {
                removed += 1;
            }
            !archived
        });
        removed
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
                let summary = raw
                    .get("tool_input")
                    .and_then(|ti| ti.get("command").or_else(|| ti.get("description")))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                // The PTY hook gateway is this session's primary feed — the
                // same feed `SessionState::apply` speaks for — so it parks
                // through the same funnel rather than writing the slot behind
                // it. (No `mcp_ask` shim is registered for a PTY session, so
                // this park has never had a rival; going through the funnel is
                // what keeps that true rather than merely observed.)
                state.park_pending(
                    SessionMode::Approval,
                    Pending::Approval {
                        tool: tool.clone(),
                        summary,
                        raw,
                    },
                );
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
        // Keep-warm pings run a real headless `claude`, so Claude Code fires the
        // user's hooks (SessionStart/Stop/…) against our `/hook` endpoint just
        // like any session. Drop them wholesale — no state row, no `hook_tx` /
        // `update_tx` broadcast — so a warm can never surface in the sidebar,
        // recent list, or fleet. The ping is recorded in the `heartbeats` table
        // instead (see `daemon::heartbeat`). The id is pinned via
        // `claude --session-id`, so it arrives verbatim with no alias to resolve.
        if self.heartbeat_ids.contains(&event.session_id) {
            return SessionState::new(event.session_id.clone(), event.cwd.clone());
        }

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

        let mut status = StatusLine::from_claude_json(raw);
        let state = {
            let mut entry = self.states.get_mut(&canonical)?;
            let session = entry.value_mut();
            if session.provider == "claude" {
                let root = session.claude_config_root();
                self.patch_rate_limits(&mut status, &root);
            }
            reconcile_model_telemetry(session, &mut status);
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

    /// Overlay the account-level rate-limit reading for one config root onto a
    /// status line. A fresh account reading wins field-wise over whatever the
    /// session's own wire delivered: the windows are account-scoped so it
    /// can't disagree except by being newer, and the stream wire's rare
    /// `utilization` (it only rides warning events) would otherwise go stale
    /// and stick. The root MUST be the session's own (`claude_config_root`) —
    /// a different root is a different login whose windows are unrelated.
    /// Absent / stale readings leave the line untouched.
    fn patch_rate_limits(&self, status: &mut StatusLine, root: &str) {
        let guard = self.account_usage.read().unwrap();
        let Some(u) = guard.get(root) else { return };
        if !u.is_fresh(OffsetDateTime::now_utc()) {
            return;
        }
        let mut injected = false;
        let mut fill = |dst: &mut Option<f64>, src: Option<f64>| {
            if src.is_some() {
                *dst = src;
                injected = true;
            }
        };
        fill(&mut status.five_hour_pct, u.five_hour_pct);
        fill(&mut status.seven_day_pct, u.seven_day_pct);
        fill(&mut status.monthly_pct, u.monthly_pct);
        // Resets: the wire reliably carries these even when it omits the
        // percent, so only fill gaps — never replace a present value.
        if status.five_hour_resets_at.is_none() {
            status.five_hour_resets_at = u.five_hour_resets_at;
        }
        if status.seven_day_resets_at.is_none() {
            status.seven_day_resets_at = u.seven_day_resets_at;
        }
        if status.monthly_resets_at.is_none() {
            status.monthly_resets_at = u.monthly_resets_at;
        }
        // Window lengths: this reading is Claude's, whose windows are named by
        // their length. Stamp each one that actually has a reading, so clients
        // can label "5 hours" / "7 days" instead of just "5h"/"7d". The monthly
        // overage window has no fixed length, so it stays unreported.
        if status.five_hour_window_minutes.is_none()
            && (status.five_hour_pct.is_some() || status.five_hour_resets_at.is_some())
        {
            status.five_hour_window_minutes = Some(CLAUDE_FIVE_HOUR_WINDOW_MINUTES);
        }
        if status.seven_day_window_minutes.is_none()
            && (status.seven_day_pct.is_some() || status.seven_day_resets_at.is_some())
        {
            status.seven_day_window_minutes = Some(CLAUDE_SEVEN_DAY_WINDOW_MINUTES);
        }
        if status.overage_out_of_credits.is_none() {
            status.overage_out_of_credits = u.out_of_credits;
        }
        if injected && status.received_at.is_none() {
            status.received_at = Some(OffsetDateTime::now_utc());
        }
    }

    /// Store a new account-level usage reading for one config root and push
    /// the patched status line to that root's live Claude sessions — and ONLY
    /// that root's: other roots are other logins whose gauges this reading
    /// says nothing about. Deliberately does NOT bump `updated_at` — a
    /// background poll is not session activity.
    pub fn set_account_usage(&self, root: &str, usage: AccountUsage) {
        self.account_usage
            .write()
            .unwrap()
            .insert(root.to_string(), usage);
        // A success retires the last failure: keeping it would leave a root
        // permanently flagged needs-reauth after the CLI rotated its token.
        self.account_usage_errors.write().unwrap().remove(root);
        let targets: Vec<(String, Option<String>, StatusLine)> = self
            .states
            .iter()
            .filter(|e| {
                e.provider == "claude"
                    && e.mode != SessionMode::Stopped
                    && e.claude_config_root() == root
            })
            .map(|e| {
                (
                    e.key().clone(),
                    e.cwd.clone(),
                    e.status_line.clone().unwrap_or_default(),
                )
            })
            .collect();
        for (session_id, cwd, mut status_line) in targets {
            self.patch_rate_limits(&mut status_line, root);
            if let Some(mut entry) = self.states.get_mut(&session_id) {
                entry.status_line = Some(status_line.clone());
            }
            let _ = self.status_tx.send(StatusLineUpdate {
                session_id,
                cwd,
                status_line,
            });
        }
    }

    /// Snapshot of the latest usage reading for one config root (`""` = the
    /// default account), if any. May be stale — callers gate on
    /// [`AccountUsage::is_fresh`] themselves.
    pub fn account_usage_for(&self, root: &str) -> Option<AccountUsage> {
        self.account_usage.read().unwrap().get(root).cloned()
    }

    /// Record a classified poll failure for one config root. Deliberately does
    /// NOT drop the previous reading: a stale-but-real number plus "and the
    /// last refresh failed because the token expired" is strictly more
    /// information than either alone, and dropping the reading would make an
    /// expired account look like a brand-new one.
    pub fn set_account_usage_error(&self, root: &str, err: UsageError) {
        self.account_usage_errors.write().unwrap().insert(
            root.to_string(),
            AccountUsageFailure {
                kind: err.kind,
                detail: err.detail,
                at: OffsetDateTime::now_utc(),
            },
        );
    }

    /// The last recorded failure for one config root, if the most recent poll
    /// did not succeed.
    pub fn account_usage_error_for(&self, root: &str) -> Option<AccountUsageFailure> {
        self.account_usage_errors.read().unwrap().get(root).cloned()
    }

    /// Every config root the store has an opinion about — a reading, a
    /// failure, or both. Union, sorted, deduped.
    pub fn known_account_roots(&self) -> Vec<String> {
        let mut roots: Vec<String> = self
            .account_usage
            .read()
            .unwrap()
            .keys()
            .cloned()
            .chain(self.account_usage_errors.read().unwrap().keys().cloned())
            .collect();
        roots.sort();
        roots.dedup();
        roots
    }

    /// Whether any non-stopped Claude session exists — the account-usage
    /// poller's gate, so the daemon never polls the endpoint while idle.
    pub fn has_live_claude_session(&self) -> bool {
        self.states
            .iter()
            .any(|e| e.provider == "claude" && e.mode != SessionMode::Stopped)
    }

    /// Distinct Claude config roots among live Claude sessions (`""` = the
    /// daemon's default) — one poll target per logged-in account. Empty while
    /// idle, which is what keeps the poller quiet.
    pub fn live_claude_config_roots(&self) -> Vec<String> {
        let mut roots: Vec<String> = self
            .states
            .iter()
            .filter(|e| e.provider == "claude" && e.mode != SessionMode::Stopped)
            .map(|e| e.claude_config_root())
            .collect();
        roots.sort();
        roots.dedup();
        roots
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

    /// Attach a wrapper's control channel to a session. First registration
    /// wins: while another wrapper (or an in-daemon PTY spawn, which shares this
    /// map) still holds the id, a second Register is refused with `None` rather
    /// than silently taking over. `/wrapper/:id` is unauthenticated, so without
    /// this any local process could name a live session's id and become the
    /// destination for every subsequent input/signal/resize — the same hijack
    /// the bus router's first-registration-wins guard closes. Ownership is
    /// released the moment the incumbent's channel drops (`deregister_wrapper`,
    /// or the sender closing when its pump task ends), so a genuine reconnect
    /// after the old socket dies still registers cleanly.
    pub fn register_wrapper(
        &self,
        session_id: &str,
        cwd: &str,
        handle: WrapperHandle,
    ) -> Option<SessionState> {
        if let Some(live) = self.wrappers.get(session_id) {
            if !live.tx.is_closed() {
                return None;
            }
        }
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
        Some(state)
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
            let mut entry = self
                .states
                .entry(session_id.to_string())
                .or_insert_with(|| {
                    SessionState::new(session_id.to_string(), Some(cwd.to_string()))
                });
            // A resume reuses the Stopped row from the id's previous life, but a
            // fresh child is now attached — leave Stopped immediately. Clients
            // treat a stopped attach target as dead and tear their viewers down,
            // so waiting for SessionStart to flip the mode loses the race.
            if entry.mode == SessionMode::Stopped {
                entry.mode = SessionMode::Unknown;
                entry.clear_pending();
                entry.updated_at = OffsetDateTime::now_utc();
            }
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

    /// Drive a managed session's mode (and its pending slot) directly, since
    /// managed backends don't emit Claude hooks. Broadcasts a SessionUpdate.
    /// Returns None if the session isn't registered.
    ///
    /// **This is the single guarded place every pending-slot write goes
    /// through.** `write` says what the caller means to do with the slot and
    /// on whose behalf (see [`PendingWrite`] and [`PendingOwner`]); the rules
    /// live in [`SessionState::write_pending`], not here and not at the call
    /// sites. A [`PendingWrite::Keep`] write — liveness or enrichment — is
    /// suppressed outright while an approval or a question is parked, mode
    /// included; a [`PendingWrite::Resolve(PendingOwner::Primary)`] releases only the request its own
    /// feed raised. That is the invariant from
    /// `docs/unresolvable-approval-findings.md` enforced structurally rather
    /// than by a hand-written condition at each of a dozen call sites, four of
    /// which forgot it and wedged real sessions.
    ///
    /// A suppressed write broadcasts nothing and does not bump `updated_at`:
    /// there is no change to announce.
    ///
    /// The returned state is what the store actually holds afterwards, NOT
    /// what was asked for. A driver mirroring the mode locally must take it
    /// from here (see [`crate::providers::set_mode`]) or its mirror drifts
    /// from the store the first time a write is suppressed — or the first time
    /// releasing its own card restores the *other* feed's, which leaves the
    /// session legitimately parked on a request the driver never raised.
    pub fn set_managed_mode(
        &self,
        session_id: &str,
        mode: SessionMode,
        write: PendingWrite,
    ) -> Option<SessionState> {
        let state = {
            let mut entry = self.states.get_mut(session_id)?;
            if !entry.write_pending(mode, write) {
                return Some(entry.clone());
            }
            if mode == SessionMode::Input {
                // Turn boundary. A subagent row still `Running` now is stale by
                // construction — see `SessionState::close_stale_subagents`. A
                // dead child that keeps `background_tasks` above zero holds the
                // parent "working" forever, so this is the one place the parent
                // and its children get reconciled.
                entry.close_stale_subagents();
            }
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

    /// Record the model string this session was asked to run (see
    /// [`SessionState::requested_model`]). Called at spawn — before any
    /// telemetry exists, which is the point: it is what makes a 1M window a
    /// known fact from token zero rather than something inferred once the
    /// session has already overflowed 200k. Also called on a live `/model`
    /// switch, so a session that moves between 200k and 1M mid-flight tracks
    /// it. Blank strings are ignored (an absent `--model` means "the CLI's own
    /// default", which says nothing about the window). No-op for unknown ids.
    pub fn set_requested_model_selection(
        &self,
        session_id: &str,
        persisted: &super::windows::PersistedModelSelection,
    ) {
        if let Some(mut entry) = self.states.get_mut(session_id) {
            entry.requested_model = Some(persisted.legacy_model.clone());
            entry.requested_selection = Some(persisted.selection.clone());
        }
    }

    /// Commit an accepted live model switch to observable session state.
    ///
    /// Provider status is authoritative once it reports the *new* selection,
    /// but the last status frame belongs to the old selection. Clear only the
    /// model/window fields at this ownership hand-off so stale telemetry cannot
    /// undo the accepted canonical pair while cost and rate-limit readings stay
    /// available. No-op for unknown ids.
    pub fn accept_requested_model_selection(
        &self,
        session_id: &str,
        persisted: &super::windows::PersistedModelSelection,
    ) {
        let state = self.states.get_mut(session_id).map(|mut entry| {
            entry.requested_model = Some(persisted.legacy_model.clone());
            entry.requested_selection = Some(persisted.selection.clone());
            entry.model_selection_epoch = entry.model_selection_epoch.saturating_add(1);
            entry.pending_model_confirmation = Some(persisted.selection.clone());
            entry.model_confirmation_suppressions_remaining = MODEL_CONFIRMATION_MAX_SUPPRESSIONS;
            if let Some(status) = entry.status_line.as_mut() {
                status.model_display = None;
                status.context_window_size = None;
                status.context_used_pct = None;
            }
            entry.updated_at = OffsetDateTime::now_utc();
            entry.clone()
        });
        if let Some(state) = state {
            let _ = self.update_tx.send(SessionUpdate {
                session_id: session_id.to_string(),
                event: "ModelSwitch".to_string(),
                state,
            });
        }
    }

    /// Record the Claude config root this session was SPAWNED with — the
    /// spawn env's `CLAUDE_CONFIG_DIR`, or the daemon's own default when the
    /// spawn set none. Stored normalized (`""` = the default account).
    ///
    /// This is the ATTRIBUTION KEY, and it is recorded here rather than
    /// derived later because deriving it is a correctness hazard. See
    /// [`SessionState::config_root`] — a profile's `projects` dir is a SYMLINK
    /// at the shared `~/.claude/projects`, so the transcript path only tells
    /// the two logins apart while nothing has resolved it.
    ///
    /// [`SessionState::config_root`]: super::state::SessionState::config_root
    pub fn set_config_root(&self, session_id: &str, root: &str) {
        let root = super::account_usage::normalize_root(root);
        if let Some(mut entry) = self.states.get_mut(session_id) {
            entry.config_root = Some(root);
        }
    }

    /// The config root recorded for a session, if the daemon spawned it.
    ///
    /// Three-valued on purpose, and the values must stay apart all the way to
    /// the wire: `Some("")` is the DEFAULT account (a known answer), `Some(p)`
    /// is a named profile, and `None` means the daemon genuinely does not know
    /// — a session it did not spawn. Collapsing `None` into `Some("")` would
    /// bill every unattributable session to the default account.
    ///
    /// Alias-aware: hook events arrive under Claude's own session id, which is
    /// the spawn id only because the daemon pins `--session-id`. A session the
    /// daemon adopted rather than pinned is reachable only through the alias.
    pub fn config_root(&self, session_id: &str) -> Option<String> {
        let canonical = self
            .aliases
            .get(session_id)
            .map(|e| e.clone())
            .unwrap_or_else(|| session_id.to_string());
        self.states
            .get(&canonical)
            .and_then(|s| s.config_root.clone())
    }

    /// The model a session was ASKED for, if anything recorded one.
    ///
    /// Read by the persistence task so the row SQLite creates for this session
    /// carries it — see `Db::record_event_with_spawn_facts` for why it
    /// cannot simply be UPDATEd at spawn time.
    pub fn requested_model_selection(
        &self,
        session_id: &str,
    ) -> Option<super::windows::PersistedModelSelection> {
        let canonical = self
            .aliases
            .get(session_id)
            .map(|entry| entry.clone())
            .unwrap_or_else(|| session_id.to_string());
        self.states.get(&canonical).and_then(|state| {
            // Always derive the compatibility projection from canonical memory.
            // On restore, a disagreement means the legacy value came from a
            // newer v8 writer and `requested_selection` already reflects it. A
            // raw legacy overwrite here would preserve the stale disagreement;
            // deriving it lets the next ordinary event heal all three columns.
            state.requested_selection.clone().map(|selection| {
                let mut persisted =
                    super::windows::PersistedModelSelection::from_selection(selection.clone());
                // Persistence predates provider storage. An exact legacy
                // companion is the only durable evidence that an opaque
                // non-Claude identity such as `vendor/model-1m` must stay
                // byte-for-byte. Invalid bare Claude 1M companions were
                // already resolved to their rollback-safe base selection
                // during restore, so preserving exactness here heals both
                // cases on the next ordinary event.
                if let Some(legacy) = state
                    .requested_model
                    .as_deref()
                    .map(str::trim)
                    .filter(|legacy| *legacy == selection.model)
                {
                    persisted.legacy_model = legacy.to_string();
                }
                persisted
            })
        })
    }

    /// Register the prompt channel for a managed session. Prompts submitted via
    /// `submit_message` are forwarded here (the adapter's driver task owns the
    /// receiver).
    pub fn register_managed_input(&self, session_id: &str, tx: mpsc::UnboundedSender<String>) {
        self.managed_inputs
            .insert(session_id.to_string(), tx.clone());
        // Drain anything queued BEFORE this channel existed — in practice the
        // spawn payload's `first_message` (see `queue_first_message`).
        //
        // This is the only place a managed session's queue can be drained, and
        // it has to be here: `register_managed` marks the row `Input` up front
        // (so the card is not born "connecting"), but no PTY wrapper is ever
        // attached and no hook `Input` transition arrives, so neither of the
        // PTY drains — `ingest`'s `became_input` flush nor `send_message_now` —
        // ever fires for it. A message left in the queue would sit there until
        // the session stopped and `clear_pending_messages` dropped it: silent
        // loss, and the worker idles forever with no prompt.
        //
        // Ordering is what makes the first message race-free: the daemon
        // enqueues it while still inside the spawn handler, before the 200, and
        // this drain runs the instant the driver's channel is live. There is no
        // window in between for the caller to have to guess at.
        for text in self.take_pending_messages(session_id) {
            if tx.send(text).is_err() {
                tracing::warn!(session_id, "managed prompt channel closed before the queued first message could be delivered");
                break;
            }
        }
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

    /// Remove the structural answer channel — but only if it is still the one
    /// this caller registered (`same_channel`). Short-lived registrants (the
    /// MCP AskUserQuestion endpoint parks a channel per question) must not
    /// clobber a newer registration made while they were awaiting: last
    /// writer wins on register, so only the last writer may unregister.
    pub fn unregister_managed_answer_if(
        &self,
        session_id: &str,
        tx: &mpsc::UnboundedSender<ManagedAnswer>,
    ) {
        self.managed_answers
            .remove_if(session_id, |_, existing| existing.same_channel(tx));
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
    /// `generation` is the token the caller claimed when it spawned. A driver
    /// whose process has finally wound down after a restart no longer owns the
    /// id, and must not wipe the successor's channels or tombstone its row —
    /// see [`SessionStore::generations`].
    /// Returns whether the teardown actually ran. A superseded caller gets
    /// `false` and must not proceed to drop anything else belonging to the id —
    /// the conversation in particular, which has no transcript to rebuild from
    /// for driver-fed providers.
    #[must_use]
    pub fn deregister_managed(&self, session_id: &str, generation: u64) -> bool {
        if !self.owns_generation(session_id, generation) {
            tracing::debug!(
                session = %session_id,
                generation,
                "skipping deregister from a superseded generation"
            );
            return false;
        }
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
            // A stopped session runs nothing — a leftover live-task count would
            // badge a dead row as "working in background".
            entry.background_tasks = 0;
            let completed_at = now_millis();
            for sub in &mut entry.subagents {
                if sub.status == SubagentStatus::Running {
                    sub.status = SubagentStatus::Complete;
                    sub.completed_at.get_or_insert(completed_at);
                }
            }
            entry.updated_at = OffsetDateTime::now_utc();
            let state = entry.clone();
            drop(entry);
            let _ = self.update_tx.send(SessionUpdate {
                session_id: session_id.to_string(),
                event: "SessionEnd".to_string(),
                state,
            });
        }
        true
    }

    /// Record the live background-task count (any task type) from the stream
    /// driver's latest `background_tasks_changed` frame. Broadcasts only on
    /// change so idle churn doesn't spam subscribers. The MODE deliberately
    /// does not ride on this: only `local_agent` tasks hold a session busy
    /// (see the stream driver) — this count is how clients surface ambient
    /// work (a dev server, a poll loop) without the mode lying "responding".
    pub fn set_background_tasks(&self, session_id: &str, live: u32) {
        let state = {
            let Some(mut entry) = self.states.get_mut(session_id) else {
                return;
            };
            if entry.background_tasks == live {
                return;
            }
            entry.background_tasks = live;
            entry.updated_at = OffsetDateTime::now_utc();
            entry.clone()
        };
        let _ = self.update_tx.send(SessionUpdate {
            session_id: session_id.to_string(),
            event: "Managed".to_string(),
            state,
        });
    }

    /// Upsert one managed-provider subagent row and mirror its live count into
    /// `background_tasks`, the field existing clients already use for ambient
    /// work. Claude hook/transcript rows are still desktop-owned; this is for
    /// providers like Codex whose native protocol reports subagent identities.
    pub fn apply_subagent_update(
        &self,
        session_id: &str,
        update: SubagentUpdate,
    ) -> Option<SessionState> {
        if update.id.trim().is_empty() {
            return None;
        }
        let now = now_millis();
        let state = {
            let mut entry = self.states.get_mut(session_id)?;
            if let Some(sub) = entry.subagents.iter_mut().find(|s| s.id == update.id) {
                if let Some(agent_type) = update.agent_type {
                    sub.agent_type = agent_type;
                }
                if let Some(description) = update.description {
                    sub.description = Some(description);
                }
                if let Some(tool_use_id) = update.tool_use_id {
                    sub.tool_use_id = Some(tool_use_id);
                }
                if let Some(model) = update.model {
                    sub.model = Some(model);
                }
                if let Some(last_tool_name) = update.last_tool_name {
                    sub.last_tool_name = Some(last_tool_name);
                }
                if let Some(last_tool_summary) = update.last_tool_summary {
                    sub.last_tool_summary = Some(last_tool_summary);
                }
                sub.status = update.status;
                match update.status {
                    SubagentStatus::Running => sub.completed_at = None,
                    SubagentStatus::Complete => {
                        sub.completed_at.get_or_insert(now);
                    }
                }
            } else {
                entry.subagents.push(SubagentInfo {
                    id: update.id,
                    agent_type: update.agent_type.unwrap_or_else(|| "codex".to_string()),
                    status: update.status,
                    started_at: now,
                    completed_at: (update.status == SubagentStatus::Complete).then_some(now),
                    description: update.description,
                    tool_use_id: update.tool_use_id,
                    model: update.model,
                    last_tool_name: update.last_tool_name,
                    last_tool_summary: update.last_tool_summary,
                });
            }
            entry.background_tasks = entry
                .subagents
                .iter()
                .filter(|s| s.status == SubagentStatus::Running)
                .count() as u32;
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

    /// Flip sessions whose process can no longer exist to `Stopped`.
    ///
    /// Every daemon-owned session has live plumbing while it runs — a PTY
    /// wrapper (`wrappers`, in-daemon and `claudemon wrap` alike) or a managed
    /// driver input (`managed_inputs`) — and its teardown path marks the row
    /// Stopped. But teardowns have escape hatches (a superseded generation, a
    /// crash between spawn and register), and an escaped row then advertises
    /// `unknown`/`input`/`responding` forever for a process that is gone: a
    /// live-looking ghost in every client. This sweep is the belt: no plumbing
    /// and no state change for `max_idle` → Stopped, broadcast as a SessionEnd
    /// like the regular teardown so clients run their ended pipeline.
    ///
    /// Hook-adopted sessions (a bare `claude` in a terminal — hooks POST here
    /// but nothing registers plumbing) can be swept while merely quiet; that is
    /// accepted and self-healing — their next hook event revives the row (the
    /// state machine sets mode on every event), while a wrongly-live ghost
    /// never corrects itself.
    pub fn sweep_ghost_sessions(&self, max_idle: time::Duration) -> Vec<String> {
        let now = OffsetDateTime::now_utc();
        let mut swept = Vec::new();
        for entry in self.states.iter() {
            let id = entry.key().clone();
            if entry.mode == SessionMode::Stopped {
                continue;
            }
            if now - entry.updated_at < max_idle {
                continue;
            }
            if self.wrappers.contains_key(&id) || self.managed_inputs.contains_key(&id) {
                continue;
            }
            swept.push(id);
        }
        let mut stopped = Vec::new();
        for id in swept {
            let state = {
                let Some(mut entry) = self.states.get_mut(&id) else {
                    continue;
                };
                // Re-check under the write lock — a hook/spawn may have raced us.
                if entry.mode == SessionMode::Stopped || now - entry.updated_at < max_idle {
                    continue;
                }
                entry.mode = SessionMode::Stopped;
                entry.clear_pending();
                entry.background_tasks = 0;
                entry.updated_at = OffsetDateTime::now_utc();
                entry.clone()
            };
            tracing::info!(session = %id, "sweeping ghost session (no plumbing, stale) to Stopped");
            let _ = self.update_tx.send(SessionUpdate {
                session_id: id.clone(),
                event: "SessionEnd".to_string(),
                state,
            });
            stopped.push(id);
        }
        stopped
    }

    fn managed_input(&self, session_id: &str) -> Option<mpsc::UnboundedSender<String>> {
        self.managed_inputs.get(session_id).map(|e| e.clone())
    }

    /// Attach model/usage/cost telemetry to a managed session (the adapter's
    /// equivalent of Claude's statusLine). Broadcasts on the status channel.
    pub fn apply_status_line(
        &self,
        session_id: &str,
        mut status: StatusLine,
    ) -> Option<SessionState> {
        let state = {
            let mut entry = self.states.get_mut(session_id)?;
            if entry.provider == "claude" {
                let root = entry.claude_config_root();
                self.patch_rate_limits(&mut status, &root);
            }
            reconcile_model_telemetry(&mut entry, &mut status);
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
    pub fn drop_pending_spawn(&self, session_id: &str, cwd: &str, generation: u64) {
        if !self.owns_generation(session_id, generation) {
            tracing::debug!(
                session = %session_id,
                generation,
                "skipping drop from a superseded generation"
            );
            return;
        }
        self.release_spawn_plumbing(session_id, cwd);
        self.states.remove(session_id);
        // Drop any hook-id → canonical-id aliases pointing at this session, so the
        // alias map doesn't accrue a permanent entry per spawn across churn.
        self.aliases.retain(|_, canonical| canonical != session_id);
    }

    /// Tear down everything a live spawn owns *except* the session's state row.
    ///
    /// This is what a PTY reader hitting EOF should do for a session that was
    /// actually used: the process is gone, so all the live plumbing must go,
    /// but the row is what makes the session resumable and what the desktop's
    /// Recent/History lists read. Removing it (as `drop_pending_spawn` does)
    /// made an agent the user quit vanish from the UI entirely instead of
    /// appearing as a stopped, resumable session.
    ///
    /// Leaves the row marked [`SessionMode::Stopped`], since by definition
    /// nothing is driving it any more — `SessionEnd` usually got there first,
    /// but the two are racing and this must hold either way.
    pub fn release_spawn(&self, session_id: &str, cwd: &str, generation: u64) {
        if !self.owns_generation(session_id, generation) {
            tracing::debug!(
                session = %session_id,
                generation,
                "skipping release from a superseded generation"
            );
            return;
        }
        self.release_spawn_plumbing(session_id, cwd);
        if let Some(mut st) = self.states.get_mut(session_id) {
            if st.mode != SessionMode::Stopped {
                st.mode = SessionMode::Stopped;
                st.updated_at = OffsetDateTime::now_utc();
            }
        }
    }

    /// The live-process plumbing shared by both teardown paths. Deliberately
    /// does not touch `states`/`aliases` — that's what distinguishes them.
    fn release_spawn_plumbing(&self, session_id: &str, cwd: &str) {
        // Only clear the cwd's pending slot if it still points at THIS session:
        // a fresh spawn in the same directory may already have claimed it while
        // the old child was still winding down.
        if self
            .pending_spawns_by_cwd
            .get(cwd)
            .is_some_and(|canonical| canonical.value() == session_id)
        {
            self.pending_spawns_by_cwd.remove(cwd);
        }
        self.wrappers.remove(session_id);
        self.buffers.remove(session_id);
        self.bytes_tx.remove(session_id);
        self.pending_messages.remove(session_id);
        self.input_since.remove(session_id);
        self.flush_epochs.remove(session_id);
        self.client_input_at.remove(session_id);
        self.paste_modes.remove(session_id);
        self.term_sizes.remove(session_id);
    }

    /// Whether a session has enough history to be worth keeping as a resumable
    /// row once its process exits.
    ///
    /// Mirrors [`SessionState::is_empty_stopped`] but without requiring the row
    /// to already be `Stopped` — at PTY EOF, `SessionEnd` may not have landed
    /// yet. A transcript path counts on its own: the conversation exists on
    /// disk regardless of what the hook counters say.
    pub fn is_resumable(&self, session_id: &str) -> bool {
        self.states.get(session_id).is_some_and(|st| {
            // Non-claude providers drive state from native events, not hooks,
            // so the prompt/tool counters mean nothing for them — never gate on
            // them there.
            st.provider != "claude"
                || st.user_prompts > 0
                || st.tool_calls > 0
                || st.transcript_path.is_some()
        })
    }

    pub fn wrapper(&self, session_id: &str) -> Option<WrapperHandle> {
        self.wrappers.get(session_id).map(|h| h.clone())
    }

    // --- terminal size ------------------------------------------------------

    /// Record the session's PTY size — called at spawn/register and on
    /// `/resize` so screen reconstruction uses the real grid. Every reporting
    /// path funnels through here, so this is the one place the grid is clamped
    /// to [`MAX_TERM_SIZE`] before anything allocates against it.
    pub fn note_term_size(&self, session_id: &str, cols: u16, rows: u16) {
        if cols == 0 || rows == 0 {
            return;
        }
        let (max_cols, max_rows) = MAX_TERM_SIZE;
        self.term_sizes.insert(
            session_id.to_string(),
            (cols.min(max_cols), rows.min(max_rows)),
        );
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
                if !self.enqueue_message(session_id, text) {
                    return MessageOutcome::QueueFull;
                }
                self.schedule_pending_flush(session_id);
                MessageOutcome::Sent
            }
            _ => {
                if !self.enqueue_message(session_id, text) {
                    return MessageOutcome::QueueFull;
                }
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

    /// Accept a Claude PTY compatibility command without routing it through a
    /// managed provider and without silently displacing an earlier message.
    /// The caller may persist its structural meaning only after this returns
    /// `Sent` or `Queued`.
    pub fn submit_pty_control_message(&self, session_id: &str, text: String) -> MessageOutcome {
        let Some(state) = self.states.get(session_id).map(|state| state.clone()) else {
            return MessageOutcome::NoSession;
        };
        if state.transport != Transport::Pty || self.is_managed(session_id) {
            return MessageOutcome::Rejected(state.mode);
        }
        if state.mode == SessionMode::Stopped {
            return MessageOutcome::Rejected(SessionMode::Stopped);
        }
        let Some(handle) = self.wrapper(session_id) else {
            return MessageOutcome::NoWrapper;
        };
        if handle.tx.is_closed() {
            return MessageOutcome::WrapperGone;
        }
        if !self.enqueue_pending_message(session_id, PendingMessage::pty_model_control(text)) {
            return MessageOutcome::QueueFull;
        }
        if state.mode == SessionMode::Input {
            self.schedule_pending_flush(session_id);
            MessageOutcome::Sent
        } else {
            if self.states.get(session_id).map(|state| state.mode) == Some(SessionMode::Input) {
                self.schedule_pending_flush(session_id);
            }
            MessageOutcome::Queued
        }
    }

    /// Queue a spawn's FIRST MESSAGE — the dispatch prompt that rode the spawn
    /// payload (`first_message`) instead of a separate `POST /message` after
    /// the id came back.
    ///
    /// Called from inside the spawn handlers, BEFORE they answer 200, which is
    /// the whole point. A caller that spawns and then sends is racing a session
    /// that is registered but not yet driven: `register_managed` marks the row
    /// `Input` with no wrapper attached, so a `POST /message` landing in that
    /// window resolves to [`MessageOutcome::NoWrapper`] (HTTP 404) — the
    /// message is refused and the worker sits with no prompt. Enqueuing here
    /// happens-before the id is even visible to the caller, so there is no
    /// window at all.
    ///
    /// Delivery differs by session kind and both routes already exist:
    ///   - PTY (`/sessions/spawn`): the existing cold-start ladder — the queue
    ///     is flushed on the first `Input` transition, settled and
    ///     submit-verified (`schedule_pending_flush`).
    ///   - managed (`/sessions/spawn-managed`): drained into the provider's
    ///     prompt channel by [`Self::register_managed_input`].
    ///
    /// Blank text is ignored, so an absent field and an empty one behave the
    /// same. If the child never launches, the row goes `Stopped` and
    /// `clear_pending_messages` drops the queue with it — the failure shows up
    /// as a stopped session, not as a live-looking card holding a lost prompt.
    pub fn queue_first_message(&self, session_id: &str, text: &str) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        self.enqueue_message(session_id, text.to_string())
    }

    fn enqueue_message(&self, session_id: &str, text: String) -> bool {
        self.enqueue_pending_message(session_id, PendingMessage::chat(text))
    }

    /// Enqueue one classified input without ever displacing an accepted model
    /// control. At capacity, either kind may replace the oldest ordinary chat;
    /// a queue containing only protected controls refuses the new input so the
    /// caller can surface that no acceptance (and therefore no persistence)
    /// occurred.
    fn enqueue_pending_message(&self, session_id: &str, message: PendingMessage) -> bool {
        let mut q = self
            .pending_messages
            .entry(session_id.to_string())
            .or_default();
        if q.len() >= MAX_PENDING_MESSAGES {
            // Bound memory under a stuck session without silently losing an
            // accepted structural control. Its owner persisted the canonical
            // selection only after this enqueue succeeded, so later sends must
            // never split durable truth from what the CLI executes. Reclaim an
            // ordinary chat slot or refuse before the caller reports success.
            let Some(drop_index) = q.iter().position(|queued| !queued.is_protected()) else {
                tracing::warn!(
                    session_id,
                    "pending-message queue full of accepted PTY controls; refusing new input"
                );
                return false;
            };
            let dropped = q.remove(drop_index);
            tracing::warn!(
                session_id,
                dropped = %dropped.text.chars().take(80).collect::<String>(),
                "pending-message queue full; dropping oldest"
            );
        }
        q.push(message);
        true
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
        let queued = self.take_pending_messages(session_id);
        let mut sent = Vec::with_capacity(queued.len());
        for text in queued {
            if self.send_message_now(session_id, text.clone()) == MessageOutcome::Sent {
                sent.push(text);
            }
        }
        sent
    }

    /// Atomically drain the queue (`mem::take` under the shard lock), leaving
    /// delivery to the caller. The PTY flush writes the batch to the child;
    /// `register_managed_input` pushes it down the provider's prompt channel.
    /// Atomicity is what lets the two drains coexist without ever double-sending.
    fn take_pending_messages(&self, session_id: &str) -> Vec<String> {
        self.pending_messages
            .get_mut(session_id)
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default()
            .into_iter()
            .map(|message| message.text)
            .collect()
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
    use crate::session::state::PendingOwner;

    fn hook(event: &str, session_id: &str, cwd: &str) -> HookEvent {
        HookEvent {
            event: event.into(),
            session_id: session_id.into(),
            cwd: Some(cwd.into()),
            timestamp: None,
            payload: serde_json::Map::new(),
        }
    }

    /// A real but immediately-exiting PTY child. `PtyHandle` is three trait
    /// objects over a live pty pair, so there is nothing to fake — and the
    /// ownership check under test is `Arc::ptr_eq`, which needs two genuinely
    /// distinct handles.
    fn fake_pty_handle() -> Arc<crate::wrapper::pty::PtyHandle> {
        let handle = crate::wrapper::pty::spawn(
            &["true".to_string()],
            ".",
            portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            },
            &std::collections::HashMap::new(),
        )
        .expect("spawn a throwaway pty");
        Arc::new(handle)
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
    fn note_term_size_clamps_absurd_grids() {
        let store = SessionStore::new();
        // `/resize`, the wrapper's Register and spawn all land here, and the
        // screen reconstruction allocates cols*rows cells from what's stored.
        store.note_term_size("s1", u16::MAX, u16::MAX);
        assert_eq!(store.term_size("s1"), MAX_TERM_SIZE);
        // Real sizes pass through untouched, and a zero dimension is ignored.
        store.note_term_size("s2", 213, 57);
        assert_eq!(store.term_size("s2"), (213, 57));
        store.note_term_size("s2", 0, 40);
        assert_eq!(store.term_size("s2"), (213, 57));
    }

    #[test]
    fn register_wrapper_refuses_to_displace_a_live_wrapper() {
        let store = SessionStore::new();
        let (incumbent, mut rx) = handle_with_rx();
        assert!(store.register_wrapper("s1", "/w", incumbent).is_some());

        let (hijacker, _hijacker_rx) = handle_with_rx();
        assert!(
            store.register_wrapper("s1", "/w", hijacker).is_none(),
            "a second wrapper must not take over a live session"
        );

        // Input still routes to the original wrapper.
        store
            .wrapper("s1")
            .expect("incumbent still registered")
            .tx
            .send(WrapperMessage::Input {
                bytes: B64.encode("hi"),
            })
            .unwrap();
        assert_eq!(next_input(&mut rx), b"hi");
    }

    #[test]
    fn register_wrapper_reclaims_a_dead_slot() {
        // The incumbent's channel is closed (its pump task is gone), so the id is
        // free again — a reconnecting wrapper must not be locked out forever.
        let store = SessionStore::new();
        let (dead, rx) = handle_with_rx();
        assert!(store.register_wrapper("s1", "/w", dead).is_some());
        drop(rx);

        let (fresh, mut fresh_rx) = handle_with_rx();
        assert!(store.register_wrapper("s1", "/w", fresh).is_some());
        store
            .wrapper("s1")
            .expect("registered")
            .tx
            .send(WrapperMessage::Input {
                bytes: B64.encode("hi"),
            })
            .unwrap();
        assert_eq!(next_input(&mut fresh_rx), b"hi");
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
    fn heartbeat_hooks_never_register_a_session() {
        let store = SessionStore::new();
        let mut hook_rx = store.subscribe_hooks();
        let mut update_rx = store.subscribe();

        // A warm ping's headless claude fires its lifecycle hooks. Marked as a
        // heartbeat, they must leave no trace: no state row, and nothing on the
        // hook / update broadcast channels a sidebar would render.
        store.mark_heartbeat("warm-1");
        store.ingest(hook("SessionStart", "warm-1", "/home/u"));
        store.ingest(hook("Stop", "warm-1", "/home/u"));

        assert!(store.get("warm-1").is_none(), "heartbeat leaked a session");
        assert!(store.list().is_empty(), "heartbeat surfaced in the list");
        assert!(
            hook_rx.try_recv().is_err(),
            "heartbeat hook was rebroadcast"
        );
        assert!(
            update_rx.try_recv().is_err(),
            "heartbeat produced a session update"
        );

        // Once unmarked, hooks for the same id behave normally again.
        store.unmark_heartbeat("warm-1");
        store.ingest(hook("SessionStart", "warm-1", "/home/u"));
        assert!(
            store.get("warm-1").is_some(),
            "post-unmark hook should register a session"
        );
    }

    #[test]
    fn unregister_managed_answer_if_only_removes_its_own_channel() {
        // The MCP AskUserQuestion endpoint parks one short-lived channel per
        // question, last-writer-wins. A stale registrant's unregister (its 6h
        // timeout or drop guard firing while a NEWER question is parked) must
        // be a no-op, or the newer question's channel is silently deleted and
        // /answer falls through to the PTY keystroke path.
        let store = SessionStore::new();
        store.register_managed("s1", "/w", "codex");
        let (tx1, _rx1) = mpsc::unbounded_channel::<ManagedAnswer>();
        let (tx2, mut rx2) = mpsc::unbounded_channel::<ManagedAnswer>();
        store.register_managed_answer("s1", tx1.clone());
        store.register_managed_answer("s1", tx2.clone()); // overwrites tx1

        // Stale unregister (tx1 lost the slot) leaves tx2's registration alone.
        store.unregister_managed_answer_if("s1", &tx1);
        assert!(store.submit_managed_answer(
            "s1",
            ManagedAnswer {
                option: Some(2),
                text: None,
                answers: None,
                answer_kinds: None,
            },
        ));
        let got = rx2.try_recv().expect("answer reaches the live channel");
        assert_eq!(got.option, Some(2));

        // The rightful (last) owner may remove its own channel.
        store.unregister_managed_answer_if("s1", &tx2);
        assert!(!store.submit_managed_answer(
            "s1",
            ManagedAnswer {
                option: Some(1),
                text: None,
                answers: None,
                answer_kinds: None,
            },
        ));
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

    #[test]
    fn respawn_revives_stopped_row_immediately() {
        let store = SessionStore::new();
        let (h, _rx) = handle_with_rx();
        store.register_spawn("s1", "/w", h);
        store.ingest(hook("SessionStart", "s1", "/w"));
        store.ingest(hook("SessionEnd", "s1", "/w"));
        assert_eq!(store.get("s1").unwrap().mode, SessionMode::Stopped);

        // Resume reuses the id. The row must leave Stopped at registration —
        // clients probe the mode right after attaching and treat a stopped
        // target as dead; SessionStart arrives too late to win that race.
        let (h2, _rx2) = handle_with_rx();
        let state = store.register_spawn("s1", "/w", h2);
        assert_eq!(state.mode, SessionMode::Unknown);
        assert_eq!(store.get("s1").unwrap().mode, SessionMode::Unknown);
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
    fn evict_stale_stopped_drops_archived_but_keeps_live_and_fresh() {
        use super::super::state::ARCHIVE_AFTER_SECONDS;
        let store = SessionStore::new();

        // Three stopped sessions, aged well past the archive window, plus one
        // recently-stopped and one live — via hydrate so we control the clock.
        store.hydrate(vec![
            crate::store::RestoredSession {
                id: "old1".into(),
                cwd: Some("/w".into()),
                tool_calls: 0,
                created_at: 1000,
                last_event_at: 1000,
                user_prompt_count: 0,
                model: None,
                requested_model: None,
                requested_selection: None,
                transcript_path: None,
                config_root: None,
            },
            crate::store::RestoredSession {
                id: "old2".into(),
                cwd: Some("/w".into()),
                tool_calls: 0,
                created_at: 1000,
                last_event_at: 1000,
                user_prompt_count: 0,
                model: None,
                requested_model: None,
                requested_selection: None,
                transcript_path: None,
                config_root: None,
            },
            crate::store::RestoredSession {
                id: "old3".into(),
                cwd: Some("/w".into()),
                tool_calls: 0,
                created_at: 1000,
                last_event_at: 1000,
                user_prompt_count: 0,
                model: None,
                requested_model: None,
                requested_selection: None,
                transcript_path: None,
                config_root: None,
            },
            crate::store::RestoredSession {
                id: "fresh".into(),
                cwd: Some("/w".into()),
                tool_calls: 0,
                created_at: 1000,
                last_event_at: 1000,
                user_prompt_count: 0,
                model: None,
                requested_model: None,
                requested_selection: None,
                transcript_path: None,
                config_root: None,
            },
        ]);
        // A live session must always survive, however old the clock says it is.
        store.ingest(hook("SessionStart", "live", "/w"));

        // Force the archive clock: `now` is far past the stale sessions'
        // last_event_at (1000) but the `fresh` row we bump to just-now first.
        let now = 1000 + ARCHIVE_AFTER_SECONDS + 10;
        if let Some(mut e) = store.states.get_mut("fresh") {
            e.updated_at = OffsetDateTime::from_unix_timestamp(now).unwrap();
        }

        assert_eq!(store.states.len(), 5);
        let evicted = store.evict_stale_stopped_at(now);
        assert_eq!(
            evicted, 3,
            "only the three archived stopped rows are dropped"
        );
        assert_eq!(store.states.len(), 2);
        assert!(store.get("old1").is_none());
        assert!(store.get("old2").is_none());
        assert!(store.get("old3").is_none());
        assert!(
            store.get("fresh").is_some(),
            "recently-stopped row survives"
        );
        assert!(store.get("live").is_some(), "live row survives");
    }

    /// THE $0.00 BUG, end to end.
    ///
    /// A session restored from SQLite must fold its real cost and tokens out of
    /// its transcript. Before `transcript_path` was persisted and rehydrated,
    /// `usage_for_session` folded from `None` — `Usage::default()`, every field
    /// zero — so on a fresh daemon 94 of 102 listed sessions reported $0.00, 0
    /// context tokens and no model however much they had actually cost. This
    /// test fails against that behaviour: drop the `st.transcript_path` line in
    /// `hydrate` and the cost goes back to 0.0.
    #[test]
    fn a_rehydrated_session_folds_real_usage_from_its_transcript() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "wks-hydrate-usage-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        crate::session::transcript::allow_root(&dir);

        // 1M output tokens of opus — a real, non-zero bill.
        let row = serde_json::json!({
            "type": "assistant",
            "message": {
                "id": "msg-1",
                "model": "claude-opus-4-8",
                "usage": {
                    "input_tokens": 0,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0,
                    "output_tokens": 1_000_000
                }
            }
        });
        let path = dir.join("restored.jsonl");
        std::fs::write(&path, format!("{}\n", serde_json::to_string(&row).unwrap())).unwrap();

        let store = SessionStore::new();
        store.hydrate(vec![crate::store::RestoredSession {
            id: "restored".into(),
            cwd: Some("/work".into()),
            tool_calls: 0,
            created_at: 1000,
            last_event_at: 2000,
            user_prompt_count: 1,
            model: None,
            requested_model: None,
            requested_selection: None,
            transcript_path: Some(path.to_str().unwrap().to_string()),
            config_root: None,
        }]);

        let state = store.get("restored").expect("hydrated");
        let u = crate::session::usage::usage_for_session(&state);
        assert!(
            u.cost_usd > 0.0,
            "a restored session must bill what its transcript says, got {u:?}",
        );
        assert_eq!(u.model.as_deref(), Some("claude-opus-4-8"));
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
                user_prompt_count: 3,
                model: Some("claude-opus-5".into()),
                // The `[1m]` marker the transcript id can never carry. Without
                // this restored, a resumed 1M session reverts to the table's
                // 200k answer for its stripped id and reads 5x too full.
                requested_model: Some("opus[1m]".into()),
                requested_selection: Some(crate::session::windows::ModelSelection {
                    model: "opus".into(),
                    context_window: Some(1_000_000),
                }),
                transcript_path: Some("/home/u/.claude/projects/-work/restored.jsonl".into()),
                config_root: None,
            },
            // Same id as the live one — must NOT overwrite it back to stopped.
            crate::store::RestoredSession {
                id: "live".into(),
                cwd: Some("/work/live".into()),
                tool_calls: 0,
                created_at: 1,
                last_event_at: 2,
                user_prompt_count: 0,
                model: None,
                requested_model: None,
                requested_selection: None,
                transcript_path: None,
                config_root: None,
            },
        ]);

        let restored = store.get("restored").expect("restored session present");
        assert_eq!(restored.mode, SessionMode::Stopped);
        assert_eq!(restored.cwd.as_deref(), Some("/work/restored"));
        assert_eq!(restored.tool_calls, 7);
        assert_eq!(
            restored.user_prompts, 3,
            "prompt count restored from the persisted event log"
        );
        assert_eq!(
            restored.transcript_path.as_deref(),
            Some("/home/u/.claude/projects/-work/restored.jsonl"),
            "the path the usage fold and the account attribution both read"
        );
        assert!(
            !restored.is_empty_stopped(),
            "a restored session with prompts is not an empty row"
        );

        let live = store.get("live").expect("live session present");
        assert_ne!(
            live.mode,
            SessionMode::Stopped,
            "live entry must win over hydrate"
        );
    }

    #[test]
    fn hydrate_preserves_an_opaque_non_claude_dash_one_m_companion() {
        let store = SessionStore::new();
        store.hydrate(vec![crate::store::RestoredSession {
            id: "opaque".into(),
            cwd: Some("/work".into()),
            tool_calls: 0,
            created_at: 1000,
            last_event_at: 2000,
            user_prompt_count: 1,
            model: None,
            requested_model: Some("vendor/custom-1m".into()),
            requested_selection: Some(crate::session::windows::ModelSelection {
                model: "vendor/custom-1m".into(),
                context_window: Some(1_000_000),
            }),
            transcript_path: None,
            config_root: None,
        }]);

        let restored = store.get("opaque").unwrap();
        assert_eq!(
            restored.requested_model.as_deref(),
            Some("vendor/custom-1m")
        );
        assert_eq!(
            restored.requested_selection,
            Some(crate::session::windows::ModelSelection {
                model: "vendor/custom-1m".into(),
                context_window: Some(1_000_000),
            })
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

    fn account_reading(five_pct: f64) -> AccountUsage {
        AccountUsage {
            five_hour_pct: Some(five_pct),
            five_hour_resets_at: Some(1_800_000_000),
            seven_day_pct: Some(61.0),
            seven_day_resets_at: None,
            monthly_pct: None,
            monthly_resets_at: None,
            out_of_credits: Some(false),
            fetched_at: OffsetDateTime::now_utc(),
        }
    }

    // The account-level reading must fill the utilization gaps a stream
    // session's own wire leaves (rate_limit_event carries resetsAt but rarely
    // utilization), while the wire's own resets keep priority over the
    // account's.
    #[test]
    fn account_usage_patches_claude_status_lines() {
        let store = SessionStore::new();
        store.register_managed("s1", "/tmp", "claude");
        store.set_account_usage("", account_reading(42.0));

        // set_account_usage alone already pushes a patched line, so a session
        // that never produced telemetry still gets gauges.
        let sl = store.get("s1").unwrap().status_line.expect("pushed line");
        assert_eq!(sl.five_hour_pct, Some(42.0));
        assert_eq!(sl.five_hour_resets_at, Some(1_800_000_000));
        assert!(
            sl.received_at.is_some(),
            "injected line must carry a timestamp"
        );

        // A wire status line (stream shape: reset without utilization) gets the
        // account pct injected, but its own fresher reset wins.
        let wire = StatusLine {
            five_hour_resets_at: Some(1_900_000_000),
            ..Default::default()
        };
        let state = store.apply_status_line("s1", wire).unwrap();
        let sl = state.status_line.unwrap();
        assert_eq!(sl.five_hour_pct, Some(42.0));
        assert_eq!(sl.five_hour_resets_at, Some(1_900_000_000));
        assert_eq!(sl.seven_day_pct, Some(61.0));
    }

    // The reading is for the *Claude* account — codex/opencode/pi sessions
    // must never inherit it.
    #[test]
    fn account_usage_never_touches_other_providers() {
        let store = SessionStore::new();
        store.register_managed("c1", "/tmp", "codex");
        store.set_account_usage("", account_reading(42.0));
        assert!(store.get("c1").unwrap().status_line.is_none());

        let state = store
            .apply_status_line("c1", StatusLine::default())
            .unwrap();
        assert_eq!(state.status_line.unwrap().five_hour_pct, None);
    }

    // A reading past its freshness window (poller stopped or failing) must not
    // keep overwriting live wire data.
    #[test]
    fn stale_account_usage_is_ignored() {
        let store = SessionStore::new();
        store.register_managed("s1", "/tmp", "claude");
        let mut old = account_reading(42.0);
        old.fetched_at = OffsetDateTime::now_utc() - time::Duration::minutes(10);
        store
            .account_usage
            .write()
            .unwrap()
            .insert(String::new(), old);

        let state = store
            .apply_status_line("s1", StatusLine::default())
            .unwrap();
        assert_eq!(state.status_line.unwrap().five_hour_pct, None);
    }

    // Two logged-in accounts = two config roots = two independent readings.
    // A reading for one root must patch ONLY that root's sessions — the old
    // single global copy stamped the default account's gauges onto every
    // session, overwriting a second account's own wire truth.
    #[test]
    fn account_usage_is_scoped_to_the_sessions_config_root() {
        let store = SessionStore::new();
        store.register_managed("def", "/tmp", "claude"); // no transcript → default root
        store.register_managed("work", "/tmp", "claude");
        store.states.get_mut("work").unwrap().transcript_path =
            Some("/home/u/.claude/accounts/work/projects/p/t.jsonl".into());
        assert_eq!(
            store.live_claude_config_roots(),
            vec![String::new(), "/home/u/.claude/accounts/work".to_string()],
        );

        // Default-account reading: patches the default session, not "work".
        store.set_account_usage("", account_reading(42.0));
        assert_eq!(
            store.get("def").unwrap().status_line.unwrap().five_hour_pct,
            Some(42.0),
        );
        assert!(store.get("work").unwrap().status_line.is_none());

        // The second account's own reading lands on its session with its own
        // numbers, leaving the default session's gauges alone.
        store.set_account_usage("/home/u/.claude/accounts/work", account_reading(7.0));
        assert_eq!(
            store
                .get("work")
                .unwrap()
                .status_line
                .unwrap()
                .five_hour_pct,
            Some(7.0),
        );
        assert_eq!(
            store.get("def").unwrap().status_line.unwrap().five_hour_pct,
            Some(42.0),
        );

        // A wire line from the "work" session is patched from ITS root's
        // reading, not the default's.
        let state = store
            .apply_status_line("work", StatusLine::default())
            .unwrap();
        assert_eq!(state.status_line.unwrap().five_hour_pct, Some(7.0));
    }

    // ── first message on the spawn payload ──────────────────────────────────
    //
    // The race this closes, measured rather than assumed: `register_managed`
    // marks the row `Input` up front but attaches no PTY wrapper, and
    // `register_managed_input` only runs deep inside the provider's driver task
    // (claude_stream.rs registers it AFTER `Command::spawn`), which starts after
    // the spawn handler has already answered 200. So a caller that spawns and
    // then posts its prompt is talking to a session that cannot take one yet.
    #[test]
    fn a_message_posted_before_the_driver_is_up_is_refused() {
        let store = SessionStore::new();
        store.register_managed("m1", "/tmp", "claude");
        // Exactly the window a spawn-then-send caller sits in.
        assert_eq!(
            store.submit_message("m1", "do the thing".into()),
            MessageOutcome::NoWrapper,
            "a managed session with no driver yet must refuse, not silently accept"
        );
    }

    // …and the fix: the prompt rides the spawn payload, is queued before the
    // 200, and the driver's own registration drains it. No window to race.
    #[test]
    fn queued_first_message_is_delivered_when_the_managed_driver_registers() {
        let store = SessionStore::new();
        store.register_managed("m1", "/tmp", "claude");
        assert!(store.queue_first_message("m1", "do the thing"));

        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        store.register_managed_input("m1", tx);

        assert_eq!(rx.try_recv().ok().as_deref(), Some("do the thing"));
        // Once only — the queue was drained, not copied.
        assert!(rx.try_recv().is_err(), "the first message must not repeat");
    }

    // Queue order is delivery order: a first message plus anything that landed
    // behind it arrives in the order it was queued, never reversed or merged.
    #[test]
    fn queued_first_messages_drain_in_order() {
        let store = SessionStore::new();
        store.register_managed("m1", "/tmp", "codex");
        store.queue_first_message("m1", "first");
        store.queue_first_message("m1", "second");

        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        store.register_managed_input("m1", tx);

        assert_eq!(rx.try_recv().ok().as_deref(), Some("first"));
        assert_eq!(rx.try_recv().ok().as_deref(), Some("second"));
    }

    // Blank is the same as absent, so an empty field never mints an empty turn
    // (and the spawn route reports `first_message_queued: false` for it).
    #[test]
    fn a_blank_first_message_is_not_queued() {
        let store = SessionStore::new();
        store.register_managed("m1", "/tmp", "pi");
        assert!(!store.queue_first_message("m1", "   \n "));

        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        store.register_managed_input("m1", tx);
        assert!(rx.try_recv().is_err());
    }

    // A PTY row is born `Unknown`, so its first message rides the existing
    // cold-start ladder instead: held until the session reaches `Input`.
    #[test]
    fn a_pty_first_message_waits_for_the_input_transition() {
        let store = SessionStore::new();
        store.register_spawn(
            "p1",
            "/tmp",
            WrapperHandle {
                tx: mpsc::unbounded_channel().0,
            },
        );
        assert!(store.queue_first_message("p1", "do the thing"));
        // Not drained by a managed registration it has nothing to do with, and
        // not lost: it is still queued for the Input flush.
        assert_eq!(
            store.take_pending_messages("p1"),
            vec!["do the thing".to_string()]
        );
    }

    #[test]
    fn accepted_pty_model_control_survives_later_chat_at_queue_capacity() {
        let store = SessionStore::new();
        let (h, _rx) = handle_with_rx();
        store.register_spawn("p1", "/tmp", h); // Unknown: every send stays queued.

        for index in 0..(MAX_PENDING_MESSAGES - 1) {
            assert_eq!(
                store.submit_message("p1", format!("chat-{index}")),
                MessageOutcome::Queued
            );
        }
        assert_eq!(
            store.submit_pty_control_message("p1", "/model opus[1m]".into()),
            MessageOutcome::Queued,
            "the owner accepted the control into the last queue slot"
        );

        assert_eq!(
            store.submit_message("p1", "later-chat".into()),
            MessageOutcome::Queued
        );
        let queued = store.take_pending_messages("p1");
        assert_eq!(queued.len(), MAX_PENDING_MESSAGES);
        assert!(
            queued.iter().any(|text| text == "/model opus[1m]"),
            "later chat must not evict an already-accepted model control"
        );
        assert!(!queued.iter().any(|text| text == "chat-0"));
        assert_eq!(queued.last().map(String::as_str), Some("later-chat"));
    }

    #[test]
    fn ordinary_model_shaped_chat_remains_evictable_at_queue_capacity() {
        let store = SessionStore::new();
        let (h, _rx) = handle_with_rx();
        store.register_spawn("p1", "/tmp", h); // Unknown: every send stays queued.

        for index in 0..MAX_PENDING_MESSAGES {
            assert_eq!(
                store.submit_message("p1", format!("/model fake chat {index}")),
                MessageOutcome::Queued,
            );
        }
        assert_eq!(
            store.submit_message("p1", "later-chat".into()),
            MessageOutcome::Queued,
        );

        let queued = store.take_pending_messages("p1");
        assert_eq!(queued.len(), MAX_PENDING_MESSAGES);
        assert_eq!(
            queued.first().map(String::as_str),
            Some("/model fake chat 1")
        );
        assert_eq!(queued.last().map(String::as_str), Some("later-chat"));
        assert!(
            !queued.iter().any(|text| text == "/model fake chat 0"),
            "ordinary chat provenance, not model-shaped content, controls eviction",
        );
    }

    #[test]
    fn a_new_pty_model_control_reclaims_ordinary_capacity_but_not_protected_capacity() {
        let store = SessionStore::new();
        let (h, _rx) = handle_with_rx();
        store.register_spawn("p1", "/tmp", h);

        assert_eq!(
            store.submit_pty_control_message("p1", "/model sonnet".into()),
            MessageOutcome::Queued,
        );
        for index in 0..(MAX_PENDING_MESSAGES - 1) {
            assert_eq!(
                store.submit_message("p1", format!("chat-{index}")),
                MessageOutcome::Queued,
            );
        }
        assert_eq!(
            store.submit_pty_control_message("p1", "/model opus".into()),
            MessageOutcome::Queued,
            "a structural control can be guaranteed by evicting ordinary chat",
        );

        let queued = store.take_pending_messages("p1");
        assert_eq!(queued.len(), MAX_PENDING_MESSAGES);
        assert!(queued.iter().any(|text| text == "/model sonnet"));
        assert!(queued.iter().any(|text| text == "/model opus"));
        assert!(!queued.iter().any(|text| text == "chat-0"));
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

        let _ = store.deregister_managed("m2", 1);
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

    // ── PTY EOF teardown ────────────────────────────────────────────────────
    //
    // A PTY reader hitting EOF used to call `drop_pending_spawn`, which removes
    // the session's state row. For a session the user had actually used that
    // meant the agent vanished from the desktop's Recent/History lists the
    // moment it exited, instead of appearing as a stopped, resumable row — the
    // post-terminate refetch burst simply got a list without it.

    #[test]
    fn a_used_session_stays_resumable_after_its_pty_exits() {
        let store = SessionStore::new();
        store.ingest(hook("SessionStart", "s1", "/work"));
        store.ingest(hook("UserPromptSubmit", "s1", "/work"));
        store.register_spawn("s1", "/work", handle());

        assert!(store.is_resumable("s1"), "a prompted session is resumable");
        store.release_spawn("s1", "/work", 1);

        let state = store.get("s1").expect("row survives PTY EOF");
        assert_eq!(state.mode, SessionMode::Stopped, "nothing drives it now");
        assert!(store.wrapper("s1").is_none(), "live plumbing is gone");
    }

    #[test]
    fn a_session_that_never_bound_is_dropped_whole() {
        let store = SessionStore::new();
        store.register_spawn("ghost", "/work", handle());
        // No SessionStart, no prompt: claude died before it ever came up.
        assert!(!store.is_resumable("ghost"));
        store.drop_pending_spawn("ghost", "/work", 1);
        assert!(store.get("ghost").is_none(), "nothing to resume, so no row");
    }

    #[test]
    fn a_tool_using_session_is_resumable_even_without_a_prompt() {
        let store = SessionStore::new();
        store.ingest(hook("SessionStart", "s1", "/work"));
        store.ingest(hook("PreToolUse", "s1", "/work"));
        assert!(
            store.is_resumable("s1"),
            "tool_calls guards a used session whose prompt events were dropped"
        );
    }

    #[test]
    fn a_managed_provider_session_is_always_resumable() {
        let store = SessionStore::new();
        store.register_managed("s1", "/work", "codex");
        // Codex/opencode/pi drive state from native events, so the hook-derived
        // prompt/tool counters mean nothing for them and must not gate this.
        assert!(store.is_resumable("s1"));
    }

    #[test]
    fn releasing_a_spawn_leaves_a_newer_spawn_in_the_same_cwd_alone() {
        let store = SessionStore::new();
        store.ingest(hook("SessionStart", "old", "/work"));
        store.ingest(hook("UserPromptSubmit", "old", "/work"));
        store.register_spawn("old", "/work", handle());
        // A replacement claims the cwd while the old child is still winding down.
        store.register_spawn("new", "/work", handle());

        store.release_spawn("old", "/work", 1);
        assert!(
            store.wrapper("new").is_some(),
            "the newer spawn keeps its plumbing"
        );
    }

    // ── spawn generations ───────────────────────────────────────────────────
    //
    // A restart reuses the session id on purpose: the desktop, the TUI and the
    // brain all close a session and immediately respawn with `resume` pinned to
    // the same id. Every close path is fire-and-forget — `terminate_managed`
    // only drops the input sender and `POST /signal` only sends SIGTERM — so the
    // old life's teardown routinely lands *after* its successor has registered.
    // Without a generation token that teardown reaches straight past its own
    // lifetime and dismantles the running one.

    #[test]
    fn generations_are_monotonic_per_session_and_independent_across_them() {
        let store = SessionStore::new();
        assert_eq!(store.claim_generation("s1"), 1);
        assert_eq!(store.claim_generation("s1"), 2);
        assert_eq!(store.claim_generation("s2"), 1, "ids count separately");

        assert!(store.owns_generation("s1", 2));
        assert!(
            !store.owns_generation("s1", 1),
            "the old life lost the slot"
        );
        // An unclaimed id answers true: there is no successor to protect, and
        // defaulting the other way would skip teardown and leak the plumbing.
        assert!(store.owns_generation("never-spawned", 1));
    }

    /// The managed restart, end to end: terminate, respawn on the same id, then
    /// let the previous driver's tail finally run. It must not touch the
    /// successor.
    #[test]
    fn a_stale_managed_driver_exit_leaves_its_successor_running() {
        let store = SessionStore::new();

        // First life.
        store.register_managed("m1", "/repo", "codex");
        let gen1 = store.claim_generation("m1");
        let (tx1, _rx1) = mpsc::unbounded_channel::<String>();
        store.register_managed_input("m1", tx1);

        // Restart: the desktop terminates and immediately respawns the same id.
        assert!(store.terminate_managed("m1"));
        store.register_managed("m1", "/repo", "codex");
        let gen2 = store.claim_generation("m1");
        let (tx2, mut rx2) = mpsc::unbounded_channel::<String>();
        store.register_managed_input("m1", tx2);
        assert_ne!(gen1, gen2);

        // Only now does the first driver's process finish winding down and its
        // tail call deregister.
        let _ = store.deregister_managed("m1", gen1);

        assert!(
            store.is_managed("m1"),
            "the successor's input channel must survive"
        );
        assert_eq!(
            store.get("m1").map(|s| s.mode),
            Some(SessionMode::Input),
            "the successor must not be marked Stopped while it is running"
        );
        // ...and the channel that survived is genuinely the successor's.
        assert!(matches!(
            store.submit_message("m1", "ping".into()),
            MessageOutcome::Sent | MessageOutcome::Queued
        ));
        assert_eq!(rx2.try_recv().ok().as_deref(), Some("ping"));
    }

    /// The owning generation still tears itself down — the guard must not turn
    /// a normal exit into a leak.
    #[test]
    fn the_current_generation_still_deregisters_itself() {
        let store = SessionStore::new();
        store.register_managed("m1", "/repo", "codex");
        let generation = store.claim_generation("m1");
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        store.register_managed_input("m1", tx);

        let _ = store.deregister_managed("m1", generation);

        assert!(!store.is_managed("m1"), "its own channels are released");
        assert_eq!(store.get("m1").map(|s| s.mode), Some(SessionMode::Stopped));
    }

    /// The PTY twin of the same race. `reap_pty` used to remove whatever handle
    /// sat under the id and SIGKILL it — so an old reader reaching EOF after the
    /// restart killed the freshly spawned child.
    #[tokio::test]
    async fn a_stale_pty_reader_does_not_reap_the_successors_handle() {
        let store = SessionStore::new();
        let first = fake_pty_handle();
        let second = fake_pty_handle();

        store.register_pty("s1", first.clone());
        store.register_pty("s1", second.clone()); // the restart replaces it

        // The first reader hits EOF late and reaps with the handle it owns.
        store.reap_pty_owned("s1", &first);

        let live = store
            .pty_handle("s1")
            .expect("successor's PTY still registered");
        assert!(
            Arc::ptr_eq(&live, &second),
            "the successor's handle must still be the registered one"
        );
    }

    #[tokio::test]
    async fn the_owning_pty_reader_still_clears_its_handle() {
        let store = SessionStore::new();
        let only = fake_pty_handle();
        store.register_pty("s1", only.clone());

        store.reap_pty_owned("s1", &only);

        assert!(
            store.pty_handle("s1").is_none(),
            "its own handle is cleared, so the map does not leak"
        );
    }

    /// A stale reader must not release the successor's plumbing or flip its row
    /// to Stopped (`release_spawn` runs right after `reap_pty` in the reader).
    #[test]
    fn a_stale_reader_does_not_release_the_successors_plumbing() {
        let store = SessionStore::new();
        store.register_spawn("s1", "/repo", handle());
        let gen1 = store.claim_generation("s1");
        store.ingest(hook("SessionStart", "s1", "/repo"));

        // Restart on the same id.
        store.register_spawn("s1", "/repo", handle());
        let gen2 = store.claim_generation("s1");
        assert_ne!(gen1, gen2);

        store.release_spawn("s1", "/repo", gen1);

        assert!(
            store.wrapper("s1").is_some(),
            "the successor keeps its input wrapper"
        );
        assert_ne!(
            store.get("s1").map(|s| s.mode),
            Some(SessionMode::Stopped),
            "the successor must not be tombstoned by the old life"
        );
    }

    /// The teardown reports whether it ran, so the caller can gate everything
    /// else it was about to drop on the same ownership check. The conversation
    /// is the one that matters: a driver-fed provider has no transcript to
    /// rebuild from, so a superseded exit erased the successor's history for
    /// good.
    #[test]
    fn a_superseded_deregister_reports_that_it_did_nothing() {
        let store = SessionStore::new();
        store.register_managed("m1", "/repo", "codex");
        let gen1 = store.claim_generation("m1");
        let (tx1, _rx1) = mpsc::unbounded_channel::<String>();
        store.register_managed_input("m1", tx1);

        // Restart on the same id.
        assert!(store.terminate_managed("m1"));
        store.register_managed("m1", "/repo", "codex");
        let gen2 = store.claim_generation("m1");
        let (tx2, _rx2) = mpsc::unbounded_channel::<String>();
        store.register_managed_input("m1", tx2);

        assert!(
            !store.deregister_managed("m1", gen1),
            "the old life must report that it tore nothing down"
        );
        assert!(
            store.deregister_managed("m1", gen2),
            "and the owning life must report that it did"
        );
    }

    /// The ghost sweep is the belt for every teardown escape hatch: a row with
    /// no live plumbing and no recent state change is a live-looking lie for a
    /// process that is gone (observed: a PTY that died at the folder-trust
    /// dialog sat in `unknown` for hours). Zero max_idle in these tests makes
    /// "stale" immediate — production passes 15 minutes.
    #[test]
    fn sweep_stops_unplumbed_rows_but_spares_live_ones() {
        let store = SessionStore::new();

        // Ghost: a row born from a hook (or an escaped teardown) — no plumbing.
        store.ingest(hook("SessionStart", "ghost", "/repo"));
        // Live managed session: driver input registered.
        store.register_managed("managed", "/repo", "claude");
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        store.register_managed_input("managed", tx);
        // Live PTY session: wrapper registered.
        store.ingest(hook("SessionStart", "pty", "/repo"));
        store.register_spawn("pty", "/repo", handle());

        let swept = store.sweep_ghost_sessions(time::Duration::ZERO);
        assert_eq!(swept, vec!["ghost".to_string()]);
        assert_eq!(store.get("ghost").unwrap().mode, SessionMode::Stopped);
        assert_ne!(store.get("managed").unwrap().mode, SessionMode::Stopped);
        assert_ne!(store.get("pty").unwrap().mode, SessionMode::Stopped);

        // Already-stopped rows are never re-swept (no broadcast churn).
        assert!(store.sweep_ghost_sessions(time::Duration::ZERO).is_empty());

        // A swept false positive self-heals: the next hook event revives it.
        store.ingest(hook("UserPromptSubmit", "ghost", "/repo"));
        assert_eq!(store.get("ghost").unwrap().mode, SessionMode::Responding);
    }

    /// `background_tasks` is the wire-visible ambient-work count. It must
    /// never survive the session's end — a stopped row badged "working in
    /// background" is the same lie the mode used to tell.
    #[test]
    fn background_tasks_count_updates_and_clears_on_teardown() {
        let store = SessionStore::new();
        store.register_managed("m1", "/repo", "claude");
        let gen = store.claim_generation("m1");

        store.set_background_tasks("m1", 2);
        assert_eq!(store.get("m1").unwrap().background_tasks, 2);
        // Unknown ids are a no-op, not a panic.
        store.set_background_tasks("nope", 3);

        assert!(store.deregister_managed("m1", gen));
        let row = store.get("m1").unwrap();
        assert_eq!(row.mode, SessionMode::Stopped);
        assert_eq!(row.background_tasks, 0);
    }

    #[test]
    fn subagent_updates_upsert_rows_and_drive_background_count() {
        let store = SessionStore::new();
        store.register_managed("m1", "/repo", "codex");

        store.apply_subagent_update(
            "m1",
            SubagentUpdate {
                id: "child-1".into(),
                agent_type: Some("codex".into()),
                status: SubagentStatus::Running,
                description: Some("inspect".into()),
                tool_use_id: Some("call-1".into()),
                model: Some("gpt-5.5-codex".into()),
                last_tool_name: Some("spawnAgent".into()),
                last_tool_summary: None,
            },
        );
        let row = store.get("m1").unwrap();
        assert_eq!(row.subagents.len(), 1);
        assert_eq!(row.subagents[0].id, "child-1");
        assert_eq!(row.subagents[0].status, SubagentStatus::Running);
        assert_eq!(row.background_tasks, 1);

        store.apply_subagent_update(
            "m1",
            SubagentUpdate {
                id: "child-1".into(),
                agent_type: None,
                status: SubagentStatus::Complete,
                description: None,
                tool_use_id: None,
                model: None,
                last_tool_name: Some("wait".into()),
                last_tool_summary: Some("done".into()),
            },
        );
        let row = store.get("m1").unwrap();
        assert_eq!(row.subagents.len(), 1);
        assert_eq!(row.subagents[0].status, SubagentStatus::Complete);
        assert_eq!(row.subagents[0].description.as_deref(), Some("inspect"));
        assert_eq!(row.subagents[0].last_tool_summary.as_deref(), Some("done"));
        assert!(row.subagents[0].completed_at.is_some());
        assert_eq!(row.background_tasks, 0);
    }

    /// The false-ACTIVE half of the subagent flap, reproduced from a live
    /// specimen: a codex row sat at `mode: input` with one `Running` subagent
    /// and `background_tasks: 1` for ten minutes across four further user
    /// turns, long after the agent itself had said the subagent was done — its
    /// completion frame simply never arrived. Nothing closed the row, so the
    /// parent read as working forever and its working→idle edge (the "worker
    /// finished" wake) never fired.
    ///
    /// The parent's own turn ending is the reconciliation point: a subagent row
    /// is scoped to the tool call that spawned it, and a tool call cannot
    /// outlive its turn.
    #[test]
    fn a_finished_child_cannot_hold_the_parent_active_past_its_turn() {
        let store = SessionStore::new();
        store.register_managed("m1", "/repo", "codex");
        let running = |id: &str| SubagentUpdate {
            id: id.into(),
            agent_type: Some("codex".into()),
            status: SubagentStatus::Running,
            description: Some("read-only check".into()),
            tool_use_id: Some("call-1".into()),
            model: None,
            last_tool_name: Some("spawnAgent".into()),
            last_tool_summary: None,
        };

        store.apply_subagent_update("m1", running("child-1"));
        store.set_managed_mode("m1", SessionMode::Responding, PendingWrite::Keep);
        assert_eq!(
            store.get("m1").unwrap().background_tasks,
            1,
            "mid-turn the child is genuinely live"
        );

        // Turn ends. The completion frame for child-1 never came.
        store.set_managed_mode("m1", SessionMode::Input, PendingWrite::Keep);
        let row = store.get("m1").unwrap();
        assert_eq!(row.mode, SessionMode::Input);
        assert_eq!(
            row.subagents[0].status,
            SubagentStatus::Complete,
            "a child still running once the parent's turn is over is stale"
        );
        assert!(row.subagents[0].completed_at.is_some());
        assert_eq!(
            row.background_tasks, 0,
            "a dead child must not hold the parent active"
        );

        // Closing is the self-healing direction: if the provider really does
        // have more to say about that agent, its next update re-opens the row.
        store.apply_subagent_update("m1", running("child-1"));
        let row = store.get("m1").unwrap();
        assert_eq!(row.subagents[0].status, SubagentStatus::Running);
        assert!(row.subagents[0].completed_at.is_none());
        assert_eq!(row.background_tasks, 1);
    }

    /// The reconciliation is scoped to providers that publish subagent rows.
    /// A Claude stream session's `background_tasks` counts background SHELLS
    /// it never has rows for, and zeroing that at every turn end would badge a
    /// live `npm run dev` as gone.
    #[test]
    fn turn_end_leaves_a_row_less_sessions_background_count_alone() {
        let store = SessionStore::new();
        store.register_managed("m1", "/repo", "claude");
        store.set_background_tasks("m1", 2);

        store.set_managed_mode("m1", SessionMode::Input, PendingWrite::Keep);
        let row = store.get("m1").unwrap();
        assert!(row.subagents.is_empty());
        assert_eq!(row.background_tasks, 2);
    }

    /// A parked approval refuses the mode write, so the turn is NOT over and
    /// nothing may be reconciled — the session is blocked on the user with its
    /// child still genuinely running.
    #[test]
    fn a_refused_turn_end_reconciles_nothing() {
        let store = SessionStore::new();
        store.register_managed("m1", "/repo", "codex");
        store.apply_subagent_update(
            "m1",
            SubagentUpdate {
                id: "child-1".into(),
                agent_type: Some("codex".into()),
                status: SubagentStatus::Running,
                description: None,
                tool_use_id: None,
                model: None,
                last_tool_name: None,
                last_tool_summary: None,
            },
        );
        store.set_managed_mode(
            "m1",
            SessionMode::Approval,
            PendingWrite::Park(PendingOwner::Primary, approval("Bash")),
        );

        store.set_managed_mode("m1", SessionMode::Input, PendingWrite::Keep);
        let row = store.get("m1").unwrap();
        assert_eq!(row.mode, SessionMode::Approval, "the block still stands");
        assert_eq!(row.subagents[0].status, SubagentStatus::Running);
        assert_eq!(row.background_tasks, 1);
    }

    fn approval(tool: &str) -> Pending {
        Pending::Approval {
            tool: Some(tool.into()),
            summary: Some("outside allowed working directories".into()),
            raw: serde_json::json!({ "tool_name": tool }),
        }
    }

    /// The funnel's whole point. A `Keep` write is liveness/enrichment — a busy
    /// ping, a message written to stdin, a background-task count changing —
    /// and it knows nothing about whether the user is still blocking the
    /// agent. While a request is parked it must change NOTHING: not the
    /// pending card (dropping it strands the session, because the parked
    /// request lives only inside the driver and nothing outside can rebuild
    /// it), and not the mode either (reporting `responding` for a session
    /// that is actually waiting on a human is the "lying state" half of the
    /// same bug).
    ///
    /// Before this guard lived in the store, every caller re-derived it by
    /// hand and four of them forgot.
    #[test]
    fn a_keep_write_changes_nothing_while_a_request_is_parked() {
        for (parked_mode, parked) in [
            (SessionMode::Approval, approval("Read")),
            (
                SessionMode::Question,
                Pending::Question {
                    questions: vec![],
                    raw: serde_json::Value::Null,
                },
            ),
        ] {
            let store = SessionStore::new();
            store.register_managed("s-keep", "/w", "claude");
            store.set_managed_mode(
                "s-keep",
                parked_mode,
                PendingWrite::Park(PendingOwner::Primary, parked),
            );
            let mut rx = store.subscribe();

            let returned = store
                .set_managed_mode("s-keep", SessionMode::Responding, PendingWrite::Keep)
                .expect("registered session");

            assert_eq!(
                returned.mode, parked_mode,
                "the returned state must be what the store HOLDS, not what was asked for — \
                 a driver mirrors its `cur_mode` from it"
            );
            let row = store.get("s-keep").expect("session state");
            assert_eq!(row.mode, parked_mode, "mode must not be demoted");
            assert!(
                row.pending().is_some(),
                "the pending card must survive a liveness write"
            );
            assert!(
                rx.try_recv().is_err(),
                "a suppressed write has nothing to announce — it must not broadcast"
            );
        }
    }

    /// The other half: with nothing parked, a `Keep` write is an ordinary
    /// liveness update and applies normally. Without this the composer would
    /// look dead after a nudge.
    #[test]
    fn a_keep_write_applies_when_nothing_is_parked() {
        let store = SessionStore::new();
        store.register_managed("s-live", "/w", "claude");
        store.set_managed_mode(
            "s-live",
            SessionMode::Input,
            PendingWrite::Resolve(PendingOwner::Primary),
        );

        let returned = store
            .set_managed_mode("s-live", SessionMode::Responding, PendingWrite::Keep)
            .expect("registered session");

        assert_eq!(returned.mode, SessionMode::Responding);
        assert_eq!(
            store.get("s-live").expect("session state").mode,
            SessionMode::Responding
        );
    }

    /// `Resolve` is the owner saying the block is over (the user answered, or a
    /// genuine turn boundary arrived). It clears the slot even from a parked
    /// state — otherwise an answered approval would leave a phantom card.
    #[test]
    fn a_resolve_write_clears_a_parked_request() {
        let store = SessionStore::new();
        store.register_managed("s-resolve", "/w", "claude");
        store.set_managed_mode(
            "s-resolve",
            SessionMode::Approval,
            PendingWrite::Park(PendingOwner::Primary, approval("Bash")),
        );

        store.set_managed_mode(
            "s-resolve",
            SessionMode::Responding,
            PendingWrite::Resolve(PendingOwner::Primary),
        );

        let row = store.get("s-resolve").expect("session state");
        assert_eq!(row.mode, SessionMode::Responding);
        assert!(row.pending().is_none());
    }

    /// `Park` replaces the card even when the mode is unchanged — a second
    /// parked request taking over the single slot from the one just answered.
    #[test]
    fn a_park_write_replaces_the_card_it_finds() {
        let store = SessionStore::new();
        store.register_managed("s-park", "/w", "claude");
        store.set_managed_mode(
            "s-park",
            SessionMode::Approval,
            PendingWrite::Park(PendingOwner::Primary, approval("Read")),
        );
        store.set_managed_mode(
            "s-park",
            SessionMode::Approval,
            PendingWrite::Park(PendingOwner::Primary, approval("Bash")),
        );

        let row = store.get("s-park").expect("session state");
        match row.pending() {
            Some(Pending::Approval { tool, .. }) => assert_eq!(tool.as_deref(), Some("Bash")),
            other => panic!("expected the newer approval card, got {other:?}"),
        }
    }

    fn question(text: &str) -> Pending {
        Pending::Question {
            questions: serde_json::from_value(serde_json::json!([
                { "question": text, "options": [{ "label": "yes" }] }
            ]))
            .expect("question fixture"),
            raw: serde_json::Value::Null,
        }
    }

    /// The cross-feed rule. A managed session has TWO feeds that can block the
    /// user at once — the driver (approval cards, held in its own FIFO) and the
    /// `AskUserQuestion` MCP shim (`daemon::mcp_ask`, registered for codex,
    /// opencode and pi). The slot shows one card, so the newer park takes it —
    /// but the older request is displaced, not destroyed, and comes back the
    /// moment the newer one is released. Dropping it instead is what strands
    /// the session: the driver only re-surfaces a queued approval from its own
    /// decision arm, and a decision can only arrive for a card the user can see.
    #[test]
    fn a_park_from_the_other_feed_displaces_the_card_and_its_resolve_restores_it() {
        let store = SessionStore::new();
        store.register_managed("s-two-feeds", "/w", "codex");
        store.set_managed_mode(
            "s-two-feeds",
            SessionMode::Approval,
            PendingWrite::Park(PendingOwner::Primary, approval("Bash")),
        );

        store.set_managed_mode(
            "s-two-feeds",
            SessionMode::Question,
            PendingWrite::Park(PendingOwner::Ask, question("Which db?")),
        );
        let row = store.get("s-two-feeds").expect("session state");
        assert!(
            matches!(row.pending(), Some(Pending::Question { .. })),
            "the newer request is the one the user is shown, got {:?}",
            row.pending()
        );
        assert_eq!(row.mode, SessionMode::Question);

        store.set_managed_mode(
            "s-two-feeds",
            SessionMode::Responding,
            PendingWrite::Resolve(PendingOwner::Ask),
        );
        let row = store.get("s-two-feeds").expect("session state");
        match row.pending() {
            Some(Pending::Approval { tool, .. }) => assert_eq!(tool.as_deref(), Some("Bash")),
            other => panic!("the displaced approval must come back, got {other:?}"),
        }
        assert_eq!(
            row.mode,
            SessionMode::Approval,
            "and the mode must report the block that is actually still live, \
             not the `Responding` the releasing feed asked for"
        );
    }

    /// A feed may only release what it raised. `mcp_ask`'s drop guard fires on
    /// every ended ask — including one that ends with a driver approval on the
    /// card — and an unattributed clear there is the unresolvable-approval
    /// shape reached across feeds.
    #[test]
    fn a_resolve_from_a_feed_that_owns_nothing_leaves_the_card_and_mode_alone() {
        let store = SessionStore::new();
        store.register_managed("s-foreign", "/w", "codex");
        store.set_managed_mode(
            "s-foreign",
            SessionMode::Approval,
            PendingWrite::Park(PendingOwner::Primary, approval("Write")),
        );
        let mut rx = store.subscribe();

        let returned = store
            .set_managed_mode(
                "s-foreign",
                SessionMode::Responding,
                PendingWrite::Resolve(PendingOwner::Ask),
            )
            .expect("registered session");

        assert_eq!(
            returned.mode,
            SessionMode::Approval,
            "returned state is what the store HOLDS"
        );
        let row = store.get("s-foreign").expect("session state");
        assert_eq!(row.mode, SessionMode::Approval);
        match row.pending() {
            Some(Pending::Approval { tool, .. }) => assert_eq!(tool.as_deref(), Some("Write")),
            other => panic!("a foreign resolve destroyed the card, got {other:?}"),
        }
        assert!(
            rx.try_recv().is_err(),
            "a write that changed nothing must not broadcast"
        );
    }

    /// The other direction, and the reason a foreign resolve is not simply a
    /// no-op: the releasing feed's OWN request — sitting displaced behind the
    /// other's card — really is over and must not be restored later.
    #[test]
    fn a_foreign_resolve_still_drops_the_releasing_feeds_displaced_request() {
        let store = SessionStore::new();
        store.register_managed("s-drop", "/w", "codex");
        // Driver parks an approval; the ask displaces it with a question.
        store.set_managed_mode(
            "s-drop",
            SessionMode::Approval,
            PendingWrite::Park(PendingOwner::Primary, approval("Bash")),
        );
        store.set_managed_mode(
            "s-drop",
            SessionMode::Question,
            PendingWrite::Park(PendingOwner::Ask, question("Which db?")),
        );

        // The user approves through `/approve` while the question is up: the
        // driver answers its FIFO head and releases. Its own displaced card is
        // spent; the question is not its to touch.
        store.set_managed_mode(
            "s-drop",
            SessionMode::Responding,
            PendingWrite::Resolve(PendingOwner::Primary),
        );
        let row = store.get("s-drop").expect("session state");
        assert!(
            matches!(row.pending(), Some(Pending::Question { .. })),
            "the ask's question is still live and still displayed, got {:?}",
            row.pending()
        );

        // Now the question is answered too — and the spent approval must NOT
        // come back as a card nothing can answer.
        store.set_managed_mode(
            "s-drop",
            SessionMode::Responding,
            PendingWrite::Resolve(PendingOwner::Ask),
        );
        let row = store.get("s-drop").expect("session state");
        assert!(
            row.pending().is_none(),
            "an already-answered approval must not be resurrected, got {:?}",
            row.pending()
        );
        assert_eq!(row.mode, SessionMode::Responding);
    }

    /// A feed replacing its OWN card (a driver surfacing the next request in
    /// its FIFO) is not a displacement — there is nothing of the other feed's
    /// to preserve, and the superseded card is spent.
    #[test]
    fn a_same_feed_park_replaces_without_displacing() {
        let store = SessionStore::new();
        store.register_managed("s-same", "/w", "codex");
        store.set_managed_mode(
            "s-same",
            SessionMode::Approval,
            PendingWrite::Park(PendingOwner::Primary, approval("Read")),
        );
        store.set_managed_mode(
            "s-same",
            SessionMode::Approval,
            PendingWrite::Park(PendingOwner::Primary, approval("Bash")),
        );
        store.set_managed_mode(
            "s-same",
            SessionMode::Responding,
            PendingWrite::Resolve(PendingOwner::Primary),
        );

        let row = store.get("s-same").expect("session state");
        assert!(
            row.pending().is_none(),
            "the replaced card was spent, not stashed, got {:?}",
            row.pending()
        );
        assert_eq!(row.mode, SessionMode::Responding);
    }

    fn hook_with(event: &str, session_id: &str, payload: serde_json::Value) -> HookEvent {
        let mut h = hook(event, session_id, "/w");
        h.payload = payload.as_object().cloned().unwrap_or_default();
        h
    }

    /// The daemon half of the same ownership rule, on the OTHER feed. Claude
    /// Code still runs the user's hooks for a headless stream session, so
    /// `PreToolUse`/`PostToolUse`/`PermissionRequest` land here routinely while
    /// the driver owns the pending slot — and `SessionState::apply` would
    /// happily clear it, replace it with a question, or resurrect an approval
    /// the driver already answered. `ingest` keeps those hooks as enrichment
    /// only. It had no test until now, which is how the same shape survived on
    /// the desktop's copy of this router.
    #[test]
    fn hooks_never_touch_the_pending_slot_of_a_daemon_owned_session() {
        for (event, payload) in [
            // Would clear the card outright.
            ("PostToolUse", serde_json::json!({ "tool_name": "Bash" })),
            // Would replace the parked approval with a question picker.
            (
                "PreToolUse",
                serde_json::json!({
                    "tool_name": "AskUserQuestion",
                    "tool_input": { "questions": [ { "question": "Which?", "options": [] } ] }
                }),
            ),
            // Would rewrite the card from the laggier of two feeds.
            (
                "PermissionRequest",
                serde_json::json!({ "tool_name": "Write" }),
            ),
            // Would declare the turn over while the CLI is still blocked.
            ("Stop", serde_json::json!({})),
        ] {
            let store = SessionStore::new();
            store.register_managed("s-hooked", "/w", "claude");
            store.set_transport("s-hooked", Transport::Stream);
            store.set_managed_mode(
                "s-hooked",
                SessionMode::Approval,
                PendingWrite::Park(PendingOwner::Primary, approval("Read")),
            );

            store.ingest(hook_with(event, "s-hooked", payload));

            let row = store.get("s-hooked").expect("session state");
            assert_eq!(
                row.mode,
                SessionMode::Approval,
                "a {event} hook must not drive the mode of a driver-owned session"
            );
            match row.pending() {
                Some(Pending::Approval { tool, .. }) => assert_eq!(
                    tool.as_deref(),
                    Some("Read"),
                    "a {event} hook must not rewrite the driver's card"
                ),
                other => panic!("a {event} hook destroyed the driver's approval: {other:?}"),
            }
        }
    }

    /// What the hook feed IS allowed to do for a daemon-owned session: enrich.
    /// `transcript_path` only ever arrives on a hook, and `/transcript` needs
    /// it.
    #[test]
    fn hooks_still_enrich_a_daemon_owned_session_with_the_transcript_path() {
        let store = SessionStore::new();
        store.register_managed("s-enrich", "/w", "claude");
        store.set_transport("s-enrich", Transport::Stream);

        store.ingest(hook_with(
            "PostToolUse",
            "s-enrich",
            serde_json::json!({ "transcript_path": "/tmp/t.jsonl" }),
        ));

        assert_eq!(
            store.get("s-enrich").and_then(|s| s.transcript_path),
            Some("/tmp/t.jsonl".to_string())
        );
    }

    /// The mirror image: for a PTY claude session the hook feed IS the owner,
    /// so the very same frame must raise the approval. Without this the guard
    /// above could be "fixed" by ignoring hooks everywhere, which would leave
    /// PTY sessions with no approvable record at all — the third defect in
    /// `docs/unresolvable-approval-findings.md`.
    #[test]
    fn the_hook_feed_still_owns_a_pty_sessions_pending_slot() {
        let store = SessionStore::new();
        store.ingest(hook("SessionStart", "s-pty", "/w"));

        store.ingest(hook_with(
            "PermissionRequest",
            "s-pty",
            serde_json::json!({ "tool_name": "Read", "summary": "outside roots" }),
        ));

        let row = store.get("s-pty").expect("session state");
        assert_eq!(row.mode, SessionMode::Approval);
        match row.pending() {
            Some(Pending::Approval { tool, .. }) => assert_eq!(tool.as_deref(), Some("Read")),
            other => panic!("the PTY hook feed must raise an approvable card, got {other:?}"),
        }
    }
}
