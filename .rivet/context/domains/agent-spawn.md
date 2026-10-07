---
title: Agent spawning and ordinary-agent collaboration
tags: [spawn, agents, providers, ipc, hub-bus, federation, skills, facade]
related_paths:
  - "apps/desktop/src/main/services/managedSpawn.ts"
  - "apps/desktop/src/main/services/claudeSpawn.ts"
  - "apps/desktop/src/main/services/agentSkillPlugins.ts"
  - "services/hub-rs/src/services/launch_instructions.rs"
  - "apps/desktop/src/main/services/claudeSessionStore.ts"
  - "services/hub/cmd/brain/handlers.go"
  - "services/hub/cmd/brain/agent_collaboration_skills.go"
owner: Damien Touchette
last_reviewed: 2026-10-07
---

# Agent spawning and ordinary-agent collaboration

## Current contract

Electron IPC (`claude:spawn`) and the hub method `agents.spawn` both launch the
same provider families. Desktop entry points converge on `claudeSpawn.ts` and
`managedSpawn.ts`; standalone and native hosts use the owned Rust implementation
in `services/hub-rs/src/services/{spawn_plan,agent_spawn,agent_lifecycle}.rs`.
Remaining Go `handlers.go` is migration reference. Keep provider, transport,
model, profile, MCP, first-message and parent metadata consistent across owners.

Authenticated spawned sessions receive ambient Workspacer tools and the tools of
every enabled plugin. `toolScope`, plugin-tool selections, profile grants,
`fleetFullAccess`, project `yolo` and related legacy grant fields remain
parse-compatible where required but do not narrow or widen the session facade.
The provider's native permission mode is provider configuration, not a
Workspacer filesystem or plugin grant.

The desktop and headless launchers verify the exact MCP facade health before
minting and injecting a lifecycle-bound identity bearer. Desktop managed
launch waits for facade readiness. A configured Rust facade must be verified
before launch; an explicitly disabled facade remains a supported mode without
credentials. The retiring Go implementation omitted an unavailable configured
facade and continued; that is not the current Rust policy. Do not describe an
omitted bearer as a narrower tool tier. The bearer records session identity/role for provenance and parent
routing; it is not a directory allowlist. It is revoked after the session ends,
with duplicate stopped observations retrying a failed persistent revocation.

## Per-session agent skills (ordinary and manager)

Workspacer's own skills ship as two Claude-Code-layout plugins generated from
`apps/desktop/assets/skills` (membership and instruction phrases in
`assets/skills/plugins.json`): `workspacer` (spawn-agent, project-brief,
scheduled-jobs, workspacer-response-cards) for ordinary agents and
`workspacer-fleet` (standup, checkpoint, handoff, response cards) for Fleet
Managers. `gen-agent-skill-plugins.mjs` (desktop) and
`scripts/generate-rust-launch-assets.py` (hub-rs) build byte-identical bundles;
`check:agent-skill-plugins` pins TS/Rust parity and the compiled launcher.

Each launch writes the bundle once to a content-addressed
`~/.workspacer/agent-skills/<version>/` (atomic, symlink-refusing, restores
altered files) and hands its role's plugin to THAT session only:

- Claude (PTY and stream): `--plugin-dir <plugin>`; skills appear as
  `workspacer:<skill>` and the init frame lists `workspacer@inline`.
- Codex on an app-server (headless and non-Windows hybrid): claudemon
  `skill_roots` → `skills/extraRoots/set` sent right after `initialize`
  (string RPC id `workspacer-skill-roots`). The setting is server-wide, so the
  hybrid TUI attaching over `--remote` sees it too.
- Copilot, OpenCode and Codex's PTY-only rollout path: an instruction line
  naming each SKILL.md. Copilot managers additionally keep the personal
  `~/.copilot/skills` install (`managerSkills.ts`) for slash commands.

Every launch also carries one instruction line naming the skills and their
directory, which doubles as the fallback when a plugin does not load (managed
policy, `--safe-mode`, an older CLI, a failed `extraRoots/set`). Nothing is
written into the project or a harness discovery root, so managers never
discover ordinary skills (or the reverse). Manager replacement passes
`strict` and fails rather than come up without /checkpoint and /handoff. Pi
gets nothing. Twins: `agentSkillPlugins.ts` and `launch_instructions.rs`
(the latter reached through `session_facade::prepare`, after facade readiness).

Migration cleanup removes byte-identical copies older builds wrote into the
project (`.claude/skills`, `.agents/skills`, `.workspacer/skills/<hash>/`) and,
desktop only, personal `~/.claude/skills` / `$CODEX_HOME/skills`
standup/checkpoint/handoff dirs recognized by their "Workspacer Fleet Manager"
frontmatter (older builds wrote older text, so not byte-matched).

Verified CLI behaviour (Claude 2.1.286, Codex 0.159.0): Codex ignores `-c`
overrides for `plugins.*` and `marketplaces.*` (plugins need a persistent
install + config.toml), and `skills.config` does not add roots — only the
app-server's `skills/extraRoots/set` is per session. An app-server client's
`$skill` text is not expanded (that needs an explicit `skill` input item); the
listing in context is what agents get. `scripts/check-agent-skill-discovery.py`
proves both CLIs against a mock API.

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

Headless skill preparation is reached through successful facade building; a
deliberately disabled facade omits both the bearer and the skills.
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
