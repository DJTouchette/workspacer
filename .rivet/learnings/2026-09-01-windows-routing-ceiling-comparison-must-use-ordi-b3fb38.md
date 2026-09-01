---
title: Windows routing ceiling comparison must use ordinal Unicode casing
date: 2026-09-01
author: Codex
confidence: high
suggested_doc: limit-aware-routing
related_paths:
  - services/hub/internal/routing/pathmatch_windows.go
  - services/hub/internal/routing/pathmatch_windows_semantics.go
  - services/hub/internal/routing/matrix.go
  - services/hub/internal/routing/matrix_windows_test.go
promoted: false
---

# Windows routing ceiling comparison must use ordinal Unicode casing

## Observation
The host-independent helper in pathmatch_windows_semantics.go lowercases only ASCII bytes. On Windows, a configured ancestor that differs from the canonical cwd only by a non-ASCII case pair (for example Cyrillic or Ä/ä) misses the ceiling entirely and falls through to the default. strings.EqualFold is also unsafe because it equates generic Unicode fold cycles such as Kelvin sign K with K, unlike Windows ordinal filename comparison.

## Impact
This is a production fail-open at the per-directory capability/tool-scope ceiling: the wrong default ceiling can govern a canonical Windows path.

## Recommendation
Use the native Windows CompareStringOrdinal API with ignoreCase=true for equality/prefix comparison, fail closed on API failure, and keep contract/mutation tests for ASCII, Cyrillic, umlaut, Kelvin, drive/UNC, separator and sibling boundaries.
