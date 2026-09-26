---
title: Documentation structure checks miss retired runtime guarantees
date: 2026-09-26
confidence: high
related_paths:
  - apps/desktop/src/renderer/src/components/ScrollContainer.tsx
  - apps/desktop/src/renderer/src/lib/paneMenu.ts
  - services/claudemon/src/daemon/api.rs
  - services/claudemon/src/daemon/hook.rs
promoted: false
---

# Documentation structure checks miss retired runtime guarantees

## Observation
The old Pane System guide claimed the rendering switch enforced exhaustiveness, but it has a runtime default; only icon/title Record maps enforce keys. The split menu now comes from paneMenu and PaneMenuContext. HTTP docs omitted hook-port Host/Origin guards and the session.resync lag event. Embedding runbooks incorrectly told Workspacer readers to commit a cache explicitly ignored by this repo.

## Recommendation
Pair Rivet lint and local-reference checks with executable source review and relevant tests; do not certify semantic accuracy from a clean lint run.
