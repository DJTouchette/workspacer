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

## Hand-authored notes (2026-08-24/29) — self-reference, node identity, credential leaks, and the missing HTTP path

- **A hub-side "is the fleet idle" predicate defeats itself twice.** Building
  `fleet.quiescence` surfaced two self-referential loops that are not obvious
  until you wire it up. (1) The hub's own loopback busclient (shared by the jobs
  runner and the quiescence sampler) is a permanently-connected bus conn, so any
  client-activity test counts **the hub asking itself** as somebody using the
  machine. Fixed with `bus.Server.SetInternalKey`: a per-process `crypto/rand`
  nonce appended to the self-dial URL as `?internal=<key>`, checked in
  `handleBus`. It is PROVENANCE, not authorization — the self-client holds the
  host token regardless. (2) The caller of `fleet.quiescence` is itself a bus
  client, so a 5-minute poller shows up in the next reading as activity. Fixed by
  `CallerIdentity.ConnID` plus a per-conn `askedAt` map: a connection whose most
  recent act was asking this question is dropped from the client list, and counts
  again the moment it does anything else. (Relatedly, a hub job whose action is
  `shell` is deliberately NOT counted as due-soon work — the shell action is how
  the check runs.)
  **Any future hub-side predicate that reads live bus connections hits the same
  two loops.** Reuse `bus.Server.Clients()` / `bus.ClientInfo.UserFacing()`
  (`services/hub/internal/bus/clients.go`) — the shared answer to "is this connection a person
  or is it infrastructure" (providers, plugin tokens and the internal client are
  all infrastructure) — rather than counting connections. **Connection presence is
  never the right test for "in use"**: a phone with `/m` in a background tab holds
  a socket forever without sending a frame, which is why `conn.lastActiveMilli` is
  bumped only on `call` and `publish`, never on `subscribe`/`register`/`result`.
- **The hub has NO provider/node identity at the HTTP layer; scoped-vs-host is
  the only non-evadable distinction.** A remote worker node authenticates with an
  operator-tier scoped token from `tokens.json`, and at the HTTP layer there is no
  way to tell "a node" from "the user's own operator pairing":
  `bus.Server.Authorized` admits the host token and any operator-scoped token
  identically, `ScopedIdent` carries only Scope/Methods/grants (now also Label),
  and the provider-ness of a connection (`conn.mayProvide`) is a property of a
  live `/bus` WebSocket, **not of an HTTP request** — so gating on "is currently a
  provider" would be trivially evaded by a bare POST carrying the token. The only
  real distinction available today is **host token vs scoped token**: the host
  token is a file on the hub (`<config>/workspacer/remote-token`) that no node is
  ever handed, and every local caller (desktop `hubAuthHeaders`→`getHubToken`, the
  CLI, `workspacer plugin dev`) presents it, while nodes are minted
  `workspacer token create --scope operator`.
  This is why `/plugins/install`, `/plugins/examples/install` and
  `/plugins/reload` are wrapped in `services/hub/cmd/hub/hostOnlyRoute`
  (`bus.Server.HostAuthorized`) rather than `guard()` — those three run code on
  the hub host and `guard()` admitted a node's token. It also BOUNDS the gate: a
  node handed the HOST token is indistinguishable from the host, and no
  credential-keyed check can see that. **Any future "refuse this to nodes" gate
  must key on the credential, not the connection.** The durable fix is a provider
  tier so `conn.mayProvide` stops promoting a connection to trusted; until then,
  capspec's `RouteHostOnly` disposition marks which routes need the stronger gate
  and its completeness guard pins the wrapper in both directions. The concrete
  escalation this leaves open (a node's operator token reaching `/bin/sh` via
  `jobs.upsert` + `jobs.run`) is in `modules/fly-node-deploy.md`.
- **Bus dial errors quote the TOKENED URL — every log site needs
  `services/hub/internal/redact`.** `coder/websocket`'s dial error embeds the full request URL
  verbatim (`failed to send handshake request: Get "https://host/bus?token=<32-char
  token>": context deadline exceeded`). Every bus dial in `services/hub` carries
  its credential as a `?token=` query param — a browser WS handshake cannot set an
  Authorization header, so `services/hub/internal/bus` treats the query form as canonical — so
  brain's per-reconnect `brain: bus disconnected (%v)` published the node's whole
  `HUB_TOKEN` once per attempt. **Observed live in Fly's log pipeline, dozens of
  lines in one 2-minute hub outage.** Fixed by `services/hub/internal/redact`
  (`Text`/`URL`/`Error`), applied at `services/hub/cmd/brain/bus.go` `session()`+`run()`,
  `services/hub/cmd/brain/main.go`'s banner, `services/hub/internal/federation` connect/disconnect,
  `services/hub/cmd/workspacer/plugindev.go` and `services/hub/cmd/mcp/main.go`. **Wrap any error or URL
  that could come from a bus/HTTP dial in `redact.Error`/`redact.URL` before
  logging it.** `services/hub/internal/busclient` still stores the tokened URL in `Client.url`
  and today only survives because it drops dial errors silently — if it ever
  starts logging them, redact there too (or give `Client` a `SafeURL()`). Note
  `workspacer serve`'s ready banner prints the token BY DESIGN (the headless
  terminal is the pairing surface) — on a Fly node that stdout IS the log
  pipeline, so a token still lands there once at startup.
- **There is NO HTTP path to call a bus method.** The bus mounts only `/bus`
  (WebSocket) and `/health` (`services/hub/internal/bus/bus.go`); every other route is a
  plugin/static `AddRoute` in `services/hub/cmd/hub`. No HTTP endpoint performs a capability
  call — which is why `workspacer fleet quiescence` exists as a bespoke CLI seam
  for ONE read (its own header says "a bus method is not reachable from a shell").
  So "how do I poke a session from a script/webhook/CI" has exactly three answers
  today: a hand-rolled `/bus` WebSocket, an MCP client at the facade, or an
  **unauthenticated** curl at claudemon. The `workspacer` CLI has no `call` verb
  (subcommands are serve|plugin|status|token|fleet|jobs|install-cli) and `fleet`
  has only `quiescence`. **Add new shell-reachable reads/writes as
  `workspacer fleet <verb>` subcommands** in `services/hub/cmd/workspacer/fleetcmd.go` — that
  file already owns token resolution, host/port flags and the exit-code contract.
  Do not assume a hub HTTP POST exists.
  **The remaining credential-less path is claudemon, not the facade.**
  claudemon's session API on `127.0.0.1:7891` has no auth check at all —
  `AllowedHosts` is a DNS-rebind guard only — so `POST /sessions/:id/message` and
  `/sessions/spawn` are a one-line wake for any local process. *(Corrected
  2026-08-29: the MCP facade is no longer part of this set. `services/hub/cmd/mcp` used to ship
  `-untokened operator`; its default is now `deny`, so a credential-less client on
  :7897 gets a 401 — see `modules/mcp-tool-facade.md`. That fix deliberately did
  NOT extend to claudemon, because closing it means distributing a secret to
  clients in three languages including the PTY wrapper inside every agent
  process.)*
