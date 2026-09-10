---
title: Task reference rebases require field intent and row origins
date: 2026-09-09
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/renderer/src/lib/taskLinksDraft.ts
  - apps/desktop/src/renderer/src/components/TaskInspector.tsx
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
promoted: false
---

# Task reference rebases require field intent and row origins

## Observation
The host links edit is a whole-object replacement protected by task revision CAS. Advancing only the editor revision after a conflict discards concurrent links. Rebase the immutable base/draft/current field values and preserve original ticket IDs and reference labels through renames; otherwise a rename can lose a concurrent URL edit. Link audits are capped at 40 and record actor/time for set edits, not per-entry origins.

## Recommendation
Keep draft merges atomic, retain the original draft and revision on collisions, and run the EXTERNAL-1 Playwright regression. Display set-level history under Details without inventing per-link attribution.
