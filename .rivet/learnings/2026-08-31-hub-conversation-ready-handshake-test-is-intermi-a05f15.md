---
title: Hub conversation ready handshake test is intermittently order-sensitive
date: 2026-08-31
confidence: medium
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub/cmd/brain/conversation_test.go
promoted: false
---

# Superseded: Hub conversation ready handshake test is intermittently order-sensitive

## Superseded
This note is superseded by the per-generation ready-handshake fix and its
regression coverage. Do not weaken handshake-first assertions by accepting an
assistant item before the ready handshake or by classifying the handshake as
optional scheduling noise. The ready event establishes the generation boundary
that makes a demanded conversation safe to consume; tests should synchronise on
that boundary and reject a stale or missing handshake.

## Observation
During this integration, TestConversationForwardsDemandedSessionsOnly failed once because the first observed publish was the seq=2 assistant item instead of the ready handshake; the unchanged full race-enabled Hub suite passed on immediate rerun.

## Impact
A sporadic ordering failure can obscure integration regressions in the otherwise fast Hub suite.

## Recommendation
The older recommendation to wait for or classify around a missing handshake is
withdrawn. Keep the handshake-first assertion, scoped to the requested
generation, and investigate any failure as a conversation ordering or
generation-fencing regression.
