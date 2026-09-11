---
title: Windows handoff paths need native ACLs and persistent private Git settings
date: 2026-09-10
confidence: high
related_paths:
  - services/hub/internal/taskartifacts/privacy_windows.go
  - services/hub/internal/taskartifacts/git.go
  - services/hub/cmd/brain/taskhandoff_crossplatform_test.go
promoted: false
---

# Windows handoff paths need native ACLs and persistent private Git settings

## Observation
Native Windows fixtures exposed two differences hidden by Linux tests: short temp paths require native volume/file identity comparisons, and 64-character transfer IDs can make ordinary Git HEAD lookup fail after successful host import unless core.longpaths is persisted in the generated repository. Real DACL inspection rejects an Everyone grant even when os mode says 0600. Hosted Windows source/Linux worker/Windows return and local implementation passed after these corrections. Source status must honor safe core.autocrlf semantics instead of misclassifying normal CRLF checkout conversion as WIP.
