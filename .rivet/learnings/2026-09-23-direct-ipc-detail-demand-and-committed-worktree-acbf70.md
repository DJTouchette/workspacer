---
title: Direct IPC detail demand and committed worktree dependency discovery reduce background cost
date: 2026-09-23
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/main/services/claudeSessionStore.ts
  - apps/desktop/src/renderer/src/hooks/useClaudeSession.ts
  - apps/desktop/src/main/services/worktreeService.ts
promoted: false
---

# Direct IPC detail demand and committed worktree dependency discovery reduce background cost

## Observation
Direct IPC now publishes compact global session snapshots and sends full detail only for sessions referenced by onClaudeSessionDetail viewers. Preload reference-counts multiple panes; main-frame navigation clears stale demand. The optional detail API must remain absent from bus/remote backends: a no-op stub would cause useClaudeSession to abandon their existing conversation fold. Hidden IPC fetches request compact snapshots. Session hook cancels pending hidden updates after ended/refresh and rejects stale async fetches; same-session/same-offset reconciliation preserves unchanged turn identities. Worktree linking now obtains candidate parent directories from git ls-tree HEAD rather than traversing ignored build artifacts, shares an inflight/30s cache per canonical repo, returns copies and revalidates source dependencies before linking. Raw discoverNodeModules remains available for non-Git fallback. Spawn timing logs must use stderr because headless desktop-host stdout is a JSON protocol.

## Impact
Background agents no longer pay full native IPC transcript delivery; repeated worktree launches share discovery while preserving clean-tree checks. Protocol and hidden-update regressions are covered by tests.

## Recommendation
Keep onClaudeSessionDetail optional and direct-only unless a real bus detail implementation replaces the existing fold. Maintain multiple-viewer, navigation-reset and ended/refresh race tests. Use discoverWorktreeNodeModules refresh:true after intentional dependency changes when immediate discovery is required; otherwise cache expires after30s. Preserve stderr operational logging.
