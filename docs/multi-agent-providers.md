# Multi-agent providers (Claude Code · Codex · OpenCode · Pi)

Status: **in progress** — phased. This doc is the architecture of record for making
workspacer drive coding agents other than Claude Code.

## Why this is two problems, not one

Workspacer does two things with an agent, and they couple very differently:

1. **Launch** — already provider-agnostic. The spawn path is
   `renderer → IPC → claudemon → PTY`, and `pty.rs` just runs `argv[0]`. The only
   Claude-specific bits are binary discovery (`claudeResolver.ts`), the flag
   builder (`buildClaudeArgv`, i.e. Claude's `--session-id/--model/--resume`),
   and the profile→env mapping (`CLAUDE_CONFIG_DIR`).

2. **Observe** — hardcoded to Claude. The "telemetry face" (Fleet Deck cards,
   ambient state, live conversation, token/cost/context, approval & question
   prompts) is reconstructed in `claudemon` from **three Claude-only channels**:
   - **hooks** — we configure Claude's `settings.json` to POST lifecycle events
     to claudemon (`SessionStart/PreToolUse/PostToolUse/Stop/…`), which drive the
     `SessionMode` state machine (`session/state.rs`).
   - **JSONL transcript** — claudemon tails `~/.claude` for conversation + usage.
   - **statusLine** — Claude pipes context %/cost/rate-limits to claudemon.

   There is no provider abstraction anywhere: `AgentWorkspace.kind` only knows
   `'supervisor'`, every IPC channel is `claude:*`, and the parser is Claude-only.

## The opportunity: drive the agents' own protocols

Unlike Claude (which we observe indirectly via injected hooks), Codex and OpenCode
expose first-class machine interfaces we can drive directly — so the rich
integration is *cleaner*, not hackier:

### OpenCode — `opencode serve`
- Headless HTTP server, default `127.0.0.1:4096`, OpenAPI 3.1 (+ generated SDK).
- Sessions: `POST /session`, `GET /session`, `GET /session/:id`, `DELETE /session/:id`.
- Drive: `POST /session/:id/message` (sync) / `POST /session/:id/prompt_async` (204).
- Observe: **SSE `GET /event`** — first frame `server.connected`, then a typed bus
  stream (80+ event types); messages are typed Parts (Text / Tool / Reasoning).
- Health: `GET /global/health`.

### Codex — `codex app-server`
- JSON-RPC 2.0 over stdio (also `--listen ws://` / unix socket), newline-delimited.
- Lifecycle: `thread/start` (model, cwd) → `turn/start` (threadId, input);
  `turn/steer`, `turn/interrupt`, `thread/resume`, `thread/fork`, `model/list`.
- Notifications: `thread/started`, `turn/started`, `turn/completed`,
  `item/started`, `item/completed`, deltas (`item/agentMessage/delta`,
  `item/reasoning/summaryTextDelta`, `item/commandExecution/outputDelta`),
  specialized items (`userMessage|agentMessage|reasoning|commandExecution|
  fileChange|mcpToolCall|webSearch|contextCompaction`).
- Usage: `thread/tokenUsage/updated`.
- Approvals: `item/commandExecution/requestApproval`,
  `item/fileChange/requestApproval` → client replies accept/decline/cancel;
  `serverRequest/resolved`.
- Fallback for batch: `codex exec --json` (JSONL: `thread.started`,
  `turn.started`, `turn.completed` w/ `usage{input_tokens, cached_input_tokens,
  output_tokens, reasoning_output_tokens}`, `turn.failed`, `item.*`, `error`;
  resume via `codex exec resume`).

### Pi — `pi --mode rpc`
- The `@mariozechner/pi-coding-agent` harness. RPC mode speaks strict
  **LF-delimited JSONL over stdio** (split on `\n` only — its own warning).
- Drive: `{"type":"prompt","message":...}` per user turn (`steer`/`follow_up`/
  `abort` also available); model via `--model` flag or `set_model`.
- Observe: `agent_start`/`agent_end` + `turn_start`/`turn_end` (lifecycle),
  `message_update` carrying an `assistantMessageEvent` (`text_delta` etc.),
  `message_end`/`turn_end` (message object carries `usage`),
  `tool_execution_start`/`_end`.
- Approvals: Pi's core auto-runs tools; a permission *extension* prompts via the
  bidirectional **Extension UI** protocol (`extension_ui_request` with a
  `confirm`/`select` dialog → client replies `extension_ui_response`
  `{confirmed}` / `{value}` / `{cancelled}`). YOLO accepts inline; otherwise the
  request is surfaced and the user's /approve decision is forwarded.
- MCP: reads the standard `.mcp.json` (`mcpServers`), so the workspacer facade
  is registered as a remote `{type:"http", url}` entry.

Caveat: these CLIs move fast; pin/verify schemas at build time. Known Codex bug:
`--json` can be silently dropped when MCP servers are active (openai/codex#15451),
which is one reason to prefer `app-server` over `exec --json`. Pi's approval
forwarding depends on a permission extension being loaded (the core doesn't gate);
the `select` reply guesses an "allow"-ish option, so live-verify against the
extension in use.

## Target architecture — two seams

### Seam A — Provider registry (launch side)
A `Provider` descriptor keyed by `'claude' | 'codex' | 'opencode'`:
```
resolveBinary(profile) -> argv0 path     (per-CLI discovery; replaces claudeResolver)
buildArgv(opts)        -> string[]        (per-CLI flags)
buildEnv(profile)      -> env             (CLAUDE_CONFIG_DIR vs CODEX_HOME vs OPENCODE_*)
profileSchema                              (what a "profile" means per provider)
mode: 'pty' | 'managed'                    (Tier-1 terminal vs Tier-2 adapter)
```
`ClaudeProvider` = today's behavior. Add `CodexProvider`, `OpenCodeProvider`.

### Seam B — AgentAdapter (observe side, in claudemon)
A trait that maps a provider's native I/O → the existing
`SessionState` / `ConversationDelta` / `Usage` / `Pending` model, so the hub bus,
renderer, and Fleet Deck stay unchanged:
```
ClaudeAdapter    — hooks + JSONL transcript + statusLine   (current behavior)
OpenCodeAdapter  — spawn `opencode serve`; create/drive session over HTTP;
                   consume `/event` SSE; translate Parts/usage/approvals.
CodexAdapter     — spawn `codex app-server`; JSON-RPC over stdio;
                   translate item/turn/token notifications + approval requests.
```

### Data-model + plumbing changes
- `provider: 'claude' | 'codex' | 'opencode'` on `AgentWorkspace`, `PaneConfig`,
  spawn options, and profiles (default `'claude'`; existing data is Claude).
- Generic `agent:*` IPC alongside (or replacing) `claude:*`.
- Per-provider model lists, profiles (`agent-profiles.json` w/ a `provider` tag),
  and config (`config.agents`).
- Provider picker in the spawn dialog; provider glyphs (the pack already ships an
  `agent` hexagon; add Codex/OpenCode marks).

## Phasing

- **Phase 0 — Foundation.** Add the `provider` field through the data model + spawn
  options (default `'claude'`, no behavior change). Compiles green.
- **Phase 1 — Tier-1 (PTY).** Provider registry: binary resolver + argv builder for
  codex/opencode; spawn-dialog provider picker; non-Claude agents render in the
  terminal view. Spawn a real Codex/OpenCode agent (no telemetry yet).
- **Phase 2 — Tier-2 OpenCode adapter.** `OpenCodeAdapter` in claudemon (serve +
  HTTP + `/event` SSE → SessionState/Conversation/Usage/Pending). Fleet Deck lights up.
- **Phase 3 — Tier-2 Codex adapter.** `CodexAdapter` (app-server JSON-RPC → same model),
  including approval/permission request bridging.
- **Phase 4 — Polish.** Per-provider models/profiles/config, branding
  generalization (model-prefix stripping, labels), provider glyphs, approvals UI.
- **Phase 5 — Tier-2 Pi adapter.** `PiAdapter` (`pi --mode rpc` JSONL stdio →
  same model), with Extension-UI approval forwarding and `.mcp.json` facade
  wiring. Pi joins as a first-class managed provider (`providers/pi.rs`).
