---
title: Empty intent Git artifact means no tracked path survived selection
date: 2026-09-13
confidence: high
suggested_doc: git-review
related_paths:
  - apps/desktop/src/main/services/intentEvidenceCapture.ts
  - apps/desktop/src/main/services/intentWindowsFiles.test.ts
promoted: false
---

# Empty intent Git artifact means no tracked path survived selection

## Observation
captureIntentGit returns exactly an empty artifact only when its selected path array is empty. Once any tracked path survives, the artifact includes staged/unstaged section headings even if both git diff outputs are empty. Therefore the real Windows CI failure expected '+after' but got '' must be isolated status enumeration or confinement/secret-path selection, not merely autocrlf diff rendering. Windows diagnostics now include original porcelain status, capture omissions, input/Git/canonical paths and individual confinement results so the next native runner identifies the boundary without speculative broadening.
