---
title: Codex default must be a provider contract fixture
date: 2026-08-31
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - contracts/model-context-windows.json
  - apps/desktop/src/main/shared/providerContext.ts
  - apps/desktop/src/renderer/src/lib/sessionStats.ts
  - services/hub/internal/modelselection/modelselection.go
  - services/claudemon/src/daemon/spawn.rs
promoted: false
---

# Codex default must be a provider contract fixture

## Observation
The reviewed context-controls feature introduced DEFAULT_CODEX_CONTEXT_WINDOW independently in desktop main while the renderer and Go each retain their own Codex fallback. The existing model-context-windows contract only proves model-window table parity; provider-level defaults need a versioned fixture and consumer-specific contract tests so a literal change in one runtime fails.

## Impact
A future Codex default change can otherwise silently split spawn argv, renderer fallback display, and hub routing.

## Recommendation
Keep the Codex default in a single cross-language JSON contract and have TypeScript, Go, and Rust tests parse it and compare their runtime consumer behavior.
