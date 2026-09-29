---
title: Workflow path identity differs between old desktop and headless ingress
date: 2026-09-29
author: codex
confidence: high
suggested_doc: fleet-manager
related_paths:
  - services/hub-rs/src/services/workflow_runtime.rs
  - services/hub-rs/src/services/task_store/project.rs
  - services/hub-rs/tests/agent_spawn.rs
  - services/hub-rs/tests/workflow_runtime.rs
promoted: false
---

# Workflow path identity differs between old desktop and headless ingress

## Observation
Desktop hubCapabilities resolves request.cwd and intents[].cwd before fleetWorkflowRequest. The old Go private-companion route forwards workflow params raw; its spawn normalizeCwd only trims before workflowSpawn checks task ownership, with filesystem resolution later. Existing headless task rows can therefore retain a symlink spelling even when manager metadata is canonical, or vice versa. A Rust coordinator that resolves before admission must normalize new ingress consistently and compare legacy project spellings only when they resolve to the same existing directory, without rewriting history. Exact owner-bound next/taskReferences reads must remain possible offline and must not grant the same exception to fresh admission.

## Impact
Canonicalizing only the spawn half breaks legitimate workflow dispatch; blindly canonicalizing every historical read also turns unavailable projects into unreadable task history.

## Recommendation
Test canonical and alias manager metadata against new and legacy task rows, symlink retarget rejection for canonically bound tasks, normalized manager-request intents, and offline reads with mutation/admission refusal when the selected object cannot be resolved.
