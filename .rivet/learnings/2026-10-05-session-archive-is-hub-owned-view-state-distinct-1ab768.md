---
title: Session archive is hub-owned view state, distinct from claudemon's archived flag
date: 2026-10-05
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/hub-rs/src/services/session_archive.rs
  - apps/native/src/ui/features.rs
  - apps/desktop/src/renderer/src/hooks/useSessionArchive.ts
  - apps/desktop/src/renderer/src/components/SideBar.tsx
promoted: false
---

# Session archive is hub-owned view state, distinct from claudemon's archived flag

## Observation
User archive lives in hub-rs services/session_archive.rs: <data_dir>/session-archive.json {version, archived:{sessionId: archivedAtMs}}, bus methods sessionArchive.get (view+triage) / sessionArchive.set (triage+operator; view refused), hub-published topic sessionArchive.changed (open-by-decision, never federated). It is NOT claudemon's row field archived (stopped + idle >7d), which snapshotLiveness/agentStatusSummaryRuntime read as not-live; overloading that would mark live archived sessions dead. Native previously kept archives only in native-settings.json per connection, and the web /app sidebar lists layout cards auto-adopted from live snapshots, so a native archive could never reach the web.

## Impact
Any client that hides sessions must read the hub document (get on every connect, apply events version-guarded, first read after connect taken as-is) or archives diverge again; archive must never call stop/close/signal.

## Recommendation
New clients: renderer useSessionArchive hook / native controller view.session_archive. A new bus method needs hub-vocabulary methods+scopes+topics, brain-capabilities + contracts/backend-capabilities hub lists (NOT currentAdditions when it is in the vocabulary), authorization-compositions actors/inert/acknowledgedActors, desktop capability-parameter-policy methodDecisions/inertMethods (parameterDecisions only for scanner-dangerous names), compositionDecisions claims+actors+parameterDecisions, backendParity BUS_BACKED, and strict fake hubs such as native tests/protocol.rs live harness.
