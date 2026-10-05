---
title: Title ownership crosses desktop and native clients
date: 2026-10-05
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/services/hostAutoTitles.ts
  - apps/desktop/src/renderer/src/hooks/useAgentAutoTitle.ts
promoted: false
---

# Title ownership crosses desktop and native clients

## Observation
Rust autoTitle metadata must survive sparse Electron remote snapshots and suppress useAgentAutoTitle inference. Explicit Electron-host bus launches enroll before registration in a host journal, claiming the attempt before inference. The renderer adopts host results but human names win. Interrupted claimed attempts are skipped after restart to avoid duplicate paid calls.
