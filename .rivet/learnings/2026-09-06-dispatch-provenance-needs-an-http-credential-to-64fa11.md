---
title: Dispatch provenance needs an HTTP credential to desktop persistence fixture
date: 2026-09-06
confidence: high
suggested_doc: agent-spawn
related_paths:
  - apps/desktop/tests/integration/dispatchChain.integration.ts
  - services/hub/cmd/mcp/dispatch_chain_fixture_test.go
promoted: false
---

# Dispatch provenance needs an HTTP credential to desktop persistence fixture

## Observation
An operator session token sent through the real MCP HTTP gate acquires callerSessionID provenance and crosses the trusted facade bus connection into the production desktop handler and history store. The identical scoped token sent directly to the bus loses that provenance. A host-token connection with peer=1 isolates the federation downgrade arm; removing only caller.federated from the provenance strip condition makes the integration case fail while the other seven cases pass.

## Impact
Separate facade and handler tests cannot establish the credential-to-owner-to-persisted-task chain, and a scoped federation fixture can accidentally pass through the scoped strip arm without testing federation at all.

## Recommendation
Run npm --prefix apps/desktop run test:dispatch-chain (wired into desktop CI). Keep the real HTTP gate, bus, desktop hubClient, session and history stores; mock provider launch only, with a headless Electron shim. Do not replace the positive path with a fake build.caller or caller-supplied owner stamp.
