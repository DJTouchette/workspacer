---
title: Native child receipt navigation is not Electron inline subagent card parity
date: 2026-10-01
promoted: false
---

# Native child receipt navigation is not Electron inline subagent card parity

## Observation
Native ui/transcript.rs shows plain description/status/lastToolName for exact toolUseId-linked provider subagents and an Open child session button for successful Workspacer spawn receipts. Native sidebar has no parent_session_id tree rendering. Electron ClaudePane anchorWork links completed subagents into WorkCard/ToolTraceCard SubagentRow and keeps running agents in the live log. Receipt navigation alone is not inline child-card parity.
