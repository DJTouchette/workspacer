---
title: Task history edits need serialized lifecycle writes
date: 2026-09-08
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/dispatchHistoryStore.ts
promoted: false
---

# Task history edits need serialized lifecycle writes

## Observation
DispatchHistoryStore caches mutable task rows and debounces observe() writes; a host CAS only around UI saves cannot protect against later lifecycle flushes overwriting another desktop process. Task Inspector changes must serialize every writer, reload inside lock, and preserve local freshness separately from persisted history.
