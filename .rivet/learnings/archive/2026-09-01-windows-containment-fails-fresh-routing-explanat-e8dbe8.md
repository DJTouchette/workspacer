---
title: Windows containment fails fresh-routing explanation arm assertion
date: 2026-09-01
confidence: high
suggested_doc: limit-aware-routing
related_paths:
  - services/hub/internal/routing/fresh_test.go
  - .github/workflows/ci.yml
promoted: true
promoted_to: limit-aware-routing
---

# Windows containment fails fresh-routing explanation arm assertion

## Observation
CI run 33503823057 on SHA 710d8e8b passed the desktop, Linux hub, Rust, and formatting jobs but containment-windows failed TestACeilingClampAndAFreshRefusalAreBothExplained. The test received only ResumeRefused=true for a deep_reviewer resume; it did not observe the expected capability-clamp and tool-scope-refusal arms.

## Impact
The failure is in the mandatory Windows containment Go suite and blocks nightly release publication; the later TypeScript drive/UNC and ACL guard phase is skipped.

## Recommendation
Review Windows-specific routing scenario setup and assertion ordering in fresh_test.go so the three intended arms are all exercised, then rerun complete CI on a reviewed successor.

## Disposition
Merged into the same `limit-aware-routing.md` section. The specific arm failure was fixed in `75220468`; the durable rule kept is the fixture one — `filepath.Join` erases the dirty paths a path test exists to exercise, and a hardcoded `/home/...` cwd is not absolute on Windows, so `CheckSpawn` skipped the ceiling arm and the run could only ever report `ResumeRefused`.
