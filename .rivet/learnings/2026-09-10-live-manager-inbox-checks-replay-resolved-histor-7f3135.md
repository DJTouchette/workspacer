---
title: Live manager inbox checks replay resolved history every turn
date: 2026-09-10
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/managerRequestService.ts
  - services/hub/cmd/mcp/manager_requests.go
promoted: false
---

# Live manager inbox checks replay resolved history every turn

## Observation
Read-only audit of the running manager conversation snapshot (459 items) found 22 list_manager_requests results totaling 375887 UTF-8 bytes, plus 16 get_manager_request calls. requestInbox returns all retained request metadata and all manager tasks each turn; taskDependencyState also repeatedly calls store.list inside the task projection. A pending-content view can eliminate repeated history and most list/get round trips without changing CAS resolution.
