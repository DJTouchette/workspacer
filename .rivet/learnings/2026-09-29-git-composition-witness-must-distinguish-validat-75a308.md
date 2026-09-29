---
title: Git composition witness must distinguish validation matches from execution dispatch
date: 2026-09-29
suggested_doc: hub-shared-cap-event-vocabulary
related_paths:
  - apps/desktop/tests/support/compositionWitnesses.ts
promoted: false
---

# Git composition witness must distinguish validation matches from execution dispatch

## Observation
Typed Git field validation legitimately matches method before cwd canonicalization. The old witness used the first lexical match method as dispatch and failed green runtime behavior. The repaired structural witness identifies the top-level match containing real run calls, requires one top-level canonical cwd declaration before root or run execution, and verifies root consumes cwd.

## Impact
Fixes the desktop CI regression without moving validation after filesystem access or weakening canonical-path execution proof.

## Recommendation
Keep mutants for canonicalization moved into one arm, after root execution, raw requested root use, and quoted guard diagnostics.
