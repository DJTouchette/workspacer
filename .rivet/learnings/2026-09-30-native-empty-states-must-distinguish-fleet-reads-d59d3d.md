---
title: Native empty states must distinguish fleet reads from empty workspaces
date: 2026-09-30
confidence: high
related_paths:
  - apps/native/src/controller.rs
  - apps/native/src/ui/states.rs
  - apps/native/tests/protocol.rs
promoted: false
---

# Native empty states must distinguish fleet reads from empty workspaces

## Observation
Native View.loading represents conversation loading and did not describe an outstanding sessions.snapshots request. View.sessions_loading now follows fleet admission, matching-epoch completion and disconnect, with dirty publication at admission. Explicit Refresh marks the selected conversation loading; background transcript polling does not disable the composer. Successful reads clear only their own Sessions unavailable or Conversation unavailable notices. Recovery UI keeps automatic reconnect distinct from generation-bound power-pause wake intent.

## Impact
Do not show no-session onboarding before the first fleet reply or promise a refresh reconnects a disconnected transport.
