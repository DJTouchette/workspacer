---
title: Remote Control & Mobile
tags: [remote, mobile, hub-bus, pwa, push]
related_paths:
  - "services/hub/**"
  - "apps/desktop/src/main/services/remoteServer.ts"
  - "apps/desktop/src/main/index.ts"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Remote and mobile clients

## Surfaces and authentication

`services/hub/cmd/hub/main.go` serves authenticated `/bus`, HTTP-guarded
`/remote`, public `/m` and PWA assets, plus the separately built full renderer
at `/app` when configured. `remote.html`, `mobile.html` and `sw.js` are embedded
at Go build time. Rebuild with `go build -o hub ./cmd/hub` to update the executable;
a multi-package build does not refresh that output artifact.

The mobile shell contains no session data or credentials and can load from
cache without a token. Data/control requires an authenticated bus connection.
The host `remote-token` differs from scoped records in `tokens.json`; revoke a
specific device record rather than deleting the store. Live scoped connections
are revalidated and closed after revocation/change.

Hello supplies scope/method metadata for UI gating. View/triage allowlists are
server-enforced; operator scope does not grant authenticated-host provenance
or owner-only administration. The mobile compatibility helpers treat missing
scope/method metadata permissively for older hubs, so do not describe its UI as
fail-closed before hello. Server authorization is the real boundary. Advertised
methods do not establish a healthy provider or successful action.

## Mobile state and actions

Fleet, Needs You, Chat, Inspector and New surfaces derive from snapshots plus
live events. The client ports its own tool summaries, activity, attention,
provider controls and stats helpers from desktop concepts; these are duplicate
implementations that need parity review when wire shapes change.

Done is an observed working-to-idle edge retained locally, not a session field;
a freshly loaded idle session does not produce a past completion item. Diff
counts are tool-input estimates, unlike desktop's optional git refinement.
Attention priority and 30-minute snoozing are client-side presentation state.
A `stuck` item may carry questions or just a progress-stall title/detail;
`inboxCard` must branch on payload shape as well as kind. Do not render an empty
question picker for a no-progress stall.

Conversation-less sessions use qualified `sessions.conversation` polling, with
a one-second refresh throttle and backwards-sequence rebuild. This mobile code
currently anchors subsequent fetches at the last returned `seq`; the full
renderer instead re-reads the latest held item to handle coalesced growth. Do
not promise equivalent streaming/retention behavior from similarly named
folders or from rich mock snapshots. Client silence is not proof of no daemon
progress; heartbeat and progress-fingerprint evidence differ by transport.

Mobile node controls treat stopping before offering wake. Wake/pause/reconnect
state, active requests and returned server state must remain separate. Socket
handshake watchdogs, visibility/online recovery and a 25-second visible-page
reconcile restore state after suspension; they do not guarantee background
WebSocket execution on a phone.

## Federation boundaries

`/m` maintains session-to-peer routing from stamps and peer seeds; per-session
calls use `hub:<peer>/<method>`. Sparse peer seed rows are accepted and folded
without discarding known detail. Offline peers leave read-only tombstones;
unlinked peers are a different state. Missing federation capability degrades
to local-only discovery. `/remote` intentionally remains single-hub.

Desktop main and standalone TUI also accept sparse peer rows. The full web
renderer can fold sparse events but currently skips sparse rows in its peer
seed; desktop bridged mode inherits that path. Do not claim universal discovery
parity. See [federation](../modules/hub-federation.md).

## Service worker and push

`sw.js` caches `/m` and an icon on install, activates the new cache and deletes
older cache names. `/m` GET navigation is network-first, caching the returned
response and falling back on fetch failure; there is no general 24-hour expiry
and no cache of authenticated session RPC data. Other fetches pass through.
External font CSS loads nonblocking, with system font fallback.

Push wakes the worker to show a notification without a running page/socket,
subject to browser support, permission and delivery. Notifications collapse by
session tag; clicking focuses an existing `/m` window and posts its session ID,
or opens an agent deep link. This is navigation, not an automatic approval.

The hub supports needs-you and configured completion/end/checkpoint preferences,
including peer-stamped events. Missing VAPID keys regenerate identity and drop
subscriptions tied to the old key; devices must resubscribe. 404/410 delivery
responses prune dead endpoints. See [push](../modules/hub-web-push.md) for
identity, preference, timeout and delivery contracts. No actual push delivery
is claimed by a mock/browser test.

## Desktop and test ownership

Desktop full-client mode skips local daemon startup and uses a selected server;
workers-only pairing preserves local ownership. Corrupt/missing sidecar input
falls back to local mode at boot; a later connection failure does not authorize
an unrelated local stack. See [desktop connection mode](desktop-remote-client-mode.md).

Parentwatch is enabled through `WORKSPACER_PARENT_PID`; both parent liveness and
stdin EOF paths matter. Scratch test launchers must set process ownership and
isolate persistent paths deliberately. Shared layout uses its own revision
contract, not config.yaml's writer locking.

`apps/desktop/tests/e2e/mobileClient.test.ts` boots a newly built real hub and
fake capability provider, then drives phone-sized Chromium. It verifies client
UI/actions, not live provider behavior, real phone OS suspension or Web Push.
Keep sparse/headless fixtures in scope; rich desktop rows alone cannot validate
all conversation fallback behavior. Audit execution evidence lives in
[the review ledger](../../../docs/reviews/workspacer-rivet-audit.md).
