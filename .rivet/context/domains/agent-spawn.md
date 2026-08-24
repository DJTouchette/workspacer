---
title: Agent Spawn (two transports)
tags: [spawn, agents, providers, ipc, hub-bus, permissions, tool-tiers, federation]
related_paths:
  - "apps/desktop/src/main/services/managedSpawn.ts"
  - "apps/desktop/src/main/services/claudeSpawn.ts"
  - "apps/desktop/src/main/ipc.ts"
  - "apps/desktop/src/main/services/hubCapabilities.ts"
  - "services/hub/cmd/brain/handlers.go"
owner: Damien Touchette
last_reviewed: 2026-08-16
---

# Agent Spawn (two transports)

## Overview

Agents spawn via two independent transports: Electron IPC (`claude:spawn`) and hub event bus (`agents.spawn`). Both dispatch to the same shared helpers on the desktop (TypeScript), but the hub (Go) maintains a parallel independent implementation. The core rule: provider dispatch, permission defaults, and MCP/profile handling must stay identical across both paths, or a spawn started via web/remote/MCP will silently diverge from one started locally (e.g., Codex agent arriving as Claude, chosen MCP servers dropped, permission mode mismatch).

## Key modules

- `apps/desktop/src/main/ipc.ts` — IPC handler for `claude:spawn` (~line 160). Routes to either `spawnClaudeAgent` (PTY) or `spawnManagedAgent` (stream/Codex/OpenCode/Pi) after resolving transport default.
- `apps/desktop/src/main/services/managedSpawn.ts` — Tier-2 managed-provider dispatch (Codex/OpenCode/Pi/Claude-stream). Shared by both IPC and hub transports so they can't drift. Pre-registers session metadata + MCP config before daemon sees it.
- `apps/desktop/src/main/services/claudeSpawn.ts` — Tier-1 Claude PTY spawn. Shared by both IPC and hub. Mirrors profile/MCP/permission resolution exactly so a spawn respawned from web arrives identical.
- `apps/desktop/src/main/services/hubCapabilities.ts` — Hub capability `agents.spawn` (~line 214). Security gate strips permission bypasses for remote callers, then delegates to the same `spawnManagedAgent` / `spawnClaudeAgent` helpers used by IPC.
- `services/hub/cmd/brain/handlers.go` — Headless hub daemon brain. `spawn()` (~line 347) and `spawnManagedSession()` (~line 446) are independent Go rewrites of the desktop logic. Must stay in sync: same provider routing, permission defaults, profile env/argv, metadata recording.

## Failure modes

- **Provider mismatch** — If hub's `spawn()` or `spawnManagedSession()` lags behind desktop's routing logic (e.g., a new provider added), remote spawn silently falls through to Claude (the default).
- **MCP library loss** — Historically, hub's spawn ignored `mcpItemIds`, so remote spawns lost chosen MCP servers; fixed by routing both transports through the same shared helpers.
- **Permission bypass leak** — Remote/web caller passes `skipPermissions=true` or `permissionMode='bypassPermissions'`. Desktop clamps it (line 257 in hubCapabilities.ts, line 360 in handlers.go), but only if both sides recognize the rule. Missing the clamp on one side = auto-approval remotely.
- **Profile/env duplication** — Claude profile (CLAUDE_CONFIG_DIR + extraArgs) must ride the spawn payload and be applied by both paths. If one path skips the profile, spawned Claude lacks its custom config.

## Gotchas

- **Two-implementation synchronization hazard**: Desktop TS (managedSpawn/claudeSpawn) and hub Go (handlers.go) are separate implementations, not shared code. A spawn bugfix or provider addition MUST be applied to both. No shared integration test catches divergence.
- **Bypass stripping scope**: Permission bypass clamping happens at the *hub-capability* boundary (hubCapabilities.ts line 257, handlers.go line 360), BEFORE the shared helper is called. If a new spawn path bypasses this clamp (e.g., calling spawnManagedAgent directly from somewhere else), that path will honor the bypass.
- **Session metadata pre-registration**: Both paths call `claudeSessionStore.setSpawnMeta()` or `r.meta.set()` BEFORE the daemon session starts. If this step is skipped, the card never picks up label/parent/supervisor until a delta arrives; the card sits in "connecting" state or shows no metadata.
- **Transport default resolution**: `opts.transport ?? config.claude.transport ?? 'pty'` is the tiebreaker. Both paths must apply this same priority (IPC line 218, hub line 390). Missing it = stream spawn may silently fall back to PTY.
- **Exhaustive `provider` cases**: Managed vs. PTY dispatch must be exhaustive. Desktop (ipc.ts ~line 190) checks `provider !== 'claude'` for managed, then resolves transport. Hub mirrors this (handlers.go ~line 381). Adding a new provider without updating both is a silent no-op.
- **Default permission modes differ by family**: Claude defaults to `'default'` (PTY or stream); managed providers default to `'ask'`. The distinction lives in managedSpawn.ts lines 143–150 (TS) and handlers.go lines 482–490 (Go). Swapping or omitting this mapping breaks approval flow.

## Hand-authored notes (2026-08-16) — tool tiers, plugin tools, targetHub, profile scrub

- **New spawn options `toolScope: 'view'|'triage'|'operator'` and `pluginTools: string[]`** ride both transports (ClaudeSpawnOptions/ManagedSpawnOptions, `ipc.ts` `claude:spawn`, `hubCapabilities.ts` `agents.spawn` ~L263/355/383) plus the facade's own `spawn_agent`. When set, the desktop mints a per-session scoped facade token (`remoteTokens.ts` `mintSessionFacadeToken`, label `session:<id>`, revoked at store eviction, swept at boot, hidden from the pairing UI) and writes it into `session-mcp/<id>.json` (0600, Authorization header for claude; `?t=` URL for codex/opencode; pi gets nothing). Spawn-param sync is now a FOUR-place concern: desktop TS helpers, hub-capability boundary, brain Go, and the facade's `spawn_agent` — the brain deliberately **declines** `toolScope`/`pluginTools` (documented in `services/hub/cmd/brain/parity_test.go` `spawnParamsDeclined`: headless cannot mint the token). See `modules/mcp-tool-facade.md`.
- **Spawn dialog**: Advanced grew a "workspacer" tier select + per-plugin tool pills (fed by `listHubPlugins` manifests' `tools`), with a deviations chip; `toolScope`/`pluginTools` persist on `AgentWorkspace` and re-apply on respawn. Library-MCP selection is dropped (with a visible hint) when a tier is set. The supervise skill now spawns summarizer workers with `toolScope: "view"` (`supervisorSkill.ts` ~L73).
- **`targetHub` routes a spawn to a federated peer**: Machine picker in the spawn dialog → `useAgentManager` → `ipc.ts` calls `hub:<peer>/agents.spawn` (pinned by `ipcFederationRouting.test.ts`); worktree creation is skipped when `targetHub` is set (the worktree would be on the wrong machine). The workspace records `hub` so respawn re-routes. See `modules/hub-federation.md`.
- **Profile-based permission bypass is scrubbed in the helpers, not just the boundary** (security fix 2026-07-30): `agents.spawn` clamped `skipPermissions`/`permissionMode` on the request but passed `profileId` through, and a bus caller could create a profile whose `extraArgs` carry `--dangerously-skip-permissions` (`claude.profiles.add` is itself a capability). The Go brain already scrubbed (`profiles.go` `scrubBypassArgs`); the desktop now ports it as `scrubBypassArgs`/`scrubBypassProfile` in `claudeProfiles.ts` (tested by `scrubBypass.test.ts`), applied via the `scrubProfileBypass` option honored by BOTH `claudeSpawn.ts` and `managedSpawn.ts` — the boundary decides, the helper enforces, so a future spawn entry point can't forget.

## Hand-authored notes (2026-08-22/23) — full-access mechanics, cwd validation, worktree deps

- **Supervisor full-access is spawn-time-mechanical; manager/fleet full-access is dispatch-time-mechanical — different mechanisms for a similar-sounding flag.** `claudeSpawn.ts`/`managedSpawn.ts` both compute `skipPermissions = !!opts.skipPermissions || supervisorFullAccess`, where `supervisorFullAccess` reads `config.supervisor.fullAccess` live — this mechanically forces `skipPermissions`/`bypassPermissions` on *every* spawn when the flag is set, not just when a caller passes it explicitly. The Fleet Manager's equivalent (`agents.fleetFullAccess` / per-project yolo) is NOT applied to the manager's own spawn opts at all — it rides as a grant on the manager's session facade TOKEN (`mintSessionFacadeToken` via `managerFullAccessFromConfig()`), applied live by the MCP facade when the manager DISPATCHES a worker, re-read per request via `reconcileSessionFacadeGrants`. Grepping `claudeSpawn`/`managedSpawn` alone for "does fleetFullAccess mechanically add skipPermissions" only finds the supervisor path and wrongly concludes the manager path is unwired — it's wired one layer up, at dispatch time.
- **A spawn `cwd` that fails to `chdir` produces a live-looking, already-dead card, not a spawn error.** `POST /sessions/spawn-managed` registers the session id and answers 200 BEFORE the provider child actually launches (`providers/claude_stream.rs` `spawn_session` runs `run_session` in a background task and only warn-logs on failure, then flips the row to Stopped) — so a bad cwd mints a normal agent card, and every subsequent message gets `409 session has ended and cannot accept chat input`. A leading `~` is the realistic trigger: `main/lib/spawnCwd.ts` `normalizeSpawnCwd` deliberately does NOT expand it (twin of the brain's `normalizeCwd`, pinned by `contracts/path-containment-cases.json`) — correct for bus-supplied paths, but `agents.fleetRoot` is a free-text Settings field where a person can literally type `~/`. Two-layer fix: (1) tilde expansion belongs where the USER's config is read, not at the spawn boundary — `fleetManager.ts` `expandHome` inside `deriveFleetRoot`, leaving `normalizeSpawnCwd`'s contract untouched; (2) `assertSpawnCwd` (`main/lib/spawnCwd.ts`) pre-flights the resolved cwd in `spawnClaudeAgent`/`spawnManagedAgent`/`spawnCodexHybrid`, raising a `notifySystem` banner and throwing before a session id ever exists. Any new spawn entry point must call it after cwd resolution. Remaining gap: claudemon itself still accepts an unusable cwd for non-desktop clients — a 400 in `handle_managed`/`handle` would be the backstop.
- **Worktree `node_modules` auto-linking now recurses to any depth** (`worktreeService.ts` `discoverNodeModules`), fixed from a depth-≤2 cap that missed anything nested deeper than `<a>/<b>/node_modules` — this repo's own `apps/desktop/src/renderer/node_modules` is 4 segments down and was silently skipped, forcing a hand-symlink in every fresh agent worktree. The walk still never descends into a `node_modules` directory itself (a nested dependency's own copy isn't walked) or into dot-directories, so cost stays bounded (~3900 dirs / ~17ms on this repo). Prefer widening this kind of depth handling in code over adding a per-project `worktreeSetup` symlink command in config — a config-only fix wouldn't help a fresh clone, and this repo's `config.yaml` has a documented history of being clobbered.
