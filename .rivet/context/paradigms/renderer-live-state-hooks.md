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
last_reviewed: 2026-07-11
---

# Renderer live-state hooks + context providers (electronAPI stream to React state)

## Overview
The renderer never talks to claudemon/the hub directly — everything flows through `window.electronAPI`, a facade implemented twice (Electron IPC in `apps/desktop/src/main/preload.ts`, hub-bus polyfill in `apps/desktop/src/renderer/src/backend/webBackend.ts`) so desktop and browser/`/remote` tabs run identical React code. A small family of hooks/providers owns the job of mirroring that push stream into React state: `ConfigProvider` for config, `useLayoutSync` for the shared window-manager doc, `useSessionLifecycle` for session save/restore, and `useUiMode` for the one config-driven rendering lens. `useHubReconnect` is the shared primitive all of them use to detect "the socket was down and might have missed events."

## Key modules
- `apps/desktop/src/renderer/src/contexts/ConfigContext.tsx` — `ConfigProvider` is the single owner of `config` state and all `getConfig`/`saveConfig`/`reloadConfig` IPC; exposes `{ config, loaded, reload, save }` via `useConfigContext()`.
- `apps/desktop/src/renderer/src/hooks/useConfig.ts` — thin re-export (`useConfig()` → `useConfigContext()`); also re-exports `DEFAULT_CONFIG`/`DEFAULT_SHORTCUTS` from `configDefaults` and defines the full `Config` type tree (`UIConfig.mode`, `PanesConfig.viewLevel`, `claude.transport`, `supervisor`, etc.).
- `apps/desktop/src/renderer/src/hooks/configDefaults.ts` — the dependency leaf holding `DEFAULT_CONFIG`; imported directly by `ConfigContext.tsx` to avoid a cycle.
- `apps/desktop/src/renderer/src/hooks/useHubReconnect.ts` — fires a callback only on the 2nd+ `connected: true` transition of `electronAPI.onHubStatus`; a no-op on the first connect.
- `apps/desktop/src/renderer/src/hooks/useLayoutSync.ts` — hydrates/pushes the hub's shared `LayoutDoc` (agents/tabs/panes + active tab), debounced 250ms, with content-based echo suppression and last-writer-wins versioning; calls `useHubReconnect` to re-pull after a drop.
- `apps/desktop/src/renderer/src/hooks/useSessionLifecycle.ts` — session picker/auto-resume/save-on-interval(30s)/save-on-change(1s debounce)/quit-handshake (`onBeforeQuit` → `notifyQuitSaved`); the local-disk half of workspace persistence, separate from the hub layout doc.
- `apps/desktop/src/renderer/src/hooks/useUiMode.ts` — reads `config.ui.mode` via `useConfig()`, resolves it through `resolveUiMode`, returns `{ mode, manifest, setMode, toggle }`.
- `apps/desktop/src/renderer/src/lib/uiMode.ts` — `MODE_MANIFEST` (the `fleet`/`focus` flag table: `sidebar`, `inspectorRail`, `fleetDeck`, `attention`, `hubFooter`) and `resolveUiMode()` (default `'fleet'`).
- `apps/desktop/src/renderer/src/App.tsx` (`refreshSessionSnapshots`, ~line 469-492) — the canonical example of the pattern: fetches `getAllClaudeSessions()` on mount and re-fetches via `useHubReconnect`, and prunes `statusBySession`/`snapshotBySession` when `onClaudeSessionUpdate` reports `status === 'ended'`.
- `apps/desktop/src/main/preload.ts` / `apps/desktop/src/renderer/src/backend/webBackend.ts` — the two `electronAPI` implementations; `onHubStatus` is defined in both (IPC `HUB_STATUS` channel vs. `client.onStatus`), which is what makes desktop and web behave identically at the hook level.

## Failure modes
- `ConfigProvider`'s initial `getConfig()` swallows errors (`.catch(() => setLoaded(true))`) — a failed fetch silently leaves `config` at `DEFAULT_CONFIG` with `loaded: true`, no visible error surfaced to the user.
- `useLayoutSync`'s initial `layoutGet()` can race a live `layout.changed` broadcast; it guards with `appliedVersionRef` so a stale read never regresses an already-applied newer version (see the `stale` check in the hydrate effect).
- `useLayoutSync` push failures drop the optimistic `lastSyncedRef` marker so the *next* local change naturally retries; a push failure does not itself retry.
- `useSessionLifecycle.saveCurrentSession` failures are only `console.error`'d — no retry, no UI indication; relies on the 30s interval and 1s post-change debounce to eventually succeed.
- `useHubReconnect` is keyed purely on `onHubStatus`'s `connected` transitions; if a consumer's hydration callback itself throws or its promise rejects silently (as most do, via `.catch(() => {})`), there is no escalation — the UI just stays stale until another reconnect.

## Gotchas
- `ConfigContext.tsx` deliberately imports `DEFAULT_CONFIG` from `../hooks/configDefaults` (the leaf module), **not** from `../hooks/useConfig` — importing via `useConfig` would form an import cycle that Vite HMR can duplicate, breaking lazy-loaded panes with "useConfig must be used inside `<ConfigProvider>`". Do not "simplify" that import.
- `useHubReconnect` fires **only on 2nd+ connect, never the first** — callers must do their own initial fetch separately (see `useLayoutSync`'s hydrate effect and `App.tsx`'s `refreshSessionSnapshots` call at mount) and treat `useHubReconnect` purely as the re-sync path.
- The hub bus re-asserts topic *subscriptions* on reconnect but does **not** replay one-shot fetches (session list, layout doc, config) — anything fetched once at mount goes stale while the socket is down and must be explicitly re-pulled via `useHubReconnect`.
- On desktop, the hub connection stays up for the app's lifetime, so `useHubReconnect` callbacks are effectively dead code there; the entire mechanism exists for the web/`/remote` transport where a backgrounded tab's socket drops and reconnects.
- `useUiMode`/`lib/uiMode.ts` is the *only* seam between `config.ui.mode` and per-mode rendering — components must branch on `manifest` flags (`sidebar`, `inspectorRail`, `fleetDeck`, `attention`, `hubFooter`), never compare `mode === 'focus'` directly, or a new UI surface will silently ignore focus mode.
- Any hook subscribing to an `electronAPI.on*` push stream must both (a) return the unsubscribe from its effect cleanup and (b) prune any per-session map entries when a session ends (`status === 'ended'` in `App.tsx`'s `onClaudeSessionUpdate` handler) — full-transcript snapshots left un-pruned leak for the life of the app.
- `useLayoutSync`'s echo-breaker (`lastSyncedRef`) compares JSON *content*, not just version numbers — an incoming document identical to what was last sent is acknowledged (version bumped) but not re-applied, preventing a self-broadcast bounce loop.
