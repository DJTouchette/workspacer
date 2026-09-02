---
title: Claudemon Multi-Provider Adapters & PTY/Stream Transports
tags: [claudemon, rust, providers, transports, control-protocol, codex, opencode, subagent, copilot]
related_paths:
  - "services/claudemon/src/providers/*.rs"
  - "services/claudemon/src/session/state.rs"
  - "services/claudemon/src/session/store.rs"
  - "services/claudemon/src/session/pricing.rs"
  - "services/claudemon/src/daemon/spawn.rs"
owner: Damien Touchette
last_reviewed: 2026-09-01
---

# Claudemon Multi-Provider Adapters & PTY/Stream Transports

## Overview
Each `services/claudemon/src/providers/*.rs` adapter drives one agent's native machine interface (Claude's own stream-json control protocol, Codex `app-server` JSON-RPC over WebSocket, Codex rollout-JSONL tailing, OpenCode `serve` HTTP+SSE, Pi `--mode rpc` JSONL-over-stdio) and translates its events into claudemon's provider-agnostic session model: `SessionMode`/`Pending` (`services/claudemon/src/session/state.rs`), the conversation delta stream, and `StatusLine`. This is what lets the hub bus, renderer, and Fleet Deck treat every provider identically to a Claude session. Translation is deliberately split into a pure per-provider layer and one shared apply layer.

## Key modules
- `services/claudemon/src/providers/mod.rs` — the shared vocabulary: `AgentUpdate` enum, `apply_updates` (shared mutation layer), `UsageAcc` (running usage/cost/rate-limit tally → `StatusLine`), `Facade` struct (MCP URL + role instructions), `ModelInfo`/model-list cache (`cached_or_fetch`, 10-min TTL, stale-fallback), `context_window_for` (model→window table), `spawn_attach_pty` (shared hybrid GUI+Term PTY wiring).
- `services/claudemon/src/providers/claude_stream.rs` — Claude's headless `--input-format stream-json` transport; owns the full bidirectional control protocol (`can_use_tool` approvals + `AskUserQuestion`, `interrupt`, `set_permission_mode`, `set_model`); only adapter registering `register_managed_answer`/`register_managed_permission_mode`.
- `services/claudemon/src/providers/codex.rs` — Codex `app-server` over `ws://127.0.0.1:<port>` (TUI-first hybrid: the PTY-run `codex --remote` TUI owns the thread, RPC client rejoins via `thread/loaded/list` + `thread/resume`); has `list_models`, live model switch (`thread/settings/update`), structural interrupt.
- `services/claudemon/src/providers/codex_rollout.rs` — a *third*, read-only Codex path: tails `$CODEX_HOME/sessions/.../rollout-*.jsonl` for the Windows case (no app-server daemon there) or as the resume seed source; owns the `~/.workspacer` codex-threads sidecar (`record_thread`/`thread_for`/`rollout_for_thread`); registers **no** control channels at all (no answer/interrupt/model-switch/permission-mode).
- `services/claudemon/src/providers/opencode.rs` — `opencode serve` + `GET /event` SSE, hybrid with `opencode attach` TUI in a PTY via `spawn_attach_pty`; live model switch is per-message (stamped each turn, no session-wide RPC); native `POST /session/:id/abort` interrupt.
- `services/claudemon/src/providers/pi.rs` — `pi --mode rpc` JSONL over stdio; approvals only exist when a permission extension is loaded (`extension_ui_request`); **no MCP support in Pi core** — AskUserQuestion/other MCP-shaped features must ride Pi's extension API instead.
- `services/claudemon/src/session/state.rs` — `SessionMode`, `Pending` (`Approval`/`Question`), `Transport` (`Pty`/`Stream`, struct-level `#[default]` is `Pty`), `Capabilities`, `Plan`/`PlanStep`, `SessionState.live_subagents`/`parent_turn_ended` (background-subagent async task bookkeeping, `#[serde(skip)]`, mirrors claude_stream's `bg_tasks_active`).
- `services/claudemon/src/session/store.rs` — per-session `DashMap`-backed channel registries (`managed_answers`, `managed_permission_modes`, `managed_interrupts`, `managed_decisions`, `managed_model`) that `apply_updates` and the `/sessions/:id/{answer,permission-mode,model,signal}` HTTP handlers route through; `set_managed_model`/`interrupt_managed`/`submit_managed_answer` all fail closed (`false`/`Err`) when a session has no registered channel, so callers can fall back to PTY/restart paths.
- `services/claudemon/src/session/pricing.rs` — the twin cost table `UsageAcc::status_line` calls via `estimate_cost` when a provider (Codex, Codex-rollout) has no native dollar figure.
- `services/claudemon/src/daemon/spawn.rs` — `SpawnManagedPayload`/`handle_managed`: the single entrypoint (`POST /sessions/spawn-managed`) that dispatches `provider` ∈ `{opencode, codex, pi, claude}`, stamps `Transport::Stream` before the driver starts for `claude` and for Codex when `transport == "stream"` or a resume thread is found, and resolves the Codex resume thread via `codex_rollout::thread_for`.

## Failure modes
- Model-list queries shell out live (`codex app-server`, `opencode models`, a Pi RPC) and are version-fragile; `cached_or_fetch` serves a stale last-known-good list on failure rather than an empty picker, and only errors when nothing has ever been cached for that `"<provider>:<bin>"` key.
- `apply_updates` debounces mode changes (skips a re-apply if the mode is unchanged) except `Approval`, which always re-applies since its `pending` payload can change turn to turn — a bug here would either flood the store or silently drop a new approval's detail.
- `AgentUpdate::Error` is surfaced as a marked `AssistantText` conversation item (`⚠️ Error: …`), not a bespoke item kind — the renderer only knows fixed item kinds, so a real bespoke error type would silently vanish.
- Every `register_managed_*`/`submit_managed_*`/`set_managed_*` pair in `store.rs` fails closed on a missing channel (returns `false`/`Err`), which callers (e.g. `POST /sessions/:id/signal` SIGINT handling) use to decide whether to fall back to a PTY keystroke or a restart — a provider path that forgets to register a channel silently downgrades to the fallback instead of erroring loudly.
- `codex_rollout.rs`'s tailer is best-effort file discovery (newest rollout whose `session_meta.cwd` matches, created right after spawn) — a race or a mismatched cwd means the GUI conversation view simply never lights up for that hybrid session.

## Gotchas
- **Two layers, both required per provider.** The FOCUS split is real and enforced by naming, not the type system: `translate`/`plan_from_value`/`rate_limits_from` are pure and unit-tested; `apply_updates` (shared, in `mod.rs`) is the only place that mutates `SessionStore`/`ConversationStore`. Adding a provider means writing both, plus `list_models` (live-queried, cached) and a `Facade` (MCP URL + instructions) — several adapters wire `facade.mcp_url` into the CLI's own `mcp_servers.workspacer.url` config override (see `codex.rs:674`).
- **Control-protocol wiring is uneven by design, not oversight.** Only `claude_stream.rs` registers `managed_answer` (structured `AskUserQuestion`) and `managed_permission_mode` (Claude's own mode vocabulary) — Codex/OpenCode/Pi approvals are plain yes/no via `register_managed_decision`. All four adapters register `managed_model` and `managed_interrupt`, **except** the Codex rollout-tailer path (`codex_rollout.rs`), which registers none — it is read-only.
- **A provider can have more than one live session shape.** Pi's default hybrid TUI session (`run_tui_session`) registers *no* RPC channels and 409s on live model-switch/interrupt (falls back to restart — the "capability cliff" `providerCaps.pi` documents on the desktop side); only Pi's RPC-driven path registers them. Codex similarly has the WS-RPC hybrid (`codex.rs`, full control) vs. the rollout tailer (`codex_rollout.rs`, zero control) — same provider name, very different capability surface depending on platform/session shape.
- **Codex never emits `AgentUpdate::Capabilities`** (confirmed: no occurrence in `codex.rs` or `codex_rollout.rs`) and **never carries native cost** — both `codex.rs:970` and `codex_rollout.rs:555` call `acc.estimate_costs()`, so Codex cost is always the `session/pricing.rs` table estimate, never a wire figure. Any change to that table must stay consistent with the twin table documented in the usage-accounting memory.
- **`Transport` struct default is `Pty`**, not `Stream` — the desktop-level default-to-stream behavior (config-two-writers memory) lives in spawn payload construction / config, not in this struct's serde default; don't assume `SessionState.transport` defaulting means the wire behavior changed.
- Codex resume is genuinely implemented (via the `~/.workspacer` codex-threads sidecar `record_thread`/`thread_for` + app-server `thread/resume`), and forces the stream transport (`spawn.rs`) since the TUI can't rejoin an arbitrary thread — resume is not purely a "gap" as older parity notes suggested; verify against `codex-parity-status` before trusting it as still-open.
- Rate-limit merging is intentionally field-wise, not event-wise (`UsageAcc::merge_rate_limits`), because Claude's `rate_limit_event` can carry `resetsAt` without `utilization` — a naive whole-struct overwrite would drop a reset time. `monthly_pct`/`monthly_resets_at` is a distinct bucket (Claude's `overage`) that Codex never populates (`rate_limits_from` hard-codes `monthly_pct: None`).

## Hand-authored notes (2026-08-26/28) — Codex subagents, Copilot, and the shared error seam

### Codex: plans and subagents on the app-server wire

- **Plans arrive on `turn/plan/updated`.** The installed Codex 0.150.1 app-server
  schema exposes an authoritative `turn/plan/updated` notification whose
  `plan[].status` values include camelCase `inProgress`. The adapter historically
  extracted plan-shaped items only from `item/started`/`item/completed`, and the
  shared `PlanStatus::from_wire` did not recognise `inProgress` — so a live Codex
  plan could be ignored entirely, or its active step render as pending. Handle
  `turn/plan/updated`, widen `from_wire` for the camelCase spelling, and do NOT
  derive full plans from the experimental `item/plan/delta` chunks.
- **ONE subagent arrives through THREE unrelated wire shapes, and the CHILD
  THREAD ID is the only join key** (`SessionStore::apply_subagent_update` upserts
  purely by `id`, so a second spelling grows a duplicate row):
  1. `thread/started` → `subagent_from_thread`, which reads `thread.id` **only
     after `parentThreadId` is present**. That guard is what stops every
     top-level thread registering itself as its own subagent — remove it and
     each session gains a phantom child.
  2. `item/started|completed` type `subAgentActivity` → **`agentThreadId`**, not
     the item `id`. `kind` is the status: `started`/`interacted` mean Running,
     anything else Complete.
  3. `item/started|completed` type `collabAgentToolCall` → the KEYS of the
     `agentsStates` object, falling back to `receiverThreadIds[]` only when that
     map is absent or empty. `senderThreadId` is the PARENT and is deliberately
     never used as a row id.
  **The dispatch inside `translate_item` is asymmetric on purpose:**
  `subAgentActivity` `return`s after emitting the Subagent update (activity rows
  are not tool cards), while `collabAgentToolCall` deliberately FALLS THROUGH so
  the generic path also emits a `ToolUse`/`ToolResult` pair — a `spawnAgent`
  renders as the familiar "Agent" tool card AND a subagent row from one wire
  item. Adding a `return` after the collab branch (the obvious symmetry "fix")
  silently deletes the Agent tool card;
  `collab_spawn_agent_yields_tool_card_and_subagent_row` asserts exactly two
  updates. The `tool_use_id` link back to the card is set only when
  `tool == "spawnAgent"` — that is what lets the UI tie a row to its card.
- **Child drill-in can replay the rollout by thread id, no live RPC needed.**
  `codex_rollout::rollout_for_thread` locates the durable rollout file directly,
  so a read-only drill-in works through the existing `ConversationItem`
  parser/applier without opening a broader app-server control-channel surface.
  Scope access through the parent session's known subagents: verify the child id
  appears on the parent `SessionState.subagents` before serving it. The remaining
  web/remote gap is transport exposure, not parsing — see
  `domains/renderer-backend-seam.md` for the four-place `runId === null` routing.

### GitHub Copilot (`copilot -p`), verified live against v1.0.81 on 2026-08-28

- **`copilot --session-id <uuid>` both CREATES and RESUMES.** Verified twice
  directly and again end-to-end through claudemon: the same `--session-id` passed
  to two separate `copilot -p` processes resumes the first one's conversation (a
  codeword set in turn 1 was recalled in turn 2). `--resume <uuid>` works too but
  is unnecessary. **This is what makes a one-shot `-p` adapter a real
  conversation** — it removes the entire `codex-threads` sidecar class of work,
  makes `restartPreservesConversation: true` honest for copilot (the only managed
  provider that can claim it), makes `--model`/`--effort` genuinely live (the next
  turn is a new argv), and means the brain needs NO wire `resume` field for
  copilot. Keep the driver parked on the input channel between turns and spawn
  one process per message.
- **Wire shape notes:** the terminal `result` event is TOP-LEVEL, not under
  `data` (`{"type":"result","sessionId":…,"exitCode":…,"usage":{…}}`) and its
  `usage` is session-CUMULATIVE, while `model.model_call_success`'s
  `responseChunk.usage` is per-model-call (hence `UsageAcc::additive`).
  `model.turn_started` carries
  `modelInfo.capabilities.limits.max_context_window_tokens` = **Copilot's OWN
  window for the model** (144000 for claude-haiku-4.5, not Anthropic's 200k).
- **Never key "turn over" on process exit alone.** A hard Copilot failure can
  print prose to stderr while exiting 0 with clean JSONL and no error event, so
  `turn_outcome()` requires empty stderr AND a `result` frame AND exitCode 0 AND
  a zero process exit AND some output before emitting a bare Idle.
- **The `--model` catalog is ACCOUNT-gated, so there is nothing to enumerate.**
  Three independent routes are dead ends: no `copilot models` subcommand and the
  generated shell completions carry no values for `--model`; the ACP handshake
  (`--acp` → `initialize` + `session/new`) returns session modes and
  `configOptions` but no model list and no model config option; and
  `api.github.com/copilot_internal/v2/token` answers 403 to a `gh` OAuth token.
  Worse, on the probe account EVERY explicit `--model <id>` was refused —
  including `gpt-5-mini` and `claude-haiku-4.5`, the two ids Copilot's own
  auto-router had just chosen for that same account. The captured `modelInfo`
  says why: `"model_picker_enabled": false`. **A curated table taken from the
  GitHub changelog would have shipped a picker where every entry fails at
  launch.** `copilot::list_models()` returns `auto` only, gated on a live
  `bin --version` liveness probe so a missing CLI errors instead of returning a
  plausible list. Free-text model entry still works for accounts whose plan
  enables the picker. **Do NOT add a hardcoded id table.** The failure is at
  least loud: a rejected model exits 1 with that message on stderr and emits no
  `result`, which `turn_outcome()` surfaces as `AgentUpdate::Error`.
- **`-p` mode has NO approval gate, and the help text says the opposite.**
  `copilot --help` claims `--allow-all-tools` is "required for non-interactive
  mode". It is not: a `copilot -p` run with no allow flags happily ran `bash`.
  What the allow flags actually change is **PATH/URL CONFINEMENT** — with none, a
  write outside the session cwd comes back
  `tool.execution_complete {success:false, error:{code:"denied"}}`;
  `--allow-all-tools`/`--allow-all` lift it. There is no approval event in the
  `-p` JSONL stream at all and the CLI states outright it has no channel to ask.
  So the two tiers that exist are cwd-confined and unconfined — a copilot
  session's permission pill reading "Ask to approve" would be lying. Keep the
  ask/yolo IDS (the whole bypass chain — bus clamp, brain `launchPermissionMode`,
  MCP facade — speaks them) but keep copilot's own LABELS ("Workspace only" /
  "Full access") in `apps/desktop/src/renderer/src/lib/providerCaps.ts`, and keep
  the `explainUnsupportedManagedOptions` line that announces it at spawn. A real
  approval gate on Copilot requires the `--acp` long-lived path, not `-p`.
  (Separately: `--deny-tool=bash` did NOT block bash, so the granular deny syntax
  is not the plain tool name.)

### Plan and subagent carriers differ per provider — check persisted state before concluding "no signal"

Verified 2026-08-28 against the installed CLIs.

- **COPILOT (v1.0.81): the plan is NOT on the wire.** `session.todos_changed` has
  an empty `data` on every occurrence; the todo list lives in a `todos` table in
  `~/.copilot/session-state/<session-id>/session.db`, which the model mutates
  through the ordinary `sql` tool (confirmed live — the conversation shows `sql`
  INSERT/UPDATE against `todos`, and the table carries the CLI's own CHECK
  constraint `status IN ('pending','in_progress','done','blocked')`). Subagents
  ARE on the wire: `subagent.started`/`configured`/`completed` carry a
  **TOP-LEVEL `agentId`** (not inside `data`), plus a `system.notification` with
  `kind.type == "agent_idle"` as a second close signal. **A subagent's OWN frames
  — its `user.message` dispatch prompt, its tool calls, its whole report — also
  ride the parent's stdout tagged with that same top-level `agentId`**; folding
  them in renders the child's report as the parent's answer.
- **OPENCODE (v1.18.25): `GET /event` is a SERVER-wide stream, not a session
  one**, and a subagent is a whole child SESSION on the same server. Events must
  be filtered by session id at the driver level or a child's traffic becomes the
  parent's conversation — opencode's own headless client does exactly this
  filter. **This is a live conversation-correctness bug class, not just a missing
  feature.** Field placement differs per event: `properties.sessionID`
  (session.*), `properties.info.sessionID` (message.updated),
  `properties.part.sessionID` (message.part.updated). Plan carrier = the
  `todowrite` tool (`{todos:[{content,status}]}`); subagent dispatch = the `task`
  tool (`{subagent_type, description}`). Tool part states are
  pending/running/completed/error. **`properties.id` on a non-session event is
  NOT a session id** (on `permission.updated` it is the permission id).
- **PI (v0.84.3):** the built-in tool set is exactly bash, edit, find, grep, ls,
  powershell, read, write (`dist/core/tools/`). No todo tool, no task tool — no
  plan and no subagents are possible out of the box.

Capability flags live in `apps/desktop/src/renderer/src/lib/providerCaps.ts`
under `delegation`.

### The conversation store COALESCES consecutive `assistant_text`, so an error runs into the next reply

Observed live on a copilot session whose facade could not attach: the pane
rendered `⚠️ Error: …no structured questions).OK` as ONE message. `apply_updates`
turns `AgentUpdate::Error` into an ordinary `ConversationItem::AssistantText`
with `format!("⚠️ Error: {msg}")` and no trailing separator, and the store merges
consecutive assistant_text items — so a mid-turn error and the turn's real reply
arrive glued together. **This affects EVERY managed provider** (codex/opencode/pi
too); copilot was simply the easiest trigger because the facade check fires
before the first token. Beyond the display bug,
`apps/desktop/src/main/shared/workerFailure.ts`'s `errorMarkerReason()` — which
the Fleet Manager's worker-finished wake reads — takes the marker line and would
have carried the glued-on reply into the failure reason.
Fixed 2026-08-28: `providers/mod.rs` now emits `format!("⚠️ Error: {msg}\n")`.
`errorMarkerReason` already takes only the first line, so both readers agree, and
`contracts/agent-error-marker-cases.json` gained a case pinning the coalesced
shape. **Don't drop the newline.**

### Hand-authored notes (2026-09-01) — Codex context windows: five numbers that are not the same number

Promoted from the 2026-08-31 Codex context learnings, re-checked against master
at `0bac5799`. Measured on a real rollout (Workspacer session `d9723c3c` →
`01a055de` via the `~/.workspacer/codex-threads` sidecar), 539
`token_count` events.

- **Keep these five apart. Conflating any two produces a confident wrong
  answer:**
  1. the **requested** window (`requestedSelection.contextWindow` — user
     intent, may never be honored),
  2. the **canonical/catalog** window (the shared model→window table and
     `codex debug models`' `context_window`/`max_context_window`),
  3. the **runtime** window the harness reports
     (`statusLine.contextWindowSize`, from `token_count`'s
     `model_context_window`),
  4. **cumulative** tokens billed for that rollout, and
  5. the **compaction threshold**.
  On that specimen `model_context_window` was 258,400 on every one of the 539
  events, the largest single input was 227,231, and five compactions happened
  around 217K–227K — while cumulative usage reached 75,586,943 and matched
  `~/.codex/state_5.sqlite`'s `threads.tokens_used`. So a >250K session token
  count is NOT evidence that the live context exceeded 258.4K. Keep the active
  context bar on the latest input against the provider-reported runtime window,
  and label cumulative totals as billed.
- **Never turn a catalog capacity into a picker option on its own.** A model's
  nominal 1.05M capacity displayed as an honored allocation, while the harness
  reports a much smaller active window, is the failure this rule prevents.
  Display the live reported operational window separately from official
  capacity.
- **CORRECTED 2026-09-01 — Codex DOES have a spawn-time context control, and it
  is now wired.** The earlier note that Workspacer "persists a generic
  contextWindow but does not pass it into `codex::spawn_session`" is stale:
  `set_context_window_in_argv` (`services/claudemon/src/daemon/spawn.rs`) and
  `codex.rs` now emit `-c model_context_window=<n>` (de-duplicating any existing
  `-c model_context_window=` first), a fresh model-less Codex life requests 1M
  by contract (`providerContextDefaults`), and `SpawnAgentDialog` offers
  "Request 1M". It remains a SPAWN-TIME request only: there is still no live
  Codex context switch, the request is provisional until a runtime frame
  confirms it, and a validated value should be checked against the selected
  CLI's catalog rather than assumed.
