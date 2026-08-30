---
title: fleet.quiescence: the hub's own bus client and the poller both had to be excluded or the signal defeats itself
date: 2026-08-24
confidence: high
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub/internal/bus/clients.go
  - services/hub/cmd/hub/quiescence.go
  - services/hub/internal/quiescence/*.go
promoted: true
promoted_to: hub-bus-control-plane
---

# fleet.quiescence: the hub's own bus client and the poller both had to be excluded or the signal defeats itself

## Observation
Building a "is the fleet idle" predicate in the hub surfaced two self-defeating loops that are not obvious until you wire it up. (1) The hub's own loopback busclient (shared by the jobs runner and the quiescence sampler) is a permanently-connected bus conn, so any client-activity test counts the hub asking itself as somebody using the machine. Fixed with bus.Server.SetInternalKey: a per-process crypto/rand nonce appended to the self-dial URL as ?internal=<key>, checked in handleBus. It is PROVENANCE not authorization (the self-client holds the host token regardless). (2) The caller of fleet.quiescence is itself a bus client, so a 5-minute poller would show up in the next reading as activity. Fixed by CallerIdentity.ConnID (new field) plus a per-conn askedAt map: a connection whose most recent act was asking this question is dropped from the client list, and counts again the moment it does anything else. Related: a hub job whose action is `shell` is deliberately NOT counted as due-soon work, because the shell action is how the check runs.

## Impact
Any future hub-side predicate that reads live bus connections hits the same two loops. bus.ClientInfo.UserFacing() (internal/bus/clients.go) is the shared answer for "is this connection a person or is it infrastructure": providers, plugin tokens and the internal client are all infrastructure.

## Recommendation
Reuse bus.Server.Clients() / ClientInfo.UserFacing() rather than counting connections. Connection presence is never the right test for "in use" — a phone with /m in a background tab holds a socket forever without sending a frame, which is why conn.lastActiveMilli is bumped only on `call` and `publish`, never on `subscribe`/`register`/`result`.
