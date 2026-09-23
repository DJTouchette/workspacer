---
title: Cleanup daemon client must be enumerated in the cross-stack route sweep
date: 2026-09-23
confidence: high
suggested_doc: claudemon-http-api
related_paths:
  - services/hub/internal/capspec/claudemoncallers_test.go
  - apps/desktop/src/main/services/worktreeArtifactCleanup.ts
promoted: false
---

# Cleanup daemon client must be enumerated in the cross-stack route sweep

## Observation
Hosted release CI caught TestEveryClaudemonCallerFileIsEnumerated after adding worktreeArtifactCleanupScheduler.ts: the scheduler contains PORTS.claudemonApi but was absent from the caller/non-caller registry. Actual paths live in worktreeArtifactCleanup.ts using an injected daemonUrl. The fix declares the core as a caller with a two-route extraction floor, the scheduler as a documented non-caller delegating paths, and loadWorktreeCleanupState as a discovery marker. Local package-focused brain checks did not cover internal/capspec, whose tests scan TypeScript consumers.

## Impact
Both Linux hub and Windows hub tests correctly block release until new daemon clients join the cross-stack HTTP contract sweep.

## Recommendation
When adding any claudemon HTTP consumer, run go test -count=1 ./internal/capspec and update extraction plus enumeration rather than bypassing the guard.
