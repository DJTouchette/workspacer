---
title: Migration backlog counts represent pending evidence not missing Rust implementations
date: 2026-09-29
related_paths:
  - scripts/hub-migration.py
promoted: false
---

# Migration backlog counts represent pending evidence not missing Rust implementations

## Observation
The migration status command reports only totals. Added read-only backlog command grouping pending ledger entries by legacy package with JSON and source-prefix filters so repeated audit assignments can use current inventory without ad-hoc parsing. Pending cutover gates remain visible even when source rows are filtered. Witness does not map the Python CLI to its test file; the complete scripts/test_hub_migration.py suite was run explicitly.
