---
title: Renderer live-state hooks + context providers (electronAPI stream to React state)
tags: [renderer-state, hooks, react-context, hub-reconnect, ui-mode]
related_paths:
  - "apps/desktop/src/renderer/src/contexts/ConfigContext.tsx"
  - "apps/desktop/src/renderer/src/hooks/useHubReconnect.ts"
  - "apps/desktop/src/renderer/src/hooks/useLayoutSync.ts"
  - "apps/desktop/src/renderer/src/hooks/useUiMode.ts"
  - "apps/desktop/src/renderer/src/lib/uiMode.ts"
  - "apps/desktop/src/renderer/src/hooks/useSessionLifecycle.ts"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Renderer live-state hooks + context providers (electronAPI stream to React state)

## Overview
React consumers use `window.electronAPI`. Its backend can be preload IPC, the renderer-owned hub client in `webBackend.ts`, or the `bridgedBackend.ts`/`remoteBackend.ts` compositions that retain native host methods. The renderer can therefore connect directly to the hub while components use the same API. See [the backend seam](../domains/renderer-backend-seam.md). A small family of hooks/providers owns the job of mirroring that push stream into React state: `ConfigProvider` for config, `useLayoutSync` for the shared window-manager doc, `useSessionLifecycle` for session save/restore, and `useUiMode` for the one config-driven rendering lens. `useHubReconnect` is available to consumers that must rehydrate after a connection drop; it is not automatically used by every provider.

## Key modules
- `apps/desktop/src/renderer/src/contexts/ConfigContext.tsx` — `ConfigProvider` is the single owner of `config` state and the shared configuration read/save/reload API path; exposes `{ config, loaded, reload, save }` via `useConfigContext()`.
- `apps/desktop/src/renderer/src/hooks/useConfig.ts` — thin re-export (`useConfig()` → `useConfigContext()`); also re-exports `DEFAULT_CONFIG`/`DEFAULT_SHORTCUTS` from `configDefaults` and defines the full `Config` type tree (`UIConfig.mode`, `PanesConfig.viewLevel`, `claude.transport`, and agent preferences).
- `apps/desktop/src/renderer/src/hooks/configDefaults.ts` — the dependency leaf holding `DEFAULT_CONFIG`; imported directly by `ConfigContext.tsx` to avoid a cycle.
- `apps/desktop/src/renderer/src/hooks/useHubReconnect.ts` — fires a callback on subsequent `connected: true` notifications after the first of `electronAPI.onHubStatus`; a no-op on the first connect.
- `apps/desktop/src/renderer/src/hooks/useLayoutSync.ts` — hydrates/pushes the hub's shared `LayoutDoc` (agents/tabs/panes + active tab), debounced 250ms, with content-based echo suppression and last-writer-wins versioning; calls `useHubReconnect` to re-pull after a drop.
- `apps/desktop/src/renderer/src/hooks/useSessionLifecycle.ts` — session picker/auto-resume/save-on-interval(30s)/save-on-change(1s debounce)/quit-handshake (`onBeforeQuit` → `notifyQuitSaved`); the local-disk half of workspace persistence, separate from the hub layout doc.
- `apps/desktop/src/renderer/src/hooks/useUiMode.ts` — reads `config.ui.mode` via `useConfig()`, resolves it through `resolveUiMode`, returns `{ mode, manifest, setMode, toggle }`.
- `apps/desktop/src/renderer/src/lib/uiMode.ts` — `MODE_MANIFEST` (the `fleet`/`focus` flag table with `feed` and `fleetDeck`) and `resolveUiMode()` (default `'fleet'`).
- `apps/desktop/src/renderer/src/hooks/useSessionSnapshots.ts` — owns initial
  and reconnect pulls, live compact snapshot batches, decision-sensitive immediate
  flushes, and termination pruning. `App.tsx` consumes the hook instead of owning
  a second implementation of those maps.
- `apps/desktop/src/main/preload.ts` / `apps/desktop/src/renderer/src/backend/webBackend.ts` — the IPC and web `electronAPI` implementations used by the backend compositions; `onHubStatus` is defined in both (IPC `HUB_STATUS` channel vs. `client.onStatus`), which is what makes desktop and web behave identically at the hook level.

## Failure modes
- `ConfigProvider` keeps built-in defaults and sets `loaded: true` after an initial fetch failure, but logs the error and posts a visible ‘Settings could not be loaded’ warning. It subscribes to `onConfigChanged` for external writes. Saves send `minimalConfigPatch` so stale siblings are not resent; a rejected save warns and returns the prior snapshot rather than painting unpersisted settings.
- `useLayoutSync`'s initial `layoutGet()` can race a live `layout.changed` broadcast; it guards with `appliedVersionRef` so a stale read never regresses an already-applied newer version (see the `stale` check in the hydrate effect).
- `useLayoutSync` push failures drop the optimistic `lastSyncedRef` marker so the *next* local change naturally retries; a push failure does not itself retry.
- A rejected workspace save logs, warns once per run, returns false, and rolls back its optimistic hash so the same payload can retry. The 30-second save runs only while active/visible; changes also arm a one-second debounce, and quit waits for the save outcome. A failed restore deliberately disables saving for that run so empty fallback state cannot overwrite the unread layout.
- `useHubReconnect` is keyed purely on `onHubStatus`'s `connected` transitions; if a consumer's hydration callback itself throws or its promise rejects silently (as most do, via `.catch(() => {})`), there is no escalation — the UI just stays stale until another reconnect.

## Gotchas
- `ConfigContext.tsx` deliberately imports `DEFAULT_CONFIG` from `../hooks/configDefaults` (the leaf module), **not** from `../hooks/useConfig` — importing via `useConfig` would form an import cycle that Vite HMR can duplicate, breaking lazy-loaded panes with "useConfig must be used inside `<ConfigProvider>`". Do not "simplify" that import.
- `useHubReconnect` fires **only on 2nd+ connect, never the first** — callers must do their own initial fetch separately (see `useLayoutSync`'s hydrate effect and `App.tsx`'s `refreshSessionSnapshots` call at mount) and treat `useHubReconnect` purely as the re-sync path.
- The hub bus re-asserts topic *subscriptions* on reconnect but does **not** replay one-shot fetches (session list, layout doc, config) — anything fetched once at mount goes stale while the socket is down and must be explicitly re-pulled via `useHubReconnect`.
- Desktop bus mode can also disconnect or adopt/reconnect to an independently owned hub. Reconciliation is not web-only. The hook does not replay missed events; each consumer must fetch its own authoritative state.
- `useUiMode`/`lib/uiMode.ts` is the *only* seam between `config.ui.mode` and per-mode rendering — components must branch on `manifest` flags (`feed`, `fleetDeck`), never compare `mode === 'focus'` directly, or a new UI surface will silently ignore focus mode.
- Any hook subscribing to an `electronAPI.on*` push stream must both (a) return the unsubscribe from its effect cleanup and (b) prune any per-session map entries when a session ends (`status === 'ended'` in `App.tsx`'s `onClaudeSessionUpdate` handler) — full-transcript snapshots left un-pruned leak for the life of the app.
- `useLayoutSync`'s echo-breaker (`lastSyncedRef`) compares JSON *content*, not just version numbers — an incoming document identical to what was last sent is acknowledged (version bumped) but not re-applied, preventing a self-broadcast bounce loop.

Saved workspace files and the hub layout document are separate persistence
surfaces. `useLayoutSync` redacts bus tokens before publishing and compares the
normalized adopted layout for echo suppression. Changing local normalization
without updating that comparison can turn a read into an unintended write-back.
A versioned layout is not a transactional merge of simultaneous user edits.

## Compaction load limits

`apps/desktop/src/main/shared/compactClaudeSnapshot.ts` combines a 1,024-entry
wire-key memo with a WeakMap for reused producer objects. Keys include session
and hub; running tools bypass memoization. The weak cache prevents repeated
serialization when large fleets evict each other from the bounded wire cache,
while cloned IPC payloads still use wire keys. Synthetic `bench:agent-load`
timings are compaction evidence, not Electron frame times or provider startup
latency. Retain the large-fleet no-reserialization regression when changing it.
