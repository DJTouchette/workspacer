# Intent provider synchronization slice

Reviewer entry points: `intentSourceSync.ts` maps provider observations;
`intentSourceSyncStore.ts` owns durable projections, artifact/event provenance,
account leases and backoff; `intentAutomationRuntime.ts` runs it on both owning
hosts. `IntentSources.tsx` exposes freshness and paged artifact review through the
existing intent request boundary. Project connections use `intentIntegrationStore.ts`
and shared `intentIntegrations.ts`; `IntentIntegrations.tsx` provides their editor
and attach flow. Configuration metadata lives in the Intent database, not config.yaml.

## Configure once per project

In an Intent's **Sources** tab, expand **Project tracker integrations**. Create a
named Jira Cloud or Azure DevOps connection and enter the **environment variable
name**, never its value. The owning host must already have that variable configured
(the existing `WORKSPACER_SOURCE_[A-Z0-9_]+` rule and provider credential formats
still apply). You can create multiple connections and reuse them from every Intent
whose project directory matches.

For Jira, use the site base URL (`https://team.atlassian.net`) and optionally a
default project key. For Azure, use the organization/project URL
(`https://dev.azure.com/org/project`) and optionally a default repository. The
registry accepts canonical HTTPS provider hosts only, without credentials, ports,
query strings or fragments. Azure project/repository names are validated as single
path segments and encoded when constructing URLs.

Choose the named connection and object type, then enter `TEAM-123` for Jira or a
positive numeric ID for an Azure work item or PR. Jira keys are uppercased; a bare
positive number uses the connection's default Jira project key, when configured.
An Azure PR needs a repository: configure a default, enter the explicit PR repository
field, or use `repo#123`. Conflicting explicit repository inputs are rejected. Review
the displayed canonical URL and **Attach source**. This imports the same source and
starts the same automatic synchronization as full-URL import. **Add a source by full
URL** remains available, including manual references.

Editing a connection affects future attachments. Each attached source retains the
resolved URL, credential variable reference, stable connection ID, connection version
and metadata at attachment time. Existing sources **do not switch tenant, project,
repository or credential variable** after edits. To use another boundary, attach a
new source and review its requirements. Disabling a connection prevents new
attachments; existing sources continue to synchronize with their original credential
reference. Removal is blocked while any source references the connection; disable
it instead. Sources show the original connection name/version, even after a rename.

The registry is stored in the existing Intent SQLite database (schema version 8),
scoped by the saved project directory, without another `config.yaml` writer.
Worktrees with different saved project directories have separate registries. Unused
removed connections retain an internal tombstone so an old ID cannot be reused.
Registry mutations use connection-version CAS and operation IDs; attachments use
Intent-revision and connection-version checks plus an idempotent source ID. A stale
connection requires reloading and reviewing the URL again. Successful attachment
retries use their original pinned metadata, including after edits or disablement.
Native, headless and hub-routed clients use the existing `intentWorkspaceRequest`
transport with `integrations`, `saveIntegration`, `removeIntegration`, `previewSource`
and `attachSource` actions. These operations never alter Intent lifecycle,
requirement revisions, evidence or automation decisions.

## Provenance and lifecycle compatibility

This branch started at **e8ebed9d**. Read-only inspection of the physical checkout
`/home/djtouchette/Work/worky/workspacer` found the same HEAD, with **no tracked
working-tree diff**. Its untracked `.agents`, `.claude/skills`, Rivet learnings,
remote-token, test-result directories and Python cache were not read as feature
source, copied, stashed, reset, cleaned or edited. No WIP bytes were reproduced.
Reactive intents were already committed in **a6b94932**, with storage-only fixture
corrections in **b952a08c/e8ebed9d**; this implementation inherits those commits.

The clarity goal of audit **9a4a3d29/ccb13cc5** is retained in the guide and existing
reactive status help/tests. Its old status-only revision assertions were not
cherry-picked: they contradict the inherited implementation. Status transitions
are history events; only requirement edits advance requirement revisions.
External observations do neither. Existing accepted snapshots, user verification,
reviews, historical launch packets and automation decisions remain independent.
A plain source-link field is still only a reference; import a linked source to sync.

## Supported read contracts

- ADO work items: ID, numeric revision, title/description and native fields,
  plus up to 50 comments including explicit deleted-comment markers.
- ADO PRs: ID/repository, source/target branches, active/completed/abandoned,
  draft and merge fields, reviewers/votes, up to 50 threads, commits, statuses and
  policy evaluations. Native fields are retained. Policy context retains build IDs
  where available; the normalized summary includes policy/status results.
- Jira Cloud issues: key/ID, updated revision, native status/resolution, assignee,
  priority and links; up to 50 newest comments and the last changelog window.
  Changelog reads use numeric startAt derived from total, not provider nextPage URLs.
  Attachment metadata is retained; body/thumbnail URLs are never fetched.

Each HTTP response is limited to 512 KiB and eight seconds. Combined artifacts are
limited to 2 MiB. Collection entries have explicit truncation/unavailable metadata;
continuation tokens are retained as provenance, not treated as authenticated URLs.
The first 50 records are retained for ADO collections; nested threads/comments
remain bounded by the response limit. This is a bounded observation window, not
an exhaustive historical mirror. Missing items outside a window are **not** inferred
to be deleted. Explicit provider deletion markers survive in native artifacts.

The adapter uses documented [ADO PR commits](https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-commits/get-pull-request-commits?view=azure-devops-rest-7.1),
[PR statuses](https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-statuses/list?view=azure-devops-rest-7.1),
[policy evaluations](https://learn.microsoft.com/en-us/rest/api/azure/devops/policy/evaluations/list?view=azure-devops-rest-7.1),
[Jira issue/changelog](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/)
and [Jira comments](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/)
contracts. Tests use synthetic fixtures only; no live account was contacted.

## Persistence, rate control and trust

Schema **v7** adds normalized external-object state, deduplicated immutable artifact
bytes, immutable observation events, and durable account leases. It leaves v1–v6
accepted source JSON and execution packets untouched. Events reference artifact
digests: recovery to previously observed content gets a new event without copying
artifact bytes. Repeated identical observations do not append duplicate events.
Artifact review pages one event at a time. History is retained without an automatic
pruning policy; per-record size is bounded, total historical database size can grow.

The owner runtime has an independent five-second scheduler tick. It processes at
most 32 due sources and four concurrent accounts, one active sync per Jira site or
ADO organization across imports/manual/background work sharing a local SQLite file.
A deterministic test verifies contention and cooldown between two OS processes
with separate connections on one machine. Cross-machine/shared-filesystem SQLite
locking and provider quota coordination are not verified or supported guarantees.
This conservative site/organization grouping also serializes different credential
references on the same service. The account lease expires after two minutes on a
crash. Successful accounts have a five-second cooldown; sources normally refresh
at five minutes. Failed syncs back off from 30 seconds exponentially (up to an hour
before 0.75–1.25 jitter), honoring Retry-After up to 24 hours. Manual refresh cannot
bypass an account lease or cooldown. Separate databases/process installations do
not coordinate their provider quotas.

ETags are sent only when returned by the provider. A 304 updates primary freshness
without rewriting accepted snapshots or artifact bytes. It cannot upgrade partial
coverage to fresh: partial persists until a full read confirms completeness, including
after reopening the database. Collection reconciliation
bypasses ETags every thirty minutes; partial/error/missing observations also force
an unconditional read. Secondary collections can therefore lag a successful 304;
the persisted reconciliation timestamp records that distinction. A collection
permission/error response marks partial coverage; 429 stops the cycle and backs
off the entire account. Failures retain the prior successful projection. 404/410
produce ambiguous deleted-or-inaccessible tombstones, never an intent transition.

Credentials remain environment-variable references. The HTTP boundary reads a
credential only at request time, rejects arbitrary hosts/paths and redirects,
bounds payloads, recursively removes echoed credential values/encoded credentials,
and redacts recognized secret fields. Errors persisted or logged by synchronization
contain fixed classifications, not response bodies. No token is sent to the UI,
SQLite, context packet or artifact. Existing deliberate issue comment publication
is unchanged; new PR/scheduler operations are GET-only.

Future agent packets add at most roughly 6 KiB of JSON-quoted external observations,
including provider revision, freshness, last collection reconciliation and artifact
digest, alongside bounded accepted source requirements. No external body is executed
or promoted into instructions. Unaccepted descriptions/comments never replace
accepted requirement content. The UI renders provider strings as React text.

## Deliberate deferrals

GitHub, webhooks, on-premises trackers, exhaustive collection pagination/backfill,
attachment downloads, dedicated build/log fetching, live agent message injection,
automatic requirement acceptance, provider writes from synchronization, and
cross-database quota coordination are deferred. Complete source history and omitted
collection records remain available in the provider UI. Polling observes linked
objects only; it does not discover new issues or PRs.

## Verification

Synthetic adapter contracts cover URL confinement/canonicalization, all three PR
states, native mapping, recent windows/cursors, conditional requests, deletion
markers, partial failures, payload limits and recursive redaction. SQLite tests
cover automatic refresh of both providers, deduplication/recovery events, immutable
history, durable backoff, account contention and v6 compatibility. An owner-runtime
test exercises real adapters against mocked HTTP without a viewer. Renderer tests
cover stale/rate-limited projections, artifact paging and safe quoted rendering.
The existing intent suites retain reactive lifecycle/revision and launch-packet
coverage. Final command results are recorded in the handoff.

### Local verification result (2026-09-13)

Environment: Linux x86_64, Node v26.2.0, npm 11.14.1; existing dependencies and
Chromium. Commands below ran from `apps/desktop` unless noted.

| Check                                                                                                                                                                                                      | Result                                                        |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------- |
| `npm run typecheck`                                                                                                                                                                                        | Main and renderer passed                                      |
| `npm run test:main -- src/main/services/intent src/main/headless/intent`                                                                                                                                   | 150 passed; 8 existing platform skips                         |
| `npm run test:main -- src/main/shared/intentSummary.test.ts src/main/ipc.test.ts src/main/ipcFederationRouting.test.ts src/main/services/claudeSessionStore.test.ts tests/main/claudeSessionStore.test.ts` | 174 passed                                                    |
| `npm run test:renderer -- tests/components/IntentSources.test.tsx tests/components/IntentWorkspaces.test.tsx`                                                                                              | 16 passed                                                     |
| `npm run build`                                                                                                                                                                                            | Main/preload, desktop renderer and web renderer passed        |
| `npm run build:desktop-host`                                                                                                                                                                               | Passed                                                        |
| `node_modules/.bin/playwright test --project=renderer intentCompletion.test.ts --workers=1`                                                                                                                | 2 passed: persistent owner workflow and responsive Work shell |
| Prettier check on all changed TypeScript/TSX files                                                                                                                                                         | Passed                                                        |
| `git diff --check` (repo root)                                                                                                                                                                             | Passed                                                        |

Rivet context/recon and `rivet witness select` guided the checks. The callable MCP
server was not exposed in this runtime; the installed Rivet/recon CLI supplied the
equivalent operations and `rivet learnings add` captured the finding. Builds emitted
existing large-chunk and tool deprecation warnings, with no build failures.

Runtime evidence comprises an owner-scheduler integration with real adapters and
mocked HTTP for both providers, plus Chromium against production UI and a real
private headless host using scratch storage. The browser flow uses manual sources;
provider-specific UI behavior is covered by renderer tests. No live account,
credential, Windows runtime, or production quota behavior was exercised. No
installation, push, merge, deployment, publishing, or nightly was performed.

### Pre-ship review corrections (2026-09-13)

Repair commit **5f9cfd41** preserves incomplete coverage on a primary-object 304;
only a full read can restore `fresh`. The regression closes and reopens an on-disk
SQLite database, checks the Sources response and context freshness label, preserves
the prior immutable artifact/reconciliation timestamp, and verifies recovery on a
full read. A renderer test checks that recent successful checks still display
`partial`. Packets omit the external-status header when all linked sources lack
external state. Accepted requirements and lifecycle/revision semantics are unchanged.

The account test now runs actual store code in separate OS processes opening the
same local SQLite file. The holder starts its competitor after acquiring the lease
and before releasing it; injected time verifies cooldown and subsequent acquisition
without sleeps. This is a local-process guarantee, not a shared-filesystem or
cross-machine guarantee.

Validation on Linux / Node v26.2.0 / npm 11.14.1 with existing dependencies:

- Focused main intent/headless suites: **153 passed, 8 existing skips**.
- Additional summary, IPC/federation and session-store suites: **174 passed**.
- Sources and Workspaces renderer suites: **17 passed**.
- Sync-store suite after tightening the process error assertion: **10 passed**.
- Main and renderer typechecks: **passed**.
- Main/preload, desktop renderer, web renderer and desktop-host builds: **passed**.
- Playwright `--project=renderer intentCompletion.test.ts --workers=1`: **2 passed**.
- Prettier on the four changed TypeScript/TSX files and `git diff --check`: **passed**.

Rivet MCP context/recon guided the repair. MCP `witness.select` returned an empty
body, so the local `rivet witness select` supplied the selection. Two Rivet learnings
record the coverage and lease guarantees. Existing large-chunk, color-environment
and deprecation warnings remain. Provider refresh remains mocked; the browser uses
manual sources with the real private headless host. No live providers, credentials,
physical-main changes, installs, push, merge, publishing or deployment occurred.
