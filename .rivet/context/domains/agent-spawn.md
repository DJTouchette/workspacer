---
title: Agent spawning and ordinary-agent collaboration
tags: [spawn, agents, providers, ipc, hub-bus, federation, skills, facade]
related_paths:
  - "apps/desktop/src/main/services/managedSpawn.ts"
  - "apps/desktop/src/main/services/claudeSpawn.ts"
  - "apps/desktop/src/main/services/agentCollaborationSkills.ts"
  - "apps/desktop/src/main/services/claudeSessionStore.ts"
  - "services/hub/cmd/brain/handlers.go"
  - "services/hub/cmd/brain/agent_collaboration_skills.go"
owner: Damien Touchette
last_reviewed: 2026-09-16
---

# Agent spawning and ordinary-agent collaboration

## Current contract

Electron IPC (`claude:spawn`) and the hub method `agents.spawn` both launch the
same provider families. Desktop entry points converge on `claudeSpawn.ts` and
`managedSpawn.ts`; the headless brain has a parallel Go implementation in
`handlers.go`. Keep provider, transport, model, profile, MCP, first-message and
parent metadata parity across both implementations.

Authenticated spawned sessions receive ambient Workspacer tools and the tools of
every enabled plugin. `toolScope`, plugin-tool selections, profile grants,
`fleetFullAccess`, project `yolo` and related legacy grant fields remain
parse-compatible where required but do not narrow or widen the session facade.
The provider's native permission mode is provider configuration, not a
Workspacer filesystem or plugin grant.

Each supported session receives a lifecycle-bound identity bearer. The desktop
and headless launchers verify the exact MCP facade health before minting and
injecting it. The bearer records session identity/role for provenance and parent
routing; it is not a directory allowlist. It is revoked after the session ends,
with duplicate stopped observations retrying a failed persistent revocation.

## Ordinary-agent skills

Ordinary agents receive pointer-only instructions for two versioned skills
installed beneath `<cwd>/.workspacer/skills/<content-hash>/`:

- `spawn-agent` teaches an ordinary agent to create child sessions and preserve
  its own parent lineage.
- `project-brief` teaches it to keep `.workspacer/brief.md` current through the
  atomic brief tools.

Desktop and headless launch use the same generated assets and hash. Managers
keep manager doctrine instead of these pointers. Pi does not receive them because
that harness has no equivalent skill-loading contract.

## Completion routing

Child completion, blocker and meaningful-progress wakes route to the direct
parent session. A regular parent does not masquerade as a Fleet Manager.
Manager request capture/workflow lineage remains manager-specific; ordinary
parent wakes are collaboration events and do not create manager inbox requests.

## Spawn invariants

- The first user message rides in the spawn request and is queued before the
  spawn response. Do not regress to spawn-then-send.
- Pre-register label, parent, role, hub and workflow metadata before the daemon
  snapshot arrives.
- Resolve provider transport/model/context values before launch and preserve
  them through federation.
- Validate that cwd is usable before creating a session row. Authenticated
  callers may choose any absolute host directory; Workspacer does not confine it
  to live agent roots.
- `targetHub` uses the peer's exact remote cwd and skips local worktree creation.
- Plugin launch callbacks stay bound to the exact pending owner-authorized spawn
  so one plugin cannot attach output to another launch.

## Verification

Run focused desktop and headless spawn/facade tests, generated-skill parity,
desktop-host integration, and the relevant provider/transport browser flow. Test
both an ordinary child and a manager because their instructions intentionally
differ.
