---
title: Remote Control & Mobile
tags: [remote, mobile, hub-bus, pwa, push]
related_paths:
  - "services/hub/**"
  - "apps/desktop/src/main/services/remoteServer.ts"
  - "apps/desktop/src/main/index.ts"
owner: Damien Touchette
last_reviewed: 2026-08-16
---

# Remote Control & Mobile

## Overview
The hub daemon (services/hub) exposes the broker as a bidirectional WebSocket (/bus) with bearer-token auth, serving two embedded web clients: /remote (desktop-like remote) and /m (mobile PWA). The /m PWA can be installed and pinned to home screen; Web Push notifications wake its service worker when an agent is blocked on approval/question, even with the app closed. Desktop clients can also run in remote mode against a `workspacer serve` instance, connecting via `remoteServer.ts` config persistence.

## Key modules
- `services/hub/cmd/hub/main.go` - hub daemon; routes /bus, /remote, /m, /app/, PWA assets, and registers push.* RPC handlers
- `services/hub/cmd/hub/remote.html` - embedded token-guarded remote client; speaks bus protocol over /bus
- `services/hub/cmd/hub/mobile.html` - embedded unguarded mobile PWA shell; gates on stored token, registers for push
- `apps/desktop/tests/e2e/mobileClient.test.ts` + `fixtures/mobileHub.ts` - the /m e2e suite: boots the real hub with a fake bus provider and drives the client in a phone-sized Chromium

## The /m client (rebuilt 2026-07-24)
Five screens behind four tabs — Fleet, Needs You, Chat, Inspector (Flows/Agents/Usage), New — all projections of `sessions.snapshots` + `agent.snapshot`. Logic ported from the desktop renderer so the two agree; each port names its source file in a comment:
- `formatToolSummary` (components/claude-shared.tsx) → the tool lines on cards, work-card steps, stream label
- `collectRecentActivity` (lib/agentActivityLog.ts) → the "what is it doing" line; `plan.activeForm` is the FALLBACK, only when there's no observable activity
- `collectEditedFiles` / `patchLineCounts` (lib/turnChanges.ts) → +added/−removed. The desktop refines these with `git.numstat`; the phone can't (git.* is operator + desktop-provider only), so it shows the tool-input estimate — the same numbers the desktop shows for restored history
- `useAttentionFeed.ts` → the inbox kinds, priorities (approval 100 → done 20) and ordering; snooze is 30 min, matching AttentionContext
- `deriveSessionStats` / formatters (lib/sessionStats.ts), `shortModelLabel` (lib/modelLabel.ts), `providerCaps.ts` (composer pills)

**The "finished" state has no snapshot field.** `done` is derived from the working→idle *edge*, held in a client-side map (the desktop does the same in a ref). A fresh page load of an already-idle agent therefore raises no `done` item — only `bigdiff` can flag it. Any test for it must drive a live transition, not seed an idle snapshot.

**Scope-gated UI.** The `hello` frame now carries `scope` + `methods` (see bus.go `helloFrame`), so /m greys out spawning on a triage token instead of offering a button that dies on tap. Operator and host tokens both report `operator` — an operator record is promoted to trusted at the handshake, so the tier is not otherwise recoverable.

**Fonts load non-blocking** (`media="print" onload`): the phone often reaches the hub over a tailnet with no route to fonts.googleapis.com, and a render-blocking stylesheet there stalls the shell behind a DNS timeout.
- `services/hub/cmd/hub/sw.js` - service worker for background Web Push + shell cache (fast startup)
- `services/hub/internal/push/push.go` - VAPID keypair generation/persistence, subscription management, push sender
- `services/hub/internal/bus/bus.go` - WebSocket transport; per-connection identity + ScopedIdent authorization (view/triage/operator)
- `services/hub/internal/authtoken/authtoken.go` - scoped token lookup and scope.Methods() method-pattern matching
- `apps/desktop/src/main/services/remoteServer.ts` - remote server config read/write to ~/.config/workspacer/remote-server.json (mode 0600)

## Failure modes
**Token auth split:** /remote guarded at route, /m unguarded but shell cached; real boundary is /bus requiring token on write. A dead or malformed token causes /bus frames to 401, but cached shell still loads. **VAPID keypair loss:** persisted under ~/.config/workspacer-hub/; if deleted, new keypair generated but all subscriptions become stale (phones won't receive push). **Dead endpoints:** push service returns 404/410; subscriptions pruned. If pruning fails (FS error), stale endpoints remain; pushes sent until service sweeps them. **Scope creep:** new bus methods default to deny for scoped tokens; must be explicitly added to ScopeView/ScopeTriage/ScopeOperator method lists or calls fail closed. **Service worker staleness:** cache versioned by CACHE const; if not bumped, stale shell cached for 24h on clients. **Remote mode boot race:** desktop checks remoteServer at startup before any other startup; if wrong/slow, boots local daemon first, then must relaunch to switch.

## Gotchas
**A stale hub binary serves a stale client.** `mobile.html`/`remote.html`/`sw.js` are `go:embed`'d, and `go build ./...` (multiple packages) *discards* executables — it does not refresh `services/hub/hub`. Any test or manual check must rebuild with `go build -o hub ./cmd/hub` first, or it silently exercises the previous client. The e2e fixture rebuilds unconditionally for exactly this reason.
**The hub shuts down when stdin closes** (parentwatch). Spawning it from a test/script with `stdio: ['ignore', …]` makes it exit within milliseconds; give it an open pipe.
**Two token systems:** host remote-token (full operator, implicit, no scope record) and scoped tokens (tokens.json, view/triage/operator tiers). When migrating from one to the other, existing pairings never break (host token always wins). **Layout document is a consistency point:** shared across all clients (desktop, remote, /m) and persisted to disk by hub; two clients writing concurrently can race (mtime-gated in config-two-writers pattern, but layout.set has no such guard). **Push is best-effort:** VAPID keys + subscriptions are the only state this domain persists; if the push dir is unwritable, the whole push subsystem silently disables, but /m still works (no error propagation to the shell). **Mode-switch semantics:** remoteServer config is read-only at main process startup; UI must call os.exit() or relaunch after connect/disconnect. **Window.electronAPI polyfill:** desktop in remote mode uses the same web backend over /bus as a real browser; any missing IPC binding fails silently or shows as undefined. **Dead socket on mobile:** OS throttling can suspend the WebSocket while the app is backgrounded; /m wakes on visibility-change or online event (not a heartbeat). **Scoped tokens expire only on revoke:** no TTL; revoking requires tokens.json deletion + hub file-watch pickup.

## Hand-authored notes (2026-08-16) — /m is federation-aware

- `/m` (mobile.html) merges peer fleets from the one hub the phone pairs with: it keeps its own `sessionHub` map (fed by envelope `hub` stamps + a `federation.peers` / `hub:<peer>/sessions.snapshots` seed) and a `qualify(id, method)` helper on EVERY per-session call — including the `sessions.conversation` polling, which is why /m shows full remote chat history while the desktop renders only the compacted snapshot window. A scoped token that may call `sessions.conversation` may equally call `hub:work/sessions.conversation` (bare-method tier check). Offline peers keep their cards as read-only tombstones (`.off` chip form), not dropped rows. Note `federation.peers` is view-tier; an older hub without it just errors the first call and /m degrades to single-hub.
- Unlike the desktop, **/m keeps sparse brain-only rows**, so agents on a headless (`workspacer serve`-only) peer are visible on the phone but not on desktop/TUI.
- **Push covers both machines**: the hub's push watcher fires for hub-stamped remote sessions too, so agent-needs-you alerts span the merged fleet through the single paired hub. `/remote` (remote.html) deliberately stays single-hub. See `modules/hub-federation.md`.

## Hand-authored notes (2026-08-23) — desktop vocabulary follows the PWA, with named exceptions

- **The desktop renderer's fleet vocabulary now matches `/m`'s, and future UI copy should keep following `/m`, not the other way round.** `/m` moved to fleet-manager language first (commit `8d5db890`): state badges Working→"In flight", Idle→"Standing by", Finished→"Landed"; verb Spawn→Dispatch; inbox tab "Needs you"→"Waiting". The desktop renderer (SideBar, FleetDeck, AgentCard, AttentionCard, InboxDrawer, OverviewPane, SpawnAgentDialog, HomeSpace, AskPane, NotificationsSection, ClaudeProfilesSection, JobsSection, LibraryPane, Onboarding, CommandPalette, `shortcuts.ts`, `pluginPermissions.ts`, `useAttentionFeed`, `AgentWatchPane`) was then renamed to match. **Three exceptions are deliberate, not oversights — keep them if sweeping vocabulary again**: (1) `WorkingTimer.tsx`'s "Working" verb stayed — no PWA equivalent exists, and renaming it broke 8+ test assertions while mismatching the component's own name; (2) InboxDrawer's "Review" tab and the bigdiff "Review"/"Review changes" labels stayed put despite the PWA saying "Landed" — "Review" is already a distinct, load-bearing feature name elsewhere (the git-diff `ReviewPane` / paneMenu `'review'` pane type), and renaming would create ambiguity between two unrelated "Review" concepts; (3) "agent" as the general noun was NOT swapped to "worker" anywhere — matches the PWA's own restraint (only the state/verb vocabulary moved, not every noun). Approval/question language ("Needs approval", "Waiting for input") was untouched by the PWA and so left alone on desktop too.

## Hand-authored notes (2026-08-23) — `attentionFor` kinds are shared across differently-shaped payloads

In `services/hub/cmd/hub/mobile.html`, `attentionFor()` can emit the `'stuck'`
kind TWO ways: (1) a stale `pendingQuestions` item past `STUCK_MS`, which carries
`it.questions`, and (2) a progress-fingerprint stall with **no `questions` at
all**, just title/detail like bigdiff/error. `inboxCard()`'s render branch was
`it.kind === 'question' || it.kind === 'stuck'`, which unconditionally rendered
the questions-shaped body (`qs2 = it.questions || []`) for BOTH — so a stall card
rendered an empty question block with a dangling "Decline & stop" button and **no
visible title or detail text at all**.

Fixed to `it.kind === 'question' || (it.kind === 'stuck' && it.questions)`, plus a
dedicated no-questions `'stuck'` branch showing `it.title` bolded above
`it.detail` — the generic bigdiff/done branch drops `it.title` in favour of
`it.detail` alone, which would have erased the Not-moving / No-signal distinction
entirely.

**`kind` alone does not select the right body shape.** Any future `attentionFor()`
item that reuses an existing kind with a different payload must have
`inboxCard()`'s branch conditions checked explicitly. See
`domains/session-lifecycle.md` for why the Not-moving / No-signal distinction
exists at all (only Claude-on-PTY has a heartbeat).
