---
title: Native workflow controls preserve drafts and provider identities across secondary views
date: 2026-09-27
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - apps/native/src/features.rs
  - apps/native/src/ui/features.rs
  - apps/native/src/controller.rs
  - services/hub/cmd/brain/handlers.go
promoted: false
---

# Native workflow controls preserve drafts and provider identities across secondary views

## Observation
Native secondary views use keyed, cancellable reads and session-bound upload receipts; mutations are never replayed. Session names and archives are local preferences scoped by hub endpoint. Resume must preserve its ID through Agent setup and seed or explicitly clear the model/context pair rather than inherit an unrelated form selection. Model picker UI must sync independently of catalog changes. Literal question labels and typed numeric answers require answerKinds=text through embedded and Go hub forwarding; bare answers are legacy numeric option guesses. History chunks full display text before applying UI bounds so earlier portions of long messages remain reachable. Clipboard TIFF/BMP conversion runs off UI with decode bounds; Windows bitmap-only clipboard uses arboard, and text paste must bypass that fallback. Windows toast identity requires the native AUMID registered by NSIS. A fresh independent reviewer identified these cases; regression tests and a final source review cover the fixes. This installed Rivet CLI witness run executes commands, unlike the AGENTS tool description; it produced invalid root-relative Go and Jest invocations. Run native full ui-tests and affected Go packages from their module directly.
