---
title: Intent execution observations are host-owned snapshots, not guaranteed archived results
date: 2026-09-12
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/services/intentWorkspaceStore.ts
  - apps/desktop/src/main/shared/intentWorkspace.ts
  - apps/desktop/src/renderer/src/components/IntentExecutions.tsx
  - docs/intent-workspaces.md
promoted: false
---

# Intent execution observations are host-owned snapshots, not guaranteed archived results

## Observation
The intent execution slice pins first-message packets and hub-qualified session links in SQLite schema v2. Execution reads capture current state and agent-reported summaries using host-supplied snapshots; request payloads cannot supply observations. hubOffline tombstones are skipped so a stale report is not stamped fresh. Capture currently runs on reads/debounced visible execution updates, so a session that finishes and disappears before a read can have no retained final result. Background lifecycle capture remains an explicit next milestone; do not describe these rows as a transcript archive or first-message delivery receipt.
