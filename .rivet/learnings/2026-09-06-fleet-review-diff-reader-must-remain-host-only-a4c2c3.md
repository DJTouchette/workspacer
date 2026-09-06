---
title: Fleet review diff reader must remain host-only
date: 2026-09-06
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/renderer/src/backend/bridgedBackend.ts
  - apps/desktop/src/main/preload.ts
promoted: false
---

# Fleet review diff reader must remain host-only

## Observation
The Fleet review renderer needs the optional preload-only htmlCardReadDiff bridge, which has no unchecked bus fallback; bridgedBackend must list it in HOST_ONLY so remote backends safely expose it as unsupported.

## Impact
Omitting the capability makes backend construction drop the optional preload API and leaves local inline review unable to open retained diffs.

## Recommendation
When adding optional preload-only renderer APIs, update bridgedBackend HOST_ONLY and assert local functionality plus remote unsupported behavior.
