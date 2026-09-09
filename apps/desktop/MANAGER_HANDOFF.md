# Local Fleet Manager handoff

The manager composer action checkpoints the source, validates its handoff,
starts a fresh manager, transfers workers and tasks, and binds the existing
workspace/tab/pane to that successor. The ordinary-agent dialog is unchanged.
Normal completion requires no final confirmation.

## Supported boundary

- An owned local desktop manager using stream transport, with recorded launch
  provenance and manager/operator grants. Missing provenance and custom launch
  integrations return unavailable rather than guessing launch settings.
- The desktop must own `agents.spawn`, `agents.sendMessage`, `agents.reparent`
  and `fleetWorkflows.request` on the hub. Remote-client mode, an adopted
  headless hub, withheld capabilities, and remote children are unavailable.
- The successor uses the real manager spawn path with an internal
  `replacementSessionId`, **no resume ID and no first message**. It must report
  zero user prompts before ownership transfer. Startup-ping integration must
  preserve this parked launch invariant.

## Transaction and recovery

`manager-replacements.json` in the desktop config directory stores operation
identity, launch provenance, current parent metadata, original transfer IDs,
validated handoff bytes, completion signatures and delivery evidence. It uses
the existing atomic JSON writer. Task storage remains JSON.

The agent writes an operation-specific proposal under the fleet root's
`.workspacer/manager-handoffs/<operationId>/`. Its receipt must correlate the
source and operation IDs and the exact file hash. Checkpoint brief pointers and
hashes, worker instructions, task IDs and pending decisions are validated.
The host writes a separate `validated-handoff.json` and retains those exact
bytes in the journal. Shared `handoff.md`, generic idle and prose are not
completion signals. There is no mechanical fallback.

Preparation/validation/spawn failure leaves the source manager and pane in
place. Dispatch admission is fenced during preparation and ownership transfer.
An adoption reservation refusal is checked inside the task-store file lock and
changes no task or worker parent. Every source-owned task transfers in place,
including standalone tasks; attempts, IDs, revisions, links, waivers and pins
remain intact. Existing review captures retain their origin and become readable
by the successor; future captures follow the transferred allocation.

Ownership intent precedes transfer. A known task-store refusal releases the old
manager; an uncertain or partial transfer requires reconciliation. Once ownership
is committed, recovery follows the new owner and same-pane binding. It never
automatically resumes the retired predecessor or a failed parked candidate.
Later accepted manager settings and explicit manual adoption update recovery
metadata without rewriting the original transfer receipt.

The daemon accepts chat input into memory and has no durable delivery receipt.
**This feature does not promise exactly-once delivery.** In-flight requests are
correlated by host request ID. Missing acknowledgement is recorded as uncertain,
automatic progression stops, and interruption is attempted. The UI retains the
bytes and offers inspection, explicit confirmation or a labelled retry that may
duplicate work. A retry has its own record; the original uncertainty remains in
the audit. Explicit daemon not-ready refusals can be retried within the bounded
delivery wait because they did not accept the message.

Claudemon restores stopped rows after its own restart; the journal does not
invent live processes. Missing/stopped successors remain a recovery error.
In-process threshold watches follow the successor, but retain their existing
non-durable restart semantics. No daemon admission protocol or SQLite migration
is introduced.

Retention is bounded at 32 operations, 128 manager launch records and 8 MiB.
Capacity exhaustion refuses new work rather than silently deleting ownership or
unacknowledged evidence. The standalone `/handoff` fallback remains available.

## Reviewer entry points

- `src/main/services/managerReplacementService.ts`: transaction and failures.
- `src/main/services/managerReplacement.ts`: actual runtime integration and host
  ownership checks; `managerReplacementState.ts`: JSON authority and fences.
- `claudeSessionStore.reparentChildren` and
  `dispatchHistoryStore.adoptWorkflowTasks`: task-first transfer and reservation
  guard. The latter retains its historical API name and transfers **all** tasks.
- `src/renderer/src/lib/managerReplacement.ts`, `useAgentManager.ts`,
  `ClaudePane.tsx` and `ManagerHandoffStatus.tsx`: same-pane binding and recovery UI.

## Validation

The service fault fixtures cover preparation, spawn, validation, transfer, pane
binding, duplicate requests, cancellation and acknowledgement ambiguity.
`managerReplacement.integration.test.ts` uses the real manager spawn, tokens,
session/task stores and wake coalescer with fixture daemon/provider boundaries.
It checks current settings reaching the spawn, pending workers, both task kinds,
wrong identity/cwd/provider/grants and later manual adoption. Reservation tests
exercise a second JSON-store instance while dispatch acceptance is outstanding.

`tests/e2e/managerHandoff.test.ts` uses its own ephemeral Vite server and Chromium;
it exercises normal replacement, reload, failed preparation and explicit recovery
actions. No live manager, worker, app or daemon is replaced/restarted by these
tests. On Node 26, run the renderer suite with
`NODE_OPTIONS=--no-experimental-webstorage` so jsdom owns browser storage.

Inspector core commits `7c32ed0f` and `6992a93b` are incorporated. Its final UI
commit `6e8411cc` is intentionally left for manager integration and independent
combined review.
