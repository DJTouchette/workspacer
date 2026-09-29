---
title: Source review exposed fleet ancestry and blocked wake differences
date: 2026-09-29
promoted: false
---

# Source review exposed fleet ancestry and blocked wake differences

## Observation
Go fleetSkipsPermissions walks trusted recorded parent ancestry with cycle protection; a Rust immediate-parent-only check missed grandchildren. SpawnCoordinator now traverses authoritative local lifecycle/replacement metadata and reads current fleetFullAccess for each launch; token scope is never derived from this approval preference. Go blockWatcher preserves one survival window across approval-to-question transitions and recomputes current recipients after20s; Rust had rearmed and captured recipients at the initial edge. New regressions preserve continuous blocks, late managers, ended/vanished/forgotten rows, first-sighting versus boot-priming, failure isolation and missed-finish grace. These findings came from matching actual Go assertions, not method registration or filenames; targeted Cargo validation is in progress.
