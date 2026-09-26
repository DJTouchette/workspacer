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
last_reviewed: 2026-09-26
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

The desktop and headless launchers verify the exact MCP facade health before
minting and injecting a lifecycle-bound identity bearer. Desktop managed
launch waits for facade readiness; headless launch can omit the facade when
its configured endpoint is unavailable. Do not describe an omitted bearer as
a narrower tool tier. The bearer records session identity/role for provenance and parent
routing; it is not a directory allowlist. It is revoked after the session ends,
with duplicate stopped observations retrying a failed persistent revocation.

## Ordinary-agent skills

Eligible ordinary launches receive pointer-only instructions for two versioned
skills installed beneath `<cwd>/.workspacer/skills/<content-hash>/`:

- `spawn-agent` teaches an ordinary agent to create child sessions and preserve
  its own parent lineage.
- `project-brief` teaches it to keep `.workspacer/brief.md` current through the
  atomic brief tools.

Desktop and headless launch use the same generated assets and hash. Managers
keep manager doctrine instead of these pointers. Installation refuses the home
or filesystem root, symlinked destinations, and conflicting preexisting files;
a failed install returns no pointer rather than overwriting user content. Pi
is rejected by normal Workspacer spawning because it lacks the required MCP
bridge, even though its low-level adapter remains in claudemon.

## Completion routing

Child completion, blocker and meaningful-progress wakes route to the direct
parent session. A regular parent does not masquerade as a Fleet Manager.
Manager request capture/workflow lineage remains manager-specific; ordinary
parent wakes are collaboration events and do not create manager inbox requests.

## Spawn invariants

- Modern spawn entry points carry a supplied first message in the spawn
  request and expose whether it was queued before responding. Preserve that
  acknowledgement. Some existing callers, including jobs and TUI handoff, still
  spawn then send; document those callers separately rather than promising
  atomic delivery for every flow.
- Preserve label, parent, role, hub and workflow metadata across pending launch
  and live adoption; check the selected launcher's pre-registration order.
- Resolve provider transport/model/context values before launch and preserve
  them through federation.
- Desktop launchers validate cwd before publishing a session; daemon admission
  must also reject an unusable runtime cwd. Headless normalization alone is
  not a filesystem existence check. Authenticated
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

## Daemon admission

Both managed and PTY daemon spawn paths participate in execution-engine
admission and persisted generation identity. Preflight happens before runtime
publication; an incompatible pinned engine does not silently fall back. See
[provider adapters](../modules/claudemon-providers.md#admission-versus-adapter-availability)
for the difference between raw adapter support and product launcher support.

## Entry-point limits

Desktop `initialPrompt` is a composer prefill, distinct from an explicitly sent
first `message`. Peer IPC spawn forwards a selected set of provider/model/cwd/
permission/message fields; it deliberately omits local profile and MCP-item
IDs and rejects local launch integrations. It checks `messageQueued` and sends
a separate message for an older peer that did not acknowledge it. Do not infer
that every local metadata field or launch feature automatically crosses this
branch. Dedicated worker dispatch has its own origin/admission contract.

Headless ordinary-skill installation is reached through successful facade
building; a missing/unhealthy facade can omit both the bearer and that pointer.
`normalizeCwd` and the desktop twin trim ASCII whitespace, preserve non-ASCII
filename characters, and use home for empty input. Keep those semantics aligned
without treating normalization as a path authorization or existence check.

## Worktree lifetime and load evidence

Launch and artifact cleanup share a canonical per-gitdir admission fence.
Overlapping launches in one daemon reference-count the same exact owned fence;
maintenance remains exclusive. See
[cleanup and admission](../modules/worktree-artifact-cleanup.md). Automatic
cleanup runs in desktop/headless services, not only on renderer card closure.

`launch_total` and HTTP spawn acceptance do not measure provider readiness or
first output. The synthetic agent-load benchmark does not launch providers.
Session token minting synchronously rewrites its persistent token store, so
profile burst costs before changing durability or calling it a proven bottleneck.
