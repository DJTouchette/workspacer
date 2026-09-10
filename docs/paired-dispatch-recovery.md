# Desktop-local manager with paired workers

The desktop manager remains local. In Connect to Server, select **Workers only**
(or convert an existing client connection with **Keep manager here**). The existing
`remote-server.json` credential stays in main; `mode: workers` makes
`getRemoteServer()` return null, so normal local daemon startup and backend
selection remain in force. No peer-file editing, repository clone, automatic
routing, deployment or credential copying is involved.

Managers explicitly use `list_dispatch_targets`, then `select_dispatch_model`
with a provider and repository from that target's own readiness answer. Dispatch
with `spawn_agent(executionTarget: "paired", remoteCwd: <remote repository>, ...)`.
`cwd`, `parentSessionId`, task/request IDs and workflow-step IDs still describe the
local manager's task. Ordinary spawns without an execution target are unchanged.

## Admission and return contract

- The local host validates the authenticated manager and pinned workflow. It
  retains an unguessable dispatch nonce, a separate local card ID, and a
  destination fingerprint bound to the pairing endpoint and credential.
- The remote host probes its actual CLI login status and existing repository
  choices. Unknown login state is unavailable. Protocol 2 is required; a peer
  at production source `3a16f4bb` is unsupported until upgraded.
- Remote preparation returns a single-use lease bound to that origin credential,
  provider and canonical remote cwd. Ship work allocates an actual isolated Git
  worktree. No setup hook, clone, push or shared-checkout fallback runs.
- Local workflow admission is recorded before remote process startup. The remote
  lease is durably claimed before provider launch. Uncertain admission is never
  retried as a new spawn. Unclaimed leases expire; claimed/unknown work is retained.
- Only supported execution metadata and the rendered task cross the boundary.
  Local workflow/request records and local full-access grants do not. Remote
  token grants and routing ceilings remain authoritative; paired workers request
  approvals on. Remote session facades mint their own scoped credentials.
- Progress, blocks and final results return over the outbound authenticated
  connection. Operator peers cannot publish admission/result events or assert a
  hub stamp. The origin verifies the nonce, destination, worker and live local
  manager before persisting evidence, updating task state and delivering a fleet
  wake. Remote snapshots cannot select recipients or grant local filesystem roots.
- Results use the original local task/request lineage. Adoption transfers origin
  ownership to the successor. The sidebar marks paired workers; Inspector records
  their execution host and remote worktree without enabling a local folder open.
  Default desktop backend actions route through the local credential-owning host.
- Reconnect replays retained state with sequence deduplication. Acknowledged
  terminal receipts may be reclaimed; unacknowledged work is not evicted to make
  room. A crash during local wake delivery leaves an explicit unknown receipt,
  requiring reconciliation rather than an automatic duplicate manager turn.

## Recovery and verification

Recovery commit `23e64e83b98f34d93549e28294b272136fdde339` preserves
`091583e75d4f3e81a3988749ad66264c0eb19019` and
`3fc1ef0c0bcd4518389e724f1f4acf548ada8499` atop `3a16f4bb`. The tracked binary diff
and the two named new source files were compared byte-for-byte. The old worktree
was untouched; generated skills, dependencies, caches and secrets were excluded.

Execution is hosted-only. The feature suite at `9489a1f5` passed in
[run 34503712829](https://github.com/DJTouchette/workspacer/actions/runs/34503712829),
and the complete CI matrix at that same commit passed in
[run 34503713309](https://github.com/DJTouchette/workspacer/actions/runs/34503713309).
Subsequent hardening requires fresh exact-commit verification; the final handoff
records those runs. No local build/test/browser/server or child agent was used.

The hosted dispatch fixture exercises real authenticated local MCP, desktop
capability handling, paired WebSocket authorization, brain admission/watchers,
and local request/task persistence. Provider output and the local manager message
transport are synthetic. It covers remote isolation, message-once behavior,
progress/block/final wakes, reconnect/deduplication, adoption, limited tokens,
wrong paths and unsupported peers. Unit regressions additionally cover journal
failures, single-use leases, origin spoofing and interrupted delivery.

Reviewer entrypoints: `pairedDispatch.ts`, `pairedWorkerConnection.ts`,
`remoteDispatchRegistry.ts`, brain `dispatchlease.go`/`remotedispatch.go`, bus
`rpc.go`/`bus.go`, and `tests/integration/dispatchChain.integration.ts`.

## Reviewed rollout only

Neither endpoint has been deployed, restarted, re-paired or mutated here, and no
real remote worker was spawned. After independent review and green exact-commit
hosted CI, coordinate upgrades of both endpoints. Keep older peers visibly
unsupported. Then, with separate live-validation authority, discover an existing
remote repository and current provider readiness, and run one benign Claude task
from a manager that stays on the desktop. Verify one initial message, authenticated
progress/block/final delivery and the original local request/task outcome. Codex
must remain unavailable until the remote host actually reports a usable login.
