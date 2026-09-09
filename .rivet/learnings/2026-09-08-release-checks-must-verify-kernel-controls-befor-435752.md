---
title: Release checks must verify kernel controls before exec
date: 2026-09-08
confidence: high
suggested_doc: auto-update-release-channel
related_paths:
  - scripts/release-check.py
  - scripts/smoke_release_check.py
  - docs/release-checks.md
promoted: false
---

# Release checks must verify kernel controls before exec

## Observation
The release runner uses a sibling transient user service and verifies memory.high, memory.max, memory.swap.max, cpu.max, pids.max and memory.oom.group before exec. systemd 260 accepts OOMPolicy=kill, which sets memory.oom.group=1; MemoryOOMGroup is not a supported transient assignment. A service OOM returns systemd-run status 1 even when its main process died with SIGKILL. A tiny memory test with high below max can throttle until timeout instead of reaching the ceiling.

## Recommendation
Use scripts/release-check.py for sequential package-bounded release checks. Keep high equal to max only in the tiny disposable OOM smoke, and require the oom-kill result rather than assuming exit 137.
