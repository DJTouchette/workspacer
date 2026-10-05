---
title: Native child Clear depends on bounded live-first projection and current work evidence
date: 2026-10-05
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/native/src/child_agents.rs
  - apps/native/src/model.rs
  - apps/native/src/controller.rs
  - apps/native/src/ui/sidebar.rs
promoted: false
---

# Native child Clear depends on bounded live-first projection and current work evidence

## Observation
The newest32 child slice can evict an old running/approval child. Keep retained unfinished children first, then other unfinished rows, then recent terminal rows, preserving source order and visibly reporting the32-row limit. Clear must revalidate the clicked mark against current terminal/work state, include pending child approvals in parent liveness, and retire the mark on newer finish/call evidence. Pending fleet reads must overlay parsed tool counts as well as subagents/workflows, or later Clear can falsely resurrect after a9-to2 counter rollback. Activity-only/provider tool-count backfill is not a new run. Raw screenshot-region hashes, not visual impressions or debug bounds alone, confirmed stable real row painting across focus.
