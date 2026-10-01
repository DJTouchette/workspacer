---
title: Skill pointer test expectations must canonicalize temporary project paths
date: 2026-10-01
suggested_doc: agent-spawn
related_paths:
  - services/hub-rs/tests/local_spawn.rs
promoted: false
---

# Skill pointer test expectations must canonicalize temporary project paths

## Observation
The launch instruction installer canonicalizes cwd before constructing its skill pointer. On macOS temporary project paths under /var alias /private/var; comparing injected instructions against an uncanonicalized project path fails despite correct installation. local_spawn now canonicalizes its expected skill root. Codex assertions inspect a portable skill-path suffix and do not compare alias roots.

## Impact
Linux-only test runs did not expose the macOS path alias difference.

## Recommendation
Match filesystem-backed instruction pointers using canonical expected paths in cross-platform fixtures.
