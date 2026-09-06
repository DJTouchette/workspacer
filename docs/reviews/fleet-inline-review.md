# Fleet inline review — local desktop slice C

Review target: `wks/workspacer-inline-fleet-review-changes`, based on landed
`ab85e61a`. Exact change range: `git diff ab85e61a..wks/workspacer-inline-fleet-review-changes`.
No merge, push, live worker action, or application restart is part of this commit.

## Read first

1. `apps/desktop/src/main/services/fleetReviewStore.ts` — capture, bounds, ownership,
   selectors, persisted bytes, revocation.
2. `apps/desktop/src/main/services/worktreeService.ts`, `hubCapabilities.ts`, and
   `supervisorNudge.ts` — host allocation and completion/teardown integration.
3. `apps/desktop/src/renderer/src/components/claude/FleetReview.tsx` and
   `FleetMessageCard.tsx` — same-card disclosure using the existing DiffViewer/parser.
4. `apps/desktop/src/main/services/fleetReviewStore.test.ts` — real Git lifecycle
   evidence; `ipc.test.ts` and `tests/e2e/fleetReview.test.ts` cover the transport and UI.

## Contract

Fleet `agents.spawn` with a successfully host-allocated local worktree records its
canonical project root, allocated directory, Git common directory, branch, and base
commit. Base is read immediately after allocation, before setup commands or the
worker run. Managed Claude, Claude PTY, and Codex dispatch routes register this same
host metadata. Caller/worker result fields cannot supply it. Each registration has a host generation; an in-flight capture cannot cross into a replacement allocation.

The completed-result wake captures the full base-to-head range from the allocated
worktree. The host resolves head and checks repository/branch identity, ancestry,
and Git dirty status. Dirty/untracked work produces an explicit unavailable-for-inline
record; it never produces a partial committed-only review. A resumed worker or
changed final reply during capture suppresses the stale finish wake.

Worktree removal awaits capture, including when it precedes the 1500 ms coalesced
wake. This capture is labelled **before worktree removal**, independently from
**turn ended** and **session ended**. None of these claims descendant writers have
been verified stopped. The wake can reuse a pre-removal capture after the directory
has disappeared. A changed branch or repository cannot reuse a stale capture.

The transcript carries only an opaque evidence ID. Read and Forget requests contain
`ownerSessionId`, `workerSessionId`, `evidenceId`, and optionally the exact recorded
file name. The UI gets the owner from its containing chat, never from worker JSON.
IPC applies the store's ownership and selector validation, including to local calls.
Unknown keys (cwd, revision, command, etc.), unknown IDs, wrong owners/workers, and
unrecorded/traversal selectors are refused. Reads return stored metadata/diff bytes;
no subsequent Git, filesystem read of the worktree, grant, or mutation is involved.

This slice deliberately exposes **no hub-bus capability**. Optional preload methods
are absent on remote/web/headless backends, which show an unsupported explanation.
There is no delegation to a peer or generic `git.*` fallback, and no Go capability
registration change. Existing stopped/federated filesystem and replay grant rules
are unchanged.

Review changes expands in the same Fleet result card and opens the first recorded
file diff. Other files are selected inline. The header shows the project, exact full
base → head, branch, originating worker, directory, capture time/status, and lifecycle.
There are no stage/commit/push/editor/pane-spawn actions. Structured checks and outcome
booleans are labelled worker-reported and rendered without a verified-pass badge.

## Retention and limits

The private `getConfigDir()/fleet-review.json` store uses atomic writes and mode
0600; it is outside the existing bus-readable config store carve-outs. Source
identity and immutable bytes persist across worker close, branch deletion, worktree
prune, and desktop restart. Ownership remains with the originating manager/result;
manager succession does not transfer this authority.

Current conversation/analytics history has no individual-result delete primitive.
**Forget review data** deletes that result's record and revokes its worker allocation,
so a late completion/capture cannot recreate it. Already-captured other result IDs
remain separately owned/revocable. Removing a worker card is not deleting manager
history and does not revoke its captured result.

Limits are 64 evidence records, 256 allocation records, 200 files and 1 MiB of diff
bytes per capture, and a 24 MiB serialized store. Oldest records are evicted when
needed; old result IDs then report unavailable/revoked. Git subprocesses have 15 s
and 1 MiB output limits; the per-file capture loop also has a 15 s deadline. Oversize
captures retain no partial file list or diff. Binary files retain Git's summary only.
Renames/deletions/mode changes and whitespace/Unicode names use NUL-delimited Git
metadata and literal pathspecs; symlinks retain only committed link text. Unsupported
filename decoding, restricted paths, or changed/unresolvable identity fail closed.

No repository copy, untracked bytes, Git config, or credential store is retained.
The existing secret-path gate plus conservative credential filename exclusions
(including `.env*`, `.ssh`, `.aws`, private-key suffixes) reject the entire range.
These exclusions are not a general secret scanner for arbitrary committed source.

## Validation

Linux, Node 22.22.2, `CI=1`; checks run sequentially with one test worker.
The supplied dependency directories were isolated into this worktree; renderer
`npm ci --ignore-scripts` repaired a missing testing-library dependency.

- Typecheck: main and renderer passed; changed-file Prettier check and `git diff --check` passed.
- Main: 407 tests across the primary capture/spawn/wake/grant/federation/lifecycle suites.
- Main: 453 tests across IPC/preload, delegation, spawn, session-store, ownership and
  capture suites, including production IPC ownership/selector refusals.
- Main: 41 Git/worktree/capture tests and 47 final capture/wake tests passed, including allocation-generation replacement and revocation during capture.
- Renderer: 111 tests across Fleet cards/manager, structured results, permission labels,
  ConversationMessage and web backend; the structured-field suite also passed (10 tests).
- Real Git tests exercise two worker commits, production allocation/finish/cleanup,
  teardown before wake, removal of the branch/worktree, changed manager HEAD, store
  reconstruction, identical retained bytes, revocation, bounds, dirty/unresolvable
  states, renamed/deleted/binary files, unusual names, symlinks, executable modes,
  and a symlinked worktree root. These are real temporary repositories; daemon
  message delivery is mocked. Retention-limit setup seeds stored records.
- Chromium 148.0.7778.96: four production Fleet component cases at 360/1280 px in
  Everforest and Light, click → first inline diff, keyboard focus/activation,
  containment, no review/worker pane-open event or extra browser page, and Forget. The evidence transport is mocked;
  screenshots were inspected. No running user app or worker was exercised.

Independent review and local merge remain the manager's next step. Full Electron
runtime against a live daemon, remote/headless evidence service, a future history-wide
delete primitive, and general credential-content scanning are outside this slice.
