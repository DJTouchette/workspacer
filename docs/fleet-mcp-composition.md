# Fleet Manager MCP consolidation

The manager can now use three calls for ordinary new work:

1. `manager_context` reads pending requests and, optionally, current task state.
2. `resolve_manager_request` records the manager's interpretation and returns
   `nextActions` with current step instructions.
3. `dispatch_workflow_step` prepares that exact step, selects its model, binds
   the host's pinned metadata, and spawns the worker.

The manager still decides what work means and whether it is authorized. The
host performs the repetitive transport and metadata work. Intermediate task
snapshots, routing capacity reports, and rendered prompt copies do not need to
pass through another model turn.

| Sequence | Before | Composed path |
| --- | --- | --- |
| Pending inbox plus four task inspections | Five calls | One `manager_context` call; independent reads run concurrently |
| Ready workflow step | Read next step, select model, spawn | One `dispatch_workflow_step` call when the current step/revision is known |
| Conditional step the manager chooses to run | Decide, select model, spawn | Supply explicit `run:true` and `reason` on dispatch |
| Dispatch and active-context watch | Spawn, then `notify_when` | Optional `watchContextUsedPct` on local dispatch |
| Resolve new requests or accept an outcome, then discover next work | Mutation followed by next-step reads | Bounded `nextActions` in the mutation receipt |

These counts describe model-facing calls, not eliminated host validation or a
measured end-to-end latency percentage. Approval questions, missing evidence,
and uncertain admission can still require separate turns.

## Dispatch contract

`dispatch_workflow_step` requires `taskId`, the local task `cwd`, the exact
`stepId`, and `expectedTaskRevision`. Supply the pinned template's task-specific
`templateParams`; optional fields include a label, explicit conditional
`run`/`reason`, routing constraints, a granted profile, permission preference,
and a local context watch.

The tool derives role, template, stage, predecessor dispatch, parent identity,
and the reviewer's previous provider. It uses the existing router and
`spawnWithGrants`, then the ordinary `agents.spawn` bus route and desktop
`managerDispatch(workflowSpawn(...))` guards. It does not introduce a second
spawn implementation or a model/permission override.

For paired execution, pass `executionTarget:"paired"` and an exact `remoteCwd`
from `list_dispatch_targets`. Routing uses that remote directory; task ownership
continues to use the local `cwd`. No remote path is inferred or translated.
The optional context watch is local-only.

A conditional `run:false` records only that skip and stops. It never launches
the following step. Required steps cannot be skipped. Explicit decisions may
commit before a later routing/spawn failure, so failures instruct the manager
to refresh state rather than blindly repeat the call.

The task revision is checked again at the actual spawn boundary. Changed,
reserved, already-dispatched, foreign-owned, cancelled, or dependency-blocked
work is refused. Repeating the original call cannot silently advance to another
step. Unknown spawn admission is reported explicitly and is never retried by
the composition. A watch failure retains the successful worker receipt and
instructs the manager to retry only the watch.

Spawn receipts retain host warnings, permission clamps, identities, delivery
and admission fields. The rendered worker prompt is omitted and marked as such;
a compact routing selection is included. Existing low-level tools remain
available for inspection and unusual operations.

## Context and next actions

`manager_context({tasks:[{taskId,cwd}, ...]})` accepts at most four tasks and
always reads the pending inbox. It returns per-task errors without discarding
successful reads. Tasks include revision, workflow step outcomes, relevant
metadata, current instructions and ready dispatch inputs. Template bodies and
historical attempts are omitted. Evidence above 24 KiB per task is explicitly
marked `contentDeferred`; fetch that task with `next_workflow_step` before
accepting an outcome. Numeric evidence is preserved without float rounding.
Each row has its own revision; this is not an atomic fleet-wide snapshot.

Request resolution and outcome acceptance return up to four `nextActions`, with
`nextActionsRemaining` when more exist. Projection failures do not disguise an
already-committed mutation as a failure. Ready followups still require an
explicit, authorized manager dispatch.

## Other sequences reviewed

- `project_status` and `respawn_with` already compose their underlying calls;
  their independent semantics remain intact.
- Reference edits retain their read/revision check so a convenience write does
  not overwrite newer human edits.
- Brief updates remain deliberate: combining them unconditionally with spawn
  would introduce partial-write/retry ambiguity and duplicate bookkeeping.
- Outcome acceptance is never bundled into automatic spawning or publishing;
  it requires the manager's judgment of concrete evidence and existing authority.

## Validation

The authenticated MCP → hub → desktop integration harness covers local and
paired composition, conditional skip, permission clamps, actual worker task
binding, foreign ownership, stale/repeated dispatches, concurrent calls while
admission is held, and a lost provider acknowledgement. MCP unit/race tests
cover route refusal/malformed replies, spoofed fields, tier boundaries, watch
failure, concurrent context reads, partial errors and deferred evidence.
