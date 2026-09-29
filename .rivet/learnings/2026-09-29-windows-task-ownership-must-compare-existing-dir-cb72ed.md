---
title: Windows task ownership must compare existing directory identity across stored spellings
date: 2026-09-29
author: codex
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/hub-rs/src/services/task_store/project.rs
  - services/hub-rs/tests/agent_spawn.rs
  - services/hub-rs/tests/task_store.rs
promoted: false
---

# Windows task ownership must compare existing directory identity across stored spellings

## Observation
Workflow start persists the caller cwd, while SpawnCoordinator resolves Windows paths to OS canonical verbatim spelling before admission. After shared canonicalization began normalizing DOS/case/native spellings, five real workflow admission tests failed because TaskStore compared raw strings. Exact historical strings remain valid; alternate Windows spellings now require two existing absolute directories with equal results from the same link-before-parent resolver. Owner IDs, CAS revisions and project separation remain checked independently. The remote dispatch fixture separately advertised a noncanonical choice despite a canonical-choice protocol and was corrected rather than relaxing that protocol.

## Impact
Fixing path canonicalization at filesystem boundaries can expose persisted logical identity comparisons that previously agreed only by spelling.

## Recommendation
Use the shared task project comparator consistently for reads, mutation fences, dependencies and admission/source linkage. Do not replace remote lease canonical equality with case-folding or prefix stripping.
