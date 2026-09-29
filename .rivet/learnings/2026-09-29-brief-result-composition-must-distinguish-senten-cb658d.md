---
title: Brief result composition must distinguish sentence references from fact text
date: 2026-09-29
related_paths:
  - services/hub-rs/src/services/briefs/report.rs
promoted: false
---

# Brief result composition must distinguish sentence references from fact text

## Observation
Go composes the final session suffix based only on the caller significance sentence; Rust previously checked the entire output after facts, so a ref merely mentioned inside arbitrary result text suppressed the explicit suffix. Rust also capped non-SHA commit text and printed whole-valued JSON floats with .0 unlike Go. report::compose now preserves those semantics; null optional metadata behaves as absent on ordinary appends and explicit empty sessionId remains optional in composition. Exact legacy vectors and long-caveat preservation regressions are in tests/briefs.rs.
