---
title: Codex ownership must cover publication and registration, not just teardown
date: 2026-09-06
suggested_doc: claudemon-providers
related_paths:
  - services/claudemon/src/providers/codex.rs
  - services/claudemon/src/session/store.rs
promoted: false
---

# Codex ownership must cover publication and registration, not just teardown

## Observation
Managed row publication formerly preceded Codex claim_generation, and startup/fallback dropped owns_generation before registry mutations. Exclusive claim_generation_with publication and with_generation synchronous mutations now share a lock with deregister_managed_with, including conversation deletion. Codex PTY output rechecks after awaiting its buffer; fallback rollout application carries the same generation. Lock order is generation then synchronous registries/conversation; buffer waits occur before generation acquisition, never inside a generation transaction. Other providers retain legacy spawn/PTY entry points.

## Impact
A checked old driver could otherwise tombstone or overwrite a successor, even when its owned process cleanup was correct.

## Recommendation
Keep generation publication, registration, and shared teardown in synchronous transactions; captured child/group cleanup remains independent and may await outside them.
