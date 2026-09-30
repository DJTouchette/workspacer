---
title: Empty node registry still needs its public capability handlers
date: 2026-09-29
confidence: high
suggested_doc: headless-desktop-services
related_paths:
  - services/hub-rs/src/services/nodes/mod.rs
  - services/hub-rs/tests/headless_completeness.rs
promoted: false
---

# Empty node registry still needs its public capability handlers

## Observation
The real headless inventory test found nodes.list, nodes.wake and nodes.sleep missing with an empty registry. Rust ported startNodes dormancy, but Go main.go creates an empty supervisor after that helper returns nil and registers all three methods unconditionally. Restore empty list and unknown-node replies without starting cloud clients or a reconciliation task. The initial test failed in /tmp/workspacer-headless-files-final.log; corrected owner target passed in /tmp/workspacer-headless-completeness-final.log.

## Recommendation
Review launcher composition as well as subsystem helpers; compare actual registered capabilities against shipped client calls.
