# Fleet Manager spike — one conversation orchestrating many agents

Research spike, 2026-08-20. Vision: the user talks to ONE first-class agent rooted in a
parent directory (e.g. `~/Work`) that knows the child projects, dispatches real workspacer
agents into them ("kick off a fix for the viewport bug in preheat, and have someone update
the workspacer docs"), tracks the workers, and reports back.

Everything below marked **[V]** was verified by reading the source this session; **[I]** is
inferred from docs/comments and not re-executed.

---

## 1. What exists today (inventory)

### The supervisor system — 90% of the plumbing already exists

- **`supervisor: true` spawn option** [V] — `apps/desktop/src/main/services/claudeSpawn.ts`
  (~L145–165): any spawn with `opts.supervisor` gets
  - the **operator-tier MCP facade** (`wantsFacade = opts.supervisor || opts.mcpFacade ||
    !!opts.toolScope`; `facadeScope` is forced to `'operator'` for supervisors),
  - the `/supervise` skill installed idempotently into `~/.claude/skills/supervise/`
    (`installSupervisorSkill()` in `supervisorSkill.ts` — `SKILL.md` + zero-dep `fleet.mjs`
    that reads claudemon's REST API for cheap fleet status/convo/reply parsing),
  - default model from `config.supervisor.model`,
  - default cwd `~/.workspacer` via `ensureSupervisorHome()` (`supervisorSkill.ts:255`)
    when no explicit cwd is given — **an explicit cwd is honored**, so a supervisor rooted
    in `~/Work` is already expressible,
  - `SUPERVISOR_SYSTEM_PROMPT` + a kick to run `/supervise` on a loop, via
    `facadeSpawnArgs()` in `mcpConfig.ts:101`.
- **Caveat — transport split** [V]: only the PTY path (`claudeSpawn.ts`) calls
  `installSupervisorSkill()`. `managedSpawn.ts` (Codex/OpenCode/Pi **and Claude
  `transport: 'stream'`**) injects `managedFacadeInstructions()` text only — a
  stream/GUI-chat supervisor never gets `/supervise` or `fleet.mjs`. Matters because a
  chat-first manager wants the stream transport (like the Guide).
- **`supervisorNudge`** [V] — `supervisorNudge.ts` + call sites in
  `claudeSessionStore.ts:641,728`: when ANY session newly enters
  `waiting_approval`/`waiting_input`, every live supervisor gets a coalesced (1500 ms)
  `[supervisor] An agent is now blocked…` message. **Blocks only — there is no nudge when a
  worker finishes** (verified: `onBlock` is the only trigger; `agentNotifier.ts` fires
  working→idle "finished" notifications, but only to the OS/user, at
  `claudeSessionStore.ts:634,724,928` via `notifyOnTransition`).

### What an operator-tier agent can actually DO via the facade

`services/hub/cmd/mcp/main.go` (tool registry), tiers derived from
`services/hub/internal/authtoken/authtoken.go`; executed by
`apps/desktop/src/main/services/hubCapabilities.ts`. All [V]:

- **Spawn**: `spawn_agent` (`spawnAgentIn`, main.go:777) — `cwd` (**arbitrary**; the only
  normalization is trim + trailing-slash strip, `apps/desktop/src/main/lib/spawnCwd.ts` —
  cwd is deliberately `unscopedByDecision`), `provider`, `model`, `effort`, `profileId`,
  `label`, `parentSessionId` (nests the card under the manager in the UI), `toolScope`
  (grant the worker its own tier), `pluginTools`, `hub` (spawn on a federated peer).
- **Clamps at the `agents.spawn` boundary** (`hubCapabilities.ts:248–340`) [V]:
  `skipPermissions` forced **false** for every bus caller (the facade included);
  permission-escalation modes stripped; `mcpItemIds` ignored; profile bypass args scrubbed
  (`scrubProfileBypass`). So dispatched workers always surface approvals — which the
  manager can then `approve` itself or escalate. **No `worktree` option** (worktrees are
  renderer-side only, `useAgentManager.spawnAgent` → `worktreeCreate` IPC). **No
  `supervisor` flag** exposed on the facade tool (the capability accepts it, the tool
  schema doesn't) — a manager can't mint another manager.
- **Drive/track**: `send_message`, `get_conversation` (with `sinceSeq` — cheap incremental
  cursors), `get_snapshot`, `get_transcript`, `approve` (yes/no/always), `answer`,
  `signal`, `set_approval_gate`, `notify` (with the `session:<id>` link convention the UI
  renders as clickable), `list_agents`/`list_snapshots` (federation-aware).
- **Project awareness**: `get_config` (returns `config.projects`), `list_dir`,
  `list_entries`, `read_file`, `write_file`, `search_project` (ripgrep),
  `list_resumable_sessions` (per-directory recent Claude sessions = "recent activity"),
  `list_models`, `get_host_cwd`. The `files` help topic already says *"Use these to
  inspect or brief work, not to do the coding yourself — spawn an agent in the directory
  instead"* (`help.go:69`) — the facade is already written with a dispatcher in mind.
- **Jobs**: `propose_job` (saved disabled until the user arms it in Settings → Jobs) — a
  manager could propose recurring work but can't self-arm it. [V]

### Spawned workers appear as normal cards

[V] `useAgentManager.adoptAgent` (~L683): a daemon session with no workspace (i.e. spawned
via the facade) is auto-adopted into the sidebar with a deterministic card id, the right
provider logo, and `parentId` nesting resolved from `parentSessionId`. The comment names
this exact case ("e.g. a Codex session spawned via the MCP facade"). So dispatch → the
worker shows up nested under the manager in the sidebar and Fleet Deck with zero new code.

### Project awareness today

- **`config.projects: Record<dir, ProjectIdentity>`** [V] — label, color, icon/favicon,
  favourite, lastOpened, per-project plugin state (`ipcTypes.ts:468`,
  `configService.ts:261`, renderer `lib/projectRegistry.ts` `listProjects()`).
  This is the app's own inventory of "the user's projects" — exposed to the manager via
  `get_config` already.
- Child-project context: a worker spawned with `cwd = ~/Work/preheat` is a normal Claude
  session there — it picks up that repo's CLAUDE.md/skills itself [I, standard Claude
  behavior]. Nothing composes a *brief* for the worker today; the closest prior art is
  cross-provider handoff (deterministic brief file + pre-filled composer,
  `agentHandoff.ts`) [I from memory/docs, not re-read].

### Prior art for "a chat agent with a role"

- **The Guide** [V] — `renderer/src/lib/guide.ts` + `panes/GuidePane.tsx` +
  `useAgentManager.spawnGuide` (L656): the strongest precedent and the pattern to copy.
  A chat-styled pane with scripted opening bubbles; on first question it spawns a REAL
  agent with a fixed name (`GUIDE_AGENT_NAME` — reuse-by-name instead of respawning),
  `transport: 'stream'` (GUI bubbles), `toolScope: 'triage'`, and — crucially —
  **`kickoffMessage`** (auto-sent via `claudeMessage` after spawn resolves,
  `useAgentManager.ts:313`). Preset chips (`GUIDE_PRESETS`) on the welcome card.
- **Ask the Fleet** [V] — `panes/AskPane.tsx` + `spawnSupervisor` (useAgentManager L607):
  spawns a *new* supervisor per question in `~/.workspacer`, with the question as
  `initialPrompt` — which only **pre-fills the composer** (`ClaudePane.tsx:221`), the user
  still presses Enter. Ephemeral, one-shot, not a persistent manager. Provider-selectable.
  Presets in `askPresets.ts` (Standup / Triage / Audit / Cost) are watch-flavored, not
  dispatch-flavored.

### UI surfaces

- **Global "Overview" workspace** [V] — `GLOBAL_WORKSPACE_ID = 'global'`
  (useAgentManager.ts:139), pinned, singleton, hosts cross-agent panes; its default tab is
  the `overview` dashboard pane (`OverviewPane.tsx` — projects grid, usage windows, plugin
  health, updates; 891 lines, pure dashboard, no chat). `ask` and `guide` panes are opened
  *into* this workspace (`App.tsx:1238,1246`). This is the natural home for a
  fleet-manager entry point.
- **Fleet Deck** [V] — `components/FleetDeck.tsx`, the radar/grid overlay of every agent
  card with attention scoring. A projection, not a conversation.
- New pane types require updating the exhaustive `Record<PaneType, …>` maps
  (`types/pane.ts` + registry) [I from memory note].

---

## 2. Gap list (ranked by how much each blocks the vision)

1. **No fleet-manager role.** `/supervise` is watch-and-triage ("you coordinate; you don't
   write the code"; surface decisions to the human). Nothing teaches an agent to
   *inventory projects and dispatch work into them*. This is the core gap — but it is
   mostly a skill/prompt + preset, not new infrastructure. The entire tool surface needed
   (list_dir, get_config, read_file, spawn_agent with cwd+label+parentSessionId,
   send_message, convo cursors, approve, notify) exists at operator tier today.
2. **No completion wake.** The manager only learns a worker finished by polling.
   `supervisorNudge` fires on blocks; `agentNotifier` detects working→idle but tells only
   the user. "Dispatch and report back" wants a `[supervisor] worker finished:
   session:<id>` nudge for children of a supervisor. Small change, big ergonomic win.
3. **No persistent, blessed manager instance.** Ask spawns a fresh supervisor per
   question; nothing reuses one long-lived manager or restores it at boot. The Guide's
   reuse-by-fixed-name pattern solves reuse; boot-time presence is a product decision
   (session-lifecycle already keeps dead sessions as resumable rows, so "click to resume
   the manager" is nearly free; auto-start is a cost question).
4. **Chat-first manager loses the skill.** Stream-transport Claude goes through
   `managedSpawn.ts`, which never installs `/supervise` (or any future `/fleet-manager`)
   skill. A bubbles-style manager (like the Guide) today = prompt text only. Fix is a few
   lines (call the installer in `managedSpawn.ts` too) but must be deliberate.
5. **Send ergonomics.** Ask uses `initialPrompt` (pre-fill, manual Enter); the vision
   ("the user talks to it") wants `kickoffMessage` auto-send + a live chat — already how
   the Guide works. Trivial, but it's the difference between "a spawner form" and "a
   conversation".
6. **No worker brief / context handoff.** The manager can read the child repo
   (read_file/search_project) and write a good opening prompt, and the worker reads its
   own CLAUDE.md — probably good enough for v1. Richer briefs (recent activity from
   `list_resumable_sessions`, git status via a shell worker, handoff-file pattern) are
   polish.
7. **No worktree isolation from the facade** — a manager can't dispatch into an isolated
   worktree the way the spawn dialog can. Acceptable v1 gap; add a `worktree` param to
   `agents.spawn`/`spawn_agent` later (four-place spawn-param sync: TS helpers, hub
   boundary, brain Go, facade — brain may decline like `toolScope`).
8. **Parent-dir cwd is not the default anywhere** — supervisors default to
   `~/.workspacer`. `config.agents.defaultCwd` exists and an explicit cwd is honored, so
   this is a preset choice, not a code gap.

---

## 3. Design directions

### (a) Fleet Manager = a blessed supervisor spawn preset  ← recommended

Clone the Guide pattern at operator tier, rooted in the parent directory:
`spawnFleetManager()` spawns (or reuses, by fixed name) ONE agent with
`cwd = <parent dir>` (from `config.agents.defaultCwd`, else a one-time picker),
`supervisor: true` (operator facade + nudges + skill install + supervisor model),
`kickoffMessage` auto-send, and a new `/fleet-manager` skill installed beside
`/supervise` that teaches: inventory (`get_config`.projects + `list_dir` of cwd +
CLAUDE.md sniffing), dispatch (`spawn_agent {cwd: child, label, parentSessionId}`),
track (convo cursors, approve-or-escalate), report (`session:<id>` links).

- **Pros**: rides only verified plumbing; smallest diff to a working demo; the manager is
  a normal agent card (attention badges, Fleet Deck, mobile /m, federation all work for
  free); skill is user-readable/editable like `/supervise`.
- **Cons**: "front and center" depends on where the entry point lives (see below); PTY vs
  stream transport tension (gap 4) must be resolved.

### (b) Overview becomes a chat

Embed a persistent manager conversation directly in `OverviewPane` (the global
workspace's dashboard): dashboard on top, manager chat below — true mission control.

- **Pros**: maximum prominence; the manager is unmissable and ambient.
- **Cons**: `OverviewPane` is a dashboard grid; embedding a real conversation means
  re-hosting the chat renderer (dual-fed stream conversation, tool cards, composer,
  approvals) inside a second surface — exactly the duplication the codebase keeps
  fighting. High UI cost, no new capability. **Rejected as the vehicle**, but its instinct
  is right: give Overview a "Talk to your fleet" hero row (chips + input, like the
  Guide's opening) that spawns/focuses the manager and jumps into its normal agent pane.

### (c) Manager as a first-class pinned workspace kind

Productized (a): `kind: 'manager'` workspace auto-created (or auto-restored) at boot,
pinned directly under Overview in the sidebar, rooted at the parent dir, with its own
icon and attention badge; Ask-the-Fleet folds into it.

- **Pros**: matches the vision's "the app provides one" phrasing; permanence and
  discoverability solved structurally.
- **Cons**: touches session-lifecycle/boot-restore and sidebar ordering (hotspot files);
  auto-start burns tokens if the user doesn't use it. This is phase 3, not the spike.

**Recommendation: (a) now, evolving into (c), stealing (b)'s hero-row entry point.**
Transport: start PTY (skill works today); make the `managedSpawn` skill-install fix part
of phase 2 so the manager can move to stream/GUI-chat like the Guide.

---

## 4. Concrete spike plan

### Phase 0 — smallest end-to-end prototype (talk to a manager in ~/Work, it spawns + tracks one worker)

Demoable with ~4 small changes, no new pane:

1. **`apps/desktop/src/renderer/src/lib/fleetManager.ts`** (new, ~60 lines, mirror of
   `lib/guide.ts`): `FLEET_MANAGER_NAME = 'Fleet Manager'`, a compact preamble (role: you
   manage the projects under your cwd; inventory via get_config projects + list_dir;
   dispatch via spawn_agent with cwd/label/parentSessionId; track via get_conversation
   sinceSeq; approve small things, notify for big ones; reference `session:<id>`; call
   `help` first), `buildManagerKickoff(question)`, 3 preset chips ("What's the state of my
   projects?", "Kick off …", "Status of everything you dispatched").
2. **`useAgentManager.spawnFleetManager(question)`** (~40 lines next to `spawnGuide`,
   `hooks/useAgentManager.ts`): reuse-by-name (live agent named `FLEET_MANAGER_NAME` →
   `claudeMessage(sessionId, kickoff)` + focus, else spawn), `cwd =
   config.agents.defaultCwd || pick`, `supervisor: true`, `kickoffMessage:
   buildManagerKickoff(question)`. PTY transport for now (gets `/supervise` + operator
   facade + nudges installed today; the fleet-manager framing rides the kickoff message in
   phase 0 — no new skill file needed to demo).
3. **Entry point**: a command-palette entry + one hero row on `OverviewPane` ("Fleet
   manager — tell it what to get done across your projects" + input) calling
   `spawnFleetManager`. (Or, cheapest possible: a third preset section in `AskPane`.)
4. **Nothing else.** Dispatch, tracking, approval, nesting, sidebar cards, notify all
   already work (§1). The demo: type "have someone bump the README in
   ~/Work/workspacer" → manager `list_dir`s, `spawn_agent{cwd:
   ~/Work/workspacer, label: 'README bump', parentSessionId: self}` → card appears nested
   → manager polls `get_conversation --sinceSeq`, approves the edit or nudge-wakes on the
   approval, reports back with `session:<id>` links.

### Productization phases

- **Phase 1 — the skill**: add `FLEET_MANAGER_SKILL` to `supervisorSkill.ts` (second
  skill dir `~/.claude/skills/fleet-manager/`), installed by the same
  `installSupervisorSkill()` call; teach inventory/dispatch/track/report procedurally +
  a `fleet.mjs`-style `projects` subcommand if needed. Spawn the manager with a
  `manager: true`-flavored option or just a distinct kickoff (`/fleet-manager`).
- **Phase 2 — completion wakes + transport parity**: `supervisorNudge.onFinish(session)`
  fired from the ambient-transition sites (`claudeSessionStore.ts:634/724/928`, where
  `notifyOnTransition` already computes working→idle) for sessions whose
  `parentSessionId` is a supervisor; `managedSpawn.ts` calls `installSupervisorSkill()`
  so a stream-transport (GUI-chat) manager keeps the skill; consider moving the manager
  to `transport: 'stream'` like the Guide.
- **Phase 3 — front and center (direction c)**: pinned `kind: 'manager'` workspace under
  Overview; boot behavior (restore-as-resumable by default, opt-in auto-start); fold or
  link Ask-the-Fleet.
- **Phase 4 — dispatch polish**: `worktree` param on `agents.spawn`/facade `spawn_agent`
  (4-place param sync; brain declines); richer briefs (recent-activity from
  `list_resumable_sessions`, handoff-file pattern); federation-aware dispatch UX
  (`hub` param already works).

---

## 5. Open questions for the user

1. **One manager or one per root?** A single global manager keyed to
   `config.agents.defaultCwd`, or one per parent directory (people with `~/Work` +
   `~/oss`)? Decides reuse-by-name vs keyed-by-cwd and the config shape.
2. **Boot behavior**: auto-start the manager at app launch (always-on, costs tokens /
   keep-warm interaction), or spawn on first use and restore-as-resumable after that?
3. **Chat-first (stream transport, Guide-style bubbles) or terminal-capable (PTY, full
   `/supervise` today)?** Chat-first pulls the `managedSpawn` skill-install fix into the
   spike.
4. **Autonomy on approvals**: may the manager approve its workers' permission prompts
   within some policy (e.g. edits inside the dispatched repo), or always escalate to you?
   Changes skill text and possibly a config knob.
5. **Where does it live**: pinned sidebar workspace above/below Overview, an Overview hero
   row, or absorbed into/replacing Ask the Fleet?

## 6. Round 2 — user requirements (2026-08-20) and design answers

Four additions from the first review, each now part of the plan:

### 6a. Profile-aware dispatch

The manager must be able to dispatch workers under different Claude account
profiles (Default vs Work — they are capacity pools as much as identities:
separate rate-limit windows, separate logins). Today this is BLOCKED on
purpose: facade/bus spawns scrub the profile wholesale (`scrubBypassProfile` —
a bus caller must not smuggle a `configDir`). The doctrine stays; the manager
gets a GRANT, not an exception:

- Extend the session facade token record (authtoken) with `profilesAllowed:
  string[]` — set ONLY by the local spawn path when the host user blesses the
  manager (the same pattern as `pluginTools`: a grant recorded at mint time,
  enforced server-side).
- `agents.spawn` (hubCapabilities) honors `profileId` iff the caller's token
  carries that profile id in its grant; everything else scrubs exactly as
  today. A plugin or web token still cannot name a profile it wasn't granted.
- The manager's role skill learns the vocabulary: "dispatch heavy work to the
  account with headroom" (it can read per-account usage from the snapshots).

### 6b. The fleet never leaves the sidebar

Workers already adopt as first-class sidebar cards (nested under the manager
via `parentSessionId`), so most of this is guaranteeing it rather than
building it:

- The manager's workspace must never hide the fleet: in focus mode the
  sidebar rails down — the manager surface should force the full sidebar (or
  the manager simply recommends fleet mode). Decision: keep it a MODE-NEUTRAL
  rule — manager children render in the rail too (the rail already shows
  status dots; that is enough signal).
- Sidebar grouping: children of the manager get the nested treatment
  consistently, with live status, so "one conversation + the whole fleet
  visibly working" is the default view, not an arrangement the user builds.

### 6c. The manager is a pure delegator — always interruptible

"Always free to send a message" decomposes into three enforcements:

1. **Role doctrine (skill)**: the manager NEVER edits code, never runs long
   tool loops, never polls. It inventories, dispatches, relays, summarizes.
   Target: every manager turn ends in seconds. The skill says this in its
   first paragraph, the way /supervise's own doctrine does.
2. **Event-driven wakes instead of polling**: the worker-finished nudge (gap
   #2) is what makes short turns POSSIBLE — the manager doesn't watch
   workers, it gets woken: `[fleet] worker "preheat-viewport" finished —
   summary attached`. Blocks already nudge via supervisorNudge; finished is
   the missing twin (agentNotifier detects working→idle at the same store
   transition sites).
3. **Queueing is the backstop, not the plan**: claudemon already buffers
   sends through the settle+verify flush, so a message typed mid-turn is
   never lost — but a manager that respects (1) and (2) is simply idle when
   you talk to it, which is the actual requirement.

### 6d. Per-repo living context — the project BRIEF

Not recon/rivet (code facts), but intent: "what we're working on, where
we're going, what changed lately." A conventional file per repo:

- **Location**: `.workspacer/brief.md` IN the repo (project truth travels
  with the project; visible to any tool; the user can edit it in any editor).
  `config.projects` entries point at it implicitly by cwd. Optionally
  gitignored per repo — the user's call, not ours.
- **Shape** (markdown, three sections, deliberately small): `## Now` (what's
  in flight), `## Direction` (where this is going), `## Recently` (append-only
  log of landed work, newest first, pruned by length).
- **Writers**: the manager updates `Recently`/`Now` when a worker finishes
  (it just got the completion nudge with a summary — appending one line is
  the natural next act); workers MAY be instructed to update it at handoff;
  the user edits freely. Merge policy: it's markdown — last writer wins,
  and the manager treats the user's text as authoritative.
- **Readers**: the manager's inventory skill (its first act on spawn: read
  every project's brief + config.projects identity → it "knows" the fleet's
  world); the Overview dashboard can later render brief summaries on project
  cards; the existing cross-provider HANDOFF brief generator is prior art
  for the writing style.

### Revised phase order

Phase 0 (unchanged, prototype) → Phase 1: role skill + worker-finished nudge
+ stream-transport skill install → Phase 2: briefs (`.workspacer/brief.md`
read/write conventions in the skill; manager maintains them) → Phase 3:
profile grants (`profilesAllowed` on the token + agents.spawn enforcement) →
Phase 4: UI blessing (Overview hero entry, nested fleet grouping polish,
pinned manager workspace kind).
