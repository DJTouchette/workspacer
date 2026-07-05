//! Pi adapter — translate `pi --mode rpc` events into claudemon's session model.
//!
//! Pi (the `@mariozechner/pi-coding-agent` harness) exposes an RPC mode that
//! speaks strict LF-delimited JSONL over stdio: each line is a JSON object
//! tagged with a `type`. We send `{"type":"prompt","message":...}` for each
//! user turn and consume the streamed events: `agent_start`/`agent_end` and
//! `turn_start`/`turn_end` (lifecycle), `message_update` carrying an
//! `assistantMessageEvent` (streamed text deltas), `message_end`/`turn_end`
//! (whose message object carries token usage), and `tool_execution_start`
//! (tool calls).
//!
//! Pi's core auto-runs tools without an approval gate; approvals only appear
//! when a permission *extension* is loaded, which prompts via the bidirectional
//! Extension UI protocol (`extension_ui_request` with a `confirm`/`select`
//! dialog). We answer those: YOLO accepts inline; otherwise we surface the
//! request and forward the user's /approve decision as an `extension_ui_response`.
//!
//! The pure `translate(event)` is unit-tested; the live stdio client needs a
//! real `pi` binary to validate end-to-end.

use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Context;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::mpsc;

use super::{apply_updates, AgentUpdate, Facade, ModelInfo, UsageAcc};
use crate::session::conversation::ConversationItem;
use crate::session::state::SessionMode;
use crate::session::{ConversationStore, ModelSwitch, SessionStore};

/// List the models Pi can launch with (cached; see [`super::cached_or_fetch`]).
pub async fn list_models(bin: &str, cwd: &str) -> anyhow::Result<Vec<ModelInfo>> {
    super::cached_or_fetch(format!("pi:{bin}"), fetch_models(bin, cwd)).await
}

/// Live query via Pi's RPC `get_available_models`: boot a throwaway
/// `pi --mode rpc`, ask for the catalog, then drop the process. Pi only returns
/// models for providers the user has authed, so an empty list (no login) is
/// normal — the picker then falls back to free text. Each model carries
/// `provider` + `id`; we join them as `provider/id`, the form `--model` accepts.
async fn fetch_models(bin: &str, cwd: &str) -> anyhow::Result<Vec<ModelInfo>> {
    let mut child = Command::new(bin)
        .arg("--mode")
        .arg("rpc")
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning `{bin} --mode rpc`"))?;

    let mut stdin = child.stdin.take().context("pi rpc: no stdin")?;
    let stdout = child.stdout.take().context("pi rpc: no stdout")?;
    let mut lines = BufReader::new(stdout).lines();

    write_msg(
        &mut stdin,
        &json!({ "type": "get_available_models", "id": "models" }),
    )
    .await?;

    let read = async {
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value.get("command").and_then(Value::as_str) != Some("get_available_models") {
                continue;
            }
            let arr = value
                .get("data")
                .and_then(|d| d.get("models"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let models = arr
                .iter()
                .filter_map(|m| {
                    let provider = m.get("provider").and_then(Value::as_str)?;
                    let id = m.get("id").and_then(Value::as_str)?;
                    let full = format!("{provider}/{id}");
                    Some(ModelInfo {
                        id: full.clone(),
                        label: full,
                        default: false,
                    })
                })
                .collect::<Vec<_>>();
            return Ok(models);
        }
        anyhow::bail!("pi rpc closed before answering get_available_models");
    };

    let result = tokio::time::timeout(std::time::Duration::from_secs(10), read)
        .await
        .context("timed out listing pi models")?;
    let _ = child.start_kill();
    result
}

/// Translate one Pi RPC event into zero or more typed updates. Pure and total:
/// unknown event types and missing fields yield an empty/partial result rather
/// than an error. `extension_ui_request` is handled out of band (it needs a
/// reply), so it is intentionally *not* translated here.
pub fn translate(event: &Value) -> Vec<AgentUpdate> {
    let ty = event.get("type").and_then(Value::as_str).unwrap_or("");
    let mut out = Vec::new();

    match ty {
        // The agent loop started / a single LLM turn started → working.
        "agent_start" | "turn_start" => out.push(AgentUpdate::Busy),

        // The whole agent loop finished → ready for the next prompt. (A
        // `turn_end` is only *one* round within the loop; more turns may follow,
        // so it must NOT flip us back to Input — we only pull its usage below.)
        "agent_end" => out.push(AgentUpdate::Idle),

        "turn_end" => {
            if let Some(u) = event.get("message").and_then(usage_from) {
                out.push(u);
            }
        }

        // Streamed assistant output. Only text deltas land in the conversation;
        // thinking / tool-call deltas are skipped (tool calls arrive as their
        // own `tool_execution_start`).
        "message_update" => {
            out.push(AgentUpdate::Busy);
            if let Some(ev) = event.get("assistantMessageEvent") {
                if ev.get("type").and_then(Value::as_str) == Some("text_delta") {
                    if let Some(delta) = ev.get("delta").and_then(Value::as_str) {
                        if !delta.is_empty() {
                            out.push(AgentUpdate::AssistantText(delta.to_string()));
                        }
                    }
                }
            }
        }

        // End of one assistant message — carries final token usage. Text was
        // already streamed via `message_update`, so we don't re-emit it.
        "message_end" => {
            if let Some(u) = event.get("message").and_then(usage_from) {
                out.push(u);
            }
        }

        "tool_execution_start" => {
            let id = event
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let name = event
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            let input = event.get("args").cloned().unwrap_or(Value::Null);
            out.push(AgentUpdate::ToolUse { id, name, input });
        }

        "extension_error" => {
            if let Some(msg) = event.get("error").and_then(Value::as_str) {
                out.push(AgentUpdate::Error(msg.to_string()));
            }
        }

        _ => {}
    }

    out
}

/// Extract a `Usage` update from a Pi message object (`usage` + `model`). Pi's
/// unified API has used a few key spellings across versions, so we probe the
/// common ones defensively.
fn usage_from(message: &Value) -> Option<AgentUpdate> {
    let usage = message.get("usage");
    let pick = |keys: &[&str]| -> Option<u64> {
        let u = usage?;
        keys.iter().find_map(|k| u.get(*k).and_then(Value::as_u64))
    };
    let input = pick(&[
        "input",
        "inputTokens",
        "input_tokens",
        "promptTokens",
        "prompt_tokens",
    ]);
    let output = pick(&[
        "output",
        "outputTokens",
        "output_tokens",
        "completionTokens",
        "completion_tokens",
    ]);
    let cost = usage
        .and_then(|u| u.get("cost"))
        .and_then(|c| {
            c.as_f64()
                .or_else(|| c.get("total").and_then(Value::as_f64))
        })
        .or_else(|| message.get("cost").and_then(Value::as_f64));
    let model = message
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if input.is_none() && output.is_none() && cost.is_none() && model.is_none() {
        return None;
    }
    // Context occupancy — Pi's own formula (compaction.ts): totalTokens when
    // present, else input + output + both cache tiers.
    let total = pick(&["totalTokens", "total_tokens", "total"]);
    let cache_read = pick(&["cacheRead", "cache_read", "cacheReadTokens"]);
    let cache_write = pick(&["cacheWrite", "cache_write", "cacheWriteTokens"]);
    let context_tokens = total.filter(|t| *t > 0).or_else(|| {
        if input.is_some() || output.is_some() {
            Some(
                input.unwrap_or(0)
                    + output.unwrap_or(0)
                    + cache_read.unwrap_or(0)
                    + cache_write.unwrap_or(0),
            )
        } else {
            None
        }
    });
    Some(AgentUpdate::Usage {
        model,
        input_tokens: input,
        output_tokens: output,
        cost_usd: cost,
        context_tokens,
        context_window: None,
    })
}

/// Build Pi's RPC `set_model` command from a picker id (`provider/modelId`, the
/// form `fetch_models` emits). Pi wants the provider and model as separate
/// fields; only the first `/` splits them (a model id may contain more). Returns
/// None for an id without a provider prefix, so the caller skips a malformed
/// switch rather than sending one Pi will reject.
fn set_model_msg(id: &str) -> Option<Value> {
    let (provider, model_id) = id.split_once('/')?;
    if provider.is_empty() || model_id.is_empty() {
        return None;
    }
    Some(json!({ "type": "set_model", "provider": provider, "modelId": model_id }))
}

/// A short human summary for an Extension UI dialog request, for the approval card.
fn dialog_summary(req: &Value) -> Option<String> {
    req.get("message")
        .and_then(Value::as_str)
        .or_else(|| req.get("title").and_then(Value::as_str))
        .or_else(|| req.get("placeholder").and_then(Value::as_str))
        .map(str::to_owned)
}

// ── Live client ─────────────────────────────────────────────────────────────

/// Spawn and drive a Pi-managed session in the background. Returns immediately;
/// the session id is already registered in `store` by the caller.
///
/// Two shapes, mirroring the Codex hybrid split:
///   - **Hybrid (default):** the native Pi TUI runs in a PTY (the Term view),
///     pinned to our canonical session id via `--session-id`, and the GUI is
///     driven by tailing the session JSONL Pi writes for that id. GUI prompts
///     are pasted into the TUI; approvals happen in the Term.
///   - **RPC (supervisors):** `pi --mode rpc` headless — needed because the
///     facade's role instructions must be prepended programmatically and the
///     dialogs must surface as GUI approvals; a supervisor has no human at a
///     Term.
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
        let headless = facade.mcp_url.is_some() || facade.instructions.is_some();
        let result = if headless {
            run_session(&store, &conv, &session_id, &cwd, model, &bin, yolo, &facade).await
        } else {
            run_tui_session(&store, &conv, &session_id, &cwd, model, &bin).await
        };
        if let Err(err) = result {
            tracing::warn!(?err, session = %session_id, "pi managed session ended with error");
        }
        store.deregister_managed(&session_id);
        conv.forget(&session_id);
    });
}

/// Merge the workspacer MCP facade into the cwd's `.mcp.json` so Pi loads it as
/// a remote (HTTP) MCP server. Preserves any existing config; only sets
/// `mcpServers.workspacer`. Pi reads the standard `.mcp.json` automatically.
fn write_pi_mcp(cwd: &str, mcp_url: &str) {
    let path = std::path::Path::new(cwd).join(".mcp.json");
    let mut root = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    root["mcpServers"]["workspacer"] = json!({
        "type": "http",
        "url": mcp_url,
    });
    if let Ok(text) = serde_json::to_string_pretty(&root) {
        if let Err(err) = std::fs::write(&path, text) {
            tracing::warn!(?err, "writing .mcp.json for the facade failed");
        }
    }
}

// ── Hybrid (TUI + session-file tail) ────────────────────────────────────────

/// Pi's per-project session dir: `~/.pi/agent/sessions/--<encoded-cwd>--/`,
/// where the encoding strips a leading separator and turns `/ \ :` into `-`
/// (mirrors `session-manager.ts`'s `safePath`).
fn pi_session_dir(cwd: &str) -> Option<std::path::PathBuf> {
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    let trimmed = cwd.trim_start_matches(['/', '\\']);
    let encoded = format!("--{}--", trimmed.replace(['/', '\\', ':'], "-"));
    Some(
        home.join(".pi")
            .join("agent")
            .join("sessions")
            .join(encoded),
    )
}

/// Find the session JSONL Pi writes for our pinned id: `<ts>_<session_id>.jsonl`
/// in the project's session dir. Polls until it appears (Pi creates it lazily on
/// the first persisted entry) or the session ends; ~3 min backstop like the
/// codex rollout discovery.
async fn discover_session_file(
    store: &SessionStore,
    session_id: &str,
    cwd: &str,
) -> Option<std::path::PathBuf> {
    let dir = pi_session_dir(cwd)?;
    let suffix = format!("_{session_id}.jsonl");
    for _ in 0..720 {
        store.get(session_id)?;
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_str()
                    .is_some_and(|n| n.ends_with(&suffix))
                {
                    return Some(entry.path());
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    None
}

/// Collect the plain text out of a Pi message `content` (a string, or an array
/// of blocks where text blocks carry `text`).
fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Translate one Pi session-file entry into typed updates. Pure and total.
///
/// Session entries are whole units appended after the fact (`{type:"message",
/// message:{…}}` and friends) — unlike the RPC stream there are no start/end
/// lifecycle events, so busy/idle is inferred: a user message means a turn is
/// beginning (Busy); an assistant message that stopped for tool use means more
/// is coming (Busy); any other assistant stop means the turn is over (Idle).
pub fn translate_session_entry(entry: &Value) -> Vec<AgentUpdate> {
    let mut out = Vec::new();
    match entry.get("type").and_then(Value::as_str).unwrap_or("") {
        "message" => {
            let Some(message) = entry.get("message") else {
                return out;
            };
            match message.get("role").and_then(Value::as_str).unwrap_or("") {
                "user" => {
                    let text = content_text(message.get("content"));
                    if !text.trim().is_empty() {
                        out.push(AgentUpdate::UserText(text));
                    }
                    out.push(AgentUpdate::Busy);
                }
                "assistant" => {
                    for block in message
                        .get("content")
                        .and_then(Value::as_array)
                        .map(|a| a.as_slice())
                        .unwrap_or(&[])
                    {
                        match block.get("type").and_then(Value::as_str).unwrap_or("") {
                            "text" => {
                                if let Some(text) = block.get("text").and_then(Value::as_str) {
                                    if !text.trim().is_empty() {
                                        out.push(AgentUpdate::AssistantText(text.to_string()));
                                    }
                                }
                            }
                            "toolCall" | "tool_call" => {
                                let id = block
                                    .get("id")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string();
                                let name = block
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or("tool")
                                    .to_string();
                                let input = block
                                    .get("arguments")
                                    .or_else(|| block.get("args"))
                                    .cloned()
                                    .unwrap_or(Value::Null);
                                out.push(AgentUpdate::ToolUse { id, name, input });
                            }
                            _ => {} // thinking etc.
                        }
                    }
                    if let Some(u) = usage_from(message) {
                        out.push(u);
                    }
                    let stop = message
                        .get("stopReason")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if stop.to_ascii_lowercase().contains("tool") {
                        out.push(AgentUpdate::Busy); // tool round — more coming
                    } else {
                        out.push(AgentUpdate::Idle);
                    }
                }
                "toolResult" => {
                    let tool_use_id = message
                        .get("toolCallId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let content = {
                        let t = content_text(message.get("content"));
                        if t.is_empty() {
                            message
                                .get("output")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string()
                        } else {
                            t
                        }
                    };
                    let is_error = message
                        .get("isError")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    out.push(AgentUpdate::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    });
                }
                _ => {} // custom / bashExecution / branch summaries — not conversation
            }
        }
        "model_change" => {
            let provider = entry.get("provider").and_then(Value::as_str);
            let model_id = entry.get("modelId").and_then(Value::as_str);
            if let Some(m) = model_id {
                let full = match provider {
                    Some(p) => format!("{p}/{m}"),
                    None => m.to_string(),
                };
                out.push(AgentUpdate::Usage {
                    model: Some(full),
                    input_tokens: None,
                    output_tokens: None,
                    cost_usd: None,
                    context_tokens: None,
                    context_window: None,
                });
            }
        }
        _ => {} // session header, labels, compaction, thinking_level_change, …
    }
    out
}

/// Tail the Pi session JSONL by byte offset, folding each complete line through
/// [`translate_session_entry`] + the shared apply path. Replays already-written
/// history first (a resumed session repopulates the GUI), then polls for
/// appends. Ends when the session is deregistered or the file vanishes.
async fn tail_session_file(
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncSeekExt};
    let mut cur_mode = SessionMode::Input;
    let mut acc = UsageAcc::new();
    let mut offset: u64 = 0;
    let mut leftover = String::new();
    loop {
        if store.get(session_id).is_none() {
            break;
        }
        let Ok(mut file) = tokio::fs::File::open(path).await else {
            break;
        };
        let len = file.metadata().await.map(|m| m.len()).unwrap_or(offset);
        if len > offset {
            file.seek(std::io::SeekFrom::Start(offset)).await?;
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            loop {
                line.clear();
                let n = reader.read_line(&mut line).await?;
                if n == 0 {
                    break;
                }
                offset += n as u64;
                if !line.ends_with('\n') {
                    leftover = std::mem::take(&mut line);
                    offset -= leftover.len() as u64;
                    break;
                }
                let full = if leftover.is_empty() {
                    line.clone()
                } else {
                    format!("{}{}", std::mem::take(&mut leftover), line)
                };
                let trimmed = full.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
                    let updates = translate_session_entry(&value);
                    if !updates.is_empty() {
                        apply_updates(store, conv, session_id, updates, &mut cur_mode, &mut acc);
                    }
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
    Ok(())
}

/// Hybrid driver: native Pi TUI in a PTY (Term view) pinned to our session id,
/// GUI driven by tailing the session JSONL it writes. GUI prompts are pasted
/// into the TUI (bracketed paste, like the codex rollout fallback); the file
/// tail echoes them back as `UserText`, so no local echo here. Approvals happen
/// in the Term.
async fn run_tui_session(
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    cwd: &str,
    model: Option<String>,
    bin: &str,
) -> anyhow::Result<()> {
    let mut argv = vec![
        bin.to_string(),
        "--session-id".to_string(),
        session_id.to_string(),
    ];
    if let Some(m) = &model {
        argv.push("--model".to_string());
        argv.push(m.clone());
    }
    let tui = super::spawn_attach_pty(store, session_id, &argv, cwd).context("spawning pi TUI")?;

    // Ready for input as soon as the TUI is up (mode-gates the GUI composer).
    store.set_managed_mode(session_id, SessionMode::Input, None);

    // Drive the GUI from the session file (background; best-effort).
    {
        let store = store.clone();
        let conv = conv.clone();
        let sid = session_id.to_string();
        let cwd = cwd.to_string();
        tokio::spawn(async move {
            let Some(path) = discover_session_file(&store, &sid, &cwd).await else {
                tracing::warn!(session = %sid, "pi session file not found; GUI view will stay empty");
                return;
            };
            tracing::info!(session = %sid, path = %path.display(), "tailing pi session file");
            if let Err(err) = tail_session_file(&store, &conv, &sid, &path).await {
                tracing::warn!(?err, session = %sid, "pi session tail ended with error");
            }
        });
    }

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    store.register_managed_input(session_id, tx);
    // Approvals happen in the Term in this mode; accept and drop decisions so a
    // stray /approve can't wedge the caller.
    let (dtx, mut drx) = mpsc::unbounded_channel::<bool>();
    store.register_managed_decision(session_id, dtx);
    let mut tui_check = tokio::time::interval(std::time::Duration::from_secs(2));

    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Some(text) => {
                    // Bracketed paste + Enter so the TUI submits it as one message.
                    let body = text.trim_end_matches(['\r', '\n']);
                    let mut bytes = b"\x1b[200~".to_vec();
                    bytes.extend_from_slice(body.as_bytes());
                    bytes.extend_from_slice(b"\x1b[201~\r");
                    let _ = crate::wrapper::pty::write_bytes(&tui, &bytes).await;
                }
                None => break, // managed input dropped → terminated
            },
            decision = drx.recv() => match decision {
                Some(_) => {} // approvals live in the Term here
                None => break,
            },
            _ = tui_check.tick() => {
                if crate::wrapper::pty::has_exited(&tui) {
                    tracing::info!(session = %session_id, "pi TUI exited; tearing down session");
                    break;
                }
            },
        }
    }

    let _ = crate::wrapper::pty::signal_child(&tui, crate::protocol::Signal::Sigkill);
    Ok(())
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
    // Register the workspacer MCP facade (supervisors) before the agent boots.
    if let Some(mcp_url) = &facade.mcp_url {
        write_pi_mcp(cwd, mcp_url);
    }

    let mut cmd = Command::new(bin);
    cmd.arg("--mode").arg("rpc");
    if let Some(m) = &model {
        cmd.arg("--model").arg(m);
    }
    let mut child = cmd
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning `{bin} --mode rpc`"))?;

    let mut stdin = child.stdin.take().context("pi rpc: no stdin")?;
    let stdout = child.stdout.take().context("pi rpc: no stdout")?;
    // Pi requires splitting on LF only; tokio's line reader does exactly that
    // (and trims a trailing CR), never on U+2028/U+2029 — safe per its spec.
    let mut lines = BufReader::new(stdout).lines();

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    store.register_managed_input(session_id, tx);
    let (dtx, mut drx) = mpsc::unbounded_channel::<bool>();
    store.register_managed_decision(session_id, dtx);
    // Live model switch (POST /sessions/:id/model): Pi's RPC applies it mid-session
    // via `set_model`, so subsequent turns use the new model. The stdin is ready
    // the instant the process is up and this loop owns it, so there's no boot-join
    // race like codex's — a switch is simply written when it arrives. NOTE: the
    // hybrid TUI path (`run_tui_session`, the default non-supervisor session) has
    // no RPC channel and registers none of this, so it 409s and falls back to a
    // restart — the capability cliff `providerCaps.pi` documents.
    let (mtx, mut mrx) = mpsc::unbounded_channel::<ModelSwitch>();
    store.register_managed_model_switch(session_id, mtx);
    // Approval policy, live-switchable via `/permission-mode`: the adapter answers
    // every `extension_ui_request` dialog, so flipping this flag changes whether
    // the next one auto-accepts. Pi's core never bypasses at the source (approvals
    // come from a permission extension we always mediate), so yolo→ask works too —
    // spawned_yolo is always false.
    let yolo_live = Arc::new(AtomicBool::new(yolo));
    store.register_managed_yolo(session_id, yolo_live.clone(), false);

    let mut cur_mode = SessionMode::Input;
    let mut acc = UsageAcc::new();
    // Extension UI requests awaiting the user's decision (non-YOLO), FIFO. We park
    // each whole request so we can echo its `id` + `method` in the reply; a queue
    // (not one slot) so concurrent requests don't drop each other and stall.
    let mut pending_approvals: std::collections::VecDeque<Value> =
        std::collections::VecDeque::new();
    // Role instructions to prepend to the first turn only (supervisors).
    let mut pending_instructions: Option<String> = facade.instructions.clone();

    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&line) {
                        handle_message(
                            &value, store, conv, session_id, &mut stdin,
                            &mut cur_mode, &mut acc, &yolo_live, &mut pending_approvals,
                        ).await;
                    }
                }
                Ok(None) => break, // stdout closed → process gone
                Err(err) => return Err(err.into()),
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
                    let _ = write_msg(&mut stdin, &json!({ "type": "prompt", "message": sent })).await;
                }
                None => break, // managed input dropped → terminated
            },
            decision = drx.recv() => match decision {
                Some(approve) => {
                    if let Some(req) = pending_approvals.pop_front() {
                        let _ = respond_ui(&mut stdin, &req, approve).await;
                        store.set_managed_mode(session_id, SessionMode::Responding, None);
                        cur_mode = SessionMode::Responding;
                    }
                }
                None => break,
            },
            switch = mrx.recv() => match switch {
                Some(sw) => {
                    // Pi has thinking levels, not an "effort" knob, so effort is
                    // ignored (providerCaps.pi has no effort control). Send the
                    // model change and reflect it on the status line right away —
                    // the `set_model` response is a plain ack we don't translate.
                    if let Some(m) = sw.model {
                        if let Some(cmd) = set_model_msg(&m) {
                            let _ = write_msg(&mut stdin, &cmd).await;
                            apply_updates(
                                store, conv, session_id,
                                vec![AgentUpdate::Usage {
                                    model: Some(m),
                                    input_tokens: None,
                                    output_tokens: None,
                                    cost_usd: None,
                                    context_tokens: None,
                                    context_window: None,
                                }],
                                &mut cur_mode, &mut acc,
                            );
                        }
                    }
                }
                None => break,
            },
            status = child.wait() => {
                tracing::info!(?status, session = %session_id, "pi rpc exited");
                break;
            }
        }
    }

    let _ = child.start_kill();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_message(
    value: &Value,
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    stdin: &mut ChildStdin,
    cur_mode: &mut SessionMode,
    acc: &mut UsageAcc,
    yolo: &AtomicBool,
    pending_approvals: &mut std::collections::VecDeque<Value>,
) {
    let ty = value.get("type").and_then(Value::as_str).unwrap_or("");

    // Command acknowledgements (`{"type":"response", ...}`) carry no telemetry.
    if ty == "response" {
        return;
    }

    // Extension UI requests are server→client *requests* that must be answered
    // or the agent blocks. Dialog methods (confirm/select/input/editor) are the
    // approval seam; fire-and-forget methods (notify/setStatus/…) need no reply.
    if ty == "extension_ui_request" {
        let method = value.get("method").and_then(Value::as_str).unwrap_or("");
        let is_dialog = matches!(method, "confirm" | "select" | "input" | "editor");
        if !is_dialog {
            return; // notify / setStatus / setWidget / setTitle / set_editor_text
        }
        if yolo.load(Ordering::Relaxed) {
            // Auto-accept so the agent keeps working without surfacing a card.
            let _ = respond_ui(stdin, value, true).await;
        } else {
            // Surface the approval and remember the request so /approve can
            // forward the user's decision (see the decision branch above).
            let updates = vec![AgentUpdate::PermissionPending {
                id: None,
                tool: Some(method.to_string()),
                summary: dialog_summary(value),
                raw: value.clone(),
            }];
            apply_updates(store, conv, session_id, updates, cur_mode, acc);
            pending_approvals.push_back(value.clone());
        }
        return;
    }

    let updates = translate(value);
    if !updates.is_empty() {
        apply_updates(store, conv, session_id, updates, cur_mode, acc);
    }
}

/// Answer an Extension UI dialog request. `confirm` takes a boolean; `select`
/// resolves to an "allow"-ish option (or cancels on reject); `input`/`editor`
/// can't be synthesized meaningfully, so accept yields empty text and reject
/// cancels.
async fn respond_ui(stdin: &mut ChildStdin, req: &Value, approve: bool) -> anyhow::Result<()> {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("confirm");
    let mut msg = json!({ "type": "extension_ui_response", "id": id });
    match method {
        "confirm" => {
            msg["confirmed"] = json!(approve);
        }
        "select" if approve => {
            // Prefer an option whose label reads like an allow; fall back to the
            // first option offered.
            let opts = req.get("options").and_then(Value::as_array);
            let value = opts
                .and_then(|o| {
                    o.iter().filter_map(Value::as_str).find(|s| {
                        let l = s.to_ascii_lowercase();
                        l.contains("allow") || l.contains("yes") || l.contains("accept")
                    })
                })
                .or_else(|| opts.and_then(|o| o.first()).and_then(Value::as_str))
                .unwrap_or("Allow");
            msg["value"] = json!(value);
        }
        "input" | "editor" if approve => {
            msg["value"] = json!("");
        }
        _ => {
            msg["cancelled"] = json!(true);
        }
    }
    write_msg(stdin, &msg).await
}

/// Write one RPC message as a single LF-delimited JSON line.
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
    fn agent_start_is_busy() {
        assert_eq!(
            translate(&json!({ "type": "agent_start" })),
            vec![AgentUpdate::Busy]
        );
        assert_eq!(
            translate(&json!({ "type": "turn_start" })),
            vec![AgentUpdate::Busy]
        );
    }

    #[test]
    fn agent_end_is_idle() {
        assert_eq!(
            translate(&json!({ "type": "agent_end" })),
            vec![AgentUpdate::Idle]
        );
    }

    #[test]
    fn turn_end_does_not_idle_but_pulls_usage() {
        // turn_end is one round inside the loop — must not flip to Input.
        let ev =
            json!({ "type": "turn_end", "message": { "usage": { "input": 10, "output": 2 } } });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::Usage {
                model: None,
                input_tokens: Some(10),
                output_tokens: Some(2),
                cost_usd: None,
                context_tokens: Some(12),
                context_window: None,
            }]
        );
        // No message → no updates (and still not Idle).
        assert!(translate(&json!({ "type": "turn_end" })).is_empty());
    }

    #[test]
    fn message_update_text_delta_is_busy_plus_text() {
        let ev = json!({
            "type": "message_update",
            "assistantMessageEvent": { "type": "text_delta", "contentIndex": 0, "delta": "Hello " }
        });
        assert_eq!(
            translate(&ev),
            vec![
                AgentUpdate::Busy,
                AgentUpdate::AssistantText("Hello ".into())
            ]
        );
    }

    #[test]
    fn message_update_non_text_delta_is_just_busy() {
        let ev = json!({
            "type": "message_update",
            "assistantMessageEvent": { "type": "thinking_delta", "delta": "hmm" }
        });
        assert_eq!(translate(&ev), vec![AgentUpdate::Busy]);
    }

    #[test]
    fn tool_execution_start_is_tool_use() {
        let ev = json!({
            "type": "tool_execution_start",
            "toolCallId": "call_123",
            "toolName": "bash",
            "args": { "command": "ls -la" }
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::ToolUse {
                id: "call_123".into(),
                name: "bash".into(),
                input: json!({ "command": "ls -la" }),
            }]
        );
    }

    #[test]
    fn tool_execution_end_is_ignored() {
        let ev = json!({ "type": "tool_execution_end", "toolCallId": "call_123", "result": {} });
        assert!(translate(&ev).is_empty());
    }

    #[test]
    fn message_end_extracts_usage_and_model() {
        let ev = json!({
            "type": "message_end",
            "message": {
                "role": "assistant",
                "model": "claude-sonnet-4-20250514",
                "usage": { "input": 1200, "output": 340, "cost": 0.0123 }
            }
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::Usage {
                model: Some("claude-sonnet-4-20250514".into()),
                input_tokens: Some(1200),
                output_tokens: Some(340),
                cost_usd: Some(0.0123),
                context_tokens: Some(1540),
                context_window: None,
            }]
        );
    }

    #[test]
    fn usage_accepts_camelcase_token_keys() {
        let ev = json!({
            "type": "message_end",
            "message": { "usage": { "inputTokens": 5, "outputTokens": 7 } }
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::Usage {
                model: None,
                input_tokens: Some(5),
                output_tokens: Some(7),
                cost_usd: None,
                context_tokens: Some(12),
                context_window: None,
            }]
        );
    }

    #[test]
    fn extension_error_maps_to_error() {
        let ev = json!({ "type": "extension_error", "error": "boom" });
        assert_eq!(translate(&ev), vec![AgentUpdate::Error("boom".into())]);
    }

    #[test]
    fn extension_ui_request_is_not_translated_here() {
        // Handled out of band (needs a reply) — translate stays silent.
        let ev = json!({ "type": "extension_ui_request", "id": "u1", "method": "confirm", "title": "Run?" });
        assert!(translate(&ev).is_empty());
    }

    #[test]
    fn unknown_and_malformed_events_are_ignored() {
        assert!(translate(&json!({ "type": "queue_update", "steering": [] })).is_empty());
        assert!(translate(&json!({})).is_empty());
        assert!(translate(&Value::Null).is_empty());
    }

    // ── session-file entries (the hybrid's GUI feed) ──

    fn msg_entry(message: Value) -> Value {
        json!({ "type": "message", "id": "e1", "parentId": null, "timestamp": "2026-07-01T00:00:00Z", "message": message })
    }

    #[test]
    fn session_user_message_is_text_plus_busy() {
        let e = msg_entry(json!({ "role": "user", "content": "fix the tests" }));
        assert_eq!(
            translate_session_entry(&e),
            vec![
                AgentUpdate::UserText("fix the tests".into()),
                AgentUpdate::Busy
            ]
        );
        // Block-array content works too.
        let e = msg_entry(json!({ "role": "user", "content": [{ "type": "text", "text": "hi" }] }));
        assert_eq!(
            translate_session_entry(&e),
            vec![AgentUpdate::UserText("hi".into()), AgentUpdate::Busy]
        );
    }

    #[test]
    fn session_assistant_message_emits_text_tools_usage_and_mode() {
        let e = msg_entry(json!({
            "role": "assistant",
            "model": "anthropic/claude-sonnet-4-5",
            "stopReason": "toolUse",
            "content": [
                { "type": "thinking", "thinking": "hmm" },
                { "type": "text", "text": "Running it." },
                { "type": "toolCall", "id": "tc1", "name": "bash", "arguments": { "command": "ls" } }
            ],
            "usage": { "input": 100, "output": 20, "cacheRead": 400, "cacheWrite": 30, "cost": 0.01 }
        }));
        assert_eq!(
            translate_session_entry(&e),
            vec![
                AgentUpdate::AssistantText("Running it.".into()),
                AgentUpdate::ToolUse {
                    id: "tc1".into(),
                    name: "bash".into(),
                    input: json!({ "command": "ls" })
                },
                AgentUpdate::Usage {
                    model: Some("anthropic/claude-sonnet-4-5".into()),
                    input_tokens: Some(100),
                    output_tokens: Some(20),
                    cost_usd: Some(0.01),
                    // input + output + cacheRead + cacheWrite
                    context_tokens: Some(550),
                    context_window: None,
                },
                AgentUpdate::Busy, // stopReason toolUse → more coming
            ]
        );
    }

    #[test]
    fn session_assistant_final_message_idles() {
        let e = msg_entry(json!({
            "role": "assistant",
            "stopReason": "stop",
            "content": [{ "type": "text", "text": "Done." }],
            "usage": { "input": 10, "output": 5, "totalTokens": 900 }
        }));
        let updates = translate_session_entry(&e);
        assert_eq!(updates.last(), Some(&AgentUpdate::Idle));
        // totalTokens wins as the context occupancy when present.
        assert!(updates.iter().any(|u| matches!(
            u,
            AgentUpdate::Usage {
                context_tokens: Some(900),
                ..
            }
        )));
    }

    #[test]
    fn session_tool_result_maps_to_tool_result() {
        let e = msg_entry(json!({
            "role": "toolResult",
            "toolCallId": "tc1",
            "content": [{ "type": "text", "text": "file1\nfile2" }],
            "isError": false
        }));
        assert_eq!(
            translate_session_entry(&e),
            vec![AgentUpdate::ToolResult {
                tool_use_id: "tc1".into(),
                content: "file1\nfile2".into(),
                is_error: false
            }]
        );
    }

    #[test]
    fn session_model_change_updates_the_model() {
        let e = json!({ "type": "model_change", "id": "e2", "provider": "anthropic", "modelId": "claude-opus-4-8" });
        assert_eq!(
            translate_session_entry(&e),
            vec![AgentUpdate::Usage {
                model: Some("anthropic/claude-opus-4-8".into()),
                input_tokens: None,
                output_tokens: None,
                cost_usd: None,
                context_tokens: None,
                context_window: None,
            }]
        );
    }

    #[test]
    fn session_noise_entries_are_ignored() {
        assert!(
            translate_session_entry(&json!({ "type": "session", "id": "s", "cwd": "/x" }))
                .is_empty()
        );
        assert!(translate_session_entry(&json!({ "type": "label", "targetId": "e1" })).is_empty());
        assert!(
            translate_session_entry(&json!({ "type": "compaction", "summary": "…" })).is_empty()
        );
        assert!(
            translate_session_entry(&msg_entry(json!({ "role": "custom", "content": "x" })))
                .is_empty()
        );
    }

    #[test]
    fn pi_session_dir_encodes_cwd_like_pi_does() {
        let dir = pi_session_dir("/home/user/Work/repo").unwrap();
        assert!(dir.ends_with(".pi/agent/sessions/--home-user-Work-repo--"));
    }

    #[test]
    fn set_model_msg_splits_provider_and_model() {
        // The RPC `set_model` wants provider + modelId as separate fields.
        assert_eq!(
            set_model_msg("anthropic/claude-sonnet-4-5"),
            Some(
                json!({ "type": "set_model", "provider": "anthropic", "modelId": "claude-sonnet-4-5" })
            )
        );
        // Only the first slash splits; the rest stays in the model id.
        assert_eq!(
            set_model_msg("openrouter/meta-llama/llama-3.1"),
            Some(
                json!({ "type": "set_model", "provider": "openrouter", "modelId": "meta-llama/llama-3.1" })
            )
        );
        // No provider prefix, or an empty half → no switch (don't send garbage).
        assert!(set_model_msg("bare-model").is_none());
        assert!(set_model_msg("/x").is_none());
        assert!(set_model_msg("x/").is_none());
    }
}
