---
title: A dead manager's identity exists ONLY in Electron main memory — which is why the tombstone is in-memory too
date: 2026-08-23
author: session: dead-manager-tombstone
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/services/claudeSessionStore.ts
  - apps/desktop/src/main/services/hubCapabilities.ts
  - services/hub/internal/capspec/capspec.go
  - apps/desktop/src/renderer/src/lib/pluginPermissions.ts
promoted: true
---

# A dead manager's identity exists ONLY in Electron main memory — which is why the tombstone is in-memory too

## Observation
`isSupervisor`, `label` and `parentSessionId` are desktop-main-process facts and nothing else. Grepping services/claudemon/src for parent_session_id / is_supervisor returns NOTHING: the daemon's session model has no such fields, and the desktop sets all three from `setSpawnMeta` at spawn time (claudeSpawn.ts / managedSpawn.ts). The only other producer is the Go brain's own metaStore (cmd/brain/enrich.go), which is likewise process memory. So the fleet's whole parent/manager graph dies with the Electron process. That settles the "should a dead-manager tombstone be persisted to disk" question: after an app restart NO live row carries a parentSessionId at all, so there are no orphans to find and a persisted tombstone would name a dead manager nothing points at — it would answer a question nobody can ask. Persistence only becomes worth anything in the same change that makes PARENTAGE durable (a claudemon-side field, or a desktop sidecar rehydrated at boot).

Two testability facts found building on this: (1) a childless tombstone is INVISIBLE through the orphan read, because candidates are derived from the live children — a parent with none is simply absent — so a retention rule ("keep while it has live children") cannot be guarded through that read at all; the store needs a `managerTombstoneCount()` for the leak to be observable. Verified by mutation: disabling the childless prune left every candidate assertion green. (2) The RENDERER suite guards main-process capability registrations — apps/desktop/src/renderer/tests/pluginPermissions.test.ts "labels every capability the main process actually registers" parses hubCapabilities.ts, so registering a capability with no CAP_LABELS entry fails the renderer run while main stays green (as does capspec's TestDesktopCapabilitiesAllClassified on the Go side). Three suites in three languages must agree before a new capability is green.</observation>
<parameter name="impact">Anyone proposing "just persist the tombstone so it survives an app restart" is proposing a record with no readers. And anyone adding a capability to hubCapabilities.ts who only runs the main suite will look green locally and break the renderer + Go runs.</parameter>
<parameter name="recommendation">Keep dead-manager tombstones in claudeSessionStore memory, written at the single teardown path (evictNow, so SessionEnd-eviction and close_session both feed it), retained while something they parented is still live, capped as a backstop. If durable succession across an app restart is ever wanted, make parentSessionId/isSupervisor durable FIRST (claudemon or a boot-restored sidecar) — the tombstone follows that, not the other way round. New capability checklist: hubCapabilities.ts + capspec.go (inertMethods/unscopedByDecision) + pluginPermissions.ts CAP_LABELS.

## Disposition
Merged with 550869 (crashed-manager orphans) and 3c6d9d (agents.list shape) into one consolidated section in .rivet/context/domains/session-lifecycle.md (manager succession is memory-only, 2026-08-23).
