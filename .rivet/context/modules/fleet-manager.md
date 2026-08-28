---
title: "Fleet Manager: the delegating manager agent, its wake loop, succession and briefs"
tags: [fleet-manager, wake, supervisor-nudge, worker, parent, briefs, mcp-facade, succession, wire-format]
related_paths:
  - "apps/desktop/src/main/services/supervisorNudge.ts"
  - "apps/desktop/src/main/shared/fleetMessages.ts"
  - "apps/desktop/src/main/services/claudeSessionStore.ts"
  - "apps/desktop/src/main/services/managerSkills.ts"
  - "apps/desktop/src/main/lib/roleModels.ts"
  - "apps/desktop/src/main/lib/roleProviders.ts"
  - "apps/desktop/src/main/lib/workspacerHome.ts"
  - "apps/desktop/src/main/lib/managedSpawnOptions.ts"
  - "apps/desktop/src/main/services/briefService.ts"
  - "apps/desktop/src/main/services/thresholdWatch.ts"
  - "apps/desktop/src/main/services/progressReports.ts"
  - "apps/desktop/src/renderer/src/lib/fleetManager.ts"
  - "apps/desktop/src/renderer/src/hooks/useAgentManager.ts"
  - "apps/desktop/src/renderer/src/components/settings/SupervisorSection.tsx"
  - "services/hub/cmd/brain/handlers.go"
  - "services/hub/cmd/hub/mobile.html"
owner: Damien Touchette
last_reviewed: 2026-08-28
---

# Fleet Manager: the delegating manager agent, its wake loop, succession and briefs

## READ THIS FIRST — the naming trap

The fleet **supervisor ROLE** was deleted from the code (merged to master at
`a6ad647d`). The **Fleet Manager** is the surviving feature, and it is wired
through modules that still carry the word "supervisor" in their names. Nothing
here is dead code:

- **`apps/desktop/src/main/services/supervisorNudge.ts` is NOT the supervisor.**
  Despite the filename it is the Fleet Manager's *entire event loop*:
  worker-finished wakes (`onFinished` → `sendFinished`), the blocked-agent
  broadcast (`onBlock`/`onBlockCleared` → `broadcastBlock`), the missed-wake
  backstop (`sweepMissedFinishes`), and `reassignPendingFinish`, which
  re-addresses in-flight wakes during succession. An agent that greps for
  "supervisor" and deletes what it finds deletes the manager.
- **`isSupervisor` is a wake-eligibility flag, not a role.** Its only input is
  now `opts.manager` (`claudeSpawn.ts` and both `setSpawnMeta` calls in
  `managedSpawn.ts`). A manager IS a supervisor for wake purposes — the Go
  brain spells the same rule as `spawnParams.isWakeTarget()`, see
  `services/hub/cmd/brain/handlers.go` (~L643). **A rename to `isWakeTarget` is
  pending and out of scope**; this doc is written against the current name.
- **`ensureSupervisorHome()`** (`apps/desktop/src/main/lib/workspacerHome.ts`)
  is just `~/.workspacer`. The file's own header says so: the name is kept
  because `app.supervisorHome` is a live IPC channel and bus capability with a
  Go twin.
- **`supervisorMcpConfigPath()` / `<userData>/supervisor-mcp.json`**
  (`mcpConfig.ts`) is the shared untokened facade MCP config — legacy name.
- **`services/hub/internal/supervisor/` and `services/hub/internal/nodes/supervisor.go`
  are OS PROCESS supervision.** Unrelated namesake, untouched by any of this —
  see `modules/hub-process-supervision.md`.
- **`components/settings/SupervisorSection.tsx` is Settings → Fleet Manager.**
  Its header documents the collapse from two roles to one.
- **"Ask the Fleet" survives as a plain `toolScope: 'triage'` spawn**
  (`useAgentManager.spawnAskAgent`, `panes/AskPane.tsx`) opening in
  `~/.workspacer`. It is no longer a role and has no settings of its own.

## Overview

The Fleet Manager is ONE long-lived agent session whose job is delegation: it
inventories the projects under its root, dispatches real worker agents into
them through the workspacer MCP facade at the **operator** tier, and reports
back. Its doctrine is in `renderer/src/lib/fleetManager.ts` (`MANAGER_PREAMBLE`,
auto-sent as the kickoff message — never a composer pre-fill). Rule 2 of that
doctrine is **NEVER POLL**: the manager ends its turn after dispatching, and the
host wakes it. That wake is the feature this module exists to deliver.

## How a manager is spawned, and what makes it wake-eligible

- **Entry points.** `FleetManagerHero` on the Overview pane → `App.tsx` →
  `useAgentManager.spawnFleetManager(ask, root, …)`. It is reuse-by-name
  (`FLEET_MANAGER_NAME = 'Fleet Manager'`): a live manager is messaged, a
  stopped card is respawned/resumed, otherwise a fresh one is spawned with
  `transport: 'stream'`, `toolScope: 'operator'`, `manager: true` and
  `kickoffMessage: buildManagerKickoff(ask, fullAccess)`. The command palette
  and a bus/MCP caller (`agents.spawn` with `manager: true`) reach the same
  spawn bodies.
- **Root.** `deriveFleetRoot(agents.fleetRoot, projectCwds, home)` — explicit
  config wins, else the common parent of configured projects, else `$HOME`.
- **Harness / model / effort are resolved IN MAIN**, so they land however the
  manager starts: `resolveManagerProvider()` (`main/lib/roleProviders.ts`,
  `agents.managerProvider`, default `claude`), `resolveManagerModel(provider)`
  and `resolveManagerEffort(provider)` (`main/lib/roleModels.ts`, per-harness
  maps `agents.managerModels` / `agents.managerEfforts` — neither has a default
  entry in `config_defaults.json`, so blank means "the harness's own default").
- **`manager: true` is the ONLY thing that sets `isSupervisor`** in the spawn
  meta, on all three spawn legs: `claudeSpawn.ts` (~L186), `managedSpawn.ts`
  (~L377 managed/stream, ~L512 the codex Windows hybrid). Headless, the brain
  does the same via `isWakeTarget()`. Without it the session is a decorative
  manager: no wake ever routes to it.
- **Skills.** `installManagerSkills(provider)` writes `/standup`,
  `/checkpoint` and `/handoff` into the harness's skill dir (`~/.claude/skills`
  or `$CODEX_HOME/skills`, same SKILL.md format) and *removes* `RETIRED_NAMES =
  ['bearings', 'stow', 'supervise']`. Called from both `claudeSpawn.ts` and
  `managedSpawn.ts`, gated on `opts.manager`.
- **Grants.** `fleetFullAccess` is a record-fidelity hint on the wire; the
  token's actual yolo grant is config-resolved at mint by
  `apps/desktop/src/main/services/fullAccessGrants.ts`, which is the single
  formula and flips live in both directions.

## The wake path: a worker finishes → the manager is invoked

1. A worker session transitions working→idle. `claudeSessionStore`'s
   `nudgeParentOnFinish` fires at every ambient-transition site. It requires a
   `parentSessionId` whose session is **live and `isSupervisor`** — otherwise
   the finish is silently dropped.
2. `supervisorNudge.onFinished(session, parentId, lastReply)` skips a boot idle
   (`hasReceivedTask` — a session with no user turn was never given its task)
   and coalesces per PARENT over `COALESCE_MS = 1500`.
3. `sendFinished` re-verifies each worker against its LIVE session at delivery:
   drops one that resumed working, re-reads the reply from the live
   conversation, marks `stopped` for an ended session, derives `failed` via
   `shared/workerFailure`, validates a `resultSchema` dispatch through
   `shared/structuredResult`, and suppresses a repeat with an identical
   `reply + stopped + failed` signature (`lastReportedReply`, see
   `apps/desktop/PER_TURN_WAKE_FINDING.md`).
4. `buildFleetMessage('worker-finished', entries)` renders the text and
   `claudemonSessionClient.message(parentId, text)` injects it as an ordinary
   user turn. The GUI re-parses it into a `FleetMessageCard`.
5. **Backstop.** `claudeSessionStore.startWakeBackstop()` runs every
   `WAKE_BACKSTOP_MS = 2 min` and calls `supervisorNudge.sweepMissedFinishes`,
   which re-nudges (kind `catch-up`) any live idle `isSupervisor` session whose
   child finished more than `MISSED_WAKE_GRACE_MS = 3 min` ago and before the
   manager last acted. The dedup is implicit: the manager acting advances its
   `lastActivity` past the child's finish.

Blocks take the other path: `onBlock` debounces `BLOCK_DEBOUNCE_MS = 20 s`
(cancelled by `onBlockCleared`), then `broadcastBlock` goes to **every** live
`isSupervisor` session except the blocked one. Two more wake kinds exist:
`threshold` (`thresholdWatch.ts`, the one-shot `notify_when` a manager arms so
it never polls) and `progress` (`progressReports.ts`, the worker-initiated
`report_progress` whose recipient is host-derived from the caller's parent and
can never be named by the caller).

## `[supervisor]` is WIRE FORMAT, not UI copy

`fleetMessages.ts` is the one place the wake format lives — builder and parser
side by side. `HEADERS.blocked` is the literal string
`'[supervisor] An agent is now blocked on a decision:'`. It is **parsed**, not
merely displayed:

- `parseFleetMessage` in `apps/desktop/src/main/shared/fleetMessages.ts` — the
  canonical implementation, consumed by the desktop (`FleetMessageCard.tsx` via
  `ConversationMessage`) and by the web `/app`, which runs the same renderer.
- `services/hub/cmd/hub/mobile.html` (~L2654) — a **hand-ported copy** of the
  headers, `ENTRY_RE` and the result-block regexes for the `/m` PWA. It cannot
  import the TS module, so it drifts by hand.
- `apps/desktop/src/main/shared/fleetMessages.test.ts` — build→parse round-trip
  tests, including the legacy single-paragraph spelling at L287, which pins the
  `[supervisor]` header in a *stored transcript* format.

Renaming that prefix breaks the wire: existing transcripts stop rendering as
cards, and `/m` stops recognising blocked wakes until mobile.html is edited to
match. Change it only as a coordinated three-site edit.

## Succession and `adopt_workers`

Fleet wakes are **parent-keyed**, so replacing a manager used to orphan every
dispatch it had in flight.

- `claudeSessionStore.reparentChildren(oldId, newId)` re-points
  `parentSessionId` on live children **and** on not-yet-registered
  `spawnMeta` dispatches, then calls
  `supervisorNudge.reassignPendingFinish(oldId, newId)` so a wake still inside
  its coalesce window follows the fleet instead of being delivered to a manager
  on its way out. It refuses loudly when the successor is unknown, ended, or
  not `isSupervisor`; it skips federated (`session.hub`) rows and never lets the
  successor become its own parent. It records no lineage — `parentSessionId` is
  the routing key and nothing else.
- `claudeSessionStore.orphanCandidates()` answers "which dead parent was mine":
  tombstones (`confirmedManager: true` — the store watched an `isSupervisor`
  session die, capped at `MAX_MANAGER_TOMBSTONES = 32`) plus bare dangling
  parent ids (`confirmedManager: false`). It REPORTS; it never picks.
- Bus capabilities `agents.orphans` / `agents.reparent`
  (`hubCapabilities.ts` ~L2042/L2058, Go twin in `services/hub/cmd/brain/agentops.go`),
  exposed as the **operator-only** MCP tools `list_orphans` and
  `adopt_workers` (`services/hub/cmd/mcp/main.go` L597/L606, tier pinned by
  `services/hub/cmd/mcp/adopt_test.go`).
- The doctrine's first-turn rule: read `.workspacer/handoff.md` if present (the
  `/handoff` skill's mid-flight state, naming the predecessor's id), else call
  `list_orphans` and adopt the confirmed manager whose label and directory
  match.

## Briefs

Every project keeps `.workspacer/brief.md` (`BRIEF_RELATIVE_PATH` in
`briefService.ts`) with `## Now` / `## Direction` / `## Recently` (prepended,
newest first) / `## User` (fleet brief only). The manager's OWN fleet brief is
the same file under its cwd (normally `~/.workspacer/brief.md`) and holds
cross-project state only — it is its memory across restarts.

- `brief_append` (`briefService.ts` → `brief.append`) is the atomic
  inspect-then-edit primitive: strictly additive (one line inserted, provably
  nothing else touched), serialized by an O_EXCL advisory lock, and
  compare-and-swap against outside writers. It **refuses** an over-long line
  rather than truncating.
- `brief_check` (`briefCheck.ts` → `brief.check`) is read-only: which `## Now`
  lines have outlived their dispatch.
- `brief_archive` (`briefBoardService.ts` → `brief.archive`) moves `## Recently`
  overflow to `brief.archive.md`; `/checkpoint` is what runs it.

## Failure modes

- Every send is best-effort: `sendFinished`, `send` and `sendCatchUp` swallow a
  rejected `claudemonSessionClient.message` (the parent may have just ended).
  `sendFinished` deliberately does NOT record dedup signatures when the send
  throws — booking a lost wake would silence the next identical edge forever.
- A worker whose parent is not live-and-`isSupervisor` just goes quiet. There is
  no error anywhere; the only symptom is a manager that never hears back.
- The codex **Windows rollout hybrid** (`spawnCodexHybrid`) spawns a bare TUI
  and has no facade wiring: a manager asked for on that path comes up with NO
  workspacer tools. It warns on the console and still sets `isSupervisor`, so
  wake routing works while the manager has nothing to act with.
- `installManagerSkills` is best-effort; a write failure leaves the manager with
  its kickoff doctrine but no `/standup`, `/checkpoint` or `/handoff`.
- `notify_when` watches are one-shot and in-memory — they do not survive a
  workspacer restart.
- Manager tombstones, orphan bookkeeping and reparenting are all in-process
  memory: none of it survives an app restart.

## Gotchas

- **Two hand-copied option literals used to drop `manager`.** `ipc.ts` and
  `hubCapabilities.ts`' `agents.spawn` each rebuilt the spawn options by hand
  and silently omitted `manager`/`fleetFullAccess`, so NO bus-spawned Fleet
  Manager ever came up as `isSupervisor` and its workers finished into the
  void. `main/lib/managedSpawnOptions.ts` now owns the mapping — add new
  role-bearing fields there, not in a call-site literal.
- **The MCP facade path is the one that matters.** The manager dispatches every
  worker through `MCP facade → agents.spawn` (the bus), never through the
  desktop IPC path. Test changes there.
- **Two stale comments in source, verified against the code:**
  `claudeSpawn.ts` (~L238) still says the manager skills are "/bearings,
  /stow" — those are retired names; the installed set is `/standup`,
  `/checkpoint`, `/handoff`. And `hubCapabilities.ts` (~L1992) calls the wake
  backstop "the 15-minute backstop"; it actually runs every 2 minutes with a
  3-minute grace (`WAKE_BACKSTOP_MS`, `MISSED_WAKE_GRACE_MS`).
- **`supervisorSessionIds()` is a live-sessions-only scan** and is deliberately
  NOT mapped into the federated/remote snapshot projection
  (`claudeSessionStore.ts` ~L1581) — a peer's manager is that peer's wake target.
- **`fleetMessages.ts` headers are load-bearing prose.** The all-failed
  `ALT_HEADERS` spelling exists because a wake whose every worker DIED must not
  open with the word "finished", and the `progress` header says STILL RUNNING
  for the same reason. Both are parsed back to their kind — edit the header and
  the parser together or the card degrades to a raw text blob.
- **`~/.workspacer/README.md` is upgraded in place** only when it is empty or
  byte-identical to `LEGACY_HOME_README`; a user-edited README is left alone.
- Related docs: `modules/mcp-tool-facade.md` (tiers, the `help` tool),
  `domains/agent-spawn.md`, `domains/session-lifecycle.md`,
  `modules/hub-process-supervision.md` (the unrelated namesake).
