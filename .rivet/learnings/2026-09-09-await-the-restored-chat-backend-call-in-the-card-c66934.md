---
title: Await the restored chat backend call in the card browser fixture
date: 2026-09-09
confidence: high
suggested_doc: renderer-live-state-hooks
related_paths:
  - apps/desktop/tests/e2e/fleetContextMenu.test.ts
promoted: false
---

# Await the restored chat backend call in the card browser fixture

## Observation
CI run 34433603032 on de3069ca passed manager handoff and native Windows tests but fleetContextMenu.test.ts card send restores a missing owning chat read an empty backend call log immediately after the text became visible. Card submission may await restoration of its owning chat, and the send path exposes an optimistic preview. Visibility is not the backend-call completion signal. Poll the same exact single-message/session/text call assertion rather than reading the log once.

## Impact
This is test synchronization only; the expected call list and all production source remain unchanged. A persistent missing or misrouted message still fails the polling assertion.

## Recommendation
Keep browser fixture waits tied to the observable being asserted; never replace delivery assertions with UI preview visibility or a fixed delay.
