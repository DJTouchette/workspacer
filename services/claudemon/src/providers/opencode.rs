//! OpenCode adapter — translate `opencode serve` events into claudemon's
//! session model.
//!
//! OpenCode exposes a headless HTTP server (`opencode serve`, default
//! 127.0.0.1:4096, OpenAPI 3.1) and a Server-Sent-Events stream at `GET /event`
//! whose frames are `{ "type": "<entity>.<action>", "properties": { … } }`
//! (e.g. `session.idle`, `message.part.updated`, `permission.updated`). The
//! stream carries 80+ event types and evolves quickly, so the translator below
//! is deliberately *defensive*: it recognizes the events that map onto our
//! model and ignores everything else (rather than failing on unknown shapes).
//!
//! The pure `translate` is unit-tested. The live client that spawns
//! `opencode serve`, creates a session, posts prompts, and pumps the SSE stream
//! through `translate` + the shared `apply_updates` needs a real `opencode`
//! binary to validate end-to-end.

use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use futures_util::StreamExt;
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{apply_updates, AgentUpdate, Facade, ModelInfo, UsageAcc};
use crate::protocol::Signal;
use crate::session::conversation::ConversationItem;
use crate::session::state::SessionMode;
use crate::session::{ConversationStore, ModelSwitch, SessionStore};
use crate::wrapper::pty;

/// List the models OpenCode can launch with (cached; see [`super::cached_or_fetch`]).
pub async fn list_models(bin: &str, cwd: &str) -> anyhow::Result<Vec<ModelInfo>> {
    super::cached_or_fetch(format!("opencode:{bin}"), fetch_models(bin, cwd)).await
}

/// Live query: shell out to `opencode models`, which prints one `provider/model`
/// id per line for every provider it knows about. The ids are exactly what
/// `--model` / the message `model` field accept, so the picker round-trips them.
async fn fetch_models(bin: &str, cwd: &str) -> anyhow::Result<Vec<ModelInfo>> {
    let out = Command::new(bin)
        .arg("models")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .with_context(|| format!("running `{bin} models`"))?;
    if !out.status.success() {
        anyhow::bail!("`{bin} models` exited with {}", out.status);
    }
    let models = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && l.contains('/'))
        .map(|id| ModelInfo {
            id: id.to_string(),
            label: id.to_string(),
            default: false,
        })
        .collect();
    Ok(models)
}

/// Translate one OpenCode SSE event into zero or more typed updates. Pure and
/// total: unknown event types and missing fields yield an empty/partial result
/// rather than an error.
pub fn translate(event: &Value) -> Vec<AgentUpdate> {
    let ty = event.get("type").and_then(Value::as_str).unwrap_or("");
    let props = event.get("properties").cloned().unwrap_or(Value::Null);
    let mut out = Vec::new();

    match ty {
        "session.idle" => out.push(AgentUpdate::Idle),

        "session.error" => {
            let msg = props
                .get("error")
                .and_then(|e| e.get("message").or_else(|| e.get("data")))
                .and_then(Value::as_str)
                .or_else(|| props.get("message").and_then(Value::as_str))
                .unwrap_or("session error");
            out.push(AgentUpdate::Error(msg.to_string()));
        }

        "permission.updated" | "permission.replied" => {
            let p = props.get("permission").unwrap_or(&props);
            let id = p
                .get("id")
                .or_else(|| p.get("permissionID"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let tool = p
                .get("title")
                .and_then(Value::as_str)
                .or_else(|| p.get("type").and_then(Value::as_str))
                .map(str::to_owned);
            let summary = p
                .get("metadata")
                .and_then(|m| m.get("command"))
                .and_then(Value::as_str)
                .or_else(|| p.get("description").and_then(Value::as_str))
                .map(str::to_owned);
            out.push(AgentUpdate::PermissionPending {
                id,
                tool,
                summary,
                raw: p.clone(),
            });
        }

        // Both the streamed-part event and the whole-message event indicate the
        // agent is working; pull text / tool / usage out of whichever shape the
        // event carries.
        "message.updated" | "message.part.updated" => {
            out.push(AgentUpdate::Busy);
            if let Some(part) = props.get("part") {
                translate_part(part, &props, &mut out);
            }
            if let Some(info) = props.get("info").or_else(|| props.get("message")) {
                if let Some(u) = usage_from(info) {
                    out.push(u);
                }
            }
        }

        _ => {}
    }

    out
}

/// Map a message Part (`text` / `tool` / `step-finish` / …) to updates.
fn translate_part(part: &Value, props: &Value, out: &mut Vec<AgentUpdate>) {
    let kind = part.get("type").and_then(Value::as_str).unwrap_or("");
    let role = part
        .get("role")
        .and_then(Value::as_str)
        .or_else(|| props.get("role").and_then(Value::as_str));
    match kind {
        "text" => {
            if let Some(text) = part
                .get("delta")
                .and_then(Value::as_str)
                .or_else(|| part.get("text").and_then(Value::as_str))
            {
                if !text.is_empty() {
                    if role == Some("user") {
                        out.push(AgentUpdate::UserText(text.to_string()));
                    } else {
                        out.push(AgentUpdate::AssistantText(text.to_string()));
                    }
                }
            }
        }
        "tool" => {
            let id = part
                .get("callID")
                .and_then(Value::as_str)
                .or_else(|| part.get("id").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            let name = part
                .get("tool")
                .and_then(Value::as_str)
                .or_else(|| part.get("name").and_then(Value::as_str))
                .unwrap_or("tool")
                .to_string();
            let input = part
                .get("state")
                .and_then(|s| s.get("input"))
                .or_else(|| part.get("input"))
                .cloned()
                .unwrap_or(Value::Null);
            out.push(AgentUpdate::ToolUse { id, name, input });
        }
        "step-finish" | "step_finish" => {
            if let Some(u) = usage_from(part) {
                out.push(u);
            }
        }
        _ => {}
    }
}

/// Extract a `Usage` update from any object carrying `tokens` / `cost` /
/// `modelID` (a message info or a step-finish part).
fn usage_from(v: &Value) -> Option<AgentUpdate> {
    let tokens = v.get("tokens");
    let input = tokens.and_then(|t| t.get("input")).and_then(Value::as_u64);
    let output = tokens.and_then(|t| t.get("output")).and_then(Value::as_u64);
    let cache = tokens.and_then(|t| t.get("cache"));
    let cache_read = cache.and_then(|c| c.get("read")).and_then(Value::as_u64);
    let cache_write = cache.and_then(|c| c.get("write")).and_then(Value::as_u64);
    let cost = v.get("cost").and_then(Value::as_f64);
    let model = v
        .get("modelID")
        .or_else(|| v.get("model"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    if input.is_none() && output.is_none() && cost.is_none() && model.is_none() {
        return None;
    }
    // Context occupancy of the step that just finished: prompt side (input +
    // both cache tiers) plus what it generated. OpenCode doesn't report the
    // model's window; the status line falls back to the window table.
    let context_tokens = if input.is_some() || output.is_some() {
        Some(
            input.unwrap_or(0)
                + output.unwrap_or(0)
                + cache_read.unwrap_or(0)
                + cache_write.unwrap_or(0),
        )
    } else {
        None
    };
    Some(AgentUpdate::Usage {
        model,
        input_tokens: input,
        output_tokens: output,
        cached_input_tokens: None,
        cost_usd: cost,
        context_tokens,
        context_window: None,
    })
}

// ── Live client ─────────────────────────────────────────────────────────────

/// Spawn and drive an OpenCode-managed session in the background. Returns
/// immediately; the session's id is already registered in `store` by the
/// caller, so the UI shows it even while `opencode serve` is still booting.
// Mirrors the shared provider spawn signature (see codex::spawn_session).
#[allow(clippy::too_many_arguments)]
pub fn spawn_session(
    store: SessionStore,
    conv: ConversationStore,
    session_id: String,
    cwd: String,
    model: Option<String>,
    bin: String,
    yolo: bool,
    facade: Facade,
) {
    tokio::spawn(async move {
        if let Err(err) =
            run_session(&store, &conv, &session_id, &cwd, model, &bin, yolo, &facade).await
        {
            tracing::warn!(?err, session = %session_id, "opencode managed session ended with error");
        }
        store.deregister_managed(&session_id);
        conv.forget(&session_id);
    });
}

/// Merge workspacer's remote MCP servers into the cwd's `opencode.json` so
/// `opencode serve` loads them. Preserves any existing config; only sets the
/// named `mcp.<name>` entries. One read-merge-write for all entries — OpenCode
/// only reads the file at boot, and a single write keeps a crash between two
/// writes from leaving a half-registered pair.
fn write_opencode_mcp(cwd: &str, servers: &[(&str, Value)]) {
    let path = std::path::Path::new(cwd).join("opencode.json");
    let mut root = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    for (name, entry) in servers {
        root["mcp"][*name] = entry.clone();
    }
    if let Ok(text) = serde_json::to_string_pretty(&root) {
        if let Err(err) = std::fs::write(&path, text) {
            tracing::warn!(
                ?err,
                "writing opencode.json for workspacer MCP servers failed"
            );
        }
    }
}

/// The `mcp.workspacer_ask` entry: the daemon's per-session AskUserQuestion
/// endpoint (see `daemon::mcp_ask`), OpenCode's stand-in for Claude's native
/// tool. The explicit `timeout` is the point: OpenCode's remote-MCP request
/// timeout defaults to 5 seconds, which would kill a question the instant it
/// parks — a real answer can legitimately take hours, so we match the
/// endpoint's own 6h answer window (codex parity: `tool_timeout_sec=21600`).
fn ask_mcp_entry(session_id: &str) -> Option<Value> {
    let api_base = crate::daemon::API_BASE.get()?;
    Some(serde_json::json!({
        "type": "remote",
        "url": format!("{api_base}/mcp/ask/{session_id}"),
        "enabled": true,
        "timeout": 21_600_000u64, // ms
    }))
}

/// Split a picker model id (`provider/model`) into the object OpenCode's message
/// API expects (`{ providerID, modelID }`). Returns None when the id has no
/// provider prefix, so the caller omits the field and OpenCode keeps its default
/// rather than sending a malformed ref. Only the first `/` splits the provider
/// off — model ids may themselves contain slashes.
fn model_ref(id: &str) -> Option<Value> {
    let (provider, model) = id.split_once('/')?;
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some(serde_json::json!({ "providerID": provider, "modelID": model }))
}

/// Best-effort: set the shared session's model server-wide (`POST
/// /api/session/:id/model`, whose `ModelRef` uses `{ providerID, id }`). Our own
/// turns already carry the model per-message; this also moves the model the
/// attached TUI shows, so both views of the hybrid session agree. Failure is
/// non-fatal — the per-message override still governs what we send.
fn set_session_model(client: &reqwest::Client, base: &str, oc_id: &str, id: &str) {
    let Some((provider, model)) = id.split_once('/') else {
        return;
    };
    if provider.is_empty() || model.is_empty() {
        return;
    }
    // Note: no `/api` prefix — every other OpenCode endpoint (session create,
    // message, permissions, event, health) posts to `{base}/…` directly, so the
    // stray prefix here made the model switch silently 404.
    let url = format!("{base}/session/{oc_id}/model");
    let body = serde_json::json!({ "model": { "providerID": provider, "id": model } });
    let c = client.clone();
    tokio::spawn(async move {
        if let Err(err) = c.post(url).json(&body).send().await {
            tracing::warn!(?err, "opencode session model set failed");
        }
    });
}

/// POST a permission reply to OpenCode: `once` (allow this time) or `reject`.
/// Mirrors the SDK's `SessionPermissionService.Respond`.
fn reply_permission(
    client: &reqwest::Client,
    base: &str,
    oc_id: &str,
    perm_id: &str,
    approve: bool,
) {
    let url = format!("{base}/session/{oc_id}/permissions/{perm_id}");
    let body = serde_json::json!({ "response": if approve { "once" } else { "reject" } });
    let c = client.clone();
    tokio::spawn(async move {
        if let Err(err) = c.post(url).json(&body).send().await {
            tracing::warn!(?err, "opencode permission reply failed");
        }
    });
}

#[allow(clippy::too_many_arguments)]
async fn run_session(
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    cwd: &str,
    model: Option<String>,
    bin: &str,
    yolo: bool,
    facade: &Facade,
) -> anyhow::Result<()> {
    // Register workspacer's remote MCP servers before `opencode serve` boots
    // (it only reads opencode.json at startup): the supervisor facade when this
    // session has one, and the AskUserQuestion endpoint for every session —
    // structured questions are baseline parity, not a supervisor feature. Note
    // the seam persists a per-session URL into the project's opencode.json;
    // the next session in this cwd overwrites it, but a stale entry outlives
    // the session (harmless: the daemon 404s an unknown session id).
    let mut mcp_servers: Vec<(&str, Value)> = Vec::new();
    if let Some(mcp_url) = &facade.mcp_url {
        mcp_servers.push((
            "workspacer",
            serde_json::json!({ "type": "remote", "url": mcp_url, "enabled": true }),
        ));
    }
    if let Some(entry) = ask_mcp_entry(session_id) {
        mcp_servers.push(("workspacer_ask", entry));
    }
    if !mcp_servers.is_empty() {
        write_opencode_mcp(cwd, &mcp_servers);
    }

    // Pick a free loopback port for this server instance (each managed session
    // gets its own `opencode serve`).
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .context("reserving a port for opencode serve")?;
        listener.local_addr()?.port()
    };

    let mut child = Command::new(bin)
        .arg("serve")
        .arg("--hostname")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .current_dir(cwd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning `{bin} serve`"))?;

    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();

    wait_healthy(&client, &base).await?;

    // Create the OpenCode session (its own id, distinct from our canonical one).
    let oc_id: String = {
        let resp = client
            .post(format!("{base}/session"))
            .json(&serde_json::json!({}))
            .send()
            .await?
            .error_for_status()?;
        let v: Value = resp.json().await?;
        v.get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .context("opencode POST /session response missing `id`")?
    };

    // Route user prompts + approval decisions from the HTTP API to us.
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    store.register_managed_input(session_id, tx);
    let (dtx, mut drx) = mpsc::unbounded_channel::<bool>();
    store.register_managed_decision(session_id, dtx);
    // Live model switch (POST /sessions/:id/model): OpenCode applies the model
    // per message, so we hold the current selection here and stamp it on each
    // turn — a switch just updates it (and best-effort sets it session-wide so
    // the attached TUI agrees). The switched model takes effect on the next turn.
    let mut model = model;
    let (mtx, mut mrx) = mpsc::unbounded_channel::<ModelSwitch>();
    store.register_managed_model_switch(session_id, mtx);
    // Structural interrupt (POST /sessions/:id/signal SIGINT): OpenCode's
    // server has a native turn abort (`POST /session/:id/abort`) that stops the
    // running turn while keeping the session alive — same semantics as codex's
    // `turn/interrupt`. Without this channel the signal falls through to a
    // Ctrl-C byte into the attach-TUI PTY (a mirror, and absent entirely when
    // the attach failed), so clients with no PTY — mobile remote, the inbox,
    // wks-tui — couldn't stop a runaway turn at all.
    let (itx, mut irx) = mpsc::unbounded_channel::<()>();
    store.register_managed_interrupt(session_id, itx);
    // Approval policy, live-switchable via `/permission-mode`: the adapter
    // mediates every permission event on the `/event` stream, so flipping this
    // flag changes whether the next request auto-approves without touching the
    // session. `opencode serve` always emits the events (we never spawn it in a
    // bypass mode), so yolo→ask works too — spawned_yolo is always false.
    let yolo_live = Arc::new(AtomicBool::new(yolo));
    store.register_managed_yolo(session_id, yolo_live.clone(), false);

    // Hybrid Term view: run the native OpenCode TUI attached to this same serve
    // + session in a PTY, so the renderer's terminal surface mirrors the GUI
    // (structured /event adapter) live — two views of one session. Best-effort:
    // if it can't start, the GUI still works and the Term is just empty.
    let attach_argv = vec![
        bin.to_string(),
        "attach".to_string(),
        base.clone(),
        "--session".to_string(),
        oc_id.clone(),
    ];
    let attach_pty = match super::spawn_attach_pty(store, session_id, &attach_argv, cwd) {
        Ok(h) => Some(h),
        Err(err) => {
            tracing::warn!(?err, session = %session_id, "opencode attach TUI failed; Term view unavailable");
            None
        }
    };

    // Subscribe to the event stream.
    let resp = client
        .get(format!("{base}/event"))
        .send()
        .await?
        .error_for_status()?;
    let mut stream = resp.bytes_stream();

    let mut buf: Vec<u8> = Vec::new();
    let mut cur_mode = SessionMode::Input;
    let mut acc = UsageAcc::new();
    // The id of the permission currently awaiting the user's decision (non-YOLO).
    // Permission ids awaiting a decision, FIFO — a queue (not one slot) so
    // concurrent permission requests don't drop each other and stall the agent.
    let mut pending_perm_ids: std::collections::VecDeque<String> =
        std::collections::VecDeque::new();
    // Role instructions to prepend to the first turn only (supervisors).
    let mut pending_instructions: Option<String> = facade.instructions.clone();

    loop {
        tokio::select! {
            chunk = stream.next() => match chunk {
                Some(Ok(bytes)) => {
                    buf.extend_from_slice(&bytes);
                    while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                        let line: Vec<u8> = buf.drain(..=pos).collect();
                        let s = String::from_utf8_lossy(&line);
                        let s = s.trim_end_matches(['\r', '\n']);
                        let Some(data) = s.strip_prefix("data:") else { continue };
                        let data = data.trim();
                        if data.is_empty() { continue; }
                        let Ok(value) = serde_json::from_str::<Value>(data) else { continue };
                        let mut updates = translate(&value);
                        if updates.is_empty() { continue; }
                        // Pull any permission request out: YOLO auto-allows it and
                        // keeps working; otherwise we surface it and remember the id
                        // so the user's decision can be forwarded.
                        if let Some(idx) = updates.iter().position(|u| matches!(u, AgentUpdate::PermissionPending { .. })) {
                            if let AgentUpdate::PermissionPending { id, .. } = &updates[idx] {
                                let perm_id = id.clone();
                                if yolo_live.load(Ordering::Relaxed) {
                                    if let Some(pid) = perm_id {
                                        reply_permission(&client, &base, &oc_id, &pid, true);
                                    }
                                    updates.remove(idx); // don't surface Approval
                                } else if let Some(pid) = perm_id {
                                    pending_perm_ids.push_back(pid);
                                }
                            }
                        }
                        apply_updates(store, conv, session_id, updates, &mut cur_mode, &mut acc);
                    }
                }
                Some(Err(err)) => return Err(err.into()),
                None => break,
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
                    let c = client.clone();
                    let url = format!("{base}/session/{oc_id}/message");
                    let mut body = serde_json::json!({ "parts": [ { "type": "text", "text": sent } ] });
                    if let Some(m) = model.as_deref().and_then(model_ref) {
                        body["model"] = m;
                    }
                    tokio::spawn(async move {
                        if let Err(err) = c.post(url).json(&body).send().await {
                            tracing::warn!(?err, "opencode message POST failed");
                        }
                    });
                }
                None => break, // managed input dropped → session terminated
            },
            decision = drx.recv() => match decision {
                Some(approve) => {
                    if let Some(pid) = pending_perm_ids.pop_front() {
                        reply_permission(&client, &base, &oc_id, &pid, approve);
                        // The agent resumes (or stops on reject); reflect Responding.
                        store.set_managed_mode(session_id, SessionMode::Responding, None);
                        cur_mode = SessionMode::Responding;
                    }
                }
                None => break,
            },
            intr = irx.recv() => match intr {
                Some(()) => {
                    // Fire-and-forget like the other posts: the abort's effect
                    // arrives on the /event stream as `session.idle`, which
                    // translate() already maps to Idle — no mode change here.
                    let c = client.clone();
                    let url = format!("{base}/session/{oc_id}/abort");
                    tokio::spawn(async move {
                        if let Err(err) = c.post(url).send().await {
                            tracing::warn!(?err, "opencode abort POST failed");
                        }
                    });
                }
                None => break,
            },
            switch = mrx.recv() => match switch {
                Some(sw) => {
                    // OpenCode has no reasoning-effort knob (effort is ignored);
                    // only the model moves. Update what the next turn stamps, set
                    // it session-wide for the TUI, and reflect it on the status
                    // line now so the pill doesn't wait for the next turn's usage.
                    if let Some(m) = sw.model {
                        set_session_model(&client, &base, &oc_id, &m);
                        apply_updates(
                            store, conv, session_id,
                            vec![AgentUpdate::Usage {
                                model: Some(m.clone()),
                                input_tokens: None,
                                output_tokens: None,
                                cached_input_tokens: None,
                                cost_usd: None,
                                context_tokens: None,
                                context_window: None,
                            }],
                            &mut cur_mode, &mut acc,
                        );
                        model = Some(m);
                    }
                }
                None => break,
            },
            status = child.wait() => {
                tracing::info!(?status, session = %session_id, "opencode serve exited");
                break;
            }
        }
    }

    let _ = child.start_kill();
    if let Some(handle) = &attach_pty {
        let _ = pty::signal_child(handle, Signal::Sigkill);
    }
    Ok(())
}

/// Poll `/global/health` until the server answers (or we give up after ~10s).
async fn wait_healthy(client: &reqwest::Client, base: &str) -> anyhow::Result<()> {
    for _ in 0..50 {
        if let Ok(resp) = client.get(format!("{base}/global/health")).send().await {
            if resp.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("opencode serve did not become healthy in time")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn idle_maps_to_idle() {
        let ev = json!({ "type": "session.idle", "properties": { "sessionID": "s1" } });
        assert_eq!(translate(&ev), vec![AgentUpdate::Idle]);
    }

    #[test]
    fn error_extracts_message() {
        let ev =
            json!({ "type": "session.error", "properties": { "error": { "message": "boom" } } });
        assert_eq!(translate(&ev), vec![AgentUpdate::Error("boom".into())]);
    }

    #[test]
    fn permission_pending_pulls_tool_and_command() {
        let props = json!({
            "id": "perm_1",
            "type": "bash",
            "title": "Bash",
            "metadata": { "command": "rm -rf build" }
        });
        let ev = json!({
            "type": "permission.updated",
            "properties": props.clone(),
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::PermissionPending {
                id: Some("perm_1".into()),
                tool: Some("Bash".into()),
                summary: Some("rm -rf build".into()),
                raw: props,
            }]
        );
    }

    #[test]
    fn text_part_is_busy_plus_assistant_text() {
        let ev = json!({
            "type": "message.part.updated",
            "properties": { "part": { "type": "text", "text": "Hello" } }
        });
        assert_eq!(
            translate(&ev),
            vec![
                AgentUpdate::Busy,
                AgentUpdate::AssistantText("Hello".into())
            ]
        );
    }

    #[test]
    fn text_part_prefers_delta_over_full_text() {
        let ev = json!({
            "type": "message.part.updated",
            "properties": { "part": { "type": "text", "text": "Hello world", "delta": " world" } }
        });
        assert_eq!(
            translate(&ev),
            vec![
                AgentUpdate::Busy,
                AgentUpdate::AssistantText(" world".into())
            ]
        );
    }

    #[test]
    fn user_role_text_maps_to_user_text() {
        let ev = json!({
            "type": "message.part.updated",
            "properties": { "part": { "type": "text", "text": "hi", "role": "user" } }
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::Busy, AgentUpdate::UserText("hi".into())]
        );
    }

    #[test]
    fn tool_part_maps_to_tool_use() {
        let ev = json!({
            "type": "message.part.updated",
            "properties": { "part": {
                "type": "tool", "tool": "bash", "callID": "c1",
                "state": { "input": { "command": "ls" } }
            } }
        });
        assert_eq!(
            translate(&ev),
            vec![
                AgentUpdate::Busy,
                AgentUpdate::ToolUse {
                    id: "c1".into(),
                    name: "bash".into(),
                    input: json!({ "command": "ls" }),
                }
            ]
        );
    }

    #[test]
    fn step_finish_part_yields_usage() {
        let ev = json!({
            "type": "message.part.updated",
            "properties": { "part": {
                "type": "step-finish",
                "tokens": { "input": 1200, "output": 340 },
                "cost": 0.0123,
                "modelID": "claude-sonnet-4"
            } }
        });
        assert_eq!(
            translate(&ev),
            vec![
                AgentUpdate::Busy,
                AgentUpdate::Usage {
                    model: Some("claude-sonnet-4".into()),
                    input_tokens: Some(1200),
                    output_tokens: Some(340),
                    cached_input_tokens: None,
                    cost_usd: Some(0.0123),
                    context_tokens: Some(1540),
                    context_window: None,
                }
            ]
        );
    }

    #[test]
    fn message_info_usage_is_extracted() {
        let ev = json!({
            "type": "message.updated",
            "properties": { "info": {
                "role": "assistant",
                "tokens": { "input": 50, "output": 9 },
                "cost": 0.001
            } }
        });
        assert_eq!(
            translate(&ev),
            vec![
                AgentUpdate::Busy,
                AgentUpdate::Usage {
                    model: None,
                    input_tokens: Some(50),
                    output_tokens: Some(9),
                    cached_input_tokens: None,
                    cost_usd: Some(0.001),
                    context_tokens: Some(59),
                    context_window: None,
                }
            ]
        );
    }

    #[test]
    fn unknown_event_is_ignored() {
        let ev = json!({ "type": "installation.updated", "properties": {} });
        assert!(translate(&ev).is_empty());
        let ev2 = json!({ "type": "lsp.diagnostics", "properties": { "anything": 1 } });
        assert!(translate(&ev2).is_empty());
    }

    #[test]
    fn malformed_event_does_not_panic() {
        assert!(translate(&json!({})).is_empty());
        assert_eq!(
            translate(&json!({ "type": "message.part.updated" })),
            vec![AgentUpdate::Busy]
        );
        assert!(translate(&Value::Null).is_empty());
    }

    #[test]
    fn write_opencode_mcp_creates_and_merges_entries() {
        let dir = std::env::temp_dir().join(format!("wks-oc-mcp-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cwd = dir.to_str().unwrap();

        // Fresh file: both servers land under `mcp`.
        write_opencode_mcp(
            cwd,
            &[
                (
                    "workspacer",
                    json!({ "type": "remote", "url": "http://sup", "enabled": true }),
                ),
                (
                    "workspacer_ask",
                    json!({ "type": "remote", "url": "http://ask", "enabled": true, "timeout": 21_600_000u64 }),
                ),
            ],
        );
        let root: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("opencode.json")).unwrap())
                .unwrap();
        assert_eq!(root["mcp"]["workspacer"]["url"], "http://sup");
        assert_eq!(root["mcp"]["workspacer_ask"]["url"], "http://ask");
        assert_eq!(root["mcp"]["workspacer_ask"]["timeout"], 21_600_000u64);

        // Rewrite for a new session: existing config keys and unrelated MCP
        // servers survive; only the named entries move.
        std::fs::write(
            dir.join("opencode.json"),
            serde_json::to_string_pretty(&json!({
                "theme": "dark",
                "mcp": { "other": { "type": "remote", "url": "http://keep" } }
            }))
            .unwrap(),
        )
        .unwrap();
        write_opencode_mcp(
            cwd,
            &[(
                "workspacer_ask",
                json!({ "type": "remote", "url": "http://ask2", "enabled": true }),
            )],
        );
        let root: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("opencode.json")).unwrap())
                .unwrap();
        assert_eq!(root["theme"], "dark");
        assert_eq!(root["mcp"]["other"]["url"], "http://keep");
        assert_eq!(root["mcp"]["workspacer_ask"]["url"], "http://ask2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ask_mcp_entry_is_remote_with_generous_timeout() {
        // API_BASE is a process-global OnceCell shared across the test binary,
        // so don't assert a specific host — another test may have set it first.
        let _ = crate::daemon::API_BASE.set("http://127.0.0.1:7777".to_string());
        let entry = ask_mcp_entry("sess-1").expect("API_BASE is set");
        assert_eq!(entry["type"], "remote");
        assert_eq!(entry["enabled"], true);
        let url = entry["url"].as_str().unwrap();
        assert!(url.ends_with("/mcp/ask/sess-1"), "url was {url}");
        // The whole point of the entry: outlive OpenCode's 5s remote-MCP
        // default while a question waits on a human.
        assert_eq!(entry["timeout"], 21_600_000u64);
    }

    #[test]
    fn model_ref_splits_provider_and_model() {
        // OpenCode's message `model` field is an object, not a string.
        assert_eq!(
            model_ref("anthropic/claude-sonnet-4"),
            Some(json!({ "providerID": "anthropic", "modelID": "claude-sonnet-4" }))
        );
        // Only the first slash splits the provider off; the rest is the model id.
        assert_eq!(
            model_ref("openrouter/meta-llama/llama-3.1"),
            Some(json!({ "providerID": "openrouter", "modelID": "meta-llama/llama-3.1" }))
        );
        // No provider prefix (or an empty half) → None, so the field is omitted
        // and OpenCode keeps its default rather than getting a malformed ref.
        assert!(model_ref("bare-model").is_none());
        assert!(model_ref("/x").is_none());
        assert!(model_ref("x/").is_none());
    }
}
