---
title: Codex PTY hybrid hidden instruction channel
date: 2026-08-30
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - apps/desktop/src/main/services/managedSpawn.ts
  - apps/desktop/src/main/services/managedSpawn.test.ts
promoted: false
---

# Codex PTY hybrid hidden instruction channel

## Observation
The Codex CLI accepts a developer_instructions config override on the PTY rollout path. Passing it with -c keeps host contracts in the developer message while firstMessage remains the user message, and it works even when the spawn initially has no firstMessage.

## Impact
Host result or escalation contracts can be delivered reliably without leaking into reconstructed user-visible prompt content.

## Recommendation
Keep firstMessage unmodified and pass host-authored PTY instructions through developer_instructions; pin both argv construction and no-firstMessage behavior in tests.
