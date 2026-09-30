---
title: Persisted import controls must consume bytes from the independent retained writer
date: 2026-09-30
confidence: high
related_paths:
  - services/hub-rs/tests/persisted_ts_imports.rs
  - services/hub-rs/assets/persisted-ts
  - apps/desktop/src/main/services/persistedWriterCapture.test.ts
promoted: false
---

# Persisted import controls must consume bytes from the independent retained writer

## Observation
Prep now captures exact disk bytes from real TypeScript DispatchHistoryStore, FleetReviewStore and ManagerReplacementState operations. The review fixture comes from real Git commits and remains readable after deleting its isolated repo/worktree. Generator does not normalize UUID/time/path data; manifest records variable provenance plus6 source hashes and3 artifact hashes. Rust persisted_ts_imports uses actual owning stores and checks read-time preservation, stale/not-live projection, owner refusal, authorized rewrite/reopen and uncertain delivery recovery. TS3, source-check mutations4 and main tsc passed; Rust target intentionally awaits branch CI, no local execution claimed.

## Recommendation
Keep current-writer provenance separate from historical user-installation claims. Preserve LF attributes for every hashed source/artifact; do not synthesize expected bytes with Rust or edit captures to force acceptance.
