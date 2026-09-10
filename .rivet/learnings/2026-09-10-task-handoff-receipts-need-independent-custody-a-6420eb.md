---
title: Task handoff receipts need independent custody and retry verification
date: 2026-09-10
confidence: high
suggested_doc: git-review
related_paths:
  - services/hub/cmd/brain/taskhandoff.go
  - apps/desktop/src/main/services/taskHandoff.ts
promoted: false
---

# Task handoff receipts need independent custody and retry verification

## Observation
Recovered handoff at 7716523b has a separate brain-owned receipt under task-handoffs. Remote terminal-message acknowledgment is not byte custody. Import retries encounter existing Git worktrees and materialized files; they must verify exact branch/common-dir/commit and checksums instead of deleting or recreating allocations. Local continuation must read the original desktop-owned returned receipt and durably claim it before launching.
