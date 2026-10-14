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
last_reviewed: 2026-10-14
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

**Who is asking.** Every question of an elicitation card carries
`PendingQuestion.elicitation = Some(<server>)`, set only by `form_card` /
`url_card`. The field is `skip_deserializing`, so an AskUserQuestion input (hook
`tool_input`, the MCP ask shim's arguments) can never set it. The hub projects
`pendingQuestionSource: "elicitation" | "question"` and `elicitationServer`
(only when every question is marked). Clients must treat a card as an MCP
elicitation, offer consent wording or open a link ONLY on that mark, never on
the card's shape: an agent can word its own question identically.

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

## Live provider status (receipts, MCP, hooks, fast mode)

`session/provider_status.rs` holds runtime facts a driver reports beside the
mode state machine. They are flattened into `SessionState`, so each is a
top-level snapshot key, omitted while it carries nothing. The hub's `compat`
projection copies them in camelCase.

| Claudemon key | Hub key | Shape |
| --- | --- | --- |
| `message_receipts` | `messageReceipts` | `[{uuid, state, at}]`, newest last: at most 32, at most 10 settled (`completed`/`cancelled`), none older than 10 minutes. `state` uses the CLI's words: `queued`, `started`, `completed` and `cancelled`. |
| `mcp_servers` | `mcpServers` | `[{name, status, error?}]`. `status` is `connected`, `failed`, `needs-auth`, `pending` or `disabled`; Codex statuses are mapped onto that set. |
| `active_hooks` | `activeHooks` | `[{id, name, event, started_at}]` (`startedAt` on the hub). Hooks are removed when they respond, and cleared at turn end (a main-thread `result`, Codex `turn/completed`), on a driver respawn and when the session stops. |
| `fast_mode` / `fast_mode_disabled_reason` | `fastMode` / `fastModeDisabledReason` | The CLI's own `fast_mode_state` (`on`, `off` or `cooldown`) and its reason. It is never set from what we requested. |

`claude_status.rs` and `codex_status.rs` see each provider message before the
driver does. They consume only answers to their own requests, which use the
`wks-status-<n>` id namespace. One `StatusControl` channel per driver carries
`McpStatus`, `McpReconnect`, `FastMode` and `Interrupt`.
`SessionStore::submit_status_control` hands a request to that channel.

Claude stream wire facts, captured live on CLI 2.1.299 (2026-10-14):

- `{"type":"command_lifecycle","command_uuid":<our user-frame uuid>,"state":…}`
  arrives for every user frame we send. The sequence is `queued`, `started`,
  `completed`. An interrupted queued message reports `cancelled`. Only uuids we
  sent are recorded.
- The uuid is stamped by us. `POST /sessions/:id/message {uuid}` lets the caller
  choose it (it must be a UUID). The same value is the frame's uuid and, for a send
  into an idle session, the echoed `user_message.uuid` (rewind/fork id); a steer
  into a running turn is echoed without one, but its receipts still arrive
  under our uuid via `command_lifecycle`. The reply's `message_uuid` names it for Claude
  stream rows only.
- With `--include-hook-events`, hooks emit `system/hook_started {hook_id,
  hook_name, hook_event}` and later `hook_response {…, exit_code, stdout,
  stderr, outcome: success|error|cancelled}`. The flag is added only when
  `claude --version` reports 2.1.204 or later. The version comes from the
  stream driver's single per-spawn probe (`cli_version`, the same one that
  gates `--thinking-display`), and both gated flags are spliced in right after
  `--replay-user-messages`, ahead of the caller's extra args. An
  `outcome: error` adds an `AssistantNotice`. No other hook becomes a
  conversation item.
- `mcp_status` returns `{mcpServers:[{name,status,config,scope,source,error?}]}`.
  Each turn's `system/init.mcp_servers` (`[{name,status,source}]`) also
  refreshes the list.
- `mcp_reconnect {serverName}` answers an unknown name with the error
  `Server not found: <name>`. A successful reconnect is followed by an
  `mcp_status` refresh.
- `apply_flag_settings {settings:{fastMode:true}}` returns plain success. On
  haiku the next init still said `fast_mode_state:"off"`; the reason changed
  from `sdk_opt_in_required` to none. Success means the CLI accepted the
  setting, not that fast mode is on. The state is read from `init`, `result`
  and the `initialize` answer.
- `interrupt` returns `{still_queued:[uuid…]}`. `cancelled` appears only with
  `cancel_queued`, which we never send. The receipt goes back to the caller of
  `POST /signal {SIGINT}` as `interrupt: {supported, still_queued, cancelled}`,
  filtered to our uuids, after waiting up to 5 seconds (otherwise
  `interrupt: null`). A notice is added only when our messages were left
  pending. Never resend `still_queued` messages: they run after the interrupt.
  Older CLIs report `supported:false`.

Codex wire facts. The MCP shapes were observed live on codex-cli 0.153.4; the
hook shapes come from the schema only:

- `mcpServer/startupStatus/updated {threadId,name,status:starting|ready|failed|cancelled,error,failureReason}`.
- `mcpServerStatus/list {threadId,detail,limit}` returns
  `{data:[{name,runtimeStatus,authStatus,…}]}`. `runtimeStatus` is null unless
  `threadId` is passed, and the reply waits for servers that are still
  starting.
- `config/mcpServer/reload` takes no params and returns `{}`. Codex has no
  per-server reconnect, so `claude.mcpReconnect` reloads all servers.
- `hook/started` and `hook/completed {run:{id,eventName,status,sourcePath,entries}}`.
  Only `status:"failed"` adds a notice.
- Fast mode and interrupt receipts are refused on Codex.

The hub projection moves these fields to camelCase; it does not duplicate the
snake_case spelling. Changes apply to the row at once, but the row is published
at most once per 150 ms per session (`STATUS_PUBLISH_DEBOUNCE`). Fast mode and
MCP updates also rewrite the status line's older `capabilities.fast_mode`, MCP
count and `inventory.mcp_servers` status, so the two views agree. Only one MCP
status read per session is in flight at a time; concurrent callers share its
answer. The embedded engine runs `mcp-status` and `mcp-reconnect` in their own
two-slot pool, so they never hold the eight command slots that message sends
use. A message held before its driver registered keeps its reported uuid.

Daemon routes: `POST /sessions/:id/mcp-status` (waits at most 5 s), which returns
`{ok, mcp_servers}`; `POST /sessions/:id/mcp-reconnect {server}`, which only
accepts a server the row lists; and `POST /sessions/:id/fast-mode {enabled}`.
A session without a status driver gets 501, and a provider refusal gets 502.

Hub bus methods:

- `sessions.mcpStatus {sessionId}` returns `{ok, mcpServers}` (view and triage
  tiers).
- `claude.mcpReconnect {sessionId, server}` returns `{ok, server}` (operator
  tier, because on Codex it reloads the whole MCP configuration).
- `claude.setFastMode {sessionId, enabled}` returns `{ok, enabled}` (operator
  tier).
- `claude.signal` now adds `interrupt: {supported, stillQueued, cancelled}`
  (or `null`), plus `interruptError` when the CLI refused the interrupt.
- A delivered `agents.sendMessage` result carries `messageUuid` when the driver
  stamped one.

Hub message queue: every delivery attempt mints `Entry.message_uuid` and
stamps it on the frame. A rejected attempt is retried under a fresh uuid. An
uncertain report records `messageUuid`. When a row's `message_receipts` shows
exactly that uuid as `started` or `completed`, `Queues::confirm` turns
`queueUncertain` into `queueConfirmed {id, text, messageUuid, delivery, at}`.
Confirmation never resends and never lifts a pause. A receipt that arrives
before the acknowledgement times out makes the attempt count as sent.

PTY Claude spawns (`argv[0]` stem `claude` with no `rollout_provider`) get
`CLAUDE_CODE_ENABLE_TODO_TOOLS=1` unless the caller or the daemon environment
already set it, as the stream driver does.

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

## Conversation history: compaction, reasoning, rewind, revert and fork

New conversation-item kinds and fields. Every client keys on `kind` and skips
kinds it does not know (native `Item::display`, TUI, desktop applier, m-next),
so older clients ignore them safely.

| Shape | Meaning |
| --- | --- |
| `{kind:"reasoning", item_id, text, timestamp?}` | The model's shown reasoning. Streams as FRAGMENTS: same-`item_id` items append, like `assistant_text`; the retained log folds a trailing same-id fragment. Never merged into `assistant_text`. |
| `{kind:"compaction", trigger?, pre_tokens?, post_tokens?, timestamp?}` | Everything before it was summarized out of context. `trigger` is `"manual"`/`"auto"` (Claude only). |
| `user_message.uuid?` | The provider-facing id of that user message: the uuid the Claude stream driver stamps on the frame (= the CLI's message uuid, echoed back by `--replay-user-messages`), the transcript row's `uuid` on replay, or the echo id the Codex driver matches to codex's `userMessage` item. Absent when unknown, and on a Claude message sent while a turn runs (a steer): the CLI files it under a uuid of its own (observed live on 2.1.299), so no rewind or fork can name it — offer rewind/fork only on items that carry a `uuid`. |

Session row: `compacting` (bool), `compaction_count`, `last_compact_at` are now
also driven by managed drivers (`SessionStore::set_compacting` /
`note_compaction`), not just the PreCompact/PostCompact hooks.

Sources:

- Claude stream: `system/status {status:"compacting"}` sets `compacting`;
  `status:null` (live: `{status:null, compact_result:"success"}`) clears it;
  `system/compact_boundary {compact_metadata:{trigger, pre_tokens,
  post_tokens}}` is the boundary. `thinking_delta` streams into
  `reasoning` (`item_id` = `<api message id>:<block index>`); a thinking
  block in the `assistant` frame lands whole only when its message did not
  stream. Current models default to an omitted thinking display
  (`thinking_display:"updates"`: empty `thinking`, only `estimated_tokens`),
  which produces nothing. So stream spawns add `--thinking-display
  summarized` (a hidden CLI option) when `<bin> --version` (cached per
  resolved binary) is ≥ 2.1.221 and nobody chose a display: no
  `--thinking-display` / `--thinking disabled` in the extra args and no
  `showThinkingSummaries` key in the config root's or project's settings.
  Verified live on 2.1.299: summarized `thinking_delta` text streams.
  Transcript rows: `system/compact_boundary {compactMetadata}` and non-empty
  thinking blocks map to the same kinds (PTY sessions and resumed history).
- Codex: `item/reasoning/summaryTextDelta` streams by `itemId`;
  `summaryPartAdded` with `summaryIndex>0` adds a paragraph break; raw
  `item/reasoning/textDelta` is NOT streamed. The completed `reasoning` item
  (summary joined, raw `content` only when no summary) is dropped when its id
  already streamed. `contextCompaction` item started/completed brackets the
  compaction; the deprecated `thread/compacted` only adds a boundary when the
  turn has none.
- Handoff briefs and summary sources skip `reasoning` and `compaction`; the hub
  MCP last-message reduction skips them inside a reply run.

Controls (one store channel, `register_managed_session_control` /
`ManagedSessionControl {action, reply}`; drivers answer with the provider's own
result or error text). Naming is deliberate: **rewindFiles restores FILES
(Claude), revert drops TURNS (Codex)**; each provider refuses the other with an
error naming the alternative.

| Bus method | claudemon route | Claude stream | Codex |
| --- | --- | --- | --- |
| `claude.compact {sessionId, instructions?}` | `POST /sessions/:id/compact {instructions?}` | `/compact [instructions]` as a user message (documented headless path; verified live on 2.1.299) plus a `slash_command` echo; reply `{queued:true}` | `thread/compact/start {threadId}`; reply `{}` |
| `claude.rewindFiles {sessionId, userMessageId, preview?}` (`preview` is the dry run) | `POST /sessions/:id/rewind-files {user_message_id, dry_run?}` | `rewind_files` control request; reply `RewindFilesResult {canRewind, error?, filesChanged?, insertions?, deletions?, skippedLinks?}` | refused |
| `claude.revert {sessionId, userMessageId \| beforeTurnId}` | `POST /sessions/:id/revert {user_message_id?, before_turn_id?}` | refused | `thread/revert {threadId, beforeTurnId}`; on success the shown conversation is cut before that turn's user message (a `reset` delta) |

HTTP answers: `200 {ok:true, result}`, `502 {ok:false, error}` (provider
refused), `501` (no live stream/Codex driver), `504` (no answer in 30s), `400`
for ids that are not bounded plain ids. All three bus methods are full-tier only.

Claude stream sessions always run with `--replay-user-messages` (replayed text
frames carry `isReplay:true` and render nothing: the `user` arm reads only tool
results) and `CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING=true` unless the spawn
env or daemon env chose a value. Verified live on 2.1.299 (persisted session,
haiku): after a Write, `rewind_files {user_message_id:<our stamped uuid>,
dry_run:true}` answered `{canRewind:true, filesChanged:[<abs path>],
insertions:0, deletions:1}` and the real call `{canRewind:true,
skippedLinks:0}` and removed the file; the transcript's user row carried the
same uuid. A session started with `--no-session-persistence` has no
checkpoints: every rewind there answers "No file checkpoint found for this
message." (a dry run as a success with `canRewind:false`, a real one as a
control error).

Fork (claudemon `/sessions/spawn-managed {provider, cwd, resume:<source>,
fork:true, fork_before?:<user message uuid>}`): a NEW row id (never the
source's). "Fork before message U" keeps the history before U. Claude runs
`--resume <source> --fork-session --session-id <new> [--resume-session-at
<the nearest user/assistant entry up U's parent chain>]`; forking before the first message is refused, and so is a message absent from the source's visible history. The direct parent is often a `system` row (turn duration, hooks); a resume at one was refused live on 2.1.299 (`error_during_execution`), so those rows (and `attachment` rows) are skipped. Verified live on 2.1.299: forking before the 2nd of three turns resumed at the 1st turn's assistant entry (walking past `stop_hook_summary` and `attachment` rows), and the fork remembered turn 1 only. A `priority:"next"` message sent while IDLE keeps our uuid (verified live); only one steered into a running turn does not. `claude.compact` while a turn runs is fine: `/compact` with `priority:"next"` ran as the command after the turn, keeping its uuid (verified live).
Codex bootstraps the new driver with `thread/fork {threadId:<source thread>,
lastTurnId?, cwd}` as request id 101 (same capture as `thread/start`); the fork
point (the turn before U's turn) comes from the LIVE source driver
(`SessionControlAction::ForkPoint`), so forking a Codex session from a message
needs the source running. The new session is seeded with the source's visible
history cut at the fork point.

Hub `agents.fork {sessionId, atMessageId?, label?}` (full tier, local only;
refused for a hub-qualified call and before launch readiness, like
agents.spawn) returns the agents.spawn receipt (`sessionId` of the new
session). It needs this hub's launch record for the source, and takes from it
provider, cwd, profileId, mcpItemIds, model and parentSessionId; never
manager, label (pass `label`), permission bypass, or a launch integration
(such sources are refused). The fork rides OUT OF BAND
(`SpawnCoordinator::fork_for` → `spawn_admitted(.., Some(ForkSource))`), so
agents.spawn's wire keys and the spawn-key registries are unchanged and no
agents.spawn caller can name a fork source. After routing the plan must be a
managed stream launch of the source's provider; the coordinator then sets
`request.{resume, fork, fork_before}` and `metadata.forkedFrom`.
