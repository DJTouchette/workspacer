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
last_reviewed: 2026-08-16
---

# The electronAPI backend seam: IPC / bridged / web / remote transport swap

## Overview
The entire React renderer talks to exactly one object, `window.electronAPI` (typed by `ElectronAPI` in `apps/desktop/src/renderer/src/types/electron.d.ts`), and never learns which transport backs it. `install.ts` runs in `main.tsx` before any component mounts and assigns one of four factory-built implementations to that global. This is what lets the same renderer bundle run as the Electron desktop app, as a plain browser client of the hub (web-workspacer), or as a remote-client shell — all driving/observing agents through identical calls.

## Key modules
- `apps/desktop/src/renderer/src/backend/install.ts` — `installBackend()` decides transport via `selectBackendMode()`; awaited before React renders.
- `apps/desktop/src/renderer/src/backend/webBackend.ts` — `createWebBackend(token, busUrl?)` builds the hub-bus-backed `ElectronAPI`; base implementation reused by bridged and remote modes.
- `apps/desktop/src/renderer/src/backend/bridgedBackend.ts` — `createBridgedBackend(ipc, token, busUrl)` wraps the web backend, delegating `LOCAL_TERMINAL` + `HOST_ONLY` method lists back to the preload IPC.
- `apps/desktop/src/renderer/src/backend/remoteBackend.ts` — `createRemoteBackend(ipc, token, busUrl)` wraps the web backend dialed at a remote hub, delegating only the small `REMOTE_HOST_ONLY` list (no local-terminal fallback — there is no local claudemon).
- `apps/desktop/src/renderer/src/backend/hubBusClient.ts` — `HubBusClient`, the raw WebSocket RPC/pub-sub client (`{op}` frames: call/result/error, subscribe/unsubscribe/event) shared by webBackend/bridgedBackend/remoteBackend; 15s call timeout (`CALL_TIMEOUT_MS`), 30s staleness detection, auto-reconnect with backoff, `onReconnect()` hook.
- `apps/desktop/src/main/services/hubCapabilities.ts` — main-process registry of what the hub RPC surface actually supports; webBackend's stub coverage is meant to track this.

## Failure modes
- `installBackend()` calls `ipc.getRemoteInfo()` through `getRemoteInfoWithRetry()` — bounded backoff `[50,150,300,600,1000]`ms (~2.1s, all before first paint), because the renderer's first paint can beat IPC handler registration ("No handler registered for…"). Only after every attempt fails does it keep the preload IPC backend, and it now logs `console.error` (not `warn`) naming the remote-client consequence explicitly. Retrying is not cosmetic: the renderer cannot distinguish "local install, IPC is correct" from "remote-client mode, IPC is dead" without this very answer, so a single failed ask used to strand remote users on a window that looked fine and did nothing.
- `selectBackendMode()` falls back to `'ipc'` whenever `info.busUrl` or `info.token` is missing, so a half-initialized hub degrades to IPC, not a broken bridged backend.
- `HubBusClient.call()` rejects with `hub call timeout: ${method}` after 15s if no `result`/`error` frame arrives; every `.call(...)` site in webBackend that doesn't `.catch(() => {})` will surface that rejection to its caller (many fire-and-forget paths like `writeTerminal`/`claudeWrite` swallow errors instead).
- After a bus reconnect, per-stream `sessions.attachTerminal` calls are NOT automatically re-issued by the bus's topic re-subscription — `webBackend.ts` compensates by registering every live PTY stream's re-attach thunk in a `reprimers` map and firing them all from `client.onReconnect()`; a stream that forgets to register here (or a new PTY-consuming method) will sit frozen after reconnect until a manual resize.
- Sparse vs rich snapshot races: `foldSparse()` in webBackend.ts merges `sparse: true` brain-provided snapshots onto the last rich desktop-provided snapshot per `sessionId` (dropped from cache on `status: 'ended'`); if a session's snapshots interleave in the wrong order or the cache map is bypassed, sparse data can wrongly present as fully-detailed.

## Gotchas
- **Three-plane split in `bridgedBackend.ts` is load-bearing.** Control/observation (message/approve/answer/snapshots/config/library/etc.) route over the bus so desktop and web drive agents identically. The PTY byte lifecycle (`createTerminal`/`spawnClaude`/`attach*`/`detach*`/`*Output`/`*Write`/`*Resize`/`*Close`, listed verbatim in `LOCAL_TERMINAL`) MUST stay on the preload IPC/MessagePort — splitting create/attach from write/resize/close across transports orphans the port. On web (no MessagePort across a wire) the same bytes cross as base64-framed `pty.bytes.<sessionId>` bus events (`decodePtyChunk`/`streamPty` in webBackend.ts).
- **`HOST_ONLY` and `REMOTE_HOST_ONLY` are exhaustive `keyof ElectronAPI` maps that must be kept current.** Any new electronAPI method has to be explicitly classified (bus-mirrored plane, LOCAL_TERMINAL, or HOST_ONLY/REMOTE_HOST_ONLY) or it silently inherits whatever `createWebBackend`'s stub does — usually `warnOnce(...)` returning a safe default, tagged `HUB-TODO` in comments, tracked loosely against `hubCapabilities.ts`.
- **`selectBackendMode()` precedence is pure and order-sensitive**: `remoteClient?.busUrl` wins outright over everything else (checked first), because in remote-client mode main spawns no local daemons at all — neither bridged bus nor IPC data paths have anything local to talk to. Only after ruling out remote does it check the `WORKSPACER_DESKTOP_DIRECT=1` kill switch (`desktopBus === false`) and missing `busUrl`/`token`, both of which fall back to `'ipc'`.
- **`bridgedBackend` vs `remoteBackend` diverge specifically on the local-terminal slice**: bridged delegates `LOCAL_TERMINAL` to IPC (there IS a local claudemon); remote does NOT — remote's PTY bytes ride the bus as `pty.bytes.*` just like a plain web client, because sessions live on the remote host.
- **`api.platform = ipc.platform` override in both bridgedBackend and remoteBackend** — `createWebBackend` always sets `platform: 'web'`, which the UI uses to gate native-only chrome (e.g. Windows titlebar overlay); both wrappers restore the genuine host platform after spreading the web backend, or native chrome breaks on Windows desktop.
- **Token plumbing has two independent sources**: web/browser reads `?token=` query param cached into `sessionStorage` (`TOKEN_KEY = 'hubToken'`, `resolveToken()` in install.ts); desktop bridged/remote modes get the token from `ipc.getRemoteInfo()` instead and never touch `sessionStorage`.

## Hand-authored notes (2026-08-16) — webBackend is federation-aware

- `createWebBackend` now merges federated peer fleets itself (a browser client has no main-process `federationBridge.ts`): a module-scoped `sessionHub` map records which hub owns each session, fed by (a) envelope `hub` stamps on `agent.snapshot` events and (b) a peer-fleet seed on connect/`hub.peer.connected` (`federation.peers` then `hub:<peer>/sessions.snapshots`, rows stamped with `hub`). Every per-session call goes through a qualify helper (`hub:<hub>/<method>` for remote sessions, bare otherwise); `hub.peer.disconnected` marks that hub's snapshots as tombstones (`hubOffline`) instead of dropping them. A new per-session electronAPI method that bypasses the qualify helper silently acts on the wrong machine (no error — the local hub just has no such session). `federationPeers` is exposed on the API for the renderer's HubChip/Machine picker. `backendParity.test.ts` + `renderer/tests/federation.test.ts` pin the shape.

## Hand-authored notes (2026-08-24/26) — the headless/bus conversation seam

Nine learnings from the "make /app work against a real `workspacer serve` node"
push. The single theme: **the seam's fixtures and the desktop's IPC both hide the
shape a real headless fleet actually publishes.**

- **A headless node publishes a session row only on MODE transitions.** For a
  managed (stream-transport) session claudemon's `SessionStore::ingest` returns
  early — hooks are enrichment only — so `update_tx` (which feeds `/events` →
  brain → `agent.snapshot`) fires ONLY from `set_managed_mode`, `set_plan`,
  `set_background_tasks` and `park_decision`. Assistant text growing is not a
  state change. Measured on a local `workspacer serve` + managed claude: one 21s
  turn produced 32 `conversation.delta` frames inside claudemon and exactly TWO
  `agent.snapshot` events on the bus. Any bus client that treats `agent.snapshot`
  as its conversation clock renders once, at turn end — the real /app bundle sat a
  median 10.3s / worst 20.8s behind the daemon. **This is the whole "remote
  replies are inconsistently slow" report**; short turns look instant, long ones
  look dead. claudemon HAS a live delta feed (`/conversation/stream`) which the
  desktop and wks-tui consume; the brain does not forward it to the bus.
  Worked around in `webBackend.ts` by ticking `sessions.conversation` every 500ms
  while a WATCHED session's row says `ambientState: 'streaming'` (median 222ms
  after). **Cost recorded there:** claudemon coalesces a streaming reply into one
  item that grows in place and `/conversation` answers with items, so every poll
  re-sends the whole in-progress message — 46 fetches / 62 KB for a 2.6 KB reply,
  quadratic in reply length. Getting off that curve means forwarding
  `/conversation/stream` deltas over the bus behind a per-open-pane
  subscribe/unsubscribe.
- **Brain-published snapshots carry NO conversation, deliberately.**
  `services/hub/cmd/brain/enrich.go`'s `compatSnapshot` overlays desktop field
  names and stamps `sparse: true`, and omits `conversation` entirely —
  `services/hub/cmd/brain/parity_test.go` records why ("folding it into every snapshot/publish
  would ship whole transcripts per state tick"). Every client is therefore
  responsible for calling `sessions.conversation` itself. Until 2026-08-26 the
  web backend did neither for its own sessions, so /app against a serve node
  rendered an empty chat AND an immortal "Sending…" bubble (ClaudePane retires
  optimistic bubbles by watching `conversation` grow a user turn); `agents.sendMessage`
  itself acked in ~2ms. Note a node attached with `brain --hub` is a capability
  PROVIDER, not a federated peer, so its rows carry no `hub` stamp and the
  federation path never covered them. Fixed by `backend/busConversation.ts` +
  webBackend wiring (104577c0). **Second gotcha, in claudemon:** a streaming
  assistant reply coalesces into one item that grows in place while `seq` races
  ahead of it, and `?since=` skips by item INDEX (`items_skip` in
  `daemon/api.rs`) — polling with "the seq you were last told" returns nothing
  forever. Anchor on (index of the newest item you hold − 1).
- **Model the SPARSE row when writing a client or a fixture.** The e2e app/mobile
  fixtures answer RICH desktop-shaped rows, so the suite structurally cannot see
  this class of bug. `FIXTURE_SESSIONS`' shape is not what a headless fleet publishes.
- **`compatSnapshot` does not overlay `statusLine` or `totalToolCalls` to
  camelCase.** It maps `sessionId`/`ambientState`/`usage`/`pendingApproval`/
  `pendingQuestions`/`lastActivity`, but for a session with no desktop attached
  `s.statusLine` and `s.totalToolCalls` are undefined and the raw claudemon
  fields ride along unrenamed as `s.status_line` (with snake_case internals like
  `received_at`, `total_output_tokens`) and `s.tool_calls`. `mobile.html`'s stall
  detector reads both spellings as an explicit fallback for exactly this reason.
  Any new feature keyed on those camelCase names needs the same fallback, or
  `enrich.go`'s overlay list (plus `snapshotFieldsRequired` in `parity_test.go`)
  needs widening.
- **The fold must be COPY-ON-WRITE — React.memo eats in-place turn mutation.**
  `ClaudePane` memoizes on the conversation ARRAY's identity and
  `ConversationMessage` is `React.memo`'d on the TURN object. The desktop never
  notices because every snapshot crosses Electron IPC and arrives as a fresh
  structured clone; the web seam hands the renderer the very objects it holds, so
  an in-place `last.content += fragment` is invisible. Measured live (real serve
  node + real /app bundle in headless Chromium, daemon-truth vs a 33ms DOM
  sampler): the transcript DOM froze on the FIRST fragment for an entire
  18-second turn while the fold state underneath was perfectly current — the
  state-edge poke's `sinceSeq` kept marching, proving the data plane was fine and
  only rendering was dead. **Content-level assertions in unit tests all pass while
  the real UI freezes**; only a DOM-clock measurement or an identity assertion
  catches it. A changed fold must replace the array AND exactly the changed turn
  objects (untouched turns keep their memo identity — that is the perf point).
  Pinned by "gives a changed fold fresh array and turn identities" in
  `tests/backend/busConversation.test.ts`.
- **`runId === null` is the Codex-vs-Claude discriminator for subagent drill-in,
  and it is re-implemented in FOUR places that must agree.**
  `workflowAgentTranscript`/`workflowAgentConversation` kept their old
  `(sessionId, runId, agentId)` signature and `runId === null` was quietly
  overloaded into the routing switch: `main/ipc.ts` (watcher first, provider
  fallback only if the watcher returned falsy AND `runId === null`);
  `backend/webBackend.ts` (provider read returns null unless `runId === null`);
  `backend/bridgedBackend.ts` (`runId !== null` → preload IPC; `runId === null` →
  bus first, then IPC); `panes/AgentWatchPane.tsx` (hardcodes `null` for
  `kind: 'subagent'`). `WorkflowTimeline.tsx`'s `txRunId` is the ONLY caller that
  ever passes non-null. **The fallback ordering differs by backend on purpose** —
  bridged tries the BUS before IPC specifically so desktop dev exercises the same
  path web/remote uses; main's ipc.ts is reversed. Do not "harmonize" them. If
  drill-in ever needs a third source, add an explicit discriminator rather than
  overloading `runId` further, and update all four sites plus
  `backendParity.test.ts` (both methods are now `BUS_BACKED`, not `KNOWN_STUBS`).
- **Codex subagent drill-in has TWO item-folders and TWO byte-identical text
  formatters that can drift apart**, none of which reference each other: two
  `applyConversationItems` (`main/services/sessionStore/conversationApplier.ts`
  and `renderer/src/backend/busConversation.ts` — separate implementations, same
  name), two one-shot folders on top of them (`foldItemsToConversation` vs
  `foldConversationItemsToTurns`), and `rawText` (main) vs `transcriptLineText`
  (webBackend) — same `⚙ `/`↳ ` markers, same `slice(0, 400)`, verified identical
  modulo the function name. Change one and the same subagent reads differently in
  the desktop than in the browser. **The main-side folder is the trap:** it
  fabricates a throwaway session object and casts it
  (`applyConversationItems(temp as Parameters<typeof applyConversationItems>[0], …)`),
  so adding a REQUIRED field to the applier's session type will NOT produce a type
  error here — it will produce `undefined` at runtime. The renderer side uses the
  real `newConversationState()` and has no such problem. Also web-path-only:
  rollout items carry no timestamp, so webBackend stamps every folded turn with
  one shared `Date.now()` fallback — turn ordering comes from item order, and a
  consumer that sorts by timestamp gets an arbitrary order. If either formatter is
  edited substantively, promote it into `main/shared/` (the existing home for
  cross-process twins like `mergeConversationWindow`) rather than keeping a third copy.
- **A method can be partly bus-backed and still need a bridged IPC override.**
  `workflowAgentTranscript`/`workflowAgentConversation` are bus-backed in
  `createWebBackend` for provider-native subagent rows, but Claude workflow-run
  drill-in still depends on local `workflowWatcher` artifact files. Leaving them
  purely on the web backend would make runId-backed workflow rows return null in
  bridged desktop mode even though preload IPC can read them. For mixed-transport
  methods: keep the web implementation for browser/remote, and add an explicit
  `createBridgedBackend` override routing the local-only branch to preload IPC.
- **Any session field the composer pills read must be answered on BOTH snapshot
  producers.** `ComposerControls` reads the permission mode from exactly one
  place — `snapshot.livePermissionMode ?? snapshot.settings.permissionMode` — and
  on the web/bus path neither existed for a headless-node session: brain sparse
  rows carried no `settings` block at all, and `webBackend.spawnClaude` discarded
  everything in the `agents.spawn` result except `sessionId`. `permissionModeLabel(provider, undefined)`
  then returned `caps.permissionModes[0].label` — 'Ask to approve' — so a remote
  FULL-ACCESS session displayed as the most cautious mode there is. (The
  `escalationScrubbed` field the hub stamps on every spawn result was read by
  NOTHING in any client.) Fixed 2026-08-26: brain `metaStore.noteLaunch` records
  the launch truth and `enrichSnapshot` fills
  `settings.permissionMode`/`settings.bypassAvailable`/`escalationScrubbed` onto
  every row; webBackend's snapshot fold gained `noteLaunch` so the spawn result's
  `fullAccess` covers the pre-first-snapshot window; `permissionModeLabel` now
  returns 'Unknown' for an absent id instead of the provider default.
  **The spawn RESULT reaches only the caller, so it is never a substitute for the
  row.** `launchPermissionMode` is now a THREE-way twin (renderer
  `lib/providerCaps.ts`, `services/hub/cmd/brain/handlers.go`, `main/services/claudeSpawn.ts` +
  `managedSpawn.ts`) — keep them in agreement or the same launch labels
  differently on a desktop row vs a headless one.
- **`<webview>` in a plain Chromium degrades to a full-size blank BLOCK, not an
  empty inline box.** Measured against a scratch hub serving /app: an Electron
  `<webview>` is an `HTMLUnknownElement` — no shadow root, empty innerHTML,
  `loadURL`/`canGoBack` undefined, `addEventListener` present and harmless. Its
  computed `display` is `block`, and `BrowserPane` gives it `flex:1; width:100%`
  in a flex column, so it takes a real box (measured 1104x802). The failure mode
  is a large blank rectangle under a fully live toolbar whose every button stays
  clickable and inert — which reads as "the app is broken", not "this feature
  needs the desktop". Zero page errors, so no error boundary or crash telemetry
  will ever surface it. `tests/guestFrameFallback.test.tsx` is the regression net;
  `backendParity.test.ts` structurally cannot catch this class (it checks
  electronAPI METHODS, not the DOM). **Any new Electron-only ELEMENT needs a
  DOM-level test the same way.**
