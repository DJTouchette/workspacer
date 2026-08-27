---
title: Library seeding is additive per ITEM, gated by library-seeded.json
date: 2026-08-26
promoted: false
---

# Library seeding is additive per ITEM, gated by library-seeded.json

## Observation
Both seeders used to no-op the moment `<config>/library` held any `.md`, so every
starter added after a user's first run was invisible to the entire installed base —
the three dispatch templates from 7317b1c5 landed for nobody. Fixed in b8b447e3:
`LibraryService.seedGlobalIfEmpty` → `seedGlobalStarters` (starters now come from a
`starters()` array of `{id, item}`) and the Go twin `seedLibraryIfEmpty` →
`seedLibraryStarters` + `starterItems()`.

The non-obvious part is `<config>/library-seeded.json` (`{"seeded": [id...]}`, written
and read by BOTH twins, same key). It exists because the directory alone cannot tell
"you deleted this starter" from "you were never offered it", and the seeder must never
resurrect the first. An id recorded there is never written again — and a starter that
already exists on disk is recorded but never overwritten, so deleting a hand-edited
starter afterwards still sticks.

Legacy installs (non-empty library, no marker) bootstrap the marker from
`PRE_MARKER_STARTER_IDS` / `preMarkerStarterIDs` — the four starters that predate the
marker. That list is FROZEN: adding to it would silently stop a new starter reaching
existing users, which is the exact bug this fixed. New starters go in `starters()` /
`starterItems()` only.

Two test consequences: `TestLibrarySeedAndList` now pins the count against
`len(starterItems())` rather than the literal 7 (the TS twin's count is still a
hand-kept literal in that same test), and any test asserting on the WHOLE library list
must call `suppressLibrarySeed(t)` — a populated-but-unseeded temp dir legitimately
gains starters now, which is how `TestListersUseTheFixtureOrdering` broke. Go seeds on
every `listLibrary` call; TS seeds only in the singleton's constructor.
