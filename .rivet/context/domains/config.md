---
title: Configuration (two writers)
tags: [config, settings, defaults, concurrency, mtime-gate, config-lock, wholesale]
related_paths:
  - "apps/desktop/src/main/services/configService.ts"
  - "services/hub/cmd/brain/config.go"
  - "services/hub/cmd/brain/config_defaults.json"
  - "apps/desktop/src/main/services/configDefaults.generated.ts"
  - "apps/desktop/scripts/gen-config-defaults.mjs"
owner: Damien Touchette
last_reviewed: 2026-08-16
---

# Configuration (two writers)

## Overview

`config.yaml` is written by two independent processes: the Electron main process (TS `configService`) and the headless brain daemon (Go `configService`). Both read and write the same file via mtime-gating to prevent clobbering. The desktop app guards against unreadable configs via a `persistBlocked` fail-safe. Defaults are single-sourced from `config_defaults.json` (embedded in Go, generated to TS at build time) to ensure web/mobile/desktop clients see identical defaults.

## Key modules

- `apps/desktop/src/main/services/configService.ts` — TS singleton with `loadedAtMs` mtime cache, `refreshIfChangedOnDisk()` gate, and `persistBlocked` fail-safe for unreadable configs.
- `services/hub/cmd/brain/config.go` — Go counterpart with mutex-protected `configService`, `loadedAt` mtime gate, `get()` method that re-reads when file changes, and concurrent-safe `save()`.
- `services/hub/cmd/brain/config_defaults.json` — canonical defaults source (go:embed'd by brain, generated to TS via npm script).
- `apps/desktop/scripts/gen-config-defaults.mjs` — builds `configDefaults.generated.ts` (both main + renderer copies) from the JSON; wired into `prebuild:main`.
- `apps/desktop/src/main/services/configService.test.ts` — drift tests verify main-process and renderer generated defaults match the canonical JSON.

## Failure modes

**Read failures**: EACCES, EBUSY, or transient I/O errors set `persistBlocked = true` in TS (Go ignores, returns defaults). TS runs on in-memory defaults and refuses all writes until the file parses on reload.

**Parse failures**: Malformed YAML backs up the broken file (timestamped `.broken-*`), sets `persistBlocked`, runs on in-memory defaults. TS never overwrites a file it couldn't parse.

**Write failures**: Logged but non-fatal; in-memory config updated anyway. Go silently swallows write errors (no error return). TS records the new mtime to prevent re-reading its own write.

**First run**: Missing config file is seeded with defaults in TS; Go also calls `writeDefaults()` on ENOENT.

## Gotchas

**mtime precision matters**: Zero time is used when a file is absent or unstat-able (so a missing file never looks "newer" than the loaded cache). Both processes record mtime after their own writes to avoid pointless re-reads.

**customThemes is replaced wholesale**: Unlike other fields, `ui.customThemes` is not deep-merged (line 499–504 in TS). Callers sending a new map must include the full desired set or deleted themes will disappear.

**Deep-merge null/undefined semantics**: null or undefined source values mean "unset" (skip, keep the default), not "delete the key." YAML `ui:` with no children parses to `{ ui: null }` and is skipped.

**Migrations must stay in sync**: Three keybindings migrations (legacy mode/leader reset, flat-chord upgrades, removed-action pruning) run in both TS and Go. A change in one must land in both or users on one runtime drift.

**Go has no persist-blocking**: Go's `configService` will write defaults on read failure; TS refuses to write if the file failed to load. This asymmetry means a web/mobile client could wipe a corrupted config that the desktop is protecting.

**Renderer gets its own generated copy**: Main and renderer build graphs don't share modules, so `apps/desktop/src/renderer/src/hooks/configDefaults.generated.ts` exists separately. Both are generated from the same JSON, and drift tests verify both (line 282–293 in configService.test.ts).

**Historical reset bug**: Without the mtime gate, a main-process save (e.g., `usageAccumulator` recording `seenModels`) would deep-merge onto the cached config from startup and clobber any changes the brain persisted after launch. The mtime gate forces a re-read when the file changes underneath, so the merge sees the brain's latest state.

## Hand-authored notes (2026-08-13)

- **`CLAUDE_CONFIG_DIR` is a THIRD config location, and it is per-spawn.** It is not part
  of `config.yaml`'s two-writer story, but it is where people look for it. Claude Code
  honours it to relocate `~/.claude`, and this repo sets it **per session** from a Claude
  profile's `configDir` (`claudeSpawn.ts` ~L85, `managedSpawn.ts` ~L170) — so a session's
  value is not necessarily in the daemon's or the desktop main process's environment.
  Anything resolving a user's skills/agents/commands has to cope with that; see
  `modules/claude-asset-roots.md` for how claudemon and libraryService each do
  (claudemon tries both the override and the default; the desktop reads its own env).
- It is also a **test-isolation requirement**: `libraryService` reads the real `~/.claude`,
  so any suite touching it must point `CLAUDE_CONFIG_DIR` at a temp dir or it silently
  asserts against the developer's own machine.

## Hand-authored notes (2026-07-30/31) — lock, patch trimming, delegation guards

- **`config.yaml` is now guarded by a cross-process O_EXCL lockfile** (`config.yaml.lock`), held across all three steps (refresh → deepMerge → atomic write) by BOTH twins: `apps/desktop/src/main/lib/configLock.ts` and `services/hub/cmd/brain/configlock.go`. The mtime gate only closed the refresh; an interleaved write from the other process was silently lost with both reporting success. Single ownership was investigated and rejected (desktop can't own it — headless serve has no Electron; brain can't — serve runs brain-less when no binary is found). A lock that can't be taken is a REFUSED save, never write-anyway; a lock older than `staleMs` is stolen. `contracts/config-lock.json` pins staleMs + filename — those are correctness parameters (asymmetric expiry = both sides writing "exclusively"). Wait budgets are deliberately per-side (desktop 250ms sync on the main thread; brain 2s). Test gotcha: `configService.test.ts` mocks `fs` as an allowlist — any new fs primitive in the write path needs a matching mock entry.
- **Renderer saves are trimmed at the seam**: `ConfigContext.save` runs partials through `minimalConfigPatch(snapshot, partial)` (`lib/configPatch.ts`) before IPC. Callers spread whole subtrees from stale snapshots, and `saveConfig` replaces `ui.customThemes` + `claude.budgets` WHOLESALE — so an untrimmed sidebar drag could delete a phone-created theme. TRAP: the diff must NOT recurse into the wholesale paths (recursing turns a deletion into an empty patch). `WHOLESALE_PATHS` is a twin of configService.saveConfig's explicit handling — keep them in step. Main also pushes changes: `configService.onChange` + fs.watch on the config DIRECTORY (atomic writes replace the inode, killing a file watch), mtime-gated against echoing its own writes, pushed over `config:changed`.
- **Delegation left the guards behind (fixed 2026-07-31)**: `DELEGATE_CATALOG_TO_BRAIN` defaults ON, so the brain is the live writer for config/layouts/saved sessions/profiles/library — but fsync, the saved-session identity check, and persistBlocked/.broken-* backups had all stayed TS-only. Fixed: brain `writeFileAtomic` now fsyncs before rename; `migrateSessionData` returns a `recognised` flag and `useSessionLifecycle` blocks every save path for the run (`restoreFailedRef` — a REF, since timers/beforeunload captured older renders) when a layout file can't be read, so a rollback/EACCES no longer autosaves `agents: []` over the user's layout; `contracts/host-trusted-config-cases.json` pins the host-trusted section list + drop semantics on both sides. Open follow-ups noted then: stores.go identity check, layout schemaVersion/.broken-* backups, silent skip of unparseable session files.
- **Defaults twins beyond config_defaults.json**: the shipped keybindings live in `config_defaults.json` AND `lib/keybindingPresets.ts`'s VSCODE preset (re-picking a preset overwrites config, so a mismatch only shows after a preset switch) — a test pins `DEFAULT_CONFIG` and `KEYBINDING_PRESETS[DEFAULT_PRESET_ID]` equal (palette default is `mod+k`). An action carries exactly ONE combo (`eventMatchesCombo` parses a single string) — alternate bindings would be a feature, not config.
- **Write-throttling pattern for high-frequency UI config** (`ui.sidebarWidth`, `lib/sidebarWidth.ts`): drags write React state per frame (rAF-coalesced) and only `saveConfig` on pointerup / after a 400ms keyboard-repeat window — a config write per mousemove would hammer the locked/mtime-gated writer. `resolveSidebarWidth()` re-clamps per window resize but deliberately does NOT commit (a width dragged on a big monitor survives a laptop session). Reuse this shape for any slider-like config value.

## Hand-authored notes (2026-08-23) — ProjectIdentity is now single-sourced

- **`ProjectIdentity` (per-project label/color/icon/yolo/delivery/worktreeSetup/plugins) used to be hand-kept in THREE places** — `configService.ts`, `useConfig.ts`, and a fourth unnamed inline copy in `main/shared/ipcTypes.ts`'s `AppConfig.projects` — and they drifted silently since `config.yaml` is untyped YAML at runtime (a missing field round-trips fine, it just vanishes from the TS type). The `ipcTypes.ts` copy was missing BOTH `yolo` and `delivery` despite that file's own doc comment warning this exact drift class is why it exists. Fixed 2026-08-23: `ProjectIdentity` is declared ONCE in `main/shared/ipcTypes.ts` (already the established main↔renderer shared-type file — renderer's `tsconfig.json` includes `../../main/shared`); `configService.ts` imports it directly, `useConfig.ts` imports and re-exports it (`export type { ProjectIdentity }`) so existing `from '../hooks/useConfig'` call sites keep working. **Any future per-project field goes in `ipcTypes.ts`'s `ProjectIdentity` only** — the risk this closed was a future config-validation pass reading `AppConfig` silently stripping `yolo`/`delivery` on write/normalize with no visible error.

## Hand-authored notes (2026-08-24/29) — `save_config` is safe by construction, except where it deleted a map

- **`save_config` deep-merges under a lock with compare-and-swap — it does not
  clobber.** The facade's `save_config` tool → `config.save` →
  `configService.saveConfig` is a read-modify-write under the cross-process
  O_EXCL lockfile: it re-reads config.yaml from disk UNCONDITIONALLY,
  deep-merges the caller's partial onto it, compare-and-swaps on `mtimeMs:size`
  immediately before publishing (retrying up to 5 times if a
  non-lock-participating writer beat it), then writes atomically. The Go twin in
  `services/hub/cmd/brain/config.go` mirrors all of it, and a failed lock is a
  REFUSED save, never a write-anyway. Bus callers (which an agent is)
  additionally have `HOST_TRUSTED_SECTIONS`/`PATHS` stripped before the merge.
  **So do not build clobber-protection machinery around agent-driven config
  edits; it already exists and predates the question.** The one thing an agent
  must be told is the WHOLESALE rule — only `ui.customThemes`, `claude.budgets`
  and `projects` are replaced rather than merged (and only when the caller sends
  them, pinned across both writers by `contracts/wholesale-config-paths.json`),
  so patching `ui.customThemes` without resending the whole map deletes every
  other custom theme and nothing warns. Put that warning in any app-owned prompt
  that lets an agent touch it — `renderer/src/lib/draftAgent.ts`'s appearance
  brief does, pinned by `tests/draftWithAgent.test.tsx`.
- **…and a wholesale path's else-branch DELETED the map. Refuse a non-object;
  never coerce to `{}`.** `applyWholesale` in `services/hub/cmd/brain/config.go` ended
  `else { dst[leaf] = map[string]any{} }`. Because a wholesale path is REPLACED
  rather than deep-merged, that branch did not degrade a malformed value — **it
  deleted the user's whole map and returned the emptied config as a SUCCESSFUL
  save.** `writeConfigYAML` backs up only an UNPARSEABLE file on read
  (`.broken-<ts>`), never a destructive successful write, so there is no
  automatic recovery path. Two non-obvious details: (1) on an otherwise-pristine
  config the wipe is INVISIBLE, because the merged document comes out exactly
  equal to the shipped defaults and `refuseWipeWithDefaults` declines the write
  for an unrelated reason — a repro test only fails once one ordinary non-default
  setting is present; (2) the TS twin took the SAME input and wrote it through
  verbatim (`dst[leaf] = src[leaf] ?? {}`), so the two config.yaml writers
  disagreed **in opposite directions** on the one input that loses data. The
  trigger was the MCP facade: `save_config` was registered with `addObjectTool`,
  whose SDK-inferred schema is exactly
  `{"type":"object","additionalProperties":true}` (verified by running), so a
  client that serialised its argument sent `projects` as a STRING and it was
  forwarded verbatim to the bus. Impact: every agent's config write answers in the
  brain, so an operator-tier agent could empty a user's project list (labels,
  colours, icons, favourite flags, delivery modes, yolo flags, worktreeSetup
  hooks) with a successful-looking save and nothing to restore from.
  Three layers now, **none sufficient alone**: `applyWholesale` returns
  `errWholesaleNotAMap` (propagated through `saveLocked`/`save` to a bus error),
  the TS twin throws `WholesaleValueError`, and `services/hub/cmd/mcp` declares a real
  `save_config` `InputSchema` built from the wholesale path list plus a local
  handler check. `contracts/wholesale-config-paths.json` grew a `valueCases`
  block pinning both writers and the facade's path list.
  **If you add a wholesale path, add it to the fixture's `paths` AND give it a
  `valueCases` row** — the Go loader fails on an unexercised path. `null` is
  refused with the rest: `{}` already spells "empty this map" in both languages.

## Hand-authored notes (2026-08-24/25) — create-once state files, and telling first run from state loss

Three loaders were shaped "read; on ENOENT, create a new one" — correct on a
first run and wrong on every later one, **because a recreated credential is a
DIFFERENT credential**: `<config>/workspacer/remote-token`
(`services/hub/cmd/workspacer/token.go`), `<config>/workspacer-hub/vapid.json`
(`services/hub/internal/push`), and `<config>/workspacer/config.yaml`
(`services/hub/cmd/brain`, `loadFromDisk`'s first-run arm). Note `loadFromDisk` already
handled unreadable / unparseable / empty / MID-RUN disappearance
(`c.current != nil`) with `persistBlocked`; only the FIRST read of the process
was silent.

`services/hub/internal/statelost` (Go) + `apps/desktop/src/main/lib/stateLoss.ts`
(the TS twin) answer the question those loaders could not: **is the directory
around the missing file empty (nobody has ever run here), or does it still hold
the rest of the state (something took this one file away)?**

The three got DIFFERENT answers on purpose, keyed to what the process IS — not to
how important the file feels:

- **remote-token → `workspacer serve` REFUSES to start.** A foreground CLI's exit
  is the loudest signal available, and there is no useful work a mis-identified
  node can do. Escapes are `--token`/`$HUB_TOKEN` and
  `--allow-new-token`/`$WORKSPACER_ALLOW_NEW_TOKEN=1`. Its desktop twin
  (`hubDaemon.ts`) only WARNS — an Electron app that will not boot leaves nobody
  able to read the message.
- **vapid.json → warn, continue, and DROP the dead subscriptions.** `push.New`'s
  error is fatal to the WHOLE hub, so refusing would stop the
  bus/sessions/federation over a notifications keypair.
  `push-subscriptions.json` is the precise evidence and the damage count: those
  subs were negotiated against the vanished public key and can never receive again.
- **config.yaml → warn, continue, persistence NOT blocked.** The brain is a
  SUPERVISED CHILD; exiting is a restart crash-loop that takes the node's whole
  capability plane down. Blocking persistence would also break the legitimate
  "config dir has other state but never had a config.yaml" install.

**Empty directories are never evidence; zero-byte FILES are.** `Suspected`
originally returned true for ANY directory entry other than the missing file,
including an EMPTY subdirectory — and `deploy/fly/node/bootstrap.sh` `mkdir -p`s
`plugins/`, `library/`, `layouts/`, `sessions/` and `logs/` inside
`~/.config/workspacer` before the brain starts, so
`statelost.Suspected(configDir(), "config.yaml")` fired on **every genuinely-first
boot** of the Fly node: `brain: STATE LOSS: …/config.yaml is missing`.
Reproduced against a real image on an empty volume, not inferred. Fixed by
requiring an entry to HOLD something (a file of any size, or a non-empty
directory) — `deploy/fly/hub/bootstrap.sh` had already reached the same
correction independently in shell (its `bs_snapshot_dir`, "TRAP 1"), so the Go
and shell halves of one rule were disagreeing. **A guard that is wrong on every
first boot is one the operator learns to scroll past, which costs it the cases it
exists for.**

Two rules for changing this: **change both twins and both test tables together**
(they mint the same remote-token), and the TS file must use `fs.readdirSync(dir)`
(name list), **NOT `{ withFileTypes: true }`** — `hubDaemon`'s test doubles mock
`readdirSync` as a plain string array, so depending on `Dirent` breaks 20
unrelated tests. New create-once state files should route their ENOENT arm
through `statelost.Suspected` and pick refuse/warn by asking *"is this process a
foreground CLI, a supervised child, or a GUI?"*
