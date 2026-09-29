---
title: Brain model-window parity is proven by the actual unchanged claudemon owner tests
date: 2026-09-29
confidence: high
suggested_doc: usage-accounting
related_paths:
  - services/claudemon/src/session/windows.rs
  - services/hub-rs/src/model_selection.rs
promoted: false
---

# Brain model-window parity is proven by the actual unchanged claudemon owner tests

## Observation
Rust hub model_selection re-exports claudemon::session::windows, so a passing hub compile does not execute the resolver contract tests. CI36635304909 at b80c9e14 actually ran all9 windows owner tests (table order,28 lookups,12 markers,24 resolutions and direct tolerance boundary), plus the hub catalog contract covering all6 alias badges. Relevant source and fixture bytes are unchanged from that commit. Reuse these exact receipts instead of adding a second resolver/table or asserting dependency tests ran locally.
