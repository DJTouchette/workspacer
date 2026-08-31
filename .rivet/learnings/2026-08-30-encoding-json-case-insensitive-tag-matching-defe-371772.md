---
title: encoding/json case-insensitive tag matching defeats any exact-key bus sanitizer
date: 2026-08-30
confidence: high
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub/internal/bus/spawnkeys.go
  - services/hub/internal/bus/rpc.go
  - services/hub/cmd/brain/handlers.go
  - apps/desktop/src/main/services/hubCapabilities.ts
promoted: false
---

# encoding/json case-insensitive tag matching defeats any exact-key bus sanitizer

## Observation
internal/bus sanitizeSpawnParams decides on EXACT lower-camel keys (yoloGranted, profileId, mcpFacade, toolScope, capability, model, effort) in a map[string]json.RawMessage. Every provider decoding that map uses encoding/json, which binds struct tags CASE-INSENSITIVELY. So `{"YoloGranted":true}` survived a sanitizer that deletes `yoloGranted` and bound to cmd/brain spawnParams.YoloGranted anyway — a full bypass of the full-access stamp, the tool-tier clamp and the capability ceiling from one capital letter. cmd/brain's rejectCaseVariantKeys did NOT catch it: it only fires when BOTH spellings are present. Introduced 5d7b7281 (2026-08-20, shipped v0.150.0) for yoloGranted and ccb44ccf (2026-08-27, shipped v0.160.0) for mcpFacade/toolScope — i.e. it is in published releases, not new to the routing work. Fixed at the bus boundary in internal/bus/spawnkeys.go: a key that case-folds to a known spawn param without being spelled as one REFUSES the call, and paramSanitizer now returns an error so both call() and federatedCall() can fail the frame.

## Impact
Any future guard in this repo that matches JSON field names exactly, in front of a Go decoder, has the same hole. The canonical key list in spawnkeys.go is the gate: a provider field missing from it reopens the bypass for that field, silently, because the spawn still works. cmd/brain/spawnkeydrift_test.go reflects over spawnParams AND parses hubCapabilities.ts's destructure block to make that drift a test failure.

## Recommendation
When adding a field to any agents.spawn surface (cmd/brain spawnParams, hubCapabilities.ts, cmd/mcp spawnAgentIn), add its canonical spelling to spawnParamKeys in services/hub/internal/bus/spawnkeys.go. More generally: never write an exact-key allow/deny check in front of encoding/json without a canonicalization gate in front of it.
