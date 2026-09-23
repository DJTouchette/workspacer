---
title: Transcript indexes require safe earlier-turn reconciliation
date: 2026-09-23
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/desktop/src/renderer/src/lib/conversationIndex.ts
  - apps/desktop/src/renderer/src/panes/ClaudePane.tsx
promoted: false
---

# Transcript indexes require safe earlier-turn reconciliation

## Observation
Renderer conversationIndex.ts compares all JSON-shaped turn/tool fields across transport clones, preserving unchanged turn and tool-array identity; it does not assume only the tail changes. The indexer walks identities once and incrementally updates user-send counts and duplicate-aware tool-ID membership. It preserves tool and orchestration indexes through text-only streaming; earlier corrections, tool completions, prepends, trims and restart arrays invalidate affected contributions. ClaudePane uses authoritative counts plus conversationUserOffset for optimistic acknowledgements and separate displayed-turn indexing for live tools/anchors. anchorWork accepts optional preindexed orchestration calls while retaining the original fallback API.

## Impact
Avoids repeated full tool-history scans and message reconciliation without suppressing corrections or breaking optimistic sends after compact/full switches.

## Recommendation
Use reconciliation only within one session and conversation offset; transport comparison remains proportional to retained history until a versioned delta protocol exists. Keep correction/reset/duplicate-ID regression tests when changing the index.
