---
title: Hub Web Push (VAPID + agent-needs-you PWA alerts)
tags: [hub, go, web-push, vapid, mobile-pwa, notifications]
related_paths:
  - "services/hub/internal/push/*.go"
  - "services/hub/cmd/hub/sw.js"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# Hub Web Push (VAPID + agent-needs-you PWA alerts)

## Overview
`services/hub/internal/push/push.go::Manager` turns "agent needs you" transitions (waiting on an approval or a question) into a lock-screen Web Push notification for the installed `/m` PWA, even with the app fully closed. It owns a VAPID keypair and the set of subscribed browsers/phones, watches the hub bus for `agent.snapshot` events, and fires push only on the un-blocked→blocked edge per session. Delivery is one-shot (fire the push, forget); it never keeps a background connection open — that is not possible on mobile.

## Key modules
- `services/hub/internal/push/push.go` — `Manager`: VAPID load/generate/persist, subscription store, bus RPC handlers (`RPCKey`/`RPCSubscribe`/`RPCUnsubscribe`), `Watch`/`onSnapshot` transition detector, `sendAll`/`sendOne` push senders.
- `services/hub/internal/push/push_test.go` — transition-edge tests using a stubbed `m.notify`; `snap()` helper builds camelCase `{sessionId, cwd, ambientState, status}` payloads matching the real wire shape.
- `services/hub/cmd/hub/main.go` — wiring: `push.New(*pushDir)` (best-effort, `-push-dir` flag, default `<UserConfigDir>/workspacer-hub`), registers the three `push.*` RPCs on the bus server, `/sw.js` route (serves `services/hub/cmd/hub/sw.js` with `Service-Worker-Allowed: /`), and `go pushMgr.Watch(ctx, b)`.
- `services/hub/cmd/hub/sw.js` — the actual service worker: `push` listener shows the notification from `{title, body, sessionId}`; `notificationclick` focuses/opens `/m` and deep-links via `sessionId`; also handles app-shell caching (unrelated to push).
- `apps/desktop/src/main/services/hubTelemetry.ts` — publishes `agent.snapshot` with full `ambientState` and rich session data when remote sharing is enabled via `isRemoteShareEnabled()` (gated by `WORKSPACER_REMOTE_SHARE` env var). The brain headless provider also publishes `agent.snapshot` with `ambientState` added via `compatSnapshot()`.

## Failure modes
- `push.New` failure (unwritable state dir) is caught in `main.go` and just logs `push: disabled (...)`, setting `pushMgr = nil` — the hub still boots, push RPCs are simply not registered and `Watch` never starts.
- `onSnapshot` silently no-ops on unmarshal failure or an empty `sessionId` (`push.go:211`) — a malformed snapshot is dropped, not logged.
- `sendOne` swallows all send errors (`push.go:268-270`) — network failures to a push service are silent, no retry.
- HTTP 404/410 from the push service means the subscription is dead; `sendOne` calls `removeEndpoint`, which takes `mu` and rewrites `push-subscriptions.json` — self-healing pruning, no operator action needed.
- On session end (`status == "ended"`), `onSnapshot` deletes the session from `states` so a later re-open starts from a clean "not blocked" baseline.

## Gotchas
- **Multiple snapshot sources publish with `ambientState`.** Both `apps/desktop/src/main/services/hubTelemetry.ts` (when remote sharing enabled) and the brain's headless provider (via `services/hub/cmd/brain/enrich.go::compatSnapshot`) emit `agent.snapshot` events with `ambientState` set. Desktop publishes full rich snapshots only when `isRemoteShareEnabled()` (the `WORKSPACER_REMOTE_SHARE` opt-in); the brain publishes in full scope mode. Push notifications work in both scenarios.
- **VAPID key stability.** `vapid.json` under the push dir must not be regenerated/lost — every phone's `PushSubscription` is bound to the public key it subscribed against; rotating it breaks all existing subscriptions until they re-subscribe via `push.key`/`push.subscribe`.
- **Concurrency split is intentional and load-bearing.** `states` (last-seen `ambientState` per session) is touched only inside `onSnapshot`, which only ever runs on the single `Watch` goroutine — no lock. `subs` is mutated from RPC handlers (any bus-server goroutine) and from `sendOne`'s pruning, so it's guarded by `mu`. Adding a second caller of `onSnapshot`/writer of `states` without a lock would be a real race.
- **No background socket.** By design there's no persistent connection kept alive in the client for background awareness — mobile OSes kill it. The service worker (`services/hub/cmd/hub/sw.js`) is woken on-demand by the push event only.
- **HTTPS requirement.** The Push API requires a secure context; `/m` must be served over HTTPS (Tailscale `serve`) for `pushManager.subscribe` to work at all — plain `http://` LAN access won't let the PWA register a subscription in the first place.
- Notification `tag` is set to `sessionId` (`renotify: true`) so repeated pushes for the same agent collapse instead of stacking — a lock screen shows one notification per blocked session, not one per event.
