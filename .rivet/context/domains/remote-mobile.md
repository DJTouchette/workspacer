---
title: Remote Control & Mobile
tags: [remote, mobile, hub-bus, pwa, push]
related_paths:
  - "services/hub/**"
  - "apps/desktop/src/main/services/remoteServer.ts"
  - "apps/desktop/src/main/index.ts"
  - "services/hub-rs/assets/web/m-next/**"
  - "services/hub-rs/src/server/web.rs"
  - "services/hub-rs/src/services/kept_sessions.rs"
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

## /m-next (native-aligned phone client, phase 2)

`services/hub-rs/assets/web/m-next/` is the rewrite of `/m` that looks and
talks like the native client. It is served beside `/m` (untouched) at
`/m-next/` (`/m-next` 308s there so relative module URLs and the service
worker scope root at `/m-next/`). It is plain ES modules + CSS with no build
step, compiled into the hub through the generated `src/server/m_next_assets.rs`
table; `web::m_next` serves it with ETags (fonts immutable) and a strict CSP on
`index.html` (script-src 'self', so no inline scripts — `js/boot.js` applies the
theme before paint).

Generated, drift-checked in `make check-hub-rust-assets`:
- `tokens.css` + `js/themes.js` from `apps/native/src/appearance.rs` (palettes)
  and `apps/native/src/ui/syntax.rs` (code colours) by
  `scripts/gen-mobile-tokens.py`; `scripts/test-gen-mobile-tokens.py` proves the
  check fails when a native colour changes.
- the asset table and the service worker's precache list/cache name (a content
  hash) by `scripts/check-mobile-next-assets.mjs`, which also parses every module
  and checks relative imports. Run it with `--write` after adding a file.
- `scripts/test-mobile-next-fleet.mjs` runs `js/fleet.js` against
  `contracts/fleet-message-cases.json`.

The data layer (bus/store/push) is a faithful port of `/m`'s: token in
`localStorage.hubToken` (shared with `/m`), hello-scope gating, handshake
watchdogs and wake, machine-stop pause/Wake, federation `hub:<peer>/` routing,
sparse folds, peer tombstones, archive ordering, conversation polling. Push uses
its own worker (`/m-next/sw.js`, scope `/m-next/`), so a phone that enables
notifications in both clients holds two subscriptions. Push prefs are shared
with `/m` (`pushPrefs`).

Paused sessions come from the host: `sessions.kept` (view/triage, hub-local,
`services/kept_sessions.rs`) reads the native client's
`<XDG_CONFIG_HOME|~/.config|%APPDATA%>/workspacer/native-settings.json`
`kept_open` (all hub scopes merged; ids that are not this hub's never resolve)
and the client reads aged-out kept rows by id with `sessions.snapshot`. A
missing file is an empty set. Without the method, stopped sessions show as
Ended and are still resumable by sending.

Tests: `apps/desktop/tests/e2e/mobileNext.test.ts` with
`fixtures/mobileNextHub.ts` (real hub + fake provider shaped like the Rust
hub's sparse rows; native-settings.json planted in the scratch config home).
`WKS_MNEXT_SHOTS=<dir>` captures the 390×844@2x screenshot set.

Class-name hazard: the CSS is global; generic class names collide (a `.other`
input rule once resized the island's `.mchip.other`). Prefer specific names.

