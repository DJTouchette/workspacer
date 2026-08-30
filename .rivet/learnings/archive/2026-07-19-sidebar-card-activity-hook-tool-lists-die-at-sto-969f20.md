---
title: Sidebar card activity: hook tool lists die at Stop
date: 2026-07-19
promoted: true
---

# Sidebar card activity: hook tool lists die at Stop

## Observation
The sidebar agent cards showed 'Working…' most of the time because their action log leaned on snapshot.activeToolCalls/completedToolCalls — but applyStopEvent (hookEventRouter) clears BOTH at every turn end ('already shown inline in conversation'), so between turns and early in a turn there was no tool history to show. The durable copy of tool calls lives on the conversation turns themselves (ConversationTurn.toolCalls, transcript-tailer-derived, survives Stop) — the hook lists are only fresher mid-turn. Fixed 2026-07-19: lib/agentActivityLog.ts collectRecentActivity() unions turns[].toolCalls + the two hook lists (dedup by tool_use id, hook entries win), merges with assistant messages, time-sorted; SideBar cards render the last 3 lines (freshest tinted green when working) and never show the 'Working…' placeholder.

## Disposition
Folded into .rivet/context/domains/mission-control-attention.md (sidebar card activity note).
