---
title: workspacer serve CLI reuses desktop's RestartBackoff semantics, not internal/supervisor
date: 2026-07-11
confidence: high
related_paths:
  - services/hub/cmd/workspacer/child.go
  - services/hub/cmd/workspacer/backoff.go
  - services/hub/internal/supervisor/*.go
  - apps/desktop/src/main/*daemonUtils*
promoted: true
---

# workspacer serve CLI reuses desktop's RestartBackoff semantics, not internal/supervisor

## Observation
services/hub/cmd/workspacer/child.go + backoff.go implement a bespoke process supervisor for `workspacer serve`'s two children (claudemon, hub) rather than reusing services/hub/internal/supervisor. Two deliberate differences: (1) child stdout/stderr is line-prefixed and forwarded to the operator's terminal (internal/supervisor discards it) via prefixWriter in child.go; (2) restartBackoff (backoff.go) gives up permanently after 10 consecutive crashes within a 1-minute healthy-reset window (base 1s, doubling, cap 30s) and sets child.gaveUp — internal/supervisor retries forever. This is a direct behavioral port of apps/desktop daemonUtils.ts RestartBackoff. Also: child.go wires a parent-death pipe (os.Pipe, child's stdin=parentR) as a second SIGKILL-of-launcher safety net alongside WORKSPACER_PARENT_PID env (see internal/parentwatch), and cmd.WaitDelay=6s escalates SIGTERM to SIGKILL if a child lingers.

## Impact
Anyone tempted to unify this with internal/supervisor (DRY refactor) would silently change headless-server UX: logs would vanish and crash-looping would retry forever instead of surfacing a clear give-up message. The two supervisors must stay separate on purpose.

## Recommendation
When touching process supervision in this repo, check whether the caller is the launcher (services/hub/cmd/workspacer, wants terminal-visible logs + bounded retries) vs the hub's own plugin/sidecar supervision (internal/supervisor, wants infinite retries, no operator terminal) before consolidating.

## Disposition
Not folded: already reflected in .rivet/context/modules/workspacer-serve-cli.md (twin-supervisor gotcha with both behavioral differences).
