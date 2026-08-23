---
title: claudemon Daemon HTTP/SSE/WS API Surface
tags: [claudemon, rust, axum, http-api, sse, websocket, mcp]
related_paths:
  - "services/claudemon/src/daemon/api.rs"
  - "services/claudemon/src/daemon/hook.rs"
  - "services/claudemon/src/daemon/wrapper_ws.rs"
  - "services/claudemon/src/daemon/mcp_ask.rs"
  - "services/claudemon/src/daemon/init.rs"
  - "services/claudemon/src/daemon/mod.rs"
owner: Damien Touchette
last_reviewed: 2026-08-16
---

# claudemon Daemon HTTP/SSE/WS API Surface

`daemon::run` (`services/claudemon/src/daemon/mod.rs`) binds two independent Axum apps on two `TcpListener`s: `hook::router` on `cfg.hook_port` (ingress-only, Claude Code's own hook/statusLine POSTs) and `api::router_with_host` on `cfg.api_port` (everything a client — desktop, TUI, hub, MCP-speaking agents — calls). Splitting them lets the hook listener stay dumb/unauthenticated (Claude Code itself is the only caller, always loopback) while the client-facing API carries the Host/CORS/body-limit hardening. `API_BASE` (`once_cell::sync::OnceCell<String>` in `mod.rs`) is set once in `run` (`0.0.0.0`→`127.0.0.1` for the announced base) so adapters can hand agents a callback URL (e.g. `/mcp/ask/:session_id`) without re-deriving the port.

## Key modules
- `services/claudemon/src/daemon/api.rs` — `router_with_host` mounts ~24 `/sessions/*` REST routes (spawn, spawn-managed, get/list, input, message, approve, answer, decide, gate, signal, permission-mode, model, resize, output, stream, transcript, conversation, handoff), plus `/conversation/stream`, `/events`, `/hooks/stream`, `/statusline/stream` SSE, `/wrapper/:id` WS upgrade, `/mcp/ask/:session_id`, `/health`. Layers (applied inner→outer): `DefaultBodyLimit::max(16MB)`, `cors_layer()` (loopback-origin-only `CorsLayer`), `host_guard` middleware (outermost, added last — runs first).
- `services/claudemon/src/daemon/hook.rs` — hook-port router: `POST /hook`, `POST /hook/:kind` (mapped via `subroute_to_event`), `POST /statusline`, `GET /health`; same 16MB `DefaultBodyLimit`. Holds `DECISION_TIMEOUT = 30s` for the PreToolUse gate.
- `services/claudemon/src/daemon/wrapper_ws.rs` — `GET /wrapper/:id` WS upgrade; wrapper sends `Register` first, then `Output`/`Exited` frames; daemon pumps `Input`/`Signal`/`Resize` back over an unbounded mpsc.
- `services/claudemon/src/daemon/mcp_ask.rs` — one-tool MCP streamable-HTTP server (`POST /mcp/ask/:session_id`; GET 405s).
- `services/claudemon/src/daemon/init.rs` — `claudemon init` / `run_with_port` / `run_overlay`: idempotent JSON merge into `~/.claude/settings.json` (or a `--settings` overlay file).
- `services/claudemon/src/daemon/mod.rs` — `run`, `ServeConfig`, `API_BASE`, graceful shutdown (`wait_for_parent_exit`, `kill_all_ptys`); out of scope for this doc beyond the two-listener bind.

## Failure modes
- `hook::process` parks a PreToolUse hook on `store.park_decision` and awaits up to `DECISION_TIMEOUT` (30s, safely under Claude Code's default 60s hook timeout); on timeout or dropped channel it falls through to an empty `{}` passthrough decision rather than blocking Claude forever. Gating only applies when `!driver_owned` (PTY, non-managed, transport != `Stream`) and `!is_ask_question` — AskUserQuestion always passes through so the picker renders and `/answer` resolves it separately.
- `mcp_ask::tools_call` blocks up to `ANSWER_TIMEOUT = 6h` on the managed answer channel; `QuestionGuard`'s `Drop` impl is the *only* path that restores `SessionMode::Question` back to `Responding` and unregisters the answer channel when the HTTP request is aborted mid-flight (agent killed) — the timeout/answered/closed paths call `guard.finish()` explicitly and defuse the drop.
- All four SSE endpoints (`event_stream`, `hook_stream`, `status_line_stream`, `conversation_stream`) wrap a `tokio::sync::broadcast::Receiver` in `BroadcastStream` and `filter_map` away `RecvError::Lagged` with only a `tracing::warn!` — a slow SSE subscriber silently drops events with no resync signal to the client. `stream_bytes` (`/sessions/:id/stream`) is the one exception: it drives the receiver directly (not via `BroadcastStream`) so on `Lagged` it can `await` `store.output_snapshot` and repaint the client with a full terminal reset (`\x1bc`) instead of dropping silently.
- `spawn_persistence_task` (mod.rs) subscribes to the hook broadcast and writes each event to SQLite on the blocking pool; a `Lagged` there just warns and continues (events are lost from persistence, not just from a live subscriber).
- `init.rs` writes are atomic (`tmpfile` + `fsync` + `rename`) and idempotent via `TAG`/`STATUS_TAG` command markers, but a malformed `settings.json` (`hooks` present as non-object, or top-level non-object) causes `merge_hooks`/`merge_status_line` to warn and skip rather than fail loudly — a user with a broken settings file silently gets no claudemon hooks registered.

## Gotchas
- `AllowedHosts` (`api.rs`) only ever allows loopback plus **one** concrete `bind_host` string (`mod.rs` passes `cfg.host` from `router_with_host`); a wildcard bind (`0.0.0.0`/`::`) contributes nothing extra (`AllowedHosts::new` filters it out), so remote/non-loopback clients cannot reach the API directly at all — they must go through the hub bus. This is a DNS-rebind guard, not an auth mechanism: it runs as the outermost middleware layer specifically so a rebound request never even reaches CORS or a handler.
- CORS (`cors_layer`) reflects only loopback `Origin` headers and allows no credentials; this is defense-in-depth for a browser context, not a substitute for `host_guard` (CORS can't block same-origin post-rebind requests).
- The 16MB `DefaultBodyLimit::max` on both the hook router and the API router matters because hook/statusline bodies are cloned, broadcast to every SSE subscriber, and persisted to SQLite — an unbounded body would be a fanout DoS, not just a memory spike on one handler.
- `valid_session_id` (api.rs) is a path-traversal guard used by `get_transcript` and `post_handoff` (session id becomes a filename under `~/.workspacer/handoffs/` or a JSONL path) — any new handler that interpolates `id` into a filesystem path must call it too; it is not applied uniformly across every route (e.g. `/sessions/:id` itself doesn't need it, no FS use).
- `init.rs`'s `HOOK_EVENTS` const array is a **manually-enumerated mirror** of `HookEventKind::REGISTERABLE` (see comment at line ~29) — the const-context limitation means this list must be hand-kept in sync with `HookEventKind` variants in `crate::session::state`; adding a registerable hook kind there without updating `HOOK_EVENTS` here silently skips installing it.
- `hook::subroute_to_event` 404s unknown `/hook/:kind` subroutes on purpose (no silent typo passthrough) — any new hook kind needs an entry here.
- The API and hook routers are two entirely separate `Router`s with independent middleware stacks; a change to CORS/host/body-limit hardening on `api.rs` does **not** apply to `hook.rs` (and shouldn't — the hook port is meant to be dumb, loopback-only-by-convention ingress from Claude Code's own hook shellouts, installed via `init.rs`'s `curl 127.0.0.1:<hook_port>/hook`).
- `post_permission_mode`/`post_model` branch on `store.has_managed_permission_mode` / `store.is_managed` to pick between PTY-shift+tab, managed-adapter-flag, and stream-control-protocol code paths — this domain doc covers only the routing/response shape; the actual switching logic lives in `SessionStore` (out of scope here, see `PermissionSwitchError` variants for the error taxonomy surfaced as 409s).
- Explicitly excluded from this doc: `services/claudemon/src/daemon/spawn.rs` (agent-spawn concern) and session state/snapshot internals (`crate::session::*`, covered elsewhere) — this doc is the transport/router surface only.

## Hand-authored notes (2026-07-30) — POST /oneshot

- `POST /oneshot {argv, model?, prompt, timeout_secs?}` → `{ok, text?, error?}` runs ONE
  headless `claude --print` turn and returns raw stdout. Added for the desktop's agent
  auto-titling; it is deliberately generic (any little "ask a cheap model a question about
  a session" job belongs here).
- **Why it exists at all**: a caller shelling out to `claude --print` itself fires the
  user's Claude Code hooks against `/hook`, and `SessionStore::ingest` registers a stray
  session for them — a ghost row in RECENT per call. Verified empirically before building
  the endpoint (`sessions` count 35 → 36 after one bare `claude --print`), and
  `--settings '{"hooks":{}}'` does NOT suppress it (settings merge, 35 → 36 again).
  `/oneshot` pins `--session-id <uuid>` and marks it via `SessionStore::mark_heartbeat`,
  the same trick `daemon::heartbeat` uses, so `ingest` drops those hooks wholesale.
  End-to-end check: sessions `[]` before and `[]` after a successful call.
- `heartbeat::home_dir` is now `pub(crate)` and shared — both the heartbeat and the
  one-shot run their child in the home dir so it picks up no project context.
- The desktop falls back to a heuristic title (not a local shell-out) when the route is
  missing, so an older claudemon binary degrades quietly instead of resurrecting the ghost.
- Its first consumer is agent auto-titling: `config.agents.autoTitle {enabled, model}`
  (default on, haiku), renderer `useAgentAutoTitle` fires once per agent when the opening
  exchange has both a user message and an assistant reply, main's `agentTitler` sanitizes
  the model output. `AgentWorkspace` carries `nameSetByUser` + `autoTitled` (persisted, so
  a restart doesn't re-title). Reminder: any new `ElectronAPI` method must be triaged in
  `tests/backend/backendParity.test.ts` or the parity test fails.
- **Hardening (2026-07-30)**: the prompt goes on **stdin** (`claude --print` reads it),
  never argv — on non-npm Windows installs `claudeBaseArgv()` returns
  `['cmd.exe','/c','claude']`, so agent-written text on the command line was an injection
  class. And both `/oneshot` and the keep-warm heartbeat used to pipe stderr without
  draining it: a child filling the 64KiB pipe blocks in write(2) and burns the whole
  timeout — `/oneshot` drains both pipes with `tokio::join!`, heartbeat uses
  `Stdio::null()`. Keep both properties for any future child-spawning endpoint.
