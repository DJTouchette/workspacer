---
title: Native sidebar must combine session lineage with provider child snapshots
date: 2026-10-01
suggested_doc: claudemon-providers
related_paths:
  - apps/native/src/ui/sidebar.rs
promoted: false
---

# Native sidebar must combine session lineage with provider child snapshots

## Observation
Workspacer children are independent Session entries carrying parent_session_id, while Codex native children only exist in their parent Session.subagents snapshot. A flat visible_sessions list omits provider children and sorts newly spawned sessions above their parent. Provider preview requests are admitted only for the currently selected parent, so sidebar navigation must select the parent before requesting SubagentHistory.

## Impact
Sidebar rows need separate session and provider identities, parent-first ordering, and provider-aware scroll offsets; native child IDs must not be fabricated into Workspacer sessions.
