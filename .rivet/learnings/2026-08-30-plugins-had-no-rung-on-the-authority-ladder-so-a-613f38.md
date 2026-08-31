---
title: Plugins had no rung on the authority ladder, so agents.spawn consent minted operator-tier children
date: 2026-08-30
confidence: high
suggested_doc: hub-plugin-system
related_paths:
  - services/hub/internal/plugin/manifest.go
  - services/hub/internal/plugin/grantpin.go
  - services/hub/internal/bus/rpc.go
  - services/hub/internal/bus/bus.go
  - apps/desktop/src/renderer/src/lib/pluginPermissions.ts
promoted: false
---

# Plugins had no rung on the authority ladder, so agents.spawn consent minted operator-tier children

## Observation
callerToolScopeCeiling() returned "" for a plugin conn, and "" meant NO CEILING — so a plugin consented only to call agents.spawn could pass `mcpFacade: true` (legacy spelling of OPERATOR) and receive a child holding the full first-party tool set, beyond anything its own bus token held. Fixed with a manifest field: `{"method":"agents.spawn","childToolScope":"view|triage|operator"}`, carried through plugin.Capability -> capspec.Grant.ChildToolScope -> bus capGrant.childToolScope. callerToolScopeCeiling now returns (max, mayDelegate); absent grant = mayDelegate false = toolScope, mcpFacade AND pluginTools all stripped (all three, or the legacy boolean walks around the clamp). It is consent-pinned in grantpin.go narrowToPin, so a sidecar cannot add or widen it by rewriting the plugin.json inside its own sandbox write root. Disclosed as its own sensitive line in the renderer's pluginPermissions.

## Impact
Catalog audit at the time of the fix: of 20 manifests in ../workspacer-plugins, only escalation-chains, second-opinion and jira declare agents.spawn, and NONE passes mcpFacade/toolScope/pluginTools — their spawns carry only cwd/provider/model/effort/label/parentSessionId. So the fail-closed default broke nothing. A future catalog plugin wanting facade workers must add the field and be reinstalled; ../workspacer-plugins/IMPLEMENTING.md does not document it yet.
