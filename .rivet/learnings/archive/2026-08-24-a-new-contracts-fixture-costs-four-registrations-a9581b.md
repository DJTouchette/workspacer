---
title: A new contracts/ fixture costs four registrations, not one
date: 2026-08-24
confidence: high
suggested_doc: hub-jobs
related_paths:
  - contracts/*.json
  - contracts/README.md
  - apps/desktop/src/main/services/contractsVocabulary.test.ts
  - services/hub/cmd/brain/corpusvocab_test.go
  - services/hub/cmd/brain/contracts_test.go
promoted: true
promoted_to: registration-checklists
---

# A new contracts/ fixture costs four registrations, not one

## Observation
Adding any file to contracts/ trips guards in both languages. A fixture with no array-of-objects case blocks (e.g. a single job spec) must be added to TWO exemption allow-lists or the suite goes red: NO_BLOCK_FIXTURES in apps/desktop/src/main/services/contractsVocabulary.test.ts and vocabExempt in services/hub/cmd/brain/corpusvocab_test.go (twins, same check IDs). On top of that, services/hub/cmd/brain/contracts_test.go requires every fixture to be named by at least two TEST files in at least two different LANGUAGES (an implementation file naming it counts only as a "mention"), and it also checks the owner table in contracts/README.md. So the real cost of a new fixture is: the JSON, a Go loader test, a TS/Rust loader test, a README row, and (if it carries no cases) both exemption lists.

## Impact
contracts/job-preset-power-down.json failed contractsVocabulary.test.ts on first run with "no `vocabulary` block". The Go twin would have failed the same way on the next hub run.

## Recommendation
Before adding a contracts/ fixture, decide whether it carries case blocks. If not, register it in both exemption lists in the same commit. Either way, land its two loaders and the README row together, or the loader-count guard fails.
