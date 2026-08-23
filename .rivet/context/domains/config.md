---
title: Configuration (two writers)
tags: [config, settings, defaults, concurrency, mtime-gate, config-lock]
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
