---
title: Windows intent file work needs event-loop isolation and matching RPC budgets
date: 2026-09-13
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/main/services/intentWindowsFiles.ts
  - apps/desktop/src/main/services/intentArtifactFiles.ts
  - apps/desktop/src/renderer/src/backend/desktopServices.ts
  - services/hub/internal/capspec/timeouts.go
promoted: false
---

# Windows intent file work needs event-loop isolation and matching RPC budgets

## Observation
The secure Windows helper uses synchronous PowerShell/.NET native operations. One helper permits30s, while knowledge promotion can read/write/verify several times and the renderer intent service used a15s default. Increasing only the renderer deadline cannot prevent owner-loop stalls or hub provider timeouts. Offload file-backed actions, avoid holding SQLite write transactions during helper I/O, and align client/hub deadlines with the bounded worker budget.
