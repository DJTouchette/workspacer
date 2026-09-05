---
title: Overview usage projections need canonical account rows and backend-owned freshness
date: 2026-09-04
confidence: high
suggested_doc: usage-accounting
related_paths:
  - services/hub/internal/limits/report.go
  - apps/desktop/src/renderer/src/lib/usagePacing.ts
promoted: false
---

# Overview usage projections need canonical account rows and backend-owned freshness

## Observation
usage.report is a no-parameter hub-native read over usageWatcher and PaceFor. Overview must use entire report account windows: legacy per-field status caches and basename account grouping cannot safely supply a paced row. Shared hook cache must be fenced by backend method identity and reset/deadline guards must rerender without a network reply. Known zero must override Go PaceReport omitempty in the projection.
