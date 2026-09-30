---
title: Hub Web Push (VAPID + agent-needs-you PWA alerts)
tags: [hub, rust, web-push, vapid, mobile-pwa, notifications]
related_paths:
  - "services/hub-rs/src/services/push/mod.rs"
  - "services/hub-rs/src/services/push/crypto.rs"
  - "services/hub-rs/src/services/push/watch.rs"
  - "services/hub-rs/src/services/push/tests.rs"
  - "services/hub-rs/assets/web/sw.js"
owner: Damien Touchette
last_reviewed: 2026-09-30
---

# Hub Web Push

## Current Rust ownership

Current identity/subscription ownership and RPCs are in
`services/hub-rs/src/services/push/mod.rs`; `crypto.rs` and `watch.rs` own
validation/encryption and observed transition handling. The shipping service
worker is `services/hub-rs/assets/web/sw.js`. Run Rust library tests filtered
by `services::push` for local contract coverage; they do not establish delivery
through a real phone service. The Go paths and test command below are retained
historical pointers, not current source ownership or build requirements.

Historical execution, when deliberately requested, uses the separate pinned
checkout described in [scripts/reference/README.md](../../../scripts/reference/README.md).
This crosswalk does not certify platform or release gates.

## Ownership and delivery

`services/hub/internal/push/push.go` owns the VAPID keypair, persisted browser
subscriptions, notification preferences, and the snapshot transition watcher.
The hub must remain running to send notifications; the installed `/m` client
can be closed because its service worker receives Web Push independently of
the application's bus socket.

`services/hub/cmd/hub/main.go` registers `push.key`, `push.subscribe`,
`push.unsubscribe`, `push.test`, `push.list`, and `push.revoke`. Subscriptions
record the bus caller's token fingerprint and scope through `RPCSubscribeAs`.
The token validator suppresses delivery after that credential is revoked.
`push.list` and `push.revoke` are not in view/triage allowlists. Keep those
administration methods separate from the phone's subscription operations.

`push.test` reports delivered, gone/pruned, and failed counts and bypasses
per-kind preferences. It sends real notifications; use unit tests rather than
invoking it against someone's configured hub during a documentation check.

## Notification triggers

`Watch` consumes `agent.snapshot`, and `onSnapshot` tracks state by session ID.
Malformed payloads or missing IDs are ignored. Current triggers are:

- Entering `waiting_approval` or `waiting_input` from a non-blocked state.
- Becoming idle after a tracked working run, including a run parked on an
  approval/question. The default per-device duration threshold is 60 seconds.
- Ending a session already tracked by this watcher. Replayed historical ended
  rows do not notify.
- Optional still-working checkpoints at 10 and 30 minutes. They are evaluated
  when snapshots arrive, not by a separate timer.

`thinking`, `streaming`, and `background` count as working. The title prefers
the dispatch label, then original cwd, then live cwd, then “Worker”. Per-device
`prefs` control needs/finished/ended notifications (default on), previews
(default on), checkpoints (default off), and `finishedAfterSec`. A zero finish
threshold means every completed run; absent/negative uses the default.

Approval/question text and the latest assistant reply can populate a preview,
clipped at a Unicode boundary. With preview disabled the payload describes the
event without the agent's words. This is a single-operator broadcast model,
not per-user fleet filtering. Subscription identity supports revocation; it
does not select which agent activity a still-authorized device may receive.

## Snapshot sources

Desktop `apps/desktop/src/main/services/hubTelemetry.ts` emits compact background
snapshots, not whole conversations. Its `isRemoteShareEnabled` gate reads the
cached effective sharing setting: an environment override or the persisted UI
toggle. The full-scope brain also emits compatible snapshots with `ambientState`
through `services/hub/cmd/brain/enrich.go`. A raw daemon state without the
compatible fields is not a drop-in replacement.

The watcher consumes only event data, not the envelope's hub stamp. Its state
keys and push payload identify a session by ID alone; do not describe this as
hub-qualified identity or assume colliding IDs from peers remain distinct.

## Persistence and failures

`vapid.json` and `push-subscriptions.json` live under the configured push directory
(default `<UserConfigDir>/workspacer-hub`). Preserve the keypair across restarts.
When keys are missing/unreadable and old subscriptions exist, `loadVAPID` generates
new keys, logs state loss, and drops subscriptions tied to the old key. Devices
must subscribe again. A failure constructing the push manager logs “push:
disabled”; it does not prevent the hub from starting.

Each delivery attempt has a 10-second timeout, a 60-second TTL, and high urgency.
Transport failures and non-2xx responses are logged; they are not silently
swallowed. There is no retry queue. HTTP 404/410 prunes the dead endpoint from
storage. Subscription-file writes currently ignore write errors, so an RPC
success is not proof that a subscription survived a disk-write failure.

Endpoint validation in `services/hub/internal/push/endpoint.go` requires HTTPS
and rejects literal non-public IP addresses. Hostnames are not resolved during
validation; the shared HTTP client has a timeout but no custom DNS/redirect
confinement. Do not describe this validation as a complete network sandbox.

`states` belongs to the single watcher goroutine. RPC handlers and delivery
pruning share `subs` under a mutex; adding another writer of `states` requires
an explicit concurrency decision.

## Service worker and verification

`services/hub/cmd/hub/sw.js` shows the payload, collapses repeated notifications
under `sessionId` (`renotify: true`), and opens/focuses `/m` with that ID. Serve
the phone UI over HTTPS for browser push support; ordinary HTTP LAN hosting
does not provide the required secure context.

From `services/hub`, run `go test ./internal/push`. This exercises trigger,
preference, revocation, key-loss, endpoint, and bounded-delivery behavior with
controlled test endpoints; it does not prove delivery through a real phone's
push service or OS notification settings.
