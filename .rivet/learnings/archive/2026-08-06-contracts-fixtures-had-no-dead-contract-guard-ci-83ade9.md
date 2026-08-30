---
title: contracts/ fixtures had no dead-contract guard; CI cannot catch one
date: 2026-08-06
promoted: true
---

# contracts/ fixtures had no dead-contract guard; CI cannot catch one

## Observation
ci.yml runs four per-stack jobs with working-directory set, so nothing ever enumerates contracts/. A fixture added with only one loader (or two loaders in the SAME language) goes fully green while pinning nothing. services/hub/cmd/brain/contracts_test.go (TestEveryContractFixtureHasAtLeastTwoLoaders) now discovers fixtures via os.ReadDir and loaders by walking the repo for .ts/.tsx/.go/.rs files containing the fixture basename, requiring >=2 files AND >=2 languages, plus a two-way check against contracts/README.md's owner table. The walk must skip .claude (the stale .claude/worktrees/electron-43-upgrade checkout holds older copies of both fixtures and loaders and would fake a second loader) as well as node_modules/target/dist/release/build/.git. All 6 fixtures pass today.

## Disposition
Folded into .rivet/context/modules/hub-shared-cap-event-vocabulary.md (contract-guard note).
