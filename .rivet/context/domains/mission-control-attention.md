---
title: Mission Control: promoted snapshot store, attention feed, and Inbox/Fleet projections
tags: [renderer-state, mission-control, attention, snapshot-store, viewLevel, resolve-actions, approval]
related_paths:
  - "apps/desktop/src/renderer/src/contexts/AttentionContext.tsx"
  - "apps/desktop/src/renderer/src/hooks/useAttentionFeed.ts"
  - "apps/desktop/src/renderer/src/lib/attentionRouter.ts"
  - "apps/desktop/src/renderer/src/lib/resolveAttention.ts"
  - "apps/desktop/src/renderer/src/types/attention.ts"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Attention feed, resolution and notification history

## Snapshot ownership

App consumes `useSessionSnapshots` for shared status and compact background
`snapshotBySession` maps. Routine observations batch at 100 ms; new sessions,
state/decision changes flush immediately, including queued observations to
preserve ordering. Mount/reconnect pulls are generation-fenced and preserve
live changes arriving during the pull. Ended/terminated sessions are pruned
and fenced against late re-adoption. Initial pull failure still releases the
preexisting-session bootstrap barrier with an empty set.

`useAttentionFeed` derives renderer-local approval, question, done, bigdiff,
stuck and error items. It is not a durable daemon-side inbox. Fleet/Sidebar
consume `topByAgent`; the Inbox uses the same feed with filters and selection.
`AttentionContext` owns view/navigation/actions. Action-only consumers use
`useAttentionActions`; destructuring actions from the full context does not
avoid subscriptions to volatile data.

## Identity and heuristics

Signatures start with session ID and kind. Approval hashes tool name plus JSON
input; questions hash the joined question texts; done uses the observed done
time; error uses the tool-call ID. Bigdiff uses file count and the rounded
20-line estimate bucket, not the exact line count. Stalls also have distinct
`stalled` and `wfstalled:<runId>` signatures that do not change with elapsed time.
These are UI deduplication identities, not collision-proof daemon request IDs.

Dismiss/snooze state is local and keyed by signature. Current pruning uses
`sig.split(':')[0]` against live session IDs: IDs containing colons, including
paired IDs, do not fit that assumption and can lose suppression state on a
live-set change. Do not document this as an arbitrary-ID-safe parser.

Done detection remembers working-to-idle transitions in refs; it does not
infer a past completion merely by attaching to an idle session. Bigdiff fires
over 80 estimated lines while idle/waiting-input and unblocked. Compacted
inputs use `originalChars / 40` as an estimate; these cards are not git truth.
Pending questions age into stuck after five minutes; working sessions/workflows
use separate progress fingerprints and stall verdicts. The five-second ticker
runs when time-sensitive work exists and the page is visible. Error recency and
all heuristics depend on the available compact projection.

Priority is approval 100, question 95, error 80, stuck 70, bigdiff 40, done 20,
then oldest first. `topByAgent` takes the first sorted item per agent. Any open
item score (`1000 + priority`) outranks bare ambient-state bands. Missing detail
must not be presented as proof of inactivity or no changes.

## Resolve and navigation behavior

Actions address session IDs, not the currently mounted pane's MessagePort.
`resolveApproval` calls the structured endpoint first. Only a rejected Promise
can trigger its Claude-PTY keystroke fallback; stream/non-Claude and concurrent
question-picker cases suppress it. Managed/stream questions use structured
answers; Claude PTY questions drive the picker directly. `resolveReply` uses
the queue-capable message endpoint and only uses bracketed-paste fallback on
rejection, not a returned `{ok:false}`. These helpers log failures and do not
return a durable resolution receipt to the caller.

Snapshots supply provider **and transport**. Stream-Claude is not PTY-Claude,
and its pending decisions arrive through managed-mode folding. Approvals also
appear in the chat's `NeedsYouDock`; the old attention-only claim is obsolete.
Keep its optimistic dismissal and transport rules consistent with triage.

While piloting, the context dismisses all current items for the active agent,
regardless of Inbox filter. `openAgent` dismisses matching items in its current
feed, closes Inbox and switches to piloting. This hides attention; it does not
approve or answer the underlying request. New signatures can surface again.
Peer-stamped sessions use owning-hub routing. Offline tombstones represent lost
connectivity rather than confirmed process termination.

## Notification history is separate

`NotificationsContext.tsx` ingests native notifications, bus `notify.post`, local
renderer posts and system notices through `notificationStore.ts`. It is a
bounded history, not the active attention queue: 200 in memory, 100 persisted
to localStorage. Same-ID delivery is ignored; same-key entries replace and move
to the top. It is therefore not append-only.

Normalization clamps text and allows only absolute HTTP(S) URLs, including on
restore. Persistence failure is nonfatal and loses reload survival. Toast
preferences affect transient presentation; disabling OS notifications does not
turn off center history. Main emits keyed needs-you/done records and makes
watched-session records silent.

Renderer-origin, nonsilent notifications can escalate when the document is
unfocused; `manager-request` receipts are explicitly excluded. Native-origin
records do not re-escalate because main already made that decision. Main
rechecks OS preferences; web uses browser notification permission. Seen-ID
tracking suppresses repeated transient effects, not all future same-key events.
Click-through marks read and navigates; it is not an implicit approval action.

## Verification

Source review covers signatures, pruning, heuristic timing, resolve guards and
notification ingestion. Focused tests cover attention actions/projection and
notification normalization. No live OS delivery or cross-device durable inbox
is claimed. Keep sparse/compacted fixtures alongside rich desktop snapshots.
