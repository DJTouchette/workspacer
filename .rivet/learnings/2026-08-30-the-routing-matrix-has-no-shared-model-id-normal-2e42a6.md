---
title: The routing matrix has no shared model-id normalizer; the [1m] window suffix is stripped only at the ceiling lookup
date: 2026-08-30
confidence: high
related_paths:
  - services/hub/internal/routing/modelid.go
  - services/hub/internal/routing/ceiling.go
  - services/hub/cmd/brain/windows.go
  - services/hub/cmd/brain/models.go
  - apps/desktop/src/main/shared/modelContextWindows.ts
  - apps/desktop/src/main/services/claudeModels.ts
  - contracts/model-context-windows.json
promoted: false
---

# The routing matrix has no shared model-id normalizer; the [1m] window suffix is stripped only at the ceiling lookup

## Observation
`opus[1m]` is claude opus with a 1M context window, not a different model, and the repo already treats that mapping as data in contracts/model-context-windows.json plus its three twins (modelContextWindows.ts, windows.rs, cmd/brain/windows.go). But those twins only answer "what window does this string imply". NONE of them splits a model id from its window suffix. The two places that do strip it are inline expressions, not reusable helpers: services/hub/cmd/brain/models.go:49 (strings.ReplaceAll inside package main, unreachable from internal/routing) and apps/desktop/src/main/services/claudeModels.ts:32 (id.replace inside a non-exported parseConcreteId). Both know only the `[1m]` spelling; neither knows the `-1m` spelling the contract also lists. So services/hub/internal/routing/modelid.go is a fourth, deliberately local stripper, pinned to the contract's `windows` block by a membership test rather than sharing code.

The only two places in internal/routing that compare a caller-supplied model to a profile entry are capabilityOfModel and providerOfModel, both in ceiling.go. routing.Request carries no Model field at all, so routing.select never compares one; policy.go touches Model only when copying the resolved assignment out.

A second consequence nobody has written down: a profile entry spelled `model: "opus[1m]"` must be QUOTED in YAML flow style, because `[` opens a flow sequence and `{ provider: claude, model: opus[1m] }` is a parse error.

## Impact
A future session looking for "the canonical model-id parse" will find the window tables and assume they cover splitting. They do not. Anyone adding a fifth model comparison in the hub must normalize it themselves or reproduce the gap where the desktop's own shipped claude.defaultModel is the one model the ceiling cannot read.

## Recommendation
Normalize only for the COMPARISON. The suffix is a context-window request and must reach the provider intact: strip it from what is forwarded and every dispatch silently drops from 1M to 200K with no symptom until an agent runs out of room. On the ceiling's substitution path the replacement takes the matrix entry verbatim, so it runs the window its own entry implies rather than inheriting the refused request. If the model-id paradigm is ever refactored to separate model from context length, modelid.go is one of the four call sites to fold in.
