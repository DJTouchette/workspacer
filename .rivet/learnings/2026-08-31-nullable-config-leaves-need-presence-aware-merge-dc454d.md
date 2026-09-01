---
title: Nullable config leaves need presence-aware merge
date: 2026-08-31
confidence: high
suggested_doc: config
related_paths:
  - apps/desktop/src/main/services/configService.ts
  - services/hub/cmd/brain/config.go
  - services/hub/cmd/brain/handlers.go
promoted: false
---

# Nullable config leaves need presence-aware merge

## Observation
Both config writers treat null as unset during deep merge, but agents.managerContextWindows uses null as a durable provider-default choice. The TS and Go writers must restore explicitly present nullable leaves after the ordinary merge, and spawn request decoding must retain a separate presence bit so omitted and JSON null remain distinct.

## Impact
Without this, selecting provider-default is silently rewritten to the previous value or the fresh Codex 1M fallback, and desktop/Hub diverge.

## Recommendation
For future nullable config leaves, add the same post-merge/presence handling to both writers and pin it with a cross-language round-trip fixture.
