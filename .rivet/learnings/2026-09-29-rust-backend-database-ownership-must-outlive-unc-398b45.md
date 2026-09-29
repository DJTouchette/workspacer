---
title: Rust backend database ownership must outlive unconfirmed cleanup
date: 2026-09-29
author: codex
confidence: high
suggested_doc: hub-process-supervision
related_paths:
  - services/hub-rs/src/backend/ownership.rs
  - services/hub-rs/tests/backend_owner.rs
promoted: false
---

# Rust backend database ownership must outlive unconfirmed cleanup

## Observation
Native and standalone Rust Backend hosts can otherwise open the same SQLite database while each owns independent daemon threads. The canonical database sidecar advisory lock must be acquired before daemon creation and retained until both hub and engine joined shutdown succeed. Dropping an incompletely joined owner intentionally retains the lock until process exit; unlinking the stable sidecar would permit concurrent inode owners. Cross-process refusal, clean release, unconfirmed-drop retention and crash release are covered by backend_owner tests.

## Impact
Moving native and TUI defaults to Rust makes cross-process ownership a correctness boundary, not just a process-local runtime concern.

## Recommendation
Use Backend for cooperating Rust hosts and preserve the stable lock inode. Standalone claudemon CLI currently does not participate and must not share an active Backend database.
