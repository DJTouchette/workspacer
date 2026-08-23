---
title: IPC Boundary
tags: [ipc, electron, typed-boundary, registry, constants, channels]
related_paths:
  - "apps/desktop/src/main/shared/ipcChannels.ts"
  - "apps/desktop/src/main/shared/ipcTypes.ts"
  - "apps/desktop/src/main/ipc.ts"
  - "apps/desktop/src/main/preload.ts"
  - "apps/desktop/src/renderer/src/types/electron.d.ts"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# IPC Boundary

## Overview
The IPC boundary is a typed request–response + push channel system between the Electron main process and renderer. Channel names are centralized in `ipcChannels.ts` as constants; shared payload types live in `ipcTypes.ts` (importable by both sides). Handlers are registered in `ipc.ts` using `ipcMain.handle()` or `ipcMain.on()`; the preload bridge in `preload.ts` exposes a `window.electronAPI` object that mirrors those handlers as TypeScript-typed functions, so the renderer never touches raw channel strings. Push updates (main → renderer) flow via `mainWindow.webContents.send()` with the same channel constants, coalescing session snapshots when needed.

## Key modules

`apps/desktop/src/main/shared/ipcChannels.ts` — Single source of truth: `IPC` object exporting ~100 channel name constants (strings like `'library:list'`, `'claude:spawn'`). Both main and renderer import from here.

`apps/desktop/src/main/shared/ipcTypes.ts` — Shared TypeScript interfaces (`GitStatus`, `AppConfig`, `ClaudeSessionSnapshot`, etc.). No imports of Electron or Node modules; safe for both tsc builds.

`apps/desktop/src/main/ipc.ts` — ~950-line handler registration file. Imports all service modules and registers handlers via `ipcMain.handle(IPC.CHANNEL_NAME, handler)` or `ipcMain.on(IPC.CHANNEL_NAME, handler)`. Pushes from services invoke `mainWindow.webContents.send(IPC.CHANNEL_NAME, payload)`.

`apps/desktop/src/main/preload.ts` — ~650 lines. Uses `contextBridge.exposeInMainWorld('electronAPI', {...})` to bridge each handler into a typed Promise-returning method in the renderer. Methods call `ipcRenderer.invoke()` for request–response or `ipcRenderer.on()` for subscriptions, always referencing `IPC` constants. Manages MessagePort pooling for terminal and Claude session byte streams.

`apps/desktop/src/renderer/src/types/electron.d.ts` — `ElectronAPI` interface, mirrors the shape of the preload's exposed object for renderer-side type checking. Some methods are optional (desktop-only: `worktreeInfo`, `appRelaunch`, `tailscaleGetInfo`).

## Failure modes

**Hardcoded channel strings** — `claudeSessionStore.ts` (lines 869, 892) and `libraryService.ts` (line 452) use hardcoded strings like `'claude-session:update'` and `'library:changed'` instead of `IPC` constants. If the constant is renamed in `ipcChannels.ts`, these pushes silently send to a non-existent channel (renderer never hears them). TypeScript catches only if you rename the constant; if you delete it, hardcoded references continue to fire invalid pushes.

**MessagePort leaks** — Preload pools terminal and Claude session ports keyed by ID (lines 33–34, 69–90). If a session closes before the port arrives, or a duplicate ID is created, port leaks or spurious `(event as any).ports[0]` dereferences can deadlock the channel. Timeout is 10s (line 37), rejecting `getPort()` and leaving callers without output handlers.

**Handler registration guard** — Line 95–97 in `ipc.ts` returns early on re-registration (macOS dock 'activate' safety). But if a second `registerIpcHandlers()` call happens concurrently, the first handler wins; no de-duplication on individual channels means stale closures can outlive the intended service.

**Config schema drift** — `AppConfig` in `ipcTypes.ts` is documented as "kept in sync manually" with the runtime shape in `configService.ts`. No codegen or schema validation; manual updates can lose fields or type mismatches silently.

## Gotchas

**Two sync hazards** — ipcChannels.ts → ipcTypes.ts → ipc.ts handler signatures → preload.ts API surface must all agree. TypeScript catches key-lookup errors (e.g., `IPC.NONEXISTENT`), but *only if* you use the constant. Hardcoded channel strings bypass this entirely.

**Exhaustive handler closure pattern** — Handlers often close over services injected at registration time (e.g., `claudemonSessionClient`, `configService`). If a service is re-instantiated post-registration, handlers see the stale instance. No hot-reload of handlers; they persist until the next app restart.

**MessagePort delivery race** — Preload's `getPort()` (line 69) waits up to 10s for a port to arrive via IPC push. If the main process never sends the port (e.g., session spawn failed, or the channel constant was renamed), the promise rejects with a generic timeout. Renderer code that ignores this rejection can still try to `postMessage()` on a null port, silently failing.

**Push coalescing** — `claudeSessionStore.pushUpdate()` (line 864 ff.) coalesces snapshot updates every ~16ms into a single IPC send per session. If the renderer subscribes *after* a snapshot was emitted, it misses the state update (no backfill on new subscribers). Clients must call `getAllClaudeSessions()` on mount to prime the cache.

**No type narrowing on "send" vs "invoke"** — Both `ipcRenderer.send()` and `ipcRenderer.invoke()` use the same `IPC` constants. If you accidentally register a handler as `ipcMain.on()` (fire-and-forget) but call it as `ipcRenderer.invoke()` (awaitable), the renderer hangs until timeout. No compile-time check because both are strings.
