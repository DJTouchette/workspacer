# Configurable Fleet workflows (desktop v1)

Settings → Fleet Manager → Workflows defines the ordered policy for **new** Fleet tasks. Choose a global default; Settings → Projects can override it or inherit. The selected no-review policy is explicitly labelled “Independent review omitted by selected policy”. Shipped starters are immutable: clone to customize, or reselect the starter to reset the policy. The default is conditional scout → implement → fresh independent review; Direct implementation is the explicit no-review starter. Read-only research definitions are valid.

The form supports 1–8 ordered steps, editable ids/labels/instructions, closed kind/stage/role/condition selectors, dispatch templates, visual move buttons and optional bounded repair links. Each configured review is required. Conditions are `always` and `material_risk`; a material-risk decision needs a persisted reason. A repair can reference one earlier review, with at most one repair per review. There are no scripts, DAGs, inferred retries, background executors or automatic launches from editing. A failed/blocked step returns control to the manager/user. Further work requires a separately authorized task.

Templates come from the host-global Library's **dispatch** items. The form lists required inputs; the manager supplies them when dispatching. The shipped `scout-task` and `ship-task` require `task`; `review-task` requires `task` and `handoff`. Their actual bodies and result schemas are read, not hard-coded examples or private machine templates. Use Library to author custom dispatch templates and result contracts. Every relevant template body/schema/input list is frozen with the task; later Library edits do not change it.

## Runtime and truth

A manager calls `start_workflow {cwd,title}` before the first worker for a new task. The host resolves project/global selection and persists the exact definition revision, templates, SHA-256 snapshot hash and planned step rows in `dispatch-history.json`. It returns the task and next instructions. `next_workflow_step {taskId,cwd}` and `decide_workflow_step {taskId,cwd,stepId,run,reason}` let the manager interpret the sequence. The host never schedules a model itself.

The manager calls existing `select_model` with the step role, then `spawn_agent` with its routing answer and the exact task/step/stage/predecessor/template metadata. The workflow facade puts the maximum requested scope through the existing hub gate; desktop then **narrows** it by host-known kind. Research/review/validation use view scope; implementation/repair/landing require successful isolated worktree allocation. Definitions cannot contain providers, models, capabilities, tool scopes, permissions, grants or worktree settings. Existing router ceilings, provider permission clamps and project delivery policy remain authoritative. View scope describes the facade tool tier, not a newly invented OS sandbox.

Admission refuses an ineligible step, missing binding, foreign manager/project, contract/message override or session resume/retry. A per-task admission lock spans asynchronous allocation. Every configured review uses a fresh worker; an implementer's session cannot be reused. Unrelated standalone launches remain unbound; unknown optional stage labels are omitted rather than blocking the launch. Historical tasks are never retroactively bound.

Actual accepted dispatches and worker results update the pinned steps. Planned, dispatched, skipped, blocked, failed and completed are distinct. Idle/ended does not establish completion. `completed` means the worker returned a valid result contract; **it does not mean its verdict passed**. The reported object is stored separately (including “changes required”). Missing/invalid results fail the step, structured escalation blocks it, and pending provider decisions are visible. Wakes carry the pinned policy and next instructions. Validation reads the pinned contract even if a restored session lacks its original schema metadata. Recent agents → task details shows the revision, skipped reasons, dispatch sessions, frozen contracts and reported outcomes. Legacy tasks say workflow unknown.

Pins survive definition edits, disable/deletion and desktop reload; active pins are not evicted to make room. The existing explicit worker-adoption operation transfers pinned task ownership to the replacement manager. Live/revived managers receive workflow discovery on the next Fleet ask; no manager restart is performed. The on-disk Claude artifact workflow watcher remains separate.

## Persistence and APIs

`<configDir>/workflow-definitions.json` is version 1, host-global, capped at 100 definitions/2 MiB. Definitions have positive integer revisions, at most 64 KiB each; pinned snapshots cap at 256 KiB. Every store operation rereads under an O_EXCL cross-process lock and writes atomically. Unknown fields and invalid versions fail without rewriting the file. Starter seed markers add new starters without overwriting existing content. Default/project-selected definitions cannot be disabled/deleted, including through update. Pins retain deleted custom definitions independently.

Selection lives separately in `config.yaml`: `agents.defaultWorkflowId`, `agents.workflowSelectionRevision`, and `projects[cwd].workflowId`. Absence inherits the shipped default/global selection. Selection changes compare `expectedRevision` against `selectionRevision`. Both TS and Go generic config writers preserve existing selection fields and refuse attempts to change them or remove a selected project. Use the workflow API to inherit before forgetting that project. An unrelated project settings save does not erase its workflow. Stale edits return `code: conflict` with `currentRevision`; the editor preserves the draft and offers reload/discard recovery.

The local IPC `fleetWorkflowRequest` and desktop capability `fleetWorkflows.request` share one service. MCP exposes:

- `list_workflows`, `get_workflow`, `validate_workflow`
- `create_workflow`, `update_workflow`, `clone_workflow`, `disable_workflow`, `delete_workflow`
- `select_default_workflow`, `select_project_workflow` (explicit `workflowId:null` inherits)
- `start_workflow`, `next_workflow_step`, `decide_workflow_step`

Mutation/clone/delete calls use the definition's `expectedRevision`; selectors use the catalog's `selectionRevision`. Custom creation accepts a unique slug id and assigns revision 1. Errors are `{ok:false,code,error,currentRevision?}`. MCP caller identity is stamped from the request credential, never accepted in the public tool schema. The bus admits this method only from the actual local host control-plane credential; scoped/plugin/federated connections are rejected. The facade requires an authenticated local session; runtime operations additionally verify live manager and project ownership. Human host management uses desktop Settings.

The headless Go brain returns explicit unavailable and rejects workflow-bound spawn metadata. Its config writer preserves selections but there is **no headless executor**. Peer forwarding is refused, and clients lacking the IPC/capability show unavailable. No runtime parity or unattended execution is claimed.

## Verification and entrypoints

Read `main/shared/fleetWorkflow.ts`, `main/services/fleetWorkflowStore.ts`, `main/services/fleetWorkflowService.ts`, `main/services/fleetWorkflowRuntime.ts` and `components/settings/FleetWorkflowsSection.tsx` first (all under `apps/desktop/src`). The spawn/wake/history integration is in `hubCapabilities.ts`, `supervisorNudge.ts` and `dispatchHistoryStore.ts`.

`npm run test:dispatch-chain` runs authenticated HTTP MCP → real Go bus/routing → desktop spawn → provider-output wake validation → persisted history. Only the provider boundary is synthetic; it uses private scratch config, real git worktrees and no credentials/model calls. It tests distinct selected sequences, scout skip, stale CAS, frozen templates/revisions, independent review, no-review labelling, owner rejection, scope/grant ceilings and restart reads. Unit/config/Go guard tests and Chromium workflow-form tests cover the remaining boundaries. The browser harness is synthetic IPC, production forms/bridge/task projection, at 360/1280 px in light and Dracula; it does not start Electron or live services.
