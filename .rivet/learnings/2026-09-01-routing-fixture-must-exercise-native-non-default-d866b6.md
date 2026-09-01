---
title: Routing fixture must exercise native non-default ceiling
date: 2026-09-01
confidence: high
suggested_doc: limit-aware-routing
related_paths:
  - services/hub/internal/routing/fresh_test.go
  - services/hub/internal/routing/matrix_test.go
promoted: false
---

# Routing fixture must exercise native non-default ceiling

## Observation
The fresh routing ceiling test must construct one filepath.Join(t.TempDir(), "project") value and use it for both CanonicalCwd and the quoted YAML ceiling key; otherwise Unix-only hard-coded paths either fail to parse on Windows or let the test pass through the default ceiling. The assertion on the ceiling key is part of the regression proof.

## Impact
medium

## Recommendation
Keep platform-sensitive routing fixtures native and assert the matched key/value, not merely the freshness refusal.
