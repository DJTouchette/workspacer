---
title: Report account attribution on a session surface must refuse to guess
date: 2026-09-04
confidence: high
suggested_doc: usage-accounting
related_paths:
  - apps/desktop/src/renderer/src/lib/usagePacing.ts
  - apps/desktop/src/renderer/src/components/SessionAccountUsage.tsx
  - apps/desktop/src/main/shared/usageReport.ts
promoted: false
---

# Report account attribution on a session surface must refuse to guess

## Observation
usage.report rows are per provider+account; the Overview never picks one (it draws them all), but any SESSION surface has to. Two traps: (1) claudeAccountOf('') returns '' — the DEFAULT login key — so a federated session (whose transcriptPath the bridge blanks) silently borrows the local default account's numbers unless snapshot.hub is checked first; (2) reportAccountKey() takes the config root's basename, so two roots ending in the same name (/a/work and /b/work) collide on one key and a transcript path does NOT separate them. lib/usagePacing.usageReportAttribution returns match | ambiguous | none | remote | unavailable for this reason; the report's null-account (unattributed) bucket is never a match for a session that can name itself.

## Impact
Showing another login's 91% weekly figure on a session card is a lie a reader cannot detect. Any future surface (TUI, /m, web) that wants per-session allowance must reuse this selector rather than "the freshest reading" or "the only account".

## Recommendation
Reuse usageReportAttribution + useUsageReport (one shared cache/poll) instead of matching provider rows by hand; mount the block only while the surface is visible so the poll's subscriber count stays honest.
