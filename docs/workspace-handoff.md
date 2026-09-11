# Workspace task handoff

An exact handoff copies a clean Git checkpoint and explicitly selected task
artifacts to a compatible paired Workspacer execution target. Git travels through
an approved shared remote using generated temporary refs. Artifact bytes travel
over the existing authenticated paired connection. The workspace manager stays
on the originating desktop. No additional model coordinates transfer steps.

`spawn_agent` accepts `taskSource` with `binding`, `artifacts`, and `outputs`.
Selections are arrays of `{ "name": "scout.md", "kind": "report" }`; supported
kinds are `report`, `criteria`, `image`, and `log`. Every selected file is required.
Use normal task/predecessor fields to continue the same owned task. The host
selects a local predecessor's actual workspace, or previously imported remote
custody. It refuses foreign task references and unavailable predecessor bytes.

The Task Inspector shows preparation on the destination, execution, and verified
review availability. It opens imported artifacts by task and manifest index,
checks their bytes again, and offers preparation/result recovery. An uncertain
spawn is never retried through the preparation recovery action. Returned test
claims remain claims; the host captures Git identity and independent review
evidence, not proof that the worker's stated tests ran.

## Repository approval

Install `handoff-bindings.json` in the Workspacer configuration directory on
both endpoints. This is host-owned policy, read by the brain in both the default
desktop catalog route and full execution mode. It is not a worker prompt or an
automatic credential installer.

For example, the source policy can contain:

```json
[
  {
    "id": "shared-repository-desktop-a",
    "revision": "1",
    "repository": "/absolute/source/repository",
    "remote": "https://git.example.test/team/approved.git",
    "refPrefix": "refs/heads/wks-transfer",
    "owner": "local-host",
    "origin": "desktop-a-stable-origin",
    "export": true,
    "import": true,
    "cleanup": true
  }
]
```

The receiver uses the same binding ID, revision, origin, approved remote, and
ref prefix. Its `repository` is its own existing repository path, and `owner`
is the authenticated credential fingerprint returned in
`list_dispatch_targets` → `handoff.ownerKey`. Never use a URL as repository
identity. Use different binding/origin IDs for different desktop origins; shared
operator credentials remain shared administrative authority, not tenant isolation.

An optional `credentialHelper` explicitly selects an already authorized,
noninteractive host Git adapter. No helper is inherited from peer configuration.
`tlsCAFile` may name a trusted local CA file. HTTPS and normalized `ssh://` URLs
are supported; embedded passwords, arbitrary transport helpers, automatic forks,
new credentials, and fallback remotes are not. Changing a binding requires an
explicit revision change. Missing mapping or Git access stops before a worker.

## Files and checkpoints

Initial report selections are relative to the selected source workspace's
`.workspacer/reports`. Continuations can select the preceding task's verified
returned artifacts. The receiver materializes them in
`.workspacer/handoffs/<attempt-id>` inside a generated isolated worktree. Required
outputs use that same folder. The host supplies the report destination to report
templates. The artifact folder is excluded from normal commits; a committed
`.workspacer` tree is refused as code input/output.

Source Git state must be clean apart from the explicitly selected artifact
inputs. The host never stages, commits, stashes, resets, or cleans user work.
It pins the full commit/object format, verifies the fetched ref's exact object
ID, and creates the execution worktree at that commit. Receiver `HEAD` is not an
input. Result import verifies base/head ancestry, creates a separate local review
worktree, and promotes a generated review ref into the origin repository without
writing its index, working tree, active branch, or `FETCH_HEAD`.

V1 materializes canonical Git blob bytes with LF policy, without setup hooks or
dependency links. Gitlinks, code symlinks, LFS pointers, Git attributes, partial
or shallow inputs, and unsupported path/configuration features fail explicitly.
Artifact names use a conservative portable ASCII subset. Traversal, absolute
paths, administrative aliases, device names, case collisions, links and special
files are refused. Markdown images must reference selected relative artifacts;
HTML embeds, network image URLs and unsupported image-reference syntax are
refused. No URLs are fetched from reports.

Default bounds are 128 files, 16 MiB per artifact, 64 MiB per task artifact set,
256 KiB chunks, and 32 retained task allocations per origin. Task summaries saved
for recovery are capped at 64 KiB. Capacity failure preserves pending data.
These are policy bounds, not throughput claims. Windows task directories use
real ACLs; Unix directories use private modes.

## Ownership, recovery and retention

The brain owns the byte stores and receipts. The desktop owns task admission and
manager delivery. A prepared receipt includes the plan digest and allocation
identity, and the single-use lease must consume that exact receipt. Dispatch
protocol 2 and handoff version 1 are negotiated separately. Exact handoffs to
older peers are refused; legacy spawns without `taskSource` retain receiver-checkout
semantics and do not transfer source code or reports.

Chunk retries compare existing bytes. Required hashes are verified before
materialization, and materialization retries cannot overwrite unexpected files.
Restarted preparation can reuse the saved plan and allocation. Missing/moved refs,
unavailable mappings, dirty results and uncertain admission remain explicit
states. Code/artifact custody acknowledgment is separate from terminal-message
acknowledgment. Manager replacement does not change the original workspace owner.

`Accept outputs` records disposition for the exact imported custody digest.
`Keep` prevents automatic cleanup. Execution-side cleanup waits seven days,
requires receiver custody and fresh stopped-worker checks, refuses changed or
unexpected files, and compare-deletes only the recorded generated refs. Partial
cleanup is journaled. Origin review custody is retained; it is not expired to
make room for new work. The bounded store can require operator archival when
retained review custody fills it. No merge or protected-branch push is automatic.

Independent execution-host managers cannot create unsolicited desktop tasks
through this protocol. An offer/accept inbox for those jobs is outside V1.

## Review and validation entry points

- `apps/desktop/src/main/services/taskHandoff.ts`: host transfer and local continuation.
- `apps/desktop/src/main/services/pairedDispatch.ts`: admission, recovery and custody-before-wake.
- `services/hub/cmd/brain/taskhandoff.go`: policy, Git/artifact lifecycle and receipts.
- `services/hub/cmd/brain/dispatchlease.go`: exact allocation claim and retention.
- `services/hub/internal/taskartifacts`: portable bytes, paths and Git boundaries.
- `apps/desktop/tests/integration/dispatchChain.integration.ts`: real MCP/desktop/paired-host flow.
- `services/hub/cmd/brain/taskhandoff_fixture_test.go`: private HTTPS Git fixtures and fake-clock checks.

The hosted workflow runs full desktop/hub suites and private fixtures. It does
not contact a personal execution target. A live deployment trial is separate
from hosted validation.
