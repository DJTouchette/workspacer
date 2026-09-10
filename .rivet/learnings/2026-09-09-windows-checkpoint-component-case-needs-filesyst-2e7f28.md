---
title: Windows checkpoint component case needs filesystem identity and a spelling boundary
date: 2026-09-09
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/managerReplacementArtifact.ts
promoted: false
---

# Windows checkpoint component case needs filesystem identity and a spelling boundary

## Observation
The drive/separator-only handoff fix still rejects valid authored checkpoint pointers when their component case differs from raw fleet, worker metadata or project roots. The recovered fix walks all components without links and compares nonzero BigInt device/inode identities for both root and brief, then fstats the opened host-known file. Identity alone also recognizes short-name aliases; a separate case/separator candidate-spelling comparison is required to avoid expanding that boundary. This supersedes the component-case restriction recommended in 2026-09-09-manager-handoff-checkpoint-errors-conflate-windo-7076c2.

## Impact
Blanket lowercasing can authorize distinct case-sensitive Windows directories, while byte-exact comparison falsely denies ordinary case-insensitive Windows paths. Linux Win32 fixtures cannot establish native Windows runtime success.

## Recommendation
Keep both spelling and filesystem-identity checks, no-link walks, exact buffer hashes and safe index/category errors. Run the artifact suites in containment-windows before delivery and obtain a redacted failure category and digest-match result if the remote failure remains.
