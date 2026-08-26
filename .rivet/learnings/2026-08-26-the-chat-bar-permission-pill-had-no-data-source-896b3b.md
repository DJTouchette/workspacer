---
title: The chat-bar permission pill had no data source on the remote path
date: 2026-08-26
confidence: high
related_paths:
  - apps/desktop/src/renderer/src/components/claude/ComposerControls.tsx
  - apps/desktop/src/renderer/src/lib/providerCaps.ts
  - apps/desktop/src/renderer/src/backend/webBackend.ts
  - services/hub/cmd/brain/enrich.go
  - services/hub/cmd/brain/handlers.go
promoted: false
---

# The chat-bar permission pill had no data source on the remote path

## Observation
ComposerControls reads the permission mode from exactly one place: `snapshot.livePermissionMode ?? snapshot.settings.permissionMode`. On the web/bus path neither field existed for a headless-node session — brain sparse rows (cmd/brain/enrich.go compatSnapshot) carried no `settings` block at all, and webBackend's `spawnClaude` discarded everything in the agents.spawn result except `sessionId`. `permissionModeLabel(provider, undefined)` then returned `caps.permissionModes[0].label` — 'Ask to approve' — so a remote full-access session (ae5688d5) displayed as the most cautious mode there is. The `escalationScrubbed` field the hub stamps on every spawn result was read by NOTHING in any client.</observation>
<parameter name="impact">Any new session field the composer pills read must be answered on BOTH snapshot producers or the remote client silently shows a default: the desktop's claudeSessionStore.setSpawnMeta settings block, and the brain's spawnMeta + enrichSnapshot overlay (the metaStore/noteLiveControl pattern). The spawn RESULT reaches only the caller, so it is never a substitute for the row.</impact>
<parameter name="recommendation">Fixed 2026-08-26: brain `metaStore.noteLaunch` records the launch truth (permission mode in the provider's own vocabulary, fullAccess, escalationScrubbed) and enrichSnapshot fills `settings.permissionMode`/`settings.bypassAvailable`/`escalationScrubbed` onto every row; webBackend's snapshot fold gained `noteLaunch` so the spawn result's `fullAccess` covers the pre-first-snapshot window; `permissionModeLabel` now returns 'Unknown' for an absent id instead of the provider default. `launchPermissionMode` is now a THREE-way twin (renderer lib/providerCaps.ts, cmd/brain/handlers.go, main/services/claudeSpawn.ts + managedSpawn.ts) — keep them in agreement or the same launch labels differently on a desktop row vs a headless one.</recommendation>
<parameter name="suggested_doc">renderer-backend-seam
