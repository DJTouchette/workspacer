---
title: "Hub Jobs: recurring/one-off tasks (spawn an agent, call a capability, run shell)"
tags: [hub, go, jobs, scheduler, automation, security-invariant, desktop]
related_paths:
  - "services/hub/internal/jobs/jobs.go"
  - "services/hub/internal/jobs/scheduler.go"
  - "services/hub/internal/jobs/jobs_test.go"
  - "services/hub/cmd/hub/main.go"
  - "apps/desktop/src/renderer/src/components/settings/JobsSection.tsx"
  - "apps/desktop/src/main/shared/ipcTypes.ts"
  - "apps/desktop/src/renderer/src/backend/webBackend.ts"
  - "landing/docs.html"
  - "services/hub/cmd/workspacer/jobscmd.go"
  - "services/hub/cmd/mcp/main.go"
owner: Damien Touchette
last_reviewed: 2026-08-18
---

# Hub Jobs: recurring/one-off tasks (spawn an agent, call a capability, run shell)

## Overview
v1 shipped 2026-08-18. A job = trigger (interval / daily HH:MM+weekday-mask /
once RFC3339 / manual) + action (spawn an agent with a prompt / call one bus
capability / run a shell command). A spawn action may first run **context
steps** (2026-08-18) — code whose output feeds the prompt, and whose guards can
cancel the run before any model is called. The HUB owns everything (`services/hub/internal/jobs`):
storage, validation, scheduling, execution, history — so jobs run in desktop
mode and `workspacer serve` alike, and keep running while the desktop is
closed. The desktop's Settings → Jobs section (and the web `/app` for a
full-control pairing) is a thin editor over the `jobs.*` RPCs.

## How execution works
- Spawn/call actions loop back through a **self-dialed busclient**
  (`selfBusURL` in cmd/hub — a wildcard bind dials 127.0.0.1) so `agents.spawn`
  is answered by whichever provider is live (desktop main or the brain) WITH
  all the bus-caller clamps: no permission bypass, no mcpItemIds, profile
  configDir scrubbed. Jobs get the remote-spawn security posture for free —
  which also means a job CANNOT spawn under a second-account profile or YOLO.
- Spawn = `agents.spawn` (label = job name) then `agents.sendMessage` with the
  prompt — claudemon buffers a pre-ready message, so cold start works.
- **Context steps run BEFORE the spawn** (`SpawnAction.Context`, 2026-08-18):
  up to `maxContextSteps` (4) shell commands or bus calls whose output is
  substituted into the prompt at `{{output}}` / `{{output.N}}` (1-based; a
  prompt naming neither gets the outputs appended as fenced blocks, never
  dropped). Each step is capped at `contextCap` (12000 chars) and elided in the
  MIDDLE — a head-only cut loses the failure summary, a tail-only cut loses
  what ran.
- **A step can veto the run** — the point of the feature, not a detail.
  `skipIfEmpty` (nothing came back; `{}`/`[]`/`null`/`""` count as empty, so it
  works on a `call` step), `skipUnlessMatch` (RE2, compiled at SAVE time so a
  bad pattern is refused rather than silently never matching at 3am), and
  `ignoreExitCode` (a nonzero exit is data — `grep` finding nothing). A veto
  returns `*skipError` from `buildPrompt`, which `execute` records as `skipped`
  and — unlike an error — publishes NO `notify.post`: a guard that fires
  nightly must not notify nightly. Nothing spawns until every step has run and
  every guard has passed, so "nothing to do" costs a shell command instead of
  an agent + a model call + a session to clean up.
- `ignoreExitCode` forgives an **`*exec.ExitError` only** (`isExitCodeErr`): a
  timeout or an unstartable command still fails the run, so a guard can't turn
  a broken job into a silent one.
- Shell runs via `/bin/sh -c` (`cmd /C` on Windows) with a 15-min timeout;
  run detail keeps the output TAIL (errors live at the end).
- Overlap is **skip, never queue** (recorded as a `skipped` run) — a backlog
  of queued agent fires is a quota-burning failure mode. `once` jobs disable
  themselves AT FIRE TIME so a failed run can't refire forever. A due time
  missed while asleep fires once on wake, then reanchors (no catch-up storm).

## The spec file is hand-editable (2026-08-24)
`jobs.json` is a supported editing surface, not just storage. The scheduler
**polls it on its existing 30s tick** and reloads on change, so an edit made in
an editor takes effect on a running hub with no restart — desktop mode and
`workspacer serve` alike. The watcher is in the HUB deliberately: a watcher in
desktop main would silently not exist headless (the same hole `library.changed`
has — the brain publishes no such event, so library hot-reload is Electron-only).
- **Change detection is a content hash, not mtime** (`specHash`/`haveSpecHash`
  in `Service`). Same hash also suppresses the service's own write echo. mtime
  was rejected because a same-size write inside one timestamp tick is missable,
  and this repo has already shipped one millisecond-collision bug.
- **Clobber fix = re-read before marshal.** `reloadIfChangedLocked()` runs at
  the top of the locked section of `tick`, `List`, `Upsert`, `Propose`,
  `Remove` and `RunNow`, so every write lands on top of the file rather than on
  top of a boot-time snapshot. The alternative (move bookkeeping to the history
  sidecar) was rejected: the ONLY spec-file bookkeeping write is the `once`
  trigger's self-disarm flipping `enabled`, and hiding that in a sidecar would
  make the file the human reads say `"enabled": true` for a job that will never
  fire again. Interval reschedules touch `nextAt` only and never write.
- **Bad parse keeps the last good schedule** and logs (once per distinct
  content). Same for a file that cannot be read at all — editors unlink during
  their own atomic saves. Clearing everything has one explicit spelling,
  `{"jobs": []}`. A single row that fails `Validate` is still dropped alone.
- **Hand-written rows get completed and written back**: missing `id` minted,
  duplicate ids split (copy-pasting a block is the obvious way to write a
  second job), `createdAt`/`updatedAt` stamped. In-memory-only ids would be
  re-minted every reload and walk the next run forward each time.
- **Only jobs whose arming/trigger changed are rescheduled** (`sameSchedule`);
  a rename must not re-anchor every interval job on the machine.
- Proven by `services/hub/internal/jobs/handedit_e2e_test.go` (real `RunScheduler`
  goroutine, real file, written from outside) and by the harness's
  hand-editing section against a real hub process, including the job firing on
  the real 30s tick.

## Security decisions (the load-bearing ones)
- **A job is persisted argv** (the scrubBypassProfile lesson), so specs live in
  the hub-owned `<user-config-dir>/workspacer-hub/jobs.json` (0600, atomic
  tmp+rename; history sidecar `jobs-history.json`) — NEVER the library
  (agent-writable by design) and NEVER the layout doc (world-readable,
  client-broadcast). The file is 0600 and out of reach of BUS callers; it is
  not out of reach of a local agent with `Bash`, and the docs say so in one
  sentence: writing `jobs.json` is equivalent to writing a crontab. What is new
  next to a live session is persistence and unattendedness, not the shell
  primitive.
- **All five `jobs.*` RPCs are trusted-only** (`CallerIdentity.IsTrusted()` at
  the registration site in cmd/hub): host token or operator-tier pairing may
  manage jobs; plugin tokens and view/triage tiers are refused at call time
  even if a plugin manifest declares `jobs.*` (declaration passes capspec —
  the method isn't path-shaped — but the identity gate holds).
- Validate refuses call actions targeting `jobs.*` (recursion) and `hub:<peer>/`
  (a job must not execute on another machine; job state doesn't federate).
- A **context step is reach, not privilege**: the same argv a `shell`/`call`
  action already persists, run by the same hub process behind the same
  trusted-only gate. Context `call` steps go through the SAME
  `validateCallMethod` as a call action — factored out for exactly this reason,
  since a step exempt from the rule would be the hole.

## Gotchas
- **No `job.*` event topics exist — deliberately.** A new topic namespace
  costs four pinned registries (eventtopics.go, contracts fixture,
  pluginPermissions EVENT_TOPIC_RULES, eventplane_test matrix). Failures ride
  the EXISTING `notify.post` event; the Jobs UI polls `jobs.list` every 10s
  while open. If jobs ever get live events, budget the four-registry change.
- **`notify.post` got classified in this change** (it was an unclassified,
  TS-only-published topic until the hub's job-failure publish tripped
  `TestEveryPublishedTopicIsClassified`): **host-only**, reason = free-text
  bodies (job failures carry shell-output tails); phones get alerts via Web
  Push, not this topic. All four registries carry the row now.
- The scheduler tick is 30s and `nextAt` is recomputed from *now* at boot —
  interval jobs re-anchor on every hub restart (saving peers.json restarts the
  hub, which re-anchors intervals; daily/once are unaffected).
- `jobs.list`/`upsert` round-trip through the desktop as plain IPC
  passthroughs (`jobs:*` → `callHub`); web `/app` calls the RPCs directly
  (`HUB_CORE` in backendParity.test.ts names them, same as layout/federation).
  A view/triage web token gets a clean refusal — JobsSection feature-detects
  by the first list failing and shows an unavailable note.
- v1 has **no per-job budget and no usage-window gate** (skip-run-if-5h>N%):
  the per-account usage plumbing (2026-08-17) makes the gate cheap to add —
  that plus event triggers (fire on a bus topic) are the designed v1.5.

## Agents may PROPOSE, never arm (2026-08-18)
`jobs.propose` is the agent-facing write, and the asymmetry is the whole design:
- an operator-scoped token is `["*"]` (authtoken `Scope.Methods`), which the bus
  turns into **trusted** — so jobs.* was ALREADY reachable by an operator agent
  at the bus level; the only thing withholding it was that cmd/mcp exposed no
  tool. The restraint therefore cannot live in the identity gate, and doesn't:
  it lives in WHICH METHOD gets a tool.
- `Propose` forces `Enabled=false`, stamps `ProposedBy`, and mints a FRESH id
  (so a proposal can never overwrite an already-approved job's argv). A stamped
  row never schedules (`rescheduleLocked` checks `IsProposal()` as well as
  `Enabled`, so a hand-edited `enabled:true` row still won't fire) and
  `jobs.run` refuses it. Pending proposals are capped at `maxPendingProposals`
  (20) — a review queue nobody reads gets approved blind.
- Approval is one trusted write clearing the stamp: JobsSection's `approve()`
  or `workspacer jobs approve <id>`. Proposals sort to the top of Settings →
  Jobs with an amber badge and a disabled enable-checkbox, and arrival rides
  `notify.post` at level info.
- Facade tools (operator tier only, `services/hub/cmd/mcp/main.go` group "jobs"):
  `list_jobs`, `job_history`, `propose_job`, `run_job`, `remove_job` — and
  `tiers_test.go` pins that NO tier ever gains an `upsert_job`/`save_job`/
  `enable_job` tool, since that would make the review step decoration.
- Four registries again: cmd/hub registration + capspec `unscopedByDecision`
  + capspec composition witness + the renderer's `CAP_LABELS` consent label
  (the last one is caught by `pluginPermissions.test.ts`, not the Go build).

## `workspacer jobs` CLI (2026-08-18)
`services/hub/cmd/workspacer/jobscmd.go` — host authority (reads `<config>/remote-token`),
so it reaches `jobs.upsert`, which the facade withholds. `add -f <file>|-`
installs a spec written anywhere (the point: an LLM-written spec no longer has
to be retyped into the UI; editing jobs.json directly is now a first-class
route too, see above), plus
list/show/history/run/approve/enable/disable/remove with unique-prefix ids and
local `jobs.Validate` before dialling.
**Gotcha that bit once:** Go's `flag` stops parsing at the first positional, so
`jobs enable <id> --hub-port 18897` bound NO flags and silently talked to the
DEFAULT hub — wrong machine, no error. `splitPositionals` normalizes flag/arg
order; pinned by `jobscmd_test.go`.

## The docs are pinned to the validator
`services/hub/internal/jobs/docs_test.go` treats `landing/docs.html#jobs` as part of the
spec, because for anyone authoring a job by hand — or an LLM asked to write one
— it IS the spec:
- every `<pre data-job-example>` block in the page is parsed and run through
  `Validate` (so a documented example can't rot into one the hub refuses);
- **every json tag** on Job/Trigger/Action/SpawnAction/ContextStep etc must
  appear somewhere in the page (reflection walk, `createdAt`/`updatedAt`
  excluded as hub-managed) — add a field without documenting it and the test
  names it;
- every documented trigger/action/step kind is asserted accepted, and a set of
  realistic "written from the docs alone" specs must validate verbatim.
The file `t.Skip`s when `landing/` isn't present (hub built standalone). Also:
the reference block in the page uses `//` annotations, so it says explicitly
that JSON has no comments — a model copying it otherwise emits invalid JSON.

## Harnesses (both proven green 2026-08-18)
- **`services/hub/scripts/jobs-harness.mjs`** — end-to-end against a REAL
  scratch hub on :18897 (temp state dir, WORKSPACER_PARENT_PID stripped) with
  a fake `agents.spawn`/`agents.sendMessage` provider playing the desktop's
  role: validation refusals, spawn→prompt round trip, context steps (real-shell
  output substituted at `{{output}}`, a guard that skips with NO spawn, a
  nonzero-exit guard, a context call step refused for targeting `jobs.*`),
  shell output tails, error runs + the notify.post publish, overlap refusal, daily nextRunAt, and
  jobs+history surviving a hub restart. 40 checks, exit-code gated (including the CLI driven against that same hub).
- **`/jobs-harness.html`** on the dev Vite server (`src/harness/jobsHarness.tsx`)
  — Settings → Jobs against a stateful in-memory electronAPI fake, seeded with
  every chip state (ok/failed/skipped/running/disabled/never-run) plus a
  guarded `Failing tests → agent` job whose history is mostly `skipped` (what a
  healthy guarded job looks like); `?empty` shows the template-chip empty state, `?theme=<name>` re-themes. Fully
  interactive: run-now "finishes" after 4s alternating ok/failed.

## Hand-authored notes (2026-08-24) — `jobs.json` is now hand-editable

`services/hub/internal/jobs` `Service` re-reads its spec file whenever the CONTENTS change.
`reloadIfChangedLocked()` runs at the top of the locked section of `tick`,
`List`, `Upsert`, `Propose`, `Remove` and `RunNow`. **Change detection is a
sha256 of the file bytes (`specHash`/`haveSpecHash`), not mtime**, and the same
hash suppresses the service's own write echo. `New()` is just the first reload,
so boot and hot-reload share one code path. `RunScheduler`'s interval is now the
`Service.tickEvery` field (default `defaultTickEvery` = 30s), which is what lets
a test drive the real scheduler goroutine.

Two things that used to be true are no longer true: an external edit to
`jobs.json` was invisible until restart, and the next hub write silently
clobbered it. **Anything that assumed `s.jobs` is a boot-time snapshot is wrong
now.**

- **Do NOT add an mtime comparison as an optimisation in front of the hash.** The
  hash exists because a same-size write inside one mtime tick is missable, and
  this repo has already shipped one millisecond-collision bug.
- If you add a new `jobs.*` RPC that mutates, call `reloadIfChangedLocked()`
  first or it will clobber hand edits.
- The ONLY bookkeeping write to the spec file is the `once` trigger's self-disarm;
  interval reschedules touch `nextAt` only.
- Bad-parse and missing-file both keep the last good schedule; only
  `{"jobs": []}` clears it.

Security note: a `shell`-kind job runs unconfined in the hub process's
environment, and `jobsTrusted` is a bare `IsTrusted()` — see
`modules/fly-node-deploy.md` for why the Fly hub passes `--jobs-file ""`.
