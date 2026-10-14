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
