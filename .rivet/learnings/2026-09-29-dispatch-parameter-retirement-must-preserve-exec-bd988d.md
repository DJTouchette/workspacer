---
title: Dispatch parameter retirement must preserve executed corpus and derived-only wire fields
date: 2026-09-29
confidence: high
suggested_doc: headless-desktop-services
related_paths:
  - services/hub-rs/tests/library.rs
  - contracts/dispatch-template-params-cases.json
promoted: false
---

# Dispatch parameter retirement must preserve executed corpus and derived-only wire fields

## Observation
The existing Rust dispatch parser matched the shared cases, but its loader did not assert its owner or executed18-case floor. Added both with the nondiscardable tally. Real bus save/list tests now prove params derive from the authored body, never persist a caller-supplied derived field, and filters only narrow. Rust keeps the TS corpus explicit empty default even though old Go omitempty omitted that optional wire property.
