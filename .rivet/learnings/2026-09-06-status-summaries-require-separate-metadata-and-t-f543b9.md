---
title: Status summaries require separate metadata and transcript projections plus adapter no-tools verification
date: 2026-09-06
confidence: high
suggested_doc: claudemon-http-api
related_paths:
  - services/claudemon/src/daemon/api.rs
  - apps/desktop/src/main/services/directCompletion.ts
promoted: false
---

# Status summaries require separate metadata and transcript projections plus adapter no-tools verification

## Observation
The normal claudemon GET /sessions/:id calls usage_for_session and may read the full transcript before any summary source check. The existing directCompletion Claude runner originally passed only --print/--model; Codex read-only sandbox and OpenCode --pure do not establish an empty tool registry. The new versioned summary_meta and summary_source projections bypass normal usage IO and bound source serialization, while requireNoTools fails closed for adapters without verified restrictions. Old daemons may ignore query fields, so callers must require the discriminator before any model call. Rivet witness run currently chooses Jest at this monorepo root; use witness select targets with the desktop Vitest runners.
