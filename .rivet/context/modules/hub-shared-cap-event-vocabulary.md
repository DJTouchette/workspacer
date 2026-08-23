---
title: Hub Shared Vocabulary: capspec Grants & the event Envelope
tags: [hub, go, capabilities, event-bus, security-invariant, wire-protocol]
related_paths:
  - "services/hub/internal/capspec/*.go"
  - "services/hub/internal/event/*.go"
owner: Damien Touchette
last_reviewed: 2026-08-16
---

# Hub Shared Vocabulary: capspec Grants & the event Envelope

## Overview
`services/hub/internal/capspec` and `services/hub/internal/event` are tiny, dependency-free packages that let `services/hub/internal/bus` (enforcement) and `services/hub/internal/plugin` (manifest validation/loading) agree on shapes without importing each other, avoiding a bus↔plugin import cycle. capspec is the single source of truth for which capability methods are filesystem-path-scoped and defines the `Grant`/`EventGrants` structs a plugin token carries; event defines the transport-agnostic `Envelope` and the topic-matching rules (`Matches`/`MatchesAny`) used identically for event delivery and for `EventGrants` pattern checks.

## Key modules
- `services/hub/internal/capspec/capspec.go` — `PathParam` map, `IsPathScoped`, `LooksPathBearing`, `MissingSpec`, `Grant`, `EventGrants`.
- `services/hub/internal/capspec/capspec_test.go` — unit tests plus `TestDesktopCapabilitiesAllScoped`, which regex-parses `apps/desktop/src/main/services/hubCapabilities.ts` (`registerCapability('name', …)` / `cat('name', …)`) and cross-checks every desktop-registered capability against `PathParam`; skips (doesn't fail) if that file isn't reachable (cross-repo checkout).
- `services/hub/cmd/brain/capspec_guard_test.go` — `TestBrainMethodsAllScoped` cross-checks the headless brain's registered methods (`r.methods()` + `r.catalogMethods()`) the same way; `TestSpawnStaysDeliberatelyUnscoped` pins `agents.spawn` as intentionally unscoped (it carries `cwd` but spawning is a separate authz decision from path confinement).
- `services/hub/internal/event/event.go` — `Envelope` struct, `Matches`, `MatchesAny`, `New` (ID/Time left blank, stamped by the broker at publish).
- `services/hub/internal/bus/bus.go` — the enforcement side: `RegisterPluginToken` (grant-time `MissingSpec` fail-closed refusal, `canonRoots`), `conn.authorize` (per-call path containment via `IsPathScoped`/`LooksPathBearing`), and event delivery/provider checks via `event.MatchesAny`.
- `services/hub/internal/plugin/manifest.go` — the validation side: manifest `Capabilities` entries must declare `Paths` if `capspec.IsPathScoped` says the method is path-scoped (line ~218), or `Load`/validate rejects the manifest.

## Failure modes
- **Fail-closed at grant time**: `bus.RegisterPluginToken` (`services/hub/internal/bus/bus.go` L227, the check at L241) calls `capspec.MissingSpec(g.Method)` and, if true, logs `"[bus] SECURITY: refusing to grant..."` and drops the grant entirely rather than admitting it unconfined.
- **Fail-closed at call time (defense in depth)**: `conn.authorize` (`bus.go:561-574`) re-checks `IsPathScoped`/`LooksPathBearing` per call; if a method somehow slipped through `mayCall` unscoped-but-path-named, it is denied with `"named like a filesystem capability but has no capspec entry; denied to avoid running unconfined"`. A path-scoped grant with zero `fsRoots`, a missing path param, or a path that can't be canonicalized (`pathWithinRoots` error) is also denied — never silently allowed.
- **Fail-closed at manifest-load time**: `manifest.go` rejects any plugin manifest declaring a path-scoped capability with no `Paths` (`"capability %q is filesystem-scoped and must declare \"paths\""`), before a grant is ever built.
- **Build-time guard, not runtime**: the whole invariant (no fs.*/search.* method escapes without a `PathParam` entry) is caught by tests, not production code paths — `capspec_test.go`'s `TestDesktopCapabilitiesAllScoped` and `services/hub/cmd/brain/capspec_guard_test.go`'s `TestBrainMethodsAllScoped`. If both are skipped/stale (e.g. the desktop TS file path changes or the regex `capNameRe` stops matching a new registration syntax), a new unscoped fs.* method could ship undetected until it's actually granted (where the runtime fail-closed logic still stops it, just silently via the log line rather than a build failure).
- **event.Matches asymmetry**: `"agent.*"` does not match bare `"agent"` — `TrimSuffix` leaves the trailing dot so the prefix is `"agent."`, requiring at least one more path segment. A pattern author expecting `"agent.*"` to also cover the literal type `"agent"` will silently under-match.

## Gotchas
- **Twin maintenance across two languages**: `PathParam` (Go) has no compile-time link to `hubCapabilities.ts` (TypeScript) — they're kept in sync only by the regex cross-check tests, which are best-effort (skip on unreachable path) rather than hard guarantees in every build context.
- **`pathVerbPrefixes` must stay a superset of `PathParam` namespaces**: `TestPathParamEntriesAreUnderKnownNamespaces` enforces that every `PathParam` key's namespace is also in `pathVerbPrefixes` (`"fs."`, `"search."`), but the guard is a naming *convention*, not a type system — a capability named e.g. `files.read` outside these prefixes would silently bypass all of this even if it touches a filesystem path.
- **`LooksPathBearing` is heuristic, not proof**: it only checks name prefix. A method under `fs.`/`search.` that legitimately has no path (unlikely but possible) would be flagged as `MissingSpec` until explicitly added to `PathParam`; conversely a path-carrying method outside those two namespaces is invisible to the whole mechanism.
- **`agents.spawn` is deliberately unscoped** despite carrying `cwd` — `TestSpawnStaysDeliberatelyUnscoped` is a tripwire: if `capspec.IsPathScoped("agents.spawn")` ever becomes true without the brain's spawn dispatch (PTY *and* spawn-managed paths) first learning root confinement, that test fails on purpose to force the conversation.
- **`FSRoots`/roots are canonicalized once, at grant time**, not per call — `canonRoots` (`bus.go:164`) resolves symlinks/`..` when `RegisterPluginToken` runs; a root that fails to canonicalize is silently dropped (not error — "can't safely grant anything"), so a plugin could end up with fewer usable roots than its manifest declared, with no loud failure.
- **Empty `EventGrants` means the plugin can do literally nothing on events** — no publish, no receive, no provide — matching capability calls' fail-closed default; trusted host connections (`cn.trusted`) bypass all `Grant`/`EventGrants` checks entirely, so trust boundary bugs elsewhere (misclassifying a conn as trusted) would defeat this whole vocabulary.
- **`event.New` never sets `ID`/`Time`** — callers must go through the broker's publish path to get those stamped; constructing and inspecting an `Envelope` from `event.New` directly (e.g. in tests) without publishing will have a zero-value `Time` and empty `ID`.

## Hand-authored notes (2026-07-22)

- **`notify.post` event + extended `notifications.post` capability** (in-app notification center). The renderer's `NotificationsProvider` (apps/desktop/src/renderer/src/contexts/NotificationsContext.tsx) ingests bus events of type `notify.post` directly (the desktop subscribes to `*`, so no forwarding change was needed); payload fields mirror the `notifications.post` capability params: `{ title (required), body, level: info|success|warn|error, source, sessionId, paneType, url, key, silent }`. The capability (hubCapabilities.ts) now records in-app via `agentNotifier.postInApp` (IPC `notify:in-app`) AND raises a clickable OS notification (click: sessionId → focus agent, url → shell.openExternal, else focus window), gated by `notifications.enabled` / `inAppOnly` param. Because the capability is an RPC (not a broadcast), a capability call and a `notify.post` publish never double-deliver — but a plugin doing BOTH will duplicate.
- The headless brain also registers a stub `notifications.post` (logged + acked) — it does not publish `notify.post`, so capability calls against a headless hub reach no UI; web clients only get the event path.

## Hand-authored notes (2026-08-16) — Envelope.Hub, new topic classes, contract guards

- **`event.Envelope` gained `Hub` (`hub,omitempty`)** — the federation peer name; empty/absent = local. It answers "which *machine*", while `Source` keeps answering "which *component*" (hub/claudemon/plugin id) — do not conflate them. Payloads are never rewritten at the federation boundary; only the envelope is stamped (`modules/hub-federation.md`).
- **Topic classification lives in FOUR pinned registries** and every new topic namespace must land in all of them: `capspec/eventtopics.go`, `contracts/event-topic-consent-cases.json`, the renderer's `pluginPermissions.ts` `EVENT_TOPIC_RULES`, and `bus/eventplane_test.go`'s `topicDeliveryKey` delivery matrix. 2026-08-15/16 added `command.*` (facade UI-navigation requests — a peer or plugin must never drive this machine's UI, hence host-classified) and `hub.peer.connected/disconnected` (open-by-decision). `plugins.tools` is classified **inert** in capspec (catalog read, not path-bearing).
- **`contracts/` fixtures now have a dead-contract guard** (2026-08-06): CI's per-stack jobs never enumerate `contracts/`, so a fixture with one loader (or two in the same language) used to pin nothing while green. `services/hub/cmd/brain/contracts_test.go` `TestEveryContractFixtureHasAtLeastTwoLoaders` walks the repo for loaders per fixture basename, requiring ≥2 files AND ≥2 languages, plus a two-way check against `contracts/README.md`'s owner table. The walk must keep skipping `.claude` (stale worktree checkouts hold old fixture/loader copies that fake a second loader) along with node_modules/target/dist/build/.git.
