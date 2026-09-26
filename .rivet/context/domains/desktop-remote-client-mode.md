---
title: Desktop Remote-Client Mode
tags: [remote, desktop, electron, hub-bus, tailscale, client-mode]
related_paths:
  - "apps/desktop/src/main/services/remoteServer.ts"
  - "apps/desktop/src/main/services/remoteServer.test.ts"
  - "apps/desktop/src/main/services/tailscaleServe.ts"
  - "apps/desktop/src/main/index.ts"
  - "apps/desktop/src/main/ipc.ts"
  - "apps/desktop/src/main/preload.ts"
  - "apps/desktop/src/main/services/hubDaemon.ts"
  - "apps/desktop/src/renderer/src/backend/install.ts"
  - "apps/desktop/src/renderer/src/backend/install.test.ts"
  - "apps/desktop/src/renderer/src/backend/remoteBackend.ts"
  - "apps/desktop/src/renderer/src/backend/webBackend.ts"
  - "apps/desktop/src/renderer/src/components/RemoteShareDialog.tsx"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Desktop remote client and worker pairing

## Startup ownership

`apps/desktop/src/main/services/remoteServer.ts` stores connection settings in
`remote-server.json` under the config directory. Main reads this before its
local daemon startup branch. Client mode skips claudemon, initialization,
bridges, hub, local hub client and MCP facade startup. The Electron renderer
then drives the selected server through the web bus backend.

`mode: workers` is different: `getRemoteServer()` returns null, so local startup
and manager ownership remain. `getPairedWorkerTarget()` provides host-only
credentials for explicit worker dispatch; `getPairedWorkerInfo()` exposes just
the HTTP URL. Legacy absent mode is treated as client mode.

The endpoint may be on this same machine. Mode specifies ownership and routing,
not physical remoteness. Adopting already-running local daemons also differs
from selecting client mode: adopted resources remain local data services even
though this Electron instance did not spawn them.

## Persistence and URL normalization

The sidecar lives outside the two-writer `config.yaml` system and is written
atomically with requested POSIX mode 0600. Do not describe that mode as a Windows
ACL guarantee. Main caches the resolved client setting until a setter call.
Missing, corrupt or unreadable input resolves to local mode rather than aborting.

`normalizeRemoteServerUrl()` accepts bare hosts (hub port 7895), explicit ports,
and HTTP/HTTPS/WS/WSS addresses. Explicit schemes without ports keep their
scheme default. It normalizes IPv6 brackets and constructs an origin HTTP URL
and `/bus` URL; pasted paths are not preserved as reverse-proxy prefixes.
Other schemes, empty input and malformed URLs are rejected. URL validation does
not verify reachability or token authority.

Saving an empty token can reuse the existing credential only for the same
normalized bus endpoint. It must not transfer the old credential to a changed
endpoint. Clearing is best-effort and logs removal errors; a successful IPC
response is not proof that an undeletable sidecar was removed.

## Renderer transport and recovery controls

`apps/desktop/src/renderer/src/backend/install.ts` selects remote mode before
the local direct-mode switch or missing local bus credentials. Workers-only
pairing does not change this selection. `getRemoteInfo` is attempted once;
on failure it warns and keeps preload IPC, which is not a useful local data
fallback when main deliberately started no services.

`remoteBackend.ts` creates the web backend with the selected URL/token and
restores only a small local shell API: titlebar, quit/system notices, external
links/logs, connection settings and relaunch. PTY bytes travel over the remote
bus with no local-terminal fallback. The platform value is the host OS for
native chrome; it does not promise host-local filesystem or data services.
See [backend seam](renderer-backend-seam.md) before classifying a new method.

`apps/desktop/src/renderer/src/components/RemoteShareDialog.tsx` currently uses
Phone, Machines and Server tabs. Client mode shows only Server. The Server tab
requires the local `setRemoteServer` method, so ordinary web clients cannot
persist this Electron setting. New connections default to workers-only, with
a separate full-client option. Existing clients can disconnect or switch to
workers-only while keeping the credential for the same endpoint.

Apply calls the setter, displays returned errors, then requests relaunch.
Changes take effect at startup; there is no live ownership switch. If relaunch
is unavailable or fails, a manual restart is needed. The old `initialSection`
prop and string-valued `showRemote` description are obsolete: current App uses
a boolean and this dialog owns its tabs.

## Offline behavior

Connection recovery controls must stay host-local even when remote config
calls fail. App keeps `welcomeDismissedLocally` so dismissing onboarding does
not require a successful remote config write. The persisted dismissal is still
attempted; it is not the only way to unblock the current UI session.

Worker/node action affordances must consider authoritative state before local
pending flags, including stopping/draining states. Keep cross-client parity
checks with `remoteNodes.test.ts`; a readable status label alone does not make
an enabled action appropriate.

`tailscaleServe.ts` is the separate host-sharing mechanism that fronts a local
hub. It does not itself select the remote-client backend. See
[remote/mobile](remote-mobile.md) for token, PWA and host-sharing contracts.

## Validation scope

URL/persistence and backend-selection tests passed (10 and 5 respectively).
This review checks the startup/UI source and renderer behavior; it does not
claim a fresh live Electron relaunch or Tailscale connection on every OS.
