---
title: wks-tui main.rs header still describes the pre-bus default
date: 2026-08-26
author: Codex
confidence: high
suggested_doc: tui-client
related_paths:
  - apps/tui/src/main.rs
  - apps/tui/README.md
promoted: true
promoted_to: tui-client
---

# wks-tui main.rs header still describes the pre-bus default

## Observation
`apps/tui/README.md` and the actual `Cli`/startup flow in `apps/tui/src/main.rs` say the TUI defaults to the hub bus with a brain provider and falls back to claudemon-direct when needed, but the file-level module comment at the top of `apps/tui/src/main.rs` still says it talks directly to claudemon and cannot rely on hub capabilities because Electron main registers them. That header is stale relative to the code below it.

## Impact
Future readers may misclassify the TUI as claudemon-direct by design and miss the bus/brain/federation paths when changing TUI behavior.

## Recommendation
When touching TUI startup docs or bus/direct behavior, update the module comment in `apps/tui/src/main.rs` to match the bus-first README and current startup path.
