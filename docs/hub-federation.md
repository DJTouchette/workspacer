# Hub Federation — Hub-of-Hubs

**Status:** ALL PHASES BUILT 2026-08-15/16. Go substrate (phases 1–3) plus the
client phases (4–5, reshaped — see notes): main-process ingest/seed/tombstones/
action-routing (`federationBridge.ts`), renderer hub badges + offline
tombstones + remote pane/respawn/pill gating + a spawn-dialog Machine picker,
and the fs-root security exclusion. Harness:
`services/hub/scripts/federation-harness.sh`. See "Implementation notes" for
where the build deviated from this proposal and why.

One workspacer client can only see one hub. This document proposes letting a
local hub link *upstream* to remote hubs and republish their events and
capabilities, so that a single client sees a single fleet spanning several
machines — without any client, plugin, or the supervisor learning to hold N
connections.

**Scope and audience.** This is a personal / small-team tool. Federation here
means "my laptop attaches to my work machine's hub," not multi-tenant service
discovery. The design is calibrated to a handful of named, manually configured
peers on a trusted network (a tailnet), reusing the auth the remote-share
feature already ships. See [remote-sharing-security.md](./remote-sharing-security.md)
for that threat model, which federation inherits unchanged.

Relevant code:

- `services/hub/internal/bus/bus.go` — `Frame`, connection classification, the
  per-conn policy gates.
- `services/hub/internal/bus/rpc.go` — `router`: provider ownership, `call`
  routing, the 30s `callTimeout`.
- `services/hub/internal/busclient/client.go` — the Go bus client, with dial,
  backoff, and call correlation already written.
- `services/hub/internal/authtoken/authtoken.go` — scoped tokens, `view` /
  `triage` / `operator` tiers.
- `services/hub/internal/event/event.go` — `Envelope` and `Matches`.
- `services/hub/cmd/hub/main.go` — flags, where peers get declared.
- `apps/desktop/src/main/services/hubClient.ts` — the desktop's bus client and
  its capability-drift report.

## The idea

Every client talks to exactly one bus. That invariant is why the renderer, the
`/m` PWA, the plugins, the supervisor, and the MCP facade all work over the same
wire without knowing where anything runs. Federation keeps it.

The local hub opens an outbound bus connection to each configured peer, using
the same `busclient` a plugin sidecar would. It subscribes to the peer's events,
stamps them with the peer's name, and republishes them onto the local broker.
Peer capabilities become callable under a hub-qualified method name. From a
client's point of view nothing changed except that the fleet got bigger and some
sessions have a hub label.

```
   desktop ─┐
   /m PWA  ─┼─► local hub ──(busclient, outbound)──► work hub ──► claudemon
   plugins ─┘        │                                    (+ its own plugins)
   supervisor        └──(busclient, outbound)──► sprite hub ──► claudemon
```

### Why not teach clients to hold N connections

It looks cheaper and isn't. Every consumer of the bus would need connection
state, per-connection auth, per-connection reconnect, and a merge policy — the
renderer, the PWA, the TUI, the MCP facade, and every plugin, each solving it
slightly differently. Federating in the hub solves it once, in Go, in the
process that already owns a broker and a router. The supervisor gains the
ability to drive remote agents as a side effect, with no supervisor changes.

### Why it stands alone

This is worth building independent of any remote-compute plan. It delivers
"check on my work machine's fleet from my laptop," which is a thing a person
wants stated without reference to a vendor. It also means that *if* a hub ever
runs somewhere else — another box, a VPS, a Fly sprite — attaching to it is
configuration rather than a project. That sequencing is deliberate: build the
general capability, let the deployment target be a footnote.

## What already exists

Federation is unusually cheap here because most of the parts are built and
tested:

- **The outbound client.** `busclient.New(busURL, token)` dials a hub over
  WebSocket with a bearer token appended as `?token=`, maintains the connection
  with backoff, resets that backoff for long-lived links, and fails outstanding
  calls with `ErrConnLost` on drop (`client.go:79-160`). A federation link is
  this client pointed at a peer.
- **Auth.** `authtoken` already mints capability-scoped tokens with
  `view` / `triage` / `operator` tiers, stored in `tokens.json` with mtime-gated
  reload. "My laptop attaches to my work hub read-only" is an existing token
  tier, not new work. The peer hub needs no concept of federation at all — it
  sees an ordinary scoped client.
- **Transport hardening.** Origin pinning, trusted-host exemptions for TLS
  front-ends, and the 64 MiB read limit are all in place from remote sharing.
- **Reconnect semantics on the client side.** `hubClient.ts` already
  re-subscribes and re-registers on every reconnect, so the pattern federation
  needs on the Go side has a working reference.

### The gap

`busclient` is a *caller*, not a subscriber. `connectAndRead` handles only
`result` and `error` frames, with the comment `hello / subscribed / registered /
event: ignored — we only call` (`client.go:163`). There is no `Subscribe`, and
no event callback. That is the first real code.

## Design decisions

### 1. Envelope carries the hub; payloads are never rewritten

The tempting move is to rewrite session IDs at the federation boundary so remote
sessions arrive as `work:abc123`. Don't — it forces the hub to parse and mutate
every event payload it forwards, which means a per-topic field table that goes
stale the moment anyone adds an event.

Instead, add one field to `event.Envelope`:

```go
type Envelope struct {
    ID     string          `json:"id"`
    Type   string          `json:"type"`
    Source string          `json:"source"`
    Hub    string          `json:"hub,omitempty"` // NEW: peer name; empty = local
    Time   time.Time       `json:"time"`
    Data   json.RawMessage `json:"data,omitempty"`
}
```

The hub stays payload-agnostic — it stamps `Hub` and republishes bytes. Clients
key their snapshot stores by `(hub, id)` instead of `id`. `Source` keeps its
current meaning (which *component* emitted: hub, claudemon, a plugin id); `Hub`
answers a different question (which *machine*), and conflating them would break
the existing meaning.

Note this differs from the obvious "namespace the IDs" sketch: the qualification
lives in the envelope and in client-side keying, not in the payload.

### 2. Topics stay unqualified

`event.Matches` is prefix-based, so prefixing remote topics (`remote.work.agent.spawned`)
would neatly hide them from existing `agent.*` subscribers — and thereby destroy
the feature, since a unified fleet is the entire point. Subscribers keep
subscribing to `agent.*` and get local and remote alike, discriminating on
`Hub` when they care.

The cost is honest and should be stated: **every existing subscriber starts
seeing more events than it did.** Anything that assumed "every session I see is
one I can act on locally" is now wrong. Auditing those assumptions is real work
and is called out as a phase below, not hand-waved.

### 3. Methods are hub-qualified; loop prevention is a tree invariant

`router.providers` is `map[string]uint64` — one method, one provider connection
(`rpc.go:69`). Two hubs both providing `agents.spawn` is a genuine collision, and
the existing hijack guard (`rpc.go:179`) will correctly refuse the second
registration rather than silently misroute it. So remote capabilities must be
registered under a qualified name:

```
agents.spawn              → local, unchanged
hub:work/agents.spawn     → routed over the federation link to peer "work"
```

Local methods keep their bare names, so nothing existing changes and the
desktop's drift report (derived from the `registered` ack in `hubClient.ts:212`)
stays meaningful.

**Loop prevention:** forbid mesh topologies in configuration and the problem
disappears. The rule is one flag on the connection — never re-forward an event
that arrived over a federation link. No hop lists, no path vectors. If peers can
link to each other this gets genuinely hard, so make it unrepresentable rather
than solvable.

## Constraints the existing code imposes

These are the sharp edges found by reading the bus, and each one is a place a
naive implementation breaks:

- **Tier allowlists are exact names, not globs.** `authtoken.go`'s `viewMethods`
  / `triageMethods` are deliberately exact-match so a broadly-named new method
  can't sneak into a low tier. A qualified `hub:work/agents.list` therefore fails
  closed for every `view` and `triage` token. The fix must be deliberate: strip
  the `hub:<peer>/` prefix, run the existing tier check against the bare method,
  and refuse if the peer name isn't a configured peer. Do *not* relax the
  allowlists to globs.
- **The broker drops, silently.** `broker.Publish` fans out non-blocking with a
  64-event per-subscriber buffer and no backpressure (`Subscription.dropped` is
  the only trace). Federating a busy remote hub multiplies event volume through
  the same buffers. Expect to need a larger buffer on federation links and a
  visible drop counter before this is trustworthy.
- **`callTimeout` is 30s and federated calls take two hops.** An outer call that
  times out at the same deadline as the inner one produces an ambiguous failure.
  The federation hop needs a shorter budget than the local one so the inner
  failure is the one the user sees.
- **Local handlers shadow remote providers.** `rt.local` is consulted before
  `rt.providers`, and `register` refuses any method the hub owns in-process. Any
  method the local hub implements itself can never be federated under its bare
  name — another reason qualification is not optional.
- **The register hijack guard applies to trusted connections too**, on purpose.
  Federation must not add an exemption; qualified names mean it never needs one.

## The path

Five phases. Each is independently useful and independently revertible; nothing
after phase 1 is required for phase 1 to be worth having.

### Phase 1 — `busclient` learns to subscribe

`services/hub/internal/busclient/client.go`

Add `Subscribe(patterns ...string)` and an `OnEvent(func(event.Envelope))`
callback. Track subscriptions in the client and re-send them on every
(re)connect, mirroring `hubClient.ts`. Handle the `event` and `subscribed` ops in
`connectAndRead`'s switch instead of ignoring them.

*Done when:* a Go test stands up a hub, connects a `busclient`, publishes an
event, and observes it — including across a forced reconnect.

### Phase 2 — Federation links

New `services/hub/internal/federation/`, wired in `cmd/hub/main.go`.

A repeatable `-peer name=work,url=ws://host:7895/bus,token=…` flag matching the
existing flag style, or a peers file if that gets unwieldy. Each peer gets a
`busclient` running in a goroutine, subscribed to `*`, stamping `Hub` on every
inbound envelope and republishing to the local broker. Mark the connection as a
federation link so its events are never re-forwarded.

*Done when:* two hubs on one machine, one configured as the other's peer, and a
`agent.snapshot` published on the peer arrives on the local bus with
`hub: "work"`.

### Phase 3 — Qualified capability routing

`services/hub/internal/bus/rpc.go`, `services/hub/internal/authtoken/authtoken.go`

Register each peer's advertised capabilities under `hub:<peer>/<method>`. On
`call`, detect the prefix, validate the peer, strip it for the tier check, and
forward over that peer's `busclient` with a reduced timeout. Relay the result or
error back to the caller unchanged.

*Done when:* `hub:work/agents.list` returns the peer's agents; a `view` token can
call it and cannot call `hub:work/agents.spawn`; an unknown peer prefix is
refused rather than treated as a literal method name.

### Phase 4 — Reachability and the unreachable-hub problem

`services/hub/internal/federation/`, then the renderer's snapshot store.

Publish `hub.peer.connected` / `hub.peer.disconnected` events with last-seen
timestamps. In the client, a persisted layout, favourite, or snoozed triage item
referencing `work:abc123` while `work` is down must render as a tombstone —
"hub unreachable, last seen 2h ago" — and not silently vanish. Silent
disappearance reads to a user as "my agent died," which is the worst available
failure mode and the reason this phase isn't optional.

*Done when:* killing a peer turns its cards into tombstones and restoring it
brings them back live, with no loss of layout state across the gap.

### Phase 5 — Audit the "every session is local" assumption

Everywhere that consumes `agent.*` events.

This is the phase that will be underestimated. Path-bearing capabilities
(`fs.*`, `search.project`, `git.*`) operate on the *local* filesystem; a remote
session's `cwd` is meaningless locally, and the confinement logic in
`hubCapabilities.ts` derives its roots from snapshot cwds
(`snapshotGrantsFsRoot`). A remote session must not contribute local filesystem
roots — that is a security-relevant bug, not a cosmetic one. Sweep for anything
that reads a session cwd, opens a file, or runs git, and make it hub-aware or
explicitly local-only.

*Done when:* a remote session in the fleet contributes no local fs roots, and
capabilities that can't act on it are hidden rather than failing on click.

## Open questions

1. **Capability discovery.** How does the local hub learn what a peer provides?
   Options: a `hub.capabilities` method on the peer, or the `hello` frame's
   `Methods` list. The latter is likely already sufficient and avoids new
   protocol surface — needs checking against what `hello` actually carries for a
   scoped connection.
2. **Event volume.** Is subscribing `*` to a peer defensible, or should
   federation subscribe a curated topic list? Measure against the 64-event
   buffer before deciding.
3. **Nested federation.** A peer that is itself federated would republish its own
   peers' events. The tree invariant permits this; whether the UI can express
   two-level hub labels is a separate question. Suggest forbidding it initially.
4. **Token lifecycle.** Peer tokens live in the local hub's config. Rotation,
   revocation, and what a client shows when a peer token is rejected (currently
   an unauthorized handshake, silently retried with backoff forever) need a
   visible failure state.

## Non-goals

- **Multi-tenant or discovery-based federation.** Peers are named and manually
  configured. No gossip, no registry.
- **Mesh topologies.** Explicitly unsupported; see loop prevention above.
- **Any particular remote-compute vendor.** Federation makes "the hub runs
  somewhere else" a config line. Which somewhere is out of scope here, and the
  case for a specific vendor should be made on its own terms rather than
  smuggled in as a motivating example.
- **Moving the working tree off the local machine.** Federation federates
  *control*, not filesystems. Phase 5 draws that line deliberately; erasing it
  is a much larger project with a much weaker case.

## Known limitation: a peer's remote nodes are invisible

Federation only forwards two event prefixes, `agent.*` and `workflow.*`
(`ForwardTopics` in `services/hub/internal/federation/federation.go`), and no
client issues a qualified `hub:<peer>/nodes.list` call to a peer hub. A remote
worker node, such as a Fly node registered in a hub's `nodes.json`, is tracked
by the hub that owns it, and its state never rides those two event prefixes,
so it never appears in a federated fleet view. This is silent: there is no
error and no tombstone, the node simply is not there. If you need to see or
wake a specific hub's nodes, do not rely on a federation link for it. Connect
a client to that hub directly instead: on desktop, use remote-client mode
(the "Connect to Server…" entry in the Remote Share dialog) to point the app
at that hub, or open that hub's own `/app` URL from a browser. Either way you
are now looking at that one hub on its own rather than the merged fleet, but
its nodes are visible again.

## Implementation notes (2026-08-15, post-build)

What shipped, and where reality corrected the proposal:

- **Phase 1** (`busclient.Subscribe`/`OnEvent`, re-sent on reconnect) — as
  designed. Proven by `internal/busclient/subscribe_test.go`, including a
  same-port hub restart. One test-harness discovery worth keeping: an
  `http.Server`'s Shutdown/Close never touches hijacked WebSocket conns, so a
  "kill the hub" test must track and sever accepted conns itself.
- **Phase 2** (`internal/federation/`) — as designed, with two corrections:
  - **The curated forward list is design, not an open question** (was open
    question 2). Subscribing `*` is a correctness/security hole, not a volume
    concern: a peer's `layout.changed` would clobber the local shared-layout
    document, its host-only `plugin.*` topics carry manifests and
    secret-bearing stderr, and its `command.*` events would drive THIS
    machine's UI. `ForwardTopics = {agent.*, workflow.*}`; a topic joins the
    list deliberately or not at all.
  - **Peers live in `peers.json` (0600), not flags or config.yaml.** A `-peer`
    flag token rides argv (`/proc` world-readable), and config.yaml is
    credential-free by design — that invariant is what keeps `config.get`
    unguarded. The flag form remains for tests/dev.
  - Forwarded envelopes get their broker `ID` cleared (per-broker id spaces)
    and `hub.peer.connected`/`disconnected` are classified open-by-decision in
    all four pinned registries (eventtopics.go, the consent fixture,
    EVENT_TOPIC_RULES, the event-plane delivery matrix).
- **Phase 3** (qualified calls) — as designed, with one addition and one
  correction:
  - **Plugin tokens are refused federated calls outright.** Prefix-stripping
    leniency for a plugin would silently extend a consented "this machine"
    grant to every peer; if plugin federation is ever wanted it must be a
    distinct, explicitly-consented grant shape. Scoped user tokens get the
    strip-and-check-bare-method rule as proposed; trusted callers pass.
  - Local argument confinement (`authorize`) is deliberately skipped for
    federated calls — paths name the PEER's filesystem. The peer's own capspec
    plus the link token's scope are the enforcement, and the link token is the
    ceiling on everything forwarded.
  - Forward budget is 25s vs the router's 30s, as proposed.
- **Open question 1 (discovery) resolved the other way:** the `hello` frame's
  `methods` is the caller's grant CEILING, not the peer's provider catalog —
  it cannot drive discovery. v1 does lazy discovery: forward and let an
  unbacked method fail with the peer's own "no provider" error.
- **A gap the proposal underplayed: the call plane doesn't federate for
  free.** Every call-seeded view (the `/m` boot seed via `sessions.snapshots`,
  the MCP facade's `list_agents`) answers with the LOCAL fleet only; events
  alone don't fix restart-blindness. The desktop main process therefore seeds
  remote sessions by calling `hub:<peer>/sessions.snapshots` on
  `hub.peer.connected` and ingesting the results — the client-side phases
  treat this as mandatory, not polish. The facade's tools remain per-hub in
  v1.
- **Client-side keying:** rather than re-keying every renderer store by
  `(hub, id)`, remote sessions are ingested into the desktop MAIN process's
  session store with a `hub` field on the snapshot (session ids are UUIDs;
  cross-hub collision is accepted as negligible). Everything downstream —
  sidebar, Triage Inbox, attention — then works off the existing snapshot
  flow, with action routing branching on `hub`. Remote sessions contribute NO
  local filesystem roots (`snapshotGrantsFsRoot` excludes them) and their
  cwd-bound panes (terminal, git review, editor) are hidden.
- **Harness:** `services/hub/scripts/federation-harness.sh` runs the fake
  second PC — a peer hub on :8895 with synthetic agents republished every 2s —
  and prints the peers.json line to point a real hub at it. (Run it with
  `WORKSPACER_PARENT_PID` unset if launched from inside a workspacer session,
  and don't bind the scratch local hub on 7895 while the desktop app runs.)
