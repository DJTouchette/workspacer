//! Claude *stream* adapter — drive a headless Claude Code session over the
//! CLI's stream-json transport instead of a PTY.
//!
//! `claude --print --input-format stream-json --output-format stream-json
//! --include-partial-messages --verbose --permission-prompt-tool stdio` speaks
//! newline-delimited JSON on stdin/stdout: user messages go in as
//! `{"type":"user",…}` lines, events come out typed (`system` / `stream_event`
//! / `assistant` / `user` / `result` / `rate_limit_event`), and a bidirectional
//! *control protocol* rides the same pipes — the CLI asks us
//! `can_use_tool` (approvals **and** `AskUserQuestion`) as a
//! `control_request` we must answer, and we send it `interrupt` /
//! `set_permission_mode` / `set_model` requests it answers with a
//! `control_response`. All shapes below were captured live against CLI
//! 2.1.201 (see the spike notes in the repo memory).
//!
//! This is a **second transport for the same provider** (`provider: "claude"`,
//! `transport: "stream"`): the PTY path stays the default and untouched. Claude
//! Code still runs the user's hooks for headless sessions, so
//! `SessionStore::ingest` keeps them enrichment-only for stream sessions —
//! the driver owns the mode state machine here.
//!
//! The pure `translate` (event → [`AgentUpdate`]s) is unit-tested; the live
//! driver needs a real `claude` binary.

use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Context;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use super::{apply_updates, AgentUpdate, Facade, UsageAcc};
use crate::session::conversation::ConversationItem;
use crate::session::state::{Pending, PendingQuestion, SessionMode};
use crate::session::store::{ManagedAnswer, ManagedPermissionSwitch};
use crate::session::transcript::{blocks, flatten_tool_result, Block};
use crate::session::{ConversationStore, SessionStore};

/// Spawn-time settings for a stream session (the `/sessions/spawn-managed`
/// payload slice this driver consumes).
#[derive(Clone)]
pub struct SpawnConfig {
    pub session_id: String,
    pub cwd: String,
    /// Resolved `claude` binary (falls back to `"claude"` upstream).
    pub bin: String,
    pub model: Option<String>,
    /// Claude's own `--permission-mode` vocabulary (`acceptEdits`, `plan`, …).
    pub permission_mode: Option<String>,
    /// Resume this prior session id (`--resume <id>`) instead of pinning a
    /// fresh one (`--session-id <session_id>`).
    pub resume: Option<String>,
    /// Extra argv appended verbatim (escape hatch for new CLI flags).
    pub extra_args: Vec<String>,
    /// Extra env vars merged on top of the daemon's environment (e.g. a Claude
    /// profile's `CLAUDE_CONFIG_DIR`) — same semantics as the PTY spawn's `env`.
    pub env: HashMap<String, String>,
    /// Skip approvals at the source (`--dangerously-skip-permissions`) AND
    /// auto-answer any `can_use_tool` that still arrives.
    pub yolo: bool,
    pub facade: Facade,
}

// ── Pure translation ─────────────────────────────────────────────────────────

/// Running token totals across the session. The CLI's `result` events carry
/// *per-turn* token counts (verified: turn 1 `input_tokens: 18`, turn 2 `0`,
/// turn 3 `10`), so the driver accumulates them here to feed the status line's
/// cumulative readout. Cost needs no accumulation — `total_cost_usd` is
/// already session-cumulative on the wire.
#[derive(Debug, Default)]
pub struct StreamTotals {
    input: u64,
    output: u64,
}

/// Translate one stdout event into typed updates. Pure and total: unknown
/// types / missing fields yield an empty/partial result. `totals` accumulates
/// the per-turn token counts `result` events carry (see [`StreamTotals`]).
pub fn translate(value: &Value, totals: &mut StreamTotals) -> Vec<AgentUpdate> {
    let mut out = Vec::new();
    // Sub-agent (Task tool) traffic is tagged with the spawning tool_use id —
    // it belongs to the sub-agent's own run, not the main timeline (the same
    // filter the transcript tailer applies via `isSidechain`).
    if value
        .get("parent_tool_use_id")
        .is_some_and(|p| !p.is_null())
    {
        return out;
    }
    match value.get("type").and_then(Value::as_str).unwrap_or("") {
        "system" => match value.get("subtype").and_then(Value::as_str).unwrap_or("") {
            // Start of every turn: names the session's current model.
            "init" => {
                if let Some(model) = value.get("model").and_then(Value::as_str) {
                    out.push(usage_model(model));
                }
            }
            // The CLI is calling the API — the turn is running.
            "status" => {
                if value.get("status").and_then(Value::as_str) == Some("requesting") {
                    out.push(AgentUpdate::Busy);
                }
            }
            _ => {}
        },
        // Live streaming: text grows token by token. Thinking / signature /
        // tool-input deltas are skipped (tool calls land whole via the
        // `assistant` block events; thinking stays out of the conversation).
        "stream_event" => {
            let event = value.get("event").unwrap_or(&Value::Null);
            match event.get("type").and_then(Value::as_str).unwrap_or("") {
                "message_start" => {
                    out.push(AgentUpdate::Busy);
                    if let Some(model) = event
                        .get("message")
                        .and_then(|m| m.get("model"))
                        .and_then(Value::as_str)
                    {
                        out.push(usage_model(model));
                    }
                }
                "content_block_delta" => {
                    if let Some(text) = event
                        .get("delta")
                        .filter(|d| d.get("type").and_then(Value::as_str) == Some("text_delta"))
                        .and_then(|d| d.get("text"))
                        .and_then(Value::as_str)
                    {
                        if !text.is_empty() {
                            out.push(AgentUpdate::AssistantText(text.to_string()));
                        }
                    }
                }
                _ => {}
            }
        }
        // One completed content block per event (verified NOT cumulative).
        // Text blocks are skipped — their content already streamed in via
        // text_deltas above. Tool uses land here, whole; TodoWrite becomes the
        // session plan. The riding `usage` is the latest API call's request —
        // i.e. what's occupying the context window right now.
        "assistant" => {
            let Some(msg) = value.get("message") else {
                return out;
            };
            for b in blocks(msg.get("content").unwrap_or(&Value::Null)) {
                if let Block::ToolUse { name, input, id } = b {
                    if name == "TodoWrite" {
                        if let Some(plan) = super::plan_from_value(input) {
                            out.push(AgentUpdate::Plan(plan));
                        }
                        continue;
                    }
                    out.push(AgentUpdate::ToolUse {
                        id: id.map(str::to_owned).unwrap_or_default(),
                        name: name.to_string(),
                        input: input.clone(),
                    });
                }
            }
            if let Some(context_tokens) = context_tokens_from(msg.get("usage")) {
                out.push(AgentUpdate::Usage {
                    model: msg.get("model").and_then(Value::as_str).map(str::to_owned),
                    input_tokens: None,
                    output_tokens: None,
                    cost_usd: None,
                    context_tokens: Some(context_tokens),
                    context_window: None,
                });
            }
        }
        // Tool results ride back as synthetic user messages. Plain user text
        // here is an echo of what we already pushed on send — skip it.
        "user" => {
            let Some(msg) = value.get("message") else {
                return out;
            };
            for b in blocks(msg.get("content").unwrap_or(&Value::Null)) {
                if let Block::ToolResult {
                    content,
                    is_error,
                    tool_use_id: Some(tid),
                } = b
                {
                    out.push(AgentUpdate::ToolResult {
                        tool_use_id: tid.to_string(),
                        content: flatten_tool_result(content),
                        is_error,
                    });
                }
            }
        }
        // Turn over: ready for input again, plus the turn's telemetry.
        // `total_cost_usd` is cumulative; token counts are per-turn and get
        // accumulated into `totals`. An interrupted turn reports
        // `error_during_execution` with `terminal_reason: "aborted_streaming"`
        // — that's the user's own Esc, not an error worth surfacing.
        "result" => {
            let usage = value.get("usage");
            let pick = |k: &str| usage.and_then(|u| u.get(k)).and_then(Value::as_u64);
            totals.input += pick("input_tokens").unwrap_or(0);
            totals.output += pick("output_tokens").unwrap_or(0);
            let context_window =
                value
                    .get("modelUsage")
                    .and_then(Value::as_object)
                    .and_then(|mu| {
                        mu.values()
                            .filter_map(|m| m.get("contextWindow").and_then(Value::as_u64))
                            .max()
                    });
            out.push(AgentUpdate::Usage {
                model: None,
                input_tokens: Some(totals.input),
                output_tokens: Some(totals.output),
                cost_usd: value.get("total_cost_usd").and_then(Value::as_f64),
                context_tokens: None,
                context_window,
            });
            let aborted =
                value.get("terminal_reason").and_then(Value::as_str) == Some("aborted_streaming");
            if value
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && !aborted
            {
                let msg = value
                    .get("errors")
                    .and_then(Value::as_array)
                    .and_then(|e| e.first())
                    .and_then(Value::as_str)
                    .or_else(|| value.get("result").and_then(Value::as_str))
                    .or_else(|| value.get("subtype").and_then(Value::as_str))
                    .unwrap_or("turn failed");
                out.push(AgentUpdate::Error(msg.to_string()));
            }
            out.push(AgentUpdate::Idle);
        }
        // The account window currently binding this session. One window per
        // event (`rateLimitType` names it); utilization is a 0–100 percent.
        "rate_limit_event" => {
            let info = value.get("rate_limit_info").unwrap_or(&Value::Null);
            let pct = info.get("utilization").and_then(Value::as_f64);
            let resets = info.get("resetsAt").and_then(Value::as_i64);
            if pct.is_some() || resets.is_some() {
                let seven_day = info
                    .get("rateLimitType")
                    .and_then(Value::as_str)
                    .is_some_and(|t| t.starts_with("seven_day"));
                out.push(if seven_day {
                    AgentUpdate::RateLimits {
                        five_hour_pct: None,
                        five_hour_resets_at: None,
                        seven_day_pct: pct,
                        seven_day_resets_at: resets,
                    }
                } else {
                    AgentUpdate::RateLimits {
                        five_hour_pct: pct,
                        five_hour_resets_at: resets,
                        seven_day_pct: None,
                        seven_day_resets_at: None,
                    }
                });
            }
        }
        _ => {}
    }
    out
}

fn usage_model(model: &str) -> AgentUpdate {
    AgentUpdate::Usage {
        model: Some(model.to_string()),
        input_tokens: None,
        output_tokens: None,
        cost_usd: None,
        context_tokens: None,
        context_window: None,
    }
}

/// Context occupancy from an assistant message's `usage`: everything the last
/// API request put in the window (fresh + cached input + output).
fn context_tokens_from(usage: Option<&Value>) -> Option<u64> {
    let u = usage?;
    let get = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    let total = get("input_tokens")
        + get("cache_read_input_tokens")
        + get("cache_creation_input_tokens")
        + get("output_tokens");
    (total > 0).then_some(total)
}

// ── Control protocol ─────────────────────────────────────────────────────────

/// A `can_use_tool` control request parked while the user decides. Holds
/// everything needed to answer it: the CLI's request id and the tool's input
/// (an allow must echo `updatedInput`, and an AskUserQuestion answer is the
/// input plus the chosen `answers`) — plus the approval-card display fields
/// (`tool`/`summary`/`raw`), so a request parked *behind* the displayed one
/// can be re-surfaced when it reaches the front of the queue. The display
/// fields are unused for the separately-parked AskUserQuestion.
#[derive(Debug)]
struct ParkedCanUse {
    request_id: Value,
    input: Value,
    tool: Option<String>,
    summary: Option<String>,
    raw: Value,
}

/// What we're waiting on for a control request *we* sent to the CLI.
#[derive(Debug)]
enum PendingControl {
    Initialize,
    Interrupt,
    SetModel { model: String },
    SetPermissionMode(ManagedPermissionSwitch),
}

/// `{"type":"control_response","response":{"subtype":"success","request_id":…,
/// "response":{…}}}` — the answer shape the CLI expects for its
/// `control_request`s (verified on the wire).
fn control_success(request_id: &Value, response: Value) -> Value {
    json!({
        "type": "control_response",
        "response": { "subtype": "success", "request_id": request_id, "response": response }
    })
}

fn control_error(request_id: &Value, error: &str) -> Value {
    json!({
        "type": "control_response",
        "response": { "subtype": "error", "request_id": request_id, "error": error }
    })
}

/// Answer a parked `AskUserQuestion` the way the CLI's own picker does
/// (verified: allow with `updatedInput = {…input, answers: {question →
/// chosen label}}` yields the `Your questions have been answered: …`
/// tool_result). Numeric answers are 1-indexed option picks (mapped to the
/// option's label); anything else passes through as free text — the tool
/// accepts custom answers ("Other").
fn answered_input(input: &Value, ans: &ManagedAnswer) -> Value {
    let questions = input
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let resolve = |q: &Value, raw: &str| -> String {
        if let Ok(n) = raw.trim().parse::<usize>() {
            if let Some(label) = q
                .get("options")
                .and_then(Value::as_array)
                .filter(|opts| n >= 1 && n <= opts.len())
                .and_then(|opts| opts[n - 1].get("label"))
                .and_then(Value::as_str)
            {
                return label.to_string();
            }
        }
        raw.to_string()
    };
    let mut answers = serde_json::Map::new();
    let mut record = |q: &Value, raw: &str| {
        if let Some(question) = q.get("question").and_then(Value::as_str) {
            answers.insert(question.to_string(), Value::String(resolve(q, raw)));
        }
    };
    if let Some(list) = &ans.answers {
        for (q, raw) in questions.iter().zip(list) {
            record(q, raw);
        }
    } else if let Some(first) = questions.first() {
        if let Some(opt) = ans.option {
            record(first, &opt.to_string());
        } else if let Some(text) = &ans.text {
            record(first, text);
        }
    }
    let mut out = input.clone();
    out["answers"] = Value::Object(answers);
    out
}

/// The questions inside an AskUserQuestion input, parsed with the same serde
/// shape the hook path uses (`SessionState::apply`), so both transports
/// surface an identical `Pending::Question`.
fn questions_from(input: &Value) -> Vec<PendingQuestion> {
    input
        .get("questions")
        .cloned()
        .and_then(|v| serde_json::from_value::<Vec<PendingQuestion>>(v).ok())
        .unwrap_or_default()
}

/// A `Pending::Approval` raw payload in the same shape hook approvals carry
/// (`tool_name` / `tool_input` / …) so the GUI's existing approval card
/// renders stream approvals identically.
fn approval_raw(request: &Value) -> Value {
    json!({
        "tool_name": request.get("tool_name").cloned().unwrap_or(Value::Null),
        "tool_input": request.get("input").cloned().unwrap_or(Value::Null),
        "tool_use_id": request.get("tool_use_id").cloned().unwrap_or(Value::Null),
        "description": request.get("description").cloned().unwrap_or(Value::Null),
        "permission_suggestions": request.get("permission_suggestions").cloned().unwrap_or(Value::Null),
    })
}

/// Short human summary for an approval card (command text for Bash, the
/// tool's own description otherwise).
fn approval_summary(request: &Value) -> Option<String> {
    let input = request.get("input");
    input
        .and_then(|i| i.get("command"))
        .and_then(Value::as_str)
        .or_else(|| request.get("description").and_then(Value::as_str))
        .or_else(|| {
            input
                .and_then(|i| i.get("file_path"))
                .and_then(Value::as_str)
        })
        .map(str::to_owned)
}

/// Surface a parked approval as the session's pending card. The store holds a
/// single pending slot, so only the *front* of `pending_approvals` is ever
/// displayed — later requests wait parked and are re-surfaced here when the
/// front is answered, keeping the displayed card and the FIFO answer in sync.
fn surface_approval(
    store: &SessionStore,
    session_id: &str,
    cur_mode: &mut SessionMode,
    parked: &ParkedCanUse,
) {
    store.set_managed_mode(
        session_id,
        SessionMode::Approval,
        Some(Pending::Approval {
            tool: parked.tool.clone(),
            summary: parked.summary.clone(),
            raw: parked.raw.clone(),
        }),
    );
    *cur_mode = SessionMode::Approval;
}

// ── Live driver ──────────────────────────────────────────────────────────────

/// Spawn and drive a stream-transport Claude session in the background.
/// Returns immediately; the session id is already registered in `store` (with
/// `transport: Stream`) by the caller.
pub fn spawn_session(store: SessionStore, conv: ConversationStore, cfg: SpawnConfig) {
    tokio::spawn(async move {
        let session_id = cfg.session_id.clone();
        if let Err(err) = run_session(&store, &conv, cfg).await {
            tracing::warn!(?err, session = %session_id, "claude stream session ended with error");
        }
        // Child gone → a Stopped, resumable row (same lifecycle as PTY spawns).
        store.deregister_managed(&session_id);
        conv.forget(&session_id);
    });
}

/// The exact headless argv, per the verified contract. `--verbose` is required
/// for stream-json output in print mode; `--permission-prompt-tool stdio`
/// routes approvals to us as `can_use_tool` control requests.
fn build_argv(cfg: &SpawnConfig) -> Vec<String> {
    let mut argv: Vec<String> = [
        "--print",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--include-partial-messages",
        "--verbose",
        "--permission-prompt-tool",
        "stdio",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    match &cfg.resume {
        // Resuming reopens the prior conversation; the CLI keeps its id.
        Some(prior) => argv.extend(["--resume".into(), prior.clone()]),
        // Fresh session: pin our row id as claude's own session id, so the
        // hook ids and the transcript filename all agree with no aliasing.
        None => argv.extend(["--session-id".into(), cfg.session_id.clone()]),
    }
    if let Some(model) = &cfg.model {
        argv.extend(["--model".into(), model.clone()]);
    }
    if let Some(mode) = &cfg.permission_mode {
        argv.extend(["--permission-mode".into(), mode.clone()]);
    }
    if cfg.yolo {
        argv.push("--dangerously-skip-permissions".into());
    }
    // Register the workspacer MCP facade (supervisors) as an inline config.
    if let Some(mcp_url) = &cfg.facade.mcp_url {
        let config = json!({ "mcpServers": { "workspacer": { "type": "http", "url": mcp_url } } });
        argv.extend(["--mcp-config".into(), config.to_string()]);
    }
    argv.extend(cfg.extra_args.iter().cloned());
    argv
}

async fn run_session(
    store: &SessionStore,
    conv: &ConversationStore,
    cfg: SpawnConfig,
) -> anyhow::Result<()> {
    let session_id = cfg.session_id.clone();
    let argv = build_argv(&cfg);
    let mut child = Command::new(&cfg.bin)
        .args(&argv)
        .envs(&cfg.env)
        .current_dir(&cfg.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning `{} {}`", cfg.bin, argv.join(" ")))?;

    let mut stdin = child.stdin.take().context("claude stream: no stdin")?;
    let stdout = child.stdout.take().context("claude stream: no stdout")?;
    let mut lines = BufReader::new(stdout).lines();

    // Surface the CLI's stderr in the daemon log — it's the only place launch
    // failures (bad flag, auth) explain themselves.
    if let Some(stderr) = child.stderr.take() {
        let sid = session_id.clone();
        tokio::spawn(async move {
            let mut err_lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = err_lines.next_line().await {
                tracing::debug!(session = %sid, line = %line, "claude stream stderr");
            }
        });
    }

    // Serialize all stdin writes through one task that owns the pipe, so the
    // several send sites (prompts, control responses, control requests) never
    // contend for the writer. Dropping `out_tx` (on return) closes stdin,
    // which is also how the CLI is told to wind down.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
    tokio::spawn(async move {
        while let Some(v) = out_rx.recv().await {
            let mut line = v.to_string();
            line.push('\n');
            if stdin.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            let _ = stdin.flush().await;
        }
        let _ = stdin.shutdown().await;
    });

    // Store-facing channels: prompts, approvals, answers, model switches,
    // permission modes, interrupts.
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    store.register_managed_input(&session_id, tx);
    let (dtx, mut drx) = mpsc::unbounded_channel::<bool>();
    store.register_managed_decision(&session_id, dtx);
    let (atx, mut arx) = mpsc::unbounded_channel::<ManagedAnswer>();
    store.register_managed_answer(&session_id, atx);
    let (mtx, mut mrx) = mpsc::unbounded_channel::<crate::session::ModelSwitch>();
    store.register_managed_model_switch(&session_id, mtx);
    let (ptx, mut prx) = mpsc::unbounded_channel::<ManagedPermissionSwitch>();
    store.register_managed_permission_mode(&session_id, ptx);
    let (itx, mut irx) = mpsc::unbounded_channel::<()>();
    store.register_managed_interrupt(&session_id, itx);
    // Auto-approve flag, live-switchable via the yolo half of
    // `/permission-mode`'s managed vocabulary as well as the structural
    // switch. A yolo *spawn* also bypasses at the source, but flipping this on
    // later auto-answers whatever `can_use_tool` still arrives.
    let yolo_live = Arc::new(AtomicBool::new(cfg.yolo));
    store.register_managed_yolo(&session_id, yolo_live.clone(), cfg.yolo);

    // The SDK-parity handshake. The CLI tolerates traffic before the response
    // lands, so nothing blocks on it.
    let mut next_ctl: u64 = 0;
    let mut pending_controls: HashMap<String, PendingControl> = HashMap::new();
    send_control(
        &out_tx,
        &mut next_ctl,
        &mut pending_controls,
        json!({ "subtype": "initialize", "hooks": null }),
        PendingControl::Initialize,
    );

    let mut cur_mode = SessionMode::Input;
    let mut acc = UsageAcc::new();
    acc.seed_model(cfg.model.as_deref());
    let mut totals = StreamTotals::default();
    // Approvals awaiting the user's /approve, FIFO (two requests can arrive
    // before the first is answered). AskUserQuestion parks separately — it's
    // answered with content, not a yes/no.
    let mut pending_approvals: VecDeque<ParkedCanUse> = VecDeque::new();
    let mut pending_question: Option<ParkedCanUse> = None;
    // Role instructions to prepend to the first prompt only (supervisors).
    let mut pending_instructions: Option<String> = cfg.facade.instructions.clone();

    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    let line = line.trim();
                    if line.is_empty() { continue; }
                    let Ok(value) = serde_json::from_str::<Value>(line) else {
                        tracing::debug!(session = %session_id, line = %line, "claude stream: unparsed stdout line");
                        continue;
                    };
                    handle_line(
                        &value, store, conv, &session_id, &out_tx,
                        &mut cur_mode, &mut acc, &mut totals,
                        &yolo_live, &mut pending_approvals, &mut pending_question,
                        &mut pending_controls,
                    );
                }
                Ok(None) => break, // stdout EOF — child is exiting
                Err(err) => return Err(err.into()),
            },
            msg = rx.recv() => match msg {
                Some(text) => {
                    // Echo the user's message verbatim, but prepend the role
                    // instructions (once) to what's actually sent to the agent.
                    conv.push(&session_id, vec![ConversationItem::UserMessage { text: text.clone(), timestamp: None }]);
                    let sent = match pending_instructions.take() {
                        Some(instr) => format!("{instr}\n\n{text}"),
                        None => text,
                    };
                    let _ = out_tx.send(json!({
                        "type": "user",
                        "message": { "role": "user", "content": [ { "type": "text", "text": sent } ] }
                    }));
                    if cur_mode != SessionMode::Responding {
                        store.set_managed_mode(&session_id, SessionMode::Responding, None);
                        cur_mode = SessionMode::Responding;
                    }
                }
                None => break, // managed input dropped → terminated
            },
            decision = drx.recv() => match decision {
                Some(approve) => {
                    if let Some(parked) = pending_approvals.pop_front() {
                        let response = if approve {
                            json!({ "behavior": "allow", "updatedInput": parked.input })
                        } else {
                            json!({ "behavior": "deny", "message": "The user denied this tool use.", "interrupt": false })
                        };
                        let _ = out_tx.send(control_success(&parked.request_id, response));
                        // Another request may have parked behind the one just
                        // answered (parallel tool calls) — surface it so the
                        // user gets its card; otherwise the turn resumes.
                        match pending_approvals.front() {
                            Some(next) => surface_approval(store, &session_id, &mut cur_mode, next),
                            None => {
                                store.set_managed_mode(&session_id, SessionMode::Responding, None);
                                cur_mode = SessionMode::Responding;
                            }
                        }
                    } else {
                        tracing::debug!(session = %session_id, "claude stream: decision with no parked approval — dropped");
                    }
                }
                None => break,
            },
            answer = arx.recv() => match answer {
                Some(ans) => {
                    if let Some(parked) = pending_question.take() {
                        let updated = answered_input(&parked.input, &ans);
                        let _ = out_tx.send(control_success(
                            &parked.request_id,
                            json!({ "behavior": "allow", "updatedInput": updated }),
                        ));
                        // An approval parked while the question was displayed
                        // must re-surface rather than being wiped to Responding.
                        match pending_approvals.front() {
                            Some(next) => surface_approval(store, &session_id, &mut cur_mode, next),
                            None => {
                                store.set_managed_mode(&session_id, SessionMode::Responding, None);
                                cur_mode = SessionMode::Responding;
                            }
                        }
                    }
                }
                None => break,
            },
            switch = mrx.recv() => match switch {
                Some(sw) => {
                    // `set_model` is real on this transport (verified: the next
                    // turn runs the new model). Claude has no effort knob —
                    // note and drop it rather than failing the whole switch.
                    if sw.effort.is_some() {
                        tracing::debug!(session = %session_id, "claude stream: `effort` has no equivalent — ignored");
                    }
                    match sw.model {
                        Some(model) => send_control(
                            &out_tx,
                            &mut next_ctl,
                            &mut pending_controls,
                            json!({ "subtype": "set_model", "model": model }),
                            PendingControl::SetModel { model },
                        ),
                        None => tracing::debug!(session = %session_id, "claude stream: model switch without a model — nothing to do"),
                    }
                }
                None => break,
            },
            psw = prx.recv() => match psw {
                Some(psw) => {
                    let request = json!({ "subtype": "set_permission_mode", "mode": psw.mode });
                    send_control(
                        &out_tx,
                        &mut next_ctl,
                        &mut pending_controls,
                        request,
                        PendingControl::SetPermissionMode(psw),
                    );
                }
                None => break,
            },
            _ = irx.recv() => {
                send_control(
                    &out_tx,
                    &mut next_ctl,
                    &mut pending_controls,
                    json!({ "subtype": "interrupt" }),
                    PendingControl::Interrupt,
                );
            },
            status = child.wait() => {
                tracing::info!(?status, session = %session_id, "claude stream child exited");
                // The unbiased select! can pick this arm while the CLI's final
                // lines (the `result` carrying the error/usage/Idle) still sit
                // buffered in the reader — drain them before winding down so
                // they aren't silently dropped. The child is gone, so its
                // stdout hits EOF instead of blocking.
                while let Ok(Some(line)) = lines.next_line().await {
                    let line = line.trim();
                    if line.is_empty() { continue; }
                    let Ok(value) = serde_json::from_str::<Value>(line) else { continue; };
                    handle_line(
                        &value, store, conv, &session_id, &out_tx,
                        &mut cur_mode, &mut acc, &mut totals,
                        &yolo_live, &mut pending_approvals, &mut pending_question,
                        &mut pending_controls,
                    );
                }
                break;
            }
        }
    }

    let _ = child.start_kill();
    Ok(())
}

/// Queue a control request to the CLI, parking what the eventual
/// `control_response` should resolve. Request ids are `wks-<n>` so they can't
/// collide with the CLI's own (UUIDs).
fn send_control(
    out_tx: &mpsc::UnboundedSender<Value>,
    next_ctl: &mut u64,
    pending_controls: &mut HashMap<String, PendingControl>,
    request: Value,
    pending: PendingControl,
) {
    *next_ctl += 1;
    let request_id = format!("wks-{next_ctl}");
    pending_controls.insert(request_id.clone(), pending);
    let _ = out_tx.send(json!({
        "type": "control_request", "request_id": request_id, "request": request
    }));
}

/// Handle one stdout line: our control responses, the CLI's control requests
/// (approvals / questions), and plain events via [`translate`].
#[allow(clippy::too_many_arguments)]
fn handle_line(
    value: &Value,
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    out_tx: &mpsc::UnboundedSender<Value>,
    cur_mode: &mut SessionMode,
    acc: &mut UsageAcc,
    totals: &mut StreamTotals,
    yolo: &AtomicBool,
    pending_approvals: &mut VecDeque<ParkedCanUse>,
    pending_question: &mut Option<ParkedCanUse>,
    pending_controls: &mut HashMap<String, PendingControl>,
) {
    match value.get("type").and_then(Value::as_str).unwrap_or("") {
        // The CLI answered one of our control requests.
        "control_response" => {
            let response = value.get("response").unwrap_or(&Value::Null);
            let Some(request_id) = response.get("request_id").and_then(Value::as_str) else {
                return;
            };
            let Some(pending) = pending_controls.remove(request_id) else {
                return;
            };
            let error =
                (response.get("subtype").and_then(Value::as_str) != Some("success")).then(|| {
                    response
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("control request failed")
                        .to_string()
                });
            match (pending, error) {
                (PendingControl::Initialize, None) => {
                    tracing::debug!(session = %session_id, "claude stream: initialized");
                }
                (PendingControl::Initialize, Some(err)) => {
                    tracing::warn!(session = %session_id, error = %err, "claude stream: initialize failed");
                }
                (PendingControl::Interrupt, err) => {
                    if let Some(err) = err {
                        tracing::warn!(session = %session_id, error = %err, "claude stream: interrupt failed");
                    }
                }
                (PendingControl::SetModel { model }, None) => {
                    tracing::info!(session = %session_id, model = %model, "claude stream: model switched");
                    apply_updates(
                        store,
                        conv,
                        session_id,
                        vec![usage_model(&model)],
                        cur_mode,
                        acc,
                    );
                }
                (PendingControl::SetModel { model }, Some(err)) => {
                    tracing::warn!(session = %session_id, model = %model, error = %err, "claude stream: model switch rejected");
                    apply_updates(
                        store,
                        conv,
                        session_id,
                        vec![AgentUpdate::Error(format!(
                            "model switch to '{model}' failed: {err}"
                        ))],
                        cur_mode,
                        acc,
                    );
                }
                (PendingControl::SetPermissionMode(psw), None) => {
                    // Success may confirm the mode back (verified: `{"mode":
                    // "acceptEdits"}`); fall back to what we asked for.
                    let confirmed = response
                        .get("response")
                        .and_then(|r| r.get("mode"))
                        .and_then(Value::as_str)
                        .unwrap_or(&psw.mode)
                        .to_string();
                    let _ = psw.reply.send(Ok(confirmed));
                }
                (PendingControl::SetPermissionMode(psw), Some(err)) => {
                    let _ = psw.reply.send(Err(err));
                }
            }
        }
        // The CLI is asking us something. `can_use_tool` is the one request
        // we answer meaningfully — approvals and AskUserQuestion both arrive
        // here. Anything else gets an explicit error so the CLI never blocks
        // on a response we'll never send.
        "control_request" => {
            let request_id = value.get("request_id").cloned().unwrap_or(Value::Null);
            let request = value.get("request").unwrap_or(&Value::Null);
            let subtype = request.get("subtype").and_then(Value::as_str).unwrap_or("");
            if subtype != "can_use_tool" {
                tracing::debug!(session = %session_id, subtype = %subtype, "claude stream: unsupported control request — declining");
                let _ = out_tx.send(control_error(
                    &request_id,
                    &format!("workspacer does not support '{subtype}'"),
                ));
                return;
            }
            let tool = request
                .get("tool_name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let input = request.get("input").cloned().unwrap_or(Value::Null);
            if tool == "AskUserQuestion" {
                // A question for the human — park it as `Question` even under
                // yolo (auto-approving it without answers would be nonsense).
                let questions = questions_from(&input);
                store.set_managed_mode(
                    session_id,
                    SessionMode::Question,
                    Some(Pending::Question {
                        questions,
                        raw: input.clone(),
                    }),
                );
                *cur_mode = SessionMode::Question;
                *pending_question = Some(ParkedCanUse {
                    request_id,
                    input,
                    tool: Some(tool.to_string()),
                    summary: None,
                    raw: Value::Null,
                });
                return;
            }
            if yolo.load(Ordering::Relaxed) {
                let _ = out_tx.send(control_success(
                    &request_id,
                    json!({ "behavior": "allow", "updatedInput": input }),
                ));
                return;
            }
            pending_approvals.push_back(ParkedCanUse {
                request_id,
                input,
                tool: Some(tool.to_string()),
                summary: approval_summary(request),
                raw: approval_raw(request),
            });
            // Only the queue head is displayed: the store's pending is a single
            // slot, so a request arriving while another is awaiting the user
            // stays parked and re-surfaces when the head is answered (the drx
            // branch) — the displayed card always matches the FIFO answer.
            if pending_approvals.len() == 1 {
                surface_approval(store, session_id, cur_mode, &pending_approvals[0]);
            }
        }
        _ => {
            // A `result` closes the turn. Any still-parked `can_use_tool`
            // requests belong to a turn the CLI abandoned (interrupt / fatal
            // error) and their request ids are dead — drop them so a later
            // `/approve` or `/answer` can't answer a canceled request.
            if value.get("type").and_then(Value::as_str) == Some("result") {
                pending_approvals.clear();
                *pending_question = None;
            }
            let updates = translate(value, totals);
            if !updates.is_empty() {
                apply_updates(store, conv, session_id, updates, cur_mode, acc);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::state::PlanStatus;

    fn t(value: Value) -> Vec<AgentUpdate> {
        translate(&value, &mut StreamTotals::default())
    }

    #[test]
    fn system_status_requesting_is_busy() {
        assert_eq!(
            t(json!({ "type": "system", "subtype": "status", "status": "requesting" })),
            vec![AgentUpdate::Busy]
        );
        assert!(t(json!({ "type": "system", "subtype": "status", "status": "idle" })).is_empty());
    }

    #[test]
    fn system_init_names_the_model() {
        assert_eq!(
            t(json!({ "type": "system", "subtype": "init", "model": "claude-haiku-4-5" })),
            vec![usage_model("claude-haiku-4-5")]
        );
    }

    #[test]
    fn message_start_is_busy_with_model() {
        let updates = t(json!({ "type": "stream_event", "event": {
            "type": "message_start", "message": { "model": "claude-sonnet-4-5" }
        }}));
        assert_eq!(
            updates,
            vec![AgentUpdate::Busy, usage_model("claude-sonnet-4-5")]
        );
    }

    #[test]
    fn text_delta_is_assistant_text_thinking_is_not() {
        assert_eq!(
            t(json!({ "type": "stream_event", "event": {
                "type": "content_block_delta", "delta": { "type": "text_delta", "text": "hi" }
            }})),
            vec![AgentUpdate::AssistantText("hi".into())]
        );
        assert!(t(json!({ "type": "stream_event", "event": {
            "type": "content_block_delta", "delta": { "type": "thinking_delta", "thinking": "hmm" }
        }}))
        .is_empty());
    }

    #[test]
    fn assistant_tool_use_maps_and_text_is_skipped() {
        // Text already streamed via deltas — re-emitting the whole block here
        // would double it.
        let updates = t(json!({ "type": "assistant", "message": { "content": [
            { "type": "text", "text": "already streamed" },
            { "type": "tool_use", "id": "tu1", "name": "Bash", "input": { "command": "ls" } }
        ]}}));
        assert_eq!(
            updates,
            vec![AgentUpdate::ToolUse {
                id: "tu1".into(),
                name: "Bash".into(),
                input: json!({ "command": "ls" }),
            }]
        );
    }

    #[test]
    fn assistant_todo_write_is_a_plan_not_a_tool_use() {
        let updates = t(json!({ "type": "assistant", "message": { "content": [
            { "type": "tool_use", "id": "tu1", "name": "TodoWrite", "input": { "todos": [
                { "content": "explore", "status": "completed" },
                { "content": "build", "status": "in_progress", "activeForm": "Building" }
            ]}}
        ]}}));
        assert_eq!(updates.len(), 1);
        match &updates[0] {
            AgentUpdate::Plan(plan) => {
                assert_eq!(plan.steps.len(), 2);
                assert_eq!(plan.steps[0].status, PlanStatus::Completed);
                assert_eq!(plan.steps[1].status, PlanStatus::InProgress);
                assert_eq!(plan.steps[1].active_form.as_deref(), Some("Building"));
            }
            other => panic!("expected Plan, got {other:?}"),
        }
    }

    #[test]
    fn assistant_usage_is_context_occupancy() {
        let updates = t(json!({ "type": "assistant", "message": {
            "model": "claude-haiku-4-5",
            "content": [],
            "usage": { "input_tokens": 10, "cache_read_input_tokens": 22474,
                       "cache_creation_input_tokens": 4427, "output_tokens": 41 }
        }}));
        assert_eq!(
            updates,
            vec![AgentUpdate::Usage {
                model: Some("claude-haiku-4-5".into()),
                input_tokens: None,
                output_tokens: None,
                cost_usd: None,
                context_tokens: Some(10 + 22474 + 4427 + 41),
                context_window: None,
            }]
        );
    }

    #[test]
    fn user_tool_result_maps_plain_user_text_is_skipped() {
        let updates = t(json!({ "type": "user", "message": { "content": [
            { "type": "tool_result", "tool_use_id": "tu1", "content": "probe-ok" }
        ]}}));
        assert_eq!(
            updates,
            vec![AgentUpdate::ToolResult {
                tool_use_id: "tu1".into(),
                content: "probe-ok".into(),
                is_error: false,
            }]
        );
        assert!(t(json!({ "type": "user", "message": { "content": [
            { "type": "text", "text": "echo of our own send" }
        ]}}))
        .is_empty());
    }

    #[test]
    fn sidechain_events_are_skipped() {
        assert!(t(json!({ "type": "assistant", "parent_tool_use_id": "tu-parent",
            "message": { "content": [ { "type": "tool_use", "id": "x", "name": "Bash", "input": {} } ] }
        }))
        .is_empty());
    }

    #[test]
    fn result_accumulates_per_turn_tokens_and_goes_idle() {
        let mut totals = StreamTotals::default();
        let result = |input: u64, output: u64, cost: f64| {
            json!({ "type": "result", "subtype": "success", "is_error": false,
                "total_cost_usd": cost,
                "usage": { "input_tokens": input, "output_tokens": output },
                "modelUsage": { "claude-haiku-4-5-20251001": { "contextWindow": 200000 } } })
        };
        let first = translate(&result(18, 235, 0.013), &mut totals);
        assert_eq!(
            first,
            vec![
                AgentUpdate::Usage {
                    model: None,
                    input_tokens: Some(18),
                    output_tokens: Some(235),
                    cost_usd: Some(0.013),
                    context_tokens: None,
                    context_window: Some(200000),
                },
                AgentUpdate::Idle,
            ]
        );
        // Second turn: tokens accumulate (the wire is per-turn), cost is
        // already cumulative and passes through.
        let second = translate(&result(10, 40, 0.019), &mut totals);
        assert_eq!(
            second[0],
            AgentUpdate::Usage {
                model: None,
                input_tokens: Some(28),
                output_tokens: Some(275),
                cost_usd: Some(0.019),
                context_tokens: None,
                context_window: Some(200000),
            }
        );
    }

    #[test]
    fn error_result_surfaces_except_user_interrupts() {
        // A real failure surfaces.
        let updates = t(
            json!({ "type": "result", "subtype": "error_during_execution",
            "is_error": true, "errors": ["boom"] }),
        );
        assert!(updates.contains(&AgentUpdate::Error("boom".into())));
        assert!(updates.contains(&AgentUpdate::Idle));
        // The user's own interrupt is not an error (verified wire:
        // `terminal_reason: "aborted_streaming"`).
        let updates = t(
            json!({ "type": "result", "subtype": "error_during_execution",
            "is_error": true, "terminal_reason": "aborted_streaming",
            "errors": ["[ede_diagnostic] …"] }),
        );
        assert!(!updates.iter().any(|u| matches!(u, AgentUpdate::Error(_))));
        assert!(updates.contains(&AgentUpdate::Idle));
    }

    #[test]
    fn rate_limit_event_buckets_by_window_type() {
        let updates = t(json!({ "type": "rate_limit_event", "rate_limit_info": {
            "status": "allowed", "resetsAt": 1783314600, "rateLimitType": "five_hour",
            "utilization": 19.0 } }));
        assert_eq!(
            updates,
            vec![AgentUpdate::RateLimits {
                five_hour_pct: Some(19.0),
                five_hour_resets_at: Some(1783314600),
                seven_day_pct: None,
                seven_day_resets_at: None,
            }]
        );
        let updates = t(json!({ "type": "rate_limit_event", "rate_limit_info": {
            "status": "allowed", "resetsAt": 1783914600, "rateLimitType": "seven_day_sonnet",
            "utilization": 3.0 } }));
        assert_eq!(
            updates,
            vec![AgentUpdate::RateLimits {
                five_hour_pct: None,
                five_hour_resets_at: None,
                seven_day_pct: Some(3.0),
                seven_day_resets_at: Some(1783914600),
            }]
        );
    }

    #[test]
    fn unknown_event_is_ignored() {
        assert!(t(json!({ "type": "tool_progress", "x": 1 })).is_empty());
        assert!(t(json!({ "type": "system", "subtype": "hook_started" })).is_empty());
    }

    #[test]
    fn answered_input_maps_options_and_free_text() {
        let input = json!({ "questions": [
            { "question": "Pick a color", "options": [ { "label": "Red" }, { "label": "Blue" } ] },
            { "question": "Name it", "options": [] }
        ]});
        // 1-indexed option pick resolves to the option label; free text
        // passes through (verified answered shape: `answers: {question → label}`).
        let out = answered_input(
            &input,
            &ManagedAnswer {
                option: None,
                text: None,
                answers: Some(vec!["2".into(), "sparkles".into()]),
            },
        );
        assert_eq!(out["answers"]["Pick a color"], "Blue");
        assert_eq!(out["answers"]["Name it"], "sparkles");
        // The original questions are preserved (updatedInput must satisfy the
        // tool's input schema).
        assert_eq!(out["questions"], input["questions"]);

        // Single `option` answers the first (or only) question.
        let out = answered_input(
            &input,
            &ManagedAnswer {
                option: Some(1),
                text: None,
                answers: None,
            },
        );
        assert_eq!(out["answers"]["Pick a color"], "Red");

        // Free `text` likewise.
        let out = answered_input(
            &input,
            &ManagedAnswer {
                option: None,
                text: Some("Chartreuse".into()),
                answers: None,
            },
        );
        assert_eq!(out["answers"]["Pick a color"], "Chartreuse");
    }

    #[test]
    fn approval_raw_matches_hook_shape() {
        let request = json!({ "subtype": "can_use_tool", "tool_name": "Bash",
            "input": { "command": "rm -rf build" }, "tool_use_id": "tu1",
            "permission_suggestions": [ { "type": "setMode", "mode": "acceptEdits" } ] });
        let raw = approval_raw(&request);
        assert_eq!(raw["tool_name"], "Bash");
        assert_eq!(raw["tool_input"]["command"], "rm -rf build");
        assert_eq!(approval_summary(&request).as_deref(), Some("rm -rf build"));
    }

    #[test]
    fn build_argv_pins_or_resumes() {
        let cfg = SpawnConfig {
            session_id: "sid-1".into(),
            cwd: "/w".into(),
            bin: "claude".into(),
            model: Some("haiku".into()),
            permission_mode: Some("plan".into()),
            resume: None,
            extra_args: vec!["--fallback-model".into(), "sonnet".into()],
            env: HashMap::new(),
            yolo: true,
            facade: Facade::default(),
        };
        let argv = build_argv(&cfg);
        let joined = argv.join(" ");
        assert!(joined.contains("--print --input-format stream-json --output-format stream-json"));
        assert!(joined.contains("--permission-prompt-tool stdio"));
        assert!(joined.contains("--session-id sid-1"));
        assert!(joined.contains("--model haiku"));
        assert!(joined.contains("--permission-mode plan"));
        assert!(joined.contains("--dangerously-skip-permissions"));
        assert!(joined.ends_with("--fallback-model sonnet"));

        let resumed = build_argv(&SpawnConfig {
            resume: Some("old-id".into()),
            yolo: false,
            model: None,
            permission_mode: None,
            extra_args: vec![],
            ..cfg
        });
        let joined = resumed.join(" ");
        assert!(joined.contains("--resume old-id"));
        assert!(!joined.contains("--session-id"));
        assert!(!joined.contains("--dangerously-skip-permissions"));
    }
}
