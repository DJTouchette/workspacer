---
title: MCP Tool Facade
tags: [hub, mcp, agents, authentication, plugins]
related_paths:
  - "services/hub-rs/src/mcp.rs"
  - "services/hub-rs/src/mcp/ui.rs"
  - "services/hub-rs/assets/mcp-tools.json"
  - "services/hub-rs/assets/mcp-help.json"
  - "services/hub-rs/src/services/session_facade.rs"
  - "services/hub-rs/tests/mcp.rs"
  - "apps/native/src/ui_requests.rs"
  - "scripts/mcp-catalog.py"
  - "apps/desktop/src/main/services/mcpFacadeDaemon.ts"
owner: Damien Touchette
last_reviewed: 2026-11-14
---

# MCP Tool Facade

`services/hub-rs/src/mcp.rs` exposes the Rust hub capability bus over `/mcp`
and `/sse`. The Go facade (`services/hub/cmd/mcp`) is retired and exists only
in the pinned historical checkout. The facade is an adapter: tools forward
calls to the hub's own services, in the native app's embedded backend and
under `workspacer serve` alike. `/health` reports the exact service, listen address, hub URL,
hub connection, and initial plugin-catalog readiness.

## Catalog

The catalog is `services/hub-rs/assets/mcp-tools.json`, keyed by scope: 103
operator tools, 18 triage, 10 view (`scripts/mcp-catalog.py --check`, `make
check-hub-mcp-catalog`). Help text is `mcp-help.json`.

`open_guide` and `open_plugin` were removed: the panes they opened do not
exist in the native app. `mcp/ui.rs` maps the remaining UI tools to bus topics
(`focus_agent`, `open_pane`/`open_browser`, `open_spawn_dialog`,
`run_ui_action`) and forwards only each tool's documented keys. The
descriptions of `open_pane`, `open_browser`, `run_ui_action` and `notify`
describe native behaviour: `open_browser` offers an `http(s)` page that the
person opens in their own browser; `open_pane` names the views the app has;
unknown pane types and action ids show a notice and do nothing. What the
native window does with each request is in `apps/native/src/ui_requests.rs`.

`notify` reaches clients as a `notify.post` event when no host claims
`notifications.post` (see
[shared desktop services](headless-desktop-services.md)).

## Rust wire compatibility

The rmcp 3.5 adapter advertises MCP `2026-07-28` as well as older revisions.
Its default `server/discover` includes cache hints, but its default list result
types omit them. The facade explicitly supplies `ttlMs: 0` and
`cacheScope: "private"` for tools, prompts, resources and resource-template lists.
Tool catalogs depend on caller scope and live registrations, so they must not
be shared between identities. Unsupported resource reads remain RPC errors.

`services/hub-rs/tests/mcp.rs` exercises real authenticated HTTP requests for
modern discovery/list metadata and all list families, plus tool calls and
view/operator filtering under `2026-07-28`, `2025-11-25` and `2025-06-18`.
The release bundle smoke additionally checks actual modern and legacy
`tools/list` replies: `/health` alone did not catch Claude Code rejecting every
tool when the modern cache fields were missing. A working native-app session
does not prove it negotiated the same version or connected to the same facade.

## Spawned-agent contract

When a healthy facade is available for injection, supported Workspacer-spawned
agents receive its ambient operator tool surface and every enabled plugin tool. `toolScope`, `pluginTools`, profile-grant
and yolo-grant fields remain accepted only for mixed-version compatibility and
do not narrow or widen that surface. Pi is not admitted because its CLI has no
MCP bridge.

Each session still receives a unique bearer (`session:<id>`) in `tokens.json`:
an Authorization header for Claude or `?t=` for URL-only clients. The bearer is
identity and lifecycle revocation, not a grant selector. Manual/remote clients
may independently mint view/triage/operator/provider credentials with
`workspacer token create`; those tiers remain enforced.

Readiness is checked before minting or injecting a bearer. An owned Rust facade
is verified through its runtime handle, exact bound MCP address and internal
launch readiness. Legacy external-facade mode requires HTTP200 plus matching
service, bind, expected hub, `hubConnected`, and `pluginCatalogReady` on every
launch; health probes carry no bearer and follow no redirects.

A configured but unverified Rust facade refuses launch before minting. This is
an explicit change from the retired Go brain's omit-and-continue fallback.
Deliberately disabled facade mode remains supported and does not invent a URL
or credential. Failed checks are not cached, so later healthy preparation can
recover. See `services/hub-rs/src/services/session_facade.rs` for these distinct
ownership modes.

## Supervision

The Rust backend owns the facade listener in the same runtime as the hub, in
the native app and under `workspacer serve`; there is no separately started
facade process or Go brain. The native default install prefers port 7897 for
it and saves the port it got (`config/mcp-port`), because agents keep the MCP
URL they were launched with and some outlive the app. In the deprecated
Electron app, `mcpFacadeDaemon.ts` adopts an exact healthy external facade or
owns a local child, with generation-fenced restart timers. Per-launch readiness and generation-bound credentials remain required
after initial startup.

## Authentication and plugins

Credential-less access defaults to `deny`; `facade.untokenedAccess` may
explicitly opt a hand-configured local client into view or operator. A present
unknown/revoked token is always 401. Enabled plugin tools are ambient in spawned
agent catalogs. The facade waits for the initial catalog before readiness and
polls later changes; clients caching `tools/list` may need to reconnect.

## Key tests

- `services/hub-rs/tests/mcp.rs`: authenticated HTTP requests, protocol
  revisions, scope filtering.
- `services/hub-rs/src/mcp/ui.rs` unit tests: UI tool to topic mapping, and
  that `open_guide`/`open_plugin` map to nothing.
- `apps/native/src/ui_requests.rs` unit tests: what the native window accepts.
- `mcpFacadeDaemon.test.ts` (Electron only): exact adoption/readiness and
  stale-restart race.
- The Go facade tests (`facade_test.go`, `auth_test.go`, `tiers_test.go`,
  `plugins_test.go`) are in the historical checkout only.

## Outbound identity delegation

A dedicated scoped operator token used by a facade does not automatically gain
permission to assert another local session’s identity. The bus’s
`mayAssertLocalSession` requires local control-plane provenance or explicitly
provisioned facade authority and still excludes revoked/plugin/federated callers.
`workspacer token facade-authority` provisions that bit for a dedicated service
label; it does not grant authenticated-host-only administration or let a session
bearer self-promote. Keep inbound session authentication and outbound delegation
checks separate when diagnosing connected-but-refused tools.
