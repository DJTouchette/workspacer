---
title: Hub templates must render after worktree allocation
date: 2026-09-06
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/services/hubCapabilities.ts
  - apps/desktop/src/main/lib/dispatchTemplate.ts
promoted: false
---

# Hub templates must render after worktree allocation

## Observation
Desktop agents.spawn resolves library templates against the requested project cwd but the execution cwd can change during worktree allocation. Automatic {{cwd}} must be rendered only after allocation while {{projectCwd}} retains the validated requested project path; templateParams must not override either host-owned value.

## Impact
Rendering before allocation can instruct an agent to inspect the primary checkout while it executes in a worktree.

## Recommendation
Keep lookup/authorization project-scoped, validate template parameters before allocation where possible, and render the final first message after allocation.
