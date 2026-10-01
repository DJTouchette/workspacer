---
title: Native child usage must use status-line totals and observed provider model
date: 2026-10-01
promoted: false
---

# Native child usage must use status-line totals and observed provider model

## Observation
Rust hub snapshots map usage to model/contextTokens/costUSD; cumulative token counters live in statusLine.totalInputTokens/totalOutputTokens (raw daemon status_line.total_input_tokens/total_output_tokens). Native child telemetry merges these components independently and never labels contextTokens as total consumption. Session.model may represent requestedSelection; child rows prefer telemetry observed_model from usage.model or actual top-level model and never substitute a requested spawn claim. Explicit null clears stale measurements, and resumed running clears old completion/duration. Provider rows deduplicate native IDs; anchors require a nonempty spawn tool ID, so a matching arbitrary Read tool cannot claim child rows.
