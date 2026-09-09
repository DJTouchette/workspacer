---
title: Hosted release CI exposes partial recording and cross-volume fixture races
date: 2026-09-08
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - services/claudemon/src/daemon/api.rs
  - services/hub/cmd/brain/contracts_test.go
promoted: false
---

# Hosted release CI exposes partial recording and cross-volume fixture races

## Observation
Candidate 371a9493 CI run 34301172213 failed claudemon execution_engine_native_launch_golden_cases after await_recording accepted a nonempty but incomplete argv file. Shell printf can write arguments in separate writes. Its recording fixtures now write a sibling .part file and rename only after completion. Windows TestContractSourceWalkExcludesRuntimeCaches also failed because t.TempDir used C: while the Go module lived on D:; extinput intentionally refuses paths that filepath.Rel cannot represent. The fixture now allocates and cleans up a unique directory on the package volume.

## Impact
Sequential Linux preparation passed both tests; default hosted concurrency and Windows volume layout exposed test harness assumptions, not product failures.

## Recommendation
Publish recordings atomically and keep synthetic extinput fixtures on the module volume; retain extinput fail-closed checks and the full argument assertions.
