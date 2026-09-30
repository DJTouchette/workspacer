---
title: Go reference removal needs retained evidence and post-removal CI
date: 2026-09-30
suggested_doc: services/hub-rs/CUTOVER_STATUS.md
related_paths:
  - services/hub-rs/migration.json
promoted: false
---

# Go reference removal needs retained evidence and post-removal CI

## Observation
All534 review records and portable guards passed with the legacy tree absent. Before tracked removal,38 non-Go inputs were checked against pinned historical hashes:32 have retained active owners,6 belong to the explicit historical checkout. Thirteen non-deletion gates now have reviewed exact-revision receipts; the deletion gate remains pending until post-removal CI succeeds.

## Impact
Source parity and a temporary absence probe do not by themselves certify physical deletion or the published artifact revision.

## Recommendation
Keep original hashes/captures and scoped CI URLs, remove tracked legacy files only, and close the final gate after reviewing post-removal CI.
