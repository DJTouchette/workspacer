---
title: Federated remote spawn requires explicit remote cwd
date: 2026-09-10
confidence: high
suggested_doc: hub-federation
related_paths:
  - apps/desktop/src/main/ipc.ts
  - apps/desktop/src/renderer/src/hooks/useAgentManager.ts
  - services/hub/internal/federation/federation.go
  - services/hub/cmd/mcp/federated.go
promoted: false
---

# Federated remote spawn requires explicit remote cwd

## Observation
Federation routes targetHub spawns as hub:<peer>/agents.spawn and deliberately skips local worktree creation. The cwd is interpreted and must be valid on the peer; a local desktop path is not transferable. The same qualified capability can carry a first message and returns messageQueued, but is distinct from the remote-client/web access path.

## Impact
Managers must use a peer-owned repository path and an operator-authorized federated link; successful remote shell/PWA pairing alone does not prove agent dispatch.

## Recommendation
When documenting or implementing cross-hub manager dispatch, make target hub, remote cwd, operator token tier, and first-message/wake behavior explicit; never map local paths implicitly.
