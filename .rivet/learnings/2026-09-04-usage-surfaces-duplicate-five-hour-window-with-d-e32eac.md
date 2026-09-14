---
title: Usage surfaces duplicate five-hour window with different meanings
date: 2026-09-04
confidence: high
suggested_doc: usage-accounting
related_paths:
  - apps/desktop/src/renderer/src/components/claude/InspectorCard.tsx
  - apps/desktop/src/renderer/src/components/SessionAccountUsage.tsx
  - apps/desktop/src/renderer/src/components/UsageReportCard.tsx
  - apps/desktop/src/renderer/src/lib/sessionStats.ts
promoted: false
---

# Usage surfaces duplicate five-hour window with different meanings

## Observation
InspectorCard renders live session status-line usage windows with severity colors, while SessionAccountUsage renders the same 5-hour/7-day account allowance from usage.report with pace verdict colors. The two sources can both appear in one Usage tab and are not interchangeable.

## Impact
Without an explicit distinction, users can read one 5-hour percentage as both session consumption and account quota, and the same percentage can carry conflicting severity versus pace semantics.

## Recommendation
Future usage UI work should establish one labeled allowance hierarchy and either remove the duplicate account window from the session block or explicitly label the session telemetry as session-local and the report rows as shared account allowance.
