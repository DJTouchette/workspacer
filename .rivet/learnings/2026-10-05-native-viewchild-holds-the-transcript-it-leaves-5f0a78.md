---
title: Native ViewChild holds the transcript it leaves for instant return
date: 2026-10-05
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/native/src/controller.rs
  - apps/native/tests/protocol.rs
promoted: false
---

# Native ViewChild holds the transcript it leaves for instant return

## Observation
controller.rs view_child now holds the settled transcript being left (the parent, or up to 8 children / 16MB) stamped with the connection epoch. Returning shows it with loading=false and still issues the same selection-fenced read, which folds in via snapshot_for_transport's identity-stable keys; the parent's conversation_limit is restored so loaded older pages survive the reconcile. Held entries from an older epoch are never used. Child reads (sessions.subagentConversation) are unpaged (up to 2000 items) and the claudemon side replays the child JSONL on every call, including the 2s poll while a child runs.

## Impact
Revisit latency drops from a full read+parse (debug ~140-170ms for 1000 items, plus real backend replay) to the next 33ms publish tick; reads per switch unchanged (1).

## Recommendation
Only Command::ViewChild holds; Select/Connected do not (Connected bumps the epoch before reselecting, so holding there would stamp pre-disconnect rows as current). Protocol test: returning_to_a_parent_or_child_shows_it_at_once_and_still_reconciles.
