---
title: "Fleet Manager: the delegating manager agent, its wake loop, succession and briefs"
tags: [fleet-manager, wake, supervisor-nudge, worker, parent, briefs, mcp-facade, succession, wire-format, codex, header, block]
related_paths:
  - "apps/desktop/src/main/services/supervisorNudge.ts"
  - "apps/desktop/src/main/shared/fleetMessages.ts"
  - "apps/desktop/src/main/services/claudeSessionStore.ts"
  - "apps/desktop/src/main/services/managerSkills.ts"
  - "apps/desktop/src/main/lib/roleModels.ts"
  - "apps/desktop/src/main/lib/roleProviders.ts"
  - "apps/desktop/src/main/lib/workspacerHome.ts"
  - "apps/desktop/src/main/lib/managedSpawnOptions.ts"
  - "apps/desktop/src/main/services/briefService.ts"
  - "apps/desktop/src/main/services/thresholdWatch.ts"
  - "apps/desktop/src/main/services/progressReports.ts"
  - "apps/desktop/src/renderer/src/lib/fleetManager.ts"
  - "apps/desktop/src/renderer/src/hooks/useAgentManager.ts"
  - "apps/desktop/src/renderer/src/components/settings/SupervisorSection.tsx"
  - "services/hub/cmd/brain/handlers.go"
  - "services/hub/cmd/hub/mobile.html"
  - "apps/desktop/src/main/shared/managerDoctrine.ts"
  - "apps/desktop/src/main/services/managerReplacementService.ts"
  - "apps/desktop/src/main/services/managerReplacementState.ts"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Fleet Manager: launch, wakes, ownership, and memory

## Names and scope

Fleet Manager is the product role. `isWakeTarget` is the current session/journal
flag; the rename from `isSupervisor` has already happened. Files named
`supervisorNudge`, `SupervisorSection`, or `ensureSupervisorHome` still implement
active manager features. Do not delete them because of that legacy name.
The Go process-supervisor packages are a separate concern.

`supervisorNudge.ts` routes child finishes, blocks and catch-up messages. It also
supports ordinary agents receiving their own children’s wakes; being a parent
is not the same as being a fleet-wide manager. Role changes must preserve this
distinction and the persisted/wire spelling of `isWakeTarget`.

`main/shared/managerDoctrine.ts` is the shared manager instruction source;
`renderer/lib/fleetManager.ts` re-exports kickoff construction. Doctrine tells the
manager to delegate substantial work, obey explicit user/workflow choices, and
rely on host wakes after dispatch rather than polling. It is not additional
permission to merge, publish, or perform work outside the user’s authorization.

## Launch and requested configuration

Renderer manager launch reuses an appropriate live manager or creates/resumes
one through the host launchers. `manager: true` produces manager role metadata.
The native/bus launch bodies also resolve this role so entry points do not rely
on a renderer remembering to supply every setting.

- Provider resolution prefers an explicit supported provider, then
  `agents.managerProvider`, then the configured fallback logic in
  `main/lib/roleProviders.ts` (Claude when no recognized setting exists).
- `main/lib/roleModels.ts` resolves per-provider `agents.managerModels`,
  `managerEfforts`, and `managerContextWindows`. Preserve null-versus-absent
  context preferences and resume behavior; do not apply a different harness’s
  model string as a generic default.
- `deriveFleetRoot` uses an explicit fleet root, otherwise project/common-parent
  logic and the home fallback. `ensureSupervisorHome` names Workspacer’s home;
  it does not resurrect a separate supervisor role.
- The manager’s own model preferences are distinct from the routing matrix used
  for worker selection. Report requested config separately from live telemetry.

Supported launches use the ordinary ambient facade tool surface; manager metadata
controls role/wake behavior, not a wider per-session tool grant. Native permission
modes are provider configuration. Pi is rejected by normal Workspacer spawning.
The Windows Codex PTY hybrid has its own facade-ready/token-injection path and
must not be described as an intentionally tool-less manager.

Managers receive standup/checkpoint/handoff (plus response cards) as the
`workspacer-fleet` plugin for their session only: Claude via `--plugin-dir`,
Codex via its app-server's `skills/extraRoots/set`, both from the bundle in
`~/.workspacer/agent-skills/<version>/` (see the agent-spawn domain doc).
`installManagerSkills` now serves only harnesses with no per-session channel
(Copilot's `~/.copilot/skills`) and refuses Claude/Codex, whose personal copies
would leak into every later session. Strict mode (manager replacement) throws
instead of degrading, on both paths. Ordinary agents get the separate
`workspacer` plugin, so neither role discovers the other's skills.

## Finish wakes

The desktop watches working→idle transitions through `nudgeParentOnFinish`.
A child with a live local parent can report to that parent even when the parent
is an ordinary agent. The manager-replacement journal can retain completion
information independently of immediate delivery.

`onFinished` ignores boot-idle sessions with no received user task and coalesces
by parent over 1.5 seconds. Delivery rechecks the live worker: a worker that
resumed working or produced a different reply during asynchronous evidence
capture must not report an obsolete finish. It derives stopped/failure state,
validates structured result or terminal escalation, captures review evidence,
and records the result in dispatch history.

Completion and escalation batches are separate. A valid escalation is not a
successful result, and malformed/missing result data does not waive the requested
contract. Deduplication includes the meaningful finish signature; unchanged
repeated idle observations do not wake the parent again.

Delivery follows current worker ownership through the replacement journal.
A message may be held durably during handoff or sent through
`claudemonSessionClient.message`. Explicit refusal/throw does not book a delivered
signature. A successful message acknowledgement proves acceptance for delivery,
not that the parent completed the requested follow-up. Ordinary parents get
ordinary-parent wording; managers also receive workflow guidance.

The desktop’s two-minute backstop scans idle live parents with children whose
unreported finish is older than three minutes and newer than the parent’s last
activity. This now includes ordinary parents, not only `isWakeTarget` rows.
It remains a recovery mechanism, not a guarantee that a missing/ended parent can
receive a message.

## Blocks, progress, and threshold watches

Blocks debounce for 20 seconds; clearing the block cancels the pending timer.
Recipients include live managers and the child’s live direct parent, excluding
self. Coalescing and replacement-aware routing avoid duplicate or misaddressed
wakes, and parked successors are not prematurely awakened.

`report_progress` derives the caller from its session credential and the recipient
from current parentage; the caller cannot nominate an arbitrary manager. The
desktop implementation flattens a note to one line, limits it to 500 string code
units, requires at least one minute between reports, and caps a worker at 20.
Refusals are explicit. Check the Go twin when changing these bounds or recipient
rules; semantic progress is not a substitute for host-observed completion.

`notify_when` is a separate one-shot, in-memory watch service. Cumulative token,
cost and idle predicates are distinct from `contextUsedPct`. The latter requires
fresh, consistent runtime context-health evidence, compatible provider identity
and a valid decimal epoch; requested/catalog capacity does not authorize an
automatic context wake. The two-minute health-age gate and future-time checks
are tested across the desktop and headless implementations.

Headless predicate accessors prefer the current raw snake-case status block where
high-frequency updates may precede the compatibility overlay. Keep raw and camel
projection types aligned and test raw-only updates. A predicate added only to the
desktop service silently leaves headless managers with different behavior.

## Wake text is a stored wire format

`main/shared/fleetMessages.ts` builds and parses worker-finished,
worker-escalated, catch-up, blocked, threshold, and progress messages. The legacy
`[supervisor]` blocked header is still parsed from stored transcripts. The desktop
and web renderer share the parser; mobile HTML has a port that must stay aligned.

Keep the builder, parser, mobile handling and round-trip fixtures together.
All-failed and still-running wording has distinct meaning. Structured extras are
joined by session ID; arbitrary full-reply prose must not be scanned as a forged
result block. A size-capped serialized result may not be valid JSON, so renderers
must preserve parse/missing-result caveats.

## Manager lineage and restart recovery

The private `manager-replacements.json` journal is now durable authority for
eligible replacement lineage and delivery state. `ManagerReplacementService`
coordinates preparation, successor creation, worker/task transfer, binding and
activation through bounded host operations. Desktop and the headless Node
companion share that service with different host adapters.

A host-owned handoff takes precedence over legacy standalone handoff files:
ownership has already been transferred by the host protocol, so the successor
must not repeat adoption, terminate/reopen the predecessor, or consume/delete an
unrelated shared handoff file. The facade’s ordinary `spawn_agent` does not let a
manager create its own replacement role.

Standalone recovery still uses the intended predecessor from a handoff file or
confirmed orphan evidence. `agents.orphans` reports candidates; it does not choose
one. A bare dangling parent ID is not proof that the parent was the intended
manager. Live tombstones are bounded projections, retained while associated
children exist, not a substitute for the durable journal.

`reparentChildren` handles live rows and pending spawn metadata, refuses invalid
successors/self-parenting, and excludes federated rows. Task ownership is persisted
before mutating in-memory parentage; pending wakes follow the new owner. Do not
reduce this to changing a `parentSessionId` string or claim no lineage is recorded.
A timeout/interrupted acknowledgement leaves an uncertain outcome that recovery
must reconcile; it is not permission to repeat a mutation blindly.

## Briefs and project memory

Project `.workspacer/brief.md` files record Now, Direction and Recently; the
manager’s fleet brief also carries user preferences and cross-project state.
The shared doctrine directs first-turn reading of its own fleet brief, then
project context relevant to the request, not an unconditional scan/write of every
project. A brief is durable task memory, not a runtime liveness database.

`brief.append` is an inspect-then-edit addition under an advisory lock with
outside-write checks; overlong lines are refused rather than truncated.
`brief.check` is a read-only reconciliation report. `brief.archive` moves old
Recently entries into the archive rather than silently deleting their history.
When result/session fields are supplied, preserve the host’s factual contribution
and keep the manager’s own text to significance rather than duplicating evidence.

A user-edited Workspacer home README is preserved; automatic migration is limited
to the recognized legacy/empty form. Manager skills, briefs, runtime metadata and
replacement journals have different lifecycles—do not treat any one as the whole
manager’s memory.

## Verification

Exercise ordinary-parent and manager wake paths, failed/held/accepted deliveries,
coalescing, restart ownership, structured result/escalation round trips, and both
threshold implementations. Manager launch changes require provider/model/context
preference tests and facade injection tests, including Windows hybrid argv.
Replacement changes require the shared service tests and real companion protocol
suite. See [agent spawning](../domains/agent-spawn.md),
[session lifecycle](../domains/session-lifecycle.md),
[facade](mcp-tool-facade.md), and [headless services](headless-desktop-services.md).
