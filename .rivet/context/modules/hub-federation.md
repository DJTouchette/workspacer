---
title: Hub Federation (hub-of-hubs): peer links, stamped events, qualified calls
tags: [hub, go, federation, event-bus, remote, multi-machine, security]
related_paths:
  - "services/hub/internal/federation/federation.go"
  - "services/hub/internal/federation/federation_test.go"
  - "services/hub/internal/busclient/client.go"
  - "services/hub/internal/busclient/subscribe_test.go"
  - "services/hub/internal/bus/rpc.go"
  - "services/hub/cmd/hub/main.go"
  - "apps/desktop/src/main/services/federationBridge.ts"
  - "apps/desktop/src/main/ipcFederationRouting.test.ts"
  - "apps/desktop/src/main/lib/snapshotLiveness.ts"
  - "apps/desktop/src/renderer/src/lib/federation.ts"
  - "apps/desktop/src/renderer/src/components/HubChip.tsx"
  - "apps/desktop/src/renderer/src/backend/webBackend.ts"
  - "apps/tui/src/federation.rs"
  - "services/hub/cmd/hub/mobile.html"
  - "services/hub/scripts/federation-harness.sh"
  - "docs/hub-federation.md"
owner: Damien Touchette
last_reviewed: 2026-08-16
---

# Hub Federation (hub-of-hubs): peer links, stamped events, qualified calls

## Overview
Federation lets the local hub link *outbound* to named peer hubs (typically another machine on the tailnet) and republish their fleet events locally, so every client — desktop, web `/app`, `/m` PWA, wks-tui, the supervisor — sees one merged fleet through the single bus connection it already holds. Peer sessions arrive with the peer's name stamped on the event **envelope** (`Envelope.Hub`; payload never rewritten; absent = local), and peer capabilities become callable as `hub:<peer>/<method>`. Shipped 2026-08-15/16 (commit 1ece854b). **`docs/hub-federation.md` is the authoritative design narrative** — read it first for the reasoning behind every decision (envelope stamping vs id rewriting, unqualified topics, the tree invariant, the curated forward list) and its post-build "Implementation notes" for where reality corrected the proposal. This doc is the module map plus operational gotchas; it does not restate the design.

## Key modules
- `services/hub/internal/busclient/client.go` — the substrate gap that had to close first: `Subscribe(patterns...)` + `OnEvent(fn)` (patterns re-sent on every reconnect, mirroring `hubClient.ts`) and `Publish`. Proven by `services/hub/internal/busclient/subscribe_test.go`, including a same-port hub restart (an `http.Server`'s Shutdown never closes hijacked WebSocket conns — the test severs them itself).
- `services/hub/internal/federation/federation.go` — one `link` goroutine per peer (a `busclient` dialed at the peer's `/bus`): subscribes `ForwardTopics = {agent.*, workflow.*}` (a deliberate allowlist — `*` would let a peer's `layout.changed` clobber the local layout, launder host-only `plugin.*`, and drive this UI via `command.*`), stamps `Hub` on each envelope and republishes locally (broker `ID` cleared — per-broker id spaces), publishes `hub.peer.connected`/`hub.peer.disconnected`, and forwards qualified calls with a 25s budget (vs the router's 30s, so the inner failure is the one seen). A hub-stamped event is never re-forwarded: loops are unrepresentable (tree invariant), not solved.
- Peers file — `~/.config/workspacer/peers.json` (0600, `DefaultPeersPath()` = `authtoken.ConfigDir()`): JSON array of `{name,url,token}`. Deliberately NOT flags (token on argv is `/proc`-world-readable) and NOT config.yaml (credential-free by design — that's what keeps `config.get` unguarded). `-peer` flag form remains for tests/dev. The token is a scoped token minted ON the peer (`workspacer token create`) and is the ceiling on everything forwarded.
- `services/hub/internal/bus/rpc.go` + `services/hub/cmd/hub/main.go` — `srv.SetFederation(fed)` wires qualified routing: on `call`, a `hub:<peer>/` prefix is stripped for the scoped-token tier check (bare method against the existing allowlists — never relaxed to globs), plugin tokens are refused federated calls outright (a consented "this machine" grant must not silently extend to peers), and local argument/path confinement (`authorize`) is skipped — paths name the PEER's filesystem; the peer's own capspec plus the link token enforce there. `federation.peers` is a hub-local method (name + connected + lastSeen only) admitted to the **view** tier (`authtoken.go` ~L87).
- `apps/desktop/src/main/services/federationBridge.ts` — desktop main-process integration: ingests hub-stamped `agent.snapshot`s into the session store (snapshot gains a `hub` field; stores are NOT re-keyed by `(hub,id)` — session ids are UUIDs, collision accepted), seeds each peer's fleet by calling `hub:<peer>/sessions.snapshots` on `hub.peer.connected` (mandatory — the call plane doesn't federate for free; events alone leave restart-blindness), marks tombstones on disconnect, and routes actions (approve/reply/interrupt) to the owning hub. `snapshotGrantsFsRoot` (`apps/desktop/src/main/lib/snapshotLiveness.ts`) fail-closed refuses any hub-stamped snapshot — a remote cwd grants NO local fs root. `keepWarmService.ts` and seen-models recording skip remote sessions (the rate-limit window belongs to the peer). `federation:peers` IPC (`ipcChannels.ts` `FEDERATION_PEERS`) feeds the renderer.
- Renderer — `apps/desktop/src/renderer/src/lib/federation.ts` (cwd-bound pane gating: terminal/git-review/editor hidden for remote sessions), `HubChip.tsx` (hub badge + offline/tombstone form), spawn dialog Machine picker → `targetHub` → `useAgentManager` → `ipc.ts` routes `hub:<peer>/agents.spawn` (worktree disabled when a targetHub is set); pinned by `apps/desktop/src/main/ipcFederationRouting.test.ts` + `renderer/tests/federation.test.ts`.
- `apps/desktop/src/renderer/src/backend/webBackend.ts` — web `/app` parity without the main process: its own `sessionHub` map fed by envelope `hub` stamps and a peer-fleet seed (`federation.peers` + `hub:<peer>/sessions.snapshots`), `qualify(sessionId, method)` for every per-session call, tombstones on `hub.peer.disconnected`.
- `services/hub/cmd/hub/mobile.html` — `/m` is federation-aware the same way (own `sessionHub` + `qualify()`, including `sessions.conversation` polling); peer cards go read-only tombstones while a hub is down. The hub's push watcher covers both machines through the one hub the phone pairs with.
- `apps/tui/src/federation.rs` — `RemoteFleet`: per-hub session store merged into the agent list, fed by hub-stamped `agent.snapshot`s, seeded/re-seeded via `federation.peers` + `hub:<peer>/sessions.snapshots`; includes the camelCase hub-snapshot-row → snake_case `Agent` adapter. `Driver` (in `apps/tui/src/bus.rs`) qualifies per-session verbs by hub; remote question answers go via `agents.sendMessage`; local-only ops (PTY term, git) toast instead of failing. Remember the TUI is bus-first with `--direct` fallback — federation only exists in bus mode.
- `services/hub/scripts/federation-harness.sh` — the fake second PC: a peer hub on :8895 republishing synthetic agents every 2s; prints the peers.json line to point a real hub at it.
- `/remote` (`remote.html`) — deliberately still single-hub/local-only.

## Failure modes
- **Peer down → tombstones, never disappearance.** Cards persist read-only ("hub unreachable, last seen …") on desktop, web, /m, and TUI; restored links reseed wholesale. Silent vanish reads as "my agent died" — the worst failure mode, so every client implements this.
- **Unknown peer prefix in a call is refused**, not treated as a literal method name; an unbacked method on a live peer fails with the peer's own "no provider" error (v1 does lazy discovery — the `hello` frame's `methods` is the caller's grant ceiling, not a provider catalog, so it can't drive discovery).
- **A topic outside `ForwardTopics` never crosses**, silently. If a new fleet-relevant topic is added (e.g. a new `agent.*` sibling namespace), it must be deliberately added to the allowlist or remote clients simply never see it.
- **Rejected peer token** = unauthorized handshake retried with backoff forever; `federation.peers` shows `connected:false` but there is no distinct "bad token" surface yet (open question 4 in the design doc).

## Gotchas
- **`WORKSPACER_PARENT_PID` kills manually launched hubs.** A hub started from inside a workspacer session inherits the env var and self-exits when that parent dies (parentwatch). Run the harness — and any scratch hub — with it unset.
- **7895 is the live desktop hub's port.** Never bind a scratch/test hub on 7895 while the desktop app runs (the desktop also port-kills on boot); the harness uses :8895 for the peer.
- **Headless-brain-only peers are visible everywhere (since 2026-08-16).** A peer running only `workspacer serve` (brain, no desktop) emits *sparse* snapshots; the desktop (`federationBridge.ts` explicit minimal mapping — never a spread, so snake_case originals and the `sparse` marker never leak into renderer snapshots) and the TUI (`federation.rs fold_row` — /m-style overlay: sparse refreshes state without clobbering rich enrichment; rich replaces wholesale) both accept them now, matching `/m`. Sparse cards render state/attention/usage/plan with working approve/reply; brain sparse `pendingApproval` carries NO timestamp — the desktop stamps once and preserves it across re-sends (dismissal logic keys on it), a rule any future sparse consumer must copy.
- **Desktop peer discovery is now proactive (race found + fixed 2026-08-17).** `federationBridge.ts` used to seed solely on a `hub.peer.connected` event or implicit discovery from a hub-stamped `agent.*` event — but saving peers.json RESTARTS the hub, severing main's bus socket, and the new hub's peer link usually publishes its (transition-only) connected event before `hubClient.ts` reconnects; with an idle peer nothing ever triggered discovery. Fixed like every other client: the bridge calls `federation.peers` on start and on every bus (re)connect (`subscribeHubConnected` hook in `hubClient.ts`), seeding connected peers and tombstoning believed-up ones the hub reports down (missed disconnects). Pinned by `federationBridge.test.ts` "proactive discovery".
- **Remote pane attach/gate are special-cased, not refused (bug found + fixed 2026-08-17).** ClaudePane derives `sessionId` from `useClaudeSpawn`, whose attach mode calls the `CLAUDE_ATTACH` IPC — and until the fix that handler's `assertLocalSession('Terminal attach')` THREW for any store row carrying `hub` (guard shipped in 1ece854b itself), landing as `spawnError`: `sessionId` stayed null, `useClaudeSession(null)` never yielded a snapshot, so remote panes could neither read nor send while sidebar cards (fed straight from the store) looked fine — the e3e4c748 conversation-fold path was unreachable from the very pane it was built for. Now `CLAUDE_ATTACH` adopts a remote id as a stream-less GUI-only viewer (returns the sessionId, wires no port — exactly `webBackend.ts`'s no-op attach) and `CLAUDE_GATE` silently no-ops (the peer's own client gates its claudemon); the OTHER local-only ops (resize/close/mode/model/effort/handoff) still refuse loudly. ClaudePane forces `hasTerminal=false` when `session.hub` is set, and the local-git pollers branch on remoteness (`SessionStatusBar` on `snapshot.hub`, `ConversationEmptyState` on its `hub` prop) so a peer's foreign cwd never hits local git. Pinned by `ipcFederationRouting.test.ts` "pane housekeeping". Rule of thumb from the fix: automatic pane housekeeping adopts/no-ops for remote; user-initiated local-only actions refuse loudly.
- **Two `lastSeen` encodings.** `federation.peers` returns unix **milliseconds** (`PeerInfo.LastSeen`, 0 = never); the `hub.peer.disconnected` event carries **RFC3339** (`federation.go` ~L245). Clients consuming both must not mix the parsers.
- **Desktop remote chat fetches the FULL conversation (since 2026-08-16)**: `federation:conversation` IPC (handler in `federationBridge.ts`, single-flight, main-side seq authoritative for `sinceSeq`, backward-seq → full rebuild) folds peer items through `applyConversationItems` into the store, after which bounded snapshot windows stop overwriting `conversation`. Gotcha inherited from the fix: `conversationOffset`/`conversationUserOffset` are undeclared on main's session state — spreading a wire window onto a store row silently adopts the WINDOW's anchors and mis-anchors full-history sessions; carry the store's own anchors forward. `/m` and the TUI fetch via qualified `sessions.conversation` directly. Tombstones pause polling at both layers; reconnect resumes incrementally.
- **Remote sessions must contribute nothing local**: no fs roots (`snapshotGrantsFsRoot`), no keep-warm, no seen-models, no cwd-bound panes. When adding any feature that reads a session's `cwd`, opens files, or runs git, branch on `snapshot.hub` or it becomes a cross-machine confusion (or security) bug — this is design-doc Phase 5 and it never ends.

## 2026-08-16 additions (same-day polish pass)
- **Peers pairing UI**: Remote Control dialog "Linked machines" (`apps/desktop/src/renderer/src/components/LinkedMachinesSection.tsx` + `apps/desktop/src/main/services/federationPeersConfig.ts`) — validated atomic peers.json writes (validation mirrors `LoadPeersFile` wording; keep-token merge: `token===undefined` keeps, `''` clears; tokens redacted to `hasToken` on read), hub restart on save (skipped in remote-client mode; an ADOPTED external hub won't pick the change up until its owner restarts it), "Link this machine" panel minting scoped tokens via the existing remote-token IPC. Web mirror gets read-only (`federationPeersConfig()` → null).
- **Cross-hub MCP facade** (`services/hub/cmd/mcp/federated.go`): `list_agents`/`list_snapshots` merge connected peers (10s per-peer budget, per-peer failures cost only their rows), remote rows tagged `hub`; per-session tools + `spawn_agent` take an optional `hub` param (embedded `hubArg` struct — the go-sdk flattens embedded structs into the schema; field stripped before forwarding). Supervisor prompt/skill/help teach the hub round-trip; `fleet.mjs` stays local-claudemon-only.
- **Untokened dial**: facade `-untokened operator|view|deny` (env `WKS_MCP_UNTOKENED`, desktop `facade.untokenedAccess` passthrough); `deny` alone satisfies the non-loopback bind policy, `view` doesn't; invalid value fails startup.
- **Harness answers calls now**: `federation-harness.sh` registers agents.list/sessions.snapshots/sessions.snapshot/sessions.conversation (sinceSeq honored)/agents.sendMessage/claude.approve/claude.signal on the fake peer, so qualified calls, remote chat, approvals, and the cross-hub facade are all exercisable live; `SPARSE=1` emulates a headless-brain peer.

## Hand-authored notes (2026-08-24) — Go's Windows file mode is SYNTHETIC, so a `0o077` check carries zero information

`os.FileInfo.Mode().Perm()` on Windows is synthesised from ONE attribute bit
(`os/types_windows.go`: `if FILE_ATTRIBUTE_READONLY { m |= 0444 } else { m |=
0666 }`), so `Perm()` is 0666 for every writable file and 0444 for every
read-only one — **`Perm()&0o077 != 0` is TRUE in both cases, for every file that
exists.** `os.Chmod` is the same story inverted: on Windows it only toggles
`FILE_ATTRIBUTE_READONLY`, so `os.Chmod(p, 0o644)` grants no principal anything.

This made `services/hub/internal/nodes.FileLooksExposed` (the on-disk guard for the Fly API
token in `nodes.json`) report "readable beyond its owner" on every Windows start
regardless of the file's real ACL, and CI caught it as a red
`TestFileLooksExposedNoticesLoosePermissions` on the containment-windows job.
**The test was a true positive; the check was the broken half.** A permission
check written the Unix way is not merely imprecise on Windows — it carries zero
information, and a warning that fires unconditionally is worse than no warning
because the user learns to scroll past it.

Two situations, two answers, and the repo now has a precedent for each:

1. **Asserting the mode WE WROTE** (a write we control) → skip on Windows:
   `if onWindows { t.Skip("POSIX mode bits") }`
   (`services/hub/cmd/brain/unpinnedguards_test.go`) or
   `runtime.GOOS != "windows" &&` (`services/hub/cmd/hub/upload_test.go`).
2. **A shipped check that renders a VERDICT about who can read a file** → do NOT
   skip; the skip leaves a live check that lies. Use `services/hub/internal/nodes.FileExposure`
   (`exposure.go` + `exposure_unix.go` + `exposure_windows.go`): three-valued
   (`ExposureLoose`/`ExposureOwnerOnly`/`ExposureUnknown`, **zero value Unknown so
   a forgotten case never reads as safe**), with a real DACL walk on Windows via
   `golang.org/x/sys/windows` (already a direct dep) —
   `GetNamedSecurityInfo` → `DACL()` → `GetAce`, read grant to a well-known
   everyone-ish SID = loose, **NULL DACL = loose** (it grants EVERYONE full
   control), domain group / unparseable ACE = unknown.

**The same trap is waiting for `peers.json`, `tokens.json`, `jobs.json` and
`remote-token`**, all of which are 0600 BY CONVENTION with no check at all today.
Lift `FileExposure` into a shared place if a second 0600 credential file ever
grows a check.
