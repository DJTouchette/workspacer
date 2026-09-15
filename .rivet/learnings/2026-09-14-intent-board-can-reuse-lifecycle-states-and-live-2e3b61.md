---
title: Intent board can reuse lifecycle states and live attention projection
date: 2026-09-14
promoted: false
---

# Intent board can reuse lifecycle states and live attention projection

## Observation
IntentWorkspace declares draft, active, review, complete. IntentWorkspaces currently renders project-grouped navigation and nine detail views. Its list response already provides executionIndex, which IntentAttentionBadge joins with live sessions through intentSessionAttention; pending questions and approvals are attention signals independent of lifecycle status. A board can reuse these four states and attention badges without introducing a blocked lifecycle state. Rich criterion coverage is computed by summarizeIntent from additional evidence data, not supplied by the list response.
