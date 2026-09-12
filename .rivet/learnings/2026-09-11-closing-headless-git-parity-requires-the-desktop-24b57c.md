---
title: Closing headless Git parity requires the desktop staging composition boundary
date: 2026-09-11
confidence: high
suggested_doc: git-review
related_paths:
  - services/hub/cmd/brain/git*.go
  - apps/desktop/src/main/services/hubCapabilities.ts
promoted: false
---

# Closing headless Git parity requires the desktop staging composition boundary

## Observation
The existing Go Git provider already uses the desktop no-exec argv and secret-definition path guards, but exposes only four read methods. The stage/unstage port must anchor explicit operands to BOTH derived repository root and workspace roots; pathless calls must become the guarded cwd pathspec. Otherwise a monorepo subdirectory token can stage sibling secrets and retrieve them with staged diff. The user explicitly requested full web parity, superseding historical comments that intentionally omitted all Git writes.
