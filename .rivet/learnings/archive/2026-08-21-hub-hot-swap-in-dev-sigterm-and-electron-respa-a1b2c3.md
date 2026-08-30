---
title: Hot-swapping the hub binary in a dev build = SIGTERM it; Electron main respawns it
date: 2026-08-21
confidence: high
related_paths:
  - apps/desktop/src/main/services/hubDaemon.ts
  - apps/desktop/src/main/lib/daemonUtils.ts
  - services/hub/cmd/hub/remote.go
  - services/hub/cmd/hub/main.go
  - services/hub/internal/busclient/client.go
promoted: true
promoted_to: hub-process-supervision
---

# Hot-swapping the hub binary in a dev build = SIGTERM it; Electron main respawns it

## Observation
In a `npm run dev` desktop build, the hub is an app-OWNED child of Electron main. hubDaemon.ts
`launch()` registers `child.on('exit') → scheduleRestart(bin)`, guarded only by `intentionalStop`
(set by `stopHub()`/quit). `scheduleRestart` uses `RestartBackoff` (daemonUtils.ts: base 1s, ×2,
cap 30s, 10 attempts / 60s reset) and calls `launch(bin)` again, which re-reads the binary from
disk and RECONSTRUCTS argv from config — so a plain `kill -TERM <hubpid>` swaps a rebuilt
`services/hub/hub` in with byte-identical flags. Measured live: new listener on :7895 ~3s later.
Process-tree facts that make this safe: `claudemon` (:7890/:7891) and the `mcp` bridge (:7897)
are SIBLINGS of the hub (both children of Electron main), not children — agent sessions are
untouched, and the mcp bridge reconnects itself via `internal/busclient` (200ms→5s backoff,
subscriptions re-sent). Only the hub's own children die and get respawned: the `brain` provider
and the bwrap plugin sidecars (`--die-with-parent`). This is the ONLY way to ship a `/m` PWA
change: mobile.html/sw.js/manifest/icons are `go:embed`-ed in cmd/hub/remote.go with no dev-mode
disk fallback, so a running hub serves its build-time copy forever.

## Impact
Anyone who rebuilds the hub and then refreshes /m sees the OLD PWA and concludes the build
didn't take. Conversely, anyone who fears killing the hub will disturb running agents or
permanently drop the Fleet Manager's MCP control plane is wrong on both counts — but only for
the app-owned hub. If `workspacer serve` started it, `startHub()` ADOPTED it (health probe wins
before the spawn path) and there is no supervision: killing that hub leaves nothing to restart it.

## Recommendation
Check the hub's PPID first — PPID == Electron main ⇒ app-owned ⇒ `kill -TERM` and wait ~3s.
Verify the swap, don't assume it: `md5sum /proc/<newpid>/exe services/hub/hub` must match (the
stale process's `/proc/<pid>/exe` reads `(deleted)`), then
`diff <(curl -s http://127.0.0.1:7895/m) <(git show HEAD:services/hub/cmd/hub/mobile.html)`
should be empty. PWA assets live at ROOT paths (`/manifest.webmanifest`, `/sw.js`,
`/icon-192.png`), not under `/m/`. Bus smoke test frames use `op`, not `type`:
`{"op":"call","id":"1","method":"agents.list"}`.
