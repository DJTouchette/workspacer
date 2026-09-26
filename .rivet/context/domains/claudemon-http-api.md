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
last_reviewed: 2026-09-26
---

# claudemon Daemon HTTP/SSE/WS API Surface

`daemon::run` in `services/claudemon/src/daemon/mod.rs` binds two independent
Axum routers: `hook::router_with_host` on the hook port (default 7890), and
`api::router_with_host` on the API port (default 7891). Both use the configured
bind address, defaulting to loopback. Both enforce Host and request-Origin
checks plus a 16 MiB body limit; only the API router also has a CORS layer.
Neither listener authenticates native callers with a bearer token. These
browser-request guards do not turn a non-loopback bind into an authenticated
remote API; remote clients normally use the hub's authenticated bus.

`API_BASE` is set once in `run` so adapters can construct callbacks such as
`/mcp/ask/:session_id` using the configured API port. For a `0.0.0.0` bind the
announced callback base uses `127.0.0.1`.

## Key modules
- `services/claudemon/src/daemon/api.rs` — `router_with_host` mounts `/sessions/*` REST routes (spawn, spawn-managed, get/list, input, message, approve, answer, decide, gate, signal, permission-mode, model, resize, output, stream, transcript, conversation, handoff), plus provider model discovery, usage/reporting, heartbeat and one-shot endpoints, `/conversation/stream`, `/events`, `/hooks/stream`, `/statusline/stream` SSE, `/wrapper/:id` WS upgrade, `/mcp/ask/:session_id`, `/health`. Layers (applied inner→outer): `DefaultBodyLimit::max(16 MiB)`, `cors_layer()` (loopback-origin-only `CorsLayer`), `origin_guard`, `host_guard` middleware (outermost, added last — runs first).
- `services/claudemon/src/daemon/hook.rs` — hook-port router: `POST /hook`, `POST /hook/:kind` (mapped via `subroute_to_event`), `POST /statusline`, `GET /health`; same 16MB `DefaultBodyLimit`. Holds `DECISION_TIMEOUT = 30s` for the PreToolUse gate.
- `services/claudemon/src/daemon/wrapper_ws.rs` — `GET /wrapper/:id` WS upgrade; wrapper sends `Register` first, then `Output`/`Exited` frames; daemon pumps `Input`/`Signal`/`Resize` back over an unbounded mpsc.
- `services/claudemon/src/daemon/mcp_ask.rs` — one-tool MCP streamable-HTTP server (`POST /mcp/ask/:session_id`; GET 405s).
- `services/claudemon/src/daemon/init.rs` — `claudemon init` / `run_with_port` / `run_overlay`: idempotent JSON merge into `~/.claude/settings.json` (or a `--settings` overlay file).
- `services/claudemon/src/daemon/mod.rs` — `run`, `ServeConfig`, `API_BASE`, graceful shutdown (`wait_for_parent_exit`, `kill_all_ptys`); out of scope for this doc beyond the two-listener bind.

## Failure modes
- `hook::process` parks a PreToolUse hook on `store.park_decision` and awaits up to `DECISION_TIMEOUT` (30s in this implementation); on timeout or dropped channel it falls through to an empty `{}` passthrough decision rather than blocking Claude forever. Gating only applies when `!driver_owned` (PTY, non-managed, transport != `Stream`) and `!is_ask_question` — AskUserQuestion always passes through so the picker renders and `/answer` resolves it separately.
- `mcp_ask::tools_call` waits up to six hours for an answer. `QuestionGuard`
  releases its own Ask-owned pending request and unregisters its channel on
  explicit completion or future drop. A displaced Primary approval may be
  restored; finishing a question does not unconditionally force Responding or
  clear another owner’s pending card. The shim still has one answer channel
  per session: the Primary-versus-Ask ownership fence does not create independent
  channels for multiple simultaneous Ask requests.
- `/events` emits `session.resync` with `{"event":"Resync","reason":"lagged"}`
  when its broadcast receiver lags. Consumers must reconcile authoritative
  state instead of assuming a later event will repair a missed session end.
  The desktop event bridge and full-scope brain handle this signal. The
  lightweight hub `services/hub/internal/claudemon` bridge only maps session updates and
  does not itself perform this reconciliation.
- `/hooks/stream`, `/statusline/stream`, and `/conversation/stream` still log
  broadcast lag and drop the missed data. They do not send the same resync
  signal. `/sessions/:id/stream` handles terminal-byte lag by fetching
  `output_snapshot` and repainting with a terminal reset (`\x1bc`).
- `spawn_persistence_task` (mod.rs) subscribes to the hook broadcast and writes each event to SQLite on the blocking pool; a `Lagged` there just warns and continues (events are lost from persistence, not just from a live subscriber).
- `init.rs` writes are atomic (`tmpfile` + `fsync` + `rename`) and idempotent via `TAG`/`STATUS_TAG` command markers, but a malformed `settings.json` (`hooks` present as non-object, or top-level non-object) causes the merge helpers to warn and skip the malformed part. Invalid JSON is a separate parse failure; do not describe every malformed settings file as the same silent-success case.

## Gotchas
- `AllowedHosts` accepts loopback and one concrete bind-host string; wildcard
  binds add no extra allowed Host value. A present disallowed Host is refused,
  but native clients can omit or choose Host. This is a DNS-rebinding guard,
  not proof of where the caller is running or who it is.
- `origin_guard` rejects cross-site browser requests before handlers run, on
  both listeners. Absent Origin is allowed for native clients; loopback origins
  and an origin authority matching Host are accepted. Opaque/malformed origins
  are refused. CORS alone only controls browser access to responses and cannot
  prevent the side effects of simple requests.
- The 16MB `DefaultBodyLimit::max` on both the hook router and the API router matters because hook/statusline bodies are cloned, broadcast to every SSE subscriber, and persisted to SQLite — an unbounded body would be a fanout DoS, not just a memory spike on one handler.
- `valid_session_id` (api.rs) is a path-traversal guard used by `get_transcript` and `post_handoff` (session id becomes a filename under `~/.workspacer/handoffs/` or a JSONL path) — any new handler that interpolates `id` into a filesystem path must call it too; it also protects caller-pinned spawn IDs. It is not an authentication credential and is not applied uniformly to every route.
- `init.rs`'s `HOOK_EVENTS` const array is a **manually-enumerated mirror** of `HookEventKind::REGISTERABLE` (see comment at line ~29) — the const-context limitation means this list must be hand-kept in sync with `HookEventKind` variants in `crate::session::state`; adding a registerable hook kind there without updating `HOOK_EVENTS` here silently skips installing it.
- `hook::subroute_to_event` 404s unknown `/hook/:kind` subroutes on purpose (no silent typo passthrough) — any new exposed hook subroute needs an entry here. Adding a registerable event also requires checking init’s explicit list and its tests.
- The routers have independent middleware stacks, but share the same
  `host_guard` and `origin_guard` functions. When changing those protections,
  test both API and hook ingress. Do not remove hook-port guards on the
  assumption that installed curl commands are its only possible callers.
- `post_permission_mode`/`post_model` branch on `store.has_managed_permission_mode` / `store.is_managed` to pick between PTY-shift+tab, managed-adapter-flag, and stream-control-protocol code paths — this domain doc covers only the routing/response shape; the actual switching logic lives in `SessionStore` (out of scope here, see `PermissionSwitchError` variants for the error taxonomy surfaced as 409s).
- Explicitly excluded from this doc: `services/claudemon/src/daemon/spawn.rs` (agent-spawn concern) and session state/snapshot internals (`crate::session::*`, covered elsewhere) — this doc is the transport/router surface only.

## Message and action responses

`POST /sessions/:id/message` delegates to `SessionStore::submit_message`.
Live sessions can accept a message during cold start, responding, approval, or
question modes: it is sent when ready or returns `{ok:true, queued:true}` for
later delivery. Stopped sessions return 409; missing sessions/wrappers return
404, disconnected wrappers 410, and a full input queue 503. Do not disable
sending solely because the snapshot mode is not Input.

Approval and answer endpoints still require their corresponding pending mode
and can reject stale actions with 409. Treat each endpoint's response contract
separately rather than applying a blanket Input-mode gate.

API `/health` returns `ok` with `x-workspacer-maintenance: 1`; artifact cleaners
use that header to detect the spawn/cleanup admission fence. The hook listener's
health endpoint does not carry that API contract.


## One-shot prompts and hook registration

`POST /oneshot` accepts argv, prompt, optional model/timeout, `no_tools`, and
`harness_default`. It runs a single headless prompt and returns `{ok,text?,error?}`.
The request is bounded by prompt/output/timeout limits in `daemon/oneshot.rs`;
a successful HTTP response still requires checking `ok`. The harness-default
option omits an explicit model choice rather than silently applying the fallback.

The prompt travels on stdin, not through a shell-sensitive argv. Both output
pipes are drained. The one-shot UUID is marked as heartbeat traffic so its hooks
do not create an ordinary fleet session; this is not a general suppression of
all hook installation. These are real model calls when invoked against a real
provider, unlike the fake-CLI unit fixtures.

`claudemon init` uses marker-based merges to preserve unrelated hook/status-line
configuration. Overlay mode writes the standalone overlay and removes previously
installed Workspacer entries from the global file to avoid double reporting.
Check registration lists, subroute vocabulary, overlay behavior and parse-failure
handling together when adding a hook event.

The auto-title client can use `/oneshot` and has a heuristic fallback; the route
is also used by other bounded summarization work. Do not describe a specific
historical ghost-session experiment as fresh end-to-end verification of every
current caller.

## Route contracts and validation

`contracts/claudemon-routes.json` and the cross-language caller sweeps pin
served paths and caller construction. A mocked client test that accepts any URL
does not catch a deleted route. Update the route registry and actual caller list
when adding/removing an endpoint, and preserve the distinction between policy,
serialization, and live-provider validation.

From `services/claudemon`, relevant library suites are `daemon::api::tests`,
`daemon::hook::tests`, `daemon::wrapper_ws::tests`, `daemon::mcp_ask::tests`,
`daemon::init::tests`, and the one-shot fake-CLI tests. From `services/hub`,
`go test ./internal/capspec` checks the shared route/caller contracts. Library
filters must select real tests; zero selected tests is not a passing API audit.
