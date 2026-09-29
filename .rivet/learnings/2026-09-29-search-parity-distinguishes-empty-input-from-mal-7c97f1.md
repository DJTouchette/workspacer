---
title: Search parity distinguishes empty input from malformed options
date: 2026-09-29
confidence: high
suggested_doc: headless-desktop-services
related_paths:
  - services/hub-rs/src/services/search.rs
promoted: false
---

# Search parity distinguishes empty input from malformed options

## Observation
The retained Go search handler validates typed options and canonical cwd before returning an empty result for an empty query. Rust had rejected empty queries and silently defaulted malformed booleans/limits. Its collector also added1 to unchecked unsigned start offsets. Typed request checks, empty-query result parity and checked signed offsets now preserve valid requests while refusing malformed input/output without overflow; ASCII-only trim, codepoint clip,500default and minified lines remain tested with real rg.
