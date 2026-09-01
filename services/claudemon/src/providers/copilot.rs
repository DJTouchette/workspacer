//! GitHub Copilot CLI adapter — drive `copilot -p … --output-format json` and
//! translate its JSONL event stream into claudemon's session model.
//!
//! # Shape: one-shot `-p` per turn, not a long-lived server
//!
//! Copilot CLI has two machine interfaces: the non-interactive `-p/--prompt`
//! mode (one process per turn, JSONL on stdout) and `--acp` (a long-lived
//! Agent Client Protocol server on stdio). This adapter drives the FIRST.
//!
//! That is only viable because of one flag: `--session-id <uuid>` **both
//! creates a session with that id and resumes an existing one** (verified live
//! 2026-08-28 — turn 1 pinned a fresh uuid, turn 2 with the same uuid recalled
//! a codeword from turn 1). So a conversation survives across N separate
//! processes with no sidecar file, no thread registry, and no rejoin dance —
//! the thing `codex.rs` needed the whole `~/.workspacer/codex-threads` sidecar
//! to approximate. It also makes `restartPreservesConversation: true` an honest
//! claim for the first time on a managed provider other than Claude, and makes
//! `--model` / `--effort` genuinely live: the next turn is a new process, so a
//! switch simply changes the argv it is spawned with.
//!
//! What ACP would add, and the evidence that would justify moving:
//!   * mediated tool approvals — `-p` mode CANNOT ask (see below), so a real
//!     "ask to approve" pill is only reachable over ACP;
//!   * one process per session instead of one per turn (~1.5s of CLI startup
//!     per turn is paid on every message today);
//!   * mid-turn model switching rather than next-turn.
//!
//! Against: an ACP handshake probe (2026-08-28) showed `session/new` returns
//! session *modes* and `configOptions` but **no model list and no model config
//! option**, so ACP buys us nothing on the model-picker front; and it is a
//! second protocol to keep alive. Move when a user actually needs interactive
//! approvals on Copilot, or when per-turn startup cost shows up as a
//! complaint — not before.
//!
//! # `-p` mode has no approval gate — verified, and it is not what the help says
//!
//! `copilot --help` says `--allow-all-tools` is "required for non-interactive
//! mode". It is not: a `-p` run with NO allow flags happily ran `bash`. What
//! the allow flags actually change is *path/URL confinement*:
//!
//! | flags | tools run? | write outside cwd? |
//! |---|---|---|
//! | none | yes, automatically | **no** — `{"code":"denied"}` |
//! | `--allow-all-tools` | yes | yes |
//! | `--allow-all` (yolo) | yes | yes, plus URLs |
//!
//! A denied tool comes back as `tool.execution_complete` with
//! `success:false, error:{message:"Permission denied and could not request
//! permission from user", code:"denied"}` — the CLI says outright that it has
//! no channel to ask. So this adapter maps the two workspacer permission modes
//! onto the only two tiers `-p` actually has:
//!   * `ask`  → no allow flags: tools auto-run but stay inside the cwd tree;
//!   * `yolo` → `--allow-all`: nothing is confined.
//!
//! The pill is labelled "Workspace only" / "Full access" for copilot rather
//! than "Ask to approve" (see `providerCaps.ts`), because a pill that promised
//! approvals here would be lying.
//!
//! # The Idle-collapse trap (codex.rs:57-67), reached by a different road
//!
//! Copilot's failure surface is worse than Codex's: a hard refusal can print
//! human prose to **stderr while the process exits 0** (the scout observed
//! exactly that on a policy denial), and stdout still carries clean JSONL with
//! no error event in it. Keying "turn over" on process exit — or reading only
//! stdout — renders a totally failed session as a clean completion.
//!
//! [`turn_outcome`] is the guard, and it is pure and unit-tested. A turn is a
//! success only when ALL of: stderr is empty, the terminal `result` event
//! arrived, its `exitCode` is 0, the process exit status is 0, and the turn
//! produced *something* (assistant text or a tool call). Anything else emits
//! `AgentUpdate::Error` **before** the `Idle` that returns the session to
//! Input. `translate` never emits `Idle` at all — `assistant.idle` is a
//! liveness event inside the run, not the verdict on it.
//!
//! # Cost
//!
//! Copilot bills in **AI credits (AIU)**, not dollars: `session.usage_checkpoint`
//! carries `totalNanoAiu` and `model.model_call_success` carries a
//! `copilot_usage.token_details` itemization priced in nano-AIU. There is no
//! dollar figure anywhere on the wire, so — exactly like Codex — cost falls
//! through to `session::pricing::estimate_cost` over the token totals, which is
//! an estimate of the underlying vendor list price, not of what GitHub charges
//! your credit balance. Tokens themselves ARE exact: `model.model_call_success`
//! reports real `prompt_tokens` / `completion_tokens` / `cached_tokens`
//! per model call (per-call, hence [`UsageAcc::additive`]), and
//! `model.turn_started` reports Copilot's own
//! `max_context_window_tokens` for the model — better than the window table.

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Context;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use directories::BaseDirs;

use super::SpawnExtras;
use super::{apply_updates, note_user_send, AgentUpdate, Facade, ModelInfo, UsageAcc};
use crate::session::conversation::ConversationItem;
use crate::session::state::{
    Plan, PlanStatus, PlanStep, SessionMode, SubagentStatus, SubagentUpdate,
};
use crate::session::{ConversationStore, ModelSwitch, SessionStore};

/// The one `--model` value this adapter can promise works: Copilot's own
/// router. See [`list_models`].
pub const AUTO_MODEL: &str = "auto";

/// Copilot's reasoning-effort ladder (`--effort`, choices printed by
/// `copilot --help` on v1.0.81). A superset of Claude's five.
pub const EFFORT_LEVELS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh", "max"];

/// List the models a Copilot session can launch with (cached; see
/// [`super::cached_or_fetch`]).
///
/// **This returns `auto` and nothing else, on purpose.** Copilot CLI exposes no
/// model enumeration, and probing for one is a dead end in three independent
/// directions (all checked live against v1.0.81 on 2026-08-28):
///
///   1. there is no `copilot models` subcommand and the generated shell
///      completions carry no model values for `--model`;
///   2. the ACP handshake (`--acp` → `initialize` + `session/new`) returns
///      session modes and `configOptions` but no model list;
///   3. GitHub's Copilot token/model API (`copilot_internal/v2/token`) answers
///      403 to a `gh` OAuth token, so there is nothing to query.
///
/// And the ids are not merely unknown, they are **account-gated**: on the probe
/// account every explicit `--model <id>` was rejected with `Model "<id>" from
/// --model flag is not available` — including `gpt-5-mini` and
/// `claude-haiku-4.5`, the two ids Copilot's own router had just *chosen* for
/// that same account. The captured `modelInfo` says why:
/// `"model_picker_enabled": false`. A curated table of ids from the GitHub
/// changelog would therefore have shipped a picker where every entry fails.
///
/// So: one entry, `auto`, which is always valid; the spawn dialog still allows
/// free-text entry for accounts whose plan does enable the picker, and an id
/// the CLI rejects fails fast and **loudly** — exit 1 with that message on
/// stderr, which [`turn_outcome`] surfaces as `AgentUpdate::Error`.
///
/// The live part that remains is a liveness gate: `bin --version` must run, so
/// a missing/broken CLI errors instead of returning a plausible-looking list.
pub async fn list_models(bin: &str, cwd: &str) -> anyhow::Result<Vec<ModelInfo>> {
    super::cached_or_fetch(format!("copilot:{bin}"), fetch_models(bin, cwd)).await
}

async fn fetch_models(bin: &str, cwd: &str) -> anyhow::Result<Vec<ModelInfo>> {
    // Same 10s ceiling as the other listings. `--version` short-circuits before
    // any auth or network work, so this is a pure "is the launcher real" probe.
    let child = Command::new(bin)
        .arg("--version")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("running `{bin} --version`"))?;
    let out = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait_with_output())
        .await
        .context("timed out probing the copilot binary")?
        .with_context(|| format!("running `{bin} --version`"))?;
    if !out.status.success() {
        anyhow::bail!("`{bin} --version` exited with {}", out.status);
    }
    Ok(vec![ModelInfo {
        id: AUTO_MODEL.to_string(),
        label: "Auto (Copilot picks)".to_string(),
        default: true,
        effort_levels: EFFORT_LEVELS.iter().map(|s| (*s).to_string()).collect(),
        default_effort: None,
        default_context_window: None,
        max_context_window: None,
        effective_context_window_percent: None,
    }])
}

// ── Pure translation ────────────────────────────────────────────────────────

/// Translate one Copilot JSONL event into zero or more typed updates. Pure and
/// total: unknown event types and missing fields yield an empty/partial result
/// rather than an error.
///
/// Deliberately does NOT emit [`AgentUpdate::Idle`]. `assistant.idle` fires
/// inside a still-running process and says nothing about whether the turn
/// succeeded; the verdict is [`turn_outcome`]'s, once the process is gone and
/// stderr has been read. That separation is the whole mitigation for the
/// exit-0-with-stderr-prose failure shape.
///
/// Assistant text comes from `assistant.message_delta` (the streaming path),
/// not from the whole-message `assistant.message`, so the two can't
/// double-render. The driver keeps a per-`messageId` tally and emits the
/// remainder from `assistant.message` when no deltas arrived for that id (a
/// `--stream` off / non-streaming model fallback) — see
/// [`message_tail`].
pub fn translate(event: &Value) -> Vec<AgentUpdate> {
    let ty = event.get("type").and_then(Value::as_str).unwrap_or("");
    let data = event.get("data").cloned().unwrap_or(Value::Null);
    let mut out = Vec::new();

    // A SUB-agent's own frames ride this same stdout stream, tagged with a
    // top-level `agentId`. They are not this session's conversation and must
    // never be folded into it: verified live, a two-subagent turn put the
    // dispatch prompts on the wire as `user.message` (rendering as messages the
    // user never typed), and the subagents' `view` calls and their whole reports
    // as the parent's own text. Claude's subagents are drill-in only for exactly
    // this reason; this keeps copilot's the same.
    //
    // The `subagent.*` lifecycle frames carry an `agentId` too, and those ARE
    // ours to read — they are the row itself, not the row's conversation.
    if let Some(agent_id) = child_agent_id(event, ty) {
        return child_activity(agent_id, ty, &data);
    }

    match ty {
        // The org-policy refusal the scout hit: the CLI reports it as a
        // *warning* on an otherwise clean stream. Surfacing it as an Error is
        // the "announce, never silently degrade" rule — a facade-less session
        // must not look like a working one.
        "session.warning" => {
            let msg = data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("copilot session warning");
            let kind = data.get("warningType").and_then(Value::as_str);
            if kind == Some("policy") {
                out.push(AgentUpdate::Error(msg.to_string()));
            } else {
                tracing::debug!(warning = %msg, ?kind, "copilot session warning");
            }
        }

        // A turn has started producing output.
        "assistant.turn_start" => out.push(AgentUpdate::Busy),

        // Streaming assistant text.
        "assistant.message_delta" => {
            if let Some(text) = data.get("deltaContent").and_then(Value::as_str) {
                if !text.is_empty() {
                    out.push(AgentUpdate::Busy);
                    out.push(AgentUpdate::AssistantText(text.to_string()));
                }
            }
        }

        // Whole-message event. Text is NOT taken here (the deltas already
        // carried it); the driver reconciles a delta-less message itself.
        "assistant.message" => out.push(AgentUpdate::Busy),

        // A tool call is starting. `arguments` is the complete argument object
        // (the `assistant.tool_call_delta` frames are partial JSON fragments and
        // are ignored). MCP tools carry `mcpServerName`/`mcpToolName` too;
        // `toolName` is already the qualified `<server>-<tool>` id, which is
        // what the GUI's tool cards key on.
        "tool.execution_start" => {
            let id = data
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let name = data
                .get("toolName")
                .and_then(Value::as_str)
                .or_else(|| data.get("toolTitle").and_then(Value::as_str))
                .unwrap_or("tool")
                .to_string();
            let input = data.get("arguments").cloned().unwrap_or(Value::Null);
            out.push(AgentUpdate::Busy);
            out.push(AgentUpdate::ToolUse { id, name, input });
        }

        // A tool call finished. `success:false` carries `error.message` — this
        // is where a path-confinement refusal lands
        // ("Permission denied and could not request permission from user").
        "tool.execution_complete" => {
            let tool_use_id = data
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // `success` is explicitly `false` on failure; treat a missing flag
            // as success so an unknown shape doesn't paint every result red.
            let is_error = data.get("success").and_then(Value::as_bool) == Some(false);
            let content = tool_result_text(&data).unwrap_or_default();
            out.push(AgentUpdate::ToolResult {
                tool_use_id,
                content,
                is_error,
            });
        }

        // Per-model-call token usage + the model's real context window. Both
        // `model.turn_started` and `model.model_call_started` carry `modelInfo`;
        // only `model.model_call_success` carries the usage numbers.
        "model.turn_started"
        | "model.model_call_started"
        | "model.model_call_success"
        | "model.turn_ended" => {
            if let Some(u) = usage_from(&data) {
                out.push(u);
            }
        }

        // Copilot's sub-agents. The `task` tool spawns one and the CLI reports
        // its whole lifecycle on dedicated events carrying a stable `agentId`
        // — verified live against v1.0.81 (see
        // `testdata/copilot-subagent-capture.jsonl`), which is why this is a
        // structural mapping and not a guess at a tool name.
        //
        // `subagent.configured` is deliberately absent: it repeats the model
        // `subagent.started` already carried and names nothing new.
        "subagent.started" | "subagent.completed" => {
            if let Some(update) = subagent_from(&data, event, ty) {
                out.push(AgentUpdate::Subagent(update));
            }
        }

        // The second, independent "that agent is no longer running" signal: the
        // CLI tells the MODEL an agent went idle through a system notification
        // carrying the same `agentId`. Taken as well as `subagent.completed`
        // because a row that never closes holds its parent busy forever (the
        // failure 6d795cc2 fixed on the codex path) — and closing is the
        // self-healing direction, since a later `started` re-opens the row.
        "system.notification" => {
            if let Some(update) = subagent_from_notification(&data) {
                out.push(AgentUpdate::Subagent(update));
            }
        }

        // Copilot's plan is NOT on the wire. `session.todos_changed` is a
        // content-free ping (`data: {}` on every one of the 6 seen in a live
        // plan-shaped run) because the todos themselves live in a SQLite table
        // the `sql` tool writes — see [`session_todos`], which the driver reads
        // when this fires. The payload probe stays anyway: it costs nothing and
        // is what would light up if a future version started carrying them.
        "session.todos_changed" => {
            if let Some(plan) = super::plan_from_value(&data) {
                out.push(AgentUpdate::Plan(plan));
            }
        }

        _ => {}
    }

    out
}

/// The sub-agent a frame belongs to, when it is a sub-agent's *own* activity
/// rather than this session's.
///
/// `subagent.*` frames are excluded: they also carry `agentId`, but they are the
/// parent telling us about the child's lifecycle, which [`subagent_from`] reads.
fn child_agent_id<'a>(event: &'a Value, ty: &str) -> Option<&'a str> {
    if ty.starts_with("subagent.") {
        return None;
    }
    event
        .get("agentId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// What a sub-agent's own frame contributes: at most a "what is it doing right
/// now" line on its row, and never a conversation item.
///
/// Only `tool.execution_start` produces an update, and the reason is the row's
/// status. [`SubagentUpdate`] carries a status unconditionally, so any update
/// re-asserts one — and a frame that arrived after the agent's
/// `subagent.completed` would re-open a closed row and hold the parent busy
/// forever (the exact failure 6d795cc2 fixed elsewhere). A tool cannot START
/// after its agent has finished, so that one frame is safe to call `Running`;
/// the rest are simply suppressed.
fn child_activity(agent_id: &str, ty: &str, data: &Value) -> Vec<AgentUpdate> {
    if ty != "tool.execution_start" {
        return Vec::new();
    }
    let name = data
        .get("toolName")
        .and_then(Value::as_str)
        .or_else(|| data.get("toolTitle").and_then(Value::as_str))
        .unwrap_or("tool")
        .to_string();
    vec![AgentUpdate::Subagent(SubagentUpdate {
        id: agent_id.to_string(),
        agent_type: None,
        status: SubagentStatus::Running,
        description: None,
        tool_use_id: None,
        model: None,
        last_tool_summary: data
            .get("arguments")
            .and_then(|a| a.get("description"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        last_tool_name: Some(name),
    })]
}

/// A `subagent.started` / `subagent.completed` frame → the subagent row it
/// describes.
///
/// The `agentId` is a TOP-LEVEL field on these frames, not part of `data` —
/// that asymmetry is the one thing here worth remembering, and it is why this
/// takes the whole event as well as its data. Without an id there is no row to
/// key, so an idless frame is dropped rather than invented.
///
/// `agentName` is the agent's TYPE (`explore`, or a `.github/agents` name);
/// `agentDisplayName` is the caller's label for this particular dispatch
/// (`explore-a.ts`), which is what the row's description should read. Both were
/// present on every frame in the live capture.
fn subagent_from(data: &Value, event: &Value, ty: &str) -> Option<SubagentUpdate> {
    let id = event
        .get("agentId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let status = if ty == "subagent.completed" {
        SubagentStatus::Complete
    } else {
        SubagentStatus::Running
    };
    Some(SubagentUpdate {
        id,
        // `agentType` and `agentName` are the same value on the wire; take
        // either, and fall back to the tool that spawns them rather than to the
        // shared default, which spells a different provider's name.
        agent_type: Some(
            data.get("agentType")
                .and_then(Value::as_str)
                .or_else(|| data.get("agentName").and_then(Value::as_str))
                .unwrap_or("task")
                .to_string(),
        ),
        status,
        description: data
            .get("agentDescription")
            .and_then(Value::as_str)
            .or_else(|| data.get("agentDisplayName").and_then(Value::as_str))
            .map(str::to_owned),
        // The id of the `task` tool call that spawned it — this is what nests
        // the row under its own tool card in the GUI.
        tool_use_id: data
            .get("toolCallId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        model: data.get("model").and_then(Value::as_str).map(str::to_owned),
        last_tool_name: None,
        // Only the completion frame has anything to summarize; a running agent's
        // inner tool calls are not reported to the parent's stream at all.
        last_tool_summary: (status == SubagentStatus::Complete)
            .then(|| completion_summary(data))
            .flatten(),
    })
}

/// The one-line tally on a `subagent.completed` frame ("1 tool call · 11,981
/// tokens · 4.8s"), or `None` when it carried no numbers.
fn completion_summary(data: &Value) -> Option<String> {
    let calls = data.get("totalToolCalls").and_then(Value::as_u64);
    let tokens = data.get("totalTokens").and_then(Value::as_u64);
    let secs = data
        .get("durationMs")
        .and_then(Value::as_u64)
        .map(|ms| ms as f64 / 1000.0);
    let mut parts = Vec::new();
    if let Some(n) = calls {
        parts.push(format!("{n} tool call{}", if n == 1 { "" } else { "s" }));
    }
    if let Some(n) = tokens {
        parts.push(format!("{n} tokens"));
    }
    if let Some(s) = secs {
        parts.push(format!("{s:.1}s"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// A `system.notification` whose `kind` says an agent went idle → close that
/// agent's row. Every other notification kind is ignored.
fn subagent_from_notification(data: &Value) -> Option<SubagentUpdate> {
    let kind = data.get("kind")?;
    if kind.get("type").and_then(Value::as_str) != Some("agent_idle") {
        return None;
    }
    let id = kind
        .get("agentId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    Some(SubagentUpdate {
        id,
        agent_type: kind
            .get("agentType")
            .and_then(Value::as_str)
            .map(str::to_owned),
        status: SubagentStatus::Complete,
        description: kind
            .get("description")
            .and_then(Value::as_str)
            .or_else(|| kind.get("displayName").and_then(Value::as_str))
            .map(str::to_owned),
        tool_use_id: None,
        model: None,
        last_tool_name: None,
        last_tool_summary: None,
    })
}

/// Copilot's todo list — its plan — as of right now, read out of the session's
/// own SQLite database.
///
/// Copilot does not put its plan on the wire. What it has instead is a
/// per-session SQLite db with a `todos` table already created in it, which the
/// model mutates through the ordinary `sql` tool; the CLI's system prompt
/// documents the table by name and tells the agent to track work in it. All the
/// wire carries is `session.todos_changed`, a ping with an empty `data` (see
/// [`translate`]). So the plan is read here, from the file, when that ping says
/// it changed.
///
/// The db is `~/.copilot/session-state/<session-id>/session.db`, and the
/// session id is the uuid *we* pinned with `--session-id` — so no discovery,
/// no globbing, and no ambiguity about which session's plan this is. Opened
/// read-only, and every failure (no home dir, no db yet, a schema without the
/// table) is `None`, which the caller treats as "nothing to say about the
/// plan" rather than as an error: this is an enrichment path, and a session
/// whose agent never wrote a todo has no db table rows and must not look broken.
fn session_todos(session_id: &str) -> Option<Plan> {
    todos_at(
        &BaseDirs::new()?
            .home_dir()
            .join(".copilot")
            .join("session-state")
            .join(session_id)
            .join("session.db"),
    )
}

/// Read the `todos` table out of one copilot session db. Split from
/// [`session_todos`] so the query — and the assumption that copilot's own
/// column names are `title` and `status` — is testable against a db built from
/// the CLI's verbatim schema.
fn todos_at(path: &std::path::Path) -> Option<Plan> {
    if !path.exists() {
        return None;
    }
    // read_only is not just hygiene: opening read-write would create/alter the
    // file the CLI owns, and a `-p` process may be writing it this instant.
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    let mut stmt = conn
        .prepare("SELECT title, status FROM todos ORDER BY rowid")
        .ok()?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            ))
        })
        .ok()?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    plan_from_todo_rows(&rows)
}

/// `(title, status)` rows from the `todos` table → a [`Plan`]. Pure, so the
/// status vocabulary is testable without a database.
///
/// Copilot's statuses are `pending` / `in_progress` / `done` / `blocked` (the
/// table's own CHECK constraint). The first three land exactly on
/// [`PlanStatus`] via `from_wire`; `blocked` has no equivalent and falls
/// through to `Pending`, which is the honest reading — it is not in progress
/// and it is certainly not done.
fn plan_from_todo_rows(rows: &[(String, String)]) -> Option<Plan> {
    let steps: Vec<PlanStep> = rows
        .iter()
        .filter(|(title, _)| !title.trim().is_empty())
        .map(|(title, status)| PlanStep {
            content: title.clone(),
            status: PlanStatus::from_wire(status),
            active_form: None,
        })
        .collect();
    (!steps.is_empty()).then_some(Plan {
        steps,
        updated_at: None,
    })
}

/// The human-readable text of a `tool.execution_complete`: the result content
/// on success, the error message on failure.
fn tool_result_text(data: &Value) -> Option<String> {
    if let Some(err) = data
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
    {
        let code = data
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(Value::as_str);
        return Some(match code {
            Some(code) => format!("{err} ({code})"),
            None => err.to_string(),
        });
    }
    let result = data.get("result")?;
    result
        .get("content")
        .and_then(Value::as_str)
        .or_else(|| result.get("detailedContent").and_then(Value::as_str))
        .map(str::to_owned)
        .or_else(|| Some(result.to_string()))
}

/// Pull a [`AgentUpdate::Usage`] out of a `model.*` event's `data`.
///
/// Two independent things live in here and either can be absent:
///   * `modelInfo.capabilities.limits.max_context_window_tokens` — Copilot's own
///     window for the model it actually used (e.g. 144000 for
///     `claude-haiku-4.5`, which is Copilot's number, not Anthropic's);
///   * `responseChunk.usage` — real `prompt_tokens` / `completion_tokens` /
///     `prompt_tokens_details.cached_tokens` for THIS model call. Per-call, not
///     cumulative (two calls in one turn reported 17515 then 17997 prompt
///     tokens), hence [`UsageAcc::additive`] in the driver.
///
/// `context_tokens` is the call's `total_tokens` — the occupancy of the window
/// at that moment, which is latest-wins in `UsageAcc` even in additive mode.
fn usage_from(data: &Value) -> Option<AgentUpdate> {
    let model = data
        .get("model")
        .and_then(Value::as_str)
        .or_else(|| data.pointer("/modelCall/model").and_then(Value::as_str))
        .map(str::to_owned);
    let context_window = data
        .pointer("/modelInfo/capabilities/limits/max_context_window_tokens")
        .and_then(Value::as_u64);
    let usage = data.pointer("/responseChunk/usage");
    let input = usage
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(Value::as_u64);
    let output = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(Value::as_u64);
    let cached = usage
        .and_then(|u| u.pointer("/prompt_tokens_details/cached_tokens"))
        .and_then(Value::as_u64);
    let context_tokens = usage
        .and_then(|u| u.get("total_tokens"))
        .and_then(Value::as_u64);

    if model.is_none() && context_window.is_none() && input.is_none() && output.is_none() {
        return None;
    }
    Some(AgentUpdate::Usage {
        model,
        input_tokens: input,
        output_tokens: output,
        cached_input_tokens: cached,
        // AI credits, not dollars — see the module doc. `UsageAcc::estimate_costs`
        // fills this in from the pricing table.
        cost_usd: None,
        context_tokens,
        context_window,
    })
}

/// Tracks how much of each assistant message the streamed deltas have already
/// carried, so a message whose deltas never arrived can still be rendered
/// without double-rendering the ones whose deltas did.
///
/// This is the one piece of per-turn state the translation needs, kept out of
/// [`translate`] so that stays pure — and kept public so the wire-capture
/// replay test drives the real reconciliation rather than a copy of it.
#[derive(Default)]
pub struct TextReconciler {
    streamed: std::collections::HashMap<String, String>,
}

impl TextReconciler {
    /// Feed one event. Returns the assistant text that [`translate`] did NOT
    /// already emit for it — always `None` for a delta (translate emits those),
    /// and `Some(tail)` for a whole message the deltas didn't cover.
    pub fn observe(&mut self, event: &Value) -> Option<String> {
        let ty = event.get("type").and_then(Value::as_str)?;
        // The SECOND door into the parent's conversation, and it has to be shut
        // too: a sub-agent's `assistant.message` reaches here with text
        // `translate` deliberately dropped, and reconciling it would emit the
        // child's whole report as the parent's — the exact leak the routing in
        // `translate` exists to stop. See [`child_agent_id`].
        if child_agent_id(event, ty).is_some() {
            return None;
        }
        let data = event.get("data")?;
        match ty {
            "assistant.message_delta" => {
                let id = data.get("messageId").and_then(Value::as_str)?;
                let text = data.get("deltaContent").and_then(Value::as_str)?;
                self.streamed
                    .entry(id.to_string())
                    .or_default()
                    .push_str(text);
                None
            }
            "assistant.message" => {
                let id = data
                    .get("messageId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let full = data.get("content").and_then(Value::as_str).unwrap_or("");
                let seen = self.streamed.get(id).map(String::as_str).unwrap_or("");
                let tail = message_tail(seen, full)?;
                self.streamed
                    .entry(id.to_string())
                    .or_default()
                    .push_str(&tail);
                Some(tail)
            }
            _ => None,
        }
    }
}

/// The part of a whole-message `content` that the streamed deltas did not
/// already carry. `None` when the deltas covered it (the normal path).
///
/// Exists so a non-streaming turn still renders: if `--stream` is off, or a
/// model emits its message in one shot, no `assistant.message_delta` ever
/// arrives and the text would otherwise be dropped entirely.
pub fn message_tail(streamed: &str, full: &str) -> Option<String> {
    if full.is_empty() {
        return None;
    }
    match full.strip_prefix(streamed) {
        // Deltas covered the whole message.
        Some("") => None,
        Some(tail) => Some(tail.to_string()),
        // The deltas didn't prefix the message (shouldn't happen). Emitting the
        // whole message would duplicate; emitting nothing loses text only when
        // we already showed something. Prefer no duplication.
        None if !streamed.is_empty() => None,
        None => Some(full.to_string()),
    }
}

// ── Turn outcome: the Idle-collapse guard ───────────────────────────────────

/// What one `copilot -p` process reported about the turn it just ran.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TurnReport {
    /// Everything the child wrote to stderr, trimmed.
    pub stderr: String,
    /// `exitCode` from the terminal `result` event, when one arrived at all.
    pub result_exit_code: Option<i64>,
    /// The process's own exit status, `None` if it was killed by a signal.
    pub process_exit_code: Option<i32>,
    /// Did the turn produce anything at all — assistant text or a tool call?
    pub produced_output: bool,
    /// Did the stream already carry an error-shaped event that `translate` has
    /// surfaced (a policy `session.warning`)? Then don't double-report.
    pub saw_stream_error: bool,
    /// The turn was interrupted on purpose (the user hit stop) — an incomplete
    /// turn is the expected outcome, not a failure.
    pub interrupted: bool,
}

/// The updates that close out one turn.
///
/// **A failed turn must never collapse to a bare `Idle`.** Copilot makes that
/// easy to get wrong in a way Codex does not: a hard refusal can print prose to
/// stderr and still exit 0, with clean JSONL on stdout carrying no error event
/// at all. So success is proved, not assumed — every one of these must hold:
///
///   * stderr is empty;
///   * the terminal `result` event arrived (its absence means the process died
///     mid-turn — the "exited 0 with a stderr refusal" shape has no `result`);
///   * `result.exitCode == 0`;
///   * the process exit status is 0;
///   * the turn produced assistant text or a tool call.
///
/// `Idle` is still emitted last in every case, because the session has to
/// return to `Input` whatever happened — but it rides *behind* the `Error`, so
/// `apply_updates` records the failure in the conversation before releasing the
/// pane.
pub fn turn_outcome(report: &TurnReport) -> Vec<AgentUpdate> {
    let mut out = Vec::new();
    if report.interrupted {
        return vec![AgentUpdate::Idle];
    }
    if !report.stderr.is_empty() {
        out.push(AgentUpdate::Error(report.stderr.clone()));
    } else if report.result_exit_code.is_none() {
        out.push(AgentUpdate::Error(match report.process_exit_code {
            Some(code) if code != 0 => format!(
                "copilot exited with status {code} before completing the turn (no `result` event)"
            ),
            Some(_) => "copilot exited without completing the turn (no `result` event)".to_string(),
            None => "copilot was killed before completing the turn".to_string(),
        }));
    } else if report.result_exit_code != Some(0) {
        let code = report.result_exit_code.unwrap_or_default();
        out.push(AgentUpdate::Error(format!(
            "copilot reported the turn failed (exitCode {code})"
        )));
    } else if report.process_exit_code.unwrap_or(0) != 0 {
        let code = report.process_exit_code.unwrap_or_default();
        out.push(AgentUpdate::Error(format!(
            "copilot exited with status {code}"
        )));
    } else if !report.produced_output && !report.saw_stream_error {
        out.push(AgentUpdate::Error(
            "copilot finished the turn without producing any output".to_string(),
        ));
    }
    out.push(AgentUpdate::Idle);
    out
}

// ── Argv + MCP config ───────────────────────────────────────────────────────

/// Everything the argv builder needs for one turn.
pub struct TurnArgs<'a> {
    pub prompt: &'a str,
    pub session_id: &'a str,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub yolo: bool,
    /// Path to the `--additional-mcp-config` file, when this session has one.
    pub mcp_config: Option<&'a std::path::Path>,
    /// Names of the workspacer MCP servers registered in that config
    /// (`expected_mcp_servers`). Each becomes an `--allow-tool <name>` grant —
    /// see `turn_argv` for why the session is unusable without them.
    pub mcp_servers: &'a [String],
    /// The profile's extra argv, appended verbatim after everything above.
    pub extra_args: &'a [String],
}

/// Build the argv for one `copilot -p` turn (the binary itself is argv[0]'s
/// job, not this function's — it returns arguments only, so it stays pure and
/// testable).
///
/// Three refusals are baked in and are not configurable:
///   * **`--share` / `--share-gist` are never passed.** `--share-gist` writes
///     the whole session transcript to a secret GitHub gist; a dispatched
///     worker quietly gisting a private repo's session is a real leak.
///   * **`--remote` / `--remote-export` are never passed, and `--no-remote
///     --no-remote-export` are passed explicitly**, so a workspacer-owned
///     session can't be driven from or exported to github.com even if a user
///     default turned remote control on.
///   * **`--no-auto-update`.** Copilot auto-updates by default; a daemon-spawned
///     turn must not swap its own binary out from under a running fleet (and on
///     a read-only install prefix it fails noisily every launch).
pub fn turn_argv(args: &TurnArgs<'_>) -> Vec<String> {
    let mut argv: Vec<String> = vec![
        "-p".into(),
        args.prompt.to_string(),
        "--output-format".into(),
        "json".into(),
        // One session id across every turn's process: creates it on the first
        // turn, resumes it on every later one (verified live).
        "--session-id".into(),
        args.session_id.to_string(),
        // See the doc comment: never let a managed session escape to github.com
        // or rewrite its own launcher.
        "--no-remote".into(),
        "--no-remote-export".into(),
        "--no-auto-update".into(),
        // `-p` cannot ask the user anything (the CLI says so itself in a denied
        // tool's error), so leaving the native `ask_user` tool enabled only
        // gives the model a tool that always fails. Structured questions are
        // served by the workspacer_ask MCP endpoint instead.
        "--no-ask-user".into(),
        // Colour codes would end up inside conversation text.
        "--no-color".into(),
    ];
    // `auto` is the CLI's own default; passing it explicitly is harmless but
    // passing an empty/blank override would be a hard error, so filter both.
    if let Some(model) = args.model.map(str::trim).filter(|m| !m.is_empty()) {
        argv.push("--model".into());
        argv.push(model.to_string());
    }
    if let Some(effort) = args.effort.map(str::trim).filter(|e| !e.is_empty()) {
        argv.push("--effort".into());
        argv.push(effort.to_string());
    }
    if args.yolo {
        // `--allow-all` == `--allow-all-tools --allow-all-paths --allow-all-urls`.
        // Without it tools still run, but writes/reads stay inside the cwd tree
        // (verified): that confinement IS the `ask` tier for this provider.
        argv.push("--allow-all".into());
    }
    if let Some(path) = args.mcp_config {
        argv.push("--additional-mcp-config".into());
        argv.push(format!("@{}", path.display()));
        // WITHOUT THIS THE FACADE IS DECORATIVE. `-p` mode cannot ask the user
        // anything, so an MCP tool call that is merely *unapproved* is not
        // parked — it is hard-denied, `{"code":"denied","message":"Permission
        // denied and could not request permission from user"}`. Verified live
        // against CLI 1.0.81: the same prompt against the same server returns
        // the tool result with an allow grant and that error without one. So a
        // non-yolo Copilot session registered the workspacer facade, connected
        // to it, listed its tools — and then could not call a single one. For a
        // FLEET MANAGER that is the whole role: `spawn_agent` denied every
        // time, by a session that otherwise looks perfectly healthy.
        //
        // The grant is per SERVER NAME (`--allow-tool 'MyMCP'` allows all of
        // that server's tools, per the CLI's own `--help` examples), so it
        // names only the servers we registered — `workspacer` and
        // `workspacer_ask`. It is NOT `--allow-all`: verified in the same run
        // that a session with only this grant still had a read outside its cwd
        // denied. That is the point — it lifts the facade out of the deny-by-
        // default tier and leaves the ask tier's path confinement exactly where
        // it was. The Claude twin is `--allowedTools mcp__workspacer` in
        // managedSpawn.ts.
        for server in args.mcp_servers {
            argv.push("--allow-tool".into());
            argv.push(server.clone());
        }
    }
    // The profile's own argv last. Nothing above is negotiable — the three
    // refusals in the doc comment are baked in and appear earlier, so an
    // extra arg cannot un-say `--no-remote`.
    argv.extend(args.extra_args.iter().cloned());
    argv
}

/// The `--additional-mcp-config` document for a session: the supervisor facade
/// (when this session has one) plus the per-session AskUserQuestion endpoint.
///
/// Copilot takes MCP servers as a **flag**, which is a strictly better seam
/// than any other provider we drive: OpenCode needs a config file written into
/// the project dir before launch, Codex needs a config override, Pi has no MCP
/// client at all. Nothing is persisted into the user's `~/.copilot/mcp-config.json`
/// — this file is session-scoped by construction.
///
/// The explicit `timeout` matches the ask endpoint's own 6h answer window: a
/// question can legitimately wait on a human for hours, and an MCP client's
/// default request timeout would kill it the moment it parked.
pub fn mcp_config(facade_url: Option<&str>, ask_url: Option<&str>) -> Option<Value> {
    let mut servers = serde_json::Map::new();
    if let Some(url) = facade_url {
        servers.insert(
            "workspacer".into(),
            serde_json::json!({ "type": "http", "url": url, "tools": ["*"] }),
        );
    }
    if let Some(url) = ask_url {
        servers.insert(
            "workspacer_ask".into(),
            serde_json::json!({
                "type": "http",
                "url": url,
                "tools": ["*"],
                "timeout": 21_600_000u64, // ms — 6h, the ask endpoint's answer window
            }),
        );
    }
    if servers.is_empty() {
        return None;
    }
    Some(Value::Object(
        [("mcpServers".to_string(), Value::Object(servers))]
            .into_iter()
            .collect(),
    ))
}

/// The per-session ask endpoint URL, or None when the daemon's HTTP API isn't
/// up (unit tests).
fn ask_url(session_id: &str) -> Option<String> {
    let api_base = crate::daemon::API_BASE.get()?;
    Some(format!("{api_base}/mcp/ask/{session_id}"))
}

/// Materialize the session's MCP config next to the other per-session sinks.
/// Kept out of the project tree (unlike OpenCode's `opencode.json`) so a stale
/// per-session URL can never leak into a repo or outlive the session there.
fn write_mcp_config(session_id: &str, doc: &Value) -> Option<std::path::PathBuf> {
    let path = std::env::temp_dir().join(format!("workspacer-copilot-mcp-{session_id}.json"));
    let text = serde_json::to_string(doc).ok()?;
    match std::fs::write(&path, text) {
        Ok(()) => Some(path),
        Err(err) => {
            tracing::warn!(
                ?err,
                session = %session_id,
                "writing the copilot MCP config failed; the facade + ask tools will be unavailable"
            );
            None
        }
    }
}

/// Names of the MCP servers we asked Copilot to load, so the driver can check
/// `session.mcp_servers_loaded` and complain loudly if one didn't attach.
fn expected_mcp_servers(doc: &Value) -> Vec<String> {
    doc.get("mcpServers")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

/// Which of the servers we registered are missing (or not `connected`) in a
/// `session.mcp_servers_loaded` event.
///
/// Copilot's capability surface is the only one in the fleet that is **not
/// determinable at build or spawn time**: an org policy can disable third-party
/// MCP servers, and when it does the CLI simply reports `servers: []` and keeps
/// going. A facade-less supervisor would then look exactly like a working one.
/// Pure so the "announce, never silently degrade" rule can be tested.
pub fn missing_mcp_servers(expected: &[String], event_data: &Value) -> Vec<String> {
    let loaded: Vec<(&str, &str)> = event_data
        .get("servers")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|s| {
                    Some((
                        s.get("name").and_then(Value::as_str)?,
                        s.get("status").and_then(Value::as_str).unwrap_or("unknown"),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    expected
        .iter()
        .filter(|want| {
            !loaded
                .iter()
                .any(|(name, status)| *name == want.as_str() && *status == "connected")
        })
        .cloned()
        .collect()
}

// ── Live driver ─────────────────────────────────────────────────────────────

/// Everything `spawn_session` needs. A struct rather than eight positional
/// arguments, matching `claude_stream::SpawnConfig`.
pub struct SpawnConfig {
    pub session_id: String,
    pub cwd: String,
    pub bin: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub yolo: bool,
    pub facade: Facade,
    /// The profile's `COPILOT_HOME` and the auth-token variable it references,
    /// plus any extra argv. Carried, not interpreted — see [`SpawnExtras`].
    ///
    /// Copilot is the one harness where the config root is NOT the account:
    /// `copilot login` puts its token in the OS credential store, so a second
    /// `COPILOT_HOME` shares the first one's login and the second IDENTITY has
    /// to come from `COPILOT_GITHUB_TOKEN` in the environment. Both arrive
    /// here, and dropping either one is what made a second Copilot account
    /// look configured while running as the first.
    pub extras: SpawnExtras,
}

/// Spawn and drive a Copilot-managed session in the background. Returns
/// immediately; the session's id is already registered in `store` by the
/// caller, so the UI shows it even before the first turn starts a process.
pub fn spawn_session(store: SessionStore, conv: ConversationStore, config: SpawnConfig) {
    // Claimed before the task starts: on a restart this driver can outlive its
    // own lifetime and must not tear down a successor.
    let generation = store.claim_generation(&config.session_id);
    let session_id = config.session_id.clone();
    tokio::spawn(async move {
        if let Err(err) = run_session(&store, &conv, &config).await {
            tracing::warn!(?err, session = %session_id, "copilot managed session ended with error");
        }
        if store.deregister_managed(&session_id, generation) {
            conv.forget(&session_id);
        }
        // The MCP config is session-scoped; a leaked file is inert but the URL
        // in it is not, so remove it.
        let _ = std::fs::remove_file(
            std::env::temp_dir().join(format!("workspacer-copilot-mcp-{session_id}.json")),
        );
    });
}

/// The driver loop. Copilot has no server to keep alive: between turns there is
/// no child process at all, so the loop parks on the input channel and spawns a
/// process per message.
async fn run_session(
    store: &SessionStore,
    conv: &ConversationStore,
    config: &SpawnConfig,
) -> anyhow::Result<()> {
    let session_id = config.session_id.as_str();

    // Session-scoped MCP registration: the supervisor facade (when there is
    // one) plus the AskUserQuestion endpoint for every session — structured
    // questions are baseline parity, not a supervisor feature.
    let mcp_doc = mcp_config(
        config.facade.mcp_url.as_deref(),
        ask_url(session_id).as_deref(),
    );
    let expected_servers = mcp_doc
        .as_ref()
        .map(expected_mcp_servers)
        .unwrap_or_default();
    let mcp_path = mcp_doc
        .as_ref()
        .and_then(|doc| write_mcp_config(session_id, doc));

    // Route user prompts + approval decisions + interrupts + model switches.
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    store.register_managed_input(session_id, tx);
    let (dtx, mut drx) = mpsc::unbounded_channel::<bool>();
    store.register_managed_decision(session_id, dtx);
    let (itx, mut irx) = mpsc::unbounded_channel::<()>();
    store.register_managed_interrupt(session_id, itx);
    let (mtx, mut mrx) = mpsc::unbounded_channel::<ModelSwitch>();
    store.register_managed_model_switch(session_id, mtx);

    // Approval policy, live-switchable. `spawned_yolo: false` because nothing
    // is baked into a long-lived process: every turn is a fresh argv, so
    // yolo→ask takes effect on the next message in both directions.
    let yolo_live = Arc::new(AtomicBool::new(config.yolo));
    store.register_managed_yolo(session_id, yolo_live.clone(), false);

    let mut model = config.model.clone();
    let mut effort = config.effort.clone();
    let mut cur_mode = SessionMode::Input;
    // Copilot's token figures are per model call, not session-cumulative, so
    // they sum (see `usage_from`).
    let mut acc = UsageAcc::new();
    acc.additive();
    acc.estimate_costs();
    acc.seed_model(model.as_deref());
    // Role instructions to prepend to the first turn only (supervisors).
    let mut pending_instructions: Option<String> = config.facade.instructions.clone();

    // Messages typed while a turn was already running: `-p` has no way to
    // inject mid-turn, so they become the next turn's prompt. The bool records
    // that they were already echoed into the conversation when they arrived.
    let mut queued: VecDeque<(String, bool)> = VecDeque::new();

    loop {
        // Between turns: drain anything queued mid-turn first, else park on the
        // control channels.
        let (text, already_echoed) = match queued.pop_front() {
            Some(entry) => entry,
            None => tokio::select! {
                msg = rx.recv() => match msg {
                    Some(text) => (text, false),
                    None => break, // managed input dropped → session terminated
                },
                decision = drx.recv() => match decision {
                    Some(approve) => { announce_decision(store, conv, session_id, approve, &mut cur_mode, &mut acc); continue }
                    None => break,
                },
                // Nothing is running between turns; an interrupt is a no-op rather
                // than an error (the user hit stop on an already-finished turn).
                intr = irx.recv() => match intr {
                    Some(()) => continue,
                    None => break,
                },
                switch = mrx.recv() => match switch {
                    Some(sw) => { apply_switch(store, conv, session_id, sw, &mut model, &mut effort, &mut cur_mode, &mut acc); continue }
                    None => break,
                },
            },
        };

        // Echo the user's message verbatim, but prepend the role instructions
        // (once) to what is actually sent to the agent.
        if !already_echoed {
            conv.push(
                session_id,
                vec![ConversationItem::UserMessage {
                    text: text.clone(),
                    timestamp: None,
                }],
            );
        }
        let prompt = match pending_instructions.take() {
            Some(instr) => format!("{instr}\n\n{text}"),
            None => text,
        };
        note_user_send(store, session_id, &mut cur_mode);

        let argv = turn_argv(&TurnArgs {
            prompt: &prompt,
            session_id,
            model: model.as_deref(),
            effort: effort.as_deref(),
            yolo: yolo_live.load(Ordering::Relaxed),
            mcp_config: mcp_path.as_deref(),
            mcp_servers: &expected_servers,
            extra_args: &config.extras.extra_args,
        });

        let turn = run_turn(
            store,
            conv,
            config,
            &argv,
            &expected_servers,
            &mut cur_mode,
            &mut acc,
            &mut rx,
            &mut drx,
            &mut irx,
            &mut mrx,
            &mut model,
            &mut effort,
        )
        .await;

        match turn {
            Ok((report, mid_turn)) => {
                queued.extend(mid_turn.into_iter().map(|text| (text, true)));
                apply_updates(
                    store,
                    conv,
                    session_id,
                    turn_outcome(&report),
                    &mut cur_mode,
                    &mut acc,
                );
            }
            Err(err) => {
                // Couldn't even start the process (missing binary, bad cwd).
                // That is a hard failure of the turn, never an Idle on its own.
                apply_updates(
                    store,
                    conv,
                    session_id,
                    vec![AgentUpdate::Error(format!("{err:#}")), AgentUpdate::Idle],
                    &mut cur_mode,
                    &mut acc,
                );
            }
        }
    }

    if let Some(path) = &mcp_path {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

/// Run one `copilot -p` process to completion, pumping its stdout JSONL through
/// [`translate`] + `apply_updates` and collecting everything [`turn_outcome`]
/// needs to judge it.
///
/// Returns the turn's report plus any user messages that arrived while it was
/// running — `-p` has no way to inject mid-turn, so they become the next turn's
/// prompts (already echoed into the conversation here so the user sees their
/// message land immediately).
#[allow(clippy::too_many_arguments)]
async fn run_turn(
    store: &SessionStore,
    conv: &ConversationStore,
    config: &SpawnConfig,
    argv: &[String],
    expected_servers: &[String],
    cur_mode: &mut SessionMode,
    acc: &mut UsageAcc,
    rx: &mut mpsc::UnboundedReceiver<String>,
    drx: &mut mpsc::UnboundedReceiver<bool>,
    irx: &mut mpsc::UnboundedReceiver<()>,
    mrx: &mut mpsc::UnboundedReceiver<ModelSwitch>,
    model: &mut Option<String>,
    effort: &mut Option<String>,
) -> anyhow::Result<(TurnReport, Vec<String>)> {
    let session_id = config.session_id.as_str();
    let mut child = Command::new(&config.bin)
        .args(argv)
        // Every TURN is its own process for this provider, so the profile's
        // COPILOT_HOME / token must be re-applied here rather than once at
        // session start — a session-scoped env would be right for a long-lived
        // child and silently wrong for this one. Merged, never replacing.
        .envs(&config.extras.env)
        .current_dir(&config.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning `{} -p`", config.bin))?;

    let mut stdout = BufReader::new(child.stdout.take().context("copilot stdout")?).lines();
    let mut stderr = BufReader::new(child.stderr.take().context("copilot stderr")?).lines();

    let mut report = TurnReport::default();
    let mut stderr_lines: Vec<String> = Vec::new();
    // Text already emitted per assistant message id, so a delta-less message
    // can be reconciled without double-rendering (see `message_tail`).
    let mut text = TextReconciler::default();
    // User messages that arrived while this turn was running; sent as the next
    // turn's prompt. `-p` has no way to inject mid-turn.
    let mut queued: Vec<String> = Vec::new();
    let exited: Option<std::process::ExitStatus>;
    // Once stderr hits EOF, `next_line()` resolves instantly forever; without
    // this guard the select would spin at 100% CPU for the rest of the turn.
    let mut stderr_done = false;

    loop {
        tokio::select! {
            line = stdout.next_line() => match line {
                Ok(Some(line)) => {
                    let line = line.trim();
                    if line.is_empty() { continue }
                    let Ok(event) = serde_json::from_str::<Value>(line) else { continue };
                    handle_event(
                        store, conv, session_id, &event, expected_servers,
                        cur_mode, acc, &mut report, &mut text,
                    );
                }
                Ok(None) => {
                    // stdout EOF — wait for the process itself, then stop.
                    exited = child.wait().await.ok();
                    break;
                }
                Err(err) => {
                    tracing::warn!(?err, session = %session_id, "copilot stdout read failed");
                    exited = child.wait().await.ok();
                    break;
                }
            },
            line = stderr.next_line(), if !stderr_done => match line {
                Ok(Some(line)) => {
                    let line = line.trim();
                    if !line.is_empty() { stderr_lines.push(line.to_string()) }
                }
                // EOF or a read error: stop polling this branch (see stderr_done).
                _ => stderr_done = true,
            },
            msg = rx.recv() => match msg {
                Some(text) => {
                    // Echo immediately so the user sees their message land, but
                    // hold the text for the next turn.
                    conv.push(session_id, vec![ConversationItem::UserMessage { text: text.clone(), timestamp: None }]);
                    queued.push(text);
                }
                None => {
                    // Session terminated mid-turn: kill the child and stop.
                    let _ = child.start_kill();
                    report.interrupted = true;
                    exited = child.wait().await.ok();
                    break;
                }
            },
            decision = drx.recv() => {
                if let Some(approve) = decision {
                    announce_decision(store, conv, session_id, approve, cur_mode, acc);
                    if !approve {
                        // "No" is the only decision with anything to act on: stop
                        // the turn, since nothing is parked waiting for a "yes".
                        let _ = child.start_kill();
                        report.interrupted = true;
                        exited = child.wait().await.ok();
                        break;
                    }
                }
            }
            intr = irx.recv() => {
                if intr.is_some() {
                    let _ = child.start_kill();
                    report.interrupted = true;
                    exited = child.wait().await.ok();
                    break;
                }
            }
            switch = mrx.recv() => {
                if let Some(sw) = switch {
                    // Applied to the NEXT turn's argv — this turn's process was
                    // already launched with the previous selection.
                    apply_switch(store, conv, session_id, sw, model, effort, cur_mode, acc);
                }
            }
        }
    }

    // Drain whatever stderr is still buffered before judging the turn — the
    // whole failure shape this guards against writes its prose there.
    if !stderr_done {
        while let Ok(Some(line)) = stderr.next_line().await {
            let line = line.trim();
            if !line.is_empty() {
                stderr_lines.push(line.to_string());
            }
        }
    }
    report.stderr = stderr_lines.join("\n");
    report.process_exit_code = exited.and_then(|s| s.code());
    Ok((report, queued))
}

/// Apply one stdout event: translate it, plus the two things translation can't
/// see (the per-message text tally, and whether our MCP servers attached).
#[allow(clippy::too_many_arguments)]
fn handle_event(
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    event: &Value,
    expected_servers: &[String],
    cur_mode: &mut SessionMode,
    acc: &mut UsageAcc,
    report: &mut TurnReport,
    text: &mut TextReconciler,
) {
    let ty = event.get("type").and_then(Value::as_str).unwrap_or("");
    let data = event.get("data").cloned().unwrap_or(Value::Null);
    let mut updates = translate(event);

    // Reconcile a message whose deltas didn't cover it (non-streaming path).
    if let Some(tail) = text.observe(event) {
        updates.push(AgentUpdate::AssistantText(tail));
    }

    match ty {
        // The dynamic capability cliff: an org policy can disable third-party
        // MCP servers, in which case Copilot reports `servers: []` and carries
        // on. Say so loudly — a facade-less supervisor must not pass for a
        // working one.
        "session.mcp_servers_loaded" => {
            let missing = missing_mcp_servers(expected_servers, &data);
            if !missing.is_empty() {
                updates.push(AgentUpdate::Error(format!(
                    "workspacer MCP server(s) did not attach to this Copilot session: {}. \
                     GitHub Copilot's org policy can disable third-party MCP servers; \
                     without them this agent has no workspacer tools (no supervisor \
                     dispatch, no structured questions).",
                    missing.join(", ")
                )));
            }
        }
        // The plan changed. The event says only that (`data` is empty — see
        // [`translate`]), so the todos themselves are read out of the session's
        // own db. Skipped when translation already produced a Plan, so a future
        // CLI that starts carrying the list on the wire wins over the file.
        "session.todos_changed" => {
            if !updates.iter().any(|u| matches!(u, AgentUpdate::Plan(_))) {
                if let Some(plan) = session_todos(session_id) {
                    updates.push(AgentUpdate::Plan(plan));
                }
            }
        }
        // The terminal event: the only place an exit code for the TURN appears.
        //
        // Also the plan's backstop. `session.todos_changed` is `ephemeral`, and
        // an ephemeral event is exactly the kind a process teardown can race, so
        // the last read is taken here where the turn is definitively over. The
        // plan is a last-write-wins full replacement, so a redundant re-read is
        // free and a missed one would leave the card a turn stale forever.
        "result" => {
            report.result_exit_code = event.get("exitCode").and_then(Value::as_i64);
            if let Some(plan) = session_todos(session_id) {
                updates.push(AgentUpdate::Plan(plan));
            }
        }
        _ => {}
    }

    if updates.iter().any(|u| {
        matches!(
            u,
            AgentUpdate::AssistantText(_) | AgentUpdate::ToolUse { .. }
        )
    }) {
        report.produced_output = true;
    }
    if updates.iter().any(|u| matches!(u, AgentUpdate::Error(_))) {
        report.saw_stream_error = true;
    }
    if !updates.is_empty() {
        apply_updates(store, conv, session_id, updates, cur_mode, acc);
    }
}

/// Answer an approval decision loudly.
///
/// Copilot's `-p` mode never parks an approval card — it cannot ask (see the
/// module doc), so this channel exists for two reasons: so the daemon's
/// `/approve` route doesn't fail closed on a copilot session, and so a "no"
/// still does the one thing it can meaningfully do, which is stop the turn (the
/// caller handles that part). Either way the user is TOLD, rather than having
/// their decision silently swallowed.
fn announce_decision(
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    approve: bool,
    cur_mode: &mut SessionMode,
    acc: &mut UsageAcc,
) {
    let note = if approve {
        "Copilot's non-interactive mode has no approval gate — tools already ran, so there was \
         nothing to approve."
    } else {
        "Copilot's non-interactive mode has no approval gate, so a denial can't block a specific \
         tool — stopping the turn instead."
    };
    apply_updates(
        store,
        conv,
        session_id,
        vec![AgentUpdate::Error(note.to_string())],
        cur_mode,
        acc,
    );
}

/// Apply a live model/effort switch. Both take effect on the NEXT turn, because
/// the next turn is a whole new process — which is why copilot can advertise
/// them as live at all.
#[allow(clippy::too_many_arguments)]
fn apply_switch(
    store: &SessionStore,
    conv: &ConversationStore,
    session_id: &str,
    sw: ModelSwitch,
    model: &mut Option<String>,
    effort: &mut Option<String>,
    cur_mode: &mut SessionMode,
    acc: &mut UsageAcc,
) {
    if let Some(e) = sw.effort {
        *effort = Some(e.clone());
        apply_updates(
            store,
            conv,
            session_id,
            vec![AgentUpdate::Effort(e)],
            cur_mode,
            acc,
        );
    }
    if let Some(m) = sw.model {
        // Reflect it on the status line now so the pill doesn't wait for the
        // next turn's usage event.
        apply_updates(
            store,
            conv,
            session_id,
            vec![AgentUpdate::Usage {
                model: Some(m.clone()),
                input_tokens: None,
                output_tokens: None,
                cached_input_tokens: None,
                cost_usd: None,
                context_tokens: None,
                context_window: None,
            }],
            cur_mode,
            acc,
        );
        *model = Some(m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Every fixture below is a verbatim line (or a faithful trim of one) from
    // .workspacer/reports/2026-08-28-copilot-wire-capture.jsonl — a real
    // `copilot -p --output-format json` capture taken against v1.0.81 on
    // 2026-08-28, not a docs-derived guess.

    /// The whole capture, replayed through the real translation + text
    /// reconciliation exactly as `handle_event` drives them.
    ///
    /// `testdata/copilot-wire-capture.jsonl` is two real `copilot -p
    /// --output-format json` runs against v1.0.81 on 2026-08-28 (one calling an
    /// MCP tool through a registered third-party server, one creating a file and
    /// running a shell command). Lines are verbatim except that opaque blobs the
    /// adapter never reads — encrypted reasoning, api ids, the full tool-schema
    /// and message-history dumps — were removed to keep the fixture reviewable;
    /// the full untrimmed capture lives at
    /// `.workspacer/reports/2026-08-28-copilot-wire-capture.jsonl`.
    fn replay_capture() -> (Vec<AgentUpdate>, Vec<Value>) {
        const CAPTURE: &str = include_str!("testdata/copilot-wire-capture.jsonl");
        let mut text = TextReconciler::default();
        let mut updates = Vec::new();
        let mut events = Vec::new();
        for line in CAPTURE.lines().filter(|l| !l.trim().is_empty()) {
            let event: Value = serde_json::from_str(line).expect("fixture line is JSON");
            updates.extend(translate(&event));
            if let Some(tail) = text.observe(&event) {
                updates.push(AgentUpdate::AssistantText(tail));
            }
            events.push(event);
        }
        (updates, events)
    }

    #[test]
    fn the_capture_reconstructs_every_assistant_message_exactly_once() {
        let (updates, events) = replay_capture();
        // What the CLI said the complete messages were.
        let expected: Vec<String> = events
            .iter()
            .filter(|e| e["type"] == "assistant.message")
            .filter_map(|e| e.pointer("/data/content").and_then(Value::as_str))
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        // Two of the five assistant messages in the capture are tool-call
        // messages with empty content; the other two carry the real replies.
        assert_eq!(expected.len(), 2, "the capture has two non-empty replies");

        // What we would have rendered: concatenated assistant text, split at
        // the turn boundaries the whole-message events mark.
        let rendered: String = updates
            .iter()
            .filter_map(|u| match u {
                AgentUpdate::AssistantText(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        let joined: String = expected.concat();
        assert_eq!(
            rendered, joined,
            "assistant text must reconstruct the CLI's own messages exactly — no \
             duplication from taking both the deltas and the whole message, and \
             nothing dropped"
        );
    }

    #[test]
    fn the_capture_pairs_every_tool_call_with_its_result() {
        let (updates, _) = replay_capture();
        let calls: Vec<&str> = updates
            .iter()
            .filter_map(|u| match u {
                AgentUpdate::ToolUse { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        let results: Vec<&str> = updates
            .iter()
            .filter_map(|u| match u {
                AgentUpdate::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(calls.len(), 3, "capture has three tool calls");
        assert_eq!(
            calls, results,
            "every ToolUse must be joined to a ToolResult by the same id, in order — \
             that pairing is what renders a call and its output as one card"
        );
        // No result in this capture failed, so nothing may be painted red.
        assert!(updates
            .iter()
            .all(|u| !matches!(u, AgentUpdate::ToolResult { is_error: true, .. })));
    }

    #[test]
    fn the_capture_never_yields_an_idle_or_an_error() {
        // Both runs in the fixture succeeded (exit 0, empty stderr, a `result`
        // event). If translation invented an `Idle` here the driver's
        // verdict would be bypassed entirely; if it invented an `Error`, two
        // clean turns would render as failures.
        let (updates, _) = replay_capture();
        assert!(!updates.contains(&AgentUpdate::Idle));
        assert!(!updates.iter().any(|u| matches!(u, AgentUpdate::Error(_))));
        // And the healthy shape does close out as a plain Idle.
        assert_eq!(turn_outcome(&good()), vec![AgentUpdate::Idle]);
    }

    #[test]
    fn the_capture_reports_real_tokens_and_a_context_window() {
        let (updates, _) = replay_capture();
        let usages: Vec<_> = updates
            .iter()
            .filter_map(|u| match u {
                AgentUpdate::Usage {
                    model,
                    input_tokens,
                    output_tokens,
                    context_window,
                    ..
                } => Some((
                    model.clone(),
                    *input_tokens,
                    *output_tokens,
                    *context_window,
                )),
                _ => None,
            })
            .collect();
        assert!(
            !usages.is_empty(),
            "the capture carries model.* usage events"
        );
        // Copilot's own window for the model it used — not the table's guess.
        assert!(
            usages.iter().any(|(_, _, _, w)| *w == Some(144_000)),
            "model.turn_started must surface Copilot's max_context_window_tokens"
        );
        // Real per-call token counts, and every one names its model.
        assert!(usages
            .iter()
            .any(|(_, i, o, _)| i.is_some_and(|v| v > 0) && o.is_some_and(|v| v > 0)));
        assert!(usages.iter().all(|(m, _, _, _)| m.is_some()));
        // Cost is never claimed from the wire: Copilot bills AI credits.
        assert!(updates.iter().all(|u| !matches!(
            u,
            AgentUpdate::Usage {
                cost_usd: Some(_),
                ..
            }
        )));
    }

    #[test]
    fn the_captures_models_are_all_priceable() {
        // Copilot's catalog spans four vendors. A model the pricing table can't
        // match yields a blank cost readout, so the ids the CLI actually reports
        // have to resolve — see session::pricing's BUILTIN and its TS twin.
        let (updates, _) = replay_capture();
        for u in &updates {
            if let AgentUpdate::Usage {
                model: Some(model), ..
            } = u
            {
                assert!(
                    crate::session::pricing::rates_for(model).is_some(),
                    "no pricing row matches the model copilot reported: {model}"
                );
            }
        }
    }

    #[test]
    fn assistant_delta_is_busy_plus_text() {
        let ev = json!({
            "type": "assistant.message_delta",
            "data": { "messageId": "17717801", "deltaContent": "4" },
            "ephemeral": true
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::Busy, AgentUpdate::AssistantText("4".into())]
        );
    }

    #[test]
    fn whole_message_carries_no_text_of_its_own() {
        // Both the deltas and the whole message carry the same content; taking
        // text from both would render every assistant reply twice.
        let ev = json!({
            "type": "assistant.message",
            "data": { "messageId": "m1", "model": "gpt-5-mini", "content": "4", "toolRequests": [] }
        });
        assert_eq!(translate(&ev), vec![AgentUpdate::Busy]);
    }

    #[test]
    fn message_tail_reconciles_a_delta_less_message() {
        // Normal streaming path: deltas covered it.
        assert_eq!(message_tail("hello world", "hello world"), None);
        // Partially streamed (stream cut short).
        assert_eq!(message_tail("hello", "hello world"), Some(" world".into()));
        // No deltas at all (--stream off): the whole message is new.
        assert_eq!(message_tail("", "hello"), Some("hello".into()));
        // Nothing to say.
        assert_eq!(message_tail("", ""), None);
        // Divergent (shouldn't happen): prefer losing nothing over duplicating.
        assert_eq!(message_tail("abc", "xyz"), None);
    }

    #[test]
    fn tool_execution_start_is_a_tool_use() {
        let ev = json!({
            "type": "tool.execution_start",
            "data": {
                "toolCallId": "toolu_01KWtV",
                "toolName": "bash",
                "arguments": { "command": "wc -c probe.txt", "description": "Count bytes" },
                "turnId": "0",
                "model": "claude-haiku-4.5"
            }
        });
        assert_eq!(
            translate(&ev),
            vec![
                AgentUpdate::Busy,
                AgentUpdate::ToolUse {
                    id: "toolu_01KWtV".into(),
                    name: "bash".into(),
                    input: json!({ "command": "wc -c probe.txt", "description": "Count bytes" }),
                }
            ]
        );
    }

    #[test]
    fn mcp_tool_keeps_its_qualified_name() {
        // The facade's tools arrive as `<server>-<tool>`; the GUI's cards key on
        // that qualified id, so it must survive translation intact.
        let ev = json!({
            "type": "tool.execution_start",
            "data": {
                "toolCallId": "toolu_01TJ",
                "toolName": "wksprobe-wks_probe_ping",
                "arguments": {},
                "toolTitle": "wks_probe_ping",
                "mcpServerName": "wksprobe",
                "mcpToolName": "wks_probe_ping"
            }
        });
        match &translate(&ev)[1] {
            AgentUpdate::ToolUse { name, .. } => assert_eq!(name, "wksprobe-wks_probe_ping"),
            other => panic!("expected ToolUse, got {other:?}"),
        }
    }

    #[test]
    fn tool_result_success_and_failure() {
        let ok = json!({
            "type": "tool.execution_complete",
            "data": {
                "toolCallId": "toolu_01KWtV",
                "success": true,
                "result": { "content": "6 probe.txt\n<shellId: 0 completed with exit code 0>" }
            }
        });
        assert_eq!(
            translate(&ok),
            vec![AgentUpdate::ToolResult {
                tool_use_id: "toolu_01KWtV".into(),
                content: "6 probe.txt\n<shellId: 0 completed with exit code 0>".into(),
                is_error: false,
            }]
        );

        // The path-confinement refusal, verbatim from the capture. `-p` mode
        // cannot ask, so this is what a "denied" looks like — and it MUST be
        // marked as an error or the pane renders a refusal as a normal result.
        let denied = json!({
            "type": "tool.execution_complete",
            "data": {
                "toolCallId": "call_S9ph",
                "success": false,
                "error": {
                    "message": "Permission denied and could not request permission from user",
                    "code": "denied"
                }
            }
        });
        assert_eq!(
            translate(&denied),
            vec![AgentUpdate::ToolResult {
                tool_use_id: "call_S9ph".into(),
                content: "Permission denied and could not request permission from user (denied)"
                    .into(),
                is_error: true,
            }]
        );
    }

    #[test]
    fn model_call_success_yields_real_token_usage() {
        let ev = json!({
            "type": "model.model_call_success",
            "data": {
                "kind": "model_call_success",
                "turn": 0,
                "modelCall": { "model": "claude-haiku-4.5" },
                "responseChunk": {
                    "model": "claude-haiku-4-5-20251001",
                    "usage": {
                        "prompt_tokens": 17515,
                        "completion_tokens": 368,
                        "total_tokens": 17883,
                        "prompt_tokens_details": { "cached_tokens": 11560, "cache_creation_tokens": 5945 }
                    }
                }
            }
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::Usage {
                model: Some("claude-haiku-4.5".into()),
                input_tokens: Some(17515),
                output_tokens: Some(368),
                cached_input_tokens: Some(11560),
                // AI credits are the only figure on the wire — never a dollar.
                cost_usd: None,
                context_tokens: Some(17883),
                context_window: None,
            }]
        );
    }

    #[test]
    fn turn_started_reports_copilots_own_context_window() {
        // 144000 is Copilot's window for claude-haiku-4.5 — not Anthropic's
        // 200k. A provider-reported window outranks the table, which is the
        // point of carrying it.
        let ev = json!({
            "type": "model.turn_started",
            "data": {
                "kind": "turn_started",
                "model": "claude-haiku-4.5",
                "modelInfo": {
                    "capabilities": { "limits": { "max_context_window_tokens": 144000 } }
                }
            }
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::Usage {
                model: Some("claude-haiku-4.5".into()),
                input_tokens: None,
                output_tokens: None,
                cached_input_tokens: None,
                cost_usd: None,
                context_tokens: None,
                context_window: Some(144_000),
            }]
        );
    }

    #[test]
    fn policy_warning_is_an_error_not_a_log_line() {
        // The exact event the scout captured when the org policy blocked
        // third-party MCP servers. Everything else on that run looked healthy.
        let ev = json!({
            "type": "session.warning",
            "data": {
                "message": "Third-party MCP servers are disabled by your organization's Copilot policy. Only built-in servers are available.",
                "warningType": "policy"
            },
            "ephemeral": true
        });
        assert_eq!(
            translate(&ev),
            vec![AgentUpdate::Error(
                "Third-party MCP servers are disabled by your organization's Copilot policy. Only built-in servers are available.".into()
            )]
        );
        // A non-policy warning is noise, not a session error.
        let noise = json!({
            "type": "session.warning",
            "data": { "message": "something minor", "warningType": "deprecation" }
        });
        assert!(translate(&noise).is_empty());
    }

    #[test]
    fn translate_never_emits_idle() {
        // THE trap (codex.rs:57-67, reached by a different road). `assistant.idle`
        // fires while the process is still alive and says nothing about whether
        // the turn succeeded — a bare Idle here would erase every failure whose
        // only evidence is stderr.
        for ty in [
            "assistant.idle",
            "assistant.turn_end",
            "model.turn_ended",
            "result",
            "session.usage_checkpoint",
        ] {
            let ev = json!({ "type": ty, "data": {} });
            assert!(
                !translate(&ev).contains(&AgentUpdate::Idle),
                "{ty} must not translate to a bare Idle"
            );
        }
    }

    #[test]
    fn unknown_and_malformed_events_are_ignored() {
        assert!(translate(&json!({})).is_empty());
        assert!(translate(&Value::Null).is_empty());
        assert!(
            translate(&json!({ "type": "session.background_tasks_changed", "data": {} }))
                .is_empty()
        );
        assert!(translate(&json!({ "type": "assistant.tool_call_delta" })).is_empty());
        assert!(
            translate(&json!({ "type": "model.captured_assignment_context", "data": {} }))
                .is_empty()
        );
    }

    // ── sub-agents (the second live capture) ───────────────────────────────

    /// The two-subagent run, replayed exactly as `handle_event` drives
    /// translation.
    ///
    /// `testdata/copilot-subagent-capture.jsonl` is one real `copilot -p
    /// --output-format json` run against v1.0.81 (2026-08-28) that dispatched
    /// two background `explore` agents in parallel. Verbatim except that long
    /// content strings were clipped and the 13 `session.background_tasks_changed`
    /// pings were cut to two (they are identical and empty — that IS the point
    /// of keeping any).
    fn replay_subagent_capture() -> (Vec<AgentUpdate>, Vec<Value>) {
        const CAPTURE: &str = include_str!("testdata/copilot-subagent-capture.jsonl");
        let mut text = TextReconciler::default();
        let mut updates = Vec::new();
        let mut events = Vec::new();
        for line in CAPTURE.lines().filter(|l| !l.trim().is_empty()) {
            let event: Value = serde_json::from_str(line).expect("fixture line is JSON");
            updates.extend(translate(&event));
            if let Some(tail) = text.observe(&event) {
                updates.push(AgentUpdate::AssistantText(tail));
            }
            events.push(event);
        }
        (updates, events)
    }

    fn subagent_updates(updates: &[AgentUpdate]) -> Vec<&SubagentUpdate> {
        updates
            .iter()
            .filter_map(|u| match u {
                AgentUpdate::Subagent(s) => Some(s),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_capture_opens_and_closes_a_row_for_each_subagent() {
        let (updates, _) = replay_subagent_capture();
        let subs = subagent_updates(&updates);
        let ids: std::collections::BTreeSet<&str> = subs.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids.len(), 2, "the run dispatched two agents");

        for id in ids {
            let mine: Vec<&&SubagentUpdate> = subs.iter().filter(|s| s.id == id).collect();
            let opened = mine
                .iter()
                .find(|s| s.status == SubagentStatus::Running)
                .expect("row opens");
            assert_eq!(opened.agent_type.as_deref(), Some("explore"));
            assert!(
                opened
                    .tool_use_id
                    .as_deref()
                    .is_some_and(|t| t.starts_with("toolu_")),
                "the row must carry the `task` call id that spawned it, so the GUI \
                 can nest it under its own tool card"
            );
            assert!(opened.description.is_some(), "row is labelled");
            // The row MUST close: a subagent row that never completes derives a
            // background-task count that holds its parent busy forever.
            let last = mine.last().expect("at least one update");
            assert_eq!(
                last.status,
                SubagentStatus::Complete,
                "the last word on agent {id} must be that it finished"
            );
        }
    }

    #[test]
    fn a_subagents_own_frames_never_become_the_parents_conversation() {
        let (updates, events) = replay_subagent_capture();
        // The capture really does carry the children's own traffic.
        let child_frames = events
            .iter()
            .filter(|e| {
                e.get("agentId").is_some()
                    && !e["type"].as_str().unwrap_or("").starts_with("subagent.")
            })
            .count();
        assert!(
            child_frames >= 10,
            "fixture carries the children's own frames"
        );

        // A subagent's dispatch prompt arrives as `user.message` on this stream.
        // Rendering it would show a message the user never typed.
        assert!(
            !updates
                .iter()
                .any(|u| matches!(u, AgentUpdate::UserText(_))),
            "no subagent prompt may surface as a user message"
        );
        // The children ran `view`; the parent ran `task` and `read_agent`.
        let parent_tools: Vec<&str> = updates
            .iter()
            .filter_map(|u| match u {
                AgentUpdate::ToolUse { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            !parent_tools.contains(&"view"),
            "a subagent's tool call must not render as the parent's: got {parent_tools:?}"
        );
        assert!(
            parent_tools.contains(&"task"),
            "the dispatch itself is the parent's"
        );
        // The children's reports must not be spliced into the parent's text.
        let rendered: String = updates
            .iter()
            .filter_map(|u| match u {
                AgentUpdate::AssistantText(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            !rendered.contains("Contents of"),
            "a subagent's own report leaked into the parent conversation"
        );
        // But its activity still shows up where it belongs — on its row.
        let subs = subagent_updates(&updates);
        assert!(
            subs.iter()
                .any(|s| s.last_tool_name.as_deref() == Some("view")),
            "the child's tool call should light up its row instead"
        );
    }

    #[test]
    fn background_tasks_changed_is_a_content_free_ping() {
        // Verified against two live captures (13 and 61 occurrences): every
        // `session.background_tasks_changed` copilot emits has an EMPTY `data`.
        // There is nothing to translate — no count, no task list, no ids — so
        // the busy/idle story is derived from the `subagent.*` rows instead.
        let (_, events) = replay_subagent_capture();
        let pings: Vec<&Value> = events
            .iter()
            .filter(|e| e["type"] == "session.background_tasks_changed")
            .collect();
        assert!(!pings.is_empty(), "the fixture keeps some");
        for ping in pings {
            assert_eq!(
                ping["data"],
                json!({}),
                "if this ever fails, copilot started carrying background-task \
                 detail and it should be wired through"
            );
            assert!(translate(ping).is_empty());
        }
    }

    // ── the plan: a SQLite table, not a wire payload ───────────────────────

    #[test]
    fn todos_changed_carries_nothing_so_the_plan_is_read_from_the_db() {
        // The live plan-shaped run emitted six of these, all with `data: {}`.
        let ping = json!({ "type": "session.todos_changed", "data": {}, "ephemeral": true });
        assert!(
            translate(&ping).is_empty(),
            "nothing to extract from the ping itself — `handle_event` reads the db"
        );
        // …but the payload probe is real: a future CLI that filled it in works.
        let filled = json!({ "type": "session.todos_changed", "data": {
            "todos": [{ "title": "a", "status": "in_progress" }]
        }});
        assert!(matches!(
            translate(&filled).as_slice(),
            [AgentUpdate::Plan(_)]
        ));
    }

    #[test]
    fn plan_from_todo_rows_maps_copilots_status_vocabulary() {
        // The four values copilot's `todos` table CHECK constraint allows,
        // in the row order the table is read in.
        let rows = vec![
            ("Creating hello.sh".to_string(), "done".to_string()),
            (
                "Making it executable".to_string(),
                "in_progress".to_string(),
            ),
            ("Running it".to_string(), "pending".to_string()),
            ("Publishing it".to_string(), "blocked".to_string()),
        ];
        let plan = plan_from_todo_rows(&rows).expect("four steps");
        assert_eq!(plan.steps.len(), 4);
        assert_eq!(plan.steps[0].content, "Creating hello.sh");
        assert_eq!(plan.steps[0].status, PlanStatus::Completed);
        assert_eq!(plan.steps[1].status, PlanStatus::InProgress);
        assert_eq!(plan.steps[2].status, PlanStatus::Pending);
        // `blocked` has no equivalent: not started is the honest reading, and
        // it must never read as done.
        assert_eq!(plan.steps[3].status, PlanStatus::Pending);
        // An empty table is "no plan", not an empty plan card.
        assert!(plan_from_todo_rows(&[]).is_none());
        assert!(plan_from_todo_rows(&[("  ".to_string(), "pending".to_string())]).is_none());
    }

    #[test]
    fn session_todos_reads_a_real_copilot_session_db() {
        // The schema is copilot's own (`CREATE TABLE todos` from the CLI's
        // session db, verbatim), so this proves the query matches the table the
        // CLI actually ships — column names included.
        let dir = std::env::temp_dir().join(format!("wks-copilot-todos-{}", uuid::Uuid::new_v4()));
        let session = "11111111-2222-3333-4444-555555555555";
        let state = dir.join(".copilot").join("session-state").join(session);
        std::fs::create_dir_all(&state).expect("temp session dir");
        {
            let conn = rusqlite::Connection::open(state.join("session.db")).expect("open");
            conn.execute_batch(
                "CREATE TABLE todos (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    description TEXT,
                    status TEXT DEFAULT 'pending' CHECK(status IN ('pending', 'in_progress', 'done', 'blocked')),
                    created_at TEXT DEFAULT (datetime('now')),
                    updated_at TEXT DEFAULT (datetime('now'))
                );
                 INSERT INTO todos (id, title, status) VALUES
                   ('create-hello-script', 'Creating hello.sh with echo command', 'done'),
                   ('make-executable', 'Making script executable', 'in_progress'),
                   ('run-hello-script', 'Running the hello.sh script', 'pending');",
            )
            .expect("copilot's own schema");
        }
        let plan = todos_at(&state.join("session.db")).expect("plan read back");
        assert_eq!(plan.steps.len(), 3);
        assert_eq!(plan.steps[0].content, "Creating hello.sh with echo command");
        assert_eq!(plan.steps[0].status, PlanStatus::Completed);
        assert_eq!(plan.steps[1].status, PlanStatus::InProgress);
        assert_eq!(plan.steps[2].status, PlanStatus::Pending);
        // A session that never wrote a todo has no db at all: no plan, no error.
        assert!(todos_at(&state.join("nope.db")).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── turn_outcome: the failure-shape guard ──────────────────────────────

    fn good() -> TurnReport {
        TurnReport {
            stderr: String::new(),
            result_exit_code: Some(0),
            process_exit_code: Some(0),
            produced_output: true,
            saw_stream_error: false,
            interrupted: false,
        }
    }

    #[test]
    fn a_clean_turn_is_just_idle() {
        assert_eq!(turn_outcome(&good()), vec![AgentUpdate::Idle]);
    }

    #[test]
    fn stderr_prose_with_exit_zero_is_an_error() {
        // The scout's exact observation: a hard policy refusal printed prose to
        // stderr and the process exited 0. Nothing on stdout said so. Keying on
        // the exit status alone renders a totally failed session as a clean
        // completion — which is what this whole guard exists to prevent.
        let report = TurnReport {
            stderr: "Error: Access denied by policy settings (Request ID: CF9C:…)".into(),
            result_exit_code: Some(0),
            process_exit_code: Some(0),
            produced_output: false,
            ..good()
        };
        assert_eq!(
            turn_outcome(&report),
            vec![
                AgentUpdate::Error(
                    "Error: Access denied by policy settings (Request ID: CF9C:…)".into()
                ),
                AgentUpdate::Idle
            ]
        );
    }

    #[test]
    fn a_rejected_model_is_an_error_not_a_silent_idle() {
        // Live-verified failure: `--model <id>` that the account can't use exits
        // 1 with prose on stderr and NO `result` event on stdout at all.
        let report = TurnReport {
            stderr: "Error: Model \"gpt-5-mini\" from --model flag is not available.".into(),
            result_exit_code: None,
            process_exit_code: Some(1),
            produced_output: false,
            ..good()
        };
        let out = turn_outcome(&report);
        assert!(matches!(out[0], AgentUpdate::Error(_)));
        assert_eq!(out[1], AgentUpdate::Idle);
    }

    #[test]
    fn a_missing_result_event_is_an_error_even_on_exit_zero() {
        // No stderr, exit 0, but the terminal `result` frame never arrived: the
        // process died mid-turn. "Looks fine" is exactly the wrong answer.
        let report = TurnReport {
            result_exit_code: None,
            produced_output: true,
            ..good()
        };
        assert_eq!(
            turn_outcome(&report),
            vec![
                AgentUpdate::Error(
                    "copilot exited without completing the turn (no `result` event)".into()
                ),
                AgentUpdate::Idle
            ]
        );
    }

    #[test]
    fn a_nonzero_result_exit_code_is_an_error() {
        let report = TurnReport {
            result_exit_code: Some(2),
            ..good()
        };
        assert_eq!(
            turn_outcome(&report),
            vec![
                AgentUpdate::Error("copilot reported the turn failed (exitCode 2)".into()),
                AgentUpdate::Idle
            ]
        );
    }

    #[test]
    fn a_turn_that_produced_nothing_is_an_error() {
        let report = TurnReport {
            produced_output: false,
            ..good()
        };
        assert_eq!(
            turn_outcome(&report),
            vec![
                AgentUpdate::Error("copilot finished the turn without producing any output".into()),
                AgentUpdate::Idle
            ]
        );
        // …unless the stream already reported the error itself, in which case
        // saying it twice is noise.
        let report = TurnReport {
            produced_output: false,
            saw_stream_error: true,
            ..good()
        };
        assert_eq!(turn_outcome(&report), vec![AgentUpdate::Idle]);
    }

    #[test]
    fn an_interrupted_turn_is_not_a_failure() {
        // The user hit stop. An incomplete turn is the expected outcome, and a
        // scary red error would be wrong.
        let report = TurnReport {
            interrupted: true,
            result_exit_code: None,
            process_exit_code: None,
            produced_output: false,
            stderr: "some teardown noise".into(),
            saw_stream_error: false,
        };
        assert_eq!(turn_outcome(&report), vec![AgentUpdate::Idle]);
    }

    #[test]
    fn idle_always_comes_last_so_the_pane_is_released() {
        for report in [
            good(),
            TurnReport {
                stderr: "boom".into(),
                ..good()
            },
            TurnReport {
                result_exit_code: None,
                ..good()
            },
            TurnReport {
                process_exit_code: Some(9),
                ..good()
            },
        ] {
            let out = turn_outcome(&report);
            assert_eq!(
                out.last(),
                Some(&AgentUpdate::Idle),
                "a turn must always end Idle so the session returns to Input: {report:?}"
            );
        }
    }

    // ── argv ───────────────────────────────────────────────────────────────

    fn argv_for(yolo: bool, model: Option<&str>, effort: Option<&str>) -> Vec<String> {
        turn_argv(&TurnArgs {
            prompt: "do the thing",
            session_id: "11111111-2222-3333-4444-555555555555",
            model,
            effort,
            yolo,
            mcp_config: None,
            mcp_servers: &[],
            extra_args: &[],
        })
    }

    #[test]
    fn argv_pins_the_session_id_on_every_turn() {
        // This one flag is what makes the whole one-shot design work: it creates
        // the session on turn 1 and resumes it on every later turn.
        let argv = argv_for(false, None, None);
        let idx = argv.iter().position(|a| a == "--session-id").unwrap();
        assert_eq!(argv[idx + 1], "11111111-2222-3333-4444-555555555555");
    }

    #[test]
    fn argv_never_shares_or_exports_the_session() {
        // A dispatched worker gisting a private repo's transcript, or a session
        // drivable from github.com, are both one flag away. Refuse explicitly.
        for yolo in [false, true] {
            let argv = argv_for(yolo, Some("auto"), Some("high"));
            for banned in ["--share", "--share-gist", "--remote", "--remote-export"] {
                assert!(
                    !argv.iter().any(|a| a == banned),
                    "{banned} must never be passed"
                );
            }
            for required in ["--no-remote", "--no-remote-export", "--no-auto-update"] {
                assert!(
                    argv.iter().any(|a| a == required),
                    "{required} must always be passed"
                );
            }
        }
    }

    #[test]
    fn yolo_is_the_only_thing_that_lifts_path_confinement() {
        // Verified against the CLI: with no allow flags tools still RUN but
        // writes outside the cwd come back `{"code":"denied"}`; `--allow-all`
        // lifts tools + paths + urls. That is the real two-tier mapping behind
        // the ask/yolo pill for this provider.
        let ask = argv_for(false, None, None);
        assert!(!ask.iter().any(|a| a.starts_with("--allow-all")));
        let yolo = argv_for(true, None, None);
        assert!(yolo.iter().any(|a| a == "--allow-all"));
    }

    #[test]
    fn model_and_effort_are_omitted_when_blank() {
        let argv = argv_for(false, Some("   "), Some(""));
        assert!(!argv.iter().any(|a| a == "--model"));
        assert!(!argv.iter().any(|a| a == "--effort"));

        let argv = argv_for(false, Some("auto"), Some("xhigh"));
        let m = argv.iter().position(|a| a == "--model").unwrap();
        assert_eq!(argv[m + 1], "auto");
        let e = argv.iter().position(|a| a == "--effort").unwrap();
        assert_eq!(argv[e + 1], "xhigh");
    }

    /// A profile's extra argv lands on the turn, AFTER everything the daemon
    /// insists on — so an extra arg can never un-say `--no-remote`.
    #[test]
    fn a_profiles_extra_argv_rides_the_turn_last() {
        let extra = vec!["--banner".to_string()];
        let argv = turn_argv(&TurnArgs {
            prompt: "p",
            session_id: "s",
            model: None,
            effort: None,
            yolo: false,
            mcp_config: None,
            mcp_servers: &[],
            extra_args: &extra,
        });
        assert_eq!(argv.last().unwrap(), "--banner");
        assert!(argv.contains(&"--no-remote".to_string()));
    }

    #[test]
    fn mcp_config_rides_in_as_an_at_prefixed_path() {
        let argv = turn_argv(&TurnArgs {
            prompt: "p",
            session_id: "s",
            model: None,
            effort: None,
            yolo: false,
            mcp_config: Some(std::path::Path::new("/tmp/wks.json")),
            mcp_servers: &[],
            extra_args: &[],
        });
        let i = argv
            .iter()
            .position(|a| a == "--additional-mcp-config")
            .unwrap();
        assert_eq!(argv[i + 1], "@/tmp/wks.json");
    }

    /// The facade is REGISTERED by `--additional-mcp-config` and made CALLABLE
    /// by `--allow-tool`. Registering without granting is the shape of bug that
    /// makes a Copilot Fleet Manager look healthy and dispatch nothing: `-p`
    /// cannot prompt, so an unapproved MCP call is denied outright rather than
    /// parked. Pinned per server name, at the ask tier, which is where it bites.
    #[test]
    fn the_registered_facade_servers_are_allowed_to_run_at_the_ask_tier() {
        let servers = vec!["workspacer".to_string(), "workspacer_ask".to_string()];
        let argv = turn_argv(&TurnArgs {
            prompt: "p",
            session_id: "s",
            model: None,
            effort: None,
            yolo: false,
            mcp_config: Some(std::path::Path::new("/tmp/wks.json")),
            mcp_servers: &servers,
            extra_args: &[],
        });
        let granted: Vec<&String> = argv
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "--allow-tool")
            .map(|(i, _)| &argv[i + 1])
            .collect();
        assert_eq!(granted, vec!["workspacer", "workspacer_ask"]);
        // The grant is scoped to the facade, NOT a blanket bypass: path/url
        // confinement is what still separates the ask tier from yolo.
        assert!(!argv.iter().any(|a| a.starts_with("--allow-all")));
    }

    /// No facade, no grant. A plain Copilot worker with neither the facade nor
    /// the ask shim must not pick up an allow-list it has no use for.
    #[test]
    fn a_session_with_no_mcp_config_gets_no_tool_grants() {
        let argv = argv_for(false, None, None);
        assert!(!argv.iter().any(|a| a == "--allow-tool"));
    }

    // ── MCP config + the dynamic capability cliff ──────────────────────────

    #[test]
    fn mcp_config_registers_facade_and_ask_over_http() {
        let doc = mcp_config(
            Some("http://127.0.0.1:9/mcp/tok"),
            Some("http://127.0.0.1:9/mcp/ask/s1"),
        )
        .expect("both servers");
        assert_eq!(doc["mcpServers"]["workspacer"]["type"], "http");
        assert_eq!(
            doc["mcpServers"]["workspacer"]["url"],
            "http://127.0.0.1:9/mcp/tok"
        );
        assert_eq!(
            doc["mcpServers"]["workspacer_ask"]["url"],
            "http://127.0.0.1:9/mcp/ask/s1"
        );
        // The ask endpoint parks on a human; a default request timeout would
        // kill the question the instant it was asked.
        assert_eq!(
            doc["mcpServers"]["workspacer_ask"]["timeout"],
            21_600_000u64
        );

        // A plain (non-supervisor) session still gets the ask endpoint.
        let doc = mcp_config(None, Some("http://x/ask")).expect("ask only");
        assert!(doc["mcpServers"].get("workspacer").is_none());
        assert!(doc["mcpServers"].get("workspacer_ask").is_some());

        // Nothing to register → no config file, no flag.
        assert!(mcp_config(None, None).is_none());
    }

    #[test]
    fn a_facade_that_did_not_attach_is_detected() {
        // Copilot's capability surface is the only DYNAMIC one in the fleet: an
        // org policy can disable third-party MCP servers, and the CLI then just
        // reports `servers: []` and carries on working. Silence there is what
        // makes a facade-less supervisor indistinguishable from a real one.
        let expected = vec!["workspacer".to_string(), "workspacer_ask".to_string()];

        // The policy-blocked shape the scout captured.
        assert_eq!(
            missing_mcp_servers(&expected, &json!({ "servers": [] })),
            vec!["workspacer".to_string(), "workspacer_ask".to_string()]
        );

        // The healthy shape, captured live once the policy allowed it.
        let ok = json!({ "servers": [
            { "name": "github-mcp-server", "status": "connected", "source": "builtin" },
            { "name": "workspacer", "status": "connected" },
            { "name": "workspacer_ask", "status": "connected" }
        ] });
        assert!(missing_mcp_servers(&expected, &ok).is_empty());

        // Present but not connected is still missing.
        let half = json!({ "servers": [
            { "name": "workspacer", "status": "failed" },
            { "name": "workspacer_ask", "status": "connected" }
        ] });
        assert_eq!(
            missing_mcp_servers(&expected, &half),
            vec!["workspacer".to_string()]
        );

        // Nothing registered → nothing can be missing.
        assert!(missing_mcp_servers(&[], &json!({ "servers": [] })).is_empty());
    }
}
