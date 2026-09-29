---
title: State-loss heuristics must exclude only their own new lock artifact
date: 2026-09-29
author: codex
confidence: high
suggested_doc: config
related_paths:
  - services/hub-rs/src/state_loss.rs
  - services/hub-rs/src/cli/identity.rs
promoted: false
---

# State-loss heuristics must exclude only their own new lock artifact

## Observation
Rust host-token initialization acquires the stable .remote-token.lock before checking for a missing pairing identity. Reusing the legacy statelost directory heuristic without excluding exactly that lock would report loss on every genuine first run. The shared helper preserves the Go10-case semantics (empty child directories are not state; files including zero-byte files and unreadable/nonempty child directories are) and provides an explicit ignored-name list for the Rust-created lock. Other lock-looking files remain evidence.

## Impact
Create-once identity recovery must not rotate existing pairings, but it must also remain usable on fresh installer-created directories.

## Recommendation
Pass explicit owned bookkeeping names only; do not exempt all dotfiles or all .lock suffixes. Config first-missing-load warnings must not latch persistence blocking.
