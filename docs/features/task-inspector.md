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

**Your references** supports PR number and/or URL, tickets with optional URLs and
named http(s) references. These are explicitly user-entered, with no external lookup
or inferred attribution. Counts, sizes, duplicate labels/URLs, URL schemes and
credentials are validated at the host. Changes use the task revision; conflicts retain
the user's draft and require an explicit reload or reapplication to the current task.

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
No hub MCP capability, headless store or external issue-provider integration is added.
Manager replacement must fence new source dispatches and wait/refuse while source-owned
tasks have a dispatch reservation, before transferring ownership.

Ownership adoption policy remains the separate manager-handoff change; in-place
transfers preserve these revisions, audits, links and reservations.

## Review entry points and acceptance

Read `src/main/services/dispatchHistoryStore.ts`, `fleetWorkflowRuntime.ts`,
`src/main/shared/dispatchHistory.ts`, `src/renderer/src/components/TaskInspector.tsx`,
then IPC/preload/backend wiring and `settingsBus.ts` / `FleetWorkflowsSection.tsx`
under `apps/desktop`.

The focused main tests are `taskInspector.test.ts`, `fleetWorkflowRuntime.test.ts`,
`dispatchHistoryStore.test.ts`, `fleetWorkflowStore.test.ts`, `ipc.test.ts` and
`preload.test.ts`. Renderer tests cover exact task selection, real pane routing,
Inspector sibling isolation and local/remote backend parity. Playwright
`tests/e2e/taskInspector.test.ts` uses the production UI and bridged backend with
synthetic IPC, including 360px and 1280px views; it does not use live fleet data.

Review must confirm: future required-review skips cannot advance past unfinished
implementation; failed evidence survives; stale/live actions refuse; lifecycle and
adoption cannot overwrite user changes; stored targets alone authorize opening;
and Settings changes reach the existing project selection consumer without changing
the selected task's pin.

On Node 26, run renderer Vitest with
`NODE_OPTIONS=--no-experimental-webstorage npm run test:renderer` so jsdom owns
`localStorage`. The default Node 26 web-storage global caused existing chat/What’s New
suites to fail with undefined `localStorage`; the same suites pass with this flag.
