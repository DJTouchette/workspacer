---
title: Claude child sidecars provide authoritative tool anchors and timestamps keep milliseconds
date: 2026-10-01
suggested_doc: workflow-subagent-watcher
related_paths:
  - services/claudemon/src/session/claude_subagents.rs
promoted: false
---

# Claude child sidecars provide authoritative tool anchors and timestamps keep milliseconds

## Observation
Electron workflowWatcher consumes subagents/agent-ID.meta.json with agentType,description,toolUseId; JSONL alone cannot guarantee which parent Agent call owns a child. Claudemon now reads an optional bounded64KiB same-parent canonical sidecar, refuses redirects and oversized identity anchors, and keeps transcript-reported runtime model. Child timestamp conversion uses unix_timestamp_nanos divided by1e6 rather than rounded seconds, preserving same-second spawn ordering and subsecond duration.

## Impact
Missing sidecars caused fallback attachment despite available exact tool IDs; second rounding broke ordering among nearby dispatches.

## Recommendation
Keep metadata anchor/missing/oversized/redirect and750ms duration regressions alongside replay tests.
