---
title: Brain-published snapshots carry NO conversation — clients must poll sessions.conversation themselves
date: 2026-08-26
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/renderer/src/backend/busConversation.ts
  - apps/desktop/src/renderer/src/backend/webBackend.ts
  - services/hub/cmd/brain/enrich.go
  - services/claudemon/src/daemon/api.rs
  - apps/desktop/tests/e2e/fixtures/mobileHub.ts
promoted: true
promoted_to: renderer-backend-seam
---

# Brain-published snapshots carry NO conversation — clients must poll sessions.conversation themselves

## Observation
The headless brain publishes claudemon's session row with the desktop field names overlaid and `sparse: true` (cmd/brain/enrich.go compatSnapshot), and it deliberately omits `conversation` entirely — cmd/brain/parity_test.go records the reason ("transcript lives in claudemon's /conversation endpoint; folding it into every snapshot/publish would ship whole transcripts per state tick"). Every client is therefore responsible for calling `sessions.conversation` itself: /m polls it, main's federationBridge folds it for peer sessions, and until 2026-08-26 the web backend did neither for its own sessions — so /app against a `workspacer serve` node (a node attached with `brain --hub` is a capability PROVIDER, not a federated peer, so its rows carry no `hub` stamp) rendered an empty chat AND an immortal "Sending…" bubble, because ClaudePane retires optimistic bubbles by watching `conversation` grow a user turn. `agents.sendMessage` itself acks in ~2ms; usage/approvals ride the sparse row and worked the whole time. Second gotcha, in claudemon: a streaming assistant reply COALESCES into one item that grows in place while `seq` races ahead of it, and `?since=` skips by item INDEX (items_skip in daemon/api.rs) — so polling with "the seq you were last told" returns nothing forever. Anchor on (index of the newest item you hold − 1) so that item comes back and its growth folds in.

## Impact
Any new bus client (or any renderer surface that expects a transcript) silently shows an empty conversation against a headless/serve fleet, and anything gated on conversation growth (optimistic sends, auto-title, turn-diff cards) never fires. The e2e app/mobile fixtures answer RICH desktop-shaped rows, so the suite cannot see this class of bug at all.

## Recommendation
Fixed by apps/desktop/src/renderer/src/backend/busConversation.ts + webBackend wiring (commit 104577c0). When adding a client or a fixture, model the SPARSE row (no conversation) — that is what a real headless fleet publishes — not FIXTURE_SESSIONS' rich shape.
