---
title: The electronAPI backend seam: IPC / bridged / web / remote transport swap
tags: [renderer-state, electronAPI, hub-bus, web-workspacer, transport]
related_paths:
  - "apps/desktop/src/renderer/src/backend/install.ts"
  - "apps/desktop/src/renderer/src/backend/webBackend.ts"
  - "apps/desktop/src/renderer/src/backend/bridgedBackend.ts"
  - "apps/desktop/src/renderer/src/backend/remoteBackend.ts"
  - "apps/desktop/src/renderer/src/backend/hubBusClient.ts"
owner: Damien Touchette
last_reviewed: 2026-08-16
---

# The electronAPI backend seam: IPC / bridged / web / remote transport swap

## Overview
The entire React renderer talks to exactly one object, `window.electronAPI` (typed by `ElectronAPI` in `apps/desktop/src/renderer/src/types/electron.d.ts`), and never learns which transport backs it. `install.ts` runs in `main.tsx` before any component mounts and assigns one of four factory-built implementations to that global. This is what lets the same renderer bundle run as the Electron desktop app, as a plain browser client of the hub (web-workspacer), or as a remote-client shell — all driving/observing agents through identical calls.

## Key modules
- `apps/desktop/src/renderer/src/backend/install.ts` — `installBackend()` decides transport via `selectBackendMode()`; awaited before React renders.
- `apps/desktop/src/renderer/src/backend/webBackend.ts` — `createWebBackend(token, busUrl?)` builds the hub-bus-backed `ElectronAPI`; base implementation reused by bridged and remote modes.
- `apps/desktop/src/renderer/src/backend/bridgedBackend.ts` — `createBridgedBackend(ipc, token, busUrl)` wraps the web backend, delegating `LOCAL_TERMINAL` + `HOST_ONLY` method lists back to the preload IPC.
- `apps/desktop/src/renderer/src/backend/remoteBackend.ts` — `createRemoteBackend(ipc, token, busUrl)` wraps the web backend dialed at a remote hub, delegating only the small `REMOTE_HOST_ONLY` list (no local-terminal fallback — there is no local claudemon).
- `apps/desktop/src/renderer/src/backend/hubBusClient.ts` — `HubBusClient`, the raw WebSocket RPC/pub-sub client (`{op}` frames: call/result/error, subscribe/unsubscribe/event) shared by webBackend/bridgedBackend/remoteBackend; 15s call timeout (`CALL_TIMEOUT_MS`), 30s staleness detection, auto-reconnect with backoff, `onReconnect()` hook.
- `apps/desktop/src/main/services/hubCapabilities.ts` — main-process registry of what the hub RPC surface actually supports; webBackend's stub coverage is meant to track this.

## Failure modes
- `installBackend()` calls `ipc.getRemoteInfo()` through `getRemoteInfoWithRetry()` — bounded backoff `[50,150,300,600,1000]`ms (~2.1s, all before first paint), because the renderer's first paint can beat IPC handler registration ("No handler registered for…"). Only after every attempt fails does it keep the preload IPC backend, and it now logs `console.error` (not `warn`) naming the remote-client consequence explicitly. Retrying is not cosmetic: the renderer cannot distinguish "local install, IPC is correct" from "remote-client mode, IPC is dead" without this very answer, so a single failed ask used to strand remote users on a window that looked fine and did nothing.
- `selectBackendMode()` falls back to `'ipc'` whenever `info.busUrl` or `info.token` is missing, so a half-initialized hub degrades to IPC, not a broken bridged backend.
- `HubBusClient.call()` rejects with `hub call timeout: ${method}` after 15s if no `result`/`error` frame arrives; every `.call(...)` site in webBackend that doesn't `.catch(() => {})` will surface that rejection to its caller (many fire-and-forget paths like `writeTerminal`/`claudeWrite` swallow errors instead).
- After a bus reconnect, per-stream `sessions.attachTerminal` calls are NOT automatically re-issued by the bus's topic re-subscription — `webBackend.ts` compensates by registering every live PTY stream's re-attach thunk in a `reprimers` map and firing them all from `client.onReconnect()`; a stream that forgets to register here (or a new PTY-consuming method) will sit frozen after reconnect until a manual resize.
- Sparse vs rich snapshot races: `foldSparse()` in webBackend.ts merges `sparse: true` brain-provided snapshots onto the last rich desktop-provided snapshot per `sessionId` (dropped from cache on `status: 'ended'`); if a session's snapshots interleave in the wrong order or the cache map is bypassed, sparse data can wrongly present as fully-detailed.

## Gotchas
- **Three-plane split in `bridgedBackend.ts` is load-bearing.** Control/observation (message/approve/answer/snapshots/config/library/etc.) route over the bus so desktop and web drive agents identically. The PTY byte lifecycle (`createTerminal`/`spawnClaude`/`attach*`/`detach*`/`*Output`/`*Write`/`*Resize`/`*Close`, listed verbatim in `LOCAL_TERMINAL`) MUST stay on the preload IPC/MessagePort — splitting create/attach from write/resize/close across transports orphans the port. On web (no MessagePort across a wire) the same bytes cross as base64-framed `pty.bytes.<sessionId>` bus events (`decodePtyChunk`/`streamPty` in webBackend.ts).
- **`HOST_ONLY` and `REMOTE_HOST_ONLY` are exhaustive `keyof ElectronAPI` maps that must be kept current.** Any new electronAPI method has to be explicitly classified (bus-mirrored plane, LOCAL_TERMINAL, or HOST_ONLY/REMOTE_HOST_ONLY) or it silently inherits whatever `createWebBackend`'s stub does — usually `warnOnce(...)` returning a safe default, tagged `HUB-TODO` in comments, tracked loosely against `hubCapabilities.ts`.
- **`selectBackendMode()` precedence is pure and order-sensitive**: `remoteClient?.busUrl` wins outright over everything else (checked first), because in remote-client mode main spawns no local daemons at all — neither bridged bus nor IPC data paths have anything local to talk to. Only after ruling out remote does it check the `WORKSPACER_DESKTOP_DIRECT=1` kill switch (`desktopBus === false`) and missing `busUrl`/`token`, both of which fall back to `'ipc'`.
- **`bridgedBackend` vs `remoteBackend` diverge specifically on the local-terminal slice**: bridged delegates `LOCAL_TERMINAL` to IPC (there IS a local claudemon); remote does NOT — remote's PTY bytes ride the bus as `pty.bytes.*` just like a plain web client, because sessions live on the remote host.
- **`api.platform = ipc.platform` override in both bridgedBackend and remoteBackend** — `createWebBackend` always sets `platform: 'web'`, which the UI uses to gate native-only chrome (e.g. Windows titlebar overlay); both wrappers restore the genuine host platform after spreading the web backend, or native chrome breaks on Windows desktop.
- **Token plumbing has two independent sources**: web/browser reads `?token=` query param cached into `sessionStorage` (`TOKEN_KEY = 'hubToken'`, `resolveToken()` in install.ts); desktop bridged/remote modes get the token from `ipc.getRemoteInfo()` instead and never touch `sessionStorage`.

## Hand-authored notes (2026-08-16) — webBackend is federation-aware

- `createWebBackend` now merges federated peer fleets itself (a browser client has no main-process `federationBridge.ts`): a module-scoped `sessionHub` map records which hub owns each session, fed by (a) envelope `hub` stamps on `agent.snapshot` events and (b) a peer-fleet seed on connect/`hub.peer.connected` (`federation.peers` then `hub:<peer>/sessions.snapshots`, rows stamped with `hub`). Every per-session call goes through a qualify helper (`hub:<hub>/<method>` for remote sessions, bare otherwise); `hub.peer.disconnected` marks that hub's snapshots as tombstones (`hubOffline`) instead of dropping them. A new per-session electronAPI method that bypasses the qualify helper silently acts on the wrong machine (no error — the local hub just has no such session). `federationPeers` is exposed on the API for the renderer's HubChip/Machine picker. `backendParity.test.ts` + `renderer/tests/federation.test.ts` pin the shape.
