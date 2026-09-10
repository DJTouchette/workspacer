# Task Inspector

The local desktop Inspector has a **Tasks** sibling beside Session and Project.
Recent agents also offers **Inspect task**. Session inspection remains a pure
snapshot projection; task data is read independently from local dispatch history.

Current tasks appear first, with recent tasks selectable. A worker selection uses
its exact recorded dispatch attempt. Manager and project ownership remain visible;
unrecorded workers produce an explicit empty selection rather than inferred membership.

## Task actions

**Skip…** is an explicit desktop user action, with a small confirmation and an
editable default reason. It applies only to the selected task. Required review and
future planned steps can be skipped while an earlier step is unfinished. Work still
runs in order: terminal skips never make later work runnable ahead of that earlier
step. A finished failed/blocked next step can be skipped only when its exact attached
local session is verified stopped. Dispatched, reserved, live, completed and unknown
states cannot be skipped. No action stops a worker.

The stored state is `waived`, displayed as **Skipped by you** with timestamp and
reason. The original definition, hash, templates, outcomes and failure/result-contract
evidence remain intact. A waiver is never completion, passing review or a fabricated
result. Manager MCP `decide_workflow_step` keeps its required-step refusal.
Dependent `independentOf`/`repairOf` evidence must exist as an exact recorded attempt
with a valid reported result; skipping a producer does not manufacture that evidence.

**References** supports PR number and/or URL, tickets with optional URLs and named
http(s) references. Nothing here is looked up or verified with a provider. Counts,
sizes, duplicate labels/URLs, URL schemes and credentials are validated at the host.
Changes use the task revision; conflicts retain the user's draft and require an
explicit reload or reapplication to the current task.

References appear as compact chips near the top, beside the recorded branch and an
open-worktree action. The form is a lightweight toggle behind **Links**, not a
permanent panel.

A manager may also record references over MCP; see "Manager task references" below.
Everything else on a task, including the step waiver, remains host-user only.

## Layout

The default view shows the task title once, a short status/manager/project line, the
reference chips, and one line per step. Identifiers (task id, manager session id,
absolute project path, snapshot hash) and the recorded-work attempts sit under
**Details** with a copy button each; the task/project filters sit under **Filters**.

A finished step (completed, skipped by manager, skipped by you) renders no skip
affordance at all and collapses its rationale, reported outcome and dispatch contract
behind **Step details**. An unfinished step that cannot be skipped keeps its button,
disabled, with the one-line reason — that state is actionable information, so it is
not hidden. A host-user waiver still reads **Skipped by you** with its timestamp and
reason inline, distinct from a completed check.

The panel renders inside a 360px rail without horizontal overflow. Colour, radius,
spacing and type come from the existing `--wks-*` tokens, `Surface` and
`settings/primitives`; no new visual system was introduced. `FleetWorkflowTask` is no
longer mounted here, so the workflow name and steps appear once.

Execution folder, project, worktree allocation/fallback and branch are recorded facts.
Opening addresses task + dispatch/reference selectors, never renderer-supplied paths
or URLs. Worktrees must retain their recorded canonical directory identity. Older
records lacking that identity remain visible but cannot authorize opening a folder.
URLs are revalidated from the stored references before invoking the OS browser.

**Configure workflow** opens the existing Fleet Manager workflow Settings and that
task's project selector. Its ordinary selection CAS applies to new tasks only; the
current task's pin is unchanged.

## Persistence and transport

JSON `dispatch-history.json` remains version 1. Legacy task revisions read as zero;
every changed task advances its revision, including lifecycle, adoption, reservation,
waiver and reference mutations. Every writer reloads inside the existing cross-process
file lock and atomically replaces the file. Failed writes restore the in-memory task
rows. There is no delayed stale flush. Process-local stale projections are not written
back over another process's lifecycle facts. Corrupt storage reports an error rather
than an empty successful history.

Audit retains all bounded workflow waivers and the latest 40 reference edits. Each
reference collection is limited to 20 entries. Existing whole-task/attempt/byte limits
apply; all-terminal waived workflows are eligible for eviction, while unfinished
workflows remain protected.

A persisted per-task reservation covers asynchronous dispatch allocation across
processes. Only its matching token releases it. A crashed host's unresolved reservation
fails closed and needs host recovery; it is not silently expired into permission to
skip or launch another worker.

`taskInspectorEdit` and `taskInspectorOpen` are host-only IPC/preload methods. Bridged
local desktop routes them to IPC; web, remote and old preload report unavailable.
No headless store or external issue-provider integration is added.

## Manager task references

`get_task_references` and `update_task_references` are the only task write a manager
agent has, added to the existing `fleetWorkflows.request` capability (ops
`taskReferences` / `setTaskReferences`). They exist so that a PR, ticket or link the
user pastes into chat lands on the task the user is looking at.

The bus already refuses scoped, plugin and federated connections for this method. The
facade stamps `callerSessionId` from its authenticated session; it is never a tool
argument. The host then re-checks ownership through the same `ownerTask` gate the
workflow tools use: a live, local, wake-target manager, its OWN task, in that task's
exact project. A pinned workflow is not required for a reference edit, so the gate is
called with `requireWorkflow=false`.

Writes are additive per entry. `upsert` matches `{kind:'pullRequest'}` as a singleton
and `{kind:'ticket',id}` / `{kind:'reference',label}` case-insensitively by identity,
merging a URL it was not given rather than dropping it; `remove` names the same
identity. Entries the manager did not name survive untouched, including ones the host
user typed. There is no whole-set replace.

`expectedTaskRevision` is required for a write and is the task row's revision, not the
workflow definition or selection revision. A stale value returns `code:"conflict"` with
the current revision and the current references, so the manager can reconcile instead
of retrying blindly. An identical repeated edit is a no-op: no audit row, no revision
change. Validation is `validateTaskLinks` — the same host validator the Inspector form
uses — so schemes, credentials, lengths, counts and duplicates behave identically.

Reference edits are audited with `actor:'manager'`, alongside the host user's
`actor:'host-user'` rows; the audit `actor` union is the only shape change. The tool
records exactly what it was given as an unverified reference. It never fetches a URL,
calls a provider API, mutates anything externally, or infers a provider: an Azure
DevOps, GitLab or GitHub PR URL is all just a `pullRequest` reference, and a Jira key
or bare ticket id is a `ticket`.

`editByHostUser` (the step waiver) is deliberately unreachable from here and has no bus
method. `decide_workflow_step` keeps its required-review refusal. A headless owner
returns the existing explicit unavailable for the whole `fleetWorkflows.request`
method, so the new ops inherit it rather than gaining a headless store.

Manager doctrine (`fleetWorkflow.ts` dispatch guidance) and the facade `help`
`workflows` topic both instruct storing task-relevant URLs and IDs from user chat or
trusted worker results, binding to the exact task and asking when ambiguous. Neither
authorises scraping transcripts or trusting arbitrary tool text.
Manager replacement must fence new source dispatches and wait/refuse while source-owned
tasks have a dispatch reservation, before transferring ownership.

Ownership adoption policy remains the separate manager-handoff change; in-place
transfers preserve these revisions, audits, links and reservations.

## Review entry points and acceptance

Read `src/main/services/dispatchHistoryStore.ts`, `fleetWorkflowRuntime.ts`,
`src/main/shared/dispatchHistory.ts`, `src/renderer/src/components/TaskInspector.tsx`,
then IPC/preload/backend wiring and `settingsBus.ts` / `FleetWorkflowsSection.tsx`
under `apps/desktop`.

The manager reference path is `managerTaskReferences.test.ts` (semantics, validation
and the host-waiver boundary at the store) and `managerTaskReferencesService.test.ts`
(own/foreign/dead/remote/no-caller access, wrong project, CAS conflict reporting), plus
`cmd/mcp/taskrefs_test.go` for tier scope and help discoverability.

The focused main tests are `taskInspector.test.ts`, `fleetWorkflowRuntime.test.ts`,
`dispatchHistoryStore.test.ts`, `fleetWorkflowStore.test.ts`, `ipc.test.ts` and
`preload.test.ts`. Renderer tests cover exact task selection, real pane routing,
Inspector sibling isolation and local/remote backend parity. Playwright
`tests/e2e/taskInspector.test.ts` uses the production UI and bridged backend with
synthetic IPC, including 360px and 1280px views; it does not use live fleet data. It
also renders 360/480/1280px in two themes to `test-results/task-inspector/` for human
review, and covers a reference recorded outside the panel appearing on the next poll.
The harness accepts `?width=` and `?mode=links`.

Review must confirm: future required-review skips cannot advance past unfinished
implementation; failed evidence survives; stale/live actions refuse; lifecycle and
adoption cannot overwrite user changes; stored targets alone authorize opening;
and Settings changes reach the existing project selection consumer without changing
the selected task's pin.

On Node 26, run renderer Vitest with
`NODE_OPTIONS=--no-experimental-webstorage npm run test:renderer` so jsdom owns
`localStorage`. The default Node 26 web-storage global caused existing chat/What’s New
suites to fail with undefined `localStorage`; the same suites pass with this flag.
