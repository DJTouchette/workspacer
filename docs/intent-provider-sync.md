# Intent provider synchronization slice

Reviewer entry points: `intentSourceSync.ts` maps provider observations;
`intentSourceSyncStore.ts` owns durable projections, artifact/event provenance,
account leases and backoff; `intentAutomationRuntime.ts` runs it on both owning
hosts. `IntentSources.tsx` exposes freshness and paged artifact review through the
existing intent request boundary. No configuration setting was introduced.

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
ADO organization across imports/manual/background work sharing the database.
This conservative site/organization grouping also serializes different credential
references on the same service. The account lease expires after two minutes on a
crash. Successful accounts have a five-second cooldown; sources normally refresh
at five minutes. Failed syncs back off from 30 seconds exponentially (up to an hour
before 0.75–1.25 jitter), honoring Retry-After up to 24 hours. Manual refresh cannot
bypass an account lease or cooldown. Separate databases/process installations do
not coordinate their provider quotas.

ETags are sent only when returned by the provider. A 304 updates primary freshness
without rewriting accepted snapshots or artifact bytes. Collection reconciliation
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
