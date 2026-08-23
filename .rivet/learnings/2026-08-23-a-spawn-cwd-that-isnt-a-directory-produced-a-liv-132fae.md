---
title: A spawn cwd that isn't a directory produced a live-looking card whose session was already stopped
date: 2026-08-23
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/lib/spawnCwd.ts
  - apps/desktop/src/main/services/claudeSpawn.ts
  - apps/desktop/src/main/services/managedSpawn.ts
  - apps/desktop/src/renderer/src/lib/fleetManager.ts
  - services/claudemon/src/providers/claude_stream.rs
  - services/claudemon/src/daemon/spawn.rs
promoted: false
---

# A spawn cwd that isn't a directory produced a live-looking card whose session was already stopped

## Observation
claudemon's POST /sessions/spawn-managed registers the session id and answers 200 BEFORE the provider child launches (providers/claude_stream.rs `spawn_session` drives `run_session` in a background task and only logs a warn on failure, then `deregister_managed` flips the row to Stopped). So a cwd that no process can chdir into does not fail the spawn — it returns a session id, the desktop mints a normal agent card, and every subsequent POST /sessions/:id/message answers `409 session has ended and cannot accept chat input`. To the user that reads as "the agent opened, it's stuck on a new/empty session, and nothing I type goes through". Verified live against the running daemon with cwd "~".

How a "~" reaches that layer: main/lib/spawnCwd.ts `normalizeSpawnCwd` deliberately does NOT expand a leading tilde (BINDING DECISION 1 — it's the twin of the brain's normalizeCwd, pinned by contracts/path-containment-cases.json), and commit 632562b3 (2026-08-07 hardening round) replaced claudeSpawn's old `fs.existsSync(cwd) ? cwd : $HOME` fallback with it. Correct for bus-supplied paths, but nothing replaced the user-visible failure — and `agents.fleetRoot` is a free-text Settings field where "~/" is exactly what a person types, so every Fleet Manager spawned into a directory literally named "~".

## Impact
Any spawn path (IPC claude:spawn, hub-bus agents.spawn, MCP-facade worker dispatch) with a stale, typo'd or tilde-spelled cwd silently produced a dead agent with no error anywhere except a claudemon warn line. It is also why the Fleet Manager looked "broken" with nothing in the logs to point at.

## Recommendation
Two-layer fix now in place: (1) tilde expansion belongs at the point the USER's config is read, not at the spawn boundary — renderer/src/lib/fleetManager.ts `expandHome` inside deriveFleetRoot; leave normalizeSpawnCwd's contract alone. (2) `assertSpawnCwd` (main/lib/spawnCwd.ts) pre-flights the resolved cwd in spawnClaudeAgent, spawnManagedAgent and spawnCodexHybrid, raising a notifySystem banner and throwing before a session id exists. When adding a new spawn entry point, call it after cwd resolution. The remaining gap: claudemon itself still accepts an unusable cwd — a 400 in handle_managed/handle would be the backstop for non-desktop clients.
