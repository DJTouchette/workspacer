---
title: Path comparison retirement needs native ordinal evidence rather than a cross-build claim
date: 2026-09-29
confidence: high
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub-rs/src/services/paths.rs
  - services/hub-rs/tests/briefs.rs
promoted: false
---

# Path comparison retirement needs native ordinal evidence rather than a cross-build claim

## Observation
The retained Go pathmatch helpers split byte-exact non-Windows and counted UTF16 CompareStringOrdinal Windows behavior. Rust consolidates them into semantic contained. Actual Windows CI e8afcb87 executed every original containment vector plus Cyrillic/umlaut/Kelvin controls and real case/slash-spelled brief operations on unchanged source; a cargo cross-check alone would not establish that evidence. Go syscall AST and copied-predicate guards can retire only with the actual native runtime assertions retained.
