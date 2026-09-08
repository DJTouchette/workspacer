---
title: Enterprise copy must distinguish configured desktop workflows from headless agent hosting
date: 2026-09-07
confidence: high
suggested_doc: fleet-manager
related_paths:
  - landing/enterprise.html
  - docs/features/fleet-workflows.md
  - apps/desktop/src/main/services/fleetWorkflowRuntime.ts
promoted: false
---

# Enterprise copy must distinguish configured desktop workflows from headless agent hosting

## Observation
The shipped workflow editor and fleetWorkflowRuntime implement per-project ordered workflows and pinned task definitions, but docs/features/fleet-workflows.md and FleetWorkflowsSection.tsx explicitly require the local desktop host. Headless agent hosting is not configured-workflow runtime parity. Landing enterprise copy also needs to distinguish local coordination from provider-bound prompts; the old index claim that code never leaves the machine was too broad.

## Recommendation
Keep enterprise claims scoped to individual desktop installations and configured provider access. Link buyers to landing/docs.html#fleet-workflows and validate runtime parity before advertising centralized or headless workflow execution.
