---
title: Release recovery must reconcile master and published nightly independently
date: 2026-09-08
confidence: high
suggested_doc: auto-update-release-channel
related_paths:
  - .github/workflows/release.yml
promoted: false
---

# Release recovery must reconcile master and published nightly independently

## Observation
On 2026-09-08 both git ls-remote and GitHub branches/master returned 3a289d1b, while published nightly and successful CI 34272696143 target f838c1f3. The nightly includes launch integrations and several repairs overlapping WIP 19bec1cd; local f11c0977 separately contains the measured-allowance profile fix. The recovery branch preserves both histories and only replays the unique Windows lock changes from WIP.

## Recommendation
Before publication fetch current master and inspect the published nightly target independently; compare patches and preserve unique committed changes. Never assume the rolling local nightly tag is current or overwrite it during preparation.
