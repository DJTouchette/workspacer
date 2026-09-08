---
title: Overview profile discovery needs a usable allowance reading
date: 2026-09-08
promoted: false
---

# Overview profile discovery needs a usable allowance reading

## Observation
usagePacingRows deliberately retains unknown windows for Inspector diagnostics. Overview must require at least one row with usedPct !== undefined before creating a profile card, using the same nowMs for filtering and rendering. Do not gate on pace availability or percentage truthiness: a valid zero-percent reading with unknown pace still identifies a real account. Keep canonical reports and session attribution intact; hiding unknown-only Overview cards must not remove profiles or change routing. Regression fixture reproduced five cards instead of two before the filter; targeted renderer checks passed afterward.
