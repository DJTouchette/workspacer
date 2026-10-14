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

### Card ids, decisions and "Always allow" (managed providers)

**Card id.** Every card the Claude stream and Codex drivers park carries
`pending.raw.pending_id` (a string: the CLI control `request_id` / the codex
JSON-RPC id). The hub projects it as top-level `pendingRequestId`, and for an
approval also as `pendingApproval.requestId`. Clients echo it as `requestId` on
`claude.approve` / `claude.answer` (claudemon `/approve` and `/answer`
`requestId`).
- claudemon refuses a `requestId` that names a card other than the one shown,
  with 409 `{error, requestId, current}`.
- When an older client sends none, claudemon binds the decision or answer to the
  card shown when it arrived.
- The driver then drops it if its queue head has moved on. So an in-flight answer
  can never land on a newer card.

OpenCode, Pi and hook approvals have no card id, and behave as before.

**Decision channel.** The decision goes to the driver as ONE message,
`ManagedDecision {approve, always_option, note, request_id}`, on the decision
channel. There are no per-session side flags, so a second client's decision
cannot consume a flag that was meant for another card. `submit_managed_decision`
also accepts a bare `bool`: a plain yes/no.

**Legacy `decision:"always"` is exactly `"yes"`.** It is a plain allow on every
provider, with no `updatedPermissions`, `acceptForSession` or `persist`. Shipped
clients send it for their "Allow all" button on every approval, without having
shown anything that would be granted.

**Explicit always option.** The card lists what it offers:
- claudemon `pending.raw.always_options` (approval cards only):
  `Array<{id: string, label: string, scope: "session", kind: string}>`, `[]`
  when nothing is offered.
- The hub projects the same array as `pendingApproval.alwaysOptions`, present
  only when non-empty. The frozen `contracts/hub-snapshot-cases.json` rows
  still hold.
- `id` is `<pending_id>:<index>`, unique to that request.

The user grants exactly ONE option with `claude.approve {sessionId,
decision:"yes", alwaysOption:<id>, requestId:<pending_id>}`.
- `alwaysOption` requires `decision:"yes"` AND `requestId`. The hub rejects it
  otherwise, and claudemon answers 400.
- claudemon answers 409 unless the shown card offers that `id`.
- The driver drops the decision if the option is not on its head card.

Claude stream suggestions come from `can_use_tool.permission_suggestions` (CLI
`PermissionUpdate[]`), allow-listed and rewritten to `destination:"session"`:
- Offered, one option each:
  - `addRules` with `behavior:"allow"`. `kind` is `addRules`; the label is
    `Always allow Bash(npm test:*) this session`.
  - `setMode` to `acceptEdits` only. The label is `Allow all edits this
    session`.
  - `addDirectories`. The label is `Always allow access to /x this session`.
- Dropped: `replaceRules` (it can wipe earlier session grants), removals,
  deny/ask rules, and any other mode (`auto`, `dontAsk`, `bypassPermissions`,
  `plan`).
- When the request has `suppress_always_allow_rule:true`, nothing is offered.

The answer is `{behavior:"allow", updatedInput, updatedPermissions:[<that one
update>], decisionClassification:"user_permanent"}`. The schema was read from
the CLI 2.1.299 bundle, and a stub test covers the end-to-end path. A live CLI
applying the rule is unverified.

The CLI's own permission mode is mirrored as claudemon `permission_mode`, which
the hub exposes as `observedPermissionMode`. It comes from `permissionMode` on
`system/init` and `system/status`, and from a confirmed `set_permission_mode`.
So an "Allow all edits" grant shows up in the session's state.

Codex offers at most one option, with id `<pending_id>:0`:

| request | `kind` | label | option → |
|---|---|---|---|
| command approval with `"acceptForSession"` in `availableDecisions` | `acceptForSession` | Always allow this command for this session | `{decision:"acceptForSession"}` |
| `item/fileChange/requestApproval` | `acceptForSession` | Always allow changes to these files / writes under `<grantRoot>` for this session | `{decision:"acceptForSession"}` |
| `item/permissions/requestApproval` | `permissionsSession` | Grant these permissions for this session | `{permissions, scope:"session"}` |
| MCP tool-call elicitation whose `_meta.persist` includes `"session"` | `mcpPersistSession` | Always allow this tool for this session | `{action:"accept", content:{}, _meta:{persist:"session"}}` |

`acceptWithExecpolicyAmendment` and `persist:"always"` are never sent: they
persist beyond the session. Codex's own default command decision list often
omits `acceptForSession`; when it does, no option is offered.

**Deny with a note.** Claude keeps `{behavior:"deny", message:"The user denied
this tool use and said: <note>"}`. Codex's `decline` carries no message, so the
driver does two things:
1. It answers `decline`.
2. It sends the note as a `Delivery::Steer` user message, `Denied <tool or
   summary>: <note>`, through the ordinary prompt path. The message is echoed
   in the conversation.

Any queued approval stays the card.

### Claude stream request queue

`can_use_tool` approvals, AskUserQuestion and MCP elicitations share ONE FIFO
(`ParkedCanUse`, `ParkedKind`), as the CLI's own UI queues its dialogs.
- Only the head is the card.
- `/approve` answers only an approval at the head; `/answer` answers only a
  question or elicitation at the head.
- Answering the head surfaces the next request of any kind, or releases the card
  to `Responding`.
- Nothing is cancelled or denied as "superseded".
- A turn-closing `result` drops the queue. A parked elicitation is answered
  `{action:"cancel"}` first, the CLI's own value for an abandoned dialog. The
  approvals and AskUserQuestion it drops get no reply, as before the queues
  merged.

### MCP elicitations on the question card (Claude stream + Codex)

`providers::elicitation` maps an MCP elicitation onto `Pending::Question`, the
one card every client already renders and answers through `/answer`. Claude and
Codex share the mapping. Elicitations park under YOLO too: they ask for the
user's data, not permission.

**Form mode** adds one question per top-level property:
- `enum` / `oneOf` const → options, labelled from `enumNames` / `title`. The
  answer sends the underlying value.
- `boolean` → `Yes` / `No`.
- `string`, `number` and `integer` → free text, parsed per type.
- `array` of enum → `multi_select`. The answer is comma-separated, and a
  comma-only answer counts as empty.

Each question's `header` is the property `title` (or its key). Its `question` is
the `description` (or title), plus ` (required)`. Questions follow Claude's
`property_order`; otherwise they follow the map order, which serde_json sorts.

A final consent question closes the card. Its header is `<server> asks`, its
text is the server's `message`, and its options are `Submit` / `Decline`. ONLY
an explicit Submit sends the user's data: option 1, or the label in any case.
Everything else declines, including unanswered, free text, an out-of-range
number, and a literal-text `"1"`.

The schema's limits are checked before anything is sent: `minimum`, `maximum`,
`minLength`, `maxLength`, `format` (email / uri / date / date-time),
`minItems` and `maxItems`.

**Url mode** is one question: the message, then ``The MCP server `<server>` asks
you to open:\n<url>``.
- The options are `Open` and `Decline`. `Open`'s `description` is the URL, so a
  UI can open it itself.
- Only `Open` (option 1, or the label) accepts, as `{action:"accept"}`.
- Only an absolute `http(s)` URL with a host is offered. Anything else is
  declined, with a notice.

**Replies.**
- An accepted form replies `{action:"accept", content:{key: typed value}}`. An
  empty optional field uses its `default` or is omitted.
- A decline replies `{action:"decline"}`.
- An answer the schema rejects is declined, and a notice explains why. Nothing
  is ever sent wrong.

**Unsupported:** nested objects, arrays of objects, and other modes
(`openai/form`, `openai/userVerification`, …). Claude declines these, and so
does headless Codex; hybrid Codex leaves them for the TUI. Each case leaves a
notice.

**Claude wire**, from the CLI 2.1.299 bundle schemas `ba` / `aAo` (there was no
live MCP probe): the CLI sends `control_request {subtype:"elicitation",
mcp_server_name, message, mode?, url?, elicitation_id?, requested_schema?,
property_order?, title?, display_name?, description?}`. The driver answers
`control_response success {action, content?}`. Previously it answered with an
error reply, which the CLI turns into `cancel`. `pending.raw` is the CLI request
plus `pending_id`.

**Codex wire:** `mcpServer/elicitation/request` params are FLAT: `threadId`,
`turnId`, `serverName`, `mode`, `message`, `requestedSchema` | `url` +
`elicitationId`, and `_meta`. See 0.159 `v2/mcp.rs` `#[serde(flatten)]` and its
serialization test.
- Until 2026-10-13 the driver read `params.request._meta…`, so on the real wire
  MCP tool-call approvals never matched, and headless sessions declined them as
  unshowable.
- `elicitation_body` reads the flat params and still accepts a nested `request`.
- Forms park as `ParkedKind::Form` in the same FIFO as approvals and questions,
  and the reply adds `_meta:null`.
- `pending.raw` is the request params plus `pending_id`.

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
  arrives as `mcpServer/elicitation/request` with (flat)
  `_meta.codex_approval_kind:"mcp_tool_call"`. Allow is
  `{action:"accept", content:{}}`; deny is `decline`. Declining these blindly breaks
  every MCP tool call.
- Form/url elicitations park as question cards (see "MCP elicitations on the
  question card" above). Unsupported ones and unknown methods: headless declines
  them or answers `-32601`. Hybrid leaves them unanswered. Codex sends a thread's
  requests to every connection and takes the first reply, so a refusal would
  pre-empt the TUI.
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
