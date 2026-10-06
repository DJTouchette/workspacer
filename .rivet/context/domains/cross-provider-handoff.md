---
title: Cross-provider agent handoff (brief authoring + successor spawn)
tags: [handoff, cross-provider, claudemon, tui, agent-spawn, session]
related_paths:
  - "apps/desktop/src/main/services/agentHandoff.ts"
  - "apps/desktop/src/main/services/claudemonSessionClient.ts"
  - "apps/desktop/src/main/ipc.ts"
  - "apps/desktop/src/main/preload.ts"
  - "apps/desktop/src/main/shared/ipcChannels.ts"
  - "apps/desktop/src/main/services/hubCapabilities.ts"
  - "apps/desktop/src/renderer/src/App.tsx"
  - "apps/desktop/src/renderer/src/panes/ClaudePane.tsx"
  - "apps/desktop/src/renderer/src/lib/watchBus.ts"
  - "apps/desktop/src/renderer/src/components/claude/ConversationEmptyState.tsx"
  - "apps/desktop/src/renderer/src/backend/webBackend.ts"
  - "services/hub-rs/src/services/live_controls/handoff.rs"
  - "services/hub-rs/src/services/live_controls.rs"
  - "apps/native/src/handoff.rs"
  - "apps/native/src/controller.rs"
  - "apps/native/src/ui/handoff.rs"
  - "apps/desktop/src/renderer/src/components/claude/HandoffDialog.tsx"
  - "services/claudemon/src/session/handoff.rs"
  - "services/claudemon/src/daemon/api.rs"
  - "apps/tui/src/claudemon.rs"
  - "apps/tui/src/bus.rs"
  - "apps/tui/src/app/input/pickers.rs"
  - "apps/tui/src/keys.rs"
owner: Damien Touchette
last_reviewed: 2026-10-05
---

# Cross-provider handoff

## What a handoff establishes

This flow prepares a Markdown brief and starts a successor in the source working
directory. It can also keep the same provider with a fresh context. It does not
mutate a running session into another provider, terminate the source, or transfer
Fleet Manager journal/task/worker ownership. See
[manager recovery](session-lifecycle.md#manager-lineage-and-restart-recovery)
for that separate transaction.

Provider-neutral history does not imply every legacy adapter is a valid launch
target. Workspacer admits Claude, Codex, OpenCode and Copilot; normal launchers
reject Pi even though raw daemon code retains it. A persisted path must be
readable on the successor's host; returning a path is not a cross-host file copy.

## Mechanical brief

`services/claudemon/src/session/handoff.rs` is the deterministic builder shared
by REST and bus callers. `POST /sessions/:id/handoff` validates the session ID,
reads cached/tailed conversation items, and returns `{ok, markdown, path}`.
Stopped sessions can work if their conversation remains available. Missing
conversation returns 404; invalid ID returns 400; persistence failure returns
500. `no_persist=true` returns Markdown with a null path. Ordinary desktop/TUI
callers request persistence.

The builder includes metadata when session state exists, up to 25 recent user
requests clipped to 240 characters each, file classifications, and a recent
exchange. Tool-name/path heuristics classify files; they are not a verified
filesystem diff. Edited paths are removed from the read-only bucket, with up
to 40 displayed paths per bucket.

The exchange walks newest-first then renders chronologically. The most recent
assistant text gets a 5,000-character cap; other text gets 1,600, tool detail
200. Successful tool results, usage and command output are omitted; errored
results keep a marker and only the latest nonempty plan is retained. The overall
10,000 budget uses rendered **byte length**, despite the source's character
wording, and is soft: the first included block may exceed it.

Files live under the host home directory's `.workspacer/handoffs`, named with
second-resolution timestamp and the first eight session-ID characters.
`persist_brief` uses a plain write, so repeated same-name writes can replace one
another; do not treat these filenames as immutable request identities.

## Agent-authored brief

Desktop `apps/desktop/src/main/services/agentHandoff.ts` and the Rust hub's
`services/hub-rs/src/services/live_controls/handoff.rs` (dispatched from
`live_controls.rs`; the Go `agenthandoff.go` is historical) both ask the source
to write a six-part brief and poll for up to 150 seconds at one-second intervals. Sending failure
or deadline triggers the mechanical fallback, with `fallback=true` on success.
Directory creation errors occur before this fallback path and can reject the
operation. The source needs to accept a turn; the mechanical fallback does not.

The implementations differ: TS uses a timestamp/session-prefix filename and
`stat` size; Rust uses a timestamp/random-nonce `-agent.md` name in a 0700
directory, a nonempty regular file via `symlink_metadata`, and returns
`ok:false` (not an error) when the fallback yields no path. The historical Go
owner validated a narrower alphanumeric/dash/underscore ID, uses a
nonce filename, honors context cancellation, and requires a nonempty regular
file via `Lstat`. Go cancellation returns the context error rather than falling
back. Neither completion check validates all six sections or proves writing
has finished. A bus caller's timeout may also expire before the service's
150-second wait; do not equate that timeout with a cancelled source-agent turn.

`claude.handoffBrief` and `claude.handoffAgentBrief` are bus methods; desktop
preload exposes the corresponding `claudeHandoff*` methods. The headless rich
tier exists now; older comments describing it as desktop-only are obsolete.

## Native successor flow

The GPUI client's header arrow (`open-handoff`) and Session details offer
"Continue with Codex…" on Claude sessions and "Continue with Claude…" on Codex
sessions; native launches admit only those two. `apps/native/src/handoff.rs`
holds the rules: Fleet Manager rows (`isWakeTarget`/`isFleetManager`, projected
as `Session.wake_target`) are refused with a pointer to manager replacement;
access carries the source's `livePermissionMode`/`settings.permissionMode`
(bypass↔yolo kept, anything the target lacks → Ask, never the wider device
default); model/effort start on the target's catalog defaults. The page reuses
the New Agent pickers like Change model does, scoping the Codex catalog to the
source folder, and checks `providers.checkAll` (no test request) to disable a
missing target.

`Command::Handoff` in the controller calls the brief method (180 s client
budget for the agent tier), then normal `agents.spawn` in the exact source cwd
with no message. The takeover prompt (desktop wording) reaches the successor's
composer via `SpawnReceipt.unsent_message`; nothing is sent for the user. One
handoff per connection: concurrent requests, launches and validation failures
produce a `handoff_receipt` instead of being dropped. A launch failure names the
brief left behind and never sets the New Agent form's spawn error. The source is
never signalled. Native lists local-hub rows only, so the brief path and the
successor are on the same host. Tests: `src/handoff.rs` units,
`src/ui/handoff.rs` UI, and `tests/protocol.rs` `handoff_*` against a fake bus.
No live Claude/Codex handoff was run for this flow.

## Desktop and TUI successor flows

`HandoffDialog.tsx` collects provider/model/effort/permission settings and brief
type, starting from source-session values. Unsupported permission modes fall
back to the target's offered mode; bypass-family intent is translated. Provider
visibility is filtered by detection, with source-provider retention; final
launcher admission remains authoritative.

`ClaudePane.tsx` obtains the brief, logs failure/fallback, then emits
`AGENT_HANDOFF_EVENT` with path, cwd and chosen settings. App requires provider,
path and cwd and calls normal `spawnAgent` with a takeover `initialPrompt`.
That prompt prefills the desktop composer for review. The empty-state treatment
currently recognizes the literal phrase `handoff brief at `; preserve that or
replace it with an explicit signal when changing the prompt.

`apps/tui/src/app/input/pickers.rs` probes offered providers, requires cwd and
refuses remote-owned sessions. Its handoff uses the mechanical tier through
`Driver::handoff` (bus when connected, direct REST otherwise). Claude targets
require a default/first profile and receive an unsent composer seed. Managed
targets spawn and then receive a separate `Driver::message`; that call's result
is currently ignored. A successful spawn/toast therefore does not prove prompt
acceptance. This is distinct from atomic first-message-in-spawn entry points.

Failed successor creation can leave an unused brief. Neither frontend rolls
back all prior steps as one transaction. Tests cover the deterministic builder,
API validation and headless fallback; dialog tests cover chosen launch settings.
No live-provider authoring or cross-host file transfer is claimed by this audit.
