---
title: Codex transport now resolves through one shared default (codex.transport, shipped 'stream')
date: 2026-08-27
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/lib/spawnTransport.ts
  - apps/desktop/src/main/services/managedSpawn.ts
  - services/hub/cmd/brain/handlers.go
  - services/claudemon/src/providers/codex.rs
promoted: false
---

# Codex transport now resolves through one shared default (codex.transport, shipped 'stream')

## Observation
Codex had no config transport at all: an absent `transport` key on the spawn-managed payload means "hybrid" to claudemon, which is indistinguishable from a dropped field. Four entry points each applied their own `?? 'pty'` for claude and nothing for codex. Now `config.codex.transport` (default 'stream') is resolved by `apps/desktop/src/main/lib/spawnTransport.ts` `resolveTransport()` inside `spawnManagedAgent` (THE choke point — IPC, hub bus, respawn, jobs, supervisor/manager all reach it) and by its Go twin `registry.transportDefault` in `services/hub/cmd/brain/handlers.go`. Both codex shapes are now STATED on the wire and in setSpawnMeta, including 'pty' on the Windows rollout hybrid.

## Impact
Two traps for anyone touching codex spawns: (1) forwarding only `transport === 'stream'` at a request-translation layer now SILENTLY OVERRIDES an explicit hybrid request, because an omitted key is re-resolved downstream to the configured default — forward both values or forward neither; (2) `spawnManagedAgent` gates the Windows rollout hybrid on `transport === 'pty'`, not on `process.platform === 'win32'`, so headless codex on Windows takes the managed app-server path. Its safety net is claudemon-side: `run_session` degrades a failed HEADLESS app-server to `run_rollout_fallback` (previously a hard error), resetting `store.set_transport(..., Transport::Pty)` so the pane grows its Term view, plus a DEGRADED_FROM_HEADLESS_NOTICE conversation item.

## Recommendation
When adding a provider with more than one session shape, add it to `TRANSPORT_FALLBACK` (TS) and `transportFallback` (Go) and resolve at the choke point rather than at each caller. To exercise a real codex session end-to-end without the desktop: run claudemon (`env -u WORKSPACER_PARENT_PID` — it otherwise inherits the live app's parent-watchdog pid and exits immediately), the hub, and the brain on spare ports with XDG_CONFIG_HOME pointed at a temp dir, then call `agents.spawn` over the bus with a `{op:'call',id,method,params}` frame on the native Node WebSocket.
