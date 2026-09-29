---
title: MCP compositions must preserve admission receipts and stopped-session reads
date: 2026-09-28
promoted: false
---

# MCP compositions must preserve admission receipts and stopped-session reads

## Observation
Rust manager_context batches only the verified caller inbox and up to four owned task reads, preserves partial errors and numeric evidence, and explicitly defers oversized task evidence instead of truncating it. dispatch_workflow_step uses the host canonical pinned plan and never routes/spawns after a skipped or mismatched step; failed watches do not retry an accepted spawn. respawn_with requires explicit snapshot reads after agents.close because Go handlers.snapshot falls back to claudemon retained data; dismissal must suppress listing/ownership without preventing that historical read. Respawn inherits canonical requestedSelection and live permission mode explicitly, never a stale routing decision ID, and verified caller identity remains the successor parent.
