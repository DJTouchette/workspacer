# Intent workspaces

Status: the planned opt-in workflow is implemented across all five milestones. The Windows and UI refinement pass is in release verification; the preview remains default-off. Product direction agreed
with Damien on 2026-09-12. This document is the delivery brief for the whole feature.

Use [the workflow and setup guide](intent-workspaces-guide.md) for the current UI,
tracker credentials, and recovery procedures. The implementation history below
records how the feature grew; the completion section describes the final scope.

## Intent

Make Workspacer a place to express, refine, steer, and realize an outcome in an
existing project. A developer should be able to return to a feature, understand
what changed and what needs their judgment, and continue without reconstructing
agent conversations. Adoption must work for one developer's features while their
team continues using Azure DevOps, Jira, Git, pull requests, and existing reviews.

## Model and ownership

- A project provides repositories, conventions, instructions, integrations, and
  reusable knowledge. Intent-owned project and repository UUIDs now coexist with
  the existing root-keyed fleet configuration, with explicit repository relocation.
- A feature workspace is one coherent intent: feature, fix, investigation, or
  maintenance task. It owns its intent revisions, decisions, sources, artifacts,
  evidence, and links to execution. It survives agent replacement and app restart.
- An intent describes the desired outcome, constraints, success criteria, and open
  questions. Source requirements and agent assumptions must remain distinguishable.
- AgentWorkspace currently means an agent's pane layout. Preserve that type and
  lifecycle. Introduce IntentWorkspace separately; execution links them later.
- ADO/Jira own shared requirements and team status. Workspacer owns personal
  exploration and interpretation. Publishing changes is a deliberate action with
  a reviewable payload; personal notes never silently override shared decisions.
- Rivet owns curated project knowledge. Workspacer records source references and
  versions used. Findings can be proposed for Rivet capture and later curation;
  feature-specific choices do not automatically become project-wide rules.

## Experience

Projects contain Work. Opening a work item gives an outcome summary, central work
surface, contextual discussion, and the decisions that need attention. The center
can show exploration, a working preview, implementation changes, or evidence.
Preserve the user's position when switching. Existing agents, panes, terminals,
approvals, and settings remain reachable throughout the rollout.

Start from an idea, ticket link, file, screenshot, or demonstration. Show the
interpreted intent and assumptions. Support annotations, direct edits, comparisons,
and conversation as ways to refine it. In previews, using the application and
annotating it are explicit interaction modes.

Steering targets the outcome, approach, or a particular result/task. Persist each
direction with attribution, target, revision, and supersession. Distinguish a
hypothetical exploration from a committed change. Show affected work and actual
worker delivery receipts; a saved instruction is not an acknowledged instruction.
Pause, retry, and undo must describe what has already happened. Undo of intent
does not silently reverse code or external actions.

## Rollout boundary

`ui.intentWorkspaces` is a separate, default-false setting. Existing `ui.mode`
fleet/focus continues to control agent attention. Enabling the experiment exposes
Work navigation; disabling it preserves records and existing pane/session identity.
The first surface layers over the mounted panes, using existing theme tokens and
components. Later milestones can reorganize navigation behind the same setting.

## Persistence and service boundary

Use a dedicated, versioned `intent-workspaces.sqlite` in the owning host's config
directory. Native desktop and headless desktop-host use the same service and
schema, so remote browser state belongs to the connected host. Open lazily.
Current records and full revision snapshots commit in one transaction. Writes use
an expected revision and reject stale edits rather than overwrite another client.
Never turn unreadable storage into an empty successful response.

Large artifacts stay in files and code stays in Git. The database stores durable
identity, relationships, provenance, versions, and relevant snapshots. Add separate
source, execution, evidence, and steering tables as those contracts are implemented.
Keep feature data out of config.yaml and browser localStorage.

## Delivery milestones

### 1. Durable work foundation (implemented)

- Default-off setting in Settings > Layout and Work entry in the tab bar.
- Create a workspace under a configured project or an explicitly entered root.
- Edit title, desired outcome, constraints, success criteria, and one source link.
- Explicit draft/active/review/complete status, independent of external ticket status.
- SQLite persistence, revision history with optional reasons, stale-write refusal,
  visible loading/save failures, and retention when the mode is disabled.
- Keep drafts intact when toggling between Work and agent panes during the session.

Acceptance: create and revise a feature, restart the storage service, and recover
the same record/history. Concurrent stale edits fail without changing history.
Enabling/disabling the experiment does not stop or recreate agent sessions.

### 2. Execution attached to intent (implemented)

- Explicitly attach existing sessions and start new work from an intent revision.
- Durable run/branch/PR links, independent of pane layouts and manager succession.
- Context packet includes project instructions and provenance, accepted intent,
  decisions, constraints, and evidence requested. Preserve provider routing.
- Returning to a feature shows results, blockers, and next decisions.

### 3. Live steering and review (implemented with explicit service/user evidence semantics)

- Targeted direction, affected-work preview, supersession, delivery receipts,
  pause/resume, and reconciliation after completion or failure.
- Route attention to feature workspaces while retaining existing approval paths.
- Evidence per success criterion; distinguish verified, reported, and unresolved.
- Working previews, annotations, alternatives, and deliberate selection.

### 4. Loose integrations (implemented: manual sources, Jira Cloud, ADO, Rivet)

- Provider-neutral source adapters: ADO and Jira are first priorities because they
  are the user's current work systems, not an exhaustive provider list. Support
  additional trackers and source systems without changing the workspace model.
  Each adapter declares import, change detection, and publishing capabilities.
  Preserve provider IDs, native IDs, URLs, and source-specific fields; do not
  assume all trackers share statuses, hierarchy, or approval semantics.
- Manually linked context first, then adapter-backed reads and change detection,
  followed by reviewed publishing actions. A workspace may link multiple sources
  across providers. No connector is required to create an intent.
- Record source revision/fetch time, show drift, preserve accepted snapshots.
- Rivet context retrieval and source visibility, reusable finding capture and
  deliberate promotion. Remain usable without Rivet.
- Existing-team compatibility: ordinary tickets, PRs, CI, and review workflows.

### 5. Expanded project experience (implemented: stable projects, artifacts, screenshot demonstrations)

- Stable project identity across root moves and multiple repositories.
- Project-first navigation, richer artifacts, recorded demonstrations, and bounded
  alternative exploration. Desired-condition automation follows proven delivery
  and reconciliation semantics rather than being part of the initial launch.

## Validation and release

Test storage reopen, transactional revision writes, malformed requests, conflicts,
project scoping, and both renderer transport paths. Test creation/editing/error
retention in the UI and default-off behavior. Run main/renderer type checks,
generated service/config parity checks, and relevant host authorization tests.
Document actual completed milestones and remaining limitations; do not enable the
setting in the user's live configuration or deploy a build as part of this change.

## Implemented on 2026-09-12

Milestone 1 is available under Settings > Layout > Intent workspaces (preview).
Enable it and open Work in the tab bar. The first surface supports project grouping,
intent creation/editing, a generic source URL, explicit status, revision reasons,
and expandable snapshot history. The source URL is a reference, with no fetch or
publishing side effects. Agent execution is explicitly outside this first slice.

Storage lives in `services/intentWorkspaceStore.ts` and its shared wire types in
`main/shared/intentWorkspace.ts`. `desktop.intentWorkspaceRequest` is registered
through the generated owner-service manifest; native IPC and the headless host
call the same service. It uses the runtime's built-in SQLite, also verified in the
installed Electron runtime, and has no renderer database or extra native addon.

Validation performed:

- Real SQLite: reopen/history recovery, two-connection stale-edit refusal,
  transactional rollback, malformed inputs, project isolation, newer-schema refusal.
- Renderer: creation, draft preservation on navigation/save failure, unavailable
  host reporting, setting defaults/errors, selected-host RPC routing.
- Production App in Chromium: default-off entry, setting search/toggle,
  creation/revision, draft retention while switching surfaces, saved-state retention
  across toggles, agent pane identity, no agent respawn, and approvals remaining
  visible while Work covers the active agent; desktop and phone widths.
- Headless production private protocol: create/update/list/history and conflicts.
- Main/renderer type checks, headless bundle build, config/service generated parity,
  and Go desktop-service authorization checks.

## Execution slice — 2026-09-12

The execution slice introduced Intent, Execution, and History views; the later
steering slice adds Direction. Execution offers:

- Start agent: opens the existing provider/model/profile/permission/worktree dialog.
  The editable task is combined with the saved intent by the host. The exact packet
  and intent revision are persisted before spawning, then sent as the existing
  atomic first-message parameter. Actual execution cwd comes back from the spawn
  callback after worktree allocation. Provider and peer routing remain unchanged.
- Link agent: associates an existing session, identified by session ID plus hub ID.
  This is explicitly tracking-only; no message is sent. Normal transcript resume
  preserves that session ID; a different successor can be linked explicitly.
- Recorded attempts: launch IDs are idempotent claims. Repeating a claim returns
  the existing attempt and never dispatches again. A lost spawn response or a failed
  link write stays visibly unconfirmed, with a Link to this attempt recovery action.
  There is no automatic replay or speculative session matching after a client crash.
- Current status and agent reports, with retained last observations when sessions
  disappear. Offline peer tombstones never refresh the observation timestamp.
  Reports are agent-reported text, not independently verified completion evidence.
- Manually recorded branch references and pull-request URLs, independent of the
  source tracker. These references do not create branches, fetch PRs, or publish.

SQLite schema v2 introduced execution and reference tables. Execution rows reference an
existing immutable intent revision. The same owner-only API handles both desktop
and headless hosts; snapshots for retained observations come from the host's session
store, never fields supplied in the request body.

Current limits are deliberate and visible:

- Observations now persist independently of the Execution view, as detailed below.
  They are bounded report excerpts, not a transcript archive or independently
  verified completion evidence. The UI calls missing sessions “Not currently observed.”
- A stored first-message packet proves what was requested. It is not a worker
  acknowledgment of understanding or a receipt for subsequent steering.
- Project instruction provenance currently means the saved project root and an
  instruction to read applicable repository/Rivet guidance. Full versioned captures
  of the documents actually consulted remain part of the knowledge integration.
- Intent edits preserve prior launch packets and mark those runs as earlier intent.
  They never silently change an agent's instructions. The Direction view sends
  separately reviewed messages tied to saved revisions.

Additional validation: real v1→v2 migration, cross-connection idempotent claims,
revision pinning, qualified session identity, recovery transitions, offline result
retention, and reference validation. Renderer tests cover no-send associations,
uncertain launches, post-spawn write failure, real worktree cwd, and draft/error
retention. Production App browser tests cover Codex dispatch, exact first-message
payload, later intent edits, a PR reference, and tracking an existing agent without
additional messages; the private headless protocol exercises the new actions too.

The remaining implementation work described at the end of this slice was completed
in the integration pass below. No team-wide adoption is required.

## Background observations — 2026-09-12

Already-linked executions now retain results while Work is closed. Native session
updates feed the shared capture operation before window checks or IPC coalescing;
explicit close captures the row before removal. Late assistant text arriving after
Stop or SessionEnd is captured too. The operation never assigns unknown launches
to sessions and always matches the hub-qualified identity.

Headless `internal.observe` feeds the same store on the brain's existing two-second
observation cycle. Brain snapshots omit conversations, so linked local sessions
also read the daemon's bounded `summary_source=1` projection without browser
transcript demand. Requests have a three-second timeout and at most four run
concurrently. Peer identities are never looked up against the local daemon.

Capture lazily opens an existing database after restart. Ordinary sessions do not
create an unused database; an absent database is checked at most every five seconds.
The linked-execution index refreshes after local changes or another SQLite
connection's writes. Duplicate observations do not write or refresh their timestamp.
Busy report changes checkpoint at most every five seconds; state/cwd changes and
changed idle/stopped reports commit immediately. Offline tombstones are ignored.
Sparse snapshots do not erase a retained report. No schema migration is needed.

Write failures retain the pending sample in process memory for retry on subsequent
capture or Execution reads. Capture warnings appear alongside retained reports;
report-read failures clear only after a successful read for that session. Native
and headless observation failures are isolated from the existing session lifecycle.
The Execution view shows saved report text even when the live headless row is sparse.

Remaining capture limits:

- The host must be running and receive the session update. A session removed between
  headless observation cycles can still be missed; slow report reads can extend that
  interval. This is periodic capture, not an event archive.
- Native text is capped at 4,000 characters. The headless daemon projection caps each
  event at 800 characters and its complete response at 5,000 bytes. Captured text is
  an excerpt of the latest available assistant message, which may precede final text.
- Peer report capture depends on text received from that peer. A sparse peer row
  alone supplies status, not a report; offline rows never count as fresh observations.
- Failed pending writes and capture warnings are process-local. A crash during a
  storage failure can lose an uncommitted sample; previously committed data survives.

Validation: 174 targeted main-process tests, 15 renderer tests, two production App
browser flows, main/renderer type checks, and the production private-protocol suite.
The protocol regression links
a session, restarts the host, captures its final report without any workspace read,
removes the session, restarts again, and verifies the retained result. Additional
tests cover native late text/explicit close without a window, streaming write
counts, duplicate/offline/wrong-hub filtering, cross-connection index refresh,
failed-write retry after disappearance, daemon failures/old responses/size bounds,
and visible errors and retained reports in the renderer.

## Targeted direction and delivery — 2026-09-12

Work > Direction now supports recording a direction for one linked execution,
reviewing the exact saved message, and explicitly sending it. Direction drafts
survive view/workspace navigation in memory. The form collapses after saving so
the recorded message and delivery controls are the focus.

The host pins the execution's hub-qualified session identity, saved intent revision,
user attribution, text, and complete message packet. The packet includes the saved
outcome, constraints, and success criteria plus the requested direction. It is
immutable after recording. Changing the intent does not send anything; an unsent
direction pinned to an earlier revision must be replaced before sending.

Saving and sending are separate operations. The host commits an uncertain attempt
before message I/O and records its receipt afterwards. Concurrent or repeated send
requests cannot replay an accepted or uncertain attempt, including after restart.
Confirmed failures permit an explicit new attempt (up to eight attempts per
direction), preserving earlier receipts. A failed claim write sends nothing; a
failed final receipt write leaves the durable attempt uncertain and reports the
storage failure. Clients cannot supply receipt status or substitute the saved target
or message through the public request API.

Native IPC and browser requests hosted by Electron share the existing native
message client. Headless requests call the Go brain's existing daemon delivery
primitive through a fixed private callback. Peers use a qualified `agents.sendMessage`
call with the recorded hub/session identity and no local fallback. Missing, ended,
or offline targets fail before sending; local manager handoff fences are respected.
Only the existing authenticated desktop owner API exposes these operations.

The three receipt states describe the messaging boundary:

- **Accepted by messaging service:** the local daemon or peer service acknowledged
  acceptance. Its normal queueing rules still apply. This is not an agent receipt
  proving consumption, understanding, application, or completion.
- **Delivery failed:** the host refused before I/O or the daemon explicitly rejected
  the message. The UI permits an explicit retry after the problem is resolved.
- **Delivery uncertain:** the request is in flight, an acknowledgment was lost, the
  host restarted mid-attempt, or its receipt could not be saved. Inspect the agent
  conversation; this state has no automatic or one-click resend. Peer exceptions
  are conservatively uncertain because that API does not reliably expose whether
  refusal happened before delivery.

Replace direction creates a new immutable record for the same execution and links
both sides of the replacement. Concurrent replacements of the same predecessor
conflict. Prior text, packet, and delivery receipts remain visible, including when a
receipt arrives after its replacement was saved. Recording a replacement blocks
new sends of its predecessor; it does not retract queued messages or undo actions.
The replacement itself must be reviewed and explicitly sent.

SQLite schema v3 adds direction records with foreign keys to executions and intent
revisions; v1/v2 data migrates in place. Implementation lives in
`services/intentSteeringStore.ts`, `services/intentDirectionDelivery.ts`,
`brain/intentsteering.go`, and the renderer's `IntentSteering.tsx`.

This slice does not add agent-authored acknowledgment, live-turn interruption,
pause/resume, cancellation of queued messages, verified application of direction,
or user reconciliation of ambiguous receipts. It records which session endpoint
was addressed and what that messaging service confirmed; a peer's internal
handoff/queueing behavior remains owned by that peer.

Validation covers real SQLite migration/reopen, concurrent two-connection claims,
idempotency, immutable packets, host-owned targets/receipts, stale revisions,
cross-workspace refusals, supersession during delivery, failed claims and receipt
writes, and qualified peer routing. Native IPC is exercised through its registered
handler. A Go/Node/private-protocol integration sends the exact saved packet through
the production daemon transport, tests explicit refusal and retry, restarts the host,
and verifies that uncertain attempts cannot replay. Renderer tests and production
App browser flows cover review-before-send, retained drafts, accepted/uncertain
receipts, replacement history, and desktop/mobile layout. Type checks and the
existing background-capture private-protocol tests also pass.

## Completed integration — 2026-09-13

The Work surface now offers Overview, Intent, Execution, Direction, Review,
Sources, Knowledge, Artifacts, and History. Projects management lives in the
sidebar. Existing agent panes remain mounted, and approval/question actions return
to the original agent flow. Linked work items display qualified live attention;
the overview separates reported progress, user review, unresolved evidence, source
drift, and uncertain deliveries without inferring feature completion from idle.

Review covers each nonblank saved success-criterion line. Evidence records pin the
criterion text and revision, distinguish reported/unresolved/user-verified claims,
and can retain a host-captured Git snapshot from a linked local execution. Git
capture isolates Git configuration and disables executable filters, external diffs,
text conversion, hooks, and submodule traversal; it retains bounded staged and
unstaged tracked changes with explicit omissions. A review acceptance requires
selected user-verified evidence for every current criterion and no unresolved
selection. Review history remains separate from workspace status and team reviews.

Interrupt/Continue controls use the existing signal/message services, with durable
pre-I/O claims and receipts. Interrupt requests SIGINT, not a rollback or a promise
that all background work stopped. Continue sends a reviewed message to the same
session and never respawns an ended agent. User reconciliation is an append-only
assessment with a reason; it does not rewrite service receipts or enable replay of
uncertain sends.

Sources retain provider/native IDs and fields, immutable accepted snapshots, new
drift candidates, and explicit acceptance history. Manual sources always work.
Jira Cloud and ADO adapters implement bounded reads and reviewed comment publishing
with host-only named environment credentials, fixed provider routes, refusal of
redirects, secret redaction, preflight revision checks, and durable receipts. Provider
capabilities are explicit; adding adapters does not require changing workspace data.
Comments never change ticket status or replace personal intent automatically.

Knowledge captures Rivet document content and hashes. Feature findings remain
separate until a reviewed proposal creates a learning file or appends to curated
context. Writes check the reviewed baseline and pin the target directory/file;
uncertain writes can be reconciled against the exact proposed content by host hash.
The written receipt is historical and does not claim the file remained unchanged.
Absence of Rivet does not block ordinary work or recording findings.

Artifacts retain immutable file bytes or URL references, versions, SHA-256 identity,
and optional criterion/execution associations. Image/text/static HTML previews,
explicit View/Annotate modes, version-pinned coordinates/text notes, ordered
screenshot demonstrations, and bounded alternative groups are implemented. Selecting
an alternative records a decision with a reason; it does not launch agents, enforce
a time budget, merge code, or publish anything. Live URLs open through the existing
browser pane; captured HTML remains sandboxed and cannot run scripts or fetch data.

Schema v5 migrates legacy work without changing old revision/launch bytes. Stable
project/repository identities support multiple roots, renames, and explicit root
relocation. Relocation advances affected current intent revisions atomically and
retains historical roots and execution cwd. It neither moves repository files nor
rewrites config.yaml. Future launch/direction/continuation packets include bounded
accepted source snapshots, captured knowledge versions, current evidence excerpts,
and selected alternatives. Old packets remain immutable and unaccepted drift is
excluded.

Validation includes combined main/renderer suites, main and renderer type checks,
generated service/config checks, owner authorization tests, private Node/Go host
protocol and restart tests, and production App browser flows on desktop/mobile.
The new end-to-end test uses a real temporary Git repository, SQLite host, retained
artifacts, and Rivet files; provider sessions and interrupt acknowledgments are
fixtures. Independent agents reviewed concurrency, provenance, secret redaction,
directory swaps, Git configuration execution, and UI receipt claims; discovered
issues received regression tests and fixes.

Release limits remain explicit:

- Secure retained-file operations and Rivet file reads/writes support Linux and
  Windows. Other owning-host platforms fail closed for those operations.
- External Jira/ADO reads and publishing were verified with API-shaped test servers,
  not the user's credentials or live trackers. Their comment APIs provide no atomic
  issue-revision condition, so an issue can change after preflight and before posting.
- Headless result capture remains periodic with bounded excerpts. It is not a full
  transcript archive or independently verified worker acknowledgment.
- Artifacts are bounded to 512 KiB each and 128 records per workspace; demonstrations
  are ordered screenshots, not video. Alternative budgets are declared, not timers.
- Service acceptance, user observation, and verified criterion coverage are separate
  records. Workspace completion remains an explicit user choice.

The initial integration pass did not deploy the feature, enable live settings,
publish live tracker comments, or perform in-app knowledge promotions. Required
developer findings were recorded in this repository's Rivet learning log.

## Windows and interface refinement — 2026-09-13

Windows file operations use native handle-relative opens and reparse-point checks
through a fixed, bundled PowerShell/.NET helper. Directory leases prevent parent
replacement while an operation is in flight. Reads are bounded, new writes refuse
existing files, and replacement/removal checks the reviewed content digest. Windows
runtime tests cover Unicode paths, reserved/device/alternate-stream paths, junctions,
parent sharing, all retained-file stores, knowledge reconciliation, and Git for Windows.
Worker isolation and database-lock responsiveness are undergoing release
verification before publication. Client and hub budgets accommodate the bounded
file-operation budget without widening unrelated operation deadlines.

The interface now opens existing work on Overview, with a clear outcome, criterion
coverage, execution count, and next decisions. New intents remain in the editor.
Compact icon tabs scroll in one row and support Arrow keys, Home/End, roving focus,
and an associated tab panel. Mobile work lists collapse after selection; the work
list can be searched without losing the selected draft. Status, success feedback,
metadata disclosures, and forms share the project design tokens in both themes.

The interaction audit tightened duplicate-click and stale-response guards, retained
artifact/assessment drafts, pinned review and selection bases, safe retry identities,
and error/receipt recovery. Saved-image annotations support keyboard placement.
Local and CI validation includes the complete persisted workflow plus 320/768/1280px
dark/light layout, tab keyboard behavior, and original approval routing.

Release verification is performed on a candidate branch before advancing master and
publishing the rolling nightly. The default-off feature setting remains unchanged.

## References

- [Anthropic AI-native SDLC introduction](https://academy.claude.com/courses/ai-native-sdlc-playbook/introduction)
- [Capture intent](https://academy.claude.com/courses/ai-native-sdlc-playbook/capture-intent)
- [Rivet](https://github.com/DJTouchette/rivet)
- Local context: ui-modes-manifest, config, renderer-backend-seam, pane-system,
  mission-control-attention, and fleet-manager.
