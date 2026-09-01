---
title: Fleet Manager ownership vocabulary needs a shared contract
date: 2026-08-31
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/shared/modelVocabulary.ts
  - services/hub/internal/modelselection/manager.go
promoted: false
---

# Fleet Manager ownership vocabulary needs a shared contract

## Observation
The Hub manager config parser independently reimplements the desktop model-vocabulary ownership patterns; without a fixture, a matcher update on either side can accept a foreign provider model while both local test suites stay green.

## Impact
A persisted manager model can reach the wrong CLI or be silently dropped only on one provider.

## Recommendation
Maintain contracts/model-vocabulary-ownership-cases.json with representative ids for every matcher and test the full provider x model foreignness matrix from both TS and Go.
