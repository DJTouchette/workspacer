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
  zero user prompts before ownership transfer. Provider readiness uses its separate disposable CLI adapter; it never
  submits a manager prompt or bypasses this parked launch invariant.

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
tests. Use the repository-pinned Node 22 for both desktop suites.

### Windows checkpoint validation regression

`managerReplacementArtifact.windows.test.ts` runs the production validator with
Node's Win32 path implementation and fixture filesystem responses. Before the
fix, both a lower-case drive changed by `realpath` and a forward-slash launch
root reproduced `Checkpoint brief pointer or content hash is invalid` with
correct SHA-256 values. Comparison now tolerates Windows drive-letter case and
slash spelling only. Component case, dot segments, root containment, allowed
basenames, link rejection, file limits and exact operation identity remain
restricted. POSIX comparisons remain byte-exact.

`managerReplacementArtifact.test.ts` uses native temporary files and includes a
Windows-only drive/separator case. Both suites run in the existing
`containment-windows` CI job. Linux execution of the Win32 fixture is source
coverage, not evidence of an executed Windows handoff.

The preparation prompt still requires SHA-256 values from final file bytes:
there is no evidence that the reported manager invented a hash, so this fix
does not change that protocol or accept missing/synthesized hashes. Hashing
reads buffers, preserving CRLF and non-ASCII bytes; proposal JSON must be valid
UTF-8 so the sealed string reproduces its exact bytes. Checkpoint errors name
the array index and category (pointer, file inspection/size, hash format, hash
mismatch), never the path or brief contents. A mismatch can mean a stale file
or an incorrect hash; it does not establish which.

The reported error is emitted by `validateManagerArtifact` during preparation,
before receipt-hash checking, sealing or spawn. `validateSuccessor` later checks
the parked session identity, settings, grants and readiness; it does not parse
checkpoint pointers again. An allocated successor ID alone proves no spawn or
ownership transfer. To classify a remaining remote failure, collect only the
offending checkpoint index/category, whether root/pointer/realpath differ in
drive, slash or component case, whether the hash is 64 lowercase hex characters,
and whether a locally computed digest matches it. Brief contents and full
private paths are unnecessary.

## Combined integration for review

Handoff base: `a8bc902e71066dd5ccc78e1cd429c3e7c10df17f`.
Reviewed Inspector source: `6e8411cc06b73150ce3519e37f9f1b1753da6fb4`.
Pinned readiness source: `115f88b0dac5e4b5dc78fc6fd333ea73ebd2c8fe`.
Both exact sources are merge ancestors on the isolated handoff branch. Inspector's
original core ancestry is retained alongside the earlier core cherry-picks
`5453df56` / `b34a7f92`; its final refinements are present once in the source.
No primary merge or independent combined review has been performed.

Inspector conflict resolutions: `playwright.config.ts`, `src/main/ipc.ts`,
`src/main/services/dispatchHistoryStore.ts`, `fleetWorkflowRuntime.ts`,
`fleetWorkflowRuntime.test.ts`, `taskInspector.test.ts`, and
`src/renderer/src/backend/webBackend.ts`. Readiness conflicts: `src/main/index.ts`,
`ipc.ts`, `ipc.test.ts`, `preload.test.ts`, and
`src/renderer/tests/backend/backendParity.test.ts`.

The combined fixture verifies manual refusal and automatic waiting while a real
source task reservation is held, a host-user waiver for a different future step,
references, delayed acceptance, and transfer without losing attempt identity or
audit. Lifecycle writes continue advancing revisions while a manager waits.
The startup scheduler runs while the successor is parked at pane binding; only
the separate provider inference boundary is stubbed there. Readiness config and
native fixtures independently exercise the real CLI adapter. Browser fixtures
use private caches and ephemeral ports for Inspector, handoff and readiness.

Additional reviewer entry points: `docs/features/task-inspector.md`,
`docs/fleet-provider-readiness.md`, `src/main/services/providerReadinessRuntime.ts`,
`src/main/services/managerReplacement.integration.test.ts`, and
`src/main/services/managerTaskOwnership.test.ts`. Acceptance requires all three
local feature entry points, task-first all-task transfer with reservation/CAS
protection, parked successors, and truthful recovery-required delivery status.
The crash/acknowledgement limitation above remains accepted and unchanged.


Combined verification on Linux, Node 22.22.2, Go 1.25.4 and Chromium
148.0.7778.96:

- Main and renderer typechecks passed.
- Main: 175 files passed, 3,439 tests passed, seven opt-in native tests skipped.
- Renderer: 199 files and 1,890 tests passed.
- Chromium: 14 Task Inspector, five manager handoff and eight readiness/detection
  cases passed, with one worker and private fixture servers.
- Installed Codex loopback/wrapper fixtures: six passed; live-account test skipped.
- Go brain config/default/startup opt-out checks passed with `-count=1`.
- Changed TypeScript/TSX formatting and `git diff --check` passed.

Primary readiness-to-integration review range: `115f88b0..HEAD` (77 files).
Full three-feature range: `551e1731..HEAD` (113 files). Integration continuation
from the accepted handoff: `a8bc902e..HEAD` (72 files). Exact source ancestry and
conflict paths are listed above. No live config/fleet mutation, daemon/app restart,
live provider inference, primary merge, push or release build was performed.
Independent combined review and manager-controlled landing remain pending.
