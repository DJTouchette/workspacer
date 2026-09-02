---
title: Current 200k symptom is legacy-marker loss, not high-water promotion
date: 2026-08-30
confidence: high
suggested_doc: usage-accounting
related_paths:
  - apps/desktop/src/main/services/hubCapabilities.ts
  - apps/desktop/src/main/services/sessionStore/usageAccumulator.ts
  - apps/desktop/src/main/services/modelUsage.ts
  - services/claudemon/src/session/windows.rs
promoted: true
promoted_to: usage-accounting
---

# Current 200k symptom is legacy-marker loss, not high-water promotion

## Observation
The present shared resolver already uses explicit provider/override/requested-marker/table precedence and treats excess context only as a drift disqualifier. `emptyUsage()` is nullable. A 1M session can nevertheless publish 200k while its raw provider status line reports 200k; bus fallback only prefers the resolver after observed occupancy exceeds the inaccurate provider claim. Before then it has no independent contextWindow selection field to override that raw status line.

## Impact
Moving the selected context window to an explicit session/wire field fixes the early 200k display only if bus/federation presentation ranks that requested selection ahead of a known-bad 200k status line for a 1M-configured session.

## Recommendation
Make the explicit selection part of the resolver input and publish resolved contextLimit rather than raw statusLine window; add regression cases below 200k and above 200k.

## Disposition
Promoted into `.rivet/context/domains/usage-accounting.md` as the SHIPPED rule, not the proposal it was written as: the recommendation ("publish resolved contextLimit rather than raw statusLine window") has since landed as `busContextLimit` plus the daemon-owned `resolvedContextWindow`. Promoting it also forced a CORRECTION of that doc's standing gotcha, which still described the old high-water 1M promotion heuristic; occupancy is now only a drift disqualifier and unknown stays `null`.
