---
title: Web/mobile statusLine overlay went stale while a session was running (blank model, NaN tokens)
date: 2026-08-28
confidence: high
related_paths:
  - services/hub/cmd/brain/store.go
  - services/hub/cmd/brain/enrich.go
  - apps/desktop/src/renderer/src/lib/sessionStats.ts
promoted: false
---

# Web/mobile statusLine overlay went stale while a session was running (blank model, NaN tokens)

## Observation
The hub brain's sessionStore.updateStatusLine (cmd/brain/store.go) merged high-frequency claudemon /statusline ticks into raw `status_line` only, without re-running enrich.go's camelCase `statusLine` overlay (statusLineOverlay). A turn with no tool calls generates no claudemon /events session.update — the only thing that re-runs the full enrich pass via store.set — so the camelCase `statusLine` key (which the renderer's sessionStats.ts deriveSessionStats reads exclusively) stayed stale or entirely absent for the whole running turn, only catching up once the turn ended and a real snapshot event fired. Separately, sessionStats.ts's tokens fallback computed `usage.totalInputTokens + usage.totalOutputTokens` unguarded; the brain's `usage` overlay never sets those two fields (claudemon's raw `usage` block has no cumulative counters — only `status_line` does), so `undefined + undefined = NaN`, rendered as the literal string "NaN" by fmtTokens.

## Impact
Web/mobile UI (not desktop, which reads claudemon directly) showed a blank model name and "NaN tok" in the sidebar/composer for any agent that was actively RUNNING but had made no tool calls yet in the turn; correct once the turn finished. fleetview.go's fleetSession already worked around the same staleness for the MCP agent facade by reading both spellings and preferring raw — that workaround didn't reach the bus-pushed snapshot the actual UI renders.

## Recommendation
Fixed: updateStatusLine now also calls statusLineOverlay(sl) to refresh the camel key on every tick (services/hub/cmd/brain/store.go + enrich.go). sessionStats.ts's tokens fallback now guards on usage.totalInputTokens/totalOutputTokens being defined before summing, so a missing pair degrades to undefined instead of NaN.
