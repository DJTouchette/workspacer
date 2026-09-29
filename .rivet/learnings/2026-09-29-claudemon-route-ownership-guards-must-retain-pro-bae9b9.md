---
title: Claudemon route ownership guards must retain production after test modules
date: 2026-09-29
author: codex
confidence: high
suggested_doc: claudemon-http-api
related_paths:
  - apps/desktop/src/main/services/claudemonRouteContract.test.ts
  - apps/desktop/tests/support/claudemonCallers.json
promoted: false
---

# Claudemon route ownership guards must retain production after test modules

## Observation
The portable caller guard found Rust sources with cfg(test) items before production callbacks and a complete test module before a production routing sampler impl. Truncating at the first cfg(test), or even the first test module, silently drops real callers. The replacement removes balanced test modules while retaining later source, inventories raw embedded Command::Request paths alongside external HTTP callers, and retains independent repository discovery plus stale orphan declarations.

## Impact
A fixture can match served routers while the caller inventory silently ignores current Rust implementations; route deletion would then appear safe.

## Recommendation
Keep the scanner mutation battery and exact caller floors. Retarget the generated claudemon route loader together with routes_contract.rs only after all four ownership checks pass.
