---
title: Windows routing ceilings need native path semantics and ACL privacy
date: 2026-09-01
author: Codex
confidence: high
suggested_doc: limit-aware-routing
related_paths:
  - services/hub/internal/routing/matrix.go
  - services/hub/internal/routing/decisionlog.go
  - services/hub/internal/nodes/exposure_windows.go
promoted: false
---

# Windows routing ceilings need native path semantics and ACL privacy

## Observation
The containment-windows failures at run 33495681039 were caused by Unix-rooted test paths and Unix mode-bit assertions, but tracing the production lookup also exposed a separate fail-open case: Windows canonical cwd values can differ from routing.yaml keys only by case or slash spelling, and the byte-exact ancestor match then falls back to the weaker default ceiling. Go reports synthetic 0666/0444 mode bits on Windows, so decision-log confidentiality must be established and checked through its DACL. Existing loose decision logs also need repair before append/rotation; OpenFile's 0600 only affects a newly created Unix file.

## Impact
A stricter per-directory routing ceiling could be missed on Windows, and audit rows could inherit a readable Windows ACL or preserve a loose pre-existing Unix mode.

## Recommendation
Use native absolute temp paths in cross-platform routing tests; normalize separators and case-fold ceiling comparisons only on Windows; use the nodes.FileExposure ACL/mode abstraction as the privacy oracle; establish an owner/SYSTEM/Administrators protected DACL before writing Windows decision-log bytes and chmod existing Unix logs before append.
