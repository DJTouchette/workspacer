---
title: Headless brain silently dropped agents.spawn's template/templateParams, causing false-success dead spawns
date: 2026-08-28
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/hub/cmd/brain/handlers.go
  - services/hub/cmd/brain/parity_test.go
  - services/hub/cmd/mcp/main.go
promoted: false
---

# Headless brain silently dropped agents.spawn's template/templateParams, causing false-success dead spawns

## Observation
The brain's spawnParams struct (services/hub/cmd/brain/handlers.go) has no Template/TemplateParams fields by design (documented as declined in parity_test.go's spawnParamsDeclined map, because rendering a template also needs the resultSchema machinery the brain already declines). But nothing in handlers.go actually enforced the decline at runtime — json.Unmarshal just silently dropped the unknown `template`/`templateParams` keys, so spawn() proceeded with an empty p.Message. The spawn still succeeded (sessionId returned, no error), spawnResult omitted `messageQueued` since message was empty, and the new session started with no first message and died almost immediately — a manager dispatching with a template got a plausible-looking success and would wait forever for a completion wake that could never arrive. cmd/mcp/main.go's spawnAgentIn.Template/TemplateParams are also purely opaque passthrough fields (never referenced anywhere in that package) — the MCP facade relies entirely on the backend (hubCapabilities.ts on desktop, or the brain) to render or refuse them.

## Impact
Any headless/bus-driven Fleet Manager dispatch using a dispatch template against a brain-only node (no desktop running) silently lost the task and looked like a hung worker, not a failed spawn.

## Recommendation
When a spawnParamsDeclined entry exists for a param, verify there's also a runtime check that refuses the call with a clear error — the parity_test.go guard only checks that the param is documented as declined and absent from the Go struct tags, it does NOT check that the brain actually errors on receiving it. Fixed by adding rejectDeclinedTemplateParams() (probes the raw JSON for template/templateParams presence before the struct is populated) called at the top of registry.spawn().
