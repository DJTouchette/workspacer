---
title: Concurrent daemon launches must share their owned worktree admission fence
date: 2026-09-26
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/claudemon/src/daemon/worktree_admission.rs
promoted: false
---

# Concurrent daemon launches must share their owned worktree admission fence

## Observation
The per-gitdir maintenance lock previously used create_new for every launch. Overlapping launches in one linked worktree therefore rejected each other with HTTP 409 even without maintenance. The daemon now reference-counts its exact owned fence in a process-local registry. Nested cwd aliases resolve to the same canonical gitdir. Final release and acquisition share a mutex; a PID alone never authorizes reuse.

## Impact
A 100-thread overlapping-admission regression passes while exclusive maintenance acquisition remains blocked until the final registration guard is dropped. This exercises admission, not provider process startup.

## Recommendation
Keep cross-process exclusion and owner-token checks. Never drop the external fence while a sibling launch still holds admission, and never infer ownership from PID or age alone.
