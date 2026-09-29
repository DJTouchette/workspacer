---
title: Driver-fed stopped conversations limit respawn parity
date: 2026-09-28
promoted: false
---

# Driver-fed stopped conversations limit respawn parity

## Observation
Actual fake-provider integration found claudemon claude_stream::spawn_session deliberately conv.forget after generation-fenced deregistration. API get_conversation replays Codex disk state only, so a stopped Claude stream can retain snapshot metadata but lack the original conversation. Go respawn_with uses that same conversation API and refuses when no first user message exists. Rust preserves the refusal (does not invent the original from launch metadata). Real composition test clones an idle worker while its observed task is available, then independently verifies closed snapshot readability/list suppression and credential revocation. Full stopped transcript replay is a separate existing engine limitation.
