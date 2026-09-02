---
title: Codex usage reports preserve missing five-hour readings
date: 2026-08-30
confidence: high
suggested_doc: usage-accounting
related_paths:
  - services/hub/internal/limits/window.go
  - apps/desktop/src/main/services/keepWarmLogic.ts
  - services/claudemon/src/session/usage_report.rs
promoted: true
promoted_to: usage-accounting
---

# Codex usage reports preserve missing five-hour readings

## Observation
Codex report ingestion treats a missing/invalid five-hour reset as an unreadable live reading (no-reset-time-reported), rather than rendering zero. This is distinct from a renderer omission and must be traced from newest rollout rate_limits through the hub report.

## Impact
Avoids incorrectly attributing an absent five-hour UI window to a generic upstream plan assumption.

## Recommendation
When diagnosing a missing Codex limit window, inspect the live usage report/window currency reason before changing parser or UI.

## Disposition
Promoted verbatim in substance into `.rivet/context/domains/usage-accounting.md` (2026-09-01 note). Re-verified against master `0bac5799`: `ReasonNoResetTime = "no-reset-time-reported"` still lives in `services/hub/internal/limits/window.go`, and `fiveHourWindowFromReport` in `keepWarmLogic.ts` still SKIPS a window with no `resets_at` rather than rendering 0%. No correction needed.
