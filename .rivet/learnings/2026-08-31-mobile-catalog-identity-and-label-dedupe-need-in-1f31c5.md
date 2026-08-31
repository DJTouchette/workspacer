---
title: Mobile catalog identity and label dedupe need independent regressions
date: 2026-08-31
confidence: high
suggested_doc: remote-mobile
related_paths:
  - apps/desktop/tests/e2e/mobileClient.test.ts
  - apps/desktop/tests/e2e/fixtures/mobileHub.ts
  - services/hub/cmd/hub/mobile.html
promoted: false
---

# Mobile catalog identity and label dedupe need independent regressions

## Observation
A canonical alias without contextWindow can be represented again by a marker-bearing seen entry, so a click assertion can pass even if alias contextWindow recovery is removed. Separately, distinct seen ids can share the same short phone label while avoiding pair identity dedupe.

## Impact
A single menu-count or selection assertion does not make both production guards load-bearing.

## Recommendation
Keep an unwindowed-alias absence assertion for recovery and a distinct same-label seen fixture for seenLabels.
