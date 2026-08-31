---
title: A deleted `model` is not a weak model — it is the provider's own default, below the ceiling's view
date: 2026-08-30
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/hub/internal/routing/ceiling.go
  - services/hub/internal/bus/rpc.go
  - apps/desktop/src/main/services/configDefaults.generated.ts
promoted: false
---

# A deleted `model` is not a weak model — it is the provider's own default, below the ceiling's view

## Observation
The routing ceiling clamped `capability` and DELETED `model`/`effort`, reasoning that keeping the refused model would be a relabelling. But an omitted model resolves to the provider's own configured default one layer below the gate — and the desktop Claude default is `opus[1m]`, a string the routing matrix never mentions (the matrix says `opus`), so capabilityOfModel cannot judge it either. The clamp therefore relabelled rather than limited. Fixed by having routing.CeilingVerdict carry a SAFE ROUTED TUPLE (provider/model/effort for the permitted capability) that the bus WRITES onto the params. routeSafely constrains to the provider the spawn named, else the provider the matrix associates with the model it named (`model: fable` means claude even with no provider field), else the active profile; a provider the matrix has no profile entry for (copilot/opencode/pi) gets NO substitution, because re-routing onto a different harness is a bigger surprise than the ceiling is entitled to.

## Impact
Any clamp in this repo that removes a field rather than replacing it should be checked for the same shape: the layer below may fill the hole from config, out of the gate's sight. Note `opus[1m]` vs `opus`: the matrix's model ids do not match the desktop config default string, so the named-model ceiling arm never judges the shipped default model at all.
