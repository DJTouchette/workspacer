---
title: Claudemon Multi-Provider Adapters & PTY/Stream Transports
tags: [claudemon, rust, providers, transports, control-protocol, codex, opencode, subagent, copilot, rollout, thread]
related_paths:
  - "services/claudemon/src/providers/*.rs"
  - "services/claudemon/src/session/state.rs"
  - "services/claudemon/src/session/store.rs"
  - "services/claudemon/src/session/pricing.rs"
  - "services/claudemon/src/daemon/spawn.rs"
  - "services/claudemon/src/execution.rs"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Claudemon provider adapters and transports

## Admission versus adapter availability

Desktop and brain Workspacer launchers support Claude, Codex, OpenCode, and
Copilot. They reject Pi because it cannot receive the required Workspacer MCP
surface. Claudemon still contains Pi implementations and accepts Pi on its
low-level managed route. A compiled adapter or a historical captured session is
not proof that the current product launcher admits that provider.

`services/claudemon/src/execution.rs` defines the internal execution-engine
contract. Production installs the compiled claudemon implementation, with no
caller-selectable engine field on the spawn wire. Admission checks the selected
executable and persisted `EngineLease` identity before publication. Incompatible
or unavailable pins fail rather than falling back to another engine. Identity is
a compatibility lineage, not the binary build hash. Leases persist separately
from session rows that may not yet have a first hook event.

The native start/control paths keep their own delivery semantics. The engine
registry does not add universal request deduplication or prove that a cancelled
wait means the provider process has exited. Keep generation/ownership fences
when publishing state and cleaning up failed or superseded starts.

## Implementation map

| Adapter | Machine interface and session shape |
| --- | --- |
| `claude_stream.rs` | Headless stream-json with bidirectional control messages |
| `codex.rs` | Local WebSocket app-server; headless thread or TUI/RPC hybrid |
| `codex_rollout.rs` | Observational rollout tail/replay and Workspacer-ID → native-thread sidecar |
| `opencode.rs` | Local HTTP server and server-wide SSE, with attached TUI |
| `copilot.rs` | One noninteractive JSONL process per turn, using a stable native session ID |
| `pi.rs` | Legacy TUI and stdio-RPC paths with different controls |

These live under `services/claudemon/src/providers`. Pure translators convert
provider messages to `AgentUpdate` values; the shared `apply_updates` layer folds
usage, conversation, plan and mode into the store. Drivers also own native
process/channel registration and lifecycle handling, so do not assume every
store write happens only inside the pure translator or shared apply function.

`daemon/spawn.rs` normalizes selection and chooses the native path through engine
admission. It stamps transport before relevant events can arrive. The serde
`Transport` default remains Pty for legacy snapshots; product-level default-to-
stream policy is in config/launch construction, not that enum default.

## Model catalogs and registered controls

Live model queries are cached by provider/binary key for ten minutes, with
coalescing for concurrent misses and stale-last-good fallback after a failed
refresh. A missing previous result remains an error. This cache key is not a
per-account authorization guarantee; a native CLI can still refuse a listed or
manually entered model at launch.

The actual registered store channels determine control availability:

- Claude stream has structured answer and permission-mode channels, plus model
  and interrupt controls.
- Codex, OpenCode, Copilot and Pi RPC use their supported model/interrupt paths;
  yes/no decision channels are not equivalent to Claude’s structured question
  protocol.
- Pi TUI retains input/decision plumbing but lacks the RPC path’s model and
  interrupt controls. Normal Workspacer Pi launch remains refused.
- The rollout reader registers no interactive control channels. Any terminal
  interaction belongs to the surrounding PTY path, not the tailer.

Missing channels fail closed or trigger an explicitly implemented fallback at
the caller. Match renderer `providerCaps.ts`, daemon registration and response
behavior; provider name alone is insufficient to infer a control capability.

## Claude stream

The adapter uses the native stream-json control protocol for permission requests,
questions, interrupt, permission-mode and model switching. Hooks can still supply
enrichment, but cannot take over a managed session’s mode/pending slot.

`background_tasks_changed` carries more than one kind of task. Agent work such
as local agents, teammates, and remote agents is distinguished from ambient
background work; a background shell must not keep an otherwise idle parent
Responding forever. The parent’s dispatch result does not end work while a
busy-holding agent task remains. Keep task classification, result handling and
background counts together.

Inventory enrichment is disk-backed after the init frame; translation remains
pure. See [Claude asset roots](claude-asset-roots.md) for path/origin resolution.

## Native child cards and Claude artifact replay

The native client joins provider child IDs to dispatch tool IDs separately from
Workspacer `spawn_agent` session receipts. Both use compact child cards, with
different source icons. Reported cumulative usage and runtime model metadata stay
separate from requested model/context selection. Missing measurements and the
Claude artifact reader's unknown-start sentinel remain unknown.

`session/claude_subagents.rs` discovers only the exact Claude session transcript
under spawn-authorized roots and reads known children within that parent's
`subagents` directory. This supplies standalone native metadata without relying
on Electron's hook setup. Hooks enrich child rows without claiming the managed
parent's mode, pending decisions or background-task counts. Async artifact scans
are fenced against newer hooks and changed session generations. Parent Input is
not completion evidence for a detached child.

The parent-scoped `sessions.subagentConversation` route now serves Claude and
Codex. Claude replay rejects redirected paths and files over 4 MiB, retains
normalized tool/result blocks and preserves sequence offsets when its 2,000-event
window clips older events. Native previews are bounded, read-only snapshots with
explicit refresh; they do not create independent Workspacer sessions for native
provider children.

## Codex ownership, controls, and resume

Hybrid mode lets the native TUI own the thread; the RPC client discovers and
resumes that thread to subscribe. Headless mode starts or resumes it through RPC.
Keep these bootstrap paths separate: a just-created thread and an acknowledged,
subscribed thread are different states. Pending prompts/settings must wait for
the appropriate bootstrap acknowledgement; a later RPC response reusing an ID
must not be mistaken for the one-time thread-start reply.

The adapter switches model/settings with `thread/settings/update` and interrupts
with the structural turn control. Requested context-window configuration is
passed at spawn; it is not a live context-capacity switch. A resumed native
thread is found through `codex_rollout::thread_for` and can seed history by
replaying its rollout. In daemon spawn logic a known resume thread forces stream
mode; a requested resume with no recorded thread warns and starts fresh.

The desktop’s explicit Windows PTY choice uses `spawnCodexHybrid`; Windows stream
mode goes through the app-server path. The desktop hybrid now checks facade
readiness and injects an operator identity token/MCP configuration. It is no
longer a “manager with no tools” exception. The daemon’s own failed app-server
fallback is another path, with its own TUI argv and generation-owned cleanup.
Do not conflate either fallback with the read-only rollout parser.

Turn failure is principally reported on `turn/completed` with failed status/error;
terminal out-of-band errors also surface, while retryable notifications are not
immediately treated as final failure. Preserve the error before the Idle update.

### Codex server requests, steering, and async questions

`*/requestApproval`, `item/tool/requestUserInput` and MCP tool-call approval
elicitations park in ONE FIFO (`ServerRequests`). Only the head is shown as the card. An
`/approve` answers only an approval at the head, and an `/answer` only a question at the head.
A request from an older turn of the same thread is stale and is pruned. Reply shapes:

- A question parks as the same `Pending::Question` Claude's AskUserQuestion uses
  (plus `secret` for `isSecret`). The reply is
  `{answers:{<questionId>:{answers:[label]}}}`. Free text is encoded the way Codex's
  own picker does it (`"user_note: …"`, behind `"None of the above"` when `isOther`).
  `isBlocking:false` is answered `{answers:{}}` after 120s, as Codex's TUI does.
- `item/permissions/requestApproval` gets the granted profile
  `{permissions, scope:"turn"}`, never `{decision}`.
- In codex 0.159, `tool_call_mcp_elicitation` is on by default. Every approval of an MCP
  tool without `readOnlyHint` (our facade and AskUserQuestion tools included)
  arrives as `mcpServer/elicitation/request` with
  `request._meta.codex_approval_kind:"mcp_tool_call"`. Allow is
  `{action:"accept", content:{}}`; deny is `decline`. Declining these blindly breaks
  every MCP tool call.
- Other elicitations and unknown methods: headless declines them or answers
  `-32601`. Hybrid leaves them unanswered. Codex sends a thread's requests to
  every connection and takes the first reply, so a refusal would pre-empt
  the TUI.
- `serverRequest/resolved` drops a parked request, and so does `turn/completed` /
  `turn/failed` for its turn or thread. Releasing the card writes `Input` when no
  turn runs, never `Responding`.

The driver and the MCP AskUserQuestion shim register separate answer channels
(`register_managed_answer` / `register_ask_answer`). `/answer` routes strictly by
the card's `PendingOwner`. A known owner without a channel is refused, not
handed to the other feed. One shared slot let the shim's unregister strip the
driver's channel for the rest of the session.

The driver uses one prompt channel (`register_managed_prompt`, `ManagedPrompt
{text, delivery}`), so send order is kept:

- A send while a turn runs is `turn/steer` with `expectedTurnId`. Only one steer is
  ever unacknowledged, and later sends wait behind it.
- A refused steer goes back to the front of the queue. "No active turn" clears the
  tracked turn and starts a new one. A mismatch steers into the turn that was found.
  A not-steerable turn waits for that turn to end.
- `TurnTracker` ignores other threads' notifications.
- A message held on a turn that has been silent for 60s goes out as `turn/start`. In 0.159
  `turn/start` is start-or-steer, so this is safe while a turn runs.
- `Delivery::Now` sends `turn/interrupt` and starts the message as the next turn.
- Errors to our own requests are tracked by id (`TurnInput::inflight`) and surfaced.

`request_user_input_async` / `send_message_to_user_async` arrive as an
`agentMessage` item with `delivery:"async"`. They come only on `item/started` +
`item/completed`, with no deltas. Questions become `ConversationItem::AssistantQuestions`
(wire kind `assistant_text` plus `questions`, `async:true` and `item_id`). It is non-blocking
and never parks. Driver notices are `AssistantNotice` (`assistant_text` +
`notice:true`). Any client that coalesces streamed assistant text MUST NOT merge
an item carrying `questions`/`async`/`notice`, and must not append later deltas to it.
The desktop applier marks such a turn `sealed`. Codex's desktop answers async questions
as ordinary input in this envelope:
`<send_user_message_question_reply>[{"questionItemId","question","answer"}]</send_user_message_question_reply>`
(see codex `tui/src/async_question_reply.rs`).

Plans use `turn/plan/updated`, including camelCase `inProgress`; partial plan delta
text is not an authoritative complete plan. Subagent identity joins three shapes:

- `thread/started` uses the child thread ID only when parent metadata is present.
- `subAgentActivity` uses `agentThreadId`, not the activity item ID.
- `collabAgentToolCall` uses `agentsStates` keys, falling back to receiver IDs;
  the sender is not the child. This path can also emit the tool card.

A requested model on a spawn-tool argument is not proof of a child’s configured
model. Preserve the distinction tested by the adapter. Child conversation replay
uses known parent/child membership before reading the rollout by thread ID.

### Codex extras (background terminals, plan mode, review, goals, names, Windows sandbox)

`providers/codex_extras.rs` rides the ws driver loop beside the chat path. Its
requests use string ids `wks-extras-<n>`; `Extras::intercept` consumes their
responses before `handle_message`, `Extras::observe` reads every message after
it. Every method is optional. Real Codex answers a method it does not know
with `-32600 "Invalid request: unknown variant `X`, expected one of …"` (its
whole method list); `-32601` / "method not found" is treated the same, and
"requires experimentalApi capability" names the opt-in. Such an error marks the
feature unavailable, or is returned to the HTTP caller as text ("this Codex
build does not support <method>", "<method> needs the codex.experimentalApi
opt-in"); any other Codex error is relayed clipped to 300 bytes. Codex 0.153.4
already has every method used here (checked with `generate-json-schema
--experimental`). Nothing waits on a reply inside the loop, so a missing method
can never wedge a session. Bootstrap results (id 2 resume, id 101 start) are
read only when they carry a `thread`, and id 101 only once.

**Experimental opt-in.** Config `codex.experimentalApi: true` → spawn payload
`codex_experimental_api: true` → `initialize.params.capabilities =
{experimentalApi: true}`. Only these need it: `thread/backgroundTerminals/*`,
`collaborationMode/list`, and `turn/start.collaborationMode`. Default off.

**Background terminals.** A `commandExecution` item with `processId` and
`source: "unifiedExecStartup"` (or no source) is tracked. It becomes a
`background_task_list` row (`id: "codex-<processId>"`,
`taskType: "local_bash"`, `description` = command, `toolUseId` = item id) when
it is still running at `turn/completed`, when `terminalInteraction` (or a
`unifiedExecInteraction` item) targets it, or when
`thread/backgroundTerminals/list` (opt-in, sent after each turn) lists it.
`item/commandExecution/outputDelta` feeds a daemon-owned log under the user's
local data dir — `<data_local>/workspacer/codex-terminals/<session>/<task>.output`
(`~/.local/share/…` on Linux, `%LOCALAPPDATA%\…` on Windows), see
`providers/codex_terminal_logs.rs` — so `sessions.taskOutput` reads it. Unix:
dirs `0700` and must be the user's own real directory, files `0600`,
`create_new` + `O_NOFOLLOW`. Bounds: 256 KiB buffered before promotion, 8 MiB per
log, 32 MiB per session. An evicted row loses its log, the driver's end removes
the session dir, and daemon start sweeps dirs untouched for 7 days. The
terminal's `item/completed` closes the row (`completed`/`failed`,
`summary: "exit code N"`). `sessions.taskStop` sends
`thread/backgroundTerminals/terminate {threadId, processId}` (opt-in; otherwise
502 naming the opt-in) and marks the row `stopped`. For managed providers,
`background_tasks` = running subagents + running task rows. Background terminals
never move the session mode.

**Plan mode.** When `collaborationMode/list` offers a `mode: "plan"` preset,
the snapshot carries `collaboration_mode: "default"` and `/permission-mode`
accepts `plan` for a Codex ws session (409 `PlanUnavailable` without the
preset; other managed sessions keep the 400 ask/yolo vocabulary). Plan rides
BESIDE the approval posture: a `plan` switch keeps the ask/yolo flag and
answers `{ok, mode: "ask"|"yolo", collaborationMode: "plan"}`, so the hub's
`livePermissionMode` stays the posture and a UI shows both (posture from
`livePermissionMode`, plan from `collaborationMode`). `ask`/`yolo` leave plan.
Codex persists the mode with the thread: a resume result's `collaborationMode`
and every `thread/settings/updated.threadSettings.collaborationMode` (ours or a
TUI change) re-seed the state, so a thread restored in plan stays in plan and
leaving it sends `mode: "default"` explicitly. Without the opt-in a thread the
TUI put in plan still shows `collaboration_mode: "plan"`. Every GUI `turn/start` then carries `collaborationMode: {mode:
"plan", settings: {model, reasoning_effort: <preset's>, developer_instructions:
null}}`. After `ask`/`yolo`, turns carry `mode: "default"` with the thread's
effort. The model comes from the spawn, the thread/start/resume result, or
`thread/settings/updated`. With no known model, the field is skipped. A
completed `plan` item becomes assistant text: `"\n\n**Proposed plan**\n\n<text>"`.
`item/plan/delta` is not rendered.

**Review.** `POST /sessions/:id/review {target, delivery?: "inline"}` →
`review/start {threadId, target}`. Inline only: detached is deprecated in 0.159
and its findings land on another thread. `target` is rebuilt from typed
fields: `uncommittedChanges`, `baseBranch{branch}` (not starting with `-`), `commit{sha(hex), title?}`,
or `custom{instructions}`. `enteredReviewMode` (started) → notice
`"Codex review started: <review>"`. `exitedReviewMode` (completed) → notice
`"Codex review finished"` + assistant text = Codex's rendered review.

**Goals.** After subscribing, the driver sends `thread/goal/get`.
`thread/goal/updated|cleared` (own thread only) keep snapshot
`goal: {objective, status, tokenBudget?, tokensUsed?, timeUsedSeconds?}` in step.
`POST /sessions/:id/goal {objective?, status?, tokenBudget?}`: `tokenBudget`
null clears it, absent leaves it alone. `POST /sessions/:id/goal/clear`.

**Thread name.** Snapshot `thread_name` comes from the resume/start result's
`thread.name` and from `thread/name/updated`. `POST /sessions/:id/thread-name
{name}` → `thread/name/set`. The hub ranks label sources: launch label
(`agents.rename`) > cwd rename > `thread_name` > hub auto-title. A hub auto-title
also names a Codex thread that has no name yet, and no title model call is
made for a Codex thread that already has a name.

**Windows sandbox** (Windows hosts only; `cfg!(windows)`). `windowsSandbox/readiness`
is sent right after `initialize` → snapshot `windows_sandbox: {ready, status,
detail?}`. A not-ready status and `windows/worldWritableWarning` (which sets
`detail`) each post a notice at most once per session, and only when the
session is not bypassed (yolo runs outside Codex's sandbox). `POST /sessions/:id/windows-sandbox/setup
{mode?: elevated|unelevated}` (default elevated) →
`windowsSandbox/setupStart {mode, cwd}` and sets `status: "settingUp"`.
`windowsSandbox/setupCompleted` re-queries readiness, or sets
`status: "setupFailed"` with the error. On other hosts the request is refused
without contacting Codex.

HTTP replies are `{ok: true, result: <Codex result>}`. Other replies: 400 bad
params, 404 no session, 501 no live Codex ws driver, 502 Codex refused (`error`
text), 504 no answer within 20 s.

Hub bus (owner tier only — none are in the view/triage scopes): `agents.review
{sessionId, target, delivery?: "inline"}`, `agents.setGoal {sessionId, objective?,
status?, tokenBudget?}`, `agents.clearGoal {sessionId}`,
`agents.windowsSandboxSetup {sessionId, mode?}`. Each answers `{ok:true,
result}` or fails with the daemon's own error text. `agents.rename {sessionId,
name}` sets the launch label (any provider with a launch record) and, for a live
Codex session, the thread name. It answers `{ok, label, codexThreadNamed?,
codexThreadError?}`. Snapshot camelCase aliases: `threadName`, `windowsSandbox`,
`collaborationMode`; `goal` is published as-is.

## OpenCode and Copilot

OpenCode’s `/event` feed is server-wide. Driver filtering must use the correct
session-ID field for each event shape so child sessions do not become the
parent’s text. The adapter observes plans through todo tools, supports per-message
model selection, and interrupts through the native session abort endpoint.

Copilot starts one process per turn with the stable session ID, retaining the
session between turns. Model/effort changes apply to the next process invocation.
Its current model-list implementation exposes `auto` after a CLI liveness probe;
free-text model input can still be refused by the account/native CLI. Historical
CLI probes explain that implementation, but are not a current vendor-wide model
catalog or guarantee about future versions.

Copilot’s terminal result is top-level and its usage is cumulative; individual
model-call usage has a different scope. `turn_outcome` considers the result,
exit status, stderr and output instead of treating process exit alone as success.
Subagent lifecycle frames carry child identity, while the child’s own prompt,
tools and reply must not be folded into the parent conversation. Todo notifications
trigger reading the provider’s local todo store rather than inventing a plan
from an empty notification payload.

Native permission labels/flags are provider-specific. Copilot’s noninteractive
path is not Claude’s interactive approval picker, and legacy ask/yolo spelling
is not a Workspacer tool grant. Treat captured CLI behavior as version-specific.

## Shared data and errors

`UsageAcc` distinguishes cumulative and additive provider reporting. Codex and
Copilot opt into token-based dollar estimation when native dollar figures are
absent; user overrides participate in the shared pricing lookup. Copilot’s account
charge and a session’s estimate remain separate. See
[usage accounting](../domains/usage-accounting.md).

Rate-limit updates merge fields rather than replacing a whole reading when one
event reports only reset time. Monthly overage is a distinct bucket, not five-hour
usage. Context-health samples have stricter runtime provenance than display
percentages; generation/model boundaries invalidate stale samples.

`AgentUpdate::Error` becomes assistant text with the shared error marker and a
trailing newline. Consecutive assistant text can coalesce, so dropping that
separator can glue a later reply onto a failure reason. Keep
`contracts/agent-error-marker-cases.json` and the fleet parser in agreement.

## Validation

From `services/claudemon`:

```bash
cargo test --lib execution::tests
cargo test --lib providers::
```

The suites cover translators, native-argv construction, fake app-server drivers,
boot acknowledgement and generation cleanup. Some real-CLI/platform fixtures are
explicitly ignored. A passing mock driver does not certify the installed external
CLI, real account permissions, or every Windows/macOS transport. For source changes,
also run the calling launcher and relevant model/context/error contract tests.
