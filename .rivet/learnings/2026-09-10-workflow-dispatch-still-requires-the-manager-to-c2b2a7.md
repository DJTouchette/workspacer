---
title: Workflow dispatch still requires the manager to copy host-owned metadata across tools
date: 2026-09-10
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/fleetWorkflowRuntime.ts
  - services/hub/cmd/mcp/fleet_workflows.go
  - services/hub/cmd/mcp/main.go
promoted: false
---

# Workflow dispatch still requires the manager to copy host-owned metadata across tools

## Observation
After the context-efficiency pass, workflowInstructions still returns role, template inputs, task/step/parent/stage and predecessor dispatch IDs as prose and instructs separate select_model then spawn_agent calls. The pending inbox consolidates list/content reads, but there is not yet a composed workflow-step dispatch operation. A future composition should resolve eligibility/routing and bind spawn metadata server-side while retaining explicit conditional decisions, authorization, reservation and uncertain-admission handling.
