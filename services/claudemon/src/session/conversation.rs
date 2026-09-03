//! Daemon-owned conversation parsing.
//!
//! Tails each session's JSONL transcript incrementally (byte offset + partial
//! line carry) and broadcasts structured `ConversationDelta`s, so clients
//! (Workspacer's Electron main, the web mirror) render the conversation
//! without ever re-reading or re-parsing the transcript themselves.
//!
//! This is the *content* channel. Hooks remain the *control* channel (mode,
//! approvals, questions); the statusLine stream remains the telemetry channel.

use std::io::SeekFrom;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::broadcast;

use super::state::{Plan, PlanStatus, PlanStep, Transport};
use super::transcript::{blocks, flatten_tool_result, path_is_allowed, Block};
use super::{SessionMode, SessionStore};

const CONV_BROADCAST_CAPACITY: usize = 1024;
const TAIL_INTERVAL: Duration = Duration::from_millis(400);
/// Cap on the conversation items retained in memory per session. A live
/// session's `items` vec is its biggest allocation; without a bound a single
/// pathologically long session (thousands of tool calls) grows it without limit
/// for the daemon's life — a slow memory leak. 5000 comfortably covers a very
/// long session (even a heavy day rarely exceeds a few hundred tool calls +
/// messages) while capping the worst case. Oldest items drain first (front-drop,
/// like `OutputBuffer`'s byte ring in `session/store.rs`); `seq` keeps counting
/// the true total so gap detection / resync stay correct — a late joiner
/// adopting the snapshot simply gets the most-recent window.
const MAX_CONVERSATION_ITEMS: usize = 5000;
/// Keep draining a stopped session's transcript briefly — the final
/// assistant message can flush to disk after the Stop/SessionEnd hook fires.
const STOPPED_DRAIN_SECS: i64 = 30;

/// One structured event parsed out of the transcript, in timeline order.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConversationItem {
    UserMessage {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
    },
    AssistantText {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
    },
    /// Token usage riding on an assistant message. `message_id` lets clients
    /// dedup the per-block repetition of one streamed message. `sidechain`
    /// marks usage from a sub-agent (isSidechain) row: real spend that belongs
    /// in the session's totals, but not in the main thread's context gauge.
    Usage {
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        usage: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        sidechain: bool,
    },
    /// The agent's current plan / checklist (Claude's `TodoWrite`, Codex's
    /// `update_plan`). Last-write-wins full replacement — clients keep the
    /// newest one they see. Fields inlined (not the shared `Plan` struct) so the
    /// serialized shape is unambiguous: `{ kind: "plan", steps: [...],
    /// updatedAt?: <rfc3339> }`.
    Plan {
        steps: Vec<PlanStep>,
        #[serde(rename = "updatedAt", skip_serializing_if = "Option::is_none")]
        updated_at: Option<String>,
    },
    /// A slash-command run. Claude Code echoes the invocation into the
    /// transcript as `<command-name>/foo</command-name>` (+ optional
    /// `<command-args>`); the stream driver also emits one directly when the
    /// user's sent text names a known command. `name` is stored without the
    /// leading slash. Clients render this as a command card, not a user bubble.
    SlashCommand {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        args: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
    },
    /// Local output of a slash command (`<local-command-stdout>` /
    /// `-stderr>` rows), ANSI-stripped. Arrives separately from the
    /// invocation; clients attach it to the nearest preceding SlashCommand
    /// (mirroring how ToolResult joins ToolUse).
    CommandOutput {
        output: String,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ConversationDelta {
    pub session_id: String,
    /// Per-session sequence of the *last* item in this delta (1-based, counts
    /// items since the log was (re)built). Clients detect gaps by checking
    /// `seq == last_seq + items.len()` and resync from the snapshot endpoint.
    pub seq: u64,
    /// True when the log was rebuilt from scratch (transcript replaced or
    /// truncated). Clients must discard prior state and adopt `items` wholesale.
    pub reset: bool,
    pub items: Vec<ConversationItem>,
}

#[derive(Default)]
struct TailLog {
    path: String,
    offset: u64,
    /// Bytes after the last newline — kept as raw bytes so a UTF-8 sequence
    /// split across reads survives intact.
    partial: Vec<u8>,
    /// Cursors for the session's sub-agent transcripts
    /// (`<transcript-stem>/subagents/*.jsonl`), keyed by absolute path.
    side: std::collections::HashMap<String, SideCursor>,
    items: Vec<ConversationItem>,
    seq: u64,
    /// The sequence each retained item carries — parallel to `items`.
    ///
    /// `seq` counts everything ever pushed, including fragments coalesced into
    /// an existing item and items since evicted, so the first item's sequence is
    /// NOT `seq - items.len() + 1`. Stream providers emit one item per token
    /// chunk and `try_coalesce_assistant_text` folds them, which makes the two
    /// counts diverge by hundreds inside a single turn. Consumers that
    /// reconstructed the window from the item count therefore looked far to the
    /// right of the real one: `?since=` returned the whole retained
    /// conversation on every poll, and the TUI's snapshot/delta continuity check
    /// could never agree, leaving it refetching a snapshot per delta forever.
    ///
    /// A coalesced fragment updates its item's entry, so an item counts as new
    /// for any `since` before the last fragment folded into it — which is what a
    /// client polling mid-stream needs.
    item_seqs: Vec<u64>,
    /// Folded TaskCreate/TaskUpdate state — the task-tool counterpart of the
    /// TodoWrite plan, carried across batches because task edits are
    /// incremental (unlike TodoWrite's full rewrites).
    tasks: TaskFold,
}

impl TailLog {
    /// Append items to the in-memory log, enforcing [`MAX_CONVERSATION_ITEMS`]
    /// by draining the oldest. Preserves ordering and the newest items. Note it
    /// does NOT touch `seq`: the caller bumps that by the number of items pushed
    /// so it stays a true running count even as older items are dropped.
    /// Append items, stamping each with its own sequence, and evict from the
    /// front past the cap. `first_of` is the sequence of the first appended item.
    fn extend_bounded(&mut self, new: impl IntoIterator<Item = ConversationItem>, first_of: u64) {
        let before = self.items.len();
        self.items.extend(new);
        for i in 0..(self.items.len() - before) {
            self.item_seqs.push(first_of + i as u64);
        }
        if self.items.len() > MAX_CONVERSATION_ITEMS {
            let overflow = self.items.len() - MAX_CONVERSATION_ITEMS;
            self.items.drain(0..overflow);
            self.item_seqs.drain(0..overflow);
        }
    }

    /// Start the log over: a resume re-tails from a new transcript path, so the
    /// previous life's items are void.
    ///
    /// `item_seqs` must go with `items`. Clearing one without the other leaves
    /// `first_seq()` reporting a sequence from the previous life — larger than
    /// the `seq` we just restarted at, which is incoherent. `items_skip` then
    /// saturates to zero and `?since=` hands back the whole retained window on
    /// every poll: the exact symptom the per-item sequences exist to remove.
    /// `extend_bounded` drains both vectors equally, so the skew never heals.
    fn reset_log(&mut self) {
        self.items.clear();
        self.item_seqs.clear();
        self.seq = 0;
        // Sub-agent usage was cleared with the items — rewind those cursors so
        // the next subagent pass re-emits it into the new log.
        self.side.clear();
    }

    /// The sequence of the first retained item, or `seq` when nothing is
    /// retained. Never derived from the item count — see [`TailLog::item_seqs`].
    fn first_seq(&self) -> u64 {
        self.item_seqs.first().copied().unwrap_or(self.seq)
    }

    /// Fold a streamed assistant-text delta into the trailing item when the log
    /// already ends in assistant text. Returns whether it was absorbed.
    ///
    /// Stream-transport providers emit one item per *token chunk*, so without
    /// this a single ordinary reply becomes hundreds of `AssistantText`
    /// entries. [`MAX_CONVERSATION_ITEMS`] is sized in messages — "a heavy day
    /// rarely exceeds a few hundred tool calls + messages" — so counting
    /// fragments instead makes it start evicting the front of the conversation
    /// partway through a single long turn. A client resyncing from
    /// `/conversation` would then adopt a history whose beginning (the user's
    /// original prompt, the early tool calls) is already gone, starting
    /// mid-sentence inside a recent reply.
    ///
    /// Only contiguous assistant text merges: a tool use or a user message
    /// landing between two chunks ends the run, which is the turn boundary.
    /// Timestamps keep the first chunk's — when the message started.
    fn try_coalesce_assistant_text(&mut self, item: &ConversationItem) -> bool {
        let ConversationItem::AssistantText { text, .. } = item else {
            return false;
        };
        match self.items.last_mut() {
            Some(ConversationItem::AssistantText { text: prev, .. }) => {
                prev.push_str(text);
                true
            }
            _ => false,
        }
    }
}

#[derive(Default, Clone)]
struct SideCursor {
    offset: u64,
    partial: Vec<u8>,
}

/// Shared handle: the tailer task writes, API handlers read/subscribe.
#[derive(Clone)]
pub struct ConversationStore {
    logs: Arc<DashMap<String, TailLog>>,
    tx: broadcast::Sender<ConversationDelta>,
}

impl ConversationStore {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CONV_BROADCAST_CAPACITY);
        Self {
            logs: Arc::new(DashMap::new()),
            tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ConversationDelta> {
        self.tx.subscribe()
    }

    /// Full parsed history + current seq for one session (for clients joining
    /// mid-session or recovering from a missed delta).
    pub fn snapshot(&self, session_id: &str) -> Option<(u64, Vec<ConversationItem>)> {
        self.logs.get(session_id).map(|l| (l.seq, l.items.clone()))
    }

    /// Like [`snapshot`], plus the sequence of the first retained item — the
    /// number a caller needs to place the window, and which cannot be derived
    /// from the item count. See [`TailLog::first_seq`].
    pub fn snapshot_windowed(&self, session_id: &str) -> Option<(u64, u64, Vec<ConversationItem>)> {
        self.logs
            .get(session_id)
            .map(|l| (l.seq, l.first_seq(), l.items.clone()))
    }

    /// Append items for a *managed* session — one not backed by a transcript
    /// file (e.g. the OpenCode / Codex adapters drive their conversation from a
    /// live event stream). Maintains the same per-session seq + broadcast
    /// contract as the transcript tailer, so existing clients consume managed
    /// and Claude sessions identically. The first push for a session carries
    /// `reset` so late joiners adopt history wholesale. No-op for empty items.
    pub fn push(&self, session_id: &str, items: Vec<ConversationItem>) {
        if items.is_empty() {
            return;
        }
        let delta = {
            let mut log = self.logs.entry(session_id.to_string()).or_default();
            let reset = log.seq == 0;
            let first_of = log.seq + 1;
            log.seq += items.len() as u64;
            // The delta broadcast below is unchanged either way — live clients
            // still stream token by token. Only the retained log folds, so an
            // item means a message again for anything adopting the snapshot.
            if items.len() == 1 && log.try_coalesce_assistant_text(&items[0]) {
                // Absorbed into the trailing item: no new entry, but that item
                // is now newer than it was, so a client polling from before this
                // fragment must still be given it.
                let now = log.seq;
                if let Some(last) = log.item_seqs.last_mut() {
                    *last = now;
                }
            } else {
                log.extend_bounded(items.iter().cloned(), first_of);
            }
            ConversationDelta {
                session_id: session_id.to_string(),
                seq: log.seq,
                reset,
                items,
            }
        };
        let _ = self.tx.send(delta);
    }

    /// Drop a session's accumulated conversation log. Called when a session ends
    /// so the full transcript (kept in memory for late-joiner snapshots) is
    /// reclaimed instead of living for the daemon's lifetime. Late `/conversation`
    /// snapshot requests after this simply return None (the session is gone).
    pub fn forget(&self, session_id: &str) {
        self.logs.remove(session_id);
    }
}

impl Default for ConversationStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn the background tail loop: every tick, read whatever new bytes each
/// live session's transcript has gained and broadcast the parsed items.
pub fn spawn_tailer(sessions: SessionStore, conv: ConversationStore) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TAIL_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            for state in sessions.list() {
                let Some(path) = state.transcript_path.clone() else {
                    continue;
                };
                if state.mode == SessionMode::Stopped {
                    let age = time::OffsetDateTime::now_utc() - state.updated_at;
                    if age.whole_seconds() > STOPPED_DRAIN_SECS {
                        // Past the drain window the session is done. Drop its
                        // in-memory transcript log (the biggest per-session
                        // allocation) so it doesn't live for the daemon's life —
                        // it's re-derivable from the on-disk JSONL, and the delta
                        // stream is finished. Idempotent.
                        conv.forget(&state.session_id);
                        continue;
                    }
                }
                if let Err(err) = tail_one(&sessions, &conv, &state.session_id, &path).await {
                    tracing::debug!(?err, session = %state.session_id, "transcript tail failed");
                }
                if let Err(err) = tail_subagents(&conv, &state.session_id, &path).await {
                    tracing::debug!(?err, session = %state.session_id, "subagent tail failed");
                }
            }
        }
    });
}

/// Read new bytes for one session and broadcast a delta if anything parsed.
///
/// DashMap guards are never held across an await: we copy the cursor out,
/// do the file I/O, then re-acquire to commit. Only the tailer task mutates
/// logs, so the copy can't go stale.
async fn tail_one(
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    path: &str,
) -> std::io::Result<()> {
    // `transcript_path` is whatever the unauthenticated hook ingress last said
    // it was, and this loop re-reads it every tick and broadcasts the parsed
    // lines to `/conversation` — a continuous read of a caller-named file, so a
    // wider primitive than the one-shot `/transcript` endpoint, not a narrower
    // one. Same predicate bounds both. The error is what surfaces the refusal in
    // the caller's log; returning Ok would hide it.
    if !path_is_allowed(std::path::Path::new(path)) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("transcript path is outside every known transcript root: {path}"),
        ));
    }

    // Who owns the agent-error marker for this session (see [`ApiErrorMarking`]).
    // A session the store has never heard of defaults to marking: the row's own
    // `isApiErrorMessage` says the turn genuinely failed, so a marker we did not
    // strictly need is far cheaper than a failure nobody reports.
    let marking = match store.get(session_id) {
        Some(state) if state.transport == Transport::Stream => ApiErrorMarking::Driver,
        _ => ApiErrorMarking::Tailer,
    };

    let (mut offset, mut partial, mut reset, mut task_fold) = match conv.logs.get(session_id) {
        Some(l) if l.path == path => (l.offset, l.partial.clone(), false, l.tasks.clone()),
        // New session, or claude switched transcript files (e.g. resume).
        _ => (0, Vec::new(), true, TaskFold::default()),
    };

    let len = tokio::fs::metadata(path).await?.len();
    if len < offset {
        // Truncated/replaced in place — rebuild from the top.
        offset = 0;
        partial.clear();
        reset = true;
        task_fold = TaskFold::default();
    }
    if len == offset && !reset {
        return Ok(());
    }

    let mut buf = partial;
    if len > offset {
        let mut file = tokio::fs::File::open(path).await?;
        file.seek(SeekFrom::Start(offset)).await?;
        // Bound the read to the length we statted so `offset` stays consistent
        // even if the file grows while we read.
        let mut chunk = Vec::with_capacity((len - offset) as usize);
        (&mut file)
            .take(len - offset)
            .read_to_end(&mut chunk)
            .await?;
        offset += chunk.len() as u64;
        buf.extend_from_slice(&chunk);
    }

    // Split off the trailing partial line (bytes after the last newline).
    let new_partial = match buf.iter().rposition(|&b| b == b'\n') {
        Some(idx) => buf.split_off(idx + 1),
        None => std::mem::take(&mut buf),
    };

    let complete = String::from_utf8_lossy(&buf);
    let mut items = Vec::new();
    // The latest plan seen in this batch (last-write-wins across the batch, and
    // across the whole file on a reset/rebuild — so a resync replays the current
    // plan, not every historical revision).
    let mut latest_plan: Option<Plan> = None;
    for line in complete.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            items.extend(items_from_row(&value, marking));
            // Task tools and TodoWrite feed the same last-write-wins plan, in
            // row order — whichever the agent used most recently wins.
            if task_fold.fold_row(&value) {
                let ts = value
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                latest_plan = Some(task_fold.to_plan(ts));
            }
            if let Some(plan) = plan_from_row(&value) {
                latest_plan = Some(plan);
            }
        }
    }

    let delta = {
        let mut entry = conv.logs.entry(session_id.to_string()).or_default();
        if reset {
            entry.reset_log();
        }
        entry.path = path.to_string();
        entry.offset = offset;
        entry.partial = new_partial;
        entry.tasks = task_fold;
        if items.is_empty() && !reset {
            // No conversation items, but a plan change must still surface
            // (e.g. a row that only confirmed a TaskCreate).
            if let Some(plan) = latest_plan {
                drop(entry);
                store.set_plan(conv, session_id, plan);
            }
            return Ok(());
        }
        let first_of = entry.seq + 1;
        entry.seq += items.len() as u64;
        entry.extend_bounded(items.iter().cloned(), first_of);
        ConversationDelta {
            session_id: session_id.to_string(),
            seq: entry.seq,
            reset,
            items,
        }
    };
    let _ = conv.tx.send(delta);
    // Surface the plan after the batch's items so it lands just past the
    // TodoWrite tool_use that carried it. `set_plan` both records it on the
    // session state and pushes a `plan` conversation item (its own delta).
    if let Some(plan) = latest_plan {
        store.set_plan(conv, session_id, plan);
    }
    Ok(())
}

/// Tail the session's sub-agent transcripts. Current Claude Code writes each
/// sub-agent (Task tool / teammate) to its own JSONL under
/// `<transcript-stem>/subagents/`, so their spend never appears in the main
/// file. Only usage is extracted — tagged `sidechain: true` — and pushed into
/// the same per-session delta stream/snapshot the main tailer feeds.
async fn tail_subagents(
    conv: &ConversationStore,
    session_id: &str,
    main_path: &str,
) -> std::io::Result<()> {
    // The sidechain dir is derived from the same untrusted path, so it inherits
    // the same bound. Quiet here — `tail_one` already reported the refusal for
    // this session on this tick.
    if !path_is_allowed(std::path::Path::new(main_path)) {
        return Ok(());
    }
    let Some(stem) = main_path.strip_suffix(".jsonl") else {
        return Ok(());
    };
    let dir = format!("{stem}/subagents");
    let mut rd = match tokio::fs::read_dir(&dir).await {
        Ok(rd) => rd,
        Err(_) => return Ok(()), // no sub-agents (yet) — the common case
    };
    let mut files: Vec<String> = Vec::new();
    while let Ok(Some(ent)) = rd.next_entry().await {
        let p = ent.path();
        if p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            if let Some(s) = p.to_str() {
                files.push(s.to_string());
            }
        }
    }
    files.sort();

    let mut items = Vec::new();
    for file in files {
        // Copy the cursor out; never hold the DashMap guard across an await.
        let (mut offset, partial) = conv
            .logs
            .get(session_id)
            .and_then(|l| l.side.get(&file).cloned())
            .map(|c| (c.offset, c.partial))
            .unwrap_or((0, Vec::new()));
        let Ok(meta) = tokio::fs::metadata(&file).await else {
            continue;
        };
        let len = meta.len();
        let mut buf = partial;
        if len < offset {
            // Truncated/replaced — re-read from the top. (Duplicate usage from
            // the re-read is deduped client-side by message id.)
            offset = 0;
            buf.clear();
        }
        if len == offset {
            continue;
        }
        let mut f = tokio::fs::File::open(&file).await?;
        f.seek(SeekFrom::Start(offset)).await?;
        let mut chunk = Vec::with_capacity((len - offset) as usize);
        (&mut f).take(len - offset).read_to_end(&mut chunk).await?;
        offset += chunk.len() as u64;
        buf.extend_from_slice(&chunk);

        let new_partial = match buf.iter().rposition(|&b| b == b'\n') {
            Some(idx) => buf.split_off(idx + 1),
            None => std::mem::take(&mut buf),
        };
        for line in String::from_utf8_lossy(&buf).lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                items.extend(sidechain_usage_from_row(&value));
            }
        }
        conv.logs
            .entry(session_id.to_string())
            .or_default()
            .side
            .insert(
                file,
                SideCursor {
                    offset,
                    partial: new_partial,
                },
            );
    }

    if items.is_empty() {
        return Ok(());
    }
    let delta = {
        let mut entry = conv.logs.entry(session_id.to_string()).or_default();
        let first_of = entry.seq + 1;
        entry.seq += items.len() as u64;
        entry.extend_bounded(items.iter().cloned(), first_of);
        ConversationDelta {
            session_id: session_id.to_string(),
            seq: entry.seq,
            reset: false,
            items,
        }
    };
    let _ = conv.tx.send(delta);
    Ok(())
}

/// Parse one transcript row into zero or more conversation items.
///
/// Mirrors what clients used to derive themselves: user text, assistant text
/// per content block, tool_use starts, tool_results joined by id, and usage.
/// Thinking blocks and meta rows are skipped.
/// Injected, non-conversational user blocks Claude Code writes into the
/// transcript: system reminders, the caveat wrapper around local-command
/// output, and the background-task notifications workflows emit. They're UI
/// noise — never what the user typed — so they're filtered out of the
/// conversation surfaced to clients. Slash-command echoes and local-command
/// output are NOT dropped here anymore: [`command_items_from_text`] parses
/// them into `SlashCommand` / `CommandOutput` items first; this filter only
/// catches the command-shaped tags it didn't recognize (e.g. a stray
/// `<command-message>`-only row, `<local-command-caveat>`).
fn is_injected_meta(text: &str) -> bool {
    const TAGS: [&str; 5] = [
        "<task-notification",
        "<system-reminder",
        "<local-command",
        "<command-name",
        "<command-message",
    ];
    let t = text.trim_start();
    TAGS.iter().any(|tag| t.starts_with(tag))
}

/// Extract the inner text of the first `<tag>…</tag>` pair, if present.
fn tag_content<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(&text[start..end])
}

/// Drop ANSI escape sequences (CSI + two-byte ESC forms). Local-command
/// stdout is captured from a terminal-oriented code path and can carry bold/
/// color codes (e.g. `/model`'s "Set model to \x1b[1mOpus\x1b[22m").
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            // CSI: parameter/intermediate bytes end at a final byte in @..=~.
            for n in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&n) {
                    break;
                }
            }
        } else {
            chars.next(); // two-byte ESC sequence — drop the follower too
        }
    }
    out
}

/// Parse a transcript text blob carrying slash-command markers into items.
///
/// Two shapes, verified live (CLI 2.1.x, both PTY and stream transports):
/// - invocation echo: `<command-name>/foo</command-name>` with optional
///   `<command-message>` and `<command-args>` siblings, in either order —
///   written as a `user` row or (newer CLIs) a `system`/`local_command` row;
/// - local output: `<local-command-stdout>…</local-command-stdout>` (or
///   `-stderr`), same two row homes.
///
/// Returns an empty vec when the text has no command markers.
fn command_items_from_text(text: &str, ts: &Option<String>) -> Vec<ConversationItem> {
    let mut out = Vec::new();
    if let Some(name) = tag_content(text, "command-name") {
        let name = name.trim().trim_start_matches('/').to_string();
        if !name.is_empty() {
            let args = tag_content(text, "command-args")
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(str::to_owned);
            out.push(ConversationItem::SlashCommand {
                name,
                args,
                timestamp: ts.clone(),
            });
        }
    }
    for (tag, is_error) in [
        ("local-command-stdout", false),
        ("local-command-stderr", true),
    ] {
        if let Some(body) = tag_content(text, tag) {
            let output = strip_ansi(body).trim().to_string();
            if !output.is_empty() {
                out.push(ConversationItem::CommandOutput {
                    output,
                    is_error,
                    timestamp: ts.clone(),
                });
            }
        }
    }
    out
}

/// Extract the `Usage` item off an assistant transcript row, if it carries a
/// usage block. `sidechain` tags spend that belongs to a sub-agent's run.
fn usage_item_from_assistant_row(value: &Value, sidechain: bool) -> Option<ConversationItem> {
    let msg = value.get("message")?;
    let usage = msg.get("usage")?;
    Some(ConversationItem::Usage {
        model: msg.get("model").and_then(Value::as_str).map(str::to_owned),
        usage: usage.clone(),
        message_id: msg
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| value.get("uuid").and_then(Value::as_str).map(str::to_owned)),
        sidechain,
    })
}

/// Parse one *sub-agent* transcript row (from a `subagents/*.jsonl` file next
/// to the main transcript). Only the usage matters — the sub-agent's timeline
/// is surfaced through the agent cards, never the main conversation.
pub fn sidechain_usage_from_row(value: &Value) -> Option<ConversationItem> {
    if value.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    usage_item_from_assistant_row(value, true)
}

/// Who owns the agent-error marker for a session's api-error turns.
///
/// The marker is a per-turn failure signal and must have exactly ONE producer.
/// A managed/stream session's driver already emits one (providers/mod.rs folds
/// `AgentUpdate::Error` into a marked assistant turn from the CLI's `result`
/// frame), so a second one from the transcript row is not extra safety: the
/// desktop coalesces a stream session's assistant text into a single bubble, so
/// the two land as `⚠️ Error: <driver text>⚠️ Error: <row text>` whenever the
/// two sources word the failure differently.
///
/// A PTY session has no driver — a real terminal produces no AgentUpdate at all
/// — so there the transcript row is the ONLY signal and the tailer must mark it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ApiErrorMarking {
    /// Stamp the marker on `isApiErrorMessage` rows (PTY: nobody else will).
    Tailer,
    /// Leave them unmarked; this session's managed driver already emits one.
    Driver,
}

pub fn items_from_row(value: &Value, marking: ApiErrorMarking) -> Vec<ConversationItem> {
    let mut out = Vec::new();
    if value
        .get("isMeta")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return out;
    }
    // Sub-agent steps (Task tool / workflow agents) tagged `isSidechain: true`
    // belong to the sub-agent's own run — surfaced separately via the
    // subagent/workflow cards — not the main timeline. Without this, every
    // tool call a sub-agent makes (Bash, Read, …) floods the conversation as
    // orphaned rows under the spawning Agent card. Their token usage is real
    // spend though, so that alone still surfaces — as a usage-only item
    // tagged `sidechain` for the accounting path. (Current Claude Code writes
    // sub-agents to their own `subagents/*.jsonl` files, tailed separately —
    // this branch covers older transcripts that interleave them.)
    if value
        .get("isSidechain")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        if value.get("type").and_then(Value::as_str) == Some("assistant") {
            out.extend(usage_item_from_assistant_row(value, true));
        }
        return out;
    }
    let row_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    let ts = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_owned);
    // Newer CLIs write slash-command traffic as `system` rows (subtype
    // `local_command`) with the text in a top-level `content` string — no
    // `message` envelope, so this must run before the message requirement.
    if row_type == "system" {
        if value.get("subtype").and_then(Value::as_str) == Some("local_command") {
            if let Some(text) = value.get("content").and_then(Value::as_str) {
                out.extend(command_items_from_text(text, &ts));
            }
        }
        return out;
    }
    let Some(msg) = value.get("message") else {
        return out;
    };

    match row_type {
        "user" => {
            let content = msg.get("content").unwrap_or(&Value::Null);
            let bs = blocks(content);
            let has_tool_result = bs.iter().any(|b| matches!(b, Block::ToolResult { .. }));
            if has_tool_result {
                // tool_result rows are API plumbing, not user messages —
                // surface them as results joined to their tool calls.
                for b in &bs {
                    if let Block::ToolResult {
                        content,
                        is_error,
                        tool_use_id: Some(tid),
                    } = b
                    {
                        out.push(ConversationItem::ToolResult {
                            tool_use_id: (*tid).to_string(),
                            content: flatten_tool_result(content),
                            is_error: *is_error,
                            timestamp: ts.clone(),
                        });
                    }
                }
            } else {
                let mut texts: Vec<&str> = Vec::new();
                for b in &bs {
                    let Block::Text { text } = b else { continue };
                    // Slash-command traffic first (invocation echoes, local
                    // output) — surfaced as typed items, in timeline order.
                    let cmd = command_items_from_text(text, &ts);
                    if !cmd.is_empty() {
                        out.extend(cmd);
                        continue;
                    }
                    // Then drop the remaining injected, non-conversational
                    // blocks (system reminders, command caveats, our
                    // background-task notifications) — transcript plumbing,
                    // not something the user typed.
                    if !is_injected_meta(text) {
                        texts.push(*text);
                    }
                }
                let text = texts.join("\n");
                if !text.trim().is_empty() {
                    out.push(ConversationItem::UserMessage {
                        text,
                        timestamp: ts,
                    });
                }
            }
        }
        "assistant" => {
            out.extend(usage_item_from_assistant_row(value, false));
            // Claude Code CLI marks a turn it never actually got a model
            // response for — an API refusal (credit balance, rate limit, a
            // 5xx) — with a row-level `isApiErrorMessage: true` (confirmed
            // against the installed CLI/SDK bundle; not documented on the
            // JSONL format). This is the PTY transport's only such signal: a
            // real terminal session has no AgentUpdate::Error to carry the
            // shared marker, so without this the transcript tailer surfaced
            // the exact same text as an ordinary reply and workerFailure.ts's
            // marker check (which the stream/managed-provider path already
            // satisfies via providers/mod.rs) never fired for it.
            //
            // Only when this tailer OWNS the marker for the session — see
            // [`ApiErrorMarking`]. 87 of the 90 api-error rows in a real
            // ~/.claude/projects corpus (2026-09-03) carry `entrypoint:
            // "sdk-cli"`, i.e. they were written by stream-transport sessions
            // whose driver marks the same failure itself.
            let is_api_error = marking == ApiErrorMarking::Tailer
                && value
                    .get("isApiErrorMessage")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            for b in blocks(msg.get("content").unwrap_or(&Value::Null)) {
                match b {
                    Block::Text { text } if !text.trim().is_empty() => {
                        let text = if is_api_error {
                            format!("⚠️ Error: {}\n", text.trim())
                        } else {
                            text.trim().to_string()
                        };
                        out.push(ConversationItem::AssistantText {
                            text,
                            timestamp: ts.clone(),
                        });
                    }
                    Block::ToolUse { name, input, id } => {
                        out.push(ConversationItem::ToolUse {
                            id: id.map(str::to_owned).unwrap_or_default(),
                            name: name.to_string(),
                            input: input.clone(),
                            timestamp: ts.clone(),
                        });
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    out
}

/// Extract the agent's current plan from a transcript row, if it carries a
/// `TodoWrite` tool call. Claude Code writes the checklist as a `TodoWrite`
/// tool_use whose input is `{ todos: [{ content, status, activeForm }] }`; the
/// last such call in the row is the current plan (a single row rarely has more
/// than one). Skips meta / sidechain rows exactly like [`items_from_row`], so a
/// sub-agent's private todos don't clobber the main session's plan.
///
/// Returns `Some` for any row that rewrote the plan — including an empty list
/// (a cleared plan is a legitimate last-write). `None` for rows with no
/// `TodoWrite`.
pub fn plan_from_row(value: &Value) -> Option<Plan> {
    if value
        .get("isMeta")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    if value
        .get("isSidechain")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    if value.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let content = value.get("message")?.get("content")?.as_array()?;
    let ts = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_owned);
    // Last TodoWrite in the row wins (last-write-wins within the row too).
    let input = content
        .iter()
        .rev()
        .find(|b| {
            b.get("type").and_then(Value::as_str) == Some("tool_use")
                && b.get("name").and_then(Value::as_str) == Some("TodoWrite")
        })?
        .get("input")?;
    Some(plan_from_todos(input, ts))
}

/// Build a [`Plan`] from a `TodoWrite` tool input (`{ todos: [...] }`). Unknown
/// statuses map to `Pending` via [`PlanStatus::from_wire`]; a todo missing
/// `content` is skipped.
fn plan_from_todos(input: &Value, updated_at: Option<String>) -> Plan {
    let steps = input
        .get("todos")
        .and_then(Value::as_array)
        .map(|todos| {
            todos
                .iter()
                .filter_map(|t| {
                    let content = t.get("content").and_then(Value::as_str)?.to_string();
                    let status = t
                        .get("status")
                        .and_then(Value::as_str)
                        .map(PlanStatus::from_wire)
                        .unwrap_or(PlanStatus::Pending);
                    let active_form = t
                        .get("activeForm")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    Some(PlanStep {
                        content,
                        status,
                        active_form,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Plan { steps, updated_at }
}

// ── Task-tool plan folding ──────────────────────────────────────────────────

/// Folds Claude Code's incremental task tools into the same [`Plan`] that
/// `TodoWrite` rows produce, so task-tool sessions light up every plan surface
/// (status bar, fleet cards, chat) instead of leaving a stale checklist.
///
/// Creation is two-phase in the transcript: the assistant's `TaskCreate`
/// tool_use carries the subject, and the paired tool_result assigns the id
/// ("Task #3 created successfully: …") — so creates wait in `pending` until
/// their result lands (an `is_error` result drops them). `TaskUpdate` mutates
/// by `taskId`: `status` (with `"deleted"` removing the task), `subject`, and
/// `activeForm`; ownership/dependency edits don't change the visible list and
/// are ignored. Skips meta/sidechain rows like [`plan_from_row`].
#[derive(Debug, Clone, Default)]
pub struct TaskFold {
    /// Confirmed tasks, in creation order.
    tasks: Vec<TaskEntry>,
    /// `TaskCreate`s awaiting their tool_result, keyed by tool_use id.
    pending: std::collections::HashMap<String, TaskEntry>,
}

#[derive(Debug, Clone)]
struct TaskEntry {
    id: String,
    subject: String,
    active_form: Option<String>,
    status: PlanStatus,
}

impl TaskFold {
    pub fn to_plan(&self, updated_at: Option<String>) -> Plan {
        Plan {
            steps: self
                .tasks
                .iter()
                .map(|t| PlanStep {
                    content: t.subject.clone(),
                    status: t.status,
                    active_form: t.active_form.clone(),
                })
                .collect(),
            updated_at,
        }
    }

    /// Fold one transcript row. Returns true when the visible task list
    /// changed — i.e. when a fresh [`Plan`] should be emitted.
    pub fn fold_row(&mut self, value: &Value) -> bool {
        if value
            .get("isMeta")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || value
                .get("isSidechain")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            return false;
        }
        let Some(content) = value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
        else {
            return false;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("assistant") => self.fold_tool_uses(content),
            Some("user") => self.fold_tool_results(content),
            _ => false,
        }
    }

    fn fold_tool_uses(&mut self, content: &[Value]) -> bool {
        let mut changed = false;
        for b in content {
            if b.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            let input = b.get("input").unwrap_or(&Value::Null);
            match b.get("name").and_then(Value::as_str) {
                Some("TaskCreate") => {
                    let Some(tool_id) = b.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let subject = input
                        .get("subject")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or_else(|| {
                            input
                                .get("description")
                                .and_then(Value::as_str)
                                .and_then(|d| d.lines().next())
                                .map(str::to_owned)
                        })
                        .unwrap_or_default();
                    if subject.is_empty() {
                        continue;
                    }
                    self.pending.insert(
                        tool_id.to_owned(),
                        TaskEntry {
                            id: String::new(),
                            subject,
                            active_form: input
                                .get("activeForm")
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                            status: PlanStatus::Pending,
                        },
                    );
                }
                Some("TaskUpdate") => {
                    let id = match input.get("taskId") {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Number(n)) => n.to_string(),
                        _ => continue,
                    };
                    let Some(pos) = self.tasks.iter().position(|t| t.id == id) else {
                        continue;
                    };
                    let status = input.get("status").and_then(Value::as_str);
                    if matches!(status, Some("deleted") | Some("cancelled")) {
                        self.tasks.remove(pos);
                        changed = true;
                        continue;
                    }
                    let t = &mut self.tasks[pos];
                    if let Some(s) = status {
                        let next = PlanStatus::from_wire(s);
                        if t.status != next {
                            t.status = next;
                            changed = true;
                        }
                    }
                    if let Some(s) = input.get("subject").and_then(Value::as_str) {
                        if t.subject != s {
                            t.subject = s.to_owned();
                            changed = true;
                        }
                    }
                    if let Some(s) = input.get("activeForm").and_then(Value::as_str) {
                        if t.active_form.as_deref() != Some(s) {
                            t.active_form = Some(s.to_owned());
                            changed = true;
                        }
                    }
                }
                _ => {}
            }
        }
        changed
    }

    fn fold_tool_results(&mut self, content: &[Value]) -> bool {
        let mut changed = false;
        for b in content {
            if b.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let Some(tid) = b.get("tool_use_id").and_then(Value::as_str) else {
                continue;
            };
            let Some(mut entry) = self.pending.remove(tid) else {
                continue;
            };
            if b.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
                continue; // rejected/failed create — never became a task
            }
            let text = b
                .get("content")
                .map(flatten_tool_result)
                .unwrap_or_default();
            // Fall back to the tool_use id when the result text doesn't carry
            // the assigned id — the task still shows, later TaskUpdates by
            // numeric id just won't match it.
            entry.id = parse_created_task_id(&text).unwrap_or_else(|| tid.to_owned());
            if self.tasks.iter().any(|t| t.id == entry.id) {
                continue; // replayed result — don't duplicate
            }
            self.tasks.push(entry);
            changed = true;
        }
        changed
    }
}

/// Pull the assigned id out of a `TaskCreate` result
/// ("Task #3 created successfully: …").
fn parse_created_task_id(text: &str) -> Option<String> {
    let rest = &text[text.find("Task #")? + "Task #".len()..];
    let id: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn push_then_forget_reclaims_the_log() {
        let conv = ConversationStore::new();
        conv.push(
            "s1",
            vec![ConversationItem::AssistantText {
                text: "hi".into(),
                timestamp: None,
            }],
        );
        assert!(conv.snapshot("s1").is_some(), "log present after push");
        conv.forget("s1");
        assert!(conv.snapshot("s1").is_none(), "log reclaimed after forget");
        // The next push after forget starts a fresh log that resets late joiners.
        conv.push(
            "s1",
            vec![ConversationItem::AssistantText {
                text: "again".into(),
                timestamp: None,
            }],
        );
        assert_eq!(conv.snapshot("s1").map(|(seq, _)| seq), Some(1));
    }

    #[tokio::test]
    async fn a_transcript_outside_every_root_is_never_tailed() {
        // A forged hook can set `transcript_path` to any JSONL-shaped file on
        // disk. The tailer would otherwise stream it, line by line, out through
        // /conversation for as long as the session lives — so the containment
        // has to sit here too, not only on the one-shot /transcript read.
        let dir = std::env::temp_dir().join(format!(
            "wks-tail-outside-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("someone-elses.jsonl");
        std::fs::write(
            &file,
            serde_json::to_string(&json!({
                "type": "assistant",
                "message": { "role": "assistant", "content": "secret" }
            }))
            .unwrap()
                + "\n",
        )
        .unwrap();
        let path = file.to_string_lossy().into_owned();

        if path_is_allowed(&file) {
            // No home dir and no registered roots: nothing to confine to.
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }

        let store = SessionStore::new();
        let conv = ConversationStore::new();
        let err = tail_one(&store, &conv, "s1", &path)
            .await
            .expect_err("out-of-root transcript must be refused");
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(
            tail_subagents(&conv, "s1", &path).await.is_ok(),
            "the sidechain pass refuses quietly"
        );
        assert!(
            conv.snapshot("s1").is_none(),
            "not one item from a file outside every root"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn push_bounds_the_in_memory_log() {
        // Push well past the cap in one go; the log must retain exactly
        // MAX_CONVERSATION_ITEMS (newest), while seq still counts the true total.
        let conv = ConversationStore::new();
        let over = MAX_CONVERSATION_ITEMS + 250;
        let items: Vec<_> = (0..over)
            .map(|i| ConversationItem::AssistantText {
                text: format!("m{i}"),
                timestamp: None,
            })
            .collect();
        conv.push("s", items);

        let (seq, kept) = conv.snapshot("s").unwrap();
        assert_eq!(seq, over as u64, "seq counts every item ever pushed");
        assert_eq!(
            kept.len(),
            MAX_CONVERSATION_ITEMS,
            "in-memory log is bounded"
        );
        // The newest item is retained; the oldest were front-dropped.
        match kept.last().unwrap() {
            ConversationItem::AssistantText { text, .. } => {
                assert_eq!(text, &format!("m{}", over - 1), "newest kept");
            }
            other => panic!("expected AssistantText, got {other:?}"),
        }
        match kept.first().unwrap() {
            ConversationItem::AssistantText { text, .. } => {
                assert_eq!(
                    text,
                    &format!("m{}", over - MAX_CONVERSATION_ITEMS),
                    "oldest surviving item is exactly cap back from the newest"
                );
            }
            other => panic!("expected AssistantText, got {other:?}"),
        }
    }

    #[test]
    fn user_text_row_yields_user_message() {
        let row = json!({
            "type": "user",
            "timestamp": "2026-06-12T10:00:00Z",
            "message": { "role": "user", "content": "hello there" }
        });
        let items = items_from_row(&row, ApiErrorMarking::Tailer);
        assert_eq!(items.len(), 1);
        match &items[0] {
            ConversationItem::UserMessage { text, timestamp } => {
                assert_eq!(text, "hello there");
                assert_eq!(timestamp.as_deref(), Some("2026-06-12T10:00:00Z"));
            }
            other => panic!("expected UserMessage, got {other:?}"),
        }
    }

    #[test]
    fn injected_meta_user_rows_are_dropped() {
        // A workflow task-notification (and other injected blocks) is transcript
        // plumbing, not a user message — it must not surface as a UserMessage.
        for content in [
            "<task-notification>\n<status>completed</status>\n</task-notification>",
            "<system-reminder>be nice</system-reminder>",
            "<local-command-caveat>Caveat: generated while running local commands</local-command-caveat>",
        ] {
            let row = json!({
                "type": "user",
                "message": { "role": "user", "content": content }
            });
            assert!(
                items_from_row(&row, ApiErrorMarking::Tailer).is_empty(),
                "expected {content:?} to be filtered out"
            );
        }
    }

    #[test]
    fn slash_command_echo_becomes_command_item() {
        // The classic PTY shape: name/message/args triple, args populated.
        let row = json!({
            "type": "user",
            "timestamp": "2026-07-14T10:00:00Z",
            "message": { "role": "user", "content":
                "<command-name>/btw</command-name>\n            <command-message>btw</command-message>\n            <command-args>is this ready?</command-args>" }
        });
        let items = items_from_row(&row, ApiErrorMarking::Tailer);
        assert_eq!(items.len(), 1);
        match &items[0] {
            ConversationItem::SlashCommand {
                name,
                args,
                timestamp,
            } => {
                assert_eq!(name, "btw");
                assert_eq!(args.as_deref(), Some("is this ready?"));
                assert_eq!(timestamp.as_deref(), Some("2026-07-14T10:00:00Z"));
            }
            other => panic!("expected SlashCommand, got {other:?}"),
        }
    }

    #[test]
    fn slash_command_echo_reversed_order_no_args() {
        // The stream-transport custom-command shape: message before name, no
        // args tag at all.
        let row = json!({
            "type": "user",
            "message": { "role": "user", "content":
                "<command-message>pingtest</command-message>\n<command-name>/pingtest</command-name>" }
        });
        let items = items_from_row(&row, ApiErrorMarking::Tailer);
        assert_eq!(items.len(), 1);
        match &items[0] {
            ConversationItem::SlashCommand { name, args, .. } => {
                assert_eq!(name, "pingtest");
                assert!(args.is_none());
            }
            other => panic!("expected SlashCommand, got {other:?}"),
        }
    }

    #[test]
    fn local_command_stdout_user_row_becomes_output_item_ansi_stripped() {
        let row = json!({
            "type": "user",
            "message": { "role": "user", "content":
                "<local-command-stdout>Set model to \u{1b}[1mOpus 4.8\u{1b}[22m and saved</local-command-stdout>" }
        });
        let items = items_from_row(&row, ApiErrorMarking::Tailer);
        assert_eq!(items.len(), 1);
        match &items[0] {
            ConversationItem::CommandOutput {
                output, is_error, ..
            } => {
                assert_eq!(output, "Set model to Opus 4.8 and saved");
                assert!(!is_error);
            }
            other => panic!("expected CommandOutput, got {other:?}"),
        }
    }

    #[test]
    fn system_local_command_rows_parse_without_message_envelope() {
        // Newer CLIs write both the invocation echo and the output as
        // `system` rows with a top-level `content` string.
        let echo = json!({
            "type": "system",
            "subtype": "local_command",
            "content": "<command-name>/context</command-name>\n            <command-message>context</command-message>\n            <command-args></command-args>",
            "timestamp": "2026-07-14T10:00:01Z"
        });
        let items = items_from_row(&echo, ApiErrorMarking::Tailer);
        assert_eq!(items.len(), 1);
        assert!(matches!(
            &items[0],
            ConversationItem::SlashCommand { name, args, .. }
                if name == "context" && args.is_none()
        ));

        let stdout = json!({
            "type": "system",
            "subtype": "local_command",
            "content": "<local-command-stdout>## Context Usage\n\n**Tokens:** 24.5k</local-command-stdout>"
        });
        let items = items_from_row(&stdout, ApiErrorMarking::Tailer);
        assert_eq!(items.len(), 1);
        assert!(matches!(
            &items[0],
            ConversationItem::CommandOutput { output, is_error: false, .. }
                if output.starts_with("## Context Usage")
        ));

        // Other system subtypes stay ignored.
        let other = json!({ "type": "system", "subtype": "stop_hook_summary" });
        assert!(items_from_row(&other, ApiErrorMarking::Tailer).is_empty());
    }

    #[test]
    fn injected_meta_block_filtered_but_real_text_kept() {
        // A user row carrying both a reminder block and real text keeps only the
        // real text.
        let row = json!({
            "type": "user",
            "message": { "role": "user", "content": [
                { "type": "text", "text": "<system-reminder>noise</system-reminder>" },
                { "type": "text", "text": "actually do this" }
            ]}
        });
        let items = items_from_row(&row, ApiErrorMarking::Tailer);
        assert_eq!(items.len(), 1);
        match &items[0] {
            ConversationItem::UserMessage { text, .. } => assert_eq!(text, "actually do this"),
            other => panic!("expected UserMessage, got {other:?}"),
        }
    }

    #[test]
    fn tool_result_rows_are_results_not_user_messages() {
        let row = json!({
            "type": "user",
            "message": { "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "tu_1", "content": "42 lines", "is_error": false }
            ]}
        });
        let items = items_from_row(&row, ApiErrorMarking::Tailer);
        assert_eq!(items.len(), 1);
        match &items[0] {
            ConversationItem::ToolResult {
                tool_use_id,
                content,
                is_error,
                ..
            } => {
                assert_eq!(tool_use_id, "tu_1");
                assert_eq!(content, "42 lines");
                assert!(!is_error);
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn assistant_row_interlaces_usage_text_and_tool_use() {
        let row = json!({
            "type": "assistant",
            "timestamp": "2026-06-12T10:00:01Z",
            "message": {
                "role": "assistant",
                "id": "msg_1",
                "model": "claude-fable-5",
                "usage": { "input_tokens": 10, "output_tokens": 5 },
                "content": [
                    { "type": "thinking", "thinking": "hmm" },
                    { "type": "text", "text": "I'll read the file." },
                    { "type": "tool_use", "id": "tu_2", "name": "Read", "input": { "file_path": "/a.rs" } }
                ]
            }
        });
        let items = items_from_row(&row, ApiErrorMarking::Tailer);
        assert_eq!(items.len(), 3, "usage + text + tool_use (thinking skipped)");
        assert!(
            matches!(&items[0], ConversationItem::Usage { model: Some(m), message_id: Some(id), .. } if m == "claude-fable-5" && id == "msg_1")
        );
        assert!(
            matches!(&items[1], ConversationItem::AssistantText { text, .. } if text == "I'll read the file.")
        );
        assert!(
            matches!(&items[2], ConversationItem::ToolUse { id, name, .. } if id == "tu_2" && name == "Read")
        );
    }

    #[test]
    fn api_error_row_gets_the_shared_agent_error_marker() {
        // PTY transport: no AgentUpdate::Error exists (that's the stream/
        // managed-provider path in providers/mod.rs), so the transcript row's
        // own `isApiErrorMessage` is the only signal. Without marking it, this
        // is indistinguishable from an ordinary reply to workerFailure.ts.
        let row = json!({
            "type": "assistant",
            "isApiErrorMessage": true,
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": "Credit balance is too low." }]
            }
        });
        let items = items_from_row(&row, ApiErrorMarking::Tailer);
        assert_eq!(items.len(), 1);
        match &items[0] {
            ConversationItem::AssistantText { text, .. } => {
                assert_eq!(text, "⚠️ Error: Credit balance is too low.\n");
            }
            other => panic!("expected AssistantText, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_tailer_marks_api_errors_only_where_no_driver_does() {
        // ITEM 5 of the credit-balance review: `spawn_tailer` tails ANY session
        // with a transcript path, so marking api-error rows unconditionally made
        // the tailer a SECOND producer of the agent-error marker on managed
        // sessions, whose driver already emits one (providers/mod.rs folds
        // `AgentUpdate::Error` into a marked assistant turn). The desktop
        // coalesces a stream session's assistant text into one bubble, so the
        // two land concatenated whenever the driver's wording and the row's
        // differ. 87 of the 90 api-error rows in a real ~/.claude/projects
        // corpus (2026-09-03) carry `entrypoint: "sdk-cli"` — the stream
        // transport — so that was the common case, not the rare one.
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!(
            "claudemon-apierror-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        crate::session::transcript::allow_root(&dir);
        let row = r#"{"type":"assistant","isApiErrorMessage":true,"message":{"role":"assistant","content":[{"type":"text","text":"You're out of usage credits."}]}}"#;

        let store = SessionStore::new();
        let conv = ConversationStore::new();

        // PTY: a real terminal produces no AgentUpdate at all, so the row is
        // the only failure signal that exists and the tailer must mark it.
        let pty = dir.join("pty.jsonl");
        {
            let mut f = std::fs::File::create(&pty).unwrap();
            writeln!(f, "{row}").unwrap();
        }
        store.register_managed("s-pty", &dir.to_string_lossy(), "claude");
        store.set_transport("s-pty", Transport::Pty);
        tail_one(&store, &conv, "s-pty", &pty.to_string_lossy())
            .await
            .unwrap();
        let (_seq, items) = conv.snapshot("s-pty").unwrap();
        assert_eq!(items.len(), 1, "one assistant turn, marked: {items:?}");
        match &items[0] {
            ConversationItem::AssistantText { text, .. } => {
                assert_eq!(text, "⚠️ Error: You're out of usage credits.\n");
            }
            other => panic!("expected AssistantText, got {other:?}"),
        }

        // STREAM: the driver already emitted its own marked turn for this same
        // failure (pinned by providers/mod.rs's
        // error_marker_matches_the_cross_language_contract). The tailer emits
        // the row's text plainly, exactly as it did before the marking landed.
        let stream = dir.join("stream.jsonl");
        {
            let mut f = std::fs::File::create(&stream).unwrap();
            writeln!(f, "{row}").unwrap();
        }
        store.register_managed("s-stream", &dir.to_string_lossy(), "claude");
        store.set_transport("s-stream", Transport::Stream);
        tail_one(&store, &conv, "s-stream", &stream.to_string_lossy())
            .await
            .unwrap();
        let (_seq, items) = conv.snapshot("s-stream").unwrap();
        assert_eq!(items.len(), 1, "one assistant turn, unmarked: {items:?}");
        match &items[0] {
            ConversationItem::AssistantText { text, .. } => {
                assert_eq!(
                    text, "You're out of usage credits.",
                    "the tailer must not stamp a marker the driver already stamped"
                );
            }
            other => panic!("expected AssistantText, got {other:?}"),
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ordinary_assistant_text_is_unmarked() {
        let row = json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [{ "type": "text", "text": "All done." }]
            }
        });
        let items = items_from_row(&row, ApiErrorMarking::Tailer);
        match &items[0] {
            ConversationItem::AssistantText { text, .. } => assert_eq!(text, "All done."),
            other => panic!("expected AssistantText, got {other:?}"),
        }
    }

    #[test]
    fn sidechain_rows_are_skipped() {
        // A sub-agent's tool_use (Task tool / workflow agent), tagged
        // isSidechain, must not flatten into the main conversation — those rows
        // belong to the sub-agent's own run, surfaced via the agent cards.
        let tool_use = json!({
            "type": "assistant",
            "isSidechain": true,
            "message": {
                "role": "assistant",
                "content": [
                    { "type": "tool_use", "id": "tu_x", "name": "Bash", "input": { "command": "ls" } }
                ]
            }
        });
        assert!(
            items_from_row(&tool_use, ApiErrorMarking::Tailer).is_empty(),
            "sidechain tool_use must be dropped"
        );

        let text = json!({
            "type": "assistant",
            "isSidechain": true,
            "message": { "role": "assistant", "content": [{ "type": "text", "text": "subagent thinking" }] }
        });
        assert!(
            items_from_row(&text, ApiErrorMarking::Tailer).is_empty(),
            "sidechain text must be dropped"
        );

        // The spawning Agent tool_use lives in the MAIN (non-sidechain) turn and
        // must still surface so its card anchors.
        let main_agent = json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "content": [
                    { "type": "tool_use", "id": "tu_a", "name": "Agent", "input": { "description": "explore" } }
                ]
            }
        });
        let items = items_from_row(&main_agent, ApiErrorMarking::Tailer);
        assert_eq!(items.len(), 1);
        assert!(matches!(&items[0], ConversationItem::ToolUse { name, .. } if name == "Agent"));
    }

    #[test]
    fn sidechain_usage_surfaces_as_tagged_usage_item() {
        // A sub-agent's assistant row stays off the timeline, but its token
        // usage is real spend — it must surface as a usage-only item tagged
        // `sidechain: true` (and nothing else: no text, no tool_use).
        let row = json!({
            "type": "assistant",
            "isSidechain": true,
            "message": {
                "role": "assistant",
                "id": "msg_sub1",
                "model": "claude-haiku-4-5",
                "usage": { "input_tokens": 10, "output_tokens": 20 },
                "content": [
                    { "type": "text", "text": "subagent narration" },
                    { "type": "tool_use", "id": "tu_x", "name": "Bash", "input": { "command": "ls" } }
                ]
            }
        });
        let items = items_from_row(&row, ApiErrorMarking::Tailer);
        assert_eq!(items.len(), 1, "only the usage item, no timeline items");
        assert!(matches!(
            &items[0],
            ConversationItem::Usage { model: Some(m), message_id: Some(id), sidechain: true, .. }
                if m == "claude-haiku-4-5" && id == "msg_sub1"
        ));

        // Main-thread usage keeps sidechain: false (and serializes without the
        // field at all — legacy clients see the exact old wire shape).
        let main = json!({
            "type": "assistant",
            "message": {
                "role": "assistant",
                "id": "msg_main",
                "model": "claude-fable-5",
                "usage": { "input_tokens": 5 },
                "content": []
            }
        });
        let items = items_from_row(&main, ApiErrorMarking::Tailer);
        assert!(matches!(
            &items[0],
            ConversationItem::Usage {
                sidechain: false,
                ..
            }
        ));
        let wire = serde_json::to_value(&items[0]).unwrap();
        assert!(
            wire.get("sidechain").is_none(),
            "sidechain: false must be omitted from the wire"
        );
    }

    #[test]
    fn meta_and_summary_rows_are_skipped() {
        assert!(items_from_row(
            &json!({ "type": "user", "isMeta": true, "message": { "content": "x" } }),
            ApiErrorMarking::Tailer
        )
        .is_empty());
        assert!(items_from_row(
            &json!({ "type": "summary", "summary": "..." }),
            ApiErrorMarking::Tailer
        )
        .is_empty());
    }

    #[test]
    fn todowrite_row_yields_plan_with_status_mapping() {
        let row = json!({
            "type": "assistant",
            "timestamp": "2026-07-04T10:00:00Z",
            "message": { "role": "assistant", "content": [
                { "type": "tool_use", "id": "tu_1", "name": "TodoWrite", "input": { "todos": [
                    { "content": "read code", "status": "completed", "activeForm": "Reading code" },
                    { "content": "write code", "status": "in_progress", "activeForm": "Writing code" },
                    { "content": "test", "status": "pending" },
                    { "content": "ship", "status": "bogus-status" }
                ]}}
            ]}
        });
        let plan = plan_from_row(&row).expect("TodoWrite row yields a plan");
        assert_eq!(plan.updated_at.as_deref(), Some("2026-07-04T10:00:00Z"));
        assert_eq!(plan.steps.len(), 4);
        assert_eq!(plan.steps[0].status, PlanStatus::Completed);
        assert_eq!(plan.steps[0].active_form.as_deref(), Some("Reading code"));
        assert_eq!(plan.steps[1].status, PlanStatus::InProgress);
        assert_eq!(plan.steps[2].status, PlanStatus::Pending);
        assert_eq!(plan.steps[2].active_form, None);
        // Unknown status maps defensively to Pending.
        assert_eq!(plan.steps[3].status, PlanStatus::Pending);
    }

    #[test]
    fn non_todowrite_and_sidechain_rows_yield_no_plan() {
        // A plain assistant row (no TodoWrite) has no plan.
        assert!(plan_from_row(&json!({
            "type": "assistant",
            "message": { "role": "assistant", "content": [{ "type": "text", "text": "hi" }] }
        }))
        .is_none());
        // A sub-agent's TodoWrite (isSidechain) must not clobber the main plan.
        assert!(plan_from_row(&json!({
            "type": "assistant",
            "isSidechain": true,
            "message": { "role": "assistant", "content": [
                { "type": "tool_use", "id": "t", "name": "TodoWrite", "input": { "todos": [
                    { "content": "sub task", "status": "pending" }
                ]}}
            ]}
        }))
        .is_none());
    }

    #[test]
    fn todowrite_empty_todos_is_a_cleared_plan() {
        // An empty list is a legitimate last-write (the plan was cleared), not a
        // "no plan" signal — it must still produce a Plan.
        let plan = plan_from_row(&json!({
            "type": "assistant",
            "message": { "role": "assistant", "content": [
                { "type": "tool_use", "id": "t", "name": "TodoWrite", "input": { "todos": [] }}
            ]}
        }))
        .expect("empty TodoWrite is a cleared plan");
        assert!(plan.steps.is_empty());
    }

    #[tokio::test]
    async fn tail_extracts_and_replaces_plan_from_todowrite_rows() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("claudemon-plan-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // The tailer only reads inside known transcript roots (see
        // transcript::path_is_allowed); a fixture dir has to be declared one the
        // way a spawn's CLAUDE_CONFIG_DIR would be.
        crate::session::transcript::allow_root(&dir);
        let path = dir.join("p.jsonl");
        let path_str = path.to_string_lossy().to_string();

        let store = SessionStore::new();
        store.register_managed("s-plan", &dir.to_string_lossy(), "claude");
        let conv = ConversationStore::new();

        // First TodoWrite: one in-progress step.
        let r1 = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"TodoWrite","input":{"todos":[{"content":"step one","status":"in_progress"}]}}]}}"#;
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(f, "{r1}").unwrap();
        }
        tail_one(&store, &conv, "s-plan", &path_str).await.unwrap();
        let plan = store.get("s-plan").unwrap().plan.expect("plan recorded");
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].status, PlanStatus::InProgress);
        // A `plan` conversation item was pushed alongside the tool_use.
        let (_seq, items) = conv.snapshot("s-plan").unwrap();
        assert!(items
            .iter()
            .any(|i| matches!(i, ConversationItem::Plan { steps, .. } if steps.len() == 1)));

        // Second TodoWrite fully replaces the first (last-write-wins).
        let r2 = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"TodoWrite","input":{"todos":[{"content":"step one","status":"completed"},{"content":"step two","status":"pending"}]}}]}}"#;
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(f, "{r2}").unwrap();
        }
        tail_one(&store, &conv, "s-plan", &path_str).await.unwrap();
        let plan = store.get("s-plan").unwrap().plan.expect("plan replaced");
        assert_eq!(plan.steps.len(), 2, "plan fully replaced, not merged");
        assert_eq!(plan.steps[0].status, PlanStatus::Completed);

        std::fs::remove_dir_all(&dir).ok();
    }

    // --- Task-tool folding (TaskCreate / TaskUpdate) ---

    fn create_row(tool_id: &str, subject: &str) -> Value {
        json!({"type":"assistant","message":{"role":"assistant","content":[
            {"type":"tool_use","id":tool_id,"name":"TaskCreate",
             "input":{"subject":subject,"description":"details","activeForm":"Working"}}]}})
    }

    fn create_result_row(tool_id: &str, text: &str, is_error: bool) -> Value {
        json!({"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":tool_id,"is_error":is_error,"content":text}]}})
    }

    fn update_row(task_id: &str, patch: Value) -> Value {
        let mut input = patch;
        input["taskId"] = json!(task_id);
        json!({"type":"assistant","message":{"role":"assistant","content":[
            {"type":"tool_use","id":"u1","name":"TaskUpdate","input":input}]}})
    }

    #[test]
    fn task_create_confirms_only_on_result() {
        let mut fold = TaskFold::default();
        assert!(
            !fold.fold_row(&create_row("t1", "Ship it")),
            "create alone is not a visible change"
        );
        assert!(fold.fold_row(&create_result_row(
            "t1",
            "Task #1 created successfully: Ship it",
            false
        )));
        let plan = fold.to_plan(None);
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].content, "Ship it");
        assert_eq!(plan.steps[0].status, PlanStatus::Pending);
        assert_eq!(plan.steps[0].active_form.as_deref(), Some("Working"));
    }

    #[test]
    fn task_create_error_result_never_becomes_a_task() {
        let mut fold = TaskFold::default();
        fold.fold_row(&create_row("t1", "Nope"));
        assert!(!fold.fold_row(&create_result_row("t1", "denied", true)));
        assert!(fold.to_plan(None).steps.is_empty());
    }

    #[test]
    fn task_update_status_subject_and_delete() {
        let mut fold = TaskFold::default();
        fold.fold_row(&create_row("t1", "One"));
        fold.fold_row(&create_result_row(
            "t1",
            "Task #1 created successfully: One",
            false,
        ));
        fold.fold_row(&create_row("t2", "Two"));
        fold.fold_row(&create_result_row(
            "t2",
            "Task #2 created successfully: Two",
            false,
        ));

        assert!(fold.fold_row(&update_row("1", json!({"status":"in_progress"}))));
        assert_eq!(fold.to_plan(None).steps[0].status, PlanStatus::InProgress);
        assert!(
            !fold.fold_row(&update_row("1", json!({"status":"in_progress"}))),
            "no-op update emits no plan"
        );
        assert!(fold.fold_row(&update_row("2", json!({"subject":"Two renamed"}))));
        assert_eq!(fold.to_plan(None).steps[1].content, "Two renamed");
        assert!(fold.fold_row(&update_row("1", json!({"status":"deleted"}))));
        let plan = fold.to_plan(None);
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].content, "Two renamed");
    }

    #[test]
    fn task_update_unknown_id_and_sidechain_rows_are_ignored() {
        let mut fold = TaskFold::default();
        assert!(!fold.fold_row(&update_row("9", json!({"status":"completed"}))));
        let mut side = create_row("t1", "Private");
        side["isSidechain"] = json!(true);
        assert!(!fold.fold_row(&side));
        assert!(!fold.fold_row(&create_result_row(
            "t1",
            "Task #1 created successfully: Private",
            false
        )));
        assert!(fold.to_plan(None).steps.is_empty());
    }

    #[test]
    fn parse_created_task_id_shapes() {
        assert_eq!(
            parse_created_task_id("Task #12 created successfully: X"),
            Some("12".to_string())
        );
        assert_eq!(parse_created_task_id("created"), None);
    }

    #[tokio::test]
    async fn tail_folds_task_tools_into_plan() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("claudemon-tasks-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Declared as a transcript root, as above — the tailer reads nowhere else.
        crate::session::transcript::allow_root(&dir);
        let path = dir.join("t.jsonl");
        let path_str = path.to_string_lossy().to_string();

        let store = SessionStore::new();
        store.register_managed("s-tasks", &dir.to_string_lossy(), "claude");
        let conv = ConversationStore::new();

        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(f, "{}", create_row("t1", "Build the thing")).unwrap();
            writeln!(
                f,
                "{}",
                create_result_row("t1", "Task #1 created successfully: Build the thing", false)
            )
            .unwrap();
        }
        tail_one(&store, &conv, "s-tasks", &path_str).await.unwrap();
        let plan = store.get("s-tasks").unwrap().plan.expect("plan from tasks");
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].status, PlanStatus::Pending);

        // The fold state carries across batches: a later status update mutates
        // the task created in the previous read.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(f, "{}", update_row("1", json!({"status":"completed"}))).unwrap();
        }
        tail_one(&store, &conv, "s-tasks", &path_str).await.unwrap();
        let plan = store.get("s-tasks").unwrap().plan.expect("plan updated");
        assert_eq!(plan.steps[0].status, PlanStatus::Completed);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn tail_picks_up_appends_and_carries_partial_lines() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("claudemon-tail-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Declared as a transcript root, as above — the tailer reads nowhere else.
        crate::session::transcript::allow_root(&dir);
        let path = dir.join("t.jsonl");
        let path_str = path.to_string_lossy().to_string();

        let store = SessionStore::new();
        let conv = ConversationStore::new();
        let mut rx = conv.subscribe();

        // First write: one complete row + the head of a second row.
        let row1 = r#"{"type":"user","message":{"role":"user","content":"first"}}"#;
        let row2 = r#"{"type":"user","message":{"role":"user","content":"second"}}"#;
        {
            let mut f = std::fs::File::create(&path).unwrap();
            write!(f, "{row1}\n{}", &row2[..20]).unwrap();
        }
        tail_one(&store, &conv, "s1", &path_str).await.unwrap();
        let d1 = rx.try_recv().expect("first delta");
        assert!(d1.reset);
        assert_eq!(d1.seq, 1);
        assert_eq!(d1.items.len(), 1, "partial second row must not parse yet");

        // Second write: the rest of row 2.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(f, "{}", &row2[20..]).unwrap();
        }
        tail_one(&store, &conv, "s1", &path_str).await.unwrap();
        let d2 = rx.try_recv().expect("second delta");
        assert!(!d2.reset);
        assert_eq!(d2.seq, 2);
        assert!(
            matches!(&d2.items[0], ConversationItem::UserMessage { text, .. } if text == "second")
        );

        // Snapshot reflects the whole log.
        let (seq, items) = conv.snapshot("s1").unwrap();
        assert_eq!(seq, 2);
        assert_eq!(items.len(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Stream-transport providers push one item per token chunk. Left
    /// un-coalesced, MAX_CONVERSATION_ITEMS — which is sized in *messages* —
    /// starts evicting the front of the conversation partway through a single
    /// long reply, so a client resyncing from `/conversation` adopts a history
    /// whose beginning is already gone.
    #[test]
    fn streamed_assistant_text_folds_into_one_item() {
        let conv = ConversationStore::new();
        for chunk in ["Hel", "lo, ", "world"] {
            conv.push(
                "s1",
                vec![ConversationItem::AssistantText {
                    text: chunk.into(),
                    timestamp: None,
                }],
            );
        }
        let (seq, items) = conv.snapshot("s1").expect("log present");
        assert_eq!(items.len(), 1, "three deltas fold into one message");
        match &items[0] {
            ConversationItem::AssistantText { text, .. } => assert_eq!(text, "Hello, world"),
            other => panic!("expected assistant text, got {other:?}"),
        }
        // seq still counts every delta, so gap detection and resync are intact.
        assert_eq!(seq, 3);
    }

    #[test]
    fn a_tool_use_between_chunks_ends_the_run() {
        let conv = ConversationStore::new();
        conv.push(
            "s1",
            vec![ConversationItem::AssistantText {
                text: "before".into(),
                timestamp: None,
            }],
        );
        conv.push(
            "s1",
            vec![ConversationItem::ToolUse {
                id: "t1".into(),
                name: "Bash".into(),
                input: json!({}),
                timestamp: None,
            }],
        );
        conv.push(
            "s1",
            vec![ConversationItem::AssistantText {
                text: "after".into(),
                timestamp: None,
            }],
        );
        let (_, items) = conv.snapshot("s1").expect("log present");
        assert_eq!(items.len(), 3, "the tool call is a turn boundary");
    }

    #[test]
    fn a_user_message_is_never_folded_into_assistant_text() {
        let conv = ConversationStore::new();
        conv.push(
            "s1",
            vec![ConversationItem::AssistantText {
                text: "reply".into(),
                timestamp: None,
            }],
        );
        conv.push(
            "s1",
            vec![ConversationItem::UserMessage {
                text: "next question".into(),
                timestamp: None,
            }],
        );
        let (_, items) = conv.snapshot("s1").expect("log present");
        assert_eq!(items.len(), 2);
        assert!(matches!(items[1], ConversationItem::UserMessage { .. }));
    }

    /// A batch is a real multi-item message, not a token chunk — folding it
    /// would merge distinct entries.
    #[test]
    fn a_multi_item_batch_is_appended_whole() {
        let conv = ConversationStore::new();
        conv.push(
            "s1",
            vec![ConversationItem::AssistantText {
                text: "one".into(),
                timestamp: None,
            }],
        );
        conv.push(
            "s1",
            vec![
                ConversationItem::AssistantText {
                    text: "two".into(),
                    timestamp: None,
                },
                ConversationItem::AssistantText {
                    text: "three".into(),
                    timestamp: None,
                },
            ],
        );
        let (_, items) = conv.snapshot("s1").expect("log present");
        assert_eq!(items.len(), 3);
    }

    /// Folding must not cost live clients their token-by-token stream.
    #[test]
    fn each_chunk_is_still_broadcast_as_its_own_delta() {
        let conv = ConversationStore::new();
        let mut rx = conv.subscribe();
        for chunk in ["a", "b"] {
            conv.push(
                "s1",
                vec![ConversationItem::AssistantText {
                    text: chunk.into(),
                    timestamp: None,
                }],
            );
        }
        let first = rx.try_recv().expect("first delta");
        assert_eq!(first.items.len(), 1);
        assert!(first.reset, "the first push resets late joiners");
        let second = rx.try_recv().expect("second delta");
        assert_eq!(second.items.len(), 1);
        assert!(!second.reset);
        assert_eq!(second.seq, 2);
    }

    /// The invariant every consumer of a snapshot depends on: with coalescing,
    /// `seq` and `items.len()` diverge, so the first item's sequence must come
    /// from the store rather than be reconstructed as `seq - len + 1`.
    ///
    /// The divergence needs an item BEFORE the coalesced run — which is the
    /// ordinary shape of a turn (the user's message, then the streamed reply).
    #[test]
    fn first_seq_is_not_the_reconstruction_once_text_coalesces() {
        let conv = ConversationStore::new();
        conv.push(
            "s1",
            vec![ConversationItem::UserMessage {
                text: "go".into(),
                timestamp: None,
            }],
        );
        // The reply streams in as 50 token fragments that fold into one item.
        for i in 0..50 {
            conv.push(
                "s1",
                vec![ConversationItem::AssistantText {
                    text: format!("chunk{i}"),
                    timestamp: None,
                }],
            );
        }

        let (seq, first_seq, items) = conv.snapshot_windowed("s1").expect("snapshot");
        assert_eq!(items.len(), 2, "one user message + one folded reply");
        assert_eq!(seq, 51, "seq counted every fragment");
        assert_eq!(first_seq, 1, "the user message is still item 0, at seq 1");

        let reconstructed = seq - items.len() as u64 + 1;
        assert_eq!(reconstructed, 50);
        assert_ne!(
            first_seq, reconstructed,
            "reconstructing from the item count lands 49 sequences too far right"
        );

        // In user terms: a client polling from seq 1 already has the user
        // message and wants only the reply. Skipping by the real first_seq
        // drops exactly one item; the reconstruction would have dropped none
        // and re-sent the whole conversation.
        let skip_real = (1u64 + 1).saturating_sub(first_seq) as usize;
        let skip_wrong = (1u64 + 1).saturating_sub(reconstructed) as usize;
        assert_eq!(skip_real, 1);
        assert_eq!(skip_wrong, 0, "the old formula re-sent everything");
    }

    /// A reset restarts the sequence counter, so the retained per-item sequences
    /// must go with it.
    ///
    /// The invariant is `item_seqs.len() == items.len()` — they are parallel, and
    /// `item_seqs[i]` describes `items[i]`. Leaving the old sequences behind
    /// breaks that correspondence permanently: `extend_bounded` drains equal
    /// counts from both, so the stale prefix shifts every later item's label by
    /// however many entries were stranded, and `first_seq()` ends up describing
    /// an item that no longer exists.
    #[test]
    fn a_reset_clears_the_item_sequences_with_the_items() {
        let conv = ConversationStore::new();
        let lens = |c: &ConversationStore| -> (usize, usize) {
            let log = c.logs.get("s1").expect("log");
            (log.items.len(), log.item_seqs.len())
        };

        for i in 0..5 {
            conv.push(
                "s1",
                vec![ConversationItem::UserMessage {
                    text: format!("m{i}"),
                    timestamp: None,
                }],
            );
        }
        assert_eq!(lens(&conv), (5, 5), "parallel before the reset");

        // A resume re-tails from a new transcript path and resets the log.
        conv.logs.get_mut("s1").expect("log").reset_log();
        assert_eq!(lens(&conv), (0, 0), "and empty straight after it");

        conv.push(
            "s1",
            vec![ConversationItem::UserMessage {
                text: "after".into(),
                timestamp: None,
            }],
        );

        let (items, seqs) = lens(&conv);
        assert_eq!(
            items, seqs,
            "item_seqs[i] must still describe items[i] — a stranded prefix shifts \
             every later label and never heals"
        );
        let (seq, first_seq, _) = conv.snapshot_windowed("s1").unwrap();
        assert_eq!((seq, first_seq), (1, 1), "the new life starts from one");
    }
}
