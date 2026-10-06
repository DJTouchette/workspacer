---
title: "Hub Jobs: recurring/one-off tasks (spawn an agent, call a capability, run shell)"
tags: [hub, rust, jobs, scheduler, automation, security-invariant, desktop]
related_paths:
  - "services/hub-rs/src/services/jobs.rs"
  - "services/hub-rs/src/services/jobs/reference_tests.rs"
  - "services/hub-rs/src/services/jobs/docs_tests.rs"
  - "services/hub-rs/src/cli/admin.rs"
  - "services/hub-rs/src/mcp.rs"
  - "services/hub-rs/tests/jobs.rs"
  - "apps/desktop/src/renderer/src/components/settings/JobsSection.tsx"
  - "apps/desktop/src/main/shared/ipcTypes.ts"
  - "apps/desktop/src/renderer/src/backend/webBackend.ts"
  - "landing/docs.html"
owner: Damien Touchette
last_reviewed: 2026-09-30
---

# Hub Jobs: scheduled agent, capability, and shell actions

## Current Rust ownership

The current scheduler/spec/history/execution owner is
`services/hub-rs/src/services/jobs.rs` and its submodules. CLI jobs use
`src/cli/admin.rs`; MCP is in `src/mcp.rs`. Run the Rust `--test jobs` target
and the owning library tests. `services/hub-rs/JOBS_MIGRATION.md` records intentional differences
and real restart/failure evidence. The policy descriptions below are retained;
Go files, helper names and Go test/harness commands are historical reference
only, not a dependency of scheduled work or current validation.

Historical execution, when deliberately requested, uses the separate pinned
checkout described in [scripts/reference/README.md](../../../scripts/reference/README.md).
This crosswalk does not certify platform or release gates.

## Ownership and entry points

The hub owns job validation, storage, scheduling, execution, and history in
`services/hub/internal/jobs`. Jobs execute while that hub is running. An
independent `workspacer serve` host can continue after a desktop client closes;
an app-owned hub is part of the desktop's supervised process tree. Closing the
owner does not leave an independent scheduler behind.

The desktop Settings → Jobs editor, web backend, MCP facade, and
`workspacer jobs` CLI call the hub's six methods: `jobs.list`, `jobs.upsert`,
`jobs.propose`, `jobs.remove`, `jobs.run`, and `jobs.history`.

All six registrations in `services/hub/cmd/hub/main.go` use `jobsTrusted`,
which requires **AuthenticatedHost AND Trusted AND scope operator**. A scoped
operator token alone is refused, as are plugin, view, triage, and provider
credentials. The error is “requires the server owner.” Do not infer access
from a full-control pairing or from a tool appearing in an MCP catalog.
`services/hub/cmd/hub/peersconfig_test.go` pins owner-versus-scoped-operator
administration; the registration code also gates `jobs.propose`.

The facade exposes `list_jobs`, `job_history`, `propose_job`, `run_job`, and
`remove_job` in its operator surface, but no upsert/enable tool. Facade calls
use its own bus connection; that connection must satisfy the hub's owner
check. The MCP client's catalog scope and the facade's bus credential are
separate checks. The CLI reads `HUB_TOKEN` or the persisted host token unless
`--token` is supplied; a supplied scoped token does not acquire owner rights.

## Spec and execution

A job has a name, enabled flag, trigger, and action. `Validate` is the
spec authority; the UI is an editor over that contract.

| Trigger | Scheduling |
| --- | --- |
| `interval` | `everyMinutes >= 1`; reanchors from now after a due tick or restart |
| `daily` | `at` in HH:MM, hub-local time; `days` is 0–6 (Sunday–Saturday), empty means daily |
| `once` | `once` is RFC3339; disables at fire time even if execution fails |
| `manual` | Runs through `jobs.run`, without a scheduled next time |

Actions are `spawn` (cwd and prompt required), `call` (method required), or
`shell` (command required). Call actions and context calls reject `jobs.*`
recursion and `hub:` federation. A shell command remains ordinary host code;
these method restrictions do not sandbox it.

Spawn first runs its context steps, then calls `agents.spawn` with cwd, job
name as label, and any provider/model/effort/permissionMode fields. It then
calls `agents.sendMessage` with the returned session ID and prepared prompt.
Calls use the hub's self-dialed bus client, so the selected provider's current
spawn validation and routing apply. The job schema has no profile ID or MCP
item selection; do not describe historical profile scrubbing as a separate
job authority boundary.

A run has one 15-minute context budget shared by context steps and action
calls. Shell commands use `/bin/sh -c`, or `cmd /C` on Windows. A successful
spawn run records that spawn and prompt delivery succeeded; it does not wait
for the agent's task to finish. Accordingly, overlap suppression protects the
job invocation, not the entire lifetime of a spawned agent.

A due invocation encountered while that job is still running records
`skipped`, never queues. Sleeping through multiple interval ticks produces
one due run on wake, then reschedules from now. Run history keeps the latest
30 records per job with status `ok`, `error`, or `skipped`; `jobs.list` also
exposes `running`, `nextRunAt`, and `lastRun`. Failed runs publish `notify.post`.
There is no dedicated job event namespace; the Jobs editor polls while open.

## Context steps and guards

At most four steps run before spawning. Each is a shell command or bus call.
A shell step uses its own optional cwd; it does not inherit the spawn cwd.
Output is trimmed before guards run:

- `skipIfEmpty` treats empty text, `{}`, `[]`, `null`, and `""` as empty.
- `skipUnlessMatch` uses a Go regexp, compiled during validation and checked
  against the full trimmed output before elision.
- `ignoreExitCode` tolerates an `exec.ExitError` from a shell step. It does not
  apply to bus-call failures or failure to start a command. In the current
  runner, a context-killed process can also return `exec.ExitError`; this flag
  must not be documented as a guarantee that every timeout remains an error.

A veto prevents spawning, records `skipped`, and sends no failure notification.
Each accepted output keeps up to 12,000 bytes from its two ends, plus an elision
marker. These are byte slices, not Unicode character limits. Run-detail and
shell-tail caps likewise count bytes.

`{{output.N}}` selects the Nth step (1-based); `{{output}}` selects the last.
If no recognized placeholder is present, all outputs are appended as fenced
blocks. If any recognized placeholder is used, unused outputs are not appended.

## Hand editing and storage

The default files are `<user-config-dir>/workspacer-hub/jobs.json` and
`jobs-history.json`, written atomically with mode 0600 on systems supporting
POSIX permissions. The `--jobs-file` hub flag selects the spec location;
empty disables job registration and scheduling. Shell jobs run with the hub's
OS permissions and environment. Editing the spec is equivalent to editing a
crontab: it installs unattended host code.

`reloadIfChangedLocked` hashes the file's contents, not its mtime. The
scheduler polls every 30 seconds; `List`, `Upsert`, `Propose`, `Remove`, and
`RunNow` also reload under the lock before operating. Do not add an mtime gate:
same-size edits within one timestamp tick must still be detected.

Unreadable files and invalid JSON preserve the last good in-memory schedule.
An individually invalid job in otherwise parseable JSON is dropped with a log.
`{"jobs": []}` is the recommended explicit clear. Actual Go decoding also accepts `{"jobs": null}`, `{}`, and a root `null` as valid empty schedules; the Rust port preserves those inputs. Missing, empty, whitespace-only, truncated, or otherwise invalid documents retain the last good schedule. Missing IDs and timestamps are
filled in, duplicate IDs split, and completed rows written back so later reloads
do not mint fresh identities. Only arming/trigger changes reschedule existing
rows; a rename does not reanchor an interval. Interval bookkeeping is in memory;
a once trigger writes its disabled state to the spec when it fires.

Reload-before-write avoids overwriting edits already visible on disk. It is
not a cross-process lock on a user's editor: an edit racing after the reload
can still be replaced by the atomic save.

## Proposals and approval

`Propose` always creates a fresh ID, forces `enabled=false`, and stamps
`proposedBy` (default “an agent”). It never overwrites a row directly. A
proposal may name `replaces: <id>` of an existing APPROVED job (validated at
propose time; a proposal cannot target another proposal). Pending
proposals are capped at 20 and publish an informational `notify.post` targeting
Settings → Jobs. The label records who proposed it; it is not authenticated
identity or a permission field.

`IsProposal` also suppresses scheduling when a hand edit sets `enabled=true`;
`RunNow` refuses proposals. Approval is an owner-authorized upsert clearing
`proposedBy` and enabling the row, exposed by the desktop Jobs view, the native
Jobs screen and `workspacer jobs approve <id>`. When that upsert carries
`replaces`, the hub instead copies name/trigger/action onto the target in place
(keeping its id, createdAt, history and enabled state, so approving an edit
never resumes a paused job) and deletes the proposal row; no new `jobs.*`
method exists for it. `jobs.remove` of a job also drops proposals replacing it.

Since 2026-10-06 there is NO job authoring UI in either app: jobs are written
by agents via the app-owned `scheduled-jobs` collaboration skill
(`apps/desktop/assets/skills/scheduled-jobs`, pointed at alongside spawn-agent
and project-brief by both installers) plus `propose_job`. The views only
approve/reject, pause/resume, run and remove. The power-down template chip was
removed with the editor; `contracts/job-preset-power-down.json` is still pinned
by the hub docs test. The facade's omission of upsert is a tool-design
constraint, not an OS boundary preventing an agent with host shell access from
editing the spec or invoking the owner CLI.

## Retained CLI semantics and historical Go validation

`services/hub/cmd/workspacer/jobscmd.go` implements list, add, show, history,
run, approve, enable, disable, and remove. `add -f <file>|-` validates a JSON
spec before dialing. IDs may be unique prefixes. `splitPositionals` lets
connection flags occur after IDs; preserve its tests so a port flag cannot
silently target the default hub.

From `services/hub`, run:

```bash
go test ./internal/jobs
go test ./cmd/hub -run TestJobAdministrationRequiresActualOwner
go test ./cmd/workspacer -run 'SplitPositionals|ReadSpec|JobLines'
go test ./cmd/mcp -run 'TierToolFiltering|Job'
```

`services/hub/internal/jobs/docs_test.go` validates the JSON examples in
[the public jobs reference](../../../landing/docs.html#jobs), checks field
coverage, and pins hand-editing instructions. Its page checks skip if the
landing file is absent, so run them from this full checkout and inspect skips.
It does not prove every statement on that page or in this context document.

`services/hub/scripts/jobs-harness.mjs` runs a scratch hub and fake agent
provider for end-to-end scheduling tests. The Vite `/jobs-harness.html` uses
an in-memory backend for interactive editor checks. Neither should be described
as currently passing without running it against the current checkout.
