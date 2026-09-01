---
title: filepath.Join erases dirty-path test fixtures
date: 2026-09-01
author: Codex
confidence: high
suggested_doc: limit-aware-routing
related_paths:
  - services/hub/internal/routing/matrix.go
  - services/hub/internal/routing/matrix_test.go
promoted: false
---

# filepath.Join erases dirty-path test fixtures

## Observation
A routing ceiling test that builds `root/a/../client` with filepath.Join never exercises CeilingFor's internal filepath.Clean: Join cleans the path before the function receives it, and can collapse supposedly distinct clean/dirty map keys into one key.

## Impact
Clean and equal-depth tie-break regressions can appear covered while the test actually has only one effective ceiling key.

## Recommendation
Construct intentionally dirty lexical paths with explicit platform separators, assert the strings remain distinct, and only then pass them to CeilingFor.
