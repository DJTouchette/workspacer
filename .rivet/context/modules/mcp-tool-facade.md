---
title: MCP Tool Facade (hub capabilities exposed as tiered MCP tools)
tags: [hub, go, mcp, model-context-protocol, agent-spawn, control-plane, security, tool-tiers, view-triage-operator, plugin-tools]
related_paths:
  - "services/hub/cmd/mcp/main.go"
  - "services/hub/cmd/mcp/help.go"
  - "services/hub/cmd/mcp/plugins.go"
  - "services/hub/cmd/mcp/ui.go"
  - "services/hub/cmd/mcp/main_test.go"
  - "services/hub/cmd/mcp/auth_test.go"
  - "services/hub/cmd/mcp/tiers_test.go"
  - "services/hub/cmd/mcp/plugins_test.go"
  - "services/hub/cmd/mcp/ui_test.go"
  - "services/hub/internal/busclient/client.go"
  - "services/hub/internal/authtoken/authtoken.go"
  - "apps/desktop/src/main/services/mcpFacadeDaemon.ts"
  - "apps/desktop/src/main/services/mcpConfig.ts"
  - "apps/desktop/src/main/services/remoteTokens.ts"
owner: Damien Touchette
last_reviewed: 2026-08-16
---

# MCP Tool Facade (hub capabilities exposed as tiered MCP tools)

## Overview
`services/hub/cmd/mcp/main.go` is a standalone Go binary (`mcp`) that exposes the hub bus's capabilities — list/spawn/drive agents and terminals, filesystem, config, profiles, saved sessions, layouts, library, analytics, notifications, UI navigation — as MCP tools over HTTP (`/mcp`, `/sse`), so Claude Code or any MCP client can drive workspacer via `--mcp-config`. It is a thin, stateless adapter: every tool call is JSON-marshalled and forwarded to the hub bus as a capability `call` (via `busclient.Client`), executed by the desktop main process (`apps/desktop/src/main/services/hubCapabilities.ts`) — the facade never touches workspacer state directly. Launched and supervised by `apps/desktop/src/main/services/mcpFacadeDaemon.ts` on port 7897, distinct from the hub bus (`hub-bus-control-plane`) and the plugin sidecar surface (`hub-plugin-system`).

Since 2026-08-15 the facade is **tiered, not all-or-nothing**: a spawn picks `toolScope: view | triage | operator` and each session presents a per-session bearer token, so the facade serves only that tier's tools (a view worker sees ~read-only tools + one-line descriptions, not 43 schemas / ~10k tokens of context tax and full operator power). Tiers are *derived* from the `authtoken` scope allowlists, so the facade and the bus can never disagree about what "view" means. Plugins can contribute opt-in tools, and triage+ sessions can steer the desktop UI via `command.*` events.

## Key modules
- `services/hub/cmd/mcp/main.go` — flag parsing (`-addr`, `-hub`, `-token`, `-mcp-token`), `checkBindPolicy`/`isLoopbackAddr` (fail-closed bind guard), `authGate` + per-request token resolution (see Auth below), `newServer` (the per-tier tool registries), `forward` (capability-call → MCP result renderer), `addTool[In]` / `addObjectTool` registrars, and every `...In` struct defining a tool's JSON input schema (`json` tags must match the hub capability's expected params; `jsonschema` tags are the per-field descriptions).
- `services/hub/cmd/mcp/help.go` — the per-tier `help` tool, rendered from the same registry the tools register into, with guidance behind topics. This fixed the old prompt-drift hazard **by construction**: tool descriptions are deliberately one line; `SUPERVISOR_SYSTEM_PROMPT` (mcpConfig.ts) shrank to role + "call help first" and can no longer drift from the actual surface.
- `services/hub/cmd/mcp/plugins.go` — `pluginCatalog`: polls the hub-local `plugins.tools` method every 15s (`catalogPollInterval`) for the consented plugin-tool surface, and `serverCache` builds/caches per-token servers keyed by (scope, plugins set, catalog generation). Plugin tools are named `<sanitized_full_plugin_id>_<name>` and grafted only onto tokens whose record opted in (`plugins` field; `"*"` = all). Pinned by `plugins_test.go`.
- `services/hub/cmd/mcp/ui.go` — UI-navigation tools `focus_agent`, `open_pane`, `open_browser`, `open_plugin`, `open_spawn_dialog`: they **publish** the renderer's existing `command.*` events (busclient gained `Publish`) rather than calling a capability. Event-backed, so their tier cannot be derived from method allowlists — `tierRank` enforces an explicit triage+ gate, pinned by `TestUiToolTierGate` (`ui_test.go`). `command.open_pane` carries an optional `url` for browser panes (`useUiCommands` → `App.tsx` `addTabWithConfig`). Fire-and-forget: "ok" means published, not acted on; a headless hub has no consumer. The `help` topic `ui` steers agents to prefer `notify` outside user-requested tours.
- `services/hub/internal/authtoken/authtoken.go` — the tier source of truth: each tier's server registers only tools whose hub method the scope allowlist admits (`event.MatchesAny(scope.Methods(), method)`) — the same patterns the bus enforces on scoped connections. `tiers_test.go` pins the nesting (view ⊂ triage ⊂ operator) and the exclusions.
- `apps/desktop/src/main/services/remoteTokens.ts` — desktop-side session-token lifecycle: `mintSessionFacadeToken` (label `session:<id>`, stored in the same `tokens.json` as pairing tokens but hidden from the pairing UI), revoked at session-store eviction, `sweepSessionFacadeTokens` at boot against the daemon's live session list.
- `apps/desktop/src/main/services/mcpConfig.ts` — writes the `--mcp-config` files: `supervisorMcpConfigPath()` and per-session `session-mcp/<sessionId>.json` (0600 — carries the live bearer as an `Authorization` header; the file path rides argv, never the token). Codex/opencode read URL-only MCP configs and can't send headers, so they get the token as `?t=` on the URL; `pi` gets no facade at all.
- `services/hub/internal/busclient/client.go` — the WebSocket caller (one connection, backoff, call correlation, `ErrConnLost` on drop, 5s `readyWait`, 64 MiB read limit). Facade-local monotonic call ids, reset per connection.
- `services/hub/internal/parentwatch` — facade self-exits when its launcher dies (no-op without `WORKSPACER_PARENT_PID`); see `hub-process-supervision`.
- `apps/desktop/src/main/services/mcpFacadeDaemon.ts` — spawns/supervises the binary (`--addr 127.0.0.1:7897 --hub <url> [--token <hub token>]`), health-poll, exit-driven restart with backoff. Still never passes `-mcp-token` — see Gotchas.
- Tests — `main_test.go` (`TestFacadeRoutesToolToHub` end-to-end against a real in-process hub; `TestFacadeNoProvider`), `auth_test.go` (bind policy, auth gate, `/health` open), `tiers_test.go`, `plugins_test.go`, `ui_test.go`.

## Auth (tiered, per request)
`requireBearer` is gone; `/mcp` and `/sse` sit behind `authGate`. A request's credential (Authorization bearer, or `?t=` query for URL-only clients) resolves to:
- the static `-mcp-token` / `WKS_MCP_TOKEN` → operator;
- a `tokens.json` scoped token → its tier (`view`/`triage`/`operator`) + its `plugins` opt-ins;
- **no credential** → governed by the `-untokened` dial, whose SHIPPED DEFAULT IS `deny` (401). `view` and `operator` are explicit opt-ins; a set static token refuses credential-less requests regardless of the dial;
- a **present-but-unknown token → 401**, never quiet operator.

`resolveRecord`'s untokened switch is ALLOWLISTED, not denylisted: only
`untokenedOperator` and `untokenedView` return a record. `deny`, the zero value,
and anything that slipped past `checkUntokenedMode` all fail closed, so an
`authGate` built without the field cannot hand out operator. Pinned by
`TestUntokenedDefaultDeniesFleetControl` (auth_test.go), which asserts the
constant, the env-override resolution (`untokenedDefault()`), the gate at both
the shipped default and the zero value, and a 401 at the HTTP boundary.

## Failure modes
- **Non-loopback bind with no token refuses to start** (`checkBindPolicy` → `log.Fatalf`); bare ports, `0.0.0.0`, `::`, LAN IPs and unresolved hostnames are all treated as non-loopback.
- **Hub bus unreachable on a tool call** → `ErrNotConnected` after 5s (or `ErrConnLost` mid-flight), rendered by `forward()` as `CallToolResult{IsError:true}` — a normal tool error, never a hang. No provider for the method → the hub's `"no provider for <method>"` verbatim as `IsError` text.
- **Empty/`"null"` capability result** normalized to the literal `"ok"` so side-effecting calls don't return confusing empties.
- **`plugins.tools` unavailable** (older hub, or hub briefly down): the catalog just stays empty/stale until a successful poll — plugin tools disappear from *new* per-token servers, no error surfaced to the session.
- **Facade death**: parentwatch self-exit on launcher death; own crash → `mcpFacadeDaemon.ts` restart with backoff.

## Gotchas
- **Untokened access is DENIED by default (changed 2026-08-29).** `defaultUntokened = untokenedDeny` in `services/hub/cmd/mcp/main.go`; a bare local client presenting nothing gets 401, not operator. This was affordable because NO legitimate consumer is credential-less: `claudeSpawn.ts` and `managedSpawn.ts` always mint a per-session token when `wantsFacade` (header on the `--mcp-config` file for Claude PTY/stream, `?t=` on the URL for codex/opencode/copilot), and `services/hub/cmd/brain/facade.go` does the same headlessly. `pi` is the one spawn path that gets the bare `MCP_FACADE_URL`, and it ships no MCP client at all — `providers/pi.rs` only logs a warning. `supervisorMcpConfigPath()` (the shared untokened `supervisor-mcp.json`) is now dead: `facadeSpawnArgs` only falls back to it when called without a token, which no caller does. `facade.untokenedAccess: operator` is the opt-in for a hand-configured client; `workspacer token create --scope operator` is the better answer. `mcpFacadeDaemon.ts` still never sets `WKS_MCP_TOKEN`, and deliberately does NOT pass `--untokened` either — the binary's default IS the shipped policy, so there is one place to change it. The untokened `view`/`operator` records get **no plugin tools** (opt-in is per token record).
- **Two distinct tokens, easily confused.** `-token`/`HUB_TOKEN` authenticates the facade's *outbound* hub-bus connection; `-mcp-token`/`WKS_MCP_TOKEN` (and the per-session tokens) guard its *inbound* HTTP surface. Opposite directions, independently set.
- **Tier membership is never edited in the facade.** Adding a bus method to `viewMethods`/`triageMethods` in `authtoken.go` is what admits the corresponding tool to those tiers — the facade derives. The ONE exception is event-backed tools (`ui.go`), which have no method to derive from and use the explicit `tierRank` gate; any new event-publishing tool must do the same and pin it in `tiers_test.go`/`ui_test.go`.
- **Plugin tools are strictly opt-in per session** (`pluginTools` spawn option → token record `plugins`); never ambient. The manifest side (`tools` validated against consent-pinned `provides`, `Manager.ConsentedTools`, sdk.js `provide()`) lives in `hub-plugin-system`.
- **`spawn_agent`'s `mcpFacade`/`toolScope`/`pluginTools` params are desktop-only.** `services/hub/cmd/brain/parity_test.go`'s `spawnParamsDeclined` documents that the headless brain declines them — headless has no facade URL to wire and cannot mint the per-session token (that's `remoteTokens.ts`, desktop-owned).
- **The facade never advertises an output schema** — results pass through as untyped `TextContent`; clients must parse the JSON text.
- **`addTool[In]` vs `addObjectTool` must be chosen deliberately**: typed structs for most tools; open `map[string]any` for inherently free-form nested params (`save_config`, `update_profile`, `save_saved_session`, `save_layout`, `save_library`). A partial struct silently drops fields.
- **Tool name ≠ hub method name; the mapping lives only in `main.go`'s registry** (snake_case tool → dotted capability). The old companion hazard — hand-written prose describing the tool set — is gone: `help` renders from the registry itself.
- **`...In` struct `json` tags are the real contract with the hub**: they build both the MCP input schema and the params object forwarded verbatim; a mismatched field name silently produces params the capability ignores. Cross-check against `hubCapabilities.ts`.
- **The facade is fleet-wide (since 2026-08-16)**: `list_agents`/`list_snapshots` merge connected federation peers (rows tagged `hub`; per-peer failures cost only their rows), and the per-session tools + `spawn_agent` take an optional `hub` param routed as `hub:<peer>/<method>` (`services/hub/cmd/mcp/federated.go`; the param is stripped before forwarding). The `-untokened deny|view|operator` dial governs credential-less access (env `WKS_MCP_UNTOKENED`, desktop config `facade.untokenedAccess`; `deny` is the default and alone satisfies the non-loopback bind policy, which is now a backstop rather than the front line). Documented publicly in `landing/docs.html` (config section), `landing/build.html#mcp`, `services/hub/README.md` and `docs/remote-sharing-security.md`. See `hub-federation`.
