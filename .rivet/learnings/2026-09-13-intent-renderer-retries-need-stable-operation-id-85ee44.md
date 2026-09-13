---
title: Intent renderer retries need stable operation IDs through lost responses
date: 2026-09-13
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/renderer/src/components/IntentSources.tsx
  - apps/desktop/src/renderer/src/components/IntentEvidence.tsx
  - apps/desktop/src/renderer/src/components/IntentKnowledge.tsx
promoted: false
---

# Intent renderer retries need stable operation IDs through lost responses

## Observation
Backend intent writes fence duplicate IDs, but the evidence Git capture and Rivet document capture buttons minted new IDs on every click; a lost successful response could create duplicate durable captures on a user retry. Source publish failure also skipped rereading its durable receipt, exposing stale publish controls. Preserve claim identity in the workspace-owned draft until the request is settled, and reload durable receipts after ambiguous publish errors without automatically resending.
