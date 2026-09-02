---
title: Requested Claude model is the only current 1M window carrier
date: 2026-08-30
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/desktop/src/main/shared/modelContextWindows.ts
  - services/claudemon/src/session/windows.rs
  - services/hub/cmd/brain/windows.go
  - services/hub/internal/routing/modelid.go
  - contracts/model-context-windows.json
promoted: false
dropped: true
dropped_reason: superseded — the requested model string is no longer the only 1M carrier. Since 66c842df / 73af1d02 / 4d4b8e9b, claudemon publishes requested_selection {model, context_window} and resolved_context_window, the brain projects both spellings, and the desktop, /app, /m and wks-tui all forward them. The durable half (Claude Code strips [1m] from the transcript model id, so a request is the only carrier until a provider reports a window) is already recorded in contracts/model-context-windows.json's markerCases block, in modules/claudemon-sqlite-store.md, and in the corrected note now in domains/session-lifecycle.md. Nothing left to promote.
---

# Requested Claude model is the only current 1M window carrier

## Observation
Across desktop, claudemon, hub brain, and contracts, `[1m]` remains embedded in requested model strings while actual transcript model ids are stripped. Context-limit derivation currently falls back to 200k until a provider window, requested marker, or >200k high-water signal reaches the respective implementation.

## Impact
Separating model identity from selected context window must preserve the selected value through every spawn/session/federation wire path; waiting for transcript usage cannot accurately represent a new 1M session.

## Recommendation
Define one canonical parse/normalize boundary at config and inbound wire edges, transport base model and contextWindow separately, and retain only legacy parsing adapters.

## Disposition
Dropped, not promoted. See `dropped_reason` above; verified against master `0bac5799`.
