---
title: Jobs history omitted fields must deserialize with Go defaults
date: 2026-09-29
promoted: false
---

# Jobs history omitted fields must deserialize with Go defaults

## Observation
Run serialized empty detail and zero finishedAt as omitted fields, but its Rust deserializer required both. Reopening a history file containing a successful silent shell run could discard all history. Run now has serde(default) and Go-compatible null-to-zero field handling, pinned with actual atomic save/reopen. Job/Trigger nullable scalar/day fields likewise retain Go defaults; valid root null/jobs:null/empty-object files clear schedules, whereas missing/empty/truncated/invalid files retain last good state. Noncanonical known-name aliases are intentionally rejected rather than silently ignoring Go Unicode-folded guards.
