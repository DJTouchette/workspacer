---
title: MCP Tool Facade
tags: [hub, mcp, agents, authentication, plugins]
related_paths:
  - "services/hub/cmd/mcp/*.go"
  - "services/hub/cmd/brain/facade*.go"
  - "apps/desktop/src/main/services/mcpFacadeDaemon.ts"
  - "apps/desktop/src/main/services/mcpConfig.ts"
owner: Damien Touchette
last_reviewed: 2026-09-16
---

# MCP Tool Facade

`services/hub/cmd/mcp` exposes the hub capability bus over `/mcp` and `/sse`.
The facade is an adapter: tools forward calls to the desktop provider or the
headless brain. `/health` reports the exact service, listen address, hub URL,
hub connection, and initial plugin-catalog readiness.

## Spawned-agent contract

Every supported Workspacer-spawned agent receives the ambient operator tool
surface and every enabled plugin tool. `toolScope`, `pluginTools`, profile-grant
and yolo-grant fields remain accepted only for mixed-version compatibility and
do not narrow or widen that surface. Pi is not admitted because its CLI has no
MCP bridge.

Each session still receives a unique bearer (`session:<id>`) in `tokens.json`:
an Authorization header for Claude or `?t=` for URL-only clients. The bearer is
identity and lifecycle revocation, not a grant selector. Manual/remote clients
may independently mint view/triage/operator/provider credentials with
`workspacer token create`; those tiers remain enforced.

Desktop and headless spawns both verify facade readiness before minting or
injecting a bearer. The headless brain treats its configured URL only as a
probe target and checks service, bind, expected hub (when known),
`hubConnected`, and `pluginCatalogReady` for every spawn. A failed check omits
the facade and mints nothing; a later healthy check recovers automatically.

## Supervision

The desktop's `mcpFacadeDaemon.ts` adopts an exact healthy external facade or
owns a local child. Owned-child restart timers carry a generation fence and are
cancelled on adoption, stop, and newer startup, so a stale timer cannot kill or
replace an adopted `workspacer serve` listener. Headless `workspacer serve`
starts the brain only after the initial exact MCP health gate; per-spawn probes
provide the lifetime guarantee after later crashes/restarts.

## Authentication and plugins

Credential-less access defaults to `deny`; `facade.untokenedAccess` may
explicitly opt a hand-configured local client into view or operator. A present
unknown/revoked token is always 401. Enabled plugin tools are ambient in spawned
agent catalogs. The facade waits for the initial catalog before readiness and
polls later changes; clients caching `tools/list` may need to reconnect.

## Key tests

- `cmd/brain/facade_test.go`: per-spawn healthy→down→healthy, identity/bind/hub
  refusal, no token leak, pointer-only ordinary skills.
- `mcpFacadeDaemon.test.ts`: exact adoption/readiness and stale-restart race.
- `claudeSessionStore.test.ts`: retry-safe, once-per-lifecycle revocation.
- `cmd/mcp/auth_test.go`, `tiers_test.go`, `plugins_test.go`: manual credential
  tiers, authentication, and plugin catalog behavior.
