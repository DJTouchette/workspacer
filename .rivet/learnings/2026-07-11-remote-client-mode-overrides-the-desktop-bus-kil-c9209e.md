---
title: Remote-client mode overrides the desktop-bus kill switch and has no IPC/PTY fallback
date: 2026-07-11
confidence: high
related_paths:
  - apps/desktop/src/renderer/src/backend/install.ts
  - apps/desktop/src/renderer/src/backend/remoteBackend.ts
  - apps/desktop/src/main/services/remoteServer.ts
promoted: true
---

# Remote-client mode overrides the desktop-bus kill switch and has no IPC/PTY fallback

## Observation
In apps/desktop/src/renderer/src/backend/install.ts, selectBackendMode() checks info.remoteClient.busUrl FIRST, before the WORKSPACER_DESKTOP_DIRECT kill switch (desktopBus===false). So when "Connect to remote server" is configured, remote mode wins even if the direct-IPC kill switch env var is set — there is no way to force pure-IPC while a remote server is configured short of disconnecting it. Additionally, remoteBackend.ts (createRemoteBackend) explicitly does NOT fall back host-shell IPC for PTY/terminal — comment says "the local-terminal slice must NOT fall back to the preload IPC here" because there is no local claudemon in this mode; PTY bytes must ride the bus as pty.bytes.* events like a real web client. Only a small allowlist (REMOTE_HOST_ONLY: setTitleBarOverlay, onBeforeQuit, onSystemNotice, openExternalUrl, openLogsFolder, getRemoteInfo, setRemoteServer, appRelaunch) stays on real Electron IPC; everything else — including terminal — goes over window.electronAPI = createWebBackend(token, remoteBusUrl).

## Impact
Anyone adding a new host-only IPC method (e.g. a new native-only feature) must remember to add it to REMOTE_HOST_ONLY in remoteBackend.ts or it silently breaks in remote-client mode (calls hit the bus backend, which won't have it). Anyone adding a new local-only fallback/kill-switch must check selectBackendMode ordering — remote-client always wins regardless of other flags.

## Recommendation
When adding new window.electronAPI methods that are inherently host-machine-specific (window chrome, native dialogs, filesystem paths local to the Electron process), add them to REMOTE_HOST_ONLY in apps/desktop/src/renderer/src/backend/remoteBackend.ts. When touching backend-mode selection logic, preserve remote-client's override priority — it is intentional (no local daemons exist to fall back to).

## Disposition
Not folded: already reflected in .rivet/context/domains/desktop-remote-client-mode.md (kill-switch override + REMOTE_HOST_ONLY gotchas, verbatim).
