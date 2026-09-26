---
title: Large fleets thrash the bounded snapshot compaction memo
date: 2026-09-26
confidence: high
suggested_doc: renderer-live-state-hooks
related_paths:
  - apps/desktop/src/main/shared/compactClaudeSnapshot.ts
  - apps/desktop/scripts/bench-agent-load.mjs
promoted: false
---

# Large fleets thrash the bounded snapshot compaction memo

## Observation
The shared snapshot compactor retains 1024 wire-key entries, but each agent can contribute 20 completed tools and 80 file changes. A fleet sweep evicts entries needed on the next sweep and repeatedly serializes unchanged payloads. A WeakMap now reuses retained producer objects while the bounded wire memo still handles IPC clones; memo keys also include session and hub to prevent cross-agent collisions.

## Impact
On this host, npm run bench:agent-load with 8KB synthetic payloads and 20 warm sweeps measured median 50-agent compaction 158.77ms before versus 1.41ms after, and 200-agent compaction 650.83ms versus 7.78ms. These are synthetic compaction timings, not Electron frame times or provider startup latency.

## Recommendation
Keep settled object reuse weak so it follows source lifetime; running tools bypass caching. Use bench:agent-load and the 50-agent no-reserialization regression when changing compaction.
