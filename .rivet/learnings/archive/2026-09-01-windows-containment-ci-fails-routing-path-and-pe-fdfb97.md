---
title: Windows containment CI fails routing path and permission assumptions
date: 2026-09-01
confidence: high
suggested_doc: workflow-subagent-watcher
related_paths:
  - .github/workflows/ci.yml
  - services/hub/internal/routing/**/*.go
promoted: true
promoted_to: limit-aware-routing
---

# Windows containment CI fails routing path and permission assumptions

## Observation
CI run 33495681039 for reviewed SHA 2d8f7fd0 passed desktop, hub Linux, claudemon, TUI, conflict-marker, formatting, typecheck, and e2e gates, but containment-windows failed Go routing tests. Failures include ancestor ceiling selection resolving to default and a decision-log mode of -rw-rw-rw- instead of 0600.

## Impact
This is a mandatory master CI job, so release.yml nightly dispatch cannot proceed even though all other required jobs pass.

## Recommendation
Fix or platform-condition the Windows routing path and private-permission assertions, then push a reviewed SHA and rerun CI before dispatching nightly.

## Disposition
Promoted into `.rivet/context/modules/limit-aware-routing.md` as the durable Windows path-semantics rule (ordinal `CompareStringOrdinal` comparison, build-tag-split pathmatch files, ACL-based decision-log privacy in place of a meaningless 0600 assertion). The incident on run `33495681039` is spent. `suggested_doc: workflow-subagent-watcher` was wrong — this is routing, not the subagent watcher. NOT recorded as repaired: `containment-windows` is still red on `0bac5799` (run `33533220982`), with different failures.
