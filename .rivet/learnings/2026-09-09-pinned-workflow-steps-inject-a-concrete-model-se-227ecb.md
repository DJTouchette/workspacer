---
title: Pinned workflow steps inject a concrete model-selection-and-spawn instruction
date: 2026-09-09
confidence: medium
suggested_doc: workflow-subagent-watcher
related_paths:
  - apps/desktop/src/main/services/fleetWorkflowRuntime.ts
promoted: false
---

# Pinned workflow steps inject a concrete model-selection-and-spawn instruction

## Observation
The workflow runtime emits a Next instruction that requires select_model and spawn_agent using the step's explicit provider/model/effort/capability/decisionId, rather than a generic free-form continuation.

## Impact
Automatic task capture must create or link task-level workflow runs deliberately; it cannot safely rely on prose alone to produce the right dispatch attributes.

## Recommendation
When adding task intents, carry the originating task and workflow decision metadata into the authoritative dispatch path.
