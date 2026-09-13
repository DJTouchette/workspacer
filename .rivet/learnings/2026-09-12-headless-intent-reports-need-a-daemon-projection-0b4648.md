---
title: Headless intent reports need a daemon projection because brain snapshots omit conversations
date: 2026-09-12
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/headless/intentObservations.ts
  - services/hub/cmd/brain/conversation.go
  - services/hub/cmd/brain/desktophost.go
  - services/claudemon/src/session/summary_source.rs
promoted: false
---

# Headless intent reports need a daemon projection because brain snapshots omit conversations

## Observation
Brain live snapshots deliberately omit conversation (services/hub/cmd/brain/conversation.go); internal.observe is a two-second poll. Intent background capture must read bounded conversation?summary_source=1 for linked local identities to retain report excerpts without an open browser pane. That source caps event text at 800 characters and total JSON at 5000 bytes. Peer IDs must never be fetched from a same-ID local daemon. A final state alone does not prove the final text has arrived.
