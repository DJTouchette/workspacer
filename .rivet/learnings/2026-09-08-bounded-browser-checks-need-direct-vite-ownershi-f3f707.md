---
title: Bounded browser checks need direct Vite ownership and explicit local startup budgets
date: 2026-09-08
confidence: high
related_paths:
  - apps/desktop/tests/e2e/chatTailPin.test.ts
  - apps/desktop/tests/e2e/usagePacing.test.ts
  - docs/release-checks.md
promoted: false
---

# Bounded browser checks need direct Vite ownership and explicit local startup budgets

## Observation
With TasksMax=128, chatTailPin using an npx wrapper hid an esbuild newosproc failure (errno 11, only six esbuild threads); direct Vite execution removed wrapper overhead and made both geometry tests pass. First-use then independently failed the five-second Welcome assertion: a routed browser probe measured 5923 ms until Welcome on CPUQuota=100%. Command-local UV_THREADPOOL_SIZE=1/GOMAXPROCS=1 and a local Playwright config with 15-second assertions and 60-second tests preserve the same selected cases without raising resource limits. CI config remains unchanged.

## Recommendation
Launch fixture Vite with process.execPath and the renderer-local bin path so teardown targets the actual server. Capture server output. Record constrained-host timeout/thread-pool overrides with the results; do not treat earlier timeouts as passing checks or weaken live-state guards.
