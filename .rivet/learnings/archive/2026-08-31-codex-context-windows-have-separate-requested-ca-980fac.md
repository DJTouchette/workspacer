---
title: Codex context windows have separate requested, canonical, and observed values
date: 2026-08-31
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - apps/desktop/src/main/shared/modelContextWindows.ts
  - apps/desktop/src/main/shared/canonicalSelection.ts
  - apps/desktop/src/main/services/managedSpawn.ts
  - services/claudemon/src/providers/codex.rs
promoted: true
promoted_to: claudemon-providers
---

# Codex context windows have separate requested, canonical, and observed values

## Observation
Codex context-window handling spans three non-interchangeable values: requestedSelection (user intent), resolvedContextWindow (canonical model/catalog resolution), and statusLine.contextWindowSize (runtime report). The shared model/window table and canonical selection utilities feed spawn/store, whereas the renderer consumes the status line for usage. Do not make a picker option from a catalog capacity unless the Codex adapter has a verified spawn/config control for it.

## Impact
Conflating these values can display a model's nominal 1.05M capacity as an honored runtime allocation even when the harness reports a much smaller active window.

## Recommendation
For Codex, display the live reported operational window separately from official capacity and only offer a selector after tracing a supported Codex argument or RPC setting.

## Disposition
Merged with the 258.4K learning into the same `claudemon-providers.md` section. CORRECTED: the recommendation was to offer a selector "only after tracing a supported Codex argument or RPC setting" — that argument was traced and the control shipped, so the surviving rule is the narrower one (never promote a catalog capacity to a picker option on its own; show the live reported window separately from official capacity).
