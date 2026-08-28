---
title: A headless node publishes a session row only on MODE transitions — streaming text is invisible to bus clients
date: 2026-08-26
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/renderer/src/backend/webBackend.ts
  - apps/desktop/src/renderer/src/backend/busConversation.ts
  - services/claudemon/src/session/store.rs
  - services/claudemon/src/daemon/api.rs
  - services/hub/cmd/brain/events.go
  - services/hub/cmd/brain/enrich.go
promoted: false
---

# A headless node publishes a session row only on MODE transitions — streaming text is invisible to bus clients

## Observation
For a managed (stream-transport) session, claudemon's `SessionStore::ingest` returns early — hooks are enrichment only — so `update_tx` (which feeds `/events` → brain → `agent.snapshot` on the bus) fires ONLY from `set_managed_mode`, `set_plan`, `set_background_tasks` and `park_decision`. Assistant text growing is not a state change. Measured on a local `workspacer serve` + managed claude: one 21s turn produced 32 `conversation.delta` frames inside claudemon and exactly TWO `agent.snapshot` events on the bus — `responding` (7ms after the send acked) and `input` at the end. Any bus client that treats `agent.snapshot` as its conversation clock therefore renders once, at turn end: the real /app bundle in Chromium sat a median 10.3s / worst case 20.8s behind the daemon. claudemon HAS a live delta feed (`/conversation/stream`, the `conversation.delta` SSE) and the desktop + wks-tui consume it; the brain does not forward it to the bus.

## Impact
This is the whole "remote replies are inconsistently slow" report — short turns look instant, long ones look dead. It is not refetch size: the `?since` anchor carries one item per fetch (~945 B). It also means any FUTURE bus client (a new pane, a plugin, the MCP facade) that watches conversations will hit the same wall unless it runs its own clock.

## Recommendation
Fixed for /app in webBackend.ts by ticking `sessions.conversation` every 500ms while a WATCHED session's row says `ambientState: 'streaming'` (median 222ms / max 499ms after). Note the cost recorded there: because claudemon coalesces a streaming reply into one item that grows in place and `/conversation` answers with items, every poll re-sends the whole in-progress message — 46 fetches / 62 KB for a 2.6 KB reply, quadratic in reply length. Getting off that curve requires forwarding `/conversation/stream` deltas over the bus behind a per-open-pane subscribe/unsubscribe, which would also preserve the sparse-row bandwidth rationale in cmd/brain/parity_test.go.
