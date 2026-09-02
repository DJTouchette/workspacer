---
title: "Limit-aware routing: roles to capabilities to (provider, model, effort), plus the two things it enforces"
tags: [hub, go, routing, fleet-manager, agent-spawn, model-selection, ceilings, security-invariant, desktop]
related_paths:
  - "services/hub/internal/routing/routing.default.yaml"
  - "services/hub/internal/routing/policy.go"
  - "services/hub/internal/routing/fresh.go"
  - "services/hub/internal/routing/ceiling.go"
  - "services/hub/internal/bus/rpc.go"
  - "services/hub/cmd/hub/main.go"
  - "services/hub/cmd/hub/routingceiling.go"
  - "services/hub/cmd/mcp/main.go"
  - "apps/desktop/src/renderer/src/lib/fleetManager.ts"
  - "apps/desktop/src/renderer/src/components/settings/SupervisorSection.tsx"
  - "docs/limit-aware-routing.md"
owner: Damien Touchette
last_reviewed: 2026-09-01
---

# Limit-aware routing

## Overview
One file decides which model a piece of work is worth:
`<config>/workspacer-hub/routing.yaml`, seeded from the compiled-in
`routing.default.yaml` and re-read on a content hash every 30s. The chain is
`role -> capability -> (provider, model, effort)`, and the point of the
indirection is that nothing outside the matrix names a model, so a vendor
rename is one edit. Roles are the vocabulary on the wire: `scout`,
`mechanical`, `implementer`, `reviewer`, `deep_reviewer`, `fixer`,
`complex_fixer`, `validator`, `diagnostician`, `judge`, plus `supervisor`
(see the exception below). Profiles resolve a capability to a concrete tuple;
`mixed`, `codex_only` and `anthropic_only` ship.

Read `docs/limit-aware-routing.md` for the per-block detail. This doc is about
which parts of the system consult it and which do not.

## Advisory by default, enforced in two places
`routing.select` is registered as an ordinary read-only RPC
(`services/hub/cmd/hub/main.go`, `RegisterLocalIdent("routing.select", ...)`). Nothing calls
it on a spawn. A caller asks, gets an answer, and is free to ignore it. That is
deliberate: it is a table lookup over a hub-owned file plus one read of
claudemon's `/usage/report`, and every action taken on the answer goes through a
separate refusable capability (`agents.spawn`).

Two rules DO bind, and both live in `sanitizeSpawnParams` (`services/hub/internal/bus/rpc.go`),
injected from `services/hub/cmd/hub/main.go`'s `SetSpawnCeiling`:

- **The per-directory ceiling.** `ceilings:` caps `max_capability` and
  `max_tool_scope` by absolute directory, longest matching ancestor wins. A
  clamped capability also drops the model and effort the caller named.
- **The freshness refusal.** A spawn declaring a `role` or `capability` whose
  active-profile entry carries `fresh: true` (the review capabilities in every
  shipped profile) may not also carry a `resumeSessionId`. It is refused rather
  than stripped, because a dropped resume would start a new session the caller
  believes is a continuation.

`sanitizeSpawnParams` is the only spawn-path function in the repo that is not a
twin, so both rules cover the desktop provider, the headless brain and the
federated hop at once. The local Electron IPC spawn door is deliberately not
sanitized: that is a human at the machine clicking Spawn.

## The wire
Three fields carry a decision onto a dispatch, all on the facade's
`spawn_agent` (`services/hub/cmd/mcp/main.go`, `spawnAgentIn`) and on bus `agents.spawn`:

| Field | What it does |
|---|---|
| `role` | recorded against the session, and read by the freshness rule. Not an authority axis: lying about it can only ever give the caller less. |
| `capability` | the enforced one. Clamped to the directory ceiling, and the model/effort go with it. There is no way to ask for a capability above what `select_model` answered. |
| `decisionId` | joins the decision and the worker it produced in `routing-decisions.jsonl`. |

`escalationScrubbed` on the spawn ANSWER is how a caller learns something was
taken: the hub deletes any incoming value and re-stamps what this router
removed, and the provider returns the union of that and its own clamps. No
silent downgrades. A caller that does not read it cannot tell a clamped spawn
from an honored one.

## Who consults it
- **The Fleet Manager doctrine** (`fleetManager.ts` `MANAGER_PREAMBLE`) tells
  the manager to call `select_model` with a role before every dispatch and to
  pass `provider`/`model`/`effort`/`role`/`capability`/`decisionId` through to
  `spawn_agent`. It names no model family anywhere, on purpose: a model noun in
  the doctrine is the second file a rename has to find. Pinned by
  `apps/desktop/src/renderer/tests/fleetManager.test.tsx`.
- **The dispatch starters** (`libraryService.ts` `starters()` and its Go twin
  `services/hub/cmd/brain/library.go` `starterItems()`) name the role to pass in each item's
  DESCRIPTION. A library item of kind `dispatch` is text-only by construction,
  so a template can carry no spawn arguments at all: the role rides the call.
  `ship-task` is `implementer`, `scout-task` is `scout`, `review-task` is
  `reviewer`, `two-explanations` is `diagnostician`.
- **Nothing else.** The desktop spawn dialog, the web `/app` and `/m` resume
  paths and the renderer's own resume path declare no role, so they make no
  freshness claim and route nothing.

## The supervisor exception, in both directions
`roles.supervisor` is in the matrix so the vocabulary is complete, and it is
NOT consulted. The manager's own harness, model and effort are resolved in
desktop main from `agents.managerProvider` / `managerModels` / `managerEfforts`
(`main/lib/roleProviders.ts`, `main/lib/roleModels.ts`), before there is a
manager to ask. Settings -> Fleet Manager is the only place that value is
chosen, and its Manager model hint says so. Keep both sides naming each other:
two mechanisms that each claim to pick the supervisor's model, with neither
naming the other, is a bug this project has already had once.

## Gotchas
- **No UI, by design.** There is no routing write RPC and there must never be
  one. `ceilings:` is a ceiling only because no bus caller can edit the file it
  comes from (that plus `fs.write` refusing the hub's state directory is the
  whole argument). Editing routing.yaml is a text editor and nothing else.
- **`select_model` is operator tier only.** It is absent from capspec's
  `viewMethods` and `triageMethods`, so `ScopeOperator`'s `["*"]` is what grants
  it. A Fleet Manager holds it; a phone token does not.
- **A ceiling value the file cannot read denies the spawn** rather than being
  skipped. An omitted key still means "this row does not cap that axis".
- **`forecast_weights` produce weighted units, not a share of an allowance**, so
  `expectedWork` on its own leaves demand UNKNOWN for the mode rules. Pass
  `forecastDemandBeforeResetPct` when the real share is known.
- **`difficulty` / `risk` / `decisionDensity` are accepted and not yet acted
  on.** They are recorded on the decision, and they change no answer today.

## Windows path semantics (the containment-windows CI job)

Promoted from the 2026-09-01 Windows-containment learnings. The two learnings
were incident reports on specific CI runs (33495681039 for `2d8f7fd0`,
33503823057 for `710d8e8b`); what follows is the durable part, plus the state of
the job as of `0bac5799`.

- **Ceiling containment is a PATH COMPARISON, and on Windows a byte compare is
  the wrong one.** Windows resolves drive letters, UNC hosts/shares and ordinary
  NTFS components case-insensitively, so a ceiling must answer for the directory
  the filesystem actually opens, not for `routing.yaml`'s casing of it — a
  Cyrillic or umlaut case variant of a configured ceiling that fails to match
  silently hands the caller the DEFAULT ceiling, which is the permissive one.
  The comparison is `CompareStringOrdinal` (`pathmatch_windows.go`), not
  `strings.EqualFold`: it uses the OS uppercase table for non-linguistic
  identifiers and so does NOT apply generic Unicode equivalences — the Kelvin
  sign stays a sibling of `K`, which is what NTFS does. The rule is
  platform-split by build tag (`pathmatch_windows.go` / `pathmatch_unix.go`,
  and the same shape again in
  `services/hub/cmd/brain/pathmatch_windows.go` / `pathmatch_other.go` for the brief tools' `workspaceRoots` confinement, which
  shares this confinement model and had the same bug). Prefix matching must
  also keep rejecting siblings: `C:\work\client-old` is not inside
  `C:\work\client\`, and `\\server\share-old\work` is not inside
  `\\server\share\`.
- **"Private file" is a mode bit on Unix and an ACL on Windows.** The routing
  decision log asserts 0600 privacy; on Windows that assertion is meaningless
  (`-rw-rw-rw-` is what a mode read returns), so the log is created through
  `CreateFile` with a protected DACL and `FILE_APPEND_DATA|WRITE_DAC`
  (`decisionlog_private_windows.go`), with the Unix mode path kept in
  `decisionlog_private_unix.go`. Any new "this file must be private" assertion
  needs both implementations and platform-split tests, not a `chmod` and a
  skip.
- **`filepath.Join` erases the fixtures a path test is trying to exercise**, so
  a Windows-safe routing fixture must build its dirty inputs literally and give
  each case a real absolute cwd (`filepath.Join(t.TempDir(), "project")`) rather
  than a hardcoded `/home/...`, which is not absolute on Windows and made
  `CheckSpawn` skip the ceiling arm entirely — that is why a run could report
  only `ResumeRefused` and never exercise the clamp and tool-scope arms it was
  written to prove (`fresh_test.go`, fixed in `75220468`).
- **STATE, checked 2026-09-01 16:40Z: `containment-windows` is STILL RED on
  master.** Run `33533220982` on `0bac5799` passed desktop, hub (Linux),
  claudemon, tui and conflict-markers, and failed `containment-windows` on
  `TestBriefToolsAcceptWindowsManagerWorkspaceSpellings` and two
  `TestPathContainmentContractCases` arms — "a case-variant spelling of a store
  carve-out is NOT exempted" and "a sibling differing from the root only in case
  is not inside it". Note the direction: those are cases where case-insensitive
  matching must NOT apply, so the ordinal fix and the containment contract now
  disagree about where case-folding stops. Do not treat the Windows containment
  work as finished, and do not assume a green Linux hub job says anything about
  it.
