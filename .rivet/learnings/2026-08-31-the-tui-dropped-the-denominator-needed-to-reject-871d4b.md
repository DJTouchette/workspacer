---
title: The TUI dropped the denominator needed to reject a contradicted status percentage
date: 2026-08-31
confidence: high
suggested_doc: tui-client
related_paths:
  - apps/tui/src/types.rs
  - services/claudemon/src/session/state.rs
promoted: false
---

# The TUI dropped the denominator needed to reject a contradicted status percentage

## Observation
claudemon and agent.statusline already carry context_window_size, but apps/tui::StatusLine omitted it while derive_stats preferred context_used_pct. The TUI therefore could not apply the desktop/bus rule that treats percentage and denominator as one claim.

## Impact
A 1M session holding 356,380 tokens could render as raw 200K/100% in the TUI even though usage.context_limit was correctly resolved to 1M.

## Recommendation
Keep context_window_size optional for skew compatibility and reject the status pair only when usage.context_tokens exceeds it past the shared 2% tolerance; fall back to resolved usage, never infer 1M from occupancy.
