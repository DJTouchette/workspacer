# Multi-agent providers (Claude Code · Codex · OpenCode · Pi)

Status: **in progress** — phased. This doc is the architecture of record for making
workspacer drive coding agents other than Claude Code.

## Why this is two problems, not one

Workspacer does two things with an agent, and they couple very differently:

1. **Launch** — already provider-agnostic. The spawn path is
   `renderer → IPC (or the hub-bus agents.spawn capability) → claudemon`. The PTY
   path (`pty.rs` just runs `argv[0]`) is now only one transport: Claude's shipped
   default is the managed `stream` transport (headless `claude --print
   --input-format stream-json` via `POST /sessions/spawn-managed`), and
   Codex/OpenCode/Pi are adapter-driven managed spawns; classic PTY spawns remain
   behind `transport: 'pty'` and the hybrid TUI paths. The only
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

   The provider abstraction now exists: `AgentWorkspace.provider`
   (`'claude' | 'codex' | 'opencode' | 'pi'`, undefined ⇒ `'claude'`; `kind` still
   only marks supervisors), a launch registry in `agentProviders.ts`, and
   per-provider adapters in claudemon's `providers/` translating each backend's
   native events into the shared session model. Session IPC channels are still
   named `claude:*` (they serve every provider), joined by
   `provider:listModels` / `provider:checkAll`.

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
- JSON-RPC 2.0 over stdio, newline-delimited; plus `--listen ws://` (the
  transport workspacer drives sessions over, since a live TUI and our RPC client
  can share it). `--listen unix://` is gated in current Codex builds.
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
- Fallback (used whenever the app-server ws path is unavailable, and always on
  Windows): run the native `codex` TUI in a PTY and tail its rollout JSONL
  (`$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl` — `response_item`
  messages/function_calls plus `event_msg` `task_started`/`task_complete`/
  `token_count`) via `providers/codex_rollout.rs`.

### Pi — hybrid (TUI + session-file tail); `--mode rpc` for supervisors
- **Hybrid (default since 2026-07):** the native Pi TUI runs in a PTY pinned to
  our canonical id (`pi --session-id <uuid>` — creates it if missing), and the
  GUI is driven by tailing the session JSONL Pi writes to
  `~/.pi/agent/sessions/--<encoded-cwd>--/<ts>_<uuid>.jsonl` (encoding: strip
  leading separator, `/ \ :` → `-`). Entries are whole `{type:"message"}` units
  (user / assistant w/ text+toolCall blocks+usage / toolResult) plus
  `model_change`; busy/idle is inferred from user messages and the assistant
  `stopReason` ("toolUse" → more coming). GUI prompts are bracketed-pasted into
  the TUI; approvals happen in the Term. Context tokens follow Pi's own formula:
  `usage.totalTokens || input+output+cacheRead+cacheWrite`.
- **RPC mode (supervisors only):** headless `pi --mode rpc`, kept because role
  instructions must be prepended programmatically and dialogs must surface as
  GUI approvals. RPC speaks strict **LF-delimited JSONL over stdio** (split on
  `\n` only — its own warning).
- The `@earendil-works/pi-coding-agent` harness (formerly @mariozechner).
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
- MCP: none — Pi has no MCP client at all (its README: "No MCP." — extensions
  are the extension point), so nothing is registered via `.mcp.json`. Workspacer's
  AskUserQuestion rides a generated per-session `-e` extension that POSTs to the
  daemon's `/mcp/ask/:session_id` endpoint; the full supervisor facade toolset is
  currently unavailable to Pi — a Pi supervisor runs on role instructions alone.

Caveat: these CLIs move fast; pin/verify schemas at build time. Known Codex bug:
`--json` can be silently dropped when MCP servers are active (openai/codex#15451),
which is one reason to prefer `app-server` over `exec --json`. Pi's approval
forwarding depends on a permission extension being loaded (the core doesn't gate);
the `select` reply guesses an "allow"-ish option, so live-verify against the
extension in use.

## Target architecture — two seams

### Seam A — Provider registry (launch side)
Shipped as `agentProviders.ts`, a registry keyed by
`'claude' | 'codex' | 'opencode' | 'pi'`:
```
resolveAgentBinary(provider) -> argv0 path  (user-configured path → PATH search → bare name)
buildAgentArgv(opts)         -> string[]    (Claude delegates to the full buildClaudeArgv;
                                             other CLIs get a minimal launch)
```
Env mapping (`CLAUDE_CONFIG_DIR`) stays in the Claude spawn helper, and
pty-vs-managed is decided by the spawn dispatchers (`claudeSpawn.ts` /
`managedSpawn.ts`) rather than a per-provider `mode` field.

### Seam B — Provider adapters (observe side, in claudemon)
Shipped in `providers/` — not as a trait: each provider module exposes a pure
`translate()` mapping its native events to shared `AgentUpdate`s, applied by one
`apply_updates()` onto the existing
`SessionState` / conversation-delta / `Usage` / `Pending` model, so the hub bus,
renderer, and Fleet Deck observe every provider identically:
```
ClaudeAdapter    — shipped default: stream adapter (claude_stream.rs, headless
                   `claude --print` stream-json w/ bidirectional control
                   protocol); hooks + JSONL transcript + statusLine remain the
                   PTY transport's channels.
OpenCodeAdapter  — spawn `opencode serve`; create/drive session over HTTP;
                   consume `/event` SSE; translate Parts/usage/approvals.
CodexAdapter     — run `codex app-server --listen ws://` (a WebSocket daemon the
                   native TUI can share; stdio JSON-RPC only for model listing);
                   translate item/turn/token notifications + approval requests.
```

### Data-model + plumbing changes
- `provider: 'claude' | 'codex' | 'opencode' | 'pi'` on `AgentWorkspace`,
  `PaneConfig`, and spawn options (undefined ⇒ `'claude'`; existing
  pre-multi-provider data is Claude).
- IPC kept the `claude:*` names (they now serve every provider), joined by
  `provider:listModels` / `provider:checkAll`; no generic `agent:*` namespace.
- Per-provider model lists and config (`config.agents`: `defaultProvider`,
  per-provider binaries). Profiles were not generalized: they remain Claude-only
  (`claude-profiles.json`, `CLAUDE_CONFIG_DIR` + extraArgs) — there is no
  `agent-profiles.json`.
- Provider picker in the spawn dialog; provider brand marks shipped —
  `agentLogos.tsx` carries inline SVG logos for Claude, Codex, OpenCode, and Pi
  (used in the picker's provider cards), alongside the glyph pack's `agent`
  hexagon.

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
- **Phase 5 — Tier-2 Pi adapter.** Shipped as `providers/pi.rs` in two shapes:
  the default is the hybrid (native Pi TUI in a PTY pinned via `--session-id`,
  GUI from the session-JSONL tail); `pi --mode rpc` JSONL stdio is used for
  supervisors, with Extension-UI approval forwarding. The planned `.mcp.json`
  facade wiring was dropped — Pi has no MCP client, so a Pi supervisor gets role
  instructions and the generated AskUserQuestion extension, but no workspacer
  facade tools. Pi joins as a first-class managed provider.
