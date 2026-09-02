---
title: Managed Codex has a second defaulting ingress
date: 2026-08-31
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/src/main/shared/providerContext.ts
  - apps/desktop/src/main/services/managedSpawn.ts
  - services/hub/internal/modelselection/modelselection.go
  - services/claudemon/src/daemon/spawn.rs
  - contracts/model-context-windows.json
promoted: true
promoted_to: agent-spawn
---

# Managed Codex has a second defaulting ingress

## Observation
Fresh model-less Codex defaults must be applied before managed model normalization at both host layers and claudemon's /sessions/spawn-managed endpoint: ResolveInput deliberately permits a Codex contextWindow with no model, while resumes must remain nil when no persisted request exists.

## Impact
Defaulting only after optional model selection drops the normal model-less Codex request; defaulting indiscriminately upgrades pre-feature resumes.

## Recommendation
Keep provider-level fresh-spawn policy separate from model-window lookup and test both fresh model-less and legacy-resume cases.

## Disposition
Promoted into `.rivet/context/domains/agent-spawn.md`. Re-verified: `modelselection.ResolveInput` still permits a `codex` contextWindow with no model and rejects it for every other provider, and claudemon's `POST /sessions/spawn-managed` still applies the fresh-life default itself. Unchanged.
