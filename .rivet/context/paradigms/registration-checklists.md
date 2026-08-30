---
title: "Registration checklists: what a new bus method, capability, fixture or notification field actually costs"
tags: [checklist, registration, bus-method, capability, capspec, contracts, fixture, ratchet, drift-guard, notification, wholesale-config]
related_paths:
  - "services/hub/internal/capspec/capspec.go"
  - "services/hub/internal/capspec/composition.go"
  - "services/hub/internal/capspec/httproutes.go"
  - "services/hub/internal/authtoken/authtoken.go"
  - "services/hub/cmd/brain/handlers.go"
  - "services/hub/cmd/brain/delegation_guard_test.go"
  - "services/hub/cmd/brain/corpusvocab_test.go"
  - "services/hub/cmd/brain/contracts_test.go"
  - "services/hub/cmd/brain/capspec_params_test.go"
  - "services/hub/cmd/mcp/main.go"
  - "services/hub/cmd/mcp/tiers_test.go"
  - "apps/desktop/src/main/services/hubCapabilities.ts"
  - "apps/desktop/src/main/services/hubCapabilitiesGuards.test.ts"
  - "apps/desktop/src/main/services/contractsVocabulary.test.ts"
  - "apps/desktop/src/renderer/src/lib/pluginPermissions.ts"
  - "contracts/path-containment-cases.json"
  - "contracts/README.md"
owner: Damien Touchette
last_reviewed: 2026-08-29
---

# Registration checklists: what a new bus method, capability, fixture or notification field actually costs

> **This doc is a SYNTHESIS.** Each checklist below was learned separately, from a
> separate change, and written down separately. Nothing in the codebase declares
> "the checklist" — the lists are reconstructed from the drift guards that fail
> when you miss an entry. Treat the counts as a floor, not a closed set: two
> censuses of the `brief.*` list taken five days apart found four items and five
> items, and neither was wrong (see *Why the counts disagree*, below). Re-derive
> from the failing test, don't trust the number.

## Why this exists

Adding one line to `apps/desktop/src/main/services/hubCapabilities.ts` (or one
JSON file to `contracts/`) is never the whole change. Both languages carry
tables, allowlists, ratchets and corpus guards that fail closed on an
unclassified name — and each one fails with a message that names *itself*, not
the thing you added. The classic symptom is four red suites in a row, each of
which reads like an unrelated regression, discovered one at a time.

The point of collecting these is that **the list is finite**. Budget the whole
list up front, then run the fast loop at the bottom of each section.

## Checklist A — a path-bearing capability (`fs.` / `search.` / `library.` / `git.` / `providers.` / `brief.`)

`capspec.go`'s `pathVerbPrefixes` makes any method under one of those prefixes
`LooksPathBearing`, and the bus **fails closed** until it is classified
everywhere. Union of the two censuses taken 2026-08-24 and 2026-08-26:

1. **`services/hub/internal/capspec/capspec.go` `PathParam`** — declare which param carries the path.
2. **`services/hub/internal/capspec/composition.go`** — a `Compositions()` record whose `Reason`
   prose literally contains the witness symbol `assertPathAllowed` and is ≥60
   characters. `TestInertClaimsCarryACheckedWitness` greps the prose for it.
3. **`contracts/path-containment-cases.json`** — a `methods` entry with the SAME
   field name. `capspec_test` fails in BOTH directions if the fixture and
   `PathParam` disagree — including for desktop-only methods, which declare
   `providers: ["main"]` (`brief.archive` is the precedent). The failure reads
   *"nothing asserts that its provider actually calls the guard"*, which sounds
   like a brain problem and is not.
4. **`apps/desktop/src/main/services/hubCapabilitiesGuards.test.ts`** — two
   hard-coded ratchets on the count of `main`-owned methods (`mainSweep` and
   `tildeSweep`) that must be bumped together.
5. **`apps/desktop/src/renderer/src/lib/pluginPermissions.ts` `CAP_LABELS`** — the
   drift guard greps `hubCapabilities.ts` for `registerCapability|cat` and fails
   on any unlabelled method. An unlabelled method also renders to the user as
   SENSITIVE.
6. **`services/hub/cmd/mcp/tiers_test.go`** — the banned lists must name the new
   tool so a `view`/`triage` scout is pinned out of it.

The MCP facade itself needs nothing beyond registering the tool: tiers DERIVE
from `authtoken`'s exact-name allowlists, so an unlisted `brief.*` method is
operator-only by construction.

Fast loop: `go test ./internal/capspec/...` plus the desktop
`hubCapabilitiesGuards` suite.

## Checklist B — a dual-provider claudemon-proxy method (brain AND desktop both provide it)

The other shape: not path-bearing, but registered by both providers.
`sessions.subagentConversation` (7f4cf276) is the worked example — seven entries,
each with its own build-failing drift guard:

1. **`services/hub/cmd/brain/handlers.go`** — the `registry.methods()` list AND the
   `handle()` switch. Two edits, one file.
2. **`apps/desktop/src/main/services/hubCapabilities.ts`** — `registerCapability`.
3. **`services/hub/cmd/brain/delegation_guard_test.go` `declaredOverlap`** —
   required *precisely because* it now has two providers. The router is
   single-owner per method and first-registration-wins, so an undeclared
   collision fails `TestMainOwnedCapabilitiesDoNotCollideWithTheBrain`.
4. **`services/hub/internal/capspec/capspec.go` `inertMethods`** — else the capspec
   guards reject it as "capspec says nothing about it".
5. **`services/hub/internal/capspec/httproutes.go`** — the claudemon HTTP twin
   route, `TwinKind: TwinMethod`.
6. **`services/hub/internal/authtoken/authtoken.go` `viewMethods`** — an exact-name
   allowlist. Omit it and web/remote scoped tokens are refused; this is the
   **silent one**, presenting as "works on desktop, null in the browser".
7. **`apps/desktop/src/renderer/src/lib/pluginPermissions.ts` `CAP_LABELS`** — same
   as A5.

A method can need BOTH lists.

**Diff-reading trap:** several of those Go tables are gofmt-aligned maps keyed by
method name. A key longer than every prior key REALIGNS the whole map —
`sessions.subagentConversation` showed 69 changed lines in `capspec.go` and 21 in
`authtoken.go` for one real entry each. Review with `git show -w` or you will
read whitespace as substance.

## Checklist C — a new `contracts/` fixture

Every file added under `contracts/` trips guards in both languages:

1. **At least TWO loaders in TWO different languages.**
   `contracts_test.go`'s `TestEveryContractFixtureHasAtLeastTwoLoaders` counts
   TEST files only — an implementation file naming the fixture counts as a
   "mention", not a loader.
2. **A row in `contracts/README.md`** naming the fixture. Checked in both directions.
3. **A `vocabulary.blocks` registry inside the fixture** declaring every
   array-of-objects block with `required` + `loaders` needles
   (`"<repo-relative file>::<needle>"`). Declaring an `optional` field that no
   case carries FAILS (optional-used), as does any case key not in
   required/optional (unknown-fields). Nested arrays inside a case row are NOT
   walked, so `expect: [ {...} ]` needs no declaration.
4. **If the fixture carries NO array-of-objects case blocks** (e.g. a single job
   spec), it must instead go in TWO exemption allowlists — `NO_BLOCK_FIXTURES`
   in `apps/desktop/src/main/services/contractsVocabulary.test.ts` and
   `vocabExempt` in `services/hub/cmd/brain/corpusvocab_test.go` (twins, same
   check ids).
5. **Both fixture-count floors bumped in lockstep** — `contractsFixtureFloor`
   (`services/hub/cmd/brain/corpusvocab_test.go`) and
   `CONTRACTS_FIXTURE_FLOOR` (`contractsVocabulary.test.ts`).
6. **A per-corpus case floor in each loader.**

Separately, a new PARAM whose name is in capspec's `dangerousParams` vocabulary
costs two more ratchets: an entry in `capspec.go`'s decision map plus BOTH scan
floors — `desktopDangerousParamFloor` (`services/hub/internal/capspec/capspec_test.go`, scans
`hubCapabilities.ts`) and `brainDangerousParamFloor`
(`services/hub/cmd/brain/capspec_params_test.go`, scans `handlers.go`'s dispatch switch).
**`RatchetError` fails on a RISE as well as a fall**, so both must move.

Fast loop: `go test ./cmd/brain/ -run 'Contract|Corpus|Vocab|Fixture'` plus
`go test ./internal/capspec/`.

## Checklist D — a new `InAppNotification` field

Six places, or the field is silently dropped (`postInApp` spreads, so it is not a
gate):

1. `notifyIn` (the Go tool schema, `services/hub/cmd/mcp/main.go`)
2. `hubCapabilities.ts`'s `notifications.post` destructure
3. …and its `postInApp` call
4. `InAppNotification` in `apps/desktop/src/main/shared/ipcTypes.ts`
5. `NotificationInput` + `normalizeNotification` in
   `apps/desktop/src/renderer/src/lib/notificationStore.ts` — an explicit allowlist
6. the consumer in `NotificationsContext.activate`

The historical failure this closed: the facade's `notify` tool carried only
title+body while the capability behind it already accepted
`sessionId`/`paneType`/`url`/`level`/`key`/`silent`/`inAppOnly`, so an agent could
not post a click-through notification at all — and `hubCapabilities`' own OS
notification branched `sessionId → url → focusWindow` and never handled
`paneType`, so even a plugin's pane target went nowhere.

## Checklist E — a new wholesale config path

`contracts/wholesale-config-paths.json` needs the path in `paths` AND a
`valueCases` row; the Go loader fails on an unexercised path. See
`domains/config.md` for why the value-shape cases matter (the else-branch that
deleted a user's whole map).

## Why the counts disagree

The `brief.*` list was written down twice — "five registrations" on 2026-08-24 and
"four registrations" on 2026-08-26 — and the two lists are not the same four/five
items. Neither author was careless: each enumerated the guards that actually
failed for the change in front of them, and the guard set grew between the two
dates. The union is Checklist A above.

The same escalation happened to the pending-slot writer census in
`domains/session-lifecycle.md` (2→6, four times running). **The pattern itself is
the lesson: a count in a doc is evidence that someone was surprised, not a
closed enumeration.** When one of these guards fails on you, add the item here.
