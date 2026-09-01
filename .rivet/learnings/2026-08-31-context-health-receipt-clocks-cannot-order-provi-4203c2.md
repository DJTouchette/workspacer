---
title: Context-health receipt clocks cannot order provider samples
date: 2026-08-31
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - services/claudemon/src/session/**
  - services/claudemon/src/providers/**
promoted: false
---

# Context-health receipt clocks cannot order provider samples

## Observation
The common context-health observed_at was minted with OffsetDateTime::now_utc() when claudemon parsed or accumulated a sample. Copilot JSONL and Codex rollout files carry provider timestamps, but Claude statusLine and live Codex app-server token updates do not expose a trustworthy provider-origin timestamp, so the shared field cannot honestly order source events. A comparison against it only orders local construction/receipt and can reject a real later compaction in tests without protecting against source reordering.

## Impact
Calling the guard out-of-order protection overstates runtime truth: it cannot prove provider order across supported paths, and a lower active-context reading is legitimate after compaction.

## Recommendation
Use observed_at only for freshness, accept the latest received correlated sample, and rely on provider/session/model telemetry epochs for ownership invalidation. Add a source timestamp later only as an explicit optional provenance field on adapters whose wire supplies one; never synthesize it for the others.
