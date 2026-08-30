---
title: Dirty-worktree ports must search sibling commits before applying a diff
date: 2026-08-28
author: Codex
confidence: high
suggested_doc: git-review
related_paths:
  - .rivet/context/domains/agent-spawn.md
  - apps/desktop/src/main/services/claudemonSessionClient.ts
  - services/claudemon/src/session/store.rs
  - services/hub/cmd/mcp/main.go
promoted: true
promoted_to: git-review
---

# Dirty-worktree ports must search sibling commits before applying a diff

## Observation
The workspacer-managed-provider-stall-detect checkout was named for its committed stall fix, but its 24-file working diff implemented spawn-carried first-message delivery. Its HEAD da5e8710 and the landed feature commit 827d6d33 were sibling commits with the same parent. At merge 55c84e1a, 19 of the 24 dirty files matched the stale working files byte-for-byte; the other five retained the behavior while adding federated version-skew fallback, bounded failure tracking, or unrelated pending-slot ownership hardening. Current master therefore has no stale-only behavior to port.

## Impact
Blindly applying the old diff would duplicate an existing feature and regress current safeguards and newer provider/template architecture. A dirty worktree's branch name and distance from master are not reliable descriptions of its working patch.

## Recommendation
For stale worktree recovery, inspect the dirty paths and vocabulary first, use git log -S or a commit-message search to find sibling implementations, then compare the stale working file hashes against the landing merge before attempting a port. Treat an already-landed superset as a successful recovery and commit only the audit record.
