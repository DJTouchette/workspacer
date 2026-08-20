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
//! `transport: "stream"`, the desktop app's shipped default; the PTY path stays
//! available behind `transport: "pty"` and untouched). Claude
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
use crate::session::state::{
    Capabilities, ContextInventory, ContextItem, Pending, PendingQuestion, SessionMode,
};
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
    /// Reasoning-effort level (`--effort <level>`: `low`/`medium`/`high`/
    /// `xhigh`/`max`). Applied at spawn; there's no live control, so a change
    /// respawns (the composer pill drives that through the restart flow).
    pub effort: Option<String>,
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
            // Start of every turn: names the session's current model, and (once)
            // enumerates the session's capabilities.
            "init" => {
                if let Some(model) = value.get("model").and_then(Value::as_str) {
                    out.push(usage_model(model));
                }
                out.push(AgentUpdate::Capabilities(capabilities_from_init(value)));
            }
            // The CLI is calling the API — the turn is running.
            "status" if value.get("status").and_then(Value::as_str) == Some("requesting") => {
                out.push(AgentUpdate::Busy);
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
                    cached_input_tokens: None,
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
                cached_input_tokens: None,
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
        // event, named by `rateLimitType`; utilization is a 0–100 percent.
        //
        // The CLI's `rateLimitType` enum is
        // `five_hour | seven_day | seven_day_opus | seven_day_sonnet
        //  | seven_day_overage_included | overage`. We bucket it three ways:
        //   - `five_hour`      → the 5h window
        //   - any `seven_day*` → the weekly window
        //   - `overage`        → the monthly overage/credit window
        // Routing by the exact type matters: an `overage` event must NOT land
        // in the 5h field — the previous `starts_with("seven_day")` default did
        // exactly that, corrupting the 5h gauge — and the monthly window needs
        // its own bucket.
        "rate_limit_event" => {
            let info = value.get("rate_limit_info").unwrap_or(&Value::Null);
            let pct = info.get("utilization").and_then(Value::as_f64);
            let resets = info.get("resetsAt").and_then(Value::as_i64);
            let kind = info.get("rateLimitType").and_then(Value::as_str);
            if pct.is_some() || resets.is_some() {
                let is_seven_day = kind.is_some_and(|t| t.starts_with("seven_day"));
                let is_overage = kind == Some("overage");
                let bucket = |on: bool| if on { (pct, resets) } else { (None, None) };
                let (five_hour_pct, five_hour_resets_at) = bucket(!is_seven_day && !is_overage);
                let (seven_day_pct, seven_day_resets_at) = bucket(is_seven_day);
                let (monthly_pct, monthly_resets_at) = bucket(is_overage);
                out.push(AgentUpdate::RateLimits {
                    five_hour_pct,
                    five_hour_resets_at,
                    seven_day_pct,
                    seven_day_resets_at,
                    monthly_pct,
                    monthly_resets_at,
                });
            }
            // Status (distinct from utilization): a warning fires only when a
            // window crosses its threshold, so it's our main "approaching limit"
            // signal. Emitted on every event so a warning clears when the account
            // is comfortable again. `out_of_credits` marks a disabled overage.
            let status = info.get("status").and_then(Value::as_str);
            let warning =
                (status == Some("allowed_warning")).then(|| rate_limit_warning(kind, pct));
            let out_of_credits = info
                .get("overageDisabledReason")
                .and_then(Value::as_str)
                .map(|r| r == "out_of_credits");
            out.push(AgentUpdate::RateLimitStatus {
                warning,
                out_of_credits,
            });
        }
        _ => {}
    }
    out
}

/// If `value` is a `system/background_tasks_changed` frame, report whether any
/// background task is currently running; `None` for every other frame.
///
/// Parse a `background_tasks_changed` frame into `(holds_busy, live_count)`.
/// The CLI emits the full *live* task set on each change (verified against a
/// CLI 2.1.204 stream capture).
///
/// Only async SUBAGENT tasks (`task_type: "local_agent"` — the async
/// Agent/Task tool) mean "the agent is still working": their output lands
/// back in this conversation, so idling on the dispatch turn's `result` would
/// show idle mid-subagent. Every other type — `local_bash` (a
/// `run_in_background` shell: a dev server, a watcher, an agent-authored poll
/// loop), `local_workflow` (ambient/housekeeping per the CLI's own copy),
/// teammates — leaves the REPL interactive in Claude Code itself. Treating
/// those as busy latched sessions "responding" FOREVER once a background
/// shell outlived its turn (observed live: a poll loop grepping the wrong
/// file held a session busy for hours after its last turn ended). They ride
/// the wire as `background_tasks` instead, so clients can badge them without
/// the mode lying.
fn background_tasks_changed(value: &Value) -> Option<(bool, u32)> {
    if value.get("type").and_then(Value::as_str) != Some("system")
        || value.get("subtype").and_then(Value::as_str) != Some("background_tasks_changed")
    {
        return None;
    }
    let tasks = value.get("tasks").and_then(Value::as_array);
    let live = tasks.map_or(0, |t| t.len()) as u32;
    let busy = tasks.is_some_and(|t| {
        t.iter()
            .any(|task| task.get("task_type").and_then(Value::as_str) == Some("local_agent"))
    });
    Some((busy, live))
}

/// Whether a `result` frame is an interrupt or a fatal error — those always
/// idle (and abandon any running background task), never held busy. A plain
/// user-Esc reports `aborted_streaming`; genuine failures set `is_error`.
fn result_is_abort_or_error(value: &Value) -> bool {
    value.get("type").and_then(Value::as_str) == Some("result")
        && (value.get("terminal_reason").and_then(Value::as_str) == Some("aborted_streaming")
            || value
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false))
}

/// Extract the session's capabilities from the stream `system/init` frame.
/// Counts are array lengths; missing keys read as 0 / `None`.
fn capabilities_from_init(v: &Value) -> Capabilities {
    // Count array items or object keys — the CLI shapes some of these as lists
    // (skills, memory_paths) and others as name-keyed maps (agents, mcp_servers).
    let len = |key: &str| match v.get(key) {
        Some(Value::Array(a)) => a.len() as u32,
        Some(Value::Object(o)) => o.len() as u32,
        _ => 0,
    };
    // `fast_mode_state` shape isn't guaranteed — accept a bool, a known string,
    // or an object carrying an enabled/active flag.
    let fast_mode = match v.get("fast_mode_state") {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::String(s)) => Some(matches!(s.as_str(), "on" | "enabled" | "active")),
        Some(Value::Object(o)) => o
            .get("enabled")
            .or_else(|| o.get("active"))
            .and_then(Value::as_bool),
        _ => None,
    };
    Capabilities {
        fast_mode,
        output_style: v
            .get("output_style")
            .and_then(Value::as_str)
            .map(str::to_owned),
        api_key_source: v
            .get("apiKeySource")
            .and_then(Value::as_str)
            .map(str::to_owned),
        mcp_servers: len("mcp_servers"),
        skills: len("skills"),
        plugins: len("plugins"),
        agents: len("agents"),
        memory_files: len("memory_paths"),
        inventory: Some(inventory_from_init(v)),
    }
}

/// Itemize the init frame into a [`ContextInventory`] — names only, no disk
/// access (that's [`enrich_inventory`]'s job). Tolerant of both wire shapes the
/// CLI has used: string lists vs `{name, …}` object lists vs name-keyed maps.
fn inventory_from_init(v: &Value) -> ContextInventory {
    // A list of items that are either plain strings or objects with a `name`.
    fn named_items(v: Option<&Value>) -> Vec<ContextItem> {
        match v {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|e| match e {
                    Value::String(s) => Some(ContextItem {
                        name: s.clone(),
                        ..Default::default()
                    }),
                    Value::Object(o) => Some(ContextItem {
                        name: o.get("name").and_then(Value::as_str)?.to_owned(),
                        path: o.get("path").and_then(Value::as_str).map(str::to_owned),
                        status: o.get("status").and_then(Value::as_str).map(str::to_owned),
                        source: o.get("source").and_then(Value::as_str).map(str::to_owned),
                        ..Default::default()
                    }),
                    _ => None,
                })
                .collect(),
            // Name-keyed map (older frames shape mcp_servers/agents this way).
            Some(Value::Object(o)) => o
                .keys()
                .map(|name| ContextItem {
                    name: name.clone(),
                    ..Default::default()
                })
                .collect(),
            _ => Vec::new(),
        }
    }
    fn names(v: Option<&Value>) -> Vec<String> {
        match v {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        }
    }
    // `memory_paths` is a kind→path map ({"auto": "/…/memory/"}) or a plain
    // list of paths. Either way each entry is one item whose `path` enrichment
    // later expands into the actual files.
    let memory_files = match v.get("memory_paths") {
        Some(Value::Object(o)) => o
            .iter()
            .filter_map(|(kind, path)| {
                Some(ContextItem {
                    name: kind.clone(),
                    path: Some(path.as_str()?.to_owned()),
                    ..Default::default()
                })
            })
            .collect(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(|p| ContextItem {
                name: p.rsplit('/').next().unwrap_or(p).to_owned(),
                path: Some(p.to_owned()),
                ..Default::default()
            })
            .collect(),
        _ => Vec::new(),
    };
    ContextInventory {
        mcp_servers: named_items(v.get("mcp_servers")),
        skills: named_items(v.get("skills")),
        agents: named_items(v.get("agents")),
        plugins: named_items(v.get("plugins")),
        memory_files,
        tools: names(v.get("tools")),
        slash_commands: names(v.get("slash_commands")),
        claude_code_version: v
            .get("claude_code_version")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

/// Size a context item's backing file: bytes from disk, tokens estimated at
/// ~4 chars per token (the ballpark Claude Code itself uses for prose).
fn size_item(item: &mut ContextItem, path: &std::path::Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.is_file() {
            item.path = Some(path.to_string_lossy().into_owned());
            item.bytes = Some(meta.len());
            item.est_tokens = Some(meta.len().div_ceil(4));
        }
    }
}

/// The origin stamped on a skill or agent that resolves to no file anywhere.
/// The CLI ships a set of both compiled into its own binary — `skills` on a
/// live init frame lists deep-research, dataviz, verify, debug, run, … and
/// `agents` lists Explore, general-purpose, Plan — none of which exist on disk.
/// "No file" is the correct answer for those, not a lookup failure, and only a
/// label tells the two apart in the pane.
const BUILTIN_SOURCE: &str = "built-in";

/// One directory a Claude asset can live under, with the origin label stamped
/// onto whatever resolves inside it. A root is either `.claude`-shaped (the
/// project's or the user's) or a plugin directory; both lay out their assets as
/// `skills/<name>/SKILL.md`, `agents/<name>.md` and `commands/<name>.md`.
struct AssetRoot {
    label: String,
    dir: std::path::PathBuf,
}

/// How many plugin directories we're willing to scan for. Enumerating plugins
/// is a directory walk on every init frame, and a marketplace clone is
/// caller-controlled in size.
const MAX_PLUGIN_ROOTS: usize = 200;

/// Every root a skill or agent may live under, in PRECEDENCE order — the
/// project's own `.claude`, then the user's, then plugins. First match wins,
/// which is the order Claude Code resolves a name in.
///
/// The plugin roots are the leg that was missing: the frame's own `plugins`
/// list only carries the INSTALLED ones, while a marketplace's plugins are
/// active (and reported in `skills`) straight out of the marketplace clone —
/// `code-review` is reported as a skill and lives at
/// `plugins/marketplaces/<mp>/plugins/code-review/commands/code-review.md`.
fn asset_roots(inv: &ContextInventory, cwd: Option<&str>) -> Vec<AssetRoot> {
    let mut roots: Vec<AssetRoot> = Vec::new();
    if let Some(cwd) = cwd {
        roots.push(AssetRoot {
            label: "project".to_string(),
            dir: std::path::Path::new(cwd).join(".claude"),
        });
    }
    for user in user_claude_dirs() {
        roots.push(AssetRoot {
            label: "user".to_string(),
            dir: user,
        });
    }

    // Plugins the frame already told us about, path included.
    for plugin in &inv.plugins {
        if let Some(path) = plugin.path.as_deref() {
            roots.push(AssetRoot {
                label: plugin.name.clone(),
                dir: std::path::PathBuf::from(path),
            });
        }
    }

    // …plus the ones on disk it didn't. `cache/<marketplace>/<plugin>/<version>`
    // for installed plugins, `marketplaces/<mp>/plugins/<plugin>` for the clone.
    // (label, dir) pairs, collected before they touch `roots` so the dedupe can
    // see the roots already there.
    let mut discovered: Vec<(String, std::path::PathBuf)> = Vec::new();
    for user in user_claude_dirs() {
        let plugins = user.join("plugins");
        for marketplace in read_dirs(&plugins.join("cache")) {
            for plugin in read_dirs(&marketplace) {
                let label = dir_name(&plugin);
                // The VERSION directory is the plugin root, not the plugin
                // directory: `cache/<marketplace>/<plugin>/<version>/skills/…`.
                for version in read_dirs(&plugin) {
                    discovered.push((label.clone(), version));
                }
            }
        }
        for marketplace in read_dirs(&plugins.join("marketplaces")) {
            for plugin in read_dirs(&marketplace.join("plugins")) {
                discovered.push((dir_name(&plugin), plugin));
            }
        }
    }
    for (label, dir) in discovered {
        if roots.len() >= MAX_PLUGIN_ROOTS {
            break;
        }
        if label.is_empty() || roots.iter().any(|r| r.dir == dir) {
            continue;
        }
        roots.push(AssetRoot { label, dir });
    }
    roots
}

/// A directory's own name, or "" when it has none (a root path).
fn dir_name(dir: &std::path::Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Claude Code's own config roots, where the user's skills, agents and plugins
/// live. `CLAUDE_CONFIG_DIR` relocates it and the CLI honours that, so an
/// install setting it would otherwise resolve none of the user's assets and
/// report every one of them as "built-in".
///
/// BOTH are returned, not one: a Claude *profile* sets `CLAUDE_CONFIG_DIR` per
/// spawn (claudeSpawn.ts / managedSpawn.ts), and that per-session value is not
/// in this daemon's own environment. Trying the default `~/.claude` as well
/// costs one stat and is the difference between resolving a profile user's
/// skills and labelling all of them built-in. Precedence still favours the
/// explicit override. Mirrors `userClaudeDir` in libraryService.ts, which runs
/// in the desktop main process and reads only its own env for the same reason.
fn user_claude_dirs() -> Vec<std::path::PathBuf> {
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !dir.trim().is_empty() {
            out.push(std::path::PathBuf::from(dir.trim()));
        }
    }
    if let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().join(".claude")) {
        if !out.contains(&home) {
            out.push(home);
        }
    }
    out
}

/// Immediate subdirectories of `dir`, sorted for a stable precedence order.
/// Missing/unreadable directories read as empty — this whole pass is
/// best-effort.
fn read_dirs(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

/// Resolve one named asset to its file, returning the origin label of the root
/// it was found under. `candidates` are tried in order within each root before
/// moving to the next root, so a project's own copy always beats a plugin's.
fn resolve_asset(
    roots: &[AssetRoot],
    candidates: &[std::path::PathBuf],
) -> Option<(std::path::PathBuf, String)> {
    for root in roots {
        for rel in candidates {
            let full = root.dir.join(rel);
            if full.is_file() {
                return Some((full, root.label.clone()));
            }
        }
    }
    None
}

/// How much of a `description:` we keep. These ride the statusLine frame to
/// every client on every tick, and a skill's description is a model-facing
/// trigger list that runs to several hundred words; the pane renders one line.
const MAX_DESCRIPTION_CHARS: usize = 300;

/// Pull `description:` out of a markdown file's YAML frontmatter. Handles both
/// shapes Claude Code writes: a scalar on one line, and a `>`/`|` block whose
/// indented continuation lines fold into one. Deliberately not a YAML parse —
/// this frontmatter is flat, and a malformed file should cost a description,
/// not the inventory.
fn frontmatter_description(path: &std::path::Path) -> Option<String> {
    // Frontmatter is at the top; a skill can carry a large body (and bundled
    // references) that there is no reason to read.
    const MAX_FRONTMATTER_BYTES: usize = 16 * 1024;
    let bytes = std::fs::read(path).ok()?;
    let head = &bytes[..bytes.len().min(MAX_FRONTMATTER_BYTES)];
    let text = String::from_utf8_lossy(head);
    let mut lines = text.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    while let Some(line) = lines.next() {
        if line.trim_end() == "---" {
            return None; // frontmatter ended without one
        }
        let Some(rest) = line.strip_prefix("description:") else {
            continue;
        };
        let rest = rest.trim();
        // A block scalar (`>`, `|`, `>-`, `|2`, …): the value is the indented
        // lines that follow, folded into one.
        if rest.is_empty() || rest.starts_with('>') || rest.starts_with('|') {
            let mut parts: Vec<&str> = Vec::new();
            for cont in lines {
                if cont.trim().is_empty() {
                    continue;
                }
                if !cont.starts_with(' ') && !cont.starts_with('\t') {
                    break; // back to column 0 — the next key, or the closing ---
                }
                parts.push(cont.trim());
            }
            return clamp_description(&parts.join(" "));
        }
        let unquoted = rest
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .or_else(|| rest.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(rest);
        return clamp_description(unquoted);
    }
    None
}

/// Trim, drop empties, and cut to [`MAX_DESCRIPTION_CHARS`] on a CHAR boundary
/// (a byte slice would panic mid-UTF-8 on the first non-ASCII description).
fn clamp_description(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if s.chars().count() <= MAX_DESCRIPTION_CHARS {
        return Some(s.to_string());
    }
    let cut: String = s.chars().take(MAX_DESCRIPTION_CHARS).collect();
    Some(format!("{}…", cut.trim_end()))
}

/// Best-effort disk enrichment of an inventory: resolve skills/agents to their
/// file in whichever root owns them (project, user, or a plugin), expand memory
/// directories into the files inside them, and stamp `source`, `description`,
/// `bytes` and `est_tokens` on everything file-backed. Runs once per init frame.
/// Anything that resolves nowhere keeps its name and is labelled
/// [`BUILTIN_SOURCE`], because that is what it is.
fn enrich_inventory(caps: &mut Capabilities, cwd: Option<&str>) {
    let Some(inv) = caps.inventory.as_mut() else {
        return;
    };
    let roots = asset_roots(inv, cwd);

    // Skills live at <root>/skills/<name>/SKILL.md — except a plugin's slash
    // command, which the frame also reports in `skills` (verified live) and
    // which lives at <root>/commands/<name>.md.
    for skill in &mut inv.skills {
        if skill.path.is_some() {
            continue;
        }
        let candidates = [
            std::path::Path::new("skills")
                .join(&skill.name)
                .join("SKILL.md"),
            std::path::Path::new("commands").join(format!("{}.md", skill.name)),
        ];
        match resolve_asset(&roots, &candidates) {
            Some((path, label)) => {
                size_item(skill, &path);
                skill.source.get_or_insert(label);
                skill.description = frontmatter_description(&path);
            }
            None => {
                skill
                    .source
                    .get_or_insert_with(|| BUILTIN_SOURCE.to_string());
            }
        }
    }
    for agent in &mut inv.agents {
        if agent.path.is_some() {
            continue;
        }
        let candidates = [std::path::Path::new("agents").join(format!("{}.md", agent.name))];
        match resolve_asset(&roots, &candidates) {
            Some((path, label)) => {
                size_item(agent, &path);
                agent.source.get_or_insert(label);
                agent.description = frontmatter_description(&path);
            }
            None => {
                agent
                    .source
                    .get_or_insert_with(|| BUILTIN_SOURCE.to_string());
            }
        }
    }

    // Memory entries are usually directories — expand each into the files
    // inside (two levels deep, capped) so the pane can show real files.
    const MAX_MEMORY_FILES: usize = 100;
    let mut expanded: Vec<ContextItem> = Vec::new();
    for entry in inv.memory_files.drain(..) {
        let Some(path) = entry.path.as_deref() else {
            expanded.push(entry);
            continue;
        };
        let path = std::path::Path::new(path);
        if path.is_file() {
            let mut item = entry.clone();
            size_item(&mut item, path);
            expanded.push(item);
            continue;
        }
        if !path.is_dir() {
            expanded.push(entry);
            continue;
        }
        let mut files = list_files(path, 2, MAX_MEMORY_FILES);
        files.sort();
        if files.is_empty() {
            expanded.push(entry.clone()); // keep the bare dir so the count stays honest
            continue;
        }
        for file in files {
            let name = file
                .strip_prefix(path)
                .unwrap_or(&file)
                .to_string_lossy()
                .into_owned();
            let mut item = ContextItem {
                name,
                source: Some(entry.name.clone()),
                ..Default::default()
            };
            size_item(&mut item, &file);
            expanded.push(item);
        }
    }
    inv.memory_files = expanded;
}

/// Regular files under `dir`, up to `depth` levels deep, capped at `max`.
fn list_files(dir: &std::path::Path, depth: usize, max: usize) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        if out.len() >= max {
            break;
        }
        let path = entry.path();
        if path.is_file() {
            out.push(path);
        } else if depth > 0 && path.is_dir() {
            let remaining = max - out.len();
            out.extend(list_files(&path, depth - 1, remaining));
        }
    }
    out
}

/// Human warning string for a window that crossed its threshold, mirroring the
/// CLI's own "You're close to your …" phrasing.
fn rate_limit_warning(kind: Option<&str>, pct: Option<f64>) -> String {
    let label = match kind {
        Some("overage") => "monthly",
        Some(t) if t.starts_with("seven_day") => "7-day",
        _ => "5-hour",
    };
    match pct {
        Some(p) => format!("You're close to your {label} usage limit — {p:.0}% used"),
        None => format!("You're close to your {label} usage limit"),
    }
}

fn usage_model(model: &str) -> AgentUpdate {
    AgentUpdate::Usage {
        model: Some(model.to_string()),
        input_tokens: None,
        output_tokens: None,
        cached_input_tokens: None,
        cost_usd: None,
        context_tokens: None,
        context_window: None,
    }
}

/// Context occupancy from an assistant message's `usage`: the request's INPUT
/// side only (fresh + cached input) — the same definition Claude Code's own
/// /context uses and the transcript-derived calc in session/usage.rs applies.
/// Output tokens are deliberately excluded: they only join the window as next
/// turn's input, and counting them here made the stream gauge read higher than
/// the PTY statusline for the same session.
fn context_tokens_from(usage: Option<&Value>) -> Option<u64> {
    let u = usage?;
    let get = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    let total =
        get("input_tokens") + get("cache_read_input_tokens") + get("cache_creation_input_tokens");
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
    // Numeric-guess resolution: a 1-indexed option number maps to its label;
    // anything else passes through as free text. This is the legacy behavior
    // the TUI and older clients depend on (they only send bare `answers`).
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
    let mut record = |q: &Value, value: String| {
        if let Some(question) = q.get("question").and_then(Value::as_str) {
            answers.insert(question.to_string(), Value::String(value));
        }
    };
    if let Some(list) = &ans.answers {
        // When the client tags each answer's kind (index-aligned with
        // `answers`), honor it exactly: `"text"` is a literal free-text answer
        // that must NOT be numerically remapped even if it parses as an option
        // index (e.g. a user literally typing "3"); `"option"` keeps the
        // digit→label mapping. A missing/short/empty kinds array falls back to
        // the numeric-guess heuristic per entry, so bare `answers` from the TUI
        // behave exactly as before.
        let kinds = ans.answer_kinds.as_ref();
        for (i, (q, raw)) in questions.iter().zip(list).enumerate() {
            let value = match kinds.and_then(|k| k.get(i)).map(String::as_str) {
                Some("text") => raw.clone(),
                _ => resolve(q, raw),
            };
            record(q, value);
        }
    } else if let Some(first) = questions.first() {
        // Single-question fast-path keeps distinct `option`/`text` fields, so
        // it never needs kind tags.
        if let Some(opt) = ans.option {
            let s = opt.to_string();
            record(first, resolve(first, &s));
        } else if let Some(text) = &ans.text {
            record(first, resolve(first, text));
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

/// If the user's sent text invokes a slash command the CLI reported at init
/// (`/name [args…]`), build the `SlashCommand` conversation item — the same
/// one the transcript tailer emits for the CLI's echo row (clients dedup the
/// pair). Text naming no known command is literal prose to the CLI, so it
/// stays a plain user message. Before init reports capabilities we can't
/// tell, and conservatively treat everything as prose.
fn slash_command_send_item(text: &str, acc: &UsageAcc) -> Option<ConversationItem> {
    let rest = text.trim().strip_prefix('/')?;
    let (name, args) = match rest.split_once(char::is_whitespace) {
        Some((n, a)) => (n, a.trim()),
        None => (rest, ""),
    };
    if name.is_empty() {
        return None;
    }
    let known = acc
        .capabilities
        .as_ref()?
        .inventory
        .as_ref()?
        .slash_commands
        .iter()
        .any(|c| c.trim_start_matches('/') == name);
    known.then(|| ConversationItem::SlashCommand {
        name: name.to_string(),
        args: (!args.is_empty()).then(|| args.to_string()),
        timestamp: None,
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
    let generation = store.claim_generation(&cfg.session_id);
    tokio::spawn(async move {
        let session_id = cfg.session_id.clone();
        if let Err(err) = run_session(&store, &conv, cfg).await {
            tracing::warn!(?err, session = %session_id, "claude stream session ended with error");
        }
        // Child gone → a Stopped, resumable row (same lifecycle as PTY spawns).
        // Both drops belong to this lifetime or to neither. Forgetting the
        // conversation unconditionally was the hole left beside the guarded
        // teardown: for a driver-fed provider there is no transcript to rebuild
        // from, so a superseded exit erased the successor's visible history for
        // good.
        if store.deregister_managed(&session_id, generation) {
            conv.forget(&session_id);
        }
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
    if let Some(effort) = &cfg.effort {
        argv.extend(["--effort".into(), effort.clone()]);
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
    // Whether an async Agent/Task SUBAGENT is running in the background. While
    // true, the parent's dispatch-turn `result` is held busy instead of idling
    // the session (the subagent is still working). Ambient background tasks
    // (`local_bash` shells etc.) never set this — they only feed the session's
    // wire-visible `background_tasks` count.
    let mut bg_tasks_active = false;
    // Whether a turn-closing `result`'s Idle was swallowed because a subagent
    // was still running — the drain frame owes it back (see handle_line).
    let mut idle_suppressed = false;
    // A resumed session reuses its row; a previous life's task count must not
    // survive into this one.
    store.set_background_tasks(&session_id, 0);
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
                        &mut pending_controls, &mut bg_tasks_active, &mut idle_suppressed,
                    );
                }
                Ok(None) => break, // stdout EOF — child is exiting
                Err(err) => return Err(err.into()),
            },
            msg = rx.recv() => match msg {
                Some(text) => {
                    // Echo the user's message, but prepend the role
                    // instructions (once) to what's actually sent to the agent.
                    // A message invoking a known slash command echoes as the
                    // same SlashCommand item the transcript tailer emits for
                    // the CLI's echo row (clients dedup the pair), so the GUI
                    // shows a command card immediately instead of a raw
                    // "/foo" bubble.
                    let echo = slash_command_send_item(&text, &acc)
                        .unwrap_or(ConversationItem::UserMessage { text: text.clone(), timestamp: None });
                    conv.push(&session_id, vec![echo]);
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
                    // turn runs the new model). `--effort` is spawn-time only —
                    // there's no live control, so the composer drives an effort
                    // change through the restart flow, never this endpoint. If
                    // one still arrives, note and drop it rather than failing.
                    if sw.effort.is_some() {
                        tracing::debug!(session = %session_id, "claude stream: `effort` is spawn-time only (respawn to change) — ignored live");
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
                        &mut pending_controls, &mut bg_tasks_active, &mut idle_suppressed,
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
    bg_tasks_active: &mut bool,
    idle_suppressed: &mut bool,
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
            // A live AskUserQuestion also owns that single slot; don't overwrite
            // it — the approval re-surfaces via the arx branch once the question
            // is answered (otherwise the question is never answered and the CLI
            // turn wedges forever).
            if pending_approvals.len() == 1 && pending_question.is_none() {
                surface_approval(store, session_id, cur_mode, &pending_approvals[0]);
            }
        }
        _ => {
            // The async Agent/Task tool runs as a *background task*: the parent's
            // dispatch turn ends (emitting a `result`) while the subagent keeps
            // working, and a fresh turn resumes only once it drains. Track the
            // live task set so that turn-closing `result` doesn't flip the parent
            // idle mid-subagent — the real idle rides the trailing `result` after
            // the task set empties. (Verified against a CLI 2.1.204 capture.)
            // Only agent tasks hold busy; ambient tasks (background shells, …)
            // update the wire count and nothing else — see
            // [`background_tasks_changed`] for why.
            if let Some((agent_tasks, live_tasks)) = background_tasks_changed(value) {
                let was_holding = *bg_tasks_active;
                *bg_tasks_active = agent_tasks;
                store.set_background_tasks(session_id, live_tasks);
                if agent_tasks && *cur_mode == SessionMode::Input {
                    // A subagent is running while the parent looks idle (its own
                    // turn already closed) — reassert working. Guarded to Input so
                    // a parked approval/question is never clobbered.
                    apply_updates(
                        store,
                        conv,
                        session_id,
                        vec![AgentUpdate::Busy],
                        cur_mode,
                        acc,
                    );
                } else if was_holding && !agent_tasks && *idle_suppressed {
                    // The last busy-holding subagent drained and the parent's own
                    // `result` already had its Idle suppressed — pay the idle debt
                    // NOW. The trailing `result` usually does this, but when other
                    // ambient tasks (a background shell) keep the set non-empty
                    // the CLI never closes the set, and waiting latched the
                    // session busy. Guarded to Responding so a parked
                    // approval/question survives.
                    *idle_suppressed = false;
                    if *cur_mode == SessionMode::Responding {
                        apply_updates(
                            store,
                            conv,
                            session_id,
                            vec![AgentUpdate::Idle],
                            cur_mode,
                            acc,
                        );
                    }
                }
                return;
            }

            let is_result = value.get("type").and_then(Value::as_str) == Some("result");
            let abort_or_error = result_is_abort_or_error(value);
            // Hold the turn-closing `result` busy iff a background SUBAGENT is
            // still running and this isn't an interrupt/error.
            let suppress_idle = is_result && *bg_tasks_active && !abort_or_error;
            if abort_or_error {
                *bg_tasks_active = false;
            }

            // A real turn-closing `result` drops any dead parked `can_use_tool`
            // requests so a later `/approve` or `/answer` can't answer a canceled
            // one. A mid-subagent dispatch `result` is NOT the end — keep the
            // parked cards.
            if is_result && !suppress_idle {
                pending_approvals.clear();
                *pending_question = None;
            }
            let mut updates = translate(value, totals);
            if suppress_idle {
                let before = updates.len();
                updates.retain(|u| !matches!(u, AgentUpdate::Idle));
                // Remember the swallowed Idle: the drain branch above owes it
                // back if the subagent finishes without a trailing `result`.
                if updates.len() < before {
                    *idle_suppressed = true;
                }
            } else if updates
                .iter()
                .any(|u| matches!(u, AgentUpdate::Busy | AgentUpdate::Idle))
            {
                // Fresh activity (a new turn's Busy) or a delivered Idle settles
                // the debt — a later subagent drain must not flip a LIVE turn
                // back to Input.
                *idle_suppressed = false;
            }
            // The init frame's inventory is names-only (translate is pure);
            // stamp file sizes on it here, where disk access is fine.
            for update in &mut updates {
                if let AgentUpdate::Capabilities(caps) = update {
                    enrich_inventory(caps, value.get("cwd").and_then(Value::as_str));
                }
            }
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
        // init also emits a Capabilities update; the model naming must still be present.
        let updates =
            t(json!({ "type": "system", "subtype": "init", "model": "claude-haiku-4-5" }));
        assert!(updates.contains(&usage_model("claude-haiku-4-5")));
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
                cached_input_tokens: None,
                cost_usd: None,
                // Input side only — output joins the window next turn.
                context_tokens: Some(10 + 22474 + 4427),
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
    fn background_tasks_changed_reports_live_task_set() {
        // A live SUBAGENT (local_agent) holds the parent busy; the CLI sends
        // the full live set on each change.
        assert_eq!(
            background_tasks_changed(&json!({ "type": "system",
                "subtype": "background_tasks_changed",
                "tasks": [{ "task_id": "a", "task_type": "local_agent" }] })),
            Some((true, 1))
        );
        assert_eq!(
            background_tasks_changed(&json!({ "type": "system",
                "subtype": "background_tasks_changed", "tasks": [] })),
            Some((false, 0))
        );
        // A background SHELL (run_in_background Bash — a dev server, a poll
        // loop) is ambient: it counts, but it must NOT hold the session busy.
        // Treating it as busy latched idle sessions "responding" forever when
        // a shell outlived its turn.
        assert_eq!(
            background_tasks_changed(&json!({ "type": "system",
                "subtype": "background_tasks_changed",
                "tasks": [{ "task_id": "b", "task_type": "local_bash" }] })),
            Some((false, 1))
        );
        // Mixed set: busy while the agent lives, and the count is the whole set.
        assert_eq!(
            background_tasks_changed(&json!({ "type": "system",
                "subtype": "background_tasks_changed",
                "tasks": [{ "task_id": "a", "task_type": "local_agent" },
                          { "task_id": "b", "task_type": "local_bash" }] })),
            Some((true, 2))
        );
        // Unknown/future task types default to ambient — the failure mode of
        // wrongly holding busy is a permanent lie, wrongly idling self-heals.
        assert_eq!(
            background_tasks_changed(&json!({ "type": "system",
                "subtype": "background_tasks_changed",
                "tasks": [{ "task_id": "c", "task_type": "local_workflow" }] })),
            Some((false, 1))
        );
        // Any other frame is None — it isn't a background-task signal.
        assert_eq!(
            background_tasks_changed(&json!({ "type": "system", "subtype": "init" })),
            None
        );
        assert_eq!(
            background_tasks_changed(&json!({ "type": "result", "subtype": "success" })),
            None
        );
        // background_tasks_changed carries nothing for `translate` — it's handled
        // by the driver, not the pure event mapper.
        assert!(t(
            json!({ "type": "system", "subtype": "background_tasks_changed",
            "tasks": [{ "task_id": "a" }] })
        )
        .is_empty());
    }

    #[test]
    fn result_abort_and_error_are_flagged_success_is_not() {
        // Success closes the turn normally (held busy only while a task runs).
        assert!(!result_is_abort_or_error(
            &json!({ "type": "result", "subtype": "success", "is_error": false })
        ));
        // A user Esc reports aborted_streaming — always idles.
        assert!(result_is_abort_or_error(&json!({ "type": "result",
            "subtype": "error_during_execution", "terminal_reason": "aborted_streaming" })));
        // A genuine failure — always idles, abandoning any running task.
        assert!(result_is_abort_or_error(
            &json!({ "type": "result", "subtype": "error_during_execution", "is_error": true })
        ));
        // Non-result frames are never abort/error.
        assert!(!result_is_abort_or_error(&json!({ "type": "assistant" })));
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
                    cached_input_tokens: None,
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
                cached_input_tokens: None,
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
        assert!(updates.contains(&AgentUpdate::RateLimits {
            five_hour_pct: Some(19.0),
            five_hour_resets_at: Some(1783314600),
            seven_day_pct: None,
            seven_day_resets_at: None,
            monthly_pct: None,
            monthly_resets_at: None,
        }));
        let updates = t(json!({ "type": "rate_limit_event", "rate_limit_info": {
            "status": "allowed", "resetsAt": 1783914600, "rateLimitType": "seven_day_sonnet",
            "utilization": 3.0 } }));
        assert!(updates.contains(&AgentUpdate::RateLimits {
            five_hour_pct: None,
            five_hour_resets_at: None,
            seven_day_pct: Some(3.0),
            seven_day_resets_at: Some(1783914600),
            monthly_pct: None,
            monthly_resets_at: None,
        }));
        // `overage` is the monthly window — it must land in `monthly_*`, never
        // in the 5h fields (the bug this parser rewrite fixes).
        let updates = t(json!({ "type": "rate_limit_event", "rate_limit_info": {
            "status": "allowed_warning", "resetsAt": 1785000000, "rateLimitType": "overage",
            "utilization": 61.0 } }));
        assert!(updates.contains(&AgentUpdate::RateLimits {
            five_hour_pct: None,
            five_hour_resets_at: None,
            seven_day_pct: None,
            seven_day_resets_at: None,
            monthly_pct: Some(61.0),
            monthly_resets_at: Some(1785000000),
        }));
    }

    #[test]
    fn rate_limit_event_emits_status_and_warning() {
        // A comfortable window: no warning, no overage flag (real-world shape
        // captured from the CLI — reset present, utilization absent).
        let updates = t(json!({ "type": "rate_limit_event", "rate_limit_info": {
            "status": "allowed", "resetsAt": 1783571400, "rateLimitType": "five_hour",
            "overageStatus": "rejected", "overageDisabledReason": "out_of_credits" } }));
        assert!(updates.contains(&AgentUpdate::RateLimitStatus {
            warning: None,
            out_of_credits: Some(true),
        }));

        // Threshold crossed → a human warning naming the window + %.
        let updates = t(json!({ "type": "rate_limit_event", "rate_limit_info": {
            "status": "allowed_warning", "resetsAt": 1783314600, "rateLimitType": "five_hour",
            "utilization": 92.0 } }));
        let warned = updates.iter().any(|u| {
            matches!(u,
            AgentUpdate::RateLimitStatus { warning: Some(w), .. }
            if w.contains("5-hour") && w.contains("92%"))
        });
        assert!(warned, "expected a 5-hour warning at 92%");
    }

    #[test]
    fn init_frame_yields_capabilities() {
        let updates = t(json!({ "type": "system", "subtype": "init",
            "model": "claude-opus-4-8",
            "output_style": "default", "apiKeySource": "none",
            "fast_mode_state": { "enabled": true },
            "mcp_servers": { "workspacer": {}, "linear": {} },
            "skills": ["a", "b", "c"],
            "plugins": [],
            "agents": { "reviewer": {} },
            "memory_paths": ["~/CLAUDE.md"] }));
        let caps = updates.iter().find_map(|u| match u {
            AgentUpdate::Capabilities(c) => Some(c.clone()),
            _ => None,
        });
        let caps = caps.expect("init should emit capabilities");
        assert_eq!(caps.fast_mode, Some(true));
        assert_eq!(caps.output_style.as_deref(), Some("default"));
        assert_eq!(caps.mcp_servers, 2);
        assert_eq!(caps.skills, 3);
        assert_eq!(caps.plugins, 0);
        assert_eq!(caps.agents, 1);
        assert_eq!(caps.memory_files, 1);
    }

    #[test]
    fn sent_slash_text_becomes_command_item_only_for_known_commands() {
        let mut acc = UsageAcc::default();
        // Before init reports capabilities: everything is prose.
        assert!(slash_command_send_item("/context", &acc).is_none());

        let caps = Capabilities {
            inventory: Some(ContextInventory {
                slash_commands: vec!["context".into(), "btw".into()],
                ..Default::default()
            }),
            ..Default::default()
        };
        acc.capabilities = Some(caps);

        match slash_command_send_item("/btw is this ready?", &acc) {
            Some(ConversationItem::SlashCommand { name, args, .. }) => {
                assert_eq!(name, "btw");
                assert_eq!(args.as_deref(), Some("is this ready?"));
            }
            other => panic!("expected SlashCommand, got {other:?}"),
        }
        match slash_command_send_item("/context", &acc) {
            Some(ConversationItem::SlashCommand { name, args, .. }) => {
                assert_eq!(name, "context");
                assert!(args.is_none());
            }
            other => panic!("expected SlashCommand, got {other:?}"),
        }
        // Unknown command / plain prose / bare slash stay user messages.
        assert!(slash_command_send_item("/notacommand hi", &acc).is_none());
        assert!(slash_command_send_item("deploy /context please", &acc).is_none());
        assert!(slash_command_send_item("/", &acc).is_none());
    }

    #[test]
    fn init_frame_yields_itemized_inventory() {
        // Field shapes verified against a real CLI 2.1.204 init frame.
        let inv = inventory_from_init(&json!({
            "type": "system", "subtype": "init", "cwd": "/tmp",
            "tools": ["Task", "Bash", "Read"],
            "mcp_servers": [
                { "name": "Neon", "status": "pending" },
                { "name": "posthog", "status": "connected" }
            ],
            "slash_commands": ["verify", "code-review"],
            "agents": ["claude", "Explore"],
            "skills": ["verify", "dataviz"],
            "plugins": [{ "name": "rust-analyzer-lsp",
                          "path": "/home/u/.claude/plugins/cache/rust-analyzer-lsp/1.0.0",
                          "source": "rust-analyzer-lsp@claude-plugins-official" }],
            "memory_paths": { "auto": "/home/u/.claude/projects/-tmp/memory/" },
            "claude_code_version": "2.1.204"
        }));
        assert_eq!(inv.mcp_servers.len(), 2);
        assert_eq!(inv.mcp_servers[0].name, "Neon");
        assert_eq!(inv.mcp_servers[0].status.as_deref(), Some("pending"));
        assert_eq!(inv.tools, vec!["Task", "Bash", "Read"]);
        assert_eq!(inv.slash_commands.len(), 2);
        assert_eq!(inv.agents.len(), 2);
        assert_eq!(inv.skills[1].name, "dataviz");
        assert_eq!(
            inv.plugins[0].source.as_deref(),
            Some("rust-analyzer-lsp@claude-plugins-official")
        );
        assert!(inv.plugins[0].path.as_deref().unwrap().ends_with("1.0.0"));
        assert_eq!(inv.memory_files.len(), 1);
        assert_eq!(inv.memory_files[0].name, "auto");
        assert_eq!(
            inv.memory_files[0].path.as_deref(),
            Some("/home/u/.claude/projects/-tmp/memory/")
        );
        assert_eq!(inv.claude_code_version.as_deref(), Some("2.1.204"));
    }

    #[test]
    fn inventory_tolerates_legacy_map_shapes() {
        // Older frames shaped mcp_servers/agents as name-keyed maps and
        // memory_paths as a plain list.
        let inv = inventory_from_init(&json!({
            "mcp_servers": { "workspacer": {}, "linear": {} },
            "agents": { "reviewer": {} },
            "memory_paths": ["/home/u/CLAUDE.md"]
        }));
        let mut names: Vec<&str> = inv.mcp_servers.iter().map(|i| i.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["linear", "workspacer"]);
        assert_eq!(inv.agents[0].name, "reviewer");
        assert_eq!(inv.memory_files[0].name, "CLAUDE.md");
        assert_eq!(
            inv.memory_files[0].path.as_deref(),
            Some("/home/u/CLAUDE.md")
        );
    }

    #[test]
    fn enrich_expands_memory_dir_and_sizes_files() {
        let dir = std::env::temp_dir().join(format!("wks-inv-test-{}", std::process::id()));
        let sub = dir.join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.join("MEMORY.md"), "x".repeat(400)).unwrap();
        std::fs::write(sub.join("fact.md"), "y".repeat(41)).unwrap();

        let mut caps = Capabilities {
            inventory: Some(ContextInventory {
                memory_files: vec![ContextItem {
                    name: "auto".into(),
                    path: Some(dir.to_string_lossy().into_owned()),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        enrich_inventory(&mut caps, None);
        let files = &caps.inventory.as_ref().unwrap().memory_files;
        assert_eq!(files.len(), 2, "dir should expand to the files inside");
        let index = files.iter().find(|f| f.name == "MEMORY.md").unwrap();
        assert_eq!(index.bytes, Some(400));
        assert_eq!(index.est_tokens, Some(100));
        assert_eq!(index.source.as_deref(), Some("auto"));
        let nested = files.iter().find(|f| f.name.contains("fact.md")).unwrap();
        assert_eq!(nested.est_tokens, Some(11)); // ceil(41/4)
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A throwaway project root that cleans itself up — the suite shares a
    /// quota'd /tmp with every other one, so a leaked mkdtemp is everyone's
    /// problem.
    struct TempProject(std::path::PathBuf);
    impl TempProject {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "wks-skills-{}-{}-{:?}",
                tag,
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        /// Write a file, creating its parents. Path is relative to the root.
        fn write(&self, rel: &str, contents: &str) {
            let full = self.0.join(rel);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, contents).unwrap();
        }
        fn cwd(&self) -> &str {
            self.0.to_str().unwrap()
        }
    }
    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn inventory_caps(inv: ContextInventory) -> Capabilities {
        Capabilities {
            inventory: Some(inv),
            ..Default::default()
        }
    }

    fn named(names: &[&str]) -> Vec<ContextItem> {
        names
            .iter()
            .map(|n| ContextItem {
                name: (*n).to_string(),
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn enrich_resolves_project_skill_and_labels_the_rest_builtin() {
        let p = TempProject::new("proj");
        // The names are deliberately not real skill names: enrichment also walks
        // the developer's own ~/.claude while this runs, and a name like
        // "dataviz" would resolve there.
        p.write(
            ".claude/skills/wks-fixture-skill/SKILL.md",
            "---\nname: wks-fixture-skill\ndescription: Fixture skill for the enrichment test.\n---\n\nbody\n",
        );
        // A plugin-style slash command, which the init frame reports in `skills`
        // too — the leg that resolved nowhere before.
        p.write(
            ".claude/commands/wks-fixture-command.md",
            "---\ndescription: Fixture command.\n---\n\nrun it\n",
        );
        p.write(
            ".claude/agents/wks-fixture-agent.md",
            "---\nname: wks-fixture-agent\ndescription: Fixture agent.\n---\n\nyou are\n",
        );

        let mut caps = inventory_caps(ContextInventory {
            skills: named(&[
                "wks-fixture-skill",
                "wks-fixture-command",
                "wks-fixture-absent",
            ]),
            agents: named(&["wks-fixture-agent", "wks-fixture-absent-agent"]),
            ..Default::default()
        });
        enrich_inventory(&mut caps, Some(p.cwd()));
        let inv = caps.inventory.unwrap();

        let skill = &inv.skills[0];
        assert!(skill.path.as_deref().unwrap().ends_with("SKILL.md"));
        assert_eq!(skill.source.as_deref(), Some("project"));
        assert_eq!(
            skill.description.as_deref(),
            Some("Fixture skill for the enrichment test.")
        );
        assert!(skill.est_tokens.is_some(), "a resolved skill is sized");

        let command = &inv.skills[1];
        assert!(
            command
                .path
                .as_deref()
                .unwrap()
                .ends_with("wks-fixture-command.md"),
            "a skill that is really a slash command still resolves"
        );
        assert_eq!(command.description.as_deref(), Some("Fixture command."));

        // The whole point: "no file" is an ANSWER, not a blank row.
        let builtin = &inv.skills[2];
        assert_eq!(builtin.source.as_deref(), Some("built-in"));
        assert!(builtin.path.is_none());
        assert!(builtin.description.is_none());

        assert_eq!(inv.agents[0].source.as_deref(), Some("project"));
        assert_eq!(inv.agents[0].description.as_deref(), Some("Fixture agent."));
        assert_eq!(inv.agents[1].source.as_deref(), Some("built-in"));
    }

    #[test]
    fn frontmatter_description_reads_both_yaml_shapes() {
        let p = TempProject::new("fm");

        p.write(
            "scalar.md",
            "---\nname: a\ndescription: One line.\n---\nbody",
        );
        assert_eq!(
            frontmatter_description(&p.0.join("scalar.md")).as_deref(),
            Some("One line.")
        );

        // The folded-block shape ~/.claude/skills/omarchy/SKILL.md actually uses.
        p.write(
            "folded.md",
            "---\nname: a\ndescription: >\n  First part of it.\n  Second part of it.\nmetadata: x\n---\nbody",
        );
        assert_eq!(
            frontmatter_description(&p.0.join("folded.md")).as_deref(),
            Some("First part of it. Second part of it."),
            "continuation lines fold into one line, and the next key ends it"
        );

        p.write("quoted.md", "---\ndescription: \"Quoted.\"\n---\nbody");
        assert_eq!(
            frontmatter_description(&p.0.join("quoted.md")).as_deref(),
            Some("Quoted.")
        );

        p.write("none.md", "---\nname: a\n---\nbody");
        assert!(frontmatter_description(&p.0.join("none.md")).is_none());

        p.write("nofm.md", "# Just a heading\ndescription: not frontmatter");
        assert!(
            frontmatter_description(&p.0.join("nofm.md")).is_none(),
            "a body line that looks like a key is not frontmatter"
        );
    }

    #[test]
    fn long_description_is_clamped_on_a_char_boundary() {
        // A multi-byte char straddling the cut is what a byte-slice truncation
        // panics on; skill descriptions are prose and routinely contain them.
        let long = "é".repeat(MAX_DESCRIPTION_CHARS + 50);
        let out = clamp_description(&long).unwrap();
        assert_eq!(out.chars().count(), MAX_DESCRIPTION_CHARS + 1); // + the ellipsis
        assert!(out.ends_with('…'));
        assert!(clamp_description("   ").is_none());
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
                answer_kinds: None,
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
                answer_kinds: None,
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
                answer_kinds: None,
            },
        );
        assert_eq!(out["answers"]["Pick a color"], "Chartreuse");
    }

    #[test]
    fn answered_input_honors_answer_kinds_for_numeric_free_text() {
        // A multi-question set where q1 has ≥3 options, so the free-text answer
        // "3" is ALSO a valid option index. With `answer_kinds = [text, option]`
        // the literal "3" must stay "3" (kind=text, no remap) while q2's "2"
        // still resolves to its label (kind=option).
        let input = json!({ "questions": [
            { "question": "Lucky number?",
              "options": [ { "label": "One" }, { "label": "Two" }, { "label": "Three" } ] },
            { "question": "Pick a color",
              "options": [ { "label": "Red" }, { "label": "Blue" } ] }
        ]});
        let out = answered_input(
            &input,
            &ManagedAnswer {
                option: None,
                text: None,
                answers: Some(vec!["3".into(), "2".into()]),
                answer_kinds: Some(vec!["text".into(), "option".into()]),
            },
        );
        // kind=text → literal, NOT options[2].label ("Three").
        assert_eq!(out["answers"]["Lucky number?"], "3");
        // kind=option → digit→label mapping still applies.
        assert_eq!(out["answers"]["Pick a color"], "Blue");
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
            effort: Some("xhigh".into()),
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
        assert!(joined.contains("--effort xhigh"));
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

    #[test]
    fn parallel_approval_does_not_clobber_a_live_question() {
        // Regression: the assistant emits AskUserQuestion + Bash in one turn.
        // The CLI sends the AskUserQuestion can_use_tool first (session parks a
        // Question), then the Bash can_use_tool. The Bash approval must NOT
        // overwrite the still-open question — otherwise the picker vanishes, the
        // question is never answered, and the CLI turn (hence the session) wedges.
        let store = SessionStore::new();
        let conv = ConversationStore::new();
        let sid = "sid-parallel-q";
        store.register_managed(sid, "/w", "claude");

        let (out_tx, _out_rx) = mpsc::unbounded_channel::<Value>();
        let mut cur_mode = SessionMode::Responding;
        let mut acc = UsageAcc::default();
        let mut totals = StreamTotals::default();
        let yolo = AtomicBool::new(false);
        let mut pending_approvals: VecDeque<ParkedCanUse> = VecDeque::new();
        let mut pending_question: Option<ParkedCanUse> = None;
        let mut pending_controls: HashMap<String, PendingControl> = HashMap::new();
        let mut bg_tasks_active = false;
        let mut idle_suppressed = false;

        // 1) AskUserQuestion arrives → session shows the picker.
        handle_line(
            &json!({ "type": "control_request", "request_id": "cli-q1", "request": {
                "subtype": "can_use_tool", "tool_name": "AskUserQuestion",
                "input": { "questions": [
                    { "question": "Pick a color", "options": [ { "label": "Red" }, { "label": "Blue" } ] }
                ]}
            }}),
            &store,
            &conv,
            sid,
            &out_tx,
            &mut cur_mode,
            &mut acc,
            &mut totals,
            &yolo,
            &mut pending_approvals,
            &mut pending_question,
            &mut pending_controls,
            &mut bg_tasks_active,
            &mut idle_suppressed,
        );
        assert_eq!(
            cur_mode,
            SessionMode::Question,
            "question should be displayed"
        );
        assert!(pending_question.is_some(), "question should be parked");

        // 2) A parallel Bash approval arrives while the question is still open.
        handle_line(
            &json!({ "type": "control_request", "request_id": "cli-b1", "request": {
                "subtype": "can_use_tool", "tool_name": "Bash",
                "input": { "command": "ls" }, "tool_use_id": "tu-b"
            }}),
            &store,
            &conv,
            sid,
            &out_tx,
            &mut cur_mode,
            &mut acc,
            &mut totals,
            &yolo,
            &mut pending_approvals,
            &mut pending_question,
            &mut pending_controls,
            &mut bg_tasks_active,
            &mut idle_suppressed,
        );

        // The question must survive: the approval parks silently behind it and
        // re-surfaces only once the question is answered (the arx branch).
        assert!(pending_question.is_some(), "question must stay parked");
        assert_eq!(
            pending_approvals.len(),
            1,
            "the Bash approval should be parked, not dropped"
        );
        assert_eq!(
            cur_mode,
            SessionMode::Question,
            "a parallel approval must not overwrite the live question"
        );
        let state = store.get(sid).expect("session state");
        assert_eq!(
            state.mode,
            SessionMode::Question,
            "store's displayed mode must stay Question"
        );
        assert!(
            matches!(state.pending, Some(Pending::Question { .. })),
            "store's pending slot must still hold the Question, got {:?}",
            state.pending
        );
    }

    /// The "agent says working when it's not" latch, end to end through
    /// handle_line. A `run_in_background` shell (a dev server, a poll loop)
    /// keeps the CLI's background task set non-empty indefinitely; treating
    /// that as "subagent running" suppressed the Idle of every later
    /// turn-closing `result`, so an idle session claimed Responding for hours
    /// (observed live). Ambient tasks must count on the wire and change
    /// nothing else; only `local_agent` tasks hold busy — and when they drain
    /// with a shell still live (so no trailing `result` ever comes), the
    /// swallowed Idle must be paid immediately.
    #[test]
    fn background_shell_never_latches_responding_and_agent_drain_pays_idle() {
        let store = SessionStore::new();
        let conv = ConversationStore::new();
        let sid = "sid-bg-latch";
        store.register_managed(sid, "/w", "claude");

        let (out_tx, _out_rx) = mpsc::unbounded_channel::<Value>();
        let mut cur_mode = SessionMode::Responding;
        let mut acc = UsageAcc::default();
        let mut totals = StreamTotals::default();
        let yolo = AtomicBool::new(false);
        let mut pending_approvals: VecDeque<ParkedCanUse> = VecDeque::new();
        let mut pending_question: Option<ParkedCanUse> = None;
        let mut pending_controls: HashMap<String, PendingControl> = HashMap::new();
        let mut bg_tasks_active = false;
        let mut idle_suppressed = false;
        let run = |value: &Value,
                   cur_mode: &mut SessionMode,
                   bg: &mut bool,
                   sup: &mut bool,
                   pa: &mut VecDeque<ParkedCanUse>,
                   pq: &mut Option<ParkedCanUse>,
                   pc: &mut HashMap<String, PendingControl>,
                   acc: &mut UsageAcc,
                   totals: &mut StreamTotals| {
            handle_line(
                value, &store, &conv, sid, &out_tx, cur_mode, acc, totals, &yolo, pa, pq, pc, bg,
                sup,
            );
        };

        // Turn 1: the agent launches a background SHELL mid-turn…
        run(
            &json!({ "type": "system", "subtype": "background_tasks_changed",
                "tasks": [{ "task_id": "sh", "task_type": "local_bash" }] }),
            &mut cur_mode,
            &mut bg_tasks_active,
            &mut idle_suppressed,
            &mut pending_approvals,
            &mut pending_question,
            &mut pending_controls,
            &mut acc,
            &mut totals,
        );
        assert!(!bg_tasks_active, "a shell must not hold the session busy");
        assert_eq!(
            store.get(sid).unwrap().background_tasks,
            1,
            "…but it must show on the wire"
        );

        // …and the turn ends while the shell lives on: the session must idle.
        run(
            &json!({ "type": "result", "subtype": "success", "is_error": false }),
            &mut cur_mode,
            &mut bg_tasks_active,
            &mut idle_suppressed,
            &mut pending_approvals,
            &mut pending_question,
            &mut pending_controls,
            &mut acc,
            &mut totals,
        );
        assert_eq!(
            cur_mode,
            SessionMode::Input,
            "turn end with only a background shell live must idle (the latch)"
        );

        // Turn 2: an async SUBAGENT dispatches (shell still live) — busy holds
        // across the dispatch turn's result…
        run(
            &json!({ "type": "system", "subtype": "background_tasks_changed",
                "tasks": [{ "task_id": "sh", "task_type": "local_bash" },
                          { "task_id": "ag", "task_type": "local_agent" }] }),
            &mut cur_mode,
            &mut bg_tasks_active,
            &mut idle_suppressed,
            &mut pending_approvals,
            &mut pending_question,
            &mut pending_controls,
            &mut acc,
            &mut totals,
        );
        assert!(bg_tasks_active, "a live subagent holds the session busy");
        assert_eq!(cur_mode, SessionMode::Responding);
        run(
            &json!({ "type": "result", "subtype": "success", "is_error": false }),
            &mut cur_mode,
            &mut bg_tasks_active,
            &mut idle_suppressed,
            &mut pending_approvals,
            &mut pending_question,
            &mut pending_controls,
            &mut acc,
            &mut totals,
        );
        assert_eq!(
            cur_mode,
            SessionMode::Responding,
            "dispatch-turn result must stay busy mid-subagent"
        );
        assert!(idle_suppressed, "the swallowed Idle must be remembered");

        // …and when the subagent drains but the shell remains (so the CLI never
        // empties the set and no trailing result comes), the idle debt is paid.
        run(
            &json!({ "type": "system", "subtype": "background_tasks_changed",
                "tasks": [{ "task_id": "sh", "task_type": "local_bash" }] }),
            &mut cur_mode,
            &mut bg_tasks_active,
            &mut idle_suppressed,
            &mut pending_approvals,
            &mut pending_question,
            &mut pending_controls,
            &mut acc,
            &mut totals,
        );
        assert_eq!(
            cur_mode,
            SessionMode::Input,
            "subagent drain with a shell still live must pay the suppressed idle"
        );
        assert!(!idle_suppressed);
        assert_eq!(store.get(sid).unwrap().background_tasks, 1);
        assert_eq!(store.get(sid).unwrap().mode, SessionMode::Input);
    }
}
