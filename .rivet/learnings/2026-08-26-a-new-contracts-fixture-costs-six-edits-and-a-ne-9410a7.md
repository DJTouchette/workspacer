---
title: A new contracts/ fixture costs six edits, and a new library.list param costs two capspec floors
date: 2026-08-26
confidence: high
suggested_doc: mcp-tool-facade
related_paths:
  - contracts/*.json
  - contracts/README.md
  - services/hub/internal/capspec/capspec*.go
  - services/hub/cmd/brain/capspec_params_test.go
  - services/hub/cmd/brain/corpusvocab_test.go
  - apps/desktop/src/main/services/contractsVocabulary.test.ts
promoted: false
---

# A new contracts/ fixture costs six edits, and a new library.list param costs two capspec floors

## Observation
Adding contracts/dispatch-template-params-cases.json required, all enforced by tests: (1) at least TWO loaders in TWO different languages (contracts_test.go TestEveryContractFixtureHasAtLeastTwoLoaders), (2) a row in contracts/README.md naming the fixture (both directions checked), (3) a `vocabulary.blocks` registry inside the fixture declaring every array-of-objects block with `required` + `loaders` needles ("<repo-relative file>::<needle>"), (4) BOTH fixture-count floors bumped in lockstep — `contractsFixtureFloor` (cmd/brain/corpusvocab_test.go) and `CONTRACTS_FIXTURE_FLOOR` (contractsVocabulary.test.ts), 8 -> 9, (5) a per-corpus case floor in each loader. Declaring an `optional` field that no case carries FAILS (optional-used), as does any case key not in required/optional (unknown-fields). Nested arrays inside a case row are NOT walked, so `expect: [ {...} ]` needs no declaration.

Separately: giving library.list a `kind`/`id` filter cost two ratchets, because `id` is in capspec's dangerousParams vocabulary (KindID) and `kind` is not. Needed a `"library.list": {"id": {KindID, reason}}` entry in capspec.go's decision map AND both scans' floors: `desktopDangerousParamFloor` 70 -> 71 (internal/capspec/capspec_test.go, scans hubCapabilities.ts) and `brainDangerousParamFloor` 64 -> 65 (cmd/brain/capspec_params_test.go, scans handlers.go's dispatch switch). RatchetError fails on a RISE as well as a fall, so both must move.</observation>
<parameter name="impact">Anyone adding a cross-language fixture or a param named from capspec's vocabulary will otherwise chase four or five red guards one at a time without knowing the list is finite.

## Recommendation
Budget these six edits up front. `go test ./cmd/brain/ -run 'Contract|Corpus|Vocab|Fixture'` plus `go test ./internal/capspec/` is the fast loop that surfaces all of them.
