---
title: Intent Git evidence must resolve cwd from live local session
date: 2026-09-12
confidence: high
suggested_doc: git-review
related_paths:
  - apps/desktop/src/main/services/intentEvidence*.ts
  - apps/desktop/src/main/shared/intentEvidence.ts
promoted: false
---

# Intent Git evidence must resolve cwd from live local session

## Observation
IntentExecution.session.cwd originates in the attach/link request and is not sufficient filesystem-read authority. IntentEvidenceStore.capture instead matches the persisted sessionId + empty hub against the host-owned live snapshots and reads liveCwd/cwd there. Git's derived repository root may contain sibling subtrees outside that cwd, so per-file paths remain confined to the canonical execution cwd. FleetReviewStore's immutable clean-commit capture cannot serve general intent executions because its source requires an isolated registered worktree allocation with a baseline commit.

## Recommendation
Preserve host-only cwd resolution, qualified local identity, subtree confinement, immutable artifact digests, and distinct user verification when extending intent evidence. Never use projectRoot or persisted renderer session.cwd as a read path.
