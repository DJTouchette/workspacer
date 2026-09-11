# Workspace task preparation and result custody

The desktop keeps manager/request/task/workflow ownership. `taskHandoff.ts`
orchestrates `agents.taskHandoff` against its catalog brain and its configured
paired execution host. Each brain owns its own receipts and byte spool.
`taskSource` carries an approved binding ID and selected artifacts/required
outputs. It contains no credentials, Git commands, or arbitrary destination.
The execution brain refuses unprepared `taskSource`; spawn consumes a receipt
through `dispatchPrepare` and its single-use lease.

## Required operator configuration

Install `handoff-bindings.json` in each host's Workspacer config directory.
This is separate from config.yaml and is read by `registry.handoffBinding` on
every operation. No UI, peer, or worker installs a binding. Example shape:

```json
[
  {
    "id": "approved-project-binding",
    "revision": "1",
    "repository": "/absolute/approved/project",
    "remote": "https://git.example.invalid/team/project.git",
    "refPrefix": "refs/heads/workspacer-transfer",
    "owner": "local-host",
    "origin": "origin-desktop-unique-id",
    "export": true,
    "import": true,
    "cleanup": true
  }
]
```

Use the already authorized shared repository remote; no new publication,
fork, credentials, or active-checkout pull is implied. IDs/origin are 16–128
lowercase ASCII letters/digits/underscore/hyphen, beginning with a letter or
digit. Both hosts share ID, revision, origin and ref prefix; each uses its own
absolute repository path. The execution binding's owner is the paired token
fingerprint computed by the hub, not the origin's local-host sentinel. Use
distinct credentials and origin IDs for distinct desktop owners. A shared
operator credential represents shared authority.

An optional host-approved `credentialHelper` supplies Git authentication;
`tlsCAFile` can name an absolute trusted CA file. These values never come from
the worker. Both directions require the corresponding import/export rights.
Unavailable mappings or remote access fail preparation. The optional
`displayName` in the existing worker pairing `remote-server.json` supplies the
task's target label; it does not affect target identity or authorization.

## Contract and limits

- Source must be a clean explicit commit. Required inputs are selected from
  `.workspacer/reports`; explicitly selected untracked reports do not require
  an ignore-file edit. Tracked modifications and other untracked WIP still
  fail the checkpoint check. Preparation
  freezes only the selected files and materializes them beneath
  `.workspacer/handoffs/<transfer-id>` before admission.
- Source and result use generated temporary Git refs with exact OID checks.
  No active receiver checkout changes. Imported results have an isolated
  local branch and required artifact bytes before review-ready.
- Local continuation selects an exact preceding remote attempt in the same
  desktop-owned task, uses its verified returned commit and artifacts, and
  durably claims a deterministic continuation receipt before launch.
- Artifacts: 128 files, 16 MiB each, 64 MiB total, 256 KiB chunks. Code tree:
  20,000 entries, 512 MiB aggregate; per-command output and time are bounded.
  Git blobs are limited to 32 MiB and verified with one bounded streaming
  cat-file process, rather than one process per file on Windows.
- Git quarantine admission counts actual retained/failed bytes plus durable
  reservations under a 6 GiB host budget, with a 2 GiB per-task reservation and
  a 2 GiB free-disk floor. A native cross-process lock prevents overlapping
  brains from double-booking. Interrupted reservations remain charged until
  the same task resumes. No pending/sole copy is evicted to create capacity.
- Fetch and worktree materialization run in a bounded helper: Windows uses a
  kill-on-close Job Object; POSIX uses a process group and a 512 MiB per-file
  kernel limit. Both monitor task bytes/free disk every 20 ms and check final
  usage. 128 MiB of the task allowance is kept for artifact materialization.
  Windows monitoring permits bounded scheduling/write overshoot; it is not an
  NTFS volume quota. The free-disk floor is headroom, not permission to delete
  unaccepted data. Provider-created files outside these Git operations are not
  sandboxed by this mechanism.
- V1 supports portable ASCII paths and ordinary blobs with literal CRLF/binary
  bytes. Unicode/Windows aliases, symlinks, submodules, attributes, LFS pointers,
  sparse/partial checkouts and execution-valued repository configuration fail
  preflight. Ordinary Windows source and return custody are supported: new
  storage gets an inheritable protected current-user/SYSTEM/Administrators DACL
  at creation, actual DACLs are checked after all materialization, and native volume
  plus file IDs bind allocations to receipts. Reparse entries are refused.
  Native extended paths and persisted private-repository `core.longpaths` handle
  production-length task IDs without changing global Git/Windows settings.
  Source status honors the safe effective `core.autocrlf` scalar; private result
  repositories persist canonical checkout settings so ordinary worker/review
  Git commands see the verified tree. POSIX mode bits are not Windows ACL proof.
- Worker test-pass text remains a worker claim. Host-verified base/head and
  immutable local review capture do not certify those claims as test evidence.
- Terminal message acknowledgment never authorizes deletion. Host-user
  accepted disposition plus matching receiver custody and seven days makes
  execution cleanup eligible; keep/rejection, unknown or active workers,
  changed refs, dirty trees and unexpected files retain the allocation.
  Cleanup removes the execution worktree and selected spools; receiver review
  custody and quarantine Git repositories remain retained. Ordinary journal
  eviction is not custody or Git object GC. Retained Git data remains charged;
  automatic Git GC is deliberately absent. Status includes used/reserved/limit
  bytes and an explicit retention explanation, surfaced by the disposition UI.
  The existing 32-retained-task limit is separate from byte admission and can
  also require explicit retention maintenance; acceptance is not receipt eviction.
  Storage inventory counts link metadata without traversing dependency links.
  Only task-root reservation records are parsed, never same-named user files.

## Review and validation entry points

Read `services/hub/cmd/brain/taskhandoff.go`,
`services/hub/internal/taskartifacts/{git,store,manifest}.go`, desktop
`taskHandoff.ts`, `pairedDispatch.ts`, and the handoff branch of
`hubCapabilities.ts`. Hosted fixtures are `taskhandoff_fixture_test.go` and
`tests/integration/dispatchChain.integration.ts`; the latter traverses real
MCP authorization, desktop admission and paired brain handlers with only
provider execution synthetic. No live deployment is required.

Recovery preserves old HEAD `7716523b90948ebd4ef82b6f181eee7250361bf0`, including
ancestors `5f814238`, `e946bafb`, and `2ed8b8ec`. The exact Go working delta was
banked in `fbbc18ec`; two additional desktop working deltas observed in the
old worktree were separately banked in `6781ac73`. Original files/dependencies
were left untouched. Existing CI at the old HEAD failed; it is not evidence
that this integrated feature was green before recovery.

## Native hosted evidence and transport limits

`workspace-handoff.yml` has native Linux/Windows custody, ACL, quota, and retry
fixtures, plus Windows source → Linux execution → Windows receipt/local
implementation jobs. The staged jobs carry frozen RPC envelopes and fixture Git
server state through GitHub workflow artifacts; each OS uses the production
handlers and a temporary HTTPS Git server. This is an explicit staged transport
fixture, not a claim of simultaneous cross-runner WebSocket connectivity.
The separate real MCP → desktop → paired backend integration covers live RPC
orchestration and local request/workflow ownership with synthetic providers.
No live Fly deployment or local application execution is part of validation.

Native API references: [Windows file security](https://learn.microsoft.com/en-us/windows/win32/fileio/file-security-and-access-rights)
and [handle-based file identity](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/ns-fileapi-by_handle_file_information).
See `task-handoff-reconciliation.md` for the retained parallel-work comparison.
