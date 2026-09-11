---
title: Composed workflow dispatch must accept the host canonical cwd
date: 2026-09-10
confidence: high
suggested_doc: fleet-manager
related_paths:
  - services/hub/cmd/mcp/workflow_dispatch.go
  - apps/desktop/src/main/services/hubCapabilities.ts
promoted: false
---

# Composed workflow dispatch must accept the host canonical cwd

## Observation
fleetWorkflows.request canonicalizes cwd through assertPathAllowed before checking task ownership. A facade composition that requires returned plan.cwd to byte-equal the original caller cwd would reject legitimate symlink/platform spellings. The composed dispatcher binds taskId/stepId/revision, trusts the already-owned host canonical cwd, and uses that exact path for routing and spawn.
