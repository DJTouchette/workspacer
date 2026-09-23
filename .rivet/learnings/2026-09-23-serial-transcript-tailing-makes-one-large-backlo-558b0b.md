---
title: Serial transcript tailing makes one large backlog delay the whole fleet
date: 2026-09-23
confidence: high
suggested_doc: claudemon-http-api
related_paths:
  - services/claudemon/src/session/conversation.rs
  - services/claudemon/src/session/store.rs
promoted: false
---

# Serial transcript tailing makes one large backlog delay the whole fleet

## Observation
services/claudemon/src/session/conversation.rs:378-405 runs one 400ms loop, serially awaiting tail_one then tail_subagents for each sessions.list entry. tail_one reads the entire remaining file and synchronously parses every line (462-493), so initial resume/adoption of a large transcript has no per-tick byte or time budget. tail_subagents re-enumerates and sorts every subagent filename, then stats every file even when unchanged (567-605). sessions.list also clones every SessionState before filtering (session/store.rs:1345-1346). These are verified code paths; the wall-clock impact is not yet profiled on a real fleet.

## Impact
Transcript update latency grows with all active sessions and historical subagent files. One slow or backlogged session can delay updates for unrelated sessions; synchronous parsing also occupies an async worker.

## Recommendation
Instrument tail-loop duration, bytes parsed and per-session lag; enforce bounded read/parse budgets and fair or bounded-concurrent scheduling; cache directory membership or use filesystem notifications with periodic reconciliation.
