---
title: Saved restore receipts differ from live layout receipts and recent history needs typed rows
date: 2026-09-29
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/hub-rs/src/services/layout.rs
  - services/hub-rs/src/services/live_controls.rs
promoted: false
---

# Saved restore receipts differ from live layout receipts and recent history needs typed rows

## Observation
Go saved boot documents report individual pane.shell/pane.initialCommand/pane.pluginId removals, while the live layout scrub reports generic pane entries. Rust shared implementation previously emitted the latter for both. Saved mode now preserves the specific loss receipt without changing live layout authority or shape behavior. The recent-history Go decoder also rejects a whole malformed typed response; Rust now validates its known scalar fields rather than coercing them into plausible history.
