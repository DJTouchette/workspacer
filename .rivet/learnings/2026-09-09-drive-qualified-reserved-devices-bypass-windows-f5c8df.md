---
title: Drive-qualified reserved devices bypass Windows handoff pointer gate
date: 2026-09-09
confidence: high
suggested_doc: cross-provider-handoff
related_paths:
  - apps/desktop/src/main/services/managerReplacementArtifact.ts
  - apps/desktop/src/main/services/managerReplacementArtifact.windows.test.ts
promoted: false
---

# Drive-qualified reserved devices bypass Windows handoff pointer gate

## Observation
The Windows plainAbsolutePath predicate in managerReplacementArtifact.ts accepts drive-absolute checkpoint pointers containing reserved DOS device components such as C:\NUL\.workspacer\brief.md, C:\NUL.txt\.workspacer\brief.md, C:\PRN\..., C:\AUX\..., C:\COM1\..., and C:\LPT9\.... These paths then reach verifyPath and lstat before root authorization.

## Impact
The narrow Windows handoff security requirement requires device paths to be rejected before any candidate filesystem I/O. Namespace-prefix filtering does not cover reserved DOS device names, which Windows recognizes in every directory and also with extensions.

## Recommendation
Reject all Windows reserved-device components (including extension forms and COM/LPT reserved indices) during plain path validation, before verifyPath. Add mocked no-candidate-I/O tests and validate natively on Windows.
