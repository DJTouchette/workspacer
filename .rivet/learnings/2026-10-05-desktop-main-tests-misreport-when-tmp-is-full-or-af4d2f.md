---
title: Desktop main tests misreport when /tmp is full or TMPDIR is under HOME
date: 2026-10-05
confidence: high
related_paths:
  - apps/desktop/src/main/ipc.test.ts
  - apps/desktop/src/main/lib/webviewGuard.test.ts
promoted: false
---

# Desktop main tests misreport when /tmp is full or TMPDIR is under HOME

## Observation
With the per-user /tmp quota exhausted, npm run test:main reports every file as '(0 test)' failures. Moving TMPDIR under $HOME avoids that but then ipc.test.ts file:read confinement, webviewGuard 'attach src' and (under load) managerTombstone retention fail, because temp paths become in-root for the home workspace root. All three passed again with the default TMPDIR (129/129). Native cargo tests and Playwright scratch dirs are fine with TMPDIR elsewhere (e2e scratch has its own root).

## Impact
These look like regressions but are environment artifacts; misreading them wastes a review cycle.

## Recommendation
Run desktop main with the default /tmp after freeing only your own temp dirs; use a non-HOME TMPDIR (outside every workspace root) only for cargo/native runs.
