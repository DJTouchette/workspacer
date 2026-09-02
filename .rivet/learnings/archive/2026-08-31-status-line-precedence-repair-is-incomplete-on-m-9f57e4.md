---
title: Status-line precedence repair is incomplete on mobile and TUI
date: 2026-08-31
confidence: high
suggested_doc: usage-accounting
related_paths:
  - apps/desktop/src/renderer/src/lib/sessionStats.ts
  - apps/desktop/src/main/services/hubCapabilities.ts
  - services/hub/cmd/hub/mobile.html
  - apps/tui/src/types.rs
promoted: true
promoted_to: usage-accounting
---

# Status-line precedence repair is incomplete on mobile and TUI

## Observation
Desktop renderer sessionStats and desktop bus publication reject a raw 200K status-line denominator when held tokens disprove it and fall back to resolved usage contextLimit. `/m` mobile.html stats() still unconditionally prefers statusLine.contextUsedPct/contextWindowSize, while TUI derive_stats() unconditionally prefers status-line percent and does not carry its denominator. A selected `opus[1m]` session can therefore still show 100%/200K outside desktop.

## Impact
Phase 2 can regress user-visible context reporting if it treats the TS resolver contract as complete; federated and headless users primarily hit the remaining consumers.

## Recommendation
Ship a small read-path slice first: port the paired contradiction rule to mobile and provide TUI enough window/provenance to apply it; add a shared 356,380-token / 1M-request / raw-200K fixture.

## Disposition
Promoted into `.rivet/context/domains/usage-accounting.md` as the FOUR-CLIENT rule; the incident is spent. `51f21232` ported the paired-contradiction rule to `mobile.html` (`CONTEXT_WINDOW_DRIFT_TOLERANCE`) and gave the TUI's `StatusLine` the `context_window_size` denominator it was missing, so `derive_stats` can apply it. What survives is the standing warning that the rule has FOUR copies and a reduced client is the one that gets forgotten.
