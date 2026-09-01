---
title: Conversation demand needs a per-generation ready barrier
date: 2026-09-01
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/hub/cmd/brain/conversation.go
  - services/hub/cmd/brain/conversation_test.go
promoted: false
---

# Conversation demand needs a per-generation ready barrier

## Observation
Starting the fleet-wide SSE consumer before announceReady returns lets forward race the handshake after setDemand unlocks. A per-demand announcing barrier, keyed by demand identity, lets callbacks arrive without publishing; ready delivery occurs outside the mutex, activation closes the barrier afterward, and release/reset close pending barriers so stale generations cannot deadlock or reactivate replacements.

## Impact
Clients must never observe a conversation delta before the frame that proves push support, while concurrent off/on and bus resets must not lose or revive demand.

## Recommendation
Represent fresh conversation demand as announcing then active; wait without the mutex in forward, re-check pointer identity after the barrier, and mutation-test activation-before-ready.
