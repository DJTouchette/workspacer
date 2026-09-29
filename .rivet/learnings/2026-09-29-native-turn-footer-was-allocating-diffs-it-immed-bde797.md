---
title: Native turn footer was allocating diffs it immediately discarded
date: 2026-09-29
promoted: false
---

# Native turn footer was allocating diffs it immediately discarded

## Observation
render_chat_row invokes transcript::turn_changes for visible assistant turn footers; it previously called Tool::changes and allocated full formatted diffs before keeping only paths/counts. A shared counts-only parsing mode preserves inline full-diff behavior. New native-harness bench-turn-summary reproduces the exact helper workload without GUI/network/models. On one unoptimized Linux run,200completed edits×80lines dropped p50 from18.413ms to9.512ms; this is not release frame latency or an Electron speed claim. Separate remaining risks include synchronous settings.save on the GPUI thread and repeated rich-text parsing, not changed in this bounded fix.
