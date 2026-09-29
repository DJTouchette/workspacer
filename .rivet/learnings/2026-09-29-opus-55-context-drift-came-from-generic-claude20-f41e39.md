---
title: Opus 5.5 context drift came from generic Claude200k table row
date: 2026-09-29
confidence: high
related_paths:
  - contracts/model-context-windows.json
  - services/claudemon/src/session/windows.rs
  - services/claudemon/src/session/usage.rs
promoted: false
---

# Opus 5.5 context drift came from generic Claude200k table row

## Observation
Observed claude-opus-5-5 peak300682 exceeds the generic Claude200000 table row. Anthropic official Opus5.5 overview and migration guide verified2026-09-29 state1M is default without a beta header: https://platform.claude.com/docs/en/models/opus-5-5/migration-guide . Added a specific suffix row and inherent-window helper across Rust/TS/Go plus shared corpus cases. usage_for_session reads status_line.context_window_size first, then table fallback; a genuine stale200k provider report can still warn before falling through to the valid1M table claim. The fix does not infer capacity from observed occupancy, suppress drift warnings, or override a still-consistent runtime report.

## Recommendation
Preserve report/user override precedence and unknown-on-disproof behavior; refresh the two changed Go oracle hashes and rebuild the daemon to apply the table fix.
