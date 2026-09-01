---
title: Hub conversation ready handshake test is intermittently order-sensitive
date: 2026-08-31
confidence: medium
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub/cmd/brain/conversation_test.go
promoted: false
---

# Hub conversation ready handshake test is intermittently order-sensitive

## Observation
During this integration, TestConversationForwardsDemandedSessionsOnly failed once because the first observed publish was the seq=2 assistant item instead of the ready handshake; the unchanged full race-enabled Hub suite passed on immediate rerun.

## Impact
A sporadic ordering failure can obscure integration regressions in the otherwise fast Hub suite.

## Recommendation
Reproduce under repetition and make the test wait for or classify the ready handshake without assuming it is always the first scheduled publish.
