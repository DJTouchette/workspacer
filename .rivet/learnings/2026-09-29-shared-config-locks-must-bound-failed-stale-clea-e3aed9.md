---
title: Shared config locks must bound failed stale cleanup and treat records as diagnostic
date: 2026-09-29
confidence: high
suggested_doc: config
related_paths:
  - services/hub-rs/src/services/config.rs
  - apps/desktop/src/main/lib/fileLock.ts
promoted: false
---

# Shared config locks must bound failed stale cleanup and treat records as diagnostic

## Observation
Fault-injected desktop fileLock regression proved stale rmSync failure retried before checking its deadline, spinning forever. Moving continue inside successful cleanup bounds the wait; 442 selected TS tests pass. Rust ConfigLock successful O_EXCL acquisition must survive diagnostic PID record failure like Go/TS, otherwise an orphan lock remains. Append lock suffix to OsString to preserve non-UTF8 host paths. Rust targeted tests cover these and unreadable-source/write-failure recovery.
