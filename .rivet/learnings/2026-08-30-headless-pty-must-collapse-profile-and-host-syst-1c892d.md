---
title: Headless PTY must collapse profile and host system prompts
date: 2026-08-30
confidence: high
suggested_doc: agent-spawn
related_paths:
  - services/hub/cmd/brain/profiles.go
  - services/hub/cmd/brain/handlers.go
  - services/hub/cmd/brain/facade_test.go
  - apps/desktop/src/main/services/claudeResolver.ts
promoted: false
---

# Headless PTY must collapse profile and host system prompts

## Observation
The Go brain's PTY argv builder preserved profile --append-system-prompt flags, then appended facade/fleet contracts as separate flags. Claude does not define repeated append flags as concatenation. The desktop already partitions split and equals-form profile pins, then joins profile fragments before host text with blank lines.

## Impact
A fleet contract can be lost or a profile instruction overridden depending on CLI repeated-flag handling.

## Recommendation
For headless PTY spawns, allow the profile pin through the remote profile scrub and collapse split/equals-form append-system-prompt entries into one flag in argv order; regression-test both forms.
