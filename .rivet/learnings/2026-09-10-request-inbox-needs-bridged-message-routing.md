---
title: Request inbox preparation and tagged delivery both need the desktop IPC route
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/renderer/src/backend/bridgedBackend.ts
  - apps/desktop/src/renderer/src/backend/webBackend.ts
  - apps/desktop/src/renderer/tests/backend/backendParity.test.ts
  - apps/desktop/src/main/ipc.test.ts
promoted: false
---

HOST_ONLY must include managerRequestPrepare, but that alone does not fix default
desktop capture: ordinary claudeMessage uses the bus, whose generic send contract
has no request ID. Route only request-tagged sends through the compatible local
preload, preserving ordinary bus messages. Missing/old preload and web/remote
must refuse a tagged send rather than silently discard its identity. IPC failure
or ambiguous acknowledgement must never fall back to a second bus send.

The transport regression installs the actual default backend over the real
preload and IPC handlers, then checks a private host request ledger. Tests that
assign a preload-shaped window.electronAPI directly cannot verify this seam.
