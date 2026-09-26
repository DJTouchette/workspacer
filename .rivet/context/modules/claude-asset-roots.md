---
title: Claude asset roots — skills, agents and commands across project/user/plugin
tags: [claude-skills, asset-resolution, twin-implementations, claude-config-dir, library, context-pane, origin]
related_paths:
  - "services/claudemon/src/providers/claude_stream.rs"
  - "services/claudemon/src/session/state.rs"
  - "apps/desktop/src/main/services/libraryService.ts"
  - "apps/desktop/src/main/services/hubCapabilities.ts"
  - "services/hub/cmd/brain/library.go"
  - "apps/desktop/src/renderer/src/panes/ContextPane.tsx"
  - "apps/desktop/src/renderer/src/panes/LibraryPane.tsx"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Claude inventory and library asset roots

## Related resolvers, distinct purposes

`services/claudemon/src/providers/claude_stream.rs` enriches assets reported in
an init frame for Context inventory. `apps/desktop/src/main/services/libraryService.ts`
enumerates editable/library assets and slash-picker entries. Go
`services/hub/cmd/brain/library.go` implements the selected-object bus library.
These implementations share layouts, but do not promise identical root sets
or authority. Test the caller being changed, not an assumed universal resolver.

The Workspacer lookup layouts are `skills/<name>/SKILL.md`, `agents/<name>.md`,
and `commands/<name>.md`. Installed plugin roots are version directories under
`plugins/cache/<marketplace>/<plugin>/<version>`; marketplace roots use
`plugins/marketplaces/<marketplace>/plugins/<plugin>`. These describe this
checkout's resolver, not a guarantee about every current upstream CLI version.

## Inventory parsing and enrichment

The pure init parser accepts string lists, named-object lists and name-keyed
maps for inventory entries. Memory paths accept a map or list. The driver
then performs best-effort disk enrichment once per capabilities/init update.
Do not depend on a historical sample's count, ordering, or skill/slash-list
relationship as a wire invariant.

Rust roots are project `.claude`, daemon-environment override and default user
`.claude`, plugin paths explicitly reported by the frame, then discovered
plugin directories. Resolution tries candidates within each root before the
next root. A reported skill can resolve through the commands fallback. Already
path-bearing skills/agents skip this resolution pass. Explicit source labels
are retained through `get_or_insert`.

Unresolved names receive `built-in`. That is a fallback label, not proof of
compiled origin: unreadable roots, unthreaded profile environments or changed
layouts can produce the same result. The literal is also consumed by Context
and skill cards. File sizes use bytes and estimated tokens `ceil(bytes/4)`,
not measured prompt tokens. Memory directories expand only to bounded depth
and count; inventory is not a complete filesystem index.

Description extraction recognizes simple scalar/block frontmatter rather than
full YAML, clamps to 300 characters plus an ellipsis, and parses at most the
first 16 KiB. It currently **reads the whole file first** before slicing that
parse window; do not describe the parse cap as a disk-I/O bound.

## Library roots, precedence and profile limits

TS roots are project `.claude`, one user root from the main process's
`CLAUDE_CONFIG_DIR` or home default, then discovered plugins. The user root is
omitted when it resolves to the project `.claude` directory. Items deduplicate
by kind plus ID with first root winning. Plugins receive `plugin:<name>` origin
and are noneditable; Rust inventory origin labels use plugin names instead.

A profile can set `CLAUDE_CONFIG_DIR` per spawn without changing either the
main process or daemon environment. Rust trying both its own override and home
does not fully recover an arbitrary per-session profile root. TS only reads
its own process environment. Do not promise session-specific inventory/library
parity from these fallbacks. Set temporary roots in tests to avoid reading a
developer's real assets.

Both resolvers define a 200-root discovery limit, but implementation differs:
TS limits added plugin roots; Rust stops discovered additions against total
roots after already accepting project/user/frame roots. Neither limit bounds
the preceding directory enumeration, and frame roots can already exceed it.
Sorted directory enumeration is lexical, not newest-version selection.

## Mutation and selected-object containment

Native library operations can expose project/user/plugin rows. Bus
`library.list` applies a per-file selected-library guard; Go enumerates project
Claude assets rather than reproducing native user/plugin discovery. A project
rooted at home still requires an actual allowed library object directory.
Do not widen this guard to ambient browse roots to make native and bus lists
match. This boundary does not restrict an authenticated session's separate
ordinary filesystem tools.

Writes/deletes reject explicit plugin origins before deriving paths. TS selects
user or project destinations from origin, with unknown origin falling back to
project; IDs must be plain basenames. Preserve origin when deleting, or the
wrong root may report a successful no-op. Go has its own item-path guard and
must not be described as a universal user-root writer.

The per-file guard remains the last argument on TS list/save/remove legs;
existing guard coverage inspects that argument. Root enumeration/read failures
usually skip entries, while mutations and guard failures can throw. An empty
inventory is not evidence that no assets exist, and full-body Library rows have
no fixed payload-size guarantee.

## Verification

Review the Rust enrichment/parser tests, TS library fixtures and Go library
containment tests together when changing a root or origin. Also inspect
`claudemonStatusLineBridge.ts` field mapping and Context/Library consumers when
adding wire fields. Historical live CLI observations remain evidence for their
recorded version only; this audit did not probe an external installed CLI.
