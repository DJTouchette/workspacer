---
title: IPC Boundary
tags: [ipc, electron, typed-boundary, registry, constants, channels, handler]
related_paths:
  - "apps/desktop/src/main/shared/ipcChannels.ts"
  - "apps/desktop/src/main/shared/ipcTypes.ts"
  - "apps/desktop/src/main/ipc.ts"
  - "apps/desktop/src/main/preload.ts"
  - "apps/desktop/src/renderer/src/types/electron.d.ts"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# IPC Boundary

## Overview
The IPC boundary is a typed request–response + push channel system between the Electron main process and renderer. Channel names are centralized in `ipcChannels.ts` as constants; shared payload types live in `ipcTypes.ts` (importable by both sides). Handlers are registered in `ipc.ts` using `ipcMain.handle()` or `ipcMain.on()`; the preload bridge in `preload.ts` exposes a `window.electronAPI` object that mirrors those handlers as TypeScript-typed functions, so components normally use the API rather than raw Electron channels. The selected renderer backend can replace that API with bus/composed implementations. Push updates (main → renderer) flow via `mainWindow.webContents.send()` with the same channel constants, coalescing session snapshots when needed.

## Key modules

`apps/desktop/src/main/shared/ipcChannels.ts` — Single source of truth: `IPC` object exporting channel name constants (strings like `'library:list'`, `'claude:spawn'`). Both main and renderer import from here.

`apps/desktop/src/main/shared/ipcTypes.ts` — Shared TypeScript interfaces (`GitStatus`, `AppConfig`, `ClaudeSessionSnapshot`, etc.). No imports of Electron or Node modules; safe for both tsc builds.

`apps/desktop/src/main/ipc.ts` — Handler registration file. Imports all service modules and registers handlers via `ipcMain.handle(IPC.CHANNEL_NAME, handler)` or `ipcMain.on(IPC.CHANNEL_NAME, handler)`. Pushes from services invoke `mainWindow.webContents.send(IPC.CHANNEL_NAME, payload)`.

`apps/desktop/src/main/preload.ts` — Uses `contextBridge.exposeInMainWorld('electronAPI', {...})` to expose request methods, fire-and-forget actions, subscriptions and host values. Request methods return promises; subscriptions return cleanup functions. Methods call `ipcRenderer.invoke()` for request–response or `ipcRenderer.on()` for subscriptions, always referencing `IPC` constants. Manages MessagePort pooling for terminal and Claude session byte streams.

`apps/desktop/src/renderer/src/types/electron.d.ts` — `ElectronAPI` interface, mirrors the shape of the preload's exposed object for renderer-side type checking. Some methods are optional or backend-dependent. A method once native-only may now have a shared headless service; classify it against the current backend registry rather than treating optionality as proof of desktop-only support.

## Failure modes

**Hardcoded channel strings** — `claudeSessionStore.ts` and `libraryService.ts` use hardcoded strings like `'claude-session:update'` and `'library:changed'` instead of `IPC` constants. If the constant is renamed in `ipcChannels.ts`, these pushes silently send to a non-existent channel (renderer never hears them). TypeScript catches only if you rename the constant; if you delete it, hardcoded references continue to fire invalid pushes.

**MessagePort ownership** — Preload caches ports by terminal ID or Claude viewer
key. Attached panes use their pane ID so several viewers of one session do not
share one delivery key. Receivers ignore absent `event.ports[0]`. `getPort()`
waits up to 10 seconds and removes timed-out waiters; output subscriptions catch
that rejection. Unsubscribing removes the listener without closing the cached
port, because writes and later subscribers still need it. Preserve these rules
when changing port delivery or cleanup; `preload.test.ts` exercises late delivery,
cancellation, and timeouts.

**Handler registration guard** — `registerIpcHandlers()` updates service window
references and the file-change sink before checking `ipcHandlersRegistered`.
The guard is set synchronously before registering handlers, so repeated window
creation does not register duplicate `ipcMain.handle()` callbacks. Existing
handler closures still need care when a new BrowserWindow replaces the old one.
Do not describe synchronous registration as a concurrent registration race.

**Config schema drift** — `AppConfig` in `ipcTypes.ts` is documented as "kept in sync manually" with the runtime shape in `configService.ts`. Type declarations do not validate incoming runtime data. Defaults have generation checks and selected save paths validate values; those mechanisms are not a complete generated IPC schema. Preserve explicit validators and shared types when extending a payload.

## Gotchas

**Two sync hazards** — ipcChannels.ts → ipcTypes.ts → ipc.ts handler signatures → preload.ts API surface must all agree. TypeScript catches key-lookup errors (e.g., `IPC.NONEXISTENT`), but *only if* you use the constant. Hardcoded channel strings bypass this entirely.

**Handler closure lifetime** — Handlers often close over services injected at registration time (e.g., `claudemonSessionClient`, `configService`). If a service is re-instantiated post-registration, handlers see the stale instance. No hot-reload of handlers; they persist until the next app restart.

**MessagePort delivery race** — A subscriber can mount before its port arrives,
or unmount while `getPort()` is pending. Keep the cancellation check in the
resolved callback and the timeout rejection handler. `writeTerminal()` checks
for a cached port and otherwise does nothing; it does not dereference null.

**Push coalescing** — `claudeSessionStore.pushUpdate()` coalesces updates over about 16 ms per session. `sendRendererSnapshot()` emits a compact background snapshot to all subscribers and a separate full detail snapshot only for watched sessions; preload reference-counts detail viewers. Local rows are also published to the hub, while federated rows are not republished as local. If the renderer subscribes *after* a snapshot was emitted, it misses the state update (no backfill on new subscribers). Fleet consumers hydrate through `getAllClaudeSessions()` and reconnect reconciliation; detail viewers use their separate watch/read path. See [session lifecycle](../domains/session-lifecycle.md).

**No type narrowing on "send" vs "invoke"** — Both `ipcRenderer.send()` and `ipcRenderer.invoke()` use the same `IPC` constants. If you accidentally register a handler as `ipcMain.on()` (fire-and-forget) but call it as `ipcRenderer.invoke()` (awaitable), the invoke has no matching request handler and rejects; the separate 10-second port timeout does not apply to invoke calls. No compile-time check because both are strings.
