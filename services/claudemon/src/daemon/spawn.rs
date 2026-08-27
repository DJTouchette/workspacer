//! `POST /sessions/spawn` — spawn a command in a PTY *inside* the daemon.
//!
//! The daemon pre-assigns a UUID, registers it as a "pending spawn" keyed by
//! cwd, and wires up the PTY's reader/writer to the same per-session output
//! buffer + bytes broadcast that `wrapper_ws` uses for external wrappers.
//!
//! When claude later posts `SessionStart` with its own session_id, the store
//! aliases that id to ours (see `SessionStore::ingest`), so every endpoint
//! that takes a session_id keeps working — clients only ever see the id we
//! handed back from this endpoint.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use portable_pty::PtySize;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::protocol::WrapperMessage;
use crate::session::store::WrapperHandle;
use crate::session::{ConversationStore, SessionStore};
use crate::wrapper::pty;

#[derive(Debug, Deserialize)]
pub struct SpawnPayload {
    /// Command + args, e.g. `["claude", "--resume", "..."]`.
    pub argv: Vec<String>,
    /// Working directory for the child.
    pub cwd: String,
    /// PTY dimensions. Defaults to 80x24 if omitted.
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    /// Extra env vars merged on top of the daemon's environment.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Caller-pinned session id. When the caller launches
    /// `claude --session-id <uuid>` it passes the same uuid here, so our id,
    /// claude's id, and the transcript filename (`<uuid>.jsonl`) all agree — no
    /// cwd-based alias guessing, correct even with many sessions in one cwd.
    #[serde(default)]
    pub session_id: Option<String>,
    /// When set (e.g. `"codex"`), this PTY is a managed agent's own TUI and we
    /// additionally tail its rollout transcript to drive the GUI conversation
    /// view — a "hybrid" session that has both a terminal and a structured GUI.
    #[serde(default)]
    pub rollout_provider: Option<String>,
    /// The agent's FIRST PROMPT, delivered as part of the spawn rather than by
    /// a second call after the id comes back. See
    /// [`SessionStore::queue_first_message`] for why the two-call form races.
    #[serde(default)]
    pub first_message: Option<String>,
    /// The model string this session was ASKED for, when the caller knows it.
    ///
    /// Preferred over sniffing `--model` off [`Self::argv`], because argv only
    /// carries it when someone spelled it out: a RESUME re-uses the prior
    /// life's model without re-stating it, and a spawn whose model came from
    /// config used to reach the CLI as its own internal default and never
    /// appear on the command line at all. Those are precisely the sessions with
    /// the most history to mis-measure.
    ///
    /// It matters because Claude Code STRIPS the `[1m]` marker from the model
    /// id it writes into the transcript, so this string is the only carrier of
    /// a 1M choice until the provider reports a window of its own — seconds
    /// away on the PTY path, a whole turn away on the stream one.
    #[serde(default)]
    pub model: Option<String>,
}

/// The value of a `--model` flag in a spawn's argv, in either spelling
/// (`--model x` / `--model=x`). The FALLBACK behind [`SpawnPayload::model`],
/// for callers that only send a command line: the model is exactly what says
/// whether this session runs a 1M window (`opus[1m]`), so read it off argv
/// rather than lose it.
fn model_from_argv(argv: &[String]) -> Option<&str> {
    let mut it = argv.iter();
    while let Some(arg) = it.next() {
        if let Some(v) = arg.strip_prefix("--model=") {
            return Some(v);
        }
        if arg == "--model" {
            return it.next().map(String::as_str);
        }
    }
    None
}

/// Reject a working directory no child could actually start in — the daemon-side
/// backstop under the desktop's own pre-flight (`main/lib/spawnCwd.ts`).
///
/// Neither route notices this on its own, and each fails to notice differently:
///
///   - `handle_managed` registers the session id and answers 200 while the
///     adapter boots in a background task, so a launch that never happens
///     surfaces as a `warn` line and a row that is already `Stopped`.
///   - `handle` LOOKS synchronous, but `spawn_command` forks — the chdir runs in
///     the CHILD, so the parent takes an `Ok` handle to a process that is already
///     dead, answers 200, and the row is gone by the time the caller asks for it.
///
/// Either way the caller is handed a session id for an agent that never existed,
/// and every message to it comes back `409 session has ended` (or a 404). The
/// desktop pre-flights this now, but it is not the only client: the brain, the
/// TUI, and any MCP-facade dispatch reach these routes directly.
///
/// `~` is the spelling that actually arrives. BINDING DECISION 1 means no layer
/// between a config field and this one expands it (`normalizeSpawnCwd` and the
/// brain's `normalizeCwd` both pass it through verbatim and are pinned that way
/// by contracts/path-containment-cases.json), so a fleet root a person typed as
/// `~/` reaches here as a directory literally named `~`. This function does not
/// expand it either — the rule stands; it just refuses it where the caller can
/// still be told why.
fn cwd_problem(cwd: &str) -> Option<String> {
    let ok = std::fs::metadata(cwd).map(|m| m.is_dir()).unwrap_or(false);
    if ok {
        return None;
    }
    // Named explicitly: "~" looks like a valid path to whoever typed it, so the
    // message has to say why it is not, rather than only that it is not.
    let hint = if cwd.starts_with('~') {
        " (a leading \"~\" is not expanded here — send an absolute path)"
    } else {
        ""
    };
    Some(format!("cwd is not an existing directory: {cwd}{hint}"))
}

pub async fn handle(
    State(store): State<SessionStore>,
    State(conv): State<ConversationStore>,
    State(db): State<crate::store::Db>,
    Json(payload): Json<SpawnPayload>,
) -> impl IntoResponse {
    if payload.argv.is_empty() {
        return (StatusCode::BAD_REQUEST, "argv must not be empty").into_response();
    }
    // Before the fork, which is the only place this can still be reported.
    if let Some(problem) = cwd_problem(&payload.cwd) {
        return (StatusCode::BAD_REQUEST, problem).into_response();
    }

    // Prefer the caller-pinned id (matches `claude --session-id <uuid>`); fall
    // back to a fresh one for callers that don't pin (e.g. plain shells).
    let session_id = payload
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let cwd = payload.cwd.clone();
    let cols = payload.cols.unwrap_or(80);
    let rows = payload.rows.unwrap_or(24);

    // A profile spawn's CLAUDE_CONFIG_DIR moves this session's transcript out of
    // ~/.claude/projects; tell the transcript reader about that root so its
    // containment check doesn't refuse our own child's file.
    crate::session::transcript::allow_spawn_env(&payload.env);

    let pty_handle = match pty::spawn(
        &payload.argv,
        &cwd,
        PtySize {
            cols,
            rows,
            pixel_width: 0,
            pixel_height: 0,
        },
        &payload.env,
    ) {
        Ok(h) => Arc::new(h),
        Err(err) => {
            tracing::error!(?err, "spawn failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("spawn failed: {err}"),
            )
                .into_response();
        }
    };

    // daemon → child input pump (mpsc<WrapperMessage> → PTY)
    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<WrapperMessage>();
    let pty_for_input = pty_handle.clone();
    let input_task = tokio::spawn(async move {
        while let Some(msg) = input_rx.recv().await {
            match msg {
                WrapperMessage::Input { bytes } => {
                    if let Ok(decoded) = B64.decode(bytes.as_bytes()) {
                        if let Err(err) = pty::write_bytes(&pty_for_input, &decoded).await {
                            // A failed write means input (possibly a chat send
                            // the store already reported delivered) was lost —
                            // make it visible instead of vanishing.
                            tracing::warn!(?err, len = decoded.len(), "PTY input write failed");
                        }
                    }
                }
                WrapperMessage::Signal { signal } => match signal {
                    // Interactive interrupt: Ctrl-C byte through the tty.
                    crate::protocol::Signal::Sigint => {
                        let _ = pty::write_bytes(&pty_for_input, b"\x03").await;
                    }
                    // Terminate / kill: real process signal so a runaway session stops.
                    other => {
                        if let Err(err) = pty::signal_child(&pty_for_input, other) {
                            tracing::warn!(?err, "signal delivery failed");
                        }
                    }
                },
                WrapperMessage::Resize { cols, rows } => {
                    let _ = pty::resize(&pty_for_input, cols, rows).await;
                }
                _ => {}
            }
        }
    });

    // child → store output pump (PTY → output buffer + bytes broadcast)
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    if let Err(err) = pty::start_reader(&pty_handle, out_tx) {
        tracing::error!(?err, "start_reader failed");
        // Kill the child and abort the input pump so nothing leaks.
        let _ = pty::signal_child(&pty_handle, crate::protocol::Signal::Sigkill);
        input_task.abort();
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not start PTY reader",
        )
            .into_response();
    }
    let store_for_reader = store.clone();
    let session_for_reader = session_id.clone();
    let cwd_for_reader = cwd.clone();
    // This spawn's identity. A restart reuses the session id, so by the time this
    // reader sees EOF the store may already belong to a newer child — the teardown
    // below is gated on still owning both the generation and the PTY handle.
    let generation = store.claim_generation(&session_id);
    let handle_for_reader = pty_handle.clone();
    tokio::spawn(async move {
        while let Some(chunk) = out_rx.recv().await {
            store_for_reader
                .record_output(&session_for_reader, &chunk)
                .await;
        }
        // Reader EOF — child exited. Reap it (so it doesn't linger as a zombie)
        // and make sure we don't leak a pending spawn entry if SessionStart never
        // fired (e.g. claude crashed at startup).
        store_for_reader.reap_pty_owned(&session_for_reader, &handle_for_reader);
        if store_for_reader.is_resumable(&session_for_reader) {
            // The session was actually used. Drop the live plumbing but KEEP the
            // row: `SessionEnd` already marked it Stopped and the desktop lists
            // it as resumable. Deleting it here raced the post-terminate refetch
            // burst and made a quit agent disappear from Recent/History.
            store_for_reader.release_spawn(&session_for_reader, &cwd_for_reader, generation);
        } else {
            // Never bound to a real agent — nothing to resume, so drop it whole.
            store_for_reader.drop_pending_spawn(&session_for_reader, &cwd_for_reader, generation);
        }
        tracing::info!(session = %session_for_reader, generation, "in-daemon PTY reader ended");
    });

    store.register_pty(&session_id, pty_handle.clone());
    store.register_spawn(&session_id, &cwd, WrapperHandle { tx: input_tx });
    // The caller's own answer first, argv only as a fallback — see
    // `SpawnPayload::model`. Recorded unconditionally when either is present:
    // the guard used to be "a model appeared on argv", which is empty for every
    // resume and for every spawn that let the CLI pick.
    if let Some(model) = payload
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .or_else(|| model_from_argv(&payload.argv))
    {
        store.set_requested_model(&session_id, model);
        db.note_requested_model(&session_id, model);
    }
    store.note_term_size(&session_id, cols, rows);
    // Queued BEFORE the 200 below, so the caller never has to send it itself
    // and never has to guess when the TUI is ready for it. A fresh PTY row is
    // `Unknown`, so this rides the existing cold-start ladder: flushed on the
    // first `Input` transition, settled past the composer redraw, then
    // submit-verified.
    let first_message_queued = payload
        .first_message
        .as_deref()
        .is_some_and(|text| store.queue_first_message(&session_id, text));
    tracing::info!(%session_id, %cwd, argv=?payload.argv, first_message_queued, "spawned in-daemon PTY");

    // Hybrid agents (e.g. Codex): the PTY above is the agent's own TUI (the Term
    // view); additionally tail its rollout transcript so the GUI conversation
    // view is populated from the same live session.
    if payload.rollout_provider.as_deref() == Some("codex") {
        crate::providers::codex_rollout::spawn_tailer(
            store.clone(),
            conv.clone(),
            session_id.clone(),
            cwd.clone(),
        );
    }

    // `first_message_queued` is the caller's CONFIRMATION that the prompt was
    // accepted for delivery. Reported rather than assumed: a client talking to
    // a daemon that predates the field would otherwise take a plain 200 as
    // "dispatched" and leave a worker idle with no prompt.
    Json(json!({
        "session_id": session_id,
        "cwd": cwd,
        "first_message_queued": first_message_queued,
    }))
    .into_response()
}

/// `POST /sessions/spawn-managed` — spawn a *managed* (adapter-driven) session.
///
/// Unlike `/sessions/spawn` (a PTY), this runs a provider's own machine
/// interface and translates its events into the session model. Currently only
/// `opencode` (drives `opencode serve` + its `/event` SSE). The session id is
/// registered up front and returned immediately; the adapter boots in the
/// background, so the UI shows the agent while the server starts.
#[derive(Debug, Deserialize)]
pub struct SpawnManagedPayload {
    /// Provider backend: `opencode`, `codex`, `pi`, or `claude` (the headless
    /// stream-json transport — the PTY path stays on `/sessions/spawn`).
    pub provider: String,
    /// Working directory for the agent.
    pub cwd: String,
    /// Optional model override (provider-specific id).
    #[serde(default)]
    pub model: Option<String>,
    /// Optional reasoning-effort level. Codex maps it to the
    /// `model_reasoning_effort` config override; other providers ignore it.
    #[serde(default)]
    pub effort: Option<String>,
    /// Resolved launcher binary (the desktop resolves it on PATH); falls back to
    /// the provider name.
    #[serde(default)]
    pub bin: Option<String>,
    /// YOLO / skip-approvals: auto-approve every command and file change instead
    /// of surfacing them for the user's decision.
    #[serde(default)]
    pub yolo: bool,
    /// Workspacer MCP facade URL to register with the provider (supervisors).
    #[serde(default)]
    pub mcp: Option<String>,
    /// Role instructions to prepend to the agent's first turn (supervisors).
    #[serde(default)]
    pub instructions: Option<String>,
    /// Caller-pinned session id, so every client converges on one card.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Codex only: `"stream"` runs headless (GUI-only, no native TUI PTY — the
    /// daemon starts the thread itself via `thread/start`), mirroring Claude's
    /// stream transport. Anything else (or absent) is the default hybrid.
    #[serde(default)]
    pub transport: Option<String>,
    /// Claude only: initial permission mode, in the CLI's own vocabulary
    /// (`acceptEdits`, `plan`, `bypassPermissions`, …) — `--permission-mode`.
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// Claude only: resume this prior session (`--resume <id>`) instead of
    /// starting fresh with a pinned id.
    #[serde(default)]
    pub resume: Option<String>,
    /// Claude only: extra argv appended verbatim (escape hatch for CLI flags
    /// the payload doesn't model).
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// Claude only: extra env vars merged on top of the daemon's environment
    /// (e.g. a Claude profile's `CLAUDE_CONFIG_DIR`) — same semantics as
    /// `/sessions/spawn`'s `env`.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// The agent's FIRST PROMPT, delivered as part of the spawn rather than by
    /// a second call after the id comes back.
    ///
    /// DISTINCT FROM `instructions`, deliberately. `instructions` is a passive
    /// PREFIX: every adapter holds it in a `pending_instructions` slot and
    /// `with_instructions` prepends it to the first prompt the session
    /// receives — it never starts a turn on its own. So the two cannot be one
    /// field: a dispatch put in `instructions` would sit in that slot forever,
    /// waiting for a prompt that is the very thing it was meant to be. Riding
    /// the normal prompt channel also gets the ordering right for free — the
    /// facade role note and the `wks-result` contract are prepended to this
    /// text, once, in one turn.
    #[serde(default)]
    pub first_message: Option<String>,
}

pub async fn handle_managed(
    State(store): State<SessionStore>,
    State(conv): State<ConversationStore>,
    State(db): State<crate::store::Db>,
    Json(payload): Json<SpawnManagedPayload>,
) -> impl IntoResponse {
    if !matches!(
        payload.provider.as_str(),
        "opencode" | "codex" | "pi" | "claude"
    ) {
        return (
            StatusCode::BAD_REQUEST,
            format!("unsupported managed provider: {}", payload.provider),
        )
            .into_response();
    }
    // Before the session id is minted and registered: the 200 below is not a
    // statement that the agent started, so an unusable cwd has to be refused
    // here or it becomes a card nobody can talk to.
    if let Some(problem) = cwd_problem(&payload.cwd) {
        return (StatusCode::BAD_REQUEST, problem).into_response();
    }
    // Resuming a claude stream session keeps the CLI's *prior* session id (see
    // the claude_stream module contract — `--resume` is not re-pinnable), so an
    // unpinned resume must reuse that id as the row id: otherwise every hook
    // arrives under the prior id and drives a stale/ghost PTY row while the
    // stream row never sees its transcript_path.
    let session_id = payload
        .session_id
        .or_else(|| payload.resume.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    // A caller-pinned id (or a resume id) becomes a filesystem sink downstream:
    // the codex-thread sidecar filename, pi's `-e` extension temp file, the
    // `mcp/ask/<id>` url, and the `--session-id` argv. Gate it with the exact
    // containment the read routes enforce, so a `..` id can't escape those roots
    // or forge argv/urls. A freshly minted UUID always passes.
    if !crate::daemon::api::valid_session_id(&session_id) {
        return (StatusCode::BAD_REQUEST, "invalid session_id").into_response();
    }
    let bin = payload.bin.unwrap_or_else(|| payload.provider.clone());
    // Same as the PTY path: a profile config dir relocates the transcript root.
    crate::session::transcript::allow_spawn_env(&payload.env);

    store.register_managed(&session_id, &payload.cwd, &payload.provider);
    // Queued BEFORE the driver task starts and before the 200 below, so it is
    // waiting when `register_managed_input` drains it. Doing it here rather
    // than leaving it to the caller is the whole point: `register_managed`
    // above marks the row `Input` while attaching no wrapper, so a caller that
    // spawned and then posted a message would be refused with a 404 for as long
    // as the provider takes to boot.
    let first_message_queued = payload
        .first_message
        .as_deref()
        .is_some_and(|text| store.queue_first_message(&session_id, text));
    // Before the driver starts, so the very first snapshot knows this session's
    // window instead of guessing 200k from the marker-stripped transcript id.
    if let Some(model) = payload
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        store.set_requested_model(&session_id, model);
        // Persisted too, so a daemon restart rehydrates a 1M session as 1M
        // instead of reverting it to the table's guess for its stripped id.
        db.note_requested_model(&session_id, model);
    }
    let facade = crate::providers::Facade {
        mcp_url: payload.mcp.clone(),
        instructions: payload.instructions.clone(),
    };
    match payload.provider.as_str() {
        // Claude over the headless stream-json transport (2nd claude
        // transport; the PTY path on `/sessions/spawn` is untouched). The
        // transport is stamped *before* the driver starts so `ingest`'s hooks
        // guard and the first snapshot already see it.
        "claude" => {
            store.set_transport(&session_id, crate::session::state::Transport::Stream);
            crate::providers::claude_stream::spawn_session(
                store.clone(),
                conv.clone(),
                crate::providers::claude_stream::SpawnConfig {
                    session_id: session_id.clone(),
                    cwd: payload.cwd.clone(),
                    bin,
                    model: payload.model.clone(),
                    effort: payload.effort.clone(),
                    permission_mode: payload.permission_mode.clone(),
                    resume: payload.resume.clone(),
                    extra_args: payload.extra_args.clone(),
                    env: payload.env.clone(),
                    yolo: payload.yolo,
                    facade,
                },
            );
        }
        "opencode" => crate::providers::opencode::spawn_session(
            store.clone(),
            conv.clone(),
            session_id.clone(),
            payload.cwd.clone(),
            payload.model.clone(),
            bin,
            payload.yolo,
            facade,
        ),
        "codex" => {
            // Resume: rejoin the prior life's app-server thread (persisted in
            // the codex-threads sidecar) and pre-seed the conversation from its
            // rollout, so the pane shows the history immediately. Resume is
            // headless-only — the TUI can't rejoin an arbitrary thread — so it
            // forces the stream transport.
            let resume_thread = payload
                .resume
                .as_deref()
                .and_then(crate::providers::codex_rollout::thread_for);
            if payload.resume.is_some() && resume_thread.is_none() {
                tracing::warn!(session = %session_id, "codex resume requested but no thread recorded — starting fresh");
            }
            let headless =
                payload.transport.as_deref() == Some("stream") || resume_thread.is_some();
            if headless {
                // Stamped before the driver starts (like claude-stream above)
                // so every snapshot/frame gates the pane GUI-only from the
                // session's first instant.
                store.set_transport(&session_id, crate::session::state::Transport::Stream);
            }
            if let Some(tid) = &resume_thread {
                // Seed only when the conversation isn't already resident (a
                // resume in the same daemon life would otherwise duplicate it).
                let empty = conv
                    .snapshot(&session_id)
                    .is_none_or(|(_, items)| items.is_empty());
                if empty {
                    if let Some(path) = crate::providers::codex_rollout::rollout_for_thread(tid) {
                        let items = crate::providers::codex_rollout::replay_conversation(&path);
                        if !items.is_empty() {
                            conv.push(&session_id, items);
                        }
                    }
                }
            }
            crate::providers::codex::spawn_session(
                store.clone(),
                conv.clone(),
                session_id.clone(),
                payload.cwd.clone(),
                payload.model.clone(),
                payload.effort.clone(),
                bin,
                payload.yolo,
                headless,
                resume_thread,
                facade,
            )
        }
        "pi" => crate::providers::pi::spawn_session(
            store.clone(),
            conv.clone(),
            session_id.clone(),
            payload.cwd.clone(),
            payload.model.clone(),
            bin,
            payload.yolo,
            facade,
        ),
        _ => unreachable!(),
    }
    tracing::info!(%session_id, provider = %payload.provider, cwd = %payload.cwd, first_message_queued, "spawned managed session");

    // The caller's CONFIRMATION that the first prompt was accepted for
    // delivery — see the PTY route for why it is reported rather than assumed.
    Json(json!({
        "session_id": session_id,
        "cwd": payload.cwd,
        "first_message_queued": first_message_queued,
    }))
    .into_response()
}

/// Query params for `GET /providers/:provider/models`.
#[derive(Debug, Deserialize)]
pub struct ProviderModelsQuery {
    /// Working directory to run the provider CLI in (it reads project/global
    /// config + auth from there). Defaults to the daemon's cwd.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Deprecated and IGNORED. This used to be the launcher path the desktop had
    /// resolved on PATH, which made a plain `GET` — the one shape a browser can
    /// send cross-origin with no preflight, against a daemon that has no token —
    /// execute a caller-chosen binary. The daemon now resolves the launcher
    /// itself (see [`resolve_provider_bin`]), exactly like the Go brain's
    /// `providersListModels`. The field stays accepted so existing clients don't
    /// start getting 400s; nothing reads it.
    #[serde(default)]
    pub bin: Option<String>,
}

/// Candidate binary names for a provider, platform-aware — the Rust twin of the
/// brain's `binNames` / the desktop's `agentProviders.binNames`.
fn bin_names(provider: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            format!("{provider}.cmd"),
            format!("{provider}.exe"),
            provider.to_string(),
        ]
    } else {
        vec![provider.to_string()]
    }
}

/// Resolve a managed provider's launcher server-side: an explicit override from
/// the daemon's own environment wins, then a fresh `PATH` probe, then the bare
/// provider name so a just-installed CLI still works.
///
/// The brain's `resolveSpawnBin` has one tier this cannot reach: `config.yaml`'s
/// `agents.binaries.<provider>`. claudemon has no YAML parser and no config file
/// of its own, and it used to get that value relayed in the `bin` query param —
/// which is exactly the caller-supplied-binary hole this route just closed, so
/// taking it back is not an option. The intended replacement is
/// `WKS_<PROVIDER>_BIN` (matching the `WKS_CLAUDE_BIN` escape hatch the TUI and
/// the brain already read), but NOTHING EXPORTS IT YET: the two processes that
/// launch this daemon — `apps/desktop/src/main/services/claudemonDaemon.ts` and
/// the hub's daemon supervisor — must copy `config.agents.binaries` into the
/// child environment for the tier to work. Until they do, a launcher configured
/// off `PATH` resolves to the bare name here, `list_models` fails, and the Spawn
/// dialog's model picker soft-fails to free-text entry.
fn resolve_provider_bin(provider: &str) -> String {
    let env_key = format!("WKS_{}_BIN", provider.to_uppercase());
    if let Some(custom) = std::env::var(env_key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        return custom;
    }
    let names = bin_names(provider);
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            for name in &names {
                let full = dir.join(name);
                if full.is_file() {
                    return full.to_string_lossy().into_owned();
                }
            }
        }
    }
    provider.to_string()
}

/// `GET /providers/:provider/models` — list the models a managed provider can
/// launch with, live-queried from its own CLI/server (so the picker always
/// matches what the installed binary actually offers). Returns
/// `{ "models": [{ "id", "label", "default" }] }`; an empty list is valid (e.g.
/// Pi with no authed providers) and the UI falls back to free-text entry.
pub async fn handle_provider_models(
    Path(provider): Path<String>,
    Query(q): Query<ProviderModelsQuery>,
) -> impl IntoResponse {
    // Validate the provider *before* resolving anything: the name is a path
    // segment and feeds a PATH probe, so only the three known ids get that far.
    if !matches!(provider.as_str(), "opencode" | "codex" | "pi") {
        return (
            StatusCode::BAD_REQUEST,
            format!("unsupported managed provider: {provider}"),
        )
            .into_response();
    }
    if let Some(ignored) = q.bin.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        tracing::debug!(%provider, ignored, "ignoring caller-supplied models binary");
    }
    let bin = resolve_provider_bin(&provider);
    let cwd = q.cwd.unwrap_or_else(|| ".".to_string());
    let result = match provider.as_str() {
        "opencode" => crate::providers::opencode::list_models(&bin, &cwd).await,
        "codex" => crate::providers::codex::list_models(&bin, &cwd).await,
        _ => crate::providers::pi::list_models(&bin, &cwd).await,
    };
    match result {
        Ok(models) => Json(json!({ "models": models })).into_response(),
        Err(err) => {
            tracing::warn!(?err, %provider, "listing provider models failed");
            (StatusCode::BAD_GATEWAY, format!("{err}")).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ghost this guard exists to prevent: both spawn routes answer 200 with
    /// a session id before anyone knows the child lives, so an unusable cwd used
    /// to reach the user as an agent card whose session was already stopped (or
    /// gone) and whose every message came back 409. Reproduced exactly that way
    /// on both routes against a live daemon before this existed.
    #[test]
    fn cwd_problem_takes_directories_and_refuses_the_rest() {
        // A real directory is the whole happy path.
        assert_eq!(
            cwd_problem(&std::env::temp_dir().to_string_lossy()),
            None,
            "an existing directory must spawn"
        );
        // Existing is not enough — a file cannot be a working directory.
        let a_file = std::env::current_exe().expect("test binary path");
        assert!(cwd_problem(&a_file.to_string_lossy())
            .expect("a file is not a cwd")
            .contains("not an existing directory"));
        // And a path that simply is not there.
        let missing = std::env::temp_dir().join("claudemon-no-such-dir-4f3a9c");
        assert!(cwd_problem(&missing.to_string_lossy()).is_some());
        // An empty cwd is the same refusal rather than a silent fallback: both
        // normalizers upstream already turn "" into $HOME, so an empty string
        // arriving here is a caller bug, not a request to guess.
        assert!(cwd_problem("").is_some());
    }

    #[test]
    fn cwd_problem_explains_the_tilde_rather_than_expanding_it() {
        // The literal that broke the Fleet Manager: `agents.fleetRoot` typed as
        // "~/", trailing slash stripped by the normalizers, handed to this
        // daemon as a directory named "~". BINDING DECISION 1 says do not
        // expand it — so say why it failed instead.
        let problem = cwd_problem("~").expect("a directory named ~ does not exist");
        assert!(problem.contains("not expanded"), "got: {problem}");
        assert!(
            cwd_problem("~/Work")
                .expect("nor does ~/Work")
                .contains("not expanded"),
            "the hint belongs on every tilde spelling"
        );
        // A real directory whose NAME contains a tilde gets no such lecture.
        let tilde_named = std::env::temp_dir().join("claudemon-a~b-test");
        std::fs::create_dir_all(&tilde_named).expect("mkdir");
        assert_eq!(cwd_problem(&tilde_named.to_string_lossy()), None);
        let _ = std::fs::remove_dir(&tilde_named);
    }

    /// THE OMITTED-MODEL DROP. The payload's own `model` is preferred over
    /// sniffing argv, and it is what makes a RESUME measurable: `claude
    /// --resume <id>` carries no `--model` at all, so argv sniffing found
    /// nothing for precisely the sessions with the most history to mis-measure.
    #[test]
    fn the_payload_model_is_preferred_over_argv_and_covers_a_resume() {
        let resolved = |payload: SpawnPayload| -> Option<String> {
            payload
                .model
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .or_else(|| model_from_argv(&payload.argv))
                .map(str::to_string)
        };
        let payload = |v: serde_json::Value| -> SpawnPayload { serde_json::from_value(v).unwrap() };

        // A resume: nothing on argv, and the caller's resolved model saves it.
        assert_eq!(
            resolved(payload(json!({
                "argv": ["claude", "--resume", "abc"],
                "cwd": "/w",
                "model": "opus[1m]",
            }))),
            Some("opus[1m]".to_string())
        );
        // The caller's answer wins over a stale argv spelling.
        assert_eq!(
            resolved(payload(json!({
                "argv": ["claude", "--model", "sonnet"],
                "cwd": "/w",
                "model": "opus[1m]",
            }))),
            Some("opus[1m]".to_string())
        );
        // An older client that sends no `model` still gets the argv fallback.
        assert_eq!(
            resolved(payload(json!({
                "argv": ["claude", "--model", "opus[1m]"],
                "cwd": "/w",
            }))),
            Some("opus[1m]".to_string())
        );
        // Blank is not an answer — it must not shadow the argv fallback.
        assert_eq!(
            resolved(payload(json!({
                "argv": ["claude", "--model", "opus[1m]"],
                "cwd": "/w",
                "model": "   ",
            }))),
            Some("opus[1m]".to_string())
        );
        // Nothing anywhere stays nothing: an unrecorded request is honest, and
        // the resolver falls through to the contract table for the model id.
        assert_eq!(
            resolved(payload(json!({ "argv": ["claude"], "cwd": "/w" }))),
            None
        );
    }

    #[test]
    fn model_from_argv_reads_both_spellings() {
        let argv = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            model_from_argv(&argv(&["claude", "--model", "opus[1m]", "--verbose"])),
            Some("opus[1m]")
        );
        assert_eq!(
            model_from_argv(&argv(&["claude", "--model=sonnet[1m]"])),
            Some("sonnet[1m]")
        );
        assert_eq!(model_from_argv(&argv(&["claude", "--resume", "x"])), None);
        // A trailing `--model` with nothing after it must not panic.
        assert_eq!(model_from_argv(&argv(&["claude", "--model"])), None);
    }

    #[test]
    fn resolve_provider_bin_prefers_the_environment_override() {
        std::env::set_var("WKS_PI_BIN", "  /opt/pi/bin/pi  ");
        assert_eq!(resolve_provider_bin("pi"), "/opt/pi/bin/pi");
        std::env::remove_var("WKS_PI_BIN");
    }

    #[test]
    fn resolve_provider_bin_falls_back_to_the_bare_name() {
        // Nothing by this name is on any PATH, so the caller gets the command
        // name and the spawn fails honestly instead of running something else.
        assert_eq!(
            resolve_provider_bin("wks-no-such-provider"),
            "wks-no-such-provider"
        );
    }

    /// The finding: `GET /providers/:provider/models` used to execute the `bin`
    /// query param, so any page in any browser on this machine could run an
    /// arbitrary binary against a daemon that has no token. The param is still
    /// accepted (old clients keep working) but must never be executed.
    #[cfg(unix)]
    #[tokio::test]
    async fn provider_models_ignores_a_caller_supplied_binary() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("wks-models-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = |name: &str, marker: &str, body: &str| {
            let path = dir.join(name);
            std::fs::write(
                &path,
                format!("#!/bin/sh\ntouch {}\n{body}\n", dir.join(marker).display()),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        };
        let resolved = script("opencode", "resolved.ran", "echo anthropic/claude-sonnet-4");
        let attacker = script("attacker", "attacker.ran", "echo evil/model");

        std::env::set_var("WKS_OPENCODE_BIN", &resolved);
        let resp = handle_provider_models(
            Path("opencode".to_string()),
            Query(ProviderModelsQuery {
                cwd: Some(dir.to_string_lossy().into_owned()),
                bin: Some(attacker.to_string_lossy().into_owned()),
            }),
        )
        .await
        .into_response();
        std::env::remove_var("WKS_OPENCODE_BIN");

        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let models: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            models["models"][0]["id"],
            json!("anthropic/claude-sonnet-4")
        );
        assert!(
            dir.join("resolved.ran").exists(),
            "the daemon-resolved launcher is the one that ran"
        );
        assert!(
            !dir.join("attacker.ran").exists(),
            "the caller-supplied binary must never be executed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The finding: a caller-pinned `session_id` was spliced verbatim into
    /// provider filesystem sinks (the codex-thread sidecar filename, pi's `-e`
    /// extension temp file, the `mcp/ask/<id>` url, the `--session-id` argv). A
    /// `..` id escaped its root — `record_thread` wrote one directory *outside*
    /// `~/.workspacer/codex-threads`. `handle_managed` must gate the id with the
    /// same containment the read routes enforce, refusing before any spawn/write.
    #[tokio::test]
    async fn spawn_managed_rejects_a_traversal_session_id() {
        use crate::session::{ConversationStore, SessionStore};

        let marker = Uuid::new_v4();
        // The escape target `record_thread` would clobber for a `..` id: one
        // directory above the codex-threads dir.
        let escaped = crate::providers::codex_rollout::threads_dir_for_test().and_then(|d| {
            d.parent()
                .map(|p| p.join(format!("wks-managed-escape-{marker}.json")))
        });

        let payload: SpawnManagedPayload = serde_json::from_value(json!({
            "provider": "codex",
            "cwd": std::env::temp_dir().to_string_lossy(),
            "session_id": format!("../wks-managed-escape-{marker}"),
        }))
        .unwrap();

        let resp = handle_managed(
            State(SessionStore::new()),
            State(ConversationStore::new()),
            State(crate::store::Db::open(crate::testtmp::db_path("spawn-test")).expect("test db")),
            Json(payload),
        )
        .await
        .into_response();

        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "a `..` session_id must be refused before any provider spawn"
        );
        if let Some(escaped) = escaped {
            let leaked = escaped.exists();
            let _ = std::fs::remove_file(&escaped);
            assert!(
                !leaked,
                "traversal session_id escaped the codex-threads root: {}",
                escaped.display()
            );
        }
    }

    #[tokio::test]
    async fn provider_models_rejects_an_unknown_provider() {
        let resp = handle_provider_models(
            Path("../../bin/sh".to_string()),
            Query(ProviderModelsQuery {
                cwd: None,
                bin: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
