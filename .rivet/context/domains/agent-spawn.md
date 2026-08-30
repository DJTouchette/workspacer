---
title: Agent Spawn (two transports)
tags: [spawn, agents, providers, ipc, hub-bus, permissions, tool-tiers, federation, prompt, starter]
related_paths:
  - "apps/desktop/src/main/services/managedSpawn.ts"
  - "apps/desktop/src/main/services/claudeSpawn.ts"
  - "apps/desktop/src/main/ipc.ts"
  - "apps/desktop/src/main/services/hubCapabilities.ts"
  - "services/hub/cmd/brain/handlers.go"
owner: Damien Touchette
last_reviewed: 2026-08-16
---

# Agent Spawn (two transports)

## Overview

Agents spawn via two independent transports: Electron IPC (`claude:spawn`) and hub event bus (`agents.spawn`). Both dispatch to the same shared helpers on the desktop (TypeScript), but the hub (Go) maintains a parallel independent implementation. The core rule: provider dispatch, permission defaults, and MCP/profile handling must stay identical across both paths, or a spawn started via web/remote/MCP will silently diverge from one started locally (e.g., Codex agent arriving as Claude, chosen MCP servers dropped, permission mode mismatch).

## Key modules

- `apps/desktop/src/main/ipc.ts` — IPC handler for `claude:spawn` (~line 160). Routes to either `spawnClaudeAgent` (PTY) or `spawnManagedAgent` (stream/Codex/OpenCode/Pi) after resolving transport default.
- `apps/desktop/src/main/services/managedSpawn.ts` — Tier-2 managed-provider dispatch (Codex/OpenCode/Pi/Claude-stream). Shared by both IPC and hub transports so they can't drift. Pre-registers session metadata + MCP config before daemon sees it.
- `apps/desktop/src/main/services/claudeSpawn.ts` — Tier-1 Claude PTY spawn. Shared by both IPC and hub. Mirrors profile/MCP/permission resolution exactly so a spawn respawned from web arrives identical.
- `apps/desktop/src/main/services/hubCapabilities.ts` — Hub capability `agents.spawn` (~line 214). Security gate strips permission bypasses for remote callers, then delegates to the same `spawnManagedAgent` / `spawnClaudeAgent` helpers used by IPC.
- `services/hub/cmd/brain/handlers.go` — Headless hub daemon brain. `spawn()` (~line 347) and `spawnManagedSession()` (~line 446) are independent Go rewrites of the desktop logic. Must stay in sync: same provider routing, permission defaults, profile env/argv, metadata recording.

## Failure modes

- **Provider mismatch** — If hub's `spawn()` or `spawnManagedSession()` lags behind desktop's routing logic (e.g., a new provider added), remote spawn silently falls through to Claude (the default).
- **MCP library loss** — Historically, hub's spawn ignored `mcpItemIds`, so remote spawns lost chosen MCP servers; fixed by routing both transports through the same shared helpers.
- **Permission bypass leak** — Remote/web caller passes `skipPermissions=true` or `permissionMode='bypassPermissions'`. Desktop clamps it (line 257 in hubCapabilities.ts, line 360 in handlers.go), but only if both sides recognize the rule. Missing the clamp on one side = auto-approval remotely.
- **Profile/env duplication** — Claude profile (CLAUDE_CONFIG_DIR + extraArgs) must ride the spawn payload and be applied by both paths. If one path skips the profile, spawned Claude lacks its custom config.

## Gotchas

- **Two-implementation synchronization hazard**: Desktop TS (managedSpawn/claudeSpawn) and hub Go (handlers.go) are separate implementations, not shared code. A spawn bugfix or provider addition MUST be applied to both. No shared integration test catches divergence.
- **Bypass stripping scope**: Permission bypass clamping happens at the *hub-capability* boundary (hubCapabilities.ts line 257, handlers.go line 360), BEFORE the shared helper is called. If a new spawn path bypasses this clamp (e.g., calling spawnManagedAgent directly from somewhere else), that path will honor the bypass.
- **Session metadata pre-registration**: Both paths call `claudeSessionStore.setSpawnMeta()` or `r.meta.set()` BEFORE the daemon session starts. If this step is skipped, the card never picks up label/parent/supervisor until a delta arrives; the card sits in "connecting" state or shows no metadata.
- **Transport default resolution**: `opts.transport ?? config.claude.transport ?? 'pty'` is the tiebreaker. Both paths must apply this same priority (IPC line 218, hub line 390). Missing it = stream spawn may silently fall back to PTY.
- **Exhaustive `provider` cases**: Managed vs. PTY dispatch must be exhaustive. Desktop (ipc.ts ~line 190) checks `provider !== 'claude'` for managed, then resolves transport. Hub mirrors this (handlers.go ~line 381). Adding a new provider without updating both is a silent no-op.
- **Default permission modes differ by family**: Claude defaults to `'default'` (PTY or stream); managed providers default to `'ask'`. The distinction lives in managedSpawn.ts lines 143–150 (TS) and handlers.go lines 482–490 (Go). Swapping or omitting this mapping breaks approval flow.

## Hand-authored notes (2026-08-16) — tool tiers, plugin tools, targetHub, profile scrub

- **New spawn options `toolScope: 'view'|'triage'|'operator'` and `pluginTools: string[]`** ride both transports (ClaudeSpawnOptions/ManagedSpawnOptions, `ipc.ts` `claude:spawn`, `hubCapabilities.ts` `agents.spawn` ~L263/355/383) plus the facade's own `spawn_agent`. When set, the desktop mints a per-session scoped facade token (`remoteTokens.ts` `mintSessionFacadeToken`, label `session:<id>`, revoked at store eviction, swept at boot, hidden from the pairing UI) and writes it into `session-mcp/<id>.json` (0600, Authorization header for claude; `?t=` URL for codex/opencode; pi gets nothing). Spawn-param sync is now a FOUR-place concern: desktop TS helpers, hub-capability boundary, brain Go, and the facade's `spawn_agent` — the brain deliberately **declines** `toolScope`/`pluginTools` (documented in `services/hub/cmd/brain/parity_test.go` `spawnParamsDeclined`: headless cannot mint the token). See `modules/mcp-tool-facade.md`.
- **Spawn dialog**: Advanced grew a "workspacer" tier select + per-plugin tool pills (fed by `listHubPlugins` manifests' `tools`), with a deviations chip; `toolScope`/`pluginTools` persist on `AgentWorkspace` and re-apply on respawn. Library-MCP selection is dropped (with a visible hint) when a tier is set. The supervise skill now spawns summarizer workers with `toolScope: "view"` (`supervisorSkill.ts` ~L73).
- **`targetHub` routes a spawn to a federated peer**: Machine picker in the spawn dialog → `useAgentManager` → `ipc.ts` calls `hub:<peer>/agents.spawn` (pinned by `ipcFederationRouting.test.ts`); worktree creation is skipped when `targetHub` is set (the worktree would be on the wrong machine). The workspace records `hub` so respawn re-routes. See `modules/hub-federation.md`.
- **Profile-based permission bypass is scrubbed in the helpers, not just the boundary** (security fix 2026-07-30): `agents.spawn` clamped `skipPermissions`/`permissionMode` on the request but passed `profileId` through, and a bus caller could create a profile whose `extraArgs` carry `--dangerously-skip-permissions` (`claude.profiles.add` is itself a capability). The Go brain already scrubbed (`profiles.go` `scrubBypassArgs`); the desktop now ports it as `scrubBypassArgs`/`scrubBypassProfile` in `claudeProfiles.ts` (tested by `scrubBypass.test.ts`), applied via the `scrubProfileBypass` option honored by BOTH `claudeSpawn.ts` and `managedSpawn.ts` — the boundary decides, the helper enforces, so a future spawn entry point can't forget.

## Hand-authored notes (2026-08-22/23) — full-access mechanics, cwd validation, worktree deps

- **Supervisor full-access is spawn-time-mechanical; manager/fleet full-access is dispatch-time-mechanical — different mechanisms for a similar-sounding flag.** `claudeSpawn.ts`/`managedSpawn.ts` both compute `skipPermissions = !!opts.skipPermissions || supervisorFullAccess`, where `supervisorFullAccess` reads `config.supervisor.fullAccess` live — this mechanically forces `skipPermissions`/`bypassPermissions` on *every* spawn when the flag is set, not just when a caller passes it explicitly. The Fleet Manager's equivalent (`agents.fleetFullAccess` / per-project yolo) is NOT applied to the manager's own spawn opts at all — it rides as a grant on the manager's session facade TOKEN (`mintSessionFacadeToken` via `managerFullAccessFromConfig()`), applied live by the MCP facade when the manager DISPATCHES a worker, re-read per request via `reconcileSessionFacadeGrants`. Grepping `claudeSpawn`/`managedSpawn` alone for "does fleetFullAccess mechanically add skipPermissions" only finds the supervisor path and wrongly concludes the manager path is unwired — it's wired one layer up, at dispatch time.
- **A spawn `cwd` that fails to `chdir` produces a live-looking, already-dead card, not a spawn error.** `POST /sessions/spawn-managed` registers the session id and answers 200 BEFORE the provider child actually launches (`providers/claude_stream.rs` `spawn_session` runs `run_session` in a background task and only warn-logs on failure, then flips the row to Stopped) — so a bad cwd mints a normal agent card, and every subsequent message gets `409 session has ended and cannot accept chat input`. A leading `~` is the realistic trigger: `main/lib/spawnCwd.ts` `normalizeSpawnCwd` deliberately does NOT expand it (twin of the brain's `normalizeCwd`, pinned by `contracts/path-containment-cases.json`) — correct for bus-supplied paths, but `agents.fleetRoot` is a free-text Settings field where a person can literally type `~/`. Two-layer fix: (1) tilde expansion belongs where the USER's config is read, not at the spawn boundary — `fleetManager.ts` `expandHome` inside `deriveFleetRoot`, leaving `normalizeSpawnCwd`'s contract untouched; (2) `assertSpawnCwd` (`main/lib/spawnCwd.ts`) pre-flights the resolved cwd in `spawnClaudeAgent`/`spawnManagedAgent`/`spawnCodexHybrid`, raising a `notifySystem` banner and throwing before a session id ever exists. Any new spawn entry point must call it after cwd resolution. Remaining gap: claudemon itself still accepts an unusable cwd for non-desktop clients — a 400 in `handle_managed`/`handle` would be the backstop.
- **Worktree `node_modules` auto-linking now recurses to any depth** (`worktreeService.ts` `discoverNodeModules`), fixed from a depth-≤2 cap that missed anything nested deeper than `<a>/<b>/node_modules` — this repo's own `apps/desktop/src/renderer/node_modules` is 4 segments down and was silently skipped, forcing a hand-symlink in every fresh agent worktree. The walk still never descends into a `node_modules` directory itself (a nested dependency's own copy isn't walked) or into dot-directories, so cost stays bounded (~3900 dirs / ~17ms on this repo). Prefer widening this kind of depth handling in code over adding a per-project `worktreeSetup` symlink command in config — a config-only fix wouldn't help a fresh clone, and this repo's `config.yaml` has a documented history of being clobbered.

## Hand-authored notes (2026-08-23) — the first message rides the spawn

- **A spawn's FIRST MESSAGE is now a spawn PARAMETER (`message`), not a second call.** Every dispatch used to be spawn → wait for the id → `agents.sendMessage`, with a manager turn boundary in between and a live worker holding no instructions in the gap. The param rides all four surfaces — `spawn_agent` (`services/hub/cmd/mcp/main.go` `spawnAgentIn.Message`), `agents.spawn` (`hubCapabilities.ts`), `claude:spawn` (`main/lib/managedSpawnOptions.ts` `AgentSpawnRequest.message` → `firstMessage` on the helper options), and the brain (`services/hub/cmd/brain/handlers.go` `spawnParams.Message`) — down to claudemon's `first_message` on BOTH spawn payloads.
- **The window it closes is measured, not theoretical, and it is asymmetric between the two daemon routes.** `register_managed` (`session/store.rs`) marks a managed row `SessionMode::Input` up front but attaches no PTY wrapper, and `register_managed_input` only runs deep inside the provider's driver task (`claude_stream.rs` registers it AFTER `Command::spawn`, well past the handler's 200). So `submit_message` in that window resolves `managed_input`=None → mode `Input` → `wrapper()`=None → **`MessageOutcome::NoWrapper`, HTTP 404 "no wrapper attached to that session"**. A PTY row is born `Unknown` with its wrapper already registered, so the same post-spawn send is queued and flushed on the first `Input` transition — which is why spawn-then-send *looked* reliable: it is, on the transport people tested it on. The desktop's `kickoffMessage` (Fleet Manager + guide tour) and `/m`'s library-prompt chip both did the racy form, both spawn `transport: 'stream'`, and both only `console.error`/`toast`ed the loss.
- **Delivery: queued inside the spawn handler, before the 200; drained by the driver's own registration.** `SessionStore::queue_first_message` enqueues into the existing `pending_messages` queue, and `register_managed_input` drains it into the provider's prompt channel — ONE shared point every adapter (claude_stream/codex/opencode/pi) already calls, so no per-provider code and no window. PTY reuses the cold-start ladder verbatim (settle past the composer redraw, then submit-verify). If the child never launches, the row goes `Stopped` and `clear_pending_messages` drops it with the session — a visible stopped card, not a live-looking one holding a lost prompt.
- **`instructions` is NOT the channel, and it structurally cannot be.** `Facade.instructions` is a passive PREFIX: every adapter parks it in `pending_instructions` and `with_instructions()` prepends it to the first prompt the session receives (`codex.rs`), so it never starts a turn. A dispatch put there would wait forever for the prompt it *is*. Keeping them separate also fixes ordering for free — the facade role note and the `wks-result` contract get prepended to the first message, once, in one turn. (PTY has no `instructions` at all; its contract is `--append-system-prompt`, a system prompt, while the dispatch is a user turn.)
- **No silent drop, at two independent layers.** claudemon answers `first_message_queued`; `claudemonSessionClient.deliverFirstMessage` falls back to `POST /message` when unconfirmed and raises a `notifySystem` error banner naming the session if that fails too. `agents.spawn` then answers `messageQueued` — the TRUTH about delivery, not "we passed it on": the client flags a total failure via `takeUndeliveredFirstMessage` (one-shot, so the set cannot grow) and the answer reflects it. The key is omitted entirely when no message was sent, so the result shape is unchanged for every other spawn and the facade's `confirmFirstMessage` (`services/hub/cmd/mcp/main.go`) falls back to `agents.sendMessage` when a provider does not confirm — which is what covers an older federated peer or a lagging brain, where the spawn looks perfectly successful and the prompt went nowhere.
- **The brain MIRRORS this param where it DECLINES `resultSchema`, and the rule behind the difference is "who honors it".** `resultSchema` has two desktop-owned halves (a prompt the desktop compiles, a wake the desktop delivers), so headless would take it and provably never honor it. `message` is claudemon's, and the brain reaches the same two claudemon routes the desktop does — declining it would mean a bus dispatch silently loses its task whenever the desktop is not running.
- **`respawn_with` does not INHERIT a first message, and should not.** It already recovers something strictly better: `firstUserMessage` reads the task out of the original's conversation (what the agent actually received) rather than what a caller asked for at spawn time; inheriting the param would add a staler second copy plus a rule about which wins. It DOES use the field to deliver, which retires its own "spawned but could not deliver the task" branch — and its explicit `sendMessage` had to be REMOVED at the same time, or the successor reads its whole dispatch twice.
- **Census, for the next spawn param.** Declared/forwarded/asserted in: `services/hub/cmd/mcp/main.go` (`spawnAgentIn` + `spawnWithGrants`), `services/hub/cmd/mcp/respawn.go`, `services/hub/cmd/mcp/help.go`, `services/hub/cmd/brain/handlers.go` (`spawnParams` + both legs + `spawnResult`), `services/hub/cmd/brain/claudemon.go` (`spawnReq`/`spawnManagedReq`), `services/hub/cmd/brain/parity_test.go` (`spawnParams`/`spawnParamsDeclined`/`spawnParamsAhead`), `hubCapabilities.ts`, `main/lib/managedSpawnOptions.ts` (the `satisfies Record<keyof AgentSpawnRequest, …>` guard — a new field is a COMPILE error until classified), `main/ipc.ts` (three branches + the federated forward), `main/preload.ts`, `main/services/claudeSpawn.ts`, `main/services/managedSpawn.ts` (managed + the codex PTY hybrid), `main/services/claudemonSessionClient.ts`, `renderer/src/types/electron.d.ts`, `renderer/src/hooks/useAgentManager.ts`, `services/hub/cmd/hub/mobile.html`, and claudemon's `daemon/spawn.rs` (both payloads) + `session/store.rs`. **`renderer/src/types/electron.d.ts` is the THIRD copy of the spawn options and no guard covers it** — the renderer builds its spawn opts as a variable, so excess-property checking does not fire and a field missing there typechecks clean while being invisible to every renderer caller. `webBackend.ts` forwards `opts` wholesale, so the web mirror needs no edit; `apps/tui/src/bus.rs` builds its own param map and would need one if the TUI ever grows a first-message field.
- **Not an authorization surface, and the tier table is the proof.** `agents.sendMessage` is a TRIAGE method (`authtoken.go` `triageMethods`) while `agents.spawn` is operator-only and named in triage's own doc comment as deliberately absent — so every caller that can spawn already holds the right to send this exact text to the session it just created. `sanitizeSpawnParams` (`services/hub/internal/bus/rpc.go`) passes unknown params through untouched, which is correct here: there is nothing to strip because there is no privilege to strip it from.

## Hand-authored notes (2026-08-24/27) — per-harness models, codex transport, and what a template may carry

- **Every model-holding config key predates multi-provider and ships a CLAUDE
  id.** `claude.defaultModel` ('opus[1m]'), `supervisor.model`,
  `supervisor.summarizerModel` ('sonnet') and `agents.autoTitle.model` ('haiku')
  are all Claude ids for historical reasons, and only `claude.defaultModel` was
  ever gated on provider (`main/lib/spawnModel.ts`, and the Go facade's
  `providerIsClaude`). The other three were read unconditionally on paths that
  can spawn codex. **A foreign model id on a spawn is a 400 at best and a
  silently-wrong model at worst.** Fixed 2026-08-27 by giving each a per-harness
  sibling map (`supervisor.models`, `supervisor.summarizerModels`,
  `agents.autoTitle.models`, `agents.managerModels`) resolved through
  `main/lib/roleModels.ts`, with `main/shared/modelVocabulary.ts` as the
  cross-harness oracle. **Any NEW config key holding a model must be born
  per-harness** — a single field is only ever right for one harness. Resolve
  through `roleModels` `perHarnessModel`; never read a `*.model` config field
  inline in a spawn path. `resolveSpawnModel` is the last-line guard and drops a
  demonstrably-foreign id to `undefined` (= the CLI's own default, the one value
  valid everywhere).
- **The headless Go brain resolves NO supervisor/manager role model at all.**
  `services/hub/cmd/brain/handlers.go` has no equivalent of `lib/supervisorModel` or
  `lib/roleModels` — it only forwards `spawnParams.Model`. So a supervisor or
  Fleet Manager spawned on a headless node ignores
  `supervisor.model`/`supervisor.models`/`agents.managerModels` entirely and gets
  the harness default. **Role model settings are desktop-only.**
  `services/hub/cmd/mcp/main.go` DOES gate `claude.defaultModel` on `providerIsClaude` (pinned
  by `spawndefaults_test.go`), so the claude-id-to-codex leak is closed there — it
  is the ROLE models that are missing, not the provider gate. Porting
  `perHarnessModel` to Go beside `launchPermissionMode` (the existing
  desktop/brain twin pattern) plus a `services/hub/cmd/brain/parity_test.go` entry is the fix;
  deliberately out of scope 2026-08-27.
- **Codex transport now resolves through ONE shared default
  (`config.codex.transport`, shipped `'stream'`).** Codex previously had no config
  transport at all: an absent `transport` key on the spawn-managed payload means
  "hybrid" to claudemon, which is indistinguishable from a dropped field, and four
  entry points each applied their own `?? 'pty'` for claude and nothing for codex.
  Now `main/lib/spawnTransport.ts` `resolveTransport()` runs inside
  `spawnManagedAgent` — **the choke point that IPC, hub bus, respawn, jobs and
  supervisor/manager all reach** — with the Go twin `registry.transportDefault` in
  `services/hub/cmd/brain/handlers.go`. Both codex shapes are now STATED on the wire and in
  `setSpawnMeta`, including `'pty'` on the Windows rollout hybrid.
  Two traps: (1) **forwarding only `transport === 'stream'` at a
  request-translation layer now SILENTLY OVERRIDES an explicit hybrid request**,
  because an omitted key is re-resolved downstream to the configured default —
  forward both values or forward neither; (2) `spawnManagedAgent` gates the
  Windows rollout hybrid on `transport === 'pty'`, not on
  `process.platform === 'win32'`, so headless codex on Windows takes the managed
  app-server path. Its safety net is claudemon-side: `run_session` degrades a
  failed HEADLESS app-server to `run_rollout_fallback` (previously a hard error),
  resetting `store.set_transport(..., Transport::Pty)` so the pane grows its Term
  view, plus a `DEGRADED_FROM_HEADLESS_NOTICE` conversation item.
  When adding a provider with more than one session shape, add it to
  `TRANSPORT_FALLBACK` (TS) and `transportFallback` (Go) and resolve at the choke
  point, not at each caller.
  *To exercise a real codex session end-to-end without the desktop:* run claudemon
  with `env -u WORKSPACER_PARENT_PID` (it otherwise inherits the live app's
  parent-watchdog pid and exits immediately), plus the hub and the brain on spare
  ports with `XDG_CONFIG_HOME` at a temp dir, then call `agents.spawn` over the
  bus with a `{op:'call',id,method,params}` frame.
- **`LibraryHost`'s `initialPrompt` is a SECURITY property: never auto-send text
  that can come off disk.** Library items have a PROJECT scope stored at
  `<cwd>/.workspacer/library/*.md` — per repo, committable — and both LibraryPane
  and CommandPalette render a Dispatch button on every one. What keeps a
  repo-shipped prompt from RUNNING on one click is a single line: `LibraryHost.tsx`'s
  spawn branch passes `initialPrompt` (composer PRE-FILL) rather than
  `kickoffMessage` (AUTO-SEND). Swapping that one field would let a cloned repo
  run a prompt of its choosing with no read step. The app's own auto-send call
  sites (`spawnGuide`, `spawnFleetManager`) are safe for a reason that does not
  transfer: they send text the APP composed in code around a question the USER
  typed. **The rule: auto-send is only for text the app owns in code.** Now pinned
  by a call-site comment, by `LibraryHost`'s Props type (which has no
  `kickoffMessage` field), and by `tests/libraryHostAutoSend.test.tsx`. The same
  rule governs `lib/draftAgent.ts`, whose bus event carries a brief ID rather than
  text so free text cannot enter that path at all.
- **Dispatch templates (`kind: 'dispatch'`) are text-only BY CONSTRUCTION.** A
  template carries template text + an optional default `resultSchema` and NO
  spawn-argument fields, so a template file cannot smuggle
  `toolScope`/`cwd`/`model`/`worktree` — the no-trust-boundary property is pinned
  in `libraryDispatch.test.ts` (TS) and `TestLibraryDispatchRoundTrip` (Go).
  Rendering is host-side in `hubCapabilities` `agents.spawn` via
  `main/lib/dispatchTemplate.ts`, which deliberately does NOT reuse the renderer's
  `applyTemplate`: placeholders are required by default and an unfilled one
  REFUSES the spawn naming the param (`applyTemplate` silently defaults, which is
  right for its form dialog and wrong for dispatch). The headless brain declines
  `template`/`templateParams` in `spawnParamsDeclined` because the default-schema
  half rides the already-declined `resultSchema` machinery.
- **Library seeding is additive per ITEM, gated by `library-seeded.json`.** Both
  seeders used to no-op the moment `<config>/library` held any `.md`, so every
  starter added after a user's first run was invisible to the entire installed
  base (the three dispatch templates from 7317b1c5 landed for nobody). Fixed in
  b8b447e3: `LibraryService.seedGlobalIfEmpty` → `seedGlobalStarters` (starters
  come from a `starters()` array of `{id, item}`) and the Go twin
  `seedLibraryIfEmpty` → `seedLibraryStarters` + `starterItems()`.
  **The non-obvious part is `<config>/library-seeded.json`** (`{"seeded":[id…]}`,
  written and read by BOTH twins, same key): the directory alone cannot tell "you
  deleted this starter" from "you were never offered it", and the seeder must
  never resurrect the first. An id recorded there is never written again — and a
  starter that already exists on disk is recorded but never overwritten, so
  deleting a hand-edited starter afterwards still sticks. Legacy installs
  (non-empty library, no marker) bootstrap from
  `PRE_MARKER_STARTER_IDS`/`preMarkerStarterIDs`, the four starters that predate
  the marker. **That list is FROZEN** — adding to it would silently stop a new
  starter reaching existing users, which is the exact bug this fixed. New starters
  go in `starters()`/`starterItems()` only. Two test consequences:
  `TestLibrarySeedAndList` pins the count against `len(starterItems())` (the TS
  twin's count is still a hand-kept literal in that same test), and any test
  asserting on the WHOLE library list must call `suppressLibrarySeed(t)` — a
  populated-but-unseeded temp dir legitimately gains starters now. Go seeds on
  every `listLibrary` call; TS seeds only in the singleton's constructor.
- **Porting a JS regex/trim to Go: neither `\s` nor `TrimSpace` is JavaScript's
  whitespace.** The dispatch-template placeholder parser
  (`main/lib/dispatchTemplate.ts`) spells its token with JavaScript's `\s` and
  trims with `String.prototype.trim()`. **Neither Go spelling matches:** Go's
  `regexp` `\s` is only `[\t\n\f\r ]` (no `\v` U+000B, no NBSP U+00A0), while
  `strings.TrimSpace`/`unicode.IsSpace` is WIDER in one direction (trims U+0085
  NEL, which JS does not) and NARROWER in another (does not trim U+FEFF BOM,
  which JS does). `services/hub/cmd/brain/dispatchparams.go` therefore writes the ECMAScript
  WhiteSpace ∪ LineTerminator set out by hand (`jsWhitespace`) and uses it for
  both the character class and `strings.Trim`; four cases in
  `contracts/dispatch-template-params-cases.json` pin all four divergent code
  points. A `TrimSpace`-based port would have made the same `library.list` call
  advertise a param name the caller cannot fill, **on one provider only** —
  invisible until a template author pasted a non-breaking space. **When porting
  any JS string parser to Go, write the whitespace class out explicitly and pin
  U+000B, U+00A0, U+0085, U+FEFF in a contract fixture.**
  Two other seam facts from that change: `LIBRARY_KINDS` had to live in
  `main/shared/libraryKinds.ts` rather than `libraryService.ts`, because SIX
  suites do `vi.mock('./libraryService', …)` wholesale and a value imported from
  there is `undefined` under the mock; and
  `hubCapabilitiesKillSwitch.test.ts` extracted the per-file guard positionally as
  `mock.calls[0].at(-1)`, which silently stopped meaning "the guard" the moment
  `library.list` grew a trailing filter argument — it now finds the single
  function-typed argument (`guardArgOf`) and asserts there is exactly one.
