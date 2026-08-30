---
title: Session ids are reused on restart; teardown must prove it still owns the generation
date: 2026-07-31
promoted: true
---

# Session ids are reused on restart; teardown must prove it still owns the generation

## Observation
Fixed 2026-07-31. Every restart path pins the id it replaces (desktop useClaudeSpawn.restartSession / useAgentManager.respawnAgentWithSettings, TUI bus.rs, brain handlers.go) and every close is fire-and-forget: terminate_managed only drops the input sender, POST /signal only sends SIGTERM. So the dying life's teardown routinely runs AFTER its successor registered under the same id. Three teardown paths keyed on the id alone and clobbered the successor: claudemon reap_pty (removed the successor's PtyHandle and SIGKILLed it), deregister_managed called from all four managed driver tails (wiped its channels, emptied the conversation, marked Stopped, broadcast SessionEnd), and the desktop's 30s SessionEnd eviction (took label, parentSessionId, isSupervisor, usage). Trigger for all three is the routine composer model / permission-mode switch. Fix: SessionStore now holds a monotonic generations DashMap; claim_generation() at every spawn path, owns_generation() checked by release_spawn, drop_pending_spawn and deregister_managed. An UNCLAIMED id answers owns_generation=true on purpose - defaulting the other way would skip teardown and leak. reap_pty became reap_pty_owned(id, &Arc) using remove_if + Arc::ptr_eq rather than a generation, because the reader already holds its Arc and must reap its own child even when superseded. Desktop side holds the eviction timer in evictionTimers so a restart cancels it (from handleHookEvent for any non-SessionEnd hook, from createSession, and from setSpawnMeta which is the earliest point in a restart), plus a status!=='ended' re-check inside the callback. The primitive already existed twice in store.rs and was never generalised: release_spawn_plumbing's cwd-slot guard and unregister_managed_answer_if's remove_if(same_channel). Found by a 3-round adversarial review workflow; no close-then-respawn test existed in any stack.

## Disposition
Folded into .rivet/context/domains/session-lifecycle.md (generation-ownership note).
