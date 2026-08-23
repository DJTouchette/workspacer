---
title: Claude asset roots — skills, agents and commands across project/user/plugin
tags: [claude-skills, asset-resolution, twin-implementations, claude-config-dir, library, context-pane]
related_paths:
  - "services/claudemon/src/providers/claude_stream.rs"
  - "services/claudemon/src/session/state.rs"
  - "apps/desktop/src/main/services/libraryService.ts"
  - "apps/desktop/src/main/services/hubCapabilities.ts"
  - "services/hub/cmd/brain/library.go"
  - "apps/desktop/src/renderer/src/panes/ContextPane.tsx"
  - "apps/desktop/src/renderer/src/panes/LibraryPane.tsx"
owner: Damien Touchette
last_reviewed: 2026-08-13
---

# Claude asset roots — skills, agents and commands across project/user/plugin

## Overview

Claude Code loads skills, agents and slash commands from several roots, and two
independent implementations in this repo have to resolve them **the same way**:
`asset_roots` (Rust, claudemon — feeds the Context pane's itemized inventory)
and `claudeRoots` (TS, libraryService — feeds the Library pane and the composer's
`/` picker). This is the same twin-implementation hazard `config.md` documents
for `config.yaml`'s two writers, with the same failure signature: the *same
skill* reads as `built-in` in one pane and as a plugin file in another.

The layout is shared by every root — `skills/<name>/SKILL.md`,
`agents/<name>.md`, `commands/<name>.md` — so a "root" is just a directory that
may contain those three, whether it is a `.claude` directory or a plugin
package.

**Precedence (first match wins, mirrors what Claude Code itself resolves):**

1. `<cwd>/.claude` — the project's own
2. `~/.claude` — the user's (see `CLAUDE_CONFIG_DIR` below)
3. plugins, in two on-disk shapes:
   - `~/.claude/plugins/cache/<marketplace>/<plugin>/<version>/` — installed;
     the **version** directory is the root, not the plugin directory
   - `~/.claude/plugins/marketplaces/<mp>/plugins/<plugin>/` — the marketplace
     clone, whose commands are live *without* appearing in
     `installed_plugins.json`

## What the init frame actually reports

Verified against a live `system/init` frame, CLI 2.1.232 — this is the part that
is counter-intuitive enough to have caused the original bug:

- `skills` is a **plain list of strings**, no paths, no descriptions.
- `skills` is a **prefix of `slash_commands`** — every skill is invocable as a
  slash command, and the two lists share their leading entries.
- A plugin's **slash command is reported as a skill**. `code-review` appears in
  `skills` but lives at
  `plugins/marketplaces/claude-plugins-official/plugins/code-review/commands/code-review.md`
  — hence the `commands/<name>.md` fallback in the skill resolver.
- Of 18 reported skills, **only 3 exist on disk**. The rest (deep-research,
  dataviz, verify, debug, batch, doctor, loop, schedule, run, …) are **compiled
  into the `claude` ELF binary** — `~/.local/share/claude/versions/<v>` is a
  single executable, not a directory. `agents` is the same story (Explore, Plan,
  general-purpose have no files).

Therefore **"resolves to no file" is a correct answer, not a lookup failure**.
Both sides label it `built-in` (`BUILTIN_SOURCE`) rather than leaving a blank
row, which is what the pane used to show for 15 of 18 skills.

## Key modules

- `services/claudemon/src/providers/claude_stream.rs` — `BUILTIN_SOURCE` (L486),
  `AssetRoot` (L492), `asset_roots` (L511), `user_claude_dirs` (L590),
  `read_dirs`/`dir_name` (L608/L572), `resolve_asset` (L624),
  `frontmatter_description` (L649) + `clamp_description` (L695),
  `enrich_inventory` (L713). Runs once per init frame, at the `AgentUpdate::Capabilities`
  branch of the driver loop — `translate` stays pure, disk access happens here.
- `services/claudemon/src/session/state.rs` — `ContextItem.source` (the resolved
  origin) and `ContextItem.description` (frontmatter), both `skip_serializing_if`.
- `apps/desktop/src/main/services/libraryService.ts` — `ClaudeOrigin` (L69),
  `assertWritableOrigin` (L159), `ClaudeRoot` (L184), `userClaudeDir` (L210),
  `subdirs` (L223), `pluginRoots` (L246), `claudeRoots` (L270),
  `writableRootDir` (L282), `readClaudeRoot` (L401), `readClaudeItems` (L479).
- `apps/desktop/src/main/services/claudemonStatusLineBridge.ts` — `mapInventory`
  maps the snake_case wire shape to `ContextItemInfo`; a new `ContextItem` field
  is invisible to the renderer until it is added here.
- `apps/desktop/src/renderer/src/panes/ContextPane.tsx` — `BUILTIN_SOURCE` (L42,
  mirrors the Rust constant), `ORIGIN_RANK` (L46), `byOrigin` (L51). Skills and
  agents are sorted owned-first rather than in frame order.
- `apps/desktop/src/renderer/src/panes/LibraryPane.tsx` — `originLabel`,
  `originBadge`, the origin picker ("this project" / "all projects") and the
  **Copy to project** button that replaces Edit/Delete for plugin items.
- `services/hub/cmd/brain/library.go` — the Go twin: `claudeOriginProject`,
  `assertWritableOrigin`, `libraryItem.Origin`/`.Editable`.

## `CLAUDE_CONFIG_DIR` is per-spawn, not per-machine

Claude Code honours `CLAUDE_CONFIG_DIR` to relocate `~/.claude`, and **this repo
sets it per spawn** from a Claude profile's `configDir`
(`claudeSpawn.ts` ~L85, `managedSpawn.ts` ~L170). Consequences:

- A **session's** value is not necessarily in the **daemon's** environment, so
  `user_claude_dirs()` returns BOTH the override and the default `~/.claude` and
  tries each. Precedence still favours the explicit override. (A fully correct
  fix would thread the spawn env down to `enrich_inventory`; `transcript.rs`
  already registers a spawn's `CLAUDE_CONFIG_DIR` as a transcript root and is
  the precedent to follow if this ever matters more.)
- `userClaudeDir()` in the desktop main process reads only its **own** env,
  which is the right answer for "the user's own claude dir".
- It is also the **test-isolation hook**. This is not optional: `libraryService`
  now reads the real `~/.claude`, so a suite that doesn't point
  `CLAUDE_CONFIG_DIR` at a temp dir silently asserts against the developer's own
  skills — a 2-item assertion in `libraryService.test.ts` became 28 the moment
  the user root was added.

## Failure modes

- Every root read is best-effort: a missing or unreadable directory reads as
  empty (`read_dirs` / `subdirs` swallow errors), so a partially-installed
  plugin degrades to fewer resolved skills, never an error.
- `frontmatter_description` is deliberately **not** a YAML parse — it handles the
  two shapes Claude Code writes (a scalar on one line, and a `>`/`|` block whose
  indented continuations fold into one) and returns `None` on anything else. A
  malformed SKILL.md costs a description, not the inventory.
- Descriptions are clamped to `MAX_DESCRIPTION_CHARS` (300) **on a char
  boundary** — a byte slice panics mid-UTF-8, and these are prose that routinely
  contains multi-byte characters. They ride the statusLine frame to every client
  on every tick, which is why they are clamped at all.
- Plugin-root enumeration is capped at `MAX_PLUGIN_ROOTS` (200) on both sides; a
  marketplace clone is a checkout of arbitrary size and this walk runs on every
  init frame / every `list()`.

## Gotchas

- **The two root enumerations must agree.** `asset_roots` (Rust) and
  `claudeRoots` (TS) duplicate the same precedence and the same two plugin
  layouts. Change one without the other and a skill's origin badge disagrees
  between the Context pane and the Library pane.
- **`BUILTIN_SOURCE` is duplicated as a literal** in `claude_stream.rs` (L486)
  and `ContextPane.tsx` (L42), and `SkillCard.tsx` has its own copy. It is a
  wire value, so all three must stay `"built-in"`.
- **The bus sees less than the desktop, on purpose.** `library.list` over the
  hub bus is confined by `libraryItemRoots` in `hubCapabilities.ts` (the
  caller's project + the global store), so user and plugin items are
  guard-**skipped** for remote callers — a refusal skips that item rather than
  failing the call. This divergence is deliberate and should not be "fixed":
  adding `~/.claude` to the item roots puts `~/.claude/.credentials.json` one
  planted symlink away from a capability that returns **file bodies**, and
  planting a symlink inside `$HOME` is an ordinary permitted write for an agent
  whose cwd is the home directory (which a bare `agents.spawn({})` produces).
  The Go twin behaves identically, for the same reason.
- **The per-file `guard` must stay the LAST argument** of `libraryService`'s
  legs (`list`/`save`/`remove`). The guard-coverage sweeps in
  `hubCapabilitiesKillSwitch.test.ts` read it positionally off the end of the
  recorded call (`mock.calls[0].at(-1)`), so inserting a parameter after it
  fails three tests with an opaque `guard is not a function`. `remove`'s
  `origin` therefore goes *before* the guard.
- **`origin` is load-bearing on delete.** A claude item's id is its real on-disk
  basename, and the root is chosen from `origin`; without it a delete of a
  `~/.claude` skill targets the project root and unlinks nothing, i.e. reports
  success while the item stays on screen.
- **Plugin items are read-only** and must stay so: editing one in place is
  reverted by the next plugin update, and deleting one corrupts the install
  (the skill branch of `remove` is a recursive, force `rmSync`).
  `assertWritableOrigin` refuses a `plugin:` origin *before* any path is
  derived, on both the TS and Go sides.
- **Payload size.** `library.list` now returns every claude asset **with its
  body**. On a machine with the official marketplace that is ~90 items / ~640 KB
  / ~6 ms warm, re-sent on every `library:changed` (150 ms debounce). Fine for a
  desktop app, but it is a ~160× increase over project-only — worth remembering
  before adding another root or another caller.
- **`~/.claude` may equal the project root.** `claudeRoots` skips the user root
  when `path.resolve(userClaudeDir()) === path.resolve(<cwd>/.claude)`, or an
  agent whose cwd is `$HOME` lists every user asset twice, once per origin.
