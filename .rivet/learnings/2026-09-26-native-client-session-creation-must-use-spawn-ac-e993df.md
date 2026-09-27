---
title: Native client session creation must use spawn acknowledgement and preserve uncertain outcomes
date: 2026-09-26
confidence: high
related_paths:
  - apps/native/src/controller.rs
  - apps/native/src/ui.rs
  - apps/desktop/src/renderer/src/backend/webBackend.ts
promoted: false
---

# Native client session creation must use spawn acknowledgement and preserve uncertain outcomes

## Observation
apps/native currently has live observation/control but no creation UI. Its live harness uses agents.spawn with stream transport. webBackend allows six minutes for spawn and captures sessionId/messageQueued from the acknowledgement; a snapshot alone cannot establish initial message delivery. Native creation should use the existing capability, not invent a daemon endpoint or resend on reconnect.
