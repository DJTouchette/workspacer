---
title: The electronAPI backend seam: IPC / bridged / web / remote transport swap
tags: [renderer-state, electronAPI, hub-bus, web-workspacer, transport]
related_paths:
  - "apps/desktop/src/renderer/src/backend/install.ts"
  - "apps/desktop/src/renderer/src/backend/webBackend.ts"
  - "apps/desktop/src/renderer/src/backend/bridgedBackend.ts"
  - "apps/desktop/src/renderer/src/backend/remoteBackend.ts"
  - "apps/desktop/src/renderer/src/backend/hubBusClient.ts"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Renderer backend selection and transport contracts

## Bootstrap and ownership

`apps/desktop/src/renderer/src/backend/install.ts` runs before React mounts.
It installs `window.electronAPI`, whose shared type is in
`apps/desktop/src/renderer/src/types/electron.d.ts`. Shared call shapes do not
imply identical capabilities, authority, platform elements or failure behavior.

| Mode | Selection | Data and terminal transport |
| --- | --- | --- |
| Web | No preload API | Hub WebSocket; token from query/sessionStorage |
| Remote desktop | `remoteClient.busUrl` present | Selected remote hub; small local shell override |
| Bridged desktop | Local bus URL/token present, desktop bus enabled | Bus data plus local terminal/native IPC |
| Direct desktop | Kill switch or missing bootstrap bus information | Existing preload IPC |

Remote wins before the direct-mode switch because main starts no local daemon
stack in that mode. `pairedWorker` does not select remote mode. Desktop obtains
credentials through `getRemoteInfo`; web stores a supplied query token under
`hubToken` in sessionStorage. A missing token does not invent authentication.

Bootstrap calls `getRemoteInfo` once and catches failure by retaining IPC.
There is no retry helper or active reachability probe in `selectBackendMode`.
Configured remote mode can therefore be left with an unusable data fallback
if bootstrap fails. A selected bus backend reconnects through its own client.

## Classifying methods

`bridgedBackend.ts` overlays `LOCAL_TERMINAL` and `HOST_ONLY` onto the web API.
Keep the complete create/spawn, attach/detach, byte output/write/resize/close
lifecycle together on IPC/MessagePorts. Control calls such as message/approve
and observation snapshots can use the bus without splitting byte ownership.

`remoteBackend.ts` overlays only `REMOTE_HOST_ONLY`: window chrome, quit/system
notices, external links/logs, remote connection settings and relaunch. Terminal
bytes remain on the remote bus. Both desktop wrappers restore `ipc.platform`;
a host OS value alone is not evidence that a method is local or available.

These arrays check valid key names, not exhaustive classification. Inspect the
actual implementation and optional-method consumers when adding an API. Some
web methods are unavailable, some return defaults, and others call real shared
services. `desktopServices.ts` maps the generated owner-service surface to
`desktop.*`; headless brain executes the TypeScript cores through its Node
companion. Availability is separate from caller authorization. See
[shared services](../modules/headless-desktop-services.md) and
`contracts/desktop-service-methods.json`.

## Connection and terminal recovery

`hubBusClient.ts` gives calls a default 15-second timeout (overridable per call),
automatically reconnects with backoff, and reasserts topic subscriptions.
Its wake handler checks a 30-second activity threshold when returning to the
page. A timeout does not establish that the remote operation had no effect.

Topic resubscription alone does not replay terminal attachment. `webBackend.ts`
tracks reprimer callbacks per PTY stream and invokes them on reconnect to
repeat `sessions.attachTerminal`. Preserve detach cleanup and the complete
transport lifecycle when adding a byte-stream consumer.

## Snapshot and conversation ownership

Brain `compatSnapshot` in `services/hub/cmd/brain/enrich.go` deliberately emits
state-only sparse rows. It now maps `statusLine` and `totalToolCalls`, plus
selection/usage/launch fields, to renderer names. Older peers may still expose
only snake_case fields; historical claims that today's brain omits these
aliases are wrong. A spawn result fills the initiating client's early launch
truth but cannot substitute for truthful snapshots seen by other clients.

`createSnapshotFold` overlays sparse data on cached richer rows, presence-merges
canonical selection, and uses `mergeConversationWindow` for bounded rich
conversation windows. Offsets are global positions, not array indices.
Stale pushes are ignored; gaps trigger one full fetch per session and withhold
the incomplete push. Ended rows release cached state. Do not mutate a prior
array or changed turn in place: React memoization depends on both identities.

For conversation-less rows, `busConversation.ts` fetches
`sessions.conversation`, coalesces overlapping pokes, re-reads the newest item
because it can grow in place, and rebuilds after a backwards sequence.
Sequence numbers and retained item positions are distinct. Preserve
`first_seq`, conversation offsets and user offsets when changing retention.

Local watched sessions subscribe to `agent.conversation.<id>`. Hub demand
tracking drives `services/hub/cmd/brain/conversation.go`, which forwards daemon
deltas only for demanded sessions. A ready signal or first delta confirms push
availability; until then, watched streaming/thinking sessions poll every
500 ms. Reconnect invalidates that confirmation and restores fallback polling.
Sessions recorded as peer/paired-owned use the qualified fetch fallback because
this client does not arm local push for them. Snapshot transitions alone are
not a sufficient clock for growing assistant text. Historical live timings are
not current performance guarantees.

## Federation, worker pairing and drill-in

The session-to-hub map belongs to each `createWebBackend` instance. Stamped
events and peer seeds populate it; calls use `hub:<peer>/<method>`. Paired
`paired:` IDs with `@paired` stay on the origin host, which holds the credential
and forwards them. Disconnects produce offline tombstones, not proof of process
termination. Current `withPeerFleets` skips sparse peer seed rows; do not infer
full headless-peer discovery parity merely from sparse snapshot folding.

For workflow transcript/conversation calls, non-null `runId` now routes through
`desktop.workflowAgentTranscript` / `desktop.workflowAgentConversation` on web.
Null `runId` selects provider-native `sessions.subagentConversation`. Bridged
desktop prefers IPC for a workflow run; for native subagents it tries bus then
falls back to IPC on an empty answer. A rejected bus call is not automatically
that empty-answer fallback. The provider folders in main and renderer must
preserve equivalent item semantics and immutable render identities.

Method parity tests cannot prove DOM parity. Native guest elements need the
browser iframe/fallback behavior described in
[webview policy](../modules/webview-security-hardening.md). Test sparse rows,
streaming deltas, reconnects, offsets and real method classifications; rich
mock snapshots alone miss the headless contract.
