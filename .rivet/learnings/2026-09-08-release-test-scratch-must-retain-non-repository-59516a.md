---
title: Release test scratch must retain non-repository and outside-home semantics
date: 2026-09-08
confidence: high
related_paths:
  - docs/release-checks.md
  - apps/desktop/tests/support/tmpdirCleanup.ts
promoted: false
---

# Release test scratch must retain non-repository and outside-home semantics

## Observation
Setting TMPDIR beneath the release checkout made seven desktop main assertions fail: plain temp directories inherited the enclosing Git repo, outside-root fixtures fell inside the real home allowlist, and webview/provider filesystem guards saw hidden workspacer ancestor paths. Keep fixture-owned os.tmpdir sandboxes on the normal temporary filesystem; private dependency/build caches can stay in the checkout. This is separate from the independent cgroup resource limit.

## Recommendation
Use scripts/release-check.py with normal TMPDIR semantics and explicit private cache paths. Do not alter production guards to accommodate a changed fixture location.
