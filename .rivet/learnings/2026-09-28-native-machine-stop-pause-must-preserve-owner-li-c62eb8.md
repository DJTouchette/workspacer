---
title: Native machine-stop pause must preserve owner lifetime and require explicit wake
date: 2026-09-28
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/bus.rs
  - apps/native/src/backend.rs
promoted: false
---

# Native machine-stop pause must preserve owner lifetime and require explicit wake

## Observation
Native WebSocket transport retried every close, including4001, which could wake a Fly host immediately after stop. The new power pause latches before handshake or after active calls, suppresses connection attempts and host reconciliation, rejects mutation replay, and consumes one explicit user resume per pause. In-process typed pause must keep the native controller channel alive: closing it would make NativeHost treat viewer pause as backend-owner shutdown.

## Recommendation
Test pre-hello and established4001, failed pending calls, no network acceptance during pause, duplicate resume isolation across later stops, explicit Refresh, and embedded owner remaining ready until its own shutdown. A close code signals pause intent, not proof the OS/cloud machine stopped.
