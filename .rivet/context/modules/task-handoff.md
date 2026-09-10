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
  `.workspacer/reports`; those reports must already be outside Git status
  (for example through the repository owner's ignore policy). Preparation
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
  These are materialized-tree limits, not a total Git-history disk quota.
- V1 supports portable ASCII paths and ordinary blobs with literal CRLF/binary
  bytes. Unicode/Windows aliases, symlinks, submodules, attributes, LFS pointers,
  sparse/partial checkouts and execution-valued repository configuration fail
  preflight. Windows hosts explicitly fail until ACL custody is implemented.
- Worker test-pass text remains a worker claim. Host-verified base/head and
  immutable local review capture do not certify those claims as test evidence.
- Terminal message acknowledgment never authorizes deletion. Host-user
  accepted disposition plus matching receiver custody and seven days makes
  execution cleanup eligible; keep/rejection, unknown or active workers,
  changed refs, dirty trees and unexpected files retain the allocation.
  Cleanup removes the execution worktree and selected spools; receiver review
  custody and quarantine Git repositories remain retained. Ordinary journal
  eviction is not custody or Git object GC.

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
