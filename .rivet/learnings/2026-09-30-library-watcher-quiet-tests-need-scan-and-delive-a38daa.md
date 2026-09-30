---
title: Library watcher quiet tests need scan and delivery barriers, not a settling sleep
date: 2026-09-30
confidence: high
suggested_doc: claude-asset-roots
related_paths:
  - services/hub-rs/src/services/library_watch.rs
promoted: false
---

# Library watcher quiet tests need scan and delivery barriers, not a settling sleep

## Observation
Full-library runs intermittently failed the unchanged100ms quiet assertion because write-until-event may receive an older write notification, while the final projection is still scanning or queued for delivery. An80ms sleep plus drain did not establish quiescence. The revised fixture observes a cfg(test)-only watch of the exact committed revision, then sends a marker through the same subscribed event stream and drains preceding empty refreshes before the existing quiet assertion. Production polling/publishing behavior and timeout budgets remain unchanged.

## Validation
Full library391 passed with the barrier in /tmp/workspacer-watcher-barrier-full-lib.log. Five additional exact watcher runs against that same binary, from the Cargo manifest directory, passed in /tmp/workspacer-watcher-barrier-repeats.log. The original100ms quiet assertions remain unchanged.
