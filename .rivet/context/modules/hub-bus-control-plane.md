---
title: Hub Event Bus & Control Plane: Scoped Tokens, Policy & RPC
tags: [hub, go, event-bus, authz, scoped-tokens, rpc, control-plane, security]
related_paths:
  - "services/hub/internal/bus/*.go"
  - "services/hub/internal/authtoken/*.go"
  - "services/hub/internal/broker/*.go"
owner: Damien Touchette
last_reviewed: 2026-08-16
---

# Hub Event Bus & Control Plane: Scoped Tokens, Policy & RPC

## Overview
`services/hub/internal/bus` is a single WebSocket endpoint (`/bus`) that fans a pub/sub broker and a request/reply router over one wire protocol (`Frame`, in `bus.go`). It never implements capabilities itself — it classifies each connection at handshake (trusted / per-plugin / scoped user token) and then gates every subsequent `publish`/`subscribe`/`register`/`call` against that identity's grants. This is the single trust boundary every remote/web/mobile/plugin/MCP surface crosses to reach the local desktop, so any bug here is an authz bypass, not just a bug.

## Key modules
- `services/hub/internal/bus/bus.go` — `Server.handleBus`: connection classification (host token / plugin token / scoped token via `SetScopedTokenLookup`), `originAllowed` same-origin policy, `conn.mayPublish/mayConsume/mayProvide/mayCall/authorize` — the per-conn policy gates.
- `services/hub/internal/bus/policy.go` — filesystem path confinement (`canonicalize`, `within`, `pathWithinRoots`, `paramString`): "canonicalize then contain" so `..` and symlinks can't escape a plugin's granted `fsRoots`.
- `services/hub/internal/bus/rpc.go` — `router`: `register` (provider ownership + hijack guard), `call`/`result`/`failCall` correlation by global id, `callTimeout` = 30s, local in-process handlers (`registerLocal`) take precedence over remote providers.
- `services/hub/internal/authtoken/authtoken.go` — scoped capability tokens (`Scope`: `view`/`triage`/`operator`), explicit method allowlists (`viewMethods`, `triageMethods`), `tokens.json` store with mtime-gated reload (`Store.Lookup`/`refreshLocked`).
- `services/hub/internal/broker/broker.go` — `Broker.Publish`/`Subscribe`: non-blocking fan-out; a slow subscriber's channel fills and drops (`Subscription.Dropped`) rather than stalling the publisher.
- `apps/desktop/src/main/services/hubCapabilities.ts` — the desktop's registrations of real capabilities (fs.*, agents.*, git.*, etc.) against this exact bus policy surface; does its own path confinement for `fs.*`/`search.project` because a remote client holding the host token is classified `trusted` by the bus, bypassing per-plugin `conn.authorize`.

## Failure modes
- Unknown/rejected token at handshake → `http.StatusUnauthorized` before the WebSocket upgrade (`handleBus`); no partial connection state.
- Cross-site browser `Origin` → `403 forbidden origin` (`originAllowed`); non-browser clients (no `Origin` header) and loopback origins pass.
- A denied `call`, `publish`, or path-scoped call returns an `error` Frame naming the reason (`callDenied`, `authorize`) rather than silently dropping — but a provider disconnecting mid-call fails the caller via `dropConn`'s `notify` sweep, and a call to a missing provider/local handler errors with `"no provider for "+method`.
- `router.call`/`result` correlate by a hub-assigned global id (`strconv.FormatUint`); a `result`/`error` frame whose id doesn't match a pending call from the claimed `provider.id` is silently ignored (`return // unknown call, or a different conn impersonating the provider`) — this is deliberate anti-spoofing, not a bug.
- `authtoken.Store.refreshLocked` fails closed: a missing or corrupt `tokens.json` yields an empty token set, never stale/cached grants.
- `broker.Publish` drops events past the per-subscriber buffer (`defaultBuffer = 64`) with no backpressure signal to the publisher — a slow consumer silently misses events, counted only in `Subscription.dropped`.

## Gotchas
- **Register hijack guard applies to trusted conns too** (`rpc.go` `register`): a remote client presenting the host token is `trusted`, so if the guard exempted trusted callers it would reopen the exact capability-hijack hole it exists to close. Ownership is released only when the owning connection actually drops (`dropConn`), verified by `register_hijack_test.go`.
- **`mayCall` (verb) vs `authorize` (argument/path scoping) are two separate checks** in `rpc.go`'s `call()`, always in that order — a method can be verb-allowed but still denied on path containment; don't collapse them.
- **`capspec.MissingSpec`/`LooksPathBearing` double defense**: `RegisterPluginToken` refuses to grant a method that looks filesystem-scoped (`fs.*`/`search.*`) but has no `capspec.PathParam` entry, and `conn.authorize` re-checks this as defense-in-depth — a new `fs.*`/`search.*` method MUST get a capspec entry or it is refused outright, never silently unconfined.
- **`viewMethods`/`triageMethods` are exact-name allowlists, not `agents.*`-style globs**, specifically so a broadly-named new method isn't accidentally admitted for scoped tokens — any new bus method must be deliberately added to `authtoken.go`'s tier lists (or it fails closed for `view`/`triage` tokens).
- **`ScopeOperator` and the legacy host `remote-token` are treated identically** (`operator()` returns true for `*`); a scoped token minted with `operator` scope passes `Server.Authorized` for guarded HTTP routes exactly like the shared secret.
- **`cn.mayConsume` grants scoped user tokens (`scopeMethods != nil`) full event visibility** even though they can't publish or register — "view includes streams" is intentional, not a leak, per the comment in `bus.go`.
- Desktop-side `apps/desktop/src/main/services/hubCapabilities.ts` is the other half of this contract: it must independently re-implement path confinement for filesystem capabilities reachable by a `trusted` remote client, since the bus's `conn.authorize` fsRoots logic only applies to non-trusted (plugin) conns.

## Hand-authored notes (2026-08-16) — federation + plugin tools

- **Qualified federated calls** (`services/hub/internal/bus/rpc.go` + `srv.SetFederation` in `services/hub/cmd/hub/main.go`): a `call` to `hub:<peer>/<method>` routes over the federation link (25s budget vs the router's 30s). The scoped-token tier check runs against the **bare** method (prefix stripped, allowlists NOT relaxed to globs); **plugin tokens are refused federated calls outright**; local `authorize` path confinement is deliberately skipped (paths name the peer's filesystem — the peer's capspec + the link token enforce there). Full picture in `modules/hub-federation.md` / `docs/hub-federation.md`.
- **New hub-local methods**: `federation.peers` (name/connected/lastSeen, registered in `services/hub/cmd/hub/main.go`, admitted to the **view** tier in `authtoken.go` viewMethods) and `plugins.tools` (the consented plugin-tool catalog served from `plugin.Manager.ConsentedTools()`, polled by the MCP facade — see `mcp-tool-facade`). `busclient` grew `Subscribe`/`OnEvent`/`Publish` for the federation link and the facade's UI tools.
- **New classified topics**: `command.*` (facade UI navigation) and `hub.peer.connected/disconnected` (federation lifecycle) joined the four pinned registries — `capspec/eventtopics.go`, `contracts/event-topic-consent-cases.json`, `pluginPermissions.ts` `EVENT_TOPIC_RULES`, and `bus/eventplane_test.go`'s delivery matrix. Any new topic namespace must land in all four or the guard tests fail.
- `event.Envelope` gained a `Hub` field (peer name; empty = local) — see `hub-shared-cap-event-vocabulary`.
