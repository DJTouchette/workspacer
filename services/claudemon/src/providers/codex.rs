//! Codex adapter — translate `codex app-server` notifications into claudemon's
//! session model.
//!
//! Codex's app server speaks JSON-RPC 2.0. We run it as a WebSocket daemon
//! (`codex app-server --listen ws://127.0.0.1:<port>`) rather than over stdio,
//! because a plain-TCP ws endpoint is the one transport a live TUI
//! (`codex --remote ws://…`) and our RPC client can *share* — so the session is
//! a **hybrid** (GUI + Term), like the OpenCode `serve` + `attach` pairing.
//! (`--listen unix://` is gated in current Codex builds and the `remote-control`
//! daemon needs the standalone installer, so ws is the portable choice — and
//! works on Windows too, unlike the unix-socket paths.)
//!
//! Ownership is TUI-first: the native TUI (`codex --remote`, in a PTY = the Term
//! view) creates and runs the session's thread — a real, "running", resumable
//! rollout — and our RPC client discovers it (`thread/loaded/list`) and *rejoins*
//! it (`thread/resume`, which subscribes us to the live stream). The reverse (RPC
//! `thread/start` + TUI `resume`) fails: a just-started thread has no rollout, so
//! `resume` errors with "no rollout found for thread id …". Once rejoined we
//! `turn/start` each GUI prompt and consume the streamed notifications:
//! `turn/started|completed|failed`,
//! `item/started|completed` (commandExecution / fileChange / mcpToolCall / …),
//! `item/agentMessage/delta` (streamed text), `thread/tokenUsage/updated`, and
//! the approval requests (`item/commandExecution/requestApproval`,
//! `item/fileChange/requestApproval`).
//!
//! The pure `translate(method, params)` is unit-tested; the live ws client
//! needs a real `codex` binary to validate end-to-end.

use std::process::Stdio;
use std::sync::Arc;

use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use super::{apply_updates, AgentUpdate, Facade, ModelInfo, UsageAcc};
use crate::protocol::Signal;
use crate::session::conversation::ConversationItem;
use crate::session::state::SessionMode;
use crate::session::{ConversationStore, SessionStore};
use crate::wrapper::pty;

/// Translate one Codex app-server message (`method` + `params`) into typed
/// updates. Pure and total: unknown methods / missing fields yield an
/// empty/partial result.
pub fn translate(method: &str, params: &Value) -> Vec<AgentUpdate> {
    let mut out = Vec::new();
    match method {
        "turn/started" => out.push(AgentUpdate::Busy),
        "turn/completed" => out.push(AgentUpdate::Idle),
        "turn/failed" => {
            let msg = params
                .get("error")
                .and_then(|e| e.get("message").or_else(|| e.get("data")))
                .and_then(Value::as_str)
                .or_else(|| params.get("message").and_then(Value::as_str))
                .unwrap_or("turn failed");
            out.push(AgentUpdate::Error(msg.to_string()));
            out.push(AgentUpdate::Idle);
        }
        // Streamed assistant text. Reasoning/command-output deltas are skipped
        // (kept out of the conversation to avoid noise).
        "item/agentMessage/delta" => {
            if let Some(text) = params
                .get("delta")
                .and_then(Value::as_str)
                .or_else(|| params.get("text").and_then(Value::as_str))
            {
                if !text.is_empty() {
                    out.push(AgentUpdate::AssistantText(text.to_string()));
                }
            }
        }
        "item/started" | "item/completed" => {
            if let Some(item) = params.get("item") {
                translate_item(method, item, &mut out);
            }
        }
        "thread/tokenUsage/updated" => {
            let u = params.get("usage").unwrap_or(params);
            let input = u.get("input_tokens").and_then(Value::as_u64);
            let output = u.get("output_tokens").and_then(Value::as_u64);
            if input.is_some() || output.is_some() {
                // Context occupancy: prefer the event's own total; else sum. The
                // window is probed on both objects and in both spellings — codex
                // has carried `model_context_window` beside the token totals.
                let context_tokens = u
                    .get("total_tokens")
                    .and_then(Value::as_u64)
                    .or_else(|| Some(input.unwrap_or(0) + output.unwrap_or(0)));
                let context_window = [u, params].iter().find_map(|v| {
                    v.get("model_context_window")
                        .or_else(|| v.get("modelContextWindow"))
                        .and_then(Value::as_u64)
                });
                out.push(AgentUpdate::Usage {
                    model: None,
                    input_tokens: input,
                    output_tokens: output,
                    cost_usd: None,
                    context_tokens,
                    context_window,
                });
            }
        }
        "item/commandExecution/requestApproval" => {
            let cmd = command_text(params);
            out.push(AgentUpdate::PermissionPending {
                id: None, // carried out of band as the JSON-RPC request id
                tool: Some("command".into()),
                summary: cmd,
            });
        }
        "item/fileChange/requestApproval" => {
            let path = params
                .get("path")
                .and_then(Value::as_str)
                .or_else(|| params.get("item").and_then(|i| i.get("path")).and_then(Value::as_str))
                .map(str::to_owned);
            out.push(AgentUpdate::PermissionPending {
                id: None,
                tool: Some("file change".into()),
                summary: path,
            });
        }
        _ => {}
    }
    out
}

/// Map a started/completed `item` to a tool-use update. Only `item/started`
/// emits tool-uses (so a tool isn't recorded twice); assistant text arrives via
/// `item/agentMessage/delta`, so completed agentMessage items are not re-emitted.
fn translate_item(method: &str, item: &Value, out: &mut Vec<AgentUpdate>) {
    if method != "item/started" {
        return;
    }
    let ty = item
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| item.get("itemType").and_then(Value::as_str))
        .unwrap_or("");
    let id = item.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    match ty {
        "commandExecution" => {
            let input = item
                .get("command")
                .cloned()
                .map(|c| json!({ "command": c }))
                .unwrap_or(Value::Null);
            out.push(AgentUpdate::ToolUse { id, name: "shell".into(), input });
        }
        "fileChange" => {
            let input = json!({ "path": item.get("path").cloned().unwrap_or(Value::Null) });
            out.push(AgentUpdate::ToolUse { id, name: "apply_patch".into(), input });
        }
        "mcpToolCall" => {
            let name = item
                .get("tool")
                .and_then(Value::as_str)
                .or_else(|| item.get("name").and_then(Value::as_str))
                .unwrap_or("mcp")
                .to_string();
            let input = item.get("arguments").or_else(|| item.get("input")).cloned().unwrap_or(Value::Null);
            out.push(AgentUpdate::ToolUse { id, name, input });
        }
        "webSearch" => {
            let input = item.get("query").cloned().map(|q| json!({ "query": q })).unwrap_or(Value::Null);
            out.push(AgentUpdate::ToolUse { id, name: "web_search".into(), input });
        }
        _ => {}
    }
}

/// The command string for an approval request, whether it's a plain string or a
/// list of argv parts.
fn command_text(params: &Value) -> Option<String> {
    let cmd = params
        .get("command")
        .or_else(|| params.get("item").and_then(|i| i.get("command")))?;
    match cmd {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    }
}

// ── Model listing ────────────────────────────────────────────────────────────

/// List the models Codex offers (cached; see [`super::cached_or_fetch`]).
pub async fn list_models(bin: &str, cwd: &str) -> anyhow::Result<Vec<ModelInfo>> {
    super::cached_or_fetch(format!("codex:{bin}"), fetch_models(bin, cwd)).await
}

/// Live query: boot a throwaway `codex app-server`, `initialize`, ask for the
/// catalog via `model/list`, then drop the process. Hidden models are skipped;
/// the rest map to the picker with their `displayName` as label and the
/// server-flagged default marked.
async fn fetch_models(bin: &str, cwd: &str) -> anyhow::Result<Vec<ModelInfo>> {
    let mut child = Command::new(bin)
        .arg("app-server")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning `{bin} app-server`"))?;

    let mut stdin = child.stdin.take().context("codex app-server: no stdin")?;
    let stdout = child.stdout.take().context("codex app-server: no stdout")?;
    let mut lines = BufReader::new(stdout).lines();

    write_msg(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "clientInfo": { "name": "workspacer", "version": "0.1" } }
        }),
    )
    .await?;
    write_msg(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "model/list", "params": {} }),
    )
    .await?;

    // Read until the response to id=2 arrives (or stdout closes). A short overall
    // timeout keeps a wedged binary from hanging the picker.
    let read = async {
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
            if value.get("id").and_then(Value::as_u64) != Some(2) {
                continue;
            }
            let data = value
                .get("result")
                .and_then(|r| r.get("data"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let models = data
                .iter()
                .filter(|m| !m.get("hidden").and_then(Value::as_bool).unwrap_or(false))
                .filter_map(|m| {
                    let id = m
                        .get("model")
                        .or_else(|| m.get("id"))
                        .and_then(Value::as_str)?
                        .to_string();
                    let label = m
                        .get("displayName")
                        .and_then(Value::as_str)
                        .unwrap_or(&id)
                        .to_string();
                    let default = m.get("isDefault").and_then(Value::as_bool).unwrap_or(false);
                    Some(ModelInfo { id, label, default })
                })
                .collect::<Vec<_>>();
            return Ok(models);
        }
        anyhow::bail!("codex app-server closed before answering model/list");
    };

    let result = tokio::time::timeout(std::time::Duration::from_secs(10), read)
        .await
        .context("timed out listing codex models")?;
    let _ = child.start_kill();
    result
}

// ── Live client ─────────────────────────────────────────────────────────────

/// Spawn and drive a Codex-managed session in the background. Returns
/// immediately; the session id is already registered in `store` by the caller.
#[allow(clippy::too_many_arguments)]
pub fn spawn_session(
    store: SessionStore,
    conv: ConversationStore,
    session_id: String,
    cwd: String,
    model: Option<String>,
    effort: Option<String>,
    bin: String,
    yolo: bool,
    facade: Facade,
) {
    tokio::spawn(async move {
        if let Err(err) = run_session(&store, &conv, &session_id, &cwd, model, effort, &bin, yolo, &facade).await {
            tracing::warn!(?err, session = %session_id, "codex managed session ended with error");
        }
        store.deregister_managed(&session_id);
        conv.forget(&session_id);
    });
}

/// A connected JSON-RPC-over-WebSocket stream to a session's `codex app-server`.
type CodexWs = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Start `codex app-server --listen ws://…` for this session on a free loopback
/// port and connect the ws client. Returns the child, the ws stream, and the ws
/// URL (the TUI attaches to it via `--remote`). An error means the ws path is
/// unavailable for this Codex build — the caller falls back to the rollout hybrid.
async fn start_appserver(
    cwd: &str,
    bin: &str,
    facade: &Facade,
) -> anyhow::Result<(tokio::process::Child, CodexWs, String)> {
    // Each managed session gets its own app-server, so threads/approvals are
    // isolated per pane.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .context("reserving a port for codex app-server")?;
        listener.local_addr()?.port()
    };
    let ws_url = format!("ws://127.0.0.1:{port}");
    let http_base = format!("http://127.0.0.1:{port}");

    let mut cmd = Command::new(bin);
    cmd.arg("app-server").arg("--listen").arg(&ws_url);
    // Register the workspacer MCP facade (supervisors) as a config override so
    // `codex app-server` exposes its tools.
    if let Some(mcp_url) = &facade.mcp_url {
        cmd.arg("-c")
            .arg(format!("mcp_servers.workspacer.url=\"{mcp_url}\""));
    }
    let child = cmd
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning `{bin} app-server --listen {ws_url}`"))?;

    // Wait for the server's HTTP `/readyz` before opening the ws client.
    wait_ready(&http_base).await?;

    let (ws, _resp) = connect_async(&ws_url)
        .await
        .with_context(|| format!("connecting to codex app-server at {ws_url}"))?;
    Ok((child, ws, ws_url))
}

/// Fallback when the app-server ws path is unavailable: run the plain `codex` TUI
/// in a PTY (the Term view) and tail its rollout transcript for the GUI — the
/// same mechanism used on Windows. Less rich than the RPC path (approvals happen
/// in the Term; text lands in rollout-sized chunks rather than token deltas) but
/// robust and version-independent, so a Codex CLI that changed `app-server` /
/// `--remote` still gives a working pane instead of an empty one.
#[allow(clippy::too_many_arguments)]
async fn run_rollout_fallback(
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    cwd: &str,
    model: Option<String>,
    effort: Option<String>,
    bin: &str,
    yolo: bool,
) -> anyhow::Result<()> {
    // Plain codex TUI (no `--remote`): it owns its own session and writes a rollout.
    let mut argv = vec![bin.to_string()];
    if let Some(m) = &model {
        argv.push("-c".to_string());
        argv.push(format!("model={}", Value::String(m.clone())));
    }
    if let Some(e) = &effort {
        argv.push("-c".to_string());
        argv.push(format!("model_reasoning_effort={}", Value::String(e.clone())));
    }
    if yolo {
        argv.push("--dangerously-bypass-approvals-and-sandbox".to_string());
    }
    let tui = super::spawn_attach_pty(store, session_id, &argv, cwd)
        .context("spawning fallback codex TUI")?;
    // Drive the GUI conversation from the rollout transcript.
    super::codex_rollout::spawn_tailer(store.clone(), conv.clone(), session_id.to_string(), cwd.to_string());

    // GUI-composer prompts arrive here; write them into the TUI's PTY (there's no
    // RPC channel in this mode — approvals and everything else happen in the Term).
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    store.register_managed_input(session_id, tx);
    let mut tui_check = tokio::time::interval(std::time::Duration::from_secs(2));

    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(text) => {
                    conv.push(session_id, vec![ConversationItem::UserMessage { text: text.clone(), timestamp: None }]);
                    // Bracketed paste + Enter (same as the Claude PTY path) so the
                    // TUI submits instead of folding the CR into the paste.
                    let body = text.trim_end_matches(['\r', '\n']);
                    let mut bytes = b"\x1b[200~".to_vec();
                    bytes.extend_from_slice(body.as_bytes());
                    bytes.extend_from_slice(b"\x1b[201~\r");
                    let _ = pty::write_bytes(&tui, &bytes).await;
                }
                None => break, // managed input dropped → terminated
            },
            _ = tui_check.tick() => {
                if pty::has_exited(&tui) {
                    tracing::info!(session = %session_id, "codex fallback TUI exited; tearing down");
                    break;
                }
            }
        }
    }
    let _ = pty::signal_child(&tui, Signal::Sigkill);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_session(
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    cwd: &str,
    model: Option<String>,
    effort: Option<String>,
    bin: &str,
    yolo: bool,
    facade: &Facade,
) -> anyhow::Result<()> {
    // Start the app-server + ws client. If that fails, the ws path is unavailable
    // for this Codex build (e.g. a version that dropped/renamed `app-server
    // --listen`, or won't bind/handshake) — degrade to the rollout hybrid rather
    // than leave the pane dead. The RPC path is preferred; this is the safety net.
    let (mut child, ws_stream, ws_url) = match start_appserver(cwd, bin, facade).await {
        Ok(t) => t,
        Err(err) => {
            tracing::warn!(?err, session = %session_id, "codex app-server ws path unavailable — falling back to the rollout hybrid (Term + transcript-tailed GUI)");
            return run_rollout_fallback(store, conv, session_id, cwd, model, effort, bin, yolo).await;
        }
    };
    let (mut ws_write, mut ws_read) = ws_stream.split();

    // Serialize all outgoing JSON-RPC through one task that owns the ws sink, so
    // the several send sites (handshake, turns, approval replies) never contend
    // for the writer. Dropping `out_tx` (on return) closes the sink.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        while let Some(v) = out_rx.recv().await {
            if ws_write.send(Message::Text(v.to_string())).await.is_err() {
                break;
            }
        }
        let _ = ws_write.close().await;
    });

    // The app server requires an `initialize` handshake before any other request.
    let _ = out_tx.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "clientInfo": { "name": "workspacer", "version": "0.1" } }
    }));

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    store.register_managed_input(session_id, tx);
    let (dtx, mut drx) = mpsc::unbounded_channel::<bool>();
    store.register_managed_decision(session_id, dtx);

    // The native TUI OWNS the thread: bare `codex --remote` creates and runs it
    // (a real, "running", resumable rollout) — then we rejoin it over RPC (below)
    // to drive the GUI, exactly the validated owner/rejoiner split. The reverse
    // (RPC `thread/start` here + TUI `resume`) fails because a just-started thread
    // has no rollout yet: "no rollout found for thread id …". Model / YOLO are set
    // on the thread's creator (the TUI) as config overrides. Kept so we can kill
    // it when the session ends.
    let tui_pty = spawn_codex_tui(store, session_id, cwd, bin, &ws_url, model.as_deref(), effort.as_deref(), yolo);

    let mut thread_id: Option<String> = None;
    // Whether our `thread/resume` has actually taken (we're receiving the thread's
    // live stream). The first resume can land before the TUI's thread is "running"
    // and fail, so we keep retrying until this flips true.
    let mut subscribed = false;
    let mut pending_prompts: Vec<String> = Vec::new();
    // id 1 = initialize, 2 = thread/resume, 100 = thread/loaded/list poll; the
    // user's turns take ids from 3 up.
    let mut req_id: u64 = 2;
    let mut cur_mode = SessionMode::Input;
    let mut acc = UsageAcc::new();
    // JSON-RPC ids of approval requests awaiting the user's decision (non-YOLO),
    // FIFO. A queue (not a single slot) so two requests arriving before the user
    // answers don't drop the first and deadlock the agent. YOLO answers inline and
    // never parks one here.
    let mut pending_approvals: std::collections::VecDeque<Value> = std::collections::VecDeque::new();
    // Role instructions to prepend to the first turn only (supervisors).
    let mut pending_instructions: Option<String> = facade.instructions.clone();
    // Poll `thread/loaded/list` until the TUI's thread appears, then rejoin it —
    // retrying the resume until we're actually subscribed. Bounded by a deadline
    // so a TUI that never creates a thread (or died at startup) can't busy-poll
    // for the daemon's life.
    let mut discover = tokio::time::interval(std::time::Duration::from_millis(300));
    // If we can't rejoin the TUI's thread within this window, the ws path is up
    // but its thread protocol drifted — fall back to the rollout hybrid rather
    // than sit on an empty GUI. Generous enough for a slow TUI cold-start.
    let discover_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut needs_fallback = false;
    // Watch the TUI (the thread owner) for death: if it exits while the app-server
    // lives, the session is dead and must tear down rather than run against a
    // dead thread. try_wait is cheap; a coarse tick is fine.
    let mut tui_check = tokio::time::interval(std::time::Duration::from_secs(2));

    loop {
        tokio::select! {
            msg = ws_read.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    // One ws frame may carry one JSON-RPC object or several
                    // newline-delimited ones; handle each line independently.
                    for line in text.split('\n') {
                        let line = line.trim();
                        if line.is_empty() { continue; }
                        let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
                        handle_message(
                            &value, store, conv, session_id, &out_tx,
                            &mut thread_id, &mut subscribed, &mut pending_prompts, &mut req_id,
                            &mut cur_mode, &mut acc, yolo, &mut pending_approvals,
                        );
                    }
                }
                Some(Ok(Message::Close(_))) | None => break, // server gone
                Some(Ok(_)) => {} // ping/pong/binary — ignore
                Some(Err(err)) => return Err(err.into()),
            },
            // Drive discovery + rejoin until we're subscribed to the TUI's thread:
            // ask which threads are loaded (there's only ever one on this
            // per-session app-server) and (re)send `thread/resume` for it. The
            // first resume can precede the thread becoming "running", so we retry.
            _ = discover.tick(), if !subscribed && !needs_fallback => {
                if tokio::time::Instant::now() >= discover_deadline {
                    tracing::warn!(session = %session_id, "codex: couldn't rejoin the TUI thread in time; falling back to the rollout hybrid");
                    needs_fallback = true;
                    break;
                } else {
                    match &thread_id {
                        None => {
                            let _ = out_tx.send(json!({ "jsonrpc": "2.0", "id": 100, "method": "thread/loaded/list", "params": {} }));
                        }
                        Some(tid) => {
                            let _ = out_tx.send(json!({ "jsonrpc": "2.0", "id": 2, "method": "thread/resume", "params": { "threadId": tid } }));
                        }
                    }
                }
            },
            _ = tui_check.tick(), if tui_pty.is_some() => {
                if tui_pty.as_ref().is_some_and(|h| pty::has_exited(h)) {
                    tracing::info!(session = %session_id, "codex TUI exited; tearing down session");
                    break;
                }
            },
            msg = rx.recv() => match msg {
                Some(text) => {
                    // Echo the user's message verbatim, but prepend the role
                    // instructions (once) to what's actually sent to the agent.
                    conv.push(session_id, vec![ConversationItem::UserMessage { text: text.clone(), timestamp: None }]);
                    let sent = match pending_instructions.take() {
                        Some(instr) => format!("{instr}\n\n{text}"),
                        None => text,
                    };
                    if cur_mode != SessionMode::Responding {
                        store.set_managed_mode(session_id, SessionMode::Responding, None);
                        cur_mode = SessionMode::Responding;
                    }
                    match &thread_id {
                        Some(tid) => {
                            req_id += 1;
                            send_turn(&out_tx, req_id, tid, &sent);
                        }
                        // Thread not open yet — buffer the (already-wrapped) prompt.
                        None => pending_prompts.push(sent),
                    }
                }
                None => break, // managed input dropped → terminated
            },
            decision = drx.recv() => match decision {
                Some(approve) => {
                    if let Some(id) = pending_approvals.pop_front() {
                        let result = json!({ "decision": if approve { "accept" } else { "decline" } });
                        let _ = out_tx.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
                        store.set_managed_mode(session_id, SessionMode::Responding, None);
                        cur_mode = SessionMode::Responding;
                    }
                }
                None => break,
            },
            status = child.wait() => {
                tracing::info!(?status, session = %session_id, "codex app-server exited");
                break;
            }
        }
    }

    // Tear down the ws attempt (app-server + the `--remote` TUI) before any
    // fallback, so the rollout path starts from a clean slate.
    let _ = child.start_kill();
    if let Some(handle) = &tui_pty {
        let _ = pty::signal_child(handle, Signal::Sigkill);
    }

    // The thread protocol drifted (ws up, but we never rejoined): degrade to the
    // rollout hybrid so the pane still works.
    if needs_fallback {
        return run_rollout_fallback(store, conv, session_id, cwd, model, effort, bin, yolo).await;
    }
    Ok(())
}

/// Launch the native Codex TUI in a PTY, connected over `--remote` to this
/// session's app-server. The TUI creates and owns the session's thread; the RPC
/// client rejoins it (see `run_session`), so the Term view and the RPC-driven GUI
/// are two views of one conversation. Best-effort: if it can't start, the GUI
/// still works and the Term is empty.
#[allow(clippy::too_many_arguments)]
fn spawn_codex_tui(
    store: &SessionStore,
    session_id: &str,
    cwd: &str,
    bin: &str,
    ws_url: &str,
    model: Option<&str>,
    effort: Option<&str>,
    yolo: bool,
) -> Option<Arc<pty::PtyHandle>> {
    let mut argv = vec![
        bin.to_string(),
        "--remote".to_string(),
        ws_url.to_string(),
    ];
    // Model / reasoning effort are config overrides on the thread's creator;
    // YOLO bypasses the approval/sandbox prompts so the shared thread doesn't
    // block on them.
    if let Some(m) = model {
        argv.push("-c".to_string());
        argv.push(format!("model={}", Value::String(m.to_string())));
    }
    if let Some(e) = effort {
        argv.push("-c".to_string());
        argv.push(format!("model_reasoning_effort={}", Value::String(e.to_string())));
    }
    if yolo {
        argv.push("--dangerously-bypass-approvals-and-sandbox".to_string());
    }
    match super::spawn_attach_pty(store, session_id, &argv, cwd) {
        Ok(h) => Some(h),
        Err(err) => {
            tracing::warn!(?err, session = %session_id, "codex TUI (--remote) failed; Term view unavailable");
            None
        }
    }
}

/// Poll the app-server's HTTP `/readyz` until it answers (or give up after ~10s).
async fn wait_ready(http_base: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    for _ in 0..50 {
        if let Ok(resp) = client.get(format!("{http_base}/readyz")).send().await {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    anyhow::bail!("codex app-server did not become ready in time")
}

#[allow(clippy::too_many_arguments)]
fn handle_message(
    value: &Value,
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    out_tx: &mpsc::UnboundedSender<Value>,
    thread_id: &mut Option<String>,
    subscribed: &mut bool,
    pending_prompts: &mut Vec<String>,
    req_id: &mut u64,
    cur_mode: &mut SessionMode,
    acc: &mut UsageAcc,
    yolo: bool,
    pending_approvals: &mut std::collections::VecDeque<Value>,
) {
    // A response to one of our requests:
    //  - `thread/loaded/list` (id=100): `result.data` lists thread ids loaded in
    //    this per-session app-server — the one the TUI created. Rejoin it
    //    (`thread/resume`), which for a *running* thread subscribes us to its live
    //    stream so the GUI mirrors the TUI.
    //  - `thread/resume` (id=2): success means we're subscribed; an error (e.g. the
    //    thread wasn't "running" yet) is logged and the discover loop retries.
    if value.get("id").is_some() && (value.get("result").is_some() || value.get("error").is_some()) {
        let id = value.get("id").and_then(Value::as_u64);
        if id == Some(2) {
            if let Some(err) = value.get("error") {
                tracing::warn!(session = %session_id, error = %err, "codex thread/resume failed; retrying");
            } else {
                *subscribed = true;
                tracing::info!(session = %session_id, thread = ?thread_id, "codex: rejoined thread — GUI stream subscribed");
            }
        }
        if thread_id.is_none() {
            if let Some(tid) = value
                .get("result")
                .and_then(|r| r.get("data"))
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_str)
            {
                *thread_id = Some(tid.to_string());
                tracing::info!(session = %session_id, thread = %tid, "codex: discovered TUI thread, resuming");
                let _ = out_tx.send(json!({
                    "jsonrpc": "2.0", "id": 2, "method": "thread/resume",
                    "params": { "threadId": tid }
                }));
                // Flush any prompts that arrived before the thread was found.
                for text in std::mem::take(pending_prompts) {
                    *req_id += 1;
                    send_turn(out_tx, *req_id, tid, &text);
                }
            }
        }
        return;
    }

    let Some(method) = value.get("method").and_then(Value::as_str) else {
        return;
    };
    let params = value.get("params").cloned().unwrap_or(Value::Null);

    // Receiving any turn/item stream event proves the resume took, even if we
    // missed its response — stop the discover/retry loop.
    if !*subscribed && (method.starts_with("turn/") || method.starts_with("item/")) {
        *subscribed = true;
    }

    let updates = translate(method, &params);
    if !updates.is_empty() {
        apply_updates(store, conv, session_id, updates, cur_mode, acc);
    }

    // Server→client *requests* (they carry an id) must be answered or the agent
    // blocks. For an approval request: YOLO accepts inline; otherwise we park the
    // request id and surface it, so the user's /approve decision is forwarded
    // (see the decision branch in run_session).
    if value.get("id").is_some() && method.ends_with("/requestApproval") {
        let id = value.get("id").cloned().unwrap_or(Value::Null);
        if yolo {
            let _ = out_tx.send(json!({ "jsonrpc": "2.0", "id": id, "result": { "decision": "accept" } }));
        } else {
            pending_approvals.push_back(id);
        }
    }
}

fn send_turn(out_tx: &mpsc::UnboundedSender<Value>, id: u64, thread_id: &str, text: &str) {
    let _ = out_tx.send(json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "turn/start",
        "params": { "threadId": thread_id, "input": [ { "type": "text", "text": text } ] }
    }));
}

/// Write one JSON-RPC message as a single newline-delimited line. Used by the
/// stdio-based `list_models` handshake (a throwaway `codex app-server`).
async fn write_msg(stdin: &mut ChildStdin, value: &Value) -> anyhow::Result<()> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    stdin.write_all(&line).await?;
    stdin.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_started_is_busy() {
        assert_eq!(translate("turn/started", &json!({})), vec![AgentUpdate::Busy]);
    }

    #[test]
    fn turn_completed_is_idle() {
        assert_eq!(translate("turn/completed", &json!({})), vec![AgentUpdate::Idle]);
    }

    #[test]
    fn turn_failed_is_error_then_idle() {
        let p = json!({ "error": { "message": "nope" } });
        assert_eq!(
            translate("turn/failed", &p),
            vec![AgentUpdate::Error("nope".into()), AgentUpdate::Idle]
        );
    }

    #[test]
    fn agent_message_delta_is_assistant_text() {
        let p = json!({ "delta": "hi there" });
        assert_eq!(
            translate("item/agentMessage/delta", &p),
            vec![AgentUpdate::AssistantText("hi there".into())]
        );
    }

    #[test]
    fn command_execution_item_started_is_tool_use() {
        let p = json!({ "item": { "type": "commandExecution", "id": "i1", "command": ["bash", "-c", "ls"] } });
        assert_eq!(
            translate("item/started", &p),
            vec![AgentUpdate::ToolUse {
                id: "i1".into(),
                name: "shell".into(),
                input: json!({ "command": ["bash", "-c", "ls"] }),
            }]
        );
    }

    #[test]
    fn item_completed_does_not_double_emit_tool_use() {
        let p = json!({ "item": { "type": "commandExecution", "id": "i1", "command": "ls" } });
        assert!(translate("item/completed", &p).is_empty());
    }

    #[test]
    fn token_usage_maps_to_usage() {
        let p = json!({ "usage": { "input_tokens": 1000, "output_tokens": 200, "cached_input_tokens": 50,
            "total_tokens": 1250, "model_context_window": 272000 } });
        assert_eq!(
            translate("thread/tokenUsage/updated", &p),
            vec![AgentUpdate::Usage {
                model: None,
                input_tokens: Some(1000),
                output_tokens: Some(200),
                cost_usd: None,
                context_tokens: Some(1250),
                context_window: Some(272000),
            }]
        );
    }

    #[test]
    fn command_approval_request_is_pending_with_joined_argv() {
        let p = json!({ "command": ["rm", "-rf", "build"] });
        assert_eq!(
            translate("item/commandExecution/requestApproval", &p),
            vec![AgentUpdate::PermissionPending {
                id: None,
                tool: Some("command".into()),
                summary: Some("rm -rf build".into()),
            }]
        );
    }

    #[test]
    fn file_change_approval_request_is_pending_with_path() {
        let p = json!({ "path": "src/main.rs" });
        assert_eq!(
            translate("item/fileChange/requestApproval", &p),
            vec![AgentUpdate::PermissionPending {
                id: None,
                tool: Some("file change".into()),
                summary: Some("src/main.rs".into()),
            }]
        );
    }

    #[test]
    fn unknown_method_is_ignored() {
        assert!(translate("session/whatever", &json!({ "x": 1 })).is_empty());
        assert!(translate("item/reasoning/summaryTextDelta", &json!({ "delta": "thinking" })).is_empty());
    }
}
