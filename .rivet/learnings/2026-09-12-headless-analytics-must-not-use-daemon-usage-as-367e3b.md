---
title: Headless analytics must not use daemon usage as cumulative tokens
date: 2026-09-12
confidence: high
suggested_doc: usage-accounting
related_paths:
  - services/claudemon/src/session/usage.rs
  - apps/desktop/src/main/services/analyticsUsage.ts
  - apps/desktop/src/main/services/sessionHistoryCore.ts
promoted: false
---

# Headless analytics must not use daemon usage as cumulative tokens

## Observation
claudemon GET /sessions includes a Usage object with cost/context/cache but no cumulative input/output tokens or per-model split. Mapping that object alone into analytics would fabricate zero-token history. Reuse the desktop streaming transcript recomputation for Claude plus status-line totals for managed providers, and the shared SQL aggregation to preserve multi-model and unrecorded-session semantics.
