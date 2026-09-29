---
title: Malformed profile append prompt flags must not consume host arguments
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/session_facade.rs
promoted: false
---

# Malformed profile append prompt flags must not consume host arguments

## Observation
Go composeAppendSystemPrompt drops a valueless --append-system-prompt flag when followed by another --flag, preserving host-owned arguments. Rust compose_instructions previously consumed that next flag as prompt text. The helper now drops only the malformed pin, skips empty inline fragments as Go does, preserves split-form empty fragments, and keeps ordinary argv positions while combining prompt fragments in declaration order. Regression asserts --model and --session-id survive and rejects non-string argv.
